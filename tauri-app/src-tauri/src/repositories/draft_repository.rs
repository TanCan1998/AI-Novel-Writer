//! DraftRepository —— 草稿（`drafts` 表 + `contents` 联动）
//!
//! 平移自 `electron/repositories/draft-repository.ts`：
//! - `status = 'finalized'` 代表定稿；正文统一存 `contents` 表，草稿只持 `content_id`；
//! - 草稿来源依赖（`source_dependencies`）以 JSON 数组落库，读取时判定 `dependenciesStale`：
//!   依赖内容哈希、定稿权威性、以及**传递闭包**是否仍然成立；
//! - 定稿不可逆：已定稿草稿的状态/正文不可再改，删除必须走定稿删除入口。
//!
//! 本子域（S3-a）只含 `draft-repository` 的 12 个频道；`draft-authority-sequence` /
//! `draft-export-*` / `draft-import-finalized-batch` 依赖 finalization 仓储，随批次 E 落地。

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::repositories::character_roster_repository::hash_text;
use crate::repositories::content_repository as contents;

/// JS `Number.isSafeInteger` 的上界（2^53 - 1）
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// 单条依赖最多允许 500 项（对齐基线 `parseDependencies`）
const MAX_DEPENDENCIES: usize = 500;

/// 依赖批量查询分块大小
const DEPENDENCY_CHUNK: usize = 500;

/// 草稿来源依赖的形态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DraftSourceDependencyKind {
    /// 候选草稿依赖（新写入显式标记）
    Candidate,
    /// 定稿依赖（携带 `finalizationId` 与章节号）
    Finalized,
    /// 旧版定稿依赖（仅有章节号，无定稿 ID）
    LegacyFinalized,
}

/// 草稿来源依赖（`kind` 缺失表示旧行的精确候选草稿依赖）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftSourceDependency {
    pub draft_id: i64,
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<DraftSourceDependencyKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_number: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalization_id: Option<String>,
}

/// 草稿元数据（不含正文，适合列表查询）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftMeta {
    pub id: i64,
    pub chapter_number: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_title: Option<String>,
    pub version: i64,
    pub status: String,
    pub source: String,
    pub content_id: i64,
    pub word_count: i64,
    pub source_dependencies: Vec<DraftSourceDependency>,
    pub dependencies_stale: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// 草稿完整数据（含正文）—— 序列化为扁平结构（对齐 `DraftFull extends DraftMeta`）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DraftFull {
    #[serde(flatten)]
    pub meta: DraftMeta,
    pub content: String,
}

/// 创建草稿的参数
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftCreateParams {
    pub chapter_number: i64,
    /// 契约要求传入，但基线在事务内重新分配 version，故此值被忽略。
    #[serde(default)]
    #[allow(dead_code)]
    pub version: Option<i64>,
    pub source: String,
    pub content: String,
    pub word_count: i64,
    #[serde(default)]
    pub source_dependencies: Option<Vec<DraftSourceDependency>>,
}

/// 已定稿草稿必须走定稿删除入口的固定文案（命令层据它回填 `errorCode`）
pub const FINALIZED_DELETE_REQUIRED_MESSAGE: &str =
    "草稿已定稿，请通过定稿删除入口清理正文及派生投影";

/// 已定稿草稿删除的失败码（对齐基线 `FINALIZED_DRAFT_DELETE_REQUIRED`）
pub const FINALIZED_DELETE_REQUIRED_CODE: &str = "FINALIZED_DRAFT_DELETE_REQUIRED";

// ============================================================================
// 依赖解析与校验
// ============================================================================

struct ParsedDependencies {
    dependencies: Vec<DraftSourceDependency>,
    valid: bool,
}

fn invalid_dependencies() -> ParsedDependencies {
    ParsedDependencies {
        dependencies: Vec::new(),
        valid: false,
    }
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// 安全整数校验（对齐 `Number.isSafeInteger`）
fn safe_positive_integer(value: Option<i64>) -> Option<i64> {
    value.filter(|number| *number >= 1 && *number <= MAX_SAFE_INTEGER)
}

/// 解析并校验依赖 JSON（对齐基线 `parseDependencies`，任一项非法则整体失效）
fn parse_dependencies(value: Option<&str>) -> ParsedDependencies {
    let Some(raw) = value else {
        return invalid_dependencies();
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw) else {
        return invalid_dependencies();
    };
    let Some(items) = parsed.as_array() else {
        return invalid_dependencies();
    };
    if items.len() > MAX_DEPENDENCIES {
        return invalid_dependencies();
    }

    let mut dependencies: Vec<DraftSourceDependency> = Vec::with_capacity(items.len());
    let mut seen: HashSet<i64> = HashSet::new();
    for item in items {
        let Some(object) = item.as_object() else {
            return invalid_dependencies();
        };

        let Some(draft_id) = safe_positive_integer(object.get("draftId").and_then(|value| value.as_i64()))
        else {
            return invalid_dependencies();
        };
        let Some(content_hash) = object
            .get("contentHash")
            .and_then(|value| value.as_str())
            .filter(|hash| is_sha256_hex(hash))
        else {
            return invalid_dependencies();
        };
        if seen.contains(&draft_id) {
            return invalid_dependencies();
        }

        // `kind` 允许缺失；出现即必须是三个已知字面量之一
        let kind = match object.get("kind") {
            None => None,
            Some(serde_json::Value::String(text)) => match text.as_str() {
                "candidate" => Some(DraftSourceDependencyKind::Candidate),
                "finalized" => Some(DraftSourceDependencyKind::Finalized),
                "legacy-finalized" => Some(DraftSourceDependencyKind::LegacyFinalized),
                _ => return invalid_dependencies(),
            },
            Some(_) => return invalid_dependencies(),
        };

        let needs_chapter = matches!(
            kind,
            Some(DraftSourceDependencyKind::Finalized) | Some(DraftSourceDependencyKind::LegacyFinalized)
        );
        let chapter_number = safe_positive_integer(
            object.get("chapterNumber").and_then(|value| value.as_i64()),
        );
        if needs_chapter && chapter_number.is_none() {
            return invalid_dependencies();
        }
        // 候选/旧格式不得携带 `chapterNumber`（显式 null 亦视为非法，对齐 `!== undefined`）
        if !needs_chapter && object.get("chapterNumber").is_some() {
            return invalid_dependencies();
        }

        let raw_finalization_id = object.get("finalizationId");
        let finalization_id = match kind {
            Some(DraftSourceDependencyKind::Finalized) => {
                let Some(text) = raw_finalization_id.and_then(|value| value.as_str()) else {
                    return invalid_dependencies();
                };
                if text.trim().is_empty() || text.encode_utf16().count() > 500 {
                    return invalid_dependencies();
                }
                Some(text.to_string())
            }
            _ => {
                if raw_finalization_id.is_some() {
                    return invalid_dependencies();
                }
                None
            }
        };

        seen.insert(draft_id);
        dependencies.push(DraftSourceDependency {
            draft_id,
            content_hash: content_hash.to_string(),
            kind,
            chapter_number: if needs_chapter { chapter_number } else { None },
            finalization_id,
        });
    }

    ParsedDependencies {
        dependencies,
        valid: true,
    }
}

/// 序列化依赖（键序由 `serde_json` 决定；Rust 侧自洽，不跨栈比对字符串）
fn serialize_dependencies(dependencies: &[DraftSourceDependency]) -> Result<String, String> {
    serde_json::to_string(dependencies).map_err(|error| format!("序列化草稿来源依赖失败：{error}"))
}

/// 依赖解析后用于判定的单条状态
struct DraftDependencyState {
    content_hash: String,
    chapter_number: i64,
    status: String,
    source_dependencies: Vec<DraftSourceDependency>,
    source_dependencies_valid: bool,
    authoritative_finalized: bool,
    finalization_id: Option<String>,
    receipt_content_hash: Option<String>,
    receipt_content_matches: bool,
}

/// 批量（并递归）取依赖状态（对齐基线 `dependencyStates`）
fn dependency_states(
    conn: &Connection,
    dependency_ids: &[i64],
) -> Result<HashMap<i64, DraftDependencyState>, String> {
    let mut states: HashMap<i64, DraftDependencyState> = HashMap::new();
    let mut queued: HashSet<i64> = HashSet::new();
    let mut pending: Vec<i64> = Vec::new();
    for id in dependency_ids {
        if queued.insert(*id) {
            pending.push(*id);
        }
    }

    let mut offset = 0;
    while offset < pending.len() {
        let end = (offset + DEPENDENCY_CHUNK).min(pending.len());
        let chunk: Vec<i64> = pending[offset..end].to_vec();
        offset = end;
        if chunk.is_empty() {
            continue;
        }

        let placeholders = (1..=chunk.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT drafts.id, drafts.chapter_number, drafts.status, drafts.source_dependencies,
                    contents.body,
                    finalization_outbox.finalization_id,
                    finalization_outbox.content_hash AS receipt_content_hash,
                    finalization_outbox.content_snapshot,
                    NOT EXISTS (
                      SELECT 1 FROM drafts newer
                      WHERE newer.chapter_number = drafts.chapter_number
                        AND newer.status = 'finalized'
                        AND (newer.version > drafts.version
                             OR (newer.version = drafts.version AND newer.id > drafts.id))
                    ) AS authoritative_finalized
             FROM drafts
             JOIN contents ON contents.id = drafts.content_id
             LEFT JOIN finalization_outbox ON finalization_outbox.draft_id = drafts.id
             WHERE drafts.id IN ({placeholders})"
        );

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|error| format!("读取草稿依赖失败：{error}"))?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(chunk.iter()),
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .map_err(|error| format!("读取草稿依赖失败：{error}"))?;

        for row in rows {
            let (
                id,
                chapter_number,
                status,
                dependencies_json,
                body,
                finalization_id,
                receipt_content_hash,
                content_snapshot,
                authoritative_finalized,
            ) = row.map_err(|error| format!("读取草稿依赖失败：{error}"))?;

            let parsed = parse_dependencies(Some(&dependencies_json));
            states.insert(
                id,
                DraftDependencyState {
                    content_hash: hash_text(&body),
                    chapter_number,
                    status: status.unwrap_or_default(),
                    source_dependencies: parsed.dependencies.clone(),
                    source_dependencies_valid: parsed.valid,
                    authoritative_finalized: authoritative_finalized == 1,
                    finalization_id,
                    receipt_content_hash,
                    receipt_content_matches: content_snapshot
                        .as_deref()
                        .map(|snapshot| snapshot == body)
                        .unwrap_or(true),
                },
            );

            if !parsed.valid {
                continue;
            }
            for dependency in &parsed.dependencies {
                if queued.insert(dependency.draft_id) {
                    pending.push(dependency.draft_id);
                }
            }
        }
    }

    Ok(states)
}

/// 单条依赖是否仍然成立（对齐 `dependencyIsCurrent`）
fn dependency_is_current(
    dependency: &DraftSourceDependency,
    state: Option<&DraftDependencyState>,
) -> bool {
    let Some(state) = state else {
        return false;
    };
    if state.content_hash != dependency.content_hash {
        return false;
    }
    let Some(chapter_number) = dependency.chapter_number else {
        return true;
    };
    if state.status != "finalized"
        || !state.authoritative_finalized
        || state.chapter_number != chapter_number
    {
        return false;
    }
    if dependency.kind == Some(DraftSourceDependencyKind::LegacyFinalized) {
        return state.finalization_id.is_none();
    }
    let Some(finalization_id) = dependency.finalization_id.as_deref() else {
        return false;
    };
    state.finalization_id.as_deref() == Some(finalization_id)
        && state.receipt_content_hash.as_deref() == Some(dependency.content_hash.as_str())
        && state.receipt_content_matches
}

/// 依赖的传递闭包是否仍然成立（显式栈，避免深链递归爆栈）
fn dependency_lineage_is_current(
    root_id: i64,
    states: &HashMap<i64, DraftDependencyState>,
    memo: &mut HashMap<i64, bool>,
) -> bool {
    if let Some(cached) = memo.get(&root_id) {
        return *cached;
    }

    let mut visiting: HashSet<i64> = HashSet::new();
    visiting.insert(root_id);
    let mut stack: Vec<(i64, usize)> = vec![(root_id, 0)];

    while !stack.is_empty() {
        let top = stack.len() - 1;
        let draft_id = stack[top].0;

        if memo.contains_key(&draft_id) {
            visiting.remove(&draft_id);
            stack.pop();
            continue;
        }

        let Some(state) = states.get(&draft_id) else {
            memo.insert(draft_id, false);
            visiting.remove(&draft_id);
            stack.pop();
            continue;
        };
        if !state.source_dependencies_valid {
            memo.insert(draft_id, false);
            visiting.remove(&draft_id);
            stack.pop();
            continue;
        }

        let next_index = stack[top].1;
        let Some(dependency) = state.source_dependencies.get(next_index).cloned() else {
            let all_current = state.source_dependencies.iter().all(|source| {
                dependency_is_current(source, states.get(&source.draft_id))
                    && memo.get(&source.draft_id) == Some(&true)
            });
            memo.insert(draft_id, all_current);
            visiting.remove(&draft_id);
            stack.pop();
            continue;
        };
        stack[top].1 += 1;

        if !dependency_is_current(&dependency, states.get(&dependency.draft_id)) {
            memo.insert(draft_id, false);
            continue;
        }
        if memo.contains_key(&dependency.draft_id) {
            continue;
        }
        if visiting.contains(&dependency.draft_id) {
            memo.insert(draft_id, false);
            continue;
        }

        visiting.insert(dependency.draft_id);
        stack.push((dependency.draft_id, 0));
    }

    memo.get(&root_id) == Some(&true)
}

fn dependencies_are_current(
    dependencies: &[DraftSourceDependency],
    states: &HashMap<i64, DraftDependencyState>,
    memo: &mut HashMap<i64, bool>,
) -> bool {
    dependencies.iter().all(|dependency| {
        dependency_is_current(dependency, states.get(&dependency.draft_id))
            && dependency_lineage_is_current(dependency.draft_id, states, memo)
    })
}

// ============================================================================
// 行 → 契约类型
// ============================================================================

/// 元数据查询（定稿时联表取章节标题）
const DRAFT_META_SELECT: &str = "
  SELECT drafts.id, drafts.chapter_number, drafts.version, drafts.status, drafts.source,
         drafts.content_id, drafts.word_count, drafts.source_dependencies,
         drafts.created_at, drafts.updated_at, finalization_outbox.chapter_title
  FROM drafts
  LEFT JOIN finalization_outbox
    ON drafts.status = 'finalized' AND finalization_outbox.draft_id = drafts.id
";

struct DraftMetaRow {
    id: i64,
    chapter_number: i64,
    version: i64,
    status: String,
    source: String,
    content_id: i64,
    word_count: i64,
    source_dependencies: String,
    created_at: String,
    updated_at: String,
    chapter_title: Option<String>,
}

fn draft_meta_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DraftMetaRow> {
    Ok(DraftMetaRow {
        id: row.get(0)?,
        chapter_number: row.get(1)?,
        version: row.get(2)?,
        status: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        source: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        content_id: row.get(5)?,
        word_count: row.get::<_, Option<i64>>(6)?.unwrap_or_default(),
        source_dependencies: row.get(7)?,
        created_at: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
        updated_at: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
        chapter_title: row.get(10)?,
    })
}

fn draft_meta_row_to_meta(
    row: &DraftMetaRow,
    parsed: &ParsedDependencies,
    states: &HashMap<i64, DraftDependencyState>,
    memo: &mut HashMap<i64, bool>,
) -> DraftMeta {
    let chapter_title = row
        .chapter_title
        .as_ref()
        .filter(|title| !title.trim().is_empty())
        .cloned();
    DraftMeta {
        id: row.id,
        chapter_number: row.chapter_number,
        chapter_title,
        version: row.version,
        status: row.status.clone(),
        source: row.source.clone(),
        content_id: row.content_id,
        word_count: row.word_count,
        source_dependencies: parsed.dependencies.clone(),
        dependencies_stale: !parsed.valid
            || !dependencies_are_current(&parsed.dependencies, states, memo),
        created_at: row.created_at.clone(),
        updated_at: row.updated_at.clone(),
    }
}

fn rows_to_meta(conn: &Connection, rows: &[DraftMetaRow]) -> Result<Vec<DraftMeta>, String> {
    let parsed: Vec<ParsedDependencies> = rows
        .iter()
        .map(|row| parse_dependencies(Some(&row.source_dependencies)))
        .collect();
    let dependency_ids: Vec<i64> = parsed
        .iter()
        .flat_map(|item| item.dependencies.iter().map(|dependency| dependency.draft_id))
        .collect();
    let states = dependency_states(conn, &dependency_ids)?;
    let mut memo: HashMap<i64, bool> = HashMap::new();
    let mut result = Vec::with_capacity(rows.len());
    for (row, parsed) in rows.iter().zip(parsed.iter()) {
        result.push(draft_meta_row_to_meta(row, parsed, &states, &mut memo));
    }
    Ok(result)
}

// ============================================================================
// 公共 API
// ============================================================================

/// 列出章节的所有草稿（不含正文，按版本升序）
pub fn list_by_chapter(conn: &Connection, chapter_number: i64) -> Result<Vec<DraftMeta>, String> {
    let sql = format!("{DRAFT_META_SELECT} WHERE drafts.chapter_number = ?1 ORDER BY drafts.version ASC");
    read_meta_rows(conn, &sql, rusqlite::params![chapter_number])
}

/// 列出全部草稿元数据（按章节、版本升序）
pub fn list_all(conn: &Connection) -> Result<Vec<DraftMeta>, String> {
    let sql = format!("{DRAFT_META_SELECT} ORDER BY drafts.chapter_number ASC, drafts.version ASC");
    read_meta_rows(conn, &sql, [])
}

/// 获取草稿元数据（不存在返回 `None`）
pub fn get_meta(conn: &Connection, id: i64) -> Result<Option<DraftMeta>, String> {
    let sql = format!("{DRAFT_META_SELECT} WHERE drafts.id = ?1");
    let rows = read_meta_rows(conn, &sql, rusqlite::params![id])?;
    Ok(rows.into_iter().next())
}

/// 获取草稿完整数据（含正文）
pub fn get_full(conn: &Connection, id: i64) -> Result<Option<DraftFull>, String> {
    let Some(meta) = get_meta(conn, id)? else {
        return Ok(None);
    };
    let content = contents::get_body(conn, meta.content_id)?.unwrap_or_default();
    Ok(Some(DraftFull { meta, content }))
}

/// 获取章节最新版本的草稿
pub fn get_latest_by_chapter(
    conn: &Connection,
    chapter_number: i64,
) -> Result<Option<DraftMeta>, String> {
    let sql = format!(
        "{DRAFT_META_SELECT} WHERE drafts.chapter_number = ?1 ORDER BY drafts.version DESC LIMIT 1"
    );
    let rows = read_meta_rows(conn, &sql, rusqlite::params![chapter_number])?;
    Ok(rows.into_iter().next())
}

/// 获取章节已定稿的草稿
pub fn get_finalized_by_chapter(
    conn: &Connection,
    chapter_number: i64,
) -> Result<Option<DraftMeta>, String> {
    let sql = format!(
        "{DRAFT_META_SELECT} WHERE drafts.chapter_number = ?1 AND drafts.status = 'finalized' \
         ORDER BY drafts.version DESC LIMIT 1"
    );
    let rows = read_meta_rows(conn, &sql, rusqlite::params![chapter_number])?;
    Ok(rows.into_iter().next())
}

/// 获取下一个可用版本号
pub fn get_next_version(conn: &Connection, chapter_number: i64) -> Result<i64, String> {
    let max_version: Option<i64> = conn
        .query_row(
            "SELECT MAX(version) FROM drafts WHERE chapter_number = ?1",
            [chapter_number],
            |row| row.get(0),
        )
        .map_err(|error| format!("读取草稿版本失败：{error}"))?;
    Ok(max_version.unwrap_or(0) + 1)
}

/// 获取最大的已定稿章节号（无定稿返回 0）
pub fn get_max_finalized_chapter(conn: &Connection) -> Result<i64, String> {
    let max_chapter: Option<i64> = conn
        .query_row(
            "SELECT MAX(chapter_number) FROM drafts WHERE status = 'finalized'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("读取已定稿章节失败：{error}"))?;
    Ok(max_chapter.unwrap_or(0))
}

/// 创建草稿（事务内原子分配 version + 校验来源依赖），返回新建 ID
pub fn create(conn: &Connection, params: &DraftCreateParams) -> Result<i64, String> {
    let dependencies = params.source_dependencies.clone().unwrap_or_default();
    let serialized = serialize_dependencies(&dependencies)?;
    let parsed = parse_dependencies(Some(&serialized));
    if !parsed.valid {
        return Err("草稿来源依赖无效".to_string());
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;

    let dependency_ids: Vec<i64> = parsed
        .dependencies
        .iter()
        .map(|dependency| dependency.draft_id)
        .collect();
    let states = dependency_states(&tx, &dependency_ids)?;
    if !dependencies_are_current(&parsed.dependencies, &states, &mut HashMap::new()) {
        return Err("草稿来源依赖已变化或不再是当前定稿，已拒绝保存".to_string());
    }

    let max_version: Option<i64> = tx
        .query_row(
            "SELECT MAX(version) FROM drafts WHERE chapter_number = ?1",
            [params.chapter_number],
            |row| row.get(0),
        )
        .map_err(|error| format!("读取草稿版本失败：{error}"))?;
    let version = max_version.unwrap_or(0) + 1;

    let content_id = contents::create(&tx, &params.content)?;
    tx.execute(
        "INSERT INTO drafts (chapter_number, version, source, content_id, word_count, source_dependencies)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            params.chapter_number,
            version,
            params.source,
            content_id,
            params.word_count,
            serialized,
        ],
    )
    .map_err(|error| format!("写入草稿失败：{error}"))?;
    let id = tx.last_insert_rowid();

    tx.commit()
        .map_err(|error| format!("提交草稿失败：{error}"))?;
    Ok(id)
}

/// 更新草稿状态（定稿不可逆规则在此把关；草稿不存在时静默成功）
pub fn update_status(
    conn: &Connection,
    id: i64,
    status: &str,
    word_count: Option<i64>,
) -> Result<(), String> {
    let Some(meta) = get_meta(conn, id)? else {
        return Ok(());
    };
    if meta.status == "finalized" && status != "finalized" {
        return Err("已定稿正文为不可变事实；如需再编辑，请创建新草稿或处理定稿冲突".to_string());
    }
    if meta.status != "finalized" && status == "finalized" {
        return Err("定稿必须通过原子定稿提交，不能单独更新草稿状态".to_string());
    }

    match word_count {
        Some(count) => conn.execute(
            "UPDATE drafts SET status = ?1, word_count = ?2, updated_at = datetime('now') WHERE id = ?3",
            rusqlite::params![status, count, id],
        ),
        None => conn.execute(
            "UPDATE drafts SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
            rusqlite::params![status, id],
        ),
    }
    .map(|_| ())
    .map_err(|error| format!("更新草稿状态失败：{error}"))
}

/// 更新草稿正文（同时更新 `contents`）
pub fn update_content(
    conn: &Connection,
    id: i64,
    content: &str,
    word_count: i64,
) -> Result<(), String> {
    let Some(meta) = get_meta(conn, id)? else {
        return Ok(());
    };
    if meta.status == "finalized" {
        return Err("已定稿正文为不可变事实；如需再编辑，请创建新草稿或处理定稿冲突".to_string());
    }

    contents::update_body(conn, meta.content_id, content)?;
    conn.execute(
        "UPDATE drafts SET word_count = ?1, updated_at = datetime('now') WHERE id = ?2",
        rusqlite::params![word_count, id],
    )
    .map(|_| ())
    .map_err(|error| format!("更新草稿正文失败：{error}"))
}

/// 删除草稿（级联删除 revisions/reviews，但 contents 需手动清理）
///
/// 已定稿时返回 [`FINALIZED_DELETE_REQUIRED_MESSAGE`]，命令层据此回填 `errorCode`。
pub fn delete(conn: &Connection, id: i64) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;

    // 状态必须与 DELETE 位于同一权威事务：菜单打开时的渲染层状态
    // 不能授权删除在确认期间已经定稿的事实。
    let target: Option<(String, i64)> = tx
        .query_row(
            "SELECT status, content_id FROM drafts WHERE id = ?1",
            [id],
            |row| Ok((row.get::<_, Option<String>>(0)?.unwrap_or_default(), row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("读取草稿失败：{error}"))?;
    let Some((status, content_id)) = target else {
        return Ok(());
    };
    if status == "finalized" {
        return Err(FINALIZED_DELETE_REQUIRED_MESSAGE.to_string());
    }

    tx.execute("DELETE FROM drafts WHERE id = ?1", [id])
        .map_err(|error| format!("删除草稿失败：{error}"))?;

    // 若 `contents.id` 仍被 revision/review 引用，外键约束会阻止删除；
    // 此处保留基线的孤立内容兼容策略（忽略失败）。
    let _ = contents::delete(&tx, content_id);

    tx.commit().map_err(|error| format!("提交删除失败：{error}"))
}

fn read_meta_rows<P: rusqlite::Params>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<DraftMeta>, String> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|error| format!("读取草稿失败：{error}"))?;
    let rows = stmt
        .query_map(params, draft_meta_row)
        .map_err(|error| format!("读取草稿失败：{error}"))?;
    let mut collected = Vec::new();
    for row in rows {
        collected.push(row.map_err(|error| format!("读取草稿失败：{error}"))?);
    }
    rows_to_meta(conn, &collected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        create_tables(&conn).expect("建表失败");
        conn
    }

    fn params(chapter: i64, content: &str) -> DraftCreateParams {
        DraftCreateParams {
            chapter_number: chapter,
            version: None,
            source: "write".to_string(),
            content: content.to_string(),
            word_count: content.chars().count() as i64,
            source_dependencies: None,
        }
    }

    fn candidate_dependency(draft_id: i64, content_hash: &str) -> DraftSourceDependency {
        DraftSourceDependency {
            draft_id,
            content_hash: content_hash.to_string(),
            kind: Some(DraftSourceDependencyKind::Candidate),
            chapter_number: None,
            finalization_id: None,
        }
    }

    #[test]
    fn create_assigns_version_and_reads_meta_test() {
        let conn = memory_db();
        let first = create(&conn, &params(1, "第一章第一版")).unwrap();
        let second = create(&conn, &params(1, "第一章第二版")).unwrap();
        assert!(second > first);

        let list = list_by_chapter(&conn, 1).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].version, 1);
        assert_eq!(list[1].version, 2);
        assert!(!list[0].dependencies_stale);

        let full = get_full(&conn, first).unwrap().unwrap();
        assert_eq!(full.content, "第一章第一版");
        assert_eq!(full.meta.id, first);

        assert_eq!(get_next_version(&conn, 1).unwrap(), 3);
        assert_eq!(list_all(&conn).unwrap().len(), 2);
        assert_eq!(get_latest_by_chapter(&conn, 1).unwrap().unwrap().id, second);
        assert!(get_meta(&conn, 9999).unwrap().is_none());
        assert!(get_full(&conn, 9999).unwrap().is_none());
    }

    #[test]
    fn create_rejects_invalid_dependencies_test() {
        let conn = memory_db();
        let mut invalid = params(1, "正文");
        invalid.source_dependencies = Some(vec![DraftSourceDependency {
            draft_id: 1,
            content_hash: "not-a-hash".to_string(),
            kind: Some(DraftSourceDependencyKind::Candidate),
            chapter_number: None,
            finalization_id: None,
        }]);
        assert_eq!(create(&conn, &invalid).unwrap_err(), "草稿来源依赖无效");
    }

    #[test]
    fn create_rejects_stale_dependencies_test() {
        let conn = memory_db();
        let mut dependent = params(1, "正文");
        dependent.source_dependencies = Some(vec![candidate_dependency(9999, HASH_A)]);
        assert_eq!(
            create(&conn, &dependent).unwrap_err(),
            "草稿来源依赖已变化或不再是当前定稿，已拒绝保存"
        );
    }

    #[test]
    fn dependencies_stale_flag_tracks_content_change_test() {
        let conn = memory_db();
        let base = create(&conn, &params(1, "底稿正文")).unwrap();
        let hash = hash_text("底稿正文");

        let mut dependent = params(2, "续写正文");
        dependent.source_dependencies = Some(vec![candidate_dependency(base, &hash)]);
        let dependent_id = create(&conn, &dependent).unwrap();

        let meta = get_meta(&conn, dependent_id).unwrap().unwrap();
        assert!(!meta.dependencies_stale, "依赖未变时不应判定过期");
        assert_eq!(meta.source_dependencies.len(), 1);
        assert_eq!(
            meta.source_dependencies[0].kind,
            Some(DraftSourceDependencyKind::Candidate)
        );

        update_content(&conn, base, "底稿被改写", 5).unwrap();
        let stale = get_meta(&conn, dependent_id).unwrap().unwrap();
        assert!(stale.dependencies_stale, "依赖正文变化后必须判定过期");
    }

    #[test]
    fn update_status_enforces_finalization_rules_test() {
        let conn = memory_db();
        let id = create(&conn, &params(1, "正文")).unwrap();

        update_status(&conn, id, "revised", Some(10)).unwrap();
        assert_eq!(get_meta(&conn, id).unwrap().unwrap().status, "revised");

        assert_eq!(
            update_status(&conn, id, "finalized", None).unwrap_err(),
            "定稿必须通过原子定稿提交，不能单独更新草稿状态"
        );

        // 模拟原子定稿提交后不可回退（finalized → finalized 幂等允许）
        conn.execute("UPDATE drafts SET status = 'finalized' WHERE id = ?1", [id])
            .unwrap();
        assert_eq!(
            update_status(&conn, id, "draft", None).unwrap_err(),
            "已定稿正文为不可变事实；如需再编辑，请创建新草稿或处理定稿冲突"
        );
        update_status(&conn, id, "finalized", None).unwrap();

        // 草稿不存在时静默成功
        update_status(&conn, 9999, "draft", None).unwrap();
    }

    #[test]
    fn update_content_rejects_finalized_and_missing_test() {
        let conn = memory_db();
        let id = create(&conn, &params(1, "正文")).unwrap();
        update_content(&conn, id, "新正文", 3).unwrap();
        assert_eq!(get_full(&conn, id).unwrap().unwrap().content, "新正文");

        conn.execute("UPDATE drafts SET status = 'finalized' WHERE id = ?1", [id])
            .unwrap();
        assert_eq!(
            update_content(&conn, id, "再改", 2).unwrap_err(),
            "已定稿正文为不可变事实；如需再编辑，请创建新草稿或处理定稿冲突"
        );

        update_content(&conn, 9999, "无关", 2).unwrap();
    }

    #[test]
    fn delete_clears_content_unless_finalized_test() {
        let conn = memory_db();
        let id = create(&conn, &params(1, "待删正文")).unwrap();
        let content_id = get_meta(&conn, id).unwrap().unwrap().content_id;

        delete(&conn, id).unwrap();
        assert!(get_meta(&conn, id).unwrap().is_none());
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM contents WHERE id = ?1",
                [content_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0, "无外键引用时正文应一并清理");

        let finalized = create(&conn, &params(2, "定稿正文")).unwrap();
        conn.execute("UPDATE drafts SET status = 'finalized' WHERE id = ?1", [finalized])
            .unwrap();
        assert_eq!(
            delete(&conn, finalized).unwrap_err(),
            FINALIZED_DELETE_REQUIRED_MESSAGE
        );
        assert!(get_meta(&conn, finalized).unwrap().is_some());

        delete(&conn, 9999).unwrap();
    }

    #[test]
    fn get_finalized_and_max_finalized_chapter_test() {
        let conn = memory_db();
        let chapter1 = create(&conn, &params(1, "章一")).unwrap();
        create(&conn, &params(3, "章三")).unwrap();
        let chapter5 = create(&conn, &params(5, "章五")).unwrap();

        assert_eq!(get_max_finalized_chapter(&conn).unwrap(), 0);
        assert!(get_finalized_by_chapter(&conn, 1).unwrap().is_none());

        conn.execute(
            "UPDATE drafts SET status = 'finalized' WHERE id IN (?1, ?2)",
            rusqlite::params![chapter1, chapter5],
        )
        .unwrap();

        assert_eq!(get_max_finalized_chapter(&conn).unwrap(), 5);
        let finalized = get_finalized_by_chapter(&conn, 1).unwrap().unwrap();
        assert_eq!(finalized.id, chapter1);
        assert_eq!(finalized.status, "finalized");
    }

    #[test]
    fn parse_dependencies_matches_baseline_rules_test() {
        let legacy = parse_dependencies(Some(&format!(
            "[{{\"draftId\":1,\"contentHash\":\"{HASH_A}\"}}]"
        )));
        assert!(legacy.valid);
        assert!(legacy.dependencies[0].kind.is_none());

        let duplicated = parse_dependencies(Some(&format!(
            "[{{\"draftId\":1,\"contentHash\":\"{HASH_A}\"}},\
              {{\"draftId\":1,\"contentHash\":\"{HASH_A}\"}}]"
        )));
        assert!(!duplicated.valid);

        let with_chapter = parse_dependencies(Some(&format!(
            "[{{\"draftId\":1,\"contentHash\":\"{HASH_A}\",\"kind\":\"candidate\",\"chapterNumber\":2}}]"
        )));
        assert!(!with_chapter.valid);

        let missing_finalization = parse_dependencies(Some(&format!(
            "[{{\"draftId\":1,\"contentHash\":\"{HASH_A}\",\"kind\":\"finalized\",\"chapterNumber\":2}}]"
        )));
        assert!(!missing_finalization.valid);

        let legacy_finalized = parse_dependencies(Some(&format!(
            "[{{\"draftId\":1,\"contentHash\":\"{HASH_A}\",\"kind\":\"legacy-finalized\",\"chapterNumber\":2}}]"
        )));
        assert!(legacy_finalized.valid);
        assert_eq!(legacy_finalized.dependencies[0].chapter_number, Some(2));

        // 超过 500 项 → 整体失效
        let many: Vec<String> = (1..=501)
            .map(|index| format!("{{\"draftId\":{index},\"contentHash\":\"{HASH_A}\"}}"))
            .collect();
        let too_many = parse_dependencies(Some(&format!("[{}]", many.join(","))));
        assert!(!too_many.valid);
    }

    #[test]
    fn serialization_shape_matches_frontend_contract_test() {
        let conn = memory_db();
        let id = create(&conn, &params(1, "正文")).unwrap();
        let full = get_full(&conn, id).unwrap().unwrap();
        let value = serde_json::to_value(&full).unwrap();

        // DraftFull 必须扁平（对齐 `DraftFull extends DraftMeta`）
        assert_eq!(value["id"], serde_json::json!(id));
        assert_eq!(value["chapterNumber"], serde_json::json!(1));
        assert_eq!(value["content"], serde_json::json!("正文"));
        assert!(value.get("meta").is_none(), "meta 不得嵌套");
        // 无章节标题时省略键（对齐基线 undefined）
        assert!(value.get("chapterTitle").is_none());
        assert_eq!(value["sourceDependencies"], serde_json::json!([]));
        assert_eq!(value["dependenciesStale"], serde_json::json!(false));
    }

    #[test]
    fn dependency_lineage_detects_transitive_staleness_test() {
        let conn = memory_db();
        let base = create(&conn, &params(1, "源头")).unwrap();

        // middle 依赖 base
        let mut middle = params(2, "中间") ;
        middle.source_dependencies = Some(vec![candidate_dependency(base, &hash_text("源头"))]);
        let middle_id = create(&conn, &middle).unwrap();

        // leaf 依赖 middle
        let mut leaf = params(3, "叶");
        leaf.source_dependencies = Some(vec![candidate_dependency(
            middle_id,
            &hash_text("中间"),
        )]);
        let leaf_id = create(&conn, &leaf).unwrap();
        assert!(!get_meta(&conn, leaf_id).unwrap().unwrap().dependencies_stale);

        // 改写源头：叶的传递闭包必须失效
        update_content(&conn, base, "源头改写", 4).unwrap();
        assert!(get_meta(&conn, leaf_id).unwrap().unwrap().dependencies_stale);
    }
}
