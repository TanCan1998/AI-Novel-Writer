//! 更新运行时装配 —— 平移自 `electron/services/update-startup.ts`（73 行）+ `main.ts:242-260`。
//!
//! 纪律（对齐基线注释）：**更新器永远不是应用启动的前置条件**。任何后端构造 /
//! 偏好读取 / 自动检查失败都降级为「更新不可用」，只记录日志，绝不中断作者使用工作区。

use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use crate::update::backend::{
    DisabledBackend, GithubReleaseBackend, UpdateBackend, GITHUB_LATEST_RELEASE_PAGE,
};
use crate::update::preferences::ConfigUpdatePreferencesStore;
use crate::update::runtime;
use crate::update::service::{OpenReleaseFn, PublishFn, UpdateService, UpdateServiceOptions};
use crate::update::types::UpdateState;

/// 装配结果（供日志/诊断与后续项 B14 使用）
pub struct UpdateStartupOutcome {
    pub service: Arc<UpdateService>,
    pub runtime_enabled: bool,
    pub is_packaged_runtime: bool,
}

/// 创建并返回更新服务；按基线的门禁与降级规则装配。
///
/// ⚠️ 与本机相关：`tauri::is_dev()` 为真（`pnpm tauri dev`）或存在 `devUrl` 时，
/// 门禁关闭 → 服务处于 `disabled`（`update:check` 返回 `UPDATES_DISABLED`），与基线一致。
pub fn start_update_runtime(handle: &AppHandle) -> UpdateStartupOutcome {
    let dev_server_url = handle
        .config()
        .build
        .dev_url
        .as_ref()
        .map(|url| url.to_string());
    let is_packaged = !tauri::is_dev();
    let platform = runtime::current_platform();
    let runtime_enabled =
        runtime::update_runtime_enabled(platform, is_packaged, dev_server_url.as_deref());
    // 本批偏离：恒认为「有可用更新配置」（见 runtime.rs 说明）
    let configuration_missing = !runtime::update_configuration_available();

    let mut backend: Arc<dyn UpdateBackend> = Arc::new(DisabledBackend);
    let mut is_packaged_runtime = false;
    if runtime_enabled && !configuration_missing {
        match GithubReleaseBackend::from_global_config() {
            Ok(github) => {
                backend = Arc::new(github);
                is_packaged_runtime = true;
            }
            Err(error) => {
                // 对齐基线 `reportFailure('初始化更新器', error)`：降级但不抛出
                eprintln!("[Lorekeeper Update] 初始化更新器失败，已降级为更新不可用：{error}");
            }
        }
    } else if runtime_enabled {
        // 保留 packaged 状态，以便手动检查能显示「配置缺失」的可行动错误
        is_packaged_runtime = true;
    }

    let emitter = handle.clone();
    let publish: PublishFn = Arc::new(move |state: &UpdateState| {
        // 单窗口应用：直接向全部 webview 广播（对齐基线 `BrowserWindow.getAllWindows()` 广播）
        let _ = emitter.emit("update:state", state.clone());
    });
    let opener = handle.clone();
    let open_release: OpenReleaseFn = Arc::new(move || {
        let handle = opener.clone();
        Box::pin(async move {
            crate::external_link::open_external_url(&handle, GITHUB_LATEST_RELEASE_PAGE)
        })
    });

    let service = Arc::new(UpdateService::new(UpdateServiceOptions {
        updater: backend,
        current_version: handle.package_info().version.to_string(),
        is_packaged: is_packaged_runtime,
        update_configuration_missing: configuration_missing,
        update_action: runtime::UPDATE_ACTION,
        open_release: Some(open_release),
        preferences: Arc::new(ConfigUpdatePreferencesStore::new()),
        now_millis: None,
        publish: Some(publish),
    }));

    // 对齐基线 `update-startup.ts:71`：启动后 fire-and-forget 自动检查（失败不阻塞启动）
    if is_packaged_runtime && !configuration_missing {
        let auto = service.clone();
        tauri::async_runtime::spawn(async move {
            // 自动检查的失败只留在主进程（`check_automatically` 自身不抛错）
            let _ = auto.check_automatically().await;
        });
    }

    UpdateStartupOutcome {
        service,
        runtime_enabled,
        is_packaged_runtime,
    }
}
