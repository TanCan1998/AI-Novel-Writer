//! BlueprintRepository —— 章节蓝图（`blueprints` 表）
//!
//! 平移自 `electron/repositories/blueprint-repository.ts`（S2-a：基础读写）。
//! - `characters` 列表以 JSON 数组落库（`'[]'` 为缺省），角色改名由角色仓储同步维护；
//! - `newCharacterCandidates` / `relationshipHints` 不属于 `blueprints` 表列，
//!   只在"范围提交"的不可变回执中保留（S2-b 落地），故此处读写不落库；
//! - `updateNotes` 只改 notes 与两个时间戳；`clearAll` 在单一事务内清空三张表。
//!
//! S2-b/S2-c（范围提交、角色同步）将在此文件续写。

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::schema::ensure_blueprint_commit_schema;
use crate::repositories::character_roster_repository::{
    self as roster, CharacterRosterEntry, CharacterRosterStatus,
};

/// 前端驼峰接口（对齐基线 `BlueprintData`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintData {
    pub chapter_number: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub key_events: String,
    #[serde(default)]
    pub characters: Vec<String>,
    /// 本章首次引入的重要常驻角色候选（非表列，随提交回执冻结）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_character_candidates: Option<Vec<serde_json::Value>>,
    /// 刚生成的蓝图携带的关系载荷（非表列，随提交回执冻结）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship_hints: Option<serde_json::Value>,
    #[serde(default)]
    pub suspense_hook: String,
    #[serde(default)]
    pub user_guidance: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub notes_updated_at: String,
}

impl BlueprintData {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let characters_json: Option<String> = row.get("characters")?;
        // 基线 `rowToData` 对损坏 JSON 容错为空数组
        let characters = characters_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
            .unwrap_or_default();
        Ok(Self {
            chapter_number: row.get("chapter_number")?,
            title: row.get::<_, Option<String>>("title")?.unwrap_or_default(),
            role: row.get::<_, Option<String>>("role")?.unwrap_or_default(),
            purpose: row.get::<_, Option<String>>("purpose")?.unwrap_or_default(),
            key_events: row
                .get::<_, Option<String>>("key_events")?
                .unwrap_or_default(),
            characters,
            // 非表列字段：读回恒为缺失（对齐基线 undefined）
            new_character_candidates: None,
            relationship_hints: None,
            suspense_hook: row
                .get::<_, Option<String>>("suspense_hook")?
                .unwrap_or_default(),
            user_guidance: row
                .get::<_, Option<String>>("user_guidance")?
                .unwrap_or_default(),
            notes: row.get::<_, Option<String>>("notes")?.unwrap_or_default(),
            notes_updated_at: row
                .get::<_, Option<String>>("notes_updated_at")?
                .unwrap_or_default(),
        })
    }
}

/// 读取全部蓝图（按章节号升序）
pub fn get_all(conn: &Connection) -> Result<Vec<BlueprintData>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT chapter_number, title, role, purpose, key_events, characters,
                    suspense_hook, user_guidance, notes, notes_updated_at
             FROM blueprints ORDER BY chapter_number ASC",
        )
        .map_err(|error| format!("读取蓝图失败：{error}"))?;
    let rows = stmt
        .query_map([], BlueprintData::from_row)
        .map_err(|error| format!("读取蓝图失败：{error}"))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|error| format!("读取蓝图失败：{error}"))?);
    }
    Ok(result)
}

/// 读取单章蓝图（未找到返回 `None`，对齐基线 `null`）
pub fn get_by_chapter(
    conn: &Connection,
    chapter_number: i64,
) -> Result<Option<BlueprintData>, String> {
    conn.query_row(
        "SELECT chapter_number, title, role, purpose, key_events, characters,
                suspense_hook, user_guidance, notes, notes_updated_at
         FROM blueprints WHERE chapter_number = ?1",
        [chapter_number],
        BlueprintData::from_row,
    )
    .optional()
    .map_err(|error| format!("读取蓝图失败：{error}"))
}

/// 蓝图表第 2–11 列（与基线 `upsert` 的 INSERT 列表一致；不含 `created_at`/`updated_at`）
const UPSERT_SQL: &str = "
INSERT INTO blueprints (
  chapter_number, title, role, purpose, key_events, characters,
  suspense_hook, user_guidance, notes, notes_updated_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
ON CONFLICT(chapter_number) DO UPDATE SET
  title = excluded.title,
  role = excluded.role,
  purpose = excluded.purpose,
  key_events = excluded.key_events,
  characters = excluded.characters,
  suspense_hook = excluded.suspense_hook,
  user_guidance = excluded.user_guidance,
  notes = excluded.notes,
  notes_updated_at = excluded.notes_updated_at,
  updated_at = datetime('now')
";

fn upsert_with(conn: &Connection, data: &BlueprintData) -> Result<(), String> {
    let characters = serde_json::to_string(&data.characters)
        .map_err(|error| format!("序列化角色列表失败：{error}"))?;
    conn.execute(
        UPSERT_SQL,
        rusqlite::params![
            data.chapter_number,
            data.title,
            data.role,
            data.purpose,
            data.key_events,
            characters,
            data.suspense_hook,
            data.user_guidance,
            data.notes,
            data.notes_updated_at,
        ],
    )
    .map(|_| ())
    .map_err(|error| format!("写入蓝图失败：{error}"))
}

/// 插入或更新单章蓝图
pub fn upsert(conn: &Connection, data: &BlueprintData) -> Result<(), String> {
    upsert_with(conn, data)
}

/// 批量插入/更新（单事务；任一条失败整体回滚）
pub fn upsert_many(conn: &Connection, items: &[BlueprintData]) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;
    for item in items {
        upsert_with(&tx, item)?;
    }
    tx.commit()
        .map_err(|error| format!("提交蓝图失败：{error}"))
}

/// 仅更新 notes（`notes_updated_at` 与 `updated_at` 取库时间）
/// 返回是否命中章节（对齐基线 `result.changes > 0`）。
pub fn update_notes(conn: &Connection, chapter_number: i64, notes: &str) -> Result<bool, String> {
    let changes = conn
        .execute(
            "UPDATE blueprints
             SET notes = ?1, notes_updated_at = datetime('now'), updated_at = datetime('now')
             WHERE chapter_number = ?2",
            rusqlite::params![notes, chapter_number],
        )
        .map_err(|error| format!("更新蓝图要点失败：{error}"))?;
    Ok(changes > 0)
}

/// 删除单章蓝图
pub fn delete(conn: &Connection, chapter_number: i64) -> Result<(), String> {
    conn.execute(
        "DELETE FROM blueprints WHERE chapter_number = ?1",
        [chapter_number],
    )
    .map(|_| ())
    .map_err(|error| format!("删除蓝图失败：{error}"))
}

/// 在调用方事务内清空蓝图相关事实（供 `clearAll` 与后续项目清理/导入复用）。
///
/// 顺序与基线 `clearBlueprintFactsWithinTransaction` 一致：
/// 先收敛 schema，再按"子表 → 父表"删除（避让外键）。
pub fn clear_blueprint_facts_within_transaction(conn: &Connection) -> Result<(), String> {
    ensure_blueprint_commit_schema(conn).map_err(|error| format!("蓝图提交表收敛失败：{error}"))?;
    conn.execute("DELETE FROM blueprint_character_sync_operations", [])
        .map_err(|error| format!("清空蓝图角色同步操作失败：{error}"))?;
    conn.execute("DELETE FROM blueprint_commit_operations", [])
        .map_err(|error| format!("清空蓝图提交记录失败：{error}"))?;
    conn.execute("DELETE FROM blueprints", [])
        .map_err(|error| format!("清空蓝图失败：{error}"))?;
    Ok(())
}

/// 删除所有章节蓝图（单事务）
pub fn clear_all(conn: &Connection) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;
    clear_blueprint_facts_within_transaction(&tx)?;
    tx.commit()
        .map_err(|error| format!("清空蓝图失败：{error}"))
}

// ============================================================================
// S2-b：范围提交（`db:blueprint-commit-range`）
// ============================================================================

/// 全量提交模式（必须从第 1 章起）
pub const COMMIT_MODE_FULL: &str = "full";
/// 区间替换模式
pub const COMMIT_MODE_REPLACE_RANGE: &str = "replace-range";

fn is_valid_commit_mode(mode: &str) -> bool {
    mode == COMMIT_MODE_FULL || mode == COMMIT_MODE_REPLACE_RANGE
}

/// 范围提交请求（对齐基线 `BlueprintRangeCommitRequest`）。
/// `mode` 保持字符串：非法值须由 [`assert_exact_range`] 给出「蓝图提交模式无效」而非反序列化错误。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintCommitRangeRequest {
    pub mode: String,
    pub operation_id: String,
    pub start_chapter: i64,
    pub end_chapter: i64,
    pub blueprints: Vec<BlueprintData>,
}

/// 角色同步完成回执中的名单证据（只存哈希/修订，事实仍在名单表）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintCharacterSyncCompletionRosterReceipt {
    pub operation_id: String,
    pub payload_hash: String,
    pub revision: i64,
    pub idempotent: bool,
}

/// 角色同步完成回执（对齐 `BlueprintCharacterSyncCompletionReceipt`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintCharacterSyncCompletionReceipt {
    pub blueprint_commit_operation_id: String,
    pub operation_id: String,
    /// `committed` | `already-satisfied`
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roster_receipt: Option<BlueprintCharacterSyncCompletionRosterReceipt>,
}

/// 持久的提交后工作项（可跨渲染层/应用重启恢复）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintCharacterSyncOperation {
    pub operation_id: String,
    pub blueprint_commit_operation_id: String,
    pub blueprint_commit_payload_hash: String,
    /// `pending` | `completed`
    pub status: String,
    pub start_chapter: i64,
    pub end_chapter: i64,
    pub character_sync_input: Vec<BlueprintData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_receipt: Option<BlueprintCharacterSyncCompletionReceipt>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// 范围提交回执（对齐基线 `BlueprintRangeCommitReceipt`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintCommitRangeReceipt {
    pub mode: String,
    pub operation_id: String,
    pub payload_hash: String,
    pub idempotent: bool,
    pub start_chapter: i64,
    pub end_chapter: i64,
    pub chapter_numbers: Vec<i64>,
    pub snapshot: Vec<BlueprintData>,
    /// 冻结的生成事实，用于重放角色候选同步
    pub character_sync_input: Vec<BlueprintData>,
    pub character_sync_operation: BlueprintCharacterSyncOperation,
}

/// `blueprint_commit_operations` 行
struct BlueprintCommitOperationRow {
    operation_id: String,
    payload_hash: String,
    mode: String,
    start_chapter: i64,
    end_chapter: i64,
    character_sync_input: String,
}

/// `blueprint_character_sync_operations` 行
struct BlueprintCharacterSyncOperationRow {
    operation_id: String,
    blueprint_commit_operation_id: String,
    blueprint_commit_payload_hash: String,
    status: String,
    start_chapter: i64,
    end_chapter: i64,
    character_sync_input: String,
    completion_receipt: Option<String>,
    created_at: String,
    updated_at: String,
    completed_at: Option<String>,
}

/// `character_roster_operations` 行（只读证据）
struct CharacterRosterOperationEvidenceRow {
    operation_id: String,
    payload_hash: String,
    committed_revision: i64,
    projection_hash: String,
}

/// 同步操作 ID 派生规则（对齐 `characterSyncOperationId`）
pub fn character_sync_operation_id(blueprint_commit_operation_id: &str) -> String {
    format!("blueprint-sync-{blueprint_commit_operation_id}")
}

/// 键序规范化（对齐基线 `canonicalize`）：对象按键排序、数组递归、其余原样。
/// 本域键均为 ASCII camelCase，`str::cmp` 的字节序与 `localeCompare('en-US')` 一致。
fn canonicalize_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonicalize_value).collect())
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                sorted.insert(key.clone(), canonicalize_value(&map[key]));
            }
            serde_json::Value::Object(sorted)
        }
        other => other.clone(),
    }
}

fn canonical_json(value: &serde_json::Value) -> Result<String, String> {
    serde_json::to_string(&canonicalize_value(value)).map_err(|error| error.to_string())
}

/// 幂等键：对规范 JSON 取 SHA-256（对齐 `commitPayloadHash`）
fn commit_payload_hash(request: &BlueprintCommitRangeRequest) -> Result<String, String> {
    let payload = serde_json::json!({
        "mode": request.mode,
        "startChapter": request.start_chapter,
        "endChapter": request.end_chapter,
        "blueprints": request.blueprints,
    });
    Ok(roster::hash_text(&canonical_json(&payload)?))
}

/// 范围与模式校验（对齐 `assertExactRange`，错误文案逐字一致）
fn assert_exact_range(request: &BlueprintCommitRangeRequest) -> Result<(), String> {
    if request.operation_id.trim().is_empty() {
        return Err("蓝图提交缺少操作 ID".to_string());
    }
    if !is_valid_commit_mode(&request.mode) {
        return Err("蓝图提交模式无效".to_string());
    }
    if request.start_chapter < 1 || request.end_chapter < request.start_chapter {
        return Err("蓝图提交范围无效".to_string());
    }
    if request.mode == COMMIT_MODE_FULL && request.start_chapter != 1 {
        return Err("全量蓝图提交必须从第 1 章开始".to_string());
    }

    let expected: Vec<i64> = (request.start_chapter..=request.end_chapter).collect();
    let actual: Vec<i64> = request
        .blueprints
        .iter()
        .map(|blueprint| blueprint.chapter_number)
        .collect();
    let unique: HashSet<i64> = actual.iter().copied().collect();
    let complete = expected.iter().all(|chapter| actual.contains(chapter));
    if actual.len() != expected.len() || unique.len() != actual.len() || !complete {
        return Err(format!(
            "蓝图提交必须完整且唯一地覆盖第 {}–{} 章",
            request.start_chapter, request.end_chapter
        ));
    }
    Ok(())
}

/// 读回精确范围并断言其完整覆盖（对齐 `readExactRange`）
fn read_exact_range(
    conn: &Connection,
    mode: &str,
    operation_id: &str,
    start_chapter: i64,
    end_chapter: i64,
) -> Result<Vec<BlueprintData>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT chapter_number, title, role, purpose, key_events, characters,
                    suspense_hook, user_guidance, notes, notes_updated_at
             FROM blueprints WHERE chapter_number BETWEEN ?1 AND ?2
             ORDER BY chapter_number ASC",
        )
        .map_err(|error| format!("读取蓝图范围失败：{error}"))?;
    let rows = stmt
        .query_map(
            rusqlite::params![start_chapter, end_chapter],
            BlueprintData::from_row,
        )
        .map_err(|error| format!("读取蓝图范围失败：{error}"))?;
    let mut snapshot = Vec::new();
    for row in rows {
        snapshot.push(row.map_err(|error| format!("读取蓝图范围失败：{error}"))?);
    }
    assert_exact_range(&BlueprintCommitRangeRequest {
        mode: mode.to_string(),
        operation_id: operation_id.to_string(),
        start_chapter,
        end_chapter,
        blueprints: snapshot.clone(),
    })?;
    Ok(snapshot)
}

fn parse_character_sync_input(serialized: &str) -> Result<Vec<BlueprintData>, String> {
    let parsed: serde_json::Value = serde_json::from_str(serialized)
        .map_err(|_| "蓝图角色同步操作的冻结输入已损坏".to_string())?;
    if !parsed.is_array() {
        return Err("蓝图角色同步操作的冻结输入格式无效".to_string());
    }
    serde_json::from_value(parsed).map_err(|_| "蓝图角色同步操作的冻结输入格式无效".to_string())
}

fn read_character_sync_input(
    row: &BlueprintCommitOperationRow,
) -> Result<Vec<BlueprintData>, String> {
    parse_character_sync_input(&row.character_sync_input)
}

fn parse_completion_receipt(
    serialized: Option<String>,
) -> Result<Option<BlueprintCharacterSyncCompletionReceipt>, String> {
    let Some(raw) = serialized else {
        return Ok(None);
    };
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).map_err(|_| "蓝图角色同步完成回执已损坏".to_string())?;
    if !parsed.is_object() {
        return Err("蓝图角色同步完成回执格式无效".to_string());
    }
    serde_json::from_value(parsed)
        .map(Some)
        .map_err(|_| "蓝图角色同步完成回执格式无效".to_string())
}

fn row_to_character_sync_operation(
    row: &BlueprintCharacterSyncOperationRow,
) -> Result<BlueprintCharacterSyncOperation, String> {
    let completion_receipt = parse_completion_receipt(row.completion_receipt.clone())?;
    if row.status == "pending" && completion_receipt.is_some() {
        return Err("待处理蓝图角色同步操作不应包含完成回执".to_string());
    }
    if row.status == "completed" && completion_receipt.is_none() {
        return Err("已完成蓝图角色同步操作缺少完成回执".to_string());
    }
    Ok(BlueprintCharacterSyncOperation {
        operation_id: row.operation_id.clone(),
        blueprint_commit_operation_id: row.blueprint_commit_operation_id.clone(),
        blueprint_commit_payload_hash: row.blueprint_commit_payload_hash.clone(),
        status: row.status.clone(),
        start_chapter: row.start_chapter,
        end_chapter: row.end_chapter,
        character_sync_input: parse_character_sync_input(&row.character_sync_input)?,
        completion_receipt,
        created_at: row.created_at.clone(),
        updated_at: row.updated_at.clone(),
        completed_at: row.completed_at.clone(),
    })
}

/// `blueprint_character_sync_operations` 的读取列（三条查询共用，避免列名漂移）
const SYNC_OPERATION_COLUMNS: &str = "operation_id, blueprint_commit_operation_id, \
     blueprint_commit_payload_hash, status, start_chapter, end_chapter, character_sync_input, \
     completion_receipt, created_at, updated_at, completed_at";

fn character_sync_operation_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<BlueprintCharacterSyncOperationRow> {
    Ok(BlueprintCharacterSyncOperationRow {
        operation_id: row.get(0)?,
        blueprint_commit_operation_id: row.get(1)?,
        blueprint_commit_payload_hash: row.get(2)?,
        status: row.get(3)?,
        start_chapter: row.get(4)?,
        end_chapter: row.get(5)?,
        character_sync_input: row.get(6)?,
        completion_receipt: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        completed_at: row.get(10)?,
    })
}

/// 读取单个角色同步操作（对齐 `readCharacterSyncOperation`）
fn read_character_sync_operation(
    conn: &Connection,
    operation_id: &str,
) -> Result<Option<BlueprintCharacterSyncOperation>, String> {
    let sql = format!(
        "SELECT {SYNC_OPERATION_COLUMNS} FROM blueprint_character_sync_operations \
         WHERE operation_id = ?1"
    );
    let row = conn
        .query_row(&sql, [operation_id], character_sync_operation_row)
        .optional()
        .map_err(|error| format!("读取蓝图角色同步操作失败：{error}"))?;
    match row {
        Some(row) => row_to_character_sync_operation(&row).map(Some),
        None => Ok(None),
    }
}

/// `db:blueprint-character-sync-list-pending` —— 列出可在重启后恢复的提交后工作项。
pub fn list_pending_character_sync_operations(
    conn: &Connection,
) -> Result<Vec<BlueprintCharacterSyncOperation>, String> {
    ensure_blueprint_commit_schema(conn).map_err(|error| error.to_string())?;
    let sql = format!(
        "SELECT {SYNC_OPERATION_COLUMNS} FROM blueprint_character_sync_operations \
         WHERE status = 'pending' ORDER BY created_at ASC, operation_id ASC"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|error| format!("读取蓝图角色同步操作失败：{error}"))?;
    let rows = stmt
        .query_map([], character_sync_operation_row)
        .map_err(|error| format!("读取蓝图角色同步操作失败：{error}"))?;
    let mut operations = Vec::new();
    for row in rows {
        let row = row.map_err(|error| format!("读取蓝图角色同步操作失败：{error}"))?;
        operations.push(row_to_character_sync_operation(&row)?);
    }
    Ok(operations)
}

/// `db:blueprint-character-sync-get` —— 读取单个操作（已完成时须与名单事实一致）。
pub fn get_character_sync_operation(
    conn: &Connection,
    operation_id: &str,
) -> Result<Option<BlueprintCharacterSyncOperation>, String> {
    if operation_id.trim().is_empty() {
        return Err("蓝图角色同步操作 ID 不能为空".to_string());
    }
    ensure_blueprint_commit_schema(conn).map_err(|error| error.to_string())?;
    let operation = read_character_sync_operation(conn, operation_id)?;
    if let Some(operation) = &operation {
        assert_authoritative_character_sync_completion(conn, operation)?;
    }
    Ok(operation)
}

/// `db:blueprint-character-sync-complete` —— 由权威事实确认并闭合待处理操作。
///
/// 幂等：已完成为 completed 时直接回读并校验，不重复 UPDATE。
pub fn complete_character_sync_operation(
    conn: &Connection,
    operation_id: &str,
) -> Result<BlueprintCharacterSyncOperation, String> {
    if operation_id.trim().is_empty() {
        return Err("蓝图角色同步操作 ID 不能为空".to_string());
    }
    ensure_blueprint_commit_schema(conn).map_err(|error| error.to_string())?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;

    let outcome: Result<BlueprintCharacterSyncOperation, String> = (|| {
        let operation = read_character_sync_operation(&tx, operation_id)?
            .ok_or_else(|| "待处理蓝图角色同步操作不存在".to_string())?;
        if operation.status == "completed" {
            assert_authoritative_character_sync_completion(&tx, &operation)?;
            return Ok(operation);
        }
        let completion_receipt = authoritative_character_sync_completion_receipt(&tx, &operation)?;
        let serialized = canonical_json(
            &serde_json::to_value(&completion_receipt).map_err(|error| error.to_string())?,
        )?;
        let changes = tx
            .execute(
                "UPDATE blueprint_character_sync_operations \
                 SET status = 'completed', completion_receipt = ?1, \
                     completed_at = datetime('now'), updated_at = datetime('now') \
                 WHERE operation_id = ?2 AND status = 'pending'",
                rusqlite::params![serialized, operation_id],
            )
            .map_err(|error| format!("完成蓝图角色同步操作失败：{error}"))?;
        if changes != 1 {
            return Err("蓝图角色同步完成状态更新失败".to_string());
        }
        let completed = read_character_sync_operation(&tx, operation_id)?;
        let Some(completed) = completed else {
            return Err("蓝图角色同步完成回执回读失败".to_string());
        };
        if completed.status != "completed" || completed.completion_receipt.is_none() {
            return Err("蓝图角色同步完成回执回读失败".to_string());
        }
        assert_authoritative_character_sync_completion(&tx, &completed)?;
        Ok(completed)
    })();

    match outcome {
        Ok(operation) => {
            tx.commit()
                .map_err(|error| format!("提交蓝图角色同步失败：{error}"))?;
            Ok(operation)
        }
        // `tx` drop 即回滚
        Err(error) => Err(error),
    }
}

/// 把冻结的生成事实叠加到持久化快照上（对齐 `snapshotWithCharacterSyncFacts`）
fn snapshot_with_character_sync_facts(
    persisted: &[BlueprintData],
    character_sync_input: &[BlueprintData],
) -> Vec<BlueprintData> {
    let input_by_chapter: HashMap<i64, &BlueprintData> = character_sync_input
        .iter()
        .map(|blueprint| (blueprint.chapter_number, blueprint))
        .collect();
    persisted
        .iter()
        .map(|blueprint| {
            let Some(input) = input_by_chapter.get(&blueprint.chapter_number) else {
                return blueprint.clone();
            };
            let mut next = blueprint.clone();
            if input.relationship_hints.is_some() {
                next.relationship_hints = input.relationship_hints.clone();
            }
            if input.new_character_candidates.is_some() {
                next.new_character_candidates = input.new_character_candidates.clone();
            }
            next
        })
        .collect()
}

/// 持久化字段等价性（对齐 `samePersistedBlueprint`；关系载荷不参与比较）
fn same_persisted_blueprint(left: &BlueprintData, right: &BlueprintData) -> bool {
    let characters_equal = serde_json::to_string(&left.characters).ok()
        == serde_json::to_string(&right.characters).ok();
    left.chapter_number == right.chapter_number
        && left.title == right.title
        && left.role == right.role
        && left.purpose == right.purpose
        && left.key_events == right.key_events
        && characters_equal
        && left.suspense_hook == right.suspense_hook
        && left.user_guidance == right.user_guidance
        && left.notes == right.notes
        && left.notes_updated_at == right.notes_updated_at
}

// ----- 角色同步事实校验（对齐 `blueprint-character-sync-evidence.ts`） -----

struct RelationshipFact {
    from: String,
    to: String,
    relation: String,
}

/// 端点解析：`from ?? source` / `to ?? target`（`??` 仅对缺失或 null 回退）
fn relationship_endpoint<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    primary: &str,
    fallback: &str,
) -> Option<&'a str> {
    object
        .get(primary)
        .filter(|value| !value.is_null())
        .or_else(|| object.get(fallback))
        .and_then(|value| value.as_str())
}

fn relationship_facts(hints: Option<&serde_json::Value>) -> Vec<RelationshipFact> {
    let Some(serde_json::Value::Array(candidates)) = hints else {
        return Vec::new();
    };
    let mut facts = Vec::new();
    for candidate in candidates {
        let Some(object) = candidate.as_object() else {
            continue;
        };
        let Some(raw_from) = relationship_endpoint(object, "from", "source") else {
            continue;
        };
        let Some(raw_to) = relationship_endpoint(object, "to", "target") else {
            continue;
        };
        if raw_from.trim().is_empty() || raw_to.trim().is_empty() {
            continue;
        }
        let relation = object
            .get("relation")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("相关");
        facts.push(RelationshipFact {
            from: raw_from.trim().to_string(),
            to: raw_to.trim().to_string(),
            relation: relation.to_string(),
        });
    }
    facts
}

fn has_legacy_relationship_notes(entry: &CharacterRosterEntry) -> bool {
    entry
        .legacy_relationship_notes
        .as_deref()
        .map(|notes| !notes.is_empty())
        .unwrap_or(false)
}

fn relationship_satisfied(entry: &CharacterRosterEntry, target_name: &str, relation: &str) -> bool {
    let target_key = roster::identity_key(target_name);
    entry.relationships.iter().any(|edge| {
        roster::identity_key(&edge.target) == target_key && edge.relation.trim() == relation
    })
}

/// 校验声明的新角色候选与关系补全（对齐 `blueprintCharacterSyncFactError`）。
/// 蓝图专属的普通名字仍属章节级规划引用，不要求成为角色卡。
pub fn blueprint_character_sync_fact_error(
    blueprints: &[BlueprintData],
    roster_entries: &[CharacterRosterEntry],
) -> Option<String> {
    let roster_by_name: HashMap<String, &CharacterRosterEntry> = roster_entries
        .iter()
        .map(|entry| (roster::identity_key(&entry.name), entry))
        .collect();

    for blueprint in blueprints {
        if let Some(candidates) = &blueprint.new_character_candidates {
            for candidate in candidates {
                let Some(name) = candidate.get("name").and_then(|value| value.as_str()) else {
                    continue;
                };
                if !roster_by_name.contains_key(&roster::identity_key(name)) {
                    return Some(format!(
                        "角色名单缺少第{}章蓝图声明的新角色候选「{}」",
                        blueprint.chapter_number, name
                    ));
                }
            }
        }
        for fact in relationship_facts(blueprint.relationship_hints.as_ref()) {
            let from = roster_by_name.get(&roster::identity_key(&fact.from));
            let to = roster_by_name.get(&roster::identity_key(&fact.to));
            let (Some(from), Some(to)) = (from, to) else {
                continue;
            };
            // 旧自由文本关系仍是只读权威证据，蓝图同步不得为闭合记账而做有损转换
            if has_legacy_relationship_notes(from) || has_legacy_relationship_notes(to) {
                continue;
            }
            if !relationship_satisfied(from, &to.name, &fact.relation)
                || !relationship_satisfied(to, &from.name, &fact.relation)
            {
                return Some(format!(
                    "角色名单缺少「{}—{}：{}」关系事实",
                    fact.from, fact.to, fact.relation
                ));
            }
        }
    }
    None
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// 从角色名单事实推导权威完成回执（对齐 `authoritativeCharacterSyncCompletionReceipt`）
fn authoritative_character_sync_completion_receipt(
    conn: &Connection,
    operation: &BlueprintCharacterSyncOperation,
) -> Result<BlueprintCharacterSyncCompletionReceipt, String> {
    let snapshot = roster::read(conn)?;
    if !matches!(
        snapshot.status,
        CharacterRosterStatus::Ready | CharacterRosterStatus::Empty
    ) {
        return Err("角色名单当前不可验证，已拒绝完成蓝图角色同步".to_string());
    }
    if let Some(fact_error) =
        blueprint_character_sync_fact_error(&operation.character_sync_input, &snapshot.entries)
    {
        return Err(format!("{fact_error}，已拒绝完成蓝图角色同步"));
    }

    let evidence = conn
        .query_row(
            "SELECT operation_id, payload_hash, committed_revision, projection_hash
             FROM character_roster_operations WHERE operation_id = ?1",
            [operation.operation_id.as_str()],
            |row| {
                Ok(CharacterRosterOperationEvidenceRow {
                    operation_id: row.get(0)?,
                    payload_hash: row.get(1)?,
                    committed_revision: row.get(2)?,
                    projection_hash: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("读取角色名单操作证据失败：{error}"))?;

    let Some(evidence) = evidence else {
        return Ok(BlueprintCharacterSyncCompletionReceipt {
            blueprint_commit_operation_id: operation.blueprint_commit_operation_id.clone(),
            operation_id: operation.operation_id.clone(),
            status: "already-satisfied".to_string(),
            roster_receipt: None,
        });
    };

    if evidence.operation_id != operation.operation_id
        || evidence.committed_revision < 1
        || evidence.committed_revision > snapshot.revision
        || !is_sha256_hex(&evidence.payload_hash)
        || !is_sha256_hex(&evidence.projection_hash)
        || (evidence.committed_revision == snapshot.revision
            && evidence.projection_hash != snapshot.projection_hash)
    {
        return Err("角色名单操作证据与当前事实不匹配，已拒绝完成蓝图角色同步".to_string());
    }

    Ok(BlueprintCharacterSyncCompletionReceipt {
        blueprint_commit_operation_id: operation.blueprint_commit_operation_id.clone(),
        operation_id: operation.operation_id.clone(),
        status: "committed".to_string(),
        roster_receipt: Some(BlueprintCharacterSyncCompletionRosterReceipt {
            operation_id: evidence.operation_id,
            payload_hash: evidence.payload_hash,
            revision: evidence.committed_revision,
            idempotent: false,
        }),
    })
}

/// 已完成操作的回执必须与当前名单事实一致（对齐 `assertAuthoritativeCharacterSyncCompletion`）
fn assert_authoritative_character_sync_completion(
    conn: &Connection,
    operation: &BlueprintCharacterSyncOperation,
) -> Result<(), String> {
    let Some(stored) = operation.completion_receipt.as_ref() else {
        return Ok(());
    };
    if operation.status != "completed" {
        return Ok(());
    }
    let authoritative = authoritative_character_sync_completion_receipt(conn, operation)?;
    let stored_json =
        canonical_json(&serde_json::to_value(stored).map_err(|error| error.to_string())?)?;
    let authoritative_json =
        canonical_json(&serde_json::to_value(&authoritative).map_err(|error| error.to_string())?)?;
    if stored_json != authoritative_json {
        return Err("蓝图角色同步完成回执与角色名单事实不匹配".to_string());
    }
    Ok(())
}

/// 读取已提交的操作行
fn read_commit_operation(
    conn: &Connection,
    operation_id: &str,
) -> Result<Option<BlueprintCommitOperationRow>, String> {
    conn.query_row(
        "SELECT operation_id, payload_hash, mode, start_chapter, end_chapter, character_sync_input
         FROM blueprint_commit_operations WHERE operation_id = ?1",
        [operation_id],
        |row| {
            Ok(BlueprintCommitOperationRow {
                operation_id: row.get(0)?,
                payload_hash: row.get(1)?,
                mode: row.get(2)?,
                start_chapter: row.get(3)?,
                end_chapter: row.get(4)?,
                character_sync_input: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(|error| format!("读取蓝图提交记录失败：{error}"))
}

/// 由已存在的提交记录重建幂等回执（对齐 `commitRange` 的幂等分支）
fn receipt_from_existing_operation(
    conn: &Connection,
    existing: &BlueprintCommitOperationRow,
    payload_hash: String,
) -> Result<BlueprintCommitRangeReceipt, String> {
    let character_sync_input = read_character_sync_input(existing)?;
    let persisted = read_exact_range(
        conn,
        &existing.mode,
        &existing.operation_id,
        existing.start_chapter,
        existing.end_chapter,
    )?;
    let snapshot = snapshot_with_character_sync_facts(&persisted, &character_sync_input);
    let character_sync_operation =
        read_character_sync_operation(conn, &character_sync_operation_id(&existing.operation_id))?
            .ok_or_else(|| "蓝图提交缺少可恢复的角色同步操作".to_string())?;
    assert_authoritative_character_sync_completion(conn, &character_sync_operation)?;
    Ok(BlueprintCommitRangeReceipt {
        mode: existing.mode.clone(),
        operation_id: existing.operation_id.clone(),
        payload_hash,
        idempotent: true,
        start_chapter: existing.start_chapter,
        end_chapter: existing.end_chapter,
        chapter_numbers: snapshot
            .iter()
            .map(|blueprint| blueprint.chapter_number)
            .collect(),
        snapshot,
        character_sync_input,
        character_sync_operation,
    })
}

/// `db:blueprint-commit-range` —— 单个逻辑范围只提交一次，并在同一事务内回读校验后返回收据。
pub fn commit_range(
    conn: &Connection,
    request: &BlueprintCommitRangeRequest,
) -> Result<BlueprintCommitRangeReceipt, String> {
    assert_exact_range(request)?;
    ensure_blueprint_commit_schema(conn).map_err(|error| error.to_string())?;
    let payload_hash = commit_payload_hash(request)?;

    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;

    let outcome: Result<BlueprintCommitRangeReceipt, String> = (|| {
        if let Some(existing) = read_commit_operation(&tx, &request.operation_id)? {
            if existing.payload_hash != payload_hash {
                return Err("操作 ID 已被用于不同的蓝图提交，已拒绝覆盖".to_string());
            }
            return receipt_from_existing_operation(&tx, &existing, payload_hash);
        }

        if request.mode == COMMIT_MODE_FULL {
            tx.execute(
                "DELETE FROM blueprints WHERE chapter_number < ?1 OR chapter_number > ?2",
                rusqlite::params![request.start_chapter, request.end_chapter],
            )
            .map_err(|error| format!("清理范围外蓝图失败：{error}"))?;
        }
        for blueprint in &request.blueprints {
            upsert_with(&tx, blueprint)?;
        }

        let persisted = read_exact_range(
            &tx,
            &request.mode,
            &request.operation_id,
            request.start_chapter,
            request.end_chapter,
        )?;

        let expected_by_chapter: HashMap<i64, &BlueprintData> = request
            .blueprints
            .iter()
            .map(|blueprint| (blueprint.chapter_number, blueprint))
            .collect();
        for saved in &persisted {
            let matches = expected_by_chapter
                .get(&saved.chapter_number)
                .map(|expected| same_persisted_blueprint(saved, expected))
                .unwrap_or(false);
            if !matches {
                return Err(format!(
                    "蓝图提交回读不一致：第 {} 章",
                    saved.chapter_number
                ));
            }
        }

        let character_sync_input: Vec<BlueprintData> = request.blueprints.clone();
        let serialized_input = serde_json::to_string(&character_sync_input)
            .map_err(|error| format!("序列化角色同步输入失败：{error}"))?;
        tx.execute(
            "INSERT INTO blueprint_commit_operations
             (operation_id, payload_hash, mode, start_chapter, end_chapter, character_sync_input)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                request.operation_id,
                payload_hash,
                request.mode,
                request.start_chapter,
                request.end_chapter,
                serialized_input,
            ],
        )
        .map_err(|error| format!("写入蓝图提交记录失败：{error}"))?;

        let sync_operation_id = character_sync_operation_id(&request.operation_id);
        tx.execute(
            "INSERT INTO blueprint_character_sync_operations
             (operation_id, blueprint_commit_operation_id, blueprint_commit_payload_hash,
              start_chapter, end_chapter, character_sync_input)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                sync_operation_id,
                request.operation_id,
                payload_hash,
                request.start_chapter,
                request.end_chapter,
                serialized_input,
            ],
        )
        .map_err(|error| format!("写入蓝图角色同步操作失败：{error}"))?;

        let snapshot = snapshot_with_character_sync_facts(&persisted, &character_sync_input);
        let character_sync_operation = read_character_sync_operation(&tx, &sync_operation_id)?
            .ok_or_else(|| "蓝图提交未创建可恢复的角色同步操作".to_string())?;

        Ok(BlueprintCommitRangeReceipt {
            mode: request.mode.clone(),
            operation_id: request.operation_id.clone(),
            payload_hash,
            idempotent: false,
            start_chapter: request.start_chapter,
            end_chapter: request.end_chapter,
            chapter_numbers: snapshot
                .iter()
                .map(|blueprint| blueprint.chapter_number)
                .collect(),
            snapshot,
            character_sync_input,
            character_sync_operation,
        })
    })();

    match outcome {
        Ok(receipt) => {
            tx.commit()
                .map_err(|error| format!("提交蓝图失败：{error}"))?;
            Ok(receipt)
        }
        // `tx` drop 即回滚
        Err(error) => Err(error),
    }
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

    fn sample(chapter: i64, notes: &str) -> BlueprintData {
        BlueprintData {
            chapter_number: chapter,
            title: format!("第 {chapter} 章"),
            role: "起".to_string(),
            purpose: "推进主线".to_string(),
            key_events: "事件".to_string(),
            characters: vec!["林清玄".to_string()],
            new_character_candidates: None,
            relationship_hints: None,
            suspense_hook: "钩子".to_string(),
            user_guidance: "指导".to_string(),
            notes: notes.to_string(),
            notes_updated_at: String::new(),
        }
    }

    #[test]
    fn upsert_get_all_and_get_by_chapter_test() {
        let conn = memory_db();
        upsert(&conn, &sample(2, "")).unwrap();
        upsert(&conn, &sample(1, "")).unwrap();

        let all = get_all(&conn).unwrap();
        assert_eq!(all.len(), 2);
        // 按章节号升序
        assert_eq!(all[0].chapter_number, 1);
        assert_eq!(all[1].chapter_number, 2);
        assert_eq!(all[0].characters, vec!["林清玄".to_string()]);
        // 非表列字段读回缺失
        assert!(all[0].new_character_candidates.is_none());
        assert!(all[0].relationship_hints.is_none());

        assert!(get_by_chapter(&conn, 3).unwrap().is_none());
        assert_eq!(get_by_chapter(&conn, 2).unwrap().unwrap().title, "第 2 章");
    }

    #[test]
    fn upsert_conflict_updates_fields_test() {
        let conn = memory_db();
        upsert(&conn, &sample(1, "旧")).unwrap();
        let mut updated = sample(1, "新");
        updated.title = "改名后的章节".to_string();
        updated.characters = vec!["苏晚".to_string()];
        upsert(&conn, &updated).unwrap();

        let row = get_by_chapter(&conn, 1).unwrap().unwrap();
        assert_eq!(row.title, "改名后的章节");
        assert_eq!(row.notes, "新");
        assert_eq!(row.characters, vec!["苏晚".to_string()]);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM blueprints", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "同章节必须覆盖而非新增");
    }

    #[test]
    fn upsert_many_commits_every_item_test() {
        let conn = memory_db();
        upsert(&conn, &sample(1, "旧")).unwrap();
        // 第 1 章为覆盖，另加第 2、3 章 → 共 3 条
        upsert_many(&conn, &[sample(1, "新"), sample(2, ""), sample(3, "")]).unwrap();
        let all = get_all(&conn).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].notes, "新", "同章节应被批量中的新值覆盖");
    }

    #[test]
    fn update_notes_hits_only_existing_chapter_test() {
        let conn = memory_db();
        upsert(&conn, &sample(1, "")).unwrap();

        assert!(update_notes(&conn, 1, "作者要点").unwrap());
        assert!(!update_notes(&conn, 99, "空章节").unwrap());

        let row = get_by_chapter(&conn, 1).unwrap().unwrap();
        assert_eq!(row.notes, "作者要点");
        assert!(!row.notes_updated_at.is_empty(), "应写入提取时间");
    }

    #[test]
    fn delete_and_clear_all_test() {
        let conn = memory_db();
        upsert(&conn, &sample(1, "")).unwrap();
        upsert(&conn, &sample(2, "")).unwrap();

        delete(&conn, 1).unwrap();
        assert_eq!(get_all(&conn).unwrap().len(), 1);

        // 预置一张提交记录，验证 clearAll 一并清空（含子表外键）
        conn.execute(
            "INSERT INTO blueprint_commit_operations
             (operation_id, payload_hash, mode, start_chapter, end_chapter, character_sync_input)
             VALUES ('op-1', 'hash', 'full', 1, 1, '[]')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO blueprint_character_sync_operations
             (operation_id, blueprint_commit_operation_id, blueprint_commit_payload_hash,
              start_chapter, end_chapter, character_sync_input)
             VALUES ('blueprint-sync-op-1', 'op-1', 'hash', 1, 1, '[]')",
            [],
        )
        .unwrap();

        clear_all(&conn).unwrap();
        assert!(get_all(&conn).unwrap().is_empty());
        let operations: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM blueprint_commit_operations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let syncs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM blueprint_character_sync_operations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(operations, 0);
        assert_eq!(syncs, 0);
    }

    #[test]
    fn ensure_blueprint_commit_schema_backfills_sync_operations_test() {
        let conn = memory_db();
        // 直接插入一条历史提交记录（模拟旧库），再跑收敛 → 自动补建同步操作
        conn.execute(
            "INSERT INTO blueprint_commit_operations
             (operation_id, payload_hash, mode, start_chapter, end_chapter, character_sync_input)
             VALUES ('legacy-op', 'legacy-hash', 'replace-range', 2, 3, '[]')",
            [],
        )
        .unwrap();
        crate::db::schema::ensure_blueprint_commit_schema(&conn).unwrap();
        let (operation_id, hash): (String, String) = conn
            .query_row(
                "SELECT operation_id, blueprint_commit_payload_hash
                 FROM blueprint_character_sync_operations
                 WHERE blueprint_commit_operation_id = 'legacy-op'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(operation_id, "blueprint-sync-legacy-op");
        assert_eq!(hash, "legacy-hash");
        // 幂等：重复收敛不新增
        crate::db::schema::ensure_blueprint_commit_schema(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM blueprint_character_sync_operations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    // ===== S2-b：范围提交 =====

    fn commit_request(
        mode: &str,
        start_chapter: i64,
        end_chapter: i64,
        chapters: &[i64],
    ) -> BlueprintCommitRangeRequest {
        BlueprintCommitRangeRequest {
            mode: mode.to_string(),
            operation_id: "op-1".to_string(),
            start_chapter,
            end_chapter,
            blueprints: chapters
                .iter()
                .map(|chapter| sample(*chapter, ""))
                .collect(),
        }
    }

    #[test]
    fn commit_payload_hash_is_canonical_and_field_sensitive_test() {
        let base = commit_request(COMMIT_MODE_FULL, 1, 1, &[1]);

        // 嵌套对象键序不影响哈希（canonicalize 排序）
        let mut left = base.clone();
        left.blueprints[0].relationship_hints = Some(serde_json::json!({ "b": 1, "a": 2 }));
        let mut right = base.clone();
        right.blueprints[0].relationship_hints = Some(serde_json::json!({ "a": 2, "b": 1 }));
        assert_eq!(
            commit_payload_hash(&left).unwrap(),
            commit_payload_hash(&right).unwrap()
        );

        // 操作 ID 不参与哈希（幂等键只描述事实）
        let mut other_id = base.clone();
        other_id.operation_id = "另一个操作".to_string();
        assert_eq!(
            commit_payload_hash(&base).unwrap(),
            commit_payload_hash(&other_id).unwrap()
        );

        // 模式与范围参与哈希
        let mut other_mode = base.clone();
        other_mode.mode = COMMIT_MODE_REPLACE_RANGE.to_string();
        assert_ne!(
            commit_payload_hash(&base).unwrap(),
            commit_payload_hash(&other_mode).unwrap()
        );

        // 哈希为 64 位小写 hex
        assert!(is_sha256_hex(&commit_payload_hash(&base).unwrap()));
    }

    #[test]
    fn assert_exact_range_covers_gaps_duplicates_and_modes_test() {
        // 完整覆盖
        assert!(assert_exact_range(&commit_request(COMMIT_MODE_FULL, 1, 2, &[1, 2])).is_ok());

        // 缺章
        let error = assert_exact_range(&commit_request(COMMIT_MODE_FULL, 1, 2, &[1])).unwrap_err();
        assert_eq!(error, "蓝图提交必须完整且唯一地覆盖第 1–2 章");

        // 重复章节（长度相同但集合缺项）
        let error =
            assert_exact_range(&commit_request(COMMIT_MODE_FULL, 1, 2, &[1, 1])).unwrap_err();
        assert_eq!(error, "蓝图提交必须完整且唯一地覆盖第 1–2 章");

        // 范围越界（起点为 0）
        assert_eq!(
            assert_exact_range(&commit_request(COMMIT_MODE_REPLACE_RANGE, 0, 1, &[0, 1]))
                .unwrap_err(),
            "蓝图提交范围无效"
        );

        // 终点小于起点
        assert_eq!(
            assert_exact_range(&commit_request(COMMIT_MODE_REPLACE_RANGE, 3, 1, &[])).unwrap_err(),
            "蓝图提交范围无效"
        );

        // full 必须从第 1 章开始
        assert_eq!(
            assert_exact_range(&commit_request(COMMIT_MODE_FULL, 2, 2, &[2])).unwrap_err(),
            "全量蓝图提交必须从第 1 章开始"
        );

        // 非法模式
        assert_eq!(
            assert_exact_range(&commit_request("replace", 1, 1, &[1])).unwrap_err(),
            "蓝图提交模式无效"
        );

        // 缺操作 ID
        let mut no_id = commit_request(COMMIT_MODE_FULL, 1, 1, &[1]);
        no_id.operation_id = "  ".to_string();
        assert_eq!(
            assert_exact_range(&no_id).unwrap_err(),
            "蓝图提交缺少操作 ID"
        );
    }

    #[test]
    fn commit_range_persists_replays_and_rejects_conflicts_test() {
        let conn = memory_db();
        let request = commit_request(COMMIT_MODE_FULL, 1, 2, &[1, 2]);

        let first = commit_range(&conn, &request).unwrap();
        assert!(!first.idempotent);
        assert_eq!(first.chapter_numbers, vec![1, 2]);
        assert_eq!(first.snapshot.len(), 2);
        assert_eq!(first.character_sync_operation.status, "pending");
        assert_eq!(
            first.character_sync_operation.operation_id,
            character_sync_operation_id("op-1")
        );
        // 提交后蓝图表已有持久化行
        assert_eq!(get_all(&conn).unwrap().len(), 2);

        // 幂等重放
        let replay = commit_range(&conn, &request).unwrap();
        assert!(replay.idempotent);
        assert_eq!(replay.payload_hash, first.payload_hash);
        assert_eq!(replay.chapter_numbers, vec![1, 2]);
        let operations: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM blueprint_commit_operations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(operations, 1, "幂等重放不得新增操作记录");

        // 同操作 ID + 不同负载
        let mut conflicting = request.clone();
        conflicting.blueprints[0].title = "被篡改".to_string();
        assert_eq!(
            commit_range(&conn, &conflicting).unwrap_err(),
            "操作 ID 已被用于不同的蓝图提交，已拒绝覆盖"
        );
    }

    #[test]
    fn commit_range_keeps_out_of_range_chapters_by_mode_test() {
        let conn = memory_db();
        upsert(&conn, &sample(1, "")).unwrap();
        upsert(&conn, &sample(2, "")).unwrap();

        // replace-range：只覆盖第 2 章，第 1 章保留
        let replaced = commit_range(
            &conn,
            &commit_request(COMMIT_MODE_REPLACE_RANGE, 2, 2, &[2]),
        )
        .unwrap();
        assert_eq!(replaced.snapshot.len(), 1);
        assert_eq!(get_all(&conn).unwrap().len(), 2);

        // full：裁剪范围外章节（不同操作 ID，避免幂等冲突）
        let mut shrink = commit_request(COMMIT_MODE_FULL, 1, 1, &[1]);
        shrink.operation_id = "op-shrink".to_string();
        commit_range(&conn, &shrink).unwrap();
        assert_eq!(get_all(&conn).unwrap().len(), 1);
    }

    #[test]
    fn commit_range_surfaces_character_sync_facts_in_receipt_test() {
        let conn = memory_db();
        let mut request = commit_request(COMMIT_MODE_FULL, 1, 1, &[1]);
        request.blueprints[0].new_character_candidates = Some(vec![serde_json::json!({
            "name": "苏晚",
            "role": "supporting"
        })]);
        request.blueprints[0].relationship_hints =
            Some(serde_json::json!([{ "from": "林清玄", "to": "苏晚", "relation": "师徒" }]));

        let receipt = commit_range(&conn, &request).unwrap();
        // 非表列字段不落库，但冻结在回执快照与角色同步输入中
        assert!(receipt.snapshot[0].new_character_candidates.is_some());
        assert!(receipt.snapshot[0].relationship_hints.is_some());
        assert!(receipt.character_sync_input[0].relationship_hints.is_some());
        // 库内行不含这些字段
        let persisted = get_by_chapter(&conn, 1).unwrap().unwrap();
        assert!(persisted.new_character_candidates.is_none());
        assert!(persisted.relationship_hints.is_none());
    }

    fn roster_entry(
        name: &str,
        relationships: Vec<roster::CharacterRosterRelationship>,
        legacy_notes: Option<&str>,
    ) -> CharacterRosterEntry {
        use crate::character_role::CharacterRole;
        CharacterRosterEntry {
            name: name.to_string(),
            role: CharacterRole::Supporting,
            gender: String::new(),
            age: String::new(),
            appearance: String::new(),
            personality: String::new(),
            background: String::new(),
            abilities: String::new(),
            motivation: String::new(),
            relationships,
            arc: String::new(),
            notes: String::new(),
            current_state: None,
            legacy_relationship_notes: legacy_notes.map(|notes| notes.to_string()),
        }
    }

    #[test]
    fn blueprint_character_sync_fact_error_matches_baseline_test() {
        // 缺声明的候选
        let mut blueprint = sample(3, "");
        blueprint.new_character_candidates = Some(vec![
            serde_json::json!({ "name": "苏晚", "role": "supporting" }),
        ]);
        assert_eq!(
            blueprint_character_sync_fact_error(&[blueprint.clone()], &[]).unwrap(),
            "角色名单缺少第3章蓝图声明的新角色候选「苏晚」"
        );
        // 名单已有候选 → 通过
        assert!(blueprint_character_sync_fact_error(
            &[blueprint.clone()],
            &[roster_entry("苏晚", Vec::new(), None)]
        )
        .is_none());

        // 关系事实缺失
        blueprint.new_character_candidates = None;
        blueprint.relationship_hints =
            Some(serde_json::json!([{ "from": "林清玄", "to": "苏晚", "relation": "师徒" }]));
        let entries = [
            roster_entry("林清玄", Vec::new(), None),
            roster_entry("苏晚", Vec::new(), None),
        ];
        assert_eq!(
            blueprint_character_sync_fact_error(&[blueprint.clone()], &entries).unwrap(),
            "角色名单缺少「林清玄—苏晚：师徒」关系事实"
        );

        // 双向关系齐备 → 通过
        let linked = [
            roster_entry(
                "林清玄",
                vec![roster::CharacterRosterRelationship {
                    target: "苏晚".to_string(),
                    relation: "师徒".to_string(),
                }],
                None,
            ),
            roster_entry(
                "苏晚",
                vec![roster::CharacterRosterRelationship {
                    target: "林清玄".to_string(),
                    relation: "师徒".to_string(),
                }],
                None,
            ),
        ];
        assert!(blueprint_character_sync_fact_error(&[blueprint.clone()], &linked).is_none());

        // 旧自由文本关系是只读权威证据 → 跳过校验
        let legacy = [
            roster_entry("林清玄", Vec::new(), Some("师徒情深")),
            roster_entry("苏晚", Vec::new(), None),
        ];
        assert!(blueprint_character_sync_fact_error(&[blueprint.clone()], &legacy).is_none());

        // 端点不在名单 → 跳过（章节级规划引用）
        assert!(blueprint_character_sync_fact_error(&[blueprint], &entries[..1]).is_none());
    }

    #[test]
    fn read_character_sync_operation_rejects_inconsistent_status_test() {
        let conn = memory_db();
        let request = commit_request(COMMIT_MODE_FULL, 1, 1, &[1]);
        commit_range(&conn, &request).unwrap();
        let sync_id = character_sync_operation_id("op-1");

        let pending = read_character_sync_operation(&conn, &sync_id)
            .unwrap()
            .unwrap();
        assert_eq!(pending.status, "pending");

        // pending 携带完整完成回执 → 视为损坏（基线：解析成功后再判状态）
        conn.execute(
            "UPDATE blueprint_character_sync_operations SET completion_receipt = ?1 WHERE operation_id = ?2",
            rusqlite::params![
                serde_json::json!({
                    "blueprintCommitOperationId": "op-1",
                    "operationId": "blueprint-sync-op-1",
                    "status": "committed"
                })
                .to_string(),
                sync_id.as_str(),
            ],
        )
        .unwrap();
        assert_eq!(
            read_character_sync_operation(&conn, &sync_id).unwrap_err(),
            "待处理蓝图角色同步操作不应包含完成回执"
        );

        // 未完成却标 completed 且无回执 → 视为损坏
        conn.execute(
            "UPDATE blueprint_character_sync_operations SET status = 'completed', completion_receipt = NULL WHERE operation_id = ?1",
            [sync_id.as_str()],
        )
        .unwrap();
        assert_eq!(
            read_character_sync_operation(&conn, &sync_id).unwrap_err(),
            "已完成蓝图角色同步操作缺少完成回执"
        );

        assert!(read_character_sync_operation(&conn, "不存在")
            .unwrap()
            .is_none());
    }

    // ===== S2-c：角色同步操作 =====

    #[test]
    fn list_and_get_character_sync_operations_test() {
        let conn = memory_db();
        assert!(list_pending_character_sync_operations(&conn)
            .unwrap()
            .is_empty());

        commit_range(&conn, &commit_request(COMMIT_MODE_FULL, 1, 1, &[1])).unwrap();
        let mut second = commit_request(COMMIT_MODE_REPLACE_RANGE, 2, 2, &[2]);
        second.operation_id = "op-2".to_string();
        commit_range(&conn, &second).unwrap();

        let pending = list_pending_character_sync_operations(&conn).unwrap();
        assert_eq!(pending.len(), 2);
        assert!(pending
            .iter()
            .all(|operation| operation.status == "pending"));

        let target = character_sync_operation_id("op-1");
        let fetched = get_character_sync_operation(&conn, &target)
            .unwrap()
            .unwrap();
        assert_eq!(fetched.blueprint_commit_operation_id, "op-1");
        assert_eq!(fetched.character_sync_input.len(), 1);

        assert!(get_character_sync_operation(&conn, "不存在")
            .unwrap()
            .is_none());
        assert_eq!(
            get_character_sync_operation(&conn, "  ").unwrap_err(),
            "蓝图角色同步操作 ID 不能为空"
        );
        assert_eq!(
            complete_character_sync_operation(&conn, "不存在").unwrap_err(),
            "待处理蓝图角色同步操作不存在"
        );

        // 完成后从待处理列表移除
        complete_character_sync_operation(&conn, &target).unwrap();
        let remaining = list_pending_character_sync_operations(&conn).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].blueprint_commit_operation_id, "op-2");
    }

    #[test]
    fn complete_character_sync_without_roster_evidence_is_already_satisfied_test() {
        let conn = memory_db();
        commit_range(&conn, &commit_request(COMMIT_MODE_FULL, 1, 1, &[1])).unwrap();

        let completed =
            complete_character_sync_operation(&conn, &character_sync_operation_id("op-1")).unwrap();
        assert_eq!(completed.status, "completed");
        let receipt = completed.completion_receipt.expect("应有完成回执");
        assert_eq!(receipt.status, "already-satisfied");
        assert!(
            receipt.roster_receipt.is_none(),
            "无名单操作证据时不返回名单证据"
        );
    }

    #[test]
    fn complete_character_sync_reports_committed_with_roster_evidence_test() {
        let conn = memory_db();
        commit_range(&conn, &commit_request(COMMIT_MODE_FULL, 1, 1, &[1])).unwrap();
        let sync_id = character_sync_operation_id("op-1");

        // 角色名单提交要求主台账已初始化（基线：打开项目时 init）
        crate::repositories::project_core_repository::init(
            &conn,
            "测试项目",
            crate::repositories::project_core_repository::DEFAULT_WRITING_LANGUAGE,
        )
        .unwrap();

        // 模拟渲染层以同一 operationId 提交名单事实（同步的唯一完成路径）
        let payload = serde_json::json!({
            "operationId": sync_id,
            "expectedRevision": 0,
            "schemaVersion": 1,
            "intent": "initialize",
            "entries": [{
                "name": "林清玄",
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
                "notes": ""
            }],
        });
        roster::commit(&conn, &payload).unwrap();

        let completed = complete_character_sync_operation(&conn, &sync_id).unwrap();
        assert_eq!(completed.status, "completed");
        let receipt = completed.completion_receipt.clone().expect("应有完成回执");
        assert_eq!(receipt.status, "committed");
        let roster_receipt = receipt.roster_receipt.expect("应有名单证据");
        assert_eq!(roster_receipt.operation_id, sync_id);
        assert_eq!(roster_receipt.revision, 1);
        assert!(!roster_receipt.idempotent);
        assert!(is_sha256_hex(&roster_receipt.payload_hash));

        // 幂等：重复完成返回同一回执（不回退已提交状态）
        let again = complete_character_sync_operation(&conn, &sync_id).unwrap();
        assert_eq!(again.completion_receipt, completed.completion_receipt);

        // 完成后的回读仍通过权威校验
        let fetched = get_character_sync_operation(&conn, &sync_id)
            .unwrap()
            .unwrap();
        assert_eq!(fetched.status, "completed");
        assert!(list_pending_character_sync_operations(&conn)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn complete_character_sync_rejects_missing_declared_candidate_test() {
        let conn = memory_db();
        let mut request = commit_request(COMMIT_MODE_FULL, 1, 1, &[1]);
        request.blueprints[0].new_character_candidates = Some(vec![serde_json::json!({
            "name": "苏晚",
            "role": "supporting"
        })]);
        commit_range(&conn, &request).unwrap();
        let sync_id = character_sync_operation_id("op-1");

        assert_eq!(
            complete_character_sync_operation(&conn, &sync_id).unwrap_err(),
            "角色名单缺少第1章蓝图声明的新角色候选「苏晚」，已拒绝完成蓝图角色同步"
        );

        // 失败必须回滚：状态仍为 pending 且仍在待处理列表
        let pending = read_character_sync_operation(&conn, &sync_id)
            .unwrap()
            .unwrap();
        assert_eq!(pending.status, "pending");
        assert!(pending.completion_receipt.is_none());
        assert_eq!(
            list_pending_character_sync_operations(&conn).unwrap().len(),
            1
        );
    }
}
