# G3 开工清单：导入运行执行租约 / 批次推进 / effect receipts

> 批次 G 第三步（用户决策 2026-10-10：**不再细分，只切两刀 —— G3a 仓储 + G3b 频道**）。
> 范围：**13 个频道** = 12 个 `db:import-run-*` 状态机频道 + `db:import-global-facts-commit`。
> 基线事实源：`electron/repositories/import-run-repository.ts`（2551 行）+ `electron/repositories/import-global-facts-repository.ts` + `electron/controllers/db-controller.ts:183-480`。

## 1. 频道盘点（G3b 一次性落地）

| 族 | 频道 | 基线仓储方法 |
|---|---|---|
| 执行租约 | `db:import-run-start-resume` | `startOrResume` |
| 执行租约 | `db:import-run-renew-execution` | `renewExecution` |
| 执行租约 | `db:import-run-restart` | `restart` |
| 执行租约 | `db:import-run-request-cancel` | `requestCancel` |
| 执行租约 | `db:import-run-cancel-at-boundary` | `cancelAtBoundary` |
| 批次推进 | `db:import-run-complete-batch` | `completeBatch`（命令层前置 `isImportRunDirectCheckpointStage` 校验） |
| 批次推进 | `db:import-run-advance-stage` | `advanceStage` |
| 批次推进 | `db:import-run-fail` | `fail` |
| 批次推进 | `db:import-run-complete` | `complete` |
| effect receipts | `db:import-run-effect-receipt-get` | `getEffectReceipt`（读频道，失败 reject） |
| effect receipts | `db:import-run-effect-receipt-prepare` | `prepareEffectReceipt` |
| effect receipts | `db:import-run-effect-receipt-commit` | `commitEffectReceipt` |
| 全局事实 | `db:import-global-facts-commit` | `ImportGlobalFactsRepository.commit` |

## 2. 已确认的决策与刻意偏离（G3）

- **D1（刻意偏离）**：`adoptLegacyCompletedRun` **不移植**。该函数把旧版（`~/.vela`）已完成的运行重定向到新指纹；Tauri 双栈隔离下 `.lore` 是全新库，**不存在 legacy 运行**，函数在 Tauri 语境为恒空操作。恢复需用户确认。
- **D2（刻意偏离）**：`import_global_fact_operations` 台账表**懒建**（对齐基线 `ensureLedger()` 的 `CREATE TABLE IF NOT EXISTS`），**不改 `db/schema.rs`**——G schema 已获批且刻意固定 9 表，不新增。
- **D3（刻意偏离）**：`canonicalize` 的键排序在 Rust 侧用**字节序**（`sort`），不模拟 JS `localeCompare('en-US')`。理由：`canonicalPayload` 的哈希只在 Tauri 内部自洽（存 → 回读比对），G schema 决策已明确「不要求与 Node 逐字节一致的哈希」。
- **D4**：`completeBatch` 的 direct-stage 前置校验（`isImportRunDirectCheckpointStage`）放在**命令层**（G3b），对齐基线 db-controller 的 handler 前置断言。
- **依赖就绪（已核实）**：`project_core_repository::{get, update}`、`character_roster_repository::{read, commit(conn, &Value)}`、`blueprint_repository::commit_range`、`finalized_draft_import_repository::{preview, commit}` 均已迁移；`blueprint_repository` 内部 helper（`read_commit_operation` / `read_exact_range` / `read_character_sync_operation` / `snapshot_with_character_sync_facts` / `assert_authoritative_character_sync_completion`）齐备，`get_committed_range_operation` 可直接拼装。

## 3. G3a 落地清单（仓储层）

### 3.1 `repositories/import_run_repository.rs` 追加

- 常量：`DEFAULT_EXECUTION_LEASE_MS = 15 * 60_000`、`MAX_EFFECT_RECEIPT_PAYLOAD_BYTES = 16 MiB`、`AUTHOR_CHAPTER_CHECKPOINT = /^chapter:([1-9]\d*)$/`
- 类型：`ImportRunExecutionAuthority` / `ImportRunExecutionLease` / `ImportRunStartResult` / `ImportRunBatchCheckpointResult` / `ImportRunEffectKind` / `ImportRunEffectReceipt` / `ImportRunPrepareEffectReceiptRequest` / `ImportRunEffectCommitResult`
- 租约族：`assert_execution_authority` / `assert_execution` / `start_or_resume` / `renew_execution` / `restart` / `request_cancel` / `cancel_at_boundary`
- 批次推进族：`next_stage_for_run`（`REFERENCE_NEXT_STAGE` / `AUTHOR_NEXT_STAGE` 映射）/ `author_checkpoint_chapter_number` / `assert_checkpoint_can_apply` / `assert_stage_checkpoint_complete` / `apply_batch_checkpoint` / `complete_batch` / `advance_stage` / `fail` / `complete`
- effect receipts：`canonicalize` / `canonical_payload` / `exact_keys` / `assert_effect_payload_schema` / `assert_effect_payload_binding` / `assert_author_effect_run_binding` / `assert_committed_effect_schema` / `assert_completed_blueprint_sync_operation` / `assert_blueprint_effect_authority` / `assert_committed_effect_authority` / `validate_effect_stage` / `row_to_effect_receipt` / `get_effect_receipt` / `prepare_effect_receipt` / `commit_effect_receipt`

### 3.2 新模块 `repositories/import_global_facts_repository.rs`

平移 `import-global-facts-repository.ts`：`normalized_request`（13 个文本字段 trim + 枚举校验 + 总章数/章节字数校验）/ `hash_request` / `core_snapshot` / `parse_receipt` / `ensure_ledger`（D2 懒建表）/ `get_committed_operation`（核心台账 + roster factHash 比对，拒绝历史操作冒充当前事实）/ `commit`（事务：幂等重放或 `ProjectCoreRepository.update` + `CharacterRosterRepository.commit`）。

### 3.3 下游读回（供 effect receipt 权威校验）

- `blueprint_repository.rs` 追加 `get_committed_range_operation(operation_id)`（对齐 `getCommittedRangeOperation`，`idempotent: false`）
- `finalized_draft_import_repository.rs` 追加 `get_committed_operation(operation_id, chapters)`（对齐 `getCommittedOperation`，复用 `parse_stored_receipt` / `verify_stored_facts`）

### 3.4 测试（目标 +30～40，移植基线）

- `electron/repositories/__tests__/import-run-execution-lease.test.ts`
- `electron/repositories/__tests__/import-run-receipt.test.ts`
- `electron/repositories/__tests__/import-run-state-machine.test.ts`
- global-facts 既有测试（若有）

## 4. G3b 落地清单（频道层）

- `commands/db.rs`：13 个 `#[tauri::command]` + 信封结构（对齐 `ipc-channels.ts` 的 `{ success, …, error? }`）；读频道（effect-receipt-get）失败 reject，其余 `{ success: true, … }`；写面错误按 MUTATING 规则带 `"Error: "` 前缀（`mutating_error`）。
- `lib.rs` `invoke_handler` 注册 13 命令。
- 前端：`ipc-client.ts` 的频道门禁登记（若批次 C 以来的门禁表存在）；`ipc-channels.ts` 类型已就位，无需改前端组件（`import-workflow.ts` 已按契约调用）。

## 5. 验收

- `cargo test --lib` 全绿（含新增移植测试）；`cargo check --all-targets` 0 告警；`cargo fmt --check` 干净
- `pnpm typecheck` / `pnpm run lint`（tauri-app 下）干净；`check:channels` orphan 空
- 13 个频道注册后「未迁移 22 → 9」（仅剩 mcp 9 个）
