//! Official Homepage 频道的 Tauri 命令实现
//!
//! 对应 `src/shared/ipc-channels.ts` 的 `OfficialHomepageChannels`：
//! - `official-homepage:open` → `official_homepage_open`

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// 官方主页 URL（固定）。待 tauri-plugin-opener 接入后由命令实际使用
/// （当前为骨架：不打开外部链接），暂抑制死代码警告。
#[allow(dead_code)]
const OFFICIAL_HOMEPAGE_URL: &str = "https://github.com/TanCan1998/Lorekeeper";

/// Official Homepage:open 命令
#[tauri::command]
pub fn official_homepage_open(_app: AppHandle) -> Result<OfficialHomepageOpenResponse, String> {
    // TODO: 实现打开官方主页逻辑
    Ok(OfficialHomepageOpenResponse {
        success: true,
        error: None,
    })
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
    fn official_homepage_url_is_constant() {
        assert_eq!(OFFICIAL_HOMEPAGE_URL, "https://github.com/TanCan1998/Lorekeeper");
    }

    #[test]
    fn official_homepage_open_response_serializes() {
        let response = OfficialHomepageOpenResponse {
            success: true,
            error: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("success"));
    }
}
