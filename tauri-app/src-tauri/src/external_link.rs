//! 打开外部链接 —— 批次 H（B12）新增的共用能力。
//!
//! 平移自基线 `shell.openExternal` 的两个调用点（`official-homepage-controller.ts` /
//! `model-provider-resource-controller.ts`），并供 H3 的 `update:open-release` 复用。
//!
//! # 安全边界（对齐基线「唯一受信外部目的地」的语义）
//!
//! - URL **只能**来自本 crate 的编译期常量表（`OFFICIAL_HOMEPAGE_URL` /
//!   `MODEL_PROVIDER_RESOURCE_URLS` / `GITHUB_LATEST_RELEASE_PAGE`），渲染层永远不能传入 URL；
//! - 经 `tauri-plugin-opener` 交由系统默认浏览器打开；不经过 shell 展开；
//! - 仅由 Rust 侧调用，渲染层未引入 `@tauri-apps/plugin-opener`，因此
//!   `capabilities/default.json` **未**开放 opener 权限（权限只门禁渲染层→插件命令）。
//!
//! ⚠️ 基线还有一层「拒绝渲染层导航替换主框架」（`preventRendererNavigation` +
//! `createOfficialHomepageWindowOpenHandler`）；Tauri 侧的等价能力（`on_navigation` /
//! 新窗口拦截）**尚未接入**，已记为后续项，不在本批扩大改动面。

use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

/// 只允许 HTTPS 外部链接（本 crate 的常量均为 https；渲染层无法传入 URL）
pub fn is_allowed_external_url(url: &str) -> bool {
    url.starts_with("https://")
}

/// 打开一个**受信常量**外部 URL
pub fn open_external_url(app: &AppHandle, url: &str) -> Result<(), String> {
    if !is_allowed_external_url(url) {
        return Err(format!("拒绝打开非 HTTPS 外部链接：{url}"));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_non_https_urls_test() {
        assert!(super::is_allowed_external_url(
            "https://github.com/TanCan1998/Lorekeeper"
        ));
        assert!(!super::is_allowed_external_url(
            "http://github.com/TanCan1998/Lorekeeper"
        ));
        assert!(!super::is_allowed_external_url(
            "file:///C:/Windows/System32/calc.exe"
        ));
        assert!(!super::is_allowed_external_url("javascript:alert(1)"));
    }
}
