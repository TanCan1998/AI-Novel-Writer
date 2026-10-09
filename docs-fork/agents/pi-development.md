# pi Agent 开发文档 — AI Novel Writer（Tauri 迁移）

> 本文面向 pi coding agent。当前项目为 **Electron 桌面应用**，正在迁移为 **Tauri 2**。本文以「迁移为 Tauri」为核心目标，提供当前 Electron 基线、迁移映射、Rust 后端规范与分步实施路径。通用命令/风格见根 `AGENTS.md`。

**决策来源**：`docs-fork/research/2026-10-04-tauri-migration-evaluation.md`（源自 `F:\Temp\ai-novel-writer-memory.md`）评估结论——UI/脚本复用率 >95%，核心工作量在 IPC 层与原生绑定重写（约 3–4 周，IPC 层 3–4 周为关键路径）。

> ⚠️ 现状：桌面应用目前仍是 **Electron**。Tauri 迁移为**进行中的目标任务**，`src-tauri/` 尚未建立；任何开发须区分「维护 Electron 基线」与「推进 Tauri 迁移」，勿混改。

---

## 0. `docs/` 与 `docs-fork/` 边界（硬规则，先读）

本 fork 的文档分为两个物理隔离的根：

| 目录 | 性质 | 规则 |
|---|---|---|
| `docs/` | **上游镜像** | 与 `upstream/master` **逐字节一致**。**永不修改、永不新增**。合并上游时直接接受上游版本。 |
| `docs-fork/` | **fork 专属** | 所有 fork 新增文档一律放这里。目录结构镜像 `docs/` 的子目录（`agents/` `adr/` `handoffs/` `plans/` `research/`）。 |

**六条硬规则**：

1. **不写 `docs/`**：任何 fork 改动都不得落在 `docs/` 下的文件上。若确需修改上游文档，
   把改动写入 `docs-fork/notes-on-upstream-changes.md`，并在其中注明位置、拟插入内容与保留理由。
2. **新文档进 `docs-fork/`**：调研、快照、计划、规则一律写入 `docs-fork/` 对应子目录。
3. **合并上游零冲突**：`git merge upstream/master` 时，`docs/` 整体接受上游版本，
   `docs-fork/` 不受影响。合并后检查 `notes-on-upstream-changes.md` 是否需重新施加。
4. **校验命令**：迁移或合并后跑 `git diff upstream/master -- docs/`，**必须无输出**。
5. **不碰 `AGENTS.md` 的忽略状态**：根 `AGENTS.md` 是上游卫生规则禁止入库的本机文件
   （见 `scripts/__tests__/public-repository-hygiene.test.ts` 的 `prohibitedPaths`）。
   可以本地编辑其内容（引用路径随迁移更新），但**不得** `git add`、**不得**移入
   `docs/` 或 `docs-fork/`、**不得**改动 `.gitignore:62` 的 `/AGENTS.md` 条目。
6. **文档入口分工**：`docs-fork/README.md` 是 fork 的**公共入库入口**（clone 后可见，
   供人 / CI / GitHub 阅读）；根 `AGENTS.md` 是**本机 Agent 会话入口**（不入库，
   仅本机可见）。两者互补，不可互相替代。

**与其他章节的关系**：§9 规定快照的文件名与结构，本节第 2 条规定快照的**存放位置**
（`docs-fork/handoffs/`，而非 `docs/handoffs/`）；§9 第 6 条「同步两处指针」中的路径
亦按本节更新。

> 迁移历史：2026-10-09 由 `docs/` 整体迁至 `docs-fork/`，使 `docs/` 与上游逐字节一致，
> 消除长期合并冲突。迁移前 fork 对上游 `docs/` 的改动见
> [`notes-on-upstream-changes.md`](../notes-on-upstream-changes.md)。

---

## 1. 迁移总览与基线

**来源**（Electron，当前可运行基线）：
```
electron/
  main.ts          应用入口 → 迁移为 src-tauri/src/lib.rs + main.rs
  preload.ts       contextBridge → 移除，前端改用 @tauri-apps/api
  ipc-handlers.ts  IPC 注册点 → 迁移为 invoke_handler（命令注册）
  controllers/     39 个文件，IPC controller → 迁移为 Rust #[command] + state
  repositories/    50 个文件，SQLite 数据访问（better-sqlite3）→ rusqlite/sqlx
  services/        51 个文件，领域服务 → 迁移为 Rust（或 sidecar 保留 Node 实现）
  llm/             9 个文件，LLM 适配（openai/gemini）
  security/        5 个文件，原生安全文件系统封装 → Tauri fs/shell 插件改写
src/
  shared/  ipc-channels.ts（44KB 类型化 IPC 契约，迁移唯一事实源）
  services/ agent(引擎)、workflows(流程)、generation  → 多数纯前端，可平移
  stores/ components/  → 纯前端，基本不动
```

**关键复用 vs 重写**：

| 模块 | 处理方式 |
|------|---------|
| React 组件、Zustand stores、Tailwind | **复用**，几乎不改 |
| `src/shared/*` 类型、`ipc-channels.ts` 契约 | **复用**，作为 Rust 命令签名蓝图 |
| IPC 层（controller → command） | **重写**，核心工作量 |
| 数据库访问（better-sqlite3 → rusqlite） | **重写**，次核心 |
| 窗口/菜单/通知/更新 | **重写**，绑定 Tauri 插件 |
| 原生模块（lancedb、原生 fs 安全层） | **评估后改写** |

---

## 2. IPC 契约 → Tauri 命令（迁移核心）

`src/shared/ipc-channels.ts` 用类型化接口声明每个频道的 `args`/`return`（如 `ConfigChannels`）。**这是迁移的唯一事实源**。规则：

1. 每个 channel（如 `'config:get'`、`'update:check'`、`'skin:get-state'`…）→ 一个 Rust `#[tauri::command]`。
2. 命令 kebab-case 命名（`tauri` 约定），与 channel 名对应，如 `config_get`、`update_check`。
3. Rust 入参/返回值与 `ipc-channels.ts` 的 `args`/`return` **类型一一对应**（`String`/`u32`/struct/`Option`/列表）。复杂联合类型先转换为 Rust `enum`（serde）。
4. 事件类频道（`UpdateStateEvents['update:state']`）→ 迁移为 Tauri `Event`（`app.emit(...)` / 前端 `listen(...)`）。
5. **结构体入参必须显式 `#[serde(rename_all = "camelCase")]`**。渲染层发的是 camelCase（如 `{ draftId, chapterNumber }`），Tauri 只对**参数名**做 camelCase→snake_case 归一，**不会**转换结构体字段。缺该属性会在参数反序列化阶段失败并 **reject promise**（`invalid args ...: missing field \`draft_id\``）；调用点通常不 catch → 表现为「确认框关闭但什么都没发生」的**静默失败**，后端零记录（连一行都不会落）。⚠️ 单元测试直接构造 Rust 结构体，**永远抓不到**这类缺陷。
   防线：`tauri-app/test/ipc-arg-struct-contract.test.ts`（源码扫描，内部结构体走白名单）；新增命令入参结构体时一并补契约测试（从 camelCase JSON 反序列化）。

```rust
// 迁移自 ConfigChannels['config:get']（args: [],  return: GlobalConfig）
#[tauri::command]
fn config_get(state: State<AppState>) -> Result<GlobalConfig, String> {
    state.db.get_global_config() // 迁移自 electron/controllers/config-controller.ts + repository
}
```

前端统一由 `window.api.xxx` → `invoke('xxx')`（`@tauri-apps/api/core`），并在 `src/shared` 保留薄封装保持调用面稳定。

---

## 3. 主进程分层迁移映射

### controllers/ → Rust commands + state
- Electron 侧 `*Controller.ts` 注册 `ipcMain.handle` 并做入参校验 → Rust 侧 `#[tauri::command]` + 参数校验 `Result<_, String>`。
- 共享状态（数据库连接、单实例租约、并发锁）存入 `AppState`（`tauri::State`），替代 Node 模块级单例。

### repositories/ → Rust 数据库层
- better-sqlite3 → `rusqlite`（`features=["bundled"]` 免系统依赖）。保持「一域一仓」结构：`blueprint_repository`、`character_roster_repository`、`finalization_repository`…
- controller/Rust command **不得直接写 SQL**，统一走 repository 模块。
- SQLite 数据库文件名、项目目录路径仍遵循 `src/shared/project-paths.ts` / `project-context.ts` 约定。写入路径改由 `app.path().app_data_dir()` 定位数据根。

### services/ → 领域服务
- `finalization-service`、`chapter-deletion-service`：**定稿不可变与专属删除生命周期必须原样保留**（ADR 0011），迁移到 Rust 时写等量单元测试。
- `update-*/electron-updater`：迁移为 `tauri-plugin-updater`（或先保留发布页跳转，见 README：macOS 只打开 Release 页）。
- `import-*`、`manuscript-publisher`、`skin-service`、`model-discovery`：逐模块迁移，逻辑平移。

### llm/ → LLM 适配
- Electron 侧 `electron/llm/{provider.interface,llm-factory,openai-provider,gemini-provider,generation-parameter-policy}.ts`。
- 迁移策略（二选一，需 Ask first）：
  1. **Rust 重写**：用 `reqwest` 直接请求 OpenAI-compatible/Gemini 端点，复刻 `generation-parameter-policy`。工作量大但彻底。
  2. **Node sidecar**：保留 Node LLM 实现，经 `tauri-plugin-shell` 作为 sidecar 调起 —— 简单，但削弱 Tauri 收益。**推荐先以 (1) 为目标**，若时间受限再降级。

### security/ → 文件系统安全
- 外部文件须显式授权（ADR 0002，`external-file-grant`）。Tauri 侧用 `tauri-plugin-fs` 的 scope 白名单 + 自研 `external_file_grant` 校验；macOS 原生 `darwin-safe-file-system.m` / Windows PowerShell 封装改写为 Tauri fs 层，**勿绕过授权校验**。

---

## 4. 渲染进程（前端）调整最小化

- React 组件、stores、Tailwind：**原样保留**。
- 唯一改动点：
  - 去除所有 `window.api` / Electron preload 依赖，改为 `@tauri-apps/api/core.invoke(...)` + `@tauri-apps/api/event`。
  - 编辑器（Monaco/Codemirror）、渲染与领域逻辑纯前端，不动。
  - 共享类型 `src/shared/*` 复用；前端封装一层 invoke 以便未来可测试。

**写作工作流链路**（renderer 编排，跨 Electron→Tauri 不变）：前提 → 角色/世界观 → 大纲/蓝图 → 章节草稿 → 审稿 → 修订/定稿 → 下一章上下文；批量任务 `batch-chapter-workflow.ts` 1–10 章/暂停/断点。

---

## 5. 实施步骤（建议按 controller 粒度迭代）

**阶段 0 — 脚手架**（Ask first：会新增 `src-tauri/` 与依赖）
1. `pnpm create tauri-app`（或手搭 Tauri 2 + Vite 前端）。
2. 保留现有 `src/` 前端与 Vite 配置，接入 `tauri.conf.json` 前端 `beforeDevCommand: pnpm dev` / build 输出。
3. 引入 `rusqlite`（bundled）、`serde`、`tauri-plugin-{updater,shell,fs,notification,store}` 按需。

**阶段 1 — IPC 契约盘点**
4. 从 `src/shared/ipc-channels.ts` 生成 channel/类型清单，作为命令注册表。
5. 搭好 `AppState` + `invoke_handler` 骨架。

**阶段 2 — 按 controller 迁移（核心）**
6. 每批迁移 1–2 个 controller：
   - `config` → `project` → `db` → `skin` → `window` → `llm` → `finalization` → `chapter-lifecycle` → `kb` → `import` …（参考 `electron/ipc-handlers.ts` 注册顺序，先易后难）。
7. 每个命令迁移后：前端对应功能跑通 + Rust 单元测试 + `cargo test` + `pnpm typecheck`/`pnpm test`（相关）。
8. 数据库层同步迁移对应 `*_repository.rs`，跑同域 `*.test.ts` 逻辑的 Rust 等价测试。

**阶段 3 — 原生绑定收尾**
9. 窗口/菜单/通知/更新迁移到 Tauri 2 插件；多平台构建工作流为 Tauri 新增 CI 路径（`tauri-action`）。

**阶段 4 — 双栈并存验证**
10. **迁移仅新增 Tauri**：`src-tauri/` 为新增目录，不删除、不覆盖 Electron 实现。迁移期间两栈均可运行，逐模块比对行为一致。
11. **保留 Electron 用于同步上游**：原 `electron/`、`src/` 保持与原仓库同源，作为 `upstream` 合入的对照；每次 `git fetch upstream` 合并时，Tauri 侧仅在 `upstream` 无改动时才随之更新，避免迁移工作与上游演进互相冲突。
12. Electron 代码**永不删除**（❌ 不因迁移完成而移除 `electron/`），它是上游同步的锚点；如需清理需 Ask first 并评估上游同步影响。

---

## 6. Rust 代码规范

- 命名：函数/变量 `snake_case`，类型 `PascalCase`；命令名 kebab-case。
- 错误：`Result<T, String>` 起步，复杂域再自定义 `enum Error + impl Display + Serialize`，保证前端可读。
- 序列化：结构体派生 `Serialize`/`Deserialize`（`serde`），字段与 `src/shared` 前端类型对齐。
- 异步：LLM/网络用 `async fn` + `tauri::async_runtime`；本地 SQL 同步即可（rusqlite 非 async，避免无谓 async）。
- 测试：命令/仓库层 Rust 单元测试，命名 `<fn>_test`，放同模块 `#[cfg(test)] mod tests`。

```rust
// 规范示例
#[derive(Serialize)] #[serde(rename_all = "camelCase")]
pub struct FinalizedReceipt { pub chapter_no: u32, pub body_hash: String }

#[tauri::command]
pub async fn chapter_finalize(
    state: State<'_, AppState>, chapter_id: String, body: String,
) -> Result<FinalizedReceipt, String> {
    let repo = state.finalization_repo();
    repo.finalize(&chapter_id, &body).map_err(|e| e.to_string())
}
```

---

## 7. 测试约定

- **前端/TS**：vitest，`*.test.ts` 与源同目录；必跑 `pnpm typecheck` + 相关 `pnpm test` + `pnpm run lint`（零告警）。
- **Rust**：`cargo test`（`src-tauri/`），覆盖最终化不可变、数据库迁移 barrier、外部文件授权边界（平移自现有 `electron/**/*.test.ts` 意图）。
- 迁移验收：每迁移一个 controller，用现有 Electron 版本作对照，验证 Tauri 对应功能行为一致（可用现有 smoke/`renderer-surface:e2e` 辅助核对）；验收通过仅代表该模块新增 Tauri 支持，不删除 Electron 版本。
- 勿把小说正文、模型 Key 写入测试 fixture/日志。

---

## 8. 开发前必读

1. `docs/README.md`（文档权威层级与导航）
2. `docs/product-domain.md`（领域词汇与边界——迁移不得偏离）
3. `docs-fork/research/2026-10-04-tauri-migration-evaluation.md`（迁移评估决策）
4. 相关 `docs/adr/`（0001 会话租约/0002 外部文件/0008 事实与工作流 seam/0011 定稿删除/0013 连续性投影/0015 skill 作用域/0017 角色状态来源）
5. `docs/agents/issue-tracker.md`（用 gh 操作 Issue，迁移任务按此建 ticket）
6. 根 `AGENTS.md`（命令/风格/Git）
7. `docs/handoffs/` 下**最新日期**的进度快照（接续工作的唯一入口；命名与维护规则见本文 §9）

**工作区边界**：接管任务先 `git worktree list --porcelain`，记录分支/SHA/`git status`；勿把其他 worktree 未提交内容当当前事实。迁移 PR 按 controller 拆分（`feat(tauri): migrate <module>`），一个 PR 一个主题。

---

## 9. 交接快照命名与维护规则

`docs-fork/handoffs/` 下的进度快照是接续工作的**唯一入口文档**，按日期冻结。**硬规则**：

1. **命名**：`docs-fork/handoffs/YYYY-MM-DD-tauri-migration-status.md`，日期取**撰写当日**。
2. **一日一新文件**：每个有实质进展的工作日**新建**一份快照；**不得**在旧日期文件里追加当日章节。
   同一份文件内**只允许一段 `## 本次更新`**：同日再有进展请**改写该段**（或在其 `### N. 收尾` 里补一行），
   **不得新增第二段**。快照是**接续入口**而非工作日志——详细过程由 Git 提交承载，旧段落不得留在文件里。
3. **旧文件冻结**：新快照产生后，旧文件即该日期的**冻结快照**，**不允许任何修改**（包括快照表、章节与头部）。
   历史轨迹由 Git 承担，不再用文件内指针维护。
   **唯一允许的追溯修正**：对历史快照做**无损结构收敛**（同文件内多段更新降级为子节、校正表头日期等），
   **不得删除正文**；删除只允许针对「复制旧正文」类段落，且须先确认。
4. **新快照结构**（沿用既有格式）：
   - 头部：`# Tauri 迁移进度快照（YYYY-MM-DD）` + `> **用途**` 说明 + 本规则指向 + 模板链接 +
     一行「全部历史快照见 `docs-fork/handoffs/` 目录」+ channel 级盘点链接；
   - `## 快照（最后更新：YYYY-MM-DD · 第 N 次）`：**活表格**（当前阶段 / 已注册命令 / GUI 冒烟 /
     自动化回归 / 双栈隔离 / 验证状态），随当日进展更新；
   - `## 本次更新（第 N 次：<批次> — <主题>）`：当日改动明细（决策 / 模块 / 接线 / 验证），
     末尾附 `### N. 收尾`（提交清单、工作区清理、规则变更）；
   - 尾部**滚动章节**（均为**无编号二级标题**，内容自上一份承接并更新）：
     `## 交接给下次会话`（下含 `### 1. 当前工作区状态` / `### 2. 下一步` / `### 3. 阻塞项与待授权项` /
     `### 4. 红线提醒` / `### 5. 自检记录`）。
     历史导航**不由快照维护**（规则 3）：头部一句「全部历史快照见 `docs-fork/handoffs/` 目录」即可，
     **不得**新增 `## 历史指针` 段（每份都要手工搬运旧表 = 复制旧正文，已废弃）。
     ⚠️ `### 5. 自检记录` 是**交接块的子节**（规则 8 的「§5 自检记录」即指此），不得写成同名 `##` 标题；
     废弃的是「带子节编号却用 `##` 层级」的旧写法（曾出现 `## 5. 自检记录` / `## 6. 历史指针`）——
     那会让编号看起来继承另一个父级、大纲断裂。
5. **章节序号全局递增**：`第 N 次` 跨文件连续，跨日后续上一份的 `N+1`，不重开计数。
6. **同步两处指针**：根 `AGENTS.md`「接续任务先看进度」、
   `docs-fork/plans/tauri-migration-channel-inventory.md`（仅当频道口径变化时）。
7. **简版原则**：快照是接续入口，不是工作总结。只保留 —— 当前状态 / 当日新增摘要 / 交接步骤 /
   阻塞与待授权项 / 红线提醒 / 自检记录。**不得复制**旧日期正文；**建议提交拆分表**、**逐条测试覆盖**、
   **基线对照逐项明细**一律外链（`详见 commit message` / `docs-fork/research/xxx`），不在快照正文展开。
   **不写勘误记录**：对后续交接无意（历史轨迹在 Git，当前值以活表格为准）；旧数字直接改掉，不保留错误值。
8. **数字硬约束**：顶部活表格只填**最新一次自检的实测值**。改表前**必须重跑**快照 `§5 自检记录`里的
   命令，把输出贴入 §5 再回填表格；不得沿用旧数字、不得估算、不得从上一份快照拷贝。
   发现旧数字错误时**直接改正**（不保留错误值、不写勘误段落，见第 7 条）。
9. **交接必填四项**：工作区是否干净、HEAD sha、**上一份快照中已过期的交接描述**（防误导）、
   下一步 1-2-3（含前置条件，如「Ask first」「需先评估」）。
10. **阻塞项与红线只增不删**：阻塞项表与红线提醒从上一份**逐条承接**；已解除的写
    「✅ 已解除（依据）」而**非删行**；新增红线必须标注来源。
11. **模板**：新建快照一律从 `docs-fork/handoffs/_TEMPLATE-tauri-migration-status.md` 复制；
    模板末尾附「维护者检查清单」，提交前逐项打勾。
12. **路径迁移例外**：当 fork 文档整体迁移目录（如 `docs/` → `docs-fork/`）时，历史快照中的
    **路径字符串**允许批量替换，且**仅限路径字符串** —— 不得改动任何正文、数字、表格或结论。
    受影响快照须在首行加一行路径说明（注明迁移日期与例外依据）。
    该例外是第 3 条「旧文件冻结」的**唯一豁免**，其余情形一律不得触碰旧快照。

> 首次按本规则产生的快照：`docs-fork/handoffs/2026-10-08-tauri-migration-status.md`（第二十三次，批次 D2-b）。
> 首次按新增第 7–11 条（简版原则 / 数字硬约束 / 模板）精简的快照：`docs-fork/handoffs/2026-10-09-tauri-migration-status.md`。
> **2026-10-09 规则变更**：旧文件改为「不允许任何修改」（取消前向指针）；快照不得写勘误记录。
> **2026-10-09 路径迁移**：fork 文档整体从 `docs/` 迁至 `docs-fork/`；
> 历史快照中的路径字符串按第 12 条批量更新，正文未改。

---

## 10. Git 提交消息规范与提交前自检

**格式**：`type(scope): 描述`。

- `type` ∈ `feat` / `fix` / `chore` / `docs` / `refactor` / `test` / `perf` / `build` / `ci` /
  `style` / `revert`（白名单与 `scripts/check-commit-msg.mjs` 的 `COMMIT_TYPES`、`AGENTS.md`
  「Git 工作流」**同源**：改一处必须同步其余两处）；
- `scope` 小写字母/数字/连字符，Tauri 迁移统一用 `tauri`（如 `feat(tauri): migrate config controller`）；
- **CI / 工作流类改动统一写 `chore(ci): 描述`**：`ci` 只作 scope，不写裸 `ci:` 类型前缀；
  与仓库 Gitleaks 系列提交（`chore(ci): 新增 Gitleaks 秘密扫描工作流` 等）保持一致。
  上游既存的裸 `ci:` 提交不追溯改写（2026-10-09 已将本 fork 最后一条 `53c54c7` 重写为
  `5ef8d64 chore(ci): 修复 Gitleaks 配置位置并添加排除规则`）；
- 描述用中文；首行与正文之间**必须留一个空行**；首行显示宽度 ≤ 120 列（CJK 计 2 列）。
- **编码硬约束**：提交消息**无 BOM、无 CRLF、无行尾空白**。
- `Merge ` / `Revert ` / `fixup! ` / `squash! ` 开头的首行只做编码检查
  （上游历史与合并提交不受本仓库前缀规范约束）。

### 10.1 事故记录：提交消息混入 UTF-8 BOM（2026-10-08，已修复）

批次 D2-c 的两个提交（`7105b13` / `612002b`）在消息文件写入时用了 PowerShell 5.1 的
`Set-Content -Encoding UTF8`，而**该版本默认写 UTF-8 BOM**，于是提交对象的消息首 3 字节
成了 `EF BB BF`：

| 现象 | 说明 |
|---|---|
| `git log` 渲染 | `612002b ﻿docs(tauri): …` —— 前缀前多一个不可见字符，肉眼极易漏看 |
| 首行真实内容 | `"\uFEFFfeat(tauri): …"`，类型前缀实为 `"\uFEFFfeat"`，**不是** `"feat"` |
| 影响 | 任何按 `type(scope):` 解析的工具都识别不到；`check:channels` 之类的自检也发现不了 |
| 定位方式 | 仅 `git cat-file commit <sha>` 的**字节级**比对可得（`git log --format=%B` 的渲染结果不可信） |
| 修复 | 两个提交当时均未推送，用 `git commit-tree` 重放剥离 BOM：`tree` / 作者 / 提交者 / 时间戳**逐字节保留**，仅 SHA 变化（`867cbba` / `d4a9ff3`） |

**正确写法**（任选）：

```bash
git commit -m "feat(tauri): 描述"        # 首选，最简单
```

```js
// 需要多行正文时：用 Node 写消息文件（Node 的 'utf8' 不写 BOM），再以 -F 读入
fs.writeFileSync(messageFile, message, 'utf8')
```

```bash
git commit -F "$messageFile"
```

**禁止**：用 PowerShell 5.1 的 `Set-Content -Encoding UTF8` / `Out-File -Encoding UTF8`
写提交消息文件（二者默认带 BOM）。需要无 BOM 时用 `-Encoding utf8NoBOM`（PowerShell 6+）
或改用 Node。

### 10.2 自动检查

| 场景 | 命令 |
|---|---|
| 本地 hook（可选，不入库） | `printf '#!/bin/sh\nexec node scripts/check-commit-msg.mjs "$1"\n' > .git/hooks/commit-msg && chmod +x .git/hooks/commit-msg` |
| 手动检查一段范围 | `node scripts/check-commit-msg.mjs --range <base>..HEAD` |
| 检查单条消息 | `node scripts/check-commit-msg.mjs <msgfile>` |
| CI 手动审计历史 | GitHub → Actions → **Commit message CI** → Run workflow，填 `range`（如 `d973c19..HEAD`）；同步上游时可再填 `ignore_authors`（逗号分隔邮箱） |

CI 侧由 `.github/workflows/commit-message-ci.yml` 承担：push 检查 `before..after`，
pull_request 检查 `base..HEAD`。**该 workflow 刻意不设 `paths-ignore`** ——
纯文档 PR 也必须检查，因为本次事故恰好发生在 docs 提交上。

**只审计增量**：上游 `EthanYoQ/AI-Novel-Writer` 的历史里有大量不符合本规范的提交
（`Update README.md` 等，共 38 条），因此检查范围必须是 `base..HEAD` 这类增量区间，
**不要**用 `--range HEAD`（会遍历全部历史）。若分支合并了上游并带入那些提交，
用 `--ignore-author <email>` 跳过。
