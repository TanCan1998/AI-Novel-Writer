# 批次 F2：知识库（`kb:*` 15 频道 + dialog 2）Schema 变更申报

**文档编号**：`docs-fork/research/2026-10-09-f2-kb-schema-proposal.md`
**申报类型**：Ask-first（需用户批准后方可写入 `db/schema.rs`）
**生效范围**：`tauri-app/src-tauri/src/db/schema.rs` 新增 KB 表 DDL；`src/db/kb/` 新模块
**前置状态**：向量层已落地（commit `4aff3f65`，`db/vector.rs` 的 `LocalVectorIndex`）

---

## 1. 为什么需要这份申报

三层边界（`AGENTS.md`）将「改数据库 schema」列为 **⚠️ Ask first**。批次 E 亦循此例先申报后实施。
本批次要在 Tauri 专属项目库 `<root>/.vela/lorekeeper.db` 中新增知识库表，属首次引入 F2 存储结构，故须先获批。

## 2. 事实前提（本轮实测）

| 项 | 结论 |
|---|---|
| 新增 Cargo 依赖 | **无**。`hnsw_rs 0.3.4` / `jieba-rs 0.7.0` / `tokio` / `rusqlite 0.40.2 [bundled]` / `tauri-plugin-dialog 2` 均已在 `Cargo.toml` |
| SQLite FTS5 | **可用**。`libsqlite3-sys` 的 bundled 构建显式带 `-DSQLITE_ENABLE_FTS5`（registry `build.rs` 实测） |
| 基线 KB 存储 | 全在 **LanceDB**（`chunks` / `documents` / `chunks__space_*` 表）+ `.vela/embedding-spaces.json` / `.vela/vectors.json`；**SQLite 中无 kb 表** |
| 基线文档记录列集 | `ChunkRecord { id, docId, fileName, chapterNumber?, chapterTitle?, text, chunkIndex, totalChunks, importedAt, corpusKind }`（`electron/vector-store.ts:16`） |
| 文档聚合列集 | `DocumentInfo { id, fileName, importedAt, chunkCount, filePath, corpusKind }` |

→ 无「基线 SQLite 最终列集」可对齐，Tauri 侧为**全新设计**；但**列语义逐项对齐基线 `ChunkRecord` / `DocumentInfo`**。

## 3. 拟新增的 SQLite DDL

```sql
-- 知识库文档（聚合 kb_chunks 的文档元信息，对齐 DocumentInfo）
CREATE TABLE IF NOT EXISTS kb_documents (
  id            TEXT PRIMARY KEY,
  file_name     TEXT NOT NULL,
  file_path     TEXT NOT NULL DEFAULT '',
  corpus_kind   TEXT NOT NULL DEFAULT 'unknown',   -- reference | project-knowledge | unknown
  imported_at   TEXT NOT NULL,
  chunk_count   INTEGER NOT NULL DEFAULT 0,
  content_hash  TEXT NOT NULL DEFAULT ''
);

-- 知识库文本块（列语义对齐 ChunkRecord；vector 不入库，由 HNSW 层持有）
CREATE TABLE IF NOT EXISTS kb_chunks (
  id             TEXT PRIMARY KEY,
  doc_id         TEXT NOT NULL,
  file_name      TEXT NOT NULL,
  chapter_number INTEGER,
  chapter_title  TEXT,
  text           TEXT NOT NULL,
  chunk_index    INTEGER NOT NULL,
  total_chunks   INTEGER NOT NULL,
  imported_at    TEXT NOT NULL,
  corpus_kind    TEXT NOT NULL DEFAULT 'unknown',
  FOREIGN KEY (doc_id) REFERENCES kb_documents(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_kb_chunks_doc ON kb_chunks(doc_id);
CREATE INDEX IF NOT EXISTS idx_kb_chunks_scope ON kb_chunks(corpus_kind, chapter_number);

-- 嵌入空间注册表（对齐基线 embedding-spaces 的 active/building/inactive 代际模型）
CREATE TABLE IF NOT EXISTS kb_embedding_spaces (
  generation        INTEGER PRIMARY KEY,
  dimension         INTEGER NOT NULL,
  model_fingerprint TEXT NOT NULL DEFAULT '',
  distance_metric   TEXT NOT NULL DEFAULT 'cosine',
  status            TEXT NOT NULL DEFAULT 'building'
                    CHECK (status IN ('active','building','inactive')),
  created_at        TEXT NOT NULL DEFAULT (datetime('now'))
);

-- FTS5 预分词虚拟表（jieba 分词后以空格连接写入；查询同样先分词）
CREATE VIRTUAL TABLE IF NOT EXISTS kb_fts USING fts5(
  chunk_id   UNINDEXED,
  doc_id     UNINDEXED,
  file_name  UNINDEXED,
  corpus_kind UNINDEXED,
  tokens,
  tokenize = 'unicode61'
);
```

**要点与偏离说明**：

1. **`tokenize = 'unicode61'`（非 `porter`）**：token 已由 jieba 预切分并以空格连接，只需按空白切词；`porter` 是英文词干器，会改写 ASCII token 语义，故不用。方案 B 决策文档 §8.5 的 `porter` 系草案笔误，此处修正。
2. **向量不入 SQLite**：向量存 `db/vector.rs` 的 HNSW 图（内存 + 二进制快照），`kb_chunks.id` 即 HNSW 的 `doc_id`。
3. **嵌入空间代际模型保留**：基线的 `kb:get-vector-rebuild-status` / `kb:backfill-vectors` / 多代际 active 指针依赖此模型，故保留 `kb_embedding_spaces` 表，而非简化为单列 `has_vector`。
4. **软删除/外键**：`kb_chunks` 对 `kb_documents` 建 `ON DELETE CASCADE`；`kb_fts` 的清理在应用层同步（FTS5 虚拟表无外键）。

## 4. 向量快照目录（**L2 隔离红线**）

基线落在**共享**路径 `<project>/.vela/lancedb/`、`.vela/embedding-spaces.json`、`.vela/vectors.json`。
Tauri 侧**必须专属**，拟定为：

```
<project>/.lore/kb/
  spaces.json                      # 空间注册表的文件镜像（可选，DB 为准）
  index-<generation>.hnsw.graph    # Hnsw::file_dump
  index-<generation>.hnsw.data
  index-<generation>.idmap.json    # LocalVectorIndex 的 ID 映射 + 墓碑 sidecar
```

与 `<root>/.lore/lorekeeper.db` 命名风格一致；**绝不触碰** `.vela/lancedb/` / `.vela/vectors.json`。

> **L3 变更（2026-10-09 用户确认）**：Tauri 项目目录已由 `.vela/` 改名 **`.lore/`**（两栈项目目录刻意不互通）。本目录随之定为 `<project>/.lore/kb/`。

## 5. 计划交付（3 个增量，按序）

| 增量 | 内容 | 频道 | 预估 |
|---|---|---|---|
| **F2-1** | `src/db/kb/`：`chunks.rs`（分块移植 `chunkText`）+ `fts.rs`（jieba 预分词 + FTS5 CRUD）+ 单元测试 | — | ~600 行 |
| **F2-2** | `src/db/kb/hybrid.rs`：RRF 融合（FTS + HNSW）+ 空间注册表 + 回填 | — | ~500 行 |
| **F2-3** | `commands/kb.rs`（15 频道）+ `dialog:select-knowledge-*` 2 频道 + 前端登记 | 17 | ~700 行 |

**已知依赖缺口的处理**：

- `kb:import-reference-text` 依赖 `ImportRunRepository.resolveReferenceImportAuthority` / `commitReferenceImportReceipt`（属批次 G 的 import-run 域，**尚未迁移**）。
  → 本批次先实现该频道但不接权威校验，**返回显式占位失败**（沿用批次 E 的「诚实化占位」先例），并在快照登记为待批次 G 收口。
- `assertKnowledgeBaseStoragePathSupported` / `projectStoragePreflightFailure`（存储预检）尚未迁移 → F2-3 一并移植最小实现。
- `windows-safe-file-system` / `childFileCapability`（安全文件系统）尚未迁移 → `kb:import-document` / `kb:import-folder` 依赖它完成授权后读取；F2-3 需移植其 Rust 等价物或复用既有 `commands/external_file_grant.rs` 的能力模型（**待定，届时单列**）。

## 6. 验收标准

- ✅ 新增表由 `create_tables()` 幂等建立；`cargo test --lib` 全绿
- ✅ 中文召回率 ≥ 70%（jieba 预分词 + FTS5；用基线 `fts5-tokenizer-findings` 的 15 用例回归）
- ✅ `kb:*` 15 频道 + dialog 2 全部注册，`check:channels` orphan 为空
- ✅ `pnpm typecheck` / `lint` 零错误
- ✅ GUI：导入 → 搜索 → 统计 → 删除 全链路可走

## 7. 待批准项（Ask first）

1. **§3 的四条 DDL**是否批准写入 `db/schema.rs`？
2. **§4 的向量快照目录** `<project>/.lore/kb/` 是否批准？（原提案 `.vela/lorekeeper-kb/` 已被否决；随 L3 定为 `.lore/kb/`）
3. **§5 的增量切分**（F2-1 → F2-2 → F2-3）是否认可，或需调整优先级？
