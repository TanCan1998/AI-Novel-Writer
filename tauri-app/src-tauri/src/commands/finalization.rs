//! 批次 E（G1）：定稿频道 —— `finalization:commit` / `finalization:retry`。
//!
//! 平移自 `electron/controllers/finalization-controller.ts` +
//! `electron/services/finalization-service.ts` + `electron/repositories/finalization-repository.ts`
//! 的定稿提交段（ADR 0003/0011：**定稿不可逆**）。
//!
//! 语义对齐：
//! - **SQLite transaction 是事实源**：正文 / 字数 / 定稿状态 / 发布 outbox 同事务提交；
//!   实体稿仅为**可恢复投影**，发布失败不回滚已提交事实（`committed: true`）；
//! - 幂等：同草稿同一冻结快照重发返回原提交（复用原 `finalizationId` 与冻结文件名）；
//! - `retry` 只接受 `finalizationId`，正文 / 标题 / 目标文件名一律从 outbox 读取；
//! - 失败文案分层：controller 层 throw → `"Error: "` 前缀；service 内部明文返回分支不前缀。

use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::manuscript_publisher::{publish_manuscript, resolve_manuscript_target};
use crate::project_access::random_uuid_v4;
use crate::repositories::finalization_repository as repo;
use crate::repositories::finalized_continuity_repository::sha256_hex;
use crate::security::{
    assert_current_project_context, guard_message, GuardKind, ProjectSessionContext,
};
use crate::state::AppState;

/// 基线 `FinalizationResult`（camelCase；未设置字段不序列化，对齐 JS 的
/// `undefined` 语义，避免前端 `commit.draftId === undefined` 判定被 `null` 破坏）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizationResult {
    pub success: bool,
    /// false 代表数据库事务从未提交；true 则数据库定稿事实已存在。
    pub committed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finalization_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_revision: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl FinalizationResult {
    /// 只带 `{ success:false, committed:false, error }` 的控制器级失败信封
    fn failure(error: String) -> Self {
        Self {
            success: false,
            committed: false,
            finalization_id: None,
            content_hash: None,
            content_revision: None,
            draft_id: None,
            publication_status: None,
            error: Some(error),
        }
    }
}

/// 复刻 JS `String(new Error(message))` 的 `"Error: "` 前缀
fn js_error(message: String) -> String {
    format!("Error: {message}")
}

/// 基线 `toResult(record, success, error)`
fn to_result(
    record: &repo::FinalizationRecord,
    success: bool,
    error: Option<String>,
) -> FinalizationResult {
    FinalizationResult {
        success,
        committed: true,
        finalization_id: Some(record.finalization_id.clone()),
        content_hash: Some(record.content_hash.clone()),
        content_revision: Some(record.content_revision),
        draft_id: Some(record.draft_id),
        publication_status: Some(record.publication_status.clone()),
        error,
    }
}

/// 基线 `snapshotIntegrityError`
fn snapshot_integrity_error(record: &repo::FinalizationRecord) -> Option<String> {
    if sha256_hex(&record.content_snapshot) == record.content_hash {
        None
    } else {
        Some("已提交定稿的不可变正文快照与内容哈希不一致，已拒绝发布".to_string())
    }
}

/// 基线 `isProjectSessionContext`：三字段均为字符串（不校验非空）
fn is_project_session_context(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    ["projectId", "leaseId", "projectPath"]
        .iter()
        .all(|key| object.get(*key).map(Value::is_string).unwrap_or(false))
}

/// 基线 `isFinalizationSnapshot`
fn is_finalization_snapshot(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let is_string = |key: &str| object.get(key).map(Value::is_string).unwrap_or(false);
    let is_integer = |key: &str| {
        object
            .get(key)
            .map(|item| item.is_i64() || item.is_u64())
            .unwrap_or(false)
    };
    is_string("tabId")
        && is_string("projectPath")
        && object
            .get("projectSession")
            .map(is_project_session_context)
            .unwrap_or(false)
        && is_integer("draftId")
        && is_integer("chapterNumber")
        && is_string("chapterTitle")
        && is_string("content")
        && is_integer("contentRevision")
}

/// 基线 `snapshotMatchesContext`
fn snapshot_matches_context(snapshot: &Value, context: &Value) -> bool {
    let (Some(snapshot), Some(context)) = (snapshot.as_object(), context.as_object()) else {
        return false;
    };
    let Some(session) = snapshot.get("projectSession").and_then(Value::as_object) else {
        return false;
    };
    let text_at = |map: &serde_json::Map<String, Value>, key: &str| {
        map.get(key).and_then(Value::as_str).map(str::to_string)
    };
    text_at(snapshot, "projectPath") == text_at(context, "projectPath")
        && text_at(session, "projectId") == text_at(context, "projectId")
        && text_at(session, "leaseId") == text_at(context, "leaseId")
        && text_at(session, "projectPath") == text_at(context, "projectPath")
}

fn session_from_value(value: &Value) -> ProjectSessionContext {
    ProjectSessionContext {
        project_id: value
            .get("projectId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        lease_id: value
            .get("leaseId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        project_path: value
            .get("projectPath")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

/// 基线 `hasSameFrozenRequest`
fn has_same_frozen_request(
    record: &repo::FinalizationRecord,
    draft_id: i64,
    chapter_number: i64,
    chapter_title: &str,
    content_revision: i64,
    content_hash: &str,
    content: &str,
) -> bool {
    record.draft_id == draft_id
        && record.chapter_number == chapter_number
        && record.chapter_title == chapter_title
        && record.content_revision == content_revision
        && record.content_hash == content_hash
        && record.content_snapshot == content
}

/// 基线 `publishCommitted` 的 catch 分支：标记待发布并把可恢复事实回传
fn mark_pending_and_report(
    state: &AppState,
    record: &repo::FinalizationRecord,
    message: String,
) -> FinalizationResult {
    let prefix = format!("定稿已提交、实体稿待发布：{message}");
    match state.with_project_db(|conn| {
        repo::mark_publication_pending(conn, &record.finalization_id, &message)
    }) {
        Ok(pending) => to_result(&pending, false, Some(prefix)),
        Err(mark_error) => to_result(
            record,
            false,
            Some(format!(
                "{prefix}；待发布状态记录失败：{}",
                js_error(mark_error)
            )),
        ),
    }
}

/// 基线 `FinalizationService.publishCommitted`
fn publish_committed(
    state: &AppState,
    project_root: &str,
    record: &repo::FinalizationRecord,
) -> FinalizationResult {
    if let Some(integrity_error) = snapshot_integrity_error(record) {
        return mark_pending_and_report(state, record, js_error(integrity_error));
    }
    if record.publication_status == "published" {
        return to_result(record, true, None);
    }
    match publish_manuscript(
        project_root,
        &record.target_file_name,
        record.chapter_number,
        &record.chapter_title,
        &record.content_snapshot,
    ) {
        Ok(()) => match state
            .with_project_db(|conn| repo::mark_published(conn, &record.finalization_id))
        {
            Ok(published) => to_result(&published, true, None),
            Err(error) => mark_pending_and_report(state, record, js_error(error)),
        },
        Err(error) => mark_pending_and_report(state, record, js_error(error)),
    }
}

/// controller try 内的全部逻辑；`Err` 等价基线 controller 的 `catch` 分支
async fn commit_inner(
    state: &AppState,
    snapshot: &Value,
    project_session: Option<&Value>,
) -> Result<FinalizationResult, String> {
    if !is_finalization_snapshot(snapshot) {
        return Err("定稿快照无效".to_string());
    }
    let Some(session) = project_session else {
        return Err("定稿快照与项目会话不匹配".to_string());
    };
    if !is_project_session_context(session) || !snapshot_matches_context(snapshot, session) {
        return Err("定稿快照与项目会话不匹配".to_string());
    }
    let context = session_from_value(session);
    let Some(active) = state.active_project_snapshot() else {
        return Err(guard_message(GuardKind::LeaseInvalid));
    };
    assert_current_project_context(&context, Some(&active)).map_err(guard_message)?;
    let project_root = active.root_path;

    let object = snapshot.as_object().expect("snapshot 已验证为对象");
    let draft_id = object
        .get("draftId")
        .and_then(Value::as_i64)
        .ok_or_else(|| "定稿快照无效".to_string())?;
    let chapter_number = object
        .get("chapterNumber")
        .and_then(Value::as_i64)
        .ok_or_else(|| "定稿快照无效".to_string())?;
    let chapter_title = object
        .get("chapterTitle")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let content = object
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let content_revision = object
        .get("contentRevision")
        .and_then(Value::as_i64)
        .unwrap_or(0);

    let finalization_id = random_uuid_v4();
    let content_hash = sha256_hex(&content);

    // ---- 服务体：内部 try/catch（失败仍返回带提交身份的信封）----
    let existing = match state.with_project_db(|conn| repo::get_by_draft_id(conn, draft_id)) {
        Ok(value) => value,
        Err(error) => {
            return Ok(FinalizationResult {
                success: false,
                committed: false,
                finalization_id: Some(finalization_id),
                content_hash: Some(content_hash),
                content_revision: Some(content_revision),
                draft_id: Some(draft_id),
                publication_status: None,
                error: Some(js_error(error)),
            })
        }
    };
    if let Some(existing) = existing {
        if !has_same_frozen_request(
            &existing,
            draft_id,
            chapter_number,
            &chapter_title,
            content_revision,
            &content_hash,
            &content,
        ) {
            return Ok(FinalizationResult::failure(
                "该草稿已有内容不同的不可替换定稿提交".to_string(),
            ));
        }
        // 上次响应可能在客户端收到前丢失。复用既有 finalizationId 与目标文件名，
        // 而不是因新的 UUID 或文件碰撞创建第二次提交。
        return Ok(publish_committed(state, &project_root, &existing));
    }

    let target = match resolve_manuscript_target(
        &project_root,
        chapter_number,
        &chapter_title,
        &finalization_id,
    ) {
        Ok(target) => target,
        Err(error) => {
            return Ok(FinalizationResult {
                success: false,
                committed: false,
                finalization_id: Some(finalization_id),
                content_hash: Some(content_hash),
                content_revision: Some(content_revision),
                draft_id: Some(draft_id),
                publication_status: None,
                error: Some(js_error(error)),
            })
        }
    };
    let input = repo::FinalizationCommitInput {
        finalization_id: finalization_id.clone(),
        draft_id,
        chapter_number,
        chapter_title: chapter_title.clone(),
        content: content.clone(),
        content_hash: content_hash.clone(),
        content_revision,
        target_file_name: target.file_name,
    };
    let record = match state.with_project_db(|conn| repo::commit(conn, &input)) {
        Ok(record) => record,
        Err(error) => {
            return Ok(FinalizationResult {
                success: false,
                committed: false,
                finalization_id: Some(finalization_id),
                content_hash: Some(content_hash),
                content_revision: Some(content_revision),
                draft_id: Some(draft_id),
                publication_status: None,
                error: Some(js_error(error)),
            })
        }
    };
    Ok(publish_committed(state, &project_root, &record))
}

async fn retry_inner(
    state: &AppState,
    finalization_id: String,
    project_session: Option<&Value>,
) -> Result<FinalizationResult, String> {
    if finalization_id.trim().is_empty() {
        return Err("缺少可重试的定稿提交身份".to_string());
    }
    let Some(session) = project_session else {
        return Err("缺少项目会话，已拒绝实体稿重试".to_string());
    };
    if !is_project_session_context(session) {
        return Err("缺少项目会话，已拒绝实体稿重试".to_string());
    }
    let context = session_from_value(session);
    let Some(active) = state.active_project_snapshot() else {
        return Err(guard_message(GuardKind::LeaseInvalid));
    };
    assert_current_project_context(&context, Some(&active)).map_err(guard_message)?;
    let project_root = active.root_path;

    let record = match state.with_project_db(|conn| repo::get(conn, &finalization_id)) {
        Ok(record) => record,
        Err(error) => {
            return Ok(FinalizationResult::failure(js_error(error)));
        }
    };
    let Some(record) = record else {
        return Ok(FinalizationResult {
            success: false,
            committed: false,
            finalization_id: Some(finalization_id),
            content_hash: None,
            content_revision: None,
            draft_id: None,
            publication_status: None,
            error: Some("未找到可重试的定稿提交".to_string()),
        });
    };
    if let Some(integrity_error) = snapshot_integrity_error(&record) {
        return Ok(to_result(&record, false, Some(integrity_error)));
    }
    if record.publication_status == "published" {
        return Ok(to_result(&record, true, None));
    }
    Ok(publish_committed(state, &project_root, &record))
}

/// `finalization:commit` —— 提交已冻结的定稿快照（args: snapshot, projectSession）
///
/// 返回 `Result` 是 Tauri 对「async 命令 + `State` 引用入参」的要求；
/// 内部已把所有失败收敛进信封，故恒为 `Ok`，不会 reject。
#[tauri::command]
pub async fn finalization_commit(
    state: State<'_, AppState>,
    snapshot: Value,
    project_session: Option<Value>,
) -> Result<FinalizationResult, String> {
    Ok(
        match commit_inner(state.inner(), &snapshot, project_session.as_ref()).await {
            Ok(receipt) => receipt,
            Err(error) => FinalizationResult::failure(js_error(error)),
        },
    )
}

/// `finalization:retry` —— 只按 finalizationId 重试实体稿发布
#[tauri::command]
pub async fn finalization_retry(
    state: State<'_, AppState>,
    finalization_id: String,
    project_session: Option<Value>,
) -> Result<FinalizationResult, String> {
    Ok(
        match retry_inner(state.inner(), finalization_id, project_session.as_ref()).await {
            Ok(receipt) => receipt,
            Err(error) => FinalizationResult::failure(js_error(error)),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn context() -> serde_json::Value {
        json!({ "projectId": "p1", "leaseId": "l1", "projectPath": "C:\\Novel" })
    }

    fn snapshot() -> serde_json::Value {
        json!({
            "tabId": "tab-1",
            "projectPath": "C:\\Novel",
            "projectSession": context(),
            "draftId": 7,
            "chapterNumber": 3,
            "chapterTitle": "初遇",
            "content": "正文",
            "contentRevision": 2
        })
    }

    #[test]
    fn snapshot_validation_matches_baseline_test() {
        assert!(is_finalization_snapshot(&snapshot()));
        // 缺字段 / 类型错误
        let mut missing = snapshot();
        missing.as_object_mut().unwrap().remove("contentRevision");
        assert!(!is_finalization_snapshot(&missing));
        let mut wrong = snapshot();
        wrong["draftId"] = json!("7");
        assert!(!is_finalization_snapshot(&wrong));
        let mut bad_session = snapshot();
        bad_session["projectSession"] = json!({ "projectId": "p1" });
        assert!(!is_finalization_snapshot(&bad_session));
        // 非整数（小数）拒绝
        let mut fractional = snapshot();
        fractional["contentRevision"] = json!(1.5);
        assert!(!is_finalization_snapshot(&fractional));
    }

    #[test]
    fn snapshot_must_match_frozen_session_test() {
        assert!(snapshot_matches_context(&snapshot(), &context()));
        // 路径漂移
        let mut other_path = context();
        other_path["projectPath"] = json!("C:\\Other");
        assert!(!snapshot_matches_context(&snapshot(), &other_path));
        // 租约漂移
        let mut other_lease = context();
        other_lease["leaseId"] = json!("l2");
        assert!(!snapshot_matches_context(&snapshot(), &other_lease));
        // 快照内 projectPath 与 session 不一致
        let mut drift = snapshot();
        drift["projectPath"] = json!("C:\\Novel\\..\\Novel");
        assert!(!snapshot_matches_context(&drift, &context()));
    }

    #[test]
    fn frozen_request_equality_guards_idempotency_test() {
        let record = repo::FinalizationRecord {
            finalization_id: "fin-1".to_string(),
            draft_id: 7,
            chapter_number: 3,
            chapter_title: "初遇".to_string(),
            content_snapshot: "正文".to_string(),
            content_hash: sha256_hex("正文"),
            content_revision: 2,
            target_file_name: "第3章 初遇.txt".to_string(),
            knowledge_document_id: String::new(),
            publication_status: "pending".to_string(),
            last_error: String::new(),
            published_at: None,
        };
        assert!(has_same_frozen_request(
            &record,
            7,
            3,
            "初遇",
            2,
            &record.content_hash,
            "正文"
        ));
        assert!(!has_same_frozen_request(
            &record,
            7,
            3,
            "初遇",
            2,
            &record.content_hash,
            "改过的正文"
        ));
        assert!(!has_same_frozen_request(
            &record,
            7,
            3,
            "初遇",
            3,
            &record.content_hash,
            "正文"
        ));

        // 快照哈希漂移 → 拒绝发布
        let mut drifted = record.clone();
        drifted.content_snapshot = "漂移".to_string();
        assert_eq!(
            snapshot_integrity_error(&drifted).unwrap(),
            "已提交定稿的不可变正文快照与内容哈希不一致，已拒绝发布"
        );
        assert!(snapshot_integrity_error(&record).is_none());
    }
}
