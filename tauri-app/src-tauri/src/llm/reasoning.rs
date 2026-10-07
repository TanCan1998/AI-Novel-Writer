//! 推理策略缝 —— 迁移自 `src/shared/reasoning-policy.ts` + `src/shared/reasoning-types.ts`。
//!
//! 该模块是**唯一的推理决策点**：调用方只提供产品意图（创作策略 + 受控语义阶段），
//! 模型档案优先级、已验证能力查表、provider 级封顶与用户可见的解析回执全部由这里拥有。
//! 任何调用方都不得自行推断推理强度（`docs/product-domain.md` 提示词合同）。

use serde::{Deserialize, Serialize};

use crate::llm::presets::{ModelCapabilityProfile, PresetValue, VerifiedReasoningMapping};

/// 创作策略（产品意图）。`auto` 表示由阶段表决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CreativeStrategy {
    Auto,
    FluentDrafting,
    ConsistencyFirst,
    DeepPlanning,
}

impl CreativeStrategy {
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("fluent-drafting") => CreativeStrategy::FluentDrafting,
            Some("consistency-first") => CreativeStrategy::ConsistencyFirst,
            Some("deep-planning") => CreativeStrategy::DeepPlanning,
            _ => CreativeStrategy::Auto,
        }
    }
}

/// 受控语义阶段 —— 由产品调用方给出，**永不**从诊断用 `purpose` 文案推断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GenerationReasoningStage {
    Drafting,
    Planning,
    Review,
    General,
}

impl GenerationReasoningStage {
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("drafting") => GenerationReasoningStage::Drafting,
            Some("planning") => GenerationReasoningStage::Planning,
            Some("review") => GenerationReasoningStage::Review,
            _ => GenerationReasoningStage::General,
        }
    }
}

/// 推理强度（产品侧命名空间）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Off,
    Low,
    Medium,
    High,
    Max,
}

impl ReasoningEffort {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(ReasoningEffort::Off),
            "low" => Some(ReasoningEffort::Low),
            "medium" => Some(ReasoningEffort::Medium),
            "high" => Some(ReasoningEffort::High),
            "max" => Some(ReasoningEffort::Max),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ReasoningEffort::Off => "off",
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
            ReasoningEffort::Max => "max",
        }
    }

    /// 强度排序（对齐基线 `EFFORT_RANK`）。
    fn rank(self) -> u8 {
        match self {
            ReasoningEffort::Off => 0,
            ReasoningEffort::Low => 1,
            ReasoningEffort::Medium => 2,
            ReasoningEffort::High => 3,
            ReasoningEffort::Max => 4,
        }
    }

    /// 仅 `low` / `medium` / `high` 可作为 OpenAI `reasoning_effort` 取值。
    fn parse_openai_reasoning_effort(value: &str) -> Option<Self> {
        match value {
            "low" | "medium" | "high" => ReasoningEffort::parse(value),
            _ => None,
        }
    }
}

/// provider 请求适配器家族。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReasoningAdapter {
    #[serde(rename = "openai-reasoning-effort")]
    OpenAiReasoningEffort,
    #[serde(rename = "gemini-thinking-budget")]
    GeminiThinkingBudget,
    #[serde(rename = "deepseek-v4-thinking")]
    DeepSeekV4Thinking,
}

/// 深寻思考开关状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingState {
    Disabled,
    Enabled,
}

/// provider 级推理指令 —— 唯一允许进入请求载荷的推理表达形式。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "adapter")]
pub enum ProviderReasoningDirective {
    #[serde(rename = "openai-reasoning-effort")]
    OpenAiReasoningEffort {
        #[serde(rename = "reasoningEffort")]
        reasoning_effort: ReasoningEffort,
    },
    #[serde(rename = "gemini-thinking-budget")]
    GeminiThinkingBudget {
        #[serde(rename = "thinkingBudget")]
        thinking_budget: i64,
    },
    #[serde(rename = "deepseek-v4-thinking")]
    DeepSeekV4Thinking {
        thinking: ThinkingState,
        #[serde(rename = "reasoningEffort", skip_serializing_if = "Option::is_none")]
        reasoning_effort: Option<ReasoningEffort>,
    },
}

/// 用户可见的解析结果状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningResolutionStatus {
    Mapped,
    Capped,
    Forced,
    Unsupported,
}

/// 决策来源：项目策略 or 模型档案覆盖。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReasoningSource {
    ProjectStrategy,
    ModelOverride,
}

/// 推理策略解析回执。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningPolicyResolution {
    pub requested: ReasoningEffort,
    pub effective: Option<ReasoningEffort>,
    pub status: ReasoningResolutionStatus,
    pub source: ReasoningSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_directive: Option<ProviderReasoningDirective>,
}

/// 阶段请求表（对齐基线 `STAGE_REQUESTS`）。
fn stage_request(strategy: CreativeStrategy, stage: GenerationReasoningStage) -> ReasoningEffort {
    use CreativeStrategy as S;
    use GenerationReasoningStage as G;
    use ReasoningEffort as E;
    match (strategy, stage) {
        (S::Auto, G::Drafting) => E::Low,
        (S::Auto, G::Planning) => E::Medium,
        (S::Auto, G::Review) => E::High,
        (S::Auto, G::General) => E::Low,
        (S::FluentDrafting, G::Drafting) => E::Off,
        (S::FluentDrafting, G::Planning) => E::Low,
        (S::FluentDrafting, G::Review) => E::Low,
        (S::FluentDrafting, G::General) => E::Low,
        (S::ConsistencyFirst, G::Drafting) => E::Low,
        (S::ConsistencyFirst, G::Planning) => E::High,
        (S::ConsistencyFirst, G::Review) => E::High,
        (S::ConsistencyFirst, G::General) => E::Medium,
        (S::DeepPlanning, G::Drafting) => E::Low,
        (S::DeepPlanning, G::Planning) => E::Max,
        (S::DeepPlanning, G::Review) => E::High,
        (S::DeepPlanning, G::General) => E::Medium,
    }
}

/// 把「解析后的有效强度」翻译为 provider 指令；无法表达即返回 `None`（= unsupported）。
fn provider_directive(
    mapping: &VerifiedReasoningMapping,
    effective: ReasoningEffort,
) -> Option<ProviderReasoningDirective> {
    let value = mapping.provider_value(effective);
    match mapping.adapter {
        ReasoningAdapter::OpenAiReasoningEffort => {
            let text = value?.as_str()?;
            let effort = ReasoningEffort::parse_openai_reasoning_effort(text)?;
            Some(ProviderReasoningDirective::OpenAiReasoningEffort {
                reasoning_effort: effort,
            })
        }
        ReasoningAdapter::DeepSeekV4Thinking => {
            // `off` 只接受文档化的 `disabled` 映射；其余强度要求 provider 取值与强度同名。
            if effective == ReasoningEffort::Off && value.and_then(PresetValue::as_str) == Some("disabled") {
                return Some(ProviderReasoningDirective::DeepSeekV4Thinking {
                    thinking: ThinkingState::Disabled,
                    reasoning_effort: None,
                });
            }
            let matches_effort = matches!(
                effective,
                ReasoningEffort::Low | ReasoningEffort::High | ReasoningEffort::Max
            ) && value.and_then(PresetValue::as_str) == Some(effective.as_str());
            if !matches_effort {
                return None;
            }
            Some(ProviderReasoningDirective::DeepSeekV4Thinking {
                thinking: ThinkingState::Enabled,
                reasoning_effort: Some(effective),
            })
        }
        ReasoningAdapter::GeminiThinkingBudget => {
            let budget = value?.as_number()?;
            Some(ProviderReasoningDirective::GeminiThinkingBudget {
                thinking_budget: budget,
            })
        }
    }
}

/// 在已验证映射内寻找最接近的可用强度（`mapped` / `capped` / `forced`）。
fn closest_effective_effort(
    requested: ReasoningEffort,
    mapping: &VerifiedReasoningMapping,
) -> Option<(ReasoningEffort, ReasoningResolutionStatus)> {
    if let Some(alias) = mapping.request_alias(requested) {
        if mapping.supported_efforts.contains(&alias) {
            return Some((alias, ReasoningResolutionStatus::Mapped));
        }
    }
    if mapping.supported_efforts.contains(&requested) {
        return Some((requested, ReasoningResolutionStatus::Mapped));
    }

    let mut supported = mapping.supported_efforts.clone();
    supported.sort_by_key(|effort| effort.rank());
    if supported.is_empty() {
        return None;
    }
    if requested == ReasoningEffort::Off {
        return Some((supported[0], ReasoningResolutionStatus::Forced));
    }

    let lower_or_equal: Vec<ReasoningEffort> = supported
        .iter()
        .copied()
        .filter(|effort| effort.rank() <= requested.rank())
        .collect();
    let effective = lower_or_equal
        .last()
        .copied()
        .unwrap_or(supported[0]);
    Some((effective, ReasoningResolutionStatus::Capped))
}

/// 解析模型档案上的覆盖值：`auto` / 合法强度 → 原样，其余（含缺失）→ `auto`。
fn resolve_override(value: Option<&str>) -> Option<ReasoningEffort> {
    match value {
        Some(text) if text != "auto" => ReasoningEffort::parse(text),
        _ => None,
    }
}

/// 单一推理策略缝：产品意图 → provider 指令 + 用户可见回执。
///
/// 返回 `unsupported` 表示该端点没有被验证过的推理映射，调用方必须**省略**推理字段，
/// 而不是猜测一个默认值。
pub fn resolve_reasoning_policy(
    profile: &ModelCapabilityProfile<'_>,
    reasoning_override: Option<&str>,
    creative_strategy: Option<&str>,
    stage: Option<&str>,
) -> ReasoningPolicyResolution {
    let strategy = CreativeStrategy::parse(creative_strategy);
    let override_effort = resolve_override(reasoning_override);
    let source = if override_effort.is_some() {
        ReasoningSource::ModelOverride
    } else {
        ReasoningSource::ProjectStrategy
    };
    let requested = override_effort.unwrap_or_else(|| {
        stage_request(strategy, GenerationReasoningStage::parse(stage))
    });

    let unsupported = |requested: ReasoningEffort| ReasoningPolicyResolution {
        requested,
        effective: None,
        status: ReasoningResolutionStatus::Unsupported,
        source,
        provider_directive: None,
    };

    let Some(mapping) = crate::llm::presets::resolve_model_profile_reasoning_mapping(profile) else {
        return unsupported(requested);
    };
    let Some((effective, status)) = closest_effective_effort(requested, &mapping) else {
        return unsupported(requested);
    };
    let Some(directive) = provider_directive(&mapping, effective) else {
        return unsupported(requested);
    };
    ReasoningPolicyResolution {
        requested,
        effective: Some(effective),
        status,
        source,
        provider_directive: Some(directive),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile<'a>(
        provider: &'a str,
        protocol: &'a str,
        base_url: &'a str,
        model_name: &'a str,
    ) -> ModelCapabilityProfile<'a> {
        ModelCapabilityProfile {
            provider: Some(provider),
            protocol: Some(protocol),
            base_url: Some(base_url),
            model_name: Some(model_name),
        }
    }

    fn deepseek_profile<'a>() -> ModelCapabilityProfile<'a> {
        profile(
            "deepseek",
            "openai",
            "https://api.deepseek.com",
            "deepseek-v4-flash",
        )
    }

    #[test]
    fn project_strategy_without_verified_mapping_is_unsupported_test() {
        // 未在内置目录中的端点 → 没有协议证据 → 必须 unsupported
        let result = resolve_reasoning_policy(
            &profile("custom", "openai", "https://example.com/v1", "any-model"),
            None,
            Some("deep-planning"),
            Some("planning"),
        );
        assert_eq!(result.requested, ReasoningEffort::Max, "阶段表仍须给出请求强度");
        assert_eq!(result.effective, None);
        assert_eq!(result.status, ReasoningResolutionStatus::Unsupported);
        assert_eq!(result.source, ReasoningSource::ProjectStrategy);
        assert!(result.provider_directive.is_none());
    }

    #[test]
    fn verified_deepseek_mapping_caps_and_aliases_test() {
        // medium 通过 requestAliases → high（mapped）
        let mapped = resolve_reasoning_policy(&deepseek_profile(), None, Some("auto"), Some("planning"));
        assert_eq!(mapped.requested, ReasoningEffort::Medium);
        assert_eq!(mapped.effective, Some(ReasoningEffort::High));
        assert_eq!(mapped.status, ReasoningResolutionStatus::Mapped);
        assert_eq!(
            mapped.provider_directive,
            Some(ProviderReasoningDirective::DeepSeekV4Thinking {
                thinking: ThinkingState::Enabled,
                reasoning_effort: Some(ReasoningEffort::High),
            })
        );

        // auto/review → high（直接支持）
        let direct = resolve_reasoning_policy(&deepseek_profile(), None, Some("auto"), Some("review"));
        assert_eq!(direct.effective, Some(ReasoningEffort::High));
        assert_eq!(direct.status, ReasoningResolutionStatus::Mapped);

        // auto/drafting → low（直接支持）
        let low = resolve_reasoning_policy(&deepseek_profile(), None, Some("auto"), Some("drafting"));
        assert_eq!(low.effective, Some(ReasoningEffort::Low));
        assert_eq!(low.status, ReasoningResolutionStatus::Mapped);
    }

    #[test]
    fn model_override_off_maps_to_disabled_thinking_test() {
        let result = resolve_reasoning_policy(&deepseek_profile(), Some("off"), Some("auto"), Some("review"));
        assert_eq!(result.source, ReasoningSource::ModelOverride);
        assert_eq!(result.requested, ReasoningEffort::Off);
        assert_eq!(result.effective, Some(ReasoningEffort::Off));
        assert_eq!(
            result.provider_directive,
            Some(ProviderReasoningDirective::DeepSeekV4Thinking {
                thinking: ThinkingState::Disabled,
                reasoning_effort: None,
            })
        );
    }

    #[test]
    fn invalid_override_falls_back_to_project_strategy_test() {
        let result = resolve_reasoning_policy(
            &deepseek_profile(),
            Some("ultra"),
            Some("auto"),
            Some("review"),
        );
        assert_eq!(result.source, ReasoningSource::ProjectStrategy, "非法覆盖值不得生效");
        assert_eq!(result.requested, ReasoningEffort::High);
    }

    #[test]
    fn openai_reasoning_effort_only_emits_documented_values_test() {
        let xai = profile("xai", "openai", "https://api.x.ai/v1", "grok-4.5");
        // deep-planning/planning 请求 max，但 grok 只支持 low/medium/high → capped 到 high
        let capped = resolve_reasoning_policy(&xai, None, Some("deep-planning"), Some("planning"));
        assert_eq!(capped.requested, ReasoningEffort::Max);
        assert_eq!(capped.effective, Some(ReasoningEffort::High));
        assert_eq!(capped.status, ReasoningResolutionStatus::Capped);
        assert_eq!(
            capped.provider_directive,
            Some(ProviderReasoningDirective::OpenAiReasoningEffort {
                reasoning_effort: ReasoningEffort::High,
            })
        );

        // fluent-drafting/drafting 请求 off → 不支持 off → forced 到最低支持值 low
        let forced = resolve_reasoning_policy(&xai, None, Some("fluent-drafting"), Some("drafting"));
        assert_eq!(forced.requested, ReasoningEffort::Off);
        assert_eq!(forced.effective, Some(ReasoningEffort::Low));
        assert_eq!(forced.status, ReasoningResolutionStatus::Forced);
    }

    #[test]
    fn gemini_thinking_budget_is_passed_through_test() {
        let gemini = profile(
            "gemini",
            "gemini",
            "https://generativelanguage.googleapis.com",
            "gemini-2.5-flash-lite",
        );
        let result = resolve_reasoning_policy(&gemini, None, Some("auto"), Some("planning"));
        assert_eq!(result.requested, ReasoningEffort::Medium);
        assert_eq!(
            result.provider_directive,
            Some(ProviderReasoningDirective::GeminiThinkingBudget {
                thinking_budget: 8192,
            })
        );

        let off = resolve_reasoning_policy(&gemini, Some("off"), None, None);
        assert_eq!(
            off.provider_directive,
            Some(ProviderReasoningDirective::GeminiThinkingBudget {
                thinking_budget: 0,
            })
        );
    }

    #[test]
    fn stage_defaults_to_general_when_absent_test() {
        let result = resolve_reasoning_policy(&deepseek_profile(), None, None, None);
        // auto + general → low
        assert_eq!(result.requested, ReasoningEffort::Low);
    }

    #[test]
    fn preset_lookup_requires_matching_endpoint_and_protocol_test() {
        // 协议不符（openai vs gemini）→ 无映射
        let wrong_protocol = profile(
            "gemini",
            "openai",
            "https://generativelanguage.googleapis.com",
            "gemini-2.5-flash-lite",
        );
        assert_eq!(
            resolve_reasoning_policy(&wrong_protocol, None, None, None).status,
            ReasoningResolutionStatus::Unsupported
        );

        // 端点不符（自建代理）→ 无映射
        let wrong_endpoint = profile(
            "deepseek",
            "openai",
            "https://my-proxy.example.com/v1",
            "deepseek-v4-flash",
        );
        assert_eq!(
            resolve_reasoning_policy(&wrong_endpoint, None, None, None).status,
            ReasoningResolutionStatus::Unsupported
        );

        // 端点带尾斜杠 / 大小写差异仍算同一端点
        let equivalent = profile(
            "deepseek",
            "openai",
            "HTTPS://API.DEEPSEEK.COM/",
            "deepseek-v4-flash",
        );
        assert_eq!(
            resolve_reasoning_policy(&equivalent, None, None, None).status,
            ReasoningResolutionStatus::Mapped
        );
    }
}
