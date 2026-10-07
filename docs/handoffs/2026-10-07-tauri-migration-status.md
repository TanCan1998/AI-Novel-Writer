# Tauri 迁移进度快照（2026-10-07）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> 批次 A–C `project_core` / `characters` 的历史细节见
> [`2026-10-06-tauri-migration-status.md`](./2026-10-06-tauri-migration-status.md)；
> channel 级盘点见 [`docs/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-07 · 第十三次）

| 项 | 值 |
|---|---|
| 仓库 | **`TanCan1998/Lorekeeper`**（原名 `AI-Novel-Writer`；仍为 `EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork） |
| 分支 | `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm 包 `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 当前阶段 | **批次 C 进行中**：`project_core` ✅、`characters/roster` ✅（`69fc50c`）、**`blueprints` 11 频道全部 ✅（S2-a/b/c）**；下一步 **批次 C 下一子域：`drafts`（16 频道）** |
| 已注册命令 | **52**（骨架 1 + A 11 + B 22 + C project_core 4 + characters 3 + blueprints 11） |
| 双栈隔离 | L0 安装标识 / L1 `~/.lorekeeper` / L2 `<root>/.vela/lorekeeper.db` 均独立；L3（`.vela` 改名）押后 |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc @ `D:\Environment\rust\`（脚本内显式设 `RUSTUP_HOME`/`CARGO_HOME`） |
| 验证状态 | ✅ `cargo check --all-targets` **0 告警** · ✅ `cargo test --lib` **114/114** · ✅ `pnpm typecheck` exit 0 · ✅ `pnpm run lint` exit 0 |

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

## 下一步：批次 C 下一子域 `drafts`（16 频道）

- 建 `src-tauri/src/repositories/draft_repository.rs`；`db/schema.rs` 已有 `contents` / `drafts` /
  `finalization_outbox` 建表（drafts 子域迁移时不再重复建表）。
- 逐条对照 `electron/repositories/draft-repository.ts`；注意：
  - `db:draft-import-finalized-batch` / `db:draft-create` / `db:draft-update-status` /
    `db:draft-update-content` / `db:draft-delete` 属 MUTATING；
  - 依赖 `draft-source-guard.ts`（`SOURCE_DRAFT_CHANGED` 等错误码）与
    `draft-source-dependency.ts` 的共享类型；
  - 蓝图/角色同步所需 `finalization_outbox` 已在 schema 就位。
- 每子域固定流程：仓储 + 命令 + `ipc-client` 登记 + `cargo check`（0 告警）+
  `cargo test --lib` + `pnpm typecheck/lint` → 单独 commit。

---

## 遗留项（沿用 2026-10-06 快照）

- 未验证：批次 C **GUI 实机验证**（打开真实项目读写 project_core/角色/蓝图）、
  双栈同库行为对照、`vitest` 全量超时定位、`cargo fmt --check` 未纳入验收。
- 押后：L3（`.vela` → `.lorekeeper`）、可见品牌（`brand.ts` / i18n 标题）。
- 未迁频道的错误文案友好化（遗留项 12）。
- 批次 G 需要的 `BlueprintRepository.getCommittedRangeOperation`（无 IPC 频道，被
  `import-run-repository.ts` 使用）尚未移植，随批次 G 一并落地。
