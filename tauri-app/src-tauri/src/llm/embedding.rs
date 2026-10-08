//! 批次 D2-c：远程 Embedding 调用 —— 迁移自 `electron/embedding.ts` 的
//! **调用面**（`generateEmbeddings` / `embedOpenAI` / `embedGemini` + 响应校验）。
//!
//! ## 迁移范围（刻意收窄，不引入无调用面的代码）
//!
//! `electron/embedding.ts` 共 360 行，其中：
//!
//! | 基线片段 | 是否迁移 | 理由 |
//! |---|---|---|
//! | `embedOpenAI` / `embedGemini` / `generateEmbeddings` | ✅ | `llm:test-connection` 的 embedding 分支直接调用 |
//! | `validate*Embeddings` / `validateEmbeddingVectors` | ✅ | 响应合法性是「连接可用」判据的组成部分 |
//! | `ollamaOpenAIEmbeddingBaseUrl` / `buildOpenAIEmbeddingUrl` | ✅ | URL 构造是调用语义的一部分 |
//! | `releaseSmokeEmbeddings` | ❌ | Electron 安装包冒烟专用（需环境变量 + 唯一 argv 令牌），Tauri 构建产物不参与该流程 |
//! | `chunkText` | ❌ | 文本分块属 RAG 侧（批次 F 知识库），本批次无调用面 |
//! | `normalizeEmbeddingOptions` 的 `batchSize` 归一 | ❌ | 仅 `configuredBatchSize` 显式传入时才生效，目前无调用面；见 [`generate_embeddings`] 注释 |
//!
//! ## 错误文案对齐（`String(error)` 的三档前缀）
//!
//! | 基线抛出点 | `String(error)` 结果 | Rust 侧 |
//! |---|---|---|
//! | `new Error(msg)` | `Error: msg` | [`EmbeddingError::Plain`] |
//! | `EmbeddingResponseValidationError` | `EmbeddingResponseValidationError: Provider Embedding 响应无效：…` | [`EmbeddingError::InvalidResponse`] |
//! | `fetch` 失败（`TypeError`） | `TypeError: fetch failed` | [`EmbeddingError::Network`]（**无法逐字复刻**，退化为 `Error: …`） |

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Value};
use url::Url;

/// `embedding.ts` 中 `[...]` 服务商编号（用于错误文案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProvider {
    OpenAi,
    Gemini,
}

impl EmbeddingProvider {
    /// 文案中的服务商名（对齐基线 `'OpenAI' | 'Gemini'` 字面量）。
    pub fn label(self) -> &'static str {
        match self {
            EmbeddingProvider::OpenAi => "OpenAI",
            EmbeddingProvider::Gemini => "Gemini",
        }
    }
}

/// Embedding 调用错误（保留基线三档文案前缀）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddingError {
    /// 基线 `new Error(...)`。
    Plain(String),
    /// 基线 `EmbeddingResponseValidationError`（自带类名前缀）。
    InvalidResponse {
        provider: EmbeddingProvider,
        details: String,
    },
    /// 基线 `fetch` 抛出的 `TypeError`（Rust 侧无法逐字复刻异常类名）。
    Network(String),
}

impl EmbeddingError {
    /// 复刻基线 catch 分支里的 `String(error)`。
    pub fn display_text(&self) -> String {
        match self {
            EmbeddingError::Plain(message) | EmbeddingError::Network(message) => {
                format!("Error: {message}")
            }
            EmbeddingError::InvalidResponse { provider, details } => format!(
                "EmbeddingResponseValidationError: {} Embedding 响应无效：{details}",
                provider.label()
            ),
        }
    }
}

/// 构造 `EmbeddingResponseValidationError`（对齐 `invalidEmbeddingResponse()`）。
fn invalid(provider: EmbeddingProvider, details: String) -> EmbeddingError {
    EmbeddingError::InvalidResponse { provider, details }
}

// ===== URL 构造 =====

/// 已知的 OpenAI 兼容根地址（对齐 `KNOWN_OPENAI_COMPATIBLE_ROOTS`）。
const KNOWN_OPENAI_COMPATIBLE_ROOTS: [&str; 4] = [
    "https://api.openai.com",
    "https://api.deepseek.com",
    "http://localhost:11434",
    "http://127.0.0.1:11434",
];

/// Ollama 本地主机白名单（对齐 `OLLAMA_LOCAL_HOSTS`）。
const OLLAMA_LOCAL_HOSTS: [&str; 2] = ["localhost", "127.0.0.1"];

/// `/v\d+[a-z0-9.-]*$` 且大小写不敏感（对齐 `/\/v\d+(?:[a-z0-9.-]*)$/i`）。
static VERSION_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)/v\d+[a-z0-9.-]*$").unwrap());

/// 对齐 `ollamaOpenAIEmbeddingBaseUrl()`：仅 `http://localhost|127.0.0.1:11434/api`
/// 这一形态被识别；命中时返回可直接替换的 OpenAI 兼容根地址。
pub fn ollama_openai_embedding_base_url(base_url: &str) -> Option<String> {
    let parsed = Url::parse(base_url).ok()?;
    if parsed.scheme() != "http" {
        return None;
    }
    // 显式端口必须正好是 11434（与 JS `parsed.port === '11434'` 同义：
    // 默认端口在两种实现里都会被规范化掉）。
    if parsed.port() != Some(11434) {
        return None;
    }
    let host = parsed.host_str()?.to_lowercase();
    if !OLLAMA_LOCAL_HOSTS.contains(&host.as_str()) {
        return None;
    }
    let pathname = {
        let trimmed = parsed.path().trim_end_matches('/');
        if trimmed.is_empty() {
            "/"
        } else {
            trimmed
        }
    };
    if pathname != "/api" {
        return None;
    }
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return None;
    }
    Some(format!("http://{}/v1", url_authority(&parsed)))
}

/// `host[:port]` 形式（对齐 JS `URL.host`；`host_str()` 已对主机名小写化，IPv6 带方括号）。
fn url_authority(url: &Url) -> String {
    match url.host_str() {
        Some(host) => match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        },
        None => String::new(),
    }
}

/// 对齐 `buildOpenAIEmbeddingUrl()`：已是 `/embeddings` 或带版本后缀时按规则补全，
/// 已知兼容根址补 `/v1/embeddings`，其余一律补 `/embeddings`（**不**推断 `/v1`）。
pub fn build_openai_embedding_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/embeddings") {
        return base.to_string();
    }
    if VERSION_SUFFIX.is_match(base) {
        return format!("{base}/embeddings");
    }
    if KNOWN_OPENAI_COMPATIBLE_ROOTS.contains(&base.to_lowercase().as_str()) {
        return format!("{base}/v1/embeddings");
    }
    format!("{base}/embeddings")
}

/// 对齐 Gemini 侧 `model.baseUrl.replace(/\/$/, '')` —— **只去掉一个**尾斜杠。
fn gemini_base_url(base_url: &str) -> &str {
    base_url.strip_suffix('/').unwrap_or(base_url)
}

/// 对齐 `model.modelName || '<默认>'` —— 键缺失、空串都回落默认值。
fn model_name_or<'a>(model: &'a Value, fallback: &'a str) -> &'a str {
    match model.get("modelName").and_then(Value::as_str) {
        Some(name) if !name.is_empty() => name,
        _ => fallback,
    }
}

// ===== 响应校验 =====

/// 复刻 JS `String(value)`（用于「不是有限数字（收到 X）」的错误文案）。
fn js_string_of(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(js_string_of)
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

/// 对齐 `validateEmbeddingVectors()`：逐条要求「非空数组、维度一致、值为有限数字」。
pub fn validate_embedding_vectors(
    provider: EmbeddingProvider,
    vectors: &[Value],
) -> Result<Vec<Vec<f64>>, EmbeddingError> {
    let mut expected_dimension: Option<usize> = None;
    let mut validated: Vec<Vec<f64>> = Vec::new();

    for (vector_index, vector) in vectors.iter().enumerate() {
        let position = vector_index + 1;
        let Some(items) = vector.as_array() else {
            return Err(invalid(provider, format!("第 {position} 个向量为空或不是数组")));
        };
        if items.is_empty() {
            return Err(invalid(provider, format!("第 {position} 个向量为空或不是数组")));
        }
        match expected_dimension {
            None => expected_dimension = Some(items.len()),
            Some(expected) if items.len() != expected => {
                return Err(invalid(
                    provider,
                    format!("第 {position} 个向量为 {} 维，期望 {expected} 维", items.len()),
                ));
            }
            Some(_) => {}
        }

        let mut values: Vec<f64> = Vec::with_capacity(items.len());
        for (value_index, value) in items.iter().enumerate() {
            match value.as_f64() {
                Some(number) if number.is_finite() => values.push(number),
                _ => {
                    return Err(invalid(
                        provider,
                        format!(
                            "第 {position} 个向量的第 {} 个值不是有限数字（收到 {}）",
                            value_index + 1,
                            js_string_of(value)
                        ),
                    ));
                }
            }
        }
        validated.push(values);
    }

    Ok(validated)
}

/// 对齐 `validateOpenAIEmbeddings()`：`index` 必须为整数、覆盖完整且不重复。
pub fn validate_openai_embeddings(
    data: &Value,
    batch_length: usize,
) -> Result<Vec<Vec<f64>>, EmbeddingError> {
    let provider = EmbeddingProvider::OpenAi;
    let Some(response_items) = data.get("data").and_then(Value::as_array) else {
        return Err(invalid(provider, "data 不是数组".to_string()));
    };

    let mut errors: Vec<String> = Vec::new();
    if response_items.len() != batch_length {
        errors.push(format!("数量 {}，期望 {batch_length}", response_items.len()));
    }

    let mut seen_indexes: Vec<usize> = Vec::new();
    let mut embeddings_by_index: Vec<Option<Value>> = vec![None; batch_length];

    for item in response_items {
        let embedding = if item.is_object() {
            item.get("embedding").filter(|value| value.is_array())
        } else {
            None
        };
        let Some(embedding) = embedding else {
            errors.push("data 项缺少 embedding 数组".to_string());
            continue;
        };

        let raw_index = item.get("index");
        let integer_index = raw_index
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && value.fract() == 0.0);
        let Some(integer_index) = integer_index else {
            let label = match raw_index {
                Some(value) => js_string_of(value),
                None => "undefined".to_string(),
            };
            errors.push(format!("index {label} 不是整数"));
            continue;
        };
        if integer_index < 0.0 || integer_index >= batch_length as f64 {
            errors.push(format!(
                "index {} 超出范围 0..{}",
                integer_index,
                batch_length.saturating_sub(1)
            ));
            continue;
        }
        let index = integer_index as usize;
        if seen_indexes.contains(&index) {
            errors.push(format!("index {index} 重复"));
            continue;
        }

        seen_indexes.push(index);
        embeddings_by_index[index] = Some(embedding.clone());
    }

    let missing: Vec<String> = (0..batch_length)
        .filter(|index| !seen_indexes.contains(index))
        .map(|index| index.to_string())
        .collect();
    if !missing.is_empty() {
        errors.push(format!("index 覆盖不完整，缺少 {}", missing.join(", ")));
    }
    if !errors.is_empty() {
        return Err(invalid(provider, errors.join("；")));
    }

    // 覆盖完整 → 无 `None`；用 `Null` 兜底只是为了不 panic（真出现会被判为空向量）。
    let ordered: Vec<Value> = embeddings_by_index
        .into_iter()
        .map(|value| value.unwrap_or(Value::Null))
        .collect();
    validate_embedding_vectors(provider, &ordered)
}

/// 对齐 `validateGeminiEmbeddings()`：Gemini 侧不携带 `index`，按返回顺序取值。
pub fn validate_gemini_embeddings(
    data: &Value,
    batch_length: usize,
) -> Result<Vec<Vec<f64>>, EmbeddingError> {
    let provider = EmbeddingProvider::Gemini;
    let Some(response_items) = data.get("embeddings").and_then(Value::as_array) else {
        return Err(invalid(provider, "embeddings 不是数组".to_string()));
    };

    let mut errors: Vec<String> = Vec::new();
    if response_items.len() != batch_length {
        errors.push(format!("数量 {}，期望 {batch_length}", response_items.len()));
    }

    let mut embeddings: Vec<Value> = Vec::new();
    for item in response_items {
        let values = if item.is_object() {
            item.get("values").filter(|value| value.is_array())
        } else {
            None
        };
        match values {
            Some(values) => embeddings.push(values.clone()),
            None => errors.push("embeddings 项缺少 values 数组".to_string()),
        }
    }

    if !errors.is_empty() {
        return Err(invalid(provider, errors.join("；")));
    }
    validate_embedding_vectors(provider, &embeddings)
}

// ===== HTTP 调用 =====

/// 对齐 `embeddingHttpError()`（`new Error(...)`）。
fn embedding_http_error(provider: EmbeddingProvider, status: u16) -> EmbeddingError {
    EmbeddingError::Plain(format!(
        "{} Embedding 调用失败（HTTP {status}）。请检查 Base URL、网关或鉴权。",
        provider.label()
    ))
}

/// 对齐 `parseEmbeddingJsonResponse()`：解析失败时把 HTTP 状态写进文案。
async fn parse_embedding_json_response(
    provider: EmbeddingProvider,
    response: reqwest::Response,
) -> Result<Value, EmbeddingError> {
    let status = response.status().as_u16();
    match response.json::<Value>().await {
        Ok(value) => Ok(value),
        Err(_) => Err(invalid(
            provider,
            format!(
                "服务端返回非 JSON 响应（HTTP {status}；响应体无法解析为 JSON）。请检查 Base URL、网关或鉴权页。"
            ),
        )),
    }
}

/// 一次 Embedding 请求的公共信封（发送 → 状态码 → JSON 解析）。
async fn post_embedding_json(
    provider: EmbeddingProvider,
    request: reqwest::RequestBuilder,
) -> Result<Value, EmbeddingError> {
    let response = request
        .send()
        .await
        .map_err(|error| EmbeddingError::Network(error.to_string()))?;
    if !response.status().is_success() {
        return Err(embedding_http_error(provider, response.status().as_u16()));
    }
    parse_embedding_json_response(provider, response).await
}

/// 对齐 `embedOpenAI()`。
pub async fn embed_openai(
    client: &reqwest::Client,
    texts: &[String],
    model: &Value,
) -> Result<Vec<Vec<f64>>, EmbeddingError> {
    let base_url = model
        .get("baseUrl")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(compatible_base_url) = ollama_openai_embedding_base_url(base_url) {
        return Err(EmbeddingError::Plain(format!(
            "Ollama 原生 /api 地址不兼容：本应用使用 OpenAI-compatible Embedding API，请将 Base URL 改为 {compatible_base_url}。 This app uses the OpenAI-compatible Embedding API; change the Base URL to {compatible_base_url}."
        )));
    }

    let embedding_model = model_name_or(model, "text-embedding-3-small");
    let api_key = model
        .get("apiKey")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let url = build_openai_embedding_url(base_url);
    let body = json!({ "model": embedding_model, "input": texts });

    let data = post_embedding_json(
        EmbeddingProvider::OpenAi,
        client
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {api_key}"),
            )
            .body(serde_json::to_string(&body).map_err(|error| {
                EmbeddingError::Plain(format!("请求体序列化失败：{error}"))
            })?),
    )
    .await?;

    validate_openai_embeddings(&data, texts.len())
}

/// 对齐 `embedGemini()`（`batchEmbedContents` 批量端点）。
pub async fn embed_gemini(
    client: &reqwest::Client,
    texts: &[String],
    model: &Value,
) -> Result<Vec<Vec<f64>>, EmbeddingError> {
    let base_url = model
        .get("baseUrl")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let api_key = model
        .get("apiKey")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let embedding_model = model_name_or(model, "text-embedding-004");

    let url = format!(
        "{}/v1beta/models/{embedding_model}:batchEmbedContents",
        gemini_base_url(base_url)
    );
    let requests: Vec<Value> = texts
        .iter()
        .map(|text| {
            json!({
                "model": format!("models/{embedding_model}"),
                "content": { "parts": [{ "text": text }] },
                "taskType": "RETRIEVAL_DOCUMENT",
            })
        })
        .collect();
    let body = json!({ "requests": requests });

    let data = post_embedding_json(
        EmbeddingProvider::Gemini,
        client
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("x-goog-api-key", api_key)
            .body(serde_json::to_string(&body).map_err(|error| {
                EmbeddingError::Plain(format!("请求体序列化失败：{error}"))
            })?),
    )
    .await?;

    validate_gemini_embeddings(&data, texts.len())
}

/// 批量切分规格（纯函数）：对齐 `generateEmbeddings()` 的按批遍历。
///
/// `configured_batch_size` 为 `None` 时使用协议默认值（Gemini 100 / OpenAI 50）。
/// 基线对显式传入值还会走 `normalizeEmbeddingOptions` 归一（上下限与取整）；
/// 该路径当前**无调用面**，故未移植，仅做 `max(1)` 的防 panic 处理。
pub fn embedding_batches(
    texts_len: usize,
    protocol: &str,
    configured_batch_size: Option<usize>,
) -> Vec<std::ops::Range<usize>> {
    if texts_len == 0 {
        return Vec::new();
    }
    let default_batch_size = if protocol == "gemini" { 100 } else { 50 };
    let batch_size = configured_batch_size.unwrap_or(default_batch_size).max(1);
    (0..texts_len)
        .step_by(batch_size)
        .map(|start| start..std::cmp::min(start + batch_size, texts_len))
        .collect()
}

/// 对齐 `generateEmbeddings()`：空输入短路 → 按批量切片 → 逐批调用 → 整体再校验。
pub async fn generate_embeddings(
    client: &reqwest::Client,
    texts: &[String],
    protocol: &str,
    model: &Value,
    configured_batch_size: Option<usize>,
) -> Result<Vec<Vec<f64>>, EmbeddingError> {
    let provider = if protocol == "gemini" {
        EmbeddingProvider::Gemini
    } else {
        EmbeddingProvider::OpenAi
    };

    let mut results: Vec<Vec<f64>> = Vec::new();
    for batch in embedding_batches(texts.len(), protocol, configured_batch_size) {
        let embeddings = if protocol == "gemini" {
            embed_gemini(client, &texts[batch], model).await?
        } else {
            embed_openai(client, &texts[batch], model).await?
        };
        results.extend(embeddings);
    }

    // 基线在末尾对**整体结果**再校验一次（跨批次的维度一致性）。
    let results_as_values: Vec<Value> = results
        .iter()
        .map(|vector| json!(vector))
        .collect();
    validate_embedding_vectors(provider, &results_as_values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_native_api_is_detected_and_migrated_test() {
        assert_eq!(
            ollama_openai_embedding_base_url("http://localhost:11434/api").as_deref(),
            Some("http://localhost:11434/v1")
        );
        assert_eq!(
            ollama_openai_embedding_base_url("http://127.0.0.1:11434/api/").as_deref(),
            Some("http://127.0.0.1:11434/v1")
        );
        // 以下形态都**不**命中（与 JS 严格判定一致）
        for rejected in [
            "https://localhost:11434/api",      // 非 http
            "http://localhost:11435/api",       // 端口不同
            "http://ollama.local:11434/api",    // 非本地白名单主机
            "http://localhost:11434/api/v1",    // 路径不同
            "http://localhost:11434/v1",        // 已兼容
            "http://user:pass@localhost:11434/api",
            "http://localhost:11434/api?x=1",
            "http://localhost:11434/api#frag",
            "not-a-url",
        ] {
            assert_eq!(ollama_openai_embedding_base_url(rejected), None, "{rejected}");
        }
    }

    #[test]
    fn openai_embedding_url_follows_baseline_inference_rules_test() {
        assert_eq!(
            build_openai_embedding_url("https://api.openai.com"),
            "https://api.openai.com/v1/embeddings",
            "已知兼容根址补 /v1"
        );
        assert_eq!(
            build_openai_embedding_url("https://api.openai.com/"),
            "https://api.openai.com/v1/embeddings"
        );
        assert_eq!(
            build_openai_embedding_url("https://api.example.com/v1"),
            "https://api.example.com/v1/embeddings",
            "带版本后缀"
        );
        assert_eq!(
            build_openai_embedding_url("https://host/openai/v1beta2"),
            "https://host/openai/v1beta2/embeddings",
            "版本后缀允许字母数字点横线"
        );
        assert_eq!(
            build_openai_embedding_url("https://api.example.com/v1/embeddings"),
            "https://api.example.com/v1/embeddings",
            "已是 embeddings 端点"
        );
        assert_eq!(
            build_openai_embedding_url("https://host/api"),
            "https://host/api/embeddings",
            "用户给出的 /api 不得被推断为兼容根址"
        );
        assert_eq!(build_openai_embedding_url(""), "/embeddings");
    }

    #[test]
    fn gemini_base_url_strips_only_one_trailing_slash_test() {
        assert_eq!(gemini_base_url("https://host/v1beta/"), "https://host/v1beta");
        assert_eq!(
            gemini_base_url("https://host/v1beta//"),
            "https://host/v1beta/",
            "对齐 replace(/\\/$/, '') 只去一个"
        );
    }

    #[test]
    fn embedding_vectors_require_dimension_and_finite_numbers_test() {
        let provider = EmbeddingProvider::OpenAi;
        let ok = validate_embedding_vectors(
            provider,
            &[json!([0.1, 0.2]), json!([0.3, -0.4])],
        )
        .unwrap();
        assert_eq!(ok, vec![vec![0.1, 0.2], vec![0.3, -0.4]]);

        assert_eq!(
            validate_embedding_vectors(provider, &[json!([])]).unwrap_err(),
            invalid(provider, "第 1 个向量为空或不是数组".to_string())
        );
        assert_eq!(
            validate_embedding_vectors(provider, &[json!("nope")]).unwrap_err(),
            invalid(provider, "第 1 个向量为空或不是数组".to_string())
        );
        assert_eq!(
            validate_embedding_vectors(provider, &[json!([0.1, 0.2]), json!([0.3])]).unwrap_err(),
            invalid(provider, "第 2 个向量为 1 维，期望 2 维".to_string())
        );
        assert_eq!(
            validate_embedding_vectors(provider, &[json!([0.1, "x"])]).unwrap_err(),
            invalid(
                provider,
                "第 1 个向量的第 2 个值不是有限数字（收到 x）".to_string()
            )
        );
        assert_eq!(
            validate_embedding_vectors(provider, &[json!([0.1, null])]).unwrap_err(),
            invalid(
                provider,
                "第 1 个向量的第 2 个值不是有限数字（收到 null）".to_string()
            )
        );
    }

    #[test]
    fn openai_embeddings_require_complete_unique_indexes_test() {
        let provider = EmbeddingProvider::OpenAi;

        let ok = validate_openai_embeddings(
            &json!({"data": [
                {"index": 1, "embedding": [0.3, 0.4]},
                {"index": 0, "embedding": [0.1, 0.2]}
            ]}),
            2,
        )
        .unwrap();
        assert_eq!(ok, vec![vec![0.1, 0.2], vec![0.3, 0.4]], "必须按 index 重排");

        assert_eq!(
            validate_openai_embeddings(&json!({"data": []}), 1).unwrap_err(),
            invalid(provider, "数量 0，期望 1；index 覆盖不完整，缺少 0".to_string())
        );
        assert_eq!(
            validate_openai_embeddings(&json!({"nope": []}), 1).unwrap_err(),
            invalid(provider, "data 不是数组".to_string())
        );
        assert_eq!(
            validate_openai_embeddings(&json!({"data": [{"index": 0}]}), 1).unwrap_err(),
            invalid(provider, "data 项缺少 embedding 数组；index 覆盖不完整，缺少 0".to_string()),
            "基线在 continue 后仍会汇总缺失 index"
        );
        assert_eq!(
            validate_openai_embeddings(&json!({"data": [{"index": 1.5, "embedding": [1.0]}]}), 1)
                .unwrap_err(),
            invalid(provider, "index 1.5 不是整数；index 覆盖不完整，缺少 0".to_string())
        );
        assert_eq!(
            validate_openai_embeddings(&json!({"data": [{"index": null, "embedding": [1.0]}]}), 1)
                .unwrap_err(),
            invalid(provider, "index null 不是整数；index 覆盖不完整，缺少 0".to_string())
        );
        assert_eq!(
            validate_openai_embeddings(&json!({"data": [{"index": 0, "embedding": [1.0]}]}), 2)
                .unwrap_err(),
            invalid(provider, "数量 1，期望 2；index 覆盖不完整，缺少 1".to_string())
        );
        assert_eq!(
            validate_openai_embeddings(
                &json!({"data": [
                    {"index": 0, "embedding": [1.0]},
                    {"index": 0, "embedding": [2.0]}
                ]}),
                1
            )
            .unwrap_err(),
            invalid(provider, "数量 2，期望 1；index 0 重复".to_string())
        );
        assert_eq!(
            validate_openai_embeddings(&json!({"data": [{"index": -1, "embedding": [1.0]}]}), 1)
                .unwrap_err(),
            invalid(provider, "index -1 超出范围 0..0；index 覆盖不完整，缺少 0".to_string())
        );
        assert_eq!(
            validate_openai_embeddings(&json!({"data": [{"index": 1, "embedding": [1.0]}]}), 1)
                .unwrap_err(),
            invalid(provider, "index 1 超出范围 0..0；index 覆盖不完整，缺少 0".to_string())
        );
    }

    #[test]
    fn gemini_embeddings_are_order_based_test() {
        let provider = EmbeddingProvider::Gemini;
        let ok = validate_gemini_embeddings(&json!({"embeddings": [
            {"values": [0.1, 0.2]},
            {"values": [0.3, 0.4]}
        ]}), 2)
        .unwrap();
        assert_eq!(ok, vec![vec![0.1, 0.2], vec![0.3, 0.4]]);

        assert_eq!(
            validate_gemini_embeddings(&json!({"models": []}), 1).unwrap_err(),
            invalid(provider, "embeddings 不是数组".to_string())
        );
        assert_eq!(
            validate_gemini_embeddings(&json!({"embeddings": [{"values": []}]}), 1).unwrap_err(),
            invalid(provider, "第 1 个向量为空或不是数组".to_string())
        );
        assert_eq!(
            validate_gemini_embeddings(&json!({"embeddings": [{"nope": 1}]}), 1).unwrap_err(),
            invalid(provider, "embeddings 项缺少 values 数组".to_string())
        );
    }

    #[test]
    fn error_display_matches_baseline_string_error_prefixes_test() {
        assert_eq!(
            EmbeddingError::Plain("boo".to_string()).display_text(),
            "Error: boo"
        );
        assert_eq!(
            embedding_http_error(EmbeddingProvider::OpenAi, 500).display_text(),
            "Error: OpenAI Embedding 调用失败（HTTP 500）。请检查 Base URL、网关或鉴权。"
        );
        assert_eq!(
            embedding_http_error(EmbeddingProvider::Gemini, 401).display_text(),
            "Error: Gemini Embedding 调用失败（HTTP 401）。请检查 Base URL、网关或鉴权。"
        );
        assert_eq!(
            invalid(EmbeddingProvider::OpenAi, "data 不是数组".to_string()).display_text(),
            "EmbeddingResponseValidationError: OpenAI Embedding 响应无效：data 不是数组"
        );
        assert_eq!(
            EmbeddingError::Network("连接被拒绝".to_string()).display_text(),
            "Error: 连接被拒绝"
        );
    }

    #[test]
    fn embedding_batches_match_protocol_defaults_and_short_circuit_empty_test() {
        // 空输入：不产生任何批次 → `generate_embeddings` 不会发起请求。
        assert!(embedding_batches(0, "openai", None).is_empty());
        assert!(embedding_batches(0, "gemini", None).is_empty());

        assert_eq!(embedding_batches(3, "openai", None), vec![0..3]);
        assert_eq!(embedding_batches(50, "openai", None), vec![0..50]);
        assert_eq!(embedding_batches(51, "openai", None), vec![0..50, 50..51]);
        assert_eq!(embedding_batches(100, "gemini", None), vec![0..100]);
        assert_eq!(embedding_batches(101, "gemini", None), vec![0..100, 100..101]);
        // 显式批量优先，且 0 被抬升为 1（避免空进度死循环）
        assert_eq!(embedding_batches(3, "openai", Some(2)), vec![0..2, 2..3]);
        assert_eq!(embedding_batches(2, "openai", Some(0)), vec![0..1, 1..2]);
        // 非 gemini 协议一律走 OpenAI 默认值
        assert_eq!(embedding_batches(101, "openai-compatible", None), vec![0..50, 50..100, 100..101]);
    }
}
