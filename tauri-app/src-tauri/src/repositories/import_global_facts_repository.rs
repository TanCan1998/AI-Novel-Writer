//! import_global_facts —— 全局事实导入台账（批次 G3a）。
//!
//! 平移自 `electron/repositories/import-global-facts-repository.ts`：
//! 配置 / 非角色架构 / 角色名单事实的原子导入接缝。
//!
//! # 刻意偏离（对齐 G3 开工清单）
//!
//! - `import_global_fact_operations` 台账表**懒建**（对齐基线 `ensureLedger()`
//!   的 `CREATE TABLE IF NOT EXISTS`），不改 `db/schema.rs`——G schema 已获批
//!   且刻意固定 9 表，不新增（D2）。
//! - 收据 JSON 的键序由 serde 结构声明序决定；哈希只在 Tauri 内部自洽
//!   （存 → 回读比对），不要求与 Node 逐字节一致（D3）。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::import::parsing::sha256_hex;
use crate::repositories::character_roster_repository as character_roster;
use crate::repositories::project_core_repository as project_core;

/// 核心文本字段（对齐 `CORE_TEXT_FIELDS`，共 13 个；序列化键为 camelCase）
const CORE_TEXT_FIELDS: [&str; 13] = [
    "genre",
    "subGenre",
    "targetAudience",
    "plotStructure",
    "narrativePov",
    "goldenFinger",
    "globalGuidance",
    "coreOutline",
    "worldSetting",
    "protagonistProfile",
    "premise",
    "worldbuilding",
    "synopsis",
];

const PLOT_STRUCTURES: [&str; 6] = [
    "three_act",
    "heros_journey",
    "save_the_cat",
    "kishotenketsu",
    "multi_thread",
    "freeform",
];

const NARRATIVE_POVS: [&str; 4] = [
    "third_limited",
    "first_person",
    "third_omniscient",
    "multi_pov",
];

/// 对齐契约 `ImportGlobalFactsCore`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGlobalFactsCore {
    pub genre: String,
    pub sub_genre: String,
    pub target_audience: String,
    pub total_chapters: i64,
    pub words_per_chapter: i64,
    pub plot_structure: String,
    pub narrative_pov: String,
    pub golden_finger: String,
    pub global_guidance: String,
    pub core_outline: String,
    pub world_setting: String,
    pub protagonist_profile: String,
    pub premise: String,
    pub worldbuilding: String,
    pub synopsis: String,
}

/// 对齐契约 `ImportGlobalFactsRequest`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGlobalFactsRequest {
    pub operation_id: String,
    pub expected_roster_revision: i64,
    pub core: ImportGlobalFactsCore,
    pub character_entries: Vec<character_roster::CharacterRosterEntry>,
}

/// 对齐契约 `ImportGlobalFactsReceipt`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGlobalFactsReceipt {
    pub operation_id: String,
    pub payload_hash: String,
    pub idempotent: bool,
    pub core: ImportGlobalFactsCore,
    pub roster: character_roster::CharacterRosterCommitReceipt,
}

/// 台账行（对齐基线 `OperationRow`）
struct OperationRow {
    operation_id: String,
    payload_hash: String,
    receipt_json: String,
}

/// 对齐 `normalizedRequest`：trim + 枚举 / 范围校验，返回规范化请求。
fn normalized_request(
    candidate: ImportGlobalFactsRequest,
) -> Result<ImportGlobalFactsRequest, String> {
    let operation_id = candidate.operation_id.trim();
    if operation_id.is_empty() || operation_id.len() > 160 {
        return Err("导入全局事实 operationId 无效".to_string());
    }
    if candidate.expected_roster_revision < 0 {
        return Err("导入全局事实角色 revision 无效".to_string());
    }
    let mut core = candidate.core;
    for field in CORE_TEXT_FIELDS {
        // 基线按 camelCase 键取值；此处按字段映射逐项 trim 校验。
        let value = match field {
            "genre" => &mut core.genre,
            "subGenre" => &mut core.sub_genre,
            "targetAudience" => &mut core.target_audience,
            "plotStructure" => &mut core.plot_structure,
            "narrativePov" => &mut core.narrative_pov,
            "goldenFinger" => &mut core.golden_finger,
            "globalGuidance" => &mut core.global_guidance,
            "coreOutline" => &mut core.core_outline,
            "worldSetting" => &mut core.world_setting,
            "protagonistProfile" => &mut core.protagonist_profile,
            "premise" => &mut core.premise,
            "worldbuilding" => &mut core.worldbuilding,
            "synopsis" => &mut core.synopsis,
            _ => unreachable!("CORE_TEXT_FIELDS 越界"),
        };
        if value.trim().is_empty() {
            return Err(format!("导入全局事实字段 {field} 无效"));
        }
        *value = value.trim().to_string();
    }
    if !PLOT_STRUCTURES.contains(&core.plot_structure.as_str()) {
        return Err("导入全局事实字段 plotStructure 无效".to_string());
    }
    if !NARRATIVE_POVS.contains(&core.narrative_pov.as_str()) {
        return Err("导入全局事实字段 narrativePov 无效".to_string());
    }
    if core.total_chapters < 1 {
        return Err("导入全局事实总章数无效".to_string());
    }
    if core.words_per_chapter < 1 {
        return Err("导入全局事实章节字数无效".to_string());
    }
    if candidate.character_entries.is_empty() {
        return Err("导入全局事实角色名单不能为空".to_string());
    }
    Ok(ImportGlobalFactsRequest {
        operation_id: operation_id.to_string(),
        expected_roster_revision: candidate.expected_roster_revision,
        core,
        character_entries: candidate.character_entries,
    })
}

/// 对齐 `hashRequest`（`sha256(JSON.stringify(request))`）。
fn hash_request(request: &ImportGlobalFactsRequest) -> Result<String, String> {
    let json = serde_json::to_string(request)
        .map_err(|error| format!("全局事实请求序列化失败：{error}"))?;
    Ok(sha256_hex(&json))
}

/// 对齐 `coreSnapshot`：当前项目主台账的核心事实投影。
fn core_snapshot(conn: &Connection) -> Result<ImportGlobalFactsCore, String> {
    let current = project_core::get(conn)?.ok_or_else(|| "项目主台账未初始化".to_string())?;
    Ok(ImportGlobalFactsCore {
        genre: current.genre,
        sub_genre: current.sub_genre,
        target_audience: current.target_audience,
        total_chapters: current.total_chapters,
        words_per_chapter: current.words_per_chapter,
        plot_structure: current.plot_structure,
        narrative_pov: current.narrative_pov,
        golden_finger: current.golden_finger,
        global_guidance: current.global_guidance,
        core_outline: current.core_outline,
        world_setting: current.world_setting,
        protagonist_profile: current.protagonist_profile,
        premise: current.premise,
        worldbuilding: current.worldbuilding,
        synopsis: current.synopsis,
    })
}

/// 对齐 `parseReceipt`：回读收据并核对身份绑定。
fn parse_receipt(row: &OperationRow) -> Result<ImportGlobalFactsReceipt, String> {
    let parsed: ImportGlobalFactsReceipt = serde_json::from_str(&row.receipt_json)
        .map_err(|_| "导入全局事实收据损坏，已拒绝重放".to_string())?;
    if parsed.operation_id != row.operation_id || parsed.payload_hash != row.payload_hash {
        return Err("导入全局事实收据损坏，已拒绝重放".to_string());
    }
    Ok(parsed)
}

/// 对齐 `ensureLedger`（D2：懒建表，不动 `db/schema.rs`）。
fn ensure_ledger(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS import_global_fact_operations (
            operation_id TEXT PRIMARY KEY,
            payload_hash TEXT NOT NULL,
            receipt_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .map_err(|error| format!("创建全局事实台账失败：{error}"))
}

fn read_operation_row(
    conn: &Connection,
    operation_id: &str,
) -> Result<Option<OperationRow>, String> {
    conn.query_row(
        "SELECT operation_id, payload_hash, receipt_json
         FROM import_global_fact_operations WHERE operation_id = ?1",
        [operation_id],
        |row| {
            Ok(OperationRow {
                operation_id: row.get(0)?,
                payload_hash: row.get(1)?,
                receipt_json: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(|error| format!("读取全局事实台账失败：{error}"))
}

/// 事务体内的提交逻辑（批次 G3a 抽取）：供 [`ImportGlobalFactsRepository::commit`]
/// 与 `import_run_repository::commit_effect_receipt` 的跨仓原子事务复用。
/// 错误返回时 `tx` drop 即回滚，与基线 `db.transaction()` 语义一致。
pub(crate) fn commit_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    candidate: ImportGlobalFactsRequest,
) -> Result<ImportGlobalFactsReceipt, String> {
    let request = normalized_request(candidate)?;
    let payload_hash = hash_request(&request)?;
    if let Some(existing) = read_operation_row(tx, &request.operation_id)? {
        if existing.payload_hash != payload_hash {
            return Err("导入全局事实 operationId 已绑定不同载荷".to_string());
        }
        let stored = parse_receipt(&existing)?;
        let (_current_core, current_roster) = assert_facts_unchanged(tx, &stored)?;
        let mut roster = stored.roster;
        roster.revision = current_roster.revision;
        roster.snapshot = current_roster;
        return Ok(ImportGlobalFactsReceipt {
            idempotent: true,
            roster,
            ..stored
        });
    }

    let core_json = serde_json::to_value(&request.core)
        .map_err(|error| format!("全局事实核心序列化失败：{error}"))?;
    let core_map = core_json
        .as_object()
        .cloned()
        .ok_or_else(|| "导入全局事实配置无效".to_string())?;
    project_core::update(tx, &core_map)?;
    let roster_payload = serde_json::json!({
        "operationId": format!("{}:roster", request.operation_id),
        "expectedRevision": request.expected_roster_revision,
        "schemaVersion": character_roster::CHARACTER_ROSTER_SCHEMA_VERSION,
        "intent": "novel_import",
        "entries": request.character_entries,
    });
    // 基线在 db.transaction() 内嵌套调用 CharacterRosterRepository.commit；
    // rusqlite 不支持嵌套 BEGIN，故复用抽取出的事务体（G3a）。
    let roster = character_roster::commit_in_transaction(tx, &roster_payload)?;
    let receipt = ImportGlobalFactsReceipt {
        operation_id: request.operation_id.clone(),
        payload_hash: payload_hash.clone(),
        idempotent: false,
        core: core_snapshot(tx)?,
        roster,
    };
    tx.execute(
        "INSERT INTO import_global_fact_operations (operation_id, payload_hash, receipt_json)
         VALUES (?1, ?2, ?3)",
        [
            &request.operation_id,
            &payload_hash,
            &serde_json::to_string(&receipt)
                .map_err(|error| format!("全局事实收据序列化失败：{error}"))?,
        ],
    )
    .map_err(|error| format!("写入全局事实台账失败：{error}"))?;
    Ok(receipt)
}

/// 基线「已被后续修改」守卫：当前核心事实与 roster factHash 必须与收据一致。
fn assert_facts_unchanged(
    conn: &Connection,
    stored: &ImportGlobalFactsReceipt,
) -> Result<
    (
        ImportGlobalFactsCore,
        character_roster::CharacterRosterSnapshot,
    ),
    String,
> {
    let current_core = core_snapshot(conn)?;
    let current_roster = character_roster::read(conn)?;
    let current_core_json = serde_json::to_value(&current_core)
        .map_err(|error| format!("全局事实核心序列化失败：{error}"))?;
    let stored_core_json = serde_json::to_value(&stored.core)
        .map_err(|error| format!("全局事实核心序列化失败：{error}"))?;
    if current_core_json != stored_core_json
        || current_roster.fact_hash != stored.roster.snapshot.fact_hash
    {
        return Err("导入全局事实已被后续修改，不能将历史操作冒充为当前事实".to_string());
    }
    Ok((current_core, current_roster))
}

/** Atomic import seam for config, non-character architecture and roster facts. */
pub struct ImportGlobalFactsRepository;

impl ImportGlobalFactsRepository {
    /// 只读权威证据：已提交导入操作的回读（对齐 `getCommittedOperation`）。
    pub fn get_committed_operation(
        conn: &Connection,
        operation_id: &str,
    ) -> Result<Option<ImportGlobalFactsReceipt>, String> {
        if operation_id.trim().is_empty() {
            return Err("导入全局事实 operationId 无效".to_string());
        }
        ensure_ledger(conn)?;
        let Some(row) = read_operation_row(conn, operation_id)? else {
            return Ok(None);
        };
        let stored = parse_receipt(&row)?;
        assert_facts_unchanged(conn, &stored)?;
        Ok(Some(stored))
    }

    /// 对齐 `commit`：事务内幂等重放或「主台账更新 + 角色名单提交」。
    pub fn commit(
        conn: &Connection,
        candidate: ImportGlobalFactsRequest,
    ) -> Result<ImportGlobalFactsReceipt, String> {
        ensure_ledger(conn)?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|error| format!("开启事务失败：{error}"))?;
        let receipt = commit_in_transaction(&tx, candidate)?;
        tx.commit()
            .map_err(|error| format!("提交事务失败：{error}"))?;
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::character_roster_repository::CharacterRosterEntry;

    /// 内存库 + 主台账 / 角色名单 schema（对齐 `project_core` 测试的
    /// `create_tables`；角色名单表由 `migrate_character_roster_schema` 建立）。
    fn memory_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库");
        crate::db::schema::create_tables(&conn).expect("schema");
        crate::db::schema::migrate_character_roster_schema(&conn).expect("角色名单 schema");
        // 基线语义：全局事实提交要求主台账已初始化（项目打开时 init）
        project_core::init(&conn, "测试项目", project_core::DEFAULT_WRITING_LANGUAGE)
            .expect("初始化主台账");
        conn
    }

    fn core(genre: &str) -> ImportGlobalFactsCore {
        ImportGlobalFactsCore {
            genre: genre.to_string(),
            sub_genre: "都市".to_string(),
            target_audience: "general".to_string(),
            total_chapters: 100,
            words_per_chapter: 3000,
            plot_structure: "three_act".to_string(),
            narrative_pov: "third_limited".to_string(),
            golden_finger: "金手指".to_string(),
            global_guidance: "全局指导".to_string(),
            core_outline: "核心大纲".to_string(),
            world_setting: "世界观".to_string(),
            protagonist_profile: "主角".to_string(),
            premise: "前提".to_string(),
            worldbuilding: "世界构建".to_string(),
            synopsis: "情节大纲".to_string(),
        }
    }

    /// 经 serde 构造条目（`CharacterRosterEntry` 无 `Default`，
    /// 其余字段均为 `#[serde(default)]`）。
    fn entry(name: &str) -> CharacterRosterEntry {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "role": "protagonist",
        }))
        .expect("角色条目构造")
    }

    fn request(genre: &str, operation_id: &str) -> ImportGlobalFactsRequest {
        ImportGlobalFactsRequest {
            operation_id: operation_id.to_string(),
            expected_roster_revision: 0,
            core: core(genre),
            character_entries: vec![entry("角色一")],
        }
    }

    #[test]
    fn normalized_request_trims_and_rejects_invalid_test() {
        let mut bad = request("  ", "op-1");
        assert!(normalized_request(bad.clone()).is_err());
        bad.core.total_chapters = 0;
        assert!(normalized_request(bad).is_err());
        let mut bad_pov = request("仙侠", "op-1");
        bad_pov.core.narrative_pov = "unknown".to_string();
        assert!(normalized_request(bad_pov).is_err());
        let mut bad_revision = request("仙侠", "op-1");
        bad_revision.expected_roster_revision = -1;
        assert!(normalized_request(bad_revision).is_err());
        let trimmed = normalized_request(request("  仙侠  ", "  op-1  ")).unwrap();
        assert_eq!(trimmed.core.genre, "仙侠");
        assert_eq!(trimmed.operation_id, "op-1");
    }

    #[test]
    fn commit_then_get_committed_operation_roundtrip_test() {
        let conn = memory_conn();
        let receipt = ImportGlobalFactsRepository::commit(&conn, request("仙侠", "op-1")).unwrap();
        assert!(!receipt.idempotent);
        assert_eq!(receipt.core.genre, "仙侠");
        assert_eq!(receipt.roster.revision, 1);

        let read_back = ImportGlobalFactsRepository::get_committed_operation(&conn, "op-1")
            .unwrap()
            .expect("已提交操作应可读回");
        assert_eq!(read_back.payload_hash, receipt.payload_hash);
        assert_eq!(read_back.core.genre, "仙侠");

        // 幂等重放：同一载荷返回 idempotent 收据
        let replay = ImportGlobalFactsRepository::commit(&conn, request("仙侠", "op-1")).unwrap();
        assert!(replay.idempotent);
        assert_eq!(replay.roster.revision, 1);
    }

    #[test]
    fn commit_rejects_conflicting_payload_for_same_operation_test() {
        let conn = memory_conn();
        ImportGlobalFactsRepository::commit(&conn, request("仙侠", "op-1")).unwrap();
        let conflicting = ImportGlobalFactsRepository::commit(&conn, request("玄幻", "op-1"));
        assert!(conflicting.is_err());
        assert!(conflicting
            .unwrap_err()
            .contains("operationId 已绑定不同载荷"));
    }

    #[test]
    fn get_committed_operation_rejects_stale_evidence_test() {
        let conn = memory_conn();
        ImportGlobalFactsRepository::commit(&conn, request("仙侠", "op-1")).unwrap();
        // 后续修改主台账 → 历史操作不得冒充当前事实
        let mut updated = core("玄幻");
        updated.plot_structure = "heros_journey".to_string();
        let mut map = serde_json::Map::new();
        map.insert("genre".to_string(), serde_json::json!("玄幻"));
        map.insert(
            "plotStructure".to_string(),
            serde_json::json!("heros_journey"),
        );
        project_core::update(&conn, &map).unwrap();
        let stale = ImportGlobalFactsRepository::get_committed_operation(&conn, "op-1");
        assert!(stale.is_err());
        assert!(stale.unwrap_err().contains("已被后续修改"));
    }

    #[test]
    fn get_committed_operation_missing_returns_none_test() {
        let conn = memory_conn();
        assert!(
            ImportGlobalFactsRepository::get_committed_operation(&conn, "missing")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn commit_validates_core_enums_test() {
        let conn = memory_conn();
        let mut bad = request("仙侠", "op-enum");
        bad.core.plot_structure = "unknown".to_string();
        assert!(ImportGlobalFactsRepository::commit(&conn, bad).is_err());
        let mut bad_pov = request("仙侠", "op-enum");
        bad_pov.core.narrative_pov = "unknown".to_string();
        assert!(ImportGlobalFactsRepository::commit(&conn, bad_pov).is_err());
    }

    #[test]
    fn commit_rejects_empty_operation_id_test() {
        let conn = memory_conn();
        assert!(ImportGlobalFactsRepository::commit(&conn, request("仙侠", "  ")).is_err());
        assert!(ImportGlobalFactsRepository::get_committed_operation(&conn, "  ").is_err());
    }

    #[test]
    fn commit_creates_roster_with_bound_operation_id_test() {
        let conn = memory_conn();
        let receipt =
            ImportGlobalFactsRepository::commit(&conn, request("仙侠", "op-roster")).unwrap();
        // roster 提交操作 ID 绑定为 `${operationId}:roster`，且名单条目已落地
        assert_eq!(receipt.roster.operation_id, "op-roster:roster");
        assert_eq!(receipt.roster.snapshot.entries.len(), 1);
        assert_eq!(receipt.roster.snapshot.entries[0].name, "角色一");
    }
}
