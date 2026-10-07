//! Model Provider Resource 频道的 Tauri 命令实现
//!
//! 对应 `src/shared/ipc-channels.ts` 的 `ModelProviderResourceChannels`：
//! - `model-provider-resource:open` → `model_provider_resource_open`

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// 模型资源 URL 映射（固定）
const MODEL_PROVIDER_RESOURCE_URLS: &[(&str, &str)] = &[
    ("siliconflow-invite", "https://cloud.siliconflow.cn/i/klFgdwZa"),
    ("siliconflow-console", "https://cloud.siliconflow.cn"),
    ("siliconflow-docs", "https://docs.siliconflow.cn"),
];

/// Model Provider Resource:open 命令
#[tauri::command]
pub fn model_provider_resource_open(
    _app: AppHandle,
    resource: String,
) -> Result<ModelProviderResourceOpenResponse, String> {
    // TODO: 实现打开模型资源逻辑（webbrowser/shell-opener）
    let _url = MODEL_PROVIDER_RESOURCE_URLS
        .iter()
        .find(|(id, _)| id == &resource)
        .map(|(_, url)| *url)
        .ok_or_else(|| format!("无效的资源 ID: {}", resource))?;

    Ok(ModelProviderResourceOpenResponse {
        success: true,
        error: None,
    })
}

/// Model Provider Resource:open 的响应
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProviderResourceOpenResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_provider_resource_urls_are_defined() {
        assert!(!MODEL_PROVIDER_RESOURCE_URLS.is_empty());
    }

    #[test]
    fn siliconflow_invite_url_exists() {
        let url = MODEL_PROVIDER_RESOURCE_URLS
            .iter()
            .find(|(id, _)| *id == "siliconflow-invite")
            .map(|(_, url)| *url);
        assert_eq!(url, Some("https://cloud.siliconflow.cn/i/klFgdwZa"));
    }

    #[test]
    fn model_provider_resource_open_response_serializes() {
        let response = ModelProviderResourceOpenResponse {
            success: true,
            error: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("success"));
    }
}
