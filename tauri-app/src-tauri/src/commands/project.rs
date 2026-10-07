//! 批次 B：项目生命周期命令 —— 迁移自 `electron/controllers/project-controller.ts`。
//!
//! 本批次落地：
//! - 真实实现：`project:get-runtime-context` / `project:recent-list` /
//!   `project:recent-remove` / `project:smoke-open-request` /
//!   `project:smoke-open-confirm`；
//! - `dialog:select-folder` 骨架返回 null（等 tauri-plugin-dialog 接入）；
//! - `project:create` / `project:open` / `project:save` / `project:update-config`
//!   / `project:delete` 为骨架占位（依赖 SQLite 初始化与租约签发，ADR 0001，
//!   批次 C 接入），返回契约形状完整的结构化失败，不用 invoke reject。

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::SimpleResult;
use crate::security::{normalized_project_path, ProjectSessionContext};
use crate::state::AppState;

const SKELETON_DB_MESSAGE: &str = "项目数据库层尚未迁移（批次 C），该命令骨架未启用。";

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

/// VELA_HOME —— 对齐 `electron/utils/config-utils.ts`：
/// `process.env.AI_NOVEL_VELA_HOME?.trim() || ~/.vela`。
pub fn vela_home() -> std::path::PathBuf {
    if let Ok(home) = std::env::var("AI_NOVEL_VELA_HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return std::path::PathBuf::from(trimmed);
        }
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    std::path::PathBuf::from(home.trim()).join(".vela")
}

fn recent_projects_path() -> std::path::PathBuf {
    vela_home().join("recent-projects.json")
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
    let active = state
        .active_project
        .lock()
        .expect("active_project 锁中毒")
        .as_ref()
        .map(|p| p.root_path.clone());
    RuntimeContext {
        active_project_path: active,
        // 骨架：数据库层未迁移，恒 false。批次 C 接入后对齐基线语义：
        // `activeProjectPath == null ? db == null : db != null`。
        db_ready: false,
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
    let active: Option<String> = {
        let active = state.active_project.lock().expect("active_project 锁中毒");
        active.as_ref().map(|p| p.root_path.clone())
    };
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

/// `project:create` 骨架 —— 依赖 SQLite 项目库初始化与租约签发（批次 C）。
#[tauri::command]
pub fn project_create(
    _config: serde_json::Value,
    request_token: String,
    _renderer_project_path: Option<String>,
) -> ProjectCreateResult {
    ProjectCreateResult {
        success: false,
        project_id: String::new(),
        project_path: None,
        request_token,
        active_project_path: None,
        database_restored: false,
        db_ready: false,
        stale: None,
        error: Some(SKELETON_DB_MESSAGE.to_string()),
        error_code: None,
    }
}

#[tauri::command]
pub fn project_open(
    _project_path: String,
    request_token: String,
    _renderer_project_path: Option<String>,
) -> ProjectOpenResult {
    ProjectOpenResult {
        success: false,
        project: None,
        request_token,
        active_project_path: None,
        database_restored: false,
        db_ready: false,
        stale: None,
        error: Some(SKELETON_DB_MESSAGE.to_string()),
        error_code: None,
    }
}

#[tauri::command]
pub fn project_save(
    _project_session: ProjectSessionContext,
    _project_id: String,
    _data: serde_json::Value,
    _expected_project_path: String,
) -> SimpleResult {
    SimpleResult { success: false, error: Some(SKELETON_DB_MESSAGE.to_string()) }
}

#[tauri::command]
pub fn project_update_config(
    _project_session: ProjectSessionContext,
    _project_id: String,
    _data: serde_json::Value,
    _expected_project_path: String,
) -> SimpleResult {
    SimpleResult { success: false, error: Some(SKELETON_DB_MESSAGE.to_string()) }
}

#[tauri::command]
pub fn project_delete(
    _project_session: ProjectSessionContext,
    _project_path: String,
    _project_id: String,
    _session_lease: String,
) -> ProjectDeleteResult {
    ProjectDeleteResult {
        success: false,
        directory_deleted: false,
        database_restored: false,
        error: Some(SKELETON_DB_MESSAGE.to_string()),
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
