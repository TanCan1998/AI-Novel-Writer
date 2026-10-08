//! 批次 D2：LLM 生成执行域（迁移自 `electron/llm/` + `electron/services/`）。
//!
//! 模块切分对齐基线文件，保持「一个基线文件 ↔ 一个 Rust 模块」的可核对性：
//!
//! | Rust 模块 | 基线来源 | 职责 |
//! |---|---|---|
//! | [`presets`] | `src/shared/provider-presets.ts` | 内置服务商目录 + **已验证能力 / 推理映射** 解析 |
//! | [`reasoning`] | `src/shared/reasoning-policy.ts` | 单一推理策略缝：产品意图 → provider 指令 |
//! | [`params`] | `electron/llm/generation-parameter-policy.ts` | 模型档案 → 有效请求参数（含 Kimi 特例） |
//! | [`lease`] | `electron/services/model-execution-lease.ts` | 不可变模型快照 + 能力证据指纹 + 租约注册表 |
//! | [`chat`] | `electron/llm/provider.interface.ts` + `llm-controller.ts` | 生成链共享类型 / `<think>` 剥离 / HTTP 客户端装配 |
//! | [`openai`] | `electron/llm/openai-provider.ts` + `openai-compatible-endpoint.ts` | OpenAI 兼容协议 + 手写 SSE 行解析 |
//! | [`gemini`] | `electron/llm/gemini-provider.ts` | Gemini 协议（宽松 SSE，不剥离推理块） |
//! | [`discovery`] | `electron/services/model-discovery-service.ts` | 供应商模型列表发现（凭据回显守卫 + 严格载荷解析） |
//! | [`embedding`] | `electron/embedding.ts`（调用面） | 远程 Embedding 调用与响应校验 |
//!
//! 领域不变量（`docs/product-domain.md`）：
//! - **作者事实优先**：能力证据只认内置预设（`verified-provider-preset`），用户填写的
//!   能力标记只是操作策略，永远不能升级为协议证据；
//! - **租约不可伪造**：模型快照在主进程内冻结，`temperature` 仅由模型档案决定，
//!   调用方只能选择工作流特征与输出预算，不能覆盖采样温度。

pub mod chat;
pub mod discovery;
pub mod embedding;
pub mod gemini;
pub mod lease;
pub mod openai;
pub mod params;
pub mod presets;
pub mod reasoning;
