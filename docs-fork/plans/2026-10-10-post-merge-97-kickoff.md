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

## 2. 分批规划（定稿 2026-10-10，R1–R4 回填）

**依赖图（顺序依据）**：
- **共同前置**：m01 generation 账本四表 + m02 `characters` 重建（`character_id` PK）+ 六辅助表 + `generation_run_repository` + owner 脚手架 —— generation 核心（21）、角色身份/提案（7）、派生 generation（40）全部依赖（R1-D-8 / R2-D-1 / R2-D-2）。
- m03 三表 = review-revision（4）+ `db:review-cycle-get` 前置（R2-D-3 / R4 §3）。
- `assertSourcesCurrent`/`withGenerationAgentChildEffect`（B3）= `db:project-core-commit-generated` 前置（R4 §1-10）。
- `exportPortableProject`/`restorePortableProject`（B7）= `cloud-backup:restore-copy` 硬前置（R3-D-9）。
- m05 两表 = 头像 4 频道前置；`import_legacy_identity_bridge`/`text_metric_versions` 刻意不建（schema.rs:1314 断言），sanitize/legacy-import 的 `DELETE FROM` 需条件化（B7）。

| 批次 | 频道 | 内容（文件面） | 前置 | 决策点 |
|---|---|---|---|---|
| **B1 生成域基座** | 0 | **W1**：schema.rs +m01 四表；新建 `repositories/generation_run_repository.rs`。**W2**：schema.rs +m02 六表 + `characters` 重建为 m02 形态；适配 `character_repository.rs`/`character_roster_repository.rs`/`finalized_continuity_repository.rs`/`project_clear_repository.rs`。**W3**：新建 `services/main_generation_owner.rs` 骨架（per-database owner 缓存 + authorizedOwner 门控 + 错误映射 + suspendForProjectClose 钩子 + 事件 emit 接线点） | 无 | m02 直接 CREATE 基线（已定，不建迁移运行时） |
| **B2 角色身份/提案 + generation 读面** | 14 | 新建 `commands/character_proposal.rs`（7 命令：proposal 6 + `character-identity:read`）、`repositories/character_identity_repository.rs`、`services/character_proposal.rs`；`commands/generation.rs` 读面 7 命令（G1）；owner 读面方法 | B1 | — |
| **B3 generation 写面 + 执行面 + 事件** | 13 + 2 事件 | G2 纯逻辑层 `services/generation_{source_binding,plan,hash}.rs`；G3 写面 9 命令；G4 执行 4 命令；事件 emit `generation:snapshot`/`generation:reasoning`；`services/draft_effects.rs` | B2 | D-3 方案 (c) service 层直调（已定）；D-2 广播+客户端过滤（已定） |
| **B4 知识源** | 1 | `services/knowledge_source.rs` 重写（FTS5+HNSW+jieba 方案 B 等价语义）+ `generation:prepare-draft-context` | B2 | D-1 方案 B 等价语义（已定，登记刻意偏离） |
| **B5 角色头像** | 4 | schema.rs +m05 两表；新建 `repositories/character_asset_repository.rs`、`services/character_asset.rs`、`services/avatar_image.rs`、`commands/character_avatar.rs` | B1 | **Ask first**：image crate（nativeImage 压缩替代） |
| **B6 派生 generation** | 40 | 按 R2-G1..G8 八个子批：import 2 / legacy-roster 7 / review-revision 4 / finalized-character 5 / finalization-gen 5 / graph 5 / editor-inline 4 / agent 8 | B1+B2+B3 | D-5 类型化入口（定）；D-11 错误码白名单复刻 Electron 映射（定）；D-4 contextId 进程内存态不持久化（定） |
| **B7 归档/旧版导入/启动态/文档副本** | 17 | schema.rs +m03 三表 +m04 `import_effect_ledger`；sanitize/legacy-import 条件化 DELETE；手写 ZIP（STORED）+ CRC32；`external_grant.rs` 新增 4 用途变体；startup 三频道（trusted-sender 等价）；kb document-copy 3 频道 | B1+B2+B3 | **Ask first**：zip/crc32fast crate、VACUUM INTO 替代 better-sqlite3 在线 backup、trusted-sender 等价物 |
| **B8 云备份** | 8 | 新建 `src/cloud_backup/{mod,webdav,credential,binding}.rs` + `commands/cloud_backup.rs` | B7 | **Ask first**：safeStorage 替代（windows-sys DPAPI / keyring / session-only）、XML crate（quick-xml / roxmltree） |

**schema 扩展登记**（整批授权范围内）：B1 追加 m01 四表 + m02 六表 + `characters` 重建；B5 追加 m05 两表；B7 追加 m03 三表 + m04 一表。`import_legacy_identity_bridge`/`text_metric_versions`/`import_runs_stage_v3` 不建（维持历史决策）。

**上游版本号事实**：上游 `CURRENT_DESKTOP_SCHEMA_VERSION = 7`（desktop-registry.ts:16），m01–m05 是版本 7 内的「installed lane」迁移，不 bump 版本号 → Tauri 侧无版本常量需改，portable 归档 schemaVersion=7 兼容。

**B1 内部波次**：W1 串行（纯增量，验收 694 绿）→ W2 ∥ W3 并行（文件面不重叠：W2 改 5 个现有文件、W3 建新文件；W3 验证降级为 `cargo check`，`cargo test` 由编排者在 W2 收口后统一跑，避免半成品干扰）。
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
**✅ 已回填（2026-10-10）**：§2 定稿 B1–B8 与依赖图；B1 三波次（W1 串行 → W2∥W3 并行）启动中，实现子代理 runId 见当日快照。
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
