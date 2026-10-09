# Tauri 迁移 — IPC 契约盘点（阶段 1 产出）

> **本文为活文档，只维护频道清单、契约缺口与批次规划；迁移进度见 [`docs/handoffs/`](../handoffs/) 最新快照。**
> 每个批次完成后如频道口径变化才更新本文。

> **用途**：Tauri 迁移的 channel → `#[tauri::command]` 映射骨架。类型细节以 [`src/shared/ipc-channels.ts`](../../src/shared/ipc-channels.ts) 为唯一事实源，本文只做结构盘点、模式标注与批次规划。
> **基线**：master @ `cb71878`（2026-10-06 盘点）。Electron 代码未改动。
> **重生成方式**：对 `src/shared/ipc-channels.ts` 与 `electron/controllers/*.ts` 重跑 Select-String 频道正则即可核对；本文为手工核对结果。

## 1. 总体统计

| 类别 | 数量 | 说明 |
|---|---|---|
| invoke 频道（请求/响应） | **193** | 渲染 → 主进程，`ipcMain.handle`（含 G1 补建 2 个） |
| 事件频道（主 → 渲染推送） | **5** | `ipcRenderer.on`，迁移为 Tauri Event |
| 合计 | **198** | 每个频道对应一个 Rust 命令或事件 |

> **【契约单源位置刻意偏移 — G1，2026-10-09】** 定稿频道 `finalization:commit` / `finalization:retry`
> 在 Electron 侧由 `finalization-controller.ts` 真实注册，但**上游两份 `ipc-channels.ts` 均未声明**。
> Tauri 迁移期在该单源改为 **`tauri-app/src/shared/ipc-channels.ts`**：
> `verify-channel-coverage.mjs`与 `test/channel-migration-coverage.test.ts` 的契约读取路径同步从
> 仓库根 `src/shared/ipc-channels.ts` 改为 tauri-app 副本；**基线 `src/` 保持不动**，以免
> 后续合并上游时在该文件产生冲突（2026-10-09 用户决策）。事件频道数口径不变（仍 5）。

> 已注册命令数、未迁移数、测试通过数等**会变动的数字不在本文维护** —— 一律见 [`docs/handoffs/`](../handoffs/) 最新快照。

**前端调用面（重大发现）**：渲染进程对 `window.velaAPI` 的直接访问收敛在 **2 个文件**，其余业务代码全部经由封装层调用：

| 文件 | 角色 | 迁移动作 |
|---|---|---|
| `src/services/ipc-client.ts` | 主封装：类型安全 `invoke/on/once/send` + 项目会话注入 + zoom | 仅替换底层 `getAPI()` 实现（`@tauri-apps/api/core.invoke` / `event.listen`），上层零改动 |
| `src/services/finalization-client.ts` | 定稿专用封装（绕过 ipc-client） | ✅ **已完成（G1）**：`getVelaApi()` → `ipc.invoke`（仍是唯一绕过 ipc-client 会话自动注入的封装，`projectSession` 显式尾参） |

`electron/preload.ts` 暴露面 = `invoke / on / once / send / setZoomLevel / setZoomFactor / getZoomLevel`（zoom 走 Electron `webFrame`，Tauri 侧需用 Webview zoom API 或 CSS 替代，归阶段 3）。

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
| G1 | **`finalization:commit` / `finalization:retry` 未在 ipc-channels.ts 声明** | 仅 `electron/controllers/finalization-controller.ts`（注册）与 `src/services/finalization-client.ts`（调用：args = `(FinalizationSnapshot, ProjectSessionContext)` / `(finalizationId, ProjectSessionContext)`，return `FinalizationResult`，类型来自 `electron/services/finalization-service.ts`） | ✅ **已解决（G1，2026-10-09）**：`FinalizationChannels` + `FinalizationResult`/`FinalizationSnapshot` 声明于 **tauri-app 副本**；基线 `src/` 未动 |
| G2 | **src 直接 import electron 目录的类型** | `ipc-channels.ts` 与 `finalization-client.ts` `import type` 自 `electron/repositories/*`、`electron/services/*` | Tauri 迁移期间保留（TS 类型不产生运行时依赖）；Rust 侧按类型定义平移；长期可把纯类型下沉 `src/shared/`（Ask first） |
| G3 | **update-controller 注册位置特殊** | 不在 `ipc-handlers.ts`，在 `electron/main.ts:259` 注册，带 `publish: publishUpdateState` 回调（发布 `update:state` 事件） | 迁移时注意 update 命令与 update:state 事件同源于 update-service |
| G4 | **window:close-requested 是「请求-决策」两段式** | 主进程发 `window:close-requested { requestId }`，渲染端回 `window:resolve-close(requestId, 'proceed'|'cancel')` | Tauri 侧拦截 `tauri://close-requested`（`on_window_event` + `api.prevent_close()`），保留 requestId 决策协议 |
| G5 | **高频流式事件** | `llm:stream-chunk` 每 chunk 一发；Tauri event 经 webview 桥，性能需实测；必要时批量合并 chunk | 阶段 2 D 批次实测 |
| G6 | **MCP 频道无 expectedProjectPath / session** | `mcp:*` 9 个频道是全局的，不注入会话 | 按 app-data 域处理 |

## 4. 分域清单（invoke 191 个）

「批次」见 §6。★ = 有 §2/§3 特殊模式。

### 4.1 ConfigChannels（2）— controller: `config-controller.ts` — 批次 A
`config:get`、`config:set`。→ `config_get` / `config_set`。

### 4.2 UpdateChannels（6 + 事件 1）— controller: `update-controller.ts`（main.ts 注册，G3）— 批次 H ✅
`update:get-state`、`update:check`、`update:download`、`update:open-release`、`update:defer-reminder(days)`、`update:quit-and-install`。
事件：`update:state → UpdateState`（迁移为 `app.emit`，见 `update/startup.rs` 的 publish 闭包）。

> ✅ **2026-10-10（第三十四次 H3）**：6 频道 + 事件已迁移（`src/update/` 9 文件 + `commands/update.rs`）。
> **零新依赖**：不引入 `tauri-plugin-updater`，后端恒为 GitHub-Release 只读元数据（仓库指向 fork）。
> 刻意偏离：`updateAction` 全平台 `open-release` → `update:download`/`update:quit-and-install` 诚实返回
> `DOWNLOAD_NOT_READY`/`INSTALL_NOT_READY`（真 Windows 自更新见 B14）；日历日节流用 UTC。
> 门禁保留（`isPackaged` + 无 devUrl）：dev 下为 `disabled`，与基线一致。

### 4.3 SkinChannels（3）— controller: `skin-controller.ts` — 批次 A
`skin:get-state`、`skin:execute(command)`、`skin:read-custom-asset`。皮肤服务初始化失败须降级不阻断（ipc-handlers 启动语义）。

### 4.4 WindowChannels（4 + 事件 1）— controller: `window-controller.ts` — 批次 A
`window:minimize`、`window:toggle-maximize`、`window:close`、`window:resolve-close(requestId, decision)`★G4。
事件：`window:close-requested { requestId }`★G4。

### 4.5 OfficialHomepage / ModelProviderResource（1+1）— 批次 A ✅

> ✅ **2026-10-09（第三十三次 B12）**：两频道曾是**假成功占位**（返回 `success:true` 却不打开 URL，
> 且未登记为骨架），已由 `tauri-plugin-opener` + `external_link.rs` 真实化。
> ⚠️ 基线还有「拒绝渲染层导航替换主框架」（`preventRendererNavigation`）的等价防护尚未接入（B13）。
`official-homepage:open`、`model-provider-resource:open(resourceId)`（主进程映射固定 HTTPS URL，不接受任意 URL）。

### 4.6 ProjectChannels（10 + dialog 1）— controller: `project-controller.ts` — 批次 B
`project:get-runtime-context`、`project:create(config, requestToken, rendererProjectPath)`、`project:open(projectPath, requestToken, rendererProjectPath)`、`project:save`★、`project:update-config`★、`project:recent-list`、`project:recent-remove`、`project:delete`★（返回含 `directoryDeleted`/`databaseRestored`，须事务性恢复）、`project:smoke-open-request`、`project:smoke-open-confirm`、`dialog:select-folder`。
核心语义：requestToken / rendererProjectPath 防陈旧窗口写入；`sessionLease` 由主进程签发冻结。
**契约要点**：`dialog:select-folder` 迁移后仍**不接收 `projectSession` 尾参**（属能力域，见 §2），返回路径仅作父目录输入，随后由 `project:create` / `project:open` 做项目根校验。

### 4.7 FileChannels（7 + grant 3 + dialog 1）— controller: `fs-controller.ts` + `external-file-grant-controller.ts` — 批次 B / H ✅

> ✅ **2026-10-09（第三十三次 H1）**：`fs:grant-read-file` / `write-file` / `mkdir` 与
> `dialog:select-export-directory` 已从占位真实化（B10/B3 解除）；
> 授权用尽语义已对齐基线（归零保留记录 + 后续消费报已用尽）。
基础（全部带 `expectedProjectPath`★）：`fs:read-file`、`fs:write-file`（返回 `commitState`）、`fs:list-dir`、`fs:mkdir`、`fs:check-exists`、`fs:read-json`、`fs:write-json`。
授权域（ADR 0002，只带 `grantId`+相对路径，不暴露绝对路径）：`fs:grant-read-file`、`fs:grant-write-file`、`fs:grant-mkdir`；`dialog:select-export-directory` 返回 `ExternalDirectoryGrant`。
Tauri 映射：`tauri-plugin-fs` scope 白名单 + 自研 grant 校验，**不得绕过**。
**依赖结论（对契约有影响）**：`tauri-plugin-dialog 2.8.1` 的硬依赖 **`tauri-plugin-fs 2.6.0` 已随之连带引入** —— 将来落地 `fs:grant-*` 与 `dialog:select-export-directory` 时**直接复用，勿重复添加**（保持版本一致）。
**批次结论**：`dialog:select-export-directory` 必须回传 `ExternalDirectoryGrant`（`grantId` + 受限相对路径，**绝不暴露绝对路径**，ADR 0002），而 grant 注册表尚未迁移，故该频道**随 `fs:grant-*` 三命令一并归入批次 H**。

### 4.8 AppDataChannels（7）— controller: `app-data-controller.ts` — 批次 H ✅
`prompt:load-global`（返回 `AppPromptLoadReceipt` 含 diagnostics）、`prompt:save-global`、`prompt:delete-global`；`skills:list-user`、`skills:inspect-github`、`skills:install-github`、`skills:uninstall-user`。固定 app-data 位置，渲染进程不决定路径。

> ✅ **2026-10-09（第三十三次 H2）**：7 频道已迁移（`commands/app_data.rs` + `writing_skills.rs`）。
> GitHub 抓取复用全局配置的代理（`proxy_from_config` + `build_client_with_timeout`，10s 超时、禁止重定向）；
> 路径与限额对齐基线（SKILL.md ≤ 64 KiB、URL ≤ 2048 字符、路径段须 percent-decode 后为安全段）。

### 4.9 LLMChannels（14 + 事件 3）— controller: `llm-controller.ts` — 批次 D（拆 D1/D2）
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
| import-run（导入运行时状态机） | `db:import-run-prepare-inspection`、`-author-preview` ✅G1、`-finalize-parsing`、`-get`、`-list-resumable`、`-list-chapters`、`-effect-receipt-{get,prepare,commit}`、`-start-resume`、`-renew-execution`、`-restart`、`-request-cancel`、`-cancel-at-boundary`、`-complete-batch`、`-advance-stage`、`-fail`、`-complete` | 18 |
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

### 4.11 KnowledgeBaseChannels（15 + dialog 2）— controller: `kb-controller.ts` — 批次 F2

> ⚠️ **2026-10-09（B10）**：UI 层唯一的知识库导入入口是 `KnowledgeOverview` 的「导入参考资料」
> → `selectPlanningMaterials()` → `dialog:select-knowledge-files` ✅ + **`fs:grant-read-file` ⛔（批次 H 占位）**。
> 即 F2 的 `kb:*` 服务端内核已可用（已被第 2 章后处理 `kb_import` 间接验证），但**界面导入在批次 H 前不可用**。
`kb:import-{document,folder}`（grantId 入口）、`kb:import-{text,planning-text,reference-text}`、`kb:search`、`kb:search-writing-context`、`kb:search-with-scope`、`kb:list-documents`、`kb:remove-document`、`kb:clear-all`、`kb:stats`、`kb:get-vectorless-count`、`kb:get-vector-rebuild-status`（纯本地状态读，不发 embedding 请求）、`kb:backfill-vectors`；`dialog:select-knowledge-{files,folder}`。
返回多为 `AppResult<T>`★。**向量存储路线（用户决策，方案 B）**：`rusqlite`（bundled）+ **SQLite FTS5 + jieba-rs 预分词 + HNSW 向量索引 + RRF 倒数排名融合**（自研混合检索）。**否决** `lancedb` Rust crate（+1680 传递依赖、需 protoc/ninja/nasm、与「减内存」目标相背）与 `cairn-search`（非通用库，紧耦合 cairn-core）。最终决策见 [`2026-10-08-final-decision-plan-b.md`](../research/2026-10-08-final-decision-plan-b.md)；实测 `FTS5 unicode61` 中文召回率 **0%**，必须写入前预分词。基线封装为 `electron/vector-store.ts`（2010 行，含 embedding space 注册表 / 重建计划 / FTS 索引 / 混合检索），专项评估已完成（见上决策文档），实施时仍须逐项对齐差异。
**✅ 隔离红线已解决（2026-10-09）**：基线向量数据在 `{project}/.vela/lancedb/` 与 `.vela/*.json`（**共享目录**）。**L3 后 Tauri 项目目录已改为 `.lore/`**（两栈项目目录刻意不互通），向量快照定为 `{project}/.lore/kb/` —— 互覆风险消除。

**实施进度（2026-10-09）**：

- **F2-1 ✅**（`99efceaf`）：四表 DDL（`kb_documents` / `kb_chunks` / `kb_embedding_spaces` / `kb_fts`）+ `db/kb/chunks.rs`（`chunkText` 逐字移植）+ `db/kb/fts.rs`（jieba 预分词 + FTS5 CRUD/检索）。
- **F2-2 ✅**（`d0538819`）：`db/kb/hybrid.rs`（基线语义默认 + RRF 可选开关 + 嵌入空间注册表 + 回填计划），并为 `db/vector.rs` 补 `live_doc_ids` / `contains`。
- **F2-3 ✅**（`419076db` 基础设施 + `20d26f42` 命令层 + `0e74971e` 前端）：`external_grant.rs`（内存态外部文件授权注册表）+ `db/kb/vectors.rs`（按项目/代际懒加载 HNSW）+ `db/kb/store.rs`（SQLite 存储与文本检索编排）+ `commands/kb.rs`（15 频道 + 2 dialog）+ 前端登记。**F2 全部完成**。
  - 默认语义：**向量可用且召回非空 → 短路**；否则文本支路（FTS5 + jieba）。嵌入空间按「指纹 + 维度」匹配，换模型触发 `reindex_required`。
  - **已知缺口**：`kb:import-reference-text` 仍为显式占位失败（依赖批次 G）；存储预检仅最小移植（Windows MAX_PATH）；基线 `vectors.json` 迁移 barrier 在双栈隔离后无适用路径。
  - **章节清理收口**：`chapter:*` 的知识库物理清理已由占位改为真实 `removeDocument`（实体稿清理仍待批次 H）。
- **⚠️ 基线并无融合（本轮读源码确认）**：`searchWithScope` 实为「向量可用且召回非空 → 短路返回；否则 → 降级 `LIKE` 子串扫描」，且**降级分支 `score` 恒为 0.5**（`relevance` 只参与排序）。经用户 2026-10-09 决定：**默认对齐基线语义**，**RRF 仅作可选开关（默认关）**，降级分支**改返回真实相关性分**（差异已记录于 `hybrid.rs` 模块文档；方案 B 文档 §8.5/§8.7 自相矛盾之处以此为准）。
- **`kb:import-reference-text` 依赖批次 G**（import-run 域未迁移）→ 实施时先注册但**显式占位失败**（沿用批次 E 诚实化占位先例）。

### 4.12 ImportChannels（1）— controller: `import-controller.ts` — 批次 G ✅ G1
`dialog:select-novel-files(request?, projectSession?)`（选择 + 检查 + 准备一体；不注入自动 session★，显式收 `projectSession`）。
**✅ G1（2026-10-10）已迁移**：交付「导入作者原稿」完整链路 —— 检视存储（`src/import/inspection_store.rs`）+ 章节解析（`src/import/parsing.rs`）+ `commands/import.rs`；`reference` 与 `.epub` 返回**诚实错误**（分别依赖 G2 状态机 / 新增解包依赖）。刻意偏离见开工清单 D1–D7。

### 4.13 ChapterLifecycleChannels（4）— controller: `chapter-lifecycle-controller.ts` — 批次 E
`chapter:delete-finalized(request)`、`chapter:retry-deletion(operationId)`、`chapter:confirm-legacy-knowledge-absent(operationId)`、`chapter:list-incomplete-deletions`。
**定稿删除专属生命周期（ADR 0011）：可恢复删除、操作幂等、知识库遗留确认** —— Rust 等量单元测试硬性要求。**请求即执行 + 状态机恢复**（非早期误记的四段式）。
**物理清理依赖**：删实体稿文件 → fs 授权域（批次 H）；删知识库文档 → ✅ **已随 F2-3 真实化**（复用 kb store + HNSW 向量清除）。

### 4.14 MCPChannels（9）— controller: 无独立 controller（`electron/mcp/`）— 批次 H
`mcp:load-config`、`mcp:connect`、`mcp:disconnect`、`mcp:disconnect-all`、`mcp:list-tools`、`mcp:list-resources`、`mcp:call-tool`、`mcp:get-servers-status`、`mcp:get-config-path`。全局域★G6（stdio/sse 子进程，Tauri 侧评估 `tauri-plugin-shell` 或 Rust MCP SDK）。

### 4.15 FinalizationChannels（2，**契约补于 tauri-app 副本**★G1）— controller: `finalization-controller.ts` — 批次 E ✅
`finalization:commit(snapshot, projectSession)` → `FinalizationResult`；`finalization:retry(finalizationId, projectSession)` → `FinalizationResult`。
**定稿不可逆**：草稿身份 + 收据 + 章节号 + 正文哈希绑定（ADR 0003/0011）。
✅ **G1 完成（2026-10-09）**：`commands::finalization_commit` / `finalization_retry`（`commands/finalization.rs`）
+ `manuscript_publisher.rs`（实体稿发布 / 删除，与 `finalized_draft_import_repository` 共用目标解析）；
前端 `finalization-client.ts` 底层切 `ipc.invoke`。契约声明落在 `tauri-app/src/shared/ipc-channels.ts`。

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
| **B** | project、fs 基础、external-file-grant、dialog | 0 | 会话租约签发/校验（ADR 0001）、fs 授权（ADR 0002）。**排查提醒**：频道登记齐全但 UI「点了没反应」时，须核对**命令实现是否为骨架**（`check:channels` 只查映射，不查实现完整度） |
| **C** | db 子域逐步：project-core → blueprints → characters/roster → drafts → revisions → reviews → post-process → summary/llm-stats | B | 一域一仓；行为与 Electron 对照 |
| **D** | llm 全部 + 3 个流事件（G5 性能实测） | C | generation-parameter-policy 复刻；finishReason 显式终态。拆 D1（模型管理）/ D2（生成·流式·租约·发现·连通性） |
| **E** | finalization(G1 ✅ 已完成)、chapter-lifecycle、continuity、recovery-candidate、draft-import-finalized-batch | C, D | 定稿不可逆 + 删除生命周期（ADR 0003/0011）等量测试。物理清理依赖：删实体稿文件（✅ **G1 已真实化**）、删 KB 文档（✅ 已随 F2-3 真实化） |
| **F** | kb 全部、plot-tree、narrative-thread、consistency-exemption | C | AppResult 错误码对齐。拆 **F1**（plot-tree 3 / narrative-thread 6 / consistency-exemption 3 = 12 频道，零新依赖，纯 SQLite 平移；剧情树 `sourceRevision` 有**黄金哈希测试**锁定与 `JSON.stringify` 逐字节一致）/ **F2**（`kb:*` 15 + `dialog:select-knowledge-*` 2）。**F2 向量路线见 §4.11**；隔离红线**已解决**（L3 后项目目录 `.lore/`，向量快照 `.lore/kb/`）。进度：**F2-1 ✅ / F2-2 ✅ / F2-3 ✅（F2 全部完成）** |
| **G** | import-run 全套（18 频道状态机）、dialog:select-novel-files、import-global-facts | C | 执行租约 `ImportRunExecutionLease`；断点恢复语义。**已知依赖**：`kb:import-reference-text` 待本批收口（当前为显式占位失败）。**进度（2026-10-10）**：**schema 已获批并落地**——`db/schema.rs` 新增 9 张表 + 7 索引（`import_runs` / `import_run_chapters` / `import_run_sources` / `import_run_source_chapters` / `import_source_chapter_map` / `import_run_receipts` / `import_run_knowledge_receipts` / `import_reference_documents` / `import_source_aliases`），**刻意不建** `import_legacy_identity_bridge`；申报书见 [`docs-fork/research/2026-10-10-g-import-run-schema-proposal.md`](../research/2026-10-10-g-import-run-schema-proposal.md)。分批：G1 检视面（3）→ G2 状态机主体（5）→ G3 租约与批次推进（11）→ G4 收口。⚠️ **2026-10-10 调研后修订分批**：渲染层只用结构化请求路径，而 `reference` 分支需 `beginParsing`（G2），故 **G1 改为交付「导入作者原稿」完整链路**（`dialog:select-novel-files` author-manuscript 路径 + 检视存储 + 章节解析 + `db:import-run-author-preview`），`reference` 与 `.epub` 返回诚实错误；开工清单见 [`docs-fork/plans/2026-10-10-g1-import-select-kickoff.md`](./2026-10-10-g1-import-select-kickoff.md)（含 D1 无密钥 sha256 别名等 4 项刻意偏离）。**✅ G1 完成（2026-10-10）**：`dialog:select-novel-files` + `db:import-run-author-preview` 已注册（2 频道），新增 `windows-sys`（已在 lock，0 新下载）与刻意偏离 D7（数字感知自然序）；**下一步 G2**（状态机主体 + `reference` 路径） |
| **H** | update（零依赖 GitHub-Release 后端，见 H3 评估；真正 Windows 自更新需 updater 插件 + 签名密钥，另立专项）、mcp（**暂缓**，仅 stdio）、prompt/skills ✅、**fs:grant-\* 三命令 ✅ + `dialog:select-export-directory` ✅** + official-homepage/model-provider-resource 打开链接 ✅ | 任意 | ✅ **进度（2026-10-09 第三十三次）**：H1（fs:grant-* + 导出目录）✅、H2（prompt/skills 7 频道）✅、B12（`tauri-plugin-opener`，修好两处假成功）✅；**H3（update 6 频道）待做**；H4（mcp 9 频道）暂缓。macOS 更新 = 仅打开 Release 页；`tauri-plugin-fs 2.6.0` 已随 dialog 插件连带引入，勿重复添加；grant 签发只回传 `grantId`（ADR 0002） |

每批次验收：Rust 单元测试 + `cargo test` + 前端 `pnpm typecheck`/相关 `pnpm test` + 与 Electron 版行为对照。**验收数字见 [`docs/handoffs/`](../handoffs/) 最新快照。**