//! 批次 C：项目生命周期命令 —— 迁移自 `electron/controllers/project-controller.ts`。
//!
//! 批次 C 已真实化（批次 B 的骨架占位全部落地）：
//! - `project:create` / `project:open`：探测或创建项目根 → 打开项目数据库 → 签发会话租约；
//! - `project:save` / `project:update-config`：经会话租约门禁写入 `project_core`；
//! - `project:delete`：会话 + 项目身份授权后关闭数据库并删除项目目录；
//! - `project:get-runtime-context` / `recent-*` / `smoke-*` 保持真实实现。
//!
//! 与基线的差异（迁移遗留，见进度快照）：
//! - 基线用「回滚边界快照 + 串行队列」处理并发打开；Rust 侧以请求令牌新鲜度检查 +
//!   单活跃项目状态替代，不做旧项目自动还原；
//! - 目录删除为 Rust 侧重试实现（对齐 `removeDirectoryWithWindowsRetry` 的退避语义）；
//! - `dialog:select-folder` 仍待 `tauri-plugin-dialog`（Ask first）。

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::SimpleResult;
use crate::project_access::{self, PROJECT_ROOT_REQUIRED_CODE, PROJECT_ROOT_REQUIRED_MESSAGE};
use crate::repositories::project_core_repository as project_core;
use crate::security::{
    assert_current_project_context, assert_required_expected_project_path, guard_message,
    normalized_project_path, ProjectSessionContext,
};
use crate::state::{ActiveProject, AppState};

/// 迁移自 project-controller.ts 内部结构 `RecentProject` + 契约返回类型。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub name: String,
    pub path: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeContext {
    pub active_project_path: Option<String>,
    pub db_ready: bool,
}

/// LOREKEEPER_HOME —— Tauri 侧的全局数据根，**刻意与 Electron 基线隔离**。
///
/// 基线读 `AI_NOVEL_VELA_HOME` 并回退 `~/.vela`。两个应用可以同时在同一台机器上
/// 运行，所以这里既不读旧环境变量、也不回退到旧目录，否则 `config.json` /
/// `models.json` / `recent-projects.json` / `prompts/` 会被两个进程同时读写。
pub fn lorekeeper_home() -> std::path::PathBuf {
    if let Ok(home) = std::env::var("AI_NOVEL_LOREKEEPER_HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return std::path::PathBuf::from(trimmed);
        }
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    std::path::PathBuf::from(home.trim()).join(".lorekeeper")
}

fn recent_projects_path() -> std::path::PathBuf {
    lorekeeper_home().join("recent-projects.json")
}

/// 读最近项目列表：缺失或损坏返回空列表（对齐 `readJsonFile(path, [])`）。
pub fn load_recent_projects() -> Vec<RecentProject> {
    let Ok(content) = std::fs::read_to_string(recent_projects_path()) else {
        return Vec::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

fn write_recent_projects(list: &[RecentProject]) -> Result<(), String> {
    let value = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(recent_projects_path(), value).map_err(|e| e.to_string())
}

/// 最近项目路径一致性：词法归一比较。基线为 canonical root 优先 + 词法键
/// 回退（`sameRecentProjectPath`）；骨架阶段导航元数据不触 realpath，
/// TODO(批次 C)：接 ProjectAccess canonical root 优先。
fn same_recent_project_path(left: &str, right: &str) -> bool {
    normalized_project_path(left) == normalized_project_path(right)
}

// ===== project:get-runtime-context =====

#[tauri::command]
pub fn project_get_runtime_context(state: State<'_, AppState>) -> RuntimeContext {
    let active_project_path = state.current_project_path();
    let database_open = state.project_database_open();
    // 对齐基线：无项目时必须无数据库（中立态也算就绪），有项目时必须有数据库。
    let db_ready = if active_project_path.is_none() {
        !database_open
    } else {
        database_open
    };
    RuntimeContext {
        active_project_path,
        db_ready,
    }
}

// ===== project:recent-list / project:recent-remove =====

#[tauri::command]
pub fn project_recent_list() -> Vec<RecentProject> {
    load_recent_projects()
}

#[tauri::command]
pub fn project_recent_remove(project_path: String) -> SimpleResult {
    let before = load_recent_projects();
    let filtered: Vec<RecentProject> = before
        .iter()
        .filter(|p| !same_recent_project_path(&p.path, &project_path))
        .cloned()
        .collect();
    if filtered.len() == before.len() {
        // 无匹配项也视为成功（幂等删除）。
        return SimpleResult { success: true, error: None };
    }
    match write_recent_projects(&filtered) {
        Ok(()) => SimpleResult { success: true, error: None },
        Err(err) => SimpleResult { success: false, error: Some(err) },
    }
}

// ===== project:smoke-open-request / confirm =====

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmokeOpenRequest {
    pub project_path: String,
    pub marker_path: String,
}

#[tauri::command]
pub fn project_smoke_open_request() -> Option<SmokeOpenRequest> {
    let project_path = std::env::var("AI_NOVEL_SMOKE_OPEN_PROJECT")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let marker_path = std::env::var("AI_NOVEL_SMOKE_PROJECT_MARKER")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    match (project_path, marker_path) {
        (Some(project_path), Some(marker_path)) => Some(SmokeOpenRequest { project_path, marker_path }),
        _ => None,
    }
}

#[tauri::command]
pub fn project_smoke_open_confirm(state: State<'_, AppState>, project_path: String) -> SimpleResult {
    let requested = std::env::var("AI_NOVEL_SMOKE_OPEN_PROJECT")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let marker_path = std::env::var("AI_NOVEL_SMOKE_PROJECT_MARKER")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let active: Option<String> = state.current_project_path();
    let matched = match (&requested, &marker_path, &active) {
        (Some(requested), Some(_), Some(current)) => {
            normalized_project_path(&project_path) == normalized_project_path(requested)
                && normalized_project_path(current) == normalized_project_path(requested)
        }
        _ => false,
    };
    if !matched {
        return SimpleResult {
            success: false,
            error: Some("烟测项目未在应用中成功打开".to_string()),
        };
    }
    let Some(marker_path_for_write) = marker_path else {
        return SimpleResult {
            success: false,
            error: Some("烟测项目未在应用中成功打开".to_string()),
        };
    };
    let receipt = serde_json::json!({
        "projectPath": active.unwrap_or_default(),
        "openedAt": iso8601_utc_from_millis(epoch_millis_now()),
    });
    match serde_json::to_string(&receipt)
        .map_err(|e| e.to_string())
        .and_then(|body| std::fs::write(&marker_path_for_write, body).map_err(|e| e.to_string()))
    {
        Ok(()) => SimpleResult { success: true, error: None },
        Err(err) => SimpleResult { success: false, error: Some(err) },
    }
}

/// 当前 UNIX 毫秒（系统时钟早于 epoch 时退回 0）。
pub fn epoch_millis_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 毫秒时间戳 → ISO 8601 UTC（`1970-01-01T00:00:00.000Z` 形态），
/// 对齐基线 `new Date().toISOString()` 的烟测回执格式（不引入 chrono 依赖，
/// 采用 Howard Hinnant civil_from_days 算法）。
pub fn iso8601_utc_from_millis(millis: u64) -> String {
    let secs = (millis / 1000) as i64;
    let ms = millis % 1000;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, hour, minute, second, ms
    )
}

// ===== dialog:select-folder =====

/// `dialog:select-folder` 骨架：返回 null（取消语义），等 tauri-plugin-dialog
/// 接入后返回真实目录路径（Ask first 决策点，见进度快照批次 B 章节）。
#[tauri::command]
pub fn dialog_select_folder() -> Option<String> {
    None
}

// ===== 骨架占位（批次 C 数据库层接入后启用）=====

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCreateResult {
    pub success: bool,
    pub project_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    pub request_token: String,
    pub active_project_path: Option<String>,
    pub database_restored: bool,
    pub db_ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOpenResult {
    pub success: bool,
    pub project: Option<serde_json::Value>,
    pub request_token: String,
    pub active_project_path: Option<String>,
    pub database_restored: bool,
    pub db_ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDeleteResult {
    pub success: bool,
    pub directory_deleted: bool,
    pub database_restored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

// ===== 项目身份 / 最近项目 / 数据库状态辅助 =====

/// 记录最近项目（去重后置顶，最多 20 条）—— 对齐基线 `addRecentProject`
fn add_recent_project(project: RecentProject) -> Result<(), String> {
    let mut list: Vec<RecentProject> = load_recent_projects()
        .into_iter()
        .filter(|item| !same_recent_project_path(&item.path, &project.path))
        .collect();
    list.insert(0, project);
    list.truncate(20);
    write_recent_projects(&list)
}

fn remove_recent_project(project_path: &str) -> Result<(), String> {
    let list: Vec<RecentProject> = load_recent_projects()
        .into_iter()
        .filter(|item| !same_recent_project_path(&item.path, project_path))
        .collect();
    write_recent_projects(&list)
}

fn now_iso8601() -> String {
    iso8601_utc_from_millis(epoch_millis_now())
}

/// 失败后的回退状态：无项目 → 中立态；有项目 → 按当前库状态（对齐基线
/// `neutralDatabaseState` / `databaseStateFor` 的组合语义）。
fn fallback_database_state(state: &AppState) -> crate::state::ProjectDatabaseState {
    match state.current_project_path() {
        None => state.neutral_database_state(),
        Some(path) => state.database_state(Some(&path)),
    }
}

/// 写频道会话门禁：租约一致 + 冻结路径与活跃项目一致
fn assert_write_session(
    state: &AppState,
    session: &ProjectSessionContext,
    expected_project_path: &str,
) -> Result<ActiveProject, String> {
    let active = state.active_project_snapshot();
    assert_current_project_context(session, active.as_ref()).map_err(guard_message)?;
    assert_required_expected_project_path(
        Some(session.project_path.as_str()),
        Some(expected_project_path),
    )
    .map_err(guard_message)?;
    active.ok_or_else(|| guard_message(crate::security::GuardKind::LeaseInvalid))
}

/// 项目身份校验 —— 对齐基线 `assertProjectIdentity`
fn assert_project_identity(
    project_id: &str,
    data: &serde_json::Value,
    session: &ProjectSessionContext,
) -> Result<(), String> {
    let mismatch = "项目身份不匹配，已拒绝操作";
    if project_id.trim().is_empty() || session.project_id != project_id {
        return Err(mismatch.to_string());
    }
    if let Some(data_id) = data.get("id").and_then(|item| item.as_str()) {
        if !data_id.is_empty() && data_id != project_id {
            return Err(mismatch.to_string());
        }
    }
    if let Some(data_lease) = data.get("sessionLease").and_then(|item| item.as_str()) {
        if !data_lease.is_empty() && data_lease != session.lease_id {
            return Err(mismatch.to_string());
        }
    }
    Ok(())
}

/// `novelConfig` → `project_core` 列映射（字段清单对齐基线 `project:save`）
fn novel_config_core_update(
    novel_config: &serde_json::Value,
    with_writing_language: bool,
) -> serde_json::Map<String, serde_json::Value> {
    let mut update = serde_json::Map::new();

    if with_writing_language {
        if let Some(value) = novel_config.get("writingLanguage").and_then(|item| item.as_str()) {
            update.insert(
                "writingLanguage".to_string(),
                serde_json::json!(project_core::resolve_writing_language(value)),
            );
        }
    }

    for (source_key, target_key) in [
        ("genre", "genre"),
        ("subGenre", "subGenre"),
        ("targetAudience", "targetAudience"),
        ("totalChapters", "totalChapters"),
        ("wordsPerChapter", "wordsPerChapter"),
        ("plotStructure", "plotStructure"),
        ("narrativePOV", "narrativePov"),
        ("goldenFinger", "goldenFinger"),
        ("globalGuidance", "globalGuidance"),
        ("coreOutline", "coreOutline"),
        ("worldSetting", "worldSetting"),
        ("protagonistProfile", "protagonistProfile"),
    ] {
        if let Some(value) = novel_config.get(source_key) {
            if !value.is_null() {
                update.insert(target_key.to_string(), value.clone());
            }
        }
    }

    // 基线：`writingStyle` / `referenceWorks` 缺失时写空串
    for key in ["writingStyle", "referenceWorks"] {
        let value = novel_config.get(key).cloned();
        let normalized = match value {
            Some(value) if !value.is_null() => value,
            _ => serde_json::json!(""),
        };
        update.insert(key.to_string(), normalized);
    }

    if let Some(value) = novel_config.get("creativeStrategy").and_then(|item| item.as_str()) {
        if !value.is_empty() {
            update.insert(
                "creativeStrategy".to_string(),
                serde_json::json!(project_core::resolve_creative_strategy(value)),
            );
        }
    }

    if let Some(value) = novel_config.get("narrativeThreadDormantChapterThreshold") {
        if !value.is_null() {
            update.insert(
                "narrativeThreadDormantChapterThreshold".to_string(),
                value.clone(),
            );
        }
    }

    update
}

/// `project_core` → 渲染层 `ProjectData`（`sessionLease` 由调用方注入）
fn project_core_to_project_json(
    core: &project_core::ProjectCoreData,
    project_id: &str,
    root_path: &str,
) -> serde_json::Value {
    let timestamp = now_iso8601();
    serde_json::json!({
        "id": project_id,
        "name": core.project_name,
        "path": root_path,
        "novelConfig": {
            "writingLanguage": core.writing_language,
            "genre": core.genre,
            "subGenre": core.sub_genre,
            "targetAudience": core.target_audience,
            "totalChapters": core.total_chapters,
            "wordsPerChapter": core.words_per_chapter,
            "creativeStrategy": core.creative_strategy,
            "narrativeThreadDormantChapterThreshold": core.narrative_thread_dormant_chapter_threshold,
            "plotStructure": core.plot_structure,
            "narrativePOV": core.narrative_pov,
            "coreOutline": core.core_outline,
            "worldSetting": core.world_setting,
            "goldenFinger": core.golden_finger,
            "protagonistProfile": core.protagonist_profile,
            "globalGuidance": core.global_guidance,
            "writingStyle": core.writing_style,
            "referenceWorks": core.reference_works,
        },
        "characterStates": core.character_states,
        "createdAt": timestamp,
        "updatedAt": timestamp,
    })
}

/// 打开项目内部流程：探测/列表收养 → 打开数据库 → 确保主台账 → 记录最近项目
#[allow(clippy::type_complexity)]
fn open_project_inner(
    state: &AppState,
    candidate_path: &str,
) -> Result<(serde_json::Value, ActiveProject), (String, Option<String>)> {
    let home = project_access::home_directory();
    let probe = project_access::probe_existing_project(candidate_path, home.as_deref()).map_err(
        |error| {
            let code = if error == PROJECT_ROOT_REQUIRED_MESSAGE {
                Some(PROJECT_ROOT_REQUIRED_CODE.to_string())
            } else {
                None
            };
            (error, code)
        },
    )?;
    let trusted = project_access::adopt_legacy_project(probe, home.as_deref())
        .map_err(|error| (error, None))?;
    let lease = ActiveProject {
        project_id: trusted.project_id.clone(),
        lease_id: project_access::random_uuid_v4(),
        root_path: trusted.root_path.clone(),
    };

    state
        .activate_project(lease.clone())
        .map_err(|error| (format!("项目数据库初始化失败：{error}"), None))?;

    let project_name = std::path::Path::new(&trusted.root_path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "未命名项目".to_string());

    let loaded = state.with_project_db(|conn| {
        if project_core::get(conn)?.is_none() {
            // 基线：从空目录新建并打开时按默认写作语言初始化
            project_core::init(conn, &project_name, project_core::DEFAULT_WRITING_LANGUAGE)?;
        }
        project_core::get(conn)
    });
    let core = match loaded {
        Ok(Some(core)) => core,
        Ok(None) => {
            state.invalidate_current_session();
            return Err(("项目主台账初始化失败".to_string(), None));
        }
        Err(error) => {
            state.invalidate_current_session();
            return Err((format!("项目数据库读取失败：{error}"), None));
        }
    };

    let _ = add_recent_project(RecentProject {
        name: core.project_name.clone(),
        path: trusted.root_path.clone(),
        updated_at: now_iso8601(),
    });

    Ok((
        project_core_to_project_json(&core, &trusted.project_id, &trusted.root_path),
        lease,
    ))
}

fn open_failure(token: &str, state: &AppState, error: String, error_code: Option<String>) -> ProjectOpenResult {
    let database_state = fallback_database_state(state);
    ProjectOpenResult {
        success: false,
        project: None,
        request_token: token.to_string(),
        active_project_path: database_state.active_project_path,
        database_restored: database_state.database_restored,
        db_ready: database_state.db_ready,
        stale: None,
        error: Some(error),
        error_code,
    }
}

fn create_failure(
    token: &str,
    state: &AppState,
    error: String,
    error_code: Option<String>,
) -> ProjectCreateResult {
    let database_state = fallback_database_state(state);
    ProjectCreateResult {
        success: false,
        project_id: String::new(),
        project_path: None,
        request_token: token.to_string(),
        active_project_path: database_state.active_project_path,
        database_restored: database_state.database_restored,
        db_ready: database_state.db_ready,
        stale: None,
        error: Some(error),
        error_code,
    }
}

fn delete_failure(database_restored: bool, error: String, warning: Option<String>) -> ProjectDeleteResult {
    ProjectDeleteResult {
        success: false,
        directory_deleted: false,
        database_restored,
        error: Some(error),
        warning,
    }
}

/// 目录删除（对齐 `removeDirectoryWithWindowsRetry` 的退避重试语义，简化版）
fn remove_directory_with_retry(path: &std::path::Path) -> std::io::Result<()> {
    let mut last_error = None;
    for attempt in 0..5u64 {
        match std::fs::remove_dir_all(path) {
            Ok(()) => {
                if !path.exists() {
                    return Ok(());
                }
            }
            Err(error) => last_error = Some(error),
        }
        std::thread::sleep(std::time::Duration::from_millis(100 * (attempt + 1)));
    }
    if !path.exists() {
        return Ok(());
    }
    Err(last_error.unwrap_or_else(|| std::io::Error::other("项目目录删除失败")))
}

// ===== project:create =====

/// `project:create` —— 创建项目根 + 初始化项目主台账 + 写入最近项目
#[tauri::command]
pub fn project_create(
    state: State<'_, AppState>,
    config: serde_json::Value,
    request_token: String,
    renderer_project_path: Option<String>,
) -> ProjectCreateResult {
    let _ = renderer_project_path;
    let app = state.inner();
    if let Err(error) = app.set_latest_open_token(&request_token) {
        return create_failure(&request_token, app, error, None);
    }

    let parent_path = config
        .get("path")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    if parent_path.trim().is_empty() {
        return create_failure(&request_token, app, "项目目录不能为空".to_string(), None);
    }
    let display_name = project_access::sanitize_project_name(
        config.get("name").and_then(|item| item.as_str()).unwrap_or_default(),
    );
    let home = project_access::home_directory();

    let trusted = match project_access::create_project(
        &parent_path,
        &display_name,
        home.as_deref(),
    ) {
        Ok(trusted) => trusted,
        Err(error) => {
            let error_code = if error == PROJECT_ROOT_REQUIRED_MESSAGE {
                Some(PROJECT_ROOT_REQUIRED_CODE.to_string())
            } else {
                None
            };
            return create_failure(&request_token, app, error, error_code);
        }
    };
    if !app.is_latest_open_token(&request_token) {
        return ProjectCreateResult {
            success: false,
            project_id: trusted.project_id.clone(),
            project_path: Some(trusted.root_path.clone()),
            request_token,
            active_project_path: app.current_project_path(),
            database_restored: false,
            db_ready: false,
            stale: Some(true),
            error: Some("项目创建请求已被更新的请求取代".to_string()),
            error_code: None,
        };
    }

    // 项目提示词目录（对齐基线 `ensureProjectPromptDirectories`）
    let _ = std::fs::create_dir_all(
        std::path::Path::new(&trusted.root_path).join(project_access::DIR_PROMPTS_RELATIVE),
    );

    let lease = ActiveProject {
        project_id: trusted.project_id.clone(),
        lease_id: project_access::random_uuid_v4(),
        root_path: trusted.root_path.clone(),
    };
    if let Err(error) = app.activate_project(lease) {
        app.invalidate_current_session();
        return create_failure(
            &request_token,
            app,
            format!("项目数据库初始化失败：{error}"),
            None,
        );
    }

    // 基线 create：init 时即写入已收敛的写作语言（缺失回落默认）
    let writing_language = config
        .get("writingLanguage")
        .and_then(|item| item.as_str())
        .map(project_core::resolve_writing_language)
        .unwrap_or_else(|| project_core::DEFAULT_WRITING_LANGUAGE.to_string());

    let initialized = app.with_project_db(|conn| {
        project_core::init(conn, &display_name, &writing_language)?;
        let mut update = serde_json::Map::new();
        if let Some(genre) = config.get("genre") {
            if !genre.is_null() {
                update.insert("genre".to_string(), genre.clone());
            }
        }
        if let Some(audience) = config.get("targetAudience") {
            if !audience.is_null() {
                update.insert("targetAudience".to_string(), audience.clone());
            }
        }
        project_core::update(conn, &update)
    });
    if let Err(error) = initialized {
        app.invalidate_current_session();
        return create_failure(&request_token, app, error, None);
    }

    // 与基线一致：创建只落盘，不保留新库打开（渲染层随后调用 `project:open`）
    app.invalidate_current_session();
    let _ = add_recent_project(RecentProject {
        name: display_name,
        path: trusted.root_path.clone(),
        updated_at: now_iso8601(),
    });
    let database_state = app.neutral_database_state();

    ProjectCreateResult {
        success: true,
        project_id: trusted.project_id,
        project_path: Some(trusted.root_path),
        request_token,
        active_project_path: database_state.active_project_path,
        database_restored: database_state.database_restored,
        db_ready: database_state.db_ready,
        stale: None,
        error: None,
        error_code: None,
    }
}

// ===== project:open =====

#[tauri::command]
pub fn project_open(
    state: State<'_, AppState>,
    project_path: String,
    request_token: String,
    renderer_project_path: Option<String>,
) -> ProjectOpenResult {
    let _ = renderer_project_path;
    let app = state.inner();
    if let Err(error) = app.set_latest_open_token(&request_token) {
        return open_failure(&request_token, app, error, None);
    }

    match open_project_inner(app, &project_path) {
        Ok((mut project, lease)) => {
            if !app.is_latest_open_token(&request_token) {
                return ProjectOpenResult {
                    success: false,
                    project: None,
                    request_token,
                    active_project_path: app.current_project_path(),
                    database_restored: false,
                    db_ready: false,
                    stale: Some(true),
                    error: Some("项目打开请求已被更新的请求取代".to_string()),
                    error_code: None,
                };
            }
            project["sessionLease"] = serde_json::json!(lease.lease_id);
            let database_state = app.database_state(Some(&lease.root_path));
            ProjectOpenResult {
                success: true,
                project: Some(project),
                request_token,
                active_project_path: database_state.active_project_path,
                database_restored: database_state.database_restored,
                db_ready: database_state.db_ready,
                stale: None,
                error: None,
                error_code: None,
            }
        }
        Err((error, error_code)) => open_failure(&request_token, app, error, error_code),
    }
}

// ===== project:save / project:update-config =====

#[tauri::command]
pub fn project_save(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    project_id: String,
    data: serde_json::Value,
    expected_project_path: String,
) -> SimpleResult {
    let app = state.inner();
    let outcome = (|| -> Result<(), String> {
        let lease = assert_write_session(app, &project_session, &expected_project_path)?;
        assert_project_identity(&project_id, &data, &project_session)?;
        if let Some(data_path) = data.get("path").and_then(|item| item.as_str()) {
            if !data_path.is_empty()
                && normalized_project_path(data_path) != normalized_project_path(&lease.root_path)
            {
                return Err("项目身份不匹配，已拒绝操作".to_string());
            }
        }

        let mut update = serde_json::Map::new();
        if let Some(name) = data.get("name").and_then(|item| item.as_str()) {
            if !name.is_empty() {
                update.insert("projectName".to_string(), serde_json::json!(name));
            }
        }
        if let Some(novel_config) = data.get("novelConfig") {
            for (key, value) in novel_config_core_update(novel_config, true) {
                update.insert(key, value);
            }
        }
        if let Some(states) = data.get("characterStates") {
            let serialized = match states.as_str() {
                Some(text) => text.to_string(),
                None => serde_json::to_string(states).map_err(|error| error.to_string())?,
            };
            update.insert("characterStates".to_string(), serde_json::json!(serialized));
        }

        app.with_project_db(|conn| project_core::update(conn, &update))
    })();

    match outcome {
        Ok(()) => {
            if let Some(path) = app.current_project_path() {
                let name = app
                    .with_project_db(|conn| project_core::get(conn))
                    .ok()
                    .flatten()
                    .map(|core| core.project_name)
                    .unwrap_or_default();
                let _ = add_recent_project(RecentProject {
                    name,
                    path,
                    updated_at: now_iso8601(),
                });
            }
            SimpleResult {
                success: true,
                error: None,
            }
        }
        Err(error) => SimpleResult {
            success: false,
            error: Some(error),
        },
    }
}

#[tauri::command]
pub fn project_update_config(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    project_id: String,
    data: serde_json::Value,
    expected_project_path: String,
) -> SimpleResult {
    let app = state.inner();
    let outcome = (|| -> Result<(), String> {
        assert_write_session(app, &project_session, &expected_project_path)?;
        assert_project_identity(&project_id, &data, &project_session)?;
        let novel_config = data.get("novelConfig").unwrap_or(&data);
        let mut update = novel_config_core_update(novel_config, false);
        if let Some(name) = data.get("name").and_then(|item| item.as_str()) {
            if !name.is_empty() {
                update.insert("projectName".to_string(), serde_json::json!(name));
            }
        }
        app.with_project_db(|conn| project_core::update(conn, &update))
    })();

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

// ===== project:delete =====

#[tauri::command]
pub fn project_delete(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    project_path: String,
    project_id: String,
    session_lease: String,
) -> ProjectDeleteResult {
    // 基线以 `_legacyProjectId` / `_legacySessionLease` 忽略这两个历史参数，
    // 授权一律基于调用时冻结的会话上下文。
    let _ = (project_id, session_lease);
    let app = state.inner();
    let Some(active) = app.active_project_snapshot() else {
        return delete_failure(true, guard_message(crate::security::GuardKind::LeaseInvalid), None);
    };
    if let Err(kind) = assert_current_project_context(&project_session, Some(&active)) {
        return delete_failure(true, guard_message(kind), None);
    }

    let home = project_access::home_directory();
    let authorized = match project_access::authorize_deletion(
        &active.to_lease(),
        &project_path,
        home.as_deref(),
    ) {
        Ok(path) => path,
        Err(error) => return delete_failure(true, error, None),
    };

    app.close_project_database();
    app.invalidate_current_session();

    let deletion = remove_directory_with_retry(std::path::Path::new(&authorized));
    if std::path::Path::new(&authorized).exists() {
        let restored = app.activate_project(active).is_ok();
        return delete_failure(
            restored,
            deletion
                .err()
                .map(|error| error.to_string())
                .unwrap_or_else(|| "项目目录删除失败".to_string()),
            None,
        );
    }

    let _ = remove_recent_project(&authorized);
    ProjectDeleteResult {
        success: true,
        directory_deleted: true,
        database_restored: false,
        error: None,
        warning: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_utc_matches_javascript_to_iso_string_test() {
        // 对齐 `new Date(ms).toISOString()`（从 2023-11-14T22:13:20.000Z 起校验）
        assert_eq!(iso8601_utc_from_millis(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601_utc_from_millis(1_700_000_000_000), "2023-11-14T22:13:20.000Z");
        assert_eq!(iso8601_utc_from_millis(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
        // 闰年 2 月 29 日
        assert_eq!(iso8601_utc_from_millis(1_709_164_800_000), "2024-02-29T00:00:00.000Z");
    }

    #[test]
    fn recent_project_path_match_is_lexical_and_case_insensitive_test() {
        assert!(same_recent_project_path("F:\\Novel\\My", "f:/novel/my"));
        assert!(same_recent_project_path("F:/novel/my/", "F:/novel/my"));
        assert!(!same_recent_project_path("F:/novel/a", "F:/novel/b"));
    }
}
