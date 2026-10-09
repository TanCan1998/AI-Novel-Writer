# H3/H4 依赖评估（批次 H 剩余）

> 2026-10-09，批次 H 开工前评估。结论先行：**H3 与 H4 都可以零新依赖落地**，前提是沿用基线自身的架构
> （可替换的更新后端；MCP 仅 stdio）；需要单独决策的不是「要不要加依赖」，而是
> **「打开外部链接」能力**（新阻塞项 B12）与 **MCP 子进程的信任模型**。

## 0. 结论摘要

| 项 | 基线实现 | 新依赖？ | 建议 |
|---|---|---|---|
| **H3 update（6 频道 + `update:state` 事件）** | `update-service.ts`（585 行状态机）+ 可替换后端：`electron-updater-adapter`（Windows 已安装包） / `github-release-update-backend`（其余平台，仅元数据）；`update-preferences-store`（45 行）；`update-runtime`（平台门禁） | **不需要**（走 GitHub Release 后端） | 本轮只移植 GitHub-Release 后端 + 状态机 + 偏好 + 门禁；**真正的 Windows 自动更新（下载＋安装）另立专项**（需 `tauri-plugin-updater` + 签名公钥 + 打包链路） |
| **H4 mcp（9 频道）** | `electron/mcp/mcp-manager.ts`（489 行，手写 JSON-RPC）；**仅 stdio**，SSE 明确未实现（`throw new Error('SSE 传输暂未实现')`） | **不需要**（`std::process::Command` + 读线程） | 用 `std::process` + 自研守卫（argv 直传、不做 shell 展开）；`tauri-plugin-shell` 的静态 scope 模型不适合「配置驱动的动态命令」 |
| **B12 打开外部链接（新）** | `official-homepage:open` / `model-provider-resource:open` 现为**假成功占位**（返回 `success:true` 但不打开任何 URL） | **需要**（除非自研 spawn） | 推荐 `tauri-plugin-opener`（一次性修好 3 处：official-homepage / model-provider-resource / update:open-release） |

## 1. H3：update

### 1.1 基线架构（可替换后端是关键）

```
electron/main.ts:256   平台选择后端
  ├─ isWindowsUpdateRuntimeEnabled() → createElectronUpdaterBackend()   （win32 + 已打包 + 无 devServerUrl）
  └─ 否则                            → createGitHubReleaseUpdateBackend()
```

- `update-runtime.ts`：`isWindowsUpdateRuntimeEnabled` = `win32 && isPackaged && !devServerUrl`；
  `isMacUpdateReminderEnabled` = `darwin && isPackaged`；`hasWindowsUpdateConfiguration` 检查
  `resources/app-update.yml`（Electron Builder 产物；缺失时**不得**把配置缺失误判为网络故障）。
- `github-release-update-backend.ts`（49 行）：GET
  `https://api.github.com/repos/EthanYoQ/AI-Novel-Writer/releases/latest`（Accept: `application/vnd.github+json`，
  `X-GitHub-Api-Version: 2022-11-28`）→ `{ updateInfo: { version, releaseName?, releaseNotes?, releaseDate? } }`；
  `downloadUpdate()` 返回 `[]`，`quitAndInstall()` 为 **no-op**（**基线的 macOS/非打包策略就是「不自动安装」**）。
- `update-service.ts` 提供 6 个频道语义 + 版本比较 + 结构化错误分类
  （`http-forbidden/not-found/rate-limited`、`proxy`、`tls`、`network`、`unknown`，含 `retryable` 与
  脱敏 `safeTechnicalDetails`）+ 状态机事件 `update:state`。
- 偏好存储：`update-preferences-store.ts`（延迟提醒等，app-local JSON）。

### 1.2 Tauri 选项对比

| 方案 | 新依赖 | 能力 | 代价/风险 |
|---|---|---|---|
| **(a) 移植 GitHub-Release 后端 + 状态机（推荐）** | 无（复用 `reqwest`） | `update:check` 真实查询 GitHub Releases；`update:download` 对齐基线返回「无自动下载」；`update:quit-and-install` 对齐基线的平台策略（macOS：打开 Release 页；Windows 已安装包：本轮不支持） | 无真正的静默自更新；需把「不支持」的文案/状态做**诚实**（不得假成功） |
| (b) `tauri-plugin-updater` | `tauri-plugin-updater` + 签名公钥 + `tauri.conf.json` 配置 | 真正的 Windows 下载+安装 | 需发布签名密钥（本机无）；开发态无法验证；与基线 GitHub-Release 策略分叉，需重新对齐 6 频道语义 |

### 1.3 工作量

状态机 + 版本比较 + 错误分类是主要工作量（基线 585 行）；频道层只是信封转换。
**建议**：先落地 (a) 的 `update:check` / `update:get-state` / `update:defer-reminder` / `update:open-release`，
`update:download` / `update:quit-and-install` 按基线后端语义（返回空数组 / no-op 或平台策略）实现并明确记录。

## 2. H4：mcp

### 2.1 基线事实

- `mcp-manager.ts`：手写 JSON-RPC（无 `@modelcontextprotocol/sdk` —— `package.json` 中**没有**该依赖）。
- 传输：**仅 stdio**（`connect(serverId)` 中 `if (config.transport === 'stdio') ... else throw new Error('SSE 传输暂未实现')`）。
  因此 **不需要 SSE 客户端**，也不需要额外的 HTTP 流式解析。
- 子进程：`spawn(command, args, { stdio: ['pipe','pipe','pipe'] })`；配置文件位于 VELA_HOME 下。
- 频道：`load-config` / `connect` / `disconnect` / `disconnect-all` / `list-tools` / `list-resources` /
  `call-tool` / `get-servers-status` / `get-config-path`（全局域，不注入项目会话）。

### 2.2 子进程信任模型（需决策）

MCP 配置来自用户文件且允许指定任意 `command` + `args`，等价于「用户明确配置的可执行文件白名单」。

| 方案 | 新依赖 | 评价 |
|---|---|---|
| **(a) `std::process::Command` + 自研守卫（推荐）** | 无 | argv 直传（不经 shell）、`command` 非空、环境变量受控合并、子进程句柄集中管理；语义最接近基线 `spawn(command, args)` |
| (b) `tauri-plugin-shell` | `tauri-plugin-shell` | 其 scope 模型要求**静态**允许的命令，而 MCP 的 `command` 来自用户配置 → 需要宽放（`shell:allow-execute` 全放开）或每次动态放行，安全收益反而不明显，且长驻双向管道不如 `std::process` 直接 |

### 2.3 工作量

管理器移植（489 行）+ 9 频道信封 + 生命周期/状态机（connect/disconnect/status）+ 单测（可在 CI 用一个
本机可执行的假 MCP server，如 `node -e` 或 Rust 侧自带 echo 子进程）。**中high**。

## 3. B12（新阻塞项）：打开外部链接

| 频道 | 现状 |
|---|---|
| `official-homepage:open` | `commands/official_homepage.rs`：返回 `Ok({success:true})`，**不打开任何 URL**（注释 `TODO: 实现打开官方主页逻辑`） |
| `model-provider-resource:open` | `commands/model_provider_resource.rs`：同上（`TODO: 实现打开模型资源逻辑（webbrowser/shell-opener）`） |
| `update:open-release`（H3 需要） | 尚未实现 |

**性质**：这两个频道**报告成功却无副作用** —— 比「诚实化占位」更隐蔽（用户点「官方主页」只会看到什么都没发生，
且界面认为成功）。它们此前**未登记**为骨架（记录里只有 `dialog:select-export-directory`）。

**建议**：引入 `tauri-plugin-opener`（官方插件，依赖很小），一次性修好三处；并在 `capabilities/default.json`
里只放行固定/白名单 URL 或限定为「用户触发的外部链接」。若坚持零新依赖，可自研
`cmd /c start` / `open` / `xdg-open` spawn，但需自行处理 URL 校验与平台差异，收益不大。

## 4. 待用户确认的问题

1. **H3**：走 (a) 零依赖 GitHub-Release 版（推荐）还是 (b) 引入 `tauri-plugin-updater`（需签名密钥）？
2. **B12**：用 `tauri-plugin-opener`（推荐，一次修好 3 处）还是自研 spawn？
3. **H4**：子进程用 (a) `std::process` + 自研守卫（推荐）还是 (b) `tauri-plugin-shell`？
