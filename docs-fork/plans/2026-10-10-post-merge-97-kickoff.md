# 上游合并后新批次开工清单 —— 97 个未迁移 invoke 频道（+2 个新事件）

> **状态**：开工准备（2026-10-10）。分批与顺序**待 4 份只读调研报告回填后定稿**（§4）。
> **前置**：上游 `master` 合并已收口并推送（`7bab3727` + `7e1ed75e` + `36b80d6e`），快照见
> [`docs-fork/handoffs/2026-10-10-tauri-migration-status.md`](../handoffs/2026-10-10-tauri-migration-status.md)（第四十四次）。
> **契约单源**：`tauri-app/src/shared/ipc-channels.ts`（模块合成，经 `tauri-app/scripts/channel-contract.mjs` 递归收集）。
> **口径复现**：`cd tauri-app && node scripts/verify-channel-coverage.mjs`（未迁移清单与按域计数由该脚本打印）。

## 0. 背景与口径

上游这次合并把契约从单文件扩成 **28 个 `*Channels` 模块合成**，新域带来的频道在 Tauri 侧**从未迁移**：

| 项 | 数值 |
|---|---|
| 契约 invoke 频道 | **290** |
| 契约事件频道 | **7**（原 5 + 上游新增 `generation:snapshot`、`generation:reasoning`） |
| 已注册命令 / 覆盖 invoke | 194 / **193** |
| **未迁移 invoke** | **97** |
| orphan | 空 ✅ |

按域计数（脚本打印，2026-10-10 实测）：

| 域 | 数 | 域 | 数 | 域 | 数 |
|---|---|---|---|---|---|
| generation | 21 | agent-generation | 8 | cloud-backup | 8 |
| legacy-roster | 7 | character-proposal | 6 | finalization-generation | 5 |
| finalized-character | 5 | graph-generation | 5 | project | 5 |
| character-avatar | 4 | dialog | 4 | editor-inline | 4 |
| review-revision | 4 | kb | 3 | startup | 3 |
| db | 2 | import-generation | 2 | character-identity | 1 |

## 1. 未迁移频道总表（按契约模块，含 args 概览）

> args 为契约源码块首行的摘要，**权威以 `tauri-app/src/shared/<模块>.ts` 为准**；
> 上游合并后 `args` 一律按**位置参数 → Tauri 命名参数**映射，需同步登记
> `tauri-app/src/services/ipc-client.ts` 的 `CHANNEL_ARG_NAMES`（未登记的非空参频道会立即抛错）。

### 1.1 `shared/generation-owner-contract.ts`（21 + 2 事件）

`generation:prepare-draft-context`、`generation:commit-draft`、`generation:read-context`、
`generation:retry-draft-short-outline`、`generation:begin-batch`、`generation:read-batch`、
`generation:list-batches`、`generation:confirm-batch-finalization`、`generation:list-directory-progress`、
`generation:compose-visible`、`generation:read-visible-composition`、`generation:begin`、
`generation:bind-material-decision`、`generation:execute`、`generation:read`、`generation:list`、
`generation:cancel`、`generation:pause`、`generation:resume`、`generation:restart`、`generation:discard-candidate`。

事件：`generation:snapshot`、`generation:reasoning`（契约 interface `GenerationOwnerEvents`）。

### 1.2 派生 generation 域（40）

| 契约模块 | 数 | 频道 |
|---|---|---|
| `agent-generation.ts` | 8 | `begin` / `read` / `resume` / `round` / `claim-tool` / `finish-tool` / `register-workflow` / `commit-domain-tool` |
| `editor-inline-generation.ts` | 4 | `begin` / `read-recovery` / `execute` / `cancel` |
| `finalization-generation.ts` | 5 | `read` / `begin` / `execute` / `commit` / `cancel` |
| `graph-generation.ts` | 5 | `begin` / `read` / `execute` / `confirm` / `cancel` |
| `legacy-roster-generation.ts` | 7 | `read-source` / `adopt-existing` / `begin` / `read` / `execute` / `stage` / `cancel` |
| `finalized-character-generation.ts` | 5 | `read-context` / `commit` / `list-state-candidates` / `read-state-candidate` / `decide-state-candidate` |
| `import-generation.ts` | 2 | `read` / `execute` |
| `review-revision-generation.ts` | 4 | `prepare` / `commit-review` / `commit-revision` / `read-recovery` |

### 1.3 角色与云备份（19）

| 契约模块 | 数 | 频道 |
|---|---|---|
| `character-proposal.ts` | 6 | `stage` / `read` / `list-pending-finalized` / `read-pending-finalized` / `approve` / `cancel` |
| （同文件）`character-identity` | 1 | `character-identity:read` |
| `character-avatar.ts` | 4 | `choose` / `commit` / `read-batch` / `remove` |
| `cloud-backup.ts` | 8 | `view` / `connect` / `confirm-binding` / `backup` / `list` / `restore-copy` / `cancel` / `clear-credential` |

### 1.4 项目归档 / 旧版导入 / 启动态 / 文档副本（17）

| 契约模块 | 数 | 频道 |
|---|---|---|
| `ipc-channels.ts` | 14 | `dialog:select-legacy-project`、`project:import-legacy-copy`、`project:peek-overview`、`project:overview-current`、`project:archive-export`、`project:archive-restore`、`dialog:select-project-archive-export`、`dialog:select-project-archive`、`dialog:select-project-restore-target`、`db:project-core-commit-generated`、`db:review-cycle-get`、`kb:read-document-copy`、`kb:save-document-copy`、`kb:reindex-document-copy` |
| `startup-contract.ts` | 3 | `startup:get-state`、`startup:skin-snapshot`、`startup:appearance-ack` |

## 2. 分批规划（初步，待 §4 回填定稿）

| 分批 | 内容 | 频道数 | 依赖 / 风险 |
|---|---|---|---|
| **I-1** | 启动态 3 + 项目归档/旧版导入/概览/文档副本 14 | 17 | 与已迁移面最近；涉及归档格式与 `.vela` 口径冲突、外部文件授权；**新表**：`review_cycles` 等（`db:review-cycle-get`） |
| **I-2** | generation owner 核心 21 + 事件 2 | 23 | **最大块**；其余派生域多依赖其对外接口面；**新表**：`generation_roots/runs/attempts/artifacts` |
| **I-3** | 派生 generation 40（agent / editor-inline / finalization / graph / legacy-roster / finalized-character / import / review-revision） | 40 | 复用 I-2 的执行框架；逐域可再拆子批 |
| **I-4** | 角色身份/提案/头像 11 | 11 | **新表**：`character_identity_*` 7 张 + `character_avatar_*` 2 张；头像需图片处理能力评估 |
| **I-5** | 云端备份 8 | 8 | 凭据存储与远端协议需专项评估（可能需新 crate/插件）；**新表/候选**：`import_effect_ledger` 等 |

**顺序建议（初步）**：I-1（近场、铺开新表流程）→ I-2 → I-3 → I-4 → I-5；
最终顺序与每批内部拆分以 §4 调研结论与我方复验为准。

## 3. 数据库 schema 扩展授权（**已有**）

- **用户 2026-10-10 决策（ask 卡 `schema_auth`）＝ 「整批授权」**：按上游 `electron/migrations/m01–m05` **逐批按需**向
  `tauri-app/src-tauri/src/db/schema.rs` 追加该批必需的表，**不再逐表询问**；每批必须在快照中登记实际追加的表与索引。
- 现状：`db/schema.rs` **36 张表 / 17 索引**；上游共 51 张（含迁移临时表与刻意不建者）。
- 上游缺口（待逐批核实后按需追加）：
  - `electron/migrations/m01-generation-runs.ts`：`generation_roots` / `generation_runs` / `generation_attempts` / `generation_artifacts`
  - `electron/migrations/m02-character-identity.ts`：`character_identity_origins` / `character_aliases` / `character_identity_proposals` / `character_identity_approvals` / `character_relationships` / `character_identity_meta`（+ 迁移临时表 `characters_m02`）
  - `electron/migrations/m03-review-cycle.ts`：`review_cycles` / `review_findings` / `review_cycle_merges`
  - `electron/migrations/m04-import-effect-ledger.ts`：`import_effect_ledger`
  - `electron/migrations/m05-character-assets.ts`：`character_avatar_assets` / `character_avatar_unresolved`
  - 另有 `import_legacy_identity_bridge`（**历史决策刻意不建**，需在新频道语境下复核）、`import_runs_stage_v3`、`text_metric_versions`。
- **红线不变**：`docs/` 永不改；项目库仍是 `<root>/.lore/lorekeeper.db`；禁读 `AI_NOVEL_VELA_HOME`、禁写 `.vela`。

## 4. 调研报告索引（只读子代理，并行 4 份）

| # | 范围 | runId | 报告路径 |
|---|---|---|---|
| R1 | generation owner 核心（21 + 2 事件） | `del_mv2g1fy3_riub` | `C:\Users\tanca\AppData\Local\Temp\lk-research-gen-core.md` |
| R2 | 派生 generation（40） | `del_mv2g1kgw_tjjw` | `C:\Users\tanca\AppData\Local\Temp\lk-research-gen-derived.md` |
| R3 | 角色身份/提案/头像 + 云备份（19） | `del_mv2g1ori_0cgt` | `C:\Users\tanca\AppData\Local\Temp\lk-research-character-cloud.md` |
| R4 | 归档/旧版导入/启动态/文档副本（17） | `del_mv2g1tpz_h00k` | `C:\Users\tanca\AppData\Local\Temp\lk-research-project-startup.md` |

每份报告的必备 7 节：逐频道语义 / 上游实现文件与行号 / 依赖表（是否已存在）/ 外部能力与新增 crate 结论 /
前端调用点 / Rust 移植建议与子批次 / 风险与刻意偏离候选（D-*）。

**回填任务**：4 份报告回来后，把 §2 的分批细化（每批的 Rust 文件面、命令签名要点、表清单、D-* 候选）
并据此派发实现子代理。

## 5. 硬约束与验收口径（每批通用）

- **一次一批**：每批由子代理实现（`tokendance/ling-3.1-flash`），编排者独立复验后才提交；禁子代理跑 `git add/commit`。
- **红线**：`docs/` 永不改；`tauri-app` 渲染层不得依赖 Electron 运行时 API；跨进程类型走 `@baseline/*`；
  全局根 `~/.lorekeeper`；项目目录 `<root>/.lore/`（库 `.lore/lorekeeper.db`，KB 向量 `.lore/kb/`）。
- **MUTATING 失败文案**带 `"Error: "` 前缀；`skills:*` 与 `prompt:load-global` 的 diagnostics **不带**。
- **每批自检**（编排者亲自重跑）：
  - `tauri-app/src-tauri/`：`cargo fmt --check`（0 行）、`cargo check --all-targets`（0 告警）、`cargo test --lib`（基线 **694/694**，只增不减）
  - `tauri-app/`：`node scripts/verify-channel-coverage.mjs`（未迁移数按下批目标递减）、`node scripts/verify-channel-coverage.mjs --emit`（无 diff）、`pnpm typecheck`、`npx eslint .`、`npx vitest run test/channel-migration-coverage.test.ts`
- **快照**：每批收口更新 `docs-fork/handoffs/` 当日快照（新增 §、活表格数字、提交表、推送状态），并登记本批 D-* 与新增表。

## 6. 待办（本清单维护）

- [ ] 回填 §2 分批细化（待 4 份调研报告）
- [ ] 定稿 I-1…I-5 顺序与内部拆分
- [ ] 派发 I-1 实现子代理 → 复验 → 提交
- [ ] 每批收口后更新快照并递减未迁移数
