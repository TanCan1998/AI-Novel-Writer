//! 定稿连续性投影（finalized continuity）—— 批次 E。
//!
//! 平移自 `electron/repositories/summary-repository.ts` 的定稿连续性段落 +
//! `continuity_projection_meta` 代际指针。领域约束（`docs/product-domain.md`）：
//! - 投影是**派生数据**，写入时必须绑定不可变定稿来源（outbox 收据 + 内容哈希）；
//! - `generation` 为全局失效水位：源正文变化即推进，过期结果按水位拒绝；
//! - 引文（`evidence`）必须能在绑定正文中精确定位，否则拒绝落盘。
//!
//! 与基线的一致性：错误文案逐字对齐；`parseFacts` / `parseCandidates` 对
//! 历史脏数据**静默降级**（解析失败返回空），只有入参校验失败才报错。

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::repositories::character_repository::{CharacterStateTextField, FinalizedSourceIdentity};
use crate::repositories::character_roster_repository as roster;

const FACT_CATEGORIES: [&str; 4] = ["character-state", "timeline", "open-thread", "plot"];

pub(crate) fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ===== 共享类型（对齐 `src/shared/finalized-continuity.ts`） =====

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedContinuityFact {
    pub category: String,
    #[serde(default)]
    pub entities: Vec<String>,
    pub statement: String,
    pub source_chapter: i64,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedCharacterStateCandidate {
    pub character_name: String,
    pub field: CharacterStateTextField,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedSourceSnapshot {
    pub source: FinalizedSourceIdentity,
    pub chapter_title: String,
    pub content: String,
    pub projection_generation: i64,
}

/// 对齐契约 `FinalizedSourceReadResult`（discriminated union，`status` 为判别键）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum FinalizedSourceReadResult {
    Valid { snapshot: FinalizedSourceSnapshot },
    Legacy {
        draft_id: i64,
        chapter_number: i64,
        chapter_title: String,
        content: String,
    },
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedContinuityProjection {
    pub draft_id: i64,
    pub chapter_number: i64,
    pub chapter_title: String,
    pub chapter_notes: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facts: Option<Vec<FinalizedContinuityFact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_state_candidates: Option<Vec<FinalizedCharacterStateCandidate>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<FinalizedSourceIdentity>,
    pub source_status: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFinalizedContinuityRequest {
    pub draft_id: i64,
    pub chapter_number: i64,
    #[serde(default)]
    pub chapter_notes: String,
    #[serde(default)]
    pub facts: Option<Vec<serde_json::Value>>,
    pub projection_generation: i64,
    pub source: FinalizedSourceIdentity,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFinalizedCharacterStateCandidatesRequest {
    pub draft_id: i64,
    pub chapter_number: i64,
    #[serde(default)]
    pub candidates: Option<Vec<serde_json::Value>>,
    pub projection_generation: i64,
    pub source: FinalizedSourceIdentity,
}

// ===== 内部工具 =====

fn same_source(left: &FinalizedSourceIdentity, right: &FinalizedSourceIdentity) -> bool {
    left == right
}

/// 对齐基线 `normalizedFacts`：逐条宽松校验后规范化（实体去重保序）
fn normalized_facts(
    value: &[serde_json::Value],
    chapter_number: i64,
) -> Result<Vec<FinalizedContinuityFact>, String> {
    if value.len() > 12 {
        return Err("连续性事实参数无效".to_string());
    }
    value
        .iter()
        .map(|fact| {
            let category = fact.get("category").and_then(|v| v.as_str()).unwrap_or("");
            let source_chapter = fact.get("sourceChapter").and_then(|v| v.as_i64());
            let entities: Vec<String> = match fact.get("entities") {
                Some(serde_json::Value::Array(arr)) => arr
                    .iter()
                    .map(|e| e.as_str().unwrap_or("").trim().to_string())
                    .collect(),
                _ => Vec::new(),
            };
            let statement = fact
                .get("statement")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let evidence = fact
                .get("evidence")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if !FACT_CATEGORIES.contains(&category)
                || source_chapter != Some(chapter_number)
                || entities.len() > 8
                || entities.iter().any(|e| e.is_empty() || e.chars().count() > 80)
                || statement.is_empty()
                || statement.chars().count() > 280
                || evidence.is_empty()
                || evidence.chars().count() > 240
            {
                return Err("连续性事实参数无效".to_string());
            }
            let mut seen = HashSet::new();
            let unique_entities = entities
                .into_iter()
                .filter(|e| seen.insert(e.clone()))
                .collect();
            Ok(FinalizedContinuityFact {
                category: category.to_string(),
                entities: unique_entities,
                statement,
                source_chapter: chapter_number,
                evidence,
            })
        })
        .collect()
}

/// 对齐基线 `parseFacts`：历史脏数据静默降级为空
fn parse_facts(value: &str, chapter_number: i64) -> Vec<FinalizedContinuityFact> {
    match serde_json::from_str::<Vec<serde_json::Value>>(value) {
        Ok(items) => normalized_facts(&items, chapter_number).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn field_matches(field: &CharacterStateTextField, text: &str) -> bool {
    let expected = serde_json::to_value(field)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    expected == text
}

/// 对齐基线 `parseCharacterStateCandidates`：逐条过滤非法项
fn parse_character_state_candidates(value: &str) -> Vec<FinalizedCharacterStateCandidate> {
    let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(value) else {
        return Vec::new();
    };
    parsed
        .into_iter()
        .filter_map(|input| {
            let name = input.get("characterName")?.as_str()?.trim().to_string();
            if name.is_empty() {
                return None;
            }
            let field_text = input.get("field")?.as_str()?;
            let field = CharacterStateTextField::ALL
                .iter()
                .copied()
                .find(|f| field_matches(f, field_text))?;
            let value = input.get("value")?.as_str()?.trim().to_string();
            Some(FinalizedCharacterStateCandidate {
                character_name: name,
                field,
                value,
            })
        })
        .collect()
}

/// 对齐基线 `normalizeCharacterStateCandidates`：按角色名单身份键归一
fn normalize_character_state_candidates(
    conn: &Connection,
    value: &[serde_json::Value],
) -> Result<Vec<FinalizedCharacterStateCandidate>, String> {
    if value.len() > 1_000 {
        return Err("角色状态候选参数无效".to_string());
    }
    let mut stmt = conn
        .prepare("SELECT name FROM characters")
        .map_err(|e| e.to_string())?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let mut by_identity = std::collections::HashMap::new();
    for name in names {
        let identity = roster::identity_key(&name);
        if identity.is_empty() || by_identity.contains_key(&identity) {
            return Err("角色名单存在同名冲突".to_string());
        }
        by_identity.insert(identity, name);
    }
    let mut normalized = std::collections::HashMap::new();
    for input in value {
        let character_name = input
            .get("characterName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let identity = roster::identity_key(&character_name);
        let stored_name = by_identity.get(&identity).cloned();
        let field_text = input.get("field").and_then(|v| v.as_str()).unwrap_or("");
        let field = CharacterStateTextField::ALL
            .iter()
            .copied()
            .find(|f| field_matches(f, field_text));
        let value_text = input.get("value").and_then(|v| v.as_str());
        let (Some(stored_name), Some(field), Some(value_text)) = (stored_name, field, value_text)
        else {
            return Err("角色状态候选参数无效".to_string());
        };
        let candidate = FinalizedCharacterStateCandidate {
            character_name: stored_name,
            field,
            value: value_text.trim().to_string(),
        };
        normalized.insert(
            format!("{}\u{0}{}", roster::identity_key(&candidate.character_name), serde_json::to_value(candidate.field).unwrap().as_str().unwrap_or("")),
            candidate,
        );
    }
    Ok(normalized.into_values().collect())
}

/// 对齐基线 `readFinalizedSourceFromDb`：正文/快照/哈希三重一致性校验
pub(crate) fn read_finalized_source_from_db(
    conn: &Connection,
    draft_id: i64,
) -> Result<Option<FinalizedSourceSnapshot>, String> {
    let row = conn
        .query_row(
            "SELECT drafts.chapter_number, drafts.status,
                    contents.body, finalization_outbox.finalization_id,
                    finalization_outbox.chapter_title,
                    finalization_outbox.content_hash,
                    finalization_outbox.content_snapshot,
                    continuity_projection_meta.generation
             FROM drafts
             JOIN contents ON contents.id = drafts.content_id
             JOIN finalization_outbox ON finalization_outbox.draft_id = drafts.id
             JOIN continuity_projection_meta ON continuity_projection_meta.id = 'main'
             WHERE drafts.id = ?1",
            [draft_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((chapter_number, status, content, finalization_id, chapter_title, content_hash, content_snapshot, generation)) = row
    else {
        return Ok(None);
    };
    let content_hash = content_hash.trim().to_string();
    let valid_hash = content_hash.len() == 64
        && content_hash
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
    if status != "finalized"
        || finalization_id.trim().is_empty()
        || !valid_hash
        || content != content_snapshot
        || sha256_hex(&content_snapshot) != content_hash
    {
        return Ok(None);
    }
    Ok(Some(FinalizedSourceSnapshot {
        source: FinalizedSourceIdentity {
            draft_id,
            finalization_id,
            chapter_number,
            content_hash,
        },
        chapter_title,
        content: content_snapshot,
        projection_generation: generation,
    }))
}

// ===== 领域操作 =====

/// 基线 `SummaryRepository.saveFinalizedContinuity`
pub fn save_finalized_continuity(
    conn: &Connection,
    request: &SaveFinalizedContinuityRequest,
) -> Result<(), String> {
    let chapter_notes = request.chapter_notes.trim().to_string();
    let empty: Vec<serde_json::Value> = Vec::new();
    let normalized = normalized_facts(request.facts.as_ref().unwrap_or(&empty), request.chapter_number)?;
    let facts = serde_json::to_string(&normalized).map_err(|e| e.to_string())?;
    if request.draft_id < 1
        || request.chapter_number < 1
        || request.projection_generation < 0
        || chapter_notes.is_empty()
    {
        return Err("连续性投影参数无效".to_string());
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    let snapshot = read_finalized_source_from_db(conn, request.draft_id)?;
    let Some(snapshot) = snapshot else {
        return Err("连续性投影来源已失效，已拒绝过期结果".to_string());
    };
    if !same_source(&snapshot.source, &request.source)
        || request.chapter_number != snapshot.source.chapter_number
    {
        return Err("连续性投影来源已失效，已拒绝过期结果".to_string());
    }
    if normalized
        .iter()
        .any(|fact| !snapshot.content.contains(&fact.evidence))
    {
        return Err("连续性事实引文无法在绑定定稿正文中精确定位".to_string());
    }
    let generation: i64 = tx
        .query_row(
            "SELECT generation FROM continuity_projection_meta WHERE id = 'main'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if request.projection_generation != generation {
        return Err("连续性投影失效水位已推进，已拒绝过期结果".to_string());
    }
    let updated = tx
        .execute(
            "UPDATE summary_snapshots
             SET chapter_number = ?1, chapter_notes = ?2, continuity_facts = ?3,
                 source_finalization_id = ?4, source_content_hash = ?5, projection_generation = ?6,
                 created_at = datetime('now')
             WHERE draft_id = ?7",
            rusqlite::params![
                request.chapter_number,
                chapter_notes,
                facts,
                request.source.finalization_id,
                request.source.content_hash,
                request.projection_generation,
                request.draft_id,
            ],
        )
        .map_err(|e| e.to_string())?;
    if updated == 0 {
        tx.execute(
            "INSERT INTO summary_snapshots (
                draft_id, chapter_number, chapter_notes, continuity_facts,
                source_finalization_id, source_content_hash, projection_generation
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                request.draft_id,
                request.chapter_number,
                chapter_notes,
                facts,
                request.source.finalization_id,
                request.source.content_hash,
                request.projection_generation,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

/// 基线 `SummaryRepository.saveFinalizedCharacterStateCandidates`
pub fn save_finalized_character_state_candidates(
    conn: &Connection,
    request: &SaveFinalizedCharacterStateCandidatesRequest,
) -> Result<(), String> {
    if request.draft_id < 1 || request.chapter_number < 1 || request.projection_generation < 0 {
        return Err("角色状态候选参数无效".to_string());
    }
    let empty: Vec<serde_json::Value> = Vec::new();
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    let snapshot = read_finalized_source_from_db(conn, request.draft_id)?;
    let Some(snapshot) = snapshot else {
        return Err("角色状态候选来源已失效，已拒绝过期结果".to_string());
    };
    if !same_source(&snapshot.source, &request.source)
        || request.chapter_number != snapshot.source.chapter_number
    {
        return Err("角色状态候选来源已失效，已拒绝过期结果".to_string());
    }
    if request.projection_generation != snapshot.projection_generation {
        return Err("连续性投影失效水位已推进，已拒绝过期结果".to_string());
    }
    let row = tx
        .query_row(
            "SELECT character_state_candidates, source_finalization_id,
                    source_content_hash, projection_generation
             FROM summary_snapshots WHERE draft_id = ?1",
            [request.draft_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((candidates_json, finalization_id, content_hash, projection_generation)) = row else {
        return Err("角色状态候选缺少同代定稿连续性投影".to_string());
    };
    if finalization_id != request.source.finalization_id
        || content_hash != request.source.content_hash
        || projection_generation != request.projection_generation
    {
        return Err("角色状态候选缺少同代定稿连续性投影".to_string());
    }

    let existing = parse_character_state_candidates(&candidates_json);
    let incoming =
        normalize_character_state_candidates(conn, request.candidates.as_ref().unwrap_or(&empty))?;
    let mut merged = std::collections::BTreeMap::new();
    for candidate in existing.into_iter().chain(incoming) {
        let key = format!(
            "{}\u{0}{}",
            roster::identity_key(&candidate.character_name),
            serde_json::to_value(candidate.field)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        );
        merged.insert(key, candidate);
    }
    tx.execute(
        "UPDATE summary_snapshots
         SET character_state_candidates = ?1, created_at = datetime('now')
         WHERE draft_id = ?2",
        rusqlite::params![
            serde_json::to_string(&merged.into_values().collect::<Vec<_>>())
                .map_err(|e| e.to_string())?,
            request.draft_id
        ],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

/// 基线 `SummaryRepository.listFinalizedContinuityBefore`
pub fn list_finalized_continuity_before(
    conn: &Connection,
    chapter_number: i64,
) -> Result<Vec<FinalizedContinuityProjection>, String> {
    if chapter_number < 1 {
        return Err("连续性投影目标章节无效".to_string());
    }
    let mut stmt = conn
        .prepare(
            "SELECT summary_snapshots.draft_id,
                    summary_snapshots.chapter_number,
                    COALESCE(finalization_outbox.chapter_title, '') AS chapter_title,
                    summary_snapshots.chapter_notes,
                    summary_snapshots.continuity_facts,
                    summary_snapshots.character_state_candidates,
                    summary_snapshots.source_finalization_id,
                    summary_snapshots.source_content_hash,
                    summary_snapshots.projection_generation,
                    finalization_outbox.finalization_id,
                    finalization_outbox.content_hash,
                    finalization_outbox.content_snapshot,
                    contents.body,
                    continuity_projection_meta.generation,
                    continuity_projection_meta.stale_from_chapter
             FROM summary_snapshots
             JOIN drafts ON drafts.id = summary_snapshots.draft_id
             JOIN contents ON contents.id = drafts.content_id
             LEFT JOIN finalization_outbox ON finalization_outbox.draft_id = drafts.id
             JOIN continuity_projection_meta ON continuity_projection_meta.id = 'main'
             WHERE summary_snapshots.draft_id IS NOT NULL
               AND summary_snapshots.chapter_number < ?1
               AND summary_snapshots.chapter_notes <> ''
               AND drafts.status = 'finalized'
               AND NOT EXISTS (
                 SELECT 1 FROM drafts newer
                 WHERE newer.chapter_number = drafts.chapter_number
                   AND newer.status = 'finalized'
                   AND (newer.version > drafts.version OR (newer.version = drafts.version AND newer.id > drafts.id))
               )
             ORDER BY summary_snapshots.chapter_number ASC, summary_snapshots.draft_id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([chapter_number], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, i64>(13)?,
                row.get::<_, Option<i64>>(14)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(
            |(draft_id, chapter_number, chapter_title, chapter_notes, facts_json, candidates_json,
              source_finalization_id, source_content_hash, projection_generation,
              current_finalization_id, current_content_hash, content_snapshot, current_content,
              current_generation, stale_from_chapter)| {
                let has_bound_source =
                    source_finalization_id.as_deref().is_some_and(|s| !s.is_empty())
                        && source_content_hash.as_deref().is_some_and(|s| !s.is_empty());
                let source_current = has_bound_source
                    && current_finalization_id.as_deref() == source_finalization_id.as_deref()
                    && current_content_hash.as_deref() == source_content_hash.as_deref()
                    && content_snapshot.as_deref() == Some(current_content.as_str())
                    && sha256_hex(&current_content) == current_content_hash.as_deref().unwrap_or("");
                let invalidated = stale_from_chapter.is_some_and(|stale| {
                    chapter_number >= stale && projection_generation < current_generation
                });
                let facts = parse_facts(&facts_json, chapter_number);
                let candidates = parse_character_state_candidates(&candidates_json);
                FinalizedContinuityProjection {
                    draft_id,
                    chapter_number,
                    chapter_title,
                    chapter_notes,
                    facts: Some(facts),
                    character_state_candidates: if candidates.is_empty() {
                        None
                    } else {
                        Some(candidates)
                    },
                    source: has_bound_source.then(|| FinalizedSourceIdentity {
                        draft_id,
                        finalization_id: source_finalization_id.clone().unwrap_or_default(),
                        chapter_number,
                        content_hash: source_content_hash.clone().unwrap_or_default(),
                    }),
                    source_status: if !has_bound_source {
                        "legacy".to_string()
                    } else if source_current && !invalidated {
                        "current".to_string()
                    } else {
                        "stale".to_string()
                    },
                }
            },
        )
        .collect())
}

/// 基线 `SummaryRepository.readFinalizedSource`
pub fn read_finalized_source(
    conn: &Connection,
    draft_id: i64,
) -> Result<FinalizedSourceReadResult, String> {
    if draft_id < 1 {
        return Err("定稿来源身份无效".to_string());
    }
    let row = conn
        .query_row(
            "SELECT drafts.chapter_number, drafts.status, contents.body,
                    COALESCE(finalization_outbox.chapter_title, blueprints.title, ''),
                    finalization_outbox.draft_id,
                    finalization_outbox.finalization_id
             FROM drafts
             JOIN contents ON contents.id = drafts.content_id
             LEFT JOIN finalization_outbox ON finalization_outbox.draft_id = drafts.id
             LEFT JOIN blueprints ON blueprints.chapter_number = drafts.chapter_number
             WHERE drafts.id = ?1",
            [draft_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((chapter_number, status, content, chapter_title, receipt_draft_id, _)) =
        row
    else {
        return Ok(FinalizedSourceReadResult::Invalid);
    };
    if status != "finalized" {
        return Ok(FinalizedSourceReadResult::Invalid);
    }
    if receipt_draft_id.is_none() {
        return Ok(FinalizedSourceReadResult::Legacy {
            draft_id,
            chapter_number,
            chapter_title,
            content,
        });
    }
    let snapshot = read_finalized_source_from_db(conn, draft_id)?;
    Ok(match snapshot {
        Some(snapshot) => FinalizedSourceReadResult::Valid { snapshot },
        None => FinalizedSourceReadResult::Invalid,
    })
}

/// 基线 `invalidateContinuityProjectionFrom`：推进全局失效水位
pub fn invalidate_continuity_projection_from(
    conn: &Connection,
    chapter_number: i64,
) -> Result<(), String> {
    if chapter_number < 1 {
        return Err("连续性投影失效章节无效".to_string());
    }
    conn.execute(
        "UPDATE continuity_projection_meta
         SET generation = generation + 1,
             stale_from_chapter = CASE
               WHEN stale_from_chapter IS NULL OR stale_from_chapter > ?1 THEN ?1
               ELSE stale_from_chapter
             END
         WHERE id = 'main'",
        [chapter_number],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
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

    const BODY: &str = "第十章正文：林决在城门与黑袍人相遇。";

    /// 建一个已定稿草稿 + outbox 收据（正文快照/哈希一致），返回 draft_id
    fn finalize_draft(conn: &Connection, chapter: i64, title: &str) -> i64 {
        conn.execute("INSERT INTO contents (body) VALUES (?1)", [BODY])
            .unwrap();
        let content_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id) VALUES (?1, 1, 'finalized', ?2)",
            rusqlite::params![chapter, content_id],
        )
        .unwrap();
        let draft_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO finalization_outbox (
                finalization_id, draft_id, chapter_number, chapter_title,
                content_hash, content_revision, content_snapshot, target_file_name
            ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)",
            rusqlite::params![
                format!("fin-{chapter}"),
                draft_id,
                chapter,
                title,
                sha256_hex(BODY),
                BODY,
                format!("ch{chapter}.md"),
            ],
        )
        .unwrap();
        draft_id
    }

    fn fact(evidence: &str) -> serde_json::Value {
        serde_json::json!({
            "category": "plot",
            "entities": ["林决"],
            "statement": "林决与黑袍人相遇",
            "sourceChapter": 10,
            "evidence": evidence
        })
    }

    fn save_request(draft_id: i64, gen: i64, evidence: &str) -> SaveFinalizedContinuityRequest {
        SaveFinalizedContinuityRequest {
            draft_id,
            chapter_number: 10,
            chapter_notes: "本章连续性备注".to_string(),
            facts: Some(vec![fact(evidence)]),
            projection_generation: gen,
            source: FinalizedSourceIdentity {
                draft_id,
                finalization_id: "fin-10".to_string(),
                chapter_number: 10,
                content_hash: sha256_hex(BODY),
            },
        }
    }

    #[test]
    fn save_and_list_roundtrip_test() {
        let conn = memory_db();
        let draft_id = finalize_draft(&conn, 10, "第十章");
        let generation: i64 = conn
            .query_row("SELECT generation FROM continuity_projection_meta WHERE id='main'", [], |r| r.get(0))
            .unwrap();

        save_finalized_continuity(&conn, &save_request(draft_id, generation, "城门")).unwrap();

        // 列表： chapter < 11 才可见，状态 current
        let list = list_finalized_continuity_before(&conn, 11).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].source_status, "current");
        assert_eq!(list[0].facts.as_ref().unwrap()[0].evidence, "城门");
        assert!(list[0].source.is_some());

        // 引文不在正文中 → 拒绝
        let bad = save_request(draft_id, generation, "不存在的引文");
        assert_eq!(
            save_finalized_continuity(&conn, &bad).unwrap_err(),
            "连续性事实引文无法在绑定定稿正文中精确定位"
        );
    }

    #[test]
    fn save_rejects_stale_watermark_and_bad_source_test() {
        let conn = memory_db();
        let draft_id = finalize_draft(&conn, 10, "第十章");
        let generation: i64 = conn
            .query_row("SELECT generation FROM continuity_projection_meta WHERE id='main'", [], |r| r.get(0))
            .unwrap();

        // 水位推进后旧代结果拒绝
        save_finalized_continuity(&conn, &save_request(draft_id, generation, "城门")).unwrap();
        invalidate_continuity_projection_from(&conn, 5).unwrap();
        let stale = save_request(draft_id, generation, "城门");
        assert_eq!(
            save_finalized_continuity(&conn, &stale).unwrap_err(),
            "连续性投影失效水位已推进，已拒绝过期结果"
        );

        // 来源收据不匹配拒绝
        let draft2 = finalize_draft(&conn, 11, "第十一章");
        let mut wrong_source = save_request(draft2, 1, "城门");
        wrong_source.source.finalization_id = "fin-99".to_string();
        assert_eq!(
            save_finalized_continuity(&conn, &wrong_source).unwrap_err(),
            "连续性投影来源已失效，已拒绝过期结果"
        );
    }

    #[test]
    fn read_source_discriminates_valid_legacy_invalid_test() {
        let conn = memory_db();
        // 未定稿草稿 → invalid
        conn.execute("INSERT INTO contents (body) VALUES ('草稿')", []).unwrap();
        let cid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id) VALUES (1, 1, 'draft', ?1)",
            [cid],
        )
        .unwrap();
        let draft_id = conn.last_insert_rowid();
        assert!(matches!(
            read_finalized_source(&conn, draft_id).unwrap(),
            FinalizedSourceReadResult::Invalid
        ));

        // 无收据的定稿 → legacy
        conn.execute(
            "UPDATE drafts SET status = 'finalized' WHERE id = ?1",
            [draft_id],
        )
        .unwrap();
        match read_finalized_source(&conn, draft_id).unwrap() {
            FinalizedSourceReadResult::Legacy { draft_id: id, .. } => assert_eq!(id, draft_id),
            other => panic!("应为 legacy，实际 {:?}", other),
        }

        // 有收据且快照一致 → valid
        let finalized = finalize_draft(&conn, 10, "第十章");
        match read_finalized_source(&conn, finalized).unwrap() {
            FinalizedSourceReadResult::Valid { snapshot } => {
                assert_eq!(snapshot.content, BODY);
                assert_eq!(snapshot.projection_generation, 0);
            }
            other => panic!("应为 valid，实际 {:?}", other),
        }
    }

    #[test]
    fn character_state_candidates_merge_and_validate_test() {
        let conn = memory_db();
        conn.execute(
            "INSERT INTO characters (name, role) VALUES ('林决', 'protagonist')",
            [],
        )
        .unwrap();
        let draft_id = finalize_draft(&conn, 10, "第十章");
        let generation: i64 = conn
            .query_row("SELECT generation FROM continuity_projection_meta WHERE id='main'", [], |r| r.get(0))
            .unwrap();

        // 先落连续性投影（候选必须绑定同代投影）
        save_finalized_continuity(&conn, &save_request(draft_id, generation, "城门")).unwrap();

        let candidates_req = SaveFinalizedCharacterStateCandidatesRequest {
            draft_id,
            chapter_number: 10,
            candidates: Some(vec![serde_json::json!({
                "characterName": "林决",
                "field": "location",
                "value": " 城门 "
            })]),
            projection_generation: generation,
            source: FinalizedSourceIdentity {
                draft_id,
                finalization_id: "fin-10".to_string(),
                chapter_number: 10,
                content_hash: sha256_hex(BODY),
            },
        };
        save_finalized_character_state_candidates(&conn, &candidates_req).unwrap();

        // 候选归一（trim）并合入投影行
        let list = list_finalized_continuity_before(&conn, 11).unwrap();
        let merged = list[0].character_state_candidates.as_ref().unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].value, "城门");

        // 不在角色名单中的候选拒绝
        let mut bad = candidates_req.clone();
        bad.candidates = Some(vec![serde_json::json!({
            "characterName": "路人甲",
            "field": "location",
            "value": "哪都"
        })]);
        assert_eq!(
            save_finalized_character_state_candidates(&conn, &bad).unwrap_err(),
            "角色状态候选参数无效"
        );
    }

    #[test]
    fn invalidate_advances_generation_and_stale_window_test() {
        let conn = memory_db();
        invalidate_continuity_projection_from(&conn, 5).unwrap();
        let (generation, stale): (i64, i64) = conn
            .query_row(
                "SELECT generation, stale_from_chapter FROM continuity_projection_meta WHERE id='main'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(generation, 1);
        assert_eq!(stale, 5);

        // 更早章节再次失效 → stale_from_chapter 收窄，代际继续递增
        invalidate_continuity_projection_from(&conn, 3).unwrap();
        let (generation, stale): (i64, i64) = conn
            .query_row(
                "SELECT generation, stale_from_chapter FROM continuity_projection_meta WHERE id='main'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(generation, 2);
        assert_eq!(stale, 3);

        // 非法章节拒绝
        assert_eq!(
            invalidate_continuity_projection_from(&conn, 0).unwrap_err(),
            "连续性投影失效章节无效"
        );
    }
}
