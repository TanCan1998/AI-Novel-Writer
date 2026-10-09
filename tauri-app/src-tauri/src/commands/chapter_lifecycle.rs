//! 批次 E 第二部分：章节生命周期（chapter-lifecycle）—— 4 频道。
//!
//! 平移自 `electron/controllers/chapter-lifecycle-controller.ts` +
//! `electron/services/chapter-deletion-service.ts`（操作状态机 + 断点恢复）。
//!
//! 诚实化边界（对齐 `dialog:select-export-directory` 先例）：
//! `resume` 的两个物理清理投影当前**不可真实执行**：
//! - `removePublishedManuscript`（删实体稿文件）→ 依赖批次 H 的 fs 授权域；
//! - `knowledgeBaseLoader.removeDocument`（删知识库文档）→ 依赖批次 F2 的 kb 能力。
//! 因此占位 cleaner 显式返回 `Err`，投影被标为 `failed`（附可读原因）；
//! SQLite 事实删除（`begin` 事务内 `deleteChapterFacts`）已真实提交（`committed: true`），
//! 渲染层可经 `chapter:retry-deletion` 在批次 H / F2 落地后继续断点恢复——
//! 与基线「清理失败 → 收据 failed → 重试」语义一致，仅失败原因固定为能力未迁移。

use rusqlite::Connection;
use serde::Serialize;
use tauri::State;

use crate::commands::db::{guard_read, mutating_error};
use crate::project_access::random_uuid_v4;
use crate::repositories::chapter_deletion_repository as repo;
use crate::security::ProjectSessionContext;
use crate::state::AppState;

/// 基线 `ChapterDeletionResult`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterDeletionResult {
    pub success: bool,
    pub committed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<repo::ChapterDeletionOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 基线 `chapter:list-incomplete-deletions` 返回
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterDeletionListResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operations: Option<Vec<repo::ChapterDeletionOperation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 基线 `operationError`：双通道失败文案合并（逐字对齐）
fn operation_error(operation: &repo::ChapterDeletionOperation) -> Option<String> {
    let mut failures: Vec<String> = Vec::new();
    if operation.manuscript_status == "failed" {
        let detail = if operation.manuscript_error.is_empty() {
            "未知错误"
        } else {
            &operation.manuscript_error
        };
        failures.push(format!("实体稿清理失败：{detail}"));
    }
    if operation.knowledge_status == "failed" {
        let detail = if operation.knowledge_error.is_empty() {
            "未知错误"
        } else {
            &operation.knowledge_error
        };
        failures.push(format!("知识库清理失败：{detail}"));
    }
    if failures.is_empty() {
        None
    } else {
        Some(failures.join("；"))
    }
}

/// 基线 `legacyKnowledgeAuthorizationRequired` 收据（文案逐字对齐）
fn legacy_authorization_receipt(
    operation: repo::ChapterDeletionOperation,
) -> ChapterDeletionResult {
    ChapterDeletionResult {
        success: false,
        committed: false,
        operation: Some(operation),
        error: Some(
            "旧定稿缺少可靠知识文档身份，已保留章节事实和知识库内容；请人工核对后确认没有需要应用自动删除的知识投影"
                .to_string(),
        ),
    }
}

/// 物理清理占位（诚实化）：能力未迁移前一律显式失败
fn manuscript_cleanup_unavailable() -> Result<(), String> {
    Err("实体稿文件清理尚未迁移（依赖批次 H 的外部文件授权），已保留实体稿".to_string())
}

fn knowledge_cleanup_unavailable() -> Result<(), String> {
    Err("知识库文档清理尚未迁移（依赖批次 F2 的知识库能力），已保留知识库内容".to_string())
}

/// 基线 `ChapterDeletionService.resume`：断点恢复。
///
/// 两个物理清理投影当前走诚实化占位（显式 failed），状态机流转与基线逐字对齐。
fn resume(conn: &Connection, operation_id: &str) -> Result<ChapterDeletionResult, String> {
    let existing = repo::get(conn, operation_id)?;
    let Some(mut operation) = existing else {
        return Ok(ChapterDeletionResult {
            success: false,
            committed: false,
            operation: None,
            error: Some("章节删除操作不存在".to_string()),
        });
    };
    if operation.status == "completed" {
        return Ok(ChapterDeletionResult {
            success: true,
            committed: true,
            operation: Some(operation),
            error: None,
        });
    }

    repo::start_attempt(conn, operation_id)?;

    // 投影一：实体稿文件清理（占位：批次 H 前 always failed）
    if operation.manuscript_status == "pending" || operation.manuscript_status == "failed" {
        match manuscript_cleanup_unavailable() {
            Ok(()) => repo::mark_projection(conn, operation_id, "manuscript", "completed", "")?,
            Err(error) => {
                repo::mark_projection(conn, operation_id, "manuscript", "failed", &error)?
            }
        }
    }

    // 投影二
    operation = repo::get(conn, operation_id)?.ok_or_else(|| format!("章节删除操作不存在：{operation_id}"))?;
    if operation.knowledge_status == "pending" || operation.knowledge_status == "failed" {
        match knowledge_cleanup_unavailable() {
            Ok(()) => repo::mark_projection(conn, operation_id, "knowledge", "completed", "")?,
            Err(error) => {
                repo::mark_projection(conn, operation_id, "knowledge", "failed", &error)?
            }
        }
    }

    let operation = repo::get(conn, operation_id)?.unwrap_or(operation);
    Ok(ChapterDeletionResult {
        success: operation.status == "completed",
        committed: true,
        operation: Some(operation.clone()),
        error: operation_error(&operation),
    })
}

/// 基线 `requiresLegacyKnowledgeAuthorization`
fn requires_legacy_knowledge_authorization(
    conn: &Connection,
    request: &repo::DeleteFinalizedChapterRequest,
) -> Result<bool, String> {
    let finalization = outbox_by_draft_id(conn, request.draft_id)?;
    let Some(finalization) = finalization else {
        return Ok(false);
    };
    if !finalization.knowledge_document_id.is_empty() {
        return Ok(false);
    }
    let run = crate::repositories::post_process_repository::get_latest_run(
        conn,
        "chapter_finalize",
        &request.chapter_number.to_string(),
    )?;
    let has_projection = match &run {
        None => true,
        Some(run) => crate::repositories::post_process_repository::get_steps(conn, &run.id)?
            .iter()
            .any(|step| step.step_key == "kb_import"),
    };
    Ok(has_projection)
}

/// 读 outbox 收据（draft_id UNIQUE，单条）；仅用于授权判定
fn outbox_by_draft_id(conn: &Connection, draft_id: i64) -> Result<Option<OutboxIdentity>, String> {
    conn.query_row(
        "SELECT knowledge_document_id FROM finalization_outbox WHERE draft_id = ?1",
        [draft_id],
        |row| {
            Ok(OutboxIdentity {
                knowledge_document_id: row.get(0)?,
            })
        },
    )
    .map(Some)
    .or_else(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other.to_string()),
    })
}

#[derive(Debug)]
struct OutboxIdentity {
    knowledge_document_id: String,
}

/// 基线 `ChapterDeletionService.delete`
fn delete_finalized_inner(
    state: &AppState,
    request: &repo::DeleteFinalizedChapterRequest,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<ChapterDeletionResult, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| {
        let existing = repo::get_by_draft_id(conn, request.draft_id)?;
        if let Some(existing) = existing {
            if existing.chapter_number != request.chapter_number {
                return Ok(ChapterDeletionResult {
                    success: false,
                    committed: false,
                    operation: Some(existing),
                    error: Some("章节删除请求与已冻结操作身份不匹配".to_string()),
                });
            }
            if existing.legacy_knowledge_authorization == "required" {
                return Ok(legacy_authorization_receipt(existing));
            }
            return resume(conn, &existing.operation_id);
        }
        let requires_authorization = requires_legacy_knowledge_authorization(conn, request)?;
        let frozen = repo::begin(conn, &random_uuid_v4(), request, requires_authorization)?;
        if requires_authorization {
            return Ok(legacy_authorization_receipt(frozen));
        }
        resume(conn, &frozen.operation_id)
    })
}

// ===== 命令 =====

/// `chapter:delete-finalized`
#[tauri::command]
pub fn chapter_delete_finalized(
    state: State<'_, AppState>,
    request: repo::DeleteFinalizedChapterRequest,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> ChapterDeletionResult {
    match delete_finalized_inner(state.inner(), &request, &expected_project_path, project_session.as_ref()) {
        Ok(result) => result,
        Err(error) => failing_receipt(mutating_error(error)),
    }
}

/// `chapter:retry-deletion`
#[tauri::command]
pub fn chapter_retry_deletion(
    state: State<'_, AppState>,
    operation_id: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> ChapterDeletionResult {
    let outcome = guard_read(state.inner(), &expected_project_path, project_session.as_ref()).and_then(|()| {
        state.with_project_db(|conn| {
            let operation = repo::get(conn, &operation_id)?;
            match operation {
                None => Ok(receipt_uncommitted("未找到可重试的章节删除操作")),
                Some(operation) if operation.legacy_knowledge_authorization == "required" => {
                    Ok(legacy_authorization_receipt(operation))
                }
                Some(_) => resume(conn, &operation_id),
            }
        })
    });
    match outcome {
        Ok(result) => result,
        Err(error) => failing_receipt(mutating_error(error)),
    }
}

/// `chapter:confirm-legacy-knowledge-absent`
#[tauri::command]
pub fn chapter_confirm_legacy_knowledge_absent(
    state: State<'_, AppState>,
    operation_id: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> ChapterDeletionResult {
    let outcome = guard_read(state.inner(), &expected_project_path, project_session.as_ref()).and_then(|()| {
        state.with_project_db(|conn| {
            repo::confirm_legacy_knowledge_absent(conn, &operation_id)?;
            resume(conn, &operation_id)
        })
    });
    match outcome {
        Ok(result) => result,
        Err(error) => failing_receipt(mutating_error(error)),
    }
}

/// `chapter:list-incomplete-deletions`
#[tauri::command]
pub fn chapter_list_incomplete_deletions(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> ChapterDeletionListResult {
    match guard_read(state.inner(), &expected_project_path, project_session.as_ref())
        .and_then(|()| state.with_project_db(repo::list_incomplete))
    {
        Ok(operations) => ChapterDeletionListResult {
            success: true,
            operations: Some(operations),
            error: None,
        },
        // 读频道失败：外层信封（对齐基线 controller catch）
        Err(error) => ChapterDeletionListResult {
            success: false,
            operations: None,
            error: Some(mutating_error(error)),
        },
    }
}

fn failing_receipt(error: String) -> ChapterDeletionResult {
    ChapterDeletionResult {
        success: false,
        committed: false,
        operation: None,
        error: Some(error),
    }
}

fn receipt_uncommitted(error: &str) -> ChapterDeletionResult {
    ChapterDeletionResult {
        success: false,
        committed: false,
        operation: None,
        error: Some(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_placeholders_fail_explicitly() {
        assert!(manuscript_cleanup_unavailable().is_err());
        assert!(knowledge_cleanup_unavailable().is_err());
    }

    #[test]
    fn operation_error_merges_projection_failures() {
        let mut operation = sample_operation();
        assert!(operation_error(&operation).is_none());
        operation.manuscript_status = "failed".into();
        operation.manuscript_error = "实体稿文件清理尚未迁移".into();
        assert!(operation_error(&operation).unwrap().starts_with("实体稿清理失败："));
        operation.knowledge_status = "failed".into();
        operation.knowledge_error = "知识库文档清理尚未迁移".into();
        let merged = operation_error(&operation).unwrap();
        assert!(merged.contains("；"), "双通道失败应合并为一句");
    }

    fn sample_operation() -> repo::ChapterDeletionOperation {
        repo::ChapterDeletionOperation {
            operation_id: "op-1".into(),
            draft_id: 1,
            chapter_number: 1,
            chapter_title: "第一章".into(),
            finalization_id: "f-1".into(),
            target_file_name: "chapter-01.md".into(),
            knowledge_document_id: String::new(),
            post_process_run_ids: vec![],
            manuscript_status: "pending".into(),
            manuscript_error: String::new(),
            knowledge_status: "not_required".into(),
            knowledge_error: String::new(),
            legacy_knowledge_authorization: "not_required".into(),
            legacy_knowledge_authorized_at: String::new(),
            status: "pending".into(),
            attempt_count: 0,
            created_at: String::new(),
            updated_at: String::new(),
            completed_at: String::new(),
        }
    }
}
