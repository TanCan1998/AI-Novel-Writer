//! finalization_outbox —— 定稿数据库事实源（批次 E）。
//!
//! 平移自 `electron/repositories/finalization-repository.ts` 中批次 E 所需的段落：
//! - `link_knowledge_document`：知识库文档回链（`db:finalization-link-knowledge-document`）；
//! - `list_authoritative_for_export`：每章**最高版本**定稿的不可变导出快照
//!   （`db:draft-export-snapshot`），正文/哈希/outbox 快照三重一致性校验；
//! - `matches_authoritative_export_receipt`：`db:draft-export-authority-current`，
//!   校验渲染层持有的导出回执是否仍与当前权威序列逐项一致。
//!
//! 红线：outbox 内冻结的不可变正文绝不回读 `contents.body`（发布/重试同理）。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::repositories::finalized_continuity_repository::sha256_hex;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizationRecord {
    pub finalization_id: String,
    pub draft_id: i64,
    pub chapter_number: i64,
    pub chapter_title: String,
    /// outbox 内冻结的不可变正文；发布和重试绝不回读 contents.body。
    pub content_snapshot: String,
    pub content_hash: String,
    pub content_revision: i64,
    pub target_file_name: String,
    pub knowledge_document_id: String,
    pub publication_status: String,
    pub last_error: String,
    pub published_at: Option<String>,
}

/// 对齐契约 `FinalizedDraftExportSnapshot`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedDraftExportSnapshot {
    pub draft_id: i64,
    pub chapter_number: i64,
    pub version: i64,
    pub title: String,
    pub content: String,
    pub finalization_id: Option<String>,
    pub content_hash: String,
}

/// 对齐契约 `FinalizedDraftExportAuthorityReceipt` 的元素
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedDraftExportAuthorityItem {
    pub draft_id: i64,
    pub chapter_number: i64,
    pub version: i64,
    pub finalization_id: Option<String>,
    pub content_hash: String,
}

struct ExportRow {
    draft_id: i64,
    chapter_number: i64,
    version: i64,
    body: String,
    finalization_id: Option<String>,
    outbox_chapter_number: Option<i64>,
    chapter_title: Option<String>,
    content_hash: Option<String>,
    content_snapshot: Option<String>,
}

const EXPORT_ROW_SELECT: &str = "SELECT drafts.id, drafts.chapter_number, drafts.version,
        contents.body, finalization_outbox.finalization_id,
        finalization_outbox.chapter_number,
        finalization_outbox.chapter_title, finalization_outbox.content_hash,
        finalization_outbox.content_snapshot
     FROM drafts
     JOIN contents ON contents.id = drafts.content_id
     LEFT JOIN finalization_outbox ON finalization_outbox.draft_id = drafts.id
     WHERE drafts.status = 'finalized'";

fn read_export_rows_desc(conn: &Connection) -> Result<Vec<ExportRow>, String> {
    let sql = format!("{EXPORT_ROW_SELECT} ORDER BY drafts.chapter_number ASC, drafts.version DESC, drafts.id DESC");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ExportRow {
                draft_id: row.get(0)?,
                chapter_number: row.get(1)?,
                version: row.get(2)?,
                body: row.get(3)?,
                finalization_id: row.get(4)?,
                outbox_chapter_number: row.get(5)?,
                chapter_title: row.get(6)?,
                content_hash: row.get(7)?,
                content_snapshot: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// 对齐基线 `selectAuthoritativeExportRows`：每章取第一个出现的行（最高版本）
fn select_authoritative_export_rows(
    rows: Vec<ExportRow>,
) -> Result<Vec<(FinalizedDraftExportAuthorityItem, FinalizedDraftExportSnapshot)>, String> {
    let mut selected: Vec<(FinalizedDraftExportAuthorityItem, FinalizedDraftExportSnapshot)> = Vec::new();
    let mut selected_versions: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for row in rows {
        if let Some(&selected_version) = selected_versions.get(&row.chapter_number) {
            if selected_version == row.version {
                return Err(format!(
                    "第 {} 章存在重复定稿版本，无法确定导出正文",
                    row.chapter_number
                ));
            }
            continue;
        }
        if row.draft_id < 1 || row.chapter_number < 1 || row.version < 1 {
            return Err("定稿导出快照身份无效".to_string());
        }
        if row.body.trim().is_empty() {
            return Err(format!("第 {} 章定稿正文为空", row.chapter_number));
        }
        let content_hash = sha256_hex(&row.body);
        if let Some(finalization_id) = &row.finalization_id {
            if finalization_id.trim().is_empty() {
                return Err("定稿导出快照身份无效".to_string());
            }
            if row.outbox_chapter_number != Some(row.chapter_number) {
                return Err("定稿导出快照身份不一致".to_string());
            }
            if row.content_snapshot.as_deref() != Some(row.body.as_str()) {
                return Err("定稿导出快照正文不一致".to_string());
            }
            if row.content_hash.as_deref() != Some(content_hash.as_str()) {
                return Err("定稿导出快照摘要不一致".to_string());
            }
        }
        selected_versions.insert(row.chapter_number, row.version);
        let title = row.chapter_title.unwrap_or_default().trim().to_string();
        selected.push((
            FinalizedDraftExportAuthorityItem {
                draft_id: row.draft_id,
                chapter_number: row.chapter_number,
                version: row.version,
                finalization_id: row.finalization_id.clone(),
                content_hash: content_hash.clone(),
            },
            FinalizedDraftExportSnapshot {
                draft_id: row.draft_id,
                chapter_number: row.chapter_number,
                version: row.version,
                title,
                content: row.body,
                finalization_id: row.finalization_id,
                content_hash,
            },
        ));
    }
    Ok(selected)
}

/// 基线 `FinalizationRepository.listAuthoritativeForExport`
pub fn list_authoritative_for_export(
    conn: &Connection,
) -> Result<Vec<FinalizedDraftExportSnapshot>, String> {
    Ok(read_authoritative_export_rows(conn)?.into_iter().map(|(_, snapshot)| snapshot).collect())
}

fn read_authoritative_export_rows(
    conn: &Connection,
) -> Result<Vec<(FinalizedDraftExportAuthorityItem, FinalizedDraftExportSnapshot)>, String> {
    let rows = read_export_rows_desc(conn)?;
    select_authoritative_export_rows(rows)
}

/// outbox 记录投影（列顺序与基线 `FinalizationRow` 一致）
fn map_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<FinalizationRecord> {
    Ok(FinalizationRecord {
        finalization_id: row.get(0)?,
        draft_id: row.get(1)?,
        chapter_number: row.get(2)?,
        chapter_title: row.get(3)?,
        content_snapshot: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        content_hash: row.get(5)?,
        content_revision: row.get(6)?,
        target_file_name: row.get(7)?,
        knowledge_document_id: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
        publication_status: row.get(9)?,
        last_error: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
        published_at: row.get(11)?,
    })
}

const RECORD_COLUMNS: &str = "SELECT finalization_id, draft_id, chapter_number, chapter_title,
        content_snapshot, content_hash, content_revision, target_file_name,
        knowledge_document_id, publication_status, last_error, published_at
 FROM finalization_outbox";

fn get_record_by_draft(conn: &Connection, draft_id: i64) -> Result<Option<FinalizationRecord>, String> {
    conn.query_row(
        &format!("{RECORD_COLUMNS} WHERE draft_id = ?1"),
        [draft_id],
        map_record,
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// 基线 `FinalizationRepository.get`
pub fn get(conn: &Connection, finalization_id: &str) -> Result<Option<FinalizationRecord>, String> {
    conn.query_row(
        &format!("{RECORD_COLUMNS} WHERE finalization_id = ?1"),
        [finalization_id],
        map_record,
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// 基线 `FinalizationRepository.getByDraftId`
pub fn get_by_draft_id(conn: &Connection, draft_id: i64) -> Result<Option<FinalizationRecord>, String> {
    get_record_by_draft(conn, draft_id)
}

/// 基线 `FinalizationRepository.linkKnowledgeDocument`
pub fn link_knowledge_document(
    conn: &Connection,
    draft_id: i64,
    document_id: &str,
) -> Result<FinalizationRecord, String> {
    let normalized_document_id = document_id.trim().to_string();
    if normalized_document_id.is_empty() {
        return Err("知识库文档身份不能为空".to_string());
    }
    let changed = conn
        .execute(
            "UPDATE finalization_outbox
             SET knowledge_document_id = ?1, updated_at = datetime('now')
             WHERE draft_id = ?2",
            rusqlite::params![normalized_document_id, draft_id],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err(format!("草稿缺少定稿提交：{draft_id}"));
    }
    get_record_by_draft(conn, draft_id)?.ok_or(format!("草稿缺少定稿提交：{draft_id}"))
}

/// 基线 `FinalizationRepository.matchesAuthoritativeExportReceipt`
pub fn matches_authoritative_export_receipt(
    conn: &Connection,
    receipt: &[FinalizedDraftExportAuthorityItem],
) -> bool {
    // 逐项结构校验（对齐基线宽松入参检查）
    let mut seen_chapters = std::collections::HashSet::new();
    for item in receipt {
        if item.draft_id < 1
            || item.chapter_number < 1
            || item.version < 1
            || item.finalization_id.as_deref().is_some_and(|s| s.trim().is_empty())
            || item.content_hash.len() != 64
            || !item
                .content_hash
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            || !seen_chapters.insert(item.chapter_number)
        {
            return false;
        }
    }
    let Ok(current) = read_authoritative_export_rows(conn) else {
        return false;
    };
    current.len() == receipt.len()
        && current.iter().zip(receipt).all(|((item, _), received)| {
            item.draft_id == received.draft_id
                && item.chapter_number == received.chapter_number
                && item.version == received.version
                && item.finalization_id == received.finalization_id
                && item.content_hash == received.content_hash
        })
}

// ===== G1：定稿提交事务（`finalization:commit` / `finalization:retry`） =====

/// 基线 `FinalizationCommitInput`
#[derive(Debug, Clone)]
pub struct FinalizationCommitInput {
    pub finalization_id: String,
    pub draft_id: i64,
    pub chapter_number: i64,
    pub chapter_title: String,
    pub content: String,
    pub content_hash: String,
    pub content_revision: i64,
    pub target_file_name: String,
}

/// 基线 `hasSameFinalizationInput`：同草稿重复提交必须逐字段一致才幂等
fn has_same_finalization_input(
    existing: &FinalizationRecord,
    input: &FinalizationCommitInput,
) -> bool {
    existing.draft_id == input.draft_id
        && existing.chapter_number == input.chapter_number
        && existing.chapter_title == input.chapter_title
        && existing.content_hash == input.content_hash
        && existing.content_revision == input.content_revision
        && existing.content_snapshot == input.content
}

/// 基线 `FinalizationRepository.commit`：正文、字数、定稿状态与发布 outbox 必须由
/// **同一个 SQLite transaction** 共同提交；任一 statement 失败都会回滚其余变化。
pub fn commit(
    conn: &Connection,
    input: &FinalizationCommitInput,
) -> Result<FinalizationRecord, String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

    if let Some(existing) = get_record_by_draft(&tx, input.draft_id)? {
        // renderer 可能在主进程已提交而响应丢失后重发同一冻结快照。此时新生成的
        // finalizationId/碰撞候选文件名都不能破坏幂等性，必须返回原提交。
        if has_same_finalization_input(&existing, input) {
            return Ok(existing);
        }
        return Err("该草稿已有不可替换的定稿提交".to_string());
    }

    let draft = tx
        .query_row(
            "SELECT id, chapter_number, status, content_id FROM drafts WHERE id = ?1",
            [input.draft_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((_id, draft_chapter, draft_status, content_id)) = draft else {
        return Err(format!("草稿不存在：{}", input.draft_id));
    };
    if draft_chapter != input.chapter_number {
        return Err("草稿与定稿章节不匹配".to_string());
    }
    if draft_status == "finalized" {
        return Err("草稿已定稿但缺少可恢复发布记录".to_string());
    }

    let replaces_finalized: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM drafts
             WHERE chapter_number = ?1 AND status = 'finalized' AND id <> ?2
             LIMIT 1",
            rusqlite::params![input.chapter_number, input.draft_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if replaces_finalized.is_some() {
        crate::repositories::finalized_continuity_repository::invalidate_continuity_projection_from(
            &tx,
            input.chapter_number,
        )?;
    }

    tx.execute(
        "UPDATE contents SET body = ?1 WHERE id = ?2",
        rusqlite::params![input.content, content_id],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "UPDATE drafts
         SET status = 'finalized', word_count = ?1, updated_at = datetime('now')
         WHERE id = ?2",
        rusqlite::params![
            crate::draft_units::count_draft_units(&input.content),
            input.draft_id
        ],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO finalization_outbox (
            finalization_id, draft_id, chapter_number, chapter_title,
            content_hash, content_revision, content_snapshot, target_file_name,
            publication_status, last_error
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', '')",
        rusqlite::params![
            input.finalization_id,
            input.draft_id,
            input.chapter_number,
            input.chapter_title,
            input.content_hash,
            input.content_revision,
            input.content,
            input.target_file_name,
        ],
    )
    .map_err(|e| e.to_string())?;

    tx.commit().map_err(|e| e.to_string())?;

    // 与基线一致：事务内构造返回记录（不重新回读）
    Ok(FinalizationRecord {
        finalization_id: input.finalization_id.clone(),
        draft_id: input.draft_id,
        chapter_number: input.chapter_number,
        chapter_title: input.chapter_title.clone(),
        content_snapshot: input.content.clone(),
        content_hash: input.content_hash.clone(),
        content_revision: input.content_revision,
        target_file_name: input.target_file_name.clone(),
        knowledge_document_id: String::new(),
        publication_status: "pending".to_string(),
        last_error: String::new(),
        published_at: None,
    })
}

/// 基线 `FinalizationRepository.markPublicationPending`
pub fn mark_publication_pending(
    conn: &Connection,
    finalization_id: &str,
    error: &str,
) -> Result<FinalizationRecord, String> {
    let changed = conn
        .execute(
            "UPDATE finalization_outbox
             SET publication_status = 'pending', last_error = ?1, updated_at = datetime('now')
             WHERE finalization_id = ?2",
            rusqlite::params![error, finalization_id],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err(format!("定稿提交不存在：{finalization_id}"));
    }
    get(conn, finalization_id)?.ok_or(format!("定稿提交不存在：{finalization_id}"))
}

/// 基线 `FinalizationRepository.markPublished`
pub fn mark_published(
    conn: &Connection,
    finalization_id: &str,
) -> Result<FinalizationRecord, String> {
    let changed = conn
        .execute(
            "UPDATE finalization_outbox
             SET publication_status = 'published', last_error = '',
                 published_at = datetime('now'), updated_at = datetime('now')
             WHERE finalization_id = ?1",
            [finalization_id],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err(format!("定稿提交不存在：{finalization_id}"));
    }
    get(conn, finalization_id)?.ok_or(format!("定稿提交不存在：{finalization_id}"))
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

    fn finalize_draft(conn: &Connection, chapter: i64, version: i64, body: &str) -> i64 {
        conn.execute("INSERT INTO contents (body) VALUES (?1)", [body]).unwrap();
        let content_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id) VALUES (?1, ?2, 'finalized', ?3)",
            rusqlite::params![chapter, version, content_id],
        )
        .unwrap();
        let draft_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO finalization_outbox (
                finalization_id, draft_id, chapter_number, chapter_title,
                content_hash, content_revision, content_snapshot, target_file_name
            ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)",
            rusqlite::params![
                format!("fin-{chapter}-{version}"),
                draft_id,
                chapter,
                format!("第{chapter}章"),
                sha256_hex(body),
                body,
                format!("ch{chapter}.txt"),
            ],
        )
        .unwrap();
        draft_id
    }

    /// 造一个可定稿的草稿（status='draft'），返回 (draft_id, content_id)
    fn seed_draft(conn: &Connection, chapter: i64, version: i64, body: &str) -> (i64, i64) {
        conn.execute("INSERT INTO contents (body) VALUES (?1)", [body])
            .unwrap();
        let content_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id) VALUES (?1, ?2, 'draft', ?3)",
            rusqlite::params![chapter, version, content_id],
        )
        .unwrap();
        (conn.last_insert_rowid(), content_id)
    }

    fn commit_input(draft_id: i64, finalization_id: &str, body: &str) -> FinalizationCommitInput {
        FinalizationCommitInput {
            finalization_id: finalization_id.to_string(),
            draft_id,
            chapter_number: 1,
            chapter_title: "初遇".to_string(),
            content: body.to_string(),
            content_hash: sha256_hex(body),
            content_revision: 1,
            target_file_name: "第1章 初遇.txt".to_string(),
        }
    }

    #[test]
    fn commit_freezes_facts_and_is_idempotent_test() {
        let conn = memory_db();
        let (draft_id, content_id) = seed_draft(&conn, 1, 1, "旧正文");
        let body = "# 标题\n\n正文";
        let input = commit_input(draft_id, "fin-1", body);

        let record = commit(&conn, &input).unwrap();
        assert_eq!(record.finalization_id, "fin-1");
        assert_eq!(record.publication_status, "pending");
        assert_eq!(record.knowledge_document_id, "");

        // 同一事务同时冻结三个事实：正文、定稿状态、字数
        let frozen_body: String = conn
            .query_row("SELECT body FROM contents WHERE id = ?1", [content_id], |row| row.get(0))
            .unwrap();
        assert_eq!(frozen_body, body);
        let (status, word_count): (String, i64) = conn
            .query_row(
                "SELECT status, word_count FROM drafts WHERE id = ?1",
                [draft_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "finalized");
        assert_eq!(word_count, crate::draft_units::count_draft_units(body));

        // 幂等：同输入重发返回原提交，不新增 outbox 行
        let again = commit(&conn, &input).unwrap();
        assert_eq!(again.finalization_id, "fin-1");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM finalization_outbox", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "同快照重发不得创建第二条提交");

        // 内容漂移 → 拒绝覆盖
        let mut drifted = input.clone();
        drifted.content = "改过的正文".to_string();
        drifted.content_hash = sha256_hex(&drifted.content);
        assert_eq!(
            commit(&conn, &drifted).unwrap_err(),
            "该草稿已有不可替换的定稿提交"
        );
    }

    #[test]
    fn commit_rejects_invalid_draft_states_test() {
        let conn = memory_db();
        let missing = commit_input(999, "fin-x", "a");
        assert_eq!(commit(&conn, &missing).unwrap_err(), "草稿不存在：999");

        let (draft_id, _) = seed_draft(&conn, 1, 1, "正文");
        let mut wrong_chapter = commit_input(draft_id, "fin-x", "a");
        wrong_chapter.chapter_number = 2;
        assert_eq!(commit(&conn, &wrong_chapter).unwrap_err(), "草稿与定稿章节不匹配");

        conn.execute("UPDATE drafts SET status = 'finalized' WHERE id = ?1", [draft_id])
            .unwrap();
        assert_eq!(
            commit(&conn, &commit_input(draft_id, "fin-x", "a")).unwrap_err(),
            "草稿已定稿但缺少可恢复发布记录"
        );
    }

    #[test]
    fn commit_replacing_finalized_advances_continuity_watermark_test() {
        let conn = memory_db();
        // 第 3 章已有 v1 定稿
        let (first_draft, _) = seed_draft(&conn, 3, 1, "v1 正文");
        let mut first = commit_input(first_draft, "fin-3-1", "v1 正文");
        first.chapter_number = 3;
        commit(&conn, &first).unwrap();

        let before: Option<i64> = conn
            .query_row(
                "SELECT stale_from_chapter FROM continuity_projection_meta WHERE id = 'main'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        // 同章 v2 定稿 → 触发连续性失效水位推进
        let (second_draft, _) = seed_draft(&conn, 3, 2, "v2 正文");
        let mut second = commit_input(second_draft, "fin-3-2", "v2 正文");
        second.chapter_number = 3;
        commit(&conn, &second).unwrap();

        let after: Option<i64> = conn
            .query_row(
                "SELECT stale_from_chapter FROM continuity_projection_meta WHERE id = 'main'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(before, None, "初始无失效水位");
        assert_eq!(after, Some(3), "同章重定稿应把失效水位推进到该章");
    }

    #[test]
    fn publication_marks_roundtrip_and_reject_unknown_id_test() {
        let conn = memory_db();
        let (draft_id, _) = seed_draft(&conn, 1, 1, "正文");
        commit(&conn, &commit_input(draft_id, "fin-9", "正文")).unwrap();

        let pending = mark_publication_pending(&conn, "fin-9", "实体稿目标已存在且内容不匹配").unwrap();
        assert_eq!(pending.publication_status, "pending");
        assert_eq!(pending.last_error, "实体稿目标已存在且内容不匹配");

        let published = mark_published(&conn, "fin-9").unwrap();
        assert_eq!(published.publication_status, "published");
        assert_eq!(published.last_error, "");
        assert!(published.published_at.is_some());

        assert_eq!(mark_published(&conn, "nope").unwrap_err(), "定稿提交不存在：nope");
        assert_eq!(
            mark_publication_pending(&conn, "nope", "e").unwrap_err(),
            "定稿提交不存在：nope"
        );
        assert!(get(&conn, "nope").unwrap().is_none());
        assert_eq!(
            get_by_draft_id(&conn, draft_id).unwrap().unwrap().finalization_id,
            "fin-9"
        );
    }

    #[test]
    fn link_knowledge_document_roundtrip_test() {
        let conn = memory_db();
        let draft_id = finalize_draft(&conn, 1, 1, "正文");

        // 空文档 id 拒绝
        assert_eq!(
            link_knowledge_document(&conn, draft_id, "  ").unwrap_err(),
            "知识库文档身份不能为空"
        );
        // 不存在的草稿拒绝
        assert_eq!(
            link_knowledge_document(&conn, 999, "doc-1").unwrap_err(),
            "草稿缺少定稿提交：999"
        );

        let record = link_knowledge_document(&conn, draft_id, " doc-abc ").unwrap();
        assert_eq!(record.knowledge_document_id, "doc-abc");
        assert_eq!(record.finalization_id, format!("fin-1-1"));
        assert_eq!(record.publication_status, "pending");
    }

    #[test]
    fn authoritative_export_picks_highest_version_test() {
        let conn = memory_db();
        finalize_draft(&conn, 1, 1, "v1 正文");
        finalize_draft(&conn, 1, 2, "v2 正文");
        finalize_draft(&conn, 2, 1, "第二章正文");

        let snapshots = list_authoritative_for_export(&conn).unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].version, 2, "同章应取最高版本");
        assert_eq!(snapshots[0].content, "v2 正文");
        assert_eq!(snapshots[1].chapter_number, 2);

        // 回执匹配：与权威序列一致 → true；换掉 hash → false
        let receipt: Vec<FinalizedDraftExportAuthorityItem> = snapshots
            .iter()
            .map(|s| FinalizedDraftExportAuthorityItem {
                draft_id: s.draft_id,
                chapter_number: s.chapter_number,
                version: s.version,
                finalization_id: s.finalization_id.clone(),
                content_hash: s.content_hash.clone(),
            })
            .collect();
        assert!(matches_authoritative_export_receipt(&conn, &receipt));

        let mut drifted = receipt.clone();
        drifted[0].content_hash = "0".repeat(64);
        assert!(!matches_authoritative_export_receipt(&conn, &drifted));

        // 长度不一致 → false
        assert!(!matches_authoritative_export_receipt(&conn, &receipt[..1]));
        // 重复章节 → false
        let mut dup = receipt.clone();
        dup[1].chapter_number = dup[0].chapter_number;
        assert!(!matches_authoritative_export_receipt(&conn, &dup));
    }

    #[test]
    fn export_detects_snapshot_drift_and_duplicates_test() {
        let conn = memory_db();
        // outbox 快照与正文漂移
        let draft_id = finalize_draft(&conn, 1, 1, "正文A");
        conn.execute(
            "UPDATE finalization_outbox SET content_snapshot = '漂移' WHERE draft_id = ?1",
            [draft_id],
        )
        .unwrap();
        assert_eq!(
            list_authoritative_for_export(&conn).unwrap_err(),
            "定稿导出快照正文不一致"
        );
        // 注：基线「同章同版本重复定稿」错误为防御性分支，drafts(chapter,version)
        // UNIQUE 索引下不可达，此处不构造。
    }
}
