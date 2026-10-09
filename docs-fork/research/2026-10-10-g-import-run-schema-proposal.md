# 批次 G schema 申报书：import-run（10 张表 + 7 个索引）

> 2026-10-10 产出。**状态：待用户批准（Ask first：改数据库 schema）**。
> 依据：`docs-fork/agents/pi-development.md`（schema 变更需 Ask first）与 F2 的先例
> （`docs-fork/research/2026-10-09-f2-kb-schema-proposal.md` 先申报、获批后再实施）。

## 0. 为什么需要申报

批次 G（import-run）对应 19 个 `db:import-*` 频道 + `dialog:select-novel-files`。
基线把这套状态机建在 **10 张专用表**上（`electron/database.ts:418-600`），
而 Tauri 侧 schema **目前一张都没有**（只有 E 批次落地的
`finalized_draft_import_operations` / `import_global_fact_operations` 两张幂等日志表）。

→ 不新增这 10 张表，批次 G 无法开工。本申报书即为**批准前的唯一门禁**。

## 1. 待新增的表（逐字对齐基线 `electron/database.ts:418-600`）

### 1.1 `import_runs`（运行主台账，28 列）

```sql
CREATE TABLE IF NOT EXISTS import_runs (
  id TEXT PRIMARY KEY,
  purpose TEXT NOT NULL DEFAULT 'reference'
    CHECK(purpose IN ('reference', 'author-manuscript')),
  root_run_id TEXT NOT NULL,
  effect_namespace TEXT NOT NULL,
  source_fingerprint TEXT NOT NULL,
  manifest_fingerprint TEXT NOT NULL,
  authority_fingerprint TEXT NOT NULL DEFAULT '',
  legacy_source_fingerprint TEXT NOT NULL DEFAULT '',
  source_display_json TEXT NOT NULL DEFAULT '[]',
  locale TEXT NOT NULL CHECK(locale IN ('zh-CN', 'en-US')),
  stage TEXT NOT NULL DEFAULT 'knowledge'
    CHECK(stage IN (
      'parsing', 'prepared', 'knowledge', 'global', 'style', 'blueprints',
      'author-commit', 'author-publish', 'author-postprocess',
      'refresh', 'completed'
    )),
  status TEXT NOT NULL DEFAULT 'ready'
    CHECK(status IN ('ready', 'running', 'failed', 'cancelled', 'completed')),
  completed_batches_json TEXT NOT NULL DEFAULT '{}',
  last_error TEXT NOT NULL DEFAULT '',
  resumable INTEGER NOT NULL DEFAULT 1 CHECK(resumable IN (0, 1)),
  cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancel_requested IN (0, 1)),
  execution_owner TEXT NOT NULL DEFAULT '',
  execution_epoch INTEGER NOT NULL DEFAULT 0,
  lease_expires_at INTEGER NOT NULL DEFAULT 0,
  total_chapters INTEGER NOT NULL,
  total_content_size INTEGER NOT NULL DEFAULT 0,
  manifest_chapter_count INTEGER NOT NULL,
  manifest_content_size INTEGER NOT NULL DEFAULT 0,
  manifest_word_count INTEGER NOT NULL DEFAULT 0,
  completed_chapters INTEGER NOT NULL DEFAULT 0,
  base_run_id TEXT DEFAULT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  completed_at TEXT DEFAULT NULL,
  FOREIGN KEY (base_run_id) REFERENCES import_runs(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_import_runs_source_status
  ON import_runs(source_fingerprint, status, updated_at);
CREATE INDEX IF NOT EXISTS idx_import_runs_resumable
  ON import_runs(resumable, status, updated_at);
```

### 1.2 `import_run_chapters`（定稿章快照，8 列）

```sql
CREATE TABLE IF NOT EXISTS import_run_chapters (
  run_id TEXT NOT NULL,
  chapter_number INTEGER NOT NULL,
  source_id TEXT NOT NULL DEFAULT '',
  source_chapter_number INTEGER NOT NULL DEFAULT 0,
  title TEXT NOT NULL DEFAULT '',
  content_fingerprint TEXT NOT NULL,
  content_size INTEGER NOT NULL,
  content_snapshot TEXT NOT NULL,
  PRIMARY KEY (run_id, chapter_number),
  FOREIGN KEY (run_id) REFERENCES import_runs(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_import_run_chapters_page
  ON import_run_chapters(run_id, chapter_number);
```

### 1.3 `import_run_sources`（来源进度，13 列）

```sql
CREATE TABLE IF NOT EXISTS import_run_sources (
  run_id TEXT NOT NULL,
  source_index INTEGER NOT NULL,
  source_id TEXT NOT NULL,
  source_fingerprint TEXT NOT NULL,
  legacy_source_fingerprint TEXT NOT NULL DEFAULT '',
  display_json TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending', 'completed', 'failed')),
  manifest_fingerprint TEXT NOT NULL DEFAULT '',
  chapter_count INTEGER NOT NULL DEFAULT 0,
  content_size INTEGER NOT NULL DEFAULT 0,
  word_count INTEGER NOT NULL DEFAULT 0,
  last_error TEXT NOT NULL DEFAULT '',
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (run_id, source_id),
  UNIQUE (run_id, source_index),
  FOREIGN KEY (run_id) REFERENCES import_runs(id) ON DELETE CASCADE
);
```

### 1.4 `import_run_source_chapters`（来源原始章快照，8 列）

```sql
CREATE TABLE IF NOT EXISTS import_run_source_chapters (
  run_id TEXT NOT NULL,
  source_id TEXT NOT NULL,
  source_chapter_number INTEGER NOT NULL,
  title TEXT NOT NULL,
  content_fingerprint TEXT NOT NULL,
  content_size INTEGER NOT NULL,
  word_count INTEGER NOT NULL,
  content_snapshot TEXT NOT NULL,
  PRIMARY KEY (run_id, source_id, source_chapter_number),
  FOREIGN KEY (run_id, source_id) REFERENCES import_run_sources(run_id, source_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_import_run_source_chapters
  ON import_run_source_chapters(run_id, source_id, source_chapter_number);
```

### 1.5 `import_source_chapter_map`（章节号映射，4 列）

```sql
CREATE TABLE IF NOT EXISTS import_source_chapter_map (
  purpose TEXT NOT NULL CHECK(purpose IN ('reference', 'author-manuscript')),
  source_id TEXT NOT NULL,
  source_chapter_number INTEGER NOT NULL,
  chapter_number INTEGER NOT NULL,
  PRIMARY KEY (purpose, source_id, source_chapter_number),
  UNIQUE (purpose, chapter_number)
);
```

### 1.6 `import_run_receipts`（效果收据，12 列）

```sql
CREATE TABLE IF NOT EXISTS import_run_receipts (
  run_id TEXT NOT NULL,
  schema_version INTEGER NOT NULL DEFAULT 1,
  effect_namespace TEXT NOT NULL,
  effect_key TEXT NOT NULL,
  stage TEXT NOT NULL,
  batch_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'prepared' CHECK(state IN ('prepared', 'committed')),
  effect_receipt_json TEXT DEFAULT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (run_id, stage, batch_id),
  UNIQUE (effect_namespace, effect_key),
  FOREIGN KEY (run_id) REFERENCES import_runs(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_import_run_receipts_state
  ON import_run_receipts(run_id, state, stage);
```

### 1.7 `import_run_knowledge_receipts`（知识库投影收据，10 列）

```sql
CREATE TABLE IF NOT EXISTS import_run_knowledge_receipts (
  run_id TEXT NOT NULL,
  chapter_number INTEGER NOT NULL,
  purpose TEXT NOT NULL,
  source_id TEXT NOT NULL,
  source_chapter_number INTEGER NOT NULL,
  content_fingerprint TEXT NOT NULL,
  document_id TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state = 'committed'),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (run_id, chapter_number),
  FOREIGN KEY (run_id, chapter_number)
    REFERENCES import_run_chapters(run_id, chapter_number) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_import_run_knowledge_receipts_affiliation
  ON import_run_knowledge_receipts(
    purpose, source_id, source_chapter_number, content_fingerprint, state
  );
```

### 1.8 `import_reference_documents`（参考文档幂等，9 列）

```sql
CREATE TABLE IF NOT EXISTS import_reference_documents (
  document_id TEXT PRIMARY KEY,
  idempotency_key_hash TEXT NOT NULL UNIQUE,
  content_hash TEXT NOT NULL,
  chunk_set_hash TEXT NOT NULL,
  expected_chunk_count INTEGER NOT NULL,
  corpus_kind TEXT NOT NULL CHECK(corpus_kind = 'reference'),
  state TEXT NOT NULL DEFAULT 'prepared' CHECK(state IN ('prepared', 'committed')),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### 1.9 ⚠️ `import_legacy_identity_bridge` + `import_source_aliases`（需你决策）

```sql
CREATE TABLE IF NOT EXISTS import_legacy_identity_bridge (
  id TEXT PRIMARY KEY,
  ciphertext_hex TEXT NOT NULL,
  iv_hex TEXT NOT NULL,
  auth_tag_hex TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS import_source_aliases (
  alias_digest TEXT PRIMARY KEY,
  alias_kind TEXT NOT NULL CHECK(alias_kind IN ('location', 'file')),
  source_id TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_import_source_aliases_source
  ON import_source_aliases(source_id);
```

**这两张表服务于基线特有的「旧版来源身份桥」**：基线用 AES-GCM 把旧的来源身份密存入
`import_legacy_identity_bridge`（密钥来自 `electron/services/import-source-identity-secret.ts`，
32 行），并用 `import_source_aliases` 记录位置/文件别名摘要，以便旧项目在迁移后仍能
把「同一来源」认出来。

**Tauri 侧是否需要的判断依据**：
- Tauri 项目目录是 `.lore/` 且**刻意不与 Electron 项目互通**（L3），Tauri 不会遇到
  Electron 时代的旧来源身份；
- 但 **Tauri 自身**若将来需要「重命名/移动来源文件后仍认出同一来源」，`import_source_aliases`
  仍有价值（`alias_kind: location | file` 的摘要别名与 Electron 无关）。

→ **建议**：`import_source_aliases` **保留**（对 Tauri 自身有用，零额外依赖）；
`import_legacy_identity_bridge` **暂不建**（无遗留数据可桥；若将来需要跨栈导入再单独申报）。
请你在确认时一并答复这一点。

## 2. 索引清单（7 个，已含在上文）

| 索引 | 表 |
|---|---|
| `idx_import_runs_source_status` | `import_runs` |
| `idx_import_runs_resumable` | `import_runs` |
| `idx_import_run_chapters_page` | `import_run_chapters` |
| `idx_import_run_source_chapters` | `import_run_source_chapters` |
| `idx_import_run_receipts_state` | `import_run_receipts` |
| `idx_import_run_knowledge_receipts_affiliation` | `import_run_knowledge_receipts` |
| `idx_import_source_aliases_source` | `import_source_aliases` |

## 3. 边界与纪律

- **不改动已有表**：`finalized_draft_import_operations` / `import_global_fact_operations`
  （E 批次已落地）保持不变；`drafts` / `contents` / `finalization_outbox` 等既有表零改动。
- **建表幂等**：全部 `IF NOT EXISTS`，与 `db/schema.rs::create_tables` 的既有约定一致
  （`read` / `commit` 路径每次打开都会复跑）。
- **外键**：`PRAGMA foreign_keys = ON` 已启用（`db/mod.rs`），故级联语义
  （`ON DELETE CASCADE` / `SET NULL`）会真实生效。
- **列集对齐基线最终列集**（含历史 `ALTER` 追加列），保留将来一次性导入基线数据的能力。
- **无新依赖**：纯 DDL + `rusqlite` 既有能力。
- **零双栈风险**：新表只写入 Tauri 的 `.lore/lorekeeper.db`，与基线 `.vela/vela.db` 无交集。

## 4. 获批后的实施分批（建议）

| 分批 | 范围 | 频道 | 风险 |
|---|---|---|---|
| **G1** | 检视面：`dialog:select-novel-files`（真实文件选择 + grant 签发）+ `import-inspection-store` + `db:import-run-prepare-inspection` + `db:import-run-author-preview` | 3 | 低（无状态机推进） |
| **G2** | 状态机主体：`import-run` 的 prepare / beginParsing / commitParsedSource / failParsedSource / finalizeParsing / get / listResumable / listChapterBatch | 5 | 中（断点恢复语义） |
| **G3** | 执行租约与批次推进：`startOrResume` / `renewExecution` / `restart` / `requestCancel` / `cancelAtBoundary` / `completeBatch` / `advanceStage` / `fail` / `complete` + effect receipts（prepare/commit）+ `db:import-global-facts-commit` | 11 | 高（租约 + 幂等收据 + 取消边界） |
| **G4** | 收口：`kb:import-reference-text` 去占位 + 前端登记 + GUI 冒烟 | — | 中 |

> 每分批完成后照例：`cargo test` / `check:channels` / `typecheck` / `lint` / `cargo fmt --check` + 快照 + 推送。

## 5. 基线规模（供分批排期参考）

| 文件 | 行数 | 分批归属 |
|---|---|---|
| `electron/repositories/import-run-repository.ts` | **2551** | G2 / G3 |
| `electron/controllers/import-controller.ts` | 671 | G1～G4 的频道语义参考 |
| `electron/repositories/finalized-draft-import-repository.ts` | 497 | **已迁移**（E 批次，`finalized_draft_import_repository.rs`） |
| `electron/services/import-inspection-store.ts` | 192 | G1 |
| `electron/repositories/import-source-identity-repository.ts` | 192 | G1/G2（若保留 aliases） |
| `electron/repositories/import-global-facts-repository.ts` | 181 | G3 |
| `electron/services/import-source-identity-secret.ts` | 32 | 依赖 §1.9 的决策 |
| `src/shared/import-run.ts`（契约类型） | 352 | 全程 |

## 6. 待确认项（请逐条答复）

1. **是否批准新增上述表？**（建议：9 张 + `import_source_aliases`，即共 10 张中建 9 张）
2. **`import_legacy_identity_bridge` 是否要建**（§1.9：建议暂不建）？
3. **分批方案（G1→G4）是否认可**？若希望先只做 G1+G2（低风险段），也可调整。
4. **`import_source_chapter_map` 的 `purpose` 取值**是否保持不变（`reference` / `author-manuscript`）？
