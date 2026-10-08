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

## 快照（最后更新：2026-10-08 · 第二十三次）
| 项 | 值 |
|---|---|
| 仓库 | **`TanCan1998/Lorekeeper`**（原名 `AI-Novel-Writer`；仍为 `EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork） |
| 分支 | `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm 包 `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 当前阶段 | **批次 C 数据库层子域全部完成 ✅** + **批次 D1（`llm:*` 模型管理 7 频道）完成 ✅** + **批次 D2-a（生成参数策略 + 模型执行租约 2 频道）完成 ✅** + **批次 D2-b（生成 / 流式 / 取消 3 频道 + 3 事件）完成 ✅**：`project_core` / `characters` / `blueprints` / `drafts`（12/16）/ `revisions` / `reviews` / `post-process` / `llm 日志与摘要` / `project 清理` / **D1 模型管理** / **D2-a 租约** / **D2-b HTTP 生成链**；`llm:*` 仅剩 2 频道（`discover-models` / `test-connection`）归 **D2-c**。依赖已落地：`reqwest 0.13`（`default-features = false` + `native-tls` + `socks`）；**`futures-util` 未引入**（取消改用 `JoinHandle::abort()`） |
| 已注册命令 | **102**（骨架 1 + A 11 + B 22 + C 子域 56 + D1 7 + D2-a 2 + D2-b 3） |
| GUI 冒烟 | ✅ **已做**（2026-10-07 起 **五轮**，末轮 2026-10-08 `pnpm tauri dev`）：窗口标题 `Lorekeeper`、vite@5190、cargo 353/353、`lorekeeper.exe` **内存 42.6 MB**（首轮）/ **30.1 MB**（D1 轮）/ D2-a 轮 vite `482 ms` / **D2-b 轮 vite `468 ms` + cargo `24.99s`**，均无 panic、渲染层 `ipc-client` 已联通 |
| 自动化回归 | `cargo test --lib` **279/279**（243 → +36；含 3 个**磁盘级**端到端：真实 `.vela/lorekeeper.db` + WAL + 外键 + 跨重开持久化）；`pnpm run check:channels` 校验契约↔命令映射（未迁移 93 → **90** 频道，`llm=` 剩 2）；`vitest` 频道覆盖 6/6 |
| 双栈隔离 | L0 安装标识 / L1 `~/.lorekeeper` / L2 `<root>/.vela/lorekeeper.db` 均独立；L3（`.vela` 改名）押后。D1 起 `~/.lorekeeper/{config.json,models.json,recent-projects.json}` 为**真实持久化**（此前 config 仅内存态） |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc @ `D:\Environment\rust\`（脚本内显式设 `RUSTUP_HOME`/`CARGO_HOME`） |
| 验证状态 | ✅ `cargo check --all-targets` **0 告警** · ✅ `cargo test`（全目标）**279/279** · ✅ `pnpm typecheck` exit 0 · ✅ `pnpm run lint` exit 0 · ✅ `check:channels` orphan 空 · ✅ D2-b GUI 冒烟（启动路径无 `Command ... not found` / 无 panic） |

---

## 本次更新（第二十三次：批次 D2-b — LLM 生成 / 流式 / 取消 3 频道 + 3 事件）

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

1. **批次 D2-c（`llm:*` 收口，2 频道）**：`discover-models` / `test-connection`
   （后者含 embedding 分支，需评估是否同步移植 `electron/embedding.ts` 的 416 行），
   并补登 `ipc-client.ts` 参数名（`llm:discover-models` → `['request']`、
   `llm:test-connection` → `['model','creativeStrategy']`）—— 防线测试会强制这一点。
   `llm:` 前缀下现已**只剩这 2 个**未迁移频道（`generate` / `generate-stream` / `cancel` 已于 D2-b 落地）。
2. **双栈同库行为对照**：同目录下 Electron（`vela.db`）与 Tauri（`lorekeeper.db`）各写各库，
   确认互不影响；顺带对照 `~/.vela/config.json` 与 `~/.lorekeeper/config.json` 的读写形态差异。
3. **`vitest` 全量超时定位**（遗留项 7）。
4. 之后进入 **批次 E**（定稿不可逆 + 删除生命周期，ADR 0003/0011 等量测试）。

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
  但「高频 chunk 下跨 IPC 桥的吞吐/延迟」尚无实测数据；建议在 D2-c 或批次 E 前补一次压测。
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
