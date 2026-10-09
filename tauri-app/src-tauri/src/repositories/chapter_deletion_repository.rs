//! 章节删除操作状态机（chapter lifecycle）—— 批次 E 第二部分。
//!
//! 平移自 `electron/repositories/chapter-deletion-repository.ts`（ADR 0011）。
//! 领域约束：
//! - 已定稿章节删除**不可逆但可恢复**：先冻结删除收据（`chapter_deletion_operations`），
//!   再在**同一事务**内删除 SQLite 事实（drafts / contents / post_process_runs）并推进
//!   连续性失效水位；实体稿文件与知识库文档是**异步投影清理**，由状态机断点恢复；
//! - legacy 知识投影人工确认是**一次性授权**（`required` → `consumed`）；
//! - 错误文案逐字对齐基线。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};


/// 对齐 `src/shared/chapter-deletion.ts`
pub type ProjectionStatus = &'static str; // pending | completed | failed | not_required

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterDeletionOperation {
    pub operation_id: String,
    pub draft_id: i64,
    pub chapter_number: i64,
    pub chapter_title: String,
    pub finalization_id: String,
    pub target_file_name: String,
    pub knowledge_document_id: String,
    pub post_process_run_ids: Vec<String>,
    pub manuscript_status: String,
    pub manuscript_error: String,
    pub knowledge_status: String,
    pub knowledge_error: String,
    pub legacy_knowledge_authorization: String,
    pub legacy_knowledge_authorized_at: String,
    pub status: String,
    pub attempt_count: i64,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: String,
}

/// 基线 `DeleteFinalizedChapterRequest`
#[derive(Debug, Deserialize)]
pub struct DeleteFinalizedChapterRequest {
    pub draft_id: i64,
    pub chapter_number: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedChapterTarget {
    pub id: i64,
    pub chapter_number: i64,
    pub status: String,
    pub content_id: i64,
    pub finalization_id: String,
    pub chapter_title: Option<String>,
    pub target_file_name: Option<String>,
    pub knowledge_document_id: Option<String>,
}

fn parse_run_ids(value: &str) -> Option<Vec<String>> {
    serde_json::from_str::<Vec<String>>(value).ok()
}

fn require_finalized_target(
    conn: &Connection,
    draft_id: i64,
    chapter_number: i64,
) -> Result<FinalizedChapterTarget, String> {
    let target = conn
        .query_row(
            "SELECT drafts.id, drafts.chapter_number, drafts.status, drafts.content_id,
                    finalization_outbox.finalization_id,
                    finalization_outbox.chapter_title,
                    finalization_outbox.target_file_name,
                    finalization_outbox.knowledge_document_id
             FROM drafts
             LEFT JOIN finalization_outbox ON finalization_outbox.draft_id = drafts.id
             WHERE drafts.id = ?1",
            [draft_id],
            |row| {
                Ok(FinalizedChapterTarget {
                    id: row.get(0)?,
                    chapter_number: row.get(1)?,
                    status: row.get(2)?,
                    content_id: row.get(3)?,
                    finalization_id: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    chapter_title: row.get(5)?,
                    target_file_name: row.get(6)?,
                    knowledge_document_id: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(target) = target else {
        return Err("草稿不存在".to_string());
    };
    if target.chapter_number != chapter_number {
        return Err("草稿与待删除章节身份不匹配".to_string());
    }
    if target.status != "finalized" || target.finalization_id.is_empty() {
        return Err("只有具备定稿提交收据的章节才能通过章节生命周期入口删除".to_string());
    }
    Ok(target)
}

fn list_post_process_run_ids(conn: &Connection, chapter_number: i64) -> Vec<String> {
    conn.prepare(
        "SELECT id FROM post_process_runs
         WHERE trigger_source_type = 'chapter_finalize' AND trigger_source_id = ?
         ORDER BY id",
    )
    .and_then(|mut stmt| {
        stmt.query_map([chapter_number.to_string()], |row| row.get::<_, String>(0))
            .map(|rows| rows.collect::<Result<Vec<_>, _>>())
    })
    .map(|rows| rows.unwrap_or_default())
    .unwrap_or_default()
}

/// 基线 `deleteChapterFacts`：推进连续性失效水位后删除草稿域事实。
fn delete_chapter_facts(
    conn: &Connection,
    target: &FinalizedChapterTarget,
    post_process_run_ids: &[String],
) -> Result<(), String> {
    crate::repositories::finalized_continuity_repository::invalidate_continuity_projection_from(
        conn,
        target.chapter_number,
    )?;
    let mut content_ids = vec![target.content_id];
    for sql in [
        "SELECT content_id FROM revisions WHERE base_draft_id = ?1",
        "SELECT content_id FROM reviews WHERE base_draft_id = ?1",
    ] {
        let rows: Vec<i64> = conn
            .prepare(sql)
            .and_then(|mut stmt| {
                stmt.query_map([target.id], |row| row.get(0))?
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap_or_default();
        content_ids.extend(rows);
    }

    for run_id in post_process_run_ids {
        conn.execute("DELETE FROM post_process_runs WHERE id = ?1", [run_id])
            .map_err(|e| e.to_string())?;
    }
    conn.execute("DELETE FROM drafts WHERE id = ?1", [target.id])
        .map_err(|e| e.to_string())?;
    for content_id in content_ids {
        // 与基线一致：被其他仍活跃事实共享的 content 行不是删除目标，
        // 外键会保护它，此处静默忽略。
        let _ = conn.execute("DELETE FROM contents WHERE id = ?1", [content_id]);
    }
    Ok(())
}

fn row_to_operation(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChapterDeletionOperation> {
    let raw_run_ids: String = row.get("post_process_run_ids")?;
    Ok(ChapterDeletionOperation {
        operation_id: row.get("operation_id")?,
        draft_id: row.get("draft_id")?,
        chapter_number: row.get("chapter_number")?,
        chapter_title: row.get("chapter_title")?,
        finalization_id: row.get("finalization_id")?,
        target_file_name: row.get("target_file_name")?,
        knowledge_document_id: row.get("knowledge_document_id")?,
        post_process_run_ids: parse_run_ids(&raw_run_ids).unwrap_or_default(),
        manuscript_status: row.get("manuscript_status")?,
        manuscript_error: row.get("manuscript_error")?,
        knowledge_status: row.get("knowledge_status")?,
        knowledge_error: row.get("knowledge_error")?,
        legacy_knowledge_authorization: row.get("legacy_knowledge_authorization")?,
        legacy_knowledge_authorized_at: row.get("legacy_knowledge_authorized_at")?,
        status: row.get("status")?,
        attempt_count: row.get("attempt_count")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        completed_at: row.get("completed_at")?,
    })
}

fn get_row<'a>(
    conn: &'a Connection,
    operation_id: &str,
) -> rusqlite::Result<Option<ChapterDeletionOperation>> {
    conn.query_row(
        "SELECT * FROM chapter_deletion_operations WHERE operation_id = ?1",
        [operation_id],
        row_to_operation,
    )
    .optional()
}

/// 基线 `refreshAggregateStatus`：由双通道投影状态派生聚合状态
fn refresh_aggregate_status(conn: &Connection, operation_id: &str) -> Result<(), String> {
    let finished = |status: &str| status == "completed" || status == "not_required";
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT manuscript_status, knowledge_status FROM chapter_deletion_operations
             WHERE operation_id = ?1",
            [operation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((manuscript_status, knowledge_status)) = row else {
        return Err(format!("章节删除操作不存在：{operation_id}"));
    };
    let status = if finished(&manuscript_status) && finished(&knowledge_status) {
        "completed"
    } else if manuscript_status == "failed" || knowledge_status == "failed" {
        "failed"
    } else {
        "pending"
    };
    conn.execute(
        "UPDATE chapter_deletion_operations
         SET status = ?1,
             completed_at = CASE WHEN ?1 = 'completed' THEN datetime('now') ELSE '' END,
             updated_at = datetime('now')
         WHERE operation_id = ?2",
        rusqlite::params![status, operation_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 基线 `ChapterDeletionRepository.begin`：冻结删除收据；无 legacy 授权需求时
/// 在同一事务内直接删除 SQLite 事实（不可逆点）。
pub fn begin(
    conn: &Connection,
    operation_id: &str,
    request: &DeleteFinalizedChapterRequest,
    legacy_knowledge_authorization_required: bool,
) -> Result<ChapterDeletionOperation, String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let outcome = (|| -> Result<String, String> {
        let existing: Option<(i64, String)> = tx
            .query_row(
                "SELECT chapter_number, operation_id FROM chapter_deletion_operations
                 WHERE draft_id = ?1",
                [request.draft_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((existing_chapter, existing_operation_id)) = existing {
            if existing_chapter != request.chapter_number {
                return Err("章节删除请求与已冻结操作不匹配".to_string());
            }
            return Ok(existing_operation_id);
        }

        let target = require_finalized_target(&tx, request.draft_id, request.chapter_number)?;
        let post_process_run_ids = list_post_process_run_ids(&tx, request.chapter_number);

        tx.execute(
            "INSERT INTO chapter_deletion_operations (
               operation_id, draft_id, chapter_number, chapter_title, finalization_id,
               target_file_name, knowledge_document_id, post_process_run_ids,
               manuscript_status, knowledge_status, legacy_knowledge_authorization, status
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                operation_id,
                target.id,
                target.chapter_number,
                target.chapter_title.clone().unwrap_or_default(),
                target.finalization_id,
                target.target_file_name.clone().unwrap_or_default(),
                target.knowledge_document_id.clone().unwrap_or_default(),
                serde_json::to_string(&post_process_run_ids).unwrap_or_else(|_| "[]".into()),
                if target.target_file_name.is_some() { "pending" } else { "not_required" },
                if legacy_knowledge_authorization_required
                    || !target.knowledge_document_id.clone().unwrap_or_default().is_empty()
                {
                    "pending"
                } else {
                    "not_required"
                },
                if legacy_knowledge_authorization_required { "required" } else { "not_required" },
                if legacy_knowledge_authorization_required { "authorization_required" } else { "pending" },
            ],
        )
        .map_err(|e| e.to_string())?;

        if !legacy_knowledge_authorization_required {
            delete_chapter_facts(&tx, &target, &post_process_run_ids)?;
        }
        Ok(operation_id.to_string())
    })();
    let operation_id = outcome?;
    tx.commit().map_err(|e| e.to_string())?;
    get(conn, &operation_id)?.ok_or_else(|| format!("章节删除操作不存在：{operation_id}"))
}

/// 基线 `confirmLegacyKnowledgeAbsent`：一次性人工授权 + 身份/后处理快照比对，
/// 然后补做事实删除。
pub fn confirm_legacy_knowledge_absent(
    conn: &Connection,
    operation_id: &str,
) -> Result<ChapterDeletionOperation, String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let outcome = (|| -> Result<String, String> {
        let row: Option<ChapterDeletionOperation> = get_row(&tx, operation_id)
            .map_err(|e| e.to_string())?;
        let Some(row) = row else {
            return Err(format!("章节删除操作不存在：{operation_id}"));
        };
        if row.status != "authorization_required"
            || row.legacy_knowledge_authorization != "required"
        {
            return Err("legacy 知识投影人工确认是一次性授权，当前删除收据不允许再次确认".to_string());
        }

        let target = require_finalized_target(&tx, row.draft_id, row.chapter_number)?;
        if target.finalization_id != row.finalization_id
            || target.chapter_title.clone().unwrap_or_default() != row.chapter_title
            || target.target_file_name.clone().unwrap_or_default() != row.target_file_name
            || !target.knowledge_document_id.clone().unwrap_or_default().is_empty()
        {
            return Err("章节事实或定稿收据在人工确认前已变化，已拒绝继续删除".to_string());
        }

        let current_run_ids = list_post_process_run_ids(&tx, row.chapter_number);
        if current_run_ids != row.post_process_run_ids {
            return Err("章节后处理记录在人工确认前已变化，已拒绝继续删除".to_string());
        }

        let changed = tx
            .execute(
                "UPDATE chapter_deletion_operations
                 SET legacy_knowledge_authorization = 'consumed',
                     legacy_knowledge_authorized_at = datetime('now'),
                     knowledge_status = 'not_required', knowledge_error = '',
                     status = 'pending', updated_at = datetime('now')
                 WHERE operation_id = ?1
                   AND status = 'authorization_required'
                   AND legacy_knowledge_authorization = 'required'",
                [operation_id],
            )
            .map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("legacy 知识投影人工确认是一次性授权，当前删除收据不允许再次确认".to_string());
        }
        delete_chapter_facts(&tx, &target, &row.post_process_run_ids)?;
        Ok(operation_id.to_string())
    })();
    let confirmed_operation_id = outcome?;
    tx.commit().map_err(|e| e.to_string())?;
    get(conn, &confirmed_operation_id)?.ok_or_else(|| format!("章节删除操作不存在：{confirmed_operation_id}"))
}

/// 基线 `get`
pub fn get(conn: &Connection, operation_id: &str) -> Result<Option<ChapterDeletionOperation>, String> {
    get_row(conn, operation_id).map_err(|e| e.to_string())
}

/// 基线 `getByDraftId`
pub fn get_by_draft_id(
    conn: &Connection,
    draft_id: i64,
) -> Result<Option<ChapterDeletionOperation>, String> {
    conn.query_row(
        "SELECT * FROM chapter_deletion_operations WHERE draft_id = ?1",
        [draft_id],
        row_to_operation,
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// 基线 `listIncomplete`
pub fn list_incomplete(conn: &Connection) -> Result<Vec<ChapterDeletionOperation>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT * FROM chapter_deletion_operations
             WHERE status != 'completed'
             ORDER BY created_at, operation_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_operation)
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

/// 基线 `startAttempt`
pub fn start_attempt(conn: &Connection, operation_id: &str) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE chapter_deletion_operations
             SET attempt_count = attempt_count + 1, status = 'pending', updated_at = datetime('now')
             WHERE operation_id = ?1 AND status != 'authorization_required'",
            [operation_id],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err(format!("章节删除操作不存在：{operation_id}"));
    }
    Ok(())
}

/// 基线 `markProjection`
pub fn mark_projection(
    conn: &Connection,
    operation_id: &str,
    projection: &str,
    status: ProjectionStatus,
    error: &str,
) -> Result<(), String> {
    let (status_column, error_column) = if projection == "manuscript" {
        ("manuscript_status", "manuscript_error")
    } else {
        ("knowledge_status", "knowledge_error")
    };
    let changed = conn
        .execute(
            &format!(
                "UPDATE chapter_deletion_operations
                 SET {status_column} = ?1, {error_column} = ?2, updated_at = datetime('now')
                 WHERE operation_id = ?3"
            ),
            rusqlite::params![status, error, operation_id],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err(format!("章节删除操作不存在：{operation_id}"));
    }
    refresh_aggregate_status(conn, operation_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        conn
    }

    /// 建一个已定稿草稿 + outbox 收据（无知识文档），返回 draft_id
    fn finalize_draft(conn: &Connection, chapter: i64) -> i64 {
        conn.execute("INSERT INTO contents (body) VALUES ('正文')", []).unwrap();
        let content_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id) VALUES (?1, 1, 'finalized', ?2)",
            rusqlite::params![chapter, content_id],
        )
        .unwrap();
        let draft_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO finalization_outbox (
                finalization_id, draft_id, chapter_number, chapter_title, content_hash,
                content_revision, target_file_name
            ) VALUES (?1, ?2, ?3, '标题', 'hash', 1, ?4)",
            rusqlite::params![format!("f-{chapter}"), draft_id, chapter, "章节"],
        )
        .unwrap();
        draft_id
    }

    #[test]
    fn begin_deletes_facts_without_legacy_authorization() {
        let conn = memory_db();
        let draft_id = finalize_draft(&conn, 3);
        let operation = begin(
            &conn,
            "op-1",
            &DeleteFinalizedChapterRequest { draft_id: draft_id as i64, chapter_number: 3 },
            false,
        )
        .unwrap();
        assert_eq!(operation.status, "pending");
        assert_eq!(operation.manuscript_status, "pending");
        assert_eq!(operation.knowledge_status, "not_required");
        let drafts_left: i64 =
            conn.query_row("SELECT COUNT(*) FROM drafts", [], |row| row.get(0)).unwrap();
        assert_eq!(drafts_left, 0, "无 legacy 授权要求时章节事实应随冻结同事务删除");
    }

    /// 同一 draft 重复冻结幂等返回同一收据；章号不匹配拒绝
    #[test]
    fn begin_is_idempotent_and_rejects_identity_mismatch() {
        let conn = memory_db();
        let draft_id = finalize_draft(&conn, 5);
        let first = begin(
            &conn,
            "op-1",
            &DeleteFinalizedChapterRequest { draft_id: draft_id as i64, chapter_number: 5 },
            false,
        )
        .unwrap();
        let second = begin(
            &conn,
            "op-2",
            &DeleteFinalizedChapterRequest { draft_id: draft_id as i64, chapter_number: 5 },
            false,
        )
        .unwrap();
        assert_eq!(first.operation_id, second.operation_id, "同 draft 重复冻结应返回已冻结收据");
        let mismatch = begin(
            &conn,
            "op-3",
            &DeleteFinalizedChapterRequest { draft_id: draft_id as i64, chapter_number: 4 },
            false,
        );
        assert!(mismatch.is_err(), "章号与已冻结操作不匹配必须拒绝");
    }

    /// authorization_required 时冻结事实；人工确认一次性消费
    #[test]
    fn confirm_legacy_knowledge_absent_is_one_shot() {
        let conn = memory_db();
        let draft_id = finalize_draft(&conn, 7);
        let frozen = begin(
            &conn,
            "op-9",
            &DeleteFinalizedChapterRequest { draft_id: draft_id as i64, chapter_number: 7 },
            true,
        )
        .unwrap();
        assert_eq!(frozen.status, "authorization_required");
        let drafts_left: i64 =
            conn.query_row("SELECT COUNT(*) FROM drafts", [], |row| row.get(0)).unwrap();
        assert_eq!(drafts_left, 1, "人工授权确认前不得删除章节事实");

        let confirmed = confirm_legacy_knowledge_absent(&conn, "op-9").unwrap();
        assert_eq!(confirmed.legacy_knowledge_authorization, "consumed");
        let drafts_left: i64 =
            conn.query_row("SELECT COUNT(*) FROM drafts", [], |row| row.get(0)).unwrap();
        assert_eq!(drafts_left, 0, "确认后补做章节事实删除");
        assert!(
            confirm_legacy_knowledge_absent(&conn, "op-9").is_err(),
            "legacy 知识投影人工确认是一次性授权，不允许再次确认"
        );
    }
}