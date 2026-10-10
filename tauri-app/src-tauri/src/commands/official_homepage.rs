//! Official Homepage 频道的 Tauri 命令实现
//!
//! 对应 `src/shared/ipc-channels.ts` 的 `OfficialHomepageChannels`：
//! - `official-homepage:open` → `official_homepage_open`
//!
//! 平移自 `electron/controllers/official-homepage-controller.ts`：
//! `shell.openExternal(OFFICIAL_HOMEPAGE_URL)` → `{ success: true }`；
//! 打开失败 → `{ success: false, error: 'Unable to open the official homepage.' }`（文案逐字对齐）。
//!
//! ⚠️ **刻意偏离（已评估）**：基线常量指向**上游仓库** `https://github.com/EthanYoQ/AI-Novel-Writer`；
//! 本 fork 的产品身份是 **Lorekeeper（设定司）**（`identifier = com.tancan1998.lorekeeper`），
//! 因此「官方主页」指向 fork 仓库。URL 只由本常量提供，渲染层无法传入。

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::external_link::open_external_url;

/// 官方主页 URL（固定；唯一受信外部目的地之一）
///
/// `pub(crate)`：批次 A 收口（B13）`navigation_guard` 复用同一常量
/// （导航钩子里的精确匹配判定）；值不变，渲染层仍无法传入 URL。
pub(crate) const OFFICIAL_HOMEPAGE_URL: &str = "https://github.com/TanCan1998/Lorekeeper";

/// Official Homepage:open 命令
#[tauri::command]
pub fn official_homepage_open(app: AppHandle) -> Result<OfficialHomepageOpenResponse, String> {
    match open_external_url(&app, OFFICIAL_HOMEPAGE_URL) {
        Ok(()) => Ok(OfficialHomepageOpenResponse {
            success: true,
            error: None,
        }),
        Err(_) => Ok(OfficialHomepageOpenResponse {
            success: false,
            error: Some("Unable to open the official homepage.".to_string()),
        }),
    }
}

/// Official Homepage:open 的响应
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficialHomepageOpenResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_homepage_url_is_https_constant() {
        assert_eq!(
            OFFICIAL_HOMEPAGE_URL,
            "https://github.com/TanCan1998/Lorekeeper"
        );
        assert!(OFFICIAL_HOMEPAGE_URL.starts_with("https://"));
    }

    #[test]
    fn official_homepage_open_response_serializes() {
        let response = OfficialHomepageOpenResponse {
            success: false,
            error: Some("Unable to open the official homepage.".to_string()),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"success\":false"));
        assert!(json.contains("Unable to open the official homepage."));
    }
}
