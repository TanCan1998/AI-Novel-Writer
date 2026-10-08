# Tauri 迁移进度快照（2026-10-08）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> **命名规则**：文件按日期命名 `YYYY-MM-DD-tauri-migration-status.md`，
> **一个工作日一个新文件**；当日新增内容只写入当日文件，跨日不回填旧文件
> （规则见 [`docs/agents/pi-development.md`](../agents/pi-development.md) §9）。
> 上一份快照（2026-10-07 冻结）：
> [`2026-10-07-tauri-migration-status.md`](./2026-10-07-tauri-migration-status.md)；
> 批次 A–C `project_core` / `characters` 的历史细节见
> [`2026-10-06-tauri-migration-status.md`](./2026-10-06-tauri-migration-status.md)；
> channel 级盘点见 [`docs/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-08 · 第二十四次）
| 项 | 值 |
|---|---|
| 仓库 | **`TanCan1998/Lorekeeper`**（原名 `AI-Novel-Writer`；仍为 `EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork） |
| 分支 | `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm 包 `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 当前阶段 | **批次 C 数据库层子域全部完成 ✅** + **批次 D1 ✅** + **批次 D2-a ✅** + **批次 D2-b ✅** + **批次 D2-c ✅（`llm:*` 收口）**：`project_core` / `characters` / `blueprints` / `drafts`（12/16）/ `revisions` / `reviews` / `post-process` / `llm 日志与摘要` / `project 清理` / **D1 模型管理** / **D2-a 租约** / **D2-b HTTP 生成链** / **D2-c 模型发现 + 连通性探测**。**`llm:` 前缀下 14 个 invoke 频道已全部迁移**（不再有 `llm=` 未迁项）。依赖仍为 `reqwest 0.13`（`default-features = false` + `native-tls` + `socks`）；**`futures-util` / `tokio` 未引入**（D2-c 的 15s 总超时改由 `reqwest::Client::timeout` 承担） |
| 已注册命令 | **104**（骨架 1 + A 11 + B 22 + C 子域 56 + D1 7 + D2-a 2 + D2-b 3 + D2-c 2） |
| GUI 冒烟 | ✅ **已做**（2026-10-07 起 **六轮**，末轮 2026-10-08 `pnpm tauri dev`）：窗口标题 `Lorekeeper`、vite@5190、cargo 390/390、`lorekeeper.exe` **内存 42.6 MB**（首轮）/ **30.1 MB**（D1 轮）/ D2-a 轮 vite `482 ms` / D2-b 轮 vite `468 ms` + cargo `24.99s` / **D2-c 轮 `Running target\debug\lorekeeper.exe` + 内存 44.1 MB**，均无 panic、渲染层 `ipc-client` 已联通 |
| 自动化回归 | `cargo test --lib` **306/306**（279 → +27；含 3 个**磁盘级**端到端：真实 `.vela/lorekeeper.db` + WAL + 外键 + 跨重开持久化）；`pnpm run check:channels` 校验契约↔命令映射（104 命令覆盖 103 invoke 频道，未迁移 90 → **88** 频道，`llm=` **已清零**，orphan 空）；`vitest` 相关用例 7/7 |
| 双栈隔离 | L0 安装标识 / L1 `~/.lorekeeper` / L2 `<root>/.vela/lorekeeper.db` 均独立；L3（`.vela` 改名）押后。D1 起 `~/.lorekeeper/{config.json,models.json,recent-projects.json}` 为**真实持久化**（此前 config 仅内存态） |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc @ `D:\Environment\rust\`（脚本内显式设 `RUSTUP_HOME`/`CARGO_HOME`） |
| 验证状态 | ✅ `cargo check --all-targets` **0 告警** · ✅ `cargo test`（全目标）**306/306** · ✅ `pnpm typecheck` exit 0 · ✅ `pnpm run lint` exit 0 · ✅ `check:channels` orphan 空 · ✅ D2-c GUI 冒烟（启动路径无 `Command ... not found` / 无 panic） |

---

## 本次更新（第二十四次：批次 D2-c — `llm:*` 收口：模型发现 + 连通性探测 2 频道）

### 0. 范围与依赖

`llm:*` 前缀下最后 2 个未迁频道：

| 频道 | 命令 | 基线来源 |
|---|---|---|
| `llm:discover-models` | `llm_discover_models` | `electron/services/model-discovery-service.ts`（204 行） |
| `llm:test-connection` | `llm_test_connection` | `llm-controller.ts` handler + `electron/embedding.ts`（调用面） |

**零新增依赖**：D2-b 引入的 `reqwest` 直接承担全部网络职责；基线的
`AbortController` 15s 总超时改由 `reqwest::Client::timeout` 表达（同样是
「含 body 读取」的总时长语义），因此**不需要** `tokio` 直接依赖，也无需
`futures-util`。测试亦不依赖异步运行时（见 §3 的纯函数切分）。

### 1. `llm/discovery.rs`（新，204 行基线的逐支复刻）

| 项 | 对齐基线 |
|---|---|
| `resolve_openai_models_url()` | 剥 `/chat/completions` → 剥 `/chat` → 空路径回落 `/v1` → 拼 `/models`；清 query/fragment/凭据 |
| `resolve_gemini_models_url()` | `/v1beta/models` 原样 / `/v1beta` 补 `/models` / 其余补 `/v1beta/models` |
| `url_contains_credential()` | 原文 → `encodeURIComponent` 形态 → `decodeURIComponent` 后形态三档命中；空凭据恒假；解码失败按未命中 |
| `safe_provider_text()` | ≤512 **字节**（`Buffer.byteLength` = UTF-8 字节）、无控制字符（`<=0x1f` 与 `0x7f..=0x9f`）、`trim` 后非空、不含凭据 |
| `parse_openai_models()` / `parse_gemini_models()` | 条目上限 500；**任一条目非法 → 整体 `invalid_response`**（不是跳过该条）；`id` 去重后保序 |
| `classify_http_failure()` | 401/403 → `auth`；408/425/429/5xx → `network`；其余 → `unsupported` |
| `interpret_discovery_response()` | 空列表 → `empty`；载荷坏 → `invalid_response`；body 读取失败分「超时 → network / 否则 invalid_response」 |

**两处刻意的环境差异**（均已注释）：

1. **重定向语义**：基线 `redirect: 'error'` 会让 fetch **reject**（→ `network`）；
   Rust 侧用 `redirect::Policy::none()`，若直接落进 `classify_http_failure(302)`
   会得到 `unsupported`（**语义错误**）—— 故在 `fetch_models()` 中**先**判
   `status.is_redirection()` 并强制归为 `network`。这条差异是最容易写错的一处。
2. **URL 规范化**：`URL.toString()` vs `url::Url::to_string()` 在极端形态上可能
   有百分号编码差异；端点语义一致，已在测试中固定关键形态。

### 2. `llm/embedding.rs`（新，**刻意收窄**的迁移范围）

`electron/embedding.ts` 共 360 行，本批次**只迁移调用面**：

| 基线片段 | 迁移 | 理由 |
|---|---|---|
| `embedOpenAI` / `embedGemini` / `generateEmbeddings` | ✅ | `llm:test-connection` 的 embedding 分支 |
| `validate*Embeddings` / `validateEmbeddingVectors` | ✅ | 响应合法性就是「连接可用」判据 |
| `ollamaOpenAIEmbeddingBaseUrl` / `buildOpenAIEmbeddingUrl` | ✅ | URL 构造属调用语义 |
| `releaseSmokeEmbeddings` | ❌ | Electron 安装包冒烟专用（需环境变量 + 唯一 argv 令牌） |
| `chunkText` | ❌ | 文本分块属 RAG 侧（批次 F），本批次无调用面 |
| `normalizeEmbeddingOptions` 的 `batchSize` 归一 | ❌ | 仅在显式传入 `configuredBatchSize` 时生效，当前无调用面 |

**`String(error)` 的三档前缀逐字复刻**（含一个此前未注意到的细节）：

- `new Error(msg)` → `Error: msg`；
- `EmbeddingResponseValidationError` → **`EmbeddingResponseValidationError: `**（不是 `Error: `！
  该类在构造器里把 `this.name` 改成了类名，`String(error)` 走 `name: message`）；
- `fetch` 失败的 `TypeError` → Rust 侧无法逐字复刻异常类名，退化为 `Error: …`（已注明）。

同时复刻了两个**容易被简化掉**的细节：

1. Gemini 侧基地址只去**一个**尾斜杠（`replace(/\/$/, '')`），OpenAI 侧去**全部**
   （`replace(/\/+$/, '')`）—— 两者不可混用，已各自单测；
2. OpenAI 响应校验在「条目缺 `embedding`」时 `continue` 后**仍会汇总**
   「`index` 覆盖不完整」错误，最终文案是 `A；B` 拼接（测试逐条固定该拼接结果）。

### 3. `commands/llm_management.rs`（新，2 频道）

| 频道 | 行为 |
|---|---|
| `llm:test-connection` | 先同步决议探测参数（失败即返回 `Error: ` 文案）→ 构建客户端 → `purposes` 含 `embedding` 走 Embedding 端点，否则发一条 `Say "hello" and nothing else.` 探测 |
| `llm:discover-models` | 构建 `Policy::none()` + 15s 超时的客户端 → 交 `llm::discovery::discover_models` |

**探测与生成链共用同一策略缝**：`maxTokens = 1024`（`CONNECTION_TEST_MAX_TOKENS`，
推理模型需要足够预算才不会被误判为截断失败）、`reasoningStage = 'general'`、
`conversationId` **恒为 `None`**（探测不是创作会话，不得共享网关粘性）——
三条都有测试锁定。

**测试可离线运行的关键切分**：把「请求前同步判定」抽成纯函数，避免为单测引入
异步运行时：

- `discovery::resolve_discovery_request()`（端点解析 + 凭据回显守卫）；
- `discovery::interpret_discovery_response()`（传输结果 → 结果档位）；
- `llm_management::resolve_connection_options()`（参数决议，含 Kimi 温度校验）；
- `embedding::embedding_batches()`（批量切分）。

### 4. 小重构（消除第二份定义）

- `is_gemini()` 上移到 `llm/chat.rs` 作为单源；`commands/llm_generation.rs` 的私有
  副本删除并改为引用（其测试保留，注释标注实现单源位置）；
- `llm/chat.rs` 新增 `build_client_with_timeout()`，`build_client()` 改为它的
  `None` 特例（行为不变，仍是「无总超时」）。

### 5. 接线

- `llm/mod.rs`：新增 `pub mod discovery; pub mod embedding;`；
- `commands/mod.rs`：新增 `mod llm_management;` + glob 再导出；
- `lib.rs`：注册 2 命令（102 → **104**）；
- `src/shared/migrated-channels.ts`：由 `pnpm run check:channels:emit` 重新生成（103 频道）；
- `src/services/ipc-client.ts`：登记 `llm:test-connection` → `['model','creativeStrategy']`、
  `llm:discover-models` → `['request']`（渲染层实际恒传 2 个实参，`creativeStrategy` 有 `?? 'auto'` 兜底）；
- `test/channel-migration-coverage.test.ts`：未迁移样本改指 `kb:search` / `update:get-state`，
  并断言 2 个新频道已迁移。

### 6. 验证与测试

| 检查 | 结果 |
|---|---|
| `cargo check --all-targets` | **0 告警** |
| `cargo test --lib` | **306/306**（279 → +27） |
| `pnpm typecheck` | exit 0 |
| `pnpm run lint` | exit 0（`--max-warnings 0`） |
| `pnpm run check:channels` | 104 命令、未迁移 **88**（90 → -2，`llm=` 清零）、orphan 空 |
| `vitest`（频道覆盖 + 发现边界） | 7/7 |
| GUI 冒烟（第六轮） | `Running BeforeDevCommand (pnpm dev)` → `Running DevCommand (cargo run …)` → `Running target\debug\lorekeeper.exe`，内存 44.1 MB，无 panic / 无编译告警；进程树已清理 |

新增单测覆盖：端点解析全分支（含 `/chat/completions` / `/chat` / 空路径 / query+fragment+凭据清除 / 非法 URL）、
凭据回显三档形态、`safe_provider_text` 六类拒绝、OpenAI/Gemini 载荷严格解析（含条目上限与去重）、
HTTP 分类五档、传输解释六分支、结果序列化的键存在性（成功不含 `errorCode`、失败不含 `models`）、
Ollama 原生 `/api` 九类不命中形态、Embedding URL 推断七例、Gemini 单斜杠剥除、
向量维度/有限性校验、OpenAI `index` 覆盖性与拼接文案、Gemini 顺序语义、
错误文案三档前缀、批量切分、探测参数意图、探测温度（档案唯一来源 / Kimi 固定采样省略）、
Kimi 越界文案、探测不带会话粘性、探测复用已验证推理指令。

---

## 上一次更新（第二十三次：批次 D2-b — LLM 生成 / 流式 / 取消 3 频道 + 3 事件）

### 0. 依赖决策（推翻了「`default-tls` = native-tls」的假设）

抽 `reqwest 0.13.5` 权威 feature 表后确认：`default = ["default-tls", "charset", "http2", "system-proxy"]`，
而 **`default-tls = ["rustls"]`** —— 0.13 起默认 TLS 后端已是 rustls，不再是 native-tls。
rustls 路径必拉 `aws-lc-rs`，本机无 VS/CMake 构建链 → 必然编译失败。

最终声明：

```toml
reqwest = { version = "0.13", default-features = false, features = ["json", "stream", "native-tls", "socks"] }
```

- `Cargo.lock` **+18 个包**（TLS 家族），Windows 走 `schannel`（纯 FFI，**无** `aws-lc-rs` / `ring`）；
- 首次引入 `reqwest` 触发真实 crates 下载并编译成功；
- 刻意**不**启用：`charset`（基线 Node `res.text()` 恒按 UTF-8 解码）、`http2`
  （Node fetch/undici 为 HTTP/1.1）、`system-proxy`（代理在代码内显式装配）；
- **未引入 `futures-util`**（原计划项）：取消改用 `tauri::async_runtime::JoinHandle::abort()`，
  零新依赖达成基线 `AbortController` 的效果（见 §4）。

### 1. `src-tauri/src/llm/chat.rs`（新，共享层）

| 项 | 对齐基线 |
|---|---|
| `strip_thinking()` | `OpenAIProvider.stripThinking()`；三条规则的**顺序**与**首匹配/全局**语义逐字同构（规则 1 全局 `replace_all`、规则 2/3 只替换首个），大小写不敏感、跨多行 |
| `proxy_from_config()` | `applyProxyConfig()` 的判定顺序：`enabled === true` **且** host 非空；`type === 'socks5'` 走 socks5、其余 http；配置读取失败静默 `None` |
| `build_client()` | 基线 `fetch` **无总超时**（长文生成可跑数分钟），故不设 timeout；仅装配显式代理 |
| `provider_error_text()` | 复刻 `String(error)` 的 `"Error: "` 前缀，**复用** `commands::db::mutating_error`，不造第二份格式定义 |
| 共享类型 | `LlmFinishReason`（6 档 snake_case）、`TokenUsage`（三项 `Option`）、`LlmResponse`（`camelCase` + `skip_serializing_if`）、`LlmGenerateOptions`、`ChatMessage`、`StreamFailure`（`Cancelled` 与 `Message` 分开） |

### 2. `src-tauri/src/llm/openai.rs`（新）

- `build_request_body()`：`temperature` 为 `None` 时**整键省略**（不得回退 `model.temperature`）；
  NovelAI 走窄兼容载荷（不吃 `reasoning_effort` / `response_format` / `stream_options`）；
  DeepSeek 专有 `thinking: { type }` + `enabled` 时才带 `reasoning_effort`；
  `stream && !isNovelAI` 才带 `stream_options.include_usage`；`max_tokens` 在两侧都缺时**不出现** `null` 占位。
- `build_request_headers()`：opencode Go 判据（`https` + `opencode.ai` + 路径 `/zen/go` 前缀）成立时补
  `x-opencode-session`（缺失会话退化为 **单请求 UUID**，绝不与创作运行共享粘性）与 `User-Agent`。
- `OpenAiSseDecoder`：纯状态机（无网络依赖，可单测）—— CRLF 切断与尾缓冲、`data:` 多行拼接、
  注释行忽略、`[DONE]` 终止、`reasoning_content` → `<think>\n…` 包裹、`content` 到达时补 `\n</think>\n\n`、
  仅三字段皆为数才收 `usage`、7 类 typed fatal error（损坏 JSON / 非对象 / `error` 载荷 / choices / choice / finish_reason / delta / content 类型）。
- `generate()`：常返回信封（不 `Err`）；非 2xx → 文案 `API 调用失败 ({status}): {text}`；
  非 `stop` 终态 → `success: false` + `API 返回的文本未正常完成`。
- `generate_stream()`：三态回调；`!sawDone` → `响应流在完成标记前结束，生成结果不完整`；
  推理块未闭合时补 `\n</think>\n\n`；失败一律走 `fail()`，把**已交付的可见候选**（空则不给）交回调用方。

### 3. `src-tauri/src/llm/gemini.rs`（新）

`system` 消息提升为 `systemInstruction`、`assistant` → `model` 重命名、
URL 用字符串拼接（`{base}/v1beta/models/{name}:generateContent` / `…:streamGenerateContent?alt=sse`，
与基线模板字面量一致，不做 URL 规范化）；SSE 解码器**宽松解析**（坏行跳过而非 fatal，CRLF 尾缓冲、
`null` finish_reason → `unknown`）；**Gemini 不剥离推理块**（prompt 不走 `<think>`，与 OpenAI 分道）。

### 4. `commands/llm_generation.rs`（新，3 频道 + 3 事件）

| 频道 | 行为 |
|---|---|
| `llm:generate` | **总是**返回信封（不 reject）；两档失败语义**不可统一** |
| `llm:generate-stream` | 立即 `{requestId, started}`，后台任务把三回调翻译为事件 |
| `llm:cancel` | 记录 `cancelled` 统计 → 交付终态事件 → `abort()` 任务 |

**两档失败语义**（基线字面量分支，刻意不统一）：

- **模型不存在**：`llm:generate` 文案 `未找到模型配置` / `llm:generate-stream` **不带** `error` 字段，均**无** `Error: ` 前缀；
- **异常路径**（租约解析失败 / Kimi 温度越界 / messages 非法 / 客户端构建失败）：走 `provider_error_text`（带前缀）。

**调用统计**（`llm_calls` 表）三重守卫，逐条对齐 `recordProviderOutcome`：
无会话上下文 → 丢弃；会话与当前活跃项目不匹配 → 丢弃；落库失败 → **吞掉**
（诊断信息绝不改变生成结果）。且统计上下文只在**模型快照解析之后**才产生 ——
对齐基线 `catch` 里的 `if (model) recordProviderOutcome(...)`（租约解析失败时 `model` 仍为 null，不留痕）。

**取消的语义补齐（本轮真实缺口）**：基线靠 `AbortSignal` 让 provider 抛 `AbortError` 后自行 `fail()`，
渲染层 `llm-store` 正是靠 `llm:stream-done` / `llm:stream-error` 才执行 `cleanup()`；
而 Rust 任务被 `abort()` **硬终止、不走任何回调** —— 若只置探针，UI 会永久停在 `running`。
故本轮引入：

- `StreamSnapshot`（`Mutex<String>` 已交付全文 + `Mutex<Option<TokenUsage>>`）：
  取消时用它派生 `stripThinking(fullText)` 与 `usage`，与基线 `fail()` 的入参同源；
- `emitted` 终态事件一次性闸门：取消方先占位再 `abort()`，任务侧回调不得重复推送；
- `task: Arc<Mutex<Option<StreamTask>>>`：`llm_cancel` 借此 `abort()` 掉挂起中的
  `chunk().await`（基线由 `AbortController` 关闭连接达成同等效果），避免连接与任务永久残留。

### 5. 与基线的**刻意差异**（均已在代码注释注明）

| # | 基线 | Tauri 侧 | 影响 |
|---|---|---|---|
| 1 | `llm:generate-stream` 的参数决议在 `try` 外 → handler **reject** | 返回 `{started:false, error:"Error: …"}` 信封 | 前端两条路径都会 `cleanup()` + `onError` + throw，**行为等价**且更贴契约 |
| 2 | 同 `requestId` 重发直接覆盖 `activeStreams`（旧流不中止） | 三闸门置位 + `abort()` 旧任务 | 修掉旧流继续推送事件/记账的隐患 |
| 3 | `messages` 无运行时校验（TS 类型） | serde 校验，非法结构记失败统计并返回信封 | 更严格 |
| 4 | 取消 → `AbortError` → `onError('已取消生成')` | `abort()` + 命令层主动补发同名终态事件 | 等价，且保证 `cleanup()` 执行 |
| 5 | `usage` 中间态为 `undefined`（structured clone 保留） | 序列化为 `null` | `?? 0` 类消费无差别 |

### 6. 接线

- `commands/mod.rs`：`mod llm_generation` + glob 再导出；
- `lib.rs`：注册 3 命令（99 → **102**），并**移除** D2 期遗留的 `#[allow(dead_code)] mod llm;`
  （D2-b 接通后推理映射/生成参数的调用面已真实使用，注解按原计划撤除，仍 0 告警）；
- `src/shared/migrated-channels.ts`：登记 3 频道；
- `src/services/ipc-client.ts`：`CHANNEL_ARG_NAMES` 补登 `llm:generate` / `llm:generate-stream` / `llm:cancel`；
- `test/channel-migration-coverage.test.ts`：前置拦截用例改指 `llm:discover-models`，并断言 3 个新频道已迁移。

事件无需前端登记：`ipc.on(channel, …)` 直通 `tauriListen`，与 Electron 版语义一致。

### 7. 验证与测试

| 检查 | 结果 |
|---|---|
| `cargo check --all-targets` | **0 告警** |
| `cargo test --lib` | **279/279**（243 → +36） |
| `pnpm typecheck` | exit 0 |
| `pnpm run lint` | exit 0（`--max-warnings 0`） |
| `pnpm run check:channels` | 102 命令、未迁移 **90**（93 → -3，`llm=` 剩 2）、orphan 空 |
| GUI 冒烟（第五轮） | vite `468 ms` @5190 → cargo `24.99s` → `Running target\debug\lorekeeper.exe`，无 panic，渲染层 `ipc-client` 已联通；进程树已清理 |

新增单测覆盖：`<think>` 三规则的顺序/首匹配语义、代理四重守卫、URL 分支、opencode Go 判据、
`normalize_finish_reason` 词表映射、`temperature` 整键省略、SSE 解码器（CRLF / 多行 data / 坏 JSON /
类型错误 / `[DONE]` / usage 收集 / 多字节切断）、Gemini 载荷形状与宽松解码、两档失败信封、
统计上下文产生时机、事件载荷 `camelCase` 契约字段、`StreamSnapshot` 派生可见候选、取消文案无 `Error: ` 前缀。

---

### 8. 收尾（工作区卫生 + 快照命名规则）

- **提交** `1bcae45`（`chore: 忽略 pi 会话数据目录（.pi/）`）：`.pi/` 新增忽略
  （内含 `agent/auth.json` 凭据、`tmp/reqwest.index` 临时索引近 1 MB）；目录**保留在磁盘**，
  仅忽略 —— 删除会导致登录态与缓存丢失，该点已在 `.gitignore` 注释中写明。
- **清理**：删除 5 个上一轮会话调试产物（`tauri-app/{chk,emit}.txt`、
  `src-tauri/{check_err,test2,test_err}.txt`）与 2 个早期冒烟轮次遗留日志
  （`tauri-app/dev.{out,err}.log`）；清理后 `git status` 干净。
- **快照命名规则确立**（**本文件即首个按新规则产生的快照**）：交接快照按日期命名
  `docs/handoffs/YYYY-MM-DD-tauri-migration-status.md`，**一个工作日一个新文件**；
  当日章节只写当日文件，跨日**不得回填**旧文件（旧文件视为该日期的冻结快照）。
  规则出处：[`docs/agents/pi-development.md`](../agents/pi-development.md) §9。
  随之完成一次**结构迁移**：本文件承接 2026-10-08 产生的全部内容（快照表 + 第二十三次
  章节 + 滚动的收口/下一步/遗留项），`2026-10-07-*.md` 回退为 `7f0ecc1` 的冻结状态
  并加上前向指针。

## 批次 C 收口状态（2026-10-07）

**已完成子域**（命令累计 **90**，其中 C 子域 56）：

| 子域 | 频道 | 最新 commit |
|---|---|---|
| project_core | 4 | `ff7fbd8` `a8d742a` |
| characters / roster | 3 | `69fc50c` |
| blueprints | 11 | `50c0fe8` `1332216` `84a3100` |
| drafts | 12/16 | `6351b2d` |
| revisions | 9 | `5e79ea7` |
| reviews | 5 | `fa07875` |
| post-process | 6 | `39ee0c2` |
| llm 日志 / 摘要 | 5 | `e0e1004` |
| project 清理 | 1/2 | 本轮 |
| **合计** | **56** | |

**剩余 `db:*` 频道全部归属后续批次**（不是 C 的欠账）：

- **批次 E**：continuity（4）、finalization-link（1）、drafts 余 4
  （`authority-sequence` / `export-snapshot` / `export-authority-current` / `import-finalized-batch`）；
- **批次 F**：一致性豁免（3）、叙事线程（6）、派生树（3）、恢复候选（3）；
- **批次 G**：import-run（18）、`db:import-global-facts-commit`。

---

## 建议的下一步

1. **⚙️ 先补 G5 压测**（本轮之后最短的债务）：`llm:stream-chunk` 已接通但「高频 chunk
   跨 webview 桥的吞吐/延迟」仍无实测数据。建议写一个不带网络的本地回放（例如直接循环
   `emit` N 次）测出 chunk/s 上限，再决定是否需要批量合并（inventory §G5 已预留该选项）。
2. **双栈同库行为对照**：同目录下 Electron（`vela.db`）与 Tauri（`lorekeeper.db`）各写各库，
   确认互不影响；顺带对照 `~/.vela/config.json` 与 `~/.lorekeeper/config.json` 的读写形态差异。
3. **`vitest` 全量超时定位**（遗留项 7）。
4. **批次 E**（定稿不可逆 + 删除生命周期，ADR 0003/0011 等量测试）：`finalization`（2，需补契约 G1）、
   `chapter-lifecycle`（4）、`continuity`（4）、`recovery-candidate`（4）、
   `drafts` 余 4（`authority-sequence` / `export-snapshot` / `export-authority-current` /
   `import-finalized-batch`）、`finalization-link`（1）。
5. **或批次 F**（16 频道，零新依赖：一致性豁免 3 / 叙事线程 6 / 派生树 3 / 恢复候选 3 +
   `kb:*` 需先定 LanceDB 取舍）。

---

## 遗留项（沿用 2026-10-06 快照）

- ✅ **批次 C GUI 实机验证（自动化部分已完成）**：三轮 `pnpm tauri dev` 冒烟通过；
  核心读写链路已由 `disk_e2e.rs` 用真实 `.vela/lorekeeper.db` 断言覆盖。
  **仍未人工验证**：界面交互本身（按钮触发、表单回显、错误提示的 UI 形式）。
- ✅ **遗留项 12 已修复**（`24c008f`）：未迁移频道给出统一友好提示 + 生成物一致性测试。
- ✅ **config 持久化缺口已补齐**（第二十一次，批次 D1）：`config:get/set` 由内存态改为
  `~/.lorekeeper/config.json` 真实持久化；`models.json` / `recent-projects.json` 同步走原子写。
- ✅ **批次 D2 依赖已批准并落地**（第二十三次，批次 D2-b）：`reqwest 0.13`
  （`default-features = false` + `native-tls` + `socks`）；**`futures-util` 未引入**（改用 `JoinHandle::abort()`）。
- ⚠️ **G5（流式事件经 webview 桥的性能实测）仍未做**：D2-b 已把 3 个事件接通，
  但「高频 chunk 下跨 IPC 桥的吞吐/延迟」尚无实测数据；D2-c 也未触及该缝
  （探测与发现均为低频调用）—— 建议在批次 E 前补一次压测。
- ⚠️ **D2-c 刻意未移植的基线片段**（均无调用面，非欠账）：`embedding.ts` 的
  `releaseSmokeEmbeddings`（Electron 安装包冒烟专用）、`chunkText`（属 RAG 侧）与
  `normalizeEmbeddingOptions` 的 `batchSize` 归一；若批次 F 接入知识库，
  `chunkText` 与 `batchSize` 归一需随该批次一并落地。
- 未验证：双栈同库行为对照、`vitest` 全量超时定位、`cargo fmt --check` 未纳入验收
  （`src-tauri/` 全域存在 rustfmt 差异，需单独提交）。
- 押后：L3（`.vela` → `.lorekeeper`）、可见品牌（`brand.ts` / i18n 标题）。
- 批次 G 需要的 `BlueprintRepository.getCommittedRangeOperation`（无 IPC 频道，被
  `import-run-repository.ts` 使用）尚未移植，随批次 G 一并落地。
- 无 IPC 频道的基线辅助方法未移植：`post_process_repository.get_failed_step_labels`、
  `DraftRepository.clearAll`（后者功能已被 `project_clear` 覆盖）。
- 已知取舍（D1）：`models.json` 内容非数组对象（如合法 JSON 对象）时，基线会把它当数组
  误用并写出怪异结果，Rust 侧改为**拒绝覆盖**（更严格，语义差别只在文件被外部破坏时出现）；
  `delete-model` 遇损坏文件时基线报 JS `SyntaxError` 原文，Rust 侧统一为
  `Error: 模型配置损坏，已拒绝覆盖`。
