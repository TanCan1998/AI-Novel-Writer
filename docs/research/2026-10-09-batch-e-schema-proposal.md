# 批次 E：定稿与章节生命周期 - Schema 变更申报

**文档编号**：`docs/research/2026-10-09-batch-e-schema-proposal.md`
**申报类型**：Ask-first（需架构组批准）
**生效范围**：`tauri-app/src-tauri/src/db/schema.rs` 中的 `CREATE TABLE` DDL 声明
**阻塞状态**：**阻塞批次 E 推进**（详见 `docs/handoffs/2026-10-08-tauri-migration-status.md` 第 5 项）

---

## 1. 申报背景

### 1.1 阻塞点定位

根据 `docs/handoffs/2026-10-08-tauri-migration-status.md` 第 5 项：
> **批次 E（定稿与章节生命周期）阻塞点**：需新增 3 张核心表以支撑草稿恢复、连续性投影审计、章节删除可恢复生命周期。当前 `db/schema.rs` 缺少这三张表的 DDL 定义，导致仓储层无法落库。

### 1.2 业务规则摘要

| 表名 | 业务规则 | 阻塞影响 |
|------|----------|----------|
| `recovery_candidates` | 支持 30 天 TTL 自动滚动清理，通过 `snapshot_hash` 实现幂等覆盖守卫 | 草稿恢复/回档功能无法落库 |
| `continuity_projection_meta` | 对接 Issue #205 逐项审稿，数据一旦落盘触发【定稿不可逆红线】，禁止发生任何 UPDATE 行为，使用 FOREIGN KEY 对 `finalization_receipts` 实施 ON DELETE RESTRICT 拦截 | 连续性投影审计功能无法落库 |
| `chapter_deletion_operations` | 支撑 ADR 0011 章节可恢复删除生命周期，通过状态机约束实施操作幂等，同一章节在未被最终清理或还原前，只能有一个激活（`pending`）的删除事务 | 章节删除审计与还原功能无法落库 |

### 1.3 架构约束

- **零代码红线**：本会话仅编写 DDL 文档，**禁止**修改 `tauri-app/src-tauri/src/` 下的任何 `.rs` 业务文件。
- **幂等建表**：所有 DDL 必须使用 `CREATE TABLE IF NOT EXISTS`，对齐 Electron 基线列集。
- **Ask-first**：变更需架构组审批通过后，方可推进批次 E 的仓储与命令实现。

---

## 2. 申报表 DDL 声明

### 2.1 recovery_candidates（草稿恢复候选表）

**业务规则**：支持 30 天 TTL 自动滚动清理，通过 `snapshot_hash` 实现幂等覆盖守卫。

**DDL 声明**：

```sql
CREATE TABLE IF NOT EXISTS recovery_candidates (
    id TEXT PRIMARY KEY NOT NULL,
    draft_id TEXT NOT NULL,
    snapshot_hash TEXT NOT NULL,
    content_blob BLOB NOT NULL,
    metadata_json TEXT DEFAULT '{}',
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    FOREIGN KEY(draft_id) REFERENCES drafts(id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_rc_draft_hash ON recovery_candidates(draft_id, snapshot_hash);
CREATE INDEX IF NOT EXISTS idx_rc_created_at ON recovery_candidates(created_at);
```

**关键字段说明**：
- `id`：主键，建议使用 UUID（由 Rust `uuid::Uuid::new_v4()` 生成）
- `draft_id`：外键关联 `drafts.id`，级联删除
- `snapshot_hash`：幂等覆盖守卫，相同 `draft_id + snapshot_hash` 的记录视为覆盖
- `content_blob`：二进制正文快照（对齐基线 `drafts.content_id` 的正文内容）
- `metadata_json`：JSON 元数据（如恢复策略、TTL 标记）
- `created_at`：Unix 时间戳（秒），用于 30 天 TTL 滚动清理

**索引设计**：
- `idx_rc_draft_hash`：唯一索引，实现幂等覆盖守卫
- `idx_rc_created_at`：普通索引，支持 TTL 清理查询

---

### 2.2 continuity_projection_meta（连续性投影元数据表）

**业务规则**：对接 Issue #205 逐项审稿，数据一旦落盘触发【定稿不可逆红线】，禁止发生任何 UPDATE 行为，使用 FOREIGN KEY 对 `finalization_receipts` 实施 ON DELETE RESTRICT 拦截。

**DDL 声明**：

```sql
CREATE TABLE IF NOT EXISTS continuity_projection_meta (
    id TEXT PRIMARY KEY NOT NULL,
    chapter_number INTEGER NOT NULL CHECK (chapter_number >= 0),
    coverage_score REAL NOT NULL CHECK (coverage_score >= 0.0 AND coverage_score <= 1.0),
    items_json TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    FOREIGN KEY(id) REFERENCES finalization_receipts(id) ON DELETE RESTRICT
);
CREATE INDEX IF NOT EXISTS idx_cpm_chapter ON continuity_projection_meta(chapter_number);
```

**关键字段说明**：
- `id`：主键，外键关联 `finalization_receipts.id`（定稿收据）
- `chapter_number`：章节号（非负整数）
- `coverage_score`：连续性覆盖评分（0.0 - 1.0，浮点数）
- `items_json`：JSON 格式的连续性事实清单（由 Rust serde 序列化）
- `version`：版本号（整数，默认 1，每次更新递增）
- `updated_at`：Unix 时间戳（秒），**禁止 UPDATE**（定稿不可逆红线）

**索引设计**：
- `idx_cpm_chapter`：普通索引，支持按章节号查询连续性投影

**约束说明**：
- `CHECK(chapter_number >= 0)`：确保章节号非负
- `CHECK(coverage_score >= 0.0 AND coverage_score <= 1.0)`：确保评分在有效范围内
- `FOREIGN KEY(id) REFERENCES finalization_receipts(id) ON DELETE RESTRICT`：定稿收据删除时，连续性投影元数据**禁止删除**（必须先清理相关数据）

---

### 2.3 chapter_deletion_operations（章节删除操作日志表）

**业务规则**：支撑 ADR 0011 章节可恢复删除生命周期，通过状态机约束实施操作幂等，同一章节在未被最终清理或还原前，只能有一个激活（`pending`）的删除事务。

**DDL 声明**：

```sql
CREATE TABLE IF NOT EXISTS chapter_deletion_operations (
    id TEXT PRIMARY KEY NOT NULL,
    chapter_number INTEGER NOT NULL,
    operation_status TEXT NOT NULL CHECK (operation_status IN ('pending', 'completed', 'failed', 'restored')),
    rollback_payload BLOB NOT NULL,
    error_message TEXT,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_cdo_active_deletion ON chapter_deletion_operations(chapter_number) WHERE operation_status = 'pending';
```

**关键字段说明**：
- `id`：主键，建议使用 UUID（由 Rust `uuid::Uuid::new_v4()` 生成）
- `chapter_number`：章节号
- `operation_status`：操作状态机（枚举值：`pending` / `completed` / `failed` / `restored`）
- `rollback_payload`：二进制回滚快照（如删除前的草稿/定稿数据）
- `error_message`：错误信息（失败时记录）
- `created_at`：Unix 时间戳（秒）
- `updated_at`：Unix 时间戳（秒），用于状态流转追踪

**索引设计**：
- `idx_cdo_active_deletion`：部分唯一索引，确保同一章节在同一时间只有一个 `pending` 状态的操作（状态机约束）

**状态机约束**：
- `pending`：待执行删除操作
- `completed`：删除操作成功完成
- `failed`：删除操作失败（需人工介入或重试）
- `restored`：删除操作被还原（恢复章节）

**业务规则**：
- 同一章节在未被最终清理或还原前，只能有一个激活（`pending`）的删除事务
- 删除操作失败后，可重新提交删除请求（生成新的 `id`，状态仍为 `pending`）
- 还原操作会重置章节状态，并标记旧删除操作为 `restored`

---

## 3. 迁移策略

### 3.1 建表时机

新增 DDL 语句应插入到 `tauri-app/src-tauri/src/db/schema.rs` 的 `create_tables()` 函数中，遵循以下顺序：

1. **在 `CREATE_SUMMARY_SNAPSHOTS` 之后插入**（批次 E 在 `summary_snapshots` 之上）
2. **在 `CREATE_IMPORT_OPERATIONS` 之前插入**（避免与其他导入表混淆）

### 3.2 幂等性保证

所有 DDL 均使用 `CREATE TABLE IF NOT EXISTS`，确保：
- 首次运行时创建表
- 重复运行时跳过已存在的表（幂等）
- 与 Electron 基线列集对齐（保留将来一次性导入原项目数据的能力）

### 3.3 外键约束

- `recovery_candidates.draft_id` → `drafts.id`（ON DELETE CASCADE）
- `continuity_projection_meta.id` → `finalization_receipts.id`（ON DELETE RESTRICT）
- `chapter_deletion_operations`：无外键约束（操作日志独立，不依赖其他表）

---

## 4. 风险评估与缓解措施

### 4.1 风险点

| 风险点 | 影响 | 概率 | 缓解措施 |
|--------|------|------|----------|
| `continuity_projection_meta` 的 `ON DELETE RESTRICT` 可能阻塞定稿收据删除 | 高 | 低 | 在删除定稿收据前，必须先清理关联的连续性投影元数据（由仓储层实现） |
| `chapter_deletion_operations` 的部分唯一索引可能因并发冲突导致插入失败 | 中 | 中 | 在 Rust 层实现乐观锁，捕获唯一约束冲突并重试（或提示用户手动处理） |
| `recovery_candidates` 的 `content_blob` 可能占用大量存储空间 | 中 | 高 | 实现定期 TTL 清理（30 天滚动），并在 `metadata_json` 中记录清理策略 |

### 4.2 向后兼容性

- **无历史数据迁移需求**：三张表均为新增表，无需回填历史数据
- **现有表无变更**：仅新增表，不影响现有 `drafts` / `finalization_receipts` / `blueprints` 等表的列定义

---

## 5. 依赖项检查

### 5.1 Rust 依赖

- **uuid**：用于生成 `id` 主键（`uuid::Uuid::new_v4()`）
- **serde**：用于序列化 `items_json` / `metadata_json`（如已有，无需新增）

### 5.2 Electron 基线对齐

- **列集对齐**：三张表的列定义已对齐 Electron 基线 `electron/database.ts` 的最终列集
- **数据结构**：JSON 字段使用标准 JSON 格式（非 SQLite JSON 扩展）

---

## 6. 批准流程

### 6.1 审批角色

| 角色 | 职责 |
|------|------|
| 架构组 | 批准 Schema 变更的合理性、风险可控性 |
| Rust 团队 | 审查 DDL 声明与 Rust 仓储实现的兼容性 |
| 前端团队 | 确认连续性投影元数据/章节删除操作在 UI 上的展示需求 |

### 6.2 审批通过后行动

1. **架构组批准** → 更新本文档状态为 **已批准**
2. **Rust 团队实现仓储层** → 在 `tauri-app/src-tauri/src/db/repositories/` 下实现三张表的仓储接口
3. **前端团队实现命令层** → 在 `tauri-app/src-tauri/src/commands/` 下实现三张表对应的 Tauri 命令
4. **推进批次 E** → 更新 `docs/handoffs/2026-10-08-tauri-migration-status.md`，标记批次 E 为 **进行中**

---

## 7. 附录

### 7.1 相关文档

- **进度快照**：`docs/handoffs/2026-10-08-tauri-migration-status.md`
- **ADR 0011**：章节可恢复删除生命周期（待补充）
- **Issue #205**：连续性投影逐项审稿（待补充）
- **基线 schema**：`electron/database.ts` 的 `createTables` 函数

### 7.2 变更历史

| 日期 | 版本 | 变更内容 | 作者 |
|------|------|----------|------|
| 2026-10-09 | 1.0 | 初始版本，申报三张核心表 DDL 声明 | 数据库审计专家 |

---

**文档状态**：**待批准**（Ask-first）
**最后更新**：2026-10-09
