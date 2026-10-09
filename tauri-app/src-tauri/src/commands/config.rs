//! Config 频道的 Tauri 命令实现
//!
//! 对应 `src/shared/ipc-channels.ts` 的 `ConfigChannels` 与
//! `electron/controllers/config-controller.ts`：
//! - `config:get` → `config_get`：读 `~/.lorekeeper/config.json`，
//!   缺失时返回 `DEFAULT_GLOBAL_CONFIG`，损坏时告警并回落默认值；
//! - `config:set` → `config_set`：读旧值（**损坏则拒绝覆盖**）→ 浅合并
//!   `Partial<GlobalConfig>` → 原子写回（对齐基线 `writeJsonFile`）。
//!
//! 批次 A 落地时这里是内存存储（骨架）；批次 D1 补齐真实持久化，因为
//! `llm:get-default-model` / `llm:set-default-model` 等频道与它共用同一份
//! `config.json`。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::app_paths;

/// 全局配置损坏时的拒绝覆盖文案（对齐基线 `new Error('全局配置损坏，已拒绝覆盖')`）。
pub const GLOBAL_CONFIG_CORRUPTED_MESSAGE: &str = "全局配置损坏，已拒绝覆盖";

/// 默认全局配置（逐字段对齐基线 `DEFAULT_GLOBAL_CONFIG`）。
pub fn default_global_config() -> serde_json::Value {
    serde_json::json!({
        "theme": "dark",
        "defaultModelId": null,
        "autoOpenNextChapterAfterFinalize": false,
        "editorFontSize": 16,
        "editorFontFamily": "Noto Serif SC",
        "autoSaveInterval": 30,
        "proxy": {
            "enabled": false,
            "type": "http",
            "host": "",
            "port": 7890,
        },
    })
}

/// 读取全局配置（对齐 `readJsonFile(GLOBAL_CONFIG_PATH, DEFAULT_GLOBAL_CONFIG)`）。
///
/// 注意：**不做默认值合并** —— 基线只在文件缺失时使用默认值，文件存在时原样返回
/// （渲染层自行决定缺省行为）。首次 `config:set` 会以默认值为基底写出完整文件。
pub fn read_global_config_at(path: &std::path::Path) -> serde_json::Value {
    crate::json_store::read_json_value_or(path, default_global_config())
}

/// 读取用于**增量更新**的全局配置：损坏时返回中文拒绝覆盖文案。
///
/// 对齐基线 `loadGlobalConfigForUpdate()`：损坏时抛错，绝不静默覆盖用户配置。
pub fn global_config_for_update_at(path: &std::path::Path) -> Result<serde_json::Value, String> {
    use crate::json_store::JsonFileReadResult;
    match crate::json_store::try_read_json_value(path) {
        JsonFileReadResult::Ok(value) => Ok(value),
        JsonFileReadResult::Missing => Ok(default_global_config()),
        JsonFileReadResult::Error(_) => Err(GLOBAL_CONFIG_CORRUPTED_MESSAGE.to_string()),
    }
}

/// 浅合并（对齐 JS `{ ...existing, ...updates }`）。
///
/// 非对象旧值（数组 / 标量）在 JS 的展开语义下会产生下标键之类的怪异结果；
/// 这里统一视为空对象（更严格的硬化，语义差别仅出现在文件已被外部破坏时）。
fn merge_global_config(
    existing: serde_json::Value,
    updates: Option<HashMap<String, serde_json::Value>>,
) -> serde_json::Value {
    let mut merged = match existing {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    if let Some(updates) = updates {
        for (key, value) in updates {
            // JS 展开：`undefined` 值会被写入（成为缺省键）；Rust 侧 `null` 表示显式清空。
            merged.insert(key, value);
        }
    }
    serde_json::Value::Object(merged)
}

/// 对齐基线失败形态 `{ success: false, error: String(error) }`，统一带 `Error: ` 前缀。
fn failure(message: String) -> ConfigSetResponse {
    ConfigSetResponse {
        success: false,
        error: Some(format!("Error: {message}")),
    }
}

/// 纯函数核心：写入指定路径的全局配置（便于注入临时目录做单元测试）。
pub fn set_global_config_at(
    path: &std::path::Path,
    config: Option<HashMap<String, serde_json::Value>>,
) -> ConfigSetResponse {
    let existing = match global_config_for_update_at(path) {
        Ok(existing) => existing,
        Err(message) => return failure(message),
    };
    let merged = merge_global_config(existing, config);
    match crate::json_store::write_json_file(path, &merged) {
        Ok(()) => ConfigSetResponse {
            success: true,
            error: None,
        },
        Err(error) => failure(error),
    }
}

/// 设置单个配置键（`llm:set-default-model` / `-embedding-model` 复用）。
pub fn set_config_key_at(
    path: &std::path::Path,
    key: &str,
    value: serde_json::Value,
) -> ConfigSetResponse {
    let mut updates = HashMap::new();
    updates.insert(key.to_string(), value);
    set_global_config_at(path, Some(updates))
}

/// Config:get 命令（契约：args `[]`，return `GlobalConfig`）
#[tauri::command]
pub fn config_get() -> serde_json::Value {
    read_global_config_at(&app_paths::global_config_path())
}

/// Config:set 命令（契约：args `[config: Partial<GlobalConfig>]`）
#[tauri::command]
pub fn config_set(config: Option<HashMap<String, serde_json::Value>>) -> ConfigSetResponse {
    set_global_config_at(&app_paths::global_config_path(), config)
}

/// Config:set 的响应
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSetResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lorekeeper-config-{}-{}",
            tag,
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn config_path(dir: &Path) -> PathBuf {
        dir.join("config.json")
    }

    fn read(path: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn defaults_match_baseline_shape_test() {
        let defaults = default_global_config();
        assert_eq!(defaults["theme"], serde_json::json!("dark"));
        assert_eq!(defaults["defaultModelId"], serde_json::Value::Null);
        assert_eq!(
            defaults["autoOpenNextChapterAfterFinalize"],
            serde_json::json!(false)
        );
        assert_eq!(defaults["editorFontSize"], serde_json::json!(16));
        assert_eq!(
            defaults["editorFontFamily"],
            serde_json::json!("Noto Serif SC")
        );
        assert_eq!(defaults["autoSaveInterval"], serde_json::json!(30));
        assert_eq!(
            defaults["proxy"],
            serde_json::json!({ "enabled": false, "type": "http", "host": "", "port": 7890 })
        );
        assert_eq!(
            defaults.as_object().unwrap().len(),
            7,
            "默认键集合必须与基线一致"
        );
    }

    #[test]
    fn get_returns_defaults_when_file_missing_test() {
        let dir = temp_dir("get-missing");
        assert_eq!(
            read_global_config_at(&config_path(&dir)),
            default_global_config()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_does_not_merge_defaults_when_file_exists_test() {
        let dir = temp_dir("get-partial");
        let path = config_path(&dir);
        std::fs::write(&path, br#"{"locale":"zh-CN"}"#).unwrap();

        // 对齐基线：文件存在时原样返回，不合并默认值
        assert_eq!(
            read_global_config_at(&path),
            serde_json::json!({ "locale": "zh-CN" })
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_falls_back_to_defaults_when_file_corrupt_test() {
        let dir = temp_dir("get-corrupt");
        let path = config_path(&dir);
        std::fs::write(&path, b"{ broken").unwrap();

        assert_eq!(read_global_config_at(&path), default_global_config());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ broken",
            "只读路径不得改写文件"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_merges_shallowly_and_keeps_unknown_keys_test() {
        let dir = temp_dir("set-merge");
        let path = config_path(&dir);
        std::fs::write(
            &path,
            br#"{"theme":"light","locale":"zh-CN","proxy":{"enabled":true,"host":"127.0.0.1","port":1,"type":"http"}}"#,
        )
        .unwrap();

        let mut updates = HashMap::new();
        updates.insert("editorFontSize".to_string(), serde_json::json!(20));
        updates.insert(
            "proxy".to_string(),
            serde_json::json!({ "enabled": false, "type": "socks5", "host": "h", "port": 2 }),
        );
        let result = set_global_config_at(&path, Some(updates));
        assert!(result.success);
        assert!(result.error.is_none());

        let stored = read(&path);
        assert_eq!(
            stored["theme"],
            serde_json::json!("light"),
            "未指定键必须保留"
        );
        assert_eq!(
            stored["locale"],
            serde_json::json!("zh-CN"),
            "未知键（locale）必须保留"
        );
        assert_eq!(stored["editorFontSize"], serde_json::json!(20));
        // 浅合并：嵌套对象整体替换，而不是深合并
        assert_eq!(
            stored["proxy"],
            serde_json::json!({ "enabled": false, "type": "socks5", "host": "h", "port": 2 })
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_without_updates_keeps_existing_content_test() {
        let dir = temp_dir("set-none");
        let path = config_path(&dir);
        std::fs::write(&path, br#"{"theme":"light"}"#).unwrap();

        let result = set_global_config_at(&path, None);
        assert!(result.success);
        assert_eq!(read(&path), serde_json::json!({ "theme": "light" }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_seeds_defaults_when_file_missing_test() {
        let dir = temp_dir("set-seed");
        let path = config_path(&dir);

        let mut updates = HashMap::new();
        updates.insert("locale".to_string(), serde_json::json!("zh-CN"));
        assert!(set_global_config_at(&path, Some(updates)).success);

        let stored = read(&path);
        // 首次写入以默认值为基底（对齐 `{ ...DEFAULT_GLOBAL_CONFIG, ...config }`）
        assert_eq!(stored["locale"], serde_json::json!("zh-CN"));
        assert_eq!(stored["theme"], serde_json::json!("dark"));
        assert_eq!(stored["editorFontSize"], serde_json::json!(16));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_refuses_to_overwrite_corrupt_file_test() {
        let dir = temp_dir("set-corrupt");
        let path = config_path(&dir);
        std::fs::write(&path, b"{ broken").unwrap();

        let mut updates = HashMap::new();
        updates.insert("theme".to_string(), serde_json::json!("light"));
        let result = set_global_config_at(&path, Some(updates));

        assert!(!result.success);
        assert_eq!(
            result.error.as_deref(),
            Some("Error: 全局配置损坏，已拒绝覆盖"),
            "对齐基线 String(new Error(...)) 的形态"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ broken",
            "拒绝覆盖必须保留原文件"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_config_key_writes_single_key_test() {
        let dir = temp_dir("set-key");
        let path = config_path(&dir);

        assert!(set_config_key_at(&path, "defaultModelId", serde_json::json!("m-1")).success);
        assert_eq!(read(&path)["defaultModelId"], serde_json::json!("m-1"));

        assert!(set_config_key_at(&path, "defaultModelId", serde_json::Value::Null).success);
        assert_eq!(read(&path)["defaultModelId"], serde_json::Value::Null);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
