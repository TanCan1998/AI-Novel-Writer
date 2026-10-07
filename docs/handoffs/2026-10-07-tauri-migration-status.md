# Tauri 迁移进度快照（2026-10-07）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> 批次 A–C `project_core` / `characters` 的历史细节见
> [`2026-10-06-tauri-migration-status.md`](./2026-10-06-tauri-migration-status.md)；
> channel 级盘点见 [`docs/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-07 · 第十八次）

| 项 | 值 |
|---|---|
| 仓库 | **`TanCan1998/Lorekeeper`**（原名 `AI-Novel-Writer`；仍为 `EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork） |
| 分支 | `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm 包 `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 当前阶段 | **批次 C 进行中**：`project_core` ✅、`characters/roster` ✅（`69fc50c`）、`blueprints` 11 ✅、`drafts` 12/16 ✅、`revisions` 9 ✅、`reviews` 5 ✅、`post-process` 6 ✅、**`llm 日志/摘要` 5 频道 ✅（2026-10-07）**；下一步 **`project 清理`（2 频道）** |
| 已注册命令 | **89**（骨架 1 + A 11 + B 22 + C：project_core 4 + characters 3 + blueprints 11 + drafts 12 + revisions 9 + reviews 5 + post-process 6 + llm/摘要 5） |
| 双栈隔离 | L0 安装标识 / L1 `~/.lorekeeper` / L2 `<root>/.vela/lorekeeper.db` 均独立；L3（`.vela` 改名）押后 |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc @ `D:\Environment\rust\`（脚本内显式设 `RUSTUP_HOME`/`CARGO_HOME`） |
| 验证状态 | ✅ `cargo check --all-targets` **0 告警** · ✅ `cargo test --lib` **160/160** · ✅ `pnpm typecheck` exit 0 · ✅ `pnpm run lint` exit 0 |

---

## 本次更新（第十一次：`blueprints` S2-a — 基础读写 7 频道）

### ⚠️ 首先修复了上次中断遗留的损坏在途改动

上个会话的未提交改动**无法编译**，本轮先修正再续写：

1. **`src-tauri/src/db/schema.rs` 的 `CREATE_BLUEPRINTS` 原始字符串未闭合** —— 后续的
   `pub const CREATE_BLUEPRINT_COMMIT_OPERATIONS` / `..._CHARACTER_SYNC_OPERATIONS` 声明
   被当作字符串内容吞入，`"#` 提前终止导致语法错误、且两个常量实际不存在。已正确闭合，
   并为两张新表补上**基线外键**（`FOREIGN KEY ... ON DELETE CASCADE`）与索引。
2. **`src-tauri/src/repositories/blueprint_repository.rs` 是臆造代码**（约 408 行）—— 持有
   `Connection` 克隆、`Statement::get()` 这种不存在的 API、引入未加入 `Cargo.toml` 的
   `chrono` / `serde_urlencoded`、`clear_all` 事务未提交、凭空发明 Response 包装类型
   （契约里 `db:blueprint-get-all` 直接返回 `BlueprintData[]`）。**已整文件重写**。

教训（与快照既有教训一致）：**在途改动接手时不得假定可编译，先核验再续写**；
长文件写入污染（重复定义/残缺片段）仍须在接线前扫描。

### 1. `repositories/blueprint_repository.rs`（重写，S2-a 范围）

逐条对齐 `electron/repositories/blueprint-repository.ts` 的**基础读写**：

| 函数 | 行为 |
|---|---|
| `get_all` | `ORDER BY chapter_number ASC`；`characters` JSON 容错解析（坏 JSON → 空数组） |
| `get_by_chapter` | 返回 `Option<BlueprintData>`（未找到 = 前端 `null`） |
| `upsert` | 10 列 `ON CONFLICT(chapter_number) DO UPDATE`，`updated_at = datetime('now')` |
| `upsert_many` | `unchecked_transaction()` 单事务批量 |
| `update_notes` | 只改 `notes` + 两个时间戳，返回 `changes > 0` |
| `delete` | 按章节号删除 |
| `clear_all` | 事务内调 `clear_blueprint_facts_within_transaction` |
| `clear_blueprint_facts_within_transaction` | 收敛 schema → 子表 → 父表（避让外键），供后续项目清理/导入复用 |

- `BlueprintData` 含 `newCharacterCandidates?` / `relationshipHints?`（非表列，读回恒缺失，
  仅在 S2-b 的提交回执中冻结）。
- **未引入任何新依赖**（无 `chrono`；ISO 时间由 SQLite `datetime('now')` 产生）。

### 2. `db/schema.rs`

- 修复 `CREATE_BLUEPRINTS` 闭合；两张新表 DDL 保留并补基线外键 + 索引。
- 新增 `ensure_blueprint_commit_schema(conn)`：幂等建表 + **回填**（为已有
  `blueprint_commit_operations` 行补建 `blueprint-sync-<op>` 同步操作），
  等价于基线在 blueprint 写路径前的惰性 `ensureBlueprintCommitSchema`；
  `create_tables` 每次打开项目库调用它。

### 3. `commands/db.rs`（+ 7 命令）

- 读频道（失败即 reject）：`db:blueprint-get-all`、`db:blueprint-get`。
- MUTATING（失败走 `mutating_error` → `"Error: {msg}"`）：
  `db:blueprint-upsert`、`db:blueprint-upsert-many`、`db:blueprint-update-notes`、
  `db:blueprint-delete`、`db:blueprint-clear-all`。
- `db:blueprint-update-notes` 用专属信封 `BlueprintUpdateNotesResult { success, updated?, error? }`
  （对齐基线 `{ success: true, updated }`）。
- 门禁链与既有 `db:*` 一致：会话租约 + 冻结 `expectedProjectPath`。

### 4. 接线

- `repositories/mod.rs`：`pub mod blueprint_repository;`
- `lib.rs`：注册 7 命令（38 → **48**，含骨架与 A/B 批次；C 子域合计 14）
- `src/services/ipc-client.ts`：`CHANNEL_ARG_NAMES` 登记 7 个 `db:blueprint-*` 频道

### 5. 验证与测试

- 新增 Rust 测试 **7 个**（仓储 6 + 命令门禁/CRUD 1）：
  `upsert_get_all_and_get_by_chapter`、`upsert_conflict_updates_fields`、
  `upsert_many_commits_every_item`、`update_notes_hits_only_existing_chapter`、
  `delete_and_clear_all`、`ensure_blueprint_commit_schema_backfills_sync_operations`、
  `blueprint_channels_guard_and_crud`。
- 结果：`cargo check --all-targets` 0 告警 · `cargo test --lib` **101/101** ·
  `pnpm typecheck` / `pnpm run lint` 均 exit 0。

---

## 本次更新（第十二次：`blueprints` S2-b — 范围提交 1 频道）

### 1. `repositories/blueprint_repository.rs`（S2-b）

新增（逐条对齐 `electron/repositories/blueprint-repository.ts` 的 `commitRange` 链）：

| 函数 | 对齐基线 |
|---|---|
| `canonicalize_value` / `canonical_json` | `canonicalize`（对象按键排序、数组递归） |
| `commit_payload_hash` | `commitPayloadHash`（SHA-256；复用 `character_roster_repository::hash_text`，已提为 `pub(crate)`） |
| `assert_exact_range` | `assertExactRange`（5 类错误文案逐字一致，含 en dash 范围文案） |
| `read_exact_range` | `readExactRange`（读回后用快照重跑范围断言） |
| `snapshot_with_character_sync_facts` | 同名（叠加冻结的 `relationshipHints` / `newCharacterCandidates`） |
| `same_persisted_blueprint` | 同名（10 个持久化字段，关系载荷不参与） |
| `read_character_sync_operation` / `row_to_character_sync_operation` / `parse_completion_receipt` | 同名（含 pending/completed 与回执的一致性判定） |
| `authoritative_character_sync_completion_receipt` | 同名（roster 状态 + 事实校验 + 操作证据 + 64-hex 校验） |
| `assert_authoritative_character_sync_completion` | 同名（规范 JSON 比对） |
| `blueprint_character_sync_fact_error` + `relationship_facts` / `relationship_satisfied` / `relationship_endpoint` | 对齐 `src/shared/blueprint-character-sync-evidence.ts`（含 `from ?? source` / `to ?? target` 语义、`相关` 缺省、legacy 文本跳过） |
| `commit_range` | `commitRange`（幂等分支 + 新提交分支，单事务，回读一致性校验） |
| `character_sync_operation_id` | 同名（`blueprint-sync-<op>`） |

- **零新增依赖**（SHA-256 走已有 `sha2`，时间戳全部由 SQLite `datetime('now')` 产生）。
- 新回执类型：`BlueprintCommitRangeReceipt` / `BlueprintCharacterSyncOperation` /
  `BlueprintCharacterSyncCompletionReceipt` / `BlueprintCharacterSyncCompletionRosterReceipt`。

### 2. 范围调整说明（相对第十次拆解）

原拆解把 `snapshot_with_character_sync_facts` / `same_persisted_blueprint` /
`read_character_sync_operation` / authoritative 校验归入 S2-c。但 `commit-range` 的**回执构造**与
**幂等读回路径**都直接依赖它们（`assert_authoritative_character_sync_completion` 在
`receipt_from_existing_operation` 中被调用），拆到 S2-c 会造成跨 commit 的行为真空。
故**提前到 S2-b 落地**；S2-c 相应缩为「3 个命令的接线 + `list_pending` 查询 +
`complete` 的 UPDATE 事务」。

### 3. `commands/db.rs` / `lib.rs` / `ipc-client.ts`

- 新增命令 `db:blueprint-commit-range`（MUTATING，失败走 `mutating_error`），
  信封 `BlueprintCommitRangeResult { success, receipt?, error? }`。
- `lib.rs` 注册（**48 → 49**）；`ipc-client.ts` 登记
  `'db:blueprint-commit-range': ['request', 'expectedProjectPath']`。

### 4. 验证与测试

新增 Rust 测试 **8 个**（仓储 7 + 命令 1）：canonical 哈希字段敏感性 / 范围断言 5 类 /
提交-幂等-冲突 / 两种模式的裁剪语义 / 回执冻结非表列字段 / 同步事实错误文案（含 legacy 跳过）/
同步操作状态一致性 / 命令门禁与端到端提交。

结果：`cargo check --all-targets` **0 告警** · `cargo test --lib` **109/109** ·
`pnpm typecheck` / `pnpm run lint` 均 exit 0。

---

## 本次更新（第十三次：`blueprints` S2-c — 角色同步 3 频道，子域收口）

### 1. `repositories/blueprint_repository.rs`（S2-c）

- 抽出 `SYNC_OPERATION_COLUMNS` + `character_sync_operation_row` 共享行读取（三条查询共用，避免列名漂移）。
- `list_pending_character_sync_operations` —— 对齐 `listPendingCharacterSyncOperations`
  （`status = 'pending'`，`ORDER BY created_at ASC, operation_id ASC`）。
- `get_character_sync_operation` —— 对齐 `getCharacterSyncOperation`
  （空 ID → 「蓝图角色同步操作 ID 不能为空」；已完成项跑 authoritative 校验）。
- `complete_character_sync_operation` —— 对齐 `completeCharacterSyncOperation`：
  单事务 + 幂等（已完成直接回读校验）+ `UPDATE ... WHERE status = 'pending'` 变更数校验
  + 回读校验；回执按 `canonical_json` 序列化存储。

### 2. `commands/db.rs` / `lib.rs` / `ipc-client.ts`

- 3 命令：`db:blueprint-character-sync-list-pending`（读）、`-get`（读）、
  `-complete`（MUTATING，信封 `{ success, operation?, error? }`）。
- `lib.rs` 注册（**49 → 52**）；`ipc-client.ts` 登记 3 个频道参数名。

### 3. 验证与测试

新增 Rust 测试 **5 个**（仓储 4 + 命令 1）：列表/单读/空 ID 校验、already-satisfied、
**committed**（以同一 operationId 先跑 `roster::commit`，验证名单证据与幂等重放）、
缺候选拒绝 + 事务回滚、命令门禁端到端。

结果：`cargo check --all-targets` **0 告警** · `cargo test --lib` **114/114** ·
`pnpm typecheck` / `pnpm run lint` 均 exit 0。

### 4. `blueprints` 子域收口状态

| 步骤 | 频道数 | 状态 |
|---|---|---|
| S2-a 基础读写 | 7 | ✅ `50c0fe8` |
| S2-b 范围提交 | 1 | ✅ `1332216` |
| S2-c 角色同步 | 3 | ✅ 本轮 |
| **合计** | **11** | ✅ **全部完成**（命令 41 → 52） |

---

## 本次更新（第十四次：`drafts` S3-a — 草稿基础读写 12 频道）

### 1. `repositories/content_repository.rs`（新）

平移 `electron/repositories/content-repository.ts`：`create` / `get_body` / `update_body` / `delete`。
删除受 `ON DELETE RESTRICT` 外键保护时失败（由调用方决定是否忽略，对齐基线）。

### 2. `repositories/draft_repository.rs`（新，本子域核心）

逐条对齐 `electron/repositories/draft-repository.ts`：

| 内容 | 要点 |
|---|---|
| `DraftMeta` / `DraftFull` | `DraftFull` 用 `#[serde(flatten)]` 序列化为**扁平结构**（对齐 `DraftFull extends DraftMeta`） |
| `DraftSourceDependency` | `kind` 可选（缺失 = 旧行候选依赖）；`chapterNumber` / `finalizationId` 按 kind 条件存在 |
| `parse_dependencies` | 逐条复刻基线校验（hash 64-hex、安全整数、重复 ID、kind 与字段组合、≤500 项）；任一非法则**整体失效** |
| `dependency_states` | BFS 分块（每批 500）拉取依赖状态，含 `authoritative_finalized`（`NOT EXISTS` 子查询）与 receipt 快照比对 |
| `dependency_lineage_is_current` | **显式栈**实现的传递闭包校验（避免深链递归爆栈），逐分支对齐基线的 visiting/memo 语义 |
| `DRAFT_META_SELECT` | `LEFT JOIN finalization_outbox`（仅定稿行取 `chapter_title`） |
| `create` | 单事务：校验依赖 → 原子分配 version（**忽略入参 version**）→ 写 contents → 写 drafts |
| `update_status` | 定稿不可逆：finalized→其它拒绝；非 finalized→finalized 拒绝（必须走原子定稿提交） |
| `update_content` | 已定稿拒绝；其它更新 contents + word_count |
| `delete` | 同事务内校验状态；定稿返回 `FINALIZED_DELETE_REQUIRED_MESSAGE`（命令层据它回 `errorCode`）；contents 清理失败静默忽略 |
| 读 API | `list_by_chapter` / `list_all` / `get_meta` / `get_full` / `get_latest_by_chapter` / `get_finalized_by_chapter` / `get_next_version` / `get_max_finalized_chapter` |

- 哈希复用 `character_roster_repository::hash_text`（`pub(crate)`）；**零新增依赖**。

### 3. `commands/db.rs` / `lib.rs` / `ipc-client.ts`

- 12 命令：`create`（MUTATING，信封 `{success,id?,error?}`）、6 读频道、
  `get-max-finalized-chapter` / `next-version`、`update-status` / `update-content`（MUTATING）、
  `delete`（MUTATING，信封 `{success,errorCode?,error?}`）。
- `db:draft-update-content` 复刻 controller 的**前置检查顺序**（「草稿不存在：{id}」/「已定稿正文为只读内容，不能再修改」）。
- `lib.rs` 注册（**52 → 64**）；`ipc-client.ts` 登记 12 个频道参数名。

### 4. 验证与测试

新增 Rust 测试 **14 个**（content 2 + draft 11 + 命令 1）：版本分配与读 API、非法/过期依赖拒绝、
依赖变旧标记（含**传递闭包**）、定稿不可逆规则、正文只读、删除与定稿删除入口、
最大定稿章节、`parse_dependencies` 规则、序列化形状、命令门禁端到端。

结果：`cargo check --all-targets` **0 告警** · `cargo test --lib` **128/128** ·
`pnpm typecheck` / `pnpm run lint` 均 exit 0。

---

## 本次更新（第十五次：`revisions` S3-b — 修稿 9 频道）

### 1. `db/schema.rs`：新增 `CREATE_REVISIONS`

按 `electron/database.ts` 的最终列集建 `revisions` 表（含 `source_draft_*` 冻结源稿四列、
`idx_revisions_draft_index` 唯一索引、指向 `drafts` 的 `ON DELETE CASCADE` 与 `contents` 的 `ON DELETE RESTRICT`）。

### 2. `src-tauri/src/draft_source_guard.rs`（新，模块级非仓储）

平移 `electron/repositories/draft-source-guard.ts`：`ExpectedDraftSource`（冻结源稿身份）、
`assert_expected_draft_source`（id/章号/版本/状态/正文全等校验）、`SOURCE_DRAFT_CHANGED` 常量与固定文案。

### 3. `repositories/revision_repository.rs`（新，本子域核心）

| 内容 | 要点 |
|---|---|
| `RevisionMeta` / `RevisionFull` | `RevisionFull` 用 `#[serde(flatten)]` + `rename_all` 扁平序列化；`sourceDraft` 可为 null |
| `create` / `replace_pending` | 单事务：源守卫（缺失即判源稿变化）→ 原子分配 `revision_index` → 写 contents → 写 revisions；后者额外把其它 pending 改为 `discarded` |
| `merge_into_draft` | 单事务；`revisionId` 为幂等身份；6 类拒绝分支逐条对齐基线（不属于目标草稿 / 非 pending / 目标不可修改 / 正文已变化 / 缺冻结源稿 / 源稿不一致） |
| `mark_merged` / `mark_discarded` | 仅 `pending` → 目标态；`changes == 0` 时回基线原文案（含 `[RevisionRepository]` 前缀） |
| 读 API | `list_by_draft` / `get_pending` / `get_full` / `get_next_index` |

### 4. 接线

- 9 命令（`create` / `replace-pending` / `merge` / `mark-merged` / `mark-discarded` 为 MUTATING）；
  `create`/`replace-pending` 失败时按守卫文案同时回填 `errorCode: SOURCE_DRAFT_CHANGED` 与 `error`（`String(err)` 形态）。
- `lib.rs` 注册（**64 → 73**）；`ipc-client.ts` 登记 9 个频道参数名。

### 5. 验证与测试

新增 Rust 测试 **12 个**（守卫 1 + 仓储 10 + 命令 1）：序号分配与源稿冻结、守卫三重校验、
`replace_pending` 弃用语义、合并写入与幂等重放、**6 类合并拒绝分支**、旧修订稿缺源稿、
标记的 pending 门槛、`RevisionFull` 序列化形状、命令层端到端（含 `errorCode` 回填）。

途中修了一个真实缺陷：`RevisionFull` 缺 `rename_all = "camelCase"`，导致 `sourceDraft` 字段名漂移（已加）。

结果：`cargo check --all-targets` **0 告警** · `cargo test --lib` **140/140** ·
`pnpm typecheck` / `pnpm run lint` 均 exit 0。

---

## 本次更新（第十六次：`reviews` S3-c — 审稿 5 频道）

### 1. `db/schema.rs`：新增 `CREATE_REVIEWS`

按 `electron/database.ts` 建 `reviews` 表（含 `source_draft_*` 冻结源稿四列、
`idx_reviews_draft_index` 唯一索引、`drafts` CASCADE / `contents` RESTRICT 外键）。
注意：`reviews` 表**没有** `updated_at` 列（与 `revisions` 不同），元数据也不含该字段。

### 2. `repositories/review_repository.rs`（新）

| 内容 | 要点 |
|---|---|
| `ReviewMeta` | `id` / `baseDraftId` / `reviewIndex` / `contentId` / `createdAt`（无 `updatedAt`） |
| `ReviewFull` | `#[serde(flatten)]` + `rename_all = "camelCase"`（吸取 S3-b 的教训，一次到位）；`sourceDraft` 可为 null |
| `create` | 单事务：源守卫（缺失即判源稿变化）→ 原子分配 `review_index` → 写 contents → 写 reviews；**忽略入参 `reviewIndex`** |
| 读 API | `list_by_draft`（按序号升序）/ `get_latest_by_draft`（序号降序 LIMIT 1）/ `get_full` / `get_next_index` |

复用 `draft_source_guard`；审稿无状态流转（不像修稿有 pending/merged/discarded）。

### 3. 接线

- 5 命令（`db:review-create` 为 MUTATING）；失败时按守卫文案同时回填
  `errorCode: SOURCE_DRAFT_CHANGED` 与 `error`（信封 `ReviewCreateResult`：
  `{ success, id?, reviewIndex?, errorCode?, error? }` —— 字段名为 `reviewIndex`，故不能复用 `RevisionCreateResult`）。
- `lib.rs` 注册（**73 → 78**）；`ipc-client.ts` 登记 5 个频道参数名。

### 4. 验证与测试

新增 Rust 测试 **5 个**（仓储 4 + 命令 1）：序号分配与源稿冻结（含“忽略入参 99”）、
源守卫双重校验、`get_latest` 取最大序号、序列化扁平 + `sourceDraft` camelCase + 旧行 null、
命令层端到端（含 `errorCode` 回填与门禁）。

结果：`cargo check --all-targets` **0 告警** · `cargo test --lib` **145/145** ·
`pnpm typecheck` / `pnpm run lint` 均 exit 0。

---

## 本次更新（第十七次：`post-process` S3-d — 后处理跑批 6 频道）

### 1. `db/schema.rs`：新增 `CREATE_POST_PROCESS`

`post_process_runs`（TEXT UUID 主键 + `all_critical_passed` 派生标志）+
`idx_post_runs_source` + `post_process_steps`（含 `attempt_count`/`completed_at`/`last_attempt_at`，
`ON DELETE CASCADE` 指向 runs）。

### 2. `repositories/post_process_repository.rs`（新）

| 内容 | 要点 |
|---|---|
| `PostProcessRunData` / `PostProcessStepData` | camelCase；两者均为布尔字段（`allCriticalPassed` / `critical` / `ok`） |
| `create_run` | 单事务写入 run + 全部 step（`critical` 转 0/1）；run ID 用 `project_access::random_uuid_v4()` |
| `get_latest_run` | `ORDER BY created_at DESC LIMIT 1` |
| `get_steps` | `ORDER BY id ASC` |
| `mark_step_ok` | **步骤收据与跑批汇总同事务**；`attempt_count + 1`；失败时回写 `completed_at`；`changes != 1` → 「后处理步骤不存在或已失效」 |
| `mark_step_failed` | 同上但 `ok = 0`、`completed_at = ''`；**汇总重算**（全量重跑不得保留旧 `true`） |
| `refresh_critical_status`（private） | `COUNT(critical = 1 AND ok = 0) == 0 → all_critical_passed = 1` |
| `is_all_critical_passed` | 取最新跑批标志；无跑批 → false |

补充：新增了共享 helper `commands::db::simple_mutating_result(outcome)`，收敛后两个
MUTATING 命令复用。

### 3. 接线

- 6 命令（`create-run` / `mark-step-ok` / `mark-step-failed` 为 MUTATING）。
- `lib.rs` 注册（**78 → 84**）；`ipc-client.ts` 登记 6 个频道参数名。

### 4. 验证与测试

新增 Rust 测试 **7 个**（仓储 6 + 命令 1）：跑批初始化与步骤明细、
逐步成功与汇总切换、失败重算与恢复、步骤不存在拒绝、空跑批语义、
最新跑批按 `created_at` 降序、命令层端到端。

结果：`cargo check --all-targets` **0 告警** · `cargo test --lib` **152/152** ·
`pnpm typecheck` / `pnpm run lint` 均 exit 0。

---

## 本次更新（第十八次：`llm 日志/摘要` — 5 频道）

### 1. `db/schema.rs`：新增 `CREATE_LLM_CALLS` 与 `CREATE_SUMMARY_SNAPSHOTS`

- `llm_calls`（表名是 `llm_calls`，非 `llm_call_logs`）；
- `summary_snapshots` 按**最终列集**建表（含 `draft_id`/`chapter_notes`/`continuity_facts`/
  `character_state_candidates`/`source_finalization_id`/`source_content_hash`/`projection_generation`）
  + `idx_summary_snapshots_draft` 部分唯一索引。`continuity_projection_meta` / `consistency_exemptions`
  属连续性/一致性豁免子域，留给批次 E。

### 2. `repositories/llm_repository.rs`（新）

| 内容 | 要点 |
|---|---|
| `log_call` | 入参保持 `serde_json::Value`（基线是 `Record<string, unknown>`）；`modelId` 非字符串即拒（对齐 NOT NULL 约束）；`success` 按 **JS 真值语义**转 0/1；数字字段非数字 → NULL |
| `get_stats` | `COUNT` 恒非空；三个 SUM 用 `COALESCE(..., 0)`；**token 三项保留 `Option`**（全无用量时为 `null` 而非 0） |
| `get_history` | 按 `id DESC LIMIT ?`；`finishReason` 由 `error_message` 的 CASE 推导（成功恒 `stop`） |
| `save_summary_snapshot` / `get_latest_summary_snapshot` | 后者只读 `draft_id IS NULL` 的行（旧式快照），定稿绑定行不可见 |

### 3. 接线

- 5 命令：`db:log-llm-call`（MUTATING）、`db:get-llm-stats`、`db:get-llm-history`
  （`limit` 可缺省，默认 **50**）、`db:save-summary-snapshot`（MUTATING）、`db:get-latest-summary`。
- `lib.rs` 注册（**84 → 89**）；`ipc-client.ts` 登记 5 个频道参数名。

### 4. 验证与测试

新增 Rust 测试 **8 个**（仓储 7 + 命令 1）：空库 token 为 null、写入与聚合统计、
`finishReason` 映射矩阵（7 用例）、`modelId` 必填、JS 真值语义、`limit` 生效、
旧式快照忽略 `draft_id` 非空行、命令层端到端。

结果：`cargo check --all-targets` **0 告警** · `cargo test --lib` **160/160** ·
`pnpm typecheck` / `pnpm run lint` 均 exit 0。

---

## 下一步：`project 清理`（2 频道）—— 批次 C 收口

| 频道 | 依赖 | 可否现在迁 |
|---|---|---|
| `db:project-clear-generated-data` | `ProjectClearRepository.clearGeneratedData(options)`（`creativeFields` / `blueprints` / `generatedText`） | ✅ 可（各子域 clear 已就位） |
| `db:import-global-facts-commit` | `ImportGlobalFactsRepository.commit(request)` | ⬜ 依赖批次 G |

- 该频道为 MUTATING（信封 `{ success, ...result, error? }`，基线带 try/catch）；
- 需对照 `electron/repositories/project-clear-repository.ts` 的选项组合与事务边界；
- 完成后 **批次 C 基本收口**（仅余依赖批次 G/E 的少数频道）。

---

## 遗留项（沿用 2026-10-06 快照）

- 未验证：批次 C **GUI 实机验证**（打开真实项目读写全部已迁子域）、
  双栈同库行为对照、`vitest` 全量超时定位、`cargo fmt --check` 未纳入验收。
- 押后：L3（`.vela` → `.lorekeeper`）、可见品牌（`brand.ts` / i18n 标题）。
- 未迁频道的错误文案友好化（遗留项 12）。
- 批次 G 需要的 `BlueprintRepository.getCommittedRangeOperation`（无 IPC 频道，被
  `import-run-repository.ts` 使用）尚未移植，随批次 G 一并落地。
- `DraftRepository.clearAll`（服务 `db:project-clear-generated-data`）**尚未移植但已进队列**：
  本库中它要删的 `reviews` / `revisions` / `post_process_*` / `summary_snapshots` 建表已全部就位。
- `drafts` 余 4 频道（`authority-sequence` / `export-snapshot` / `export-authority-current` /
  `import-finalized-batch`）依赖 finalization 仓储，随批次 E 落地。
- `post_process_repository` 的 `get_failed_step_labels`、`llm` 侧无频道的辅助方法未移植。
