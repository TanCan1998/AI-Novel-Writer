//! BlueprintRepository —— 章节蓝图（`blueprints` 表）
//!
//! 平移自 `electron/repositories/blueprint-repository.ts`（S2-a：基础读写）。
//! - `characters` 列表以 JSON 数组落库（`'[]'` 为缺省），角色改名由角色仓储同步维护；
//! - `newCharacterCandidates` / `relationshipHints` 不属于 `blueprints` 表列，
//!   只在"范围提交"的不可变回执中保留（S2-b 落地），故此处读写不落库；
//! - `updateNotes` 只改 notes 与两个时间戳；`clearAll` 在单一事务内清空三张表。
//!
//! S2-b/S2-c（范围提交、角色同步）将在此文件续写。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// 前端驼峰接口（对齐基线 `BlueprintData`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintData {
    pub chapter_number: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub key_events: String,
    #[serde(default)]
    pub characters: Vec<String>,
    /// 本章首次引入的重要常驻角色候选（非表列，随提交回执冻结）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_character_candidates: Option<Vec<serde_json::Value>>,
    /// 刚生成的蓝图携带的关系载荷（非表列，随提交回执冻结）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship_hints: Option<serde_json::Value>,
    #[serde(default)]
    pub suspense_hook: String,
    #[serde(default)]
    pub user_guidance: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub notes_updated_at: String,
}

impl BlueprintData {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let characters_json: Option<String> = row.get("characters")?;
        // 基线 `rowToData` 对损坏 JSON 容错为空数组
        let characters = characters_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
            .unwrap_or_default();
        Ok(Self {
            chapter_number: row.get("chapter_number")?,
            title: row.get::<_, Option<String>>("title")?.unwrap_or_default(),
            role: row.get::<_, Option<String>>("role")?.unwrap_or_default(),
            purpose: row.get::<_, Option<String>>("purpose")?.unwrap_or_default(),
            key_events: row.get::<_, Option<String>>("key_events")?.unwrap_or_default(),
            characters,
            // 非表列字段：读回恒为缺失（对齐基线 undefined）
            new_character_candidates: None,
            relationship_hints: None,
            suspense_hook: row
                .get::<_, Option<String>>("suspense_hook")?
                .unwrap_or_default(),
            user_guidance: row
                .get::<_, Option<String>>("user_guidance")?
                .unwrap_or_default(),
            notes: row.get::<_, Option<String>>("notes")?.unwrap_or_default(),
            notes_updated_at: row
                .get::<_, Option<String>>("notes_updated_at")?
                .unwrap_or_default(),
        })
    }
}

/// 读取全部蓝图（按章节号升序）
pub fn get_all(conn: &Connection) -> Result<Vec<BlueprintData>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT chapter_number, title, role, purpose, key_events, characters,
                    suspense_hook, user_guidance, notes, notes_updated_at
             FROM blueprints ORDER BY chapter_number ASC",
        )
        .map_err(|error| format!("读取蓝图失败：{error}"))?;
    let rows = stmt
        .query_map([], BlueprintData::from_row)
        .map_err(|error| format!("读取蓝图失败：{error}"))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|error| format!("读取蓝图失败：{error}"))?);
    }
    Ok(result)
}

/// 读取单章蓝图（未找到返回 `None`，对齐基线 `null`）
pub fn get_by_chapter(conn: &Connection, chapter_number: i64) -> Result<Option<BlueprintData>, String> {
    conn.query_row(
        "SELECT chapter_number, title, role, purpose, key_events, characters,
                suspense_hook, user_guidance, notes, notes_updated_at
         FROM blueprints WHERE chapter_number = ?1",
        [chapter_number],
        BlueprintData::from_row,
    )
    .optional()
    .map_err(|error| format!("读取蓝图失败：{error}"))
}

/// 蓝图表第 2–11 列（与基线 `upsert` 的 INSERT 列表一致；不含 `created_at`/`updated_at`）
const UPSERT_SQL: &str = "
INSERT INTO blueprints (
  chapter_number, title, role, purpose, key_events, characters,
  suspense_hook, user_guidance, notes, notes_updated_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
ON CONFLICT(chapter_number) DO UPDATE SET
  title = excluded.title,
  role = excluded.role,
  purpose = excluded.purpose,
  key_events = excluded.key_events,
  characters = excluded.characters,
  suspense_hook = excluded.suspense_hook,
  user_guidance = excluded.user_guidance,
  notes = excluded.notes,
  notes_updated_at = excluded.notes_updated_at,
  updated_at = datetime('now')
";

fn upsert_with(conn: &Connection, data: &BlueprintData) -> Result<(), String> {
    let characters = serde_json::to_string(&data.characters)
        .map_err(|error| format!("序列化角色列表失败：{error}"))?;
    conn.execute(
        UPSERT_SQL,
        rusqlite::params![
            data.chapter_number,
            data.title,
            data.role,
            data.purpose,
            data.key_events,
            characters,
            data.suspense_hook,
            data.user_guidance,
            data.notes,
            data.notes_updated_at,
        ],
    )
    .map(|_| ())
    .map_err(|error| format!("写入蓝图失败：{error}"))
}

/// 插入或更新单章蓝图
pub fn upsert(conn: &Connection, data: &BlueprintData) -> Result<(), String> {
    upsert_with(conn, data)
}

/// 批量插入/更新（单事务；任一条失败整体回滚）
pub fn upsert_many(conn: &Connection, items: &[BlueprintData]) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;
    for item in items {
        upsert_with(&tx, item)?;
    }
    tx.commit()
        .map_err(|error| format!("提交蓝图失败：{error}"))
}

/// 仅更新 notes（`notes_updated_at` 与 `updated_at` 取库时间）
/// 返回是否命中章节（对齐基线 `result.changes > 0`）。
pub fn update_notes(conn: &Connection, chapter_number: i64, notes: &str) -> Result<bool, String> {
    let changes = conn
        .execute(
            "UPDATE blueprints
             SET notes = ?1, notes_updated_at = datetime('now'), updated_at = datetime('now')
             WHERE chapter_number = ?2",
            rusqlite::params![notes, chapter_number],
        )
        .map_err(|error| format!("更新蓝图要点失败：{error}"))?;
    Ok(changes > 0)
}

/// 删除单章蓝图
pub fn delete(conn: &Connection, chapter_number: i64) -> Result<(), String> {
    conn.execute("DELETE FROM blueprints WHERE chapter_number = ?1", [chapter_number])
        .map(|_| ())
        .map_err(|error| format!("删除蓝图失败：{error}"))
}

/// 在调用方事务内清空蓝图相关事实（供 `clearAll` 与后续项目清理/导入复用）。
///
/// 顺序与基线 `clearBlueprintFactsWithinTransaction` 一致：
/// 先收敛 schema，再按"子表 → 父表"删除（避让外键）。
pub fn clear_blueprint_facts_within_transaction(conn: &Connection) -> Result<(), String> {
    crate::db::schema::ensure_blueprint_commit_schema(conn)
        .map_err(|error| format!("蓝图提交表收敛失败：{error}"))?;
    conn.execute("DELETE FROM blueprint_character_sync_operations", [])
        .map_err(|error| format!("清空蓝图角色同步操作失败：{error}"))?;
    conn.execute("DELETE FROM blueprint_commit_operations", [])
        .map_err(|error| format!("清空蓝图提交记录失败：{error}"))?;
    conn.execute("DELETE FROM blueprints", [])
        .map_err(|error| format!("清空蓝图失败：{error}"))?;
    Ok(())
}

/// 删除所有章节蓝图（单事务）
pub fn clear_all(conn: &Connection) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;
    clear_blueprint_facts_within_transaction(&tx)?;
    tx.commit().map_err(|error| format!("清空蓝图失败：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        create_tables(&conn).expect("建表失败");
        conn
    }

    fn sample(chapter: i64, notes: &str) -> BlueprintData {
        BlueprintData {
            chapter_number: chapter,
            title: format!("第 {chapter} 章"),
            role: "起".to_string(),
            purpose: "推进主线".to_string(),
            key_events: "事件".to_string(),
            characters: vec!["林清玄".to_string()],
            new_character_candidates: None,
            relationship_hints: None,
            suspense_hook: "钩子".to_string(),
            user_guidance: "指导".to_string(),
            notes: notes.to_string(),
            notes_updated_at: String::new(),
        }
    }

    #[test]
    fn upsert_get_all_and_get_by_chapter_test() {
        let conn = memory_db();
        upsert(&conn, &sample(2, "")).unwrap();
        upsert(&conn, &sample(1, "")).unwrap();

        let all = get_all(&conn).unwrap();
        assert_eq!(all.len(), 2);
        // 按章节号升序
        assert_eq!(all[0].chapter_number, 1);
        assert_eq!(all[1].chapter_number, 2);
        assert_eq!(all[0].characters, vec!["林清玄".to_string()]);
        // 非表列字段读回缺失
        assert!(all[0].new_character_candidates.is_none());
        assert!(all[0].relationship_hints.is_none());

        assert!(get_by_chapter(&conn, 3).unwrap().is_none());
        assert_eq!(get_by_chapter(&conn, 2).unwrap().unwrap().title, "第 2 章");
    }

    #[test]
    fn upsert_conflict_updates_fields_test() {
        let conn = memory_db();
        upsert(&conn, &sample(1, "旧")).unwrap();
        let mut updated = sample(1, "新");
        updated.title = "改名后的章节".to_string();
        updated.characters = vec!["苏晚".to_string()];
        upsert(&conn, &updated).unwrap();

        let row = get_by_chapter(&conn, 1).unwrap().unwrap();
        assert_eq!(row.title, "改名后的章节");
        assert_eq!(row.notes, "新");
        assert_eq!(row.characters, vec!["苏晚".to_string()]);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM blueprints", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "同章节必须覆盖而非新增");
    }

    #[test]
    fn upsert_many_commits_every_item_test() {
        let conn = memory_db();
        upsert(&conn, &sample(1, "旧")).unwrap();
        // 第 1 章为覆盖，另加第 2、3 章 → 共 3 条
        upsert_many(&conn, &[sample(1, "新"), sample(2, ""), sample(3, "")]).unwrap();
        let all = get_all(&conn).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].notes, "新", "同章节应被批量中的新值覆盖");
    }

    #[test]
    fn update_notes_hits_only_existing_chapter_test() {
        let conn = memory_db();
        upsert(&conn, &sample(1, "")).unwrap();

        assert!(update_notes(&conn, 1, "作者要点").unwrap());
        assert!(!update_notes(&conn, 99, "空章节").unwrap());

        let row = get_by_chapter(&conn, 1).unwrap().unwrap();
        assert_eq!(row.notes, "作者要点");
        assert!(!row.notes_updated_at.is_empty(), "应写入提取时间");
    }

    #[test]
    fn delete_and_clear_all_test() {
        let conn = memory_db();
        upsert(&conn, &sample(1, "")).unwrap();
        upsert(&conn, &sample(2, "")).unwrap();

        delete(&conn, 1).unwrap();
        assert_eq!(get_all(&conn).unwrap().len(), 1);

        // 预置一张提交记录，验证 clearAll 一并清空（含子表外键）
        conn.execute(
            "INSERT INTO blueprint_commit_operations
             (operation_id, payload_hash, mode, start_chapter, end_chapter, character_sync_input)
             VALUES ('op-1', 'hash', 'full', 1, 1, '[]')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO blueprint_character_sync_operations
             (operation_id, blueprint_commit_operation_id, blueprint_commit_payload_hash,
              start_chapter, end_chapter, character_sync_input)
             VALUES ('blueprint-sync-op-1', 'op-1', 'hash', 1, 1, '[]')",
            [],
        )
        .unwrap();

        clear_all(&conn).unwrap();
        assert!(get_all(&conn).unwrap().is_empty());
        let operations: i64 = conn
            .query_row("SELECT COUNT(*) FROM blueprint_commit_operations", [], |row| row.get(0))
            .unwrap();
        let syncs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM blueprint_character_sync_operations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(operations, 0);
        assert_eq!(syncs, 0);
    }

    #[test]
    fn ensure_blueprint_commit_schema_backfills_sync_operations_test() {
        let conn = memory_db();
        // 直接插入一条历史提交记录（模拟旧库），再跑收敛 → 自动补建同步操作
        conn.execute(
            "INSERT INTO blueprint_commit_operations
             (operation_id, payload_hash, mode, start_chapter, end_chapter, character_sync_input)
             VALUES ('legacy-op', 'legacy-hash', 'replace-range', 2, 3, '[]')",
            [],
        )
        .unwrap();
        crate::db::schema::ensure_blueprint_commit_schema(&conn).unwrap();
        let (operation_id, hash): (String, String) = conn
            .query_row(
                "SELECT operation_id, blueprint_commit_payload_hash
                 FROM blueprint_character_sync_operations
                 WHERE blueprint_commit_operation_id = 'legacy-op'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(operation_id, "blueprint-sync-legacy-op");
        assert_eq!(hash, "legacy-hash");
        // 幂等：重复收敛不新增
        crate::db::schema::ensure_blueprint_commit_schema(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM blueprint_character_sync_operations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
