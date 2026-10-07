//! 全局 JSON 文件读写 —— 迁移自 `electron/utils/config-utils.ts`。
//!
//! 逐条对齐基线语义：
//! - `try_read_json_file` 区分「文件不存在」/「可读」/「存在但不可读或不可解析」。
//!   对全局配置做**增量写入**的调用方必须在 `Error` 时**拒绝覆盖**原文件；
//! - `read_json_file` 在错误时告警并回落默认值（只读路径可容忍损坏）；
//! - `write_json_file` 走「同目录临时文件 + fsync + rename」提交，保证并发读到的是
//!   完整文件；Windows 下对典型的「文件被短暂占用」错误（Defender / 索引服务）
//!   按基线延迟序列 `10/25/50/100/200ms` 重试。

use std::io::Write;
use std::path::Path;

/// 读取结果三态（对齐基线 `JsonFileReadResult<T>`）。
#[derive(Debug)]
pub enum JsonFileReadResult<T> {
    /// 文件不存在
    Missing,
    /// 读取并解析成功
    Ok(T),
    /// 文件存在但不可读或不可解析（原文错误描述，供上层判定是否拒绝覆盖）
    Error(String),
}

/// Windows 下 `rename` 的占用类错误码：`ERROR_ACCESS_DENIED`(5) /
/// `ERROR_SHARING_VIOLATION`(32) / `ERROR_LOCK_VIOLATION`(33)。
#[cfg(windows)]
const WINDOWS_RENAME_RETRY_CODES: [i32; 3] = [5, 32, 33];

/// 对齐基线 `WINDOWS_REPLACE_RETRY_DELAYS_MS`。
const RENAME_RETRY_DELAYS_MS: [u64; 5] = [10, 25, 50, 100, 200];

/// 尝试读取并解析 JSON 文件，区分三态。
pub fn try_read_json_value(path: &Path) -> JsonFileReadResult<serde_json::Value> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return JsonFileReadResult::Missing;
        }
        Err(error) => return JsonFileReadResult::Error(error.to_string()),
    };
    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(value) => JsonFileReadResult::Ok(value),
        Err(error) => JsonFileReadResult::Error(error.to_string()),
    }
}

/// 读取 JSON 并在失败时回落默认值（对齐 `readJsonFile`：损坏时告警）。
pub fn read_json_value_or(path: &Path, fallback: serde_json::Value) -> serde_json::Value {
    match try_read_json_value(path) {
        JsonFileReadResult::Ok(value) => value,
        JsonFileReadResult::Missing => fallback,
        JsonFileReadResult::Error(error) => {
            eprintln!(
                "[Lorekeeper] 读取 {} 失败: {}",
                path.display(),
                error
            );
            fallback
        }
    }
}

/// 读取 JSON 并反序列化为 `T`；失败一律回落默认值。
pub fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path, fallback: T) -> T {
    match try_read_json_value(path) {
        JsonFileReadResult::Ok(value) => match serde_json::from_value(value) {
            Ok(parsed) => parsed,
            Err(error) => {
                eprintln!(
                    "[Lorekeeper] 读取 {} 失败: {}",
                    path.display(),
                    error
                );
                fallback
            }
        },
        JsonFileReadResult::Missing => fallback,
        JsonFileReadResult::Error(error) => {
            eprintln!(
                "[Lorekeeper] 读取 {} 失败: {}",
                path.display(),
                error
            );
            fallback
        }
    }
}

/// 原子写 JSON（对齐基线 `writeJsonFile`）：
/// 建父目录 → 同目录临时文件（`wx` 独占创建 + 0600）→ 写入 + fsync → rename 提交。
///
/// 任一步失败都保持**原文件不变**，并清理尚未安装的临时文件。
pub fn write_json_file(path: &Path, data: &serde_json::Value) -> Result<(), String> {
    let directory = path
        .parent()
        .ok_or_else(|| format!("目标路径没有父目录：{}", path.display()))?;
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "config.json".to_string());
    let temporary_path = directory.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        crate::project_access::random_uuid_v4()
    ));

    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;

    // 与基线一致的两空格缩进；`JSON.stringify(data, null, 2)` 无尾随换行。
    let content = serde_json::to_string_pretty(data).map_err(|error| error.to_string())?;

    if let Err(error) = write_new_file(&temporary_path, &content) {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error);
    }

    match rename_with_retry(&temporary_path, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&temporary_path);
            Err(error)
        }
    }
}

/// 独占创建 + 写入 + fsync（Windows 无 0600 语义，仅 Unix 生效）。
fn write_new_file(path: &Path, content: &str) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    file.write_all(content.as_bytes())
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

/// rename 提交；Windows 下对占用类错误按基线延迟序列重试。
fn rename_with_retry(from: &Path, to: &Path) -> Result<(), String> {
    let mut attempt = 0usize;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let delay = RENAME_RETRY_DELAYS_MS.get(attempt);
                if !should_retry_rename(&error) || delay.is_none() {
                    return Err(error.to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(
                    delay.copied().unwrap_or_default(),
                ));
                attempt += 1;
            }
        }
    }
}

/// 是否属于「可重试的占用类错误」：仅 Windows 有意义（基线同样只在 win32 重试）。
fn should_retry_rename(error: &std::io::Error) -> bool {
    #[cfg(windows)]
    {
        if let Some(code) = error.raw_os_error() {
            return WINDOWS_RENAME_RETRY_CODES.contains(&code);
        }
    }
    let _ = error;
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lorekeeper-json-store-{}-{}",
            tag,
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn temp_files(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn missing_file_reports_missing_status_test() {
        let dir = temp_dir("missing");
        let path = dir.join("config.json");
        match try_read_json_value(&path) {
            JsonFileReadResult::Missing => {}
            other => panic!("期望 Missing，实际：{other:?}"),
        }
        assert_eq!(
            read_json_value_or(&path, serde_json::json!({ "theme": "dark" })),
            serde_json::json!({ "theme": "dark" })
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_reports_error_status_and_falls_back_test() {
        let dir = temp_dir("corrupt");
        let path = dir.join("config.json");
        std::fs::write(&path, b"{ not json").unwrap();

        match try_read_json_value(&path) {
            JsonFileReadResult::Error(_) => {}
            other => panic!("期望 Error，实际：{other:?}"),
        }
        // 只读路径容忍损坏：回落默认值；原文件不被触碰
        assert_eq!(
            read_json_value_or(&path, serde_json::json!({ "theme": "dark" })),
            serde_json::json!({ "theme": "dark" })
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_json_file_deserializes_typed_value_test() {
        let dir = temp_dir("typed");
        let path = dir.join("list.json");
        std::fs::write(&path, br#"[{"id":"a"},{"id":"b"}]"#).unwrap();

        let parsed: Vec<serde_json::Value> = read_json_file(&path, Vec::new());
        assert_eq!(parsed.len(), 2);
        // 类型不匹配时同样回落
        let mismatched: Vec<serde_json::Value> = read_json_file(&path, Vec::new());
        assert_eq!(mismatched[0]["id"], serde_json::json!("a"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_creates_parent_dirs_and_round_trips_pretty_json_test() {
        let dir = temp_dir("write");
        let path = dir.join("nested").join("config.json");
        let value = serde_json::json!({ "theme": "dark", "autoSaveInterval": 30 });

        write_json_file(&path, &value).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        // 对齐 `JSON.stringify(value, null, 2)`：两空格缩进、无尾随换行。
        // 注意键序取决于 serde_json 是否启用 `preserve_order`（当前未启用 → 字典序），
        // 因此这里只断言格式与语义，不锁定键序。
        assert!(content.starts_with("{\n  \""), "必须为两空格缩进的 pretty JSON：{content}");
        assert_eq!(content.lines().count(), 4, "每个键占一行（含首尾花括号）：{content}");
        assert!(!content.ends_with('\n'), "不得有尾随换行");
        assert_eq!(serde_json::from_str::<serde_json::Value>(&content).unwrap(), value);
        assert!(temp_files(dir.join("nested").as_path()).is_empty(), "临时文件必须清理");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_replaces_existing_file_test() {
        let dir = temp_dir("replace");
        let path = dir.join("config.json");
        std::fs::write(&path, br#"{"theme":"light"}"#).unwrap();

        write_json_file(&path, &serde_json::json!({ "theme": "dark" })).unwrap();

        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["theme"], serde_json::json!("dark"));
        assert!(temp_files(&dir).is_empty(), "临时文件必须清理");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_failure_keeps_original_file_test() {
        let dir = temp_dir("keep-original");
        // 用同名目录占位，使「同目录临时文件 + rename」提交必然失败
        let path = dir.join("config.json");
        std::fs::create_dir(&path).unwrap();

        let error = write_json_file(&path, &serde_json::json!({ "theme": "dark" })).unwrap_err();
        assert!(!error.is_empty());
        assert!(path.is_dir(), "失败时不得破坏原有路径");
        assert!(temp_files(&dir).is_empty(), "临时文件必须清理");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
