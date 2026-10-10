# H4 开工清单：`mcp`（9 频道，仅 stdio 传输）

> 2026-10-10 产出（开工前调研，**只写文档、未写任何代码**）。
> 基线评估：[`2026-10-09-h3-h4-dependency-evaluation.md`](../research/2026-10-09-h3-h4-dependency-evaluation.md) §2（H4 基线事实）/ §2.2（信任模型 (a)/(b)）/ §2.3（工作量）/ §4 第 3 条（待确认问题）。
> ⚠️ **未决项：§4 信任模型 (a)/(b) 需用户确认后才能开工**——本清单不做最终决定、不标记「已批准」。
> 本清单目标：下次会话可以**不再调研**，直接按 §7 的任务队列实现。

## 1. 目标与范围

| 项 | 值 |
|---|---|
| 频道 | `mcp:load-config` / `mcp:connect` / `mcp:disconnect` / `mcp:disconnect-all` / `mcp:list-tools` / `mcp:list-resources` / `mcp:call-tool` / `mcp:get-servers-status` / `mcp:get-config-path`（9 个，**当前仅剩的全部未迁移频道**） |
| 域 | 全部为**全局域**：无 `expectedProjectPath`、不注入项目会话（对照 `project_access` / `ProjectSession`） |
| 传输 | **仅 stdio**（基线 `mcp-manager.ts:228`：非 stdio → `throw new Error('SSE 传输暂未实现')`）；**SSE 不在本次范围** |
| 预期指标变化 | 未迁移 invoke **9 → 0**；已注册命令 **185 → 194**；覆盖 invoke **184 → 193**；`mcp=9` 从未迁移清单消失 |
| 不做什么 | 不实现 SSE；不引入 `@modelcontextprotocol/sdk`（基线手写 JSON-RPC，`package.json` 无该依赖）；不引入 `tauri-plugin-shell`（除非用户在 §4 选 (b)）；不改契约镜像（`src/shared/ipc-channels.ts`、`tauri-app/src/shared/ipc-channels.ts` 只读）；不改 `tauri.conf.json`；不新增事件频道（基线无 MCP 事件推送，见 §3） |

## 2. 契约与 9 频道交付清单表（逐条对齐，勿改名）

`MCPChannels`（上游 `src/shared/ipc-channels.ts:1138-1149`；Tauri 镜像 `tauri-app/src/shared/ipc-channels.ts:1184-1195`，逐字一致）：

| 频道 | args 要点 | return 要点 | 域 | 状态 / 数据来源 |
|---|---|---|---|---|
| `mcp:load-config` | `[]` | `MCPConfigLoadResponse`：`{success:true, status:'missing', servers:[]}` / `{success:true, status:'loaded', servers:[{id,name,transport}]}` / `{success:false, status:'error', servers:[], error}` | 全局 | 读配置文件；基线 `loadConfig()`（`mcp-manager.ts:127-203`） |
| `mcp:connect` | `[serverId: string]` | `{success: boolean; error?: string}`；catch 一律 `'MCP 服务器连接失败'`（**内部细节不透传**） | 全局 | `loadedConfigs` → spawn stdio 子进程 → initialize + discover（`mcp-manager.ts:205-251`） |
| `mcp:disconnect` | `[serverId: string]` | `{success: boolean; error?: string}`；catch 为 `String(error)` | 全局 | `servers` 运行时表（`:438-449`） |
| `mcp:disconnect-all` | `[]` | 同上 | 全局 | `servers` 运行时表（`:451-456`） |
| `mcp:list-tools` | `[]` | `MCPToolDescription[]`（**无信封**） | 全局 | 已连接（`status==='connected'`）服务器的 `runtime.tools`（`tools/list` 发现结果，`:458-467`） |
| `mcp:list-resources` | `[]` | `MCPResourceDescription[]`（**无信封**） | 全局 | 已连接服务器的 `runtime.resources`（`:469-478`） |
| `mcp:call-tool` | `[serverId: string, toolName: string, args: Record<string, unknown>]` | `{success: boolean; content: string; error?: string}`（bridge 无 try/catch，`callTool` 自身不抛） | 全局 | JSON-RPC `tools/call`；`content` = 结果中 `type==='text'` 片段的 `text` 以 `'\n'` 连接（`:410-437`） |
| `mcp:get-servers-status` | `[]` | `MCPServerStatus[]`（`{id, name, status, toolCount, error?}`，**无信封**） | 全局 | `servers` 运行时表（`:480-493`）；渲染层**轮询**此频道（无事件推送） |
| `mcp:get-config-path` | `[]` | `string`（默认配置文件路径，**无信封**） | 全局 | `lorekeeper_home()/mcp_config.json`（Tauri；基线为 `VELA_HOME/mcp_config.json`，见 D-H4-1） |

类型全集（`src/shared/ipc-channels.ts:1096-1151`，Tauri 镜像 `:1142-1195`）：`MCPConnectionStatus`（`'disconnected' | 'connecting' | 'connected' | 'error'`，:1097）/ `MCPServerSummary`（:1099）/ `MCPServerStatus`（:1105）/ `MCPToolDescription`（:1113）/ `MCPResourceDescription`（:1120）/ `MCPConfigLoadResult`（:1128）/ `MCPConfigLoadResponse`（:1133）。Rust 侧按 camelCase 逐字段镜像。

**无 MCP 事件频道**：契约事件频道仅 4 个（`AllEventChannels`，`:1198`：`LLMStreamEvents` + `UpdateStateEvents` + `WindowEvents`）。基线 `MCPManagerImpl.setCallbacks`（`mcp-manager.ts:110`）虽定义了 `onStatusChange`/`onToolsChange`，但**全仓库无调用点**（grep 仅命中定义本身）→ 基线实际靠 `mcp:get-servers-status` 轮询（`src/stores/mcp-store.ts:102-109` `refreshStatus`）。Tauri 侧**不需要**新增事件频道。

## 3. 基线事实源清单（以下行号均为本次实际打开文件核对）

### 3.1 Electron 基线

| 文件 | 行数 | 关键函数 / 类型（已核对行号） | 本批处理 |
|---|---|---|---|
| `electron/mcp/mcp-manager.ts` | 500（研究文档记「约 489」，实测 500） | `MCPServerConfig`（:30）/ `MCPConfig`（:48，Claude Desktop 格式）/ `MCPToolDesc`（:58）/ `MCPResourceDesc`（:70）/ `MCPServerRuntime`（:80）；`class MCPManagerImpl`（:100）；`setCallbacks`（:110，无调用点）；`getDefaultConfigPath`（:119，`join(VELA_HOME, 'mcp_config.json')`）；`loadConfig`（:127-203）；`connect`（:205-251）；`connectStdio`（:255-290）；`processBuffer`（:301-318）；`handleMessage`（:320-336）；`sendRequest`（:338-361）；`initializeSession`（:363-380）；`discoverTools`（:382-391）；`discoverResources`（:395-404）；`callTool`（:410-437）；`disconnect`（:438-449）；`disconnectAll`（:451-456）；`getAllTools`（:458-467）；`getAllResources`（:469-478）；`getServersStatus`（:480-493）；单例 `mcpManager`（:500） | **核心：完整移植** |
| `electron/mcp/mcp-ipc-bridge.ts` | 87 | `registerMCPHandlers()` 注册 9 个 `ipcMain.handle`：`mcp:load-config`（:17）/ `mcp:connect`（:33）/ `mcp:disconnect`（:43）/ `mcp:disconnect-all`（:53）/ `mcp:list-tools`（:63）/ `mcp:list-resources`（:68）/ `mcp:call-tool`（:73）/ `mcp:get-servers-status`（:78）/ `mcp:get-config-path`（:83） | **频道映射 + 错误信封范本**（逐字对齐） |
| `electron/main.ts` | — | `import { registerMCPHandlers }`（:3）；`registerMCPHandlers()`（:239） | 装配点参考 |
| `electron/utils/config-utils.ts` | — | `VELA_HOME`（:7-8）= `process.env.AI_NOVEL_VELA_HOME?.trim() \|\| path.join(os.homedir(), '.vela')` | **仅参考**：Tauri 侧必须改道 `lorekeeper_home()`（D-H4-1） |
| `electron/mcp/__tests__/mcp-manager.test.ts` | — | `vi.mock('child_process')` + EventEmitter 假进程（`initializeError` / `failToStart` / `neverRespond` 三种故障注入）；临时目录写 `mcp_config.json` | 单测范式参考 |

关键语义（实现时必须保留）：

1. **仅 stdio**：`connect` 中 `config.transport === 'stdio'` 走 `connectStdio`，否则 `throw new Error('SSE 传输暂未实现')`（:228）——但经 bridge catch 后，渲染层看到的统一是 `{success:false, error:'MCP 服务器连接失败'}`（`mcp-ipc-bridge.ts:33-41`）。
2. **spawn 参数**（:261-266）：`spawn(command, args, { env: { ...process.env, ...env }, stdio: ['pipe','pipe','pipe'], windowsHide: true })` —— **argv 直传、不经 shell**。
3. **JSON-RPC 帧**：`\n` 分隔；请求体 `{jsonrpc:'2.0', id, method, params: params ?? {}}` 写 stdin（:338-361）；**10 秒超时** → reject `MCP 请求超时: ${method}`（:352-358）；响应按 `id` 匹配 pending 表，`error` → `err?.message ?? 'MCP error'`（:320-336）；非 JSON 行忽略（:301-318）。
4. **初始化**：`initialize` 请求 `{protocolVersion:'2024-11-05', capabilities:{}, clientInfo:{name:'vela', version:'1.0.0'}}`，随后写 `notifications/initialized` 通知（:363-380）。
5. **发现**：`tools/list` / `resources/list` 失败 → 空数组，**不报错**（:382-404）。
6. **callTool**：未连接 → `{success:false, content:'', error:'服务器 ${serverId} 未连接'}`；成功 → `content` = `type==='text'` 片段 `join('\n')`；catch → `{success:false, content:'', error: String(error)}`（:410-437）。
7. **loadConfig 校验链**（:127-203）：结构/字段类型校验失败或 `command`/`url` 同有同缺（`hasCommand === hasUrl`）→ 先 `disconnectAll()` 再返回 `{status:'error', servers:[], error:'MCP 配置损坏，未加载任何服务器'}`（revoke 语义）；ENOENT → `{status:'missing', servers:[]}`；其余读取异常 → `'MCP 配置损坏或无法读取，未加载任何服务器'`；已连接服务器与新配置做 `isDeepStrictEqual` 深度比较，不一致则 disconnect 旧服务器。
8. **无事件推送**：见 §2 末段（`setCallbacks` 无调用点）。

### 3.2 Tauri 侧现状

| 文件 | 行号 | 现状 |
|---|---|---|
| `tauri-app/src/shared/ipc-channels.ts` | :1142-1195 | `MCPChannels` 等类型已就位（与上游逐字镜像），**只读不改** |
| `tauri-app/src/services/ipc-client.ts` | :230-231 | 参数登记范式 `'update:defer-reminder': ['days']`；**当前无 mcp 条目**，需补 `mcp:connect` / `mcp:disconnect` / `mcp:call-tool` 三条 |
| `tauri-app/src/shared/migrated-channels.ts` | — | 当前 0 个 mcp 条目；迁移后 `node scripts/verify-channel-coverage.mjs --emit` 重新生成（脚本 :15-16 / :31 / :74 / :146） |
| `tauri-app/src/stores/mcp-store.ts` | — | 已就位（上游镜像）：消费全部 9 频道（:66 / :70 / :79 / :104 / :112 / :123 / :132 / :139-140 / :168），含 `refreshStatus` 轮询（:102-109）；**无需改动** |
| `tauri-app/src-tauri/src/commands/` | — | **无 `mcp.rs`**（需新建）；`commands/mod.rs` 提供 `pub mod` + `pub use` 注册范式与 `SimpleResult`；`lib.rs:89` `generate_handler![...]` |
| `tauri-app/src-tauri/src/app_paths.rs` | :34-40 | `lorekeeper_home()` = `AI_NOVEL_LOREKEEPER_HOME` 或 `~/.lorekeeper`（`LOREKEEPER_HOME_DIR`） |
| `tauri-app/src-tauri/src/json_store.rs` | :49 / :82 | `read_json_value_or(path, fallback)` / `write_json_file(path, data)` |
| `tauri-app/src-tauri/src/state.rs` | :35 / :72 / :127 | `AppState`；`update: Mutex<Option<Arc<UpdateService>>>` + `update_service()` —— MCPManager 照此范式装配 |
| `tauri-app/src-tauri/src/update/` | — | H3 模块范式（`backend` / `preferences` / `runtime` / `service` / `startup` / `time` / `types`） |
| `tauri-app/src-tauri/capabilities/default.json` | — | `permissions: ["core:default"]`（无 shell 权限；文件描述要求「按批次追加最小权限，禁止一次性放开」） |
| `tauri-app/scripts/verify-channel-coverage.mjs` | — | 当前实测输出：`契约 invoke 频道 193（事件频道 4）` / `已注册命令 185 → 覆盖 invoke 频道 184` / `未迁移 invoke 频道 9  [mcp=9]` / 9 行 `mcp:*` 清单 / `命令名与契约频道一一对应 ✅` |

## 4. Ask-first 决策项：MCP 子进程信任模型 (a) vs (b)

**此项需用户确认后才能开工**（对应研究文档 §4 待确认问题第 3 条；本清单不做最终决定）。

背景：MCP 配置来自用户文件且允许指定任意 `command` + `args`，等价于「用户明确配置的可执行文件白名单」（研究文档 §2.2）。

| 维度 | (a) `std::process::Command` + 自研守卫（研究文档 §2.2 推荐） | (b) `tauri-plugin-shell` |
|---|---|---|
| 新依赖 | **无**（符合硬约束「禁止新增任何依赖」） | **有**（`tauri-plugin-shell`，违反硬约束，需用户特批） |
| 命令执行 | argv 直传（`Command::new(command).args(args)`），不经 shell，无 shell 注入面 | 经插件 execute；其 scope 模型要求**静态**允许的命令 |
| 与「`command` 来自用户配置」的匹配 | 天然匹配（动态命令即任意 argv） | 需 `shell:allow-execute` 全放开（或每次动态放行），**安全收益不明显** |
| 长驻双向管道 | `Child` + `Stdio::piped()` 直接持有，读线程 / 写句柄集中管理 | 插件面向短命令，长驻双向管道不如 `std::process` 直接 |
| `capabilities/default.json` | 无需改动（`core:default` 已够） | 需追加 shell 权限（与 capabilities「按批次追加最小权限、禁止一次性放开」的描述冲突） |
| 自研工作量 | 需自研：进程退出监听（exit → `disconnected`）、stdout 读线程 + `\n` 分帧、10s 超时、pending 请求表、Windows `windowsHide` 等价物（`CREATE_NO_WINDOW` creation flag） | 插件承担部分生命周期，但 scope / 权限成本抵消收益 |

与「本仓库既有约束」对照：① 硬约束「禁止新增任何依赖」→ (a) 直接满足，(b) 需破例；② `capabilities/default.json` 现状 `["core:default"]` → (a) 无需触碰，(b) 需扩权；③ 基线语义 `spawn(command, args)` → (a) 一一对应。

**倾向**：(a)（与研究文档 §0 / §2.2 一致）。**但按 Task 要求，此项不做最终决定——需用户确认 (a)/(b) 后才能开工。**

## 5. 刻意偏离登记（预登记，开工时逐条确认）

- **D-H4-1（必须，架构性）**：MCP 配置文件根由基线 `VELA_HOME`（`AI_NOVEL_VELA_HOME` / `~/.vela`，`config-utils.ts:7-8`）改道为 `lorekeeper_home()`（`AI_NOVEL_LOREKEEPER_HOME` / `~/.lorekeeper`，`app_paths.rs:34-40`）。基线 `mcp-manager.ts:119-121` 用 `VELA_HOME` 属 Electron 遗留。**Tauri 侧绝不能**落到 `~/.vela` / `.vela` / `AI_NOVEL_VELA_HOME`。
- **D-H4-2（待用户确认）**：`initializeSession` 的 `clientInfo.name` 基线硬编码 `'vela'`（`mcp-manager.ts:363-380`）。建议照抄基线（协议层 server 通常不校验 clientInfo）；若改 `'lorekeeper'` 属协议可见行为变更，需登记。
- **D-H4-3（实现时确认）**：基线 `windowsHide: true`（:264）→ Windows 下 `Command::creation_flags(CREATE_NO_WINDOW)`，Unix 无等价物（无需处理）。
- 其余**照抄基线、无偏离**：全部错误文案（`MCP 配置损坏，未加载任何服务器` / `MCP 配置损坏或无法读取，未加载任何服务器` / `MCP 配置加载失败` / `MCP 服务器连接失败` / `MCP 服务器未配置或配置尚未加载` / `stdio 模式需要 command 参数` / `服务器 <id> 未连接` / `MCP 请求超时: <method>` / `连接已断开` / `MCP error`）、10s 超时、`protocolVersion: '2024-11-05'`、`\n` 分帧、`{jsonrpc:'2.0', id, method, params: params ?? {}}` 请求体。

## 6. 风险与未决问题

| # | 项 | 说明 |
|---|---|---|
| 1 | **配置文件位置（红线）** | Tauri 侧必须是 `lorekeeper_home().join("mcp_config.json")`。基线在 `VELA_HOME` 下（`mcp-manager.ts:119-121`），迁移时必须改道；任何测试 / 迁移代码都不得读写 `~/.vela` / `.vela` / `AI_NOVEL_VELA_HOME`。 |
| 2 | **子进程生命周期与关窗清理** | Rust `Child` drop 时只关 stdin、不 kill 子进程 → 需在管理器显式 `disconnect_all`（kill 全部子进程，对齐基线 :451-456）并挂到应用退出 / 窗口销毁路径（`DestroyEvent` 或 `lib.rs` 的清理钩子），否则关窗后 MCP 子进程泄漏。 |
| 3 | **`mcp:call-tool` 取消 / 超时语义** | 基线仅有 10s 请求超时（:352-358），**无取消语义**（无 JSON-RPC cancel 通知）。Tauri 侧是否支持前端 abort → 待确认；最低限度保留 10s 超时对齐基线。 |
| 4 | **单测如何造假 MCP server** | 基线用 `vi.mock('child_process')`（EventEmitter 假进程）；Rust 侧建议用**真实子进程**：`node -e` echo 脚本（按 `\n` 回显 JSON-RPC 响应）或 Rust 测试自带 echo 子进程，配合临时 `AI_NOVEL_LOREKEEPER_HOME` 写 `mcp_config.json`。需确认 CI 环境有 `node` 可用（或用平台等价物）。 |
| 5 | **错误信封不透传** | 基线 bridge 的 catch 会把所有内部 throw（含 `SSE 传输暂未实现`、`MCP 服务器未配置或配置尚未加载`）统一转成 `mcp:connect` 的 `{success:false, error:'MCP 服务器连接失败'}`（`mcp-ipc-bridge.ts:33-41`）——Tauri 侧须逐字对齐，**不得**把内部错误细节透传给渲染层。 |
| 6 | **SSE 条目可见但不可连** | 配置中只有 `url` 的条目（`transport:'sse'`）仍会被 `load-config` 列出，但 `connect` 必然失败（对齐基线 :228）。Tauri 侧保持该行为，不实现 SSE。 |
| 7 | **环境变量合并** | 基线 `{ ...process.env, ...env }`（:262）→ Rust 侧 `Command` 继承 `std::env` 再 `envs(user_env)` 覆盖；注意 `std::env` 与 Electron `process.env` 在 Windows 下的差异（PATH 等），实现时用注入式 env 便于测试。 |
| 8 | **并发与锁** | `connect` 内部「已连接则先 disconnect 再重连」（:209-211）；Tauri 侧 Manager 用 `Mutex` 保护 `servers` / `loaded_configs`，避免 connect / disconnect 并发竞态。 |
| 9 | **配置比对** | `isDeepStrictEqual`（:186-191）→ Rust 侧用 `serde_json::Value` 相等比较（或为配置结构派生 `PartialEq`）。 |

## 7. 建议的开工任务队列（Task 1..7）

**Task 1：类型镜像 + 管理器骨架 + AppState 装配**
- **Target**：`tauri-app/src-tauri/src/mcp/mod.rs`、`tauri-app/src-tauri/src/mcp/types.rs`（新增）；`tauri-app/src-tauri/src/state.rs`（加 `mcp: Mutex<Option<Arc<McpManager>>>` + getter，对齐 `update_service()` :127 范式）；`tauri-app/src-tauri/src/commands/mod.rs`（`pub mod mcp` + `pub use mcp::*`）；`tauri-app/src-tauri/src/lib.rs`（`generate_handler!` 追加 9 命令）
- **Prompt 要点**：镜像 §2 全部类型（`#[serde(rename_all = "camelCase")]`，字段逐字对齐 `tauri-app/src/shared/ipc-channels.ts:1142-1195`）；管理器持有 `servers` / `loaded_configs` 两张表（对齐 `mcp-manager.ts:100-117`）；本卡**不实现传输**（Task 3 落地）
- **自检**：`cargo check --all-targets`（0 告警）

**Task 2：配置读写 + `mcp:load-config` / `mcp:get-config-path`**
- **Target**：`tauri-app/src-tauri/src/mcp/config.rs`、`tauri-app/src-tauri/src/commands/mcp.rs`
- **Prompt 要点**：路径 = `app_paths::lorekeeper_home().join("mcp_config.json")`（**红线 D-H4-1**）；`read_json_value_or` 读取；校验链逐条对齐基线 `loadConfig`（:127-203）：结构 / 字段类型校验、`command`/`url` 互斥（`hasCommand === hasUrl` 拒绝）、损坏 → 先 `disconnect_all()` 再回 `{status:'error', error:'MCP 配置损坏，未加载任何服务器'}`、ENOENT → `{status:'missing', servers:[]}`、其余读取失败 → `'MCP 配置损坏或无法读取，未加载任何服务器'`；bridge 外层 catch → `{success:false, status:'error', servers:[], error:'MCP 配置加载失败'}`（`mcp-ipc-bridge.ts:17-30`）
- **自检**：`cargo test --lib mcp`

**Task 3：stdio 传输层（信任模型落点，依赖 §4 决策）**
- **Target**：`tauri-app/src-tauri/src/mcp/transport.rs`
- **Prompt 要点**：按用户确认的 (a)/(b) 实现；(a) 时：`command` 非空校验（对齐 :258 `'stdio 模式需要 command 参数'`）、`Command::new(command).args(args)` argv 直传、env 受控合并（继承 + 用户覆盖，对齐 :262）、`Child` 句柄集中管理；stdout 读线程按 `\n` 分帧（`processBuffer` :301-318）；请求 `{jsonrpc:'2.0', id, method, params}` + **10s 超时** `MCP 请求超时: <method>`（:338-361）；pending 表按 id 匹配（error → `err.message ?? 'MCP error'`，:320-336）；进程退出 → `disconnected`、spawn 失败 → `error`（:270-289）
- **自检**：`cargo test --lib mcp::transport`（假 MCP server：`node -e` echo 或自带 echo 子进程）

**Task 4：连接状态机 + `mcp:connect` / `mcp:disconnect` / `mcp:disconnect-all` / `mcp:get-servers-status`**
- **Target**：`tauri-app/src-tauri/src/mcp/manager.rs`、`tauri-app/src-tauri/src/commands/mcp.rs`
- **Prompt 要点**：`connect` = 查 `loaded_configs`（未命中 → 内部 `'MCP 服务器未配置或配置尚未加载'`，经信封统一为 `'MCP 服务器连接失败'`）→ 已连接先 disconnect → `connecting` → `initialize`（`protocolVersion:'2024-11-05'`、`clientInfo` 见 D-H4-2）+ `notifications/initialized` → `tools/list` / `resources/list`（失败 → 空）→ `connected`；catch → kill + reject pending（`'MCP 服务器连接失败'`）+ `error` 态 + **bridge 信封 `{success:false, error:'MCP 服务器连接失败'}`**；`disconnect` = kill + reject pending（`'连接已断开'`）+ 删表；`get_servers_status` 返回 `{id, name, status, toolCount, error}`（:480-493）；**无事件推送**（渲染层轮询 `get-servers-status`）
- **自检**：`cargo test --lib mcp`

**Task 5：`mcp:call-tool` / `mcp:list-tools` / `mcp:list-resources`**
- **Target**：`tauri-app/src-tauri/src/commands/mcp.rs`
- **Prompt 要点**：`call_tool` 未连接 → `{success:false, content:'', error:'服务器 <id> 未连接'}`；`tools/call {name, arguments}`；返回 `content` = `type==='text'` 片段的 `text` 以 `'\n'` 连接（:410-437）；`list_tools` / `list_resources` 只聚合 `status==='connected'` 服务器的发现结果（:458-478）
- **自检**：`cargo test --lib mcp`

**Task 6：前端登记**
- **Target**：`tauri-app/src/services/ipc-client.ts`（参数登记：`'mcp:connect': ['serverId']`、`'mcp:disconnect': ['serverId']`、`'mcp:call-tool': ['serverId', 'toolName', 'args']`，对齐 :230-231 范式）；`tauri-app/src/shared/migrated-channels.ts`（`node scripts/verify-channel-coverage.mjs --emit` 重新生成）
- **Prompt 要点**：`tauri-app/src/stores/mcp-store.ts` 已就位、无需改动；核对 9 频道 invoke 参数顺序与契约一致
- **自检**：`pnpm typecheck`、`pnpm run lint`、`node scripts/verify-channel-coverage.mjs`（期望：未迁移 0、命令 194、覆盖 193）

**Task 7：单测补强 + 收口**
- **Target**：`tauri-app/src-tauri/src/mcp/*` 的 `#[cfg(test)]`
- **Prompt 要点**：配置解析三态（missing / loaded / error）、`loadConfig` 损坏即 `disconnect_all`、connect 状态机（成功 / 失败 / 重连）、call-tool 文本聚合、假 MCP server 子进程（`node -e` 或 echo）；目标 +10～20 测试
- **自检**：§8 全部收口命令

## 8. 收口自检命令

```bash
# tauri-app/
pnpm typecheck
pnpm run lint
node scripts/verify-channel-coverage.mjs
#   期望：契约 invoke 频道 193（事件频道 4）
#         已注册命令 194 → 覆盖 invoke 频道 193
#         未迁移 invoke 频道 0（mcp=9 从清单消失）
#         命令名与契约频道一一对应 ✅

# tauri-app/src-tauri/（先设 RUSTUP_HOME=D:\Environment\rust\rustup、CARGO_HOME=D:\Environment\rust\cargo）
cargo fmt --check
cargo check --all-targets   # 0 告警
cargo test --lib            # 652 + 新增（Task 7 目标 +10～20）
```

GUI 冒烟建议（可复用 `F:\Temp\loretest\lore-smoke` 夹具）：临时 `AI_NOVEL_LOREKEEPER_HOME` 下写 `mcp_config.json` 指向 `node -e` echo server，验证 `mcp:load-config` → `mcp:connect` → `mcp:list-tools` → `mcp:call-tool` 全链路；`mcp:get-config-path` 返回的路径必须在 `~/.lorekeeper`（或 env 指定根）下。
