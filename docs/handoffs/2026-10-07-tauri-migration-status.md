# Tauri 迁移进度快照（2026-10-07）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> 批次 A–C `project_core` / `characters` 的历史细节见
> [`2026-10-06-tauri-migration-status.md`](./2026-10-06-tauri-migration-status.md)；
> channel 级盘点见 [`docs/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-07 · 第十一次）

| 项 | 值 |
|---|---|
| 仓库 | **`TanCan1998/Lorekeeper`**（原名 `AI-Novel-Writer`；仍为 `EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork） |
| 分支 | `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm 包 `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 当前阶段 | **批次 C 进行中**：`project_core` ✅、`characters/roster` ✅（`69fc50c`）、**`blueprints` S2-a ✅ 完成**（本轮）；下一步 **S2-b**（`db:blueprint-commit-range`） |
| 已注册命令 | **48**（骨架 1 + A 11 + B 22 + C project_core 4 + characters 3 + blueprints S2-a 7） |
| 双栈隔离 | L0 安装标识 / L1 `~/.lorekeeper` / L2 `<root>/.vela/lorekeeper.db` 均独立；L3（`.vela` 改名）押后 |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc @ `D:\Environment\rust\`（脚本内显式设 `RUSTUP_HOME`/`CARGO_HOME`） |
| 验证状态 | ✅ `cargo check --all-targets` **0 告警** · ✅ `cargo test --lib` **101/101** · ✅ `pnpm typecheck` exit 0 · ✅ `pnpm run lint` exit 0 |

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

## 下一步：S2-b（1 频道）

`db:blueprint-commit-range` —— 需实现（基线与 Rust 两侧对照）：

- `canonicalize` / `commit_payload_hash`（SHA-256，键序规范化；可复用 `sha2` crate，
  已在 `Cargo.lock`，无需新增下载）
- `assert_exact_range` / `read_exact_range`（范围完整且唯一，`full` 模式必须从第 1 章起）
- 幂等提交事务 + 回读校验（`same_persisted_blueprint`）+ `blueprint_commit_operations` 落库
  + 同步创建 `blueprint-sync-<op>` 待处理操作
- 返回 `{ success, receipt?, error? }`，receipt 含 `characterSyncInput` / `characterSyncOperation`

随后 **S2-c**（3 频道）：`db:blueprint-character-sync-{list-pending,get,complete}`，
依赖 `CharacterRosterRepository` 回读与事实校验。

---

## 遗留项（沿用 2026-10-06 快照）

- 未验证：批次 C **GUI 实机验证**（打开真实项目读写 project_core/角色/蓝图）、
  双栈同库行为对照、`vitest` 全量超时定位、`cargo fmt --check` 未纳入验收。
- 押后：L3（`.vela` → `.lorekeeper`）、可见品牌（`brand.ts` / i18n 标题）。
- 未迁频道的错误文案友好化（遗留项 12）。
