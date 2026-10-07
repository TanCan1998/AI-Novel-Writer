//! 草稿源守卫 —— 平移自 `electron/repositories/draft-source-guard.ts`
//!
//! 修稿/审稿在创建前必须证明「生成时的源稿」仍是当前草稿事实：
//! 冻结的 `ExpectedDraftSource`（章号/版本/状态/正文）必须与库中完全一致，
//! 否则拒绝写入（前端据 `errorCode: SOURCE_DRAFT_CHANGED` 提示重新生成）。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// 源稿变化错误码（对齐 `SOURCE_DRAFT_CHANGED`）
pub const SOURCE_DRAFT_CHANGED: &str = "SOURCE_DRAFT_CHANGED";

/// 守卫失败时的固定文案（命令层据此回填 `errorCode`）
pub const SOURCE_DRAFT_CHANGED_MESSAGE: &str = SOURCE_DRAFT_CHANGED;

/// 生成时冻结的源稿身份（对齐 `ExpectedDraftSource`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpectedDraftSource {
    pub id: i64,
    pub chapter_number: i64,
    pub version: i64,
    pub status: String,
    pub content: String,
}

/// 断言源稿仍是当前事实；任一字段不一致即失败
pub fn assert_expected_draft_source(
    conn: &Connection,
    base_draft_id: i64,
    expected: &ExpectedDraftSource,
) -> Result<(), String> {
    let source: Option<(i64, i64, i64, String, String)> = conn
        .query_row(
            "SELECT drafts.id, drafts.chapter_number, drafts.version, drafts.status, contents.body
             FROM drafts
             JOIN contents ON contents.id = drafts.content_id
             WHERE drafts.id = ?1",
            [base_draft_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("读取源稿失败：{error}"))?;

    let matches = source
        .map(|(id, chapter_number, version, status, content)| {
            id == expected.id
                && chapter_number == expected.chapter_number
                && version == expected.version
                && status == expected.status
                && content == expected.content
        })
        .unwrap_or(false);

    if base_draft_id != expected.id || !matches {
        return Err(SOURCE_DRAFT_CHANGED_MESSAGE.to_string());
    }
    Ok(())
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

    fn seed_draft(conn: &Connection) -> i64 {
        conn.execute("INSERT INTO contents (body) VALUES ('正文')", [])
            .unwrap();
        let content_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, source, content_id, word_count, source_dependencies)
             VALUES (1, 2, 'write', ?1, 2, '[]')",
            [content_id],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn expected(id: i64) -> ExpectedDraftSource {
        ExpectedDraftSource {
            id,
            chapter_number: 1,
            version: 2,
            status: "draft".to_string(),
            content: "正文".to_string(),
        }
    }

    #[test]
    fn matching_source_passes_and_drift_is_rejected_test() {
        let conn = memory_db();
        let id = seed_draft(&conn);
        assert!(assert_expected_draft_source(&conn, id, &expected(id)).is_ok());

        // 正文漂移
        let mut drifted = expected(id);
        drifted.content = "被改写".to_string();
        assert_eq!(
            assert_expected_draft_source(&conn, id, &drifted).unwrap_err(),
            SOURCE_DRAFT_CHANGED_MESSAGE
        );

        // 状态漂移
        let mut status_changed = expected(id);
        status_changed.status = "finalized".to_string();
        assert!(assert_expected_draft_source(&conn, id, &status_changed).is_err());

        // 身份不一致 / 源稿不存在
        let mut wrong_id = expected(id);
        wrong_id.id = id + 1;
        assert!(assert_expected_draft_source(&conn, id, &wrong_id).is_err());
        assert!(assert_expected_draft_source(&conn, 9999, &expected(9999)).is_err());
    }
}
