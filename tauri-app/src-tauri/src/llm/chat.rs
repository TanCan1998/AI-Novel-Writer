//! 批次 D2-b：LLM 生成链共享类型、`<think>` 剥离与 HTTP 客户端装配。
//!
//! 迁移自 `electron/llm/openai-provider.ts` / `gemini-provider.ts` 的**共用部分**
//! 以及 `electron/controllers/llm-controller.ts` 的 `applyProxyConfig()`：
//!
//! | 本模块项 | 基线来源 |
//! |---|---|
//! | [`LlmFinishReason`] / [`TokenUsage`] / [`LlmResponse`] | `src/shared/ipc-channels.ts` |
//! | [`LlmGenerateOptions`] / [`LlmStreamOptions`] | `electron/llm/provider.interface.ts` |
//! | [`strip_thinking`] | `OpenAIProvider.stripThinking()`（三条替换规则） |
//! | [`build_client`] / [`proxy_from_config`] | `llm-controller.ts::applyProxyConfig()` |
//!
//! ## 与基线的刻意差异（均为环境差异，不是语义放宽）
//!
//! 1. **代理装配方式**：基线写 `process.env.HTTP_PROXY/HTTPS_PROXY` 让 `fetch` 读取；
//!    Rust 侧改为在 `reqwest::Client` 上显式装配 [`reqwest::Proxy`]。语义等价，且不会
//!    让「多窗口各自的项目设置」通过进程环境互相污染。
//! 2. **取消机制**：基线的 `AbortSignal` 立即中断在途请求；Rust 侧用原子探针
//!    （见 [`LlmStreamOptions::is_cancelled`]），在下一个 chunk 到达时终止。LLM 流式
//!    场景下 chunk 间隔极短，取消延迟可忽略；终止后 `Response` 被 drop 即关闭连接。
//! 3. **错误文案前缀**：基线的 `String(error)` 对 `Error` 实例会带上 `"Error: "`
//!    前缀；Rust 侧经 [`provider_error_text`] 复刻同一格式，但异常对象名（如
//!    `TypeError`）无法逐字复刻 —— 只保留 `"Error: "` 一档。

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::reasoning::ProviderReasoningDirective;

/// 生成终态（provider-neutral）。
///
/// `stop` 是唯一被下游工作流与 Agent 视为「完整回应」的终态；缺失供应商值时
/// 归一为 `unknown`，它保留文本可检视性，但**不是**可提交的证据。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmFinishReason {
    Stop,
    Length,
    ContentFilter,
    Cancelled,
    Error,
    Unknown,
}

impl LlmFinishReason {
    /// 序列化形式（与 `ipc-channels.ts` 的 `LLMFinishReason` 字面量一致）。
    pub fn as_str(self) -> &'static str {
        match self {
            LlmFinishReason::Stop => "stop",
            LlmFinishReason::Length => "length",
            LlmFinishReason::ContentFilter => "content_filter",
            LlmFinishReason::Cancelled => "cancelled",
            LlmFinishReason::Error => "error",
            LlmFinishReason::Unknown => "unknown",
        }
    }
}

/// token 用量（对齐基线 `TokenUsage`：三项均可为 `null`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
}

/// 非流式生成响应（对齐 `LLMResponse` 的 `{ success, content, finishReason, usage?, error? }`）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmResponse {
    pub success: bool,
    pub content: String,
    pub finish_reason: LlmFinishReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl LlmResponse {
    /// 供失败路径构造信封（`content` 恒为空 —— 对齐基线 provider 的 catch 分支）。
    pub fn failure(error: String) -> Self {
        LlmResponse {
            success: false,
            content: String::new(),
            finish_reason: LlmFinishReason::Error,
            usage: None,
            error: Some(error),
        }
    }
}

/// 一次生成请求的有效参数。
///
/// `temperature` 已由 [`crate::llm::params::resolve_generation_parameters`] 决议：
/// `None` 表示**必须省略**该字段（模型自行掌管温度），不得回退到模型档案默认值。
#[derive(Debug, Clone)]
pub struct LlmGenerateOptions {
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub response_format: Option<Value>,
    pub reasoning: Option<ProviderReasoningDirective>,
    /// 调用方作用域的逻辑会话身份，逐字透传给要求会话粘性的网关（opencode Go）。
    /// 缺失表示单请求作用域，**绝不**退化为进程级共享默认值。
    pub conversation_id: Option<String>,
}

/// 一条对话消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// 流式回调集合（对齐基线 `LLMStreamOptions` 的三个回调）。
pub struct StreamCallbacks<'a> {
    pub on_chunk: &'a (dyn Fn(&str) + Send + Sync),
    /// `finish_reason` 恒为归一后的终态证据。
    pub on_done: &'a (dyn Fn(String, Option<TokenUsage>, LlmFinishReason) + Send + Sync),
    /// `content` 是**已交付的可见候选文本**，绝不是隐藏推理内容。
    pub on_error: &'a (dyn Fn(String, Option<String>, Option<TokenUsage>) + Send + Sync),
}

/// 流式生成选项（生成参数 + 回调 + 取消探针）。
pub struct LlmStreamOptions<'a> {
    pub generate: LlmGenerateOptions,
    pub callbacks: StreamCallbacks<'a>,
    /// 取消探针：返回 `true` 时流式循环在下个 chunk 处终止（对齐基线 `AbortSignal`）。
    pub is_cancelled: &'a (dyn Fn() -> bool + Send + Sync),
}

/// 流式失败原因：取消与一般错误必须分开（对齐基线对 `AbortError` 的特判）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamFailure {
    Cancelled,
    Message(String),
}

impl StreamFailure {
    /// 归一为对用户可见的文案（取消文案对应用户主动中止）。
    pub fn into_message(self) -> String {
        match self {
            StreamFailure::Cancelled => "已取消生成".to_string(),
            StreamFailure::Message(message) => message,
        }
    }
}

/// 复刻基线 `String(error)` 对 `Error` 实例的 `"Error: "` 前缀。
///
/// 复用 [`crate::commands::db::mutating_error`] 以免出现第二份格式定义。
pub fn provider_error_text(error: String) -> String {
    crate::commands::mutating_error(error)
}

/// 规则 1：剥离 `<think>…</think>` 块；未闭合时延伸到输入末尾（`\z` 等价 JS 无 `m` 标志的 `$`）。
static THINK_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<think>[\s\S]*?(?:</think>|\z)").unwrap());
/// 规则 2：剥离**首个** `</think>` 之前的全部内容及其后空白（非全局替换，故用 `replace`）。
static THINK_HEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[\s\S]*?</think>\s*").unwrap());
/// 规则 3：剥离**首个**残留的 `<think>` / `</think>` 标签（非全局替换）。
static THINK_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</?think>").unwrap());

/// 对齐基线 `OpenAIProvider.stripThinking()`。
///
/// 三条规则的**顺序**与**全局/首个匹配**语义都必须一致：
/// 规则 1 是全局替换，规则 2/3 只替换首个匹配；最后 `trim()`。
pub fn strip_thinking(content: &str) -> String {
    let step1 = THINK_BLOCK.replace_all(content, "");
    let step2 = THINK_HEAD.replace(&step1, "");
    let step3 = THINK_TAG.replace(&step2, "");
    step3.trim().to_string()
}

/// 代理规格（对齐基线 `GlobalConfig.proxy` 的判定结果）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxySpec {
    /// `http` 或 `socks5`（基线仅支持这两档）。
    pub scheme: String,
    pub host: String,
    pub port: u64,
}

impl ProxySpec {
    /// 代理 URL（`reqwest::Proxy::all` 的入参形式）。
    pub fn url(&self) -> String {
        format!("{}://{}:{}", self.scheme, self.host, self.port)
    }
}

/// 从全局配置的 `proxy` 段解析代理（对齐基线 `applyProxyConfig()` 的判定顺序）。
///
/// 基线要求 `enabled === true` **且** `host` 非空才启用；`type === 'socks5'` 走
/// socks5，其余一律 http。配置读取本身失败时基线静默忽略 —— 此处同样返回 `None`。
pub fn proxy_from_config(config: &Value) -> Option<ProxySpec> {
    let proxy = config.get("proxy")?;
    if !proxy.get("enabled").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let host = proxy.get("host").and_then(Value::as_str)?.trim();
    if host.is_empty() {
        return None;
    }
    let port = proxy.get("port").and_then(Value::as_u64)?;
    let scheme = match proxy.get("type").and_then(Value::as_str) {
        Some("socks5") => "socks5",
        _ => "http",
    };
    Some(ProxySpec {
        scheme: scheme.to_string(),
        host: host.to_string(),
        port,
    })
}

/// 构建一次生成调用所用的 HTTP 客户端。
///
/// 刻意不设总超时：基线 `fetch` 无超时，长文生成可能持续数分钟。
pub fn build_client(proxy: Option<&ProxySpec>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder();
    if let Some(spec) = proxy {
        builder = builder.proxy(
            reqwest::Proxy::all(spec.url()).map_err(|error| format!("代理配置无效：{error}"))?,
        );
    }
    builder
        .build()
        .map_err(|error| format!("HTTP 客户端初始化失败：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strip_thinking_matches_baseline_rules_test() {
        // 规则 1：完整 think 块被剥离（全局）
        assert_eq!(strip_thinking("<think>推理中</think>正文"), "正文");
        // 规则 1：未闭合时延伸到输入末尾
        assert_eq!(strip_thinking("前文<think>推理中"), "前文");
        // 规则 2：无起始标签时剥离首个 `</think>` **之前的全部内容**
        assert_eq!(strip_thinking("推理中</think>\n\n正文"), "正文");
        assert_eq!(strip_thinking("正文</think>后半"), "后半");
        // 大小写不敏感
        assert_eq!(strip_thinking("<THINK>a</Think>正文"), "正文");
        // 多行（`[\s\S]` 语义）
        assert_eq!(strip_thinking("<think>a\nb</think>\n正文"), "正文");
    }

    #[test]
    fn strip_thinking_rules_2_and_3_replace_only_first_test() {
        // 规则 2/3 非全局（对齐基线 `.replace(re, '')` 无 `g` 标志）：
        // 规则 2 吃掉首个 `</think>` 及其前缀，规则 3 只消掉下一个残留标签，
        // 因此第三个 `</think>` 会被保留。
        assert_eq!(strip_thinking("a</think>b</think>c"), "bc");
        assert_eq!(strip_thinking("a</think>b</think>c</think>d"), "bc</think>d");
        // 规则 1 是全局替换：每个 `<think>` 都会吞到 `</think>` 或输入末尾
        assert_eq!(strip_thinking("x<think>y<think>z"), "x");
    }

    #[test]
    fn proxy_from_config_follows_baseline_guards_test() {
        assert_eq!(proxy_from_config(&json!({})), None);
        assert_eq!(
            proxy_from_config(&json!({"proxy": {"enabled": false, "host": "h", "port": 1}})),
            None
        );
        assert_eq!(
            proxy_from_config(&json!({"proxy": {"enabled": true, "host": "  ", "port": 1}})),
            None
        );
        assert_eq!(
            proxy_from_config(&json!({"proxy": {"enabled": true, "host": "h"}})),
            None
        );
        assert_eq!(
            proxy_from_config(&json!({"proxy": {"enabled": true, "host": "127.0.0.1", "port": 7890}})),
            Some(ProxySpec {
                scheme: "http".to_string(),
                host: "127.0.0.1".to_string(),
                port: 7890
            })
        );
        assert_eq!(
            proxy_from_config(
                &json!({"proxy": {"enabled": true, "host": "127.0.0.1", "port": 7891, "type": "socks5"}})
            )
            .map(|spec| spec.url()),
            Some("socks5://127.0.0.1:7891".to_string())
        );
    }

    #[test]
    fn provider_error_text_prefixes_js_error_shape_test() {
        assert_eq!(provider_error_text("失败".to_string()), "Error: 失败");
    }

    #[test]
    fn finish_reason_serializes_to_contract_literals_test() {
        for (reason, literal) in [
            (LlmFinishReason::Stop, "stop"),
            (LlmFinishReason::Length, "length"),
            (LlmFinishReason::ContentFilter, "content_filter"),
            (LlmFinishReason::Cancelled, "cancelled"),
            (LlmFinishReason::Error, "error"),
            (LlmFinishReason::Unknown, "unknown"),
        ] {
            assert_eq!(reason.as_str(), literal);
            assert_eq!(serde_json::to_value(reason).unwrap(), json!(literal));
        }
    }
}
