//! LLMHistoryRepository —— LLM 调用日志（`llm_calls` 表）
//!
//! 平移自 `electron/repositories/llm-repository.ts`：调用日志、聚合统计与历史列表。
//! - `getStats` 的 token 三项在所有行都无用量时返回 `null`（不是 0）；
//! - `getHistory` 的 `finishReason` 由 `error_message` 推导（成功恒为 `stop`）。
//!
//! 另含 `summary_snapshots` 的旧式快照读写（对齐 `SummaryRepository` 的
//! `saveSnapshot` / `getLatestSnapshot`）；连续性投影部分随批次 E。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// LLM 调用聚合统计（对齐 `LLMHistoryRepository.getStats`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmCallStats {
    pub total_calls: i64,
    pub successful_calls: i64,
    pub failed_calls: i64,
    pub known_usage_calls: i64,
    /// 无任何已知用量时为 `null`
    pub total_tokens: Option<i64>,
    pub total_prompt_tokens: Option<i64>,
    pub total_completion_tokens: Option<i64>,
}

/// 历史记录条目（对齐 `getHistory` 的 SELECT 形状）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmCallHistoryEntry {
    pub id: i64,
    pub model_id: String,
    pub model_name: String,
    pub purpose: String,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub duration_ms: Option<i64>,
    /// 由 `success = 1` 判定（基线的列返回 0/1，此处按布尔语义输出）
    pub success: bool,
    /// `stop` / `length` / `content_filter` / `cancelled` / `error` / `unknown` / `null`
    pub finish_reason: Option<String>,
    pub created_at: String,
}

/// 旧式快照读数（对齐 `getLatestSnapshot`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummarySnapshot {
    pub character_states: String,
    pub chapter_number: i64,
}

/// JS 真值语义（用于 `call.success`：数字/字符串/对象均按其真值映射为 0/1）
fn json_truthy(value: Option<&serde_json::Value>) -> bool {
    match value {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::Bool(flag)) => *flag,
        Some(serde_json::Value::Number(number)) => {
            number.as_f64().map(|value| value != 0.0).unwrap_or(false)
        }
        Some(serde_json::Value::String(text)) => !text.is_empty(),
        Some(_) => true,
    }
}

/// 数字字段读取（非数字/缺失 → `None`，落库为 NULL）
fn json_number(value: Option<&serde_json::Value>) -> Option<i64> {
    match value {
        Some(serde_json::Value::Number(number)) => number.as_i64(),
        _ => None,
    }
}

fn json_text<'a>(value: Option<&'a serde_json::Value>) -> &'a str {
    value.and_then(|value| value.as_str()).unwrap_or_default()
}

/// 记录一次 LLM 调用
pub fn log_call(conn: &Connection, call: &serde_json::Value) -> Result<(), String> {
    // `model_id` 为 NOT NULL：缺失即拒绝（基线传给 better-sqlite3 会撞约束）
    let Some(model_id) = call.get("modelId").and_then(|value| value.as_str()) else {
        return Err("缺少模型 ID".to_string());
    };

    conn.execute(
        "INSERT INTO llm_calls (
           model_id, model_name, purpose, prompt_tokens, completion_tokens, total_tokens,
           duration_ms, success, error_message
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            model_id,
            json_text(call.get("modelName")),
            json_text(call.get("purpose")),
            json_number(call.get("promptTokens")),
            json_number(call.get("completionTokens")),
            json_number(call.get("totalTokens")),
            json_number(call.get("durationMs")),
            if json_truthy(call.get("success")) { 1 } else { 0 },
            json_text(call.get("errorMessage")),
        ],
    )
    .map(|_| ())
    .map_err(|error| format!("记录 LLM 调用失败：{error}"))
}

/// 获取调用统计（未记录任何调用时 token 三项为 `null`）
pub fn get_stats(conn: &Connection) -> Result<LlmCallStats, String> {
    conn.query_row(
        "SELECT
           COUNT(*),
           COALESCE(SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END), 0),
           COALESCE(SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END), 0),
           COALESCE(SUM(CASE WHEN total_tokens IS NOT NULL THEN 1 ELSE 0 END), 0),
           SUM(total_tokens),
           SUM(prompt_tokens),
           SUM(completion_tokens)
         FROM llm_calls",
        [],
        |row| {
            Ok(LlmCallStats {
                total_calls: row.get(0)?,
                successful_calls: row.get(1)?,
                failed_calls: row.get(2)?,
                known_usage_calls: row.get(3)?,
                total_tokens: row.get(4)?,
                total_prompt_tokens: row.get(5)?,
                total_completion_tokens: row.get(6)?,
            })
        },
    )
    .map_err(|error| format!("读取 LLM 统计失败：{error}"))
}

/// 获取最近 LLM 调用记录（按自增 ID 降序）
pub fn get_history(conn: &Connection, limit: i64) -> Result<Vec<LlmCallHistoryEntry>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, model_id, model_name, purpose, prompt_tokens, completion_tokens,
                    total_tokens, duration_ms, success,
                    CASE
                      WHEN success = 1 THEN 'stop'
                      WHEN error_message = 'finish:length' THEN 'length'
                      WHEN error_message = 'finish:content_filter' THEN 'content_filter'
                      WHEN error_message IN ('finish:cancelled', 'cancelled') THEN 'cancelled'
                      WHEN error_message = 'finish:error' THEN 'error'
                      WHEN error_message = 'finish:unknown' THEN 'unknown'
                      ELSE NULL
                    END AS finish_reason,
                    created_at
             FROM llm_calls ORDER BY id DESC LIMIT ?1",
        )
        .map_err(|error| format!("读取 LLM 历史失败：{error}"))?;
    let rows = stmt
        .query_map([limit], |row| {
            Ok(LlmCallHistoryEntry {
                id: row.get(0)?,
                model_id: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                model_name: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                purpose: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                prompt_tokens: row.get(4)?,
                completion_tokens: row.get(5)?,
                total_tokens: row.get(6)?,
                duration_ms: row.get(7)?,
                success: row.get::<_, Option<i64>>(8)?.unwrap_or_default() == 1,
                finish_reason: row.get(9)?,
                created_at: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
            })
        })
        .map_err(|error| format!("读取 LLM 历史失败：{error}"))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|error| format!("读取 LLM 历史失败：{error}"))?);
    }
    Ok(result)
}

/// 保存旧的跨用角色状态快照（`draft_id` 保持 NULL）
pub fn save_summary_snapshot(
    conn: &Connection,
    chapter_number: i64,
    character_states: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO summary_snapshots (chapter_number, character_states) VALUES (?1, ?2)",
        rusqlite::params![chapter_number, character_states],
    )
    .map(|_| ())
    .map_err(|error| format!("保存角色状态快照失败：{error}"))
}

/// 读取最新的旧式快照（只看 `draft_id IS NULL` 的行）
pub fn get_latest_summary_snapshot(conn: &Connection) -> Result<Option<SummarySnapshot>, String> {
    conn.query_row(
        "SELECT character_states, chapter_number
         FROM summary_snapshots
         WHERE draft_id IS NULL
         ORDER BY id DESC LIMIT 1",
        [],
        |row| {
            Ok(SummarySnapshot {
                character_states: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                chapter_number: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(|error| format!("读取最新快照失败：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;
    use serde_json::json;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        create_tables(&conn).expect("建表失败");
        conn
    }

    #[test]
    fn empty_stats_return_nulls_for_tokens_test() {
        let conn = memory_db();
        let stats = get_stats(&conn).unwrap();
        assert_eq!(stats.total_calls, 0);
        assert_eq!(stats.successful_calls, 0);
        assert_eq!(stats.failed_calls, 0);
        assert_eq!(stats.known_usage_calls, 0);
        // 无任何行时 SUM 为 NULL → 不落成 0
        assert_eq!(stats.total_tokens, None);
        assert_eq!(stats.total_prompt_tokens, None);
        assert_eq!(stats.total_completion_tokens, None);
        assert!(get_history(&conn, 50).unwrap().is_empty());
    }

    #[test]
    fn log_call_and_stats_aggregate_test() {
        let conn = memory_db();
        log_call(
            &conn,
            &json!({
                "modelId": "deepseek-chat",
                "modelName": "DeepSeek Chat",
                "purpose": "write",
                "promptTokens": 100,
                "completionTokens": 200,
                "totalTokens": 300,
                "durationMs": 1234,
                "success": true
            }),
        )
        .unwrap();
        log_call(
            &conn,
            &json!({
                "modelId": "deepseek-chat",
                "purpose": "write",
                "promptTokens": null,
                "completionTokens": null,
                "totalTokens": null,
                "durationMs": 10,
                "success": false,
                "errorMessage": "finish:length"
            }),
        )
        .unwrap();

        let stats = get_stats(&conn).unwrap();
        assert_eq!(stats.total_calls, 2);
        assert_eq!(stats.successful_calls, 1);
        assert_eq!(stats.failed_calls, 1);
        assert_eq!(stats.known_usage_calls, 1, "仅统计 total_tokens 非空的行");
        assert_eq!(stats.total_tokens, Some(300));
        assert_eq!(stats.total_prompt_tokens, Some(100));
        assert_eq!(stats.total_completion_tokens, Some(200));

        let history = get_history(&conn, 50).unwrap();
        assert_eq!(history.len(), 2);
        // 按自增 ID 降序
        assert!(history[0].id > history[1].id);
        assert!(!history[0].success);
        assert_eq!(history[0].finish_reason.as_deref(), Some("length"));
        assert_eq!(history[0].prompt_tokens, None);
        assert_eq!(history[0].model_name, "", "缺失字段回落空串");
        assert!(history[1].success);
        assert_eq!(history[1].finish_reason.as_deref(), Some("stop"));
        assert_eq!(history[1].duration_ms, Some(1234));
    }

    #[test]
    fn finish_reason_mapping_matches_baseline_test() {
        let conn = memory_db();
        let cases = [
            ("finish:content_filter", Some("content_filter")),
            ("finish:cancelled", Some("cancelled")),
            ("cancelled", Some("cancelled")),
            ("finish:error", Some("error")),
            ("finish:unknown", Some("unknown")),
            ("其它错误", None),
            ("", None),
        ];
        for (index, (error_message, expected)) in cases.iter().enumerate() {
            log_call(
                &conn,
                &json!({
                    "modelId": "m",
                    "success": false,
                    "errorMessage": error_message
                }),
            )
            .unwrap();
            let history = get_history(&conn, 1).unwrap();
            assert_eq!(
                history[0].finish_reason.as_deref(),
                *expected,
                "用例 {index}（{error_message}）映射错误"
            );
        }
    }

    #[test]
    fn log_call_requires_model_id_test() {
        let conn = memory_db();
        assert_eq!(
            log_call(&conn, &json!({ "purpose": "write", "success": true })).unwrap_err(),
            "缺少模型 ID"
        );
        // 非字符串 modelId 同样拒绝
        assert!(log_call(&conn, &json!({ "modelId": 42 })).is_err());
        assert_eq!(get_stats(&conn).unwrap().total_calls, 0);
    }

    #[test]
    fn success_uses_js_truthiness_test() {
        let conn = memory_db();
        // 数字真值：1 → 成功；0 → 失败
        log_call(&conn, &json!({ "modelId": "m", "success": 1 })).unwrap();
        log_call(&conn, &json!({ "modelId": "m", "success": 0 })).unwrap();
        // 缺失 success → false（列默认值不会被用，因为已显式写入 0）
        log_call(&conn, &json!({ "modelId": "m" })).unwrap();
        let stats = get_stats(&conn).unwrap();
        assert_eq!(stats.total_calls, 3);
        assert_eq!(stats.successful_calls, 1);
        assert_eq!(stats.failed_calls, 2);
    }

    #[test]
    fn history_respects_limit_test() {
        let conn = memory_db();
        for index in 0..5 {
            log_call(&conn, &json!({ "modelId": "m", "purpose": format!("p{index}") })).unwrap();
        }
        assert_eq!(get_history(&conn, 2).unwrap().len(), 2);
        assert_eq!(get_history(&conn, 50).unwrap().len(), 5);
    }

    #[test]
    fn summary_snapshot_ignores_draft_bound_rows_test() {
        let conn = memory_db();
        assert!(get_latest_summary_snapshot(&conn).unwrap().is_none());

        save_summary_snapshot(&conn, 3, "{\"林清玄\":{}}").unwrap();
        let latest = get_latest_summary_snapshot(&conn).unwrap().unwrap();
        assert_eq!(latest.chapter_number, 3);
        assert_eq!(latest.character_states, "{\"林清玄\":{}}");

        // 绑定定稿的行（批次 E 写入）不得被旧式读取看到
        conn.execute("INSERT INTO contents (body) VALUES ('正文')", [])
            .unwrap();
        let content_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, source, content_id, word_count, source_dependencies)
             VALUES (4, 1, 'write', ?1, 2, '[]')",
            [content_id],
        )
        .unwrap();
        let draft_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO summary_snapshots (draft_id, chapter_number, character_states)
             VALUES (?1, 4, '{\"苏晚\":{}}')",
            [draft_id],
        )
        .unwrap();

        let still_latest = get_latest_summary_snapshot(&conn).unwrap().unwrap();
        assert_eq!(still_latest.chapter_number, 3, "必须忽略 draft_id 非空的行");
    }
}
