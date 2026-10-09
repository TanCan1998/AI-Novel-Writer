//! Model Provider Resource 频道的 Tauri 命令实现
//!
//! 对应 `src/shared/ipc-channels.ts` 的 `ModelProviderResourceChannels`：
//! - `model-provider-resource:open` → `model_provider_resource_open`

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::external_link::open_external_url;

/// 模型资源 URL 映射（固定；与 `src/shared/model-provider-resources.ts` 逐字对齐）
const MODEL_PROVIDER_RESOURCE_URLS: &[(&str, &str)] = &[
    (
        "siliconflow-invite",
        "https://cloud.siliconflow.cn/i/klFgdwZa",
    ),
    ("siliconflow-console", "https://cloud.siliconflow.cn"),
    ("siliconflow-docs", "https://docs.siliconflow.cn"),
];

/// Model Provider Resource:open 命令
///
/// 平移自 `electron/controllers/model-provider-resource-controller.ts`：
/// 无效 id → `{ success:false, error:'Unsupported model provider resource.' }`（正常响应，非 reject）；
/// 打开失败 → `{ success:false, error:'Unable to open the model provider resource.' }`（文案逐字对齐）。
#[tauri::command]
pub fn model_provider_resource_open(
    app: AppHandle,
    resource: String,
) -> Result<ModelProviderResourceOpenResponse, String> {
    let Some(url) = MODEL_PROVIDER_RESOURCE_URLS
        .iter()
        .find(|(id, _)| id == &resource)
        .map(|(_, url)| *url)
    else {
        return Ok(ModelProviderResourceOpenResponse {
            success: false,
            error: Some("Unsupported model provider resource.".to_string()),
        });
    };

    match open_external_url(&app, url) {
        Ok(()) => Ok(ModelProviderResourceOpenResponse {
            success: true,
            error: None,
        }),
        Err(_) => Ok(ModelProviderResourceOpenResponse {
            success: false,
            error: Some("Unable to open the model provider resource.".to_string()),
        }),
    }
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
