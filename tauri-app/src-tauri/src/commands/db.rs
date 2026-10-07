//! 批次 C：项目数据库命令 —— 平移自 `electron/controllers/db-controller.ts`。
//!
//! 门禁语义与基线一致（`registerProjectDatabaseHandler`）：
//! 1. 尾参 `projectSession`（渲染层自动注入）必须通过真实租约校验；
//! 2. `expectedProjectPath` 必须与当前已打开项目一致（路径从不单独构成授权）；
//! 3. 写频道失败返回 `{ success: false, error }`，读频道失败直接拒绝（invoke reject）。
//!
//! 本批次落地的子域：`db:close` + `project_core`（get / update / synopsis-commit）。
//! 其余 `db:*` 子域（blueprint / draft / revision / import-run / llm 等）按批次 C 后续
//! 子域逐个迁移。

use tauri::State;

use crate::commands::SimpleResult;
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
            error: Some(error),
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
                error: Some(error),
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
    let outcome = guard_read(state, expected_project_path, session).and_then(|()| {
        state.with_project_db(|conn| project_core::commit_synopsis(conn, request))?
            .then_some(())
            .ok_or_else(|| project_core::SYNOPSIS_CONFLICT_MESSAGE.to_string())
    });
    match outcome {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => SimpleResult {
            success: false,
            error: Some(error),
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
        assert_eq!(
            result.error.as_deref(),
            Some(project_core::CHARACTERS_ARCH_READONLY_MESSAGE)
        );

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
        assert_eq!(result.error.as_deref(), Some("项目租约已失效，已拒绝操作。"));
    }
}