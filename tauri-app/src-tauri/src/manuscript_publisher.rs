//! 实体稿发布投影 —— 平移自 `electron/services/manuscript-publisher.ts`（批次 G1）。
//!
//! 纪律（ADR 0003/0011 定稿不可逆）：
//! - 目标文件名在 SQLite outbox 中**冻结**，重试绝不重新接受渲染进程提供的路径或标题；
//! - 所有读写都被约束在**受信项目根目录的直接子文件**，越界即拒绝；
//! - 临时文件写入后在同一目录原位 rename；若重试时发现同内容目标已存在，视为上次
//!   rename 成功但状态回写中断，保持幂等；不同内容则永不覆盖。

use std::io::Write;
use std::path::{Component, Path};

/// 受信项目根内的实体稿目标
#[derive(Debug, Clone)]
pub struct ManuscriptTarget {
    pub file_name: String,
    pub absolute_path: std::path::PathBuf,
}

/// Windows 非法文件名字符（对齐基线 `WINDOWS_ILLEGAL_FILE_NAME_CHARS`）
const WINDOWS_ILLEGAL_FILE_NAME_CHARS: &str = r#"[<>:"/\\|?*]"#;
/// Windows 保留设备名（对齐基线 `WINDOWS_RESERVED_FILE_NAME`）
const WINDOWS_RESERVED_FILE_NAME: &str = r"(?i)^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\..*)?$";

fn replace_windows_control_characters(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if (character as u32) <= 0x1f {
                '_'
            } else {
                character
            }
        })
        .collect()
}

/// 基线 `sanitizeWindowsFileNamePart`
pub fn sanitize_windows_file_name_part(value: &str) -> String {
    let illegal = regex::Regex::new(WINDOWS_ILLEGAL_FILE_NAME_CHARS).unwrap();
    let normalized = replace_windows_control_characters(&illegal.replace_all(value, "_"));
    let normalized = normalized.trim().trim_end_matches(['.', ' ']).to_string();
    if normalized.is_empty() {
        return String::new();
    }
    let reserved = regex::Regex::new(WINDOWS_RESERVED_FILE_NAME).unwrap();
    if reserved.is_match(&normalized) {
        format!("{normalized}_")
    } else {
        normalized
    }
}

/// 基线 `manuscriptFileName`
fn manuscript_file_name(chapter_number: i64, chapter_title: &str) -> String {
    let title = sanitize_windows_file_name_part(chapter_title);
    if title.is_empty() {
        format!("第{chapter_number}章.txt")
    } else {
        format!("第{chapter_number}章 {title}.txt")
    }
}

/// 基线 `containedDirectChild`：目标必须是项目根的**直接子文件**
fn contained_direct_child(project_root: &str, file_name: &str) -> Result<ManuscriptTarget, String> {
    let mut components = Path::new(file_name).components();
    let single_normal =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if !single_normal {
        return Err("实体稿目标越出受信项目 manuscript 边界".to_string());
    }
    let root = Path::new(project_root);
    let absolute_path = root.join(file_name);
    if absolute_path.parent() != Some(root) {
        return Err("实体稿目标越出受信项目 manuscript 边界".to_string());
    }
    Ok(ManuscriptTarget {
        file_name: file_name.to_string(),
        absolute_path,
    })
}

/// 基线 `resolveManuscriptTarget`：生成仅属于受信项目根目录的实体稿路径
pub fn resolve_manuscript_target(
    project_root: &str,
    chapter_number: i64,
    chapter_title: &str,
    finalization_id: &str,
) -> Result<ManuscriptTarget, String> {
    if chapter_number < 1 {
        return Err("章节号无效，无法生成实体稿目标".to_string());
    }
    let preferred = manuscript_file_name(chapter_number, chapter_title);
    let first = contained_direct_child(project_root, &preferred)?;
    if !first.absolute_path.exists() {
        return Ok(first);
    }

    let path = Path::new(&preferred);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_string();
    let extension = path
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy()))
        .unwrap_or_default();
    let marker: String = sanitize_windows_file_name_part(finalization_id)
        .chars()
        .take(12)
        .collect();
    let marker = if marker.is_empty() {
        "finalized".to_string()
    } else {
        marker
    };

    for index in 1..1000 {
        let suffix = if index == 1 {
            format!(" ({marker})")
        } else {
            format!(" ({marker}-{index})")
        };
        let candidate =
            contained_direct_child(project_root, &format!("{stem}{suffix}{extension}"))?;
        if !candidate.absolute_path.exists() {
            return Ok(candidate);
        }
    }
    Err("实体稿文件名碰撞过多，拒绝覆盖现有文件".to_string())
}

/// 基线 `resolveStoredManuscriptTarget`：只接受 outbox 中冻结的裸文件名
fn resolve_stored_manuscript_target(
    project_root: &str,
    target_file_name: &str,
) -> Result<ManuscriptTarget, String> {
    if target_file_name.is_empty()
        || Path::new(target_file_name)
            .file_name()
            .and_then(|value| value.to_str())
            != Some(target_file_name)
    {
        return Err("已提交的实体稿目标无效".to_string());
    }
    contained_direct_child(project_root, target_file_name)
}

/// 基线 `serializeManuscript`：章节头 + 剥离首个 Markdown 标题行
pub fn serialize_manuscript(chapter_number: i64, chapter_title: &str, content: &str) -> String {
    let title = sanitize_windows_file_name_part(chapter_title);
    let header = if title.is_empty() {
        format!("第{chapter_number}章\n\n")
    } else {
        format!("第{chapter_number}章 {title}\n\n")
    };
    let heading = regex::Regex::new(r"^#+ .*\r?\n*").unwrap();
    format!("{header}{}", heading.replace(content, ""))
}

fn write_new_file(path: &Path, body: &str) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(body.as_bytes())
        .map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())
}

/// 基线 `publishManuscript`
pub fn publish_manuscript(
    project_root: &str,
    target_file_name: &str,
    chapter_number: i64,
    chapter_title: &str,
    content: &str,
) -> Result<(), String> {
    let target = resolve_stored_manuscript_target(project_root, target_file_name)?;
    let serialized = serialize_manuscript(chapter_number, chapter_title, content);
    if target.absolute_path.exists() {
        let current =
            std::fs::read_to_string(&target.absolute_path).map_err(|error| error.to_string())?;
        if current == serialized {
            return Ok(());
        }
        return Err(format!(
            "实体稿目标已存在且内容不匹配：{}",
            target.file_name
        ));
    }

    let temporary_name = format!(
        ".{}.{}.tmp",
        target.file_name,
        crate::project_access::random_uuid_v4()
    );
    let temporary = contained_direct_child(project_root, &temporary_name)?;
    let outcome = write_new_file(&temporary.absolute_path, &serialized).and_then(|()| {
        std::fs::rename(&temporary.absolute_path, &target.absolute_path)
            .map_err(|error| error.to_string())
    });
    if temporary.absolute_path.exists() {
        let _ = std::fs::remove_file(&temporary.absolute_path);
    }
    outcome
}

/// 基线 `removePublishedManuscript`：缺失文件视为投影已清理，幂等成功
pub fn remove_published_manuscript(
    project_root: &str,
    target_file_name: &str,
) -> Result<(), String> {
    let target = resolve_stored_manuscript_target(project_root, target_file_name)?;
    if !target.absolute_path.exists() {
        return Ok(());
    }
    std::fs::remove_file(&target.absolute_path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "lorekeeper-manuscript-{name}-{}",
            crate::project_access::random_uuid_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn sanitize_matches_baseline_rules_test() {
        assert_eq!(
            sanitize_windows_file_name_part("我的/小说:01"),
            "我的_小说_01"
        );
        assert_eq!(sanitize_windows_file_name_part("  标题.  "), "标题");
        assert_eq!(sanitize_windows_file_name_part(""), "");
        assert_eq!(sanitize_windows_file_name_part("con"), "con_");
        assert_eq!(sanitize_windows_file_name_part("COM1.txt"), "COM1.txt_");
    }

    #[test]
    fn serialize_strips_leading_markdown_heading_test() {
        assert_eq!(
            serialize_manuscript(3, "初遇", "# 初遇\n\n正文开始"),
            "第3章 初遇\n\n正文开始"
        );
        // 无标题行时原样
        assert_eq!(serialize_manuscript(4, "", "正文"), "第4章\n\n正文");
    }

    #[test]
    fn resolve_avoids_collision_with_frozen_marker_test() {
        let root = temp_root("collision");
        let root_text = root.to_string_lossy().to_string();
        let file = root.join("第1章 起点.txt");
        std::fs::write(&file, "已存在").unwrap();

        let target = resolve_manuscript_target(&root_text, 1, "起点", "abcdef123456789").unwrap();
        assert_eq!(target.file_name, "第1章 起点 (abcdef123456).txt");
        assert!(target.absolute_path.parent() == Some(root.as_path()));

        // 越界文件名拒绝
        assert!(contained_direct_child(&root_text, "../escape.txt").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn publish_is_idempotent_and_rejects_content_mismatch_test() {
        let root = temp_root("publish");
        let root_text = root.to_string_lossy().to_string();

        publish_manuscript(&root_text, "第1章.txt", 1, "", "正文A").unwrap();
        let written = std::fs::read_to_string(root.join("第1章.txt")).unwrap();
        assert_eq!(written, "第1章\n\n正文A");
        // 同内容重复发布 → 幂等成功
        publish_manuscript(&root_text, "第1章.txt", 1, "", "正文A").unwrap();
        // 不同内容 → 拒绝覆盖
        assert_eq!(
            publish_manuscript(&root_text, "第1章.txt", 1, "", "正文B").unwrap_err(),
            "实体稿目标已存在且内容不匹配：第1章.txt"
        );

        // 清理幂等：删除后再次删除仍成功
        remove_published_manuscript(&root_text, "第1章.txt").unwrap();
        assert!(!root.join("第1章.txt").exists());
        remove_published_manuscript(&root_text, "第1章.txt").unwrap();

        // 冻结目标必须为裸文件名
        assert_eq!(
            resolve_stored_manuscript_target(&root_text, "../x.txt").unwrap_err(),
            "已提交的实体稿目标无效"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
