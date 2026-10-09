//! import_run —— 导入运行台账（批次 G2）。
//!
//! 平移自 `electron/repositories/import-run-repository.ts`（2552 行）。
//!
//! # 本批（G2a）范围：**读面**
//!
//! | 基线 | 本模块 |
//! |---|---|
//! | `rowToSnapshot` / `persistedProgress` / `sourceProgress` / `unfinishedSourceDisplay` | [`row_to_snapshot`] / [`persisted_progress`] / [`source_progress`] / [`unfinished_source_display`] |
//! | `completedBatches` / `chapterRowsForRange` / `checkpointChapterNumbers` | [`completed_batches`] / [`chapter_rows_for_range`] / [`checkpoint_chapter_numbers`] |
//! | `chapterRowToSnapshot` / `assertFrozenChapterSnapshot` | [`chapter_row_to_snapshot`] / [`assert_frozen_chapter_snapshot`] |
//! | `get` / `listResumable` / `listChapterBatch` | [`get`] / [`list_resumable`] / [`list_chapter_batch`] |
//!
//! **G2b 追加**（本模块）：`begin_parsing` / `commit_parsed_source` / `fail_parsed_source` /
//! `finalize_parsing` / `prepare` 与其辅助（`normalize_*` / `assign_stable_chapter_numbers` /
//! `completed_chapter_manifest` / `create_preparation_inspection` / `hash_manifest` 等）。
//! **G3 追加**：`assert_execution` / `apply_batch_checkpoint` / effect receipts 与批次推进。
//!
//! # 刻意偏离
//!
//! - `import_source_chapter_map` / `import_run_chapters` 等表在 Tauri 侧同样由 schema 建好，
//!   列集与基线一致，故本层 SQL 可逐字平移；
//! - `completedBatches` 与 `rowToSnapshot` 对 `completed_batches_json` 的**宽松度刻意不同**
//!   （前者校验并抛错，后者 `parseJson(…, {})` 兜底）——与基线一致，勿合并。

use std::collections::{BTreeMap, HashSet};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::draft_units::count_draft_units;
use crate::import::batch_checkpoint::{
    parse_import_run_chapter_batch_checkpoint_id, IMPORT_RUN_BLUEPRINT_BATCH_SIZE,
    IMPORT_RUN_KNOWLEDGE_BATCH_SIZE,
};
use crate::import::limits::{
    MAX_IMPORT_CHAPTERS, MAX_IMPORT_CHAPTER_BYTES, MAX_IMPORT_SOURCE_FILES, MAX_IMPORT_TOTAL_BYTES,
};
use crate::import::parsing::sha256_hex;
use crate::import::{ImportPurpose, ImportRunLocale};

/// 单页最大章节数（对齐 `MAX_PAGE_SIZE`）
pub const MAX_PAGE_SIZE: i64 = 100;

/// 合法的导入运行阶段（对齐 `IMPORT_RUN_STAGE_VALUES`）
pub const IMPORT_RUN_STAGES: [&str; 11] = [
    "parsing",
    "prepared",
    "knowledge",
    "global",
    "style",
    "blueprints",
    "author-commit",
    "author-publish",
    "author-postprocess",
    "refresh",
    "completed",
];

/// 对齐契约 `ImportSourceDisplayMetadata`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSourceDisplayMetadata {
    pub display_name: String,
    pub media_type: String,
    pub size: i64,
}

impl Default for ImportSourceDisplayMetadata {
    fn default() -> Self {
        Self {
            display_name: String::new(),
            media_type: String::new(),
            size: 0,
        }
    }
}

/// 对齐契约 `ImportRunSnapshot`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunSnapshot {
    pub id: String,
    pub purpose: ImportPurpose,
    pub root_run_id: String,
    pub effect_namespace: String,
    /// 仅 `author-manuscript` 快照可见（基线用条件展开）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority_fingerprint: Option<String>,
    pub source_display: Vec<ImportSourceDisplayMetadata>,
    /// 仍需用户重新授权的来源展示事实
    pub unfinished_source_display: Vec<ImportSourceDisplayMetadata>,
    pub locale: ImportRunLocale,
    pub stage: String,
    pub status: String,
    pub completed_batches: BTreeMap<String, Vec<String>>,
    pub last_error: String,
    pub resumable: bool,
    pub cancel_requested: bool,
    pub total_chapters: i64,
    pub total_content_size: i64,
    pub manifest_chapter_count: i64,
    pub manifest_content_size: i64,
    pub manifest_word_count: i64,
    pub completed_chapters: i64,
    pub completed_sources: i64,
    pub total_sources: i64,
    pub progress_completed: i64,
    pub progress_total: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_run_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// 对齐契约 `ImportRunChapterSnapshot`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunChapterSnapshot {
    pub number: i64,
    pub title: String,
    pub content_fingerprint: String,
    pub content_size: i64,
    pub content: String,
}

/// `import_runs` 行（对齐基线 `ImportRunRow`）。
///
/// 列集完整保留 schema 契约；`source_fingerprint` / `legacy_source_fingerprint` /
/// `execution_owner` / `execution_epoch` / `lease_expires_at` 在 G2a（读面）不参与投影，
/// 由 **G2b（写面）与 G3（执行租约）** 消费，故暂抑制死代码警告（对齐 `security.rs` 先例）。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ImportRunRow {
    pub id: String,
    pub purpose: String,
    pub root_run_id: String,
    pub effect_namespace: String,
    pub source_fingerprint: String,
    pub manifest_fingerprint: String,
    pub authority_fingerprint: String,
    pub legacy_source_fingerprint: String,
    pub source_display_json: String,
    pub locale: String,
    pub stage: String,
    pub status: String,
    pub completed_batches_json: String,
    pub last_error: String,
    pub resumable: i64,
    pub cancel_requested: i64,
    pub execution_owner: String,
    pub execution_epoch: i64,
    pub lease_expires_at: i64,
    pub total_chapters: i64,
    pub total_content_size: i64,
    pub manifest_chapter_count: i64,
    pub manifest_content_size: i64,
    pub manifest_word_count: i64,
    pub completed_chapters: i64,
    pub base_run_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

/// `import_run_chapters` 行（对齐基线 `ImportRunChapterRow`）。
///
/// `source_id` / `source_chapter_number` 供 G2b 的 finalize 与章节归属计算使用。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ImportRunChapterRow {
    pub chapter_number: i64,
    pub source_id: String,
    pub source_chapter_number: i64,
    pub title: String,
    pub content_fingerprint: String,
    pub content_size: i64,
    pub content_snapshot: String,
}

/// `import_run_sources` 行（对齐基线 `ImportRunSourceRow`）—— **由 G2b 构建**（本批仅保留结构体）
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ImportRunSourceRow {
    pub run_id: String,
    pub source_index: i64,
    pub source_id: String,
    pub source_fingerprint: String,
    pub legacy_source_fingerprint: String,
    pub display_json: String,
    pub status: String,
    pub manifest_fingerprint: String,
    pub chapter_count: i64,
    pub content_size: i64,
    pub word_count: i64,
    pub last_error: String,
    pub updated_at: String,
}

/// 来源进度汇总（对齐 `sourceProgress` 返回）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceProgress {
    pub completed_sources: i64,
    pub total_sources: i64,
    pub completed_chapters: i64,
}

/// 持久化进度（对齐 `persistedProgress` 返回）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedProgress {
    pub completed_sources: i64,
    pub total_sources: i64,
    pub completed_chapters: i64,
    pub progress_completed: i64,
    pub progress_total: i64,
}

/// `import_runs` 的 SELECT 列序（与 [`map_run_row`] 严格对应）
const RUN_COLUMNS: &str = "id, purpose, root_run_id, effect_namespace, source_fingerprint, \
     manifest_fingerprint, authority_fingerprint, legacy_source_fingerprint, source_display_json, \
     locale, stage, status, completed_batches_json, last_error, resumable, cancel_requested, \
     execution_owner, execution_epoch, lease_expires_at, total_chapters, total_content_size, \
     manifest_chapter_count, manifest_content_size, manifest_word_count, completed_chapters, \
     base_run_id, created_at, updated_at, completed_at";

fn map_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImportRunRow> {
    Ok(ImportRunRow {
        id: row.get(0)?,
        purpose: row.get(1)?,
        root_run_id: row.get(2)?,
        effect_namespace: row.get(3)?,
        source_fingerprint: row.get(4)?,
        manifest_fingerprint: row.get(5)?,
        authority_fingerprint: row.get(6)?,
        legacy_source_fingerprint: row.get(7)?,
        source_display_json: row.get(8)?,
        locale: row.get(9)?,
        stage: row.get(10)?,
        status: row.get(11)?,
        completed_batches_json: row.get(12)?,
        last_error: row.get(13)?,
        resumable: row.get(14)?,
        cancel_requested: row.get(15)?,
        execution_owner: row.get(16)?,
        execution_epoch: row.get(17)?,
        lease_expires_at: row.get(18)?,
        total_chapters: row.get(19)?,
        total_content_size: row.get(20)?,
        manifest_chapter_count: row.get(21)?,
        manifest_content_size: row.get(22)?,
        manifest_word_count: row.get(23)?,
        completed_chapters: row.get(24)?,
        base_run_id: row.get(25)?,
        created_at: row.get(26)?,
        updated_at: row.get(27)?,
        completed_at: row.get(28)?,
    })
}

/// 平移自 `readRunRow`
pub fn read_run_row(conn: &Connection, run_id: &str) -> Result<Option<ImportRunRow>, String> {
    conn.query_row(
        &format!("SELECT {RUN_COLUMNS} FROM import_runs WHERE id = ?"),
        rusqlite::params![run_id],
        map_run_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

/// 平移自 `parseJson`（解析失败或类型不符即回退）
fn parse_json<T: serde::de::DeserializeOwned>(source: &str, fallback: T) -> T {
    serde_json::from_str::<T>(source).unwrap_or(fallback)
}

/// 平移自 `completedBatches`（校验 stage 合法性与 batch 字符串数组，非法即抛错）
pub fn completed_batches(row: &ImportRunRow) -> Result<BTreeMap<String, Vec<String>>, String> {
    let parsed: serde_json::Value = serde_json::from_str(&row.completed_batches_json)
        .map_err(|_| "导入运行 checkpoint 损坏".to_string())?;
    let Some(object) = parsed.as_object() else {
        return Err("导入运行 checkpoint 损坏".to_string());
    };
    let mut result = BTreeMap::new();
    for (stage, batches) in object {
        if !IMPORT_RUN_STAGES.contains(&stage.as_str()) {
            return Err("导入运行 checkpoint 损坏".to_string());
        }
        let Some(array) = batches.as_array() else {
            return Err("导入运行 checkpoint 损坏".to_string());
        };
        let mut collected = Vec::with_capacity(array.len());
        for batch in array {
            let Some(value) = batch.as_str() else {
                return Err("导入运行 checkpoint 损坏".to_string());
            };
            collected.push(value.to_string());
        }
        result.insert(stage.clone(), collected);
    }
    Ok(result)
}

/// 平移自 `sourceProgress`
pub fn source_progress(conn: &Connection, run_id: &str) -> Result<SourceProgress, String> {
    conn.query_row(
        "SELECT COUNT(*) AS total_sources,
                COALESCE(SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'completed' THEN chapter_count ELSE 0 END), 0)
         FROM import_run_sources WHERE run_id = ?",
        rusqlite::params![run_id],
        |row| {
            Ok(SourceProgress {
                total_sources: row.get(0)?,
                completed_sources: row.get(1)?,
                completed_chapters: row.get(2)?,
            })
        },
    )
    .map_err(|error| error.to_string())
}

/// 平移自 `unfinishedSourceDisplay`
pub fn unfinished_source_display(
    conn: &Connection,
    run_id: &str,
) -> Result<Vec<ImportSourceDisplayMetadata>, String> {
    let mut statement = conn
        .prepare(
            "SELECT display_json FROM import_run_sources
             WHERE run_id = ? AND status <> 'completed' ORDER BY source_index",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(rusqlite::params![run_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .map(|json| parse_json(&json, ImportSourceDisplayMetadata::default()))
        .collect())
}

/// 平移自 `chapterRowsForRange`
pub fn chapter_rows_for_range(
    conn: &Connection,
    run_id: &str,
    start_chapter: i64,
    end_chapter: i64,
) -> Result<Vec<(i64, String)>, String> {
    let mut statement = conn
        .prepare(
            "SELECT chapter_number, content_fingerprint FROM import_run_chapters
             WHERE run_id = ? AND chapter_number BETWEEN ? AND ?
             ORDER BY chapter_number ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            rusqlite::params![run_id, start_chapter, end_chapter],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

/// 平移自 `checkpointChapterNumbers`（knowledge / blueprints 两条分支）
pub fn checkpoint_chapter_numbers(
    conn: &Connection,
    row: &ImportRunRow,
    stage: &str,
    batch_id: &str,
) -> Result<Vec<i64>, String> {
    let max_batch = if stage == "knowledge" {
        IMPORT_RUN_KNOWLEDGE_BATCH_SIZE
    } else {
        IMPORT_RUN_BLUEPRINT_BATCH_SIZE
    };
    let parsed = parse_import_run_chapter_batch_checkpoint_id(batch_id)
        .filter(|parsed| parsed.content_fingerprint_prefixes.len() <= max_batch)
        .ok_or_else(|| "导入批次 ID 无效".to_string())?;
    let chapters = chapter_rows_for_range(conn, &row.id, parsed.start_chapter, parsed.end_chapter)?;
    let prefixes_match = chapters.len() == parsed.content_fingerprint_prefixes.len()
        && chapters
            .iter()
            .enumerate()
            .all(|(index, (number, fingerprint))| {
                *number == parsed.start_chapter + index as i64
                    && fingerprint.starts_with(&parsed.content_fingerprint_prefixes[index])
            });
    if !prefixes_match {
        return Err("导入批次 ID 无效".to_string());
    }
    if stage != "knowledge" {
        return Ok(chapters.into_iter().map(|(number, _)| number).collect());
    }

    // knowledge：还要求参照知识 receipt 已完成且与冻结章节逐条对齐
    let receipts = conn
        .prepare(
            "SELECT chapter_number, purpose, source_id, source_chapter_number,
                    content_fingerprint, document_id, state
             FROM import_run_knowledge_receipts
             WHERE run_id = ? AND chapter_number BETWEEN ? AND ?
             ORDER BY chapter_number ASC",
        )
        .map_err(|error| error.to_string())?
        .query_map(
            rusqlite::params![row.id, parsed.start_chapter, parsed.end_chapter],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let affiliations = conn
        .prepare(
            "SELECT chapter_number, source_id, source_chapter_number, content_fingerprint
             FROM import_run_chapters
             WHERE run_id = ? AND chapter_number BETWEEN ? AND ?
             ORDER BY chapter_number ASC",
        )
        .map_err(|error| error.to_string())?
        .query_map(
            rusqlite::params![row.id, parsed.start_chapter, parsed.end_chapter],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let aligned = receipts.len() == affiliations.len()
        && receipts.iter().zip(affiliations.iter()).all(
            |(
                (
                    chapter_number,
                    purpose,
                    source_id,
                    source_chapter_number,
                    fingerprint,
                    document_id,
                    state,
                ),
                affiliation,
            )| {
                *chapter_number == affiliation.0
                    && *purpose == row.purpose
                    && *source_id == affiliation.1
                    && *source_chapter_number == affiliation.2
                    && *fingerprint == affiliation.3
                    && document_id.len() == 64
                    && document_id
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    && state == "committed"
            },
        );
    if !aligned {
        return Err("参照知识 receipt 未完成或与冻结章节不匹配".to_string());
    }
    Ok(chapters.into_iter().map(|(number, _)| number).collect())
}

/// 平移自 `completedCheckpointChapters`
fn completed_checkpoint_chapters(
    conn: &Connection,
    row: &ImportRunRow,
    stage: &str,
) -> Result<i64, String> {
    let batches = completed_batches(row)?;
    let mut covered: HashSet<i64> = HashSet::new();
    for batch_id in batches.get(stage).map(Vec::as_slice).unwrap_or(&[]) {
        for chapter_number in checkpoint_chapter_numbers(conn, row, stage, batch_id)? {
            covered.insert(chapter_number);
        }
    }
    Ok(covered.len() as i64)
}

/// 平移自 `persistedProgress`
pub fn persisted_progress(
    conn: &Connection,
    row: &ImportRunRow,
) -> Result<PersistedProgress, String> {
    let sources = source_progress(conn, &row.id)?;
    let base =
        |completed_chapters: i64, progress_completed: i64, progress_total: i64| PersistedProgress {
            completed_sources: sources.completed_sources,
            total_sources: sources.total_sources,
            completed_chapters,
            progress_completed,
            progress_total,
        };
    if row.stage == "parsing" {
        return Ok(base(
            sources.completed_chapters,
            sources.completed_sources,
            sources.total_sources,
        ));
    }
    let completed_chapters = match row.stage.as_str() {
        "prepared" => 0,
        "knowledge" => completed_checkpoint_chapters(conn, row, "knowledge")?,
        _ => row.total_chapters,
    };
    if row.stage == "knowledge" || row.stage == "prepared" {
        return Ok(base(
            completed_chapters,
            completed_chapters,
            row.total_chapters,
        ));
    }
    if row.stage == "blueprints" {
        return Ok(base(
            completed_chapters,
            completed_checkpoint_chapters(conn, row, "blueprints")?,
            row.total_chapters,
        ));
    }
    if row.stage == "global" || row.stage == "style" || row.stage == "refresh" {
        let done = completed_batches(row)?
            .get(&row.stage)
            .map(|batches| batches.iter().any(|batch| batch == "done"))
            .unwrap_or(false);
        return Ok(base(completed_chapters, if done { 1 } else { 0 }, 1));
    }
    Ok(base(
        row.total_chapters,
        row.total_chapters,
        row.total_chapters,
    ))
}

/// 平移自 `rowToSnapshot`
pub fn row_to_snapshot(conn: &Connection, row: &ImportRunRow) -> Result<ImportRunSnapshot, String> {
    let purpose = ImportPurpose::parse_contract(&row.purpose)?;
    let progress = persisted_progress(conn, row)?;
    // 基线：`completedBatches` 输出字段走 `parseJson(…, {})`（宽松），与上面校验版刻意不同
    let completed_batches =
        parse_json::<BTreeMap<String, Vec<String>>>(&row.completed_batches_json, BTreeMap::new());
    let is_author = purpose == ImportPurpose::AuthorManuscript;
    Ok(ImportRunSnapshot {
        id: row.id.clone(),
        purpose,
        root_run_id: row.root_run_id.clone(),
        effect_namespace: row.effect_namespace.clone(),
        manifest_fingerprint: if is_author {
            Some(row.manifest_fingerprint.clone())
        } else {
            None
        },
        authority_fingerprint: if is_author && !row.authority_fingerprint.is_empty() {
            Some(row.authority_fingerprint.clone())
        } else {
            None
        },
        source_display: parse_json(&row.source_display_json, Vec::new()),
        unfinished_source_display: unfinished_source_display(conn, &row.id)?,
        locale: ImportRunLocale::parse_contract(&row.locale)?,
        stage: row.stage.clone(),
        status: row.status.clone(),
        completed_batches,
        last_error: row.last_error.clone(),
        resumable: row.resumable == 1,
        cancel_requested: row.cancel_requested == 1,
        total_chapters: row.total_chapters,
        total_content_size: row.total_content_size,
        manifest_chapter_count: row.manifest_chapter_count,
        manifest_content_size: row.manifest_content_size,
        manifest_word_count: row.manifest_word_count,
        completed_chapters: progress.completed_chapters,
        completed_sources: progress.completed_sources,
        total_sources: progress.total_sources,
        progress_completed: progress.progress_completed,
        progress_total: progress.progress_total,
        base_run_id: row.base_run_id.clone(),
        created_at: row.created_at.clone(),
        updated_at: row.updated_at.clone(),
        completed_at: row.completed_at.clone(),
    })
}

/// 平移自 `assertFrozenChapterSnapshot`
pub fn assert_frozen_chapter_snapshot(
    content_fingerprint: &str,
    content_size: i64,
    content_snapshot: &str,
) -> Result<(), String> {
    let actual_size = content_snapshot.len() as i64;
    let actual_fingerprint = sha256_hex(content_snapshot);
    let fingerprint_well_formed = content_fingerprint.len() == 64
        && content_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if content_size < 1
        || content_size > MAX_IMPORT_CHAPTER_BYTES as i64
        || actual_size != content_size
        || !fingerprint_well_formed
        || actual_fingerprint != content_fingerprint
    {
        return Err("导入冻结章节快照损坏，已拒绝继续".to_string());
    }
    Ok(())
}

/// 平移自 `chapterRowToSnapshot`
pub fn chapter_row_to_snapshot(
    row: &ImportRunChapterRow,
) -> Result<ImportRunChapterSnapshot, String> {
    assert_frozen_chapter_snapshot(
        &row.content_fingerprint,
        row.content_size,
        &row.content_snapshot,
    )?;
    Ok(ImportRunChapterSnapshot {
        number: row.chapter_number,
        title: row.title.clone(),
        content_fingerprint: row.content_fingerprint.clone(),
        content_size: row.content_size,
        content: row.content_snapshot.clone(),
    })
}

/// 平移自 `get`（`db:import-run-get`）
pub fn get(conn: &Connection, run_id: &str) -> Result<Option<ImportRunSnapshot>, String> {
    match read_run_row(conn, run_id)? {
        Some(row) => Ok(Some(row_to_snapshot(conn, &row)?)),
        None => Ok(None),
    }
}

/// 平移自 `listResumable`（`db:import-run-list-resumable`）
pub fn list_resumable(conn: &Connection) -> Result<Vec<ImportRunSnapshot>, String> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT {RUN_COLUMNS} FROM import_runs
             WHERE resumable = 1 AND status IN ('ready', 'running', 'failed', 'cancelled')
             ORDER BY updated_at DESC, rowid DESC"
        ))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], map_run_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    rows.iter().map(|row| row_to_snapshot(conn, row)).collect()
}

/// 平移自 `listChapterBatch`（`db:import-run-list-chapters`；`limit` 夹到 1..=100）
pub fn list_chapter_batch(
    conn: &Connection,
    run_id: &str,
    after_chapter_number: i64,
    limit: i64,
) -> Result<Vec<ImportRunChapterSnapshot>, String> {
    let limit = limit.clamp(1, MAX_PAGE_SIZE);
    let mut statement = conn
        .prepare(
            "SELECT chapter_number, source_id, source_chapter_number,
                    title, content_fingerprint, content_size, content_snapshot
             FROM import_run_chapters
             WHERE run_id = ? AND chapter_number > ?
             ORDER BY chapter_number ASC LIMIT ?",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            rusqlite::params![run_id, after_chapter_number, limit],
            |row| {
                Ok(ImportRunChapterRow {
                    chapter_number: row.get(0)?,
                    source_id: row.get(1)?,
                    source_chapter_number: row.get(2)?,
                    title: row.get(3)?,
                    content_fingerprint: row.get(4)?,
                    content_size: row.get(5)?,
                    content_snapshot: row.get(6)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    rows.iter().map(chapter_row_to_snapshot).collect()
}

// ==================== 批次 G2b：解析写入面 ====================

/// sha256 小写 hex（64 位）——与 `inspection_store` / `identity` 同口径
fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// 不透明来源 id（对齐基线 `OPAQUE_SOURCE_ID`：UUID v4，大小写不敏感）
fn is_opaque_source_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    let mut index = 0usize;
    for byte in bytes {
        let expected_dash = matches!(index, 8 | 13 | 18 | 23);
        if expected_dash {
            if *byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
        index += 1;
    }
    // 版本位 1..=5，变体位 8/9/a/b
    matches!(bytes[14].to_ascii_lowercase(), b'1'..=b'5')
        && matches!(bytes[19].to_ascii_lowercase(), b'8' | b'9' | b'a' | b'b')
}

/// 对齐契约 `ImportRunChapterInput`（主进程冻结快照，跨进程入参）
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunChapterInput {
    pub number: i64,
    #[serde(default)]
    pub source_index: Option<i64>,
    #[serde(default)]
    pub source_chapter_number: Option<i64>,
    pub title: String,
    pub content_fingerprint: String,
    pub content_size: i64,
    pub content: String,
}

/// 对齐基线 `NormalizedImportRunChapter`（补齐来源归属后的章节）
#[derive(Debug, Clone)]
pub struct NormalizedImportRunChapter {
    pub number: i64,
    pub source_index: i64,
    pub source_id: String,
    pub source_chapter_number: i64,
    pub title: String,
    pub content_fingerprint: String,
    pub content_size: i64,
    pub content: String,
}

/// 对齐基线 `ImportRunBeginParsingRequest`（主进程内部构造，不跨 IPC）
#[derive(Debug, Clone)]
pub struct ImportRunBeginParsingRequest {
    pub run_id: String,
    pub purpose: ImportPurpose,
    pub source_fingerprint: String,
    pub source_ids: Option<Vec<String>>,
    pub source_fingerprints: Option<Vec<String>>,
    pub legacy_source_fingerprints: Option<Vec<String>>,
    pub legacy_collection_fingerprint: Option<String>,
    pub source_display: Vec<ImportSourceDisplayMetadata>,
    pub locale: ImportRunLocale,
}

/// manifest 单章（**字段声明序 = 基线 `JSON.stringify` 的键序**，勿调整）
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalManifestChapter<'a> {
    number: i64,
    source_id: &'a str,
    source_chapter_number: i64,
    title: &'a str,
    content_fingerprint: &'a str,
    content_size: i64,
}

/// manifest 根（键序 purpose → chapters）
#[derive(Debug, serde::Serialize)]
struct CanonicalManifest<'a> {
    purpose: ImportPurpose,
    chapters: Vec<CanonicalManifestChapter<'a>>,
}

/// 平移自 `canonicalManifest`：serde 按字段声明序输出，故与基线 `JSON.stringify` 逐字节一致
pub fn canonical_manifest(
    purpose: ImportPurpose,
    chapters: &[NormalizedImportRunChapter],
) -> String {
    let manifest = CanonicalManifest {
        purpose,
        chapters: chapters
            .iter()
            .map(|chapter| CanonicalManifestChapter {
                number: chapter.number,
                source_id: &chapter.source_id,
                source_chapter_number: chapter.source_chapter_number,
                title: &chapter.title,
                content_fingerprint: &chapter.content_fingerprint,
                content_size: chapter.content_size,
            })
            .collect(),
    };
    serde_json::to_string(&manifest).unwrap_or_default()
}

/// 平移自 `hashManifest`
pub fn hash_manifest(purpose: ImportPurpose, chapters: &[NormalizedImportRunChapter]) -> String {
    sha256_hex(&canonical_manifest(purpose, chapters))
}

/// 平移自 `normalizeDisplay`
pub fn normalize_display(
    items: &[ImportSourceDisplayMetadata],
) -> Result<Vec<ImportSourceDisplayMetadata>, String> {
    if items.is_empty() || items.len() > MAX_IMPORT_SOURCE_FILES {
        return Err("导入来源展示信息无效".to_string());
    }
    items
        .iter()
        .map(|item| {
            let display_name = item.display_name.trim();
            let media_type = item.media_type.trim();
            if display_name.is_empty()
                || display_name.encode_utf16().count() > 255
                || display_name.contains('/')
                || display_name.contains('\\')
            {
                return Err("导入来源展示名无效".to_string());
            }
            if media_type.is_empty() || media_type.encode_utf16().count() > 100 || item.size < 0 {
                return Err("导入来源展示信息无效".to_string());
            }
            Ok(ImportSourceDisplayMetadata {
                display_name: display_name.to_string(),
                media_type: media_type.to_string(),
                size: item.size,
            })
        })
        .collect()
}

/// 平移自 `normalizeSourceIds`（`source_ids == None` 退化为 legacy 单来源身份）
pub fn normalize_source_ids(
    items: Option<&[String]>,
    source_display: &[ImportSourceDisplayMetadata],
    source_fingerprint: &str,
) -> Result<Vec<String>, String> {
    let Some(items) = items else {
        return Ok(vec![format!("legacy:{source_fingerprint}")]);
    };
    if items.is_empty() || items.len() != source_display.len() {
        return Err("导入来源身份与展示信息不匹配".to_string());
    }
    let normalized: Vec<String> = items.iter().map(|item| item.trim().to_string()).collect();
    let mut seen = HashSet::new();
    if normalized.iter().any(|item| !is_opaque_source_id(item))
        || normalized.iter().any(|item| !seen.insert(item.clone()))
    {
        return Err("导入来源身份无效或重复".to_string());
    }
    Ok(normalized)
}

/// 平移自 `normalizeSourceFingerprints`（`None` → 空表，由调用方做长度校验）
pub fn normalize_source_fingerprints(
    items: Option<&[String]>,
    source_ids: &[String],
) -> Result<Vec<String>, String> {
    let Some(items) = items else {
        return Ok(Vec::new());
    };
    if items.len() != source_ids.len() || items.iter().any(|item| !is_sha256(item)) {
        return Err("导入来源单文件指纹无效".to_string());
    }
    Ok(items.to_vec())
}

/// 平移自 `normalizeChapters`
pub fn normalize_chapters(
    items: &[ImportRunChapterInput],
    source_ids: &[String],
) -> Result<Vec<NormalizedImportRunChapter>, String> {
    if items.is_empty() || items.len() > MAX_IMPORT_CHAPTERS {
        return Err("导入章节清单无效".to_string());
    }
    let mut affiliations: HashSet<(String, i64)> = HashSet::new();
    let mut aggregate_bytes: i64 = 0;
    let mut normalized = Vec::with_capacity(items.len());
    for item in items {
        let source_index = item.source_index.unwrap_or(0);
        let source_chapter_number = item.source_chapter_number.unwrap_or(item.number);
        if item.number < 1
            || source_index < 0
            || source_index as usize >= source_ids.len()
            || source_chapter_number < 1
        {
            return Err("导入章节归属无效".to_string());
        }
        let source_id = source_ids[source_index as usize].clone();
        if !affiliations.insert((source_id.clone(), source_chapter_number)) {
            return Err("导入章节来源归属重复".to_string());
        }
        let bytes = item.content.len() as i64;
        if item.title.encode_utf16().count() > 500
            || !is_sha256(&item.content_fingerprint)
            || item.content_size != bytes
            || bytes == 0
            || bytes > MAX_IMPORT_CHAPTER_BYTES as i64
        {
            return Err(format!("导入章节 {} 快照无效", item.number));
        }
        aggregate_bytes += bytes;
        if aggregate_bytes > MAX_IMPORT_TOTAL_BYTES as i64 {
            return Err("导入正文总字节数超过安全上限".to_string());
        }
        normalized.push(NormalizedImportRunChapter {
            number: item.number,
            source_index,
            source_id,
            source_chapter_number,
            title: item.title.trim().to_string(),
            content_fingerprint: item.content_fingerprint.clone(),
            content_size: item.content_size,
            content: item.content.clone(),
        });
    }
    Ok(normalized)
}

/// 平移自 `parsedSourceStatus`
pub fn parsed_source_status(
    conn: &Connection,
    run_id: &str,
    source_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT status FROM import_run_sources WHERE run_id = ? AND source_id = ?",
        rusqlite::params![run_id, source_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn read_source_row(
    conn: &Connection,
    run_id: &str,
    source_id: &str,
) -> Result<Option<ImportRunSourceRow>, String> {
    conn.query_row(
        "SELECT run_id, source_index, source_id, source_fingerprint, legacy_source_fingerprint,
                display_json, status, manifest_fingerprint, chapter_count, content_size,
                word_count, last_error, updated_at
         FROM import_run_sources WHERE run_id = ? AND source_id = ?",
        rusqlite::params![run_id, source_id],
        |row| {
            Ok(ImportRunSourceRow {
                run_id: row.get(0)?,
                source_index: row.get(1)?,
                source_id: row.get(2)?,
                source_fingerprint: row.get(3)?,
                legacy_source_fingerprint: row.get(4)?,
                display_json: row.get(5)?,
                status: row.get(6)?,
                manifest_fingerprint: row.get(7)?,
                chapter_count: row.get(8)?,
                content_size: row.get(9)?,
                word_count: row.get(10)?,
                last_error: row.get(11)?,
                updated_at: row.get(12)?,
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())
}

/// 平移自 `beginParsing`（仅 `reference`；作者原稿走 [`crate-]` 的 `prepare`，G2b-2 落地）
pub fn begin_parsing(
    conn: &Connection,
    candidate: &ImportRunBeginParsingRequest,
) -> Result<ImportRunSnapshot, String> {
    let run_id = candidate.run_id.trim();
    if run_id.is_empty()
        || run_id.encode_utf16().count() > 160
        || !is_sha256(&candidate.source_fingerprint)
    {
        return Err("导入运行身份无效".to_string());
    }
    if candidate.purpose != ImportPurpose::Reference {
        return Err("当前版本不支持作者手稿导入".to_string());
    }
    let source_display = normalize_display(&candidate.source_display)?;
    let source_ids = normalize_source_ids(
        candidate.source_ids.as_deref(),
        &source_display,
        &candidate.source_fingerprint,
    )?;
    let source_fingerprints =
        normalize_source_fingerprints(candidate.source_fingerprints.as_deref(), &source_ids)?;
    if source_fingerprints.len() != source_ids.len() {
        return Err("导入来源单文件指纹无效".to_string());
    }
    let legacy_source_fingerprints = candidate
        .legacy_source_fingerprints
        .clone()
        .unwrap_or_else(|| source_ids.iter().map(|_| String::new()).collect());
    if legacy_source_fingerprints.len() != source_ids.len()
        || legacy_source_fingerprints
            .iter()
            .any(|value| !value.is_empty() && !is_sha256(value))
    {
        return Err("旧导入来源单文件指纹无效".to_string());
    }
    if candidate
        .legacy_collection_fingerprint
        .as_deref()
        .is_some_and(|value| !value.is_empty() && !is_sha256(value))
    {
        return Err("旧导入来源集合指纹无效".to_string());
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    // 分支 1：显式 runId 已存在 → 「重新授权未完成来源」
    if let Some(explicit_run) = read_run_row(&tx, run_id)? {
        let persisted = read_sources_in_order(&tx, &explicit_run.id)?;
        if explicit_run.purpose != candidate.purpose.as_str()
            || explicit_run.stage != "parsing"
            || explicit_run.resumable != 1
            || explicit_run.locale != candidate.locale.as_str()
        {
            return Err("指定的导入运行当前不可重新授权未完成来源".to_string());
        }
        let by_id: std::collections::HashMap<&str, &ImportRunSourceRow> = persisted
            .iter()
            .map(|source| (source.source_id.as_str(), source))
            .collect();
        for (index, source_id) in source_ids.iter().enumerate() {
            let Some(persisted_source) = by_id.get(source_id.as_str()) else {
                return Err("未完成导入的来源清单与本次重新授权不一致".to_string());
            };
            if persisted_source.status == "completed"
                || persisted_source.source_fingerprint != source_fingerprints[index]
            {
                return Err("未完成导入的来源清单与本次重新授权不一致".to_string());
            }
        }
        for (index, source_id) in source_ids.iter().enumerate() {
            let changed = tx
                .execute(
                    "UPDATE import_run_sources
                     SET legacy_source_fingerprint = ?, display_json = ?, updated_at = datetime('now')
                     WHERE run_id = ? AND source_id = ? AND source_fingerprint = ? AND status <> 'completed'",
                    rusqlite::params![
                        legacy_source_fingerprints[index],
                        serde_json::to_string(&source_display[index]).unwrap_or_default(),
                        explicit_run.id,
                        source_id,
                        source_fingerprints[index],
                    ],
                )
                .map_err(|error| error.to_string())?;
            if changed != 1 {
                return Err("未完成导入的来源清单与本次重新授权不一致".to_string());
            }
        }
        // 回写整批展示信息（按 source_index 顺序）
        let full_display: Vec<ImportSourceDisplayMetadata> = persisted
            .iter()
            .map(|source| parse_json(&source.display_json, ImportSourceDisplayMetadata::default()))
            .collect();
        tx.execute(
            "UPDATE import_runs SET source_display_json = ?, updated_at = datetime('now') WHERE id = ?",
            rusqlite::params![
                serde_json::to_string(&full_display).unwrap_or_default(),
                explicit_run.id
            ],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        return get(conn, &explicit_run.id)?.ok_or_else(|| "导入运行读取失败".to_string());
    }

    // 分支 2：命中同指纹的既有 parsing run → 重建来源清单
    let existing = tx
        .query_row(
            &format!(
                "SELECT {RUN_COLUMNS} FROM import_runs
                 WHERE purpose = ? AND source_fingerprint = ? AND stage = 'parsing' AND resumable = 1
                 ORDER BY updated_at DESC, rowid DESC LIMIT 1"
            ),
            rusqlite::params![candidate.purpose.as_str(), candidate.source_fingerprint],
            map_run_row,
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(existing) = existing {
        let sources = read_sources_in_order(&tx, &existing.id)?;
        let by_id: std::collections::HashMap<&str, &ImportRunSourceRow> = sources
            .iter()
            .map(|source| (source.source_id.as_str(), source))
            .collect();
        if sources.len() != source_ids.len()
            || source_ids.iter().enumerate().any(|(index, source_id)| {
                by_id
                    .get(source_id.as_str())
                    .map(|source| source.source_fingerprint.as_str())
                    != Some(source_fingerprints[index].as_str())
            })
        {
            return Err("未完成导入的来源清单与本次重新授权不一致".to_string());
        }
        // UNIQUE(run_id, source_index) 避让：先整体顶到 MAX_IMPORT_SOURCE_FILES 之后，再逐条写回真实 index
        tx.execute(
            "UPDATE import_run_sources SET source_index = source_index + ? WHERE run_id = ?",
            rusqlite::params![MAX_IMPORT_SOURCE_FILES as i64, existing.id],
        )
        .map_err(|error| error.to_string())?;
        for (index, source_id) in source_ids.iter().enumerate() {
            let changed = tx
                .execute(
                    "UPDATE import_run_sources
                     SET source_index = ?, legacy_source_fingerprint = ?, display_json = ?, updated_at = datetime('now')
                     WHERE run_id = ? AND source_id = ? AND source_fingerprint = ?",
                    rusqlite::params![
                        index as i64,
                        legacy_source_fingerprints[index],
                        serde_json::to_string(&source_display[index]).unwrap_or_default(),
                        existing.id,
                        source_id,
                        source_fingerprints[index],
                    ],
                )
                .map_err(|error| error.to_string())?;
            if changed != 1 {
                return Err("未完成导入的来源清单与本次重新授权不一致".to_string());
            }
        }
        tx.execute(
            "UPDATE import_runs SET source_display_json = ?, updated_at = datetime('now') WHERE id = ?",
            rusqlite::params![
                serde_json::to_string(&source_display).unwrap_or_default(),
                existing.id
            ],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        return get(conn, &existing.id)?.ok_or_else(|| "导入运行读取失败".to_string());
    }

    // 分支 3：新建 parsing run
    if read_run_row(&tx, run_id)?.is_some() {
        return Err("导入运行 ID 已存在".to_string());
    }
    tx.execute(
        "INSERT INTO import_runs (
            id, purpose, root_run_id, effect_namespace, source_fingerprint, manifest_fingerprint,
            legacy_source_fingerprint, source_display_json, locale, stage, status,
            total_chapters, total_content_size, manifest_chapter_count, manifest_content_size,
            manifest_word_count, completed_chapters
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'parsing', 'ready', 0, 0, 0, 0, 0, 0)",
        rusqlite::params![
            run_id,
            candidate.purpose.as_str(),
            run_id,
            format!("import:{}:{run_id}", candidate.purpose.as_str()),
            candidate.source_fingerprint,
            "0".repeat(64),
            candidate
                .legacy_collection_fingerprint
                .clone()
                .unwrap_or_default(),
            serde_json::to_string(&source_display).unwrap_or_default(),
            candidate.locale.as_str(),
        ],
    )
    .map_err(|error| error.to_string())?;
    for (index, source_id) in source_ids.iter().enumerate() {
        tx.execute(
            "INSERT INTO import_run_sources (
                run_id, source_index, source_id, source_fingerprint, legacy_source_fingerprint, display_json
             ) VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                run_id,
                index as i64,
                source_id,
                source_fingerprints[index],
                legacy_source_fingerprints[index],
                serde_json::to_string(&source_display[index]).unwrap_or_default(),
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    tx.commit().map_err(|error| error.to_string())?;
    get(conn, run_id)?.ok_or_else(|| "导入运行读取失败".to_string())
}

fn read_sources_in_order(
    conn: &Connection,
    run_id: &str,
) -> Result<Vec<ImportRunSourceRow>, String> {
    let mut statement = conn
        .prepare(
            "SELECT run_id, source_index, source_id, source_fingerprint, legacy_source_fingerprint,
                    display_json, status, manifest_fingerprint, chapter_count, content_size,
                    word_count, last_error, updated_at
             FROM import_run_sources WHERE run_id = ? ORDER BY source_index",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(rusqlite::params![run_id], |row| {
            Ok(ImportRunSourceRow {
                run_id: row.get(0)?,
                source_index: row.get(1)?,
                source_id: row.get(2)?,
                source_fingerprint: row.get(3)?,
                legacy_source_fingerprint: row.get(4)?,
                display_json: row.get(5)?,
                status: row.get(6)?,
                manifest_fingerprint: row.get(7)?,
                chapter_count: row.get(8)?,
                content_size: row.get(9)?,
                word_count: row.get(10)?,
                last_error: row.get(11)?,
                updated_at: row.get(12)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

/// 平移自 `commitParsedSource`
pub fn commit_parsed_source(
    conn: &Connection,
    run_id: &str,
    source_id: &str,
    chapters: &[ImportRunChapterInput],
) -> Result<ImportRunSnapshot, String> {
    if !is_opaque_source_id(source_id) {
        return Err("导入解析来源身份无效".to_string());
    }
    if chapters.is_empty() {
        return Err("导入解析来源没有可导入的正文".to_string());
    }
    if chapters
        .iter()
        .any(|chapter| sha256_hex(&chapter.content) != chapter.content_fingerprint)
    {
        return Err("导入解析来源内容指纹与冻结快照不一致".to_string());
    }
    let source_ids = vec![source_id.to_string()];
    let normalized = normalize_chapters(chapters, &source_ids)?;
    let renumbered: Vec<NormalizedImportRunChapter> = normalized
        .iter()
        .enumerate()
        .map(|(index, chapter)| NormalizedImportRunChapter {
            number: index as i64 + 1,
            ..chapter.clone()
        })
        .collect();
    let manifest_fingerprint = hash_manifest(ImportPurpose::Reference, &renumbered);

    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let Some(run) = read_run_row(&tx, run_id)? else {
        return Err("导入解析运行当前不可写入来源".to_string());
    };
    if run.purpose != "reference"
        || run.stage != "parsing"
        || !matches!(run.status.as_str(), "ready" | "failed")
    {
        return Err("导入解析运行当前不可写入来源".to_string());
    }
    let Some(source) = read_source_row(&tx, run_id, source_id)? else {
        return Err("导入解析来源不存在".to_string());
    };
    if source.status == "completed" {
        if source.manifest_fingerprint != manifest_fingerprint {
            return Err("已完成来源与本次重新授权内容不一致".to_string());
        }
        tx.commit().map_err(|error| error.to_string())?;
        return get(conn, run_id)?.ok_or_else(|| "导入运行读取失败".to_string());
    }
    tx.execute(
        "DELETE FROM import_run_source_chapters WHERE run_id = ? AND source_id = ?",
        rusqlite::params![run_id, source_id],
    )
    .map_err(|error| error.to_string())?;
    let mut content_size: i64 = 0;
    let mut word_count: i64 = 0;
    for chapter in &normalized {
        let chapter_words = count_draft_units(&chapter.content);
        tx.execute(
            "INSERT INTO import_run_source_chapters (
                run_id, source_id, source_chapter_number, title, content_fingerprint,
                content_size, word_count, content_snapshot
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                run_id,
                source_id,
                chapter.source_chapter_number,
                chapter.title,
                chapter.content_fingerprint,
                chapter.content_size,
                chapter_words,
                chapter.content,
            ],
        )
        .map_err(|error| error.to_string())?;
        content_size += chapter.content_size;
        word_count += chapter_words;
    }
    tx.execute(
        "UPDATE import_run_sources
         SET status = 'completed', manifest_fingerprint = ?, chapter_count = ?,
             content_size = ?, word_count = ?, last_error = '', updated_at = datetime('now')
         WHERE run_id = ? AND source_id = ?",
        rusqlite::params![
            manifest_fingerprint,
            normalized.len() as i64,
            content_size,
            word_count,
            run_id,
            source_id
        ],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "UPDATE import_runs
         SET status = 'ready', last_error = '',
             total_chapters = (SELECT COALESCE(SUM(chapter_count), 0) FROM import_run_sources WHERE run_id = ? AND status = 'completed'),
             total_content_size = (SELECT COALESCE(SUM(content_size), 0) FROM import_run_sources WHERE run_id = ? AND status = 'completed'),
             updated_at = datetime('now')
         WHERE id = ?",
        rusqlite::params![run_id, run_id, run_id],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    get(conn, run_id)?.ok_or_else(|| "导入运行读取失败".to_string())
}

/// 平移自 `failParsedSource`（`error` 截断到 2000 个 UTF-16 单元）
pub fn fail_parsed_source(
    conn: &Connection,
    run_id: &str,
    source_id: &str,
    error: &str,
) -> Result<ImportRunSnapshot, String> {
    if error.trim().is_empty() {
        return Err("导入解析失败原因无效".to_string());
    }
    let message = {
        let mut buffer = String::new();
        for unit in error.encode_utf16().take(2_000) {
            if let Some(character) = char::from_u32(unit as u32) {
                buffer.push(character);
            }
        }
        buffer
    };
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let Some(run) = read_run_row(&tx, run_id)? else {
        return Err("导入解析运行当前不可标记失败".to_string());
    };
    if run.stage != "parsing" || !matches!(run.status.as_str(), "ready" | "failed") {
        return Err("导入解析运行当前不可标记失败".to_string());
    }
    let changed = tx
        .execute(
            "UPDATE import_run_sources SET status = 'failed', last_error = ?, updated_at = datetime('now')
             WHERE run_id = ? AND source_id = ? AND status <> 'completed'",
            rusqlite::params![message, run_id, source_id],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("导入解析来源当前不可标记失败".to_string());
    }
    tx.execute(
        "UPDATE import_runs SET status = 'failed', last_error = ?, resumable = 1,
           updated_at = datetime('now') WHERE id = ?",
        rusqlite::params![message, run_id],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    get(conn, run_id)?.ok_or_else(|| "导入运行读取失败".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn memory_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库应可打开");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("应启用外键");
        schema::create_tables(&conn).expect("建表应成功");
        conn
    }

    fn insert_run(conn: &Connection, id: &str, stage: &str, status: &str, purpose: &str) {
        conn.execute(
            "INSERT INTO import_runs (
                id, purpose, root_run_id, effect_namespace, source_fingerprint, manifest_fingerprint,
                authority_fingerprint, source_display_json, locale, stage, status,
                completed_batches_json, resumable, total_chapters, total_content_size,
                manifest_chapter_count, manifest_content_size, manifest_word_count, completed_chapters
             ) VALUES (?, ?, ?, ?, ?, ?, '', ?, 'zh-CN', ?, ?, '{}', 1, 2, 20, 2, 20, 6, 0)",
            rusqlite::params![
                id,
                purpose,
                id,
                format!("import:{purpose}:{id}"),
                "a".repeat(64),
                "b".repeat(64),
                serde_json::json!([{ "displayName": "第1章.txt", "mediaType": "text/plain", "size": 10 }]).to_string(),
                stage,
                status,
            ],
        )
        .unwrap();
    }

    fn insert_source(
        conn: &Connection,
        run_id: &str,
        index: i64,
        status: &str,
        chapter_count: i64,
    ) {
        conn.execute(
            "INSERT INTO import_run_sources (
                run_id, source_index, source_id, source_fingerprint, display_json,
                status, chapter_count, content_size, word_count
             ) VALUES (?, ?, ?, ?, ?, ?, ?, 10, 3)",
            rusqlite::params![
                run_id,
                index,
                format!("1111111{index}-1111-4111-8111-11111111111{index}"),
                "c".repeat(64),
                serde_json::json!({ "displayName": format!("s{index}.txt"), "mediaType": "text/plain", "size": 10 }).to_string(),
                status,
                chapter_count,
            ],
        )
        .unwrap();
    }

    fn insert_chapter(conn: &Connection, run_id: &str, number: i64, content: &str) -> String {
        let fingerprint = sha256_hex(content);
        conn.execute(
            "INSERT INTO import_run_chapters (
                run_id, chapter_number, source_id, source_chapter_number,
                title, content_fingerprint, content_size, content_snapshot
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                run_id,
                number,
                "11111111-1111-4111-8111-111111111111",
                number,
                format!("第{number}章"),
                fingerprint,
                content.len() as i64,
                content,
            ],
        )
        .unwrap();
        fingerprint
    }

    #[test]
    fn parse_helpers_roundtrip_contract_values_test() {
        assert_eq!(
            ImportPurpose::parse_contract("reference").unwrap(),
            ImportPurpose::Reference
        );
        assert_eq!(
            ImportPurpose::parse_contract("author-manuscript").unwrap(),
            ImportPurpose::AuthorManuscript
        );
        assert!(ImportPurpose::parse_contract("bogus").is_err());
        assert_eq!(
            ImportRunLocale::parse_contract("en-US").unwrap(),
            ImportRunLocale::EnUs
        );
        assert!(ImportRunLocale::parse_contract("fr-FR").is_err());
    }

    #[test]
    fn row_to_snapshot_exposes_reference_shape_test() {
        let conn = memory_conn();
        insert_run(&conn, "run-ref", "parsing", "ready", "reference");
        insert_source(&conn, "run-ref", 0, "completed", 1);
        insert_source(&conn, "run-ref", 1, "pending", 1);

        let snapshot = get(&conn, "run-ref").unwrap().unwrap();
        assert_eq!(snapshot.purpose, ImportPurpose::Reference);
        assert_eq!(snapshot.stage, "parsing");
        assert_eq!(snapshot.status, "ready");
        assert!(snapshot.resumable);
        assert!(!snapshot.cancel_requested);
        // reference 不暴露 manifest/authority 指纹
        assert!(snapshot.manifest_fingerprint.is_none());
        assert!(snapshot.authority_fingerprint.is_none());
        // parsing 阶段进度 = 来源数
        assert_eq!(snapshot.completed_sources, 1);
        assert_eq!(snapshot.total_sources, 2);
        assert_eq!(snapshot.completed_chapters, 1);
        assert_eq!(snapshot.progress_completed, 1);
        assert_eq!(snapshot.progress_total, 2);
        // 未完成来源展示事实只含 pending 那一条
        assert_eq!(snapshot.unfinished_source_display.len(), 1);
        assert_eq!(snapshot.unfinished_source_display[0].display_name, "s1.txt");
        assert_eq!(snapshot.source_display.len(), 1);
        assert_eq!(snapshot.source_display[0].display_name, "第1章.txt");
        assert!(snapshot.base_run_id.is_none());
        assert!(snapshot.completed_at.is_none());
    }

    #[test]
    fn row_to_snapshot_exposes_author_hashes_test() {
        let conn = memory_conn();
        insert_run(
            &conn,
            "run-author",
            "author-commit",
            "ready",
            "author-manuscript",
        );
        let snapshot = get(&conn, "run-author").unwrap().unwrap();
        assert_eq!(snapshot.purpose, ImportPurpose::AuthorManuscript);
        assert_eq!(
            snapshot.manifest_fingerprint.as_deref(),
            Some("b".repeat(64).as_str())
        );
        // authority_fingerprint 为空串时不下发
        assert!(snapshot.authority_fingerprint.is_none());
        // author 的非 parsing 阶段：completedChapters = total_chapters
        assert_eq!(snapshot.completed_chapters, 2);
        assert_eq!(snapshot.progress_completed, 2);
        assert_eq!(snapshot.progress_total, 2);
    }

    #[test]
    fn persisted_progress_knowledge_uses_checkpoint_batches_test() {
        let conn = memory_conn();
        insert_run(&conn, "run-kb", "knowledge", "running", "reference");
        let first = insert_chapter(&conn, "run-kb", 1, "正文甲");
        let second = insert_chapter(&conn, "run-kb", 2, "正文乙");
        let batch_id = format!("1-2-{}.{}", &first[..8], &second[..8]);
        conn.execute(
            "UPDATE import_runs SET completed_batches_json = ? WHERE id = 'run-kb'",
            rusqlite::params![serde_json::json!({ "knowledge": [batch_id] }).to_string()],
        )
        .unwrap();
        // 只有 1 章有 receipt，checkpoint 校验失败 → 不能算完成
        let broken = persisted_progress(&conn, &read_run_row(&conn, "run-kb").unwrap().unwrap());
        assert!(broken.is_err());

        for (number, fingerprint) in [(1_i64, first.clone()), (2_i64, second.clone())] {
            conn.execute(
                "INSERT INTO import_run_knowledge_receipts (
                    run_id, chapter_number, purpose, source_id, source_chapter_number,
                    content_fingerprint, document_id, state
                 ) VALUES ('run-kb', ?, 'reference', '11111111-1111-4111-8111-111111111111', ?, ?, ?, 'committed')",
                rusqlite::params![number, number, fingerprint, "d".repeat(64)],
            )
            .unwrap();
        }
        let progress =
            persisted_progress(&conn, &read_run_row(&conn, "run-kb").unwrap().unwrap()).unwrap();
        assert_eq!(progress.completed_chapters, 2);
        assert_eq!(progress.progress_completed, 2);
        assert_eq!(progress.progress_total, 2);
    }

    #[test]
    fn persisted_progress_global_done_batch_test() {
        let conn = memory_conn();
        insert_run(&conn, "run-global", "global", "running", "reference");
        let before =
            persisted_progress(&conn, &read_run_row(&conn, "run-global").unwrap().unwrap())
                .unwrap();
        assert_eq!(before.progress_completed, 0);
        assert_eq!(before.progress_total, 1);
        let after = persisted_progress(&conn, &read_run_row(&conn, "run-global").unwrap().unwrap());
        assert!(after.is_ok());
        conn.execute(
            "UPDATE import_runs SET completed_batches_json = '{\"global\":[\"done\"]}' WHERE id = 'run-global'",
            [],
        )
        .unwrap();
        let done = persisted_progress(&conn, &read_run_row(&conn, "run-global").unwrap().unwrap())
            .unwrap();
        assert_eq!(done.progress_completed, 1);
        assert_eq!(done.progress_total, 1);
    }

    #[test]
    fn completed_batches_rejects_corrupt_json_test() {
        let conn = memory_conn();
        insert_run(&conn, "run-bad", "knowledge", "running", "reference");
        for corrupt in ["[]", "{\"nope\":[]}", "{\"knowledge\":5}", "not-json"] {
            conn.execute(
                "UPDATE import_runs SET completed_batches_json = ? WHERE id = 'run-bad'",
                rusqlite::params![corrupt],
            )
            .unwrap();
            assert!(
                completed_batches(&read_run_row(&conn, "run-bad").unwrap().unwrap()).is_err(),
                "{corrupt} 应被判为损坏"
            );
        }
    }

    #[test]
    fn list_resumable_filters_and_orders_test() {
        let conn = memory_conn();
        insert_run(&conn, "run-a", "knowledge", "running", "reference");
        insert_run(&conn, "run-b", "knowledge", "completed", "reference");
        insert_run(&conn, "run-c", "parsing", "failed", "reference");
        conn.execute(
            "UPDATE import_runs SET resumable = 0 WHERE id = 'run-c'",
            [],
        )
        .unwrap();
        let runs = list_resumable(&conn).unwrap();
        let ids: Vec<&str> = runs.iter().map(|run| run.id.as_str()).collect();
        assert!(ids.contains(&"run-a"));
        assert!(!ids.contains(&"run-b"), "completed 不进入可恢复清单");
        assert!(!ids.contains(&"run-c"), "resumable=0 不进入可恢复清单");
    }

    #[test]
    fn list_chapter_batch_pages_and_clamps_limit_test() {
        let conn = memory_conn();
        insert_run(&conn, "run-page", "prepared", "ready", "reference");
        for number in 1..=5 {
            insert_chapter(&conn, "run-page", number, &format!("正文{number}"));
        }
        let first = list_chapter_batch(&conn, "run-page", 0, 2).unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].number, 1);
        assert_eq!(first[0].content, "正文1");
        let second = list_chapter_batch(&conn, "run-page", 2, 2).unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(second[0].number, 3);
        // limit 夹取：0 → 1、1000 → 100（此处仅验证下界与总数）
        assert_eq!(
            list_chapter_batch(&conn, "run-page", 4, 0).unwrap().len(),
            1
        );
        assert_eq!(
            list_chapter_batch(&conn, "run-page", 0, 1000)
                .unwrap()
                .len(),
            5
        );
    }

    #[test]
    fn frozen_chapter_snapshot_rejects_tampering_test() {
        let conn = memory_conn();
        insert_run(&conn, "run-tamper", "prepared", "ready", "reference");
        insert_chapter(&conn, "run-tamper", 1, "正文甲");
        // 篡改正文但保留原指纹
        conn.execute(
            "UPDATE import_run_chapters SET content_snapshot = '正文乙' WHERE run_id = 'run-tamper'",
            [],
        )
        .unwrap();
        let error = list_chapter_batch(&conn, "run-tamper", 0, 10).unwrap_err();
        assert_eq!(error, "导入冻结章节快照损坏，已拒绝继续");

        // 尺寸不符同样拒绝
        assert!(assert_frozen_chapter_snapshot(&sha256_hex("正文甲"), 99, "正文甲").is_err());
        assert!(assert_frozen_chapter_snapshot("短指纹", 9, "正文甲").is_err());
        assert!(assert_frozen_chapter_snapshot(&sha256_hex(""), 0, "").is_err());
        assert!(assert_frozen_chapter_snapshot(&sha256_hex("正文甲"), 9, "正文甲").is_ok());
    }

    #[test]
    fn get_returns_none_for_unknown_run_test() {
        let conn = memory_conn();
        assert!(get(&conn, "missing").unwrap().is_none());
    }

    // ==================== G2b：解析写入面 ====================

    fn chapter_input(number: i64, content: &str) -> ImportRunChapterInput {
        ImportRunChapterInput {
            number,
            source_index: Some(0),
            source_chapter_number: Some(number),
            title: format!("第{number}章"),
            content_fingerprint: sha256_hex(content),
            content_size: content.len() as i64,
            content: content.to_string(),
        }
    }

    fn begin_request(run_id: &str, source_id: &str) -> ImportRunBeginParsingRequest {
        ImportRunBeginParsingRequest {
            run_id: run_id.to_string(),
            purpose: ImportPurpose::Reference,
            source_fingerprint: "a".repeat(64),
            source_ids: Some(vec![source_id.to_string()]),
            source_fingerprints: Some(vec!["b".repeat(64)]),
            legacy_source_fingerprints: None,
            legacy_collection_fingerprint: None,
            source_display: vec![ImportSourceDisplayMetadata {
                display_name: "第1章.txt".to_string(),
                media_type: "text/plain".to_string(),
                size: 10,
            }],
            locale: ImportRunLocale::ZhCn,
        }
    }

    const SOURCE_ID: &str = "11111111-1111-4111-8111-111111111111";

    #[test]
    fn begin_parsing_creates_and_reuses_run_test() {
        let conn = memory_conn();
        let snapshot = begin_parsing(&conn, &begin_request("run-g2b", SOURCE_ID)).unwrap();
        assert_eq!(snapshot.id, "run-g2b");
        assert_eq!(snapshot.stage, "parsing");
        assert_eq!(snapshot.status, "ready");
        assert_eq!(snapshot.effect_namespace, "import:reference:run-g2b");
        assert_eq!(snapshot.total_sources, 1);
        assert_eq!(snapshot.source_display[0].display_name, "第1章.txt");
        assert_eq!(
            parsed_source_status(&conn, "run-g2b", SOURCE_ID)
                .unwrap()
                .as_deref(),
            Some("pending")
        );

        // 同指纹 + 不同 runId → 命中既有 parsing run（重建来源清单）
        let reused = begin_parsing(&conn, &begin_request("run-other", SOURCE_ID)).unwrap();
        assert_eq!(reused.id, "run-g2b");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM import_runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "不应新建第二个 run");
    }

    #[test]
    fn begin_parsing_rejects_invalid_identity_test() {
        let conn = memory_conn();
        let mut request = begin_request("run-bad", SOURCE_ID);
        request.source_fingerprint = "not-a-digest".to_string();
        assert_eq!(
            begin_parsing(&conn, &request).unwrap_err(),
            "导入运行身份无效"
        );

        // author-manuscript 不支持（走 prepare）
        let mut author = begin_request("run-author", SOURCE_ID);
        author.purpose = ImportPurpose::AuthorManuscript;
        assert_eq!(
            begin_parsing(&conn, &author).unwrap_err(),
            "当前版本不支持作者手稿导入"
        );

        // sourceId 非 UUID v4
        let mut bad_id = begin_request("run-bad-id", SOURCE_ID);
        bad_id.source_ids = Some(vec!["not-a-uuid".to_string()]);
        assert_eq!(
            begin_parsing(&conn, &bad_id).unwrap_err(),
            "导入来源身份无效或重复"
        );
    }

    #[test]
    fn manifest_canonical_json_matches_baseline_key_order_test() {
        let chapters = vec![NormalizedImportRunChapter {
            number: 1,
            source_index: 0,
            source_id: SOURCE_ID.to_string(),
            source_chapter_number: 3,
            title: "开端".to_string(),
            content_fingerprint: "c".repeat(64),
            content_size: 9,
            content: String::new(),
        }];
        let json = canonical_manifest(ImportPurpose::Reference, &chapters);
        // 键序 = 基线 JSON.stringify（purpose → chapters → number/sourceId/sourceChapterNumber/title/contentFingerprint/contentSize）
        assert_eq!(
            json,
            format!(
                "{{\"purpose\":\"reference\",\"chapters\":[{{\"number\":1,\"sourceId\":\"{SOURCE_ID}\",\"sourceChapterNumber\":3,\"title\":\"开端\",\"contentFingerprint\":\"{}\",\"contentSize\":9}}]}}",
                "c".repeat(64)
            )
        );
        assert_eq!(
            hash_manifest(ImportPurpose::Reference, &chapters),
            sha256_hex(&json)
        );
        // purpose 参与指纹
        assert_ne!(
            hash_manifest(ImportPurpose::AuthorManuscript, &chapters),
            hash_manifest(ImportPurpose::Reference, &chapters)
        );
    }

    #[test]
    fn commit_and_fail_parsed_source_roundtrip_test() {
        let conn = memory_conn();
        begin_parsing(&conn, &begin_request("run-commit", SOURCE_ID)).unwrap();

        // 指纹被篡改 → 拒绝
        let mut tampered = chapter_input(1, "正文甲");
        tampered.content_fingerprint = "0".repeat(64);
        assert_eq!(
            commit_parsed_source(&conn, "run-commit", SOURCE_ID, &[tampered]).unwrap_err(),
            "导入解析来源内容指纹与冻结快照不一致"
        );

        // 正常提交：来源 completed，run 汇总回填
        let chapters = vec![chapter_input(1, "正文甲"), chapter_input(2, "正文乙")];
        let snapshot = commit_parsed_source(&conn, "run-commit", SOURCE_ID, &chapters).unwrap();
        assert_eq!(snapshot.status, "ready");
        assert_eq!(snapshot.total_chapters, 2);
        assert_eq!(snapshot.total_content_size, 18);
        assert_eq!(
            parsed_source_status(&conn, "run-commit", SOURCE_ID)
                .unwrap()
                .as_deref(),
            Some("completed")
        );
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM import_run_source_chapters WHERE run_id = 'run-commit'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 2);

        // 幂等：同内容重授权 → 直接返回；内容变了 → 报冲突
        commit_parsed_source(&conn, "run-commit", SOURCE_ID, &chapters).unwrap();
        assert_eq!(
            commit_parsed_source(
                &conn,
                "run-commit",
                SOURCE_ID,
                &[chapter_input(1, "正文丙")]
            )
            .unwrap_err(),
            "已完成来源与本次重新授权内容不一致"
        );
    }

    #[test]
    fn fail_parsed_source_marks_run_and_source_test() {
        let conn = memory_conn();
        begin_parsing(&conn, &begin_request("run-fail", SOURCE_ID)).unwrap();
        let snapshot = fail_parsed_source(&conn, "run-fail", SOURCE_ID, "读取失败").unwrap();
        assert_eq!(snapshot.status, "failed");
        assert_eq!(snapshot.last_error, "读取失败");
        assert!(snapshot.resumable);
        assert_eq!(
            parsed_source_status(&conn, "run-fail", SOURCE_ID)
                .unwrap()
                .as_deref(),
            Some("failed")
        );
        // 空原因拒绝
        assert_eq!(
            fail_parsed_source(&conn, "run-fail", SOURCE_ID, "   ").unwrap_err(),
            "导入解析失败原因无效"
        );
    }

    #[test]
    fn normalize_chapters_rejects_bounds_and_duplicates_test() {
        let source_ids = vec![SOURCE_ID.to_string()];
        assert_eq!(
            normalize_chapters(&[], &source_ids).unwrap_err(),
            "导入章节清单无效"
        );
        // sourceIndex 越界
        let mut out_of_range = chapter_input(1, "正文");
        out_of_range.source_index = Some(3);
        assert_eq!(
            normalize_chapters(&[out_of_range], &source_ids).unwrap_err(),
            "导入章节归属无效"
        );
        // 同一来源内 (sourceId, sourceChapterNumber) 重复
        let mut first = chapter_input(1, "正文甲");
        let mut second = chapter_input(2, "正文乙");
        second.source_chapter_number = first.source_chapter_number;
        first.source_index = Some(0);
        second.source_index = Some(0);
        assert_eq!(
            normalize_chapters(&[first, second], &source_ids).unwrap_err(),
            "导入章节来源归属重复"
        );
        // 空正文
        let mut empty = chapter_input(1, "");
        empty.content_size = 0;
        empty.content_fingerprint = sha256_hex("");
        assert_eq!(
            normalize_chapters(&[empty], &source_ids).unwrap_err(),
            "导入章节 1 快照无效"
        );
    }
}
