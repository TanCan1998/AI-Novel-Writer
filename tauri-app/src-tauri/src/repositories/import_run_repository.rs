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

use std::collections::{BTreeMap, BTreeSet, HashSet};

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
// G2b-4：author 分支复用 E 批次已迁移的原稿导入预览
use crate::repositories::finalized_draft_import_repository as draft_import;
// G3a：effect receipt 提交的下游仓储（跨仓原子事务）
use crate::repositories::{
    blueprint_repository, import_global_facts_repository, project_core_repository,
};

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

// ==================== 批次 G2b-3：finalizeParsing 与四态分类 ====================

const IMPORT_PREPARATION_PREVIEW_LIMIT: usize = 8;

/// 对齐契约 `ImportChapterPreview & { targetStatus }`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparationChapterPreview {
    pub number: i64,
    pub title: String,
    pub word_count: i64,
    pub target_status: String,
}

/// 对齐契约 `ImportRunPreparationInspection`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunPreparationInspection {
    pub inspection_id: String,
    pub purpose: ImportPurpose,
    pub source_count: usize,
    pub source_display_names: Vec<String>,
    pub chapter_count: usize,
    pub total_words: i64,
    pub total_bytes: i64,
    pub preview: Vec<PreparationChapterPreview>,
    pub preview_remaining: usize,
}

/// 对齐契约 `ImportRunPreparationResult`（`classification` 为
/// `exact-duplicate` | `new` | `conflict` | `resumable`）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunPreparationResult {
    pub classification: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<ImportRunSnapshot>,
    pub new_chapter_numbers: Vec<i64>,
    pub conflict_chapter_numbers: Vec<i64>,
    pub duplicate_chapter_numbers: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inspection: Option<ImportRunPreparationInspection>,
}

/// 供 `createPreparationInspection` 的单章元数据
struct PreparationChapter {
    number: i64,
    title: String,
    word_count: i64,
    content_size: i64,
}

/// 已完成台账中的单章记录（`completedChapterManifest` 的值）
#[derive(Debug, Clone)]
struct CompletedChapterEntry {
    title: String,
    content_fingerprint: String,
    content_size: i64,
}

fn source_chapter_key(source_id: &str, source_chapter_number: i64) -> String {
    format!("{source_id}:{source_chapter_number}")
}

/// 平移自 `createPreparationInspection`（含 targetStatus 优先级：conflict > duplicate > new > duplicate）
fn create_preparation_inspection(
    inspection_id: &str,
    purpose: ImportPurpose,
    source_display: &[ImportSourceDisplayMetadata],
    chapters: &[PreparationChapter],
    new_numbers: &[i64],
    conflict_numbers: &[i64],
    duplicate_numbers: &[i64],
) -> ImportRunPreparationInspection {
    let conflicts: HashSet<i64> = conflict_numbers.iter().copied().collect();
    let duplicates: HashSet<i64> = duplicate_numbers.iter().copied().collect();
    let fresh: HashSet<i64> = new_numbers.iter().copied().collect();
    let preview: Vec<PreparationChapterPreview> = chapters
        .iter()
        .take(IMPORT_PREPARATION_PREVIEW_LIMIT)
        .map(|chapter| PreparationChapterPreview {
            number: chapter.number,
            title: chapter.title.clone(),
            word_count: chapter.word_count,
            target_status: if conflicts.contains(&chapter.number) {
                "conflict"
            } else if duplicates.contains(&chapter.number) {
                "duplicate"
            } else if fresh.contains(&chapter.number) {
                "new"
            } else {
                "duplicate"
            }
            .to_string(),
        })
        .collect();
    ImportRunPreparationInspection {
        inspection_id: inspection_id.to_string(),
        purpose,
        source_count: source_display.len(),
        source_display_names: source_display
            .iter()
            .map(|source| source.display_name.clone())
            .collect(),
        chapter_count: chapters.len(),
        total_words: chapters.iter().map(|chapter| chapter.word_count).sum(),
        total_bytes: chapters.iter().map(|chapter| chapter.content_size).sum(),
        preview_remaining: chapters.len().saturating_sub(preview.len()),
        preview,
    }
}

/// 平移自 `completedChapterManifest`（仅统计 `status = 'completed'` 的 run）
fn completed_chapter_manifest(
    conn: &Connection,
    purpose: ImportPurpose,
    source_ids: &[String],
) -> Result<std::collections::HashMap<String, CompletedChapterEntry>, String> {
    let mut entries = std::collections::HashMap::new();
    if source_ids.is_empty() {
        return Ok(entries);
    }
    let placeholders = vec!["?"; source_ids.len()].join(", ");
    let sql = format!(
        "SELECT source_map.chapter_number, chapters.source_id, chapters.source_chapter_number,
                chapters.title, chapters.content_fingerprint, chapters.content_size
         FROM import_run_chapters AS chapters
         JOIN import_runs AS runs ON runs.id = chapters.run_id
         JOIN import_source_chapter_map AS source_map
           ON source_map.purpose = runs.purpose
           AND source_map.source_id = chapters.source_id
           AND source_map.source_chapter_number = chapters.source_chapter_number
         WHERE runs.purpose = ? AND runs.status = 'completed'
           AND chapters.source_id IN ({placeholders})
         ORDER BY runs.completed_at ASC, runs.rowid ASC, chapters.chapter_number ASC"
    );
    let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let mut params: Vec<rusqlite::types::Value> = Vec::with_capacity(1 + source_ids.len());
    params.push(rusqlite::types::Value::Text(purpose.as_str().to_string()));
    for id in source_ids {
        params.push(rusqlite::types::Value::Text(id.clone()));
    }
    let rows = statement
        .query_map(rusqlite::params_from_iter(params), |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    // 后写入覆盖先写入：与基线 `new Map(rows.map(...))` 一致（后者胜）
    for (source_id, source_chapter_number, title, content_fingerprint, content_size) in rows {
        entries.insert(
            source_chapter_key(&source_id, source_chapter_number),
            CompletedChapterEntry {
                title,
                content_fingerprint,
                content_size,
            },
        );
    }
    Ok(entries)
}

/// 待写入 `import_source_chapter_map` 的新映射
#[derive(Debug, Clone)]
struct ChapterMapping {
    source_id: String,
    source_chapter_number: i64,
    chapter_number: i64,
}

/// 平移自 `assignStableChapterNumbers`：已有映射的章号不因本批而变化
fn assign_stable_chapter_numbers(
    conn: &Connection,
    purpose: ImportPurpose,
    chapters: &[NormalizedImportRunChapter],
) -> Result<(Vec<NormalizedImportRunChapter>, Vec<ChapterMapping>), String> {
    let mut next_chapter_number: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(chapter_number), 0) FROM import_source_chapter_map WHERE purpose = ?",
            rusqlite::params![purpose.as_str()],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT chapter_number FROM import_source_chapter_map
             WHERE purpose = ? AND source_id = ? AND source_chapter_number = ?",
        )
        .map_err(|error| error.to_string())?;
    let mut numbered = Vec::with_capacity(chapters.len());
    let mut new_mappings = Vec::new();
    for chapter in chapters {
        let existing: Option<i64> = statement
            .query_row(
                rusqlite::params![
                    purpose.as_str(),
                    chapter.source_id,
                    chapter.source_chapter_number
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let number = match existing {
            Some(value) => value,
            None => {
                next_chapter_number += 1;
                new_mappings.push(ChapterMapping {
                    source_id: chapter.source_id.clone(),
                    source_chapter_number: chapter.source_chapter_number,
                    chapter_number: next_chapter_number,
                });
                next_chapter_number
            }
        };
        numbered.push(NormalizedImportRunChapter {
            number,
            ..chapter.clone()
        });
    }
    numbered.sort_by_key(|chapter| chapter.number);
    Ok((numbered, new_mappings))
}

fn matching_resumable_run(
    conn: &Connection,
    purpose: ImportPurpose,
    source_fingerprint: &str,
    manifest_fingerprint: &str,
) -> Result<Option<ImportRunRow>, String> {
    conn.query_row(
        &format!(
            "SELECT {RUN_COLUMNS} FROM import_runs
             WHERE purpose = ? AND source_fingerprint = ? AND manifest_fingerprint = ?
               AND stage <> 'parsing' AND resumable = 1
               AND status IN ('ready', 'running', 'failed', 'cancelled')
             ORDER BY created_at DESC, rowid DESC LIMIT 1"
        ),
        rusqlite::params![purpose.as_str(), source_fingerprint, manifest_fingerprint],
        map_run_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn latest_completed_run(
    conn: &Connection,
    purpose: ImportPurpose,
    source_fingerprint: &str,
) -> Result<Option<ImportRunRow>, String> {
    conn.query_row(
        &format!(
            "SELECT {RUN_COLUMNS} FROM import_runs
             WHERE purpose = ? AND source_fingerprint = ? AND status = 'completed'
             ORDER BY completed_at DESC, rowid DESC LIMIT 1"
        ),
        rusqlite::params![purpose.as_str(), source_fingerprint],
        map_run_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn overlapping_resumable_source_run(
    conn: &Connection,
    run_id: &str,
    purpose: ImportPurpose,
) -> Result<Option<ImportRunRow>, String> {
    conn.query_row(
        &format!(
            "SELECT DISTINCT {}
             FROM import_runs AS other_runs
             JOIN import_run_sources AS other_sources ON other_sources.run_id = other_runs.id
             JOIN import_run_sources AS current_sources
               ON current_sources.run_id = ?
               AND current_sources.source_fingerprint = other_sources.source_fingerprint
             WHERE other_runs.id <> ?
               AND other_runs.purpose = ?
               AND other_runs.stage <> 'parsing'
               AND other_runs.resumable = 1
               AND other_runs.status IN ('ready', 'running', 'failed', 'cancelled')
             ORDER BY other_runs.updated_at DESC, other_runs.rowid DESC
             LIMIT 1",
            RUN_COLUMNS
                .split(", ")
                .map(|column| format!("other_runs.{column}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        rusqlite::params![run_id, run_id, purpose.as_str()],
        map_run_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

/// 平移自 `discardProvisionalParsingRun`（带 stage/status 守卫，与 `exact-duplicate` 的裸 DELETE 不同）
fn discard_provisional_parsing_run(conn: &Connection, run_id: &str) -> Result<(), String> {
    let changed = conn
        .execute(
            "DELETE FROM import_runs WHERE id = ? AND stage = 'parsing' AND status IN ('ready', 'failed')",
            rusqlite::params![run_id],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("导入解析临时运行无法安全终止".to_string());
    }
    Ok(())
}

/// 平移自 `finalizeParsing`：四态分类（`new` / `resumable` / `conflict` / `exact-duplicate`）。
///
/// **D3′**：基线在 `source.legacy_source_fingerprint` / `run.legacy_source_fingerprint` 非空时
/// 调用 `adoptLegacyCompletedRun` 做旧身份迁移；两栈项目目录不通、该字段恒为 `''`，
/// 故此路径在 Tauri 侧**结构不可达**，不移植。
pub fn finalize_parsing(
    conn: &Connection,
    run_id: &str,
) -> Result<ImportRunPreparationResult, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let Some(run) = read_run_row(&tx, run_id)? else {
        return Err("导入解析运行当前不可完成".to_string());
    };
    if run.stage != "parsing" || !matches!(run.status.as_str(), "ready" | "failed") {
        return Err("导入解析运行当前不可完成".to_string());
    }
    let purpose = ImportPurpose::parse_contract(&run.purpose)?;
    let sources = read_sources_in_order(&tx, run_id)?;
    if sources.is_empty() || sources.iter().any(|source| source.status != "completed") {
        return Err("导入来源解析尚未完成，请重新授权未完成来源".to_string());
    }

    // 来源原始章元数据（按 source_index, source_chapter_number）
    let mut statement = tx
        .prepare(
            "SELECT sources.source_index, chapters.source_id, chapters.source_chapter_number,
                    chapters.title, chapters.content_fingerprint, chapters.content_size, chapters.word_count
             FROM import_run_source_chapters AS chapters
             JOIN import_run_sources AS sources
               ON sources.run_id = chapters.run_id AND sources.source_id = chapters.source_id
             WHERE chapters.run_id = ?
             ORDER BY sources.source_index, chapters.source_chapter_number",
        )
        .map_err(|error| error.to_string())?;
    let metadata = statement
        .query_map(rusqlite::params![run_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);

    let total_source_chapters: i64 = sources.iter().map(|source| source.chapter_count).sum();
    let total_source_bytes: i64 = sources.iter().map(|source| source.content_size).sum();
    if metadata.is_empty()
        || metadata.len() > MAX_IMPORT_CHAPTERS
        || metadata.len() as i64 != total_source_chapters
        || total_source_bytes > MAX_IMPORT_TOTAL_BYTES as i64
    {
        return Err("导入解析 manifest 无效".to_string());
    }

    let normalized: Vec<NormalizedImportRunChapter> = metadata
        .iter()
        .enumerate()
        .map(|(index, entry)| NormalizedImportRunChapter {
            number: index as i64 + 1,
            source_index: entry.0,
            source_id: entry.1.clone(),
            source_chapter_number: entry.2,
            title: entry.3.clone(),
            content_fingerprint: entry.4.clone(),
            content_size: entry.5,
            content: String::new(),
        })
        .collect();

    let (chapters, new_mappings) = assign_stable_chapter_numbers(&tx, purpose, &normalized)?;
    let source_display: Vec<ImportSourceDisplayMetadata> = sources
        .iter()
        .map(|source| parse_json(&source.display_json, ImportSourceDisplayMetadata::default()))
        .collect();
    let word_counts: std::collections::HashMap<String, i64> = metadata
        .iter()
        .map(|entry| (source_chapter_key(&entry.1, entry.2), entry.6))
        .collect();
    let preview_chapters: Vec<PreparationChapter> = chapters
        .iter()
        .map(|chapter| PreparationChapter {
            number: chapter.number,
            title: chapter.title.clone(),
            word_count: word_counts
                .get(&source_chapter_key(
                    &chapter.source_id,
                    chapter.source_chapter_number,
                ))
                .copied()
                .unwrap_or(0),
            content_size: chapter.content_size,
        })
        .collect();
    let inspection_for =
        |new_numbers: &[i64], conflict_numbers: &[i64], duplicate_numbers: &[i64]| {
            create_preparation_inspection(
                run_id,
                purpose,
                &source_display,
                &preview_chapters,
                new_numbers,
                conflict_numbers,
                duplicate_numbers,
            )
        };

    let manifest_fingerprint = hash_manifest(purpose, &chapters);
    if let Some(resumable) =
        matching_resumable_run(&tx, purpose, &run.source_fingerprint, &manifest_fingerprint)?
    {
        if resumable.id != run_id {
            let duplicate: Vec<i64> = chapters.iter().map(|chapter| chapter.number).collect();
            discard_provisional_parsing_run(&tx, run_id)?;
            let inspection = inspection_for(&[], &[], &duplicate);
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(ImportRunPreparationResult {
                classification: "resumable".to_string(),
                run: Some(row_to_snapshot(conn, &resumable)?),
                new_chapter_numbers: Vec::new(),
                conflict_chapter_numbers: Vec::new(),
                duplicate_chapter_numbers: duplicate,
                inspection: Some(inspection),
            });
        }
    }
    if overlapping_resumable_source_run(&tx, run_id, purpose)?.is_some() {
        return Err(if run.locale == "en-US" {
            "Another resumable import already contains the same source. Complete or cancel that import, then try again."
        } else {
            "另一个可恢复导入已包含相同来源，请先完成或取消该导入后重试"
        }
        .to_string());
    }

    let source_ids: Vec<String> = sources
        .iter()
        .map(|source| source.source_id.clone())
        .collect();
    let completed = latest_completed_run(&tx, purpose, &run.source_fingerprint)?;
    let completed_manifest = completed_chapter_manifest(&tx, purpose, &source_ids)?;
    let previous_for = |chapter: &NormalizedImportRunChapter| {
        completed_manifest.get(&source_chapter_key(
            &chapter.source_id,
            chapter.source_chapter_number,
        ))
    };
    let conflict_numbers: Vec<i64> = chapters
        .iter()
        .filter(|chapter| {
            previous_for(chapter).is_some_and(|previous| {
                previous.title != chapter.title
                    || previous.content_fingerprint != chapter.content_fingerprint
                    || previous.content_size != chapter.content_size
            })
        })
        .map(|chapter| chapter.number)
        .collect();
    let duplicate_numbers: Vec<i64> = chapters
        .iter()
        .filter(|chapter| {
            previous_for(chapter).is_some_and(|previous| {
                previous.title == chapter.title
                    && previous.content_fingerprint == chapter.content_fingerprint
                    && previous.content_size == chapter.content_size
            })
        })
        .map(|chapter| chapter.number)
        .collect();
    let new_chapters: Vec<&NormalizedImportRunChapter> = chapters
        .iter()
        .filter(|chapter| previous_for(chapter).is_none())
        .collect();

    if !conflict_numbers.is_empty() {
        discard_provisional_parsing_run(&tx, run_id)?;
        let new_numbers: Vec<i64> = new_chapters.iter().map(|chapter| chapter.number).collect();
        let inspection = inspection_for(&new_numbers, &conflict_numbers, &duplicate_numbers);
        tx.commit().map_err(|error| error.to_string())?;
        return Ok(ImportRunPreparationResult {
            classification: "conflict".to_string(),
            run: None,
            new_chapter_numbers: new_numbers,
            conflict_chapter_numbers: conflict_numbers,
            duplicate_chapter_numbers: duplicate_numbers,
            inspection: Some(inspection),
        });
    }
    if new_chapters.is_empty() {
        // 与基线一致：此处为**裸 DELETE**（无 stage/status 守卫），不同于 discardProvisionalParsingRun
        tx.execute(
            "DELETE FROM import_runs WHERE id = ?",
            rusqlite::params![run_id],
        )
        .map_err(|error| error.to_string())?;
        let inspection = inspection_for(&[], &[], &duplicate_numbers);
        tx.commit().map_err(|error| error.to_string())?;
        return Ok(ImportRunPreparationResult {
            classification: "exact-duplicate".to_string(),
            run: None,
            new_chapter_numbers: Vec::new(),
            conflict_chapter_numbers: Vec::new(),
            duplicate_chapter_numbers: duplicate_numbers,
            inspection: Some(inspection),
        });
    }

    for mapping in &new_mappings {
        tx.execute(
            "INSERT INTO import_source_chapter_map (purpose, source_id, source_chapter_number, chapter_number)
             VALUES (?, ?, ?, ?)",
            rusqlite::params![
                purpose.as_str(),
                mapping.source_id,
                mapping.source_chapter_number,
                mapping.chapter_number
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    let mut total_new_bytes: i64 = 0;
    for chapter in &new_chapters {
        tx.execute(
            "INSERT INTO import_run_chapters (
                run_id, chapter_number, source_id, source_chapter_number,
                title, content_fingerprint, content_size, content_snapshot
             )
             SELECT ?, ?, source_id, source_chapter_number, title, content_fingerprint, content_size, content_snapshot
             FROM import_run_source_chapters
             WHERE run_id = ? AND source_id = ? AND source_chapter_number = ?",
            rusqlite::params![
                run_id,
                chapter.number,
                run_id,
                chapter.source_id,
                chapter.source_chapter_number
            ],
        )
        .map_err(|error| error.to_string())?;
        total_new_bytes += chapter.content_size;
    }
    let total_source_words: i64 = sources.iter().map(|source| source.word_count).sum();
    tx.execute(
        "UPDATE import_runs
         SET stage = 'prepared', status = 'ready', manifest_fingerprint = ?,
             total_chapters = ?, total_content_size = ?, manifest_chapter_count = ?,
             manifest_content_size = ?, manifest_word_count = ?, completed_chapters = 0,
             base_run_id = ?, last_error = '', updated_at = datetime('now')
         WHERE id = ? AND stage = 'parsing'",
        rusqlite::params![
            manifest_fingerprint,
            new_chapters.len() as i64,
            total_new_bytes,
            chapters.len() as i64,
            total_source_bytes,
            total_source_words,
            completed.as_ref().map(|row| row.id.clone()),
            run_id
        ],
    )
    .map_err(|error| error.to_string())?;
    let new_numbers: Vec<i64> = new_chapters.iter().map(|chapter| chapter.number).collect();
    let inspection = inspection_for(&new_numbers, &[], &duplicate_numbers);
    tx.commit().map_err(|error| error.to_string())?;
    let snapshot = get(conn, run_id)?.ok_or_else(|| "导入运行读取失败".to_string())?;
    Ok(ImportRunPreparationResult {
        classification: "new".to_string(),
        run: Some(snapshot),
        new_chapter_numbers: new_numbers,
        conflict_chapter_numbers: Vec::new(),
        duplicate_chapter_numbers: duplicate_numbers,
        inspection: Some(inspection),
    })
}

// ==================== 批次 G2b-4：prepare（author / reference 两分支） ====================

/// 对齐基线 `ImportRunPrepareRequest`（主进程内部构造，不跨 IPC）
#[derive(Debug, Clone)]
pub struct ImportRunPrepareRequest {
    pub run_id: String,
    pub purpose: ImportPurpose,
    pub source_fingerprint: String,
    pub source_ids: Option<Vec<String>>,
    pub source_fingerprints: Option<Vec<String>>,
    pub source_display: Vec<ImportSourceDisplayMetadata>,
    pub locale: ImportRunLocale,
    /// 作者原稿必填（来自只读项目预览）
    pub authority_fingerprint: Option<String>,
    /// 作者原稿必填（绑定已确认的章节清单）
    pub expected_manifest_fingerprint: Option<String>,
    pub chapters: Vec<ImportRunChapterInput>,
}

/// `prepare` 的错误：`AuthorPreviewStale` 需由命令层映射为
/// `{ success:false, errorCode:'AUTHOR_IMPORT_PREVIEW_STALE' }` 信封（D10）。
#[derive(Debug, Clone)]
pub enum PrepareError {
    /// 业务失败（文案即基线 `Error.message`）
    Message(String),
    /// 作者原稿预览已过期（契约 `AUTHOR_IMPORT_PREVIEW_STALE`）
    AuthorPreviewStale(String),
}

impl PrepareError {
    pub fn message(&self) -> &str {
        match self {
            Self::Message(message) | Self::AuthorPreviewStale(message) => message,
        }
    }
}

impl std::fmt::Display for PrepareError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl From<String> for PrepareError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// 平移自 `hasCommittedAuthorFinalizationReceipt`。
///
/// ⚠️ **G3 待补**：基线在返回前还会调 `rowToEffectReceipt(receipt, run)` 做收据结构/绑定校验
/// （损坏时抛错并拒绝继续）；本批只做**存在性判定**（`prepare` 实际只依赖该语义）。
fn has_committed_author_finalization_receipt(
    conn: &Connection,
    run: &ImportRunRow,
) -> Result<bool, String> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM import_run_receipts
             WHERE run_id = ? AND kind = 'author-finalized-batch' AND state = 'committed'
             ORDER BY updated_at DESC, rowid DESC LIMIT 1",
            rusqlite::params![run.id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(exists.is_some())
}

/// 平移自 `fenceUncommittedAuthorRun`：把无法继续的作者运行「栅栏化」（resumable=0 + 换代）。
fn fence_uncommitted_author_run(
    conn: &Connection,
    run: &ImportRunRow,
    now: i64,
) -> Result<(), String> {
    if run.status == "running" && !run.execution_owner.is_empty() && run.lease_expires_at > now {
        return Err(if run.locale == "en-US" {
            "The previous author import is still running. Wait for it to stop, then confirm the latest preview again."
        } else {
            "之前的作者原稿导入仍在运行，请等待其停止后重新确认最新预览"
        }
        .to_string());
    }
    let guidance = if run.locale == "en-US" {
        "Author manuscript authority changed. Confirm the latest preview to create a new import run."
    } else {
        "作者原稿权威状态已变化，请根据最新预览重新确认导入"
    };
    let changed = conn
        .execute(
            "UPDATE import_runs
             SET resumable = 0, cancel_requested = 0, last_error = ?,
                 execution_owner = '', execution_epoch = execution_epoch + 1, lease_expires_at = 0,
                 updated_at = datetime('now')
             WHERE id = ? AND purpose = 'author-manuscript' AND resumable = 1
               AND status IN ('ready', 'running', 'failed', 'cancelled')
               AND (status <> 'running' OR execution_owner = '' OR lease_expires_at <= ?)
               AND NOT EXISTS (
                 SELECT 1 FROM import_run_receipts
                 WHERE run_id = import_runs.id
                   AND kind = 'author-finalized-batch' AND state = 'committed'
               )",
            rusqlite::params![guidance, run.id, now],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err(if run.locale == "en-US" {
            "The author import state changed. Confirm the latest preview again."
        } else {
            "作者原稿导入状态已变化，请重新确认最新预览"
        }
        .to_string());
    }
    Ok(())
}

/// 作者原稿预览→运行创建（对齐 `prepare` 的 author 分支）。
/// 不返回 `inspection`——与基线一致（author 分支的分类信息全在 `preview` 里）。
fn prepare_author_manuscript(
    conn: &Connection,
    candidate: &ImportRunPrepareRequest,
    run_id: &str,
    source_display: &[ImportSourceDisplayMetadata],
    normalized_chapters: &[NormalizedImportRunChapter],
) -> Result<ImportRunPreparationResult, PrepareError> {
    let authority_fingerprint = candidate
        .authority_fingerprint
        .as_deref()
        .unwrap_or_default();
    let expected_manifest_fingerprint = candidate
        .expected_manifest_fingerprint
        .as_deref()
        .unwrap_or_default();
    if !is_sha256(authority_fingerprint) || !is_sha256(expected_manifest_fingerprint) {
        return Err(PrepareError::Message(
            "作者原稿缺少已确认的权威预览".to_string(),
        ));
    }

    let mut author_chapters: Vec<draft_import::FinalizedDraftImportChapter> = normalized_chapters
        .iter()
        .map(|chapter| draft_import::FinalizedDraftImportChapter {
            chapter_number: chapter.number,
            title: chapter.title.clone(),
            content: chapter.content.clone(),
            word_count: count_draft_units(&chapter.content),
        })
        .collect();
    author_chapters.sort_by_key(|chapter| chapter.chapter_number);

    let preview = draft_import::preview(conn, &author_chapters).map_err(PrepareError::Message)?;
    if preview.manifest_fingerprint != expected_manifest_fingerprint {
        return Err(PrepareError::Message(
            "作者原稿清单与已确认预览不一致".to_string(),
        ));
    }
    let resumable = matching_resumable_run(
        conn,
        candidate.purpose,
        &candidate.source_fingerprint,
        &preview.manifest_fingerprint,
    )
    .map_err(PrepareError::Message)?;
    if preview.authority_fingerprint != authority_fingerprint {
        return Err(PrepareError::AuthorPreviewStale(
            "项目权威章节已变化，作者原稿预览已过期".to_string(),
        ));
    }
    let duplicate_numbers: Vec<i64> = author_chapters
        .iter()
        .map(|chapter| chapter.chapter_number)
        .collect();
    if let Some(resumable) = resumable {
        let committed_receipt = has_committed_author_finalization_receipt(conn, &resumable)
            .map_err(PrepareError::Message)?;
        if committed_receipt || resumable.authority_fingerprint == preview.authority_fingerprint {
            return Ok(ImportRunPreparationResult {
                classification: "resumable".to_string(),
                run: Some(row_to_snapshot(conn, &resumable).map_err(PrepareError::Message)?),
                new_chapter_numbers: Vec::new(),
                conflict_chapter_numbers: Vec::new(),
                duplicate_chapter_numbers: duplicate_numbers,
                inspection: None,
            });
        }
        fence_uncommitted_author_run(conn, &resumable, now_ms()).map_err(PrepareError::Message)?;
    }

    if preview.classification == "conflict" {
        let message = if preview.authority_invalid {
            format!(
                "现有权威正文章节不连续；请先修复第 {} 章附近的数据",
                preview
                    .first_gap_chapter_number
                    .map(|number| number.to_string())
                    .unwrap_or_else(|| "?".to_string())
            )
        } else if !preview.conflict_chapter_numbers.is_empty() {
            format!(
                "作者原稿与现有正文冲突：第 {} 章",
                preview
                    .conflict_chapter_numbers
                    .iter()
                    .map(|number| number.to_string())
                    .collect::<Vec<_>>()
                    .join("、")
            )
        } else {
            format!(
                "作者原稿存在缺章；请从第 {} 章连续导入",
                preview
                    .first_gap_chapter_number
                    .map(|number| number.to_string())
                    .unwrap_or_else(|| "?".to_string())
            )
        };
        return Err(PrepareError::Message(message));
    }
    if preview.classification == "exact-duplicate" {
        return Ok(ImportRunPreparationResult {
            classification: "exact-duplicate".to_string(),
            run: None,
            new_chapter_numbers: Vec::new(),
            conflict_chapter_numbers: Vec::new(),
            duplicate_chapter_numbers: preview.duplicate_chapter_numbers.clone(),
            inspection: None,
        });
    }
    if read_run_row(conn, run_id)
        .map_err(PrepareError::Message)?
        .is_some()
    {
        return Err(PrepareError::Message("导入运行 ID 已存在".to_string()));
    }

    let new_chapter_set: HashSet<i64> = preview.new_chapter_numbers.iter().copied().collect();
    let mut chapters_to_persist: Vec<&NormalizedImportRunChapter> = normalized_chapters
        .iter()
        .filter(|chapter| new_chapter_set.contains(&chapter.number))
        .collect();
    chapters_to_persist.sort_by_key(|chapter| chapter.number);
    let manifest_content_size: i64 = normalized_chapters
        .iter()
        .map(|chapter| chapter.content_size)
        .sum();
    let manifest_word_count: i64 = normalized_chapters
        .iter()
        .map(|chapter| count_draft_units(&chapter.content))
        .sum();
    let persisted_content_size: i64 = chapters_to_persist
        .iter()
        .map(|chapter| chapter.content_size)
        .sum();

    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    tx.execute(
        "INSERT INTO import_runs (
            id, purpose, root_run_id, effect_namespace, source_fingerprint, manifest_fingerprint,
            authority_fingerprint, source_display_json, locale, stage, status,
            total_chapters, total_content_size, completed_chapters, base_run_id,
            manifest_chapter_count, manifest_content_size, manifest_word_count
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'author-commit', 'ready', ?, ?, 0, NULL, ?, ?, ?)",
        rusqlite::params![
            run_id,
            candidate.purpose.as_str(),
            run_id,
            format!("import:{}:{run_id}", candidate.purpose.as_str()),
            candidate.source_fingerprint,
            preview.manifest_fingerprint,
            authority_fingerprint,
            serde_json::to_string(source_display).unwrap_or_default(),
            candidate.locale.as_str(),
            chapters_to_persist.len() as i64,
            persisted_content_size,
            normalized_chapters.len() as i64,
            manifest_content_size,
            manifest_word_count,
        ],
    )
    .map_err(|error| error.to_string())?;
    for chapter in &chapters_to_persist {
        tx.execute(
            "INSERT INTO import_run_chapters (
                run_id, chapter_number, source_id, source_chapter_number,
                title, content_fingerprint, content_size, content_snapshot
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                run_id,
                chapter.number,
                chapter.source_id,
                chapter.source_chapter_number,
                chapter.title,
                chapter.content_fingerprint,
                chapter.content_size,
                chapter.content
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    tx.commit().map_err(|error| error.to_string())?;
    let Some(created) = read_run_row(conn, run_id).map_err(PrepareError::Message)? else {
        return Err(PrepareError::Message(
            "作者原稿导入运行创建失败".to_string(),
        ));
    };
    Ok(ImportRunPreparationResult {
        classification: "new".to_string(),
        run: Some(row_to_snapshot(conn, &created).map_err(PrepareError::Message)?),
        new_chapter_numbers: preview.new_chapter_numbers.clone(),
        conflict_chapter_numbers: Vec::new(),
        duplicate_chapter_numbers: preview.duplicate_chapter_numbers.clone(),
        inspection: None,
    })
}

/// 平移自 `prepare`（author 分支见 [`prepare_author_manuscript`]）。
///
/// **D3′**：基线的两处 `adoptLegacyCompletedRun` 在 Tauri 侧恒不可达（legacy 指纹恒空），不移植。
pub fn prepare(
    conn: &Connection,
    candidate: &ImportRunPrepareRequest,
) -> Result<ImportRunPreparationResult, PrepareError> {
    let run_id = candidate.run_id.trim();
    if run_id.is_empty()
        || run_id.encode_utf16().count() > 160
        || !is_sha256(&candidate.source_fingerprint)
    {
        return Err(PrepareError::Message("导入运行身份无效".to_string()));
    }
    let source_display = normalize_display(&candidate.source_display)?;
    let source_ids = normalize_source_ids(
        candidate.source_ids.as_deref(),
        &source_display,
        &candidate.source_fingerprint,
    )?;
    // 基线的 prepare **只校验、不比长度**（与 `begin_parsing` 不同）；返回值仅服务于 legacy 迁移（D3′）
    normalize_source_fingerprints(candidate.source_fingerprints.as_deref(), &source_ids)?;
    let normalized_chapters = normalize_chapters(&candidate.chapters, &source_ids)?;

    if candidate.purpose == ImportPurpose::AuthorManuscript {
        return prepare_author_manuscript(
            conn,
            candidate,
            run_id,
            &source_display,
            &normalized_chapters,
        );
    }

    // ===== reference 分支：直接写冻结章（不经 parsing 阶段）=====
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let (chapters, new_mappings) =
        assign_stable_chapter_numbers(&tx, candidate.purpose, &normalized_chapters)?;
    let manifest_fingerprint = hash_manifest(candidate.purpose, &chapters);
    let manifest_content_size: i64 = chapters.iter().map(|chapter| chapter.content_size).sum();
    let manifest_word_count: i64 = chapters
        .iter()
        .map(|chapter| count_draft_units(&chapter.content))
        .sum();
    let preview_chapters: Vec<PreparationChapter> = chapters
        .iter()
        .map(|chapter| PreparationChapter {
            number: chapter.number,
            title: chapter.title.clone(),
            word_count: count_draft_units(&chapter.content),
            content_size: chapter.content_size,
        })
        .collect();
    let inspection_for =
        |new_numbers: &[i64], conflict_numbers: &[i64], duplicate_numbers: &[i64]| {
            create_preparation_inspection(
                run_id,
                candidate.purpose,
                &source_display,
                &preview_chapters,
                new_numbers,
                conflict_numbers,
                duplicate_numbers,
            )
        };

    if let Some(resumable) = matching_resumable_run(
        &tx,
        candidate.purpose,
        &candidate.source_fingerprint,
        &manifest_fingerprint,
    )? {
        // 注意：与 `finalize_parsing` 不同，此处**不做** id 比对、也不丢弃本次运行
        let duplicate_numbers: Vec<i64> = chapters.iter().map(|chapter| chapter.number).collect();
        let inspection = inspection_for(&[], &[], &duplicate_numbers);
        tx.commit().map_err(|error| error.to_string())?;
        return Ok(ImportRunPreparationResult {
            classification: "resumable".to_string(),
            run: Some(row_to_snapshot(conn, &resumable)?),
            new_chapter_numbers: Vec::new(),
            conflict_chapter_numbers: Vec::new(),
            duplicate_chapter_numbers: duplicate_numbers,
            inspection: Some(inspection),
        });
    }

    let completed = latest_completed_run(&tx, candidate.purpose, &candidate.source_fingerprint)?;
    let completed_manifest = completed_chapter_manifest(&tx, candidate.purpose, &source_ids)?;
    let previous_for = |chapter: &NormalizedImportRunChapter| {
        completed_manifest.get(&source_chapter_key(
            &chapter.source_id,
            chapter.source_chapter_number,
        ))
    };
    let conflict_numbers: Vec<i64> = chapters
        .iter()
        .filter(|chapter| {
            previous_for(chapter).is_some_and(|previous| {
                previous.title != chapter.title
                    || previous.content_fingerprint != chapter.content_fingerprint
                    || previous.content_size != chapter.content_size
            })
        })
        .map(|chapter| chapter.number)
        .collect();
    let duplicate_numbers: Vec<i64> = chapters
        .iter()
        .filter(|chapter| {
            previous_for(chapter).is_some_and(|previous| {
                previous.title == chapter.title
                    && previous.content_fingerprint == chapter.content_fingerprint
                    && previous.content_size == chapter.content_size
            })
        })
        .map(|chapter| chapter.number)
        .collect();
    let new_chapters: Vec<&NormalizedImportRunChapter> = chapters
        .iter()
        .filter(|chapter| previous_for(chapter).is_none())
        .collect();

    if !conflict_numbers.is_empty() {
        let new_numbers: Vec<i64> = new_chapters.iter().map(|chapter| chapter.number).collect();
        let inspection = inspection_for(&new_numbers, &conflict_numbers, &duplicate_numbers);
        tx.commit().map_err(|error| error.to_string())?;
        return Ok(ImportRunPreparationResult {
            classification: "conflict".to_string(),
            run: None,
            new_chapter_numbers: new_numbers,
            conflict_chapter_numbers: conflict_numbers,
            duplicate_chapter_numbers: duplicate_numbers,
            inspection: Some(inspection),
        });
    }
    if new_chapters.is_empty() {
        let inspection = inspection_for(&[], &[], &duplicate_numbers);
        tx.commit().map_err(|error| error.to_string())?;
        return Ok(ImportRunPreparationResult {
            classification: "exact-duplicate".to_string(),
            run: None,
            new_chapter_numbers: Vec::new(),
            conflict_chapter_numbers: Vec::new(),
            duplicate_chapter_numbers: duplicate_numbers,
            inspection: Some(inspection),
        });
    }
    if read_run_row(&tx, run_id)?.is_some() {
        return Err(PrepareError::Message("导入运行 ID 已存在".to_string()));
    }

    let persisted_content_size: i64 = new_chapters
        .iter()
        .map(|chapter| chapter.content_size)
        .sum();
    for mapping in &new_mappings {
        tx.execute(
            "INSERT INTO import_source_chapter_map (purpose, source_id, source_chapter_number, chapter_number)
             VALUES (?, ?, ?, ?)",
            rusqlite::params![
                candidate.purpose.as_str(),
                mapping.source_id,
                mapping.source_chapter_number,
                mapping.chapter_number
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    tx.execute(
        "INSERT INTO import_runs (
            id, purpose, root_run_id, effect_namespace, source_fingerprint, manifest_fingerprint,
            source_display_json, locale, stage, status, total_chapters, total_content_size,
            completed_chapters, base_run_id, manifest_chapter_count, manifest_content_size,
            manifest_word_count
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'knowledge', 'ready', ?, ?, 0, ?, ?, ?, ?)",
        rusqlite::params![
            run_id,
            candidate.purpose.as_str(),
            run_id,
            format!("import:{}:{run_id}", candidate.purpose.as_str()),
            candidate.source_fingerprint,
            manifest_fingerprint,
            serde_json::to_string(&source_display).unwrap_or_default(),
            candidate.locale.as_str(),
            new_chapters.len() as i64,
            persisted_content_size,
            completed.as_ref().map(|row| row.id.clone()),
            chapters.len() as i64,
            manifest_content_size,
            manifest_word_count,
        ],
    )
    .map_err(|error| error.to_string())?;
    // 基线按 INSERT_BATCH_SIZE=50 分批复用同一条语句；本地 SQLite 无 RPC 成本，语义等价，故不分批
    for chapter in &new_chapters {
        tx.execute(
            "INSERT INTO import_run_chapters (
                run_id, chapter_number, source_id, source_chapter_number,
                title, content_fingerprint, content_size, content_snapshot
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                run_id,
                chapter.number,
                chapter.source_id,
                chapter.source_chapter_number,
                chapter.title,
                chapter.content_fingerprint,
                chapter.content_size,
                chapter.content
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    let new_numbers: Vec<i64> = new_chapters.iter().map(|chapter| chapter.number).collect();
    let inspection = inspection_for(&new_numbers, &[], &duplicate_numbers);
    tx.commit().map_err(|error| error.to_string())?;
    let Some(created) = read_run_row(conn, run_id)? else {
        return Err(PrepareError::Message("导入运行创建失败".to_string()));
    };
    Ok(ImportRunPreparationResult {
        classification: "new".to_string(),
        run: Some(row_to_snapshot(conn, &created)?),
        new_chapter_numbers: new_numbers,
        conflict_chapter_numbers: Vec::new(),
        duplicate_chapter_numbers: duplicate_numbers,
        inspection: Some(inspection),
    })
}

// ==================== 批次 G3：执行租约 / 批次推进 / effect receipts ====================

/// 默认执行租约时长（对齐 `DEFAULT_EXECUTION_LEASE_MS = 15 * 60_000`）
pub const DEFAULT_EXECUTION_LEASE_MS: i64 = 15 * 60_000;
/// effect receipt 载荷字节上限（对齐 `MAX_EFFECT_RECEIPT_PAYLOAD_BYTES`）
const MAX_EFFECT_RECEIPT_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
/// effect receipt schema 版本（对齐 `IMPORT_RUN_EFFECT_RECEIPT_SCHEMA_VERSION`）
const IMPORT_RUN_EFFECT_RECEIPT_SCHEMA_VERSION: i64 = 1;
/// 角色同步操作的基础键集（对齐 `BLUEPRINT_SYNC_BASE_KEYS`）
const BLUEPRINT_SYNC_BASE_KEYS: [&str; 9] = [
    "operationId",
    "blueprintCommitOperationId",
    "blueprintCommitPayloadHash",
    "status",
    "startChapter",
    "endChapter",
    "characterSyncInput",
    "createdAt",
    "updatedAt",
];

/// 对齐契约 `ImportRunExecutionAuthority`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunExecutionAuthority {
    pub owner: String,
    pub epoch: i64,
}

/// 对齐契约 `ImportRunExecutionLease`（`extends ImportRunExecutionAuthority`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunExecutionLease {
    pub owner: String,
    pub epoch: i64,
    pub expires_at: i64,
}

/// 对齐契约 `ImportRunStartResult`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunStartResult {
    pub run: ImportRunSnapshot,
    pub execution: ImportRunExecutionLease,
}

/// 对齐 `completeBatch` 返回（`{ newlyCompleted, cancelApplied, run }`）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunBatchCheckpointResult {
    pub newly_completed: bool,
    pub cancel_applied: bool,
    pub run: ImportRunSnapshot,
}

/// 对齐契约 `ImportRunEffectKind`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImportRunEffectKind {
    ProjectGlobalFacts,
    ProjectWritingStyle,
    ChapterBlueprintRange,
    AuthorFinalizedBatch,
}

impl ImportRunEffectKind {
    /// 持久化标识（kebab-case，与 `import_run_receipts.kind` 列取值一致）。
    /// 基线直接存字符串枚举值；Rust 侧 `serde_json::to_string` 会带 JSON
    /// 引号，故落库走 `as_str` 而非序列化（`parse_effect_kind` 按纯值反序列化）。
    pub fn as_str(self) -> &'static str {
        match self {
            ImportRunEffectKind::ProjectGlobalFacts => "project-global-facts",
            ImportRunEffectKind::ProjectWritingStyle => "project-writing-style",
            ImportRunEffectKind::ChapterBlueprintRange => "chapter-blueprint-range",
            ImportRunEffectKind::AuthorFinalizedBatch => "author-finalized-batch",
        }
    }
}

/// 对齐契约 `ImportRunEffectReceipt`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunEffectReceipt {
    pub schema_version: i64,
    pub run_id: String,
    pub effect_namespace: String,
    pub effect_key: String,
    pub stage: String,
    pub batch_id: String,
    pub kind: ImportRunEffectKind,
    pub payload_hash: String,
    pub state: String,
    pub payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect_receipt: Option<serde_json::Value>,
    pub created_at: String,
    pub updated_at: String,
}

/// 对齐契约 `ImportRunPrepareEffectReceiptRequest`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunPrepareEffectReceiptRequest {
    pub run_id: String,
    pub stage: String,
    pub batch_id: String,
    pub effect_key: String,
    pub kind: ImportRunEffectKind,
    pub payload: serde_json::Value,
}

/// 对齐契约 `ImportRunEffectCommitResult`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunEffectCommitResult {
    pub receipt: ImportRunEffectReceipt,
    pub run: ImportRunSnapshot,
    pub cancel_applied: bool,
}

/// `import_run_receipts` 行（对齐基线 `ImportRunEffectReceiptRow`）
struct ImportRunEffectReceiptRow {
    run_id: String,
    schema_version: i64,
    effect_namespace: String,
    effect_key: String,
    stage: String,
    batch_id: String,
    kind: String,
    payload_json: String,
    payload_hash: String,
    state: String,
    effect_receipt_json: Option<String>,
    created_at: String,
    updated_at: String,
}

fn read_effect_receipt_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImportRunEffectReceiptRow> {
    Ok(ImportRunEffectReceiptRow {
        run_id: row.get(0)?,
        schema_version: row.get(1)?,
        effect_namespace: row.get(2)?,
        effect_key: row.get(3)?,
        stage: row.get(4)?,
        batch_id: row.get(5)?,
        kind: row.get(6)?,
        payload_json: row.get(7)?,
        payload_hash: row.get(8)?,
        state: row.get(9)?,
        effect_receipt_json: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

/// 对齐 `corruptedReceipt`
fn corrupted_receipt() -> String {
    "导入 effect receipt 损坏，已拒绝重放".to_string()
}

/// 对齐 `assertExecutionAuthority`
fn assert_execution_authority(
    conn: &Connection,
    run_id: &str,
    authority: &ImportRunExecutionAuthority,
    now: i64,
) -> Result<ImportRunRow, String> {
    let row = read_run_row(conn, run_id)?
        .ok_or_else(|| "导入执行租约已失效，已拒绝旧执行器写入".to_string())?;
    if row.status != "running"
        || authority.owner.is_empty()
        || row.execution_owner != authority.owner
        || row.execution_epoch != authority.epoch
        || row.lease_expires_at <= now
    {
        return Err("导入执行租约已失效，已拒绝旧执行器写入".to_string());
    }
    Ok(row)
}

/// 对齐 `assertExecution`（租约三要素：身份 + epoch + 到期时刻）
fn assert_execution(
    conn: &Connection,
    run_id: &str,
    execution: &ImportRunExecutionLease,
    now: i64,
) -> Result<ImportRunRow, String> {
    let authority = ImportRunExecutionAuthority {
        owner: execution.owner.clone(),
        epoch: execution.epoch,
    };
    let row = assert_execution_authority(conn, run_id, &authority, now)?;
    if row.lease_expires_at != execution.expires_at {
        return Err("导入执行租约已失效，已拒绝旧执行器写入".to_string());
    }
    Ok(row)
}

/// 对齐契约 `ImportReferenceImportBinding`（`resolveReferenceImportAuthority`
/// 的返回值）：冻结章节的参照知识写入绑定事实。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReferenceImportBinding {
    pub chapter_number: i64,
    pub title: String,
    pub content: String,
    pub content_fingerprint: String,
    pub stable_key: String,
}

/// 对齐 `resolveReferenceImportAuthority`：解析参照知识写入授权。
/// 仅授权「当前导入冻结章节」的 KB 写入；非 reference 用途、非 knowledge
/// 阶段、取消请求已挂起、章节号无效或章节快照损坏时一律拒绝。
pub fn resolve_reference_import_authority(
    conn: &Connection,
    run_id: &str,
    authority: &ImportRunExecutionAuthority,
    chapter_number: i64,
    now: i64,
) -> Result<ImportReferenceImportBinding, String> {
    let run = assert_execution_authority(conn, run_id, authority, now)?;
    if run.purpose != "reference"
        || run.stage != "knowledge"
        || run.cancel_requested == 1
        || chapter_number < 1
    {
        return Err("参照知识写入未绑定当前导入的冻结章节".to_string());
    }
    let chapter = conn
        .query_row(
            "SELECT source_id, source_chapter_number, title, content_fingerprint,
                    content_size, content_snapshot
             FROM import_run_chapters
             WHERE run_id = ? AND chapter_number = ?",
            rusqlite::params![run_id, chapter_number],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "参照知识写入未绑定当前导入的冻结章节".to_string())?;
    let (
        source_id,
        source_chapter_number,
        title,
        content_fingerprint,
        content_size,
        content_snapshot,
    ) = chapter;
    assert_frozen_chapter_snapshot(&content_fingerprint, content_size, &content_snapshot)?;
    // 对齐基线 affiliationHash = sha256(JSON.stringify({ purpose, sourceId,
    // sourceChapterNumber, contentFingerprint }))。键序由 serde 映射序决定；
    // 哈希只在 Tauri 内部自洽（stableKey → documentId 派生），不要求与
    // Node 逐字节一致（G3 决策 D3）。
    let affiliation_payload = serde_json::json!({
        "purpose": run.purpose,
        "sourceId": source_id,
        "sourceChapterNumber": source_chapter_number,
        "contentFingerprint": content_fingerprint,
    });
    let affiliation_hash = sha256_hex(&affiliation_payload.to_string());
    Ok(ImportReferenceImportBinding {
        chapter_number,
        title,
        content: content_snapshot,
        content_fingerprint,
        stable_key: format!("reference:{affiliation_hash}"),
    })
}

/// 对齐 `commitReferenceImportReceipt`：参照知识文档按冻结章节完整提交后，
/// 登记知识库投影收据（`import_run_knowledge_receipts`）。幂等：同一绑定
/// 重复提交不产生新行；已绑定不同效果时拒绝（基线错误文案：不同效果）。
pub fn commit_reference_import_receipt(
    conn: &Connection,
    run_id: &str,
    authority: &ImportRunExecutionAuthority,
    chapter_number: i64,
    document_id: &str,
) -> Result<(), String> {
    if !is_sha256(document_id) {
        return Err("参照知识文档身份无效".to_string());
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let binding =
        resolve_reference_import_authority(&tx, run_id, authority, chapter_number, now_ms())?;
    let expected_document_id = sha256_hex(&format!("reference-import:{}", binding.stable_key));
    let committed_document = tx
        .query_row(
            "SELECT document_id, content_hash, state
             FROM import_reference_documents WHERE document_id = ?",
            rusqlite::params![expected_document_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if document_id != expected_document_id
        || committed_document.as_ref().map(|row| row.0.as_str())
            != Some(expected_document_id.as_str())
        || committed_document.as_ref().map(|row| row.1.as_str())
            != Some(binding.content_fingerprint.as_str())
        || committed_document.as_ref().map(|row| row.2.as_str()) != Some("committed")
    {
        return Err("参照知识文档尚未按冻结章节完整提交".to_string());
    }
    let (source_id, source_chapter_number) = tx
        .query_row(
            "SELECT source_id, source_chapter_number
             FROM import_run_chapters WHERE run_id = ? AND chapter_number = ?",
            rusqlite::params![run_id, chapter_number],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| error.to_string())?;
    let existing = tx
        .query_row(
            "SELECT document_id, purpose, source_id, source_chapter_number,
                    content_fingerprint, state
             FROM import_run_knowledge_receipts
             WHERE run_id = ? AND chapter_number = ?",
            rusqlite::params![run_id, chapter_number],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some((
        existing_document_id,
        existing_purpose,
        existing_source_id,
        existing_source_chapter_number,
        existing_content_fingerprint,
        existing_state,
    )) = existing
    {
        if existing_document_id != document_id
            || existing_purpose != "reference"
            || existing_source_id != source_id
            || existing_source_chapter_number != source_chapter_number
            || existing_content_fingerprint != binding.content_fingerprint
            || existing_state != "committed"
        {
            return Err("参照知识 receipt 已绑定不同效果".to_string());
        }
    } else {
        tx.execute(
            "INSERT INTO import_run_knowledge_receipts (
                run_id, chapter_number, purpose, source_id, source_chapter_number,
                content_fingerprint, document_id, state
             ) VALUES (?, ?, 'reference', ?, ?, ?, ?, 'committed')",
            rusqlite::params![
                run_id,
                chapter_number,
                source_id,
                source_chapter_number,
                binding.content_fingerprint,
                document_id,
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    tx.commit().map_err(|error| error.to_string())?;
    Ok(())
}

/// 对齐 `nextStageForRun`（`REFERENCE_NEXT_STAGE` / `AUTHOR_NEXT_STAGE`）
fn next_stage_for_run(purpose: &str, stage: &str) -> Option<&'static str> {
    match (purpose, stage) {
        ("reference", "prepared") => Some("knowledge"),
        ("reference", "knowledge") => Some("global"),
        ("reference", "global") => Some("style"),
        ("reference", "style") => Some("blueprints"),
        ("reference", "blueprints") => Some("refresh"),
        ("author-manuscript", "author-commit") => Some("author-publish"),
        ("author-manuscript", "author-publish") => Some("author-postprocess"),
        ("author-manuscript", "author-postprocess") => Some("refresh"),
        _ => None,
    }
}

/// 对齐 `authorCheckpointChapterNumber`（`AUTHOR_CHAPTER_CHECKPOINT = /^chapter:([1-9]\d*)$/`）
fn author_checkpoint_chapter_number(batch_id: &str) -> Result<i64, String> {
    let rest = batch_id.strip_prefix("chapter:").unwrap_or("");
    let well_formed = !rest.is_empty()
        && !rest.starts_with('0')
        && rest.bytes().all(|byte| byte.is_ascii_digit());
    let chapter_number: i64 = if well_formed {
        rest.parse().unwrap_or(0)
    } else {
        0
    };
    if chapter_number < 1 {
        return Err("原稿导入批次 ID 无效".to_string());
    }
    Ok(chapter_number)
}

/// 对齐 `assertCheckpointCanApply`（`source`：`'direct' | 'receipt'`）
fn assert_checkpoint_can_apply(
    conn: &Connection,
    row: &ImportRunRow,
    stage: &str,
    batch_id: &str,
    source: &str,
) -> Result<(), String> {
    if row.status != "running" || row.stage != stage {
        return Err("导入批次与当前阶段不匹配".to_string());
    }
    if stage == "parsing" || stage == "prepared" || stage == "completed" {
        return Err("导入批次 ID 无效".to_string());
    }
    let receipt_stage =
        stage == "global" || stage == "style" || stage == "blueprints" || stage == "author-commit";
    if (receipt_stage && source != "receipt") || (!receipt_stage && source != "direct") {
        return Err(if receipt_stage {
            "该导入阶段必须通过 receipt 提交"
        } else {
            "该导入阶段不接受 receipt 提交"
        }
        .to_string());
    }
    if stage == "global" || stage == "style" || stage == "author-commit" || stage == "refresh" {
        if batch_id != "done" {
            return Err("导入批次 ID 无效".to_string());
        }
        return Ok(());
    }
    if stage == "author-publish" || stage == "author-postprocess" {
        let chapter_number = author_checkpoint_chapter_number(batch_id)?;
        let existing = completed_batches(row)?;
        let stage_batches = existing.get(stage).map(Vec::as_slice).unwrap_or(&[]);
        if stage_batches.iter().any(|existing_batch_id| {
            existing_batch_id != batch_id
                && author_checkpoint_chapter_number(existing_batch_id)
                    .map(|number| number == chapter_number)
                    .unwrap_or(false)
        }) {
            return Err("原稿导入批次与已完成 checkpoint 重叠".to_string());
        }
        return Ok(());
    }
    if stage != "knowledge" && stage != "blueprints" {
        return Err("导入批次 ID 无效".to_string());
    }
    checkpoint_chapter_numbers(conn, row, stage, batch_id)?;
    let range = parse_import_run_chapter_batch_checkpoint_id(batch_id)
        .ok_or_else(|| "导入批次 ID 无效".to_string())?;
    let existing = completed_batches(row)?;
    let stage_batches = existing.get(stage).map(Vec::as_slice).unwrap_or(&[]);
    for existing_batch_id in stage_batches {
        if existing_batch_id == batch_id {
            continue;
        }
        let existing_range = parse_import_run_chapter_batch_checkpoint_id(existing_batch_id)
            .ok_or_else(|| "导入批次 ID 无效".to_string())?;
        if range.start_chapter <= existing_range.end_chapter
            && existing_range.start_chapter <= range.end_chapter
        {
            return Err("导入批次与已完成 checkpoint 重叠".to_string());
        }
    }
    Ok(())
}

/// 对齐 `assertStageCheckpointComplete`
fn assert_stage_checkpoint_complete(
    conn: &Connection,
    row: &ImportRunRow,
    stage: &str,
) -> Result<(), String> {
    let batches = completed_batches(row)?;
    let stage_batches = batches.get(stage).map(Vec::as_slice).unwrap_or(&[]);
    if stage == "global" || stage == "style" || stage == "author-commit" || stage == "refresh" {
        if stage_batches.len() != 1 || stage_batches[0] != "done" {
            return Err("导入阶段 checkpoint 未完成".to_string());
        }
        return Ok(());
    }
    let all_chapters = chapter_rows_for_range(conn, &row.id, 1, i64::MAX)?;
    if stage == "author-publish" || stage == "author-postprocess" {
        let covered: BTreeSet<i64> = stage_batches
            .iter()
            .map(|batch_id| author_checkpoint_chapter_number(batch_id))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if covered.len() != all_chapters.len()
            || all_chapters
                .iter()
                .any(|(chapter_number, _)| !covered.contains(chapter_number))
        {
            return Err("原稿导入阶段 checkpoint 未完成".to_string());
        }
        return Ok(());
    }
    if stage != "knowledge" && stage != "blueprints" {
        return Err("导入阶段 checkpoint 未完成".to_string());
    }
    let mut covered: BTreeSet<i64> = BTreeSet::new();
    for batch_id in stage_batches {
        for chapter_number in checkpoint_chapter_numbers(conn, row, stage, batch_id)? {
            if !covered.insert(chapter_number) {
                return Err("导入阶段 checkpoint 重叠".to_string());
            }
        }
    }
    if covered.len() != all_chapters.len()
        || all_chapters
            .iter()
            .any(|(chapter_number, _)| !covered.contains(chapter_number))
    {
        return Err("导入阶段 checkpoint 未完成".to_string());
    }
    Ok(())
}

/// 对齐 `applyBatchCheckpoint`：返回 `(newlyCompleted, cancelApplied)`。
/// 取消请求在批次落地时一并生效（基线把 `cancel_requested` 的副作用
/// 折叠进本函数——阶段推进族共享此语义）。
fn apply_batch_checkpoint(
    conn: &Connection,
    row: &ImportRunRow,
    stage: &str,
    batch_id: &str,
    source: &str,
) -> Result<(bool, bool), String> {
    assert_checkpoint_can_apply(conn, row, stage, batch_id, source)?;
    let mut completed = completed_batches(row)?;
    let stage_batches = completed.get(stage).cloned().unwrap_or_default();
    let newly_completed = !stage_batches.contains(&batch_id.to_string());
    if newly_completed {
        let mut updated = stage_batches;
        updated.push(batch_id.to_string());
        completed.insert(stage.to_string(), updated);
    }
    let cancel_applied = row.cancel_requested == 1;
    let cancel_flag = i64::from(cancel_applied);
    conn.execute(
        "UPDATE import_runs
         SET completed_batches_json = ?1,
             status = CASE WHEN ?2 = 1 THEN 'cancelled' ELSE status END,
             resumable = 1,
             execution_owner = CASE WHEN ?2 = 1 THEN '' ELSE execution_owner END,
             execution_epoch = execution_epoch + CASE WHEN ?2 = 1 THEN 1 ELSE 0 END,
             lease_expires_at = CASE WHEN ?2 = 1 THEN 0 ELSE lease_expires_at END,
             updated_at = datetime('now')
         WHERE id = ?3",
        rusqlite::params![
            serde_json::to_string(&completed).map_err(|error| error.to_string())?,
            cancel_flag,
            row.id,
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok((newly_completed, cancel_applied))
}

// ---------- effect receipts：规范化与载荷校验 ----------

/// 对齐 `canonicalize`（键按字节序排序；JSON 无 `undefined`，故不过滤 null）
fn canonicalize_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonicalize_value).collect())
        }
        serde_json::Value::Object(map) => {
            let mut entries: Vec<(&String, &serde_json::Value)> = map.iter().collect();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            let sorted = serde_json::Map::from_iter(
                entries
                    .into_iter()
                    .map(|(key, entry)| (key.clone(), canonicalize_value(entry))),
            );
            serde_json::Value::Object(sorted)
        }
        other => other.clone(),
    }
}

/// 对齐 `canonicalEqual`
fn canonical_equal(left: &serde_json::Value, right: &serde_json::Value) -> bool {
    canonicalize_value(left) == canonicalize_value(right)
}

/// 对齐 `canonicalPayload`：返回 `(json, hash)`
fn canonical_payload(payload: &serde_json::Value) -> Result<(String, String), String> {
    if !payload.is_object() {
        return Err("导入 effect receipt 载荷无效".to_string());
    }
    let json = serde_json::to_string(&canonicalize_value(payload))
        .map_err(|error| format!("effect receipt 载荷规范化失败：{error}"))?;
    if json.len() > MAX_EFFECT_RECEIPT_PAYLOAD_BYTES {
        return Err("导入 effect receipt 载荷超过安全上限".to_string());
    }
    let hash = sha256_hex(&json);
    Ok((json, hash))
}

/// 对齐 `exactKeys`
fn exact_keys(map: &serde_json::Map<String, serde_json::Value>, expected: &[&str]) -> bool {
    let mut actual: Vec<&str> = map.keys().map(|key| key.as_str()).collect();
    let mut expected_sorted = expected.to_vec();
    actual.sort_unstable();
    expected_sorted.sort_unstable();
    actual == expected_sorted
}

fn is_text(value: Option<&serde_json::Value>) -> bool {
    matches!(value, Some(serde_json::Value::String(_)))
}

fn is_sha256_value(value: Option<&serde_json::Value>) -> bool {
    matches!(value, Some(serde_json::Value::String(text)) if is_sha256(text))
}

fn is_integer_value(value: Option<&serde_json::Value>) -> bool {
    value.and_then(|entry| entry.as_i64()).is_some()
}

/// 对齐 `assertEffectPayloadSchema`（基线抛空消息错误，由读回路径统一转「损坏」）
fn assert_effect_payload_schema(
    kind: ImportRunEffectKind,
    payload: &serde_json::Value,
) -> Result<(), String> {
    let Some(map) = payload.as_object() else {
        return Err(String::new());
    };
    match kind {
        ImportRunEffectKind::AuthorFinalizedBatch => {
            if !exact_keys(
                map,
                &[
                    "operationId",
                    "runId",
                    "authorityFingerprint",
                    "manifestFingerprint",
                ],
            ) || !is_text(map.get("operationId"))
                || !is_text(map.get("runId"))
                || !is_sha256_value(map.get("authorityFingerprint"))
                || !is_sha256_value(map.get("manifestFingerprint"))
            {
                return Err(String::new());
            }
            Ok(())
        }
        ImportRunEffectKind::ProjectWritingStyle => {
            let writing_style = map.get("writingStyle");
            if !exact_keys(map, &["writingStyle"])
                || !is_text(writing_style)
                || writing_style
                    .and_then(|value| value.as_str())
                    .map(|text| text.trim().is_empty())
                    .unwrap_or(true)
            {
                return Err(String::new());
            }
            Ok(())
        }
        ImportRunEffectKind::ProjectGlobalFacts => {
            let character_entries = map.get("characterEntries");
            if !exact_keys(
                map,
                &[
                    "operationId",
                    "expectedRosterRevision",
                    "core",
                    "characterEntries",
                ],
            ) || !is_text(map.get("operationId"))
                || !is_integer_value(map.get("expectedRosterRevision"))
                || !map.get("core").is_some_and(|value| value.is_object())
                || !matches!(character_entries, Some(serde_json::Value::Array(entries)) if !entries.is_empty())
            {
                return Err(String::new());
            }
            Ok(())
        }
        ImportRunEffectKind::ChapterBlueprintRange => {
            let blueprints = map.get("blueprints");
            if !exact_keys(
                map,
                &[
                    "mode",
                    "operationId",
                    "startChapter",
                    "endChapter",
                    "blueprints",
                ],
            ) || map.get("mode").and_then(|value| value.as_str()) != Some("replace-range")
                || !is_text(map.get("operationId"))
                || !is_integer_value(map.get("startChapter"))
                || !is_integer_value(map.get("endChapter"))
                || !matches!(blueprints, Some(serde_json::Value::Array(items)) if !items.is_empty())
            {
                return Err(String::new());
            }
            Ok(())
        }
    }
}

/// 对齐 `assertEffectPayloadBinding`
fn assert_effect_payload_binding(
    kind: ImportRunEffectKind,
    run_id: &str,
    batch_id: &str,
    payload: &serde_json::Value,
) -> Result<(), String> {
    match kind {
        ImportRunEffectKind::AuthorFinalizedBatch => {
            if batch_id != "done" {
                return Err(String::new());
            }
            Ok(())
        }
        ImportRunEffectKind::ProjectGlobalFacts => {
            if payload.get("operationId").and_then(|value| value.as_str())
                != Some(format!("novel-import-global-{run_id}").as_str())
            {
                return Err(String::new());
            }
            Ok(())
        }
        ImportRunEffectKind::ChapterBlueprintRange => {
            let Some(checkpoint) = parse_import_run_chapter_batch_checkpoint_id(batch_id) else {
                return Err(String::new());
            };
            let expected_operation_id = format!(
                "import-blueprints-{run_id}-{}-{}",
                checkpoint.start_chapter, checkpoint.end_chapter
            );
            if payload.get("startChapter").and_then(|value| value.as_i64())
                != Some(checkpoint.start_chapter)
                || payload.get("endChapter").and_then(|value| value.as_i64())
                    != Some(checkpoint.end_chapter)
                || payload.get("operationId").and_then(|value| value.as_str())
                    != Some(expected_operation_id.as_str())
            {
                return Err(String::new());
            }
            Ok(())
        }
        ImportRunEffectKind::ProjectWritingStyle => Ok(()),
    }
}

/// 对齐 `assertAuthorEffectRunBinding`
fn assert_author_effect_run_binding(
    run: &ImportRunRow,
    kind: ImportRunEffectKind,
    payload: &serde_json::Value,
    allow_committed_later_stage: bool,
) -> Result<(), String> {
    if kind != ImportRunEffectKind::AuthorFinalizedBatch {
        return Ok(());
    }
    let stage_matches = run.stage == "author-commit"
        || (allow_committed_later_stage
            && matches!(
                run.stage.as_str(),
                "author-publish" | "author-postprocess" | "refresh" | "completed"
            ));
    let Some(map) = payload.as_object() else {
        return Err("作者原稿提交 receipt 未绑定冻结导入运行".to_string());
    };
    if run.purpose != "author-manuscript"
        || !stage_matches
        || map.get("runId").and_then(|value| value.as_str()) != Some(run.id.as_str())
        || map.get("operationId").and_then(|value| value.as_str())
            != Some(format!("author-import:{}", run.id).as_str())
        || map
            .get("authorityFingerprint")
            .and_then(|value| value.as_str())
            != Some(run.authority_fingerprint.as_str())
        || map
            .get("manifestFingerprint")
            .and_then(|value| value.as_str())
            != Some(run.manifest_fingerprint.as_str())
    {
        return Err("作者原稿提交 receipt 未绑定冻结导入运行".to_string());
    }
    Ok(())
}

/// 对齐 `assertCommittedEffectSchema`
fn assert_committed_effect_schema(
    kind: ImportRunEffectKind,
    payload: &serde_json::Value,
    effect_receipt: &serde_json::Value,
) -> Result<(), String> {
    let Some(payload_map) = payload.as_object() else {
        return Err(String::new());
    };
    let Some(receipt_map) = effect_receipt.as_object() else {
        return Err(String::new());
    };
    match kind {
        ImportRunEffectKind::AuthorFinalizedBatch => {
            let chapter_numbers_len = receipt_map
                .get("chapterNumbers")
                .and_then(|value| value.as_array())
                .map(|items| items.len());
            let drafts_len = receipt_map
                .get("drafts")
                .and_then(|value| value.as_array())
                .map(|items| items.len());
            if receipt_map.get("operationId") != payload_map.get("operationId")
                || !is_text(receipt_map.get("payloadHash"))
                || !matches!(
                    receipt_map.get("chapterNumbers"),
                    Some(serde_json::Value::Array(_))
                )
                || !matches!(receipt_map.get("drafts"), Some(serde_json::Value::Array(_)))
                || chapter_numbers_len != drafts_len
            {
                return Err(String::new());
            }
            Ok(())
        }
        ImportRunEffectKind::ProjectWritingStyle => {
            if !exact_keys(receipt_map, &["writingStyle"])
                || receipt_map.get("writingStyle") != payload_map.get("writingStyle")
            {
                return Err(String::new());
            }
            Ok(())
        }
        ImportRunEffectKind::ProjectGlobalFacts => {
            if !exact_keys(
                receipt_map,
                &["operationId", "payloadHash", "idempotent", "core", "roster"],
            ) || receipt_map.get("operationId") != payload_map.get("operationId")
                || !is_text(receipt_map.get("payloadHash"))
                || receipt_map
                    .get("idempotent")
                    .and_then(|value| value.as_bool())
                    .is_none()
                || !receipt_map
                    .get("core")
                    .is_some_and(|value| value.is_object())
                || !receipt_map
                    .get("roster")
                    .is_some_and(|value| value.is_object())
            {
                return Err(String::new());
            }
            Ok(())
        }
        ImportRunEffectKind::ChapterBlueprintRange => {
            if !exact_keys(
                receipt_map,
                &[
                    "mode",
                    "operationId",
                    "payloadHash",
                    "idempotent",
                    "startChapter",
                    "endChapter",
                    "chapterNumbers",
                    "snapshot",
                    "characterSyncInput",
                    "characterSyncOperation",
                ],
            ) || receipt_map.get("operationId") != payload_map.get("operationId")
                || receipt_map.get("mode") != payload_map.get("mode")
                || receipt_map.get("startChapter") != payload_map.get("startChapter")
                || receipt_map.get("endChapter") != payload_map.get("endChapter")
                || !is_text(receipt_map.get("payloadHash"))
                || receipt_map
                    .get("idempotent")
                    .and_then(|value| value.as_bool())
                    .is_none()
                || !matches!(
                    receipt_map.get("chapterNumbers"),
                    Some(serde_json::Value::Array(_))
                )
                || !matches!(
                    receipt_map.get("snapshot"),
                    Some(serde_json::Value::Array(_))
                )
                || !matches!(
                    receipt_map.get("characterSyncInput"),
                    Some(serde_json::Value::Array(_))
                )
                || !receipt_map
                    .get("characterSyncOperation")
                    .is_some_and(|value| value.is_object())
            {
                return Err(String::new());
            }
            Ok(())
        }
    }
}

/// 对齐 `assertCompletedBlueprintSyncOperation`
fn assert_completed_blueprint_sync_operation(
    operation: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    if operation.get("status").and_then(|value| value.as_str()) != Some("completed")
        || !is_text(operation.get("completedAt"))
        || operation
            .get("completedAt")
            .and_then(|value| value.as_str())
            .map(|text| text.is_empty())
            .unwrap_or(true)
        || !operation
            .get("completionReceipt")
            .is_some_and(|value| value.is_object())
    {
        return Err(String::new());
    }
    let completion = operation
        .get("completionReceipt")
        .and_then(|value| value.as_object())
        .ok_or_else(String::new)?;
    if completion.get("operationId") != operation.get("operationId")
        || completion.get("blueprintCommitOperationId")
            != operation.get("blueprintCommitOperationId")
    {
        return Err(String::new());
    }
    match completion.get("status").and_then(|value| value.as_str()) {
        Some("already-satisfied") => {
            if !exact_keys(
                completion,
                &["blueprintCommitOperationId", "operationId", "status"],
            ) {
                return Err(String::new());
            }
            Ok(())
        }
        Some("committed") => {
            if !exact_keys(
                completion,
                &[
                    "blueprintCommitOperationId",
                    "operationId",
                    "status",
                    "rosterReceipt",
                ],
            ) || !completion
                .get("rosterReceipt")
                .is_some_and(|value| value.is_object())
            {
                return Err(String::new());
            }
            let roster_receipt = completion
                .get("rosterReceipt")
                .and_then(|value| value.as_object())
                .ok_or_else(String::new)?;
            if !exact_keys(
                roster_receipt,
                &["operationId", "payloadHash", "revision", "idempotent"],
            ) || roster_receipt.get("operationId") != operation.get("operationId")
                || !is_sha256_value(roster_receipt.get("payloadHash"))
                || !is_integer_value(roster_receipt.get("revision"))
                || roster_receipt
                    .get("revision")
                    .and_then(|value| value.as_i64())
                    .map(|revision| revision < 1)
                    .unwrap_or(true)
                || roster_receipt
                    .get("idempotent")
                    .and_then(|value| value.as_bool())
                    .is_none()
            {
                return Err(String::new());
            }
            Ok(())
        }
        _ => Err(String::new()),
    }
}

/// `{ ...source, idempotent: false }` 且删除 `characterSyncOperation`
/// （对齐基线 `storedOuter` / `authoritativeOuter`）
fn outer_without_sync(source: &serde_json::Value) -> serde_json::Value {
    let mut map = source.as_object().cloned().unwrap_or_default();
    map.insert("idempotent".to_string(), serde_json::Value::Bool(false));
    map.remove("characterSyncOperation");
    serde_json::Value::Object(map)
}

/// 对齐 `syncBinding`（7 键投影）
fn sync_binding(operation: &serde_json::Value) -> serde_json::Value {
    let source = operation.as_object().cloned().unwrap_or_default();
    let keys = [
        "operationId",
        "blueprintCommitOperationId",
        "blueprintCommitPayloadHash",
        "startChapter",
        "endChapter",
        "characterSyncInput",
        "createdAt",
    ];
    let mut map = serde_json::Map::new();
    for key in keys {
        if let Some(value) = source.get(key) {
            map.insert(key.to_string(), value.clone());
        }
    }
    serde_json::Value::Object(map)
}

/// 对齐基线 `storedKeys` / `authoritativeKeys` 的动态键集
fn blueprint_sync_keys(operation: &serde_json::Value) -> Vec<&'static str> {
    let completed = operation.get("status").and_then(|value| value.as_str()) == Some("completed");
    if completed {
        let mut keys = BLUEPRINT_SYNC_BASE_KEYS.to_vec();
        keys.push("completionReceipt");
        keys.push("completedAt");
        keys
    } else {
        BLUEPRINT_SYNC_BASE_KEYS.to_vec()
    }
}

/// 对齐 `assertBlueprintEffectAuthority`
fn assert_blueprint_effect_authority(
    stored: &serde_json::Value,
    authoritative: &serde_json::Value,
) -> Result<(), String> {
    let stored_sync = stored
        .get("characterSyncOperation")
        .ok_or_else(String::new)?;
    let authoritative_sync = authoritative
        .get("characterSyncOperation")
        .ok_or_else(String::new)?;
    if !canonical_equal(
        &outer_without_sync(stored),
        &outer_without_sync(authoritative),
    ) {
        return Err(String::new());
    }
    if !canonical_equal(
        &sync_binding(stored_sync),
        &sync_binding(authoritative_sync),
    ) {
        return Err(String::new());
    }
    let stored_sync_map = stored_sync.as_object().ok_or_else(String::new)?;
    let authoritative_sync_map = authoritative_sync.as_object().ok_or_else(String::new)?;
    if !exact_keys(stored_sync_map, &blueprint_sync_keys(stored_sync))
        || !exact_keys(
            authoritative_sync_map,
            &blueprint_sync_keys(authoritative_sync),
        )
    {
        return Err(String::new());
    }
    let stored_status = stored_sync_map
        .get("status")
        .and_then(|value| value.as_str());
    let authoritative_status = authoritative_sync_map
        .get("status")
        .and_then(|value| value.as_str());
    if stored_status == authoritative_status {
        if !canonical_equal(stored_sync, authoritative_sync) {
            return Err(String::new());
        }
        return Ok(());
    }
    if stored_status != Some("pending") || authoritative_status != Some("completed") {
        return Err(String::new());
    }
    if stored_sync_map.get("updatedAt") != authoritative_sync_map.get("createdAt") {
        return Err(String::new());
    }
    assert_completed_blueprint_sync_operation(authoritative_sync_map)
}

/// 对齐 `assertCommittedEffectAuthority`
fn assert_committed_effect_authority(
    conn: &Connection,
    kind: ImportRunEffectKind,
    payload: &serde_json::Value,
    effect_receipt: &serde_json::Value,
) -> Result<(), String> {
    if kind == ImportRunEffectKind::ProjectWritingStyle {
        return Ok(());
    }
    match kind {
        ImportRunEffectKind::AuthorFinalizedBatch => {
            let operation_id = payload
                .get("operationId")
                .and_then(|value| value.as_str())
                .ok_or_else(String::new)?;
            let run_id = payload
                .get("runId")
                .and_then(|value| value.as_str())
                .ok_or_else(String::new)?;
            let chapters: Vec<draft_import::FinalizedDraftImportChapter> =
                read_all_chapter_rows(conn, run_id)?
                    .iter()
                    .map(chapter_row_to_snapshot)
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .map(|chapter| {
                        let word_count = count_draft_units(&chapter.content) as i64;
                        draft_import::FinalizedDraftImportChapter {
                            chapter_number: chapter.number,
                            title: chapter.title,
                            content: chapter.content,
                            word_count,
                        }
                    })
                    .collect();
            let Some(authoritative) =
                draft_import::get_committed_operation(conn, operation_id, &chapters)?
            else {
                return Err(String::new());
            };
            let authoritative_json =
                serde_json::to_value(&authoritative).map_err(|_| String::new())?;
            if !canonical_equal(&authoritative_json, effect_receipt) {
                return Err(String::new());
            }
            Ok(())
        }
        ImportRunEffectKind::ChapterBlueprintRange => {
            let operation_id = payload
                .get("operationId")
                .and_then(|value| value.as_str())
                .ok_or_else(String::new)?;
            let Some(authoritative) =
                blueprint_repository::get_committed_range_operation(conn, operation_id)?
            else {
                return Err(String::new());
            };
            let authoritative_json =
                serde_json::to_value(&authoritative).map_err(|_| String::new())?;
            assert_blueprint_effect_authority(effect_receipt, &authoritative_json)
        }
        ImportRunEffectKind::ProjectGlobalFacts => {
            let operation_id = payload
                .get("operationId")
                .and_then(|value| value.as_str())
                .ok_or_else(String::new)?;
            let Some(authoritative) =
                import_global_facts_repository::ImportGlobalFactsRepository::get_committed_operation(
                    conn,
                    operation_id,
                )?
            else {
                return Err(String::new());
            };
            let authoritative_json =
                serde_json::to_value(&authoritative).map_err(|_| String::new())?;
            let stored = canonicalize_value(&outer_without_sync(effect_receipt));
            let read_back = canonicalize_value(&outer_without_sync(&authoritative_json));
            if stored != read_back {
                return Err(String::new());
            }
            Ok(())
        }
        ImportRunEffectKind::ProjectWritingStyle => unreachable!("writing style 已在函数开头短路"),
    }
}

/// 对齐 `validateEffectStage`
fn validate_effect_stage(kind: ImportRunEffectKind, stage: &str) -> Result<(), String> {
    let expected = match kind {
        ImportRunEffectKind::ProjectGlobalFacts => "global",
        ImportRunEffectKind::ProjectWritingStyle => "style",
        ImportRunEffectKind::ChapterBlueprintRange => "blueprints",
        ImportRunEffectKind::AuthorFinalizedBatch => "author-commit",
    };
    if expected != stage {
        return Err("导入 effect receipt 类型与阶段不匹配".to_string());
    }
    Ok(())
}

/// 对齐共享契约 `expectedImportRunEffectKey`
fn expected_import_run_effect_key(
    kind: ImportRunEffectKind,
    stage: &str,
    batch_id: &str,
) -> Option<String> {
    match (kind, stage, batch_id) {
        (ImportRunEffectKind::ProjectGlobalFacts, "global", "done") => {
            Some("global-facts".to_string())
        }
        (ImportRunEffectKind::ProjectWritingStyle, "style", "done") => {
            Some("writing-style".to_string())
        }
        (ImportRunEffectKind::ChapterBlueprintRange, "blueprints", _) => {
            parse_import_run_chapter_batch_checkpoint_id(batch_id)
                .map(|_| format!("blueprints:{batch_id}"))
        }
        (ImportRunEffectKind::AuthorFinalizedBatch, "author-commit", "done") => {
            Some("author-finalized-batch".to_string())
        }
        _ => None,
    }
}

/// 对齐共享契约 `assertImportRunEffectReceiptMetadata`
fn assert_import_run_effect_receipt_metadata(
    receipt: &ImportRunEffectReceipt,
    run: &ImportRunRow,
) -> Result<(), String> {
    let expected_namespace = format!("import:{}:{}", run.purpose, run.id);
    let expected_key =
        expected_import_run_effect_key(receipt.kind, &receipt.stage, &receipt.batch_id);
    if receipt.schema_version != IMPORT_RUN_EFFECT_RECEIPT_SCHEMA_VERSION
        || receipt.run_id != run.id
        || run.effect_namespace != expected_namespace
        || receipt.effect_namespace != expected_namespace
        || expected_key.as_deref() != Some(receipt.effect_key.as_str())
    {
        return Err("导入 effect receipt 元数据损坏".to_string());
    }
    Ok(())
}

/// 读取运行的全部冻结章节行（按章号升序）
fn read_all_chapter_rows(
    conn: &Connection,
    run_id: &str,
) -> Result<Vec<ImportRunChapterRow>, String> {
    let mut statement = conn
        .prepare(
            "SELECT chapter_number, source_id, source_chapter_number,
                    title, content_fingerprint, content_size, content_snapshot
             FROM import_run_chapters WHERE run_id = ? ORDER BY chapter_number ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok(ImportRunChapterRow {
                chapter_number: row.get(0)?,
                source_id: row.get(1)?,
                source_chapter_number: row.get(2)?,
                title: row.get(3)?,
                content_fingerprint: row.get(4)?,
                content_size: row.get(5)?,
                content_snapshot: row.get(6)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn parse_effect_kind(value: &str) -> Result<ImportRunEffectKind, String> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .map_err(|_| corrupted_receipt())
}

/// 对齐 `rowToEffectReceipt`：任何非「损坏」错误统一转为 `corruptedReceipt`
/// （基线 try/catch 的等价语义）。
fn row_to_effect_receipt(
    conn: &Connection,
    row: &ImportRunEffectReceiptRow,
    run: &ImportRunRow,
) -> Result<ImportRunEffectReceipt, String> {
    row_to_effect_receipt_inner(conn, row, run).map_err(|error| {
        if error.contains("receipt 损坏") {
            error
        } else {
            corrupted_receipt()
        }
    })
}

fn row_to_effect_receipt_inner(
    conn: &Connection,
    row: &ImportRunEffectReceiptRow,
    run: &ImportRunRow,
) -> Result<ImportRunEffectReceipt, String> {
    if row.state != "prepared" && row.state != "committed" {
        return Err(corrupted_receipt());
    }
    let payload: serde_json::Value =
        serde_json::from_str(&row.payload_json).map_err(|_| corrupted_receipt())?;
    let (canonical_json, canonical_hash) = canonical_payload(&payload)?;
    if canonical_json != row.payload_json || canonical_hash != row.payload_hash {
        return Err(corrupted_receipt());
    }
    let kind = parse_effect_kind(&row.kind)?;
    assert_effect_payload_schema(kind, &payload)?;
    assert_effect_payload_binding(kind, &row.run_id, &row.batch_id, &payload)?;
    assert_author_effect_run_binding(run, kind, &payload, row.state == "committed")?;
    let effect_receipt: Option<serde_json::Value> = match &row.effect_receipt_json {
        Some(json) => Some(serde_json::from_str(json).map_err(|_| corrupted_receipt())?),
        None => None,
    };
    if (row.state == "prepared" && effect_receipt.is_some())
        || (row.state == "committed" && effect_receipt.is_none())
    {
        return Err(corrupted_receipt());
    }
    if row.state == "committed" {
        let committed = effect_receipt.as_ref().ok_or_else(|| corrupted_receipt())?;
        assert_committed_effect_schema(kind, &payload, committed)?;
        assert_committed_effect_authority(conn, kind, &payload, committed)?;
    }
    let receipt = ImportRunEffectReceipt {
        schema_version: row.schema_version,
        run_id: row.run_id.clone(),
        effect_namespace: row.effect_namespace.clone(),
        effect_key: row.effect_key.clone(),
        stage: row.stage.clone(),
        batch_id: row.batch_id.clone(),
        kind,
        payload_hash: row.payload_hash.clone(),
        state: row.state.clone(),
        payload,
        effect_receipt,
        created_at: row.created_at.clone(),
        updated_at: row.updated_at.clone(),
    };
    assert_import_run_effect_receipt_metadata(&receipt, run)?;
    Ok(receipt)
}

// ---------- effect receipts：公开 API ----------

/// 平移自 `getEffectReceipt`（读频道：失败 reject）
pub fn get_effect_receipt(
    conn: &Connection,
    run_id: &str,
    stage: &str,
    batch_id: &str,
) -> Result<Option<ImportRunEffectReceipt>, String> {
    let mut statement = conn
        .prepare(
            "SELECT run_id, schema_version, effect_namespace, effect_key, stage, \
             batch_id, kind, payload_json, payload_hash, state, effect_receipt_json, \
             created_at, updated_at
             FROM import_run_receipts
             WHERE run_id = ? AND stage = ? AND batch_id = ?",
        )
        .map_err(|error| error.to_string())?;
    let row = statement
        .query_row(
            rusqlite::params![run_id, stage, batch_id],
            read_effect_receipt_row,
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    let run = read_run_row(conn, run_id)?.ok_or_else(|| corrupted_receipt())?;
    row_to_effect_receipt(conn, &row, &run).map(Some)
}

/// 平移自 `prepareEffectReceipt`
pub fn prepare_effect_receipt(
    conn: &Connection,
    request: &ImportRunPrepareEffectReceiptRequest,
    execution: &ImportRunExecutionLease,
    now: i64,
) -> Result<ImportRunEffectReceipt, String> {
    let effect_key = request.effect_key.trim();
    let batch_id = request.batch_id.trim();
    if effect_key.is_empty()
        || effect_key.len() > 200
        || batch_id.is_empty()
        || batch_id.len() > 160
    {
        return Err("导入 effect receipt 键无效".to_string());
    }
    validate_effect_stage(request.kind, &request.stage)?;
    let (payload_json, payload_hash) = canonical_payload(&request.payload)?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let outcome: Result<ImportRunEffectReceipt, String> = (|| {
        let run = assert_execution(&tx, &request.run_id, execution, now)?;
        assert_author_effect_run_binding(&run, request.kind, &request.payload, false)?;
        assert_checkpoint_can_apply(&tx, &run, &request.stage, batch_id, "receipt")?;
        let existing = tx
            .query_row(
                "SELECT run_id, schema_version, effect_namespace, effect_key, stage, \
                 batch_id, kind, payload_json, payload_hash, state, effect_receipt_json, \
                 created_at, updated_at
                 FROM import_run_receipts
                 WHERE run_id = ? AND stage = ? AND batch_id = ?",
                rusqlite::params![request.run_id, request.stage, batch_id],
                read_effect_receipt_row,
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(existing) = existing {
            if existing.effect_namespace != run.effect_namespace
                || existing.effect_key != effect_key
                || parse_effect_kind(&existing.kind)? != request.kind
                || existing.payload_hash != payload_hash
            {
                return Err("导入 effect receipt 已绑定不同载荷".to_string());
            }
            return row_to_effect_receipt(&tx, &existing, &run);
        }
        tx.execute(
            "INSERT INTO import_run_receipts (
                run_id, schema_version, effect_namespace, effect_key, stage, batch_id,
                kind, payload_json, payload_hash
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                request.run_id,
                IMPORT_RUN_EFFECT_RECEIPT_SCHEMA_VERSION,
                run.effect_namespace,
                effect_key,
                request.stage,
                batch_id,
                request.kind.as_str(),
                payload_json,
                payload_hash,
            ],
        )
        .map_err(|error| error.to_string())?;
        get_effect_receipt(&tx, &request.run_id, &request.stage, batch_id)?
            .ok_or_else(|| "导入 effect receipt 写入后回读失败".to_string())
    })();
    match outcome {
        Ok(receipt) => {
            tx.commit().map_err(|error| error.to_string())?;
            Ok(receipt)
        }
        Err(error) => Err(error),
    }
}

/// 平移自 `commitEffectReceipt`（跨仓原子事务：下游提交 + checkpoint +
/// 收据状态在同一 `BEGIN` 内完成，依赖各下游仓储的 `_in_transaction` 抽取件）。
pub fn commit_effect_receipt(
    conn: &Connection,
    project_root: &str,
    run_id: &str,
    stage: &str,
    batch_id: &str,
    execution: &ImportRunExecutionLease,
    now: i64,
) -> Result<ImportRunEffectCommitResult, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let outcome: Result<ImportRunEffectCommitResult, String> = (|| {
        let run = assert_execution(&tx, run_id, execution, now)?;
        let mut statement = tx
            .prepare(
                "SELECT run_id, schema_version, effect_namespace, effect_key, stage, \
                 batch_id, kind, payload_json, payload_hash, state, effect_receipt_json, \
                 created_at, updated_at
                 FROM import_run_receipts
                 WHERE run_id = ? AND stage = ? AND batch_id = ?",
            )
            .map_err(|error| error.to_string())?;
        let row = statement
            .query_row(
                rusqlite::params![run_id, stage, batch_id],
                read_effect_receipt_row,
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "导入 effect receipt 不存在".to_string())?;
        assert_checkpoint_can_apply(&tx, &run, stage, batch_id, "receipt")?;
        let validated_receipt = row_to_effect_receipt(&tx, &row, &run)?;
        // 基线两处返回均用 `this.get(runId)!`：从库重读更新后的运行行
        // （`applyBatchCheckpoint` 会改写 `completed_batches_json` 等列）
        let refreshed_run = || -> Result<ImportRunRow, String> {
            read_run_row(&tx, run_id)?.ok_or_else(|| "导入运行提交后回读失败".to_string())
        };
        if row.state == "committed" {
            return Ok(ImportRunEffectCommitResult {
                receipt: validated_receipt,
                run: row_to_snapshot(&tx, &refreshed_run()?)?,
                cancel_applied: run.cancel_requested == 1,
            });
        }
        let effect_receipt: serde_json::Value = match validated_receipt.kind {
            ImportRunEffectKind::ProjectGlobalFacts => {
                let request: import_global_facts_repository::ImportGlobalFactsRequest =
                    serde_json::from_value(validated_receipt.payload.clone())
                        .map_err(|_| String::new())?;
                serde_json::to_value(import_global_facts_repository::commit_in_transaction(
                    &tx, request,
                )?)
                .map_err(|_| String::new())?
            }
            ImportRunEffectKind::ProjectWritingStyle => {
                let writing_style = validated_receipt
                    .payload
                    .get("writingStyle")
                    .and_then(|value| value.as_str())
                    .map(|text| text.trim().to_string())
                    .filter(|text| !text.is_empty())
                    .ok_or_else(|| "导入文风 receipt 载荷无效".to_string())?;
                let mut map = serde_json::Map::new();
                map.insert(
                    "writingStyle".to_string(),
                    serde_json::Value::String(writing_style.clone()),
                );
                project_core_repository::update(&tx, &map)?;
                serde_json::Value::Object(map)
            }
            ImportRunEffectKind::ChapterBlueprintRange => {
                let request: blueprint_repository::BlueprintCommitRangeRequest =
                    serde_json::from_value(validated_receipt.payload.clone())
                        .map_err(|_| String::new())?;
                let payload_hash = blueprint_repository::commit_payload_hash(&request)?;
                serde_json::to_value(blueprint_repository::commit_range_in_transaction(
                    &tx,
                    &request,
                    payload_hash,
                )?)
                .map_err(|_| String::new())?
            }
            ImportRunEffectKind::AuthorFinalizedBatch => {
                assert_author_effect_run_binding(
                    &run,
                    validated_receipt.kind,
                    &validated_receipt.payload,
                    true,
                )?;
                let chapters: Vec<draft_import::FinalizedDraftImportChapter> =
                    read_all_chapter_rows(&tx, run_id)?
                        .iter()
                        .map(chapter_row_to_snapshot)
                        .collect::<Result<Vec<_>, _>>()?
                        .into_iter()
                        .map(|chapter| {
                            let word_count = count_draft_units(&chapter.content) as i64;
                            draft_import::FinalizedDraftImportChapter {
                                chapter_number: chapter.number,
                                title: chapter.title,
                                content: chapter.content,
                                word_count,
                            }
                        })
                        .collect();
                let commit_manifest_fingerprint =
                    draft_import::preview(&tx, &chapters)?.manifest_fingerprint;
                let request = draft_import::FinalizedDraftImportRequest {
                    operation_id: format!("author-import:{run_id}"),
                    expected_authority_fingerprint: Some(run.authority_fingerprint.clone()),
                    expected_manifest_fingerprint: Some(run.manifest_fingerprint.clone()),
                    expected_commit_manifest_fingerprint: Some(commit_manifest_fingerprint),
                    chapters,
                };
                serde_json::to_value(draft_import::commit_in_transaction(
                    &tx,
                    project_root,
                    &request,
                )?)
                .map_err(|_| String::new())?
            }
        };
        let (newly_completed, cancel_applied) =
            apply_batch_checkpoint(&tx, &run, stage, batch_id, "receipt")?;
        debug_assert!(newly_completed);
        tx.execute(
            "UPDATE import_run_receipts
             SET state = 'committed', effect_receipt_json = ?1, updated_at = datetime('now')
             WHERE run_id = ?2 AND stage = ?3 AND batch_id = ?4 AND state = 'prepared'",
            rusqlite::params![
                serde_json::to_string(&effect_receipt).map_err(|error| error.to_string())?,
                run_id,
                stage,
                batch_id,
            ],
        )
        .map_err(|error| error.to_string())?;
        let receipt = get_effect_receipt(&tx, run_id, stage, batch_id)?
            .ok_or_else(|| "导入 effect receipt 提交后回读失败".to_string())?;
        Ok(ImportRunEffectCommitResult {
            receipt,
            run: row_to_snapshot(&tx, &refreshed_run()?)?,
            cancel_applied,
        })
    })();
    match outcome {
        Ok(result) => {
            tx.commit().map_err(|error| error.to_string())?;
            Ok(result)
        }
        Err(error) => Err(error),
    }
}

// ---------- 执行租约族 ----------

/// 平移自 `startOrResume`：获取/续期执行租约，并把 `prepared` 推进到 `knowledge`。
pub fn start_or_resume(
    conn: &Connection,
    run_id: &str,
    owner: &str,
    now: i64,
    lease_ms: i64,
) -> Result<ImportRunStartResult, String> {
    let normalized_owner = owner.trim();
    if normalized_owner.is_empty() || normalized_owner.len() > 160 || lease_ms < 1 {
        return Err("导入执行器身份无效".to_string());
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let outcome: Result<ImportRunStartResult, String> = (|| {
        let row = read_run_row(&tx, run_id)?
            .filter(|row| {
                row.resumable == 1
                    && matches!(
                        row.status.as_str(),
                        "ready" | "running" | "failed" | "cancelled"
                    )
            })
            .ok_or_else(|| {
                read_run_row(&tx, run_id)
                    .ok()
                    .flatten()
                    .filter(|row| row.resumable == 0 && !row.last_error.is_empty())
                    .map(|row| row.last_error.clone())
                    .unwrap_or_else(|| "导入运行当前不可启动".to_string())
            })?;
        if row.stage == "parsing" {
            return Err("导入解析尚未完成，请重新选择并授权未完成来源".to_string());
        }
        let active_project_run = tx
            .query_row(
                "SELECT id FROM import_runs
                 WHERE id <> ? AND status = 'running' AND execution_owner <> '' AND lease_expires_at > ?
                 LIMIT 1",
                rusqlite::params![run_id, now],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if active_project_run.is_some() {
            return Err("项目中另一导入运行正在执行".to_string());
        }
        if !row.execution_owner.is_empty()
            && row.execution_owner != normalized_owner
            && row.lease_expires_at > now
        {
            return Err("导入运行正在由另一执行器运行".to_string());
        }
        let same_active_owner =
            row.execution_owner == normalized_owner && row.lease_expires_at > now;
        let epoch = if same_active_owner {
            row.execution_epoch
        } else {
            row.execution_epoch + 1
        };
        let expires_at = now + lease_ms;
        tx.execute(
            "UPDATE import_runs
             SET stage = CASE WHEN stage = 'prepared' THEN 'knowledge' ELSE stage END,
                 status = 'running', resumable = 1, cancel_requested = 0,
                 last_error = '', execution_owner = ?, execution_epoch = ?, lease_expires_at = ?,
                 updated_at = datetime('now')
             WHERE id = ?",
            rusqlite::params![normalized_owner, epoch, expires_at, run_id],
        )
        .map_err(|error| error.to_string())?;
        let updated =
            read_run_row(&tx, run_id)?.ok_or_else(|| "导入运行启动后回读失败".to_string())?;
        Ok(ImportRunStartResult {
            run: row_to_snapshot(&tx, &updated)?,
            execution: ImportRunExecutionLease {
                owner: normalized_owner.to_string(),
                epoch,
                expires_at,
            },
        })
    })();
    match outcome {
        Ok(start) => {
            tx.commit().map_err(|error| error.to_string())?;
            Ok(start)
        }
        Err(error) => Err(error),
    }
}

/// 平移自 `renewExecution`
pub fn renew_execution(
    conn: &Connection,
    run_id: &str,
    execution: &ImportRunExecutionLease,
    now: i64,
    lease_ms: i64,
) -> Result<ImportRunExecutionLease, String> {
    assert_execution(conn, run_id, execution, now)?;
    let expires_at = now + lease_ms;
    conn.execute(
        "UPDATE import_runs SET lease_expires_at = ?, updated_at = datetime('now') WHERE id = ?",
        rusqlite::params![expires_at, run_id],
    )
    .map_err(|error| error.to_string())?;
    Ok(ImportRunExecutionLease {
        owner: execution.owner.clone(),
        epoch: execution.epoch,
        expires_at,
    })
}

/// 平移自 `restart`：把失败/取消/租约过期的运行复制为新运行。
pub fn restart(
    conn: &Connection,
    run_id: &str,
    next_run_id: &str,
    now: i64,
) -> Result<ImportRunSnapshot, String> {
    let normalized_next_id = next_run_id.trim();
    if normalized_next_id.is_empty() || normalized_next_id.len() > 160 {
        return Err("新导入运行 ID 无效".to_string());
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let outcome: Result<ImportRunSnapshot, String> = (|| {
        let source = read_run_row(&tx, run_id)?;
        let restartable = source.as_ref().map(|row| {
            row.status == "failed"
                || row.status == "cancelled"
                || (row.status == "running" && row.lease_expires_at <= now)
        });
        if source.is_none()
            || !restartable.unwrap_or(false)
            || source.as_ref().map(|row| row.resumable) != Some(1)
        {
            return Err("导入运行当前不可重新开始".to_string());
        }
        let source = source.expect("restartable 已校验非空");
        if source.purpose == "author-manuscript" {
            return Err("作者原稿导入不能重新开始；请继续原运行以完成发布和后处理".to_string());
        }
        if read_run_row(&tx, normalized_next_id)?.is_some() {
            return Err("新导入运行 ID 已存在".to_string());
        }
        let fenced = tx
            .execute(
                "UPDATE import_runs
                 SET resumable = 0, cancel_requested = 0, last_error = 'Restarted by user',
                     execution_owner = '', execution_epoch = execution_epoch + 1, lease_expires_at = 0,
                     updated_at = datetime('now')
                 WHERE id = ? AND execution_epoch = ?",
                rusqlite::params![run_id, source.execution_epoch],
            )
            .map_err(|error| error.to_string())?;
        if fenced != 1 {
            return Err("导入运行重新开始时已被其他执行器接管".to_string());
        }
        let restart_stage = source.stage.clone();
        let restarting_parsing = restart_stage == "parsing";
        let manifest_fingerprint = if restarting_parsing {
            "0".repeat(64)
        } else {
            source.manifest_fingerprint.clone()
        };
        let total_chapters = if restarting_parsing {
            0
        } else {
            source.total_chapters
        };
        let total_content_size = if restarting_parsing {
            0
        } else {
            source.total_content_size
        };
        let base_run_id = if restarting_parsing {
            None
        } else {
            source.base_run_id.clone()
        };
        let manifest_chapter_count = if restarting_parsing {
            0
        } else {
            source.manifest_chapter_count
        };
        let manifest_content_size = if restarting_parsing {
            0
        } else {
            source.manifest_content_size
        };
        let manifest_word_count = if restarting_parsing {
            0
        } else {
            source.manifest_word_count
        };
        tx.execute(
            "INSERT INTO import_runs (
                id, purpose, root_run_id, effect_namespace, source_fingerprint, manifest_fingerprint,
                authority_fingerprint, legacy_source_fingerprint, source_display_json, locale,
                stage, status, total_chapters, total_content_size, completed_chapters, base_run_id,
                manifest_chapter_count, manifest_content_size, manifest_word_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'ready', ?12, ?13, 0, ?14, ?15, ?16, ?17)",
            rusqlite::params![
                normalized_next_id,
                source.purpose,
                source.root_run_id,
                format!("import:{}:{}", source.purpose, normalized_next_id),
                source.source_fingerprint,
                manifest_fingerprint,
                source.authority_fingerprint,
                source.legacy_source_fingerprint,
                source.source_display_json,
                source.locale,
                restart_stage,
                total_chapters,
                total_content_size,
                base_run_id,
                manifest_chapter_count,
                manifest_content_size,
                manifest_word_count,
            ],
        )
        .map_err(|error| error.to_string())?;
        if restarting_parsing {
            tx.execute(
                "INSERT INTO import_run_sources (
                    run_id, source_index, source_id, source_fingerprint, legacy_source_fingerprint,
                    display_json, status, manifest_fingerprint, chapter_count, content_size, word_count,
                    last_error
                )
                SELECT ?, source_index, source_id, source_fingerprint, legacy_source_fingerprint,
                       display_json, 'pending', '', 0, 0, 0, ''
                FROM import_run_sources WHERE run_id = ? ORDER BY source_index",
                rusqlite::params![normalized_next_id, run_id],
            )
            .map_err(|error| error.to_string())?;
        } else {
            tx.execute(
                "INSERT INTO import_run_sources (
                    run_id, source_index, source_id, source_fingerprint, legacy_source_fingerprint,
                    display_json, status, manifest_fingerprint, chapter_count, content_size, word_count,
                    last_error
                )
                SELECT ?, source_index, source_id, source_fingerprint, legacy_source_fingerprint,
                       display_json, status, manifest_fingerprint, chapter_count, content_size, word_count,
                       last_error
                FROM import_run_sources WHERE run_id = ? ORDER BY source_index",
                rusqlite::params![normalized_next_id, run_id],
            )
            .map_err(|error| error.to_string())?;
            tx.execute(
                "INSERT INTO import_run_chapters (
                    run_id, chapter_number, source_id, source_chapter_number,
                    title, content_fingerprint, content_size, content_snapshot
                )
                SELECT ?, chapter_number, source_id, source_chapter_number,
                       title, content_fingerprint, content_size, content_snapshot
                FROM import_run_chapters WHERE run_id = ? ORDER BY chapter_number",
                rusqlite::params![normalized_next_id, run_id],
            )
            .map_err(|error| error.to_string())?;
        }
        read_run_row(&tx, normalized_next_id)?
            .ok_or_else(|| "新导入运行回读失败".to_string())
            .and_then(|row| row_to_snapshot(&tx, &row))
    })();
    match outcome {
        Ok(snapshot) => {
            tx.commit().map_err(|error| error.to_string())?;
            Ok(snapshot)
        }
        Err(error) => Err(error),
    }
}

/// 平移自 `requestCancel`
pub fn request_cancel(
    conn: &Connection,
    run_id: &str,
    execution: &ImportRunExecutionLease,
) -> Result<ImportRunSnapshot, String> {
    assert_execution(conn, run_id, execution, now_ms())?;
    let changes = conn
        .execute(
            "UPDATE import_runs SET cancel_requested = 1, updated_at = datetime('now')
             WHERE id = ? AND status IN ('ready', 'running')",
            [run_id],
        )
        .map_err(|error| error.to_string())?;
    if changes == 0 {
        return Err("导入运行当前不可取消".to_string());
    }
    get(conn, run_id)?.ok_or_else(|| "导入运行取消后回读失败".to_string())
}

/// 平移自 `cancelAtBoundary`
pub fn cancel_at_boundary(
    conn: &Connection,
    run_id: &str,
    execution: &ImportRunExecutionLease,
) -> Result<ImportRunSnapshot, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let outcome: Result<ImportRunSnapshot, String> = (|| {
        assert_execution(&tx, run_id, execution, now_ms())?;
        let changes = tx
            .execute(
                "UPDATE import_runs
                 SET status = 'cancelled', resumable = 1, cancel_requested = 1,
                     execution_owner = '', execution_epoch = execution_epoch + 1, lease_expires_at = 0,
                     updated_at = datetime('now')
                 WHERE id = ? AND status IN ('ready', 'running', 'failed')",
                [run_id],
            )
            .map_err(|error| error.to_string())?;
        if changes == 0 {
            return Err("导入运行当前不可在安全边界取消".to_string());
        }
        read_run_row(&tx, run_id)?
            .ok_or_else(|| "导入运行取消后回读失败".to_string())
            .and_then(|row| row_to_snapshot(&tx, &row))
    })();
    match outcome {
        Ok(snapshot) => {
            tx.commit().map_err(|error| error.to_string())?;
            Ok(snapshot)
        }
        Err(error) => Err(error),
    }
}

/// 平移自 `completeBatch`
pub fn complete_batch(
    conn: &Connection,
    run_id: &str,
    stage: &str,
    batch_id: &str,
    execution: &ImportRunExecutionLease,
) -> Result<ImportRunBatchCheckpointResult, String> {
    if batch_id.trim().is_empty() || batch_id.len() > 160 {
        return Err("导入批次 ID 无效".to_string());
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let outcome: Result<ImportRunBatchCheckpointResult, String> = (|| {
        let row = assert_execution(&tx, run_id, execution, now_ms())?;
        let (newly_completed, cancel_applied) =
            apply_batch_checkpoint(&tx, &row, stage, batch_id, "direct")?;
        // 基线返回 `this.get(runId)!` —— 检查点落地**之后**的快照
        // （取消生效时 status 已变为 'cancelled'），而非 assert 时的旧行。
        let updated =
            read_run_row(&tx, run_id)?.ok_or_else(|| "导入批次检查点后回读失败".to_string())?;
        Ok(ImportRunBatchCheckpointResult {
            newly_completed,
            cancel_applied,
            run: row_to_snapshot(&tx, &updated)?,
        })
    })();
    match outcome {
        Ok(result) => {
            tx.commit().map_err(|error| error.to_string())?;
            Ok(result)
        }
        Err(error) => Err(error),
    }
}

/// 平移自 `advanceStage`
pub fn advance_stage(
    conn: &Connection,
    run_id: &str,
    completed_stage: &str,
    next_stage: &str,
    execution: &ImportRunExecutionLease,
) -> Result<ImportRunSnapshot, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let outcome: Result<ImportRunSnapshot, String> = (|| {
        let row = assert_execution(&tx, run_id, execution, now_ms())?;
        if row.status != "running" || row.stage != completed_stage {
            return Err("导入阶段与当前阶段不匹配".to_string());
        }
        if next_stage_for_run(&row.purpose, completed_stage) != Some(next_stage) {
            return Err("导入下一阶段转换无效".to_string());
        }
        assert_stage_checkpoint_complete(&tx, &row, completed_stage)?;
        let completed_chapters =
            if completed_stage == "blueprints" || completed_stage == "author-postprocess" {
                row.total_chapters
            } else {
                row.completed_chapters
            };
        let changes = tx
            .execute(
                "UPDATE import_runs
                 SET stage = ?, completed_chapters = ?, status = 'running', last_error = '',
                     resumable = 1, updated_at = datetime('now')
                 WHERE id = ? AND stage = ? AND status = 'running'",
                rusqlite::params![next_stage, completed_chapters, run_id, completed_stage],
            )
            .map_err(|error| error.to_string())?;
        if changes != 1 {
            return Err("导入阶段转换已过期".to_string());
        }
        read_run_row(&tx, run_id)?
            .ok_or_else(|| "导入阶段推进后回读失败".to_string())
            .and_then(|row| row_to_snapshot(&tx, &row))
    })();
    match outcome {
        Ok(snapshot) => {
            tx.commit().map_err(|error| error.to_string())?;
            Ok(snapshot)
        }
        Err(error) => Err(error),
    }
}

/// 平移自 `fail`
pub fn fail_run(
    conn: &Connection,
    run_id: &str,
    stage: &str,
    error: &str,
    execution: &ImportRunExecutionLease,
) -> Result<ImportRunSnapshot, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let outcome: Result<ImportRunSnapshot, String> = (|| {
        let row = assert_execution(&tx, run_id, execution, now_ms())?;
        if row.status != "running" || row.stage != stage {
            return Err("导入失败阶段与当前阶段不匹配".to_string());
        }
        let truncated: String = error.chars().take(2_000).collect();
        let changes = tx
            .execute(
                "UPDATE import_runs
                 SET status = 'failed', last_error = ?, resumable = 1,
                     execution_owner = '', execution_epoch = execution_epoch + 1, lease_expires_at = 0,
                     updated_at = datetime('now')
                 WHERE id = ? AND stage = ? AND status = 'running'",
                rusqlite::params![truncated, run_id, stage],
            )
            .map_err(|error| error.to_string())?;
        if changes == 0 {
            return Err("导入运行当前不可标记失败".to_string());
        }
        read_run_row(&tx, run_id)?
            .ok_or_else(|| "导入运行失败后回读失败".to_string())
            .and_then(|row| row_to_snapshot(&tx, &row))
    })();
    match outcome {
        Ok(snapshot) => {
            tx.commit().map_err(|error| error.to_string())?;
            Ok(snapshot)
        }
        Err(error) => Err(error),
    }
}

/// 平移自 `complete`（只能从 `refresh` 阶段完成）
pub fn complete_run(
    conn: &Connection,
    run_id: &str,
    execution: &ImportRunExecutionLease,
) -> Result<ImportRunSnapshot, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let outcome: Result<ImportRunSnapshot, String> = (|| {
        let row = assert_execution(&tx, run_id, execution, now_ms())?;
        if row.status != "running" || row.stage != "refresh" {
            return Err("导入运行只能从 refresh 刷新阶段完成".to_string());
        }
        assert_stage_checkpoint_complete(&tx, &row, "refresh")?;
        let changes = tx
            .execute(
                "UPDATE import_runs
                 SET stage = 'completed', status = 'completed', completed_chapters = total_chapters,
                     resumable = 0, cancel_requested = 0, last_error = '',
                     execution_owner = '', execution_epoch = execution_epoch + 1, lease_expires_at = 0,
                     completed_at = datetime('now'), updated_at = datetime('now')
                 WHERE id = ? AND stage = 'refresh' AND status = 'running'",
                [run_id],
            )
            .map_err(|error| error.to_string())?;
        if changes == 0 {
            return Err("导入运行当前不可完成".to_string());
        }
        read_run_row(&tx, run_id)?
            .ok_or_else(|| "导入运行完成后回读失败".to_string())
            .and_then(|row| row_to_snapshot(&tx, &row))
    })();
    match outcome {
        Ok(snapshot) => {
            tx.commit().map_err(|error| error.to_string())?;
            Ok(snapshot)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use crate::repositories::character_roster_repository;

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

    // ==================== G2b-3：finalizeParsing 四态分类 ====================

    /// 造一条「已完成」的参考导入运行（台账 + 冻结章 + 章号映射）
    fn seed_completed_run(
        conn: &Connection,
        run_id: &str,
        source_id: &str,
        content: &str,
        title: &str,
    ) {
        conn.execute(
            "INSERT INTO import_runs (
                id, purpose, root_run_id, effect_namespace, source_fingerprint, manifest_fingerprint,
                source_display_json, locale, stage, status, completed_batches_json,
                resumable, total_chapters, total_content_size, manifest_chapter_count,
                manifest_content_size, manifest_word_count, completed_chapters, completed_at
             ) VALUES (?, 'reference', ?, ?, ?, ?, '[]', 'zh-CN', 'refresh', 'completed', '{}',
                0, 1, ?, 1, ?, ?, 0, datetime('now'))",
            rusqlite::params![
                run_id,
                run_id,
                format!("import:reference:{run_id}"),
                "a".repeat(64),
                "b".repeat(64),
                content.len() as i64,
                content.len() as i64,
                count_draft_units(content),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO import_run_chapters (run_id, chapter_number, source_id, source_chapter_number,
                title, content_fingerprint, content_size, content_snapshot)
             VALUES (?, 1, ?, 1, ?, ?, ?, ?)",
            rusqlite::params![
                run_id,
                source_id,
                title,
                sha256_hex(content),
                content.len() as i64,
                content
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO import_source_chapter_map (purpose, source_id, source_chapter_number, chapter_number)
             VALUES ('reference', ?, 1, 1)",
            rusqlite::params![source_id],
        )
        .unwrap();
    }

    fn run_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM import_runs", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn finalize_parsing_new_classification_test() {
        let conn = memory_conn();
        begin_parsing(&conn, &begin_request("run-fin", SOURCE_ID)).unwrap();
        commit_parsed_source(
            &conn,
            "run-fin",
            SOURCE_ID,
            &[chapter_input(1, "正文甲"), chapter_input(2, "正文乙")],
        )
        .unwrap();

        let result = finalize_parsing(&conn, "run-fin").unwrap();
        assert_eq!(result.classification, "new");
        let run = result.run.clone().unwrap();
        assert_eq!(run.stage, "prepared");
        assert_eq!(run.status, "ready");
        assert_eq!(run.total_chapters, 2);
        assert_eq!(run.total_content_size, 18);
        assert_eq!(run.manifest_chapter_count, 2);
        assert_eq!(run.manifest_content_size, 18);
        assert_eq!(result.new_chapter_numbers, vec![1, 2]);
        assert!(result.conflict_chapter_numbers.is_empty());

        let inspection = result.inspection.unwrap();
        assert_eq!(inspection.source_count, 1);
        assert_eq!(
            inspection.source_display_names,
            vec!["第1章.txt".to_string()]
        );
        assert_eq!(inspection.chapter_count, 2);
        assert_eq!(inspection.total_words, 6);
        assert_eq!(inspection.total_bytes, 18);
        assert_eq!(inspection.preview.len(), 2);
        assert_eq!(inspection.preview[0].target_status, "new");
        assert_eq!(inspection.preview[0].word_count, 3);
        assert_eq!(inspection.preview_remaining, 0);

        // 章号映射与冻结章已落库
        let mapped: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM import_source_chapter_map",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mapped, 2);
        let frozen: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM import_run_chapters WHERE run_id = 'run-fin'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(frozen, 2);

        // 已离开 parsing → 再次 finalize 被拒
        assert_eq!(
            finalize_parsing(&conn, "run-fin").unwrap_err(),
            "导入解析运行当前不可完成"
        );
        // 未知运行同样拒绝
        assert_eq!(
            finalize_parsing(&conn, "missing").unwrap_err(),
            "导入解析运行当前不可完成"
        );
    }

    #[test]
    fn finalize_parsing_incomplete_sources_rejected_test() {
        let conn = memory_conn();
        begin_parsing(&conn, &begin_request("run-partial", SOURCE_ID)).unwrap();
        assert_eq!(
            finalize_parsing(&conn, "run-partial").unwrap_err(),
            "导入来源解析尚未完成，请重新授权未完成来源"
        );
    }

    #[test]
    fn finalize_parsing_exact_duplicate_and_conflict_test() {
        let conn = memory_conn();
        seed_completed_run(&conn, "run-done", SOURCE_ID, "正文甲", "第1章");

        // 同来源同内容 → exact-duplicate，且临时解析运行被删除
        begin_parsing(&conn, &begin_request("run-dup", SOURCE_ID)).unwrap();
        commit_parsed_source(&conn, "run-dup", SOURCE_ID, &[chapter_input(1, "正文甲")]).unwrap();
        let duplicated = finalize_parsing(&conn, "run-dup").unwrap();
        assert_eq!(duplicated.classification, "exact-duplicate");
        assert!(duplicated.run.is_none());
        assert_eq!(duplicated.duplicate_chapter_numbers, vec![1]);
        assert!(duplicated.new_chapter_numbers.is_empty());
        assert_eq!(run_count(&conn), 1, "临时解析运行应被删除");
        assert_eq!(
            duplicated.inspection.unwrap().preview[0].target_status,
            "duplicate"
        );

        // 同来源不同内容 → conflict，同样丢弃临时运行
        begin_parsing(&conn, &begin_request("run-conflict", SOURCE_ID)).unwrap();
        commit_parsed_source(
            &conn,
            "run-conflict",
            SOURCE_ID,
            &[chapter_input(1, "正文丙")],
        )
        .unwrap();
        let conflict = finalize_parsing(&conn, "run-conflict").unwrap();
        assert_eq!(conflict.classification, "conflict");
        assert!(conflict.run.is_none());
        assert_eq!(conflict.conflict_chapter_numbers, vec![1]);
        assert!(conflict.new_chapter_numbers.is_empty());
        assert_eq!(run_count(&conn), 1, "临时解析运行应被丢弃");
        assert_eq!(
            conflict.inspection.unwrap().preview[0].target_status,
            "conflict"
        );
    }

    #[test]
    fn finalize_parsing_resumable_hits_existing_run_test() {
        let conn = memory_conn();
        let chapters = vec![chapter_input(1, "正文甲"), chapter_input(2, "正文乙")];
        // 第一次：解析完成为 prepared 运行
        begin_parsing(&conn, &begin_request("run-first", SOURCE_ID)).unwrap();
        commit_parsed_source(&conn, "run-first", SOURCE_ID, &chapters).unwrap();
        assert_eq!(
            finalize_parsing(&conn, "run-first").unwrap().classification,
            "new"
        );

        // 第二次：同来源同内容 → 命中既有可恢复运行（同 manifest 指纹）
        begin_parsing(&conn, &begin_request("run-second", SOURCE_ID)).unwrap();
        commit_parsed_source(&conn, "run-second", SOURCE_ID, &chapters).unwrap();
        let resumable = finalize_parsing(&conn, "run-second").unwrap();
        assert_eq!(resumable.classification, "resumable");
        assert_eq!(
            resumable.run.as_ref().map(|run| run.id.as_str()),
            Some("run-first")
        );
        assert_eq!(resumable.duplicate_chapter_numbers, vec![1, 2]);
        assert!(resumable.new_chapter_numbers.is_empty());
        assert_eq!(run_count(&conn), 1);
    }

    // ==================== G2b-4：prepare 两分支 ====================

    fn prepare_request(
        run_id: &str,
        purpose: ImportPurpose,
        chapters: Vec<ImportRunChapterInput>,
    ) -> ImportRunPrepareRequest {
        ImportRunPrepareRequest {
            run_id: run_id.to_string(),
            purpose,
            source_fingerprint: "a".repeat(64),
            source_ids: Some(vec![SOURCE_ID.to_string()]),
            source_fingerprints: Some(vec!["b".repeat(64)]),
            source_display: vec![ImportSourceDisplayMetadata {
                display_name: "第1章.txt".to_string(),
                media_type: "text/plain".to_string(),
                size: 10,
            }],
            locale: ImportRunLocale::ZhCn,
            authority_fingerprint: None,
            expected_manifest_fingerprint: None,
            chapters,
        }
    }

    #[test]
    fn prepare_reference_creates_knowledge_run_then_resumes_test() {
        let conn = memory_conn();
        let request = prepare_request(
            "run-prep",
            ImportPurpose::Reference,
            vec![chapter_input(1, "正文甲"), chapter_input(2, "正文乙")],
        );
        let prepared = prepare(&conn, &request).unwrap();
        assert_eq!(prepared.classification, "new");
        let run = prepared.run.clone().unwrap();
        assert_eq!(run.stage, "knowledge");
        assert_eq!(run.status, "ready");
        assert_eq!(run.total_chapters, 2);
        assert_eq!(run.total_content_size, 18);
        assert_eq!(run.manifest_chapter_count, 2);
        assert_eq!(run.manifest_content_size, 18);
        assert_eq!(prepared.new_chapter_numbers, vec![1, 2]);
        assert_eq!(
            prepared.inspection.as_ref().map(|i| i.preview_remaining),
            Some(0)
        );
        // 冻结章直接落库（reference 分支不经 parsing）
        let frozen: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM import_run_chapters WHERE run_id = 'run-prep'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(frozen, 2);
        let mapped: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM import_source_chapter_map",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mapped, 2);

        // 同来源同清单再次 prepare → 命中既有可恢复运行
        let again = prepare(
            &conn,
            &prepare_request(
                "run-prep-2",
                ImportPurpose::Reference,
                request.chapters.clone(),
            ),
        )
        .unwrap();
        assert_eq!(again.classification, "resumable");
        assert_eq!(
            again.run.as_ref().map(|run| run.id.as_str()),
            Some("run-prep")
        );
        assert_eq!(again.duplicate_chapter_numbers, vec![1, 2]);
        assert_eq!(run_count(&conn), 1, "resumable 不应新建运行");

        // runId 重复（同清单但命中 exact-duplicate 之前：先改内容使之非重复）→ '导入运行 ID 已存在'
        let mut conflict_request = prepare_request(
            "run-prep",
            ImportPurpose::Reference,
            vec![chapter_input(1, "全新正文")],
        );
        conflict_request.source_fingerprint = "c".repeat(64);
        assert_eq!(
            prepare(&conn, &conflict_request).unwrap_err().message(),
            "导入运行 ID 已存在"
        );
    }

    #[test]
    fn prepare_author_manuscript_guards_and_happy_path_test() {
        let conn = memory_conn();
        let chapters = vec![chapter_input(1, "正文甲"), chapter_input(2, "正文乙")];
        let author_chapters: Vec<draft_import::FinalizedDraftImportChapter> = chapters
            .iter()
            .map(|chapter| draft_import::FinalizedDraftImportChapter {
                chapter_number: chapter.number,
                title: chapter.title.clone(),
                content: chapter.content.clone(),
                word_count: count_draft_units(&chapter.content),
            })
            .collect();
        let preview = draft_import::preview(&conn, &author_chapters).unwrap();

        // 缺权威预览 → 拒绝
        let bare = prepare_request(
            "run-author",
            ImportPurpose::AuthorManuscript,
            chapters.clone(),
        );
        assert_eq!(
            prepare(&conn, &bare).unwrap_err().message(),
            "作者原稿缺少已确认的权威预览"
        );

        // 权威指纹过期 → AuthorPreviewStale（命令层映射为 errorCode）
        let mut stale = prepare_request(
            "run-author",
            ImportPurpose::AuthorManuscript,
            chapters.clone(),
        );
        stale.authority_fingerprint = Some("f".repeat(64));
        stale.expected_manifest_fingerprint = Some(preview.manifest_fingerprint.clone());
        match prepare(&conn, &stale) {
            Err(PrepareError::AuthorPreviewStale(message)) => {
                assert_eq!(message, "项目权威章节已变化，作者原稿预览已过期")
            }
            other => panic!("应为 AuthorPreviewStale，实际 {other:?}"),
        }

        // 正常创建 → stage=author-commit，带 authority_fingerprint
        let mut request = prepare_request(
            "run-author",
            ImportPurpose::AuthorManuscript,
            chapters.clone(),
        );
        request.authority_fingerprint = Some(preview.authority_fingerprint.clone());
        request.expected_manifest_fingerprint = Some(preview.manifest_fingerprint.clone());
        let prepared = prepare(&conn, &request).unwrap();
        assert_eq!(prepared.classification, "new");
        assert!(
            prepared.inspection.is_none(),
            "author 分支不下发 inspection"
        );
        let run = prepared.run.clone().unwrap();
        assert_eq!(run.stage, "author-commit");
        assert_eq!(
            run.manifest_fingerprint.as_deref(),
            Some(preview.manifest_fingerprint.as_str())
        );
        assert_eq!(
            run.authority_fingerprint.as_deref(),
            Some(preview.authority_fingerprint.as_str())
        );
        assert_eq!(run.total_chapters, preview.new_chapter_numbers.len() as i64);
        assert_eq!(run.manifest_chapter_count, 2);

        // 同一权威再确认 → 命中既有运行（authority 相等即 resumable）
        let mut again = prepare_request(
            "run-author-2",
            ImportPurpose::AuthorManuscript,
            chapters.clone(),
        );
        again.authority_fingerprint = Some(preview.authority_fingerprint.clone());
        again.expected_manifest_fingerprint = Some(preview.manifest_fingerprint.clone());
        let resumed = prepare(&conn, &again).unwrap();
        assert_eq!(resumed.classification, "resumable");
        assert_eq!(
            resumed.run.as_ref().map(|run| run.id.as_str()),
            Some("run-author")
        );
        assert_eq!(run_count(&conn), 1);
    }

    // ---------- 批次 G3a 移植：执行租约 / 状态机 / effect receipts ----------

    /// 对齐基线 `import-run-execution-lease.test.ts` 的 prepare 请求
    /// （单章 reference 运行，章节内容 `'x'`）。
    fn leased_run_request() -> ImportRunPrepareRequest {
        ImportRunPrepareRequest {
            run_id: "leased-run".to_string(),
            purpose: ImportPurpose::Reference,
            source_fingerprint: "a".repeat(64),
            source_ids: None,
            source_fingerprints: None,
            source_display: vec![ImportSourceDisplayMetadata {
                display_name: "reference.txt".to_string(),
                media_type: "text/plain".to_string(),
                size: 1,
            }],
            locale: ImportRunLocale::EnUs,
            authority_fingerprint: None,
            expected_manifest_fingerprint: None,
            chapters: vec![ImportRunChapterInput {
                number: 1,
                source_index: None,
                source_chapter_number: None,
                title: "One".to_string(),
                content_fingerprint: sha256_hex("x"),
                content_size: 1,
                content: "x".to_string(),
            }],
        }
    }

    fn prepared_leased_run(conn: &Connection) {
        prepare(conn, &leased_run_request()).unwrap();
    }

    /// 文件库（供「关闭重开」语义测试）：Tauri 侧无全局项目 DB 单例，
    /// 用同一文件路径重新打开即等价于基线的 `closeProjectDatabase` +
    /// `initProjectDatabase`（含 `createTables` 的会话边界围栏迁移）。
    fn file_conn(dir: &std::path::Path) -> Connection {
        let conn = Connection::open(dir.join("lorekeeper-g3a.db")).expect("文件库应可打开");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("应启用外键");
        schema::create_tables(&conn).expect("建表应成功");
        conn
    }

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("系统时钟应晚于 UNIX 纪元")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("lorekeeper-g3a-{label}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("应创建临时目录");
        dir
    }

    fn authority_of(execution: &ImportRunExecutionLease) -> ImportRunExecutionAuthority {
        ImportRunExecutionAuthority {
            owner: execution.owner.clone(),
            epoch: execution.epoch,
        }
    }

    #[test]
    fn execution_lease_heartbeat_stable_and_takeover_fences_test() {
        let conn = memory_conn();
        prepared_leased_run(&conn);
        let first = start_or_resume(&conn, "leased-run", "renderer-a", 1_000, 100).unwrap();
        let authority = authority_of(&first.execution);

        assert_execution_authority(&conn, "leased-run", &authority, 1_050).unwrap();
        renew_execution(&conn, "leased-run", &first.execution, 1_050, 100).unwrap();
        assert_execution_authority(&conn, "leased-run", &authority, 1_149).unwrap();

        start_or_resume(&conn, "leased-run", "renderer-b", 1_151, 100).unwrap();
        let error = assert_execution_authority(&conn, "leased-run", &authority, 1_151)
            .expect_err("接管后旧执行器应被围栏");
        assert!(error.contains("执行租约"), "实际：{error}");
    }

    #[test]
    fn resolve_reference_import_authority_binds_frozen_chapter_test() {
        let conn = memory_conn();
        prepared_leased_run(&conn);
        let started = start_or_resume(&conn, "leased-run", "renderer-a", 1_000, 100).unwrap();
        let authority = authority_of(&started.execution);
        let binding = resolve_reference_import_authority(&conn, "leased-run", &authority, 1, 1_050)
            .expect("冻结章节应授权");
        assert_eq!(binding.chapter_number, 1);
        assert_eq!(binding.title, "One");
        assert_eq!(binding.content, "x");
        assert_eq!(binding.content_fingerprint, sha256_hex("x"));
        // source_id 由 `normalize_source_ids` 的 `None` 分支派生
        // （`legacy:{source_fingerprint}`）；键序由 serde 映射序决定（D3）。
        let source_id = format!("legacy:{}", "a".repeat(64));
        assert_eq!(
            binding.stable_key,
            format!("reference:{}", {
                let payload = serde_json::json!({
                    "purpose": "reference",
                    "sourceId": source_id,
                    "sourceChapterNumber": 1,
                    "contentFingerprint": sha256_hex("x"),
                });
                sha256_hex(&payload.to_string())
            })
        );
        let error = resolve_reference_import_authority(&conn, "leased-run", &authority, 2, 1_050)
            .expect_err("未冻结章节应拒绝");
        assert!(error.contains("冻结章节"), "实际：{error}");
    }

    #[test]
    fn request_cancel_revokes_knowledge_authority_test() {
        let conn = memory_conn();
        prepared_leased_run(&conn);
        // 取消请求 / 续期族内部以 `now_ms()` 为当前时刻，故时间基准
        // 必须取真实时钟（对齐基线 `Date.now()` 默认参数）。
        let now = now_ms();
        let started = start_or_resume(&conn, "leased-run", "renderer-a", now, 60_000).unwrap();
        let authority = authority_of(&started.execution);
        resolve_reference_import_authority(&conn, "leased-run", &authority, 1, now)
            .expect("取消前应授权");

        request_cancel(&conn, "leased-run", &started.execution).unwrap();

        let error = resolve_reference_import_authority(&conn, "leased-run", &authority, 1, now)
            .expect_err("取消请求后应撤销知识库授权");
        assert!(error.contains("冻结章节"), "实际：{error}");
    }

    #[test]
    fn concurrent_owner_rejected_and_stale_mutations_fenced_test() {
        let conn = memory_conn();
        prepared_leased_run(&conn);
        let first = start_or_resume(&conn, "leased-run", "renderer-a", 1_000, 100).unwrap();
        let error = start_or_resume(&conn, "leased-run", "renderer-b", 1_050, 100)
            .expect_err("并发执行器应拒绝");
        assert!(error.contains("正在由另一执行器运行"), "实际：{error}");

        let takeover = start_or_resume(&conn, "leased-run", "renderer-b", 1_101, 100).unwrap();
        assert_eq!(takeover.execution.epoch, first.execution.epoch + 1);

        let stale_writes: [(&str, Result<(), String>); 5] = [
            (
                "completeBatch",
                complete_batch(&conn, "leased-run", "knowledge", "1", &first.execution).map(|_| ()),
            ),
            (
                "advanceStage",
                advance_stage(&conn, "leased-run", "knowledge", "global", &first.execution)
                    .map(|_| ()),
            ),
            (
                "fail",
                fail_run(&conn, "leased-run", "knowledge", "late", &first.execution).map(|_| ()),
            ),
            (
                "cancelAtBoundary",
                cancel_at_boundary(&conn, "leased-run", &first.execution).map(|_| ()),
            ),
            (
                "complete",
                complete_run(&conn, "leased-run", &first.execution).map(|_| ()),
            ),
        ];
        for (label, outcome) in stale_writes {
            let error = outcome.expect_err("旧执行器写入应拒绝");
            assert!(error.contains("执行租约"), "{label} 实际：{error}");
        }
    }

    #[test]
    fn failure_releases_lease_for_immediate_resume_test() {
        let conn = memory_conn();
        prepared_leased_run(&conn);
        let base = now_ms();
        let first = start_or_resume(&conn, "leased-run", "renderer-a", base, 60_000).unwrap();

        let failed = fail_run(
            &conn,
            "leased-run",
            "knowledge",
            "provider unavailable",
            &first.execution,
        )
        .unwrap();
        assert_eq!(failed.status, "failed");

        let resumed = start_or_resume(&conn, "leased-run", "renderer-b", base + 1, 60_000).unwrap();
        assert!(resumed.execution.epoch > first.execution.epoch);
        let error = complete_batch(
            &conn,
            "leased-run",
            "knowledge",
            "late-old-runner",
            &first.execution,
        )
        .expect_err("旧执行器应拒绝");
        assert!(error.contains("执行租约"), "实际：{error}");
    }

    #[test]
    fn cancel_and_completion_terminal_boundaries_test() {
        let conn = memory_conn();
        prepared_leased_run(&conn);
        let base = now_ms();
        let first = start_or_resume(&conn, "leased-run", "renderer-a", base, 60_000).unwrap();
        let binding = resolve_reference_import_authority(
            &conn,
            "leased-run",
            &authority_of(&first.execution),
            1,
            base,
        )
        .unwrap();
        let document_id = sha256_hex(&format!("reference-import:{}", binding.stable_key));
        conn.execute(
            "INSERT INTO import_reference_documents (
                document_id, idempotency_key_hash, content_hash, chunk_set_hash,
                expected_chunk_count, corpus_kind, state
             ) VALUES (?, ?, ?, ?, 1, 'reference', 'committed')",
            rusqlite::params![
                document_id,
                sha256_hex(&binding.stable_key),
                binding.content_fingerprint.clone(),
                sha256_hex("chunks:1"),
            ],
        )
        .unwrap();
        commit_reference_import_receipt(
            &conn,
            "leased-run",
            &authority_of(&first.execution),
            1,
            &document_id,
        )
        .unwrap();
        let checkpoint = format!("1-1-{}", &sha256_hex("x")[..8]);
        request_cancel(&conn, "leased-run", &first.execution).unwrap();
        let checkpoint_result = complete_batch(
            &conn,
            "leased-run",
            "knowledge",
            &checkpoint,
            &first.execution,
        )
        .unwrap();
        assert!(checkpoint_result.cancel_applied);
        assert_eq!(checkpoint_result.run.status, "cancelled");

        let resumed = start_or_resume(&conn, "leased-run", "renderer-b", base + 1, 60_000).unwrap();
        assert!(resumed.execution.epoch > first.execution.epoch);
        let error =
            cancel_at_boundary(&conn, "leased-run", &first.execution).expect_err("旧执行器应拒绝");
        assert!(error.contains("执行租约"), "实际：{error}");

        conn.execute(
            "UPDATE import_runs
             SET stage = 'refresh', completed_batches_json = '{\"refresh\":[\"done\"]}'
             WHERE id = 'leased-run'",
            [],
        )
        .unwrap();
        let completed = complete_run(&conn, "leased-run", &resumed.execution).unwrap();
        assert_eq!(completed.status, "completed");
        assert!(!completed.resumable);
        let error = start_or_resume(&conn, "leased-run", "renderer-c", base + 2, 60_000)
            .expect_err("已完成运行不可启动");
        assert!(error.contains("不可启动"), "实际：{error}");
        let error = fail_run(
            &conn,
            "leased-run",
            "knowledge",
            "late-after-complete",
            &resumed.execution,
        )
        .expect_err("完成后写入应拒绝");
        assert!(error.contains("执行租约"), "实际：{error}");
    }

    #[test]
    fn reopen_fences_running_lease_for_immediate_takeover_test() {
        let dir = unique_temp_dir("reopen-lease");
        let base = now_ms();
        let first_epoch = {
            let conn = file_conn(&dir);
            prepared_leased_run(&conn);
            let first = start_or_resume(&conn, "leased-run", "renderer-a", base, 60_000).unwrap();
            first.execution.epoch
            // conn 在此 drop（等价于基线 closeProjectDatabase）
        };
        let conn = file_conn(&dir);
        let resumed = start_or_resume(&conn, "leased-run", "renderer-b", base + 1, 60_000).unwrap();
        assert!(resumed.execution.epoch > first_epoch);
        let error = fail_run(
            &conn,
            "leased-run",
            "knowledge",
            "stale process",
            &ImportRunExecutionLease {
                owner: "renderer-a".to_string(),
                epoch: first_epoch,
                expires_at: base + 60_000,
            },
        )
        .expect_err("重开后旧执行器应被围栏");
        assert!(error.contains("执行租约"), "实际：{error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restart_only_terminal_or_expired_runs_test() {
        let conn = memory_conn();
        prepared_leased_run(&conn);
        let error =
            restart(&conn, "leased-run", "ready-restart", 1_000).expect_err("就绪运行不可重新开始");
        assert!(error.contains("不可重新开始"), "实际：{error}");

        let first = start_or_resume(&conn, "leased-run", "renderer-a", 1_000, 100).unwrap();
        let error = restart(&conn, "leased-run", "active-restart", 1_050)
            .expect_err("活跃运行不可重新开始");
        assert!(error.contains("不可重新开始"), "实际：{error}");

        let restarted = restart(&conn, "leased-run", "expired-restart", 1_101).unwrap();
        assert_eq!(restarted.id, "expired-restart");
        assert_eq!(restarted.status, "ready");
        let error = complete_batch(
            &conn,
            "leased-run",
            "knowledge",
            "late-after-restart",
            &first.execution,
        )
        .expect_err("旧执行器应拒绝");
        assert!(error.contains("执行租约"), "实际：{error}");
    }

    #[test]
    fn project_level_serialization_across_runs_test() {
        let conn = memory_conn();
        prepared_leased_run(&conn);
        let mut second_request = leased_run_request();
        second_request.run_id = "second-run".to_string();
        second_request.source_fingerprint = "c".repeat(64);
        prepare(&conn, &second_request).unwrap();
        let first = start_or_resume(&conn, "leased-run", "renderer-a", 1_000, 100).unwrap();

        let error = start_or_resume(&conn, "second-run", "renderer-b", 1_050, 100)
            .expect_err("项目级互斥应拒绝");
        assert!(
            error.contains("项目") || error.contains("另一") || error.contains("正在运行"),
            "实际：{error}"
        );

        let takeover = start_or_resume(&conn, "second-run", "renderer-b", 1_101, 100).unwrap();
        assert_eq!(takeover.run.id, "second-run");
        assert_eq!(takeover.run.status, "running");
        let error = start_or_resume(&conn, "leased-run", "renderer-c", 1_102, 100)
            .expect_err("项目级互斥应拒绝");
        assert!(
            error.contains("项目") || error.contains("另一") || error.contains("正在运行"),
            "实际：{error}"
        );
        let error = renew_execution(&conn, "leased-run", &first.execution, 1_102, 100)
            .expect_err("过期租约应拒绝");
        assert!(error.contains("执行租约"), "实际：{error}");
    }

    #[test]
    fn tampered_snapshot_rejected_after_reopen_test() {
        let dir = unique_temp_dir("reopen-tamper");
        {
            let conn = file_conn(&dir);
            prepared_leased_run(&conn);
            start_or_resume(&conn, "leased-run", "renderer-a", 1_000, 100).unwrap();
            conn.execute(
                "UPDATE import_run_chapters SET content_snapshot = 'y' WHERE run_id = 'leased-run'",
                [],
            )
            .unwrap();
            // conn 在此 drop（等价于基线 closeProjectDatabase）
        }
        let conn = file_conn(&dir);
        let resumed = start_or_resume(&conn, "leased-run", "renderer-b", 1_050, 100).unwrap();

        let error =
            list_chapter_batch(&conn, "leased-run", 0, 10).expect_err("分页读回应拒绝损坏快照");
        assert!(
            error.contains("冻结") || error.contains("快照") || error.contains("损坏"),
            "实际：{error}"
        );
        let error = resolve_reference_import_authority(
            &conn,
            "leased-run",
            &authority_of(&resumed.execution),
            1,
            1_051,
        )
        .expect_err("知识库授权应拒绝损坏快照");
        assert!(
            error.contains("冻结") || error.contains("快照") || error.contains("损坏"),
            "实际：{error}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn utf8_byte_size_checked_independently_test() {
        let conn = memory_conn();
        prepared_leased_run(&conn);
        let content = "é";
        conn.execute(
            "UPDATE import_run_chapters
             SET content_snapshot = ?, content_fingerprint = ?
             WHERE run_id = 'leased-run'",
            rusqlite::params![content, sha256_hex(content)],
        )
        .unwrap();

        let error =
            list_chapter_batch(&conn, "leased-run", 0, 10).expect_err("UTF-8 字节大小应独立校验");
        assert!(
            error.contains("冻结") || error.contains("快照") || error.contains("损坏"),
            "实际：{error}"
        );
    }

    // ---------- 批次 G3a 移植：状态机（import-run-state-machine.test.ts） ----------

    fn state_chapter(number: i64) -> ImportRunChapterInput {
        let content = format!("reference-{number}");
        ImportRunChapterInput {
            number,
            source_index: None,
            source_chapter_number: None,
            title: format!("Chapter {number}"),
            content_fingerprint: sha256_hex(&content),
            content_size: content.len() as i64,
            content,
        }
    }

    /// `beforeEach`：两章 reference 运行（`state-run`）并启动执行租约。
    fn prepared_state_run(conn: &Connection) -> ImportRunExecutionLease {
        let mut request = leased_run_request();
        request.run_id = "state-run".to_string();
        request.source_display = vec![ImportSourceDisplayMetadata {
            display_name: "reference.txt".to_string(),
            media_type: "text/plain".to_string(),
            size: 22,
        }];
        request.chapters = vec![state_chapter(1), state_chapter(2)];
        prepare(conn, &request).unwrap();
        start_or_resume(conn, "state-run", "state-test", now_ms(), 60_000)
            .unwrap()
            .execution
    }

    /// 逐章落地参照知识收据，返回章批次 checkpoint ID
    /// （对齐 `createImportRunChapterBatchCheckpointId`）。
    fn commit_knowledge(
        conn: &Connection,
        execution: &ImportRunExecutionLease,
        chapters: &[ImportRunChapterInput],
    ) -> String {
        let authority = authority_of(execution);
        for chapter in chapters {
            let binding = resolve_reference_import_authority(
                conn,
                "state-run",
                &authority,
                chapter.number,
                now_ms(),
            )
            .unwrap();
            let document_id = sha256_hex(&format!("reference-import:{}", binding.stable_key));
            conn.execute(
                "INSERT INTO import_reference_documents (
                    document_id, idempotency_key_hash, content_hash, chunk_set_hash,
                    expected_chunk_count, corpus_kind, state
                 ) VALUES (?, ?, ?, ?, 1, 'reference', 'committed')",
                rusqlite::params![
                    document_id,
                    sha256_hex(&binding.stable_key),
                    binding.content_fingerprint.clone(),
                    sha256_hex(&format!("chunks:{}", chapter.number)),
                ],
            )
            .unwrap();
            commit_reference_import_receipt(
                conn,
                "state-run",
                &authority,
                chapter.number,
                &document_id,
            )
            .unwrap();
        }
        let batch_chapters: Vec<(i64, String)> = chapters
            .iter()
            .map(|chapter| (chapter.number, chapter.content_fingerprint.clone()))
            .collect();
        crate::import::batch_checkpoint::create_import_run_chapter_batch_checkpoint_id(
            &batch_chapters,
        )
        .unwrap()
    }

    /// 对齐基线 `expectRejectedWithoutMutation`：动作应抛错且
    /// 运行快照不变（基线用 `serialize()` 字节比对；Rust 侧 `ImportRunSnapshot`
    /// 未派生 `PartialEq`，用 `Debug` 格式化做等价快照比对）。
    fn expect_rejected_without_mutation(
        conn: &Connection,
        action: impl FnOnce() -> Result<(), String>,
        message: &str,
    ) {
        let before = format!("{:?}", get(conn, "state-run").unwrap());
        let error = action().expect_err("应拒绝且不变更数据库");
        assert!(
            error.contains(message),
            "错误文案应含 {message}，实际：{error}"
        );
        let after = format!("{:?}", get(conn, "state-run").unwrap());
        assert_eq!(after, before, "拒绝时不应变更运行快照");
    }

    #[test]
    fn state_machine_rejects_wrong_stage_and_malformed_checkpoints_test() {
        let conn = memory_conn();
        let execution = prepared_state_run(&conn);
        expect_rejected_without_mutation(
            &conn,
            || complete_batch(&conn, "state-run", "global", "done", &execution).map(|_| ()),
            "当前阶段",
        );
        expect_rejected_without_mutation(
            &conn,
            || complete_batch(&conn, "state-run", "knowledge", "1", &execution).map(|_| ()),
            "无效",
        );
        expect_rejected_without_mutation(
            &conn,
            || complete_batch(&conn, "state-run", "knowledge", "1-3", &execution).map(|_| ()),
            "无效",
        );
    }

    #[test]
    fn state_machine_advances_only_after_stage_checkpoints_complete_test() {
        let conn = memory_conn();
        let execution = prepared_state_run(&conn);
        expect_rejected_without_mutation(
            &conn,
            || advance_stage(&conn, "state-run", "knowledge", "global", &execution).map(|_| ()),
            "checkpoint",
        );

        let chapters = vec![state_chapter(1), state_chapter(2)];
        let checkpoint = commit_knowledge(&conn, &execution, &chapters);
        complete_batch(&conn, "state-run", "knowledge", &checkpoint, &execution).unwrap();
        expect_rejected_without_mutation(
            &conn,
            || advance_stage(&conn, "state-run", "knowledge", "style", &execution).map(|_| ()),
            "下一阶段",
        );

        let advanced =
            advance_stage(&conn, "state-run", "knowledge", "global", &execution).unwrap();
        assert_eq!(advanced.stage, "global");
        assert_eq!(advanced.status, "running");
        // global/style/blueprints 是 receipt 阶段：direct 提交应拒绝
        expect_rejected_without_mutation(
            &conn,
            || complete_batch(&conn, "state-run", "global", "done", &execution).map(|_| ()),
            "receipt",
        );
        expect_rejected_without_mutation(
            &conn,
            || advance_stage(&conn, "state-run", "global", "style", &execution).map(|_| ()),
            "checkpoint",
        );
    }

    #[test]
    fn state_machine_fails_only_active_stage_test() {
        let conn = memory_conn();
        let execution = prepared_state_run(&conn);
        expect_rejected_without_mutation(
            &conn,
            || fail_run(&conn, "state-run", "global", "wrong stage", &execution).map(|_| ()),
            "当前阶段",
        );

        let failed = fail_run(
            &conn,
            "state-run",
            "knowledge",
            "provider unavailable",
            &execution,
        )
        .unwrap();
        assert_eq!(failed.stage, "knowledge");
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.last_error, "provider unavailable");
        let error = complete_batch(&conn, "state-run", "knowledge", "1-2", &execution)
            .expect_err("失败后旧执行器应被围栏");
        assert!(error.contains("执行租约"), "实际：{error}");
    }

    #[test]
    fn state_machine_rejects_mismatched_blueprint_receipt_keys_test() {
        let conn = memory_conn();
        let execution = prepared_state_run(&conn);
        conn.execute(
            "UPDATE import_runs SET stage = 'blueprints' WHERE id = 'state-run'",
            [],
        )
        .unwrap();
        let payload = serde_json::json!({
            "mode": "replace-range",
            "operationId": "state-blueprint-operation",
            "startChapter": 1,
            "endChapter": 2,
            "blueprints": [
                { "chapterNumber": 1, "title": "One" },
                { "chapterNumber": 2, "title": "Two" },
            ],
        });
        expect_rejected_without_mutation(
            &conn,
            || {
                prepare_effect_receipt(
                    &conn,
                    &ImportRunPrepareEffectReceiptRequest {
                        run_id: "state-run".to_string(),
                        stage: "blueprints".to_string(),
                        batch_id: "1-2".to_string(),
                        effect_key: "blueprints:1-2".to_string(),
                        kind: ImportRunEffectKind::ChapterBlueprintRange,
                        payload: payload.clone(),
                    },
                    &execution,
                    now_ms(),
                )
                .map(|_| ())
            },
            "无效",
        );

        // 章批次 checkpoint ID：`1-2-{fp1[..8]}.{fp2[..8]}`
        // （章节内容 `reference-1` / `reference-2`）
        let checkpoint =
            crate::import::batch_checkpoint::create_import_run_chapter_batch_checkpoint_id(&[
                (1, sha256_hex("reference-1")),
                (2, sha256_hex("reference-2")),
            ])
            .unwrap();
        expect_rejected_without_mutation(
            &conn,
            || {
                prepare_effect_receipt(
                    &conn,
                    &ImportRunPrepareEffectReceiptRequest {
                        run_id: "state-run".to_string(),
                        stage: "blueprints".to_string(),
                        batch_id: checkpoint.clone(),
                        effect_key: "blueprints:mismatched".to_string(),
                        kind: ImportRunEffectKind::ChapterBlueprintRange,
                        payload: payload.clone(),
                    },
                    &execution,
                    now_ms(),
                )
                .map(|_| ())
            },
            "损坏",
        );
        let mismatched_payload = serde_json::json!({
            "mode": "replace-range",
            "operationId": "state-blueprint-operation",
            "startChapter": 2,
            "endChapter": 3,
            "blueprints": [
                { "chapterNumber": 1, "title": "One" },
                { "chapterNumber": 2, "title": "Two" },
            ],
        });
        expect_rejected_without_mutation(
            &conn,
            || {
                prepare_effect_receipt(
                    &conn,
                    &ImportRunPrepareEffectReceiptRequest {
                        run_id: "state-run".to_string(),
                        stage: "blueprints".to_string(),
                        batch_id: checkpoint.clone(),
                        effect_key: format!("blueprints:{checkpoint}"),
                        kind: ImportRunEffectKind::ChapterBlueprintRange,
                        payload: mismatched_payload,
                    },
                    &execution,
                    now_ms(),
                )
                .map(|_| ())
            },
            "损坏",
        );
    }

    #[test]
    fn state_machine_completes_only_running_refresh_stage_test() {
        let conn = memory_conn();
        let execution = prepared_state_run(&conn);
        expect_rejected_without_mutation(
            &conn,
            || complete_run(&conn, "state-run", &execution).map(|_| ()),
            "refresh",
        );

        conn.execute(
            "UPDATE import_runs
             SET stage = 'refresh', completed_batches_json = '{}'
             WHERE id = 'state-run'",
            [],
        )
        .unwrap();
        expect_rejected_without_mutation(
            &conn,
            || complete_run(&conn, "state-run", &execution).map(|_| ()),
            "checkpoint",
        );

        complete_batch(&conn, "state-run", "refresh", "done", &execution).unwrap();
        let completed = complete_run(&conn, "state-run", &execution).unwrap();
        assert_eq!(completed.stage, "completed");
        assert_eq!(completed.status, "completed");
        assert!(!completed.resumable);
        let error = fail_run(&conn, "state-run", "refresh", "late failure", &execution)
            .expect_err("完成后写入应拒绝");
        assert!(error.contains("执行租约"), "实际：{error}");
    }

    // ==================== effect receipts（基线 import-run-receipt.test.ts） ====================

    /// 基线 `const content = 'reference'`
    const RECEIPT_CONTENT: &str = "reference";

    /// 基线 `beforeEach` 的 prepare 请求（单章 reference 运行 `receipt-run`）
    fn receipt_run_request() -> ImportRunPrepareRequest {
        ImportRunPrepareRequest {
            run_id: "receipt-run".to_string(),
            purpose: ImportPurpose::Reference,
            source_fingerprint: "a".repeat(64),
            source_ids: None,
            source_fingerprints: None,
            source_display: vec![ImportSourceDisplayMetadata {
                display_name: "book.txt".to_string(),
                media_type: "text/plain".to_string(),
                size: RECEIPT_CONTENT.len() as i64,
            }],
            locale: ImportRunLocale::EnUs,
            authority_fingerprint: None,
            expected_manifest_fingerprint: None,
            chapters: vec![ImportRunChapterInput {
                number: 1,
                source_index: None,
                source_chapter_number: None,
                title: "One".to_string(),
                content_fingerprint: sha256_hex(RECEIPT_CONTENT),
                content_size: RECEIPT_CONTENT.len() as i64,
                content: RECEIPT_CONTENT.to_string(),
            }],
        }
    }

    /// 基线 `beforeEach`：独立目录（`mkdtempSync` 等价物）+ 文件库 +
    /// 主台账初始化 + `receipt-run` 准备。目录供「关闭重开 / 离线篡改」语义使用。
    fn receipt_file_conn(label: &str) -> (std::path::PathBuf, Connection) {
        let dir = unique_temp_dir(label);
        let conn = file_conn(&dir);
        project_core_repository::init(&conn, "Receipt Test", "zh-CN").expect("主台账初始化应成功");
        prepare(&conn, &receipt_run_request()).expect("receipt-run 应准备成功");
        (dir, conn)
    }

    /// 基线 `afterEach`：关闭连接并删除临时目录
    fn cleanup_receipt_dir(conn: Connection, dir: &std::path::Path) {
        drop(conn);
        std::fs::remove_dir_all(dir).ok();
    }

    /// 基线 `tamperOffline`：模拟「关闭项目库 → 直连篡改 → 重新打开」。
    /// 调用前须先 `drop` 连接（对齐 `closeProjectDatabase`）。
    fn tamper_offline(dir: &std::path::Path, sql: &str) {
        let offline = Connection::open(dir.join("lorekeeper-g3a.db")).expect("离线篡改库应可打开");
        offline.execute_batch(sql).expect("篡改 SQL 应执行成功");
        drop(offline);
    }

    /// 基线 `moveRunToStyle` / `prepareCommitted*Receipt` 的阶段推进 SQL
    fn move_run_to_stage(conn: &Connection, stage: &str, completed_batches_json: &str) {
        conn.execute(
            "UPDATE import_runs SET stage = ?1, completed_batches_json = ?2 \
             WHERE id = 'receipt-run'",
            rusqlite::params![stage, completed_batches_json],
        )
        .unwrap();
    }

    /// 基线 `prepareStyleReceipt`：推进到 style 并 prepare writing-style receipt
    fn prepare_style_receipt(conn: &Connection, now: i64) -> ImportRunExecutionLease {
        let started = start_or_resume(conn, "receipt-run", "renderer-a", now, 60_000)
            .expect("执行租约应获取");
        move_run_to_stage(conn, "style", r#"{"knowledge":["1-1"],"global":["done"]}"#);
        prepare_effect_receipt(
            conn,
            &ImportRunPrepareEffectReceiptRequest {
                run_id: "receipt-run".to_string(),
                stage: "style".to_string(),
                batch_id: "done".to_string(),
                effect_key: "writing-style".to_string(),
                kind: ImportRunEffectKind::ProjectWritingStyle,
                payload: serde_json::json!({ "writingStyle": "Frozen generated style" }),
            },
            &started.execution,
            now,
        )
        .expect("style receipt 应 prepare");
        started.execution
    }

    /// 基线 `blueprint` 常量（单章蓝图载荷）
    fn receipt_blueprint() -> serde_json::Value {
        serde_json::json!({
            "chapterNumber": 1,
            "title": "Chapter One",
            "role": "setup",
            "purpose": "Introduce the conflict",
            "keyEvents": "The protagonist discovers a clue",
            "characters": ["Protagonist"],
            "suspenseHook": "Someone knocks at the door",
            "userGuidance": "",
            "notes": "",
            "notesUpdatedAt": "",
        })
    }

    /// 基线 `prepareCommittedBlueprintReceipt`：推进到 blueprints 并
    /// prepare + commit 章节蓝图范围 receipt。返回批次 checkpoint ID。
    fn prepare_committed_blueprint_receipt(conn: &Connection, now: i64) -> String {
        // 批次 checkpoint ID 格式 `{start}-{end}-{fp8}`，由共享件创建（勿手写 format）
        let batch_id =
            crate::import::batch_checkpoint::create_import_run_chapter_batch_checkpoint_id(&[(
                1,
                sha256_hex(RECEIPT_CONTENT),
            )])
            .expect("章批次 checkpoint ID 应可创建");
        let started = start_or_resume(conn, "receipt-run", "renderer-a", now, 60_000)
            .expect("执行租约应获取");
        move_run_to_stage(
            conn,
            "blueprints",
            r#"{"knowledge":["1-1"],"global":["done"],"style":["done"]}"#,
        );
        prepare_effect_receipt(
            conn,
            &ImportRunPrepareEffectReceiptRequest {
                run_id: "receipt-run".to_string(),
                stage: "blueprints".to_string(),
                batch_id: batch_id.clone(),
                effect_key: format!("blueprints:{batch_id}"),
                kind: ImportRunEffectKind::ChapterBlueprintRange,
                payload: serde_json::json!({
                    "mode": "replace-range",
                    "operationId": "import-blueprints-receipt-run-1-1",
                    "startChapter": 1,
                    "endChapter": 1,
                    "blueprints": [receipt_blueprint()],
                }),
            },
            &started.execution,
            now,
        )
        .expect("blueprints receipt 应 prepare");
        commit_effect_receipt(
            conn,
            "",
            "receipt-run",
            "blueprints",
            &batch_id,
            &started.execution,
            now,
        )
        .expect("blueprints receipt 应 commit");
        batch_id
    }

    /// 基线 `prepareCommittedGlobalReceipt`：推进到 global 并
    /// prepare + commit 全局事实 receipt
    fn prepare_committed_global_receipt(conn: &Connection, now: i64) {
        let started = start_or_resume(conn, "receipt-run", "renderer-a", now, 60_000)
            .expect("执行租约应获取");
        move_run_to_stage(conn, "global", r#"{"knowledge":["1-1"]}"#);
        prepare_effect_receipt(
            conn,
            &ImportRunPrepareEffectReceiptRequest {
                run_id: "receipt-run".to_string(),
                stage: "global".to_string(),
                batch_id: "done".to_string(),
                effect_key: "global-facts".to_string(),
                kind: ImportRunEffectKind::ProjectGlobalFacts,
                payload: serde_json::json!({
                    "operationId": "novel-import-global-receipt-run",
                    "expectedRosterRevision": 0,
                    "core": {
                        "genre": "Literary",
                        "subGenre": "Drama",
                        "targetAudience": "General",
                        "totalChapters": 1,
                        "wordsPerChapter": 2000,
                        "plotStructure": "three_act",
                        "narrativePov": "third_limited",
                        "goldenFinger": "None",
                        "globalGuidance": "Concise",
                        "coreOutline": "A conflict unfolds",
                        "worldSetting": "A small town",
                        "protagonistProfile": "A careful observer",
                        "premise": "Truth has a cost",
                        "worldbuilding": "Contemporary",
                        "synopsis": "A discovery changes a life",
                    },
                    "characterEntries": [{
                        "name": "Protagonist",
                        "role": "protagonist",
                        "gender": "",
                        "age": "",
                        "appearance": "",
                        "personality": "",
                        "background": "",
                        "abilities": "",
                        "motivation": "",
                        "relationships": [],
                        "arc": "",
                        "notes": "",
                    }],
                }),
            },
            &started.execution,
            now,
        )
        .expect("global receipt 应 prepare");
        commit_effect_receipt(
            conn,
            "",
            "receipt-run",
            "global",
            "done",
            &started.execution,
            now,
        )
        .expect("global receipt 应 commit");
    }

    /// 基线角色同步名单提交载荷（`blueprint_sync` 意图）
    fn roster_sync_payload(operation_id: &str) -> serde_json::Value {
        serde_json::json!({
            "operationId": operation_id,
            "expectedRevision": 0,
            "schemaVersion": 1,
            "intent": "blueprint_sync",
            "entries": [{
                "name": "Protagonist",
                "role": "supporting",
                "gender": "",
                "age": "",
                "appearance": "",
                "personality": "",
                "background": "",
                "abilities": "",
                "motivation": "",
                "relationships": [],
                "arc": "",
                "notes": "",
            }],
        })
    }

    /// 基线 `getProjectDb().prepare('SELECT stage, status ...').get()`
    fn receipt_run_stage_status(conn: &Connection) -> (String, String) {
        conn.query_row(
            "SELECT stage, status FROM import_runs WHERE id = 'receipt-run'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    }

    /// 基线 `expect(() => getEffectReceipt(...)).toThrow(/receipt.*损坏|收据.*损坏/i)`
    /// （`rowToEffectReceipt` 把任何非「损坏」错误统一转为 corruptedReceipt）
    fn expect_receipt_corrupted(conn: &Connection, stage: &str, batch_id: &str, context: &str) {
        let error = match get_effect_receipt(conn, "receipt-run", stage, batch_id) {
            Err(error) => error,
            Ok(_) => panic!("{context}：损坏 receipt 应拒绝重放"),
        };
        assert!(
            error.contains("receipt 损坏"),
            "{context}：错误应为 receipt 损坏，实际：{error}"
        );
    }

    #[test]
    fn receipt_reopens_committed_blueprint_after_durable_character_sync_test() {
        let (dir, conn) = receipt_file_conn("receipt-blueprint-reopen");
        let now = now_ms();
        let batch_id = prepare_committed_blueprint_receipt(&conn, now);

        let operation = blueprint_repository::get_committed_range_operation(
            &conn,
            "import-blueprints-receipt-run-1-1",
        )
        .expect("已提交范围操作应回读")
        .expect("范围操作应存在");
        assert_eq!(operation.character_sync_operation.status, "pending");

        character_roster_repository::commit(
            &conn,
            &roster_sync_payload(&operation.character_sync_operation.operation_id),
        )
        .expect("角色同步名单应提交");
        let completed = blueprint_repository::complete_character_sync_operation(
            &conn,
            &operation.character_sync_operation.operation_id,
        )
        .expect("角色同步操作应完成");
        assert_eq!(completed.status, "completed");

        // 基线 `closeProjectDatabase` + `initProjectDatabase`
        drop(conn);
        let conn = file_conn(&dir);

        let receipt = get_effect_receipt(&conn, "receipt-run", "blueprints", &batch_id)
            .expect("reopen 后 receipt 应重放")
            .expect("committed receipt 应存在");
        assert_eq!(receipt.state, "committed");
        // 存储回执冻结的是提交时刻的同步快照（pending），权威事实已演进为 completed
        let sync_status = receipt
            .effect_receipt
            .as_ref()
            .expect("committed 收据应有回执")
            .get("characterSyncOperation")
            .expect("应有角色同步操作")
            .get("status")
            .expect("应有状态")
            .as_str()
            .expect("状态应为文本");
        assert_eq!(sync_status, "pending");

        cleanup_receipt_dir(conn, &dir);
    }

    #[test]
    fn receipt_rejects_offline_forged_sync_receipt_without_roster_proof_test() {
        let (dir, conn) = receipt_file_conn("receipt-forged-sync");
        let now = now_ms();
        let batch_id = prepare_committed_blueprint_receipt(&conn, now);

        // 提交一个「不相干」的名单操作：权威证据查不到同步操作的名单提交，
        // 权威回执将降级为 already-satisfied，与离线伪造的 committed 不匹配
        character_roster_repository::commit(
            &conn,
            &roster_sync_payload("different-roster-operation"),
        )
        .expect("无关名单操作应提交");
        drop(conn);
        tamper_offline(
            &dir,
            &format!(
                r#"
      UPDATE blueprint_character_sync_operations
      SET status = 'completed',
          completion_receipt = json_object(
            'blueprintCommitOperationId', 'import-blueprints-receipt-run-1-1',
            'operationId', 'blueprint-sync-import-blueprints-receipt-run-1-1',
            'status', 'committed',
            'rosterReceipt', json_object(
              'operationId', 'blueprint-sync-import-blueprints-receipt-run-1-1',
              'payloadHash', '{forged_hash}',
              'revision', 1,
              'idempotent', json('false')
            )
          ),
          completed_at = created_at
      WHERE operation_id = 'blueprint-sync-import-blueprints-receipt-run-1-1'
      "#,
                forged_hash = "f".repeat(64),
            ),
        );
        let conn = file_conn(&dir);
        expect_receipt_corrupted(&conn, "blueprints", &batch_id, "离线伪造同步回执");

        cleanup_receipt_dir(conn, &dir);
    }

    #[test]
    fn receipt_accepts_already_satisfied_sync_completion_test() {
        let (dir, conn) = receipt_file_conn("receipt-already-satisfied");
        let now = now_ms();
        let batch_id = prepare_committed_blueprint_receipt(&conn, now);
        drop(conn);
        // 无名单提交的已完成同步：权威回执同样是 already-satisfied（无 rosterReceipt），
        // 与离线补齐的完成回执一致，应接受重放
        tamper_offline(
            &dir,
            r#"
      UPDATE blueprint_character_sync_operations
      SET status = 'completed',
          completion_receipt = json_object(
            'blueprintCommitOperationId', 'import-blueprints-receipt-run-1-1',
            'operationId', 'blueprint-sync-import-blueprints-receipt-run-1-1',
            'status', 'already-satisfied'
          ),
          completed_at = created_at
      WHERE operation_id = 'blueprint-sync-import-blueprints-receipt-run-1-1'
      "#,
        );
        let conn = file_conn(&dir);
        let receipt = get_effect_receipt(&conn, "receipt-run", "blueprints", &batch_id)
            .expect("reopen 后 receipt 应重放")
            .expect("committed receipt 应存在");
        assert_eq!(receipt.state, "committed");

        cleanup_receipt_dir(conn, &dir);
    }

    #[test]
    fn receipt_freezes_output_before_commit_and_commits_atomically_after_reopen_test() {
        let (dir, conn) = receipt_file_conn("receipt-style-freeze");
        let now = now_ms();
        let started = start_or_resume(&conn, "receipt-run", "renderer-a", now, 60_000)
            .expect("执行租约应获取");
        move_run_to_stage(&conn, "style", r#"{"knowledge":["1-1"],"global":["done"]}"#);
        let prepared = prepare_effect_receipt(
            &conn,
            &ImportRunPrepareEffectReceiptRequest {
                run_id: "receipt-run".to_string(),
                stage: "style".to_string(),
                batch_id: "done".to_string(),
                effect_key: "writing-style".to_string(),
                kind: ImportRunEffectKind::ProjectWritingStyle,
                payload: serde_json::json!({ "writingStyle": "Frozen generated style" }),
            },
            &started.execution,
            now,
        )
        .expect("style receipt 应 prepare");
        assert_eq!(prepared.state, "prepared");
        assert_eq!(prepared.payload_hash.len(), 64);
        assert!(prepared
            .payload_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));
        // prepare 不落地主台账：生成输出在 commit 前保持冻结
        assert_eq!(
            project_core_repository::get(&conn)
                .expect("主台账应可读")
                .expect("主台账应存在")
                .writing_style,
            ""
        );

        // 基线 `closeProjectDatabase` + `initProjectDatabase`
        drop(conn);
        let conn = file_conn(&dir);
        let started = start_or_resume(&conn, "receipt-run", "renderer-a", now_ms(), 60_000)
            .expect("reopen 后执行租约应续期");
        let reopened = get_effect_receipt(&conn, "receipt-run", "style", "done")
            .expect("reopen 后 receipt 应重放")
            .expect("prepared receipt 应存在");
        assert_eq!(reopened.state, "prepared");

        let committed = commit_effect_receipt(
            &conn,
            "",
            "receipt-run",
            "style",
            "done",
            &started.execution,
            now_ms(),
        )
        .expect("style receipt 应 commit");
        assert_eq!(
            project_core_repository::get(&conn)
                .expect("主台账应可读")
                .expect("主台账应存在")
                .writing_style,
            "Frozen generated style"
        );
        assert_eq!(committed.receipt.state, "committed");
        assert_eq!(
            committed
                .run
                .completed_batches
                .get("style")
                .map(Vec::as_slice),
            Some(&["done".to_string()][..]),
            "commit 后 style 批次应记为完成"
        );

        // 幂等重放：已 committed 的 receipt 直接回读
        let replayed = commit_effect_receipt(
            &conn,
            "",
            "receipt-run",
            "style",
            "done",
            &started.execution,
            now_ms(),
        )
        .expect("已提交 receipt 应幂等重放");
        assert_eq!(replayed.receipt.state, "committed");

        cleanup_receipt_dir(conn, &dir);
    }

    #[test]
    fn receipt_binds_key_to_one_payload_and_rejects_stale_lease_test() {
        let (dir, conn) = receipt_file_conn("receipt-payload-binding");
        let base = now_ms();
        let first =
            start_or_resume(&conn, "receipt-run", "renderer-a", base, 100).expect("执行租约应获取");
        move_run_to_stage(&conn, "style", r#"{"knowledge":["1-1"],"global":["done"]}"#);
        let request = || ImportRunPrepareEffectReceiptRequest {
            run_id: "receipt-run".to_string(),
            stage: "style".to_string(),
            batch_id: "done".to_string(),
            effect_key: "writing-style".to_string(),
            kind: ImportRunEffectKind::ProjectWritingStyle,
            payload: serde_json::json!({ "writingStyle": "First" }),
        };
        prepare_effect_receipt(&conn, &request(), &first.execution, base + 1)
            .expect("首个 receipt 应 prepare");
        let error = prepare_effect_receipt(
            &conn,
            &ImportRunPrepareEffectReceiptRequest {
                payload: serde_json::json!({ "writingStyle": "Different" }),
                ..request()
            },
            &first.execution,
            base + 2,
        )
        .expect_err("同一键绑定不同载荷应拒绝");
        assert!(error.contains("不同载荷"), "实际：{error}");

        // renderer-b 接管后，旧执行器的租约过期，commit 应被围栏
        start_or_resume(&conn, "receipt-run", "renderer-b", base + 101, 100).expect("接管应成功");
        let error = commit_effect_receipt(
            &conn,
            "",
            "receipt-run",
            "style",
            "done",
            &first.execution,
            base + 102,
        )
        .expect_err("过期执行租约应拒绝");
        assert!(error.contains("执行租约"), "实际：{error}");

        cleanup_receipt_dir(conn, &dir);
    }

    #[test]
    fn receipt_fails_closed_when_stored_receipt_columns_tampered_test() {
        // 基线 `it.each` 六项：存储收据的各列被离线篡改后，重放必须失败关闭
        let cases: Vec<(&str, String)> = vec![
            (
                "payload",
                r#"UPDATE import_run_receipts SET payload_json = '{"writingStyle":"tampered"}';"#
                    .to_string(),
            ),
            (
                "hash",
                format!(
                    "UPDATE import_run_receipts SET payload_hash = '{}';",
                    "0".repeat(64)
                ),
            ),
            (
                "namespace",
                "UPDATE import_run_receipts SET effect_namespace = 'import:reference:other';"
                    .to_string(),
            ),
            (
                "effect key",
                "UPDATE import_run_receipts SET effect_key = 'other-key';".to_string(),
            ),
            (
                "kind/stage binding",
                "UPDATE import_run_receipts SET kind = 'project-global-facts';".to_string(),
            ),
            (
                "schema version",
                "UPDATE import_run_receipts SET schema_version = 2;".to_string(),
            ),
        ];
        for (label, sql) in cases {
            let (dir, conn) = receipt_file_conn("receipt-column-tamper");
            let now = now_ms();
            let _ = prepare_style_receipt(&conn, now);
            drop(conn);
            tamper_offline(&dir, &sql);
            let conn = file_conn(&dir);
            expect_receipt_corrupted(&conn, "style", "done", label);
            cleanup_receipt_dir(conn, &dir);
        }
    }

    #[test]
    fn receipt_fails_closed_when_committed_effect_result_invalid_for_kind_test() {
        let (dir, conn) = receipt_file_conn("receipt-effect-result");
        let now = now_ms();
        let execution = prepare_style_receipt(&conn, now);
        commit_effect_receipt(&conn, "", "receipt-run", "style", "done", &execution, now)
            .expect("style receipt 应 commit");
        drop(conn);
        tamper_offline(
            &dir,
            "UPDATE import_run_receipts SET effect_receipt_json = '{}';",
        );
        let conn = file_conn(&dir);
        expect_receipt_corrupted(&conn, "style", "done", "committed 结果与类型不匹配");

        cleanup_receipt_dir(conn, &dir);
    }

    #[test]
    fn receipt_fails_closed_when_blueprint_receipt_swaps_nested_sync_operation_test() {
        let (dir, conn) = receipt_file_conn("receipt-sync-swap");
        let now = now_ms();
        let batch_id = prepare_committed_blueprint_receipt(&conn, now);
        drop(conn);
        tamper_offline(
            &dir,
            r#"
      INSERT INTO blueprint_commit_operations (
        operation_id, payload_hash, mode, start_chapter, end_chapter, character_sync_input
      ) SELECT
        'import-blueprints-another-run-1-1', payload_hash, mode, start_chapter, end_chapter, character_sync_input
      FROM blueprint_commit_operations
      WHERE operation_id = 'import-blueprints-receipt-run-1-1';
      INSERT INTO blueprint_character_sync_operations (
        operation_id, blueprint_commit_operation_id, blueprint_commit_payload_hash,
        status, start_chapter, end_chapter, character_sync_input
      ) SELECT
        'blueprint-sync-import-blueprints-another-run-1-1',
        'import-blueprints-another-run-1-1', blueprint_commit_payload_hash,
        status, start_chapter, end_chapter, character_sync_input
      FROM blueprint_character_sync_operations
      WHERE operation_id = 'blueprint-sync-import-blueprints-receipt-run-1-1';
      UPDATE import_run_receipts
      SET effect_receipt_json = json_set(
        effect_receipt_json,
        '$.characterSyncOperation.operationId',
        'blueprint-sync-import-blueprints-another-run-1-1',
        '$.characterSyncOperation.blueprintCommitOperationId',
        'import-blueprints-another-run-1-1'
      );
      "#,
        );
        let conn = file_conn(&dir);
        expect_receipt_corrupted(&conn, "blueprints", &batch_id, "嵌套同步操作被替换");
        // 校验失败不产生副作用：运行保持原阶段与状态
        let (stage, status) = receipt_run_stage_status(&conn);
        assert_eq!(stage, "blueprints");
        assert_eq!(status, "running");

        cleanup_receipt_dir(conn, &dir);
    }

    #[test]
    fn receipt_fails_closed_when_authoritative_blueprint_tampered_test() {
        // 基线 `it.each` 三项：权威蓝图证据被离线篡改后，重放必须失败关闭
        let cases: Vec<(&str, String)> = vec![
            (
                "payload hash",
                format!(
                    "UPDATE blueprint_commit_operations SET payload_hash = '{}' \
                     WHERE operation_id = 'import-blueprints-receipt-run-1-1';",
                    "0".repeat(64)
                ),
            ),
            (
                "range",
                "UPDATE blueprint_commit_operations SET end_chapter = 2 \
                 WHERE operation_id = 'import-blueprints-receipt-run-1-1';"
                    .to_string(),
            ),
            (
                "sync status",
                "UPDATE blueprint_character_sync_operations SET status = 'completed' \
                 WHERE operation_id = 'blueprint-sync-import-blueprints-receipt-run-1-1';"
                    .to_string(),
            ),
        ];
        for (label, sql) in cases {
            let (dir, conn) = receipt_file_conn("receipt-authority-tamper");
            let now = now_ms();
            let batch_id = prepare_committed_blueprint_receipt(&conn, now);
            drop(conn);
            tamper_offline(&dir, &sql);
            let conn = file_conn(&dir);
            expect_receipt_corrupted(&conn, "blueprints", &batch_id, label);
            let (stage, status) = receipt_run_stage_status(&conn);
            assert_eq!(stage, "blueprints", "{label}：阶段不应变更");
            assert_eq!(status, "running", "{label}：状态不应变更");
            cleanup_receipt_dir(conn, &dir);
        }
    }

    #[test]
    fn receipt_does_not_recreate_deleted_authoritative_operation_test() {
        let (dir, conn) = receipt_file_conn("receipt-deleted-operation");
        let now = now_ms();
        let batch_id = prepare_committed_blueprint_receipt(&conn, now);
        drop(conn);
        tamper_offline(
            &dir,
            "DELETE FROM blueprint_commit_operations \
             WHERE operation_id = 'import-blueprints-receipt-run-1-1';",
        );
        let conn = file_conn(&dir);
        expect_receipt_corrupted(&conn, "blueprints", &batch_id, "权威操作被删除");
        // 重放校验不得重建被删除的权威操作
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM blueprint_commit_operations \
                 WHERE operation_id = 'import-blueprints-receipt-run-1-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);

        cleanup_receipt_dir(conn, &dir);
    }

    #[test]
    fn receipt_rejects_committed_blueprint_with_tampered_receipt_test() {
        // 基线 `it.each` 四项：committed 收据本体被离线篡改后，重放必须失败关闭
        let cases: Vec<(&str, String)> = vec![
            (
                "extra key",
                "UPDATE import_run_receipts SET effect_receipt_json = \
                 json_set(effect_receipt_json, '$.unexpected', 1);"
                    .to_string(),
            ),
            (
                "missing key",
                "UPDATE import_run_receipts SET effect_receipt_json = \
                 json_remove(effect_receipt_json, '$.payloadHash');"
                    .to_string(),
            ),
            (
                "nested payload hash",
                format!(
                    "UPDATE import_run_receipts SET effect_receipt_json = \
                     json_set(effect_receipt_json, \
                     '$.characterSyncOperation.blueprintCommitPayloadHash', '{}');",
                    "0".repeat(64)
                ),
            ),
            (
                "range",
                "UPDATE import_run_receipts SET effect_receipt_json = \
                 json_set(effect_receipt_json, '$.endChapter', 2);"
                    .to_string(),
            ),
        ];
        for (label, sql) in cases {
            let (dir, conn) = receipt_file_conn("receipt-receipt-tamper");
            let now = now_ms();
            let batch_id = prepare_committed_blueprint_receipt(&conn, now);
            drop(conn);
            tamper_offline(&dir, &sql);
            let conn = file_conn(&dir);
            expect_receipt_corrupted(&conn, "blueprints", &batch_id, label);
            // 校验失败不产生副作用：角色同步操作保持 pending
            let sync_status: String = conn
                .query_row(
                    "SELECT status FROM blueprint_character_sync_operations \
                     WHERE operation_id = 'blueprint-sync-import-blueprints-receipt-run-1-1'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(sync_status, "pending", "{label}：同步操作不应变更");
            cleanup_receipt_dir(conn, &dir);
        }
    }

    #[test]
    fn receipt_rejects_global_receipt_with_changed_ledger_hash_test() {
        let (dir, conn) = receipt_file_conn("receipt-global-ledger");
        let now = now_ms();
        prepare_committed_global_receipt(&conn, now);
        drop(conn);
        tamper_offline(
            &dir,
            &format!(
                "UPDATE import_global_fact_operations SET payload_hash = '{}' \
                 WHERE operation_id = 'novel-import-global-receipt-run';",
                "0".repeat(64)
            ),
        );
        let conn = file_conn(&dir);
        expect_receipt_corrupted(&conn, "global", "done", "全局事实台账哈希被篡改");

        cleanup_receipt_dir(conn, &dir);
    }
}
