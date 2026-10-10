# Tauri 迁移进度快照（2026-10-10）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> **规则**：按日期命名，一个工作日一个新文件；当日新增只写当日文件，跨日不回填旧文件；
> 旧文件冻结后不允许修改（见 [`docs-fork/agents/pi-development.md`](../agents/pi-development.md) §9）。
> **模板**：[`_TEMPLATE-tauri-migration-status.md`](./_TEMPLATE-tauri-migration-status.md)。
>
> 全部历史快照见 `docs-fork/handoffs/` 目录（按日期命名）。
>
> - channel 级盘点：[`docs-fork/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-10 · 第四十三次）

> 本表只填**最新一次自检的实测值**。改表前必须重跑对应命令，不得沿用旧数字、不得估算。
> 本轮实测命令与输出见下方「[§5 自检记录](#5-自检记录2026-10-10-实测)」。

| 项 | 值 |
|---|---|
| 仓库 / 分支 | **`TanCan1998/Lorekeeper`**（`EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork）· `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 已注册命令 | **194** |
| 覆盖 invoke 频道 | **193**（契约总数 193，事件频道 **5**） |
| 未迁移 invoke 频道 | **0**（`db=0 mcp=0`）✅ |
| orphan | **空** ✅ |
| `cargo test --lib` | **689/689** ✅（第四十三次：670 + H4 Task 3/4 新增 14 + Task 7 补强 5） |
| `cargo fmt --check` | **干净（0 差异）** ✅ |
| `cargo check --all-targets` | **0 告警** ✅ |
| `pnpm typecheck` / `lint` | exit 0 / exit 0 ✅ |
| 定向 `vitest` | **7/7**（2 文件：契约覆盖 / 入参结构体契约）✅ |
| 已完成批次 | A ✅ / B ✅ / C ✅ / D1 ✅ / D2-a ✅ / D2-b ✅ / D2-c ✅ / **E ✅** / **F1 ✅** / **L3 ✅** / **F2 ✅** / **批次 E G1 ✅** / **H1 ✅** / **H2 ✅** / **H3 ✅** / **B12 ✅** / **批次 G 的 G1 ✅** / **批次 G2a ✅** / **批次 G2b ✅（1–6 步全部完成）** / **批次 G3a ✅** / **批次 G3b ✅** / **批次 G4 ✅（代码收口）** / **B13 ✅** / **H4 ✅（本次）** |
| 当前阶段 | **H4 已收口（第四十三次）**：mcp 9 频道全部真实化并注册，**未迁移 invoke 9 → 0**（`db=0 mcp=0`）、已注册命令 **194** → 覆盖 **193**、自检 **689/689**。提交 `a2348a79`（stdio 传输层 + 连接状态机）+ `14d224d9`（注册 9 命令 + 前端登记 + `--emit` + 断言适配）+ P2 收尾（非 UTF-8 行不再断连、`drop_stale_servers` 去 TOCTOU + 5 例单测）。前序：**G4 代码已收口**（`94a1fe6a` + `5c4fd26e`）、**B13 已关闭**（`ef76662b`）。**⚠️ G4 的 GUI 冒烟（导入参照章节链路）仍待做**（用户决定稍后）；**B22 两段式关窗 dirty 分支仍待 GUI 实测**。其它待办：B14（真 Windows 自更新）、**上游合并专项（H4 收口后立即执行）**、B24（整屏瞬黑，暂缓） |
| 依赖 | `reqwest 0.13`（`default-features=false` + `native-tls` + `socks`）、`tauri-plugin-dialog 2`（锁 **2.8.1**）、`tauri-plugin-opener 2.7.0`、`windows-sys 0.61`（`[target.'cfg(windows)'.dependencies]`，仅 lock 提级，**0 新下载**；Ask first 已批准 2026-10-10）。**G2a 零新依赖**，**G3a 零新依赖**（仓储层平移，无 Cargo.toml 改动），**G3b 零新依赖**（命令层平移，无 Cargo.toml 改动）。向量层 `hnsw_rs 0.3.4` / `jieba-rs 0.7.0` / `tokio`；FTS5 由 `libsqlite3-sys` bundled 提供 |
| GUI 冒烟 | ✅ 自 2026-10-07 起 **十三轮**。⏳ **第十四轮（G4 参照章节链路）待做**：① 作者原稿全链路回归；② 参考语料选 `.md` → 应得 `preparation` 而非诚实错误，重选应得 `exact-duplicate`；③ `importReference` 成功写库并记「参照章节 N 已进入知识库」（重复显示「（已存在）」）。**第十三轮（2026-10-10，弹窗动画统一）**：设置弹窗明显变快（修复前实为“静置 400ms + 播 220ms”）、四类弹窗进出场一致、Radix 弹窗仍居中且尺寸正常 ✅。**第十二轮（2026-10-10，冒烟发现的三项缺陷修复）**：① 弹窗 **ESC 关闭**（根因：Radix `DismissableLayer` 仅在 `index === layers.length-1` 时注册 ESC，而 Radix 关闭后仍保留 `DialogContent` 挂载——实测 `layers.length=7`，可见弹窗永远不是最高层）；② **窗口命令真实化**（批次 A 四个命令原为假成功骨架）；③ **标题栏拖拽**（`-webkit-app-region` 在 WebView2 无效 → 补 `data-tauri-drag-region`）。4 项人工验证全部 ✅。近三轮：第十一轮（G2a）、第十轮（G1）、第九轮（H 前三项 + B12） |
| 双栈隔离 | **L0/L1/L2/L3 全部独立**：安装标识 / `~/.lorekeeper` / `<root>/.lore/`（库 `.lore/lorekeeper.db`、KB 向量 `.lore/kb/`）。基线为 `~/.vela` / `<root>/.vela/`。**两栈项目目录刻意不互通**（`ee40aaab`） |
| Rust 工具链 | rustc/cargo **1.99.0 stable-msvc** @ `D:\Environment\rust\`（脚本内须显式设 `RUSTUP_HOME` / `CARGO_HOME`）。`tauri-plugin-dialog 2.8.1` 要求 **rustc ≥ 1.90**（CI 最低版本需相应抬高） |

<sub>*命令与频道差额：**185 命令**中 1 个为阶段 0 骨架（`app_health_check`，不对应任何契约频道），其余 **184** 与 invoke 频道一一对应。</sub>

---

## 本次更新（2026-10-10 · 第四十三～四十次）

### 1. H4 收口 ✅ Task 3+4 + Task 6+7（mcp 9 频道全部真实化；未迁移 9 → 0）

落盘（提交 `a2348a79` + `14d224d9`）：

- 新增 `tauri-app/src-tauri/src/mcp/transport.rs`（538 行）：`McpStdioTransport`（`Command` argv 直传不经 shell、env 继承后叠加、stdin/stdout/stderr 三管道）+ 读线程 `\n` 分帧 + `pending` 表（超时与 `close` 双路清理）+ 10s 超时 `MCP 请求超时: <method>` + Windows `CREATE_NO_WINDOW`（D-H4-3）+ `Drop` 兜底 kill 子进程；
- `tauri-app/src-tauri/src/mcp/mod.rs`（780 行，+507/−93）：`connect` / `disconnect` / `disconnect_all` / `call_tool` / `get_servers_status` / `drop_stale_servers` 真实实现；握手 `initialize`（`protocolVersion:'2024-11-05'` + `clientInfo {name:'vela',version:'1.0.0'}`，D-H4-2）+ `notifications/initialized`；`tools/list` / `resources/list` 失败退空数组；未连接文案 `服务器 <id> 未连接`；运行时表改持 `Arc<McpServerRuntime>`（D-H4-6）；
- `tauri-app/src-tauri/src/lib.rs:267-276` 注册全部 9 个 `commands::mcp_*`；`tauri-app/src-tauri/src/state.rs` 常驻装配 `mcp: Mutex<Arc<McpManager>>`（`mcp_manager()`，锁中毒经 `into_inner` 恢复）；删除 `commands/mcp.rs::manager_or_transient` 临时实例回退与 `state.rs` / `commands/mod.rs` 两处临时 `#[allow]`；
- 前端：`tauri-app/src/services/ipc-client.ts:233-237` 登记 `mcp:connect` / `mcp:disconnect` → `['serverId']`、`mcp:call-tool` → `['serverId','toolName','args']`；`tauri-app/src/shared/migrated-channels.ts` 由 `node scripts/verify-channel-coverage.mjs --emit` 重生成；`tauri-app/test/channel-migration-coverage.test.ts:125` 的「真子集」断言改写为「等于契约 invoke 全集」（含反向包含循环），并补 9 频道状态断言（D-H4-4 收口）。

**独立复验（编排者本人终端，非引用子代理）**：`cargo fmt --check` exit 0；`cargo check --all-targets` exit 0（0 告警）；`cargo test --lib` → **684 passed / 0 failed**（670 + 14）；`pnpm typecheck` / `pnpm run lint` exit 0；`node scripts/verify-channel-coverage.mjs` → 契约 invoke **193**（事件 **5**）/ 已注册命令 **194** → 覆盖 **193** / **未迁移 0** / 命令名与契约频道一一对应 ✅；定向 `vitest` 2 文件 7/7。

**只读评审结论：无 P0/P1，可进下一批**；5 项 P2 中 2 项必修 + 单测补强已完成（见下段），另 2 项登记为偏离（D-H4-8 / D-H4-9）。

**Task 7 余项已完成**：① `transport.rs` 读线程改按字节读行（`read_until` + `String::from_utf8_lossy`）——非 UTF-8 行只损失该行，连接不再被 `close("连接已断开")` 杀掉（对齐基线 `processBuffer` :301-318）；② `mod.rs::drop_stale_servers` 在同一把 `servers` 表锁内 `retain` 摘表并同步 `loaded_configs`、锁释放后再 `shutdown`，消除「无锁收集 → 逐个 disconnect」期间并发 connect 同 id 被误杀的 TOCTOU（锁序 servers → loaded_configs 为全局唯一嵌套序，无反转；`load_config` 不在持 `loaded_configs` 锁时调用它，无自锁风险）；③ 新增 5 例实质单测：`read_loop_frames_crlf_lines_test` / `read_loop_skips_invalid_utf8_line_test` / `wait_response_returns_disconnected_on_close_test` / `close_is_idempotent_test` / `spawn_node_subprocess_e2e_test`（`node -e` 真子进程应答 initialize / tools/list / resources/list，环境无 node 时跳过）。复验：`cargo fmt --check` exit 0、`cargo check --all-targets` **0 告警**、`cargo test --lib` **689 passed / 0 failed**。⚠️ **既有 flake（非本轮引入）**：全量并行跑时 `db::vector::tests::insert_and_search_maps_doc_id_back_test`（`src/db/vector.rs:503`）偶发失败（本轮首跑 688/689）；单测复跑 3/3、全量复跑 689/689 均通过——即 B16 登记的 HNSW 墓碑测试 flake。

**刻意偏离（第四十三次登记，详见开工清单 §5 / §5.1）**：D-H4-6（`McpServerRuntime` 删 `Clone` 改持 `Arc`）；D-H4-7（同步模型 4 条等价替换：阻塞式请求-响应、stdin 写失败立即可报 `连接已断开`、进程退出状态动态反映、`connect` I/O 不持表锁 + `Arc::ptr_eq` 校验）；D-H4-8（`as_i64` 只认数字 id，字符串 id 响应被忽略——请求 id 恒为自增数字，基线 `Map.get` 不会遇到此形态）；D-H4-9（`write_all` 无写超时，子进程不读 stdin 时写可阻塞，与基线同样可能挂起）。

### 2. H4 起步 ✅ Task 1+2（mcp 类型镜像 / 管理器骨架 / 配置层；**刻意不注册命令**）

用户 2026-10-10 定下 H4 前提：信任模型 **(a) `std::process::Command` + 自研守卫（零新依赖）**；顺序 **先 H4 收口，再合并上游**；合并方式 `git merge upstream/master --no-ff`。

落盘文件（本批尚未提交）：

- 新增 `tauri-app/src-tauri/src/mcp/mod.rs`（366 行）、`mcp/types.rs`（254 行）、`mcp/config.rs`（399 行）、`tauri-app/src-tauri/src/commands/mcp.rs`（148 行）；
- `tauri-app/src-tauri/src/state.rs`（+14）：`pub(crate) mcp: Mutex<Option<Arc<crate::mcp::McpManager>>>` + `mcp_manager()`（照 `update` / `update_service()` 范式）；
- `tauri-app/src-tauri/src/commands/mod.rs`（+5）：`pub mod mcp;` + `pub use mcp::*;`（带临时 `#[allow(dead_code)]` / `#[allow(unused_imports)]`）；
- `tauri-app/src-tauri/src/lib.rs`（+5）：**仅** `pub mod mcp;` 模块声明（**未**加入 `generate_handler!`）。

实现范围：`mcp_load_config` / `mcp_get_config_path` **真实实现**（配置层三态 + `lorekeeper_home()/mcp_config.json`，D-H4-1）；其余 7 个命令返回契约类型占位（`'MCP 传输尚未实现'` 等），传输层 Task 3 落地。与基线逐条对齐：`name = id`（`config.rs:150`）、`has_command === has_url` 且 `trim()` 判空（`config.rs:133-144`）、三条失败分支均先 `disconnect_all()`（`mod.rs::load_config`）、`drop_stale_servers` 用 `runtime.config != config`、异常文案逐字一致。

**刻意偏离（第四十二次登记）**：

- **D-H4-4**：本轮**不注册** `lib.rs` 的 `generate_handler!`（相对开工清单 §7 Task 1 的调整）。根因：`tauri-app/test/channel-migration-coverage.test.ts:110` 断言 `migrated-channels.ts` 与 `lib.rs` 注册命令**精确相等**，中途注册立即变红；且中途 `--emit` 重生成会让「未迁移 0」提前变绿、掩盖桩实现。注册 + `--emit` + 断言适配（`:125-128` 的「已迁移集合是契约真子集」断言在未迁移归零后必须改写）统一收口于 Task 6/7。
- **D-H4-5**：`load_config` 成功分支的 `servers` 摘要数组为 **id 字典序**（`BTreeMap`），基线 `Object.entries` 为**文件顺序**；内部 `servers` / `loaded_configs` 两表同样用 `BTreeMap`（基线 `Map` 为插入序）。渲染层仅列表展示，取确定性序。

**自检（2026-10-10 · 第四十二次实测，编排者本人终端，非引用子代理）**：`cargo fmt --check` exit 0；`cargo check --all-targets` exit 0（**0 告警**）；`cargo test --lib` → `test result: ok. 670 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 51.85s`（即基线 655 + 新增 15）；频道覆盖口径**未变**（未迁移仍 **9** `[mcp=9]`，因未注册）。

**待下批（Task 3..7）**：stdio 传输层（`mcp/transport.rs`，`std::process::Command` + 自研守卫）→ 连接状态机 → `call-tool` / `list-tools` / `list-resources` → 前端登记 + 注册 + `--emit` + 测试适配 → 单测补强。**收口前必须删除**：`commands/mcp.rs::manager_or_transient` 的临时实例回退（否则状态不共享），以及 `state.rs` / `commands/mod.rs` 的 `#[allow(...)]` 临时豁免。

### 3. B13 ✅ 渲染层导航防护收口（提交 `ef76662b`）

HEAD 自 `01d0f6e2` 前进到 `bdd9d3c3`，含两个提交：`ef76662b`
`feat(tauri): 新增 B13 渲染层导航防护插件` 与 `bdd9d3c3`
`docs(tauri): 新增 H4（mcp 9 频道）开工清单`。

B13 已关闭（基线 `preventRendererNavigation` + 新窗口拦截的 Tauri 侧对应实现）：

- 新增 `tauri-app/src-tauri/src/navigation_guard.rs`：
  `tauri::plugin::Builder::new("navigation-guard").on_navigation(...)`
  拦截渲染层导航（放行 app 源 / 官方主页，其余拒绝）；
- `tauri-app/src-tauri/src/lib.rs`：`mod navigation_guard;` +
  注册 `.plugin(navigation_guard::init())`；
- `tauri-app/src-tauri/src/commands/official_homepage.rs` 的常量改
  `pub(crate)` 复用（`OFFICIAL_HOMEPAGE_URL`，守卫放行官方主页）。

阻塞项表 B13 行已标记「✅ 已解除（依据：`ef76662b`）」（只增不删，原行保留）。
`cargo test --lib` 由 652 增至 **655**（+3：`allows_app_origins_test` /
`rejects_foreign_and_dangerous_urls_test` / `official_homepage_matches_exactly_test`）。

### 4. H4（mcp 9 频道）开工清单已产出（第四十一次）—— 前提已于第四十二次确认

- 开工清单：`docs-fork/plans/2026-10-10-h4-mcp-kickoff.md`（提交 `bdd9d3c3`）；
- 仍**待用户确认**开工清单 §4 信任模型：(a) `std::process::Command` + 自研守卫
  vs (b) `tauri-plugin-shell`——**确认后才能开工**（未迁移仍为 **9**，仅剩 `mcp=9`）。

### 5. 刻意偏离（D-B13-1 / D-B22-1 / D-B22-2，第四十一次登记）

- **D-B13-1**：`navigation_guard::init` 为非泛型
  `pub fn init() -> TauriPlugin<tauri::Wry>`，而非 `init<R: Runtime>()`。
  根因：`external_link::open_external_url(app: &AppHandle, ...)` 的形参是具体
  `AppHandle<Wry<EventLoopMessage>>`，泛型闭包内只能拿到 `&AppHandle<R>`
  → E0308 无法统一。本 crate 桌面端唯一运行时即 Wry，行为不变。
- **D-B22-1**：窗口关窗守卫是**全局单例**（`tauri-app/src-tauri/src/state.rs`
  中单一 `Mutex<WindowCloseGuard>`，即 `window_close_guard` 字段），基线
  `closeGuardStates` 是 per-window；当前仅 `main` 一个窗口故不可达，将来新增
  第二窗口会出现「已有 pending 被静默吞掉」的风险。
- **D-B22-2**：基线 `isDestroyed()` / `isLoadingMainFrame()` 放行分支在 Tauri
  无对应概念（已在 `tauri-app/src-tauri/src/commands/window.rs` 头注释记录：
  `CloseRequested` 只对存活窗口触发，故仅保留 `approved` 一条放行条件）。

### 6. 第四十一次本轮验证事实

- **B22 两段式关窗的端到端仍需 GUI 人工实测，本轮未做、不得写成已通过**：
  OS 关闭按钮 / Alt+F4、保存并退出 / 放弃并退出 / 取消三按钮、
  脏状态与 workflow 运行中分支、approved 后二次 close 放行、
  WebView2 事件竞态。

### 7. 自检（第四十一次实测，2026-10-10）

- `pnpm typecheck` → exit 0；`pnpm run lint` → exit 0
  （`eslint . --ext ts,tsx --report-unused-disable-directives --max-warnings 0`）；
- `node scripts/verify-channel-coverage.mjs` → 契约 invoke 频道 **193**（事件频道 **5**）/
  已注册命令 **185** → 覆盖 invoke 频道 **184** / 未迁移 **9**（`mcp=9`）·
  命令名与契约频道一一对应 ✅；
  ⚠️ 口径更正：脚本正则曾漏计 `ipc-channels.ts` 的 `'update:state': UpdateState`（类型引用而非对象字面量），故事件频道旧值 **4** 为正则 artifact（实为 **5**：`llm:stream-chunk` / `llm:stream-done` / `llm:stream-error` / `update:state` / `window:close-requested`），已由 Task C-1 修正；
- `cargo fmt --check` → 0 行输出（exit 0）；
- `cargo check --all-targets` → `Finished \`dev\` profile [unoptimized + debuginfo] target(s) in 1.07s`，**0 告警**（exit 0）；
- `cargo test --lib` → `test result: ok. **655 passed; 0 failed**; 0 ignored; 0 measured; 0 filtered out; finished in 47.91s`（exit 0）；
- `git status --porcelain -- docs` → **0 行**（`docs/` 零改动）。

### 本次更新（第四十次：批次 G3b 收口）

#### 1. G3b ✅ 频道注册（13 频道，未迁移 22 → 9）
提交 `f31446ec`（5 文件，+770/−3）按开工清单 §4
完成命令层注册。仓储层（G3a，`c8c56ad9`）提供的全部
seam 在本层接上 IPC：

- **执行租约 5 频道**：`db:import-run-start-resume` /
  `renew-execution` / `restart` / `request-cancel` /
  `cancel-at-boundary`——`start_or_resume` / `renew_execution`
  传 `now_ms()` + `DEFAULT_EXECUTION_LEASE_MS`（15 min）；
  `restart` 传 `now_ms()`；`request_cancel` /
  `cancel_at_boundary` 无时间参数（仓储内部取当前时间）
- **批次推进 4 频道**：`complete-batch` / `advance-stage` /
  `fail` / `complete`——仓储内部取当前时间，命令层不传
- **effect receipts 3 频道**：`effect-receipt-get`（**读频道**，
  失败直接 reject，无 `{success}` 信封，对齐基线 handler）/
  `effect-receipt-prepare`（传 `now_ms()`）/
  `effect-receipt-commit`（`project_root` 取
  `expected_project_path`，对齐基线仓储内部
  `getCurrentProjectPath()`；跨仓原子事务）
- **全局事实 1 频道**：`db:import-global-facts-commit`——
  `ImportGlobalFactsRepository::commit`（事务：幂等重放或
  核心台账 + roster 提交）
- **D4 落地**：`completeBatch` 的 direct-stage 前置校验
  （`is_import_run_direct_checkpoint_stage`：仅 `knowledge` /
  `author-publish` / `author-postprocess` / `refresh`）在
  命令层实现并先于仓储调用，对齐基线 db-controller handler
  前置断言；`'parsing'` 等非 direct stage 返回
  `该导入阶段不接受直接 checkpoint`（经 `mutating_error` 包装）
- **命令层自算时间**：仓储 `now_ms()` 为私有，命令层自带
  `import_run_now_ms()` helper（SystemTime 毫秒）
- **信封结构**：7 个 camelCase `#[serde(rename_all =
  "camelCase")]` 响应结构体（`ImportRunStartResumeResult` /
  `ImportRunRenewExecutionResult` / `ImportRunRunMutationResult`
  / `ImportRunCompleteBatchResult`（含 `newlyCompleted` /
  `cancelApplied`）/ `ImportRunEffectReceiptPrepareResult` /
  `ImportRunEffectReceiptCommitResult` / `ImportGlobalFactsCommitResult`），
  `Option` 字段全部 `skip_serializing_if`，与契约返回结构一致
- **每命令** `command + _inner` 双函数模式：命令函数供
  `invoke_handler` 注册（`map_err(mutating_error)`，写面带
  `Error: ` 前缀），`_inner` 供测试直调（错误无前缀）
- **前端门禁**：`ipc-client.ts` 的 `CHANNEL_ARG_NAMES` 登记
  13 频道参数名（如 `complete-batch` =
  `['runId','stage','batchId','execution','expectedProjectPath']`）；
  `migrated-channels.ts` 生成物经 `check:channels:emit` 重新
  生成（184 频道）；`channel-migration-coverage.test.ts` 的
  迁移状态断言同步更新（G3 频道由「仍未迁移」改为已迁移）
- **新增命令层测试 2 例**：D4 拒绝（非 direct stage +
  MUTATING 包装断言 + 缺会话拒绝）、start-resume 签发租约 →
  running + 同一执行器 renew 顺延 + 缺会话拒绝（经 reference
  通道 `prepare-inspection` 落地 'prepared' 可恢复运行）

**GUI 冒烟**：本轮无新增冒烟轮次——13 频道均为导入执行器
内部状态机 seam，前端调用方（导入执行 UI）属后续批次，当前无
用户可触达路径；覆盖由单元测试（仓储 36 例 + 命令层 2 例）
与频道契约测试保证。

### 2. 前置：G3a ✅ 仓储层（已收口，提交 `c8c56ad9`）

#### 2. 前置：G3a ✅ 仓储层（已收口，提交 `c8c56ad9`）
8 文件 +5202/−861，移植测试 36 例。）

### 3. G3a 仓储层细节（13 频道的全部仓储 seam）
#### 3. G3a 仓储层细节（13 频道的全部仓储 seam）
提交 `c8c56ad9`（8 文件，+5202/−861）平移基线
`electron/repositories/import-run-repository.ts`（2551 行）与
`import-global-facts-repository.ts`，落地开工清单 §3 全部内容：

- **执行租约族**：`assert_execution_authority` / `assert_execution` /
  `start_or_resume` / `renew_execution` / `restart` / `request_cancel` /
  `cancel_at_boundary`（`DEFAULT_EXECUTION_LEASE_MS = 15 * 60_000` 对齐）
- **批次推进族**：`next_stage_for_run` / `author_checkpoint_chapter_number` /
  `assert_checkpoint_can_apply` / `assert_stage_checkpoint_complete` /
  `apply_batch_checkpoint` / `complete_batch` / `advance_stage` / `fail` / `complete`
- **effect receipts**：`canonicalize` / `canonical_payload` / `exact_keys` /
  `assert_effect_payload_schema` / `assert_effect_payload_binding` /
  `assert_author_effect_run_binding` / `assert_committed_effect_schema` /
  `assert_completed_blueprint_sync_operation` / `assert_blueprint_effect_authority` /
  `assert_committed_effect_authority` / `validate_effect_stage` /
  `row_to_effect_receipt` / `get_effect_receipt` / `prepare_effect_receipt` /
  `commit_effect_receipt`（跨仓原子事务，复用下游 `_in_transaction` 抽取件；
  `MAX_EFFECT_RECEIPT_PAYLOAD_BYTES = 16 MiB`）
- **新模块** `import_global_facts_repository`：`normalized_request`（13 文本字段
  trim + 枚举校验 + 总章数/章节字数校验）/ `hash_request` / `core_snapshot` /
  `parse_receipt` / `ensure_ledger`（D2 懒建表）/ `get_committed_operation`
  （核心台账 + roster factHash 比对，拒绝历史操作冒充当前事实）/ `commit`
  （事务：幂等重放或 `ProjectCoreRepository.update` + `CharacterRosterRepository.commit`）
- **下游读回 helper**：`blueprint_repository::get_committed_range_operation`、
  `finalized_draft_import_repository::get_committed_operation`、
  `character_roster_repository` 事务体抽取
- **会话边界围栏**：`create_tables` 末尾 `fence_import_run_execution_leases`
  （对齐基线 `createTables` 的「打开项目即围栏上一会话租约」迁移）

**测试**：移植基线三个测试文件，净增 **36 例**（execution-lease 11 /
state-machine 5 / receipt 12 例 22 用例，含 committed 回放、离线伪造拒绝、
失败关闭、键-载荷绑定、过期租约围栏、全局事实台账哈希校验）。

#### 4. 刻意偏离（D1–D4，见开工清单 §2）

`adoptLegacyCompletedRun` 不移植（双栈隔离下 `.lore` 无 legacy 运行）；全局事实
台账懒建表（不改 `db/schema.rs`，G schema 刻意固定 9 表）；`canonicalize` 键排序
用字节序（哈希仅 Tauri 内部自洽）；`completeBatch` 的 direct-stage 前置校验
（`isImportRunDirectCheckpointStage`）留在命令层 G3b（对齐基线 db-controller
handler 前置断言，D4）。恢复 D1–D3 需用户确认。

### 5. 自检（本轮，G3b）
#### 5. 自检（本轮，G3b）
`cargo test --lib` **645/645**（G3a +36 后再 +2 命令层）·
`cargo check --all-targets` **0 告警** · `cargo fmt --check` 干净 ·
`pnpm typecheck` / `lint` exit 0 · 定向 `vitest` **7/7** ·
`check:channels` 193 契约 / **185 命令 / 184 覆盖 / 未迁移 9**
（仅剩 `mcp=9`）· 提交消息检查 + pre-commit gitleaks 无命中。

（G3a 轮自检：643/643、193/172/171/22，无频道变化——纯仓储层。）

（**2026-10-10 12:23 G4 后复测——实跑定案**：快照表此前停在 **645**，而 `docs-fork/todo.md` 的 G4 完成说明记 **652**，两处不一致，故重跑全部指标命令定案：
`cargo test --lib` → `test result: ok. **652 passed; 0 failed**; 0 ignored; 0 measured; 0 filtered out; finished in 50.93s`（exit 0）；
`cargo fmt --check` → 0 行输出（exit 0）；`cargo check --all-targets` → `Finished \`dev\` profile [unoptimized + debuginfo] target(s) in 0.85s`、**0 告警**（exit 0）；
`pnpm typecheck` / `pnpm run lint` → exit 0 / exit 0（lint 命令：`eslint . --ext ts,tsx --report-unused-disable-directives --max-warnings 0`）；
`node scripts/verify-channel-coverage.mjs` → 契约 invoke 频道 **193**（事件频道 **5**）/ 已注册命令 **185** → 覆盖 invoke 频道 **184** / 未迁移 **9**（`mcp=9`）、`命令名与契约频道一一对应 ✅`。

**结论：645 是 G4 之前的旧值；G4 的 `commands::kb` 新增 7 例后为 652**（与 `docs-fork/todo.md` 的 G4 记录一致）。频道覆盖各项与快照原值一致，无变化。）

---

## 交接给下次会话（**从这里接**）

### 1. 当前工作区状态

**跟踪文件干净**（唯一未跟踪项为 `docs-fork/todo.md`，按约定永不入库）；`docs-fork/research/2026-10-10-b24-black-flash-investigation.md`（B24 暂缓中的排查记录）已补提交 `2f28d4a9` 入库。G3a / G3b / G4 均以单提交落地：

| Commit | 说明 |
|---|---|
| `b4530e50` | `fix(tauri): ClearProjectDataDialog 纳入统一弹窗动画（B25 收口）` |
| `1e0c1ce3` | `feat(tauri): G2b-6 复活 dialog:select-novel-files 的 reference 分支` |
| `237f838a` | `feat(tauri): G2b-5 导入运行两频道（prepare-inspection / finalize-parsing）` |
| `720337db` | `feat(tauri): G2b-4 prepare 两分支（author / reference）` |
| `adb0e74e` | `feat(tauri): G2b-3 finalizeParsing 四态分类与准备检视` |
| `ad7a77ea` | `fix(tauri): 导入弹窗重开或切换项目时复位会话状态` |
| `a91d9f5c` | `chore(tauri): 对齐浏览器测试依赖并补浏览器测试配置` |
| `c8c56ad9` | `feat(tauri): G3a 导入运行执行租约/批次推进/effect receipts 仓储层` |
| `461c087c` | `docs(tauri): 第四十次快照（批次 G3a 收口）` |
| `f31446ec` | `feat(tauri): G3b 导入运行执行租约/批次推进/effect receipts（13 频道注册）` |
| `40123891` | `docs(tauri): 扩写第四十次快照（批次 G3b 收口）` |
| `94a1fe6a` | `feat(tauri): G4-1 参照文档幂等存储层（哈希/完整性/stable-id 重写）` |
| `5c4fd26e` | `feat(tauri): G4-2 kb:import-reference-text 去占位真实化` |
| `7eeea498` | `docs(tauri): G4 收口登记与刻意偏离 D-G4-1` |
| `c0a0c5c2` | `docs(fork): 子代理编排验证开工清单（PiDeck 子代理面板）` |
| `2f28d4a9` | `docs(fork): B24 整屏瞬黑排查记录` |
| `2f28d4a9` | `docs(fork): B24 整屏瞬黑排查记录` |
| `ef76662b` | `feat(tauri): 新增 B13 渲染层导航防护插件` |
| `bdd9d3c3` | `docs(tauri): 新增 H4（mcp 9 频道）开工清单` |
| `a2348a79` | `feat(tauri): H4 Task 3+4——mcp stdio 传输层与连接状态机` |
| `14d224d9` | `feat(tauri): H4 Task 6+7——注册 9 个 mcp 命令并归零未迁移频道` |
- HEAD（写入本轮时）：`14d224d9 feat(tauri): H4 Task 6+7——注册 9 个 mcp 命令并归零未迁移频道`
- **推送状态（2026-10-10）**：`origin/master` 曾落后 10 个提交（末个远端为 `03c9f230`），本轮补提交后**已全部推送**（`03c9f230..2f28d4a9`，`ahead 0 / behind 0`）。
- **推送状态（2026-10-10）**：上一轮已推送至 `2f28d4a9`；本轮两提交（`ef76662b` + `bdd9d3c3`）**尚未推送**（`git status -sb`：`master...origin/master [ahead 2]`）。
- **推送状态（2026-10-10 · 第四十三次）**：H4 Task 3+4 与 Task 6+7 两提交（`a2348a79` + `14d224d9`）**尚未推送**（`git rev-list --left-right --count origin/master...HEAD` = `0 2`）；按用户决定「H4 收口后一次性提交并推送」处理。
  1. 第三十九次「`cargo test --lib` **607/607**」→ G3a 移植 36 例后为 643，G3b 再 +2 命令层测试后实测为 **645/645**；
  2. 第三十九次「下一步 G3（执行租约 / 批次推进 / effect receipts 11 频道 + `db:import-global-facts-commit`）」→ **G3a（仓储层）与 G3b（频道注册）均已完成**，下一步是 **G4**；
  3. 第四十次 G3a 版「下一步 **G3b**（13 频道注册）」→ **G3b 已完成**（提交 `f31446ec`，未迁移 22 → 9）；
  4. 第三十九次快照的 §5 自检已冻结，**不得据其回填**本份数字。
  4. 第三十九次快照的 §5 自检已冻结，**不得据其回填**本份数字。
  5. 第四十次「当前阶段」待办中的 **B13（导航防护）** → **已关闭**（`ef76662b`，第四十一次）；
  6. 第四十次「下一步」所述 **H4（mcp 9 频道，暂缓）** → 开工清单已产出（`docs-fork/plans/2026-10-10-h4-mcp-kickoff.md`），**待用户确认 §4 信任模型后才能开工**（见下方 §2）。
### 2. 下一步（1-2-3）

1. **批次 G4 ✅ 代码已完成**（2026-10-10，提交 `94a1fe6a` + `5c4fd26e`）：`kb:import-reference-text` 去占位真实化（G4-1 存储层 `db/kb/store.rs` 参照文档幂等 seam + G4-2 命令层 `commands/kb.rs`）；为占位频道真实化，未改变未迁移计数（仍为 9，仅剩 mcp）。**⚠️ G4 的 GUI 冒烟（导入参照章节链路）仍未做**（用户决定稍后）：清单为 ① 作者原稿全链路回归；② 参考语料选 `.md` → 应得 `preparation`（classification + 预览）而非诚实错误，同一文件重选应得 `exact-duplicate`；③ 工作流跑到 `importReference` 时应成功写库并打日志「参照章节 N 已进入知识库」，重复运行显示「（已存在）」。
2. **子代理编排通道已验**（开工清单 [`docs-fork/plans/2026-10-10-subagent-pideck-kickoff.md`](../plans/2026-10-10-subagent-pideck-kickoff.md) §8）：`lorekeeper-task` + `bash` 端到端可用（子代理返回 `c0a0c5c2` 与编排者一致）；PiDeck 面板出条目已确认；`toolUses`/`tokens` 实时跳动**未取得界面证据**。
3. **其它待办**：B13（导航防护）、B14（真 Windows 自更新）、H4（mcp，暂缓）、上游合并专项、B24（整屏瞬黑，暂缓）。
3. **B13 ✅ 已关闭**（`ef76662b`）；**H4 待用户确认**开工清单 §4 信任模型（(a) `std::process::Command` + 自研守卫 vs (b) `tauri-plugin-shell`）后开工。其它待办：B14（真 Windows 自更新）、上游合并专项、B24（整屏瞬黑，暂缓）。
4. **H4 已收口（第四十三次）**：Task 3+4（stdio 传输层 + 连接状态机）与 Task 6+7（注册 9 命令 + 前端登记 + `--emit` + 断言适配）均已落地（提交 `a2348a79` + `14d224d9`），**未迁移 invoke 0**；Task 7 的 P2 必修项与 5 例单测补强亦已完成（自检 **689/689**）。收口后立即执行 **上游合并 `git merge upstream/master --no-ff`**（上游 tip `21d67211`，分叉 `509 150`，共同祖先 `992b3f5f`，唯一冲突文件 `README.md`；实测上游 mcp 基线仅 +1 行 `assertGlobalDataReady()`（Tauri 侧不适用）、mcp 9 契约两版逐字一致；上游契约 **+14 频道、0 删除** → 合并后未迁移将为 **23**）。
**刻意偏离登记（批次 G4）**：**D-G4-1** —— `tauri-app/src-tauri/src/db/kb/store.rs::document_integrity` 的 `complete` **只反映 canonical 完整性**（文档行唯一 + 块序列严格 `0..n` + `total_chunks`/`corpus_kind` 自洽），**不含基线的 `embeddingGenerations` 检查**（基线 `electron/vector-store.ts:1464-1470`：`complete = canonicalComplete && embeddingGenerations.every(g => g.status === 'building' || g.complete)`）。原因：Tauri 侧向量由文件型 `LocalVectorIndex` 持有、不入 SQLite，无法在 SQL 层复现代际检查。恢复需用户确认。

### 3. 阻塞项与待授权项（**不得删除，须逐条确认后更新**）

| # | 项 | 状态 |
|---|---|---|
| B1 | 批次 E 定稿频道 | ✅ 完成（第三十二次） |
| B2 | `chapter:*` 物理清理 | ✅ 全部解除（F2-3 删 KB 文档 + 第三十二次删实体稿） |
| B3 | `dialog:select-export-directory` | ✅ 已真实化（第三十三次 H1） |
| B4 | L3 双栈隔离 | ✅ 已执行 |
| B5 | `cargo fmt --check` | ✅ 已解除；⚠️ **未加入 CI**，改 CI 仍需 Ask first |
| B6 | `tauri-app` 全量 `pnpm test` | ⚠️ 仍超时；凡未 mock `ipc-client`/未注入 `__TAURI_INTERNALS__` 的流程测试在 node 下会失败（既有，非本轮） |
| B7 | 「证据不在正文中」反例 | ⚠️ 未验证（无正文数据） |
| B8 | 2 个既有 `vitest` 失败 | ✅ 已解除（第三十三次） |
| B9 | `tauri-plugin-dialog 2.8.1` 要求 rustc ≥ 1.90 | ⚠️ CI 最低版本需相应抬高 |
| B10 | `fs:grant-*` 占位阻塞 KB 导入/导出 | ✅ 已解除（第三十三次 H1） |
| B11 | `chapter:delete-finalized` 入参缺 camelCase | ✅ 已修复 + 源码扫描防线 |
| B12 | 「打开外部链接」缺失 | ✅ 已解除（第三十三次） |
| B13 | 渲染层导航防护未接入 | ✅ 已解除（依据：`ef76662b`：新增 `src/navigation_guard.rs`（`navigation-guard` 插件，`on_navigation` 拦截渲染层导航），`lib.rs` 注册 `.plugin(navigation_guard::init())`） |
| B14 | 真正的 Windows 自动更新 | ⚠️ 需 `tauri-plugin-updater` + 签名公钥 + 打包链路 |
| B15 | H4（mcp 9 频道） | ✅ **已解除（第四十三次）**：`a2348a79`（stdio 传输层 + 连接状态机）+ `14d224d9`（注册 9 命令 + 前端登记 + `--emit`），未迁移 **9 → 0**；仅实现 stdio（SSE 对齐基线仍不实现），配置根改 `lorekeeper_home()/mcp_config.json`（D-H4-1） |
| B16 | `db/vector.rs` HNSW 墓碑测试偶发失败 | ✅ 已修（第三十四次） |
| B17 | 批次 G schema | ✅ 已获批并落地 9 张表（`cca792cd`） |
| **B18** | **`.epub` 导入依赖** | ⚠️ **新增（本次）**：需 `zip` 类 crate 解包，属 Ask first；G1 返回 D4 诚实错误，对话框仍列出 epub |
| **B19** | **`reference`（参考语料）路径** | ✅ **已解除**（G2b-6 `1e0c1ce3`：`dialog:select-novel-files` 的 reference 分支复活，选择期即落地解析运行） |
| **B20** | **D7 zh-CN 排序不等价** | ⚠️ **新增（本次）**：无 ICU 依赖，来源文件名排序用数字感知自然序近似；如需逐字对齐须 Ask first 引 ICU |
| **B21** | **G1 GUI 冒烟** | ✅ **已解除（本次）**：第十轮冒烟 3 项全部通过（作者原稿预览 / reference 诚实错误 / epub 诚实错误），dev 日志无 error/panic |
| **B22** | **两段式关窗的未保存内容确认未验证** | ⚠️ **新增（本次）**：`window:close` 现在会拦截并广播 `window:close-requested`，渲染层无 dirty 时直接 `proceed`；**dirty 分支（确认框 + cancel / 再次关窗）尚未实测**，需构造未保存内容后再验 |
| **B23** | **手写弹层无退出动画** | ✅ **已解除（本次）**：`SettingsModal` 已改为延迟卸载 + 统一进出场（`.lk-dialog-backdrop` / `.lk-dialog-panel`），第十三轮实测有淡出 |
| **B24** | **窗口最小/最大化后整屏瞬黑（闪烁）** | ⚠️ **新增（本次）·用户决定暂缓到专门批次**。现象：最小/最大化后鼠标在窗口内移动时**整屏瞬黑**（偶发）；**浏览器打开同一页面拖动不闪** → 壳层问题。已排查且排除：透明/effect 配置、常驻 `backdrop-filter`、resize 重渲染风暴、`backgroundColor` 缺失、`shadow:false`（实测无效已回滚）；事件日志无 TDR/dxgkrnl/DWM 错误。机器：AMD Radeon(2021‑11‑30 驱动) + RTX 3060 Laptop 混合显卡、单屏 2560×1440@**165Hz**、**FreeSync/VRR 开启**。候选方案：M3 给 `lorekeeper.exe` 指定单一 GPU ／ M4 临时 60Hz ／ A2 WebView2 `--disable-direct-composition` ／ M1 关 MPO（注册表，需审批+重启）／ M2 更新 AMD 驱动 |
| **B25** | **`ClearProjectDataDialog` 未纳入统一动画** | ✅ **已解除**（`b4530e50`：手写全屏弹层纳入统一进出场，GUI 已验淡入淡出） |

### 4. 红线提醒（每次接手都要过一遍）

- 🚫 Tauri 侧**禁止**读 `AI_NOVEL_VELA_HOME`、**禁止**回退 `~/.vela`、**禁止**写 `.vela/vela.db`。
- 项目库 = `<root>/.lore/lorekeeper.db`；KB 向量 = `<root>/.lore/kb/`；全局数据根 = `AI_NOVEL_LOREKEEPER_HOME` 或 `~/.lorekeeper`。
- 失败文案按基线 MUTATING 规则带 `"Error: "` 前缀（`commands/db.rs::mutating_error`）；但 `skills:*` 与 `prompt:load-global` 的 diagnostics 按基线**不带**前缀。
- 定稿不可逆：`finalization:` 相关改动一律 **Ask first**。
- 兜底约定：新增命令入参结构体必须带 `#[serde(rename_all = "camelCase")]`（`pi-development.md` §2.5 + 源码扫描测试）。
- ⚠️ **新增红线（本次，来源 B18/D1–D8）**：`src/import/` 的 8 项刻意偏离**不得在未改动基线对照的情况下静默去除**；
  尤其 D1（无密钥 sha256）与 D2（无 `webContentsId`）是**安全相关**决定，恢复需用户确认。
- 🚫 **导入路径安全**：来源绝对路径**永久禁止**回传渲染层；检视存储只暴露 `ImportInspectionSummary`（令牌 + 展示事实 + 前 8 章预览）。
- **更新域约定**：`update:*` 后端恒为 GitHub-Release 只读元数据（B14 前），`updateAction` 恒 `open-release`；不得伪造「已下载/已安装」，`defer-reminder` 写入全局配置时**必须保留**用户其他设置。
- 提交消息**无 BOM / 无 CRLF / 无行尾空白**（2026-10-08 出过 BOM 事故）。

---

### 5. 自检记录（2026-10-10 实测）

> 生成本表快照时在本机实跑，命令与输出如下。**下次更新快照表必须先重跑这些命令。**

| 命令 | 工作目录 | 实测输出 |
|---|---|---|
| `node scripts/verify-channel-coverage.mjs --quiet` | `tauri-app/` | 契约 invoke 频道 **193**（事件频道 **5**）· 已注册命令 **185** → 覆盖 **184** · 未迁移 **9** `[db=0 mcp=9]` · 命令名与契约频道一一对应 ✅ |
| `cargo test --lib` | `tauri-app/src-tauri/` | `test result: ok. 655 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 47.91s`（第四十一次重测） |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | `Finished dev profile ... `（**0 告警**） |
| `cargo fmt --check` | `tauri-app/src-tauri/` | 输出 **0 行**（干净） |
| `pnpm typecheck` / `pnpm run lint` | `tauri-app/` | exit 0 / exit 0 |
| `npx vitest run test/channel-migration-coverage.test.ts test/ipc-arg-struct-contract.test.ts` | `tauri-app/` | `Test Files 2 passed`，`Tests 7 passed` |
| `git status --porcelain` | 仓库根 | 跟踪文件改动：仅本快照 + 并行子任务项（`docs-fork/plans/tauri-migration-channel-inventory.md`、`.gitignore` 等，非 Task E-1 改动）；未跟踪：`docs-fork/todo.md`（按约定永不入库）+ 并行子任务产出 |
| `git log -1` | 仓库根 | `bdd9d3c3 docs(tauri): 新增 H4（mcp 9 频道）开工清单` |
| `pnpm tauri dev`（第十轮冒烟，G1） | `tauri-app/` | VITE `ready in 441 ms` · cargo `Finished dev profile in 47.50s` · `lorekeeper.exe` **90 MB** · 3 项人工验证全部 ✅ |
| `pnpm tauri dev`（第十一轮冒烟，G2a） | `tauri-app/` | VITE `ready in 812 ms` · cargo `Finished dev profile in 42.46s` · `lorekeeper.exe` **45 MB** · 4 项人工验证全部 ✅（唯一 console.error 为预期的 G2b 频道未迁移） |
| `pnpm tauri dev`（第十二轮冒烟，ESC/窗口修复） | `tauri-app/` | VITE `ready` · cargo 增量重建 · `lorekeeper.exe` **32 MB** · 4 项人工验证全部 ✅（窗口最小/最大化、标题栏拖拽、设置弹窗 ESC、关闭按钮） |

