//! ContentRepository —— 文本内容池（`contents` 表）
//!
//! 平移自 `electron/repositories/content-repository.ts`：所有长文本（草稿正文、修稿内容、
//! 审稿报告）统一存池，元数据表仅通过 `content_id` 外键引用。

use rusqlite::{Connection, OptionalExtension};

/// 创建一条内容记录，返回自增 ID
pub fn create(conn: &Connection, body: &str) -> Result<i64, String> {
    conn.execute("INSERT INTO contents (body) VALUES (?1)", [body])
        .map_err(|error| format!("写入内容失败：{error}"))?;
    Ok(conn.last_insert_rowid())
}

/// 按 ID 读取正文（不存在返回 `None`）
pub fn get_body(conn: &Connection, id: i64) -> Result<Option<String>, String> {
    conn.query_row("SELECT body FROM contents WHERE id = ?1", [id], |row| {
        row.get(0)
    })
    .optional()
    .map_err(|error| format!("读取内容失败：{error}"))
}

/// 更新正文内容
pub fn update_body(conn: &Connection, id: i64, body: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE contents SET body = ?1 WHERE id = ?2",
        rusqlite::params![body, id],
    )
    .map(|_| ())
    .map_err(|error| format!("更新内容失败：{error}"))
}

/// 删除内容（仅在确认无外键引用时调用）
pub fn delete(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM contents WHERE id = ?1", [id])
        .map(|_| ())
        .map_err(|error| format!("删除内容失败：{error}"))
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

    #[test]
    fn create_read_update_delete_roundtrip_test() {
        let conn = memory_db();
        let id = create(&conn, "第一章正文").unwrap();
        assert!(id > 0);
        assert_eq!(get_body(&conn, id).unwrap().as_deref(), Some("第一章正文"));

        update_body(&conn, id, "改后的正文").unwrap();
        assert_eq!(get_body(&conn, id).unwrap().as_deref(), Some("改后的正文"));

        assert!(get_body(&conn, 9999).unwrap().is_none());

        delete(&conn, id).unwrap();
        assert!(get_body(&conn, id).unwrap().is_none());
    }

    #[test]
    fn delete_is_blocked_by_draft_foreign_key_test() {
        let conn = memory_db();
        let content_id = create(&conn, "正文").unwrap();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, source, content_id, word_count, source_dependencies)
             VALUES (1, 1, 'write', ?1, 2, '[]')",
            [content_id],
        )
        .unwrap();

        // `ON DELETE RESTRICT`：仍被草稿引用时删除必须失败（基线保留该兼容策略）
        assert!(delete(&conn, content_id).is_err());
    }
}
