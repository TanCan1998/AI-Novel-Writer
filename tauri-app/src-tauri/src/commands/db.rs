//! 批次 C：项目数据库命令 —— 平移自 `electron/controllers/db-controller.ts`。
//!
//! 门禁语义与基线一致（`registerProjectDatabaseHandler`）：
//! 1. 尾参 `projectSession`（渲染层自动注入）必须通过真实租约校验；
//! 2. `expectedProjectPath` 必须与当前已打开项目一致（路径从不单独构成授权）；
//! 3. 写频道失败返回 `{ success: false, error }`，读频道失败直接拒绝（invoke reject）。
//!
//! 本批次落地的子域：`db:close` + `project_core`（get / update / synopsis-commit）
//! + `characters`（get-all / roster-read / roster-commit）。
//! 其余 `db:*` 子域（blueprint / draft / revision / import-run / llm 等）按批次 C 后续
//! 子域逐个迁移。

use tauri::State;
use std::path::Path;

use crate::commands::SimpleResult;
use crate::repositories::blueprint_repository as blueprints;
use crate::repositories::character_repository as characters;
use crate::repositories::character_roster_repository as roster;
use crate::repositories::consistency_exemption_repository as consistency;
use crate::repositories::draft_repository as drafts;
use crate::repositories::finalized_continuity_repository as continuity;
use crate::repositories::finalized_draft_import_repository as draft_import;
use crate::repositories::finalization_repository as finalization;
use crate::repositories::llm_repository as llm;
use crate::repositories::narrative_thread_repository as threads;
use crate::repositories::plot_tree_repository as plot_tree;
use crate::repositories::post_process_repository as post_process;
use crate::repositories::recovery_candidate_repository as recovery;
use crate::repositories::project_clear_repository as project_clear;
use crate::repositories::project_core_repository as project_core;
use crate::repositories::review_repository as reviews;
use crate::repositories::revision_repository as revisions;
use crate::security::{
    assert_current_project_context, assert_required_expected_project_path, guard_message,
    ProjectSessionContext,
};
use crate::state::AppState;

/// 项目会话门禁：会话必须与活跃项目租约完全一致
fn assert_session(
    state: &AppState,
    session: Option<&ProjectSessionContext>,
) -> Result<(), String> {
    let Some(context) = session else {
        return Err("缺少项目会话上下文，已拒绝操作".to_string());
    };
    let active = state.active_project_snapshot();
    assert_current_project_context(context, active.as_ref()).map_err(guard_message)
}

/// 冻结路径门禁：`expectedProjectPath` 必填且与当前项目一致
fn assert_project_path(state: &AppState, expected_project_path: &str) -> Result<(), String> {
    assert_required_expected_project_path(
        state.current_project_path().as_deref(),
        Some(expected_project_path),
    )
    .map_err(guard_message)
}

/// 读频道门禁（失败即拒绝）
fn guard_read(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<(), String> {
    assert_session(state, session)?;
    assert_project_path(state, expected_project_path)
}

/// 基线对 MUTATING 频道的失败统一走 `String(error)`，JS 的 `Error` 会带上 `"Error: "`
/// 前缀（如「Error: 角色名单 revision 已过期，已拒绝覆盖」）。复刻该格式，避免前端
/// 展示文案与 Electron 版出现差异。
pub(crate) fn mutating_error(error: String) -> String {
    format!("Error: {error}")
}

/// 把 `Result<(), String>` 收敛为 MUTATING 信封（失败加 `"Error: "` 前缀）
pub(crate) fn simple_mutating_result(outcome: Result<(), String>) -> SimpleResult {
    match outcome {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => SimpleResult {
            success: false,
            error: Some(mutating_error(error)),
        },
    }
}

pub(crate) fn close_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = assert_session(state, session)
        .and_then(|()| assert_project_path(state, expected_project_path));
    if let Err(error) = outcome {
        return SimpleResult {
            success: false,
            error: Some(mutating_error(error)),
        };
    }
    state.close_project_database();
    state.invalidate_current_session();
    SimpleResult {
        success: true,
        error: None,
    }
}

pub(crate) fn project_core_get_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Option<project_core::ProjectCoreData>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(project_core::get)
}

pub(crate) fn project_core_update_inner(
    state: &AppState,
    data: &serde_json::Map<String, serde_json::Value>,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| project_core::update(conn, data)));
    match outcome {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => {
            eprintln!("[db:project-core-update] 失败: {error}");
            SimpleResult {
                success: false,
                error: Some(mutating_error(error)),
            }
        }
    }
}

pub(crate) fn project_core_synopsis_commit_inner(
    state: &AppState,
    request: &project_core::ProjectCoreSynopsisCommitRequest,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| project_core::commit_synopsis(conn, request)));
    match outcome {
        // 快照冲突是基线**显式返回**的裸文案（不经过 `String(err)` 包装）
        Ok(false) => SimpleResult {
            success: false,
            error: Some(project_core::SYNOPSIS_CONFLICT_MESSAGE.to_string()),
        },
        Ok(true) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => SimpleResult {
            success: false,
            error: Some(mutating_error(error)),
        },
    }
}

/// `db:close` —— 关闭项目数据库并使会话失效
#[tauri::command]
pub fn db_close(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    close_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:project-core-get` —— 读取项目主台账（未初始化时返回 null）
#[tauri::command]
pub fn db_project_core_get(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Option<project_core::ProjectCoreData>, String> {
    project_core_get_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:project-core-update` —— 局部更新项目配置
#[tauri::command]
pub fn db_project_core_update(
    state: State<'_, AppState>,
    data: serde_json::Map<String, serde_json::Value>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    project_core_update_inner(
        state.inner(),
        &data,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:project-core-synopsis-commit` —— 乐观并发提交情节大纲
#[tauri::command]
pub fn db_project_core_synopsis_commit(
    state: State<'_, AppState>,
    request: project_core::ProjectCoreSynopsisCommitRequest,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    project_core_synopsis_commit_inner(
        state.inner(),
        &request,
        &expected_project_path,
        project_session.as_ref(),
    )
}

// ===== 角色与角色名单（characters 子域） =====

/// `db:character-roster-commit` 的 IPC 信封（对齐基线 `{ success, receipt?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterRosterCommitResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<roster::CharacterRosterCommitReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub(crate) fn character_get_all_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Vec<characters::CharacterData>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(characters::get_all)
}

pub(crate) fn character_roster_read_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<roster::CharacterRosterSnapshot, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(roster::read)
}

/// 请求保持 `serde_json::Value`：校验与错误文案由 [`roster::normalize_request`] 收口，
/// 与基线 `normalizeRequest` 逐条对齐。
pub(crate) fn character_roster_commit_inner(
    state: &AppState,
    request: &serde_json::Value,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> CharacterRosterCommitResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| roster::commit(conn, request)));
    match outcome {
        Ok(receipt) => CharacterRosterCommitResult {
            success: true,
            receipt: Some(receipt),
            error: None,
        },
        Err(error) => CharacterRosterCommitResult {
            success: false,
            receipt: None,
            error: Some(mutating_error(error)),
        },
    }
}

/// `db:character-get-all` —— 读取全部角色卡（按定位排序）
#[tauri::command]
pub fn db_character_get_all(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<characters::CharacterData>, String> {
    character_get_all_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:character-roster-read` —— 读取结构化角色名单快照
#[tauri::command]
pub fn db_character_roster_read(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<roster::CharacterRosterSnapshot, String> {
    character_roster_read_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:character-roster-commit` —— 角色名单唯一提交 seam（MUTATING）
#[tauri::command]
pub fn db_character_roster_commit(
    state: State<'_, AppState>,
    request: serde_json::Value,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> CharacterRosterCommitResult {
    character_roster_commit_inner(
        state.inner(),
        &request,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:blueprint-update-notes` 的 IPC 信封（对齐基线 `{ success, updated?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintUpdateNotesResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ===== 章节蓝图（blueprints 子域 S2-a） =====

pub(crate) fn blueprint_get_all_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Vec<blueprints::BlueprintData>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(blueprints::get_all)
}

pub(crate) fn blueprint_get_inner(
    state: &AppState,
    chapter_number: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Option<blueprints::BlueprintData>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| blueprints::get_by_chapter(conn, chapter_number))
}

pub(crate) fn blueprint_upsert_inner(
    state: &AppState,
    data: &blueprints::BlueprintData,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| blueprints::upsert(conn, data)));
    match outcome {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => SimpleResult {
            success: false,
            error: Some(mutating_error(error)),
        },
    }
}

pub(crate) fn blueprint_upsert_many_inner(
    state: &AppState,
    items: &[blueprints::BlueprintData],
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| blueprints::upsert_many(conn, items)));
    match outcome {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => SimpleResult {
            success: false,
            error: Some(mutating_error(error)),
        },
    }
}

pub(crate) fn blueprint_update_notes_inner(
    state: &AppState,
    chapter_number: i64,
    notes: &str,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> BlueprintUpdateNotesResult {
    let outcome = guard_read(state, expected_project_path, session).and_then(|()| {
        state.with_project_db(|conn| blueprints::update_notes(conn, chapter_number, notes))
    });
    match outcome {
        Ok(updated) => BlueprintUpdateNotesResult {
            success: true,
            updated: Some(updated),
            error: None,
        },
        Err(error) => BlueprintUpdateNotesResult {
            success: false,
            updated: None,
            error: Some(mutating_error(error)),
        },
    }
}

pub(crate) fn blueprint_delete_inner(
    state: &AppState,
    chapter_number: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| blueprints::delete(conn, chapter_number)));
    match outcome {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => SimpleResult {
            success: false,
            error: Some(mutating_error(error)),
        },
    }
}

pub(crate) fn blueprint_clear_all_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(blueprints::clear_all));
    match outcome {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => SimpleResult {
            success: false,
            error: Some(mutating_error(error)),
        },
    }
}

/// `db:blueprint-get-all` —— 读取全部章节蓝图（按章节号升序）
#[tauri::command]
pub fn db_blueprint_get_all(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<blueprints::BlueprintData>, String> {
    blueprint_get_all_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:blueprint-get` —— 读取单章蓝图（未找到返回 null）
#[tauri::command]
pub fn db_blueprint_get(
    state: State<'_, AppState>,
    chapter_number: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Option<blueprints::BlueprintData>, String> {
    blueprint_get_inner(
        state.inner(),
        chapter_number,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:blueprint-upsert` —— 插入或更新单章蓝图（MUTATING）
#[tauri::command]
pub fn db_blueprint_upsert(
    state: State<'_, AppState>,
    data: blueprints::BlueprintData,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    blueprint_upsert_inner(
        state.inner(),
        &data,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:blueprint-upsert-many` —— 批量插入/更新章节蓝图（MUTATING）
#[tauri::command]
pub fn db_blueprint_upsert_many(
    state: State<'_, AppState>,
    items: Vec<blueprints::BlueprintData>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    blueprint_upsert_many_inner(
        state.inner(),
        &items,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:blueprint-update-notes` —— 仅更新 notes 字段（MUTATING）
#[tauri::command]
pub fn db_blueprint_update_notes(
    state: State<'_, AppState>,
    chapter_number: i64,
    notes: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> BlueprintUpdateNotesResult {
    blueprint_update_notes_inner(
        state.inner(),
        chapter_number,
        &notes,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:blueprint-delete` —— 删除单章蓝图（MUTATING）
#[tauri::command]
pub fn db_blueprint_delete(
    state: State<'_, AppState>,
    chapter_number: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    blueprint_delete_inner(
        state.inner(),
        chapter_number,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:blueprint-clear-all` —— 清空所有章节蓝图（MUTATING）
#[tauri::command]
pub fn db_blueprint_clear_all(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    blueprint_clear_all_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:blueprint-commit-range` 的 IPC 信封（对齐 `{ success, receipt?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintCommitRangeResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<blueprints::BlueprintCommitRangeReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub(crate) fn blueprint_commit_range_inner(
    state: &AppState,
    request: &blueprints::BlueprintCommitRangeRequest,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> BlueprintCommitRangeResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| blueprints::commit_range(conn, request)));
    match outcome {
        Ok(receipt) => BlueprintCommitRangeResult {
            success: true,
            receipt: Some(receipt),
            error: None,
        },
        Err(error) => BlueprintCommitRangeResult {
            success: false,
            receipt: None,
            error: Some(mutating_error(error)),
        },
    }
}

/// `db:blueprint-commit-range` —— 逻辑范围只提交一次（MUTATING）
#[tauri::command]
pub fn db_blueprint_commit_range(
    state: State<'_, AppState>,
    request: blueprints::BlueprintCommitRangeRequest,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> BlueprintCommitRangeResult {
    blueprint_commit_range_inner(
        state.inner(),
        &request,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:blueprint-character-sync-complete` 的 IPC 信封（对齐 `{ success, operation?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintCharacterSyncCompleteResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<blueprints::BlueprintCharacterSyncOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub(crate) fn blueprint_character_sync_list_pending_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Vec<blueprints::BlueprintCharacterSyncOperation>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(blueprints::list_pending_character_sync_operations)
}

pub(crate) fn blueprint_character_sync_get_inner(
    state: &AppState,
    operation_id: &str,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Option<blueprints::BlueprintCharacterSyncOperation>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| blueprints::get_character_sync_operation(conn, operation_id))
}

pub(crate) fn blueprint_character_sync_complete_inner(
    state: &AppState,
    operation_id: &str,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> BlueprintCharacterSyncCompleteResult {
    let outcome = guard_read(state, expected_project_path, session).and_then(|()| {
        state.with_project_db(|conn| blueprints::complete_character_sync_operation(conn, operation_id))
    });
    match outcome {
        Ok(operation) => BlueprintCharacterSyncCompleteResult {
            success: true,
            operation: Some(operation),
            error: None,
        },
        Err(error) => BlueprintCharacterSyncCompleteResult {
            success: false,
            operation: None,
            error: Some(mutating_error(error)),
        },
    }
}

/// `db:blueprint-character-sync-list-pending` —— 列出待恢复的角色同步操作
#[tauri::command]
pub fn db_blueprint_character_sync_list_pending(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<blueprints::BlueprintCharacterSyncOperation>, String> {
    blueprint_character_sync_list_pending_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:blueprint-character-sync-get` —— 读取单个角色同步操作
#[tauri::command]
pub fn db_blueprint_character_sync_get(
    state: State<'_, AppState>,
    operation_id: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Option<blueprints::BlueprintCharacterSyncOperation>, String> {
    blueprint_character_sync_get_inner(
        state.inner(),
        &operation_id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:blueprint-character-sync-complete` —— 闭合待处理操作（MUTATING）
#[tauri::command]
pub fn db_blueprint_character_sync_complete(
    state: State<'_, AppState>,
    operation_id: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> BlueprintCharacterSyncCompleteResult {
    blueprint_character_sync_complete_inner(
        state.inner(),
        &operation_id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

// ===== 草稿（drafts 子域 S3-a，12 频道） =====

/// `db:draft-create` 的 IPC 信封（对齐 `{ success, id?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftCreateResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `db:draft-delete` 的 IPC 信封（对齐 `{ success, errorCode?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftDeleteResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub(crate) fn draft_create_inner(
    state: &AppState,
    params: &drafts::DraftCreateParams,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> DraftCreateResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| drafts::create(conn, params)));
    match outcome {
        Ok(id) => DraftCreateResult {
            success: true,
            id: Some(id),
            error: None,
        },
        Err(error) => DraftCreateResult {
            success: false,
            id: None,
            error: Some(mutating_error(error)),
        },
    }
}

pub(crate) fn draft_list_inner(
    state: &AppState,
    chapter_number: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Vec<drafts::DraftMeta>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| drafts::list_by_chapter(conn, chapter_number))
}

pub(crate) fn draft_list_all_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Vec<drafts::DraftMeta>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(drafts::list_all)
}

pub(crate) fn draft_get_meta_inner(
    state: &AppState,
    id: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Option<drafts::DraftMeta>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| drafts::get_meta(conn, id))
}

pub(crate) fn draft_get_full_inner(
    state: &AppState,
    id: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Option<drafts::DraftFull>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| drafts::get_full(conn, id))
}

pub(crate) fn draft_get_latest_inner(
    state: &AppState,
    chapter_number: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Option<drafts::DraftMeta>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| drafts::get_latest_by_chapter(conn, chapter_number))
}

pub(crate) fn draft_get_finalized_inner(
    state: &AppState,
    chapter_number: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Option<drafts::DraftMeta>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| drafts::get_finalized_by_chapter(conn, chapter_number))
}

pub(crate) fn draft_get_max_finalized_chapter_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<i64, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(drafts::get_max_finalized_chapter)
}

pub(crate) fn draft_next_version_inner(
    state: &AppState,
    chapter_number: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<i64, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| drafts::get_next_version(conn, chapter_number))
}

pub(crate) fn draft_update_status_inner(
    state: &AppState,
    id: i64,
    status: &str,
    word_count: Option<i64>,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| drafts::update_status(conn, id, status, word_count)));
    match outcome {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => SimpleResult {
            success: false,
            error: Some(mutating_error(error)),
        },
    }
}

/// `db:draft-update-content`：先复刻 controller 的前置检查（缺失/定稿只读文案），再写正文。
pub(crate) fn draft_update_content_inner(
    state: &AppState,
    id: i64,
    content: &str,
    word_count: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session).and_then(|()| {
        state.with_project_db(|conn| {
            let Some(meta) = drafts::get_meta(conn, id)? else {
                return Err(format!("草稿不存在：{id}"));
            };
            if meta.status == "finalized" {
                return Err("已定稿正文为只读内容，不能再修改".to_string());
            }
            drafts::update_content(conn, id, content, word_count)
        })
    });
    match outcome {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => SimpleResult {
            success: false,
            error: Some(mutating_error(error)),
        },
    }
}

pub(crate) fn draft_delete_inner(
    state: &AppState,
    id: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> DraftDeleteResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| drafts::delete(conn, id)));
    match outcome {
        Ok(()) => DraftDeleteResult {
            success: true,
            error_code: None,
            error: None,
        },
        // 定稿删除入口：基线只回 `errorCode`，不带 `error` 文案
        Err(message) if message == drafts::FINALIZED_DELETE_REQUIRED_MESSAGE => DraftDeleteResult {
            success: false,
            error_code: Some(drafts::FINALIZED_DELETE_REQUIRED_CODE.to_string()),
            error: None,
        },
        Err(message) => DraftDeleteResult {
            success: false,
            error_code: None,
            error: Some(mutating_error(message)),
        },
    }
}

/// `db:draft-create` —— 创建草稿（MUTATING）
#[tauri::command]
pub fn db_draft_create(
    state: State<'_, AppState>,
    params: drafts::DraftCreateParams,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> DraftCreateResult {
    draft_create_inner(
        state.inner(),
        &params,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:draft-list` —— 列出章节草稿
#[tauri::command]
pub fn db_draft_list(
    state: State<'_, AppState>,
    chapter_number: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<drafts::DraftMeta>, String> {
    draft_list_inner(
        state.inner(),
        chapter_number,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:draft-list-all` —— 列出全部草稿元数据
#[tauri::command]
pub fn db_draft_list_all(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<drafts::DraftMeta>, String> {
    draft_list_all_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:draft-get-meta` —— 读取草稿元数据
#[tauri::command]
pub fn db_draft_get_meta(
    state: State<'_, AppState>,
    id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Option<drafts::DraftMeta>, String> {
    draft_get_meta_inner(
        state.inner(),
        id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:draft-get-full` —— 读取草稿（含正文）
#[tauri::command]
pub fn db_draft_get_full(
    state: State<'_, AppState>,
    id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Option<drafts::DraftFull>, String> {
    draft_get_full_inner(
        state.inner(),
        id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:draft-get-latest` —— 读取章节最新草稿
#[tauri::command]
pub fn db_draft_get_latest(
    state: State<'_, AppState>,
    chapter_number: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Option<drafts::DraftMeta>, String> {
    draft_get_latest_inner(
        state.inner(),
        chapter_number,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:draft-get-finalized` —— 读取章节定稿草稿
#[tauri::command]
pub fn db_draft_get_finalized(
    state: State<'_, AppState>,
    chapter_number: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Option<drafts::DraftMeta>, String> {
    draft_get_finalized_inner(
        state.inner(),
        chapter_number,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:draft-get-max-finalized-chapter` —— 最大已定稿章节号（无则 0）
#[tauri::command]
pub fn db_draft_get_max_finalized_chapter(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<i64, String> {
    draft_get_max_finalized_chapter_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:draft-next-version` —— 下一个可用版本号
#[tauri::command]
pub fn db_draft_next_version(
    state: State<'_, AppState>,
    chapter_number: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<i64, String> {
    draft_next_version_inner(
        state.inner(),
        chapter_number,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:draft-update-status` —— 更新草稿状态（MUTATING）
#[tauri::command]
pub fn db_draft_update_status(
    state: State<'_, AppState>,
    id: i64,
    status: String,
    word_count: Option<i64>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    draft_update_status_inner(
        state.inner(),
        id,
        &status,
        word_count,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:draft-update-content` —— 更新草稿正文（MUTATING）
#[tauri::command]
pub fn db_draft_update_content(
    state: State<'_, AppState>,
    id: i64,
    content: String,
    word_count: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    draft_update_content_inner(
        state.inner(),
        id,
        &content,
        word_count,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:draft-delete` —— 删除草稿（MUTATING，定稿需走定稿删除入口）
#[tauri::command]
pub fn db_draft_delete(
    state: State<'_, AppState>,
    id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> DraftDeleteResult {
    draft_delete_inner(
        state.inner(),
        id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

// ===== 修稿（revisions 子域 S3-b，9 频道） =====

/// `db:revision-create` / `-replace-pending` 的 IPC 信封（对齐 `{ success, id?, revisionIndex?, errorCode?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionCreateResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_index: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `db:revision-merge` 的 IPC 信封（对齐 `{ success, receipt?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionMergeResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<revisions::MergeRevisionReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 创建/替换修稿的公共实现：源稿守卫失败额外回填 `errorCode`
fn revision_create_result(outcome: Result<revisions::RevisionCreated, String>) -> RevisionCreateResult {
    match outcome {
        Ok(created) => RevisionCreateResult {
            success: true,
            id: Some(created.id),
            revision_index: Some(created.revision_index),
            error_code: None,
            error: None,
        },
        Err(message) => {
            // 基线的源守卫失败同时带 `errorCode` 与 `error`（`String(err)` 形态）
            let error_code = (message == crate::draft_source_guard::SOURCE_DRAFT_CHANGED_MESSAGE)
                .then(|| crate::draft_source_guard::SOURCE_DRAFT_CHANGED.to_string());
            RevisionCreateResult {
                success: false,
                id: None,
                revision_index: None,
                error_code,
                error: Some(mutating_error(message)),
            }
        }
    }
}

pub(crate) fn revision_create_inner(
    state: &AppState,
    params: &revisions::RevisionCreateParams,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> RevisionCreateResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| revisions::create(conn, params)));
    revision_create_result(outcome)
}

pub(crate) fn revision_replace_pending_inner(
    state: &AppState,
    params: &revisions::RevisionCreateParams,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> RevisionCreateResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| revisions::replace_pending(conn, params)));
    revision_create_result(outcome)
}

pub(crate) fn revision_list_inner(
    state: &AppState,
    base_draft_id: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Vec<revisions::RevisionMeta>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| revisions::list_by_draft(conn, base_draft_id))
}

pub(crate) fn revision_get_pending_inner(
    state: &AppState,
    base_draft_id: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Vec<revisions::RevisionMeta>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| revisions::get_pending(conn, base_draft_id))
}

pub(crate) fn revision_get_full_inner(
    state: &AppState,
    id: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Option<revisions::RevisionFull>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| revisions::get_full(conn, id))
}

pub(crate) fn revision_next_index_inner(
    state: &AppState,
    base_draft_id: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<i64, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| revisions::get_next_index(conn, base_draft_id))
}

pub(crate) fn revision_merge_inner(
    state: &AppState,
    request: &revisions::MergeRevisionRequest,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> RevisionMergeResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| revisions::merge_into_draft(conn, request)));
    match outcome {
        Ok(receipt) => RevisionMergeResult {
            success: true,
            receipt: Some(receipt),
            error: None,
        },
        Err(error) => RevisionMergeResult {
            success: false,
            receipt: None,
            error: Some(mutating_error(error)),
        },
    }
}

pub(crate) fn revision_mark_merged_inner(
    state: &AppState,
    id: i64,
    merged_to_draft_id: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session).and_then(|()| {
        state.with_project_db(|conn| revisions::mark_merged(conn, id, merged_to_draft_id))
    });
    match outcome {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => SimpleResult {
            success: false,
            error: Some(mutating_error(error)),
        },
    }
}

pub(crate) fn revision_mark_discarded_inner(
    state: &AppState,
    id: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| revisions::mark_discarded(conn, id)));
    match outcome {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => SimpleResult {
            success: false,
            error: Some(mutating_error(error)),
        },
    }
}

/// `db:revision-create` —— 创建修稿（MUTATING）
#[tauri::command]
pub fn db_revision_create(
    state: State<'_, AppState>,
    params: revisions::RevisionCreateParams,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> RevisionCreateResult {
    revision_create_inner(
        state.inner(),
        &params,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:revision-replace-pending` —— 原子替换 pending 修稿（MUTATING）
#[tauri::command]
pub fn db_revision_replace_pending(
    state: State<'_, AppState>,
    params: revisions::RevisionCreateParams,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> RevisionCreateResult {
    revision_replace_pending_inner(
        state.inner(),
        &params,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:revision-list` —— 列出草稿的全部修稿
#[tauri::command]
pub fn db_revision_list(
    state: State<'_, AppState>,
    base_draft_id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<revisions::RevisionMeta>, String> {
    revision_list_inner(
        state.inner(),
        base_draft_id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:revision-get-pending` —— 列出草稿的 pending 修稿
#[tauri::command]
pub fn db_revision_get_pending(
    state: State<'_, AppState>,
    base_draft_id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<revisions::RevisionMeta>, String> {
    revision_get_pending_inner(
        state.inner(),
        base_draft_id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:revision-get-full` —— 读取修稿（含正文与冻结源稿）
#[tauri::command]
pub fn db_revision_get_full(
    state: State<'_, AppState>,
    id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Option<revisions::RevisionFull>, String> {
    revision_get_full_inner(
        state.inner(),
        id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:revision-next-index` —— 下一个修稿序号
#[tauri::command]
pub fn db_revision_next_index(
    state: State<'_, AppState>,
    base_draft_id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<i64, String> {
    revision_next_index_inner(
        state.inner(),
        base_draft_id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:revision-merge` —— 合并修稿到目标草稿（MUTATING，幂等）
#[tauri::command]
pub fn db_revision_merge(
    state: State<'_, AppState>,
    request: revisions::MergeRevisionRequest,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> RevisionMergeResult {
    revision_merge_inner(
        state.inner(),
        &request,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:revision-mark-merged` —— 标记修稿已合并（MUTATING）
#[tauri::command]
pub fn db_revision_mark_merged(
    state: State<'_, AppState>,
    id: i64,
    merged_to_draft_id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    revision_mark_merged_inner(
        state.inner(),
        id,
        merged_to_draft_id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:revision-mark-discarded` —— 标记修稿已弃用（MUTATING）
#[tauri::command]
pub fn db_revision_mark_discarded(
    state: State<'_, AppState>,
    id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    revision_mark_discarded_inner(
        state.inner(),
        id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

// ===== 审稿（reviews 子域 S3-c，5 频道） =====

/// `db:review-create` 的 IPC 信封（对齐 `{ success, id?, reviewIndex?, errorCode?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCreateResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_index: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 创建审稿：源稿守卫失败时额外回填 `errorCode`
pub(crate) fn review_create_inner(
    state: &AppState,
    params: &reviews::ReviewCreateParams,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> ReviewCreateResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| reviews::create(conn, params)));
    match outcome {
        Ok(created) => ReviewCreateResult {
            success: true,
            id: Some(created.id),
            review_index: Some(created.review_index),
            error_code: None,
            error: None,
        },
        Err(message) => {
            let error_code = (message == crate::draft_source_guard::SOURCE_DRAFT_CHANGED_MESSAGE)
                .then(|| crate::draft_source_guard::SOURCE_DRAFT_CHANGED.to_string());
            ReviewCreateResult {
                success: false,
                id: None,
                review_index: None,
                error_code,
                error: Some(mutating_error(message)),
            }
        }
    }
}

pub(crate) fn review_list_inner(
    state: &AppState,
    base_draft_id: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Vec<reviews::ReviewMeta>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| reviews::list_by_draft(conn, base_draft_id))
}

pub(crate) fn review_get_latest_inner(
    state: &AppState,
    base_draft_id: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Option<reviews::ReviewFull>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| reviews::get_latest_by_draft(conn, base_draft_id))
}

pub(crate) fn review_get_full_inner(
    state: &AppState,
    id: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Option<reviews::ReviewFull>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| reviews::get_full(conn, id))
}

pub(crate) fn review_next_index_inner(
    state: &AppState,
    base_draft_id: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<i64, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| reviews::get_next_index(conn, base_draft_id))
}

/// `db:review-create` —— 创建审稿（MUTATING）
#[tauri::command]
pub fn db_review_create(
    state: State<'_, AppState>,
    params: reviews::ReviewCreateParams,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> ReviewCreateResult {
    review_create_inner(
        state.inner(),
        &params,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:review-list` —— 列出草稿的全部审稿
#[tauri::command]
pub fn db_review_list(
    state: State<'_, AppState>,
    base_draft_id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<reviews::ReviewMeta>, String> {
    review_list_inner(
        state.inner(),
        base_draft_id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:review-get-latest` —— 读取草稿最新审稿
#[tauri::command]
pub fn db_review_get_latest(
    state: State<'_, AppState>,
    base_draft_id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Option<reviews::ReviewFull>, String> {
    review_get_latest_inner(
        state.inner(),
        base_draft_id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:review-get-full` —— 读取审稿（含报告正文）
#[tauri::command]
pub fn db_review_get_full(
    state: State<'_, AppState>,
    id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Option<reviews::ReviewFull>, String> {
    review_get_full_inner(
        state.inner(),
        id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:review-next-index` —— 下一个审稿序号
#[tauri::command]
pub fn db_review_next_index(
    state: State<'_, AppState>,
    base_draft_id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<i64, String> {
    review_next_index_inner(
        state.inner(),
        base_draft_id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

// ===== 后处理（post-process 子域 S3-d，6 频道） =====

/// `db:post-process-create-run` 的 IPC 信封（对齐 `{ success, id?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostProcessCreateRunResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub(crate) fn post_process_create_run_inner(
    state: &AppState,
    params: &post_process::PostProcessCreateParams,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> PostProcessCreateRunResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| post_process::create_run(conn, params)));
    match outcome {
        Ok(id) => PostProcessCreateRunResult {
            success: true,
            id: Some(id),
            error: None,
        },
        Err(error) => PostProcessCreateRunResult {
            success: false,
            id: None,
            error: Some(mutating_error(error)),
        },
    }
}

pub(crate) fn post_process_get_latest_run_inner(
    state: &AppState,
    source_type: &str,
    source_id: &str,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Option<post_process::PostProcessRunData>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| post_process::get_latest_run(conn, source_type, source_id))
}

pub(crate) fn post_process_get_steps_inner(
    state: &AppState,
    run_id: &str,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Vec<post_process::PostProcessStepData>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| post_process::get_steps(conn, run_id))
}

pub(crate) fn post_process_mark_step_ok_inner(
    state: &AppState,
    run_id: &str,
    step_key: &str,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| post_process::mark_step_ok(conn, run_id, step_key)));
    simple_mutating_result(outcome)
}

pub(crate) fn post_process_mark_step_failed_inner(
    state: &AppState,
    run_id: &str,
    step_key: &str,
    error_msg: &str,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session).and_then(|()| {
        state.with_project_db(|conn| {
            post_process::mark_step_failed(conn, run_id, step_key, error_msg)
        })
    });
    simple_mutating_result(outcome)
}

pub(crate) fn post_process_is_all_passed_inner(
    state: &AppState,
    source_type: &str,
    source_id: &str,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<bool, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| post_process::is_all_critical_passed(conn, source_type, source_id))
}

/// `db:post-process-create-run` —— 创建后处理跑批（MUTATING）
#[tauri::command]
pub fn db_post_process_create_run(
    state: State<'_, AppState>,
    params: post_process::PostProcessCreateParams,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> PostProcessCreateRunResult {
    post_process_create_run_inner(
        state.inner(),
        &params,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:post-process-get-latest-run` —— 读取最新跑批
#[tauri::command]
pub fn db_post_process_get_latest_run(
    state: State<'_, AppState>,
    source_type: String,
    source_id: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Option<post_process::PostProcessRunData>, String> {
    post_process_get_latest_run_inner(
        state.inner(),
        &source_type,
        &source_id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:post-process-get-steps` —— 读取跑批步骤明细
#[tauri::command]
pub fn db_post_process_get_steps(
    state: State<'_, AppState>,
    run_id: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<post_process::PostProcessStepData>, String> {
    post_process_get_steps_inner(
        state.inner(),
        &run_id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:post-process-mark-step-ok` —— 标记步骤成功（MUTATING）
#[tauri::command]
pub fn db_post_process_mark_step_ok(
    state: State<'_, AppState>,
    run_id: String,
    step_key: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    post_process_mark_step_ok_inner(
        state.inner(),
        &run_id,
        &step_key,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:post-process-mark-step-failed` —— 标记步骤失败（MUTATING）
#[tauri::command]
pub fn db_post_process_mark_step_failed(
    state: State<'_, AppState>,
    run_id: String,
    step_key: String,
    error_msg: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    post_process_mark_step_failed_inner(
        state.inner(),
        &run_id,
        &step_key,
        &error_msg,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:post-process-is-all-passed` —— 最新跑批是否全部关键步骤通过
#[tauri::command]
pub fn db_post_process_is_all_passed(
    state: State<'_, AppState>,
    source_type: String,
    source_id: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<bool, String> {
    post_process_is_all_passed_inner(
        state.inner(),
        &source_type,
        &source_id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

// ===== LLM 日志与摘要（5 频道） =====

/// `db:log-llm-call` —— 记录一次 LLM 调用（MUTATING）
///
/// 契约只声明 `{ success: boolean }`；本地统一走 MUTATING 信封（失败时额外带 `error`），
/// 比基线的内部 catch 多保留了错误信息。
pub(crate) fn log_llm_call_inner(
    state: &AppState,
    call: &serde_json::Value,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| llm::log_call(conn, call)));
    simple_mutating_result(outcome)
}

pub(crate) fn get_llm_stats_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<llm::LlmCallStats, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(llm::get_stats)
}

pub(crate) fn get_llm_history_inner(
    state: &AppState,
    limit: Option<i64>,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Vec<llm::LlmCallHistoryEntry>, String> {
    guard_read(state, expected_project_path, session)?;
    // 契约允许缺省 limit；基线默认 50
    let effective_limit = limit.unwrap_or(50);
    state.with_project_db(|conn| llm::get_history(conn, effective_limit))
}

pub(crate) fn save_summary_snapshot_inner(
    state: &AppState,
    chapter_number: i64,
    character_states: &str,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session).and_then(|()| {
        state.with_project_db(|conn| llm::save_summary_snapshot(conn, chapter_number, character_states))
    });
    simple_mutating_result(outcome)
}

pub(crate) fn get_latest_summary_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Option<llm::SummarySnapshot>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(llm::get_latest_summary_snapshot)
}

/// `db:log-llm-call` —— 记录一次 LLM 调用（MUTATING）
#[tauri::command]
pub fn db_log_llm_call(
    state: State<'_, AppState>,
    call: serde_json::Value,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    log_llm_call_inner(
        state.inner(),
        &call,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:get-llm-stats` —— LLM 调用聚合统计
#[tauri::command]
pub fn db_get_llm_stats(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<llm::LlmCallStats, String> {
    get_llm_stats_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:get-llm-history` —— 最近 LLM 调用记录
#[tauri::command]
pub fn db_get_llm_history(
    state: State<'_, AppState>,
    limit: Option<i64>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<llm::LlmCallHistoryEntry>, String> {
    get_llm_history_inner(
        state.inner(),
        limit,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:save-summary-snapshot` —— 保存旧式角色状态快照（MUTATING）
#[tauri::command]
pub fn db_save_summary_snapshot(
    state: State<'_, AppState>,
    chapter_number: i64,
    character_states: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    save_summary_snapshot_inner(
        state.inner(),
        chapter_number,
        &character_states,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:get-latest-summary` —— 读取最新旧式快照
#[tauri::command]
pub fn db_get_latest_summary(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Option<llm::SummarySnapshot>, String> {
    get_latest_summary_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:project-clear-generated-data` 的 IPC 信封
///（对齐 `{ success, cleared?, physicalFilesDeleted?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectClearGeneratedDataResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleared: Option<Vec<project_clear::ProjectClearScope>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_files_deleted: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub(crate) fn project_clear_generated_data_inner(
    state: &AppState,
    options: &project_clear::ProjectClearOptions,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> ProjectClearGeneratedDataResult {
    let outcome = guard_read(state, expected_project_path, session).and_then(|()| {
        // 项目路径来自活跃租约（不从入参拼装），用于定位根目录成稿实体稿
        let project_path = state.current_project_path();
        state.with_project_db(|conn| {
            project_clear::clear_generated_data(
                conn,
                project_path.as_deref().map(Path::new),
                options,
            )
        })
    });
    match outcome {
        Ok(result) => ProjectClearGeneratedDataResult {
            success: true,
            cleared: Some(result.cleared),
            physical_files_deleted: Some(result.physical_files_deleted),
            error: None,
        },
        Err(error) => {
            eprintln!("[db:project-clear-generated-data] 失败: {error}");
            ProjectClearGeneratedDataResult {
                success: false,
                cleared: None,
                physical_files_deleted: None,
                error: Some(mutating_error(error)),
            }
        }
    }
}

/// `db:project-clear-generated-data` —— 清空选定范围的生成数据（MUTATING）
#[tauri::command]
pub fn db_project_clear_generated_data(
    state: State<'_, AppState>,
    options: project_clear::ProjectClearOptions,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> ProjectClearGeneratedDataResult {
    project_clear_generated_data_inner(
        state.inner(),
        &options,
        &expected_project_path,
        project_session.as_ref(),
    )
}

// ===== 批次 F1：一致性豁免（consistency-exemption 子域，3 频道） =====

/// `db:consistency-exemption-list` —— 列出全部豁免（含已撤销）
#[tauri::command]
pub fn db_consistency_exemption_list(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<consistency::ConsistencyExemption>, String> {
    consistency_exemption_list_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

pub(crate) fn consistency_exemption_list_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Vec<consistency::ConsistencyExemption>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(consistency::list)
}

/// `db:consistency-exemption-save` —— 保存（upsert）豁免（MUTATING）
#[tauri::command]
pub fn db_consistency_exemption_save(
    state: State<'_, AppState>,
    stable_fact_key: String,
    reason: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    consistency_exemption_save_inner(
        state.inner(),
        &stable_fact_key,
        &reason,
        &expected_project_path,
        project_session.as_ref(),
    )
}

pub(crate) fn consistency_exemption_save_inner(
    state: &AppState,
    stable_fact_key: &str,
    reason: &str,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session).and_then(|()| {
        state.with_project_db(|conn| consistency::save(conn, stable_fact_key, reason))
    });
    simple_mutating_result(outcome)
}

/// `db:consistency-exemption-revoke` —— 撤销豁免（软删除，MUTATING）
#[tauri::command]
pub fn db_consistency_exemption_revoke(
    state: State<'_, AppState>,
    stable_fact_key: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    consistency_exemption_revoke_inner(
        state.inner(),
        &stable_fact_key,
        &expected_project_path,
        project_session.as_ref(),
    )
}

pub(crate) fn consistency_exemption_revoke_inner(
    state: &AppState,
    stable_fact_key: &str,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| consistency::revoke(conn, stable_fact_key)));
    simple_mutating_result(outcome)
}

// ===== 批次 F1：叙事线索（narrative-thread 子域，6 频道） =====

/// `db:narrative-thread-plan-create` / `-update` 的信封（对齐契约 `{ success, plan?, error? }`）
///
/// ⚠️ **失败路径不会填充 `error`**：基线这两个 handler **没有** try/catch，
/// 仓储异常直接 reject，`{ success: false }` 分支实际不可达。此处忠实平移该形状
/// （命令返回 `Err` → 前端 `invoke` reject），仅保留成功信封。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrativeThreadPlanResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<threads::NarrativeThreadPlanRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `db:narrative-thread-event-confirm` 的信封（对齐契约 `{ success, event?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrativeThreadEventResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<threads::NarrativeThreadEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `db:narrative-thread-list` —— 列出全部线索（含派生状态）
#[tauri::command]
pub fn db_narrative_thread_list(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<threads::NarrativeThreadView>, String> {
    narrative_thread_list_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

pub(crate) fn narrative_thread_list_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Vec<threads::NarrativeThreadView>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(threads::list)
}

/// `db:narrative-thread-list-relevant` —— 列出与本章相关的活跃线索
#[tauri::command]
pub fn db_narrative_thread_list_relevant(
    state: State<'_, AppState>,
    context: threads::NarrativeThreadChapterContext,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<threads::NarrativeThreadView>, String> {
    narrative_thread_list_relevant_inner(
        state.inner(),
        &context,
        &expected_project_path,
        project_session.as_ref(),
    )
}

pub(crate) fn narrative_thread_list_relevant_inner(
    state: &AppState,
    context: &threads::NarrativeThreadChapterContext,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Vec<threads::NarrativeThreadView>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| threads::list_relevant_active(conn, context))
}

/// `db:narrative-thread-plan-create` —— 创建线索计划
#[tauri::command]
pub fn db_narrative_thread_plan_create(
    state: State<'_, AppState>,
    input: threads::NarrativeThreadPlanInput,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<NarrativeThreadPlanResult, String> {
    narrative_thread_plan_create_inner(
        state.inner(),
        &input,
        &expected_project_path,
        project_session.as_ref(),
    )
}

pub(crate) fn narrative_thread_plan_create_inner(
    state: &AppState,
    input: &threads::NarrativeThreadPlanInput,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<NarrativeThreadPlanResult, String> {
    guard_read(state, expected_project_path, session)?;
    let plan = state.with_project_db(|conn| threads::create_plan(conn, input))?;
    Ok(NarrativeThreadPlanResult {
        success: true,
        plan: Some(plan),
        error: None,
    })
}

/// `db:narrative-thread-plan-update` —— 更新线索计划
#[tauri::command]
pub fn db_narrative_thread_plan_update(
    state: State<'_, AppState>,
    id: i64,
    input: threads::NarrativeThreadPlanInput,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<NarrativeThreadPlanResult, String> {
    narrative_thread_plan_update_inner(
        state.inner(),
        id,
        &input,
        &expected_project_path,
        project_session.as_ref(),
    )
}

pub(crate) fn narrative_thread_plan_update_inner(
    state: &AppState,
    id: i64,
    input: &threads::NarrativeThreadPlanInput,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<NarrativeThreadPlanResult, String> {
    guard_read(state, expected_project_path, session)?;
    let plan = state.with_project_db(|conn| threads::update_plan(conn, id, input))?;
    Ok(NarrativeThreadPlanResult {
        success: true,
        plan: Some(plan),
        error: None,
    })
}

/// `db:narrative-thread-plan-delete` —— 删除线索计划（级联清除事件）
#[tauri::command]
pub fn db_narrative_thread_plan_delete(
    state: State<'_, AppState>,
    id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<SimpleResult, String> {
    narrative_thread_plan_delete_inner(
        state.inner(),
        id,
        &expected_project_path,
        project_session.as_ref(),
    )
}

pub(crate) fn narrative_thread_plan_delete_inner(
    state: &AppState,
    id: i64,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<SimpleResult, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(|conn| threads::delete_plan(conn, id))?;
    Ok(SimpleResult {
        success: true,
        error: None,
    })
}

/// `db:narrative-thread-event-confirm` —— 确认一条线索事件（需短证据来自定稿正文）
#[tauri::command]
pub fn db_narrative_thread_event_confirm(
    state: State<'_, AppState>,
    input: threads::NarrativeThreadEventInput,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<NarrativeThreadEventResult, String> {
    narrative_thread_event_confirm_inner(
        state.inner(),
        &input,
        &expected_project_path,
        project_session.as_ref(),
    )
}

pub(crate) fn narrative_thread_event_confirm_inner(
    state: &AppState,
    input: &threads::NarrativeThreadEventInput,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<NarrativeThreadEventResult, String> {
    guard_read(state, expected_project_path, session)?;
    let event = state.with_project_db(|conn| threads::confirm_event(conn, input))?;
    Ok(NarrativeThreadEventResult {
        success: true,
        event: Some(event),
        error: None,
    })
}

// ===== 批次 F1：剧情树（plot-tree 子域，3 频道） =====

/// `db:plot-tree-save` 的信封（对齐契约 `{ success, snapshot?, errorCode?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotTreeSaveResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<crate::plot_tree::PlotTreeSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `db:plot-tree-read` —— 读取来源包（含存量快照与乐观锁版本号）
#[tauri::command]
pub fn db_plot_tree_read(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<plot_tree::PlotTreeSourceBundle, String> {
    plot_tree_read_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

pub(crate) fn plot_tree_read_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<plot_tree::PlotTreeSourceBundle, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(plot_tree::read)
}

/// `db:plot-tree-save` —— 乐观锁保存快照（MUTATING）
#[tauri::command]
pub fn db_plot_tree_save(
    state: State<'_, AppState>,
    snapshot: serde_json::Value,
    expected_source_revision: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> PlotTreeSaveResult {
    plot_tree_save_inner(
        state.inner(),
        &snapshot,
        &expected_source_revision,
        &expected_project_path,
        project_session.as_ref(),
    )
}

pub(crate) fn plot_tree_save_inner(
    state: &AppState,
    snapshot: &serde_json::Value,
    expected_source_revision: &str,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> PlotTreeSaveResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| {
            if !crate::plot_tree::is_plot_tree_source_revision(expected_source_revision) {
                return Err("剧情树来源版本无效".to_string());
            }
            state.with_project_db(|conn| {
                plot_tree::save(conn, snapshot, expected_source_revision)
            })
        });
    match outcome {
        Ok(saved) => PlotTreeSaveResult {
            success: true,
            snapshot: Some(saved),
            error_code: None,
            error: None,
        },
        Err(message) => PlotTreeSaveResult {
            success: false,
            snapshot: None,
            error_code: message
                .contains("剧情资料在生成期间已更新")
                .then(|| "sources-changed".to_string()),
            error: Some(mutating_error(message)),
        },
    }
}

/// `db:plot-tree-clear` —— 清除快照缓存（不动来源事实）
#[tauri::command]
pub fn db_plot_tree_clear(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<SimpleResult, String> {
    plot_tree_clear_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

pub(crate) fn plot_tree_clear_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<SimpleResult, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(plot_tree::clear)?;
    Ok(SimpleResult {
        success: true,
        error: None,
    })
}

// ===== 批次 E：生成失败恢复候选（recovery-candidate 子域，4 频道） =====

/// `db:recovery-candidate-{record,update}` 的 IPC 信封（对齐基线 `{ success, candidate?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCandidateResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<recovery::RecoveryCandidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `projectId` 由已校验租约注入（基线 `active.projectId`），不经渲染层传入
pub(crate) fn recovery_candidate_record_inner(
    state: &AppState,
    request: &recovery::RecoveryCandidateRecordRequest,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> RecoveryCandidateResult {
    let outcome = guard_read(state, expected_project_path, session).and_then(|()| {
        let project_id = session
            .map(|s| s.project_id.clone())
            .unwrap_or_default();
        state.with_project_db(|conn| recovery::record(conn, &project_id, request))
    });
    match outcome {
        Ok(candidate) => RecoveryCandidateResult {
            success: true,
            candidate: Some(candidate),
            error: None,
        },
        Err(error) => {
            eprintln!("[db:recovery-candidate-record] 失败: {error}");
            RecoveryCandidateResult {
                success: false,
                candidate: None,
                error: Some(mutating_error(error)),
            }
        }
    }
}

/// `db:recovery-candidate-list`（读频道：失败直接拒绝）
pub(crate) fn recovery_candidate_list_inner(
    state: &AppState,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Vec<recovery::RecoveryCandidate>, String> {
    guard_read(state, expected_project_path, session)?;
    state.with_project_db(recovery::list_pending)
}

pub(crate) fn recovery_candidate_update_inner(
    state: &AppState,
    candidate_id: &str,
    visible_text: &str,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> RecoveryCandidateResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| recovery::update_pending(conn, candidate_id, visible_text)));
    match outcome {
        Ok(candidate) => RecoveryCandidateResult {
            success: true,
            candidate: Some(candidate),
            error: None,
        },
        Err(error) => {
            eprintln!("[db:recovery-candidate-update] 失败: {error}");
            RecoveryCandidateResult {
                success: false,
                candidate: None,
                error: Some(mutating_error(error)),
            }
        }
    }
}

/// `db:recovery-candidate-resolve`（基线返回 void，信封对齐 `{ success }`）
pub(crate) fn recovery_candidate_resolve_inner(
    state: &AppState,
    candidate_id: &str,
    status: &str,
    expected_project_path: &str,
    session: Option<&ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state, expected_project_path, session)
        .and_then(|()| state.with_project_db(|conn| recovery::resolve(conn, candidate_id, status)));
    simple_mutating_result(outcome)
}

/// `db:recovery-candidate-record`
#[tauri::command]
pub fn db_recovery_candidate_record(
    state: State<'_, AppState>,
    request: recovery::RecoveryCandidateRecordRequest,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> RecoveryCandidateResult {
    recovery_candidate_record_inner(
        state.inner(),
        &request,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:recovery-candidate-list`
#[tauri::command]
pub fn db_recovery_candidate_list(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<recovery::RecoveryCandidate>, String> {
    recovery_candidate_list_inner(
        state.inner(),
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:recovery-candidate-update`
#[tauri::command]
pub fn db_recovery_candidate_update(
    state: State<'_, AppState>,
    candidate_id: String,
    visible_text: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> RecoveryCandidateResult {
    recovery_candidate_update_inner(
        state.inner(),
        &candidate_id,
        &visible_text,
        &expected_project_path,
        project_session.as_ref(),
    )
}

/// `db:recovery-candidate-resolve`
#[tauri::command]
pub fn db_recovery_candidate_resolve(
    state: State<'_, AppState>,
    candidate_id: String,
    status: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    recovery_candidate_resolve_inner(
        state.inner(),
        &candidate_id,
        &status,
        &expected_project_path,
        project_session.as_ref(),
    )
}

// ===== 批次 E：定稿连续性投影（continuity 子域，4 频道） =====

/// `db:continuity-save-finalized`
#[tauri::command]
pub fn db_continuity_save_finalized(
    state: State<'_, AppState>,
    request: continuity::SaveFinalizedContinuityRequest,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state.inner(), &expected_project_path, project_session.as_ref())
        .and_then(|()| state.with_project_db(|conn| continuity::save_finalized_continuity(conn, &request)));
    simple_mutating_result(outcome)
}

/// `db:continuity-save-character-state-candidates`
#[tauri::command]
pub fn db_continuity_save_character_state_candidates(
    state: State<'_, AppState>,
    request: continuity::SaveFinalizedCharacterStateCandidatesRequest,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> SimpleResult {
    let outcome = guard_read(state.inner(), &expected_project_path, project_session.as_ref()).and_then(|()| {
        state
            .with_project_db(|conn| continuity::save_finalized_character_state_candidates(conn, &request))
    });
    simple_mutating_result(outcome)
}

/// `db:continuity-list-before`（读频道）
#[tauri::command]
pub fn db_continuity_list_before(
    state: State<'_, AppState>,
    chapter_number: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<continuity::FinalizedContinuityProjection>, String> {
    guard_read(state.inner(), &expected_project_path, project_session.as_ref())?;
    state.with_project_db(|conn| continuity::list_finalized_continuity_before(conn, chapter_number))
}

/// `db:continuity-read-source`（读频道；基线在库未开时返回 invalid 而非报错，
/// 但本侧 with_project_db 未开库即报错，语义由门禁先行拦截，行为等价）
#[tauri::command]
pub fn db_continuity_read_source(
    state: State<'_, AppState>,
    draft_id: i64,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<continuity::FinalizedSourceReadResult, String> {
    guard_read(state.inner(), &expected_project_path, project_session.as_ref())?;
    state.with_project_db(|conn| continuity::read_finalized_source(conn, draft_id))
}

// ===== 批次 E：定稿回链 + 导出权威（finalization-link / draft 收尾 4 频道之一部分） =====

/// `db:finalization-link-knowledge-document` 的 IPC 信封
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizationLinkResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finalization: Option<finalization::FinalizationRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `db:finalization-link-knowledge-document`
#[tauri::command]
pub fn db_finalization_link_knowledge_document(
    state: State<'_, AppState>,
    draft_id: i64,
    document_id: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> FinalizationLinkResult {
    let outcome = guard_read(state.inner(), &expected_project_path, project_session.as_ref())
        .and_then(|()| state.with_project_db(|conn| finalization::link_knowledge_document(conn, draft_id, &document_id)));
    match outcome {
        Ok(record) => FinalizationLinkResult {
            success: true,
            finalization: Some(record),
            error: None,
        },
        Err(error) => {
            eprintln!("[db:finalization-link-knowledge-document] 失败: {error}");
            FinalizationLinkResult {
                success: false,
                finalization: None,
                error: Some(mutating_error(error)),
            }
        }
    }
}

/// `db:draft-authority-sequence`（读频道）
#[tauri::command]
pub fn db_draft_authority_sequence(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<draft_import::AuthoritativeChapterSequence, String> {
    guard_read(state.inner(), &expected_project_path, project_session.as_ref())?;
    state.with_project_db(draft_import::authority_sequence)
}

/// `db:draft-export-snapshot`（读频道）
#[tauri::command]
pub fn db_draft_export_snapshot(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Vec<finalization::FinalizedDraftExportSnapshot>, String> {
    guard_read(state.inner(), &expected_project_path, project_session.as_ref())?;
    state.with_project_db(finalization::list_authoritative_for_export)
}

/// `db:draft-export-authority-current`（读频道；基线对结构不合法返回 false 而非报错）
#[tauri::command]
pub fn db_draft_export_authority_current(
    state: State<'_, AppState>,
    receipt: Vec<finalization::FinalizedDraftExportAuthorityItem>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<bool, String> {
    guard_read(state.inner(), &expected_project_path, project_session.as_ref())?;
    state
        .with_project_db(|conn| Ok(finalization::matches_authoritative_export_receipt(conn, &receipt)))
}

/// `db:draft-import-finalized-batch` 的 IPC 信封（对齐基线 `{ success, receipt?, error? }`）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedDraftImportResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<draft_import::FinalizedDraftImportReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `db:draft-import-finalized-batch` —— projectRoot 取自已校验路径（基线 currentProjectPath）
#[tauri::command]
pub fn db_draft_import_finalized_batch(
    state: State<'_, AppState>,
    request: draft_import::FinalizedDraftImportRequest,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> FinalizedDraftImportResult {
    let outcome = guard_read(state.inner(), &expected_project_path, project_session.as_ref())
        .and_then(|()| state.with_project_db(|conn| draft_import::commit(conn, &expected_project_path, &request)));
    match outcome {
        Ok(receipt) => FinalizedDraftImportResult {
            success: true,
            receipt: Some(receipt),
            error: None,
        },
        Err(error) => {
            eprintln!("[db:draft-import-finalized-batch] 失败: {error}");
            FinalizedDraftImportResult {
                success: false,
                receipt: None,
                error: Some(mutating_error(error)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_access::random_uuid_v4;
    use crate::state::ActiveProject;

    /// 建一个带真实项目库与活跃会话的测试状态
    fn activated_state(name: &str) -> (AppState, String, ProjectSessionContext) {
        let root = std::env::temp_dir().join(format!("anw-db-cmd-{name}-{}", random_uuid_v4()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let root_text = root.to_string_lossy().to_string();

        let state = AppState::new();
        let lease = ActiveProject {
            project_id: random_uuid_v4(),
            lease_id: random_uuid_v4(),
            root_path: root_text.clone(),
        };
        let session = ProjectSessionContext {
            project_id: lease.project_id.clone(),
            lease_id: lease.lease_id.clone(),
            project_path: lease.root_path.clone(),
        };
        state.activate_project(lease).unwrap();
        // 打开后初始化主台账（基线：open 空目录时按默认写作语言 init）
        state
            .with_project_db(|conn| {
                project_core::init(conn, "测试项目", project_core::DEFAULT_WRITING_LANGUAGE)
            })
            .unwrap();
        (state, root_text, session)
    }

    fn cleanup(root: &str) {
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn project_core_get_requires_session_and_path_test() {
        let (state, root, session) = activated_state("guard");

        // 缺会话
        let error = project_core_get_inner(&state, &root, None).unwrap_err();
        assert_eq!(error, "缺少项目会话上下文，已拒绝操作");

        // 会话租约正确但路径不符
        let error = project_core_get_inner(&state, "C:\\other", Some(&session)).unwrap_err();
        assert_eq!(error, "检测到跨项目读写，已拒绝操作。");

        // 会话租约伪造（lease 不匹配）
        let forged = ProjectSessionContext {
            project_id: session.project_id.clone(),
            lease_id: random_uuid_v4(),
            project_path: session.project_path.clone(),
        };
        let error = project_core_get_inner(&state, &root, Some(&forged)).unwrap_err();
        assert_eq!(error, "项目租约已失效，已拒绝操作。");

        cleanup(&root);
    }

    #[test]
    fn project_core_update_and_get_roundtrip_test() {
        let (state, root, session) = activated_state("roundtrip");

        let mut data = serde_json::Map::new();
        data.insert("projectName".to_string(), serde_json::json!("我的项目"));
        data.insert("genre".to_string(), serde_json::json!("仙侠"));
        data.insert("totalChapters".to_string(), serde_json::json!(200));

        let result = project_core_update_inner(&state, &data, &root, Some(&session));
        assert!(result.success, "更新应成功：{:?}", result.error);

        let core = project_core_get_inner(&state, &root, Some(&session))
            .unwrap()
            .expect("初始化后应可读取");
        assert_eq!(core.project_name, "我的项目");
        assert_eq!(core.genre, "仙侠");
        assert_eq!(core.total_chapters, 200);

        cleanup(&root);
    }

    #[test]
    fn project_core_update_rejects_characters_arch_test() {
        let (state, root, session) = activated_state("readonly");
        let mut data = serde_json::Map::new();
        data.insert("charactersArch".to_string(), serde_json::json!("派生投影"));

        let result = project_core_update_inner(&state, &data, &root, Some(&session));
        assert!(!result.success);
        let expected = format!("Error: {}", project_core::CHARACTERS_ARCH_READONLY_MESSAGE);
        assert_eq!(result.error.as_deref(), Some(expected.as_str()));

        cleanup(&root);
    }

    #[test]
    fn synopsis_commit_success_and_conflict_test() {
        let (state, root, session) = activated_state("synopsis");

        let request = project_core::ProjectCoreSynopsisCommitRequest {
            synopsis: "新大纲".to_string(),
            expected: project_core::ProjectCoreSynopsisExpected {
                synopsis: String::new(),
                premise: String::new(),
                characters_arch: String::new(),
                worldbuilding: String::new(),
                genre: String::new(),
                total_chapters: 100,
                words_per_chapter: 3000,
                writing_language: "zh-CN".to_string(),
                plot_structure: "three_act".to_string(),
                narrative_pov: "third_limited".to_string(),
                global_guidance: String::new(),
            },
        };
        let result = project_core_synopsis_commit_inner(&state, &request, &root, Some(&session));
        assert!(result.success, "快照一致应写入：{:?}", result.error);

        // 再次提交：库中 synopsis 已变化，expected 仍是空 → 拒绝覆盖
        let conflict = project_core_synopsis_commit_inner(&state, &request, &root, Some(&session));
        assert!(!conflict.success);
        assert_eq!(
            conflict.error.as_deref(),
            Some(project_core::SYNOPSIS_CONFLICT_MESSAGE)
        );

        cleanup(&root);
    }

    #[test]
    fn close_releases_database_and_session_test() {
        let (state, root, session) = activated_state("close");

        let mismatch = close_inner(&state, "C:\\other", Some(&session));
        assert!(!mismatch.success, "路径不符时不得关闭");

        let no_session = close_inner(&state, &root, None);
        assert!(!no_session.success, "缺会话时不得关闭");

        let result = close_inner(&state, &root, Some(&session));
        assert!(result.success, "路径一致应关闭成功");
        assert!(state.current_project_path().is_none(), "会话应失效");

        // 关闭后读频道必须拒绝
        let error = project_core_get_inner(&state, &root, Some(&session)).unwrap_err();
        assert_eq!(error, "项目租约已失效，已拒绝操作。");

        cleanup(&root);
    }

    #[test]
    fn update_without_open_database_reports_structured_error_test() {
        let state = AppState::new();
        let mut data = serde_json::Map::new();
        data.insert("genre".to_string(), serde_json::json!("科幻"));
        let result = project_core_update_inner(
            &state,
            &data,
            "C:\\x",
            Some(&ProjectSessionContext {
                project_id: "p".to_string(),
                lease_id: "l".to_string(),
                project_path: "C:\\x".to_string(),
            }),
        );
        assert!(!result.success);
        assert_eq!(
            result.error.as_deref(),
            Some("Error: 项目租约已失效，已拒绝操作。")
        );
    }

    #[test]
    fn character_roster_channels_guard_and_commit_test() {
        let (state, root, session) = activated_state("roster");
        let payload = serde_json::json!({
            "operationId": "op-1",
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

        let missing_session = character_roster_commit_inner(&state, &payload, &root, None);
        assert!(!missing_session.success);
        assert_eq!(
            missing_session.error.as_deref(),
            Some("Error: 缺少项目会话上下文，已拒绝操作")
        );

        let wrong_path =
            character_roster_commit_inner(&state, &payload, "C:\\other", Some(&session));
        assert!(!wrong_path.success);
        assert_eq!(
            wrong_path.error.as_deref(),
            Some("Error: 检测到跨项目读写，已拒绝操作。")
        );

        let result = character_roster_commit_inner(&state, &payload, &root, Some(&session));
        assert!(result.success, "提交应成功：{:?}", result.error);
        let receipt = result.receipt.expect("应返回回执");
        assert_eq!(receipt.revision, 1);
        assert!(!receipt.idempotent);

        // 校验失败同样走 MUTATING 包装
        let invalid =
            character_roster_commit_inner(&state, &serde_json::json!([1, 2, 3]), &root, Some(&session));
        assert!(!invalid.success);
        assert_eq!(
            invalid.error.as_deref(),
            Some("Error: 角色名单提交请求格式无效")
        );

        let characters = character_get_all_inner(&state, &root, Some(&session)).unwrap();
        assert_eq!(characters.len(), 1);
        assert_eq!(characters[0].name, "林清玄");

        let snapshot = character_roster_read_inner(&state, &root, Some(&session)).unwrap();
        assert_eq!(snapshot.revision, 1);
        assert_eq!(
            snapshot.migration_state,
            roster::CharacterRosterMigrationState::Ready
        );

        // 读频道缺会话 → 直接拒绝（不包装）
        let denied = character_get_all_inner(&state, &root, None).unwrap_err();
        assert_eq!(denied, "缺少项目会话上下文，已拒绝操作");

        cleanup(&root);
    }

    fn sample_blueprint(chapter: i64) -> blueprints::BlueprintData {
        blueprints::BlueprintData {
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
            notes: String::new(),
            notes_updated_at: String::new(),
        }
    }

    #[test]
    fn blueprint_channels_guard_and_crud_test() {
        let (state, root, session) = activated_state("blueprint");

        // 读频道门禁：缺会话 / 跨项目路径
        assert_eq!(
            blueprint_get_all_inner(&state, &root, None).unwrap_err(),
            "缺少项目会话上下文，已拒绝操作"
        );
        assert_eq!(
            blueprint_get_all_inner(&state, "C:\\other", Some(&session)).unwrap_err(),
            "检测到跨项目读写，已拒绝操作。"
        );

        // 写频道门禁：失败走 MUTATING 包装
        let denied = blueprint_upsert_inner(&state, &sample_blueprint(1), &root, None);
        assert!(!denied.success);
        assert_eq!(
            denied.error.as_deref(),
            Some("Error: 缺少项目会话上下文，已拒绝操作")
        );

        // 正常写入 + 回读
        let one = blueprint_upsert_inner(&state, &sample_blueprint(1), &root, Some(&session));
        assert!(one.success, "单条写入应成功：{:?}", one.error);
        let many = blueprint_upsert_many_inner(
            &state,
            &[sample_blueprint(2), sample_blueprint(3)],
            &root,
            Some(&session),
        );
        assert!(many.success, "批量写入应成功：{:?}", many.error);

        let all = blueprint_get_all_inner(&state, &root, Some(&session)).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].chapter_number, 1);
        assert_eq!(all[0].characters, vec!["林清玄".to_string()]);

        let single = blueprint_get_inner(&state, 2, &root, Some(&session))
            .unwrap()
            .expect("第 2 章应存在");
        assert_eq!(single.title, "第 2 章");
        assert!(blueprint_get_inner(&state, 99, &root, Some(&session))
            .unwrap()
            .is_none());

        // 仅更新 notes，返回 updated 判定
        let notes = blueprint_update_notes_inner(&state, 1, "要点", &root, Some(&session));
        assert!(notes.success);
        assert_eq!(notes.updated, Some(true));
        let missing_notes = blueprint_update_notes_inner(&state, 99, "空", &root, Some(&session));
        assert!(missing_notes.success);
        assert_eq!(missing_notes.updated, Some(false));
        assert_eq!(
            blueprint_get_inner(&state, 1, &root, Some(&session))
                .unwrap()
                .unwrap()
                .notes,
            "要点"
        );

        // 删除与清空
        assert!(blueprint_delete_inner(&state, 1, &root, Some(&session)).success);
        assert_eq!(blueprint_get_all_inner(&state, &root, Some(&session)).unwrap().len(), 2);
        assert!(blueprint_clear_all_inner(&state, &root, Some(&session)).success);
        assert!(blueprint_get_all_inner(&state, &root, Some(&session)).unwrap().is_empty());

        cleanup(&root);
    }

    fn commit_request(
        mode: &str,
        operation_id: &str,
        start_chapter: i64,
        end_chapter: i64,
        chapters: &[i64],
    ) -> blueprints::BlueprintCommitRangeRequest {
        blueprints::BlueprintCommitRangeRequest {
            mode: mode.to_string(),
            operation_id: operation_id.to_string(),
            start_chapter,
            end_chapter,
            blueprints: chapters.iter().map(|chapter| sample_blueprint(*chapter)).collect(),
        }
    }

    #[test]
    fn blueprint_commit_range_channels_test() {
        let (state, root, session) = activated_state("commit-range");
        let request = commit_request("full", "op-1", 1, 2, &[1, 2]);

        // 门禁：缺会话 → MUTATING 包装
        let denied = blueprint_commit_range_inner(&state, &request, &root, None);
        assert!(!denied.success);
        assert_eq!(
            denied.error.as_deref(),
            Some("Error: 缺少项目会话上下文，已拒绝操作")
        );

        // 范围不完整
        let incomplete = blueprint_commit_range_inner(
            &state,
            &commit_request("full", "op-x", 1, 2, &[1]),
            &root,
            Some(&session),
        );
        assert!(!incomplete.success);
        assert_eq!(
            incomplete.error.as_deref(),
            Some("Error: 蓝图提交必须完整且唯一地覆盖第 1–2 章")
        );

        // full 模式必须从第 1 章开始
        let not_from_one = blueprint_commit_range_inner(
            &state,
            &commit_request("full", "op-y", 2, 2, &[2]),
            &root,
            Some(&session),
        );
        assert!(!not_from_one.success);
        assert_eq!(
            not_from_one.error.as_deref(),
            Some("Error: 全量蓝图提交必须从第 1 章开始")
        );

        // 首次提交
        let first = blueprint_commit_range_inner(&state, &request, &root, Some(&session));
        assert!(first.success, "提交应成功：{:?}", first.error);
        let receipt = first.receipt.expect("应返回回执");
        assert!(!receipt.idempotent);
        assert_eq!(receipt.mode, "full");
        assert_eq!(receipt.chapter_numbers, vec![1, 2]);
        assert_eq!(receipt.snapshot.len(), 2);
        assert_eq!(receipt.character_sync_input.len(), 2);
        assert_eq!(receipt.character_sync_operation.status, "pending");
        assert_eq!(
            receipt.character_sync_operation.operation_id,
            "blueprint-sync-op-1"
        );
        assert!(receipt.character_sync_operation.completion_receipt.is_none());

        // 幂等重放：同操作 ID + 同负载 → 回读回执
        let replay = blueprint_commit_range_inner(&state, &request, &root, Some(&session));
        assert!(replay.success, "重放应成功：{:?}", replay.error);
        let replay_receipt = replay.receipt.expect("重放应返回回执");
        assert!(replay_receipt.idempotent);
        assert_eq!(replay_receipt.payload_hash, receipt.payload_hash);
        assert_eq!(replay_receipt.chapter_numbers, vec![1, 2]);

        // 同操作 ID + 不同负载 → 拒绝覆盖
        let mut conflicting = request.clone();
        conflicting.blueprints[1].title = "被篡改的标题".to_string();
        let conflict = blueprint_commit_range_inner(&state, &conflicting, &root, Some(&session));
        assert!(!conflict.success);
        assert_eq!(
            conflict.error.as_deref(),
            Some("Error: 操作 ID 已被用于不同的蓝图提交，已拒绝覆盖")
        );

        // replace-range：仅覆盖第 2 章范围，保留范围外章节
        let replaced = blueprint_commit_range_inner(
            &state,
            &commit_request("replace-range", "op-2", 2, 2, &[2]),
            &root,
            Some(&session),
        );
        assert!(replaced.success, "区间提交应成功：{:?}", replaced.error);
        let all = blueprint_get_all_inner(&state, &root, Some(&session)).unwrap();
        assert_eq!(all.len(), 2, "区间模式不得删除范围外章节");

        // full 模式反向裁剪范围外章节
        let shrunk = blueprint_commit_range_inner(
            &state,
            &commit_request("full", "op-3", 1, 1, &[1]),
            &root,
            Some(&session),
        );
        assert!(shrunk.success, "全量裁剪应成功：{:?}", shrunk.error);
        assert_eq!(blueprint_get_all_inner(&state, &root, Some(&session)).unwrap().len(), 1);

        cleanup(&root);
    }

    #[test]
    fn blueprint_character_sync_channels_test() {
        let (state, root, session) = activated_state("sync");
        let request = commit_request("full", "op-1", 1, 1, &[1]);
        assert!(blueprint_commit_range_inner(&state, &request, &root, Some(&session)).success);
        let sync_id = "blueprint-sync-op-1";

        // 读频道门禁：缺会话 → 直接拒绝
        assert_eq!(
            blueprint_character_sync_list_pending_inner(&state, &root, None).unwrap_err(),
            "缺少项目会话上下文，已拒绝操作"
        );

        let pending =
            blueprint_character_sync_list_pending_inner(&state, &root, Some(&session)).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].status, "pending");
        assert_eq!(pending[0].operation_id, sync_id);

        let fetched = blueprint_character_sync_get_inner(&state, sync_id, &root, Some(&session))
            .unwrap()
            .expect("应读到同步操作");
        assert_eq!(fetched.blueprint_commit_operation_id, "op-1");
        assert!(blueprint_character_sync_get_inner(&state, "不存在", &root, Some(&session))
            .unwrap()
            .is_none());

        // 完成（MUTATING）：无名单证据 → already-satisfied
        let completed =
            blueprint_character_sync_complete_inner(&state, sync_id, &root, Some(&session));
        assert!(completed.success, "完成应成功：{:?}", completed.error);
        let operation = completed.operation.expect("应返回操作");
        assert_eq!(operation.status, "completed");
        assert_eq!(
            operation.completion_receipt.as_ref().map(|receipt| receipt.status.as_str()),
            Some("already-satisfied")
        );

        // MUTATING 失败包装
        let missing =
            blueprint_character_sync_complete_inner(&state, "不存在", &root, Some(&session));
        assert!(!missing.success);
        assert_eq!(
            missing.error.as_deref(),
            Some("Error: 待处理蓝图角色同步操作不存在")
        );
        let denied = blueprint_character_sync_complete_inner(&state, sync_id, &root, None);
        assert!(!denied.success);
        assert_eq!(
            denied.error.as_deref(),
            Some("Error: 缺少项目会话上下文，已拒绝操作")
        );

        // 完成后待处理列表清空
        assert!(blueprint_character_sync_list_pending_inner(&state, &root, Some(&session))
            .unwrap()
            .is_empty());

        cleanup(&root);
    }

    #[test]
    fn draft_channels_test() {
        let (state, root, session) = activated_state("draft");

        // 读频道门禁
        assert_eq!(
            draft_list_all_inner(&state, &root, None).unwrap_err(),
            "缺少项目会话上下文，已拒绝操作"
        );
        assert_eq!(
            draft_list_all_inner(&state, "C:\\other", Some(&session)).unwrap_err(),
            "检测到跨项目读写，已拒绝操作。"
        );

        let params = drafts::DraftCreateParams {
            chapter_number: 1,
            // 契约必传，但基线忽略入参 version、在事务内重新分配
            version: Some(99),
            source: "write".to_string(),
            content: "第一章正文".to_string(),
            word_count: 5,
            source_dependencies: None,
        };

        // 写入门禁：缺会话 → MUTATING 包装
        let denied = draft_create_inner(&state, &params, &root, None);
        assert!(!denied.success);
        assert_eq!(
            denied.error.as_deref(),
            Some("Error: 缺少项目会话上下文，已拒绝操作")
        );

        let created = draft_create_inner(&state, &params, &root, Some(&session));
        assert!(created.success, "创建应成功：{:?}", created.error);
        let id = created.id.expect("应返回草稿 ID");

        let list = draft_list_inner(&state, 1, &root, Some(&session)).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].version, 1, "version 必须由事务分配，而非入参 99");
        assert_eq!(
            draft_next_version_inner(&state, 1, &root, Some(&session)).unwrap(),
            2
        );

        let full = draft_get_full_inner(&state, id, &root, Some(&session))
            .unwrap()
            .expect("应读到草稿正文");
        assert_eq!(full.content, "第一章正文");
        assert_eq!(full.meta.id, id);
        assert!(draft_get_meta_inner(&state, 9999, &root, Some(&session))
            .unwrap()
            .is_none());
        assert!(draft_get_finalized_inner(&state, 1, &root, Some(&session))
            .unwrap()
            .is_none());

        // 更新正文
        let updated = draft_update_content_inner(&state, id, "改后正文", 4, &root, Some(&session));
        assert!(updated.success, "更新应成功：{:?}", updated.error);
        assert_eq!(
            draft_get_full_inner(&state, id, &root, Some(&session))
                .unwrap()
                .unwrap()
                .content,
            "改后正文"
        );

        // 更新状态
        assert!(draft_update_status_inner(&state, id, "revised", None, &root, Some(&session)).success);
        assert_eq!(
            draft_get_meta_inner(&state, id, &root, Some(&session))
                .unwrap()
                .unwrap()
                .status,
            "revised"
        );

        // 草稿不存在的前置文案（controller 层）
        let missing = draft_update_content_inner(&state, 9999, "x", 1, &root, Some(&session));
        assert!(!missing.success);
        assert_eq!(missing.error.as_deref(), Some("Error: 草稿不存在：9999"));

        assert_eq!(
            draft_get_max_finalized_chapter_inner(&state, &root, Some(&session)).unwrap(),
            0
        );

        // 删除普通草稿
        let deleted = draft_delete_inner(&state, id, &root, Some(&session));
        assert!(deleted.success, "删除应成功：{:?}", deleted.error);
        assert!(deleted.error_code.is_none());
        assert!(draft_list_all_inner(&state, &root, Some(&session)).unwrap().is_empty());

        // 定稿草稿：删除→errorCode；正文→只读文案
        let again = draft_create_inner(&state, &params, &root, Some(&session));
        let finalized_id = again.id.expect("应返回草稿 ID");
        state
            .with_project_db(|conn| {
                conn.execute(
                    "UPDATE drafts SET status = 'finalized' WHERE id = ?1",
                    [finalized_id],
                )
                .map(|_| ())
                .map_err(|error| error.to_string())
            })
            .unwrap();
        assert_eq!(
            draft_get_max_finalized_chapter_inner(&state, &root, Some(&session)).unwrap(),
            1
        );

        let blocked = draft_delete_inner(&state, finalized_id, &root, Some(&session));
        assert!(!blocked.success);
        assert_eq!(
            blocked.error_code.as_deref(),
            Some("FINALIZED_DRAFT_DELETE_REQUIRED")
        );
        assert!(blocked.error.is_none(), "定稿删除入口只回 errorCode");

        let read_only =
            draft_update_content_inner(&state, finalized_id, "改", 1, &root, Some(&session));
        assert!(!read_only.success);
        assert_eq!(
            read_only.error.as_deref(),
            Some("Error: 已定稿正文为只读内容，不能再修改")
        );

        cleanup(&root);
    }

    fn revision_params(
        base_draft_id: i64,
        content: &str,
        source: crate::draft_source_guard::ExpectedDraftSource,
    ) -> revisions::RevisionCreateParams {
        revisions::RevisionCreateParams {
            base_draft_id,
            revision_type: "refine".to_string(),
            user_prompt: Some("润色".to_string()),
            review_source_id: None,
            content: content.to_string(),
            word_count: content.chars().count() as i64,
            expected_source: Some(source),
        }
    }

    #[test]
    fn revision_channels_test() {
        use crate::draft_source_guard::ExpectedDraftSource;

        let (state, root, session) = activated_state("revision");

        // 读频道门禁
        assert_eq!(
            revision_list_inner(&state, 1, &root, None).unwrap_err(),
            "缺少项目会话上下文，已拒绝操作"
        );

        // 建草稿
        let draft = draft_create_inner(
            &state,
            &drafts::DraftCreateParams {
                chapter_number: 1,
                version: None,
                source: "write".to_string(),
                content: "基础正文".to_string(),
                word_count: 4,
                source_dependencies: None,
            },
            &root,
            Some(&session),
        );
        let draft_id = draft.id.expect("应返回草稿 ID");
        let source = ExpectedDraftSource {
            id: draft_id,
            chapter_number: 1,
            version: 1,
            status: "draft".to_string(),
            content: "基础正文".to_string(),
        };
        let params = revision_params(draft_id, "修稿正文", source.clone());

        // create（MUTATING）
        let created = revision_create_inner(&state, &params, &root, Some(&session));
        assert!(created.success, "创建修稿应成功：{:?}", created.error);
        let revision_id = created.id.expect("应返回修稿 ID");
        assert_eq!(created.revision_index, Some(1));

        // 源守卫失败 → 同时带 errorCode 与 error（`String(err)` 形态）
        let mut missing_source = params.clone();
        missing_source.expected_source = None;
        let denied = revision_create_inner(&state, &missing_source, &root, Some(&session));
        assert!(!denied.success);
        assert_eq!(denied.error_code.as_deref(), Some("SOURCE_DRAFT_CHANGED"));
        assert_eq!(denied.error.as_deref(), Some("Error: SOURCE_DRAFT_CHANGED"));

        // 读频道
        let list = revision_list_inner(&state, draft_id, &root, Some(&session)).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(
            revision_get_pending_inner(&state, draft_id, &root, Some(&session))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            revision_next_index_inner(&state, draft_id, &root, Some(&session)).unwrap(),
            2
        );
        let full = revision_get_full_inner(&state, revision_id, &root, Some(&session))
            .unwrap()
            .expect("应读到修稿");
        assert_eq!(full.content, "修稿正文");
        assert_eq!(full.source_draft.as_ref().unwrap().id, draft_id);

        // replace-pending：新修稿取代旧 pending
        let replaced = revision_replace_pending_inner(&state, &params, &root, Some(&session));
        assert!(replaced.success, "替换应成功：{:?}", replaced.error);
        let replaced_id = replaced.id.expect("应返回修稿 ID");
        assert_eq!(replaced.revision_index, Some(2));
        let pending = revision_get_pending_inner(&state, draft_id, &root, Some(&session)).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, replaced_id);

        // merge（幂等）
        let request = revisions::MergeRevisionRequest {
            revision_id: replaced_id,
            target_draft_id: draft_id,
            expected_draft_content: "基础正文".to_string(),
            merged_content: "合并后正文".to_string(),
            word_count: 5,
        };
        let merged = revision_merge_inner(&state, &request, &root, Some(&session));
        assert!(merged.success, "合并应成功：{:?}", merged.error);
        let receipt = merged.receipt.expect("应返回合并回执");
        assert!(!receipt.idempotent);
        assert_eq!(receipt.status, "revised");
        let replay = revision_merge_inner(&state, &request, &root, Some(&session));
        assert!(
            replay.receipt.expect("重放应返回回执").idempotent,
            "同一 revisionId 重放必须幂等"
        );

        // 基于合并后的草稿事实新建修稿，再测「目标正文已变化」
        let merged_source = ExpectedDraftSource {
            id: draft_id,
            chapter_number: 1,
            version: 1,
            status: "revised".to_string(),
            content: "合并后正文".to_string(),
        };
        let another = revision_create_inner(
            &state,
            &revision_params(draft_id, "后续修稿", merged_source),
            &root,
            Some(&session),
        );
        let another_id = another.id.expect("应返回修稿 ID");
        let mismatch = revision_merge_inner(
            &state,
            &revisions::MergeRevisionRequest {
                revision_id: another_id,
                target_draft_id: draft_id,
                expected_draft_content: "不是当前正文".to_string(),
                merged_content: "x".to_string(),
                word_count: 1,
            },
            &root,
            Some(&session),
        );
        assert!(!mismatch.success);
        assert_eq!(
            mismatch.error.as_deref(),
            Some("Error: 目标草稿正文已变化，请重新打开修订对比")
        );

        // 非 pending 修稿不得再标记（MUTATING 文案）
        let failed = revision_mark_discarded_inner(&state, replaced_id, &root, Some(&session));
        assert!(!failed.success);
        let expected_error = format!(
            "Error: [RevisionRepository] 无法弃用修稿 #{replaced_id}：不存在或非 pending 状态"
        );
        assert_eq!(failed.error.as_deref(), Some(expected_error.as_str()));

        // mark-discarded 成功路径
        assert!(revision_mark_discarded_inner(&state, another_id, &root, Some(&session)).success);
        assert!(revision_get_pending_inner(&state, draft_id, &root, Some(&session))
            .unwrap()
            .is_empty());

        cleanup(&root);
    }

    #[test]
    fn review_channels_test() {
        use crate::draft_source_guard::ExpectedDraftSource;

        let (state, root, session) = activated_state("review");

        // 读频道门禁
        assert_eq!(
            review_list_inner(&state, 1, &root, None).unwrap_err(),
            "缺少项目会话上下文，已拒绝操作"
        );

        let draft = draft_create_inner(
            &state,
            &drafts::DraftCreateParams {
                chapter_number: 1,
                version: None,
                source: "write".to_string(),
                content: "基础正文".to_string(),
                word_count: 4,
                source_dependencies: None,
            },
            &root,
            Some(&session),
        );
        let draft_id = draft.id.expect("应返回草稿 ID");
        let source = ExpectedDraftSource {
            id: draft_id,
            chapter_number: 1,
            version: 1,
            status: "draft".to_string(),
            content: "基础正文".to_string(),
        };
        let params = reviews::ReviewCreateParams {
            base_draft_id: draft_id,
            review_index: Some(99),
            content: "审稿报告".to_string(),
            expected_source: Some(source.clone()),
        };

        // create（MUTATING）
        let created = review_create_inner(&state, &params, &root, Some(&session));
        assert!(created.success, "创建审稿应成功：{:?}", created.error);
        let review_id = created.id.expect("应返回审稿 ID");
        assert_eq!(created.review_index, Some(1), "序号必须由事务分配");

        // 源守卫失败 → 同时带 errorCode 与 error
        let mut missing_source = params.clone();
        missing_source.expected_source = None;
        let denied = review_create_inner(&state, &missing_source, &root, Some(&session));
        assert!(!denied.success);
        assert_eq!(denied.error_code.as_deref(), Some("SOURCE_DRAFT_CHANGED"));
        assert_eq!(denied.error.as_deref(), Some("Error: SOURCE_DRAFT_CHANGED"));

        // 缺会话同样走 MUTATING 包装
        let unauthenticated = review_create_inner(&state, &params, &root, None);
        assert!(!unauthenticated.success);
        assert_eq!(
            unauthenticated.error.as_deref(),
            Some("Error: 缺少项目会话上下文，已拒绝操作")
        );

        // 读频道
        let list = review_list_inner(&state, draft_id, &root, Some(&session)).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(review_next_index_inner(&state, draft_id, &root, Some(&session)).unwrap(), 2);
        let full = review_get_full_inner(&state, review_id, &root, Some(&session))
            .unwrap()
            .expect("应读到审稿");
        assert_eq!(full.content, "审稿报告");
        assert_eq!(full.source_draft.as_ref().unwrap().id, draft_id);

        // 最新审稿 = 序号最大者
        let second = review_create_inner(&state, &params, &root, Some(&session));
        let second_id = second.id.expect("应返回审稿 ID");
        assert_eq!(second.review_index, Some(2));
        let latest = review_get_latest_inner(&state, draft_id, &root, Some(&session))
            .unwrap()
            .expect("应读到最新审稿");
        assert_eq!(latest.meta.id, second_id);
        assert!(review_get_full_inner(&state, 9999, &root, Some(&session))
            .unwrap()
            .is_none());

        cleanup(&root);
    }

    #[test]
    fn post_process_channels_test() {
        let (state, root, session) = activated_state("post-process");

        // 读频道门禁
        assert_eq!(
            post_process_get_steps_inner(&state, "run", &root, None).unwrap_err(),
            "缺少项目会话上下文，已拒绝操作"
        );

        let params = post_process::PostProcessCreateParams {
            trigger_source_type: "chapter_finalize".to_string(),
            trigger_source_id: "3".to_string(),
            source_label: "第 3 章".to_string(),
            steps: vec![
                post_process::PostProcessStepInput {
                    key: "extract".to_string(),
                    label: "提取要点".to_string(),
                    critical: true,
                },
                post_process::PostProcessStepInput {
                    key: "summary".to_string(),
                    label: "生成摘要".to_string(),
                    critical: true,
                },
            ],
        };

        // 写入门禁
        let denied = post_process_create_run_inner(&state, &params, &root, None);
        assert!(!denied.success);
        assert_eq!(
            denied.error.as_deref(),
            Some("Error: 缺少项目会话上下文，已拒绝操作")
        );

        let created = post_process_create_run_inner(&state, &params, &root, Some(&session));
        assert!(created.success, "创建跑批应成功：{:?}", created.error);
        let run_id = created.id.expect("应返回 run ID");
        assert_eq!(run_id.len(), 36);

        let run = post_process_get_latest_run_inner(
            &state,
            "chapter_finalize",
            "3",
            &root,
            Some(&session),
        )
        .unwrap()
        .expect("应读到跑批");
        assert_eq!(run.id, run_id);
        assert_eq!(run.source_label, "第 3 章");
        assert!(!run.all_critical_passed);

        let steps = post_process_get_steps_inner(&state, &run_id, &root, Some(&session)).unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].step_key, "extract");

        // 初始汇总为 false
        assert!(!post_process_is_all_passed_inner(
            &state,
            "chapter_finalize",
            "3",
            &root,
            Some(&session)
        )
        .unwrap());

        // 逐步标记成功
        assert!(post_process_mark_step_ok_inner(&state, &run_id, "extract", &root, Some(&session)).success);
        assert!(!post_process_is_all_passed_inner(
            &state,
            "chapter_finalize",
            "3",
            &root,
            Some(&session)
        )
        .unwrap());
        assert!(post_process_mark_step_ok_inner(&state, &run_id, "summary", &root, Some(&session)).success);
        assert!(post_process_is_all_passed_inner(
            &state,
            "chapter_finalize",
            "3",
            &root,
            Some(&session)
        )
        .unwrap());

        // 重跑失败 → 汇总重算为 false
        assert!(post_process_mark_step_failed_inner(
            &state,
            &run_id,
            "extract",
            "提取超时",
            &root,
            Some(&session)
        )
        .success);
        assert!(!post_process_is_all_passed_inner(
            &state,
            "chapter_finalize",
            "3",
            &root,
            Some(&session)
        )
        .unwrap());

        // 步骤不存在 → MUTATING 文案
        let missing = post_process_mark_step_ok_inner(&state, &run_id, "不存在", &root, Some(&session));
        assert!(!missing.success);
        assert_eq!(
            missing.error.as_deref(),
            Some("Error: 后处理步骤不存在或已失效")
        );

        cleanup(&root);
    }

    #[test]
    fn llm_and_summary_channels_test() {
        let (state, root, session) = activated_state("llm-summary");

        // 读频道门禁
        assert_eq!(
            get_llm_stats_inner(&state, &root, None).unwrap_err(),
            "缺少项目会话上下文，已拒绝操作"
        );
        assert_eq!(
            get_llm_stats_inner(&state, "C:\\other", Some(&session)).unwrap_err(),
            "检测到跨项目读写，已拒绝操作。"
        );

        // 空库统计：token 三项为 null
        let empty = get_llm_stats_inner(&state, &root, Some(&session)).unwrap();
        assert_eq!(empty.total_calls, 0);
        assert_eq!(empty.total_tokens, None);

        // 写入门禁：缺会话 → MUTATING 包装
        let denied = log_llm_call_inner(
            &state,
            &serde_json::json!({ "modelId": "m", "success": true }),
            &root,
            None,
        );
        assert!(!denied.success);
        assert_eq!(
            denied.error.as_deref(),
            Some("Error: 缺少项目会话上下文，已拒绝操作")
        );

        // 记录两次调用
        assert!(log_llm_call_inner(
            &state,
            &serde_json::json!({
                "modelId": "deepseek-chat",
                "modelName": "DeepSeek Chat",
                "purpose": "write",
                "promptTokens": 10,
                "completionTokens": 20,
                "totalTokens": 30,
                "durationMs": 500,
                "success": true
            }),
            &root,
            Some(&session)
        )
        .success);
        assert!(log_llm_call_inner(
            &state,
            &serde_json::json!({
                "modelId": "deepseek-chat",
                "purpose": "review",
                "success": false,
                "errorMessage": "finish:content_filter"
            }),
            &root,
            Some(&session)
        )
        .success);

        let stats = get_llm_stats_inner(&state, &root, Some(&session)).unwrap();
        assert_eq!(stats.total_calls, 2);
        assert_eq!(stats.successful_calls, 1);
        assert_eq!(stats.failed_calls, 1);
        assert_eq!(stats.known_usage_calls, 1);
        assert_eq!(stats.total_tokens, Some(30));

        // 缺省 limit → 50
        let history = get_llm_history_inner(&state, None, &root, Some(&session)).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(
            history[0].finish_reason.as_deref(),
            Some("content_filter"),
            "历史按 ID 降序，首条为失败调用"
        );
        assert!(history[1].success);
        // 显式 limit 生效
        assert_eq!(
            get_llm_history_inner(&state, Some(1), &root, Some(&session))
                .unwrap()
                .len(),
            1
        );

        // 摘要快照（MUTATING）
        let saved = save_summary_snapshot_inner(&state, 2, "{\"林清玄\":{}}", &root, Some(&session));
        assert!(saved.success, "保存快照应成功：{:?}", saved.error);
        let latest = get_latest_summary_inner(&state, &root, Some(&session))
            .unwrap()
            .expect("应读到快照");
        assert_eq!(latest.chapter_number, 2);
        assert_eq!(latest.character_states, "{\"林清玄\":{}}");

        cleanup(&root);
    }

    #[test]
    fn project_clear_generated_data_channel_test() {
        let (state, root, session) = activated_state("clear");
        let options = project_clear::ProjectClearOptions {
            generated_text: Some(true),
            ..Default::default()
        };

        // 门禁：缺会话 → MUTATING 包装
        let denied = project_clear_generated_data_inner(&state, &options, &root, None);
        assert!(!denied.success);
        assert_eq!(
            denied.error.as_deref(),
            Some("Error: 缺少项目会话上下文，已拒绝操作")
        );

        // 造出待清理的数据：草稿 + 根目录成稿实体稿
        let draft = draft_create_inner(
            &state,
            &drafts::DraftCreateParams {
                chapter_number: 1,
                version: None,
                source: "write".to_string(),
                content: "正文".to_string(),
                word_count: 2,
                source_dependencies: None,
            },
            &root,
            Some(&session),
        );
        assert!(draft.success, "建草稿应成功：{:?}", draft.error);
        let finalized_file = std::path::Path::new(&root).join("第1章 起始.txt");
        std::fs::write(&finalized_file, "定稿正文").unwrap();

        let cleared = project_clear_generated_data_inner(&state, &options, &root, Some(&session));
        assert!(cleared.success, "清空应成功：{:?}", cleared.error);
        assert_eq!(
            cleared.cleared,
            Some(vec![project_clear::ProjectClearScope::GeneratedText])
        );
        assert_eq!(cleared.physical_files_deleted, Some(1));
        assert!(!finalized_file.exists(), "成稿实体稿应移出根目录");
        assert!(draft_list_all_inner(&state, &root, Some(&session))
            .unwrap()
            .is_empty());

        cleanup(&root);
    }

    // ===== 批次 F1：一致性豁免 / 叙事线索 / 剧情树 跨层测试 =====

    /// 写入一条有内容的章节蓝图（供剧情树与线索测试使用）
    fn seed_blueprint(state: &AppState, chapter: i64, title: &str, key_events: &str) {
        state
            .with_project_db(|conn| {
                conn.execute(
                    "INSERT INTO blueprints (chapter_number, title, purpose, key_events)
                     VALUES (?1, ?2, '开端', ?3)",
                    rusqlite::params![chapter, title, key_events],
                )
                .map(|_| ())
                .map_err(|error| format!("写入蓝图失败：{error}"))
            })
            .unwrap();
    }

    /// 写入一条已定稿草稿（连带 `finalization_outbox` 投影）并返回 draft_id
    fn seed_finalized(state: &AppState, chapter: i64, title: &str, body: &str) -> i64 {
        state
            .with_project_db(|conn| {
                let content_id = crate::repositories::content_repository::create(conn, body)?;
                conn.execute(
                    "INSERT INTO drafts (chapter_number, version, status, content_id, word_count)
                     VALUES (?1, 1, 'finalized', ?2, ?3)",
                    rusqlite::params![chapter, content_id, body.chars().count() as i64],
                )
                .map_err(|error| format!("写入草稿失败：{error}"))?;
                let draft_id = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO finalization_outbox (
                       finalization_id, draft_id, chapter_number, chapter_title, content_hash,
                       content_revision, content_snapshot, target_file_name, publication_status
                     ) VALUES (?1, ?2, ?3, ?4, 'hash', 1, ?5, 'f.md', 'published')",
                    rusqlite::params![format!("fin-{draft_id}"), draft_id, chapter, title, body],
                )
                .map_err(|error| format!("写入定稿投影失败：{error}"))?;
                Ok(draft_id)
            })
            .unwrap()
    }

    fn thread_plan_input() -> threads::NarrativeThreadPlanInput {
        threads::NarrativeThreadPlanInput {
            title: "失落的信物".to_string(),
            thread_type: "foreshadow".to_string(),
            target_start_chapter: 1,
            target_end_chapter: 5,
            author_intent: "主角寻回遗物".to_string(),
        }
    }

    #[test]
    fn consistency_exemption_channels_test() {
        let (state, root, session) = activated_state("consistency");

        // 读频道门禁
        assert_eq!(
            consistency_exemption_list_inner(&state, &root, None).unwrap_err(),
            "缺少项目会话上下文，已拒绝操作"
        );
        assert_eq!(
            consistency_exemption_list_inner(&state, "C:\\other", Some(&session)).unwrap_err(),
            "检测到跨项目读写，已拒绝操作。"
        );

        // 空列表
        assert!(consistency_exemption_list_inner(&state, &root, Some(&session))
            .unwrap()
            .is_empty());

        // 保存 → 信封成功
        let saved = consistency_exemption_save_inner(
            &state,
            "fact:abc",
            "作者已确认",
            &root,
            Some(&session),
        );
        assert!(saved.success);
        assert_eq!(saved.error, None);

        // 保存校验失败 → MUTATING 文案
        let invalid = consistency_exemption_save_inner(&state, "  ", "原因", &root, Some(&session));
        assert!(!invalid.success);
        assert_eq!(invalid.error.as_deref(), Some("Error: 稳定事实键无效"));

        // 保存门禁失败也走 MUTATING 包装
        let denied = consistency_exemption_save_inner(&state, "fact:x", "原因", &root, None);
        assert!(!denied.success);
        assert_eq!(
            denied.error.as_deref(),
            Some("Error: 缺少项目会话上下文，已拒绝操作")
        );

        // 撤销 → 软删除（仍在列表中，revoked = true）
        let revoked = consistency_exemption_revoke_inner(&state, "fact:abc", &root, Some(&session));
        assert!(revoked.success);
        let items = consistency_exemption_list_inner(&state, &root, Some(&session)).unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].revoked);
        assert_eq!(items[0].stable_fact_key, "fact:abc");

        cleanup(&root);
    }

    #[test]
    fn narrative_thread_plan_channels_test() {
        let (state, root, session) = activated_state("thread-plan");

        // 空列表
        assert!(narrative_thread_list_inner(&state, &root, Some(&session))
            .unwrap()
            .is_empty());

        // 创建 → 成功信封（含 plan）
        let created = narrative_thread_plan_create_inner(
            &state,
            &thread_plan_input(),
            &root,
            Some(&session),
        )
        .unwrap();
        assert!(created.success);
        assert_eq!(created.error, None);
        let plan = created.plan.expect("应返回计划");
        assert_eq!(plan.thread_type, "foreshadow");

        // 参数非法 → **reject**（基线无 try/catch，故不填信封）
        assert_eq!(
            narrative_thread_plan_create_inner(
                &state,
                &threads::NarrativeThreadPlanInput {
                    title: "  ".to_string(),
                    ..thread_plan_input()
                },
                &root,
                Some(&session),
            )
            .unwrap_err(),
            "叙事线索计划参数无效"
        );

        // 更新 → 成功信封
        let updated = narrative_thread_plan_update_inner(
            &state,
            plan.id,
            &threads::NarrativeThreadPlanInput {
                title: "新标题".to_string(),
                ..thread_plan_input()
            },
            &root,
            Some(&session),
        )
        .unwrap();
        assert!(updated.success);
        assert_eq!(updated.plan.unwrap().title, "新标题");

        // 更新不存在的计划 → reject
        assert_eq!(
            narrative_thread_plan_update_inner(&state, 404, &thread_plan_input(), &root, Some(&session))
                .unwrap_err(),
            "叙事线索计划不存在"
        );

        // 列表投影：无事件 → planned
        let views = narrative_thread_list_inner(&state, &root, Some(&session)).unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].status, "planned");

        // 相关线索（章号在区间内）
        let relevant = narrative_thread_list_relevant_inner(
            &state,
            &threads::NarrativeThreadChapterContext {
                chapter_number: 3,
                title: "第三章".to_string(),
                key_events: String::new(),
                characters: vec![],
            },
            &root,
            Some(&session),
        )
        .unwrap();
        assert_eq!(relevant.len(), 1);

        // 删除 → 成功信封；再删 → reject
        assert!(narrative_thread_plan_delete_inner(&state, plan.id, &root, Some(&session))
            .unwrap()
            .success);
        assert_eq!(
            narrative_thread_plan_delete_inner(&state, plan.id, &root, Some(&session)).unwrap_err(),
            "叙事线索计划不存在"
        );

        cleanup(&root);
    }

    #[test]
    fn narrative_thread_event_channel_test() {
        let (state, root, session) = activated_state("thread-event");
        let plan = narrative_thread_plan_create_inner(
            &state,
            &thread_plan_input(),
            &root,
            Some(&session),
        )
        .unwrap()
        .plan
        .unwrap();
        let draft_id = seed_finalized(&state, 2, "第二章", "她握紧了那枚青铜钥匙。");

        // 证据不在定稿正文中 → reject（前端据此提示「请粘贴短原文」）
        assert_eq!(
            narrative_thread_event_confirm_inner(
                &state,
                &threads::NarrativeThreadEventInput {
                    plan_id: plan.id,
                    draft_id,
                    event_type: "planted".to_string(),
                    evidence: "银色戒指".to_string(),
                    reason: "理由".to_string(),
                },
                &root,
                Some(&session),
            )
            .unwrap_err(),
            "短证据必须来自绑定的定稿正文"
        );

        // 正常确认 → 成功信封
        let confirmed = narrative_thread_event_confirm_inner(
            &state,
            &threads::NarrativeThreadEventInput {
                plan_id: plan.id,
                draft_id,
                event_type: "planted".to_string(),
                evidence: "那枚青铜钥匙".to_string(),
                reason: "首次出现".to_string(),
            },
            &root,
            Some(&session),
        )
        .unwrap();
        assert!(confirmed.success);
        let event = confirmed.event.expect("应返回事件");
        assert_eq!(event.chapter_number, 2);
        assert_eq!(event.chapter_title, "第二章");

        // 绑定不存在的草稿 → reject
        assert_eq!(
            narrative_thread_event_confirm_inner(
                &state,
                &threads::NarrativeThreadEventInput {
                    plan_id: plan.id,
                    draft_id: 9999,
                    event_type: "planted".to_string(),
                    evidence: "x".to_string(),
                    reason: "y".to_string(),
                },
                &root,
                Some(&session),
            )
            .unwrap_err(),
            "线索事件只能绑定已定稿章节"
        );

        // 列表投影：状态派生为 planted
        let views = narrative_thread_list_inner(&state, &root, Some(&session)).unwrap();
        assert_eq!(views[0].status, "planted");
        assert_eq!(views[0].events.len(), 1);

        cleanup(&root);
    }

    #[test]
    fn plot_tree_channels_test() {
        let (state, root, session) = activated_state("plot-tree");
        seed_blueprint(&state, 1, "第一章", "开端事件");

        // 读频道门禁
        assert_eq!(
            plot_tree_read_inner(&state, &root, None).unwrap_err(),
            "缺少项目会话上下文，已拒绝操作"
        );

        let bundle = plot_tree_read_inner(&state, &root, Some(&session)).unwrap();
        assert!(bundle.snapshot.is_none());
        assert_eq!(bundle.stored_snapshot_invalid, None);
        assert_eq!(bundle.facts.blueprints.len(), 1);
        assert_eq!(bundle.source_revision.len(), 64);
        let revision = bundle.source_revision.clone();

        // 版本形状非法 → 信封失败，**不**带 errorCode
        let bad_shape = plot_tree_save_inner(
            &state,
            &serde_json::json!({}),
            "not-a-hash",
            &root,
            Some(&session),
        );
        assert!(!bad_shape.success);
        assert_eq!(bad_shape.error_code, None);
        assert_eq!(bad_shape.error.as_deref(), Some("Error: 剧情树来源版本无效"));

        // 版本过期 → errorCode = sources-changed
        let stale = plot_tree_save_inner(
            &state,
            &serde_json::json!({}),
            &"a".repeat(64),
            &root,
            Some(&session),
        );
        assert!(!stale.success);
        assert_eq!(stale.error_code.as_deref(), Some("sources-changed"));
        assert_eq!(
            stale.error.as_deref(),
            Some("Error: 剧情资料在生成期间已更新")
        );

        // 结构非法（空轨道）→ 信封失败，无 errorCode
        let invalid = plot_tree_save_inner(
            &state,
            &serde_json::json!({ "version": 1 }),
            &revision,
            &root,
            Some(&session),
        );
        assert!(!invalid.success);
        assert_eq!(invalid.error_code, None);
        assert_eq!(
            invalid.error.as_deref(),
            Some("Error: 剧情树快照版本或写作语言无效")
        );

        // 合法快照 → 成功信封，且能回读
        let snapshot = serde_json::json!({
            "version": 1,
            "generatedAt": "2026-10-08T10:00:00.000Z",
            "writingLanguage": "zh-CN",
            "sourceRevision": revision,
            "tracks": [{
                "id": "main-1",
                "title": "主线",
                "role": "main",
                "startChapter": 1,
                "endChapter": 1,
                "summary": "主线摘要",
                "events": [{
                    "status": "planned",
                    "chapterNumber": 1,
                    "summary": "规划",
                    "sources": [{ "type": "blueprint", "chapterNumber": 1 }]
                }]
            }]
        });
        let saved = plot_tree_save_inner(&state, &snapshot, &revision, &root, Some(&session));
        assert!(saved.success, "保存应成功：{:?}", saved.error);
        assert_eq!(saved.snapshot.as_ref().unwrap().tracks.len(), 1);

        let reread = plot_tree_read_inner(&state, &root, Some(&session)).unwrap();
        assert!(reread.snapshot.is_some(), "保存后应可回读快照");
        assert_eq!(reread.stored_snapshot_invalid, None);

        // 清除 → 只清缓存列
        assert!(plot_tree_clear_inner(&state, &root, Some(&session))
            .unwrap()
            .success);
        let cleared = plot_tree_read_inner(&state, &root, Some(&session)).unwrap();
        assert!(cleared.snapshot.is_none());
        assert_eq!(cleared.facts.blueprints.len(), 1, "来源事实不受清快照影响");

        cleanup(&root);
    }

    #[test]
    fn recovery_candidate_command_guard_and_roundtrip_test() {
        let (state, root, session) = activated_state("recovery");

        // 门禁：缺会话 → 读频道拒绝（裸文案），写频道返回带前缀信封
        assert_eq!(
            recovery_candidate_list_inner(&state, &root, None).unwrap_err(),
            "缺少项目会话上下文，已拒绝操作"
        );
        let req = serde_json::from_value::<recovery::RecoveryCandidateRecordRequest>(serde_json::json!({
            "runId": "run-1",
            "stepId": "step-1",
            "chapterNumber": 1,
            "chapterTitle": "第一章",
            "source": {
                "chapterNumber": 1,
                "title": "标题",
                "role": "main",
                "purpose": "推进",
                "keyEvents": "事件A",
                "characters": ["甲"]
            },
            "sourceDraft": null,
            "visibleText": "<think>思考</think>正文",
            "failureCode": "TIMEOUT",
            "failureReason": "超时"
        }))
        .unwrap();
        let blocked = recovery_candidate_record_inner(&state, &req, &root, None);
        assert!(!blocked.success);
        assert_eq!(
            blocked.error.as_deref(),
            Some("Error: 缺少项目会话上下文，已拒绝操作")
        );

        // record：projectId 由租约注入，正文剥除思考链
        let recorded = recovery_candidate_record_inner(&state, &req, &root, Some(&session));
        assert!(recorded.success, "record 应成功：{:?}", recorded.error);
        let candidate = recorded.candidate.as_ref().unwrap();
        assert_eq!(candidate.visible_text, "正文");
        assert_eq!(candidate.project_id, session.project_id, "projectId 注入自租约");

        // list → update → resolve 全链路
        let listed = recovery_candidate_list_inner(&state, &root, Some(&session)).unwrap();
        assert_eq!(listed.len(), 1);

        // update 的时效校验要求蓝图存在且与 source 一致
        state
            .with_project_db(|conn| {
                blueprints::upsert(
                    conn,
                    &blueprints::BlueprintData {
                        chapter_number: 1,
                        title: "标题".to_string(),
                        role: "main".to_string(),
                        purpose: "推进".to_string(),
                        key_events: "事件A".to_string(),
                        characters: vec!["甲".to_string()],
                        new_character_candidates: None,
                        relationship_hints: None,
                        suspense_hook: String::new(),
                        user_guidance: String::new(),
                        notes: String::new(),
                        notes_updated_at: String::new(),
                    },
                )
            })
            .unwrap();

        let updated = recovery_candidate_update_inner(
            &state,
            &candidate.candidate_id,
            "修订正文",
            &root,
            Some(&session),
        );
        assert!(updated.success, "update 应成功：{:?}", updated.error);
        assert_eq!(updated.candidate.as_ref().unwrap().visible_text, "修订正文");

        // resolve：无效动作 → 带前缀信封；然后 discarded 成功
        let invalid = recovery_candidate_resolve_inner(
            &state,
            &candidate.candidate_id,
            "nope",
            &root,
            Some(&session),
        );
        assert!(!invalid.success);
        assert_eq!(invalid.error.as_deref(), Some("Error: 恢复候选动作无效"));

        let resolved = recovery_candidate_resolve_inner(
            &state,
            &candidate.candidate_id,
            "discarded",
            &root,
            Some(&session),
        );
        assert!(resolved.success, "resolve 应成功：{:?}", resolved.error);
        let listed = recovery_candidate_list_inner(&state, &root, Some(&session)).unwrap();
        assert!(listed.is_empty(), "终态后不再出现在待处理列表");

        cleanup(&root);
    }
}