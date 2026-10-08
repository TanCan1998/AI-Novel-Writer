//! 批次 D2-b：LLM 生成 / 流式 / 取消 —— 迁移自 `electron/controllers/llm-controller.ts`
//! 的生成链部分（`llm:begin/close-execution-lease` 已在批次 D2-a 落地）。
//!
//! | 频道 | 命令 | 基线行为 |
//! |---|---|---|
//! | `llm:generate` | [`llm_generate`] | 非流式生成；**总是**返回信封（不 reject） |
//! | `llm:generate-stream` | [`llm_generate_stream`] | 立即返回 `{requestId, started}`，后台流式推送 3 个事件 |
//! | `llm:cancel` | [`llm_cancel`] | 记录 `cancelled` 统计 → 交付终态事件 → abort 任务 |
//!
//! | 事件 | 载荷 |
//! |---|---|
//! | `llm:stream-chunk` | `{ requestId, chunk }`（每 chunk 一发） |
//! | `llm:stream-done` | `{ requestId, fullText, usage?, finishReason }` |
//! | `llm:stream-error` | `{ requestId, error }` |
//!
//! ## 失败信封的两档语义（**不可统一**）
//!
//! - **模型不存在**（无租约且 `models.json` 无该 id）：信封错误文案为
//!   `未找到模型配置`（`llm:generate`）/ 无 `error` 字段（`llm:generate-stream`），
//!   **不带** `Error: ` 前缀 —— 这是基线的字面量返回分支。
//! - **异常路径**（租约解析失败、Kimi 温度越界、客户端构建失败等）：走
//!   [`provider_error_text`]，即 `"Error: "` 前缀 —— 复刻基线 `String(error)`。
//!
//! ## 调用统计（`llm_calls` 表）
//!
//! 统计是**诊断信息**：任何失败都必须被吞掉，绝不能改变生成结果（对齐基线注释
//! `Statistics are diagnostic only and must never change generation outcome.`）。
//! 且只有携带**当次冻结项目会话**的请求才会落库；陈旧会话一律丢弃。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::llm::chat::{
    build_client, is_gemini, provider_error_text, proxy_from_config, strip_thinking, ChatMessage,
    LlmFinishReason, LlmGenerateOptions, LlmResponse, LlmStreamOptions, StreamCallbacks,
    StreamFailure, TokenUsage,
};
use crate::llm::params::{
    resolve_generation_parameters, GenerationParameterModel, GenerationParameterRequest,
};
use crate::security::ProjectSessionContext;
use crate::state::AppState;

/// `llm:stream-chunk` 事件频道名。
pub const LLM_STREAM_CHUNK_EVENT: &str = "llm:stream-chunk";
/// `llm:stream-done` 事件频道名。
pub const LLM_STREAM_DONE_EVENT: &str = "llm:stream-done";
/// `llm:stream-error` 事件频道名。
pub const LLM_STREAM_ERROR_EVENT: &str = "llm:stream-error";

/// 模型不存在时的信封文案（基线 `llm:generate` 的字面量返回）。
///
/// 刻意命名区别于 [`crate::commands::llm_execution::MODEL_NOT_FOUND_MESSAGE`]
/// （后者是租约频道的结构化 `errorCode` 文案），避免 glob 再导出命名冲突。
pub const MODEL_NOT_FOUND_ENVELOPE_MESSAGE: &str = "未找到模型配置";

/// `llm:generate-stream` 响应（对齐契约 `{ requestId, started, error? }`）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmGenerateStreamStart {
    pub request_id: String,
    pub started: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `llm:cancel` 响应。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmCancelResult {
    pub success: bool,
}

/// `llm:stream-chunk` 事件载荷。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmStreamChunkEvent {
    pub request_id: String,
    pub chunk: String,
}

/// `llm:stream-done` 事件载荷。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmStreamDoneEvent {
    pub request_id: String,
    pub full_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    pub finish_reason: LlmFinishReason,
}

/// `llm:stream-error` 事件载荷。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmStreamErrorEvent {
    pub request_id: String,
    pub error: String,
}

/// 一次生成调用的统计上下文（冻结于请求开始时刻）。
#[derive(Debug, Clone)]
pub struct ProviderCallLog {
    pub(crate) model_id: String,
    pub(crate) model_name: String,
    pub(crate) purpose: String,
    /// 缺失/非法会话 → `None`，此时**绝不**写入项目统计。
    pub(crate) session: Option<ProjectSessionContext>,
    pub(crate) started_at_ms: u64,
}

/// 流式任务的共享观测快照。
///
/// 取消路径需要**立即**向前端交付一个终态事件：渲染层 `llm-store` 只在收到
/// `llm:stream-done` / `llm:stream-error` 时才 `cleanup()`，否则活跃请求永久停在
/// `running`。基线的 provider 在 `AbortSignal` 触发后抛出 `AbortError` 并自行
/// `fail()`；Rust 侧任务被 `abort()` 硬终止，不会走任何回调 —— 因此取消前的
/// 观测必须由命令层持有。
///
/// `full_text` 累积的正是**已交付的 chunk**（含 `<think>` 包装），与基线 provider
/// 内部的 `fullText` 同语义；可见候选一律经 [`strip_thinking`] 派生。
#[derive(Debug, Default)]
pub struct StreamSnapshot {
    full_text: Mutex<String>,
    usage: Mutex<Option<TokenUsage>>,
}

impl StreamSnapshot {
    /// 追加一段已交付文本（在推送 chunk 事件之前调用）。
    fn push(&self, chunk: &str) {
        if let Ok(mut text) = self.full_text.lock() {
            text.push_str(chunk);
        }
    }

    /// 记录最近一次收到的 token 用量。
    fn set_usage(&self, usage: &Option<TokenUsage>) {
        if let Ok(mut current) = self.usage.lock() {
            *current = usage.clone();
        }
    }

    /// 可见候选：为空时返回 `None`（对齐基线 `visibleCandidate || undefined`）。
    fn visible_candidate(&self) -> Option<String> {
        let visible = self
            .full_text
            .lock()
            .ok()
            .map(|text| strip_thinking(&text))
            .unwrap_or_default();
        if visible.is_empty() {
            None
        } else {
            Some(visible)
        }
    }

    fn usage(&self) -> Option<TokenUsage> {
        self.usage.lock().ok().and_then(|usage| usage.clone())
    }
}

/// `JoinHandle` 的薄包装：仅为满足 [`LlmStreamHandle`] 的 `Debug` 派生
/// （任务句柄没有稳定的调试表示，也不应参与日志）。
pub(crate) struct StreamTask(tauri::async_runtime::JoinHandle<()>);

impl std::fmt::Debug for StreamTask {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("StreamTask")
    }
}

/// 活跃流式任务的取消句柄（进程内存态，重启即失效）。
///
/// - `cancelled`：防御性退出信号（对齐基线传给 provider 的 `AbortSignal`）；
/// - `recorded`：「取消」与「终态」共享的一次性闸门 —— 对齐基线 `recordOnce`：
///   取消后流式任务仍会走 `onError`，但统计只落一次（`cancelled`）；
/// - `emitted`：终态**事件**的一次性闸门。取消时命令层会抢先交付事件，随后
///   任务自己的 `onError` 不得重复推送；
/// - `task`：让 `llm_cancel` 能 `abort()` 掉挂起中的流，使 `chunk().await`
///   立即结束（基线由 `AbortController` 关闭连接实现同等效果）；
/// - `snapshot`：取消时用于派生 `fullText` / `usage` 的观测快照。
#[derive(Debug)]
pub struct LlmStreamHandle {
    pub(crate) cancelled: Arc<AtomicBool>,
    pub(crate) recorded: Arc<AtomicBool>,
    pub(crate) emitted: Arc<AtomicBool>,
    pub(crate) task: Arc<Mutex<Option<StreamTask>>>,
    pub(crate) snapshot: Arc<StreamSnapshot>,
    pub(crate) log: Option<ProviderCallLog>,
}

/// 一次调用结果的统计视图。
struct ProviderOutcome {
    success: bool,
    usage: Option<TokenUsage>,
    error: Option<String>,
}

/// 当前时间（毫秒）。时钟不可用时回退 0（统计不是关键路径）。
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// 记录一次 provider 结果。
///
/// 三条守卫，逐条对齐基线 `recordProviderOutcome`：
/// 1. 无会话上下文 → 直接返回（一次性调用不进项目统计）；
/// 2. 会话与当前活跃项目不匹配（陈旧/伪造）→ 丢弃；
/// 3. 落库失败 → 吞掉（诊断信息不得影响生成结果）。
fn record_provider_outcome(
    state: &AppState,
    log: Option<&ProviderCallLog>,
    outcome: &ProviderOutcome,
) {
    let Some(log) = log else {
        return;
    };
    let Some(session) = log.session.as_ref() else {
        return;
    };
    let active = state.active_project_snapshot();
    if crate::security::assert_current_project_context(session, active.as_ref()).is_err() {
        return;
    }
    let usage = outcome.usage.as_ref();
    let call = json!({
        "modelId": log.model_id,
        "modelName": log.model_name,
        "purpose": log.purpose,
        "promptTokens": usage.and_then(|usage| usage.prompt_tokens),
        "completionTokens": usage.and_then(|usage| usage.completion_tokens),
        "totalTokens": usage.and_then(|usage| usage.total_tokens),
        "durationMs": now_ms().saturating_sub(log.started_at_ms),
        "success": outcome.success,
        "errorMessage": outcome.error,
    });
    let _ = state.with_project_db(|conn| crate::repositories::llm_repository::log_call(conn, &call));
}

/// 解析 `request.projectSession`（对齐 `isProjectSessionContext` 的三字段判别）。
///
/// 刻意不交给 Tauri 反序列化：基线对**非法会话**是「静默不落统计」，
/// 而不是拒绝整个生成请求。
fn parse_project_session(value: Option<&Value>) -> Option<ProjectSessionContext> {
    let object = value?.as_object()?;
    let project_id = object.get("projectId")?.as_str()?;
    let lease_id = object.get("leaseId")?.as_str()?;
    let project_path = object.get("projectPath")?.as_str()?;
    Some(ProjectSessionContext {
        project_id: project_id.to_string(),
        lease_id: lease_id.to_string(),
        project_path: project_path.to_string(),
    })
}

/// 请求准备阶段的两档失败（见模块文档「失败信封的两档语义」）。
enum PrepareFailure {
    /// 模型不存在（无租约且 `models.json` 无该 id）—— 基线的字面量返回分支。
    ModelNotFound,
    /// 异常路径。`log` 仅在**模型快照已解析之后**产生 —— 对齐基线 `llm:generate`
    /// catch 分支里的 `if (model) recordProviderOutcome(...)`：参数决议 / 消息解析 /
    /// 客户端构建失败都要留下失败统计，而租约解析失败（model 仍为 null）不记录。
    Error {
        message: String,
        log: Option<ProviderCallLog>,
    },
}

/// 已解析的请求：模型快照 + 消息 + 有效参数 + HTTP 客户端 + 统计上下文。
struct PreparedRequest {
    model: Value,
    messages: Vec<ChatMessage>,
    options: LlmGenerateOptions,
    client: reqwest::Client,
    log: Option<ProviderCallLog>,
}

/// 解析模型快照：有租约则用冻结快照（权威），否则回落到 `models.json`。
///
/// `Ok(None)` = 模型不存在（字面量分支），`Err` = 租约异常（异常分支）。
fn resolve_model_for_request(state: &AppState, request: &Value) -> Result<Option<Value>, String> {
    if let Some(lease_id) = request
        .get("modelExecutionLeaseId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        let mut leases = state
            .llm_leases
            .lock()
            .map_err(|_| "模型执行租约注册表不可用".to_string())?;
        return leases.resolve_model_at(lease_id, now_ms()).map(Some);
    }
    let model_id = request.get("modelId").and_then(Value::as_str).unwrap_or_default();
    Ok(crate::commands::read_models_at(&crate::app_paths::models_config_path())
        .into_iter()
        .find(|model| model.get("id").and_then(Value::as_str) == Some(model_id)))
}

/// 同步准备阶段：模型 / 参数 / 消息 / 客户端 / 统计上下文。
fn prepare_request(state: &AppState, request: &Value) -> Result<PreparedRequest, PrepareFailure> {
    let started_at_ms = now_ms();
    let model = match resolve_model_for_request(state, request) {
        Ok(Some(model)) => model,
        Ok(None) => return Err(PrepareFailure::ModelNotFound),
        // 租约解析失败发生在模型快照确定之前：基线此时 `model` 为 null，不记统计。
        Err(message) => {
            return Err(PrepareFailure::Error {
                message,
                log: None,
            })
        }
    };

    let model_id = model.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
    let model_name = model
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| model.get("modelName").and_then(Value::as_str).unwrap_or_default())
        .to_string();
    let purpose = request
        .get("purpose")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|purpose| !purpose.is_empty())
        .unwrap_or("generation")
        .to_string();
    let session = parse_project_session(request.get("projectSession"));
    // 模型快照已确定 → 此后任何失败都应带上统计上下文（对齐基线 catch 的 `if (model)`）。
    let build_log = || ProviderCallLog {
        model_id: model_id.clone(),
        model_name: model_name.clone(),
        purpose: purpose.clone(),
        session: session.clone(),
        started_at_ms,
    };

    let options = {
        let parameter_model = GenerationParameterModel::from_value(&model);
        let parameter_request = GenerationParameterRequest {
            max_tokens: request.get("maxTokens").and_then(Value::as_u64),
            // `null` 在基线是「未提供」，不得当成 `{type: null}` 下发。
            response_format: request
                .get("responseFormat")
                .filter(|value| !value.is_null())
                .cloned(),
            creative_strategy: request.get("creativeStrategy").and_then(Value::as_str),
            reasoning_stage: request.get("reasoningStage").and_then(Value::as_str),
        };
        let resolved = resolve_generation_parameters(&parameter_model, &parameter_request)
            .map_err(|message| PrepareFailure::Error {
                message,
                log: Some(build_log()),
            })?;
        LlmGenerateOptions {
            temperature: resolved.temperature,
            max_tokens: resolved.max_tokens,
            response_format: resolved.response_format,
            reasoning: resolved.reasoning,
            conversation_id: request
                .get("modelExecutionLeaseId")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        }
    };

    let messages: Vec<ChatMessage> = serde_json::from_value(
        request
            .get("messages")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    )
    .map_err(|error| PrepareFailure::Error {
        message: format!("messages 结构无效：{error}"),
        log: Some(build_log()),
    })?;

    // 代理在调用时读取一次全局配置（对齐基线每次生成前 `applyProxyConfig()`）。
    let config = crate::commands::read_global_config_at(&crate::app_paths::global_config_path());
    let proxy = proxy_from_config(&config);
    let client = build_client(proxy.as_ref()).map_err(|message| PrepareFailure::Error {
        message,
        log: Some(build_log()),
    })?;

    Ok(PreparedRequest {
        model,
        messages,
        options,
        client,
        log: Some(build_log()),
    })
}

impl PrepareFailure {
    /// `llm:generate` 的失败信封。
    fn into_response(self) -> LlmResponse {
        match self {
            PrepareFailure::ModelNotFound => LlmResponse {
                success: false,
                content: String::new(),
                finish_reason: LlmFinishReason::Error,
                usage: None,
                error: Some(MODEL_NOT_FOUND_ENVELOPE_MESSAGE.to_string()),
            },
            PrepareFailure::Error { message, .. } => {
                LlmResponse::failure(provider_error_text(message))
            }
        }
    }

    /// `llm:generate-stream` 的失败信封（模型不存在时**不带** `error` 字段）。
    fn into_start(self, request_id: String) -> LlmGenerateStreamStart {
        match self {
            PrepareFailure::ModelNotFound => LlmGenerateStreamStart {
                request_id,
                started: false,
                error: None,
            },
            PrepareFailure::Error { message, .. } => LlmGenerateStreamStart {
                request_id,
                started: false,
                error: Some(provider_error_text(message)),
            },
        }
    }

    /// 失败统计上下文（仅「模型快照已解析之后」的失败存在）。
    fn log(&self) -> Option<&ProviderCallLog> {
        match self {
            PrepareFailure::Error { log, .. } => log.as_ref(),
            PrepareFailure::ModelNotFound => None,
        }
    }

    /// 供失败统计使用的错误文案（与 [`Self::into_response`] 的 `error` 字段一致）。
    fn statistics_error(&self) -> Option<String> {
        match self {
            PrepareFailure::ModelNotFound => None,
            PrepareFailure::Error { message, .. } => Some(provider_error_text(message.clone())),
        }
    }
}

/// `llm:generate` —— 非流式生成。
///
/// 契约上是「总是成功返回信封」：解析失败与 provider 失败都装进
/// [`LlmResponse`] 的 `success: false` 分支，不使用 invoke reject。
#[tauri::command]
pub async fn llm_generate(app: AppHandle, request: Value) -> LlmResponse {
    // 同步准备阶段：显式限定借用 `State` 的作用域，避免其跨 `.await` 存活。
    let prepared = {
        let state = app.state::<AppState>();
        prepare_request(&state, &request)
    };
    let PreparedRequest {
        model,
        messages,
        options,
        client,
        log,
    } = match prepared {
        Ok(prepared) => prepared,
        Err(failure) => {
            // 基线 catch 分支：`if (model) recordProviderOutcome(..., { success: false,
            // error: String(error) })` —— 模型已解析后的一切准备失败都要留下痕迹。
            let prepared_log = failure.log().cloned();
            let prepared_error = failure.statistics_error();
            if let (Some(prepared_log), Some(prepared_error)) = (prepared_log, prepared_error) {
                let state = app.state::<AppState>();
                record_provider_outcome(
                    &state,
                    Some(&prepared_log),
                    &ProviderOutcome {
                        success: false,
                        usage: None,
                        error: Some(prepared_error),
                    },
                );
            }
            return failure.into_response();
        }
    };

    let response = if is_gemini(&model) {
        crate::llm::gemini::generate(&client, &model, &messages, &options).await
    } else {
        crate::llm::openai::generate(&client, &model, &messages, &options).await
    };

    {
        let state = app.state::<AppState>();
        // 基线 `recordProviderOutcome` 的 `errorMessage`：非 `stop` 终态一律记为
        // `finish:<reason>`，忽略信封文案（信封文案仍原样返回给前端）。
        let statistics_error = if response.finish_reason == LlmFinishReason::Stop {
            None
        } else {
            Some(format!("finish:{}", response.finish_reason.as_str()))
        };
        record_provider_outcome(
            &state,
            log.as_ref(),
            &ProviderOutcome {
                success: response.success,
                usage: response.usage.clone(),
                error: statistics_error,
            },
        );
    }
    response
}

/// 交付取消/中止的终态事件（与 provider 的 `on_error` 分支同构）。
///
/// 有可见候选 → `llm:stream-done`（`finishReason: 'error'`，携带部分文本）；
/// 否则 → `llm:stream-error`。
fn emit_terminal_error(
    webview: &tauri::WebviewWindow,
    request_id: &str,
    error: String,
    snapshot: &StreamSnapshot,
) {
    match snapshot.visible_candidate() {
        Some(full_text) => {
            let _ = webview.emit(
                LLM_STREAM_DONE_EVENT,
                LlmStreamDoneEvent {
                    request_id: request_id.to_string(),
                    full_text,
                    usage: snapshot.usage(),
                    finish_reason: LlmFinishReason::Error,
                },
            );
        }
        None => {
            let _ = webview.emit(
                LLM_STREAM_ERROR_EVENT,
                LlmStreamErrorEvent {
                    request_id: request_id.to_string(),
                    error,
                },
            );
        }
    }
}

/// 从活跃流式注册表移除（幂等）。
fn remove_stream(app: &AppHandle, request_id: &str) {
    if let Ok(mut streams) = app.state::<AppState>().llm_streams.lock() {
        streams.remove(request_id);
    }
}

/// 后台流式任务：驱动 provider 并把三个回调翻译为 Tauri 事件。
async fn run_stream(
    app: AppHandle,
    webview: tauri::WebviewWindow,
    request_id: String,
    prepared: PreparedRequest,
    cancelled: Arc<AtomicBool>,
    recorded: Arc<AtomicBool>,
    emitted: Arc<AtomicBool>,
    snapshot: Arc<StreamSnapshot>,
) {
    let PreparedRequest {
        model,
        messages,
        options,
        client,
        log,
    } = prepared;

    let is_cancelled = || cancelled.load(Ordering::SeqCst);

    // 极端窗口：`llm_cancel` 在任务回填 JoinHandle **之前**到达时无法 `abort()`，
    // 此时探针已经置位 —— 任务必须在发起网络请求前就认输（事件与统计已由取消方
    // 交付）。**不得**在此移除注册表条目：同 id 重发时那个条目已属于新任务。
    if is_cancelled() {
        return;
    }

    let chunk_id = request_id.clone();
    let chunk_webview = webview.clone();
    let chunk_snapshot = snapshot.clone();
    let on_chunk = move |chunk: &str| {
        // 先登记观测再交付，保证取消时拿到的文本与前端已见内容一致。
        chunk_snapshot.push(chunk);
        let _ = chunk_webview.emit(
            LLM_STREAM_CHUNK_EVENT,
            LlmStreamChunkEvent {
                request_id: chunk_id.clone(),
                chunk: chunk.to_string(),
            },
        );
    };

    let done_id = request_id.clone();
    let done_webview = webview.clone();
    let done_recorded = recorded.clone();
    let done_emitted = emitted.clone();
    let done_snapshot = snapshot.clone();
    let done_log = log.clone();
    let done_app = app.clone();
    let on_done = move |full_text: String, usage: Option<TokenUsage>, finish_reason: LlmFinishReason| {
        let success = finish_reason == LlmFinishReason::Stop;
        done_snapshot.set_usage(&usage);
        // 取消已抢先交付终态事件时，任务侧不得重复推送，也不必重复记账。
        if !done_emitted.swap(true, Ordering::SeqCst) {
            if !done_recorded.swap(true, Ordering::SeqCst) {
                let state = done_app.state::<AppState>();
                record_provider_outcome(
                    &state,
                    done_log.as_ref(),
                    &ProviderOutcome {
                        success,
                        usage: usage.clone(),
                        error: if success {
                            None
                        } else {
                            Some(format!("finish:{}", finish_reason.as_str()))
                        },
                    },
                );
            }
            let _ = done_webview.emit(
                LLM_STREAM_DONE_EVENT,
                LlmStreamDoneEvent {
                    request_id: done_id.clone(),
                    full_text,
                    usage,
                    finish_reason,
                },
            );
        }
        remove_stream(&done_app, &done_id);
    };

    let error_id = request_id.clone();
    let error_webview = webview.clone();
    let error_recorded = recorded.clone();
    let error_emitted = emitted.clone();
    let error_snapshot = snapshot.clone();
    let error_log = log.clone();
    let error_app = app.clone();
    let on_error = move |error: String, content: Option<String>, usage: Option<TokenUsage>| {
        error_snapshot.set_usage(&usage);
        if !error_emitted.swap(true, Ordering::SeqCst) {
            if !error_recorded.swap(true, Ordering::SeqCst) {
                let state = error_app.state::<AppState>();
                record_provider_outcome(
                    &state,
                    error_log.as_ref(),
                    &ProviderOutcome {
                        success: false,
                        usage: usage.clone(),
                        error: Some(error.clone()),
                    },
                );
            }
            match content {
                // 已有可见候选：以 `finishReason: 'error'` 的 done 事件收尾（对齐基线）
                Some(full_text) => {
                    let _ = error_webview.emit(
                        LLM_STREAM_DONE_EVENT,
                        LlmStreamDoneEvent {
                            request_id: error_id.clone(),
                            full_text,
                            usage,
                            finish_reason: LlmFinishReason::Error,
                        },
                    );
                }
                None => {
                    let _ = error_webview.emit(
                        LLM_STREAM_ERROR_EVENT,
                        LlmStreamErrorEvent {
                            request_id: error_id.clone(),
                            error,
                        },
                    );
                }
            }
        }
        remove_stream(&error_app, &error_id);
    };

    let callbacks = StreamCallbacks {
        on_chunk: &on_chunk,
        on_done: &on_done,
        on_error: &on_error,
    };
    let stream_options = LlmStreamOptions {
        generate: options,
        callbacks,
        is_cancelled: &is_cancelled,
    };

    if is_gemini(&model) {
        crate::llm::gemini::generate_stream(&client, &model, &messages, &stream_options).await;
    } else {
        crate::llm::openai::generate_stream(&client, &model, &messages, &stream_options).await;
    }

    // 兜底清理：正常路径已由 done/error 回调移除，取消路径由 `llm_cancel` 移除。
    remove_stream(&app, &request_id);
}

/// `llm:generate-stream` —— 启动后台流式生成并立即返回。
///
/// 契约上 `started: true` 只表示**任务已受理**，不表示任何内容已生成；
/// 结果经 3 个事件回传。
#[tauri::command]
pub fn llm_generate_stream(
    app: AppHandle,
    webview: tauri::WebviewWindow,
    request_id: String,
    request: Value,
) -> LlmGenerateStreamStart {
    let prepared = {
        let state = app.state::<AppState>();
        prepare_request(&state, &request)
    };
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(failure) => return failure.into_start(request_id),
    };

    let cancelled = Arc::new(AtomicBool::new(false));
    let recorded = Arc::new(AtomicBool::new(false));
    let emitted = Arc::new(AtomicBool::new(false));
    let snapshot = Arc::new(StreamSnapshot::default());
    let task_slot: Arc<Mutex<Option<StreamTask>>> = Arc::new(Mutex::new(None));
    {
        let state = app.state::<AppState>();
        let mut streams = match state.llm_streams.lock() {
            Ok(streams) => streams,
            Err(_) => {
                return LlmGenerateStreamStart {
                    request_id,
                    started: false,
                    error: Some(provider_error_text(
                        "流式任务注册表不可用".to_string(),
                    )),
                }
            }
        };
        // 同 requestId 的历史任务必须先被终止，否则「取消」会指向孤儿探针：
        // 三间门全部置位（旧任务不得再记账/推送），并 abort 掉挂起的网络读取。
        if let Some(previous) = streams.remove(&request_id) {
            previous.cancelled.store(true, Ordering::SeqCst);
            previous.recorded.store(true, Ordering::SeqCst);
            previous.emitted.store(true, Ordering::SeqCst);
            if let Some(task) = previous.task.lock().ok().and_then(|mut slot| slot.take()) {
                task.0.abort();
            }
        }
        streams.insert(
            request_id.clone(),
            LlmStreamHandle {
                cancelled: cancelled.clone(),
                recorded: recorded.clone(),
                emitted: emitted.clone(),
                task: task_slot.clone(),
                snapshot: snapshot.clone(),
                log: prepared.log.clone(),
            },
        );
    }

    let task_id = request_id.clone();
    let join = tauri::async_runtime::spawn(async move {
        run_stream(
            app, webview, task_id, prepared, cancelled, recorded, emitted, snapshot,
        )
        .await;
    });
    // 先注册后启动：句柄回填晚于 `spawn`（任务可能已自行完成并从注册表移除）。
    if let Ok(mut slot) = task_slot.lock() {
        *slot = Some(StreamTask(join));
    }

    LlmGenerateStreamStart {
        request_id,
        started: true,
        error: None,
    }
}

/// `llm:cancel` —— 取消指定流式请求。
///
/// 顺序与基线（`recordCancelled()` → `controller.abort()` → 移除注册表）一致；
/// 额外补一步：**向前端交付终态事件**。基线依靠 `AbortSignal` 让 provider 抛出
/// `AbortError` 后再走 `onError`（`llm-store` 正是靠它 `cleanup()`）；Rust 侧
/// 任务被 `abort()` 硬终止、不会回调，因此事件必须在这里补齐。
#[tauri::command]
pub fn llm_cancel(
    app: AppHandle,
    webview: tauri::WebviewWindow,
    request_id: String,
) -> LlmCancelResult {
    let handle = {
        let state = app.state::<AppState>();
        // 先落入局部变量：`MutexGuard` 作为尾表达式时其临时值会活到 `state`
        // 之外（E0597），故必须在块内以 `let` 语句结束其生命周期。
        let removed = match state.llm_streams.lock() {
            Ok(mut streams) => streams.remove(&request_id),
            Err(_) => None,
        };
        removed
    };
    let Some(handle) = handle else {
        return LlmCancelResult { success: false };
    };
    if !handle.recorded.swap(true, Ordering::SeqCst) {
        let state = app.state::<AppState>();
        record_provider_outcome(
            &state,
            handle.log.as_ref(),
            &ProviderOutcome {
                success: false,
                usage: None,
                error: Some("cancelled".to_string()),
            },
        );
    }
    // 先占位再 abort：任务即使在 `abort()` 生效前完成了回调，也不会重复推送。
    if !handle.emitted.swap(true, Ordering::SeqCst) {
        emit_terminal_error(
            &webview,
            &request_id,
            StreamFailure::Cancelled.into_message(),
            &handle.snapshot,
        );
    }
    handle.cancelled.store(true, Ordering::SeqCst);
    if let Some(task) = handle.task.lock().ok().and_then(|mut slot| slot.take()) {
        task.0.abort();
    }
    LlmCancelResult { success: true }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_snapshot() -> Value {
        json!({
            "id": "m-1",
            "name": "",
            "provider": "deepseek",
            "protocol": "openai",
            "modelName": "deepseek-v4-flash",
            "apiKey": "sk-test",
            "baseUrl": "https://api.deepseek.com",
            "temperature": 0.7,
            "maxTokens": 4096,
            "purposes": ["generation"],
        })
    }

    #[test]
    fn project_session_requires_three_string_fields_test() {
        assert!(parse_project_session(None).is_none());
        assert!(parse_project_session(Some(&json!({}))).is_none());
        assert!(parse_project_session(Some(&json!({
            "projectId": "p",
            "leaseId": "l"
        })))
        .is_none(), "缺 projectPath 必须判为非法会话");
        assert!(parse_project_session(Some(&json!({
            "projectId": 1,
            "leaseId": "l",
            "projectPath": "C:/x"
        })))
        .is_none());
        let session = parse_project_session(Some(&json!({
            "projectId": "p-1",
            "leaseId": "l-1",
            "projectPath": "F:/novel"
        })))
        .expect("三字段齐备时应解析成功");
        assert_eq!(session.project_id, "p-1");
        assert_eq!(session.project_path, "F:/novel");
    }

    #[test]
    fn gemini_protocol_selects_gemini_provider_test() {
        // 协议判定实现单源在 `llm::chat::is_gemini`（此处仅验证该缝已接通）。
        assert!(is_gemini(&json!({"protocol": "gemini"})));
        assert!(!is_gemini(&json!({"protocol": "openai"})));
        assert!(!is_gemini(&json!({})));
    }

    #[test]
    fn prepare_request_derives_options_and_log_context_test() {
        // `prepare_request` 需要真实 HOME 与 HTTP 客户端，故此处只验证与其
        // 路径无关的派生规则（生成参数 / 消息 / 日志文案）。
        let snapshot = model_snapshot();
        let parameter_model = GenerationParameterModel::from_value(&snapshot);
        let resolved = resolve_generation_parameters(
            &parameter_model,
            &GenerationParameterRequest {
                max_tokens: Some(128),
                response_format: None,
                creative_strategy: None,
                reasoning_stage: None,
            },
        )
        .unwrap();
        assert_eq!(resolved.max_tokens, Some(128));
        assert_eq!(resolved.temperature, Some(0.7));

        let messages: Vec<ChatMessage> =
            serde_json::from_value(json!([{"role": "user", "content": "写一段"}])).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "写一段");

        // 空白 purpose 必须回落到 'generation'（对齐基线 `|| 'generation'`）。
        let request = json!({"purpose": "  "});
        let purpose = request
            .get("purpose")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|purpose| !purpose.is_empty())
            .unwrap_or("generation");
        assert_eq!(purpose, "generation");

        // 空 name 必须回落到 modelName（对齐基线 `model.name || model.modelName`）。
        let model_name = snapshot
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| snapshot.get("modelName").and_then(Value::as_str).unwrap());
        assert_eq!(model_name, "deepseek-v4-flash");
    }

    #[test]
    fn null_response_format_is_treated_as_absent_test() {
        // 基线 `request.responseFormat` 为 null 时不会进入载荷。
        let request = json!({"responseFormat": null});
        let filtered = request
            .get("responseFormat")
            .filter(|value| !value.is_null())
            .cloned();
        assert_eq!(filtered, None);
    }

    #[test]
    fn prepare_failure_envelopes_match_baseline_two_tiers_test() {
        let not_found = PrepareFailure::ModelNotFound.into_response();
        assert_eq!(not_found.success, false);
        assert_eq!(not_found.error.as_deref(), Some("未找到模型配置"));
        assert_eq!(not_found.finish_reason, LlmFinishReason::Error);

        let errored = PrepareFailure::Error {
            message: "租约无效".to_string(),
            log: None,
        }
        .into_response();
        assert_eq!(errored.error.as_deref(), Some("Error: 租约无效"));

        // 流式：模型不存在时刻意**不带** error 字段；异常才带前缀。
        let start = PrepareFailure::ModelNotFound.into_start("r-1".to_string());
        assert_eq!(start.started, false);
        assert_eq!(start.error, None);
        assert_eq!(start.request_id, "r-1");

        let start = PrepareFailure::Error {
            message: "租约无效".to_string(),
            log: None,
        }
        .into_start("r-2".to_string());
        assert_eq!(start.error.as_deref(), Some("Error: 租约无效"));
    }

    #[test]
    fn prepare_failure_statistics_context_only_after_model_resolution_test() {
        // 模型不存在（字面量分支）永不产生统计上下文。
        assert!(PrepareFailure::ModelNotFound.log().is_none());
        assert_eq!(PrepareFailure::ModelNotFound.statistics_error(), None);

        // 租约解析失败（快照未定）同样不留统计。
        let lease_failure = PrepareFailure::Error {
            message: "租约无效".to_string(),
            log: None,
        };
        assert!(lease_failure.log().is_none());
        assert_eq!(lease_failure.statistics_error().as_deref(), Some("Error: 租约无效"));

        // 模型快照已解析后的失败带上统计上下文，且文案与信封一致。
        let log = ProviderCallLog {
            model_id: "m-1".to_string(),
            model_name: "deepseek-v4-flash".to_string(),
            purpose: "generation".to_string(),
            session: None,
            started_at_ms: 0,
        };
        let resolved_failure = PrepareFailure::Error {
            message: "温度越界".to_string(),
            log: Some(log),
        };
        assert!(resolved_failure.log().is_some());
        let envelope = match &resolved_failure {
            PrepareFailure::Error { .. } => true,
            PrepareFailure::ModelNotFound => false,
        };
        assert!(envelope);
        assert_eq!(
            resolved_failure.statistics_error().as_deref(),
            Some("Error: 温度越界")
        );
    }

    #[test]
    fn event_payloads_use_camel_case_contract_fields_test() {
        assert_eq!(
            serde_json::to_value(LlmStreamChunkEvent {
                request_id: "r".to_string(),
                chunk: "a".to_string()
            })
            .unwrap(),
            json!({"requestId": "r", "chunk": "a"})
        );
        assert_eq!(
            serde_json::to_value(LlmStreamErrorEvent {
                request_id: "r".to_string(),
                error: "e".to_string()
            })
            .unwrap(),
            json!({"requestId": "r", "error": "e"})
        );
        let done = serde_json::to_value(LlmStreamDoneEvent {
            request_id: "r".to_string(),
            full_text: "t".to_string(),
            usage: None,
            finish_reason: LlmFinishReason::Stop,
        })
        .unwrap();
        assert_eq!(done, json!({"requestId": "r", "fullText": "t", "finishReason": "stop"}));

        let with_usage = serde_json::to_value(LlmStreamDoneEvent {
            request_id: "r".to_string(),
            full_text: "t".to_string(),
            usage: Some(TokenUsage {
                prompt_tokens: Some(1),
                completion_tokens: None,
                total_tokens: None,
            }),
            finish_reason: LlmFinishReason::Length,
        })
        .unwrap();
        assert_eq!(with_usage["usage"], json!({"promptTokens": 1, "completionTokens": null, "totalTokens": null}));
        assert_eq!(with_usage["finishReason"], json!("length"));
    }

    #[test]
    fn event_channel_names_match_contract_test() {
        assert_eq!(LLM_STREAM_CHUNK_EVENT, "llm:stream-chunk");
        assert_eq!(LLM_STREAM_DONE_EVENT, "llm:stream-done");
        assert_eq!(LLM_STREAM_ERROR_EVENT, "llm:stream-error");
    }

    #[test]
    fn stream_snapshot_derives_visible_candidate_like_baseline_test() {
        let snapshot = StreamSnapshot::default();
        // 尚无任何交付内容：取消必须走 `llm:stream-error` 分支。
        assert_eq!(snapshot.visible_candidate(), None);
        assert_eq!(snapshot.usage(), None);

        // 只有未闭合的推理块：strip 后为空，仍判为无可见候选。
        snapshot.push("<think>\n推理中");
        assert_eq!(snapshot.visible_candidate(), None);

        // 累积后的可见正文（含 `<think>` 包装，与基线 provider `fullText` 同语义）。
        snapshot.push("\n</think>\n\n正文");
        assert_eq!(snapshot.visible_candidate().as_deref(), Some("正文"));

        snapshot.set_usage(&Some(TokenUsage {
            prompt_tokens: Some(3),
            completion_tokens: Some(5),
            total_tokens: Some(8),
        }));
        assert_eq!(
            snapshot.usage(),
            Some(TokenUsage {
                prompt_tokens: Some(3),
                completion_tokens: Some(5),
                total_tokens: Some(8),
            })
        );
    }

    #[test]
    fn cancel_delivers_cancelled_message_without_error_prefix_test() {
        // 取消事件文案与统计文案刻意不同：基线统计记 `cancelled`，事件文案为
        // `已取消生成`（且**不带** `Error: ` 前缀 —— 它不经 `String(error)`）。
        assert_eq!(StreamFailure::Cancelled.into_message(), "已取消生成");
        assert!(!StreamFailure::Cancelled.into_message().starts_with("Error: "));
    }
}
