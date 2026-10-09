//! 更新运行时门禁 —— 平移自 `electron/services/update-runtime.ts`（30 行）+ `main.ts:242-252`。
//!
//! 基线门禁：只有**已打包**且**无 dev-server URL** 的 Windows / macOS 才启用更新运行时；
//! 其余一律使用 disabled 后端（`update:check` 返回 `UPDATES_DISABLED`）。
//!
//! 本模块还集中记录本批的**两处刻意偏离**（见文末常量），便于后续（B14）一处改动复活基线行为。

use crate::update::types::UpdateAction;

/// 当前平台标识（对齐 `process.platform` 的取值域，便于测试注入）
pub fn current_platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

/// 对齐基线 `isWindowsUpdateRuntimeEnabled(isPackaged, devServerUrl, platform)`
pub fn is_windows_update_runtime_enabled(
    platform: &str,
    is_packaged: bool,
    dev_server_url: Option<&str>,
) -> bool {
    platform == "win32" && is_packaged && dev_server_url.is_none()
}

/// 对齐基线 `isMacUpdateReminderEnabled(isPackaged, devServerUrl, platform)`
pub fn is_mac_update_reminder_enabled(
    platform: &str,
    is_packaged: bool,
    dev_server_url: Option<&str>,
) -> bool {
    platform == "darwin" && is_packaged && dev_server_url.is_none()
}

/// 对齐基线 `main.ts:244`：`updateRuntimeEnabled = windows || mac`
pub fn update_runtime_enabled(
    platform: &str,
    is_packaged: bool,
    dev_server_url: Option<&str>,
) -> bool {
    is_windows_update_runtime_enabled(platform, is_packaged, dev_server_url)
        || is_mac_update_reminder_enabled(platform, is_packaged, dev_server_url)
}

/// 本批统一的更新动作：**全平台 `open-release`**。
///
/// **刻意偏离基线**：基线在 Windows 已安装包上使用 `'download'`（electron-updater 就地更新），
/// 其余平台 `'open-release'`。Tauri 侧本批不引入 `tauri-plugin-updater`（B14），因此所有平台
/// 都只能「检查新版本 + 打开 Release 页」，`update:download` 诚实返回 `DOWNLOAD_NOT_READY`。
pub const UPDATE_ACTION: UpdateAction = UpdateAction::OpenRelease;

/// 对齐基线 `hasWindowsUpdateConfiguration`（检查 Electron Builder 的 `resources/app-update.yml`）。
///
/// **刻意偏离**：Tauri 构建不产生该文件且本批不接 updater 插件；本批确实拥有可用的
/// GitHub-Release 配置，故 `updateConfiguration` 恒为 `available`（而不是基线的 `missing`）。
/// 这样 Windows 打包版同样能查到新版本并打开 Release 页（用户已确认该策略）。
pub fn update_configuration_available() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_gate_requires_packaged_without_dev_server_test() {
        // Windows 打包 + 无 dev URL → 启用
        assert!(is_windows_update_runtime_enabled("win32", true, None));
        // 未打包（dev）→ 关闭
        assert!(!is_windows_update_runtime_enabled("win32", false, None));
        // 有 dev URL → 关闭
        assert!(!is_windows_update_runtime_enabled(
            "win32",
            true,
            Some("http://127.0.0.1:5190")
        ));
        // 其它平台 → 关闭
        assert!(!is_windows_update_runtime_enabled("darwin", true, None));

        assert!(is_mac_update_reminder_enabled("darwin", true, None));
        assert!(!is_mac_update_reminder_enabled("darwin", false, None));
        assert!(!is_mac_update_reminder_enabled("win32", true, None));
    }

    #[test]
    fn update_runtime_enabled_matches_baseline_union_test() {
        assert!(update_runtime_enabled("win32", true, None));
        assert!(update_runtime_enabled("darwin", true, None));
        assert!(!update_runtime_enabled("linux", true, None));
        assert!(!update_runtime_enabled("win32", false, None));
        assert!(!update_runtime_enabled("darwin", true, Some("http://x")));
        // 本机（dev）应为关闭
        assert!(!update_runtime_enabled(current_platform(), false, None));
    }

    #[test]
    fn batch_deviations_are_explicit_test() {
        assert_eq!(UPDATE_ACTION, UpdateAction::OpenRelease);
        assert!(update_configuration_available());
    }
}
