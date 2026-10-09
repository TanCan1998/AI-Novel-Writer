//! 批次 D2-c：`llm:*` 收口 —— 连通性探测与模型发现（最后 2 个 `llm:` 频道）。
//!
//! 迁移自 `electron/controllers/llm-controller.ts` 的 `llm:test-connection` /
//! `llm:discover-models` 两个 handler：
//!
//! | 频道 | 命令 | 基线行为 |
//! |---|---|---|
//! | `llm:test-connection` | [`llm_test_connection`] | `purposes` 含 `embedding` → 走 Embedding 批量端点；否则发一条 `Say "hello"…` 探测 |
//! | `llm:discover-models` | [`llm_discover_models`] | 用**未保存**的表单字段请求供应商模型列表（`electron/services/model-discovery-service.ts`） |
//!
//! ## 与基线一致性要点
//!
//! 1. **探测共享同一套生成参数策略** —— 基线测试断言普通生成、流式生成、连通性
//!    探测三者的 `reasoning` 完全一致，因此这里同样走
//!    [`resolve_generation_parameters`]，只把「预算 + 阶段」换成探测值。
//! 2. **推理模型需要足够预算** —— `maxTokens = 1024`（基线
//!    `CONNECTION_TEST_MAX_TOKENS`）；预算太小会把「可用模型」误判为截断失败。
//! 3. **探测不携带会话粘性** —— `conversationId` 恒为 `None`（基线测试
//!    `does not give the connection probe a creative conversation scope`）。
//! 4. **失败文案形态与生成链一致** —— 参数解析失败走 `Error: ` 前缀；Embedding
//!    失败保留基线两档前缀（`Error: ` / `EmbeddingResponseValidationError: `）。
//!
//! ## 与基线的**刻意差异**
//!
//! | # | 基线 | Tauri 侧 | 影响 |
//! |---|---|---|---|
//! | 1 | `const provider = LLMFactory.getProvider(model)` 在 embedding 分支前也执行 | 仅在生成分支取 provider | 该调用无副作用（只是 `new`），行为等价 |
//! | 2 | 客户端构建失败会抛 `TypeError` | 返回 `{success:false, error:"Error: …"}` | 基线只有代理配置非法时才可能发生 |
//! | 3 | `embedding.ts` 含 `releaseSmokeEmbeddings` / `chunkText` | 未移植（无调用面） | 见 `llm/embedding.rs` 模块注释 |

use serde::Serialize;
use serde_json::Value;

use crate::app_paths;
use crate::llm::chat::{
    build_client, is_gemini, provider_error_text, proxy_from_config, ChatMessage,
    LlmGenerateOptions,
};
use crate::llm::discovery::{
    build_discovery_client, discover_models, ModelDiscoveryErrorCode, ModelDiscoveryResult,
};
use crate::llm::embedding::generate_embeddings;
use crate::llm::params::{
    resolve_generation_parameters, GenerationParameterModel, GenerationParameterRequest,
};
use crate::llm::{gemini, openai};

/// 连通性探测的输出预算（对齐 `CONNECTION_TEST_MAX_TOKENS`）。
pub const CONNECTION_TEST_MAX_TOKENS: u64 = 1024;

/// 连通性探测提示词（对齐基线字面量）。
pub const CONNECTION_TEST_PROMPT: &str = "Say \"hello\" and nothing else.";

/// 连通性探测的受控阶段（对齐基线 `reasoningStage: 'general'`）。
pub const CONNECTION_TEST_REASONING_STAGE: &str = "general";

/// `creativeStrategy` 缺省值（对齐基线形参默认值 `'auto'`）。
pub const DEFAULT_CREATIVE_STRATEGY: &str = "auto";

/// `llm:test-connection` 响应（契约 `{ success: boolean; error?: string }`）。
#[derive(Debug, Serialize)]
pub struct LlmTestConnectionResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 探测失败信封（**无** `Error: ` 前缀，由各来源决定是否自带）。
fn failure(message: String) -> LlmTestConnectionResult {
    LlmTestConnectionResult {
        success: false,
        error: Some(message),
    }
}

/// 探测类型判定：`model.purposes?.includes('embedding')`。
///
/// `purposes` 缺失或非数组时按 `false` 处理（对齐可选链的短路语义）。
pub fn is_embedding_probe(model: &Value) -> bool {
    model
        .get("purposes")
        .and_then(Value::as_array)
        .map(|purposes| {
            purposes
                .iter()
                .any(|purpose| purpose.as_str() == Some("embedding"))
        })
        .unwrap_or(false)
}

/// 探测请求的参数意图（对齐基线字面量：预算 1024 / 阶段 `general` / 无结构化输出）。
pub fn connection_test_parameter_request(
    creative_strategy: &str,
) -> GenerationParameterRequest<'_> {
    GenerationParameterRequest {
        max_tokens: Some(CONNECTION_TEST_MAX_TOKENS),
        // 基线未传 `responseFormat`（键缺失 → 整体缺省），不是 `{type: null}`。
        response_format: None,
        creative_strategy: Some(creative_strategy),
        reasoning_stage: Some(CONNECTION_TEST_REASONING_STAGE),
    }
}

/// 探测消息（对齐基线字面量）。
pub fn connection_test_messages() -> Vec<ChatMessage> {
    vec![ChatMessage {
        role: "user".to_string(),
        content: CONNECTION_TEST_PROMPT.to_string(),
    }]
}

/// 同步解析探测用的生成参数。
///
/// 与生成链共用同一策略：温度只来自模型档案（固定采样家族则省略字段），
/// 推理指令与 `creativeStrategy` / `general` 阶段绑定；显式清空会话粘性。
pub fn resolve_connection_options(
    model: &Value,
    creative_strategy: &str,
) -> Result<LlmGenerateOptions, String> {
    let parameter_model = GenerationParameterModel::from_value(model);
    let parameter_request = connection_test_parameter_request(creative_strategy);
    resolve_generation_parameters(&parameter_model, &parameter_request)
        .map(|resolved| LlmGenerateOptions {
            temperature: resolved.temperature,
            max_tokens: resolved.max_tokens,
            response_format: resolved.response_format,
            reasoning: resolved.reasoning,
            // 探测不是创作会话：不得共享会话粘性。
            conversation_id: None,
        })
        .map_err(provider_error_text)
}

/// `llm:test-connection` —— 用给定模型发一次最小请求验证连通性。
///
/// 返回值**不带** `Error: ` 前缀的部分是 provider 信封；参数解析与 Embedding
/// 失败自带前缀（见模块注释）。
#[tauri::command]
pub async fn llm_test_connection(
    model: Value,
    creative_strategy: Option<String>,
) -> LlmTestConnectionResult {
    let options = match resolve_connection_options(
        &model,
        creative_strategy
            .as_deref()
            .unwrap_or(DEFAULT_CREATIVE_STRATEGY),
    ) {
        Ok(options) => options,
        Err(message) => return failure(message),
    };

    // 代理在调用时读取一次全局配置（对齐基线每次探测前的 `applyProxyConfig()`）。
    let config = crate::commands::read_global_config_at(&app_paths::global_config_path());
    let client = match build_client(proxy_from_config(&config).as_ref()) {
        Ok(client) => client,
        Err(message) => return failure(provider_error_text(message)),
    };

    if is_embedding_probe(&model) {
        let texts = vec![CONNECTION_TEST_PROMPT.to_string()];
        let protocol = model
            .get("protocol")
            .and_then(Value::as_str)
            .unwrap_or("openai");
        return match generate_embeddings(&client, &texts, protocol, &model, None).await {
            Ok(_) => LlmTestConnectionResult {
                success: true,
                error: None,
            },
            Err(error) => failure(error.display_text()),
        };
    }

    let messages = connection_test_messages();
    let response = if is_gemini(&model) {
        gemini::generate(&client, &model, &messages, &options).await
    } else {
        openai::generate(&client, &model, &messages, &options).await
    };
    LlmTestConnectionResult {
        success: response.success,
        error: response.error,
    }
}

/// `llm:discover-models` —— 用**未保存**的表单字段拉取供应商模型列表。
///
/// 只要不抛错就总是返回结构化结果（`success` 判别联合），客户端构建失败按
/// `invalid_response` 归一（基线仅在代理配置非法时可能走到）。
#[tauri::command]
pub async fn llm_discover_models(request: Value) -> ModelDiscoveryResult {
    let config = crate::commands::read_global_config_at(&app_paths::global_config_path());
    let client = match build_discovery_client(proxy_from_config(&config).as_ref()) {
        Ok(client) => client,
        Err(_) => return ModelDiscoveryResult::failed(ModelDiscoveryErrorCode::InvalidResponse),
    };
    discover_models(&client, &request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn generic_model() -> Value {
        json!({
            "id": "m-1",
            "provider": "custom",
            "protocol": "openai",
            "baseUrl": "https://api.example.com/v1",
            "modelName": "gpt-4o-mini",
            "apiKey": "sk-test",
            "temperature": 1,
            "maxTokens": 8192,
            "purposes": ["generation"],
        })
    }

    #[test]
    fn embedding_probe_detection_matches_optional_chaining_test() {
        assert!(!is_embedding_probe(&json!({})), "purposes 缺失");
        assert!(!is_embedding_probe(&json!({"purposes": []})), "空数组");
        assert!(
            !is_embedding_probe(&json!({"purposes": "embedding"})),
            "非数组"
        );
        assert!(!is_embedding_probe(
            &json!({"purposes": ["generation", "summary"]})
        ));
        assert!(is_embedding_probe(
            &json!({"purposes": ["generation", "embedding"]})
        ));
        assert!(is_embedding_probe(&json!({"purposes": ["embedding"]})));
    }

    #[test]
    fn connection_probe_parameter_intent_matches_baseline_literals_test() {
        let request = connection_test_parameter_request("deep-planning");
        assert_eq!(request.max_tokens, Some(CONNECTION_TEST_MAX_TOKENS));
        assert_eq!(request.max_tokens, Some(1024));
        assert_eq!(request.reasoning_stage, Some("general"));
        assert_eq!(request.creative_strategy, Some("deep-planning"));
        assert!(request.response_format.is_none(), "基线未传 responseFormat");

        let messages = connection_test_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Say \"hello\" and nothing else.");

        assert_eq!(DEFAULT_CREATIVE_STRATEGY, "auto");
    }

    #[test]
    fn connection_probe_takes_temperature_from_profile_only_test() {
        let options = resolve_connection_options(&generic_model(), "auto").unwrap();
        assert_eq!(options.temperature, Some(1.0));
        assert_eq!(options.max_tokens, Some(CONNECTION_TEST_MAX_TOKENS));
        assert!(
            options.conversation_id.is_none(),
            "探测不得携带创作会话粘性"
        );
    }

    #[test]
    fn connection_probe_omits_temperature_for_fixed_sampling_family_test() {
        let kimi = json!({
            "provider": "custom",
            "protocol": "openai",
            "baseUrl": "https://api.moonshot.cn/v1",
            "modelName": "kimi-k3",
            "temperature": 1,
            "maxTokens": 4096,
        });
        let options = resolve_connection_options(&kimi, "auto").unwrap();
        assert_eq!(
            options.temperature, None,
            "固定采样家族必须省略 temperature"
        );
    }

    #[test]
    fn connection_probe_reports_invalid_model_settings_with_error_prefix_test() {
        let invalid_kimi = json!({
            "provider": "custom",
            "protocol": "openai",
            "baseUrl": "https://api.moonshot.cn/v1",
            "modelName": "moonshot-v1-8k",
            "temperature": 1.5,
            "maxTokens": 4096,
        });
        let error = resolve_connection_options(&invalid_kimi, "auto").unwrap_err();
        assert_eq!(
            error,
            "Error: Kimi API 的 temperature 必须在 0 到 1 之间。请在模型设置中调整后重试。"
        );
    }

    #[test]
    fn connection_probe_carries_verified_reasoning_directive_test() {
        let deepseek = json!({
            "provider": "deepseek",
            "protocol": "openai",
            "baseUrl": "https://api.deepseek.com",
            "modelName": "deepseek-v4-flash",
            "temperature": 0.7,
            "maxTokens": 4096,
        });
        // `general` 阶段与生成链共用同一推理策略缝（不是第二份映射表）。
        let probe = resolve_connection_options(&deepseek, "auto").unwrap();
        let review = resolve_generation_parameters(
            &GenerationParameterModel::from_value(&deepseek),
            &GenerationParameterRequest {
                max_tokens: None,
                response_format: None,
                creative_strategy: Some("auto"),
                reasoning_stage: Some("review"),
            },
        )
        .unwrap();
        assert!(probe.reasoning.is_some(), "内置端点必须带出已验证推理指令");
        assert_ne!(
            probe.reasoning, review.reasoning,
            "受控阶段必须影响推理指令（探测用 general，审阅用 review）"
        );
    }
}
