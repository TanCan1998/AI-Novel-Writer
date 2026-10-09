//! ConsistencyExemptionRepository —— 一致性预检豁免（`consistency_exemptions` 表）
//!
//! 平移自 `electron/repositories/consistency-exemption-repository.ts`（批次 F1）。
//! 语义要点：
//! - 豁免是「作者显式放行某条稳定事实」的**证据**，不是写作禁令的反转；
//! - 保存采用 upsert：同一 `stable_fact_key` 再次保存会**复位 `revoked`**（重新生效）；
//! - 撤销是软删除（`revoked = 1`），行保留以便界面展示历史；
//! - 校验失败文案与基线逐字一致（`稳定事实键无效` / `豁免原因无效`）。

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// 豁免记录（对齐 `ConsistencyExemption`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsistencyExemption {
    pub stable_fact_key: String,
    pub reason: String,
    pub revoked: bool,
}

/// 基线的 `requireText(value, label)`：`trim()` 后非空且 `length <= 500`。
///
/// 上限按 **UTF-16 码元**计数（`encode_utf16().count()`），对齐 JS `String.prototype.length`；
/// 与 `draft_repository.rs` 的既有先例保持一致，避免含增补平面字符时长度判定漂移。
fn require_text(value: &str, label: &str) -> Result<String, String> {
    let normalized = value.trim();
    if normalized.is_empty() || normalized.encode_utf16().count() > 500 {
        return Err(format!("{label}无效"));
    }
    Ok(normalized.to_string())
}

/// 列出全部豁免（含已撤销），按 `stable_fact_key` 升序
pub fn list(conn: &Connection) -> Result<Vec<ConsistencyExemption>, String> {
    let mut statement = conn
        .prepare(
            "SELECT stable_fact_key AS stableFactKey, reason, revoked
             FROM consistency_exemptions ORDER BY stable_fact_key",
        )
        .map_err(|error| format!("读取一致性豁免失败：{error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(ConsistencyExemption {
                stable_fact_key: row.get("stableFactKey")?,
                reason: row.get("reason")?,
                revoked: row.get::<_, i64>("revoked")? == 1,
            })
        })
        .map_err(|error| format!("读取一致性豁免失败：{error}"))?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|error| format!("读取一致性豁免失败：{error}"))?);
    }
    Ok(items)
}

/// 保存（upsert）豁免：已存在则更新原因并**复位 `revoked = 0`**
pub fn save(conn: &Connection, stable_fact_key: &str, reason: &str) -> Result<(), String> {
    let key = require_text(stable_fact_key, "稳定事实键")?;
    let reason = require_text(reason, "豁免原因")?;
    conn.execute(
        "INSERT INTO consistency_exemptions (stable_fact_key, reason, revoked)
         VALUES (?1, ?2, 0)
         ON CONFLICT(stable_fact_key) DO UPDATE SET reason = excluded.reason, revoked = 0",
        rusqlite::params![key, reason],
    )
    .map_err(|error| format!("保存一致性豁免失败：{error}"))?;
    Ok(())
}

/// 撤销豁免（软删除）。基线的 `UPDATE` 不检查影响行数，故此处也不因不存在而报错。
pub fn revoke(conn: &Connection, stable_fact_key: &str) -> Result<(), String> {
    let key = require_text(stable_fact_key, "稳定事实键")?;
    conn.execute(
        "UPDATE consistency_exemptions SET revoked = 1 WHERE stable_fact_key = ?1",
        [key],
    )
    .map_err(|error| format!("撤销一致性豁免失败：{error}"))?;
    Ok(())
}

/// 读取单条豁免（仅供测试断言 upsert 语义；基线未暴露此频道）
#[cfg(test)]
pub fn find(
    conn: &Connection,
    stable_fact_key: &str,
) -> Result<Option<ConsistencyExemption>, String> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT stable_fact_key AS stableFactKey, reason, revoked
         FROM consistency_exemptions WHERE stable_fact_key = ?1",
        [stable_fact_key],
        |row| {
            Ok(ConsistencyExemption {
                stable_fact_key: row.get("stableFactKey")?,
                reason: row.get("reason")?,
                revoked: row.get::<_, i64>("revoked")? == 1,
            })
        },
    )
    .optional()
    .map_err(|error| format!("读取一致性豁免失败：{error}"))
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

    #[test]
    fn list_is_empty_initially_test() {
        let conn = memory_conn();
        assert!(list(&conn).unwrap().is_empty());
    }

    #[test]
    fn save_then_list_roundtrip_test() {
        let conn = memory_conn();
        save(&conn, "fact:abc", "作者已确认设定一致").unwrap();
        let items = list(&conn).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].stable_fact_key, "fact:abc");
        assert_eq!(items[0].reason, "作者已确认设定一致");
        assert!(!items[0].revoked);
    }

    #[test]
    fn save_trims_key_and_reason_test() {
        let conn = memory_conn();
        save(&conn, "  fact:abc  ", "  原因  ").unwrap();
        let found = find(&conn, "fact:abc").unwrap().unwrap();
        assert_eq!(found.stable_fact_key, "fact:abc");
        assert_eq!(found.reason, "原因");
    }

    #[test]
    fn save_upsert_resets_revoked_test() {
        let conn = memory_conn();
        save(&conn, "fact:abc", "首次原因").unwrap();
        revoke(&conn, "fact:abc").unwrap();
        assert!(find(&conn, "fact:abc").unwrap().unwrap().revoked);

        save(&conn, "fact:abc", "重新放行").unwrap();
        let found = find(&conn, "fact:abc").unwrap().unwrap();
        assert!(!found.revoked, "再次保存应复位 revoked");
        assert_eq!(found.reason, "重新放行");
        assert_eq!(list(&conn).unwrap().len(), 1, "upsert 不应产生第二行");
    }

    #[test]
    fn revoke_missing_key_is_silent_test() {
        let conn = memory_conn();
        // 基线不检查 changes，故不存在时也不报错
        revoke(&conn, "fact:missing").unwrap();
        assert!(list(&conn).unwrap().is_empty());
    }

    #[test]
    fn list_orders_by_key_test() {
        let conn = memory_conn();
        save(&conn, "fact:zzz", "z").unwrap();
        save(&conn, "fact:aaa", "a").unwrap();
        let keys: Vec<String> = list(&conn)
            .unwrap()
            .into_iter()
            .map(|item| item.stable_fact_key)
            .collect();
        assert_eq!(keys, vec!["fact:aaa", "fact:zzz"]);
    }

    #[test]
    fn empty_and_blank_inputs_are_rejected_test() {
        let conn = memory_conn();
        assert_eq!(save(&conn, "", "原因").unwrap_err(), "稳定事实键无效");
        assert_eq!(save(&conn, "   ", "原因").unwrap_err(), "稳定事实键无效");
        assert_eq!(save(&conn, "fact:abc", "   ").unwrap_err(), "豁免原因无效");
        assert_eq!(revoke(&conn, "").unwrap_err(), "稳定事实键无效");
    }

    #[test]
    fn length_limit_uses_utf16_code_units_test() {
        let conn = memory_conn();
        // 500 个 BMP 字符：恰好通过
        let ok = "a".repeat(500);
        assert!(save(&conn, "fact:ok", &ok).is_ok());
        // 501 个：拒绝
        let too_long = "a".repeat(501);
        assert_eq!(
            save(&conn, "fact:bad", &too_long).unwrap_err(),
            "豁免原因无效"
        );
        // 增补平面字符按 2 个码元计数（对齐 JS `.length`）
        let emoji = "😀".repeat(251); // JS length = 502
        assert_eq!(
            save(&conn, "fact:emoji", &emoji).unwrap_err(),
            "豁免原因无效"
        );
        let emoji_ok = "😀".repeat(250); // JS length = 500
        assert!(save(&conn, "fact:emoji-ok", &emoji_ok).is_ok());
    }

    #[test]
    fn revoked_column_has_check_constraint_test() {
        let conn = memory_conn();
        let error = conn
            .execute(
                "INSERT INTO consistency_exemptions (stable_fact_key, reason, revoked)
                 VALUES ('fact:x', 'r', 2)",
                [],
            )
            .unwrap_err();
        assert!(
            error.to_string().contains("CHECK"),
            "revoked 应受 CHECK(0,1) 约束：{error}"
        );
    }
}
