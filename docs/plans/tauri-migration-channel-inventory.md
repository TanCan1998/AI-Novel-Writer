# Tauri 迁移 — IPC 契约盘点（阶段 1 产出）

> **用途**：Tauri 迁移的 channel → `#[tauri::command]` 映射骨架。类型细节以 [`src/shared/ipc-channels.ts`](../../src/shared/ipc-channels.ts) 为唯一事实源，本文只做结构盘点、模式标注与批次规划。
> **基线**：master @ `cb71878`（2026-10-06 盘点）。Electron 代码未改动。
> **重生成方式**：对 `src/shared/ipc-channels.ts` 与 `electron/controllers/*.ts` 重跑 Select-String 频道正则即可核对；本文为手工核对结果。

## 1. 总体统计

| 类别 | 数量 | 说明 |
|---|---|---|
| invoke 频道（请求/响应） | **193** | 渲染 → 主进程，`ipcMain.handle` |
| 事件频道（主 → 渲染推送） | **5** | `ipcRenderer.on`，迁移为 Tauri Event |
| 合计 | **198** | 每个频道对应一个 Rust 命令或事件 |

**前端调用面（重大发现）**：渲染进程对 `window.velaAPI` 的直接访问收敛在 **2 个文件**，其余业务代码全部经由封装层调用：

| 文件 | 角色 | 迁移动作 |
|---|---|---|
| `src/services/ipc-client.ts` | 主封装：类型安全 `invoke/on/once/send` + 项目会话注入 + zoom | 仅替换底层 `getAPI()` 实现（`@tauri-apps/api/core.invoke` / `event.listen`），上层零改动 |
| `src/services/finalization-client.ts` | 定稿专用封装（绕过 ipc-client） | 同上，仅替换 `getVelaApi()` |

`electron/preload.ts` 暴露面 = `invoke / on / once / send / setZoomLevel / setZoomFactor / getZoomLevel`（zoom 走 Electron `webFrame`，Tauri 侧需用 Webview zoom API 或 CSS 替代，归阶段 3）。

**迁移进度（截至 2026-10-06 第九次更新）**：已注册 Rust 命令 **41** 个（阶段 0 联调 1 + 批次 A 11 + 批次 B 22 + 批次 C `project_core` 4 + 批次 C `characters/roster` 3），覆盖 193 个 invoke 频道中的 **41** 个；批次 A–C 已提交（`ff7fbd8` + `a8d742a` + `69fc50c`），批次 C 剩余 db 子域待续。同时已完成 **Lorekeeper 身份与双栈隔离 L0/L1/L2**（见 §4.10 末「双栈隔离约束」）。逐批状态与接续入口见 [`docs/handoffs/2026-10-06-tauri-migration-status.md`](../handoffs/2026-10-06-tauri-migration-status.md)。

## 2. 必须知晓的传输层约定（Rust 签名要预留）

1. **项目会话注入**：`ipc-client.ts` 的 `invokeWithSession` 对项目域频道（`db:*`、`kb:*`、`chapter:*`、`fs:*`、`project:save|update-config|delete`）**自动在参数尾部追加 `projectSession`**（`ProjectSessionContext { projectId, leaseId, projectPath }`）。契约文件的 `args` 不包含它 —— **Rust 命令必须接受这个尾参**并校验租约（对应 ADR 0001 会话租约）。
2. **能力域不注入会话**：`fs:grant-*`、`dialog:select-*`、`prompt:*`、`skills:*` 属于授权/app-data 域，禁止借用项目会话。
3. **`expectedProjectPath` 防护参数**：几乎所有 `db:/fs:` 频道带此参数，主进程须做规范根目录校验。
4. **`AppResult<T>`**：`T | { success: false, errorCode: AppErrorCode, error? }`；`AppErrorCode` 共 6 种（KNOWLEDGE_BASE_NATIVE_UNAVAILABLE / LEGACY_VECTOR_MIGRATION_BLOCKED / PROJECT_NOT_OPEN / EMBEDDING_MODEL_NOT_CONFIGURED / PROJECT_STORAGE_PATH_UNSUPPORTED / PROJECT_ROOT_REQUIRED），Rust 侧建等价 enum。
5. **事件载荷为单对象**：事件频道回调收到的参数形如 `{ requestId, chunk }`，Tauri `emit` 时保持同构。
6. **参数命名映射（源码级核实，`tauri-macros 2.7.1`）**：Rust 命令参数名与前端 `invoke(name, obj)` 的 key 经 **`to_lower_camel_case()`** 对应 —— Rust `expected_project_path` ↔ JS `expectedProjectPath`（前导下划线如 `_project_id` 同样归并为 `projectId`）；`Option<T>` 参数在 key 缺失时注入 `None`（可省略不传）。因此契约 `args` 的位置参数在 Tauri 侧**一律改为命名参数**，由 `tauri-app/src/services/ipc-client.ts` 的 `CHANNEL_ARG_NAMES` 登记表维护；未登记的非空参频道会立即抛错（防静默错配）。

## 3. 契约缺口与异常点（迁移前须补齐/决策）

| # | 问题 | 详情 | 处理建议 |
|---|---|---|---|
| G1 | **`finalization:commit` / `finalization:retry` 未在 ipc-channels.ts 声明** | 仅 `electron/controllers/finalization-controller.ts`（注册）与 `src/services/finalization-client.ts`（调用：args = `(FinalizationSnapshot, ProjectSessionContext)` / `(finalizationId, ProjectSessionContext)`，return `FinalizationResult`，类型来自 `electron/services/finalization-service.ts`） | 迁移时补建 `FinalizationChannels` 接口；`FinalizationResult`/`FinalizationSnapshot` 类型随迁入 `src/shared/` |
| G2 | **src 直接 import electron 目录的类型** | `ipc-channels.ts` 与 `finalization-client.ts` `import type` 自 `electron/repositories/*`、`electron/services/*` | Tauri 迁移期间保留（TS 类型不产生运行时依赖）；Rust 侧按类型定义平移；长期可把纯类型下沉 `src/shared/`（Ask first） |
| G3 | **update-controller 注册位置特殊** | 不在 `ipc-handlers.ts`，在 `electron/main.ts:259` 注册，带 `publish: publishUpdateState` 回调（发布 `update:state` 事件） | 迁移时注意 update 命令与 update:state 事件同源于 update-service |
| G4 | **window:close-requested 是「请求-决策」两段式** | 主进程发 `window:close-requested { requestId }`，渲染端回 `window:resolve-close(requestId, 'proceed'|'cancel')` | Tauri 侧拦截 `tauri://close-requested`（`on_window_event` + `api.prevent_close()`），保留 requestId 决策协议 |
| G5 | **高频流式事件** | `llm:stream-chunk` 每 chunk 一发；Tauri event 经 webview 桥，性能需实测；必要时批量合并 chunk | 阶段 2 D 批次实测 |
| G6 | **MCP 频道无 expectedProjectPath / session** | `mcp:*` 9 个频道是全局的，不注入会话 | 按 app-data 域处理 |

## 4. 分域清单（invoke 193 个）

「批次」见 §6。★ = 有 §2/§3 特殊模式。

### 4.1 ConfigChannels（2）— controller: `config-controller.ts` — 批次 A
`config:get`、`config:set`。→ `config_get` / `config_set`。

### 4.2 UpdateChannels（6 + 事件 1）— controller: `update-controller.ts`（main.ts 注册，G3）— 批次 H
`update:get-state`、`update:check`、`update:download`、`update:open-release`、`update:defer-reminder(days)`、`update:quit-and-install`。
事件：`update:state → UpdateState`（迁移为 `tauri-plugin-updater` + `app.emit`）。macOS 语义：只打开 Release 页不下载。

### 4.3 SkinChannels（3）— controller: `skin-controller.ts` — 批次 A
`skin:get-state`、`skin:execute(command)`、`skin:read-custom-asset`。皮肤服务初始化失败须降级不阻断（ipc-handlers 启动语义）。

### 4.4 WindowChannels（4 + 事件 1）— controller: `window-controller.ts` — 批次 A
`window:minimize`、`window:toggle-maximize`、`window:close`、`window:resolve-close(requestId, decision)`★G4。
事件：`window:close-requested { requestId }`★G4。

### 4.5 OfficialHomepage / ModelProviderResource（1+1）— 批次 A
`official-homepage:open`、`model-provider-resource:open(resourceId)`（主进程映射固定 HTTPS URL，不接受任意 URL）。

### 4.6 ProjectChannels（10 + dialog 1）— controller: `project-controller.ts` — 批次 B
`project:get-runtime-context`、`project:create(config, requestToken, rendererProjectPath)`、`project:open(projectPath, requestToken, rendererProjectPath)`、`project:save`★、`project:update-config`★、`project:recent-list`、`project:recent-remove`、`project:delete`★（返回含 `directoryDeleted`/`databaseRestored`，须事务性恢复）、`project:smoke-open-request`、`project:smoke-open-confirm`、`dialog:select-folder`。
核心语义：requestToken / rendererProjectPath 防陈旧窗口写入；`sessionLease` 由主进程签发冻结。
**状态（2026-10-06 第八次更新）**：批次 B 已把这 10 频道接线完成（后 5 个当时为骨架）；批次 C 已将 `project:create/open/save/update-config/delete` **全部真实化**（项目清单探测/创建、rusqlite 打开 `.vela/vela.db`、租约签发、ProjectData 回传），已编译验证（`cargo test --lib` 64/64）并提交（`a8d742a`）。
**状态（2026-10-08 第二十五次更新）**：`dialog:select-folder` 已由「恒返回 `None` 的骨架」改为**真实原生目录选择**（`tauri-plugin-dialog 2`，`async` + `spawn_blocking` + 主窗口 `set_parent`，返回**绝对路径**）。注意：该频道在迁移后仍**不接收 `projectSession` 尾参**（属能力域，见上文 §2），返回路径仅作父目录输入，随后由 `project:create`/`project:open` 做项目根校验。**弹窗交互待人工点验**。

### 4.7 FileChannels（7 + grant 3 + dialog 1）— controller: `fs-controller.ts` + `external-file-grant-controller.ts` — 批次 B
基础（全部带 `expectedProjectPath`★）：`fs:read-file`、`fs:write-file`（返回 `commitState`）、`fs:list-dir`、`fs:mkdir`、`fs:check-exists`、`fs:read-json`、`fs:write-json`。
授权域（ADR 0002，只带 `grantId`+相对路径，不暴露绝对路径）：`fs:grant-read-file`、`fs:grant-write-file`、`fs:grant-mkdir`；`dialog:select-export-directory` 返回 `ExternalDirectoryGrant`。
Tauri 映射：`tauri-plugin-fs` scope 白名单 + 自研 grant 校验，**不得绕过**。

**状态（2026-10-08 第二十五次更新）**：`tauri-plugin-dialog 2.8.1` 已获批接入，其**硬依赖
`tauri-plugin-fs 2.6.0` 已随之连带引入** —— 本批次将来落地 `fs:grant-*` 与
`dialog:select-export-directory` 时**直接复用，勿重复添加**（注意保持版本一致）。
`dialog:select-export-directory` 目前**仍返回 `None`（取消语义）**：弹窗能力已就绪，
但该频道必须回传 `ExternalDirectoryGrant`（`grantId` + 受限相对路径，**绝不暴露绝对路径**，ADR 0002），
而 grant 注册表尚未迁移 —— 若此刻签发假 `grantId`，界面会显示「已选择导出目录」而后续写入必然失败。
故该频道**随 `fs:grant-*` 三命令一并归入批次 H**。

### 4.8 AppDataChannels（7）— controller: `app-data-controller.ts` — 批次 H
`prompt:load-global`（返回 `AppPromptLoadReceipt` 含 diagnostics）、`prompt:save-global`、`prompt:delete-global`；`skills:list-user`、`skills:inspect-github`、`skills:install-github`、`skills:uninstall-user`。固定 app-data 位置，渲染进程不决定路径。

### 4.9 LLMChannels（14 + 事件 3）— controller: `llm-controller.ts` — 批次 D（拆 D1/D2）
**D1 ✅（2026-10-07）：模型管理 7 频道**（`llm:list-models` / `save-model` / `delete-model` /
`get|set-default-model` / `get|set-default-embedding-model`）—— 仅依赖 JSON 文件读写，零新增依赖；
顺带补齐了 `config:get/set` 的真实持久化（`~/.lorekeeper/config.json`）。
**D2 ✅（已全部完成，2026-10-08）**：生成 / 流式 / 租约 / 发现 / 连通性 7 频道 + 3 事件均已落地，按子批次推进：
**D2-a ✅** 生成参数策略 + 模型执行租约（2 频道）；
**D2-b ✅** HTTP 生成 / 流式 / 取消（3 频道 + 3 事件，新增 `reqwest` 已经 Ask first 批准）；
**D2-c ✅**（`llm:*` 收口）2 频道：`llm:discover-models`（`llm/discovery.rs`）与
`llm:test-connection`（`llm/embedding.rs` + `commands/llm_management.rs`）。
租约：`llm:begin-execution-lease(modelId)`（返回 `ModelExecutionLeaseReceipt` 能力证据）、`llm:close-execution-lease(leaseId)`。
生成：`llm:generate(request)`（显式终态 `finishReason`）、`llm:generate-stream(requestId, request)`、`llm:cancel(requestId)`。
模型管理：`llm:list-models`、`llm:discover-models`、`llm:save-model`、`llm:delete-model`（级联返回默认模型）、`llm:set-default-model`、`llm:get-default-model`、`llm:set-default-embedding-model`、`llm:get-default-embedding-model`、`llm:test-connection(model, creativeStrategy?)`。
事件★G5：`llm:stream-chunk { requestId, chunk }`、`llm:stream-done { requestId, fullText, usage?, finishReason }`、`llm:stream-error { requestId, error }`。
迁移策略：Rust `reqwest` 重写 openai/gemini provider，复刻 `generation-parameter-policy`（sidecar 为降级方案，Ask first）。

### 4.10 DatabaseChannels（100）— controller: `db-controller.ts` — 批次 C/E/F/G 按子域拆分
全部带 `expectedProjectPath`★ + 会话注入★。按表域分组：

| 子域 | 频道 | 数 |
|---|---|---|
| project_core / 清理 / 全局事实 | `db:close`、`db:project-core-{get,update,synopsis-commit}`、`db:import-global-facts-commit`、`db:project-clear-generated-data`（返回 `cleared`/`physicalFilesDeleted`） | 6 |
| import-run（导入运行时状态机） | `db:import-run-prepare-inspection`、`-author-preview`、`-finalize-parsing`、`-get`、`-list-resumable`、`-list-chapters`、`-effect-receipt-{get,prepare,commit}`、`-start-resume`、`-renew-execution`、`-restart`、`-request-cancel`、`-cancel-at-boundary`、`-complete-batch`、`-advance-stage`、`-fail`、`-complete` | 18 |
| blueprints | `db:blueprint-{get-all,get,upsert,upsert-many,commit-range,update-notes,delete,clear-all}`、`db:blueprint-character-sync-{list-pending,get,complete}` | 11 |
| characters / roster | `db:character-get-all`、`db:character-roster-read`（唯一读 seam，事务内 read-back）、`db:character-roster-commit`（唯一提交 seam） | 3 |
| drafts | `db:draft-import-finalized-batch`、`db:draft-{create,list,list-all,get-meta,get-full,get-latest,get-finalized,get-max-finalized-chapter,authority-sequence,next-version,update-status,update-content,delete}`、`db:draft-export-{snapshot,authority-current}` | 16 |
| continuity（定稿连续性投影） | `db:continuity-{save-finalized,save-character-state-candidates,list-before,read-source}` | 4 |
| consistency-exemption | `db:consistency-exemption-{list,save,revoke}` | 3 |
| narrative-thread | `db:narrative-thread-{list,list-relevant,plan-create,plan-update,plan-delete,event-confirm}` | 6 |
| plot-tree（派生快照，乐观锁） | `db:plot-tree-{read,save,clear}`（save 带 `expectedSourceRevision`，失败 `errorCode: 'sources-changed'`） | 3 |
| recovery-candidate | `db:recovery-candidate-{record,list,update,resolve}` | 4 |
| finalization 关联 | `db:finalization-link-knowledge-document` | 1 |
| revisions | `db:revision-{create,replace-pending,list,get-pending,get-full,next-index,merge,mark-merged,mark-discarded}`（create/replace 支持 `expectedSource` 守卫，errorCode `SOURCE_DRAFT_CHANGED`；merge 返回幂等收据） | 9 |
| reviews | `db:review-{create,list,get-latest,get-full,next-index}` | 5 |
| post-process | `db:post-process-{create-run,get-latest-run,get-steps,mark-step-ok,mark-step-failed,is-all-passed}` | 6 |
| llm 日志 / 摘要 | `db:log-llm-call`、`db:get-llm-stats`、`db:get-llm-history`、`db:save-summary-snapshot`、`db:get-latest-summary` | 5 |

数据库层迁移：better-sqlite3 → `rusqlite`（bundled）；一域一仓（`blueprint_repository.rs`…）；controller 不写 SQL。

**迁移进度（2026-10-06 第九次更新）**：

| 子域 | 频道数 | 状态 |
|---|---|---|
| project_core / 清理 / 全局事实 | 6 | 🟡 **部分完成 4/6**：`db:close`、`db:project-core-{get,update,synopsis-commit}` 已注册并验证（`repositories/project_core_repository.rs`，`ff7fbd8`/`a8d742a`）；`db:project-clear-generated-data` 可直接续迁，`db:import-global-facts-commit` 依赖批次 G |
| blueprints | 11 | ⬜ 待迁移（**下一步**） |
| characters / roster | 3 | ✅ **已完成**（`69fc50c`）：`repositories/character_repository.rs` + `character_roster_repository.rs` + `character_role.rs`，`cargo test --lib` 94/94、0 告警 |
| drafts | 16 | ⬜ 待迁移 |
| revisions | 9 | ⬜ 待迁移 |
| reviews | 5 | ⬜ 待迁移 |
| post-process | 6 | ⬜ 待迁移 |
| llm 日志 / 摘要 | 5 | ⬜ 待迁移 |
| import-run（批次 G） | 18 | ⬜ 待迁移 |
| continuity / consistency-exemption / narrative-thread / plot-tree / recovery-candidate / finalization-link | 21 | ⬜ 待迁移（部分归批次 E/F） |

**双栈隔离约束（2026-10-06 第九次更新，取代早期「双栈同库」前提）**：Tauri 版命名为 **Lorekeeper（`com.tancan1998.lorekeeper`）**，与原项目**必须能在同一台机器同时运行**，且用户已确认**不需要复用同一 SQLite、不需要解码原项目 DB 数据**。因此：① 全局数据根 `AI_NOVEL_LOREKEEPER_HOME` / `~/.lorekeeper`（**不读** `AI_NOVEL_VELA_HOME`、**不回退** `~/.vela`）；② 项目库文件 `<root>/.vela/lorekeeper.db`（基线为 `.vela/vela.db`）；③ 安装标识、exe 名、窗口标题全独立。Rust 侧 DDL 仍与 Electron `electron/database.ts:67` 的**最终列集**保持一致，但目的已从「共库写入」变为「保留将来一次性导入的能力」；逐子域增量 DDL 依然安全（两栈均 `CREATE TABLE IF NOT EXISTS`，互不读写对方库文件）。`.vela` 目录本身仍共享（L3 押后）。

### 4.11 KnowledgeBaseChannels（15 + dialog 2）— controller: `kb-controller.ts` — 批次 F
`kb:import-{document,folder}`（grantId 入口）、`kb:import-{text,planning-text,reference-text}`、`kb:search`、`kb:search-writing-context`、`kb:search-with-scope`、`kb:list-documents`、`kb:remove-document`、`kb:clear-all`、`kb:stats`、`kb:get-vectorless-count`、`kb:get-vector-rebuild-status`（纯本地状态读，不发 embedding 请求）、`kb:backfill-vectors`；`dialog:select-knowledge-{files,folder}`。
返回多为 `AppResult<T>`★。LanceDB → Rust `lancedb` crate 或降级 SQLite FTS（评估项）。

### 4.12 ImportChannels（1）— controller: `import-controller.ts` — 批次 G
`dialog:select-novel-files(request?, projectSession?)`（选择 + 检查 + 准备一体；不注入自动 session★，显式收 `projectSession`）。

### 4.13 ChapterLifecycleChannels（4）— controller: `chapter-lifecycle-controller.ts` — 批次 E
`chapter:delete-finalized(request)`、`chapter:retry-deletion(operationId)`、`chapter:confirm-legacy-knowledge-absent(operationId)`、`chapter:list-incomplete-deletions`。
**定稿删除专属生命周期（ADR 0011）：可恢复删除、操作幂等、知识库遗留确认** —— Rust 等量单元测试硬性要求。

### 4.14 MCPChannels（9）— controller: 无独立 controller（`electron/mcp/`）— 批次 H
`mcp:load-config`、`mcp:connect`、`mcp:disconnect`、`mcp:disconnect-all`、`mcp:list-tools`、`mcp:list-resources`、`mcp:call-tool`、`mcp:get-servers-status`、`mcp:get-config-path`。全局域★G6（stdio/sse 子进程，Tauri 侧评估 `tauri-plugin-shell` 或 Rust MCP SDK）。

### 4.15 FinalizationChannels（2，**未声明**★G1）— controller: `finalization-controller.ts` — 批次 E
`finalization:commit(snapshot, projectSession)` → `FinalizationResult`；`finalization:retry(finalizationId, projectSession)` → `FinalizationResult`。
**定稿不可逆**：草稿身份 + 收据 + 章节号 + 正文哈希绑定（ADR 0003/0011）。迁移时补声明 + 等量测试。

## 5. 事件频道汇总（5 个 → Tauri Event）

| 事件 | 载荷 | 频率 | 批次 |
|---|---|---|---|
| `llm:stream-chunk` | `{ requestId, chunk }` | 每 token/chunk | D |
| `llm:stream-done` | `{ requestId, fullText, usage?, finishReason }` | 每请求 1 次 | D |
| `llm:stream-error` | `{ requestId, error }` | 异常时 | D |
| `update:state` | `UpdateState` | 更新状态机各阶段 | H |
| `window:close-requested` | `{ requestId }` | 用户关窗时 | A |

## 6. 迁移批次规划（与 pi-development.md §5 顺序一致，按依赖补细分）

| 批次 | 内容 | 前置 | 关键约束 |
|---|---|---|---|
| **0** | Tauri 脚手架（Ask first：新增 `src-tauri/` + 依赖）、AppState + invoke_handler 骨架 | — | 不动 Electron 代码 |
| **A** | window(+G4 决策协议)、config、skin、official-homepage、model-provider-resource | 0 | 前端可见性最快，先打通 invoke/event 双通道 |
| **B** | project、fs 基础、external-file-grant、dialog | 0 | 会话租约签发/校验（ADR 0001）、fs 授权（ADR 0002）。**2026-10-08 补记**：批次 B 交付时 `dialog:select-folder` / `dialog:select-export-directory` 均为**恒返回 `None` 的骨架**（频道登记齐全、命令已注册，故 `check:channels` 不报异常，UI 仅表现为「点了没反应」——排查类似问题须核对**命令实现是否为骨架**）。**已补齐 ✅**：`dialog:select-folder` 真实化（`tauri-plugin-dialog 2`，Ask first 已批准）；`dialog:select-export-directory` 仍为取消骨架，**有意为之**，随批次 H 的 grant 域一并落地 |
| **C** | db 子域逐步：project-core → blueprints → characters/roster → drafts → revisions → reviews → post-process → summary/llm-stats | B | 🟡 **进行中（2026-10-06 第八次更新）**：`project_core` 子域**已完成并提交**（`ff7fbd8` + `a8d742a`，Rust 64/64、0 告警）；剩余子域按上方进度表逐个迁移（入口见 `docs/handoffs/2026-10-06-tauri-migration-status.md` 第八次快照「下一子域接续入口」）。一域一仓；行为与 Electron 对照 |
| **D** | llm 全部 + 3 个流事件（G5 性能实测） | C | generation-parameter-policy 复刻；finishReason 显式终态。**2026-10-07 拆分**：**D1 ✅** 模型管理 7 频道（零新增依赖）+ config 真实持久化；**2026-10-08 完成 D2-a/b/c ✅**：生成参数策略与执行租约（D2-a）、HTTP 生成/流式/取消（D2-b）、`llm:*` 收口——模型发现与连通性探测（D2-c）。`llm:` 前缀下 14 个 invoke 频道已**全部迁移** |
| **E** | finalization(G1 补契约)、chapter-lifecycle、continuity、recovery-candidate、draft-import-finalized-batch | C, D | 定稿不可逆 + 删除生命周期（ADR 0003/0011）等量测试 |
| **F** | kb 全部、plot-tree、narrative-thread、consistency-exemption | C | AppResult 错误码对齐；LanceDB 评估 |
| **G** | import-run 全套（18 频道状态机）、dialog:select-novel-files、import-global-facts | C | 执行租约 `ImportRunExecutionLease`；断点恢复语义 |
| **H** | update（updater 插件，G3）、mcp、prompt/skills、**fs:grant-\* 三命令 + `dialog:select-export-directory`**（第二十五次归入） | 任意 | macOS 更新 = 仅打开 Release 页；`tauri-plugin-fs 2.6.0` **已随 dialog 插件连带引入**，勿重复添加；grant 签发只回传 `grantId`（ADR 0002） |

每批次验收：Rust 单元测试 + `cargo test` + 前端 `pnpm typecheck`/相关 `pnpm test` + 与 Electron 版行为对照。
