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
use crate::repositories::project_core_repository as project_core;
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
}