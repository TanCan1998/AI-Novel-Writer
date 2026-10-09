//! 批次 D2-c：模型发现服务 —— 迁移自 `electron/services/model-discovery-service.ts`。
//!
//! | 本模块项 | 基线来源 |
//! |---|---|
//! | [`resolve_openai_models_url`] | `resolveOpenAIModelsUrl()` |
//! | [`resolve_gemini_models_url`] | `resolveGeminiModelsUrl()` |
//! | [`safe_provider_text`] / [`url_contains_credential`] | 同名守卫（凭据回显 / 控制字符 / 长度上限） |
//! | [`parse_openai_models`] / [`parse_gemini_models`] | 同名解析器（严格：任一条目非法 → 整体 `invalid_response`） |
//! | [`interpret_discovery_response`] | `discoverModels()` 的失败分类与空列表判定 |
//!
//! ## 安全边界（对齐基线的「可信边界」注释）
//!
//! 发现请求来自**尚未保存**的表单字段，因此：
//! 1. 请求 URL 中若回显了 API Key（原文 / `encodeURIComponent` 形态 / 解码后形态）→
//!    不发起请求，直接 `invalid_response`；
//! 2. 供应商返回的模型文本经 [`safe_provider_text`] 清洗（≤512 字节、无控制字符、
//!    不含凭据、`trim` 后非空），任何一条不合规 → 整体拒绝而不是丢弃该条；
//! 3. 条目数上限 [`MAX_DISCOVERED_MODEL_ENTRIES`]。
//!
//! ## 与基线的环境差异（非语义放宽）
//!
//! | # | 基线 | Tauri 侧 |
//! |---|---|---|
//! | 1 | `AbortController` 15s 总超时 | [`reqwest::Client`] 的 `timeout`（同样是含 body 读取的总超时） |
//! | 2 | `redirect: 'error'` → fetch reject → `network` | `Policy::none()` + 显式判 `is_redirection()` → `network`（3xx **不**落入 `unsupported`） |
//! | 3 | `URL.toString()` | [`url::Url::to_string()`]（URL 规范化实现不同，端点语义一致） |
//! | 4 | 代理经 `process.env.HTTP_PROXY` 隐式生效 | 显式 `reqwest::Proxy`（见 [`build_discovery_client`]） |

use std::time::Duration;

use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use serde_json::Value;
use url::Url;

use crate::llm::chat::ProxySpec;

/// 发现请求的总超时（对齐 `DEFAULT_MODEL_DISCOVERY_TIMEOUT_MS`）。
pub const DEFAULT_MODEL_DISCOVERY_TIMEOUT_MS: u64 = 15_000;

/// 单次发现允许的最大条目数（对齐 `MAX_DISCOVERED_MODEL_ENTRIES`）。
pub const MAX_DISCOVERED_MODEL_ENTRIES: usize = 500;

/// 供应商文本字段的字节上限（对齐 `MAX_DISCOVERED_MODEL_TEXT_BYTES`）。
pub const MAX_DISCOVERED_MODEL_TEXT_BYTES: usize = 512;

/// 一条已发现的模型（契约 `DiscoveredModel`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredModel {
    /// 供应商侧的模型标识（原样）。
    pub id: String,
    /// 供应商侧的展示名（缺失时回落为 `id`）。
    pub name: String,
    /// 可直接写入 `ModelProfile.modelName` 的值。
    pub value: String,
}

/// 发现失败档位（契约 `ModelDiscoveryErrorCode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelDiscoveryErrorCode {
    /// 401 / 403。
    Auth,
    /// 其它 4xx（非 401/403/408/425/429）。
    Unsupported,
    /// 408 / 425 / 429 / 5xx / 网络失败 / 超时 / 重定向。
    Network,
    /// 载荷形状不符（含未通过安全清洗）。
    InvalidResponse,
    /// 载荷合法但模型列表为空。
    Empty,
}

impl ModelDiscoveryErrorCode {
    /// 契约字面量（`ModelDiscoveryErrorCode`）。
    pub fn as_str(self) -> &'static str {
        match self {
            ModelDiscoveryErrorCode::Auth => "auth",
            ModelDiscoveryErrorCode::Unsupported => "unsupported",
            ModelDiscoveryErrorCode::Network => "network",
            ModelDiscoveryErrorCode::InvalidResponse => "invalid_response",
            ModelDiscoveryErrorCode::Empty => "empty",
        }
    }
}

/// `llm:discover-models` 结果（契约 `ModelDiscoveryResult` 的判别联合）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelDiscoveryResult {
    Found(Vec<DiscoveredModel>),
    Failed(ModelDiscoveryErrorCode),
}

impl ModelDiscoveryResult {
    /// 便捷构造失败结果。
    pub fn failed(code: ModelDiscoveryErrorCode) -> Self {
        ModelDiscoveryResult::Failed(code)
    }

    /// 成功分支的模型列表（失败时为 `None`），供测试断言读取。
    #[cfg(test)]
    pub fn models(&self) -> Option<&[DiscoveredModel]> {
        match self {
            ModelDiscoveryResult::Found(models) => Some(models),
            ModelDiscoveryResult::Failed(_) => None,
        }
    }

    /// 失败档位（成功时为 `None`），供测试断言读取。
    #[cfg(test)]
    pub fn error_code(&self) -> Option<ModelDiscoveryErrorCode> {
        match self {
            ModelDiscoveryResult::Found(_) => None,
            ModelDiscoveryResult::Failed(code) => Some(*code),
        }
    }
}

/// 手写序列化：契约是 `{success:true, models}` / `{success:false, errorCode}`，
/// 成功分支**不得**出现 `errorCode` 键，失败分支**不得**出现 `models` 键。
impl Serialize for ModelDiscoveryResult {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self {
            ModelDiscoveryResult::Found(models) => {
                map.serialize_entry("success", &true)?;
                map.serialize_entry("models", models)?;
            }
            ModelDiscoveryResult::Failed(code) => {
                map.serialize_entry("success", &false)?;
                map.serialize_entry("errorCode", code.as_str())?;
            }
        }
        map.end()
    }
}

// ===== 安全清洗 =====

/// `encodeURIComponent` 的等价实现（不编码 `A-Za-z0-9-_.!~*'()`）。
fn encode_uri_component(value: &str) -> String {
    const KEEP: &[u8] = b"-_.!~*'()";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        let byte = *byte;
        if byte.is_ascii_alphanumeric() || KEEP.contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// `decodeURIComponent` 的等价实现；非法 `%` 转义或非法 UTF-8 时返回 `None`。
fn decode_uri_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let high = (bytes[index + 1] as char).to_digit(16)?;
            let low = (bytes[index + 2] as char).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// 对齐 `urlContainsCredential()`：原文 / `encodeURIComponent` 形态 / 解码后形态任一命中即真。
///
/// 空凭据恒为假（`if (!credential) return false`）；解码失败静默按未命中处理。
pub fn url_contains_credential(url: &str, credential: &str) -> bool {
    if credential.is_empty() {
        return false;
    }
    if url.contains(credential) || url.contains(&encode_uri_component(credential)) {
        return true;
    }
    match decode_uri_component(url) {
        Some(decoded) => decoded.contains(credential),
        None => false,
    }
}

/// 对齐 `containsControlCharacter()`：C0 控制字符与 C1 控制字符（`0x7f..=0x9f`）。
fn contains_control_character(value: &str) -> bool {
    value
        .chars()
        .any(|character| character <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&character))
}

/// 对齐 `safeProviderText()`：不合规返回 `None`（调用方须整体拒绝，不得静默丢弃该条）。
pub fn safe_provider_text(value: Option<&Value>, credential: &str) -> Option<String> {
    let text = value?.as_str()?;
    if text.len() > MAX_DISCOVERED_MODEL_TEXT_BYTES {
        return None;
    }
    if contains_control_character(text) {
        return None;
    }
    let normalized = text.trim();
    if normalized.is_empty() || url_contains_credential(normalized, credential) {
        return None;
    }
    Some(normalized.to_string())
}

// ===== 端点解析 =====

/// 对齐 `resolveOpenAIModelsUrl()`：剥掉 `/chat/completions` 或 `/chat` 后缀，
/// 空路径回落 `/v1`，再拼 `/models`；query / fragment / 凭据一律清除。
pub fn resolve_openai_models_url(base_url: &str) -> Option<String> {
    let mut endpoint = Url::parse(base_url.trim()).ok()?;
    let mut configured_path = endpoint.path().trim_end_matches('/').to_string();

    if let Some(stripped) = configured_path.strip_suffix("/chat/completions") {
        configured_path = stripped.to_string();
    } else if let Some(stripped) = configured_path.strip_suffix("/chat") {
        configured_path = stripped.to_string();
    }
    if configured_path.is_empty() {
        configured_path = "/v1".to_string();
    }

    endpoint.set_path(&format!("{configured_path}/models"));
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    let _ = endpoint.set_username("");
    let _ = endpoint.set_password(None);
    Some(endpoint.to_string())
}

/// 对齐 `resolveGeminiModelsUrl()`：`/v1beta/models` 原样、`/v1beta` 补 `/models`、
/// 其余补 `/v1beta/models`；query / fragment / 凭据一律清除。
pub fn resolve_gemini_models_url(base_url: &str) -> Option<String> {
    let mut endpoint = Url::parse(base_url.trim()).ok()?;
    let configured_path = endpoint.path().trim_end_matches('/').to_string();

    let next_path = if configured_path.ends_with("/v1beta/models") {
        configured_path
    } else if configured_path.ends_with("/v1beta") {
        format!("{configured_path}/models")
    } else {
        format!("{configured_path}/v1beta/models")
    };

    endpoint.set_path(&next_path);
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    let _ = endpoint.set_username("");
    let _ = endpoint.set_password(None);
    Some(endpoint.to_string())
}

// ===== 载荷解析 =====

/// 对齐 `parseOpenAIModels()`：`{ data: [...] }`；任一条目非法即整体返回 `None`。
pub fn parse_openai_models(payload: &Value, credential: &str) -> Option<Vec<DiscoveredModel>> {
    let entries = payload.get("data")?.as_array()?;
    if entries.len() > MAX_DISCOVERED_MODEL_ENTRIES {
        return None;
    }

    let mut models: Vec<DiscoveredModel> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for item in entries {
        if !item.is_object() {
            return None;
        }
        let id = safe_provider_text(item.get("id"), credential)?;
        // `name` 键缺失（`undefined`）才回落为 `id`；键存在但非法 → 整体拒绝。
        let name = match item.get("name") {
            None => id.clone(),
            Some(value) => safe_provider_text(Some(value), credential)?,
        };
        if seen.iter().any(|existing| existing == &id) {
            continue;
        }
        seen.push(id.clone());
        models.push(DiscoveredModel {
            name,
            value: id.clone(),
            id,
        });
    }
    Some(models)
}

/// 对齐 `parseGeminiModels()`：`{ models: [...] }`；`value` 去掉 `models/` 前缀。
pub fn parse_gemini_models(payload: &Value, credential: &str) -> Option<Vec<DiscoveredModel>> {
    let entries = payload.get("models")?.as_array()?;
    if entries.len() > MAX_DISCOVERED_MODEL_ENTRIES {
        return None;
    }

    let mut models: Vec<DiscoveredModel> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for item in entries {
        if !item.is_object() {
            return None;
        }
        let id = safe_provider_text(item.get("name"), credential)?;
        let name = match item.get("displayName") {
            None => id.clone(),
            Some(value) => safe_provider_text(Some(value), credential)?,
        };
        let value = match id.strip_prefix("models/") {
            Some(stripped) => stripped.to_string(),
            None => id.clone(),
        };
        if value.is_empty() {
            return None;
        }
        if seen.iter().any(|existing| existing == &value) {
            continue;
        }
        seen.push(value.clone());
        models.push(DiscoveredModel { id, name, value });
    }
    Some(models)
}

// ===== 传输层与解释 =====

/// HTTP 失败分类（对齐 `classifyHttpFailure()`）。
pub fn classify_http_failure(status: u16) -> ModelDiscoveryErrorCode {
    if status == 401 || status == 403 {
        return ModelDiscoveryErrorCode::Auth;
    }
    if status == 408 || status == 425 || status == 429 || status >= 500 {
        return ModelDiscoveryErrorCode::Network;
    }
    ModelDiscoveryErrorCode::Unsupported
}

/// 传输层归一结果 —— 抽出纯数据以便离线覆盖全部分支（无网络依赖）。
#[derive(Debug, Clone, PartialEq)]
pub enum DiscoveryTransport {
    /// fetch reject：网络失败 / 超时 / `redirect: 'error'` 命中重定向。
    Rejected,
    /// 非 2xx 响应（基线在读取 body **之前**就分类返回）。
    HttpStatus(u16),
    /// 2xx 响应体：`json` 为 `None` 表示 body 读取或 JSON 解析失败；
    /// `timed_out` 对应基线 `abortController.signal.aborted`。
    Payload {
        json: Option<Value>,
        timed_out: bool,
    },
}

/// 解释传输结果（对齐 `discoverModels()` 的判定顺序）。
pub fn interpret_discovery_response(
    protocol: &str,
    credential: &str,
    transport: DiscoveryTransport,
) -> ModelDiscoveryResult {
    match transport {
        DiscoveryTransport::Rejected => {
            ModelDiscoveryResult::Failed(ModelDiscoveryErrorCode::Network)
        }
        DiscoveryTransport::HttpStatus(status) => {
            ModelDiscoveryResult::Failed(classify_http_failure(status))
        }
        DiscoveryTransport::Payload { json, timed_out } => {
            let Some(payload) = json else {
                // 基线：`response.json()` 抛出时按 abort 与否分流。
                return ModelDiscoveryResult::Failed(if timed_out {
                    ModelDiscoveryErrorCode::Network
                } else {
                    ModelDiscoveryErrorCode::InvalidResponse
                });
            };
            let parsed = if protocol == "gemini" {
                parse_gemini_models(&payload, credential)
            } else {
                parse_openai_models(&payload, credential)
            };
            match parsed {
                None => ModelDiscoveryResult::Failed(ModelDiscoveryErrorCode::InvalidResponse),
                Some(models) if models.is_empty() => {
                    ModelDiscoveryResult::Failed(ModelDiscoveryErrorCode::Empty)
                }
                Some(models) => ModelDiscoveryResult::Found(models),
            }
        }
    }
}

/// 构建发现专用客户端：`Policy::none()`（对齐 `redirect: 'error'`）+ 总超时 + 显式代理。
///
/// 因需额外设定 `redirect::Policy`，此处不复用 [`crate::llm::chat::build_client`]，
/// 但代理装配方式与它保持一致（显式 `reqwest::Proxy`，不经进程环境变量）。
pub fn build_discovery_client(proxy: Option<&ProxySpec>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(DEFAULT_MODEL_DISCOVERY_TIMEOUT_MS));
    if let Some(spec) = proxy {
        builder = builder.proxy(
            reqwest::Proxy::all(spec.url()).map_err(|error| format!("代理配置无效：{error}"))?,
        );
    }
    builder
        .build()
        .map_err(|error| format!("HTTP 客户端初始化失败：{error}"))
}

/// 真实 HTTP 抓取（薄层：把响应压成 [`DiscoveryTransport`]）。
async fn fetch_models(
    client: &reqwest::Client,
    protocol: &str,
    request_url: &str,
    api_key: &str,
) -> DiscoveryTransport {
    let mut request = client
        .get(request_url)
        .header(reqwest::header::ACCEPT, "application/json");
    request = if protocol == "gemini" {
        request.header("x-goog-api-key", api_key)
    } else {
        request.header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
    };

    let response = match request.send().await {
        Ok(response) => response,
        Err(_) => return DiscoveryTransport::Rejected,
    };
    let status = response.status();
    if status.is_redirection() {
        // 基线 `redirect: 'error'` 在收到重定向时直接 reject。
        return DiscoveryTransport::Rejected;
    }
    if !status.is_success() {
        return DiscoveryTransport::HttpStatus(status.as_u16());
    }

    match response.text().await {
        Ok(text) => DiscoveryTransport::Payload {
            json: serde_json::from_str::<Value>(&text).ok(),
            timed_out: false,
        },
        Err(error) => DiscoveryTransport::Payload {
            json: None,
            timed_out: error.is_timeout(),
        },
    }
}

/// 一次发现请求的同步准备结果：`protocol` / 已解析端点 / `apiKey`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedDiscoveryRequest {
    pub protocol: String,
    pub request_url: String,
    pub api_key: String,
}

/// 请求前的同步判定（端点解析 + 凭据回显守卫）。
///
/// `Err(code)` 表示**不发起请求**，直接以该档位返回 —— 这正是基线的两条早退分支：
/// - 非法端点 → `new URL()` 抛错 → 外层 catch → `network`；
/// - 端点回显凭据 → 显式返回 `invalid_response`。
///
/// 抽成同步纯函数后，这两条路径可在无网络、无异步运行时的条件下完整单测。
pub fn resolve_discovery_request(
    model: &Value,
) -> Result<PreparedDiscoveryRequest, ModelDiscoveryErrorCode> {
    let protocol = model
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or("openai")
        .to_string();
    let base_url = model
        .get("baseUrl")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let api_key = model
        .get("apiKey")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let request_url = if protocol == "gemini" {
        resolve_gemini_models_url(base_url)
    } else {
        resolve_openai_models_url(base_url)
    }
    .ok_or(ModelDiscoveryErrorCode::Network)?;

    if url_contains_credential(&request_url, &api_key) {
        return Err(ModelDiscoveryErrorCode::InvalidResponse);
    }

    Ok(PreparedDiscoveryRequest {
        protocol,
        request_url,
        api_key,
    })
}

/// `llm:discover-models` 核心：解析端点 → 凭据回显守卫 → 抓取 → 解释。
///
/// 未保存的表单字段直接进入此边界，因此任何非法端点都归入 `network`
/// （对齐基线 `new URL()` 抛错后落到外层 catch 的行为）。
pub async fn discover_models(client: &reqwest::Client, model: &Value) -> ModelDiscoveryResult {
    let prepared = match resolve_discovery_request(model) {
        Ok(prepared) => prepared,
        Err(code) => return ModelDiscoveryResult::Failed(code),
    };

    let transport = fetch_models(
        client,
        &prepared.protocol,
        &prepared.request_url,
        &prepared.api_key,
    )
    .await;
    interpret_discovery_response(&prepared.protocol, &prepared.api_key, transport)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_models_url_matches_baseline_branches_test() {
        assert_eq!(
            resolve_openai_models_url("https://api.example.com/v1").as_deref(),
            Some("https://api.example.com/v1/models")
        );
        // 已带 `/chat/completions` → 剥掉再拼
        assert_eq!(
            resolve_openai_models_url("https://api.example.com/v1/chat/completions").as_deref(),
            Some("https://api.example.com/v1/models")
        );
        // 已带 `/chat` → 剥掉再拼
        assert_eq!(
            resolve_openai_models_url("https://api.example.com/v1/chat").as_deref(),
            Some("https://api.example.com/v1/models")
        );
        // 空路径 → `/v1/models`
        assert_eq!(
            resolve_openai_models_url("https://api.example.com").as_deref(),
            Some("https://api.example.com/v1/models")
        );
        // 尾斜杠被规范化
        assert_eq!(
            resolve_openai_models_url("https://api.example.com/v1/").as_deref(),
            Some("https://api.example.com/v1/models")
        );
        // query / fragment / 凭据全部清除
        assert_eq!(
            resolve_openai_models_url("https://user:secret@api.example.com/v1?token=abc#frag")
                .as_deref(),
            Some("https://api.example.com/v1/models")
        );
        // 非法 URL → None（→ network）
        assert_eq!(resolve_openai_models_url("api.example.com/v1"), None);
        assert_eq!(resolve_openai_models_url(""), None);
    }

    #[test]
    fn gemini_models_url_matches_baseline_branches_test() {
        assert_eq!(
            resolve_gemini_models_url("https://generativelanguage.googleapis.com/v1beta")
                .as_deref(),
            Some("https://generativelanguage.googleapis.com/v1beta/models")
        );
        assert_eq!(
            resolve_gemini_models_url("https://generativelanguage.googleapis.com/v1beta/models")
                .as_deref(),
            Some("https://generativelanguage.googleapis.com/v1beta/models")
        );
        assert_eq!(
            resolve_gemini_models_url("https://generativelanguage.googleapis.com").as_deref(),
            Some("https://generativelanguage.googleapis.com/v1beta/models")
        );
        assert_eq!(
            resolve_gemini_models_url("https://host/proxy?key=k").as_deref(),
            Some("https://host/proxy/v1beta/models")
        );
    }

    #[test]
    fn credential_guard_covers_raw_encoded_and_decoded_forms_test() {
        assert!(
            !url_contains_credential("https://a.com/v1/models", ""),
            "空凭据恒为假"
        );
        assert!(url_contains_credential(
            "https://a.com/sk-live-123/models",
            "sk-live-123"
        ));
        assert!(
            url_contains_credential("https://a.com/sk%2Flive/models", "sk/live"),
            "encodeURIComponent 形态必须命中"
        );
        assert!(
            url_contains_credential("https://a.com/sk%2Flive/models", "sk/live"),
            "解码形态必须命中"
        );
        assert!(!url_contains_credential(
            "https://a.com/v1/models",
            "sk-live-123"
        ));
        // 非法百分号转义 → decodeURIComponent 抛错 → 按未命中处理
        assert!(!url_contains_credential(
            "https://a.com/100%/models",
            "sk-live-123"
        ));
    }

    #[test]
    fn safe_provider_text_rejects_unsafe_values_test() {
        assert_eq!(
            safe_provider_text(Some(&json!("  gpt-4o-mini  ")), "sk"),
            Some("gpt-4o-mini".to_string())
        );
        assert_eq!(safe_provider_text(None, "sk"), None, "缺失");
        assert_eq!(safe_provider_text(Some(&json!(42)), "sk"), None, "非字符串");
        assert_eq!(safe_provider_text(Some(&json!("   ")), "sk"), None, "空白");
        assert_eq!(
            safe_provider_text(Some(&json!("bad\u{0007}name")), "sk"),
            None,
            "控制字符"
        );
        assert_eq!(
            safe_provider_text(
                Some(&json!("a".repeat(MAX_DISCOVERED_MODEL_TEXT_BYTES + 1))),
                "sk"
            ),
            None,
            "超长"
        );
        assert_eq!(
            safe_provider_text(Some(&json!("https://a.com/sk-secret")), "sk-secret"),
            None,
            "不得回显凭据"
        );
    }

    #[test]
    fn parse_openai_models_is_strict_about_entries_test() {
        let parsed = parse_openai_models(
            &json!({"data": [
                {"id": "m-1"},
                {"id": "m-2", "name": "模型二"},
                {"id": "m-1", "name": "重复应被跳过"}
            ]}),
            "sk",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "m-1", "缺失 name 回落为 id");
        assert_eq!(parsed[1].name, "模型二");
        assert_eq!(parsed[1].value, "m-2");

        // 任一条目非法 → 整体 None（不是丢弃该条）
        assert!(parse_openai_models(&json!({"data": [{"id": "ok"}, {"id": 42}]}), "sk").is_none());
        assert!(
            parse_openai_models(&json!({"data": [{"id": "ok"}, "not-an-object"]}), "sk").is_none()
        );
        assert!(parse_openai_models(&json!({"data": [{"name": "无 id"}]}), "sk").is_none());
        // 形状不符
        assert!(parse_openai_models(&json!({"data": "nope"}), "sk").is_none());
        assert!(parse_openai_models(&json!([]), "sk").is_none());
        // 上限
        let too_many: Vec<Value> = (0..=MAX_DISCOVERED_MODEL_ENTRIES)
            .map(|index| json!({"id": format!("m-{index}")}))
            .collect();
        assert!(parse_openai_models(&json!({"data": too_many}), "sk").is_none());
    }

    #[test]
    fn parse_gemini_models_strips_models_prefix_test() {
        let parsed = parse_gemini_models(
            &json!({"models": [
                {"name": "models/gemini-2.5-pro", "displayName": "Gemini 2.5 Pro"},
                {"name": "models/gemini-2.5-flash"},
                {"name": "models/gemini-2.5-pro", "displayName": "重复"}
            ]}),
            "sk",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "models/gemini-2.5-pro");
        assert_eq!(parsed[0].value, "gemini-2.5-pro");
        assert_eq!(parsed[0].name, "Gemini 2.5 Pro");
        assert_eq!(
            parsed[1].name, "models/gemini-2.5-flash",
            "缺失 displayName 回落为 id"
        );
        assert_eq!(parsed[1].value, "gemini-2.5-flash");

        // `models/` 之后为空 → 整体拒绝
        assert!(parse_gemini_models(&json!({"models": [{"name": "models/"}]}), "sk").is_none());
        assert!(parse_gemini_models(&json!({"models": [{"name": 1}]}), "sk").is_none());
    }

    #[test]
    fn http_failure_classification_matches_baseline_test() {
        assert_eq!(classify_http_failure(401), ModelDiscoveryErrorCode::Auth);
        assert_eq!(classify_http_failure(403), ModelDiscoveryErrorCode::Auth);
        for status in [408, 425, 429, 500, 503] {
            assert_eq!(
                classify_http_failure(status),
                ModelDiscoveryErrorCode::Network,
                "{status}"
            );
        }
        for status in [400, 404, 405, 422] {
            assert_eq!(
                classify_http_failure(status),
                ModelDiscoveryErrorCode::Unsupported,
                "{status}"
            );
        }
    }

    #[test]
    fn interpretation_covers_every_transport_branch_test() {
        let found = interpret_discovery_response(
            "openai",
            "sk",
            DiscoveryTransport::Payload {
                json: Some(json!({"data": [{"id": "m-1"}]})),
                timed_out: false,
            },
        );
        assert_eq!(
            found.models().map(|models| models[0].id.clone()),
            Some("m-1".to_string())
        );

        assert_eq!(
            interpret_discovery_response("openai", "sk", DiscoveryTransport::Rejected).error_code(),
            Some(ModelDiscoveryErrorCode::Network),
            "fetch reject → network"
        );
        assert_eq!(
            interpret_discovery_response("openai", "sk", DiscoveryTransport::HttpStatus(302))
                .error_code(),
            Some(ModelDiscoveryErrorCode::Unsupported),
            "3xx 若走到这里属 classify 规则（真实路径已在 fetch 层判为 Rejected）"
        );
        assert_eq!(
            interpret_discovery_response("openai", "sk", DiscoveryTransport::HttpStatus(401))
                .error_code(),
            Some(ModelDiscoveryErrorCode::Auth)
        );
        assert_eq!(
            interpret_discovery_response(
                "openai",
                "sk",
                DiscoveryTransport::Payload {
                    json: None,
                    timed_out: true
                }
            )
            .error_code(),
            Some(ModelDiscoveryErrorCode::Network),
            "超时 → network"
        );
        assert_eq!(
            interpret_discovery_response(
                "openai",
                "sk",
                DiscoveryTransport::Payload {
                    json: None,
                    timed_out: false
                }
            )
            .error_code(),
            Some(ModelDiscoveryErrorCode::InvalidResponse),
            "非 JSON → invalid_response"
        );
        assert_eq!(
            interpret_discovery_response(
                "openai",
                "sk",
                DiscoveryTransport::Payload {
                    json: Some(json!({"data": []})),
                    timed_out: false
                }
            )
            .error_code(),
            Some(ModelDiscoveryErrorCode::Empty)
        );
        assert_eq!(
            interpret_discovery_response(
                "openai",
                "sk",
                DiscoveryTransport::Payload {
                    json: Some(json!({"nope": true})),
                    timed_out: false
                }
            )
            .error_code(),
            Some(ModelDiscoveryErrorCode::InvalidResponse)
        );
    }

    #[test]
    fn result_serializes_to_contract_shape_test() {
        let success = serde_json::to_value(ModelDiscoveryResult::Found(vec![DiscoveredModel {
            id: "m-1".to_string(),
            name: "模型一".to_string(),
            value: "m-1".to_string(),
        }]))
        .unwrap();
        assert_eq!(success["success"], json!(true));
        assert_eq!(success["models"][0]["id"], json!("m-1"));
        assert_eq!(success["models"][0]["value"], json!("m-1"));
        assert!(
            success.get("errorCode").is_none(),
            "成功分支不得出现 errorCode"
        );

        let failure =
            serde_json::to_value(ModelDiscoveryResult::Failed(ModelDiscoveryErrorCode::Empty))
                .unwrap();
        assert_eq!(failure["success"], json!(false));
        assert_eq!(failure["errorCode"], json!("empty"));
        assert!(failure.get("models").is_none(), "失败分支不得出现 models");
    }

    #[test]
    fn request_preparation_short_circuits_credential_echo_test() {
        // 凭据回显守卫在请求之前短路 —— 客户端从未被使用，故无需任何运行时。
        let error = resolve_discovery_request(&json!({
            "protocol": "openai",
            "baseUrl": "https://api.example.com/sk-secret",
            "apiKey": "sk-secret",
        }))
        .unwrap_err();
        assert_eq!(error, ModelDiscoveryErrorCode::InvalidResponse);
    }

    #[test]
    fn request_preparation_reports_network_for_unparsable_base_url_test() {
        let error = resolve_discovery_request(&json!({
            "protocol": "openai",
            "baseUrl": "not a url",
            "apiKey": "sk",
        }))
        .unwrap_err();
        assert_eq!(error, ModelDiscoveryErrorCode::Network);
    }

    #[test]
    fn request_preparation_returns_ready_endpoint_for_each_protocol_test() {
        let openai = resolve_discovery_request(&json!({
            "protocol": "openai",
            "baseUrl": "https://api.example.com/v1/chat/completions",
            "apiKey": "sk",
        }))
        .unwrap();
        assert_eq!(openai.protocol, "openai");
        assert_eq!(openai.request_url, "https://api.example.com/v1/models");
        assert_eq!(openai.api_key, "sk");

        let gemini = resolve_discovery_request(&json!({
            "protocol": "gemini",
            "baseUrl": "https://generativelanguage.googleapis.com/v1beta",
            "apiKey": "g-key",
        }))
        .unwrap();
        assert_eq!(
            gemini.request_url,
            "https://generativelanguage.googleapis.com/v1beta/models"
        );

        // 缺 `protocol` 时与 `LLMFactory.getProvider()` 一致地按 openai 兼容处理。
        let defaulted = resolve_discovery_request(&json!({
            "baseUrl": "https://api.example.com/v1",
            "apiKey": "sk",
        }))
        .unwrap();
        assert_eq!(defaulted.protocol, "openai");
    }
}
