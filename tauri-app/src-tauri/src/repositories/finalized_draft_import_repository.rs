//! finalized_draft_import —— 原稿导入权威序列与幂等提交（批次 E / G 共用）。
//!
//! 平移自 `electron/repositories/finalized-draft-import-repository.ts`。
//! 本轮（批次 E draft 收尾）落地：`authority_sequence`（`db:draft-authority-sequence`）
//! 与其指纹基座；`commit` / `preview` 随导入频道在本批次第二主题补齐。
//!
//! 指纹口径与基线一致：`authorityFingerprint = sha256(JSON(行数组))`，
//! 行数组按 `chapter ASC, version ASC, id ASC`，元素键序 = JS 对象插入序
//! （draftId, chapterNumber, version, bodyHash）。

use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::manuscript_publisher::resolve_manuscript_target;

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

#[derive(Clone)]
struct AuthorityRow {
    draft_id: i64,
    chapter_number: i64,
    version: i64,
    title: Option<String>,
    body: String,
}

fn authority_rows(conn: &Connection) -> Result<Vec<AuthorityRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT drafts.id, drafts.chapter_number, drafts.version, contents.body,
                    (SELECT chapter_title FROM finalization_outbox WHERE draft_id = drafts.id)
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
                title: row.get(4)?,
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

// ===== 导入提交（批次 E 第二主题：db:draft-import-finalized-batch） =====

use crate::draft_units::{count_draft_units, count_legacy_draft_units_v1};

/// 对齐契约 `FinalizedDraftImportChapter`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedDraftImportChapter {
    pub chapter_number: i64,
    pub title: String,
    pub content: String,
    pub word_count: i64,
}

/// 对齐契约 `FinalizedDraftImportRequest`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedDraftImportRequest {
    pub operation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_authority_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_manifest_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_commit_manifest_fingerprint: Option<String>,
    pub chapters: Vec<FinalizedDraftImportChapter>,
}

/// 对齐契约 `FinalizedDraftImportDraftReceipt`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedDraftImportDraftReceipt {
    pub chapter_number: i64,
    pub draft_id: i64,
    pub finalization_id: String,
    pub content_hash: String,
    pub target_file_name: String,
    pub status: String,
    pub publication_status: String,
}

/// 对齐契约 `FinalizedDraftImportReceipt`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedDraftImportReceipt {
    pub operation_id: String,
    pub payload_hash: String,
    pub chapter_numbers: Vec<i64>,
    pub drafts: Vec<FinalizedDraftImportDraftReceipt>,
    pub idempotent: bool,
}

fn require_non_empty_operation_id(operation_id: &str) -> Result<String, String> {
    let normalized = operation_id.trim();
    if normalized.is_empty() || normalized.chars().count() > 200 {
        return Err("定稿导入 operationId 无效".to_string());
    }
    Ok(normalized.to_string())
}

/// 对齐基线 `normalizeChapters`：校验 + 按章节号升序稳定排序
pub(crate) fn normalize_chapters(
    chapters: &[FinalizedDraftImportChapter],
) -> Result<Vec<FinalizedDraftImportChapter>, String> {
    if chapters.is_empty() {
        return Err("定稿导入至少需要一个章节".to_string());
    }
    let mut seen = std::collections::HashSet::new();
    let mut normalized = Vec::with_capacity(chapters.len());
    for chapter in chapters {
        if chapter.chapter_number < 1 {
            return Err("定稿导入章节号必须是唯一正整数".to_string());
        }
        if !seen.insert(chapter.chapter_number) {
            return Err(format!("定稿导入章节号重复：{}", chapter.chapter_number));
        }
        if chapter.content.trim().is_empty() {
            return Err(format!("第 {} 章正文不能为空", chapter.chapter_number));
        }
        if chapter.word_count != count_draft_units(&chapter.content) {
            return Err(format!("第 {} 章字数与正文不一致", chapter.chapter_number));
        }
        normalized.push(FinalizedDraftImportChapter {
            chapter_number: chapter.chapter_number,
            title: chapter.title.clone(),
            content: chapter.content.clone(),
            word_count: chapter.word_count,
        });
    }
    normalized.sort_by_key(|chapter| chapter.chapter_number);
    Ok(normalized)
}

/// 对齐基线 `requestPayloadHash`：键序手工对齐 `JSON.stringify` 插入序
fn request_payload_hash(
    request: &FinalizedDraftImportRequest,
    chapters: &[FinalizedDraftImportChapter],
) -> String {
    let chapters_json = serde_json::to_string(&chapters).unwrap_or_else(|_| "[]".to_string());
    if request.expected_authority_fingerprint.is_none()
        && request.expected_manifest_fingerprint.is_none()
        && request.expected_commit_manifest_fingerprint.is_none()
    {
        return sha256_hex(&format!(
            "{{\"operationId\":{},\"chapters\":{}}}",
            serde_json::to_string(&request.operation_id).unwrap_or_default(),
            chapters_json
        ));
    }
    if request.expected_commit_manifest_fingerprint.is_none() {
        return sha256_hex(&format!(
            "{{\"operationId\":{},\"expectedAuthorityFingerprint\":{},\"expectedManifestFingerprint\":{},\"chapters\":{}}}",
            serde_json::to_string(&request.operation_id).unwrap_or_default(),
            serde_json::to_string(&request.expected_authority_fingerprint).unwrap_or_default(),
            serde_json::to_string(&request.expected_manifest_fingerprint).unwrap_or_default(),
            chapters_json
        ));
    }
    sha256_hex(&format!(
        "{{\"operationId\":{},\"expectedAuthorityFingerprint\":{},\"expectedManifestFingerprint\":{},\"expectedCommitManifestFingerprint\":{},\"chapters\":{}}}",
        serde_json::to_string(&request.operation_id).unwrap_or_default(),
        serde_json::to_string(&request.expected_authority_fingerprint).unwrap_or_default(),
        serde_json::to_string(&request.expected_manifest_fingerprint).unwrap_or_default(),
        serde_json::to_string(&request.expected_commit_manifest_fingerprint).unwrap_or_default(),
        chapters_json
    ))
}

/// 对齐基线 `requestPayloadHashCandidates`：现行口径 + 两个旧计数器
fn request_payload_hash_candidates(
    request: &FinalizedDraftImportRequest,
    chapters: &[FinalizedDraftImportChapter],
) -> std::collections::HashSet<String> {
    let mut candidates = std::collections::HashSet::new();
    candidates.insert(request_payload_hash(request, chapters));
    for count in [
        count_legacy_draft_units_v1 as fn(&str) -> i64,
        |content: &str| content.chars().count() as i64,
    ] {
        let remapped: Vec<FinalizedDraftImportChapter> = chapters
            .iter()
            .map(|chapter| FinalizedDraftImportChapter {
                word_count: count(&chapter.content),
                ..chapter.clone()
            })
            .collect();
        candidates.insert(request_payload_hash(request, &remapped));
    }
    candidates
}

/// 对齐基线 `manifestFingerprint`：元素键序 chapterNumber, title, contentHash, wordCount
pub(crate) fn manifest_fingerprint(chapters: &[FinalizedDraftImportChapter]) -> String {
    let items: Vec<String> = chapters
        .iter()
        .map(|chapter| {
            format!(
                "{{\"chapterNumber\":{},\"title\":{},\"contentHash\":\"{}\",\"wordCount\":{}}}",
                chapter.chapter_number,
                serde_json::to_string(&chapter.title).unwrap_or_default(),
                sha256_hex(&chapter.content),
                chapter.word_count
            )
        })
        .collect();
    sha256_hex(&format!("[{}]", items.join(",")))
}

fn finalization_id(operation_id: &str, chapter_number: i64) -> String {
    format!(
        "import-{}",
        &sha256_hex(&format!("{operation_id}:{chapter_number}"))[..32]
    )
}

struct ImportOperationRow {
    operation_id: String,
    payload_hash: String,
    receipt_json: String,
}

fn get_operation_row(
    conn: &Connection,
    operation_id: &str,
) -> Result<Option<ImportOperationRow>, String> {
    conn.query_row(
        "SELECT operation_id, payload_hash, receipt_json
         FROM finalized_draft_import_operations WHERE operation_id = ?1",
        [operation_id],
        |row| {
            Ok(ImportOperationRow {
                operation_id: row.get(0)?,
                payload_hash: row.get(1)?,
                receipt_json: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// 对齐基线 `parseStoredReceipt`
fn parse_stored_receipt(row: &ImportOperationRow) -> Result<FinalizedDraftImportReceipt, String> {
    let receipt: FinalizedDraftImportReceipt = serde_json::from_str(&row.receipt_json)
        .map_err(|_| "定稿导入收据损坏，已拒绝重放".to_string())?;
    if receipt.operation_id != row.operation_id
        || receipt.payload_hash != row.payload_hash
        || receipt.chapter_numbers.len() != receipt.drafts.len()
    {
        return Err("定稿导入收据与操作事实不一致，已拒绝重放".to_string());
    }
    Ok(receipt)
}

/// 对齐基线 `verifyStoredFacts`
fn verify_stored_facts(
    conn: &Connection,
    receipt: &FinalizedDraftImportReceipt,
    chapters: &[FinalizedDraftImportChapter],
) -> Result<(), String> {
    if receipt.chapter_numbers.len() != chapters.len()
        || receipt.drafts.len() != chapters.len()
        || receipt
            .chapter_numbers
            .iter()
            .zip(chapters)
            .any(|(number, chapter)| *number != chapter.chapter_number)
    {
        return Err("定稿导入收据章节覆盖不完整，已拒绝重放".to_string());
    }
    for (draft_receipt, chapter) in receipt.drafts.iter().zip(chapters) {
        if draft_receipt.chapter_number != chapter.chapter_number
            || draft_receipt.status != "finalized"
            || draft_receipt.publication_status != "pending"
            || draft_receipt.content_hash != sha256_hex(&chapter.content)
        {
            return Err("定稿导入收据内容不一致，已拒绝重放".to_string());
        }
        let fact = conn
            .query_row(
                "SELECT drafts.chapter_number, drafts.status, drafts.word_count, contents.body,
                        finalization_outbox.content_hash, finalization_outbox.content_snapshot,
                        finalization_outbox.target_file_name, finalization_outbox.publication_status
                 FROM drafts
                 JOIN contents ON contents.id = drafts.content_id
                 JOIN finalization_outbox ON finalization_outbox.draft_id = drafts.id
                 WHERE drafts.id = ?1 AND finalization_outbox.finalization_id = ?2",
                rusqlite::params![draft_receipt.draft_id, draft_receipt.finalization_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some((
            chapter_number,
            status,
            word_count,
            body,
            content_hash,
            content_snapshot,
            target_file_name,
            publication_status,
        )) = fact
        else {
            return Err("定稿导入已提交事实缺失或漂移，已拒绝重放".to_string());
        };
        if chapter_number != chapter.chapter_number
            || status != "finalized"
            || word_count != chapter.word_count
            || body != chapter.content
            || content_snapshot != chapter.content
            || content_hash != draft_receipt.content_hash
            || target_file_name != draft_receipt.target_file_name
            || (publication_status != "pending" && publication_status != "published")
        {
            return Err("定稿导入已提交事实缺失或漂移，已拒绝重放".to_string());
        }
    }
    Ok(())
}

// ===== 实体稿目标 =====
// 目标文件名解析已统一到 `crate::manuscript_publisher`（G1 定稿发布同源），
// 避免两份 `resolveManuscriptTarget` 漂移。

/// 对齐契约 `AuthorManuscriptImportPreview`（commit 内部消费，非 IPC 频道）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorManuscriptImportPreview {
    pub classification: String,
    pub authority_fingerprint: String,
    pub manifest_fingerprint: String,
    pub chapter_count: usize,
    pub target_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_chapter_number: Option<i64>,
    pub chapters: Vec<serde_json::Value>,
    pub new_chapter_numbers: Vec<i64>,
    pub duplicate_chapter_numbers: Vec<i64>,
    pub conflict_chapter_numbers: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_gap_chapter_number: Option<i64>,
    pub authority_invalid: bool,
}

/// 基线 `FinalizedDraftImportRepository.preview`
pub fn preview(
    conn: &Connection,
    input: &[FinalizedDraftImportChapter],
) -> Result<AuthorManuscriptImportPreview, String> {
    let chapters = normalize_chapters(input)?;
    let rows = authority_rows(conn)?;
    let sequence = sequence_from_rows(rows.clone());
    let mut existing_by_chapter: std::collections::HashMap<i64, &AuthorityRow> =
        std::collections::HashMap::new();
    for row in &rows {
        existing_by_chapter.insert(row.chapter_number, row);
    }
    let mut conflict_chapter_numbers = Vec::new();
    let mut duplicate_chapter_numbers = Vec::new();
    let mut new_chapter_numbers = Vec::new();
    let mut preview_chapters = Vec::new();
    for chapter in &chapters {
        let entry = serde_json::json!({
            "number": chapter.chapter_number,
            "title": chapter.title,
            "wordCount": chapter.word_count,
        });
        match existing_by_chapter.get(&chapter.chapter_number) {
            None => {
                new_chapter_numbers.push(chapter.chapter_number);
                let mut entry = entry.clone();
                entry["disposition"] = serde_json::json!("new");
                preview_chapters.push(entry);
            }
            Some(existing) => {
                if sha256_hex(&existing.body) == sha256_hex(&chapter.content)
                    && (existing.title.is_none()
                        || existing.title.as_deref() == Some(chapter.title.as_str()))
                {
                    duplicate_chapter_numbers.push(chapter.chapter_number);
                    let mut entry = entry.clone();
                    entry["disposition"] = serde_json::json!("duplicate");
                    preview_chapters.push(entry);
                } else {
                    conflict_chapter_numbers.push(chapter.chapter_number);
                    let mut entry = entry.clone();
                    entry["disposition"] = serde_json::json!("conflict");
                    preview_chapters.push(entry);
                }
            }
        }
    }

    let expected_next = sequence.next_chapter_number;
    let mut candidate_gap: Option<i64> = None;
    if let Some(expected_next) = expected_next {
        if !new_chapter_numbers.is_empty() {
            let mut ordered = new_chapter_numbers.clone();
            ordered.sort_unstable();
            let mut expected = expected_next;
            for chapter_number in ordered {
                if chapter_number != expected {
                    candidate_gap = Some(expected);
                    break;
                }
                expected += 1;
            }
        }
    }
    let authority_invalid = sequence.status == "invalid";
    let classification =
        if authority_invalid || !conflict_chapter_numbers.is_empty() || candidate_gap.is_some() {
            "conflict"
        } else if new_chapter_numbers.is_empty() {
            "exact-duplicate"
        } else {
            "ready"
        };
    let next_chapter_number = if classification == "conflict" {
        None
    } else {
        Some(sequence.last_chapter_number + new_chapter_numbers.len() as i64 + 1)
    };
    let first_gap = sequence.first_gap_chapter_number.or(candidate_gap);
    Ok(AuthorManuscriptImportPreview {
        classification: classification.to_string(),
        authority_fingerprint: sequence.authority_fingerprint,
        manifest_fingerprint: manifest_fingerprint(&chapters),
        chapter_count: chapters.len(),
        target_status: "finalized".to_string(),
        next_chapter_number,
        chapters: preview_chapters,
        new_chapter_numbers,
        duplicate_chapter_numbers,
        conflict_chapter_numbers,
        first_gap_chapter_number: first_gap,
        authority_invalid,
    })
}

/// 基线 `FinalizedDraftImportRepository.commit`：不可分割的幂等导入提交
pub fn commit(
    conn: &Connection,
    project_root: &str,
    request: &FinalizedDraftImportRequest,
) -> Result<FinalizedDraftImportReceipt, String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let receipt = commit_in_transaction(&tx, project_root, request)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(receipt)
}

/// 基线 `FinalizedDraftImportRepository.getCommittedOperation`：
/// 已提交操作的权威回读（含章节事实核对）；用于 effect receipt
/// 的已提交权威校验（批次 G3a）。
pub fn get_committed_operation(
    conn: &Connection,
    operation_id: &str,
    chapters: &[FinalizedDraftImportChapter],
) -> Result<Option<FinalizedDraftImportReceipt>, String> {
    let normalized_operation_id = require_non_empty_operation_id(operation_id)?;
    let chapters = normalize_chapters(chapters)?;
    let Some(row) = get_operation_row(conn, &normalized_operation_id)? else {
        return Ok(None);
    };
    let receipt = parse_stored_receipt(&row)?;
    verify_stored_facts(conn, &receipt, &chapters)?;
    Ok(Some(receipt))
}

/// 事务体内的提交逻辑（批次 G3a 抽取）：供 [`commit`] 与
/// `import_run_repository::commit_effect_receipt` 的跨仓原子事务复用。
/// 错误返回时 `tx` drop 即回滚，与基线 `db.transaction()` 语义一致。
pub(crate) fn commit_in_transaction(
    tx: &Transaction<'_>,
    project_root: &str,
    request: &FinalizedDraftImportRequest,
) -> Result<FinalizedDraftImportReceipt, String> {
    let operation_id = require_non_empty_operation_id(&request.operation_id)?;
    let chapters = normalize_chapters(&request.chapters)?;
    let mut normalized_request = request.clone();
    normalized_request.operation_id = operation_id.clone();
    let payload_hash = request_payload_hash(&normalized_request, &chapters);
    let accepted_payload_hashes = request_payload_hash_candidates(&normalized_request, &chapters);

    if let Some(existing) = get_operation_row(tx, &operation_id)? {
        if !accepted_payload_hashes.contains(&existing.payload_hash) {
            return Err("定稿导入 operationId 已绑定不同载荷".to_string());
        }
        let receipt = parse_stored_receipt(&existing)?;
        verify_stored_facts(tx, &receipt, &chapters)?;
        return Ok(FinalizedDraftImportReceipt {
            idempotent: true,
            ..receipt
        });
    }

    let expected_commit_manifest_fingerprint = request
        .expected_commit_manifest_fingerprint
        .clone()
        .or_else(|| request.expected_manifest_fingerprint.clone());
    if let Some(expected) = &expected_commit_manifest_fingerprint {
        if *expected != manifest_fingerprint(&chapters) {
            return Err("原稿清单与已确认预览不一致，预览已过期".to_string());
        }
    }
    if let Some(expected_authority) = &request.expected_authority_fingerprint {
        let current_authority = sequence_from_rows(authority_rows(tx)?).authority_fingerprint;
        if *expected_authority != current_authority {
            return Err("项目权威章节已变化，原稿预览已过期".to_string());
        }
        let preview = preview(tx, &chapters)?;
        if preview.classification != "ready" {
            return Err("原稿章节不能形成连续且无冲突的权威正文".to_string());
        }
    }

    let mut drafts: Vec<FinalizedDraftImportDraftReceipt> = Vec::new();
    for chapter in &chapters {
        let max_version: Option<i64> = tx
            .query_row(
                "SELECT MAX(version) FROM drafts WHERE chapter_number = ?1",
                [chapter.chapter_number],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO contents (body) VALUES (?1)",
            [&chapter.content],
        )
        .map_err(|e| e.to_string())?;
        let content_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO drafts (chapter_number, version, status, source, content_id, word_count)
             VALUES (?1, ?2, 'finalized', 'write', ?3, ?4)",
            rusqlite::params![
                chapter.chapter_number,
                max_version.unwrap_or(0) + 1,
                content_id,
                chapter.word_count,
            ],
        )
        .map_err(|e| e.to_string())?;
        let draft_id = tx.last_insert_rowid();
        let frozen_content_hash = sha256_hex(&chapter.content);
        let frozen_finalization_id = finalization_id(&operation_id, chapter.chapter_number);
        let file_name = resolve_manuscript_target(
            project_root,
            chapter.chapter_number,
            &chapter.title,
            &frozen_finalization_id,
        )?
        .file_name;
        tx.execute(
            "INSERT INTO finalization_outbox (
                finalization_id, draft_id, chapter_number, chapter_title,
                content_hash, content_revision, content_snapshot, target_file_name,
                publication_status, last_error
            ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, 'pending', '')",
            rusqlite::params![
                frozen_finalization_id,
                draft_id,
                chapter.chapter_number,
                chapter.title,
                frozen_content_hash,
                chapter.content,
                file_name,
            ],
        )
        .map_err(|e| e.to_string())?;
        drafts.push(FinalizedDraftImportDraftReceipt {
            chapter_number: chapter.chapter_number,
            draft_id,
            finalization_id: frozen_finalization_id,
            content_hash: frozen_content_hash,
            target_file_name: file_name,
            status: "finalized".to_string(),
            publication_status: "pending".to_string(),
        });
    }

    let receipt = FinalizedDraftImportReceipt {
        operation_id: operation_id.clone(),
        payload_hash,
        chapter_numbers: chapters.iter().map(|c| c.chapter_number).collect(),
        drafts,
        idempotent: false,
    };
    tx.execute(
        "INSERT INTO finalized_draft_import_operations (operation_id, payload_hash, receipt_json)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![
            operation_id,
            receipt.payload_hash,
            serde_json::to_string(&receipt).map_err(|e| e.to_string())?,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(receipt)
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
        conn.execute("INSERT INTO contents (body) VALUES (?1)", [body])
            .unwrap();
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

    // ===== 导入提交（批次 E 第二主题） =====

    use crate::draft_units::count_draft_units;

    fn import_chapter(number: i64, title: &str, content: &str) -> FinalizedDraftImportChapter {
        FinalizedDraftImportChapter {
            chapter_number: number,
            title: title.to_string(),
            content: content.to_string(),
            word_count: count_draft_units(content),
        }
    }

    fn import_request(
        operation: &str,
        chapters: Vec<FinalizedDraftImportChapter>,
    ) -> FinalizedDraftImportRequest {
        FinalizedDraftImportRequest {
            operation_id: operation.to_string(),
            expected_authority_fingerprint: None,
            expected_manifest_fingerprint: None,
            expected_commit_manifest_fingerprint: None,
            chapters,
        }
    }

    #[test]
    fn import_commit_creates_finalized_facts_and_replays_idempotently_test() {
        let conn = memory_db();
        let root = std::env::temp_dir().join(format!(
            "anw-import-{}",
            crate::project_access::random_uuid_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let root_text = root.to_string_lossy().to_string();

        let request = import_request(
            "op-1",
            vec![
                import_chapter(1, "第一章", "第一章正文内容"),
                import_chapter(2, "第二章", "第二章正文内容"),
            ],
        );
        let receipt = commit(&conn, &root_text, &request).unwrap();
        assert!(!receipt.idempotent);
        assert_eq!(receipt.chapter_numbers, vec![1, 2]);
        assert_eq!(receipt.drafts.len(), 2);
        assert!(receipt.drafts[0].finalization_id.starts_with("import-"));
        assert!(receipt.drafts[0].target_file_name.starts_with("第1章"));

        // 数据库事实：finalized 草稿 + outbox 快照 + 幂等日志
        let finalized_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM drafts WHERE status = 'finalized'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(finalized_count, 2);
        let outbox_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM finalization_outbox", [], |r| r.get(0))
            .unwrap();
        assert_eq!(outbox_count, 2);

        // 同 operationId 重放 → 幂等返回原收据，不新建行
        let replay = commit(&conn, &root_text, &request).unwrap();
        assert!(replay.idempotent);
        assert_eq!(replay.drafts.len(), 2);
        let finalized_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM drafts WHERE status = 'finalized'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(finalized_count, 2, "重放不得新建草稿");

        // 同 operationId 不同载荷 → 拒绝
        let mut different = request.clone();
        different.chapters = vec![import_chapter(9, "第九章", "完全不同的正文")];
        assert_eq!(
            commit(&conn, &root_text, &different).unwrap_err(),
            "定稿导入 operationId 已绑定不同载荷"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_validates_chapters_and_word_count_test() {
        let conn = memory_db();
        let root = std::env::temp_dir().to_string_lossy().to_string();

        // 空章节列表
        assert_eq!(
            commit(&conn, &root, &import_request("op-x", vec![])).unwrap_err(),
            "定稿导入至少需要一个章节"
        );
        // 字数不一致
        let mut bad = import_chapter(1, "第一章", "正文");
        bad.word_count = 999;
        assert_eq!(
            commit(&conn, &root, &import_request("op-x", vec![bad])).unwrap_err(),
            "第 1 章字数与正文不一致"
        );
        // 重复章节号
        assert_eq!(
            commit(
                &conn,
                &root,
                &import_request(
                    "op-x",
                    vec![
                        import_chapter(1, "一", "正文甲"),
                        import_chapter(1, "一", "正文乙")
                    ]
                )
            )
            .unwrap_err(),
            "定稿导入章节号重复：1"
        );
        // 非法 operationId
        assert_eq!(
            commit(
                &conn,
                &root,
                &import_request("  ", vec![import_chapter(1, "一", "正文")])
            )
            .unwrap_err(),
            "定稿导入 operationId 无效"
        );
    }

    #[test]
    fn import_with_stale_authority_fingerprint_rejected_test() {
        let conn = memory_db();
        let root = std::env::temp_dir().to_string_lossy().to_string();
        let current = authority_sequence(&conn).unwrap().authority_fingerprint;
        let mut request =
            import_request("op-auth", vec![import_chapter(1, "第一章", "第一章正文")]);
        request.expected_authority_fingerprint = Some("deadbeef".to_string());
        assert_eq!(
            commit(&conn, &root, &request).unwrap_err(),
            "项目权威章节已变化，原稿预览已过期"
        );
        // 正确指纹 + ready 预览 → 提交成功
        request.expected_authority_fingerprint = Some(current);
        let receipt = commit(&conn, &root, &request).unwrap();
        assert_eq!(receipt.chapter_numbers, vec![1]);
        let _ = std::fs::remove_dir_all(std::path::Path::new(&root).join("第1章 第一章.txt"));
    }
}
