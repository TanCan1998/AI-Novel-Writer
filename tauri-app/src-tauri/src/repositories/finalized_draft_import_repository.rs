//! finalized_draft_import —— 原稿导入权威序列与幂等提交（批次 E / G 共用）。
//!
//! 平移自 `electron/repositories/finalized-draft-import-repository.ts`。
//! 本轮（批次 E draft 收尾）落地：`authority_sequence`（`db:draft-authority-sequence`）
//! 与其指纹基座；`commit` / `preview` 随导入频道在本批次第二主题补齐。
//!
//! 指纹口径与基线一致：`authorityFingerprint = sha256(JSON(行数组))`，
//! 行数组按 `chapter ASC, version ASC, id ASC`，元素键序 = JS 对象插入序
//! （draftId, chapterNumber, version, bodyHash）。

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 对齐契约 `AuthoritativeChapterSequence`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthoritativeChapterSequence {
    pub status: String,
    pub last_chapter_number: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_chapter_number: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_gap_chapter_number: Option<i64>,
    pub duplicate_chapter_numbers: Vec<i64>,
    pub authority_fingerprint: String,
}

struct AuthorityRow {
    draft_id: i64,
    chapter_number: i64,
    version: i64,
    body: String,
}

fn authority_rows(conn: &Connection) -> Result<Vec<AuthorityRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT drafts.id, drafts.chapter_number, drafts.version, contents.body
             FROM drafts
             JOIN contents ON contents.id = drafts.content_id
             WHERE drafts.status = 'finalized'
             ORDER BY drafts.chapter_number ASC, drafts.version ASC, drafts.id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(AuthorityRow {
                draft_id: row.get(0)?,
                chapter_number: row.get(1)?,
                version: row.get(2)?,
                body: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// 手工拼串对齐基线 `JSON.stringify` 的键序（draftId, chapterNumber, version, bodyHash）
fn authority_fingerprint(rows: &[AuthorityRow]) -> String {
    let items: Vec<String> = rows
        .iter()
        .map(|row| {
            format!(
                "{{\"draftId\":{},\"chapterNumber\":{},\"version\":{},\"bodyHash\":\"{}\"}}",
                row.draft_id,
                row.chapter_number,
                row.version,
                sha256_hex(&row.body)
            )
        })
        .collect();
    sha256_hex(&format!("[{}]", items.join(",")))
}

fn sequence_from_rows(rows: Vec<AuthorityRow>) -> AuthoritativeChapterSequence {
    let fingerprint = authority_fingerprint(&rows);
    if rows.is_empty() {
        return AuthoritativeChapterSequence {
            status: "empty".to_string(),
            last_chapter_number: 0,
            next_chapter_number: Some(1),
            first_gap_chapter_number: None,
            duplicate_chapter_numbers: Vec::new(),
            authority_fingerprint: fingerprint,
        };
    }
    let mut counts: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    for row in &rows {
        *counts.entry(row.chapter_number).or_insert(0) += 1;
    }
    let duplicate_chapter_numbers: Vec<i64> = counts
        .iter()
        .filter(|(_, &count)| count > 1)
        .map(|(&chapter, _)| chapter)
        .collect();
    let max_chapter = rows.iter().map(|r| r.chapter_number).max().unwrap_or(0);
    let first_gap_chapter_number = (1..=max_chapter).find(|chapter| !counts.contains_key(chapter));
    if !duplicate_chapter_numbers.is_empty() || first_gap_chapter_number.is_some() {
        return AuthoritativeChapterSequence {
            status: "invalid".to_string(),
            last_chapter_number: max_chapter,
            next_chapter_number: None,
            first_gap_chapter_number,
            duplicate_chapter_numbers,
            authority_fingerprint: fingerprint,
        };
    }
    AuthoritativeChapterSequence {
        status: "continuous".to_string(),
        last_chapter_number: max_chapter,
        next_chapter_number: Some(max_chapter + 1),
        first_gap_chapter_number: None,
        duplicate_chapter_numbers: Vec::new(),
        authority_fingerprint: fingerprint,
    }
}

/// 基线 `FinalizedDraftImportRepository.authoritySequence`
pub fn authority_sequence(conn: &Connection) -> Result<AuthoritativeChapterSequence, String> {
    Ok(sequence_from_rows(authority_rows(conn)?))
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

    fn add_finalized(conn: &Connection, chapter: i64, version: i64, body: &str) {
        conn.execute("INSERT INTO contents (body) VALUES (?1)", [body]).unwrap();
        let content_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id) VALUES (?1, ?2, 'finalized', ?3)",
            rusqlite::params![chapter, version, content_id],
        )
        .unwrap();
    }

    #[test]
    fn empty_sequence_reports_next_one_test() {
        let conn = memory_db();
        let seq = authority_sequence(&conn).unwrap();
        assert_eq!(seq.status, "empty");
        assert_eq!(seq.next_chapter_number, Some(1));
        assert_eq!(seq.duplicate_chapter_numbers, Vec::<i64>::new());
        assert!(!seq.authority_fingerprint.is_empty());
    }

    #[test]
    fn continuous_sequence_and_fingerprint_stability_test() {
        let conn = memory_db();
        add_finalized(&conn, 1, 1, "一");
        add_finalized(&conn, 2, 1, "二");
        let seq = authority_sequence(&conn).unwrap();
        assert_eq!(seq.status, "continuous");
        assert_eq!(seq.last_chapter_number, 2);
        assert_eq!(seq.next_chapter_number, Some(3));

        // 指纹对正文变化敏感；对同章多版本仅取行集本身（ASC 序全行参与）
        let before = seq.authority_fingerprint.clone();
        add_finalized(&conn, 2, 2, "二改");
        let after = authority_sequence(&conn).unwrap();
        assert_ne!(before, after.authority_fingerprint);
    }

    #[test]
    fn invalid_sequence_reports_gap_and_duplicates_test() {
        let conn = memory_db();
        add_finalized(&conn, 1, 1, "一");
        add_finalized(&conn, 3, 1, "三");
        add_finalized(&conn, 3, 2, "三改");
        let seq = authority_sequence(&conn).unwrap();
        assert_eq!(seq.status, "invalid");
        assert_eq!(seq.first_gap_chapter_number, Some(2));
        assert_eq!(seq.duplicate_chapter_numbers, vec![3]);
        assert_eq!(seq.last_chapter_number, 3);
        assert_eq!(seq.next_chapter_number, None);
    }
}
