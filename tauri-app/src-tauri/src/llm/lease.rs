//! 模型执行租约 —— 迁移自 `electron/services/model-execution-lease.ts`。
//!
//! 租约把「模型档案快照」冻结在主进程内：生成期间配置变更不会中途改变请求参数，
//! 回执本身**不含密钥**，只携带可核对的指纹与能力证据。
//!
//! 哈希口径说明：基线用 `JSON.stringify` + sha256（JS 对象键序 = 插入序）。本
//! Rust 侧改用**规范化 JSON**（对象键排序、无空白），因此与 Node 的哈希**不要求
//! 逐字节一致**（见 `AGENTS.md` 的双栈隔离约定）；但同一进程内必须是确定性的，
//! 且与 `serde_json` 的 `preserve_order` feature 是否被其它依赖开启无关。

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::llm::presets::{
    resolve_model_profile_capabilities, ModelCapabilityProfile, MAX_SAFE_INTEGER,
};

/// 租约默认有效期：4 小时（对齐基线 `DEFAULT_MODEL_EXECUTION_LEASE_TTL_MS`）。
pub const DEFAULT_MODEL_EXECUTION_LEASE_TTL_MS: u64 = 4 * 60 * 60 * 1_000;

/// 已关闭租约墓碑的存活时间：5 分钟。用于把「重复关闭」判为幂等成功。
pub const CLOSED_EXECUTION_LEASE_TOMBSTONE_TTL_MS: u64 = 5 * 60 * 1_000;

/// 能力证据来源。`verified-provider-preset` 是唯一的协议证据等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityEvidenceSource {
    VerifiedProviderPreset,
    UserOperationalCap,
    LegacyProfile,
    Unknown,
}

/// 各字段的证据来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityEvidenceSourceSet {
    pub context_window_tokens: CapabilityEvidenceSource,
    pub max_output_tokens: CapabilityEvidenceSource,
    pub feature_flags: CapabilityEvidenceSource,
}

/// 非密钥的能力证据（随回执冻结返回）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelExecutionCapabilityEvidence {
    pub source: CapabilityEvidenceSourceSet,
    pub subject_fingerprint: String,
    pub context_window_tokens: Option<u64>,
    pub max_output_tokens: u64,
    pub reasoning: Option<bool>,
    pub structured_output: Option<bool>,
    pub usage: Option<bool>,
}

/// 模型执行租约回执（完整模型快照的非密钥证据）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelExecutionLeaseReceipt {
    pub lease_id: String,
    pub model_id: String,
    pub provider: String,
    pub protocol: String,
    pub model_name: String,
    pub model_revision: String,
    pub endpoint_fingerprint: String,
    pub capability_evidence: ModelExecutionCapabilityEvidence,
    pub created_at: u64,
    pub expires_at: u64,
}

/// 租约失败原因（对齐基线 `ModelExecutionLeaseError.code`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelExecutionLeaseError {
    ModelNotFound,
    InvalidOutputCapability,
}

impl ModelExecutionLeaseError {
    pub fn message(self) -> &'static str {
        match self {
            ModelExecutionLeaseError::ModelNotFound => "未找到模型配置",
            ModelExecutionLeaseError::InvalidOutputCapability => "模型输出上限无效",
        }
    }
}

impl std::fmt::Display for ModelExecutionLeaseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

/// 参与租约构建的模型档案字段（缺省一律容错，避免旧配置解析失败）。
#[derive(Debug, Clone, Copy)]
pub struct LeaseModelProfile<'a> {
    pub id: &'a str,
    pub provider: &'a str,
    pub protocol: &'a str,
    pub base_url: &'a str,
    pub model_name: &'a str,
    pub temperature: Option<&'a serde_json::Value>,
    pub max_tokens: Option<&'a serde_json::Value>,
    pub capabilities: Option<&'a serde_json::Value>,
    pub purposes: Option<&'a serde_json::Value>,
    pub embedding_options: Option<&'a serde_json::Value>,
    /// 基线 `ModelProfile.reasoningOverride` 的逐字镜像。
    ///
    /// **刻意保留**：租约快照必须与基线 `ModelProfile` 字段集一致（快照是冻结的
    /// 作者事实，不得因当前调用面未消费就删字段）。推理覆盖的**消费点**是
    /// [`crate::llm::params::GenerationParameterModel::from_value`]，它直接读快照
    /// JSON；[`Self::capability_profile`] 按基线语义只透传四项端点身份。
    #[allow(dead_code)]
    pub reasoning_override: Option<&'a str>,
}

impl<'a> LeaseModelProfile<'a> {
    /// 从模型 JSON 条目读取（缺失字段按空串 / `None` 处理）。
    ///
    /// 空串哨兵是安全的：内置目录里不存在空 provider / 空 protocol / 空端点，
    /// 因此「缺字段」与基线的 `undefined` 在能力解析上等价（都拿不到证据）。
    pub fn from_value(model: &'a serde_json::Value) -> Self {
        LeaseModelProfile {
            id: model.get("id").and_then(|value| value.as_str()).unwrap_or_default(),
            provider: model
                .get("provider")
                .and_then(|value| value.as_str())
                .unwrap_or_default(),
            protocol: model
                .get("protocol")
                .and_then(|value| value.as_str())
                .unwrap_or_default(),
            base_url: model
                .get("baseUrl")
                .and_then(|value| value.as_str())
                .unwrap_or_default(),
            model_name: model
                .get("modelName")
                .and_then(|value| value.as_str())
                .unwrap_or_default(),
            temperature: model.get("temperature"),
            max_tokens: model.get("maxTokens"),
            capabilities: model.get("capabilities"),
            purposes: model.get("purposes"),
            embedding_options: model.get("embeddingOptions"),
            reasoning_override: model
                .get("reasoningOverride")
                .and_then(|value| value.as_str()),
        }
    }

    fn capability_profile(&self) -> ModelCapabilityProfile<'a> {
        ModelCapabilityProfile {
            provider: Some(self.provider),
            protocol: Some(self.protocol),
            base_url: Some(self.base_url),
            model_name: Some(self.model_name),
        }
    }
}

fn sha256_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 规范化 JSON：对象键排序、无空白（与 serde_json 的 map 后端无关）。
fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(flag) => flag.to_string(),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::String(text) => {
            serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
        }
        serde_json::Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let body = keys
                .iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_default(),
                        canonical_json(&map[*key])
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
    }
}

/// 端点规范化 —— 去掉 fragment / query，尾斜杠归一。
///
/// 解析失败时退化为「去掉尾斜杠的原始文本」（对齐基线 catch 分支）。
pub fn normalize_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim();
    match url::Url::parse(trimmed) {
        Ok(mut endpoint) => {
            endpoint.set_fragment(None);
            endpoint.set_query(None);
            let trimmed_path = endpoint.path().trim_end_matches('/').to_string();
            let path = if trimmed_path.is_empty() {
                "/".to_string()
            } else {
                trimmed_path
            };
            endpoint.set_path(&path);
            let text = endpoint.to_string();
            text.strip_suffix('/').unwrap_or(&text).to_string()
        }
        Err(_) => trimmed.trim_end_matches('/').to_string(),
    }
}

fn endpoint_subject_text(provider: &str, protocol: &str, base_url: &str) -> String {
    format!(
        "provider={provider}\nprotocol={protocol}\nendpoint={}",
        normalize_endpoint(base_url)
    )
}

/// 端点指纹（不含模型名）。
pub fn endpoint_fingerprint(provider: &str, protocol: &str, base_url: &str) -> String {
    sha256_hex(&endpoint_subject_text(provider, protocol, base_url))
}

/// 主体指纹（端点 + 模型名）。
pub fn subject_fingerprint(
    provider: &str,
    protocol: &str,
    base_url: &str,
    model_name: &str,
) -> String {
    sha256_hex(&format!(
        "{}\nmodelName={model_name}",
        endpoint_subject_text(provider, protocol, base_url)
    ))
}

/// 模型修订指纹：身份 + 端点 + 模型名 + 采样与预算 + 能力 + 用途 + embedding 选项。
fn model_revision(model: &LeaseModelProfile<'_>) -> String {
    let capabilities = canonical_json(
        model
            .capabilities
            .unwrap_or(&serde_json::Value::Null),
    );
    let purposes = canonical_json(model.purposes.unwrap_or(&serde_json::Value::Null));
    let embedding_options = canonical_json(
        model
            .embedding_options
            .unwrap_or(&serde_json::Value::Null),
    );
    let temperature = canonical_json(model.temperature.unwrap_or(&serde_json::Value::Null));
    let max_tokens = canonical_json(model.max_tokens.unwrap_or(&serde_json::Value::Null));
    sha256_hex(&format!(
        "id={}\n{}\nmodelName={}\ntemperature={temperature}\nmaxTokens={max_tokens}\ncapabilities={capabilities}\npurposes={purposes}\nembeddingOptions={embedding_options}",
        model.id,
        endpoint_subject_text(model.provider, model.protocol, model.base_url),
        model.model_name
    ))
}

/// 正整数读取（对齐 `Number.isSafeInteger(x) && x > 0`）。
fn positive_safe_integer(value: Option<&serde_json::Value>) -> Option<u64> {
    let raw = value?;
    let number = raw
        .as_u64()
        .or_else(|| raw.as_f64().filter(|float| float.fract() == 0.0).map(|float| float as u64))?;
    if number == 0 || number > MAX_SAFE_INTEGER {
        return None;
    }
    Some(number)
}

fn capability_field<'a>(
    capabilities: Option<&'a serde_json::Value>,
    key: &str,
) -> Option<&'a serde_json::Value> {
    capabilities?.get(key)
}

/// 构建能力证据（有效输出上限不可推导时返回 `InvalidOutputCapability`）。
pub fn resolve_model_execution_capability_evidence(
    model: &LeaseModelProfile<'_>,
) -> Result<ModelExecutionCapabilityEvidence, ModelExecutionLeaseError> {
    let subject = subject_fingerprint(
        model.provider,
        model.protocol,
        model.base_url,
        model.model_name,
    );
    let verified = resolve_model_profile_capabilities(&model.capability_profile());
    let explicit_context_window =
        positive_safe_integer(capability_field(model.capabilities, "contextWindowTokens"));
    let explicit_output_cap =
        positive_safe_integer(capability_field(model.capabilities, "maxOutputTokens"));
    let legacy_output_cap = positive_safe_integer(model.max_tokens);
    let operational_output_cap = explicit_output_cap.or(legacy_output_cap);
    let verified_output_limit = verified
        .as_ref()
        .map(|capabilities| capabilities.max_output_tokens)
        .filter(|value| *value > 0 && *value <= MAX_SAFE_INTEGER);

    let unconstrained_output_tokens = match (verified_output_limit, operational_output_cap) {
        (Some(verified_limit), Some(operational)) => Some(verified_limit.min(operational)),
        _ => verified_output_limit.or(operational_output_cap),
    };
    let context_window_tokens = verified
        .as_ref()
        .and_then(|capabilities| capabilities.context_window_tokens)
        .or(explicit_context_window);
    let max_output_tokens = match (unconstrained_output_tokens, context_window_tokens) {
        (Some(unconstrained), Some(context_window)) => Some(unconstrained.min(context_window)),
        _ => unconstrained_output_tokens,
    };
    let Some(max_output_tokens) = max_output_tokens else {
        return Err(ModelExecutionLeaseError::InvalidOutputCapability);
    };

    let max_output_source = if verified_output_limit == Some(max_output_tokens) {
        CapabilityEvidenceSource::VerifiedProviderPreset
    } else if explicit_output_cap.is_some() {
        CapabilityEvidenceSource::UserOperationalCap
    } else {
        CapabilityEvidenceSource::LegacyProfile
    };
    let context_source = if verified.is_some() {
        CapabilityEvidenceSource::VerifiedProviderPreset
    } else if explicit_context_window.is_some() {
        CapabilityEvidenceSource::UserOperationalCap
    } else {
        CapabilityEvidenceSource::Unknown
    };
    let feature_flags_source = if verified.is_some() {
        CapabilityEvidenceSource::VerifiedProviderPreset
    } else {
        CapabilityEvidenceSource::Unknown
    };

    Ok(ModelExecutionCapabilityEvidence {
        source: CapabilityEvidenceSourceSet {
            context_window_tokens: context_source,
            max_output_tokens: max_output_source,
            feature_flags: feature_flags_source,
        },
        subject_fingerprint: subject,
        context_window_tokens,
        max_output_tokens,
        reasoning: verified.as_ref().map(|capabilities| capabilities.reasoning),
        structured_output: verified
            .as_ref()
            .map(|capabilities| capabilities.structured_output),
        usage: verified.as_ref().map(|capabilities| capabilities.usage),
    })
}

/// 用一份不可变快照构建回执（注册表与资格判定共用，避免能力规划漂移）。
pub fn create_model_execution_lease_receipt(
    model: &LeaseModelProfile<'_>,
    lease_id: String,
    created_at: u64,
    expires_at: u64,
) -> Result<ModelExecutionLeaseReceipt, ModelExecutionLeaseError> {
    Ok(ModelExecutionLeaseReceipt {
        lease_id,
        model_id: model.id.to_string(),
        provider: model.provider.to_string(),
        protocol: model.protocol.to_string(),
        model_name: model.model_name.to_string(),
        model_revision: model_revision(model),
        endpoint_fingerprint: endpoint_fingerprint(model.provider, model.protocol, model.base_url),
        capability_evidence: resolve_model_execution_capability_evidence(model)?,
        created_at,
        expires_at,
    })
}

/// `close` 的三态结果（区分「本次关闭」/「此前已关闭」/「未知租约」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmLeaseStoreCloseOutcome {
    Closed,
    AlreadyClosed,
    Unknown,
}

struct LeaseRecord {
    receipt: ModelExecutionLeaseReceipt,
    snapshot: serde_json::Value,
}

impl std::fmt::Debug for LeaseRecord {
    /// 快照含 API Key：`Debug` 一律打码，避免任何日志 / 诊断输出泄露凭据。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LeaseRecord")
            .field("receipt", &self.receipt)
            .field("snapshot", &"<redacted>")
            .finish()
    }
}

/// 主进程内的模型执行租约注册表（含已关闭租约墓碑）。
///
/// 状态全部驻留进程内存，不落盘：重启即失效（对齐基线模块级 `Map`）。
#[derive(Debug)]
pub struct LlmLeaseStore {
    records: HashMap<String, LeaseRecord>,
    tombstones: HashMap<String, u64>,
    ttl_ms: u64,
}

impl Default for LlmLeaseStore {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmLeaseStore {
    pub fn new() -> Self {
        LlmLeaseStore {
            records: HashMap::new(),
            tombstones: HashMap::new(),
            ttl_ms: DEFAULT_MODEL_EXECUTION_LEASE_TTL_MS,
        }
    }

    /// 以自定义 TTL 构造（仅测试需要；生产路径用 [`Self::new`] 的默认 TTL）。
    #[cfg(test)]
    pub fn with_ttl(ttl_ms: u64) -> Self {
        LlmLeaseStore {
            ttl_ms,
            ..Self::new()
        }
    }

    /// 开启租约：从模型配置文件冻结快照并签发回执。
    pub fn begin_at(
        &mut self,
        models_path: &Path,
        model_id: &str,
        lease_id: String,
        now_ms: u64,
    ) -> Result<ModelExecutionLeaseReceipt, ModelExecutionLeaseError> {
        let models = crate::commands::read_models_at(models_path);
        let snapshot = models
            .into_iter()
            .find(|entry| entry.get("id").and_then(|value| value.as_str()) == Some(model_id))
            .ok_or(ModelExecutionLeaseError::ModelNotFound)?;
        let profile = LeaseModelProfile::from_value(&snapshot);
        let receipt = create_model_execution_lease_receipt(
            &profile,
            lease_id,
            now_ms,
            now_ms.saturating_add(self.ttl_ms),
        )?;
        self.records.insert(
            receipt.lease_id.clone(),
            LeaseRecord {
                receipt: receipt.clone(),
                snapshot,
            },
        );
        Ok(receipt)
    }

    /// 解析租约到冻结快照（过期即失效）。
    pub fn resolve_model_at(
        &mut self,
        lease_id: &str,
        now_ms: u64,
    ) -> Result<serde_json::Value, String> {
        let Some(record) = self.records.get(lease_id) else {
            return Err("模型执行租约无效".to_string());
        };
        if now_ms >= record.receipt.expires_at {
            self.records.remove(lease_id);
            return Err("模型执行租约已过期".to_string());
        }
        Ok(record.snapshot.clone())
    }

    /// 关闭租约：先清理过期墓碑，再判定「关闭 / 幂等重放 / 未知」。
    pub fn close(&mut self, lease_id: &str, now_ms: u64) -> LlmLeaseStoreCloseOutcome {
        self.prune_tombstones(now_ms);
        if self.records.remove(lease_id).is_some() {
            self.tombstones.insert(
                lease_id.to_string(),
                now_ms.saturating_add(CLOSED_EXECUTION_LEASE_TOMBSTONE_TTL_MS),
            );
            return LlmLeaseStoreCloseOutcome::Closed;
        }
        if self.tombstones.contains_key(lease_id) {
            return LlmLeaseStoreCloseOutcome::AlreadyClosed;
        }
        LlmLeaseStoreCloseOutcome::Unknown
    }

    fn prune_tombstones(&mut self, now_ms: u64) {
        self.tombstones.retain(|_, expires_at| *expires_at > now_ms);
    }

    /// 当前存活租约数量（仅测试与诊断用）。
    #[cfg(test)]
    pub fn live_lease_count(&self) -> usize {
        self.records.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lorekeeper-lease-{}-{}",
            tag,
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_models(dir: &Path, models: serde_json::Value) -> std::path::PathBuf {
        let path = dir.join("models.json");
        std::fs::write(&path, serde_json::to_string(&models).unwrap()).unwrap();
        path
    }

    fn deepseek_model(temperature: f64) -> serde_json::Value {
        json!({
            "id": "m-1",
            "name": "深寻",
            "provider": "deepseek",
            "protocol": "openai",
            "modelName": "deepseek-v4-flash",
            "apiKey": "sk-secret",
            "baseUrl": "https://api.deepseek.com",
            "temperature": temperature,
            "maxTokens": 4096,
            "purposes": ["generation"],
        })
    }

    fn receipt_for(model: &serde_json::Value) -> ModelExecutionLeaseReceipt {
        let profile = LeaseModelProfile::from_value(model);
        create_model_execution_lease_receipt(&profile, "lease-1".to_string(), 1_000, 2_000).unwrap()
    }

    #[test]
    fn verified_endpoint_uses_provider_preset_evidence_test() {
        // legacy 上限与已验证上限一致时，三项证据全部落到 verified-provider-preset
        let model = json!({
            "id": "m-1",
            "provider": "deepseek",
            "protocol": "openai",
            "modelName": "deepseek-v4-flash",
            "baseUrl": "https://api.deepseek.com",
            "temperature": 0.7,
            "maxTokens": 384_000,
        });
        let evidence = receipt_for(&model).capability_evidence;
        assert_eq!(
            evidence.source.context_window_tokens,
            CapabilityEvidenceSource::VerifiedProviderPreset
        );
        assert_eq!(
            evidence.source.max_output_tokens,
            CapabilityEvidenceSource::VerifiedProviderPreset
        );
        assert_eq!(
            evidence.source.feature_flags,
            CapabilityEvidenceSource::VerifiedProviderPreset
        );
        assert_eq!(evidence.context_window_tokens, Some(1_000_000));
        assert_eq!(evidence.max_output_tokens, 384_000);
        assert_eq!(evidence.reasoning, Some(true));
        assert_eq!(evidence.structured_output, Some(true));
        assert_eq!(evidence.usage, Some(true));
    }

    #[test]
    fn operational_cap_bounds_verified_limit_test() {
        // 用户把输出上限压到 4096（legacy maxTokens）→ 取 min，来源标记为 legacy
        let model = deepseek_model(0.7);
        let receipt = receipt_for(&model);
        assert_eq!(
            receipt.capability_evidence.max_output_tokens,
            4_096,
            "legacy 上限必须封顶已验证上限"
        );
        assert_eq!(
            receipt.capability_evidence.source.max_output_tokens,
            CapabilityEvidenceSource::LegacyProfile
        );
        assert_eq!(
            receipt.capability_evidence.context_window_tokens,
            Some(1_000_000),
            "上下文窗口仍来自已验证预设"
        );
    }

    #[test]
    fn explicit_capabilities_without_verified_preset_are_user_evidence_test() {
        let model = json!({
            "id": "m-2",
            "provider": "custom",
            "protocol": "openai",
            "baseUrl": "https://relay.example.com/v1",
            "modelName": "whatever",
            "temperature": 0.5,
            "maxTokens": 2048,
            "capabilities": { "contextWindowTokens": 32_768, "maxOutputTokens": 8_192 },
        });
        let evidence = receipt_for(&model).capability_evidence;
        assert_eq!(
            evidence.source.context_window_tokens,
            CapabilityEvidenceSource::UserOperationalCap
        );
        assert_eq!(
            evidence.source.max_output_tokens,
            CapabilityEvidenceSource::UserOperationalCap,
            "显式用户上限优先于 legacy"
        );
        assert_eq!(
            evidence.source.feature_flags,
            CapabilityEvidenceSource::Unknown
        );
        assert_eq!(evidence.max_output_tokens, 8_192);
        assert_eq!(evidence.context_window_tokens, Some(32_768));
        assert_eq!(evidence.reasoning, None, "无协议证据时能力标记必须为 null");
        assert_eq!(evidence.structured_output, None);
        assert_eq!(evidence.usage, None);
    }

    #[test]
    fn output_cap_is_capped_by_context_window_test() {
        let model = json!({
            "id": "m-3",
            "provider": "custom",
            "protocol": "openai",
            "baseUrl": "https://relay.example.com/v1",
            "modelName": "whatever",
            "temperature": 0.5,
            "maxTokens": 100_000,
            "capabilities": { "contextWindowTokens": 16_384, "maxOutputTokens": 65_536 },
        });
        let evidence = receipt_for(&model).capability_evidence;
        assert_eq!(
            evidence.max_output_tokens, 16_384,
            "输出上限不得超过上下文窗口"
        );
        assert_eq!(evidence.context_window_tokens, Some(16_384));
    }

    #[test]
    fn invalid_output_capability_is_rejected_test() {
        let model = json!({
            "id": "m-4",
            "provider": "custom",
            "protocol": "openai",
            "baseUrl": "https://relay.example.com/v1",
            "modelName": "whatever",
            "temperature": 0.5,
        });
        let profile = LeaseModelProfile::from_value(&model);
        assert_eq!(
            resolve_model_execution_capability_evidence(&profile).unwrap_err(),
            ModelExecutionLeaseError::InvalidOutputCapability
        );
        assert_eq!(
            ModelExecutionLeaseError::InvalidOutputCapability.message(),
            "模型输出上限无效"
        );

        for bad in [json!(0), json!(-1), json!(1.5), json!("4096")] {
            let model = json!({
                "id": "m-5",
                "provider": "custom",
                "protocol": "openai",
                "baseUrl": "https://relay.example.com/v1",
                "modelName": "whatever",
                "maxTokens": bad,
            });
            assert!(
                resolve_model_execution_capability_evidence(&LeaseModelProfile::from_value(&model))
                    .is_err(),
                "非法上限 {bad} 必须被拒绝"
            );
        }
    }

    #[test]
    fn fingerprints_are_stable_and_distinguish_subjects_test() {
        let first = receipt_for(&deepseek_model(0.7));
        let second = receipt_for(&deepseek_model(0.7));
        assert_eq!(first.model_revision, second.model_revision, "同快照必须同指纹");
        assert_eq!(first.endpoint_fingerprint, second.endpoint_fingerprint);
        assert_eq!(
            first.capability_evidence.subject_fingerprint,
            second.capability_evidence.subject_fingerprint
        );

        let temperature_changed = receipt_for(&deepseek_model(0.9));
        assert_ne!(
            first.model_revision, temperature_changed.model_revision,
            "采样温度变化必须改变修订指纹"
        );
        assert_eq!(
            first.endpoint_fingerprint, temperature_changed.endpoint_fingerprint,
            "采样温度不改变端点指纹"
        );

        let other_endpoint = receipt_for(&json!({
            "id": "m-1",
            "provider": "deepseek",
            "protocol": "openai",
            "modelName": "deepseek-v4-flash",
            "baseUrl": "https://api.deepseek.com/",
            "temperature": 0.7,
            "maxTokens": 4096,
        }));
        assert_eq!(
            first.endpoint_fingerprint, other_endpoint.endpoint_fingerprint,
            "尾斜杠差异不得改变端点指纹"
        );
    }

    #[test]
    fn canonical_json_sorts_object_keys_test() {
        assert_eq!(
            canonical_json(&json!({ "b": 1, "a": [ { "z": true, "y": null } ] })),
            r#"{"a":[{"y":null,"z":true}],"b":1}"#
        );
    }

    #[test]
    fn normalize_endpoint_strips_query_fragment_and_slash_test() {
        assert_eq!(
            normalize_endpoint("https://api.deepseek.com/v1/?a=1#frag"),
            "https://api.deepseek.com/v1"
        );
        assert_eq!(normalize_endpoint("  https://API.DeepSeek.com  "), "https://api.deepseek.com");
        assert_eq!(normalize_endpoint(""), "");
        assert_eq!(normalize_endpoint("not a url/"), "not a url");
    }

    #[test]
    fn receipt_never_contains_credentials_test() {
        let receipt = receipt_for(&deepseek_model(0.7));
        let serialized = serde_json::to_string(&receipt).unwrap();
        assert!(
            !serialized.contains("sk-secret"),
            "回执不得携带 API Key：{serialized}"
        );
    }

    #[test]
    fn begin_freezes_snapshot_and_rejects_unknown_model_test() {
        let dir = temp_dir("begin");
        let models_path = write_models(&dir, json!([deepseek_model(0.7)]));
        let mut store = LlmLeaseStore::new();

        let receipt = store
            .begin_at(&models_path, "m-1", "lease-a".to_string(), 1_000)
            .unwrap();
        assert_eq!(receipt.model_id, "m-1");
        assert_eq!(receipt.created_at, 1_000);
        assert_eq!(receipt.expires_at, 1_000 + DEFAULT_MODEL_EXECUTION_LEASE_TTL_MS);
        assert_eq!(store.live_lease_count(), 1);

        // 冻结语义：签发后改写档案不影响已发出的租约
        std::fs::write(&models_path, "[]").unwrap();
        let snapshot = store.resolve_model_at("lease-a", 1_001).unwrap();
        assert_eq!(snapshot["modelName"], json!("deepseek-v4-flash"));
        assert_eq!(snapshot["apiKey"], json!("sk-secret"), "快照保留完整档案（主进程内）");

        assert_eq!(
            store
                .begin_at(&models_path, "missing", "lease-b".to_string(), 1_000)
                .unwrap_err(),
            ModelExecutionLeaseError::ModelNotFound
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expired_lease_is_dropped_on_resolve_test() {
        let dir = temp_dir("expire");
        let models_path = write_models(&dir, json!([deepseek_model(0.7)]));
        let mut store = LlmLeaseStore::with_ttl(1_000);
        let receipt = store
            .begin_at(&models_path, "m-1", "lease-a".to_string(), 5_000)
            .unwrap();

        assert_eq!(
            store.resolve_model_at("lease-a", receipt.expires_at).unwrap_err(),
            "模型执行租约已过期"
        );
        assert_eq!(store.live_lease_count(), 0, "过期租约必须被清理");
        assert_eq!(
            store.resolve_model_at("lease-a", 9_999).unwrap_err(),
            "模型执行租约无效"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn close_is_idempotent_within_tombstone_window_test() {
        let dir = temp_dir("close");
        let models_path = write_models(&dir, json!([deepseek_model(0.7)]));
        let mut store = LlmLeaseStore::new();
        store
            .begin_at(&models_path, "m-1", "lease-a".to_string(), 1_000)
            .unwrap();

        assert_eq!(
            store.close("lease-a", 1_100),
            LlmLeaseStoreCloseOutcome::Closed
        );
        assert_eq!(
            store.close("lease-a", 1_200),
            LlmLeaseStoreCloseOutcome::AlreadyClosed,
            "墓碑窗口内重复关闭必须幂等"
        );
        assert_eq!(
            store.close("never-issued", 1_200),
            LlmLeaseStoreCloseOutcome::Unknown
        );

        let after_expiry = 1_100 + CLOSED_EXECUTION_LEASE_TOMBSTONE_TTL_MS;
        assert_eq!(
            store.close("lease-a", after_expiry),
            LlmLeaseStoreCloseOutcome::Unknown,
            "墓碑过期后必须恢复为未知租约"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
