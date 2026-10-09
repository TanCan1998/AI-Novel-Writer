//! 生成参数策略 —— 迁移自 `electron/llm/generation-parameter-policy.ts`。
//!
//! 不变量：**模型档案的 `temperature` 是唯一用户可见的采样输入**。调用方只能选择
//! 工作流特征（创作策略 / 受控阶段）与输出预算，不能覆盖采样温度。`None` 表示
//! provider **必须省略**该字段（而不是回退到某个默认值），这是与部分端点
//! （固定采样参数家族）正确对话的前提。

use crate::llm::presets::ModelCapabilityProfile;
use crate::llm::reasoning::{resolve_reasoning_policy, ProviderReasoningDirective};
use serde_json::Value;

/// 从模型档案解析出的有效请求参数。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedGenerationParameters {
    /// `None` = 省略该字段。
    pub temperature: Option<f64>,
    /// `None` = 调用方与档案都未给出预算（provider 自行决定）。
    pub max_tokens: Option<u64>,
    /// 结构化输出要求，原样透传给 provider 载荷。
    pub response_format: Option<Value>,
    pub reasoning: Option<ProviderReasoningDirective>,
}

/// 参与参数解析的模型档案字段。
#[derive(Debug, Clone, Copy)]
pub struct GenerationParameterModel<'a> {
    pub provider: Option<&'a str>,
    pub protocol: Option<&'a str>,
    pub base_url: &'a str,
    pub model_name: &'a str,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub reasoning_override: Option<&'a str>,
}

impl<'a> GenerationParameterModel<'a> {
    /// 从模型 JSON 条目读取缺失字段（一律按缺省处理，避免旧配置解析失败）。
    pub fn from_value(model: &'a Value) -> Self {
        GenerationParameterModel {
            provider: model.get("provider").and_then(|value| value.as_str()),
            protocol: model.get("protocol").and_then(|value| value.as_str()),
            base_url: model
                .get("baseUrl")
                .and_then(|value| value.as_str())
                .unwrap_or_default(),
            model_name: model
                .get("modelName")
                .and_then(|value| value.as_str())
                .unwrap_or_default(),
            temperature: model.get("temperature").and_then(|value| value.as_f64()),
            max_tokens: model.get("maxTokens").and_then(|value| value.as_u64()),
            reasoning_override: model
                .get("reasoningOverride")
                .and_then(|value| value.as_str()),
        }
    }

    fn capability_profile(&self) -> ModelCapabilityProfile<'a> {
        ModelCapabilityProfile {
            provider: self.provider,
            protocol: self.protocol,
            base_url: Some(self.base_url),
            model_name: Some(self.model_name),
        }
    }
}

/// 调用方给出的参数意图。
#[derive(Debug, Clone, Default)]
pub struct GenerationParameterRequest<'a> {
    pub max_tokens: Option<u64>,
    pub response_format: Option<Value>,
    pub creative_strategy: Option<&'a str>,
    pub reasoning_stage: Option<&'a str>,
}

/// 官方 Kimi 端点（仅这两个主机名被认可为官方）。
const OFFICIAL_KIMI_HOSTS: [&str; 2] = ["api.moonshot.cn", "api.moonshot.ai"];

/// 采样温度由端点固定的模型家族前缀。
const KIMI_FIXED_TEMPERATURE_MODEL_PREFIXES: [&str; 4] =
    ["kimi-k3", "kimi-k2.7", "kimi-k2.6", "kimi-k2.5"];

/// 采样温度非法时的用户可见文案（命令层会再加 `Error: ` 前缀）。
pub const KIMI_TEMPERATURE_MESSAGE: &str =
    "Kimi API 的 temperature 必须在 0 到 1 之间。请在模型设置中调整后重试。";

fn has_model_family_prefix(model_name: &str, prefixes: &[&str]) -> bool {
    let normalized = model_name.trim().to_lowercase();
    prefixes
        .iter()
        .any(|prefix| normalized == *prefix || normalized.starts_with(&format!("{prefix}-")))
}

/// 是否官方 Kimi 端点（要求 https 且主机名精确匹配，路径不限）。
fn is_official_kimi_host(base_url: &str) -> bool {
    let Ok(endpoint) = url::Url::parse(base_url.trim()) else {
        return false;
    };
    if endpoint.scheme() != "https" {
        return false;
    }
    match endpoint.host_str() {
        Some(host) => OFFICIAL_KIMI_HOSTS.contains(&host.to_lowercase().as_str()),
        None => false,
    }
}

fn validate_kimi_temperature(temperature: Option<f64>) -> Result<(), String> {
    match temperature {
        Some(value) if value.is_finite() && (0.0..=1.0).contains(&value) => Ok(()),
        _ => Err(KIMI_TEMPERATURE_MESSAGE.to_string()),
    }
}

/// 解析有效请求参数。
///
/// 返回 `Err` 仅在模型档案自身非法（官方 Kimi 端点的温度越界）时发生 —— 这类失败
/// 必须**阻断**调用，而不是静默回退到默认温度。
pub fn resolve_generation_parameters(
    model: &GenerationParameterModel<'_>,
    request: &GenerationParameterRequest<'_>,
) -> Result<ResolvedGenerationParameters, String> {
    let is_official_kimi = is_official_kimi_host(model.base_url);
    let uses_fixed_kimi_temperature = is_official_kimi
        && has_model_family_prefix(model.model_name, &KIMI_FIXED_TEMPERATURE_MODEL_PREFIXES);

    if is_official_kimi && !uses_fixed_kimi_temperature {
        validate_kimi_temperature(model.temperature)?;
    }

    let resolution = resolve_reasoning_policy(
        &model.capability_profile(),
        model.reasoning_override,
        request.creative_strategy,
        request.reasoning_stage,
    );

    Ok(ResolvedGenerationParameters {
        temperature: if uses_fixed_kimi_temperature {
            None
        } else {
            model.temperature
        },
        max_tokens: request.max_tokens.or(model.max_tokens),
        response_format: request.response_format.clone(),
        reasoning: resolution.provider_directive,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::reasoning::ReasoningEffort;
    use serde_json::json;

    fn deepseek_model() -> Value {
        json!({
            "id": "m-1",
            "provider": "deepseek",
            "protocol": "openai",
            "baseUrl": "https://api.deepseek.com",
            "modelName": "deepseek-v4-flash",
            "temperature": 0.7,
            "maxTokens": 4096,
        })
    }

    fn request<'a>() -> GenerationParameterRequest<'a> {
        GenerationParameterRequest {
            max_tokens: None,
            response_format: None,
            creative_strategy: Some("auto"),
            reasoning_stage: Some("drafting"),
        }
    }

    #[test]
    fn temperature_comes_from_profile_only_test() {
        let model = deepseek_model();
        let resolved = resolve_generation_parameters(
            &GenerationParameterModel::from_value(&model),
            &request(),
        )
        .unwrap();
        assert_eq!(resolved.temperature, Some(0.7));
        assert_eq!(resolved.max_tokens, Some(4096), "缺省预算来自档案");
    }

    #[test]
    fn request_budget_wins_and_absent_budget_stays_absent_test() {
        let model = deepseek_model();
        let with_budget = GenerationParameterRequest {
            max_tokens: Some(1024),
            ..request()
        };
        let resolved = resolve_generation_parameters(
            &GenerationParameterModel::from_value(&model),
            &with_budget,
        )
        .unwrap();
        assert_eq!(resolved.max_tokens, Some(1024));

        let bare = json!({
            "provider": "deepseek",
            "protocol": "openai",
            "baseUrl": "https://api.deepseek.com",
            "modelName": "deepseek-v4-flash",
        });
        let without_limits =
            resolve_generation_parameters(&GenerationParameterModel::from_value(&bare), &request())
                .unwrap();
        assert_eq!(
            without_limits.temperature, None,
            "缺省温度必须省略字段而非回退"
        );
        assert_eq!(without_limits.max_tokens, None);
    }

    #[test]
    fn verified_mapping_is_attached_for_builtin_endpoint_test() {
        let model = deepseek_model();
        let resolved = resolve_generation_parameters(
            &GenerationParameterModel::from_value(&model),
            &request(),
        )
        .unwrap();
        assert!(
            resolved.reasoning.is_some(),
            "内置端点必须带出已验证推理指令"
        );
    }

    #[test]
    fn unverified_endpoint_omits_reasoning_test() {
        let model = json!({
            "provider": "custom",
            "protocol": "openai",
            "baseUrl": "https://relay.example.com/v1",
            "modelName": "deepseek-v4-flash",
            "temperature": 0.5,
            "maxTokens": 4096,
        });
        let resolved = resolve_generation_parameters(
            &GenerationParameterModel::from_value(&model),
            &request(),
        )
        .unwrap();
        assert!(resolved.reasoning.is_none(), "无协议证据时必须省略推理字段");
        assert_eq!(resolved.temperature, Some(0.5));
    }

    #[test]
    fn official_kimi_official_model_omits_temperature_test() {
        for model_name in [
            "kimi-k2.5",
            "kimi-k2.5-preview",
            "KIMI-K3",
            "  kimi-k2.7  ",
            "kimi-k2.6-turbo",
        ] {
            let model = json!({
                "provider": "custom",
                "protocol": "openai",
                "baseUrl": "https://api.moonshot.cn/v1",
                "modelName": model_name,
                "temperature": 0.7,
                "maxTokens": 4096,
            });
            let resolved = resolve_generation_parameters(
                &GenerationParameterModel::from_value(&model),
                &request(),
            )
            .unwrap_or_else(|error| panic!("{model_name} 属于固定采样家族，不应报错：{error}"));
            assert_eq!(
                resolved.temperature, None,
                "{model_name} 属于固定采样家族，必须省略 temperature"
            );
        }
    }

    #[test]
    fn official_kimi_other_models_validate_temperature_test() {
        let out_of_range = json!({
            "provider": "custom",
            "protocol": "openai",
            "baseUrl": "https://api.moonshot.cn/v1",
            "modelName": "moonshot-v1-8k",
            "temperature": 1.5,
            "maxTokens": 4096,
        });
        let error = resolve_generation_parameters(
            &GenerationParameterModel::from_value(&out_of_range),
            &request(),
        )
        .unwrap_err();
        assert_eq!(error, KIMI_TEMPERATURE_MESSAGE);

        let missing = json!({
            "provider": "custom",
            "protocol": "openai",
            "baseUrl": "https://api.moonshot.cn/v1",
            "modelName": "moonshot-v1-8k",
            "maxTokens": 4096,
        });
        assert!(resolve_generation_parameters(
            &GenerationParameterModel::from_value(&missing),
            &request()
        )
        .is_err());

        let in_range = json!({
            "provider": "custom",
            "protocol": "openai",
            "baseUrl": "https://api.moonshot.cn/v1",
            "modelName": "moonshot-v1-8k",
            "temperature": 1,
            "maxTokens": 4096,
        });
        assert!(
            resolve_generation_parameters(
                &GenerationParameterModel::from_value(&in_range),
                &request()
            )
            .is_ok(),
            "边界值 1 必须被接受"
        );
    }

    #[test]
    fn kimi_rules_only_apply_to_official_https_hosts_test() {
        for base_url in [
            "http://api.moonshot.cn/v1",
            "https://api.moonshot.com.cn/v1",
            "https://api.moonshot.cn.evil.example/v1",
            "https://my-proxy.example.com/v1",
        ] {
            let model = json!({
                "provider": "custom",
                "protocol": "openai",
                "baseUrl": base_url,
                "modelName": "kimi-k2.5",
                "temperature": 0.7,
                "maxTokens": 4096,
            });
            let resolved = resolve_generation_parameters(
                &GenerationParameterModel::from_value(&model),
                &request(),
            )
            .unwrap();
            assert_eq!(
                resolved.temperature,
                Some(0.7),
                "{base_url} 不是官方 Kimi 端点，不得触发固定温度规则"
            );
        }
    }

    #[test]
    fn response_format_is_passed_through_verbatim_test() {
        let model = deepseek_model();
        let with_format = GenerationParameterRequest {
            response_format: Some(json!({ "type": "json_object" })),
            ..request()
        };
        let resolved = resolve_generation_parameters(
            &GenerationParameterModel::from_value(&model),
            &with_format,
        )
        .unwrap();
        assert_eq!(
            resolved.response_format,
            Some(json!({ "type": "json_object" }))
        );

        let empty = resolve_generation_parameters(
            &GenerationParameterModel::from_value(&model),
            &request(),
        )
        .unwrap();
        assert!(empty.response_format.is_none(), "未给出时必须整体缺省");
    }

    #[test]
    fn stage_selects_reasoning_effort_for_builtin_endpoint_test() {
        let model = deepseek_model();
        let drafting = resolve_generation_parameters(
            &GenerationParameterModel::from_value(&model),
            &GenerationParameterRequest {
                reasoning_stage: Some("drafting"),
                creative_strategy: Some("auto"),
                ..request()
            },
        )
        .unwrap();
        let review = resolve_generation_parameters(
            &GenerationParameterModel::from_value(&model),
            &GenerationParameterRequest {
                reasoning_stage: Some("review"),
                creative_strategy: Some("auto"),
                ..request()
            },
        )
        .unwrap();
        assert_ne!(drafting.reasoning, review.reasoning, "阶段必须影响推理指令");

        // review 阶段（high）与 override=off 必须给出不同指令
        let overridden = resolve_generation_parameters(
            &GenerationParameterModel::from_value(&json!({
                "provider": "deepseek",
                "protocol": "openai",
                "baseUrl": "https://api.deepseek.com",
                "modelName": "deepseek-v4-flash",
                "temperature": 0.7,
                "maxTokens": 4096,
                "reasoningOverride": "off",
            })),
            &GenerationParameterRequest {
                reasoning_stage: Some("review"),
                creative_strategy: Some("auto"),
                ..request()
            },
        )
        .unwrap();
        assert_ne!(overridden.reasoning, review.reasoning);
        assert_eq!(
            overridden.reasoning.unwrap(),
            crate::llm::reasoning::ProviderReasoningDirective::DeepSeekV4Thinking {
                thinking: crate::llm::reasoning::ThinkingState::Disabled,
                reasoning_effort: None,
            }
        );
    }

    #[test]
    fn effort_type_is_shared_with_reasoning_module_test() {
        // 编译期对齐：`ReasoningEffort` 在同一命名空间内被两个模块共用
        assert_eq!(ReasoningEffort::Off.as_str(), "off");
    }
}
