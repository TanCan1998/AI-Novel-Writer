//! ProjectClearRepository —— 清空项目生成数据
//!
//! 平移自 `electron/repositories/project-clear-repository.ts`：
//! - 三个可选范围：`generatedText`（正文档/审稿/后处理/快照）、`blueprints`、
//!   `creativeFields`（角色卡与创作四大件）；
//! - 物理文件（根目录 `第N章*.txt`）在事务**之前**移入 `.vela/trash/clear-<ts>/`，
//!   事务失败则回滚移动；成功后才删除回收目录；
//! - `cleared` 的追加顺序与基线一致：generatedText → blueprints → creativeFields。

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::schema::migrate_character_roster_schema;
use crate::repositories::blueprint_repository::clear_blueprint_facts_within_transaction;

/// 清理范围（对齐 `ProjectClearScope`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectClearScope {
    CreativeFields,
    Blueprints,
    GeneratedText,
}

/// 清理选项（三个开关均可缺省）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectClearOptions {
    #[serde(default)]
    pub creative_fields: Option<bool>,
    #[serde(default)]
    pub blueprints: Option<bool>,
    #[serde(default)]
    pub generated_text: Option<bool>,
}

impl ProjectClearOptions {
    fn wants_creative_fields(&self) -> bool {
        self.creative_fields.unwrap_or(false)
    }

    fn wants_blueprints(&self) -> bool {
        self.blueprints.unwrap_or(false)
    }

    fn wants_generated_text(&self) -> bool {
        self.generated_text.unwrap_or(false)
    }
}

/// 清理结果（对齐 `ProjectClearResult`）
///
/// 与命令层信封同形，故直接可序列化（字段名 camelCase）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectClearResult {
    pub cleared: Vec<ProjectClearScope>,
    pub physical_files_deleted: usize,
}

struct MovedFile {
    from: PathBuf,
    to: PathBuf,
}

/// 是否匹配基线 `^第\d+章(?: .*)?\.txt$`
fn is_finalized_chapter_file(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('第') else {
        return false;
    };
    let Some(digits_end) = rest.find(|character: char| !character.is_ascii_digit()) else {
        return false;
    };
    if digits_end == 0 {
        return false;
    }
    let Some(suffix) = rest[digits_end..].strip_prefix('章') else {
        return false;
    };
    if suffix == ".txt" {
        return true;
    }
    match suffix.strip_prefix(' ') {
        // ` .*\.txt`：允许 `.*` 为空（即 " .txt"）
        Some(tail) => tail.ends_with(".txt"),
        None => false,
    }
}

fn list_generated_chapter_files(project_path: &Path) -> Vec<PathBuf> {
    if !project_path.exists() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(project_path) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if is_finalized_chapter_file(name) {
            files.push(entry.path());
        }
    }
    // 基线依赖 readdir 顺序；此处排序以消除平台差异（结果集相同）
    files.sort();
    files
}

fn restore_moved_files(moved: &[MovedFile]) {
    for item in moved.iter().rev() {
        if item.to.exists() && !item.from.exists() {
            let _ = std::fs::rename(&item.to, &item.from);
        }
    }
}

fn remove_moved_files(moved: &[MovedFile]) {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for item in moved {
        if let Some(parent) = item.to.parent() {
            if !dirs.iter().any(|dir| dir == parent) {
                dirs.push(parent.to_path_buf());
            }
        }
    }
    for dir in dirs {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 把根目录的成稿实体稿移入回收目录（失败即回滚已移动的文件）
fn move_generated_files_to_trash(project_path: &Path) -> Result<Vec<MovedFile>, String> {
    let files = list_generated_chapter_files(project_path);
    if files.is_empty() {
        return Ok(Vec::new());
    }

    let stamp = crate::commands::project::iso8601_utc_from_millis(
        crate::commands::project::epoch_millis_now(),
    )
    .replace([':', '.'], "-");
    let trash_dir = project_path
        .join(".vela")
        .join("trash")
        .join(format!("clear-{stamp}"));
    std::fs::create_dir_all(&trash_dir)
        .map_err(|error| format!("创建回收目录失败：{error}"))?;

    let mut moved: Vec<MovedFile> = Vec::new();
    for file in files {
        let Some(file_name) = file.file_name() else {
            continue;
        };
        let target = trash_dir.join(file_name);
        if let Err(error) = std::fs::rename(&file, &target) {
            restore_moved_files(&moved);
            return Err(format!("移动生成章节文件失败：{error}"));
        }
        moved.push(MovedFile { from: file, to: target });
    }
    Ok(moved)
}

/// 清空选定范围的生成数据；`generatedText` 会一并清理根目录成稿实体稿
pub fn clear_generated_data(
    conn: &Connection,
    project_path: Option<&Path>,
    options: &ProjectClearOptions,
) -> Result<ProjectClearResult, String> {
    if options.wants_generated_text() && project_path.is_none() {
        return Err("项目路径未初始化".to_string());
    }

    let moved = match (options.wants_generated_text(), project_path) {
        (true, Some(path)) => move_generated_files_to_trash(path)?,
        _ => Vec::new(),
    };

    let outcome: Result<ProjectClearResult, String> = (|| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|error| format!("开启事务失败：{error}"))?;
        let mut cleared: Vec<ProjectClearScope> = Vec::new();

        if options.wants_generated_text() {
            for statement in [
                "DELETE FROM finalized_draft_import_operations",
                "DELETE FROM post_process_steps",
                "DELETE FROM post_process_runs",
                "DELETE FROM reviews",
                "DELETE FROM revisions",
                "DELETE FROM drafts",
                "DELETE FROM contents",
                "DELETE FROM summary_snapshots",
            ] {
                tx.execute(statement, [])
                    .map_err(|error| format!("清空生成正文失败：{error}"))?;
            }
            cleared.push(ProjectClearScope::GeneratedText);
        }

        if options.wants_blueprints() {
            clear_blueprint_facts_within_transaction(&tx)?;
            cleared.push(ProjectClearScope::Blueprints);
        }

        if options.wants_creative_fields() {
            // 即使项目创建于 roster 元数据迁移之前，也必须先按唯一迁移规则建档，
            // 再在同一事务内清空角色事实与 receipt，使下次 read 重新分类为空项目
            migrate_character_roster_schema(&tx).map_err(|error| error.to_string())?;
            for statement in [
                "DELETE FROM character_roster_operations",
                "DELETE FROM character_roster_meta",
                "DELETE FROM characters",
            ] {
                tx.execute(statement, [])
                    .map_err(|error| format!("清空创作字段失败：{error}"))?;
            }
            tx.execute(
                "UPDATE project_core
                 SET writing_style = '', reference_works = '', global_guidance = '',
                     golden_finger = '', premise = '', worldbuilding = '',
                     characters_arch = '', synopsis = '', character_states = '',
                     updated_at = datetime('now')
                 WHERE id = 'main'",
                [],
            )
            .map_err(|error| format!("清空创作字段失败：{error}"))?;
            cleared.push(ProjectClearScope::CreativeFields);
        }

        tx.commit()
            .map_err(|error| format!("提交清空失败：{error}"))?;

        Ok(ProjectClearResult {
            cleared,
            physical_files_deleted: moved.len(),
        })
    })();

    match outcome {
        Ok(result) => {
            remove_moved_files(&moved);
            Ok(result)
        }
        Err(error) => {
            restore_moved_files(&moved);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;
    use crate::repositories::character_repository as characters;
    use crate::repositories::draft_repository as drafts;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        create_tables(&conn).expect("建表失败");
        conn
    }

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "anw-clear-{name}-{}",
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed_draft(conn: &Connection) -> i64 {
        drafts::create(
            conn,
            &drafts::DraftCreateParams {
                chapter_number: 1,
                version: None,
                source: "write".to_string(),
                content: "正文".to_string(),
                word_count: 2,
                source_dependencies: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn chapter_file_pattern_matches_baseline_test() {
        for accepted in ["第1章.txt", "第12章 标题.txt", "第3章 .txt"] {
            assert!(is_finalized_chapter_file(accepted), "应接受 {accepted}");
        }
        for rejected in [
            "第章.txt",
            "第1章标题.txt",
            "前言第1章.txt",
            "第1章.md",
            "第1章.txt.bak",
        ] {
            assert!(!is_finalized_chapter_file(rejected), "应拒绝 {rejected}");
        }
    }

    #[test]
    fn generated_text_clear_removes_drafts_and_moves_physical_files_test() {
        let conn = memory_db();
        let dir = temp_project("generated-text");
        let draft_id = seed_draft(&conn);

        let chapter = dir.join("第1章 起始.txt");
        std::fs::write(&chapter, "定稿正文").unwrap();
        // 非成稿文件必须保留
        let other = dir.join("notes.txt");
        std::fs::write(&other, "笔记").unwrap();

        let result = clear_generated_data(
            &conn,
            Some(&dir),
            &ProjectClearOptions {
                generated_text: Some(true),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(result.cleared, vec![ProjectClearScope::GeneratedText]);
        assert_eq!(result.physical_files_deleted, 1);
        assert!(!chapter.exists(), "成稿实体稿应被移出根目录");
        assert!(other.exists(), "非成稿文件不得被移动");
        // 回收子目录在成功后删除（`.vela/trash` 父目录保留，与基线一致）
        let trash_root = dir.join(".vela").join("trash");
        let leftover = if trash_root.exists() {
            std::fs::read_dir(&trash_root).unwrap().count()
        } else {
            0
        };
        assert_eq!(leftover, 0, "回收子目录应在清空成功后删除");
        assert!(drafts::get_meta(&conn, draft_id).unwrap().is_none());

        // 未开启 generatedText 时不触碰文件
        let kept = dir.join("第2章 后续.txt");
        std::fs::write(&kept, "定稿").unwrap();
        clear_generated_data(&conn, Some(&dir), &ProjectClearOptions::default()).unwrap();
        assert!(kept.exists());
        assert!(drafts::list_all(&conn).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_requires_project_path_for_generated_text_test() {
        let conn = memory_db();
        assert_eq!(
            clear_generated_data(
                &conn,
                None,
                &ProjectClearOptions {
                    generated_text: Some(true),
                    ..Default::default()
                }
            )
            .unwrap_err(),
            "项目路径未初始化"
        );
    }

    #[test]
    fn blueprints_and_creative_fields_clear_test() {
        let conn = memory_db();
        // project_core 主台账 + 角色卡 + 蓝图 + 名单元数据
        crate::repositories::project_core_repository::init(
            &conn,
            "项目",
            crate::repositories::project_core_repository::DEFAULT_WRITING_LANGUAGE,
        )
        .unwrap();
        conn.execute(
            "UPDATE project_core SET synopsis = '旧大纲', characters_arch = '旧图谱' WHERE id = 'main'",
            [],
        )
        .unwrap();
        characters::upsert(
            &conn,
            &characters::CharacterData {
                name: "林清玄".to_string(),
                role: crate::character_role::CharacterRole::Protagonist,
                gender: String::new(),
                age: String::new(),
                appearance: String::new(),
                personality: String::new(),
                background: String::new(),
                abilities: String::new(),
                motivation: String::new(),
                relationships: String::new(),
                arc: String::new(),
                notes: String::new(),
                current_state: None,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO blueprint_commit_operations
             (operation_id, payload_hash, mode, start_chapter, end_chapter, character_sync_input)
             VALUES ('op-1', 'hash', 'full', 1, 1, '[]')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO blueprints (chapter_number, title) VALUES (1, '第 1 章')",
            [],
        )
        .unwrap();

        let result = clear_generated_data(
            &conn,
            None,
            &ProjectClearOptions {
                blueprints: Some(true),
                creative_fields: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        // 顺序对齐基线：blueprints 先于 creativeFields
        assert_eq!(
            result.cleared,
            vec![ProjectClearScope::Blueprints, ProjectClearScope::CreativeFields]
        );
        assert_eq!(result.physical_files_deleted, 0);

        let (synopsis, characters_arch): (String, String) = conn
            .query_row(
                "SELECT synopsis, characters_arch FROM project_core WHERE id = 'main'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(synopsis, "");
        assert_eq!(characters_arch, "");
        assert!(characters::get_all(&conn).unwrap().is_empty());
        let blueprints: i64 = conn
            .query_row("SELECT COUNT(*) FROM blueprints", [], |row| row.get(0))
            .unwrap();
        let operations: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM blueprint_commit_operations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(blueprints, 0);
        assert_eq!(operations, 0);
        // 名单元数据与 receipt 被清空（表中无 main 行，待下次读取重新建档）
        let meta_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM character_roster_meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(meta_rows, 0);
        let snapshot = crate::repositories::character_roster_repository::read(&conn).unwrap();
        assert_eq!(
            snapshot.migration_state,
            crate::repositories::character_roster_repository::CharacterRosterMigrationState::Empty,
            "清空创作字段后必须重新分类为空项目"
        );
        assert!(snapshot.entries.is_empty());
    }

    #[test]
    fn serialization_shape_matches_contract_test() {
        let value = serde_json::to_value(ProjectClearResult {
            cleared: vec![
                ProjectClearScope::CreativeFields,
                ProjectClearScope::GeneratedText,
            ],
            physical_files_deleted: 2,
        })
        .unwrap();
        assert_eq!(
            value["cleared"],
            serde_json::json!(["creativeFields", "generatedText"])
        );
        assert_eq!(value["physicalFilesDeleted"], serde_json::json!(2));

        // 选项解析：camelCase 且可缺省
        let options: ProjectClearOptions =
            serde_json::from_value(serde_json::json!({ "generatedText": true })).unwrap();
        assert!(options.wants_generated_text());
        assert!(!options.wants_blueprints());
        assert!(!options.wants_creative_fields());
    }
}
