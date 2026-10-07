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

use crate::commands::SimpleResult;
use crate::repositories::blueprint_repository as blueprints;
use crate::repositories::character_repository as characters;
use crate::repositories::character_roster_repository as roster;
use crate::repositories::draft_repository as drafts;
use crate::repositories::llm_repository as llm;
use crate::repositories::post_process_repository as post_process;
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
}