//! 渲染层导航防护 —— 批次 A 收口（B13）。
//!
//! 平移自基线 Electron 的两道防线（`electron/services/official-homepage-navigation.ts`，
//! 接线于 `electron/main.ts`）：
//!
//! 1. `preventRendererNavigation`：渲染进程不能把现有主窗口导航到外部内容。
//!    → 本插件对「非应用自身来源」的导航一律返回 `false`（取消导航）。
//! 2. `createOfficialHomepageWindowOpenHandler`：所有新窗口请求一律 `deny`，
//!    唯一例外是 URL **精确等于** `OFFICIAL_HOMEPAGE_URL` 的请求，
//!    交给系统默认浏览器打开（失败仅告警）。
//!    → 新窗口侧：wry 在无 handler 时默认 `SetHandled(true)`（即默认拒绝），
//!    且渲染层经实测没有任何 `window.open(` / `target="_blank"` 用法
//!    （官方主页只经 `official-homepage:open` 频道），弹窗路径天然对齐基线。
//!    → 导航侧：URL 精确等于官方主页时同样交给系统浏览器，但**仍然返回 false**
//!    （对齐基线「永远 deny」）。
//!
//! 主窗口由 `tauri.conf.json` 的 `app.windows[0]`（`label: "main"`）声明，
//! 不是 builder 创建，`WebviewBuilder::on_navigation` 用不上；
//! `tauri::plugin::Builder::on_navigation` 会被插件 store 对**所有 webview**
//! 逐插件调用（任一插件返回 false 即取消导航），故以自有插件注册即可
//! 非侵入覆盖配置声明的窗口。
//!
//! # 放行规则（应用自身来源；否则初始加载 / dev HMR 会被自己拦死）
//!
//! - scheme 为 `tauri`（生产包）；
//! - host 为 `tauri.localhost`（生产包内部页面）；
//! - host 为 `localhost` / `127.0.0.1`（dev server 5190，见 tauri.conf.json `devUrl`）。
//!
//! 其余一律拒绝（对齐基线 `preventRendererNavigation`）。

use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Url,
};

use crate::{commands::OFFICIAL_HOMEPAGE_URL, external_link};

/// 是否为应用自身来源（初始加载 / dev HMR 依赖此放行）。
fn is_app_origin(url: &Url) -> bool {
    url.scheme() == "tauri"
        || url.host_str() == Some("tauri.localhost")
        || matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
}

/// URL 是否**精确等于**官方主页。
///
/// 对齐基线 `isOfficialHomepageUrl`（`url === OFFICIAL_HOMEPAGE_URL`）：
/// 带路径 / 查询 / 仿冒主机都不算。
fn is_official_homepage(url: &Url) -> bool {
    url.as_str() == OFFICIAL_HOMEPAGE_URL
}

/// 导航决策纯函数：`true` = 放行，`false` = 取消。
fn should_allow_navigation(url: &Url) -> bool {
    is_app_origin(url)
}

/// 注册导航防护插件（对所有 webview 生效，详见模块头注释）。
///
/// ⚠️ 刻意偏离 Task 建议的泛型签名 `init<R: Runtime>()`：
/// `external_link::open_external_url` 的形参是 `&AppHandle`
/// （默认类型参数 = `AppHandle<Wry<EventLoopMessage>>`，具体运行时），
/// 泛型 `init<R>` 的 `on_navigation` 闭包只能拿到 `&AppHandle<R>`，
/// 无法统一为 `&AppHandle<Wry>`（E0308）。本 crate 桌面端（WebView2）
/// 唯一运行时即 Wry，与 `lib.rs` 的 `tauri::Builder::default()`
/// （默认 R = Wry）一致，故钉死为 `TauriPlugin<tauri::Wry>`。
pub fn init() -> TauriPlugin<tauri::Wry> {
    Builder::new("navigation-guard")
        .on_navigation(|webview, url| {
            if is_official_homepage(url) {
                // 对齐基线：精确匹配官方主页的请求交给系统浏览器；
                // 打开失败仅告警（对齐基线 `console.warn`），不 panic、不阻塞。
                if let Err(error) =
                    external_link::open_external_url(webview.app_handle(), OFFICIAL_HOMEPAGE_URL)
                {
                    eprintln!(
                        "[Lorekeeper] Unable to open official homepage from a navigation request: {error}"
                    );
                }
                // 基线 createOfficialHomepageWindowOpenHandler 永远 deny，导航仍被取消。
                return false;
            }
            should_allow_navigation(url)
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Url {
        Url::parse(source).expect("测试 URL 必须可解析")
    }

    #[test]
    fn allows_app_origins_test() {
        assert!(should_allow_navigation(&parse("tauri://localhost")));
        assert!(should_allow_navigation(&parse("http://tauri.localhost")));
        assert!(should_allow_navigation(&parse("http://127.0.0.1:5190")));
        assert!(should_allow_navigation(&parse("http://localhost:5190")));
    }

    #[test]
    fn rejects_foreign_and_dangerous_urls_test() {
        assert!(!should_allow_navigation(&parse("https://evil.example/")));
        // 官方主页带查询 / 尾斜杠 ≠ 精确匹配，同样拒绝（对齐基线精确等值语义）。
        assert!(!should_allow_navigation(&parse(
            "https://github.com/TanCan1998/Lorekeeper?x=1"
        )));
        assert!(!should_allow_navigation(&parse(
            "https://github.com/TanCan1998/Lorekeeper/"
        )));
        assert!(!should_allow_navigation(&parse("javascript:alert(1)")));
        assert!(!should_allow_navigation(&parse(
            "file:///C:/Windows/System32/calc.exe"
        )));
    }

    #[test]
    fn official_homepage_matches_exactly_test() {
        assert!(is_official_homepage(&parse(OFFICIAL_HOMEPAGE_URL)));
        assert!(!is_official_homepage(&parse(
            "https://github.com/TanCan1998/Lorekeeper?x=1"
        )));
        assert!(!is_official_homepage(&parse(
            "https://github.com/TanCan1998/Lorekeeper/"
        )));
        assert!(!is_official_homepage(&parse(
            "https://github.com/TanCan1998/"
        )));
    }
}
