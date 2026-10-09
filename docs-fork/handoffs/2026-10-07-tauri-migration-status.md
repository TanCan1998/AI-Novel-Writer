# Tauri 迁移进度快照（2026-10-07）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> **⚠️ 本文件是 2026-10-07 的冻结快照** —— 其后的进展（自「第二十三次」起，含
批次 D2-b）见 [`2026-10-08-tauri-migration-status.md`](./2026-10-08-tauri-migration-status.md)。
> 批次 A–C `project_core` / `characters` 的历史细节见
> [`2026-10-06-tauri-migration-status.md`](./2026-10-06-tauri-migration-status.md)；
> channel 级盘点见 [`docs-fork/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-07 · 第二十二次）

| 项 | 值 |
|---|---|
| 仓库 | **`TanCan1998/Lorekeeper`**（原名 `AI-Novel-Writer`；仍为 `EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork） |
| 分支 | `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm 包 `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 当前阶段 | **批次 C 数据库层子域全部完成 ✅** + **批次 D1（`llm:*` 模型管理 7 频道）完成 ✅** + **批次 D2-a（生成参数策略 + 模型执行租约 2 频道）完成 ✅**：`project_core` / `characters` / `blueprints` / `drafts`（12/16）/ `revisions` / `reviews` / `post-process` / `llm 日志与摘要` / `project 清理` / **D1 模型管理** / **D2-a 租约**；`llm:*` 剩余生成链 5 频道（`generate` / `generate-stream` / `cancel` / `discover-models` / `test-connection`）归 **D2-b/c**（依赖已批准：`reqwest` + `native-tls` + `socks`） |
| 已注册命令 | **99**（骨架 1 + A 11 + B 22 + C 子域 56 + D1 7 + D2-a 2） |
| GUI 冒烟 | ✅ **已做**（2026-10-07 **四轮** `pnpm tauri dev`）：窗口标题 `Lorekeeper`、vite@5190、cargo 353/353、`lorekeeper.exe` **内存 42.6 MB**（首轮）/ **30.1 MB**（D1 轮）/ D2-a 轮 vite `482 ms` 起服且无 panic |
| 自动化回归 | `cargo test --lib` **243/243**（含 3 个**磁盘级**端到端：真实 `.vela/lorekeeper.db` + WAL + 外键 + 跨重开持久化）；`pnpm run check:channels` 校验契约↔命令映射（未迁移 93 频道）；`vitest` 频道覆盖 6/6（含 2 个**新增防线**测试） |
| 双栈隔离 | L0 安装标识 / L1 `~/.lorekeeper` / L2 `<root>/.vela/lorekeeper.db` 均独立；L3（`.vela` 改名）押后。D1 起 `~/.lorekeeper/{config.json,models.json,recent-projects.json}` 为**真实持久化**（此前 config 仅内存态） |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc @ `D:\Environment\rust\`（脚本内显式设 `RUSTUP_HOME`/`CARGO_HOME`） |
| 验证状态 | ✅ `cargo check --all-targets` **0 告警** · ✅ `cargo test`（全目标）**243/243** · ✅ `pnpm typecheck` exit 0 · ✅ `pnpm run lint` exit 0 · ✅ `check:channels` orphan 空 · ✅ D2-a GUI 冒烟（启动路径无 `Command ... not found` / 无 panic） |

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

## 本次会话结束状态（2026-10-07 · 第二轮）

**工作区**：干净（仅未跟踪的 `.pi/` agent 临时目录，不入 Git）；`master` 已与 `origin/master` 同步。

| 项 | 值 |
|---|---|
| HEAD | `acb1b03` |
| 本会话提交（共 9 个） | `fa07875` reviews · `39ee0c2` post-process · `e0e1004` llm/摘要 · `d89861f` project 清理 · `da4073f` 校验脚本 · `3c9aa2b` 结束状态快照 · `24c008f` 未迁移频道友好提示（遗留项 12） · `4d5ef61` 磁盘级端到端回归 · `acb1b03` 第二十次快照 |
| 已注册命令 | **90**（骨架 1 + A 11 + B 22 + C 子域 56） |
| 验证 | `cargo check --all-targets` 0 告警 · `cargo test --lib` **169/169** · `pnpm typecheck`/`lint` exit 0 · `pnpm run check:channels` orphan 空 |
| GUI 冒烟 | ✅ 两轮（首轮采集信号；次轮闭环验证友好文案） |
| 未迁移频道 | 102（契约 191 invoke 频道；已按批次归类） |

### 下一次会话的推荐起手

1. **批次 D（`llm:*` 14 频道）** —— 当前最低风险的实质推进项；做完可让 `llm:list-models`
   等启动即调用的频道真正可用（遗留项 12 的友好提示仍会保留作为「未迁」兜底）。
2. 批次 F（豁免/叙事线程/派生树/恢复候选，16 频道，互依赖少）。
3. 批次 E（continuity + finalization + `chapter:*`）需先落 finalization 仓储，风险最大。

### GUI 冒烟清单（本轮已做部分已勾选）

```bash
cd tauri-app && pnpm tauri dev
```

- [x] 窗口标题为 `Lorekeeper`（进程 `lorekeeper.exe`，vite@5190，cargo 353/353）
- [x] 全程无 `Command ... not found`（→ 已改为「尚未迁移到 Tauri 侧」友好文案）
- [ ] 项目主台账读写（新建/重开项目，配置保存后重进不漂移）——*磁盘级测试已覆盖持久化*
- [ ] 蓝图：批量写入 → 列表回读 → 范围提交 → 角色同步 pending → complete ——*磁盘级测试已覆盖*
- [ ] 草稿：创建（version 自增）→ 改正文 ——*磁盘级测试已覆盖*
- [ ] 修稿：create → merge（幂等重放）——*磁盘级测试已覆盖*
- [ ] 审稿：create → get-latest ——*磁盘级测试已覆盖*
- [ ] 后处理：create-run → mark-step-ok/failed → is-all-passed ——*磁盘级测试已覆盖*
- [ ] LLM：get-stats / get-history 数值合理 ——*磁盘级测试已覆盖*
- [ ] 清理：`generatedText` 后根目录 `第N章*.txt` 进入 `.vela/trash/` ——*磁盘级测试已覆盖*

> 说明：*磁盘级测试已覆盖* = 该路径已在 `disk_e2e.rs` 中以真实 `.vela/lorekeeper.db`
> 断言（不再依赖人工点击）。**尚未人工验证**的是「界面呈现与交互」本身
> （按钮触发、表单回显、错误提示的 UI 形式），仍建议在后续会话中人工过一遍。

---

## 本次更新（第十九次：`project 清理` — 批次 C 数据库层收口）

### 1. `db/schema.rs`：新增 `CREATE_IMPORT_OPERATIONS`

补 `finalized_draft_import_operations` / `import_global_fact_operations` 两张幂等日志表
（本子域只需前者：`generatedText` 清理要删它；仓储与命令仍随批次 E / G）。

### 2. `repositories/project_clear_repository.rs`（新）

| 内容 | 要点 |
|---|---|
| `ProjectClearScope` / `ProjectClearOptions` / `ProjectClearResult` | camelCase；三个开关均可缺省（`Option<bool>`，缺省 = false） |
| `is_finalized_chapter_file`（private） | 手工实现基线 `^第\d+章(?: .*)?\.txt$`（**不引入 regex 依赖**），覆盖 `第1章.txt` / `第12章 标题.txt` / `第3章 .txt` |
| `move_generated_files_to_trash` | 事务**之前**把根目录成稿移入 `.vela/trash/clear-<ISO 时间戳>/`（时间戳复用 `commands::project::{epoch_millis_now, iso8601_utc_from_millis}`）；失败即按逆序回滚已移动文件 |
| `clear_generated_data` | 单事务：`generatedText`（8 张表）→ `blueprints`（复用 `clear_blueprint_facts_within_transaction`）→ `creativeFields`（先 `migrate_character_roster_schema` 再删三张表 + `project_core` 九个字段）；成功后才删除回收子目录，失败则回滚文件移动 |

### 3. 可见性调整

`commands/mod.rs` 的 `mod project;` → `pub mod project;`（crate 内可见），
使时间戳 helper 可被仓储层复用，避免重复实现 ISO8601。

### 4. 接线

- 1 命令：`db:project-clear-generated-data`（MUTATING，信封
  `{ success, cleared?, physicalFilesDeleted?, error? }`）；项目路径取自活跃租约而非入参。
- `lib.rs` 注册（**89 → 90**）；`ipc-client.ts` 登记该频道参数名。

### 5. 验证与测试

新增 Rust 测试 **6 个**（仓储 5 + 命令 1）：文件名模式矩阵（3 接受 / 5 拒绝）、
`generatedText` 清空 + 物理文件移动 + 非成稿文件保留、缺项目路径拒绝、
`blueprints`+`creativeFields` 组合与顺序、序列化/解析形状、命令层端到端。

结果：`cargo check --all-targets` **0 告警** · `cargo test --lib` **166/166** ·
`pnpm typecheck` / `pnpm run lint` 均 exit 0。

### 6. 新增契约覆盖校验脚本（静态回归工具）

`tauri-app/scripts/verify-channel-coverage.mjs` + `pnpm run check:channels`（零依赖纯 node）：
不启动应用即可核对「契约频道 ↔ 已注册命令」的机械映射，输出未迁移清单与 orphan 清单
（存在 orphan 时退出码 1）。当前结果：

```
契约 invoke 频道 191（事件频道 4）
已注册命令 90 → 覆盖 invoke 频道 89
未迁移 invoke 频道 102  [db=44 kb=15 llm=14 mcp=9 update=6 chapter=4 skills=4 dialog=3 prompt=3]
命令名与契约频道一一对应 ✅
```

**未迁移 102 频道按批次归类**（供后续排期）：

| 批次 | 数量 | 频道 |
|---|---|---|
| **E / G** | 28 | continuity(4) + finalization-link(1) + drafts 余 4 + import-run(18) + import-global-facts(1) |
| **F** | 16 | consistency-exemption(3) + narrative-thread(6) + plot-tree(3) + recovery-candidate(4) |
| **E** | 4 | `chapter:*`（定稿删除与生命周期） |
| **D** | 14 | `llm:*`（模型管理 / 生成 / 流式 / 租约） |
| 向量库 | 15 | `kb:*` |
| MCP | 9 | `mcp:*` |
| 其它 | 16 | prompt(3) + skills(4) + dialog(3) + update(6) |

---

## 本次更新（第二十次：GUI 冒烟验证 + 遗留项 12 修复 + 磁盘级端到端回归）

本轮完成的是上一轮「结束本次」所中断的事项：**GUI 实机冒烟**（批次 C 交付 56 命令后
从未在真实项目上验证过，是当时最大的风险敷口）。

### 1. GUI 实机冒烟（两轮）

**首轮（采集信号）**：

| 信号 | 结果 |
|---|---|
| vite | `ready in 1420 ms` @ `http://127.0.0.1:5190/` |
| cargo | 353/353 编译完成，`Running target\debug\lorekeeper.exe` |
| 进程 | `lorekeeper.exe`（PID 26088），**内存 42.6 MB** ← 迁移目标（Electron 通常 150–300 MB） |
| 窗口 | 可见窗口标题 `Lorekeeper`（EnumWindows + GetWindowText 采集） |
| WebView2 | 18 个子进程（页面真实渲染） |
| 前端日志 | `setZoomFactor 尚未在 Tauri 侧实现`（阶段 3 预期）；`Unknown Error: Command llm_list_models not found`（遗留项 12） |
| 副作用 | `Cargo.toml` **未被重写** → 遗留项 14 的注释规避写法有效 ✅ |

**次轮（闭环验证修复）**：原生错误消失，友好文案 6 次命中 ——
`[Tauri 适配] 频道 llm:list-models 尚未迁移到 Tauri 侧（后续批次），已拒绝调用`。
两轮冒烟后进程树均已清理（端口 5190 释放）。

### 2. 遗留项 12 修复：未迁移频道友好提示（`24c008f`）

- `scripts/verify-channel-coverage.mjs` 新增 `--emit`：生成
  `src/shared/migrated-channels.ts`（89 个已迁移频道；内容未变时不写盘）。
- `ipc-client`：未迁移频道**前置拦截** + 对 Tauri 原生
  `Unknown Error: Command xxx not found` 的**兜底翻译**（生成物落后于 Rust 注册时仍友好）。
- `package.json` 新增 `check:channels:emit`。
- 新增 `test/channel-migration-coverage.test.ts`（4 用例）：重算契约↔命令映射并与生成物
  比对，**防止生成物过期或被手改**；断言已迁移集合是契约的真子集。

### 3. 磁盘级端到端回归（`4d5ef61`）—— 冒烟的可自动化版本

新增 `src-tauri/src/disk_e2e.rs`（`#[cfg(test)] mod`）。与各仓储的内存库单测不同，它走
**真机等价路径**：真实 `<root>/.vela/lorekeeper.db`、WAL、`foreign_keys=ON`、
真实目录上的物理文件、**跨连接重开后的持久化**。

| 用例 | 覆盖 |
|---|---|
| `real_project_creation_lifecycle_persists_across_reopen_test` | 主台账 → 蓝图范围提交 → 蓝图角色同步 → 草稿 → 修稿合并 → 审稿 → 后处理跑批 → LLM 日志与统计；随后重开库逐域断言（含步骤失败原因、修订 `merged` 状态） |
| `real_project_sqlite_pragmas_are_effective_on_disk_test` | 文件库上 WAL 真实生效；外键 `RESTRICT` 阻止删除被草稿引用的正文；删草稿**级联**带走修稿与审稿 |
| `real_project_clear_moves_physical_files_and_keeps_library_test` | 清理移动真实成稿文件、保留非成稿文件与未勾选范围，库本身仍可重开 |

这三项**内存库测不出来**（WAL、真实路径、跨重开），正是此前「56 命令未经真机验证」的核心缺口。

### 4. 验证

`cargo test --lib` **169/169**（166 + 3 磁盘级）· `cargo check --all-targets` **0 告警** ·
`pnpm typecheck` / `pnpm run lint` exit 0 · `pnpm run check:channels` orphan 空 ·
两轮 dev 冒烟通过。

---

## 本次更新（第二十一次：批次 D1 — 全局 JSON 存储层 + `llm:*` 模型管理 7 频道）

### 0. 为什么拆成 D1 / D2

批次 D 的 14 个频道分两类，依赖面完全不同：

| 子批次 | 频道 | 依赖 | 状态 |
|---|---|---|---|
| **D1** | `list-models` / `save-model` / `delete-model` / `get|set-default-model` / `get|set-default-embedding-model`（7） | 仅 JSON 文件读写（`config.json` / `models.json`） | ✅ 本轮完成，**零新增依赖** |
| **D2** | `begin|close-execution-lease` / `generate` / `generate-stream` / `cancel` / `discover-models` / `test-connection`（7 + 3 事件） | HTTP 客户端 + SSE 流式（`reqwest` 或 `tauri-plugin-http`）+ 生成参数策略复刻 | ⬜ 需 **Ask first**（新增 Rust crate） |

### 1. 着手前修掉的**真实缺口**：`config:get/set` 此前只是内存态

批次 A 落地的 `ConfigStore` 是**内存 HashMap**（当时标注「骨架阶段」）。它是 D1 的硬前置：
`llm:get-default-model` / `llm:set-default-model` / `llm:delete-model` 与它共用同一份
`config.json`。因此本轮把配置改为**真实文件持久化**，并对齐基线语义：

| 语义 | 基线 | 现在 |
|---|---|---|
| `config:get` 文件缺失 | `DEFAULT_GLOBAL_CONFIG` | ✅ 同 |
| `config:get` 文件存在 | **原样返回，不合并默认值** | ✅ 同（此前是「默认值 + 内存」） |
| `config:get` 文件损坏 | 告警 + 默认值，**不改写文件** | ✅ 同 |
| `config:set` | 读旧值（损坏则拒覆盖）→ 浅合并 → 原子写 | ✅ 同，错误文案 `Error: 全局配置损坏，已拒绝覆盖` |
| 首次 `config:set` | `{...DEFAULT_GLOBAL_CONFIG, ...updates}` 写出完整文件 | ✅ 同 |

`AppState.config`（内存存储）已移除；`Default for AppState` 相应同步。

### 2. 新增 `src-tauri/src/app_paths.rs`（全局数据根）

把原先只存在于 `commands/project.rs` 的 `lorekeeper_home()` 提为**独立模块**（避免
`json_store` → `commands` 的反向依赖），并补齐 `config.json` / `models.json` /
`recent-projects.json` 路径与 `ensure_lorekeeper_home()`（对齐基线 `ensureVelaHome()`）。

- **L1 隔离不可回退**：只认 `AI_NOVEL_LOREKEEPER_HOME`，缺省 `~/.lorekeeper`；不读
  `AI_NOVEL_VELA_HOME`、不回退 `~/.vela`。
- 路径解析拆为**注入式纯函数** `resolve_home(env_home, user_home)`，测试无需污染进程环境。
- `lib.rs` 增加 `setup` 钩子调用 `ensure_lorekeeper_home()`：启动即建立
  `~/.lorekeeper/{prompts,logs}`（失败不阻断启动，只告警）。**冒烟实测**：首轮启动后
  `~/.lorekeeper/` 已含 `logs`、`prompts`。

### 3. 新增 `src-tauri/src/json_store.rs`（迁移 `electron/utils/config-utils.ts`）

| 函数 | 对齐基线 |
|---|---|
| `try_read_json_value` | `tryReadJsonFile` 三态（`Missing` / `Ok` / `Error`）；增量写入方在 `Error` 时必须拒绝覆盖 |
| `read_json_value_or` / `read_json_file` | `readJsonFile`：失败告警 + 回落默认值（只读路径容忍损坏） |
| `write_json_file` | `writeJsonFile`：同目录临时文件（独占创建 + `0600`）→ `write_all` + `sync_all` → `rename` 提交；Windows 占用类错误（5/32/33）按 `10/25/50/100/200ms` 重试；失败保持原文件不变并清理临时文件 |

`commands/project.rs` 的最近项目读写改为复用该层（此前是裸 `fs::write`），
`recent-projects.json` 由此获得与基线一致的原子写语义。

### 4. 新增 `commands/llm.rs`（7 命令）

| 频道 | 命令 | 关键行为 |
|---|---|---|
| `llm:list-models` | `llm_list_models` | 缺失/损坏/形状不符 → `[]` |
| `llm:save-model` | `llm_save_model` | 按 `id` 就地替换否则追加；损坏拒绝覆盖；非对象条目拒绝 |
| `llm:delete-model` | `llm_delete_model` | **先清引用再删对象**；第二步失败回滚配置；回滚也失败时给出组合文案 |
| `llm:get-default-model` | `llm_get_default_model` | `defaultModelId`（`null`/缺键 → `null`） |
| `llm:set-default-model` | `llm_set_default_model` | 写 `defaultModelId` |
| `llm:get-default-embedding-model` | `llm_get_default_embedding_model` | `defaultEmbeddingModelId ?? null` |
| `llm:set-default-embedding-model` | `llm_set_default_embedding_model` | 写 `defaultEmbeddingModelId` |

- **模型条目原样透传 `serde_json::Value`**：`ModelProfile` 的字段集由渲染层契约定义
  （`capabilities` / `reasoningOverride` / `embeddingOptions` 等可选字段），Rust 只解释
  `id` —— 避免漏字段导致用户配置被静默裁剪（有专门测试断言未知字段不被丢弃）。
- 身份比对 `same_model_identity` 复刻 `m.id === model.id` 的严格相等语义（两侧都缺 `id`
  视为同一身份）。
- `llm:delete-model` 的字段存在性逐条对齐基线：成功时 `defaultModelId`（缺键则整体缺省）
  与 `defaultEmbeddingModelId ?? null`（恒存在）同时出现；失败时两者整体缺省 → 用
  `Option<Value>` 区分「缺省」与「显式 null」。
- 回滚失败文案逐字复刻基线的**双 `Error: ` 前缀**（`String(new Error(\`${String(e1)}；恢复默认模型配置失败：${String(e2)}\`))` 的链式结果），并单测锁定。

### 5. 接线

- `commands/mod.rs`：`mod llm;` + `pub use llm::*;`
- `lib.rs`：`mod app_paths; mod json_store;` + `setup` 建目录 + 注册 7 命令（**90 → 97**）
- `state.rs`：移除 `config: Mutex<ConfigStore>` 字段
- `ipc-client.ts`：登记 4 个带参频道（`llm:save-model` → `model`；`llm:delete-model` /
  `llm:set-default-model` / `llm:set-default-embedding-model` → `modelId`）
- `pnpm run check:channels:emit` 重新生成 `src/shared/migrated-channels.ts`（**96** 个频道）
- `test/channel-migration-coverage.test.ts`：负例改用未迁频道 `llm:generate`，并加断言
  `llm:list-models` 已迁移

### 6. 验证与测试

新增 Rust 测试 **33 个**（app_paths 6 + json_store 6 + config 9 + llm 12；
旧 config 的内存态测试 3 个被替换，故净 +30）：

- **app_paths**：环境变量优先/去空白、空白回落用户主目录、路径派生、建目录幂等、不可用父路径报错；
- **json_store**：三态判定、损坏回落且不改文件、按类型反序列化、建父目录 + pretty 格式（两空格/无尾随换行）、
  覆盖已有文件、失败保持原文件且清理临时文件；
- **config**：默认值逐字段对齐、缺文件/部分文件/损坏三种读取语义、浅合并保留未知键（`locale`）、
  首次写入以默认值为基底、损坏拒绝覆盖（`Error: ` 前缀）；
- **llm**：缺失/损坏/形状不符的空列表、未知字段透传、插入/就地更新、损坏与非对象拒绝、
  删除同时清两个默认项、非目标默认项保留、失败时字段缺省、**models 写失败回滚 config**、
  回滚失败文案、默认模型往返、损坏配置拒绝、**命令层端到端**（经 `AI_NOVEL_LOREKEEPER_HOME` 注入临时根调真实命令）。

结果：`cargo check --all-targets` **0 告警** · `cargo test --lib` **199/199** ·
`pnpm typecheck` exit 0 · `pnpm run lint` exit 0 · `pnpm run check:channels` orphan 空。

### 7. GUI 实机冒烟（第三轮）

`pnpm tauri dev`：vite `ready in 436 ms` @5190 → cargo 编译 `lorekeeper` →
`Running target\debug\lorekeeper.exe`（**30.1 MB**）。

- **关键验证**：启动即调用的 `llm:list-models` 不再报错 —— 日志里既无
  `Unknown Error: Command llm_list_models not found`，也无「尚未迁移」提示；
  唯一告警仍是阶段 3 既知项 `setZoomFactor 尚未在 Tauri 侧实现`。
- 首次启动即创建了 `~/.lorekeeper/{logs,prompts}`（L1 隔离生效；`~/.vela` 未被 Tauri 触碰）。
- 冒烟后已 `taskkill /T /F` 清理本次启动的进程树（端口 5190 释放，无 `lorekeeper.exe` 残留）。
- `Cargo.toml` 未被 `tauri dev` 重写 ✅（遗留项 14 的注释规避写法继续有效）。

---

## 本次更新（第二十二次：批次 D2-a — 生成参数策略 + 模型执行租约 2 频道）

### 0. 决策：D2 采用「Rust 重写」（用户已批准 `reqwest` 依赖）

D2 的两种策略经**实测对比**后由用户拍板 **Rust 重写**。关键实测事实：

| 事实 | 值 |
|---|---|
| `reqwest` 是否已在依赖树 | ✅ **是**（`tauri 2.12.1` 的直接依赖，`default-features = false`）——`hyper` / `tokio` / `futures-util` / `http-body-util` / `tower-http` / `bytes` / `base64` 全部已在 `Cargo.lock`（共 416 包） |
| TLS 后端 | ❌ 不在（`native-tls` / `schannel` / `openssl` / `rustls` / `ring` 计数均为 0）→ **唯一的新增下载** |
| 选定 feature | `["json", "stream", "native-tls", "socks"]`；`native-tls` 在 Windows 走系统 schannel（纯 FFI、无 C 编译），`socks` 是基线 `socks5://` 代理所需 |
| `electron/llm/*` 是否依赖 Electron 运行时 | ❌ 不依赖（仅本地模块 + `package.json` 版本号；HTTP 走全局 `fetch`）——即 sidecar「零改动复用上游」的论据成立，但被 **+~100 MB `node.exe` 与 +40~60 MB 内存**否决（与本次迁移的立项动机直接冲突） |
| Node sidecar 的真实代价 | `node.exe`(v26) **104 714 056 B ≈ 99.9 MB**；对照 `lorekeeper.exe`(debug) 17.6 MB、实跑内存 30.1 MB |

D2 拆为三层推进：**D2-a 零依赖地基（本轮完成）** → D2-b 生成 / 流式（`reqwest`）→ D2-c 模型发现 / 连通性。

### 1. 新增 `src-tauri/src/llm/`（4 模块，与基线「一文件 ↔ 一模块」对齐）

| Rust 模块 | 行数 | 基线来源 | 内容 |
|---|---|---|---|
| `presets.rs` | 642 | `src/shared/provider-presets.ts` | 9 个内置服务商目录全量镜像；`resolve_model_profile_capabilities` / `resolve_model_profile_reasoning_mapping` |
| `reasoning.rs` | 500 | `src/shared/reasoning-policy.ts` + `reasoning-types.ts` | 单一推理策略缝：阶段表 → 别名 / 封顶 / 强制 → provider 指令 |
| `params.rs` | 408 | `electron/llm/generation-parameter-policy.ts` | 有效请求参数；Kimi 官方端点温度校验 + 固定采样家族省略温度 |
| `lease.rs` | 812 | `electron/services/model-execution-lease.ts` | 能力证据 + 端点 / 主体 / 修订指纹 + 租约注册表（含关闭墓碑） |

逐条对齐基线的要点（含**反直觉但必须复刻**项）：

- **能力证据不可越级**：`verified-provider-preset` 仅在「provider + protocol + 规范化端点 + **精确模型 slug**」四重命中内置目录时成立；用户填写的 `capabilities` 最高只能到 `user-operational-cap`，`featureFlags` 一律 `unknown`。
- **`maxOutputSource` 的 min() 语义原样复刻**：`maxTokens: 4096` 的 DeepSeek 档案会把已验证上限 384 000 压到 4 096 并标记 `legacy-profile`；随后再被上下文窗口**二次封顶**。
- **端点比对含路径**：`bigmodel` 的 `/api/paas/v4` 必须整体匹配；大小写与尾斜杠差异视为同一端点；带用户名 / 密码 / 查询 / 片段一律拒绝。
- **推理策略是唯一缝**：4 策略 × 4 阶段表 → 别名（deepseek `medium`→`high`）/ 封顶 / 强制 → provider 指令；无可信映射时状态 `unsupported` 且**不产生**指令（绝不猜默认值）。
- **`temperature: undefined` = 必须省略字段**，绝不在 provider 层回退 —— NovelAI 与 Kimi 固定采样家族（`kimi-k3` / `kimi-k2.5~2.7` 前缀）的正确前提；官方 Kimi 端点的其它模型则**校验**温度 0~1，越界直接阻断（`KIMI_TEMPERATURE_MESSAGE` 与基线逐字一致）。
- **租约**：冻结模型快照（配置变更不影响在途生成）、4 小时 TTL、过期即清理；`close` 在 5 分钟墓碑窗口内**幂等成功**，之后回落「无效或已关闭」。
- **失败信封差异是刻意的**：租约失败返回结构化 `errorCode`（`MODEL_NOT_FOUND` / `LEASE_BEGIN_FAILED`）且**不带** `Error: ` 前缀；生成类命令走异常路径带前缀（`commands::db::mutating_error`）。

两处超出基线的加固：

1. `LeaseRecord` 手写 `Debug`，快照（含 `apiKey`）一律打码成 `<redacted>`，避免任何日志 / 诊断输出泄露凭据（有专门测试断言回执与序列化结果均不含密钥）；
2. 指纹改用**规范化 JSON**（对象键排序）而非依赖 `serde_json` 的 map 后端 → 与 `preserve_order` feature 是否被其它依赖开启无关；与 Node 的哈希**刻意不要求逐字节一致**（`AGENTS.md` 双栈隔离约定，已在模块文档写明）。

### 2. 新增 `commands/llm_execution.rs`（2 命令）

| 频道 | 命令 | 关键行为 |
|---|---|---|
| `llm:begin-execution-lease` | `llm_begin_execution_lease` | `AppState.llm_leases` 冻结快照并签发**非密钥**回执 |
| `llm:close-execution-lease` | `llm_close_execution_lease` | 幂等关闭（墓碑窗口内重复关闭仍 `success: true`） |

### 3. 接线（含一处**真实缺口**修复）

- `lib.rs`：`mod llm;` + 注册 2 命令（**97 → 99**）。该模块暂带 `#[allow(dead_code)]`：推理 / 参数模块的调用面在 D2-b 落地，注解处已写明「D2-b 接通后移除」。
- `state.rs`：新增 `llm_leases: Mutex<LlmLeaseStore>`（进程内存态，重启即失效，对齐基线模块级 `Map`）。
- **缺口**：`tauri-app/src/services/ipc-client.ts` 的 `buildNamedArgs` 对「带参但未登记参数名」的频道会**直接抛错**（`尚未迁移（参数名未登记）`）。本轮补登 `llm:begin-execution-lease` → `['modelId']`、`llm:close-execution-lease` → `['leaseId']`。
- **新增回归防线**（`test/channel-migration-coverage.test.ts`，+2 测试）：机械比对契约源码中每个**已迁移**频道的 `args` 个数与 `CHANNEL_ARG_NAMES` 登记 —— 漏登记 / 多登记 / 个数不符 / 命名非 lowerCamelCase 都会在 `pnpm test` 阶段失败（这类缺陷原本只有真机点开对应功能才暴露）。解析器已处理多行元组、尾逗号、以及 `Record<string, unknown>` 这类泛型实参内部的逗号。
- `pnpm run check:channels:emit` 重新生成 `src/shared/migrated-channels.ts`（**98** 个频道）。

### 4. 验证与测试

新增 Rust 测试 **44 个**（presets 9 + reasoning 8 + params 10 + lease 12 + llm_execution 4 + state 1），`cargo test --lib` **199 → 243**：

- **presets**：9 服务商目录完整性、四重守卫（端点 / 协议 / slug / 类型）、大小写与尾斜杠等价、凭据 / 查询 / 片段拒绝、路径必须整体匹配、映射克隆隔离全局预设；
- **reasoning**：无映射 → `unsupported` 且不给指令；deepseek 别名（medium→high）与 `off`→`disabled thinking`；非法覆盖值回落项目策略；xAI `max`→`high` 封顶、`off`→`low` 强制；Gemini thinkingBudget 透传（含 0）；阶段缺省 `general`；端点 / 协议不一致即无映射；
- **params**：温度只来自档案、请求预算优先且「都缺省则整体省略」、已验证端点才带推理指令、Kimi 固定家族 5 种写法（含大小写 / 空白）全部省略温度、越界与缺失温度阻断（边界值 1 接受）、非官方主机（http / 别名域 / 伪造后缀 / 自建代理）不触发 Kimi 规则、`responseFormat` 原样透传、阶段影响指令；
- **lease**：三项证据来源等级、min() 封顶（legacy 压已验证、上下文窗口压输出上限）、无协议证据时能力标记为 `null`、非法上限（0 / 负数 / 小数 / 字符串）拒绝、指纹稳定性与区分度（温度改变修订指纹但不改端点指纹、尾斜杠等价）、规范化 JSON 键序、端点规范化、**回执与序列化不含密钥**、冻结语义（签发后改写档案不影响在途租约）、过期清理、墓碑幂等与过期回落；
- **llm_execution**：往返（开启 → 关闭 → 幂等重放 → 未知租约失败）、`MODEL_NOT_FOUND` 结构化信封且无 `Error: ` 前缀、能力非法落 `LEASE_BEGIN_FAILED`、回执字段集与 TTL 断言。

结果：`cargo check --all-targets` **0 告警** · `cargo test`（全目标，bin 亦链接成功）**243/243** ·
`pnpm typecheck` exit 0 · `pnpm run lint` exit 0 · `pnpm run check:channels` orphan 空
（已注册 **99** → 覆盖 98 频道 → 未迁移 **93**，其中 `llm=5` 即 D2-b/c） · `vitest` 频道覆盖 **6/6**。

### 5. GUI 实机冒烟（第四轮）

`pnpm tauri dev`：vite `ready in 482 ms` @5190 → `Running target\debug\lorekeeper.exe`；
webview 已联通（日志出现渲染层 `ipc-client` 输出），**无 panic**，无 `Command ... not found`；
`AI_NOVEL_LOREKEEPER_HOME` 首次启动自动创建 `logs/` + `prompts/`（L1 隔离继续生效）；
冒烟后进程树已清理（已核实残留 `node` 进程命令行均为 pi 自身，非本项目产物）。

### 6. 依赖成本实测

`Cargo.lock` **仅 +1 行**（`lorekeeper` 的依赖边新增 `url`），**零新增 crate 下载** ——
`url` 早已在 `tauri → reqwest` 的传递依赖中。D2-b 将首次真正新增 crate（TLS 后端家族）。

---

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

1. **批次 D2-b（`llm:*` 生成 / 流式，5 频道 + 3 事件）**：依赖已批准
   （`reqwest = { version = "0.13", default-features = false, features = ["json", "stream", "native-tls", "socks"] }`
   + `futures-util`）。范围：`generate`（非流式 + 调用统计落库 `db:log-llm-call`）、
   `generate-stream` / `cancel` / 3 个流事件（`llm:stream-chunk|done|error`）、绕过 CORS 的 SSE 手工解析、
   `finishReason` 显式终态、以及盘点缺口 **G5（流式事件经 webview 桥的性能实测）**；
   随后 **D2-c**：`discover-models` / `test-connection`（后者含 embedding 分支，需评估是否同步引入 `electron/embedding.ts` 的 416 行移植）。
   同时必须补登 `ipc-client.ts` 参数名（`llm:generate` → `['request']`、`llm:generate-stream` → `['requestId','request']`、`llm:cancel` → `['requestId']`、`llm:discover-models` → `['request']`、`llm:test-connection` → `['model','creativeStrategy']`），新防线测试会强制这一点。
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
- ⚠️ **批次 D2 待用户批准依赖**：`reqwest`（或 `tauri-plugin-http`）+ LLM 生成链策略复刻。
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
