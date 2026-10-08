//! NarrativeThreadRepository —— 叙事线索（`narrative_thread_plans` + `narrative_thread_confirmations`）
//!
//! 平移自 `electron/repositories/narrative-thread-repository.ts`（批次 F1）。
//! 领域要点：
//! - **线索状态是纯派生投影**：`status` 取最后一条确认事件的事件类型，无事件即 `planned`；
//!   不落库，避免与确认事件产生第二份事实源；
//! - **短证据必须来自绑定的定稿正文**：确认事件时把 `evidence` 与
//!   `finalization_outbox.content_snapshot` 都去掉空白后做包含判定，
//!   拒绝模型凭空编造「章节里根本没写过」的线索证据；
//! - **只能绑定已定稿章节**：`drafts.status = 'finalized'` 且存在对应 outbox 行；
//! - `dormantChapters` / `overdue` 相对**当前最大定稿章号**计算，`resolved` / `abandoned`
//!   视为终态（`dormantChapters` 恒 0、`overdue` 恒 false）。
//!
//! 错误文案与基线逐字一致（`叙事线索计划参数无效` 等），由命令层决定是否加 `"Error: "` 前缀。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// 事件类型白名单（对齐基线 `['planted','progressing','resolved','abandoned']`）
const EVENT_TYPES: [&str; 4] = ["planted", "progressing", "resolved", "abandoned"];
/// 终态事件：出现即不再计休眠、不再判超期
const TERMINAL_STATUSES: [&str; 2] = ["resolved", "abandoned"];
/// JS `Number.isSafeInteger` 上界（2^53 - 1）
const JS_MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// 计划入参（对齐 `NarrativeThreadPlanInput`）
///
/// `type` 是 Rust 关键字，字段名取 `thread_type`，经 `serde(rename = "type")`
/// 与契约字段一一对应（显式 rename 优先于容器的 `rename_all`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrativeThreadPlanInput {
    pub title: String,
    #[serde(rename = "type")]
    pub thread_type: String,
    pub target_start_chapter: i64,
    pub target_end_chapter: i64,
    pub author_intent: String,
}

/// 计划记录（对齐 `NarrativeThreadPlanRecord`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrativeThreadPlanRecord {
    pub id: i64,
    pub title: String,
    #[serde(rename = "type")]
    pub thread_type: String,
    pub target_start_chapter: i64,
    pub target_end_chapter: i64,
    pub author_intent: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 事件确认入参（对齐 `NarrativeThreadEventInput`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrativeThreadEventInput {
    pub plan_id: i64,
    pub draft_id: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub evidence: String,
    pub reason: String,
}

/// 事件确认结果（对齐 `NarrativeThreadEvent` = 入参 + 章节上下文与时间戳）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrativeThreadEvent {
    pub id: i64,
    pub plan_id: i64,
    pub draft_id: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub evidence: String,
    pub reason: String,
    pub chapter_number: i64,
    pub chapter_title: String,
    pub created_at: String,
}

/// 列表视图（对齐 `NarrativeThreadView`）—— 计划 + 派生状态 + 事件
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrativeThreadView {
    pub id: i64,
    pub title: String,
    #[serde(rename = "type")]
    pub thread_type: String,
    pub target_start_chapter: i64,
    pub target_end_chapter: i64,
    pub author_intent: String,
    pub status: String,
    pub dormant_chapters: i64,
    pub overdue: bool,
    pub events: Vec<NarrativeThreadEvent>,
    pub created_at: String,
    pub updated_at: String,
}

/// 「相关线索」查询上下文（对齐 `NarrativeThreadChapterContext`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrativeThreadChapterContext {
    pub chapter_number: i64,
    pub title: String,
    pub key_events: String,
    pub characters: Vec<String>,
}

/// JS `String.prototype.length`（UTF-16 码元数），用于长度上限判定
fn js_length(value: &str) -> usize {
    value.encode_utf16().count()
}

/// 移除 **JS `\s`** 定义的空白字符（对齐 `/\s+/gu` 的 `replace`）。
///
/// Rust `regex` 的 `\s` 等价于 `\p{White_Space}`，与 ECMAScript 的集合并不相同：
/// - JS 含 `U+FEFF`（ZWNBSP）而 Rust 不含；
/// - Rust 含 `U+0085`（NEL）而 JS 不含。
///
/// 该归一化是「短证据是否出现在定稿正文中」的判据，差异会导致误判，
/// 故此处显式枚举 ECMAScript 集合，不借用 `\s`。
fn strip_js_whitespace(value: &str) -> String {
    value
        .chars()
        .filter(|character| !is_js_whitespace(*character))
        .collect()
}

/// ECMAScript `WhiteSpace` + `LineTerminator`（见 ECMA-262 11.2/11.3）
fn is_js_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'      // TAB
        | '\u{000A}'    // LF
        | '\u{000B}'    // VT
        | '\u{000C}'    // FF
        | '\u{000D}'    // CR
        | '\u{0020}'    // SP
        | '\u{00A0}'    // NBSP
        | '\u{1680}'    // OGHAM SPACE MARK
        | '\u{2000}'..='\u{200A}' // EN QUAD … HAIR SPACE
        | '\u{2028}'    // LINE SEPARATOR
        | '\u{2029}'    // PARAGRAPH SEPARATOR
        | '\u{202F}'    // NARROW NBSP
        | '\u{205F}'    // MEDIUM MATHEMATICAL SPACE
        | '\u{3000}'    // IDEOGRAPHIC SPACE
        | '\u{FEFF}'    // ZWNBSP (BOM)
    )
}

/// 计划参数校验（对齐基线的 `validatePlan`，含错误文案）
fn validate_plan(input: &NarrativeThreadPlanInput) -> Result<NarrativeThreadPlanInput, String> {
    let title = input.title.trim().to_string();
    let thread_type = input.thread_type.trim().to_string();
    let author_intent = input.author_intent.trim().to_string();
    if title.is_empty()
        || js_length(&title) > 120
        || thread_type.is_empty()
        || js_length(&thread_type) > 60
        || author_intent.is_empty()
        || js_length(&author_intent) > 1000
    {
        return Err("叙事线索计划参数无效".to_string());
    }
    if !(1..=JS_MAX_SAFE_INTEGER).contains(&input.target_start_chapter)
        || !(1..=JS_MAX_SAFE_INTEGER).contains(&input.target_end_chapter)
        || input.target_end_chapter < input.target_start_chapter
    {
        return Err("叙事线索目标章节范围无效".to_string());
    }
    Ok(NarrativeThreadPlanInput {
        title,
        thread_type,
        target_start_chapter: input.target_start_chapter,
        target_end_chapter: input.target_end_chapter,
        author_intent,
    })
}

fn row_to_plan(row: &rusqlite::Row<'_>) -> rusqlite::Result<NarrativeThreadPlanRecord> {
    Ok(NarrativeThreadPlanRecord {
        id: row.get("id")?,
        title: row.get("title")?,
        thread_type: row.get("type")?,
        target_start_chapter: row.get("target_start_chapter")?,
        target_end_chapter: row.get("target_end_chapter")?,
        author_intent: row
            .get::<_, Option<String>>("author_intent")?
            .unwrap_or_default(),
        created_at: row
            .get::<_, Option<String>>("created_at")?
            .unwrap_or_default(),
        updated_at: row
            .get::<_, Option<String>>("updated_at")?
            .unwrap_or_default(),
    })
}

/// 读取单条计划（仅供 `create_plan` / `update_plan` 回读；基线同名方法为私有）
pub fn get_plan(conn: &Connection, id: i64) -> Result<Option<NarrativeThreadPlanRecord>, String> {
    conn.query_row(
        "SELECT * FROM narrative_thread_plans WHERE id = ?1",
        [id],
        row_to_plan,
    )
    .optional()
    .map_err(|error| format!("读取叙事线索计划失败：{error}"))
}

/// 创建计划并回读（对齐基线 `createPlan`）
pub fn create_plan(
    conn: &Connection,
    input: &NarrativeThreadPlanInput,
) -> Result<NarrativeThreadPlanRecord, String> {
    let value = validate_plan(input)?;
    conn.execute(
        "INSERT INTO narrative_thread_plans (
           title, type, target_start_chapter, target_end_chapter, author_intent
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            value.title,
            value.thread_type,
            value.target_start_chapter,
            value.target_end_chapter,
            value.author_intent,
        ],
    )
    .map_err(|error| format!("创建叙事线索计划失败：{error}"))?;
    let id = conn.last_insert_rowid();
    get_plan(conn, id)?.ok_or_else(|| "创建叙事线索计划失败：回读为空".to_string())
}

/// 更新计划（对齐基线 `updatePlan`：`changes !== 1` 即判不存在）
pub fn update_plan(
    conn: &Connection,
    id: i64,
    input: &NarrativeThreadPlanInput,
) -> Result<NarrativeThreadPlanRecord, String> {
    let value = validate_plan(input)?;
    let changes = conn
        .execute(
            "UPDATE narrative_thread_plans
             SET title = ?1, type = ?2, target_start_chapter = ?3, target_end_chapter = ?4,
                 author_intent = ?5, updated_at = datetime('now')
             WHERE id = ?6",
            rusqlite::params![
                value.title,
                value.thread_type,
                value.target_start_chapter,
                value.target_end_chapter,
                value.author_intent,
                id,
            ],
        )
        .map_err(|error| format!("更新叙事线索计划失败：{error}"))?;
    if changes != 1 {
        return Err("叙事线索计划不存在".to_string());
    }
    get_plan(conn, id)?.ok_or_else(|| "叙事线索计划不存在".to_string())
}

/// 删除计划（事件由外键级联清除）
pub fn delete_plan(conn: &Connection, id: i64) -> Result<(), String> {
    let changes = conn
        .execute("DELETE FROM narrative_thread_plans WHERE id = ?1", [id])
        .map_err(|error| format!("删除叙事线索计划失败：{error}"))?;
    if changes != 1 {
        return Err("叙事线索计划不存在".to_string());
    }
    Ok(())
}

/// 确认一条线索事件（环节顺序严格对齐基线：校验 → 取绑定章节 → 证据包含判定 → 计划存在性 → 落库）
pub fn confirm_event(
    conn: &Connection,
    input: &NarrativeThreadEventInput,
) -> Result<NarrativeThreadEvent, String> {
    let evidence = input.evidence.trim().to_string();
    let reason = input.reason.trim().to_string();
    if !EVENT_TYPES.contains(&input.event_type.as_str())
        || evidence.is_empty()
        || js_length(&evidence) > 240
        || reason.is_empty()
        || js_length(&reason) > 500
    {
        return Err("叙事线索事件参数无效".to_string());
    }

    let source = conn
        .query_row(
            "SELECT drafts.id AS draft_id, drafts.chapter_number,
                    COALESCE(finalization_outbox.chapter_title, '') AS chapter_title,
                    finalization_outbox.content_snapshot
             FROM drafts
             JOIN finalization_outbox ON finalization_outbox.draft_id = drafts.id
             WHERE drafts.id = ?1 AND drafts.status = 'finalized'",
            [input.draft_id],
            |row| {
                Ok((
                    row.get::<_, i64>("draft_id")?,
                    row.get::<_, i64>("chapter_number")?,
                    row.get::<_, String>("chapter_title")?,
                    row.get::<_, Option<String>>("content_snapshot")?.unwrap_or_default(),
                ))
            },
        )
        .optional()
        .map_err(|error| format!("读取线索绑定章节失败：{error}"))?;
    let Some((_, chapter_number, chapter_title, content_snapshot)) = source else {
        return Err("线索事件只能绑定已定稿章节".to_string());
    };

    if !strip_js_whitespace(&content_snapshot).contains(&strip_js_whitespace(&evidence)) {
        return Err("短证据必须来自绑定的定稿正文".to_string());
    }

    if get_plan(conn, input.plan_id)?.is_none() {
        return Err("叙事线索计划不存在".to_string());
    }

    conn.execute(
        "INSERT INTO narrative_thread_confirmations (
           plan_id, draft_id, event_type, evidence, reason
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![input.plan_id, input.draft_id, input.event_type, evidence, reason],
    )
    .map_err(|error| format!("写入叙事线索事件失败：{error}"))?;
    let id = conn.last_insert_rowid();
    let created_at: Option<String> = conn
        .query_row(
            "SELECT created_at FROM narrative_thread_confirmations WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .map_err(|error| format!("读取叙事线索事件时间失败：{error}"))?;

    Ok(NarrativeThreadEvent {
        id,
        plan_id: input.plan_id,
        draft_id: input.draft_id,
        event_type: input.event_type.clone(),
        evidence,
        reason,
        chapter_number,
        chapter_title,
        created_at: created_at.unwrap_or_default(),
    })
}

/// 某计划的确认事件（只认已定稿章节；按章号升序、同章按确认顺序升序）
fn plan_events(conn: &Connection, plan_id: i64) -> Result<Vec<NarrativeThreadEvent>, String> {
    let mut statement = conn
        .prepare(
            "SELECT confirmations.id, confirmations.plan_id AS planId,
                    confirmations.draft_id AS draftId, confirmations.event_type AS type,
                    confirmations.evidence, confirmations.reason,
                    confirmations.created_at AS createdAt,
                    drafts.chapter_number AS chapterNumber,
                    COALESCE(finalization_outbox.chapter_title, '') AS chapterTitle
             FROM narrative_thread_confirmations confirmations
             JOIN drafts ON drafts.id = confirmations.draft_id AND drafts.status = 'finalized'
             JOIN finalization_outbox ON finalization_outbox.draft_id = drafts.id
             WHERE confirmations.plan_id = ?1
             ORDER BY drafts.chapter_number ASC, confirmations.id ASC",
        )
        .map_err(|error| format!("读取叙事线索事件失败：{error}"))?;
    let rows = statement
        .query_map([plan_id], |row| {
            Ok(NarrativeThreadEvent {
                id: row.get("id")?,
                plan_id: row.get("planId")?,
                draft_id: row.get("draftId")?,
                event_type: row.get("type")?,
                evidence: row.get("evidence")?,
                reason: row
                    .get::<_, Option<String>>("reason")?
                    .unwrap_or_default(),
                chapter_number: row.get("chapterNumber")?,
                chapter_title: row.get("chapterTitle")?,
                created_at: row
                    .get::<_, Option<String>>("createdAt")?
                    .unwrap_or_default(),
            })
        })
        .map_err(|error| format!("读取叙事线索事件失败：{error}"))?;
    let mut events = Vec::new();
    for row in rows {
        events.push(row.map_err(|error| format!("读取叙事线索事件失败：{error}"))?);
    }
    Ok(events)
}

/// 当前最大定稿章号（无定稿时为 0）
fn current_finalized_chapter(conn: &Connection) -> Result<i64, String> {
    conn.query_row(
        "SELECT COALESCE(MAX(chapter_number), 0) FROM drafts WHERE status = 'finalized'",
        [],
        |row| row.get(0),
    )
    .map_err(|error| format!("读取定稿章号失败：{error}"))
}

/// 列出全部线索（含派生 `status` / `dormantChapters` / `overdue`），按 `id` 升序
pub fn list(conn: &Connection) -> Result<Vec<NarrativeThreadView>, String> {
    let finalized = current_finalized_chapter(conn)?;
    let mut statement = conn
        .prepare("SELECT * FROM narrative_thread_plans ORDER BY id ASC")
        .map_err(|error| format!("读取叙事线索计划失败：{error}"))?;
    let plans = statement
        .query_map([], row_to_plan)
        .map_err(|error| format!("读取叙事线索计划失败：{error}"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("读取叙事线索计划失败：{error}"))?;

    let mut views = Vec::with_capacity(plans.len());
    for plan in plans {
        let events = plan_events(conn, plan.id)?;
        let status = events
            .last()
            .map(|event| event.event_type.clone())
            .unwrap_or_else(|| "planned".to_string());
        let terminal = TERMINAL_STATUSES.contains(&status.as_str());
        let last_chapter = events
            .last()
            .map(|event| event.chapter_number)
            .unwrap_or(plan.target_start_chapter);
        views.push(NarrativeThreadView {
            id: plan.id,
            title: plan.title,
            thread_type: plan.thread_type,
            target_start_chapter: plan.target_start_chapter,
            target_end_chapter: plan.target_end_chapter,
            author_intent: plan.author_intent,
            status,
            dormant_chapters: if terminal {
                0
            } else {
                (finalized - last_chapter).max(0)
            },
            overdue: !terminal && finalized > plan.target_end_chapter,
            events,
            created_at: plan.created_at,
            updated_at: plan.updated_at,
        });
    }
    Ok(views)
}

/// 列出与当前章节相关的活跃线索（终态线索恒不返回）
///
/// 相关性判定（与基线 `listRelevantActive` 逐支对齐）：
/// 1. 终态（`resolved` / `abandoned`）恒排除；
/// 2. 章号落在 `targetStartChapter..=targetEndChapter` 内即相关；
/// 3. 否则看文本：上下文角色名（`trim` 后非空）出现在线索文本中，
///    或上下文（`title + "\n" + keyEvents`）包含线索标题（标题需 ≥ 2 个 UTF-16 码元）。
pub fn list_relevant_active(
    conn: &Connection,
    context: &NarrativeThreadChapterContext,
) -> Result<Vec<NarrativeThreadView>, String> {
    let current_text = format!("{}\n{}", context.title, context.key_events);
    let characters: Vec<&str> = context
        .characters
        .iter()
        .map(|character| character.trim())
        .filter(|character| !character.is_empty())
        .collect();
    let threads = list(conn)?;
    Ok(threads
        .into_iter()
        .filter(|thread| {
            if TERMINAL_STATUSES.contains(&thread.status.as_str()) {
                return false;
            }
            if context.chapter_number >= thread.target_start_chapter
                && context.chapter_number <= thread.target_end_chapter
            {
                return true;
            }
            is_relevant_without_range(thread, &current_text, &characters)
        })
        .collect())
}

/// 相关性的「非章号」分支（与章号区间判定分开，便于单测覆盖）
fn is_relevant_without_range(    thread: &NarrativeThreadView,
    context_text: &str,
    characters: &[&str],
) -> bool {
    let mut thread_text = String::new();
    thread_text.push_str(&thread.title);
    thread_text.push('\n');
    thread_text.push_str(&thread.thread_type);
    thread_text.push('\n');
    thread_text.push_str(&thread.author_intent);
    for event in &thread.events {
        thread_text.push('\n');
        thread_text.push_str(&event.evidence);
        thread_text.push('\n');
        thread_text.push_str(&event.reason);
    }
    characters
        .iter()
        .any(|character| thread_text.contains(character))
        || (js_length(&thread.title) >= 2 && context_text.contains(&thread.title))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use crate::repositories::draft_repository;

    fn memory_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库应可打开");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("应启用外键");
        schema::create_tables(&conn).expect("建表应成功");
        conn
    }

    fn plan_input() -> NarrativeThreadPlanInput {
        NarrativeThreadPlanInput {
            title: "失落的信物".to_string(),
            thread_type: "foreshadow".to_string(),
            target_start_chapter: 3,
            target_end_chapter: 12,
            author_intent: "主角在终局前寻回母亲遗物".to_string(),
        }
    }

    #[test]
    fn plan_type_field_serializes_as_type_test() {
        let json = serde_json::to_value(plan_input()).unwrap();
        assert_eq!(json["type"], "foreshadow");
        assert!(json.get("threadType").is_none(), "不得输出 threadType");
        let back: NarrativeThreadPlanInput = serde_json::from_value(json).unwrap();
        assert_eq!(back.thread_type, "foreshadow");
    }

    #[test]
    fn create_plan_returns_record_with_trims_test() {
        let conn = memory_conn();
        let input = NarrativeThreadPlanInput {
            title: "  失落的信物  ".to_string(),
            thread_type: "  foreshadow ".to_string(),
            author_intent: "  意图  ".to_string(),
            ..plan_input()
        };
        let record = create_plan(&conn, &input).unwrap();
        assert_eq!(record.title, "失落的信物");
        assert_eq!(record.thread_type, "foreshadow");
        assert_eq!(record.author_intent, "意图");
        assert!(record.id >= 1);
        assert!(!record.created_at.is_empty());
    }

    #[test]
    fn create_plan_rejects_invalid_params_test() {
        let conn = memory_conn();
        let blank = NarrativeThreadPlanInput {
            title: "   ".to_string(),
            ..plan_input()
        };
        assert_eq!(create_plan(&conn, &blank).unwrap_err(), "叙事线索计划参数无效");

        let long_title = NarrativeThreadPlanInput {
            title: "字".repeat(121),
            ..plan_input()
        };
        assert_eq!(create_plan(&conn, &long_title).unwrap_err(), "叙事线索计划参数无效");

        let long_type = NarrativeThreadPlanInput {
            thread_type: "t".repeat(61),
            ..plan_input()
        };
        assert_eq!(create_plan(&conn, &long_type).unwrap_err(), "叙事线索计划参数无效");

        let long_intent = NarrativeThreadPlanInput {
            author_intent: "字".repeat(1001),
            ..plan_input()
        };
        assert_eq!(create_plan(&conn, &long_intent).unwrap_err(), "叙事线索计划参数无效");
    }

    #[test]
    fn create_plan_chapter_range_validation_test() {
        let conn = memory_conn();
        let zero_start = NarrativeThreadPlanInput {
            target_start_chapter: 0,
            ..plan_input()
        };
        assert_eq!(
            create_plan(&conn, &zero_start).unwrap_err(),
            "叙事线索目标章节范围无效"
        );

        let reversed = NarrativeThreadPlanInput {
            target_start_chapter: 10,
            target_end_chapter: 9,
            ..plan_input()
        };
        assert_eq!(
            create_plan(&conn, &reversed).unwrap_err(),
            "叙事线索目标章节范围无效"
        );

        // 超出 JS 安全整数上界
        let unsafe_range = NarrativeThreadPlanInput {
            target_end_chapter: JS_MAX_SAFE_INTEGER + 1,
            ..plan_input()
        };
        assert_eq!(
            create_plan(&conn, &unsafe_range).unwrap_err(),
            "叙事线索目标章节范围无效"
        );
    }

    #[test]
    fn update_and_delete_missing_plan_test() {
        let conn = memory_conn();
        assert_eq!(
            update_plan(&conn, 404, &plan_input()).unwrap_err(),
            "叙事线索计划不存在"
        );
        assert_eq!(delete_plan(&conn, 404).unwrap_err(), "叙事线索计划不存在");
    }

    #[test]
    fn update_plan_rewrites_fields_test() {
        let conn = memory_conn();
        let created = create_plan(&conn, &plan_input()).unwrap();
        let updated = update_plan(
            &conn,
            created.id,
            &NarrativeThreadPlanInput {
                title: "新的标题".to_string(),
                target_start_chapter: 5,
                target_end_chapter: 20,
                ..plan_input()
            },
        )
        .unwrap();
        assert_eq!(updated.title, "新的标题");
        assert_eq!(updated.target_start_chapter, 5);
        assert_eq!(updated.target_end_chapter, 20);
        assert_eq!(updated.id, created.id);
    }

    #[test]
    fn status_is_planned_without_events_test() {
        let conn = memory_conn();
        create_plan(&conn, &plan_input()).unwrap();
        let views = list(&conn).unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].status, "planned");
        assert_eq!(views[0].dormant_chapters, 0);
        assert!(!views[0].overdue);
        assert!(views[0].events.is_empty());
    }

    fn seed_finalized_draft(conn: &Connection, chapter: i64, title: &str, body: &str) -> i64 {
        let content_id = crate::repositories::content_repository::create(conn, body).unwrap();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id, word_count)
             VALUES (?1, 1, 'finalized', ?2, ?3)",
            rusqlite::params![chapter, content_id, body.chars().count() as i64],
        )
        .unwrap();
        let draft_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO finalization_outbox (
               finalization_id, draft_id, chapter_number, chapter_title, content_hash,
               content_revision, content_snapshot, target_file_name, publication_status
             ) VALUES (?1, ?2, ?3, ?4, 'hash', 1, ?5, 'f.md', 'published')",
            rusqlite::params![
                format!("fin-{draft_id}"),
                draft_id,
                chapter,
                title,
                body
            ],
        )
        .unwrap();
        draft_id
    }

    #[test]
    fn confirm_event_requires_finalized_draft_test() {
        let conn = memory_conn();
        let plan = create_plan(&conn, &plan_input()).unwrap();
        let error = confirm_event(
            &conn,
            &NarrativeThreadEventInput {
                plan_id: plan.id,
                draft_id: 999,
                event_type: "planted".to_string(),
                evidence: "信物".to_string(),
                reason: "首次出现".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(error, "线索事件只能绑定已定稿章节");
    }

    #[test]
    fn confirm_event_rejects_non_finalized_status_test() {
        let conn = memory_conn();
        let plan = create_plan(&conn, &plan_input()).unwrap();
        let content_id = crate::repositories::content_repository::create(&conn, "正文").unwrap();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id, word_count)
             VALUES (3, 1, 'draft', ?1, 2)",
            [content_id],
        )
        .unwrap();
        let draft_id = conn.last_insert_rowid();
        let error = confirm_event(
            &conn,
            &NarrativeThreadEventInput {
                plan_id: plan.id,
                draft_id,
                event_type: "planted".to_string(),
                evidence: "正文".to_string(),
                reason: "理由".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(error, "线索事件只能绑定已定稿章节");
    }

    #[test]
    fn confirm_event_evidence_must_come_from_finalized_body_test() {
        let conn = memory_conn();
        let plan = create_plan(&conn, &plan_input()).unwrap();
        let draft_id = seed_finalized_draft(&conn, 3, "第三章", "她握紧了那枚青铜钥匙。");
        let error = confirm_event(
            &conn,
            &NarrativeThreadEventInput {
                plan_id: plan.id,
                draft_id,
                event_type: "planted".to_string(),
                evidence: "银色的戒指".to_string(),
                reason: "理由".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(error, "短证据必须来自绑定的定稿正文");
    }

    #[test]
    fn confirm_event_ignores_whitespace_when_matching_evidence_test() {
        let conn = memory_conn();
        let plan = create_plan(&conn, &plan_input()).unwrap();
        let draft_id = seed_finalized_draft(&conn, 3, "第三章", "她 握紧 了那枚 青铜钥匙 。");
        let event = confirm_event(
            &conn,
            &NarrativeThreadEventInput {
                plan_id: plan.id,
                draft_id,
                event_type: "planted".to_string(),
                evidence: "那枚青铜钥匙".to_string(),
                reason: "理由".to_string(),
            },
        )
        .unwrap();
        assert_eq!(event.chapter_number, 3);
        assert_eq!(event.chapter_title, "第三章");
        assert_eq!(event.event_type, "planted");
        assert!(!event.created_at.is_empty());
    }

    #[test]
    fn confirm_event_rejects_invalid_type_or_length_test() {
        let conn = memory_conn();
        let plan = create_plan(&conn, &plan_input()).unwrap();
        let draft_id = seed_finalized_draft(&conn, 3, "第三章", "青铜钥匙。");
        let invalid_type = confirm_event(
            &conn,
            &NarrativeThreadEventInput {
                plan_id: plan.id,
                draft_id,
                event_type: "planned".to_string(),
                evidence: "青铜钥匙".to_string(),
                reason: "理由".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(invalid_type, "叙事线索事件参数无效");

        let long_evidence = confirm_event(
            &conn,
            &NarrativeThreadEventInput {
                plan_id: plan.id,
                draft_id,
                event_type: "planted".to_string(),
                evidence: "字".repeat(241),
                reason: "理由".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(long_evidence, "叙事线索事件参数无效");

        let blank_reason = confirm_event(
            &conn,
            &NarrativeThreadEventInput {
                plan_id: plan.id,
                draft_id,
                event_type: "planted".to_string(),
                evidence: "青铜钥匙".to_string(),
                reason: "   ".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(blank_reason, "叙事线索事件参数无效");
    }

    #[test]
    fn confirm_event_missing_plan_test() {
        let conn = memory_conn();
        let draft_id = seed_finalized_draft(&conn, 3, "第三章", "青铜钥匙。");
        let error = confirm_event(
            &conn,
            &NarrativeThreadEventInput {
                plan_id: 999,
                draft_id,
                event_type: "planted".to_string(),
                evidence: "青铜钥匙".to_string(),
                reason: "理由".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(error, "叙事线索计划不存在");
    }

    #[test]
    fn list_derives_status_dormant_and_overdue_test() {
        let conn = memory_conn();
        let plan = create_plan(&conn, &plan_input()).unwrap();
        let draft_id = seed_finalized_draft(&conn, 5, "第五章", "青铜钥匙出现了。");
        confirm_event(
            &conn,
            &NarrativeThreadEventInput {
                plan_id: plan.id,
                draft_id,
                event_type: "planted".to_string(),
                evidence: "青铜钥匙出现了".to_string(),
                reason: "首次出现".to_string(),
            },
        )
        .unwrap();
        // 定稿推进到第 20 章（超过 target_end_chapter = 12）
        seed_finalized_draft(&conn, 20, "第二十章", "终局。");

        let views = list(&conn).unwrap();
        assert_eq!(views[0].status, "planted");
        assert_eq!(views[0].dormant_chapters, 15, "20 - 5 = 15");
        assert!(views[0].overdue, "20 > 12 应判超期");
        assert_eq!(views[0].events.len(), 1);
    }

    #[test]
    fn terminal_status_freezes_dormant_and_overdue_test() {
        let conn = memory_conn();
        let plan = create_plan(&conn, &plan_input()).unwrap();
        let draft_id = seed_finalized_draft(&conn, 5, "第五章", "谜题揭晓。");
        confirm_event(
            &conn,
            &NarrativeThreadEventInput {
                plan_id: plan.id,
                draft_id,
                event_type: "resolved".to_string(),
                evidence: "谜题揭晓".to_string(),
                reason: "线索收束".to_string(),
            },
        )
        .unwrap();
        seed_finalized_draft(&conn, 40, "第四十章", "后日谈。");

        let views = list(&conn).unwrap();
        assert_eq!(views[0].status, "resolved");
        assert_eq!(views[0].dormant_chapters, 0);
        assert!(!views[0].overdue);
        // 终态线索不出现在「相关活跃线索」中
        let relevant = list_relevant_active(
            &conn,
            &NarrativeThreadChapterContext {
                chapter_number: 41,
                title: "第四十一章".to_string(),
                key_events: "".to_string(),
                characters: vec![],
            },
        )
        .unwrap();
        assert!(relevant.is_empty());
    }

    #[test]
    fn dormant_never_negative_test() {
        let conn = memory_conn();
        create_plan(&conn, &plan_input()).unwrap();
        // 无定稿章节：finalized = 0，lastChapter = targetStartChapter = 3 → max(0, -3) = 0
        let views = list(&conn).unwrap();
        assert_eq!(views[0].dormant_chapters, 0);
    }

    #[test]
    fn events_exclude_non_finalized_chapters_test() {
        let conn = memory_conn();
        let plan = create_plan(&conn, &plan_input()).unwrap();
        let draft_id = seed_finalized_draft(&conn, 5, "第五章", "青铜钥匙。");
        confirm_event(
            &conn,
            &NarrativeThreadEventInput {
                plan_id: plan.id,
                draft_id,
                event_type: "planted".to_string(),
                evidence: "青铜钥匙".to_string(),
                reason: "理由".to_string(),
            },
        )
        .unwrap();
        // 把定稿行改回草稿态：事件应立即从列表投影中消失（status 回落 planned）
        conn.execute("UPDATE drafts SET status = 'draft' WHERE id = ?1", [draft_id])
            .unwrap();
        let views = list(&conn).unwrap();
        assert_eq!(views[0].status, "planned");
        assert!(views[0].events.is_empty());
    }

    #[test]
    fn list_relevant_active_matches_chapter_range_test() {
        let conn = memory_conn();
        create_plan(&conn, &plan_input()).unwrap(); // 3..=12
        let relevant = list_relevant_active(
            &conn,
            &NarrativeThreadChapterContext {
                chapter_number: 7,
                title: "第七章".to_string(),
                key_events: "".to_string(),
                characters: vec![],
            },
        )
        .unwrap();
        assert_eq!(relevant.len(), 1, "章号落在区间内应相关");

        let outside = list_relevant_active(
            &conn,
            &NarrativeThreadChapterContext {
                chapter_number: 30,
                title: "第三十章".to_string(),
                key_events: "".to_string(),
                characters: vec![],
            },
        )
        .unwrap();
        assert!(outside.is_empty(), "区间外且无文本命中应不相关");
    }

    #[test]
    fn list_relevant_active_matches_character_and_title_test() {
        let conn = memory_conn();
        create_plan(&conn, &plan_input()).unwrap();
        // 角色名命中线索文本
        let by_character = list_relevant_active(
            &conn,
            &NarrativeThreadChapterContext {
                chapter_number: 30,
                title: "第三十章".to_string(),
                key_events: "".to_string(),
                characters: vec![" 失落的信物 ".to_string()],
            },
        )
        .unwrap();
        assert_eq!(by_character.len(), 1, "线索文本含角色名应相关");

        // 章节上下文标题命中线索标题（标题长度 >= 2）
        let by_context = list_relevant_active(
            &conn,
            &NarrativeThreadChapterContext {
                chapter_number: 30,
                title: "关于失落的信物的回忆".to_string(),
                key_events: "".to_string(),
                characters: vec![],
            },
        )
        .unwrap();
        assert_eq!(by_context.len(), 1, "上下文含线索标题应相关");

        // 单字符标题不参与文本命中（js_length >= 2 门槛）
        let short = NarrativeThreadPlanInput {
            title: "信".to_string(),
            ..plan_input()
        };
        let short_plan = create_plan(&conn, &short).unwrap();
        let short_hit = list_relevant_active(
            &conn,
            &NarrativeThreadChapterContext {
                chapter_number: 30,
                title: "关于信的回忆".to_string(),
                key_events: "".to_string(),
                characters: vec![],
            },
        )
        .unwrap();
        assert!(
            !short_hit.iter().any(|thread| thread.id == short_plan.id),
            "单字符标题不应参与文本命中"
        );
    }

    #[test]
    fn delete_plan_cascades_events_test() {
        let conn = memory_conn();
        let plan = create_plan(&conn, &plan_input()).unwrap();
        let draft_id = seed_finalized_draft(&conn, 5, "第五章", "青铜钥匙。");
        confirm_event(
            &conn,
            &NarrativeThreadEventInput {
                plan_id: plan.id,
                draft_id,
                event_type: "planted".to_string(),
                evidence: "青铜钥匙".to_string(),
                reason: "理由".to_string(),
            },
        )
        .unwrap();
        delete_plan(&conn, plan.id).unwrap();
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM narrative_thread_confirmations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 0, "删除计划应级联清除事件");
    }

    #[test]
    fn duplicate_confirmation_is_rejected_by_unique_index_test() {
        let conn = memory_conn();
        let plan = create_plan(&conn, &plan_input()).unwrap();
        let draft_id = seed_finalized_draft(&conn, 5, "第五章", "青铜钥匙。");
        let input = NarrativeThreadEventInput {
            plan_id: plan.id,
            draft_id,
            event_type: "planted".to_string(),
            evidence: "青铜钥匙".to_string(),
            reason: "理由".to_string(),
        };
        confirm_event(&conn, &input).unwrap();
        let error = confirm_event(&conn, &input).unwrap_err();
        assert!(
            error.contains("UNIQUE") || error.contains("唯一"),
            "重复确认应被唯一索引拒绝：{error}"
        );
    }

    #[test]
    fn strip_js_whitespace_matches_ecmascript_set_test() {
        // JS 含 U+FEFF（ZWNBSP）
        assert_eq!(strip_js_whitespace("a\u{FEFF}b"), "ab");
        // Rust 的 \s 含 U+0085（NEL），但 JS 不含 → 必须保留
        assert_eq!(strip_js_whitespace("a\u{0085}b"), "a\u{0085}b");
        // 常规空白与全角空格均移除
        assert_eq!(strip_js_whitespace(" a\tb\nc\u{3000}d "), "abcd");
    }

    #[test]
    fn js_length_counts_utf16_units_test() {
        assert_eq!(js_length("😀"), 2, "增补平面字符按 2 个码元计数");
        assert_eq!(js_length("字"), 1);
        // 60 个 emoji = 120 码元 > 120 上限？恰好等于上限 → 通过
        let conn = memory_conn();
        let title = "😀".repeat(60);
        let input = NarrativeThreadPlanInput {
            title,
            ..plan_input()
        };
        assert_eq!(js_length(&input.title), 120);
        assert!(create_plan(&conn, &input).is_ok(), "120 码元恰好允许");
        let too_long = NarrativeThreadPlanInput {
            title: "😀".repeat(61),
            ..plan_input()
        };
        assert_eq!(
            create_plan(&conn, &too_long).unwrap_err(),
            "叙事线索计划参数无效"
        );
        // 避免未使用告警：显式引用一次
        let _ = draft_repository::get_max_finalized_chapter(&conn);
    }

    #[test]
    fn narrative_thread_tables_have_expected_columns_test() {
        let conn = memory_conn();
        let plan_columns: Vec<String> = conn
            .prepare("PRAGMA table_info(narrative_thread_plans)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            plan_columns,
            vec![
                "id",
                "title",
                "type",
                "target_start_chapter",
                "target_end_chapter",
                "author_intent",
                "created_at",
                "updated_at"
            ]
        );
        let confirmation_columns: Vec<String> = conn
            .prepare("PRAGMA table_info(narrative_thread_confirmations)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            confirmation_columns,
            vec![
                "id",
                "plan_id",
                "draft_id",
                "event_type",
                "evidence",
                "reason",
                "created_at"
            ]
        );
    }
}
