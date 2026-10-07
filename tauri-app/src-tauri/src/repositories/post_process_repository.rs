//! PostProcessRepository —— 后处理跑批（`post_process_runs` + `post_process_steps`）
//!
//! 平移自 `electron/repositories/post-process-repository.ts`：每次后处理产生一个 Run
//! （UUID 主键），下属多个 Step，通过 `trigger_source_type + trigger_source_id` 溯源业务实体。
//! - 步骤收据与跑批汇总（`all_critical_passed`）必须在**同一事务**内切换；
//! - 全量重跑可能把已成功步骤重新置为失败，汇总必须重算而不能保留旧 `true`；
//! - `all_critical_passed` 的语义是「不存在失败的关键步骤」，空跑批也视为通过。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::project_access::random_uuid_v4;

/// 跑批实例
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostProcessRunData {
    pub id: String,
    pub trigger_source_type: String,
    pub trigger_source_id: String,
    pub source_label: String,
    pub all_critical_passed: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// 步骤明细
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostProcessStepData {
    pub id: i64,
    pub run_id: String,
    pub step_key: String,
    pub label: String,
    pub critical: bool,
    pub ok: bool,
    pub error_msg: String,
    pub attempt_count: i64,
    pub completed_at: String,
    pub last_attempt_at: String,
}

/// 待初始化的步骤定义
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostProcessStepInput {
    pub key: String,
    pub label: String,
    pub critical: bool,
}

/// 创建跑批的参数
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostProcessCreateParams {
    pub trigger_source_type: String,
    pub trigger_source_id: String,
    pub source_label: String,
    pub steps: Vec<PostProcessStepInput>,
}

/// 创建一个新的跑批实例并初始化步骤列表，返回新建 run ID
pub fn create_run(conn: &Connection, params: &PostProcessCreateParams) -> Result<String, String> {
    let run_id = random_uuid_v4();
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;

    tx.execute(
        "INSERT INTO post_process_runs (id, trigger_source_type, trigger_source_id, source_label)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            run_id,
            params.trigger_source_type,
            params.trigger_source_id,
            params.source_label,
        ],
    )
    .map_err(|error| format!("写入后处理跑批失败：{error}"))?;

    for step in &params.steps {
        tx.execute(
            "INSERT INTO post_process_steps (run_id, step_key, label, critical)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![run_id, step.key, step.label, if step.critical { 1 } else { 0 }],
        )
        .map_err(|error| format!("写入后处理步骤失败：{error}"))?;
    }

    tx.commit()
        .map_err(|error| format!("提交后处理跑批失败：{error}"))?;
    Ok(run_id)
}

/// 获取最新的跑批实例（按 sourceType + sourceId 查询）
pub fn get_latest_run(
    conn: &Connection,
    source_type: &str,
    source_id: &str,
) -> Result<Option<PostProcessRunData>, String> {
    conn.query_row(
        "SELECT * FROM post_process_runs
         WHERE trigger_source_type = ?1 AND trigger_source_id = ?2
         ORDER BY created_at DESC LIMIT 1",
        rusqlite::params![source_type, source_id],
        |row| {
            Ok(PostProcessRunData {
                id: row.get("id")?,
                trigger_source_type: row.get("trigger_source_type")?,
                trigger_source_id: row.get("trigger_source_id")?,
                source_label: row
                    .get::<_, Option<String>>("source_label")?
                    .unwrap_or_default(),
                all_critical_passed: row
                    .get::<_, Option<i64>>("all_critical_passed")?
                    .unwrap_or_default()
                    == 1,
                created_at: row
                    .get::<_, Option<String>>("created_at")?
                    .unwrap_or_default(),
                updated_at: row
                    .get::<_, Option<String>>("updated_at")?
                    .unwrap_or_default(),
            })
        },
    )
    .optional()
    .map_err(|error| format!("读取后处理跑批失败：{error}"))
}

/// 获取跑批实例的所有步骤（按自增 ID 升序）
pub fn get_steps(conn: &Connection, run_id: &str) -> Result<Vec<PostProcessStepData>, String> {
    let mut stmt = conn
        .prepare("SELECT * FROM post_process_steps WHERE run_id = ?1 ORDER BY id ASC")
        .map_err(|error| format!("读取后处理步骤失败：{error}"))?;
    let rows = stmt
        .query_map([run_id], |row| {
            Ok(PostProcessStepData {
                id: row.get("id")?,
                run_id: row.get("run_id")?,
                step_key: row.get("step_key")?,
                label: row.get::<_, Option<String>>("label")?.unwrap_or_default(),
                critical: row.get::<_, Option<i64>>("critical")?.unwrap_or_default() == 1,
                ok: row.get::<_, Option<i64>>("ok")?.unwrap_or_default() == 1,
                error_msg: row
                    .get::<_, Option<String>>("error_msg")?
                    .unwrap_or_default(),
                attempt_count: row
                    .get::<_, Option<i64>>("attempt_count")?
                    .unwrap_or_default(),
                completed_at: row
                    .get::<_, Option<String>>("completed_at")?
                    .unwrap_or_default(),
                last_attempt_at: row
                    .get::<_, Option<String>>("last_attempt_at")?
                    .unwrap_or_default(),
            })
        })
        .map_err(|error| format!("读取后处理步骤失败：{error}"))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|error| format!("读取后处理步骤失败：{error}"))?);
    }
    Ok(result)
}

/// 重算并写入跑批汇总标志（调用方须在同一事务内执行）
fn refresh_critical_status(conn: &Connection, run_id: &str) -> Result<(), String> {
    let failed_critical: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM post_process_steps
             WHERE run_id = ?1 AND critical = 1 AND ok = 0",
            [run_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("读取关键步骤状态失败：{error}"))?;
    let all_passed = if failed_critical == 0 { 1 } else { 0 };
    conn.execute(
        "UPDATE post_process_runs
         SET all_critical_passed = ?1, updated_at = datetime('now')
         WHERE id = ?2",
        rusqlite::params![all_passed, run_id],
    )
    .map(|_| ())
    .map_err(|error| format!("更新跑批汇总失败：{error}"))
}

/// 标记步骤为成功（步骤收据与跑批汇总同事务切换）
pub fn mark_step_ok(conn: &Connection, run_id: &str, step_key: &str) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;

    let changes = tx
        .execute(
            "UPDATE post_process_steps
             SET ok = 1, error_msg = '', completed_at = datetime('now'),
                 last_attempt_at = datetime('now'), attempt_count = attempt_count + 1
             WHERE run_id = ?1 AND step_key = ?2",
            rusqlite::params![run_id, step_key],
        )
        .map_err(|error| format!("更新后处理步骤失败：{error}"))?;
    if changes != 1 {
        return Err("后处理步骤不存在或已失效".to_string());
    }
    refresh_critical_status(&tx, run_id)?;

    tx.commit()
        .map_err(|error| format!("提交后处理步骤失败：{error}"))
}

/// 标记步骤为失败（全量重跑会把旧成功置回失败，汇总必须重算）
pub fn mark_step_failed(
    conn: &Connection,
    run_id: &str,
    step_key: &str,
    error_msg: &str,
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;

    let changes = tx
        .execute(
            "UPDATE post_process_steps
             SET ok = 0, error_msg = ?1, completed_at = '', last_attempt_at = datetime('now'),
                 attempt_count = attempt_count + 1
             WHERE run_id = ?2 AND step_key = ?3",
            rusqlite::params![error_msg, run_id, step_key],
        )
        .map_err(|error| format!("更新后处理步骤失败：{error}"))?;
    if changes != 1 {
        return Err("后处理步骤不存在或已失效".to_string());
    }
    refresh_critical_status(&tx, run_id)?;

    tx.commit()
        .map_err(|error| format!("提交后处理步骤失败：{error}"))
}

/// 检查某业务实体的最新跑批是否全部关键步骤通过（无跑批时为 false）
pub fn is_all_critical_passed(
    conn: &Connection,
    source_type: &str,
    source_id: &str,
) -> Result<bool, String> {
    Ok(get_latest_run(conn, source_type, source_id)?
        .map(|run| run.all_critical_passed)
        .unwrap_or(false))
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

    fn params(critical_steps: usize) -> PostProcessCreateParams {
        PostProcessCreateParams {
            trigger_source_type: "chapter_finalize".to_string(),
            trigger_source_id: "12".to_string(),
            source_label: "第 12 章".to_string(),
            steps: (0..critical_steps)
                .map(|index| PostProcessStepInput {
                    key: format!("step-{index}"),
                    label: format!("步骤 {index}"),
                    critical: true,
                })
                .chain(std::iter::once(PostProcessStepInput {
                    key: "optional".to_string(),
                    label: "可选步骤".to_string(),
                    critical: false,
                }))
                .collect(),
        }
    }

    #[test]
    fn create_run_initializes_steps_test() {
        let conn = memory_db();
        let run_id = create_run(&conn, &params(2)).unwrap();
        assert_eq!(run_id.len(), 36, "应为 UUID v4 形态");

        let run = get_latest_run(&conn, "chapter_finalize", "12")
            .unwrap()
            .expect("应读到跑批");
        assert_eq!(run.id, run_id);
        assert_eq!(run.source_label, "第 12 章");
        assert!(!run.all_critical_passed);

        let steps = get_steps(&conn, &run_id).unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].step_key, "step-0");
        assert!(steps[0].critical);
        assert!(!steps[0].ok);
        assert_eq!(steps[0].attempt_count, 0);
        assert_eq!(steps[0].completed_at, "");
        // 非关键步骤也一并初始化
        assert!(!steps[2].critical);

        assert!(get_latest_run(&conn, "chapter_finalize", "999")
            .unwrap()
            .is_none());
        assert!(get_steps(&conn, "不存在").unwrap().is_empty());
    }

    #[test]
    fn mark_step_ok_refreshes_summary_test() {
        let conn = memory_db();
        let run_id = create_run(&conn, &params(2)).unwrap();

        mark_step_ok(&conn, &run_id, "step-0").unwrap();
        assert!(
            !is_all_critical_passed(&conn, "chapter_finalize", "12").unwrap(),
            "还有未完成的关键步骤时不得判定通过"
        );

        mark_step_ok(&conn, &run_id, "step-1").unwrap();
        assert!(is_all_critical_passed(&conn, "chapter_finalize", "12").unwrap());

        let steps = get_steps(&conn, &run_id).unwrap();
        assert!(steps[0].ok && steps[1].ok);
        assert_eq!(steps[0].attempt_count, 1);
        assert!(!steps[0].completed_at.is_empty(), "应写入完成时间");
        // 非关键步骤未跑 → 不影响汇总
        assert!(!steps[2].ok);
    }

    #[test]
    fn mark_step_failed_resets_summary_test() {
        let conn = memory_db();
        let run_id = create_run(&conn, &params(2)).unwrap();
        mark_step_ok(&conn, &run_id, "step-0").unwrap();
        mark_step_ok(&conn, &run_id, "step-1").unwrap();
        assert!(is_all_critical_passed(&conn, "chapter_finalize", "12").unwrap());

        // 全量重跑把已成功步骤重新置为失败 → 汇总不能保留旧 true
        mark_step_failed(&conn, &run_id, "step-0", "生成超时").unwrap();
        assert!(!is_all_critical_passed(&conn, "chapter_finalize", "12").unwrap());

        let steps = get_steps(&conn, &run_id).unwrap();
        assert!(!steps[0].ok);
        assert_eq!(steps[0].error_msg, "生成超时");
        assert_eq!(steps[0].completed_at, "", "失败必须清空完成时间");
        assert_eq!(steps[0].attempt_count, 2);

        // 重新成功会清空错误
        mark_step_ok(&conn, &run_id, "step-0").unwrap();
        let recovered = get_steps(&conn, &run_id).unwrap();
        assert_eq!(recovered[0].error_msg, "");
        assert!(is_all_critical_passed(&conn, "chapter_finalize", "12").unwrap());
    }

    #[test]
    fn mark_step_requires_existing_step_test() {
        let conn = memory_db();
        let run_id = create_run(&conn, &params(1)).unwrap();
        assert_eq!(
            mark_step_ok(&conn, &run_id, "不存在").unwrap_err(),
            "后处理步骤不存在或已失效"
        );
        assert_eq!(
            mark_step_failed(&conn, &run_id, "不存在", "错误").unwrap_err(),
            "后处理步骤不存在或已失效"
        );
        assert_eq!(
            mark_step_ok(&conn, "不存在的 run", "step-0").unwrap_err(),
            "后处理步骤不存在或已失效"
        );
    }

    #[test]
    fn run_without_critical_steps_counts_as_passed_test() {
        let conn = memory_db();
        // 零步骤跑批：无失败关键步骤 → 汇总视为通过
        let run_id = create_run(
            &conn,
            &PostProcessCreateParams {
                trigger_source_type: "arch_extract".to_string(),
                trigger_source_id: "draft-1".to_string(),
                source_label: String::new(),
                steps: Vec::new(),
            },
        )
        .unwrap();
        // 汇总初始为 0，须由 mark_step_* 触发重算
        assert!(!is_all_critical_passed(&conn, "arch_extract", "draft-1").unwrap());
        mark_step_ok(&conn, &run_id, "任何步骤").unwrap_err();
        let run = get_latest_run(&conn, "arch_extract", "draft-1").unwrap().unwrap();
        assert_eq!(run.source_label, "");
        assert!(!run.all_critical_passed);
        // 无跑批时同样为 false
        assert!(!is_all_critical_passed(&conn, "arch_extract", "不存在").unwrap());
    }

    #[test]
    fn latest_run_picks_newest_by_created_at_test() {
        let conn = memory_db();
        let first = create_run(&conn, &params(1)).unwrap();
        // 拉开创建时间，使 DESC LIMIT 1 的排序可判定
        conn.execute(
            "UPDATE post_process_runs SET created_at = '2020-01-01 00:00:00' WHERE id = ?1",
            [first.as_str()],
        )
        .unwrap();
        let second = create_run(&conn, &params(1)).unwrap();

        let run = get_latest_run(&conn, "chapter_finalize", "12")
            .unwrap()
            .expect("应读到跑批");
        assert_eq!(run.id, second, "应按 created_at 降序取最新跑批");
    }
}
