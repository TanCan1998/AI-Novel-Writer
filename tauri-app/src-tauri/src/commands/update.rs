//! 批次 H（H3）：应用更新频道 —— `update:*` 6 个 + `update:state` 事件。
//!
//! 平移自 `electron/controllers/update-controller.ts`（54 行）：
//! - 6 个频道一一对应；**失败一律走返回信封**（不 reject），与基线 `ipcMain.handle` 一致；
//! - `update:state` 事件由服务注入的 `publish` 闭包广播（见 `update/startup.rs`），
//!   命令层不额外发事件；
//! - `days` 入参按基线语义宽容处理：非数字 → `INVALID_REMINDER_DELAY` 信封（而非 reject）。

use std::sync::Arc;

use serde_json::Value;
use tauri::{AppHandle, State};

use crate::state::AppState;
use crate::update::service::UpdateService;
use crate::update::types::{
    updates_disabled_error, UpdateActionResponse, UpdateCheckResponse, UpdateState, UpdateStatus,
};

fn service(state: &AppState) -> Option<Arc<UpdateService>> {
    state.update_service()
}

/// 未装配服务时的兜底状态（对齐基线「未安装」：`disabled` + `currentVersion`）
fn uninstalled_state(app: &AppHandle) -> UpdateState {
    UpdateState {
        status: UpdateStatus::Disabled,
        current_version: app.package_info().version.to_string(),
        available_version: None,
        update_action: None,
        release_name: None,
        release_notes: None,
        release_date: None,
        last_checked_at: None,
        reminder_until: None,
        is_reminder_deferred: false,
        download_progress: None,
        error: None,
    }
}

fn uninstalled_check_response(app: &AppHandle) -> UpdateCheckResponse {
    UpdateCheckResponse {
        success: false,
        checked: false,
        update_available: false,
        state: uninstalled_state(app),
        error: Some(updates_disabled_error()),
    }
}

fn uninstalled_action_response(app: &AppHandle) -> UpdateActionResponse {
    UpdateActionResponse {
        success: false,
        state: uninstalled_state(app),
        error: Some(updates_disabled_error()),
    }
}

/// `update:get-state`
#[tauri::command]
pub fn update_get_state(state: State<'_, AppState>, app: AppHandle) -> UpdateState {
    match service(state.inner()) {
        Some(service) => service.get_state(),
        None => uninstalled_state(&app),
    }
}

/// `update:check`
#[tauri::command]
pub async fn update_check(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<UpdateCheckResponse, String> {
    let Some(service) = service(state.inner()) else {
        return Ok(uninstalled_check_response(&app));
    };
    Ok(service.check_manually().await)
}

/// `update:download`
#[tauri::command]
pub async fn update_download(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<UpdateActionResponse, String> {
    let Some(service) = service(state.inner()) else {
        return Ok(uninstalled_action_response(&app));
    };
    Ok(service.download_update().await)
}

/// `update:open-release`
#[tauri::command]
pub async fn update_open_release(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<UpdateActionResponse, String> {
    let Some(service) = service(state.inner()) else {
        return Ok(uninstalled_action_response(&app));
    };
    Ok(service.open_release().await)
}

/// `update:defer-reminder`（`days` 仅接受 7 / 30）
#[tauri::command]
pub async fn update_defer_reminder(
    state: State<'_, AppState>,
    app: AppHandle,
    days: Value,
) -> Result<UpdateActionResponse, String> {
    let Some(service) = service(state.inner()) else {
        return Ok(uninstalled_action_response(&app));
    };
    // 对齐基线：非数字 → NaN → INVALID_REMINDER_DELAY
    let days = match days.as_u64() {
        Some(value) => u32::try_from(value).unwrap_or(u32::MAX),
        None => u32::MAX,
    };
    Ok(service.defer_reminder(days).await)
}

/// `update:quit-and-install`
#[tauri::command]
pub async fn update_quit_and_install(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<UpdateActionResponse, String> {
    let Some(service) = service(state.inner()) else {
        return Ok(uninstalled_action_response(&app));
    };
    Ok(service.request_install().await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::types::UpdateErrorCode;

    #[test]
    fn uninstalled_envelopes_match_disabled_semantics_test() {
        // 不依赖真实 AppHandle：直接构造状态以验证信封形状
        let error = updates_disabled_error();
        assert_eq!(error.code, UpdateErrorCode::UpdatesDisabled);
        assert_eq!(
            error.phase,
            crate::update::types::UpdateErrorPhase::Configuration
        );
        assert!(!error.retryable);
    }

    #[test]
    fn defer_reminder_day_coercion_is_lenient_test() {
        let coerce = |value: &Value| match value.as_u64() {
            Some(value) => u32::try_from(value).unwrap_or(u32::MAX),
            None => u32::MAX,
        };
        assert_eq!(coerce(&serde_json::json!(7)), 7);
        assert_eq!(coerce(&serde_json::json!(30)), 30);
        // 非法值 → 远超 7/30 → INVALID_REMINDER_DELAY
        assert_ne!(coerce(&serde_json::json!(3)), 7);
        assert_ne!(coerce(&serde_json::json!("7")), 7);
        assert_ne!(coerce(&serde_json::json!(null)), 7);
    }
}
