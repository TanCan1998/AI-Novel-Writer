# 批次 E：定稿与章节生命周期 - Schema 变更申报（重写版 v2.0）

**文档编号**：`docs/research/2026-10-09-batch-e-schema-proposal.md`
**申报类型**：Ask-first（需架构组批准）
**生效范围**：`tauri-app/src-tauri/src/db/schema.rs` 新增 3 个 `CREATE_*` DDL 常量
**阻塞状态**：**阻塞批次 E 推进**（详见 `docs/handoffs/2026-10-08-tauri-migration-status.md` 第 5 项）

---

## 0. v2.0 重写说明（2026-10-09）

v1.0 的三张表 DDL 经与 Electron 基线 `electron/database.ts`（上游同源、无新提交）逐列比对，
**与基线最终列集严重不符，属凭空重构，全部作废**。典型错误：

| v1.0 错误 | 基线事实 |
|---|---|
| `recovery_candidates` 用 `id`（UUID）+ `snapshot_hash` + `content_blob` | 基线主键为 `candidate_id`（业务键），关联 `run_id/step_id/project_id`，存 `visible_text`/`content_hash`/失败码，状态机 `pending/continued/discarded` |
| `continuity_projection_meta` 按章节多行（coverage_score/items_json/version） | 基线是**单行全局代际指针**：`id CHECK(id='main')` + `generation` + `stale_from_chapter` |
| `chapter_deletion_operations` 用 `rollback_payload BLOB` + 部分唯一索引 | 基线幂等靠 `draft_id UNIQUE`，含稿件/知识库双通道清理状态与 `legacy_knowledge_*` 遗留确认列 |
| 时间戳 `INTEGER strftime('%s','now')` | 基线与 Tauri `schema.rs` 既有约定均为 `TEXT datetime('now')` |
| `draft_id TEXT` 外键 | 基线 `drafts.id` 为 `INTEGER` |

本版 DDL **逐列对齐基线最终列集**（含 `database.ts:602-608`、`995-1005` 迁移补列后的列集），
仅保留将来一次性导入的能力（双栈隔离约束下不要求逐字节一致，但列名/类型/约束必须可对接）。

**范围核查**：批次 E/G 相关其余表经核对**已在 Tauri `schema.rs` 中存在**，本轮不动：
- `finalization_outbox`（含 `content_snapshot` / `knowledge_document_id` 最终列集）✅
- `finalized_draft_import_operations` / `import_global_fact_operations`（导入幂等日志）✅
- `summary_snapshots` 已含 `continuity_facts` / `source_finalization_id` / `projection_generation` ✅
- 4 张散表（`blueprint_commit_operations` / `blueprint_character_sync_operations` / `character_roster_meta` / `character_roster_operations`）经逐行比对**与基线定义完全一致** ✅（基线定义分散在 `electron/repositories/{blueprint-repository,character-roster-schema}.ts`，非 `database.ts`）
- ⚠️ **新发现遗漏**：`idx_llm_calls_time`（基线 `database.ts` 对 `llm_calls(created_at)` 的时间索引）在 Tauri `schema.rs` 中缺失——`llm_calls` 表本身已建，仅缺此索引，随本轮一并补上（见 §2.4）
- ℹ️ 基线 `import_*` 系 11 张表 + 9 个索引缺失，属**批次 G** 范围，本轮不处理

---

## 1. 业务规则摘要

| 表名 | 业务规则 | 阻塞影响 |
|------|----------|----------|
| `recovery_candidates` | 生成失败时的**恢复候选**：按 run/step 记录失败快照与失败码；`source_draft_identity_captured` 锁定草稿身份；`replaces_candidate_id` 自引用实现候选替换链；`status ∈ (pending/continued/discarded)` 状态机 | 草稿恢复/回档频道无法落库 |
| `continuity_projection_meta` | 连续性投影的**全局代际元数据**（单行 `main`）：`generation` 单调递增，`stale_from_chapter` 标记自哪章起投影失效；投影数据一旦落盘受【定稿不可逆红线】约束 | 连续性投影审计频道无法落库 |
| `chapter_deletion_operations` | ADR 0011 章节可恢复删除生命周期：`draft_id UNIQUE` 保证同一草稿只有一个删除事务（幂等）；`manuscript_*` 与 `knowledge_*` 双通道清理状态；`legacy_knowledge_*` 记录知识库遗留确认；`attempt_count` 支持幂等重试 | 章节删除审计/还原频道（4 个）无法落库 |

---

## 2. 申报表 DDL 声明（对齐基线最终列集）

### 2.1 recovery_candidates（生成失败恢复候选表）

**基线出处**：`electron/database.ts:179-204`（建表）+ `:602-608`（旧库补列 `source_draft_id` / `source_draft_version` / `source_draft_identity_captured`，本版直接并入最终列集）。

```sql
CREATE TABLE IF NOT EXISTS recovery_candidates (
  candidate_id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  step_id TEXT NOT NULL,
  project_id TEXT NOT NULL,
  chapter_number INTEGER NOT NULL CHECK(chapter_number > 0),
  chapter_title TEXT NOT NULL DEFAULT '',
  source_snapshot TEXT NOT NULL,
  source_hash TEXT NOT NULL,
  source_draft_id INTEGER DEFAULT NULL,
  source_draft_version INTEGER DEFAULT NULL,
  source_draft_identity_captured INTEGER NOT NULL DEFAULT 0
    CHECK(source_draft_identity_captured IN (0, 1)),
  visible_text TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  failure_code TEXT NOT NULL DEFAULT '',
  failure_reason TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'pending'
    CHECK(status IN ('pending', 'continued', 'discarded')),
  replaces_candidate_id TEXT DEFAULT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  resolved_at TEXT DEFAULT NULL,
  FOREIGN KEY (replaces_candidate_id) REFERENCES recovery_candidates(candidate_id)
);
CREATE INDEX IF NOT EXISTS idx_recovery_candidates_pending
  ON recovery_candidates(status, created_at);
```

**要点**：
- 主键 `candidate_id` 为业务键（非 UUID 自增），与基线导入对接。
- `source_snapshot` / `visible_text` 为 TEXT 正文快照（非 BLOB），对齐基线。
- 自引用 FK：候选被替换后，旧行通过 `replaces_candidate_id` 指向新行。
- 时间戳 `TEXT datetime('now')`，与 Tauri `schema.rs` 既有约定一致（区别于 v1.0 的 INTEGER 秒）。

### 2.2 continuity_projection_meta（连续性投影全局代际元数据）

**基线出处**：`electron/database.ts:657-662`。注意：这是**单行表**，建表后立即种子化。

```sql
CREATE TABLE IF NOT EXISTS continuity_projection_meta (
  id TEXT PRIMARY KEY CHECK (id = 'main'),
  generation INTEGER NOT NULL DEFAULT 0 CHECK (generation >= 0),
  stale_from_chapter INTEGER DEFAULT NULL CHECK (stale_from_chapter IS NULL OR stale_from_chapter > 0)
);
INSERT OR IGNORE INTO continuity_projection_meta (id) VALUES ('main');
```

**要点**：
- v1.0 的「按章节多行 + coverage_score + items_json + FK→finalization_receipts」模型**作废**——基线无此结构；逐章连续性事实存于 `summary_snapshots.continuity_facts`（Tauri 已有）。
- `id = 'main'` CHECK 约束保证物理上单行；`INSERT OR IGNORE` 幂等种子化，须与建表语句同批执行。
- `generation` 与 `summary_snapshots.projection_generation` 配合实现投影代际推进；写入即触发定稿不可逆红线（仓储层禁止 UPDATE 已落盘的投影内容，本表仅推进代际指针）。

### 2.3 chapter_deletion_operations（已定稿章节可恢复删除操作日志）

**基线出处**：`electron/database.ts:233-255`（建表）+ `:995-1005`（旧库补列 `legacy_knowledge_authorization` / `legacy_knowledge_authorized_at`，本版直接并入最终列集）。

```sql
CREATE TABLE IF NOT EXISTS chapter_deletion_operations (
  operation_id TEXT PRIMARY KEY,
  draft_id INTEGER NOT NULL UNIQUE,
  chapter_number INTEGER NOT NULL,
  chapter_title TEXT NOT NULL DEFAULT '',
  finalization_id TEXT NOT NULL,
  target_file_name TEXT NOT NULL DEFAULT '',
  knowledge_document_id TEXT NOT NULL DEFAULT '',
  post_process_run_ids TEXT NOT NULL DEFAULT '[]',
  manuscript_status TEXT NOT NULL DEFAULT 'pending',
  manuscript_error TEXT NOT NULL DEFAULT '',
  knowledge_status TEXT NOT NULL DEFAULT 'pending',
  knowledge_error TEXT NOT NULL DEFAULT '',
  legacy_knowledge_authorization TEXT NOT NULL DEFAULT 'not_required',
  legacy_knowledge_authorized_at TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'pending',
  attempt_count INTEGER NOT NULL DEFAULT 0,
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now')),
  completed_at TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_chapter_deletion_status
  ON chapter_deletion_operations(status);
```

**要点**：
- **幂等约束 = `draft_id UNIQUE`**（同一草稿同时只允许一个删除事务），不是 v1.0 虚构的「chapter_number 上的 partial unique index」。
- `manuscript_status` / `knowledge_status` 双通道独立清理：实体稿发布与知识库跨存储清理可分别失败、分别重试，`*_error` 记录各自失败原因。
- `legacy_knowledge_authorization`（默认 `not_required`）对应 `chapter:confirm-legacy-knowledge-absent` 频道的遗留知识库确认语义（ADR 0011）。
- `finalization_id` / `knowledge_document_id` 为跨存储引用（文件系统 / 知识库），**不建 SQL 外键**——目标不在 SQLite 内，与基线一致。
- `attempt_count` 幂等重试计数；`completed_at` 与双通道全绿后由仓储层写入。

### 2.4 idx_llm_calls_time（既有表补索引，遗漏修正）

**基线出处**：`electron/database.ts`（`CREATE INDEX IF NOT EXISTS idx_llm_calls_time ON llm_calls(created_at);`）。Tauri 侧 `llm_calls` 表已建但缺此索引，导致按时间查询 LLM 调用日志（`llm-stats` 子域）退化为全表扫描。

```sql
CREATE INDEX IF NOT EXISTS idx_llm_calls_time ON llm_calls(created_at);
```

---

## 3. 迁移策略

### 3.1 建表位置与时机

- 三个 DDL 常量新增到 `tauri-app/src-tauri/src/db/schema.rs`，命名沿用既有风格：
  `CREATE_RECOVERY_CANDIDATES` / `CREATE_CONTINUITY_PROJECTION_META` / `CREATE_CHAPTER_DELETION_OPERATIONS`。
- 插入顺序：`CREATE_SUMMARY_SNAPSHOTS` 之后、`CREATE_FINALIZATION_OUTBOX` 附近的定稿生命周期区块内（连续性投影元数据的种子化 INSERT 紧随其建表语句）。
- 因 Tauri 侧为全新库（无旧库兼容负担），基线中 `PRAGMA table_info` 探测补列的迁移代码**不需要平移**——直接按最终列集建表即可。

### 3.2 幂等性

三张表均 `CREATE TABLE IF NOT EXISTS`，重复打开项目安全；`continuity_projection_meta` 的种子行用 `INSERT OR IGNORE`。

### 3.3 外键关系汇总

| 本表列 | 目标 | 行为 |
|---|---|---|
| `recovery_candidates.replaces_candidate_id` | `recovery_candidates.candidate_id`（自引用） | 无级联（对齐基线） |
| `chapter_deletion_operations.draft_id` | `drafts.id` | **不建 FK**：基线未声明（删除流程本身管理草稿生命周期，避免 CASCADE 与可恢复删除语义冲突），对齐基线 |
| `continuity_projection_meta` | 无外键 | 单行元数据 |

---

## 4. 风险评估

| 风险点 | 缓解 |
|---|---|
| `recovery_candidates.source_snapshot`/`visible_text` 文本体积 | 基线同构；候选行随 `status` 流转由 `db:project-clear-generated-data` 等既有清理路径覆盖 |
| `continuity_projection_meta` 单行并发推进 | `generation` 推进走事务内 `UPDATE ... WHERE id='main'`，行级唯一，无并发分叉面 |
| `chapter_deletion_operations` 幂等插入冲突（`draft_id UNIQUE`） | 仓储层捕获约束冲突后回读既有 operation 复用（对齐基线「按定稿身份幂等重试」语义），而非重试自增 |

---

## 5. 依赖项检查

- **Rust 依赖**：无新增。`candidate_id` / `operation_id` 由仓储层按基线语义生成（非 `uuid` crate 强依赖；如需 UUID 可复用现有工具函数）。
- **前端契约**：对应频道已在 `ipc-channels.ts` 声明（continuity 4 / recovery-candidate 4 / finalization-link 1 / chapter-lifecycle 4 / draft 收尾 4），Rust 命令签名随后续仓储实现对齐，无需前端改动。

---

## 6. 批准流程

| 步骤 | 内容 |
|---|---|
| 1 | 架构组批准本文档（v2.0 DDL） |
| 2 | `schema.rs` 落地 3 个常量 + `cargo test`（含建表幂等测试） |
| 3 | 按子域实现仓储 + 命令：recovery-candidate（4）→ continuity（4）→ chapter-lifecycle（4，含 ADR 0011 状态机单元测试硬性要求）→ finalization-link（1）+ draft 收尾 4 |
| 4 | `migrated-channels.ts` 登记 + `check:channels` orphan 清零核对 + 更新 handoff 快照 |

---

## 7. 变更历史

| 日期 | 版本 | 变更内容 |
|------|------|----------|
| 2026-10-09 | 1.0 | 初始版本（三张表 DDL，后经比对证实与基线不符） |
| 2026-10-09 | 2.0 | **重写**：逐列对齐 `electron/database.ts` 最终列集（含 602-608 / 995-1005 迁移补列）；修正 v1.0 全部结构性错误；补范围核查（outbox 等表已存在） |

---

**文档状态**：**待批准**（Ask-first）
**最后更新**：2026-10-09
