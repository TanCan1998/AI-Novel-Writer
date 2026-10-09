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

fn get_record_by_draft(conn: &Connection, draft_id: i64) -> Result<Option<FinalizationRecord>, String> {
    conn.query_row(
        "SELECT finalization_id, draft_id, chapter_number, chapter_title,
                content_snapshot, content_hash, content_revision, target_file_name,
                knowledge_document_id, publication_status, last_error, published_at
         FROM finalization_outbox WHERE draft_id = ?1",
        [draft_id],
        |row| {
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
        },
    )
    .optional()
    .map_err(|e| e.to_string())
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
