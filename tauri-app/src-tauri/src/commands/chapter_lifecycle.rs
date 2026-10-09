//! 批次 E 第二部分：章节生命周期（chapter-lifecycle）—— 4 频道。
//!
//! 平移自 `electron/controllers/chapter-lifecycle-controller.ts` +
//! `electron/services/chapter-deletion-service.ts`（操作状态机 + 断点恢复）。
//!
//! 物理清理投影状态（2026-10-09 更新，G1 收口）：
//! - 删实体稿文件（`removePublishedManuscript`）→ **已真实化**：删除项目根内 outbox
//!   冻结的实体稿文件（缺失视为幂等成功），与基线 cleaner 同源；
//! - 知识库文档清理 → **已真实化**：调用 `db/kb/store::remove_document` 删除 SQLite 事实
//!   并同步清除 HNSW 向量（与 `kb:remove-document` 同一路径）。
//!
//! SQLite 事实删除（`begin` 事务内 `deleteChapterFacts`）已真实提交（`committed: true`），
//! 渲染层可经 `chapter:retry-deletion` 断点恢复；两个物理投影现均真实执行。

use rusqlite::Connection;
use serde::Serialize;
use tauri::State;

use crate::commands::db::{guard_read, mutating_error};
use crate::commands::kb::purge_vectors;
use crate::db::kb::{hybrid, store};
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

/// 实体稿物理清理：删除 outbox 冻结的实体稿文件（项目根内，缺失视为幂等成功）
fn manuscript_cleanup(project_root: &str, target_file_name: &str) -> Result<(), String> {
    crate::manuscript_publisher::remove_published_manuscript(project_root, target_file_name)
}

/// 知识库文档真实清理：删除 `kb_documents` / `kb_chunks` / `kb_fts` 事实并清除向量。
///
/// 与 `kb:remove-document` 同一路径；文档不存在视为幂等成功（对齐基线 cleaner 语义）。
async fn knowledge_cleanup(
    state: &AppState,
    project_root: &str,
    operation: &repo::ChapterDeletionOperation,
) -> Result<(), String> {
    let doc_id = operation.knowledge_document_id.clone();
    if doc_id.is_empty() {
        return Ok(());
    }
    let (ids, spaces) = state.with_project_db(|conn| {
        let ids = store::document_chunk_ids(conn, &doc_id)?;
        let spaces = hybrid::list_spaces(conn).map_err(|error| error.to_string())?;
        store::remove_document(conn, &doc_id)?;
        Ok((ids, spaces))
    })?;
    for space in spaces {
        purge_vectors(state, project_root, space.generation, space.dimension, &ids).await;
    }
    Ok(())
}

/// 基线 `ChapterDeletionService.resume`：断点恢复。
///
/// 实体稿与知识库投影均为真实清理。
async fn resume(
    state: &AppState,
    project_root: &str,
    operation_id: &str,
) -> Result<ChapterDeletionResult, String> {
    let existing = state.with_project_db(|conn| repo::get(conn, operation_id))?;
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

    state.with_project_db(|conn| repo::start_attempt(conn, operation_id))?;

    // 投影一：实体稿文件清理（真实：项目根内 outbox 冻结目标）
    if operation.manuscript_status == "pending" || operation.manuscript_status == "failed" {
        match manuscript_cleanup(project_root, &operation.target_file_name) {
            Ok(()) => state.with_project_db(|conn| {
                repo::mark_projection(conn, operation_id, "manuscript", "completed", "")
            })?,
            Err(error) => state.with_project_db(|conn| {
                repo::mark_projection(conn, operation_id, "manuscript", "failed", &error)
            })?,
        }
    }

    // 投影二：知识库文档清理（真实）
    operation = state
        .with_project_db(|conn| repo::get(conn, operation_id))?
        .ok_or_else(|| format!("章节删除操作不存在：{operation_id}"))?;
    if operation.knowledge_status == "pending" || operation.knowledge_status == "failed" {
        match knowledge_cleanup(state, project_root, &operation).await {
            Ok(()) => state.with_project_db(|conn| {
                repo::mark_projection(conn, operation_id, "knowledge", "completed", "")
            })?,
            Err(error) => state.with_project_db(|conn| {
                repo::mark_projection(conn, operation_id, "knowledge", "failed", &error)
            })?,
        }
    }

    let operation = state
        .with_project_db(|conn| repo::get(conn, operation_id))?
        .unwrap_or(operation);
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

/// 基线 `ChapterDeletionService.delete`（异步：知识库清理需要 await 向量层）
async fn delete_finalized_inner(
    state: &AppState,
    request: &repo::DeleteFinalizedChapterRequest,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<ChapterDeletionResult, String> {
    guard_read(state, expected_project_path, session)?;

    enum Flow {
        Receipt(ChapterDeletionResult),
        Resume(String),
    }

    let flow = state.with_project_db(|conn| {
        let existing = repo::get_by_draft_id(conn, request.draft_id)?;
        if let Some(existing) = existing {
            if existing.chapter_number != request.chapter_number {
                return Ok(Flow::Receipt(ChapterDeletionResult {
                    success: false,
                    committed: false,
                    operation: Some(existing),
                    error: Some("章节删除请求与已冻结操作身份不匹配".to_string()),
                }));
            }
            if existing.legacy_knowledge_authorization == "required" {
                return Ok(Flow::Receipt(legacy_authorization_receipt(existing)));
            }
            return Ok(Flow::Resume(existing.operation_id));
        }
        let requires_authorization = requires_legacy_knowledge_authorization(conn, request)?;
        let frozen = repo::begin(conn, &random_uuid_v4(), request, requires_authorization)?;
        if requires_authorization {
            return Ok(Flow::Receipt(legacy_authorization_receipt(frozen)));
        }
        Ok(Flow::Resume(frozen.operation_id))
    })?;

    match flow {
        Flow::Receipt(result) => Ok(result),
        Flow::Resume(operation_id) => {
            resume(state, expected_project_path, &operation_id).await
        }
    }
}

// ===== 命令 =====

/// `chapter:delete-finalized`
#[tauri::command]
pub async fn chapter_delete_finalized(
    state: State<'_, AppState>,
    request: repo::DeleteFinalizedChapterRequest,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<ChapterDeletionResult, String> {
    match delete_finalized_inner(
        state.inner(),
        &request,
        &expected_project_path,
        project_session.as_ref(),
    )
    .await
    {
        Ok(result) => Ok(result),
        Err(error) => Ok(failing_receipt(mutating_error(error))),
    }
}

/// `chapter:retry-deletion`
#[tauri::command]
pub async fn chapter_retry_deletion(
    state: State<'_, AppState>,
    operation_id: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<ChapterDeletionResult, String> {
    if let Err(error) = guard_read(state.inner(), &expected_project_path, project_session.as_ref()) {
        return Ok(failing_receipt(mutating_error(error)));
    }
    let operation = match state.with_project_db(|conn| repo::get(conn, &operation_id)) {
        Ok(operation) => operation,
        Err(error) => return Ok(failing_receipt(mutating_error(error))),
    };
    match operation {
        None => Ok(receipt_uncommitted("未找到可重试的章节删除操作")),
        Some(operation) if operation.legacy_knowledge_authorization == "required" => {
            Ok(legacy_authorization_receipt(operation))
        }
        Some(_) => match resume(state.inner(), &expected_project_path, &operation_id).await {
            Ok(result) => Ok(result),
            Err(error) => Ok(failing_receipt(mutating_error(error))),
        },
    }
}

/// `chapter:confirm-legacy-knowledge-absent`
#[tauri::command]
pub async fn chapter_confirm_legacy_knowledge_absent(
    state: State<'_, AppState>,
    operation_id: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<ChapterDeletionResult, String> {
    if let Err(error) = guard_read(state.inner(), &expected_project_path, project_session.as_ref()) {
        return Ok(failing_receipt(mutating_error(error)));
    }
    if let Err(error) = state.with_project_db(|conn| {
        repo::confirm_legacy_knowledge_absent(conn, &operation_id)
    }) {
        return Ok(failing_receipt(mutating_error(error)));
    }
    match resume(state.inner(), &expected_project_path, &operation_id).await {
        Ok(result) => Ok(result),
        Err(error) => Ok(failing_receipt(mutating_error(error))),
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
    fn manuscript_cleanup_is_idempotent_inside_project_root() {
        let root = std::env::temp_dir().join(format!(
            "lorekeeper-chapter-cleanup-{}",
            crate::project_access::random_uuid_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let root_text = root.to_string_lossy().to_string();
        std::fs::write(root.join("第1章 起点.txt"), "正文").unwrap();

        manuscript_cleanup(&root_text, "第1章 起点.txt").unwrap();
        assert!(!root.join("第1章 起点.txt").exists(), "应删除实体稿文件");
        // 缺失文件视为幂等成功
        manuscript_cleanup(&root_text, "第1章 起点.txt").unwrap();
        // 冻结目标必须为裸文件名（越界拒绝）
        assert!(manuscript_cleanup(&root_text, "../escape.txt").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn operation_error_merges_projection_failures() {
        let mut operation = sample_operation();
        assert!(operation_error(&operation).is_none());
        operation.manuscript_status = "failed".into();
        operation.manuscript_error = "实体稿目标无效".into();
        assert!(operation_error(&operation).unwrap().starts_with("实体稿清理失败："));
        operation.knowledge_status = "failed".into();
        operation.knowledge_error = "知识库文档清理失败".into();
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
