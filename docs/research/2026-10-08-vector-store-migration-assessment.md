# 向量检索迁移专项评估（F2 批次开工前置）

> **生成时间**：2026-10-08  
> **评估目的**：为批次 F2（`kb:*` 15 频道 + `dialog:select-knowledge-*` 2 频道）制定技术方案，
> 明确 Electron 基线 `vector-store.ts` 行为差异、存储路径改名方案、改动量估算。  
> **用户决策前提**（已确认）：走 **SQLite FTS5 + Rust 侧向量存储**，
> **否决 `lancedb` Rust crate**（需 protoc/ninja/nasm + arrow 58 + datafusion 54，与「减内存」目标相背）。

---

## 一、基线 `electron/vector-store.ts` 行为对照

### 1.1 核心依赖
```typescript
// electron/vector-store.ts:7-11
import * as lancedb from '@lancedb/lancedb'
import { Field, FixedSizeList as ArrowFixedSizeList, Float32, Int32, Utf8, 
         Schema as ArrowSchema } from 'apache-arrow'
import fs from 'node:fs'
import path from 'node:path'
import { createHash, randomUUID } from 'node:crypto'
```

| 依赖 | 版本（根 package.json） | 用途 |
|---|---|---|
| `@lancedb/lancedb` | 0.22.3 | 向量数据库（Node 原生模块，含平台特定二进制） |
| `apache-arrow` | 18.2.2 | Arrow schema 定义（`FixedSizeList<Float32>` 向量列） |
| Node 标准库 | — | 文件系统、路径、哈希 |

### 1.2 导出函数（公开 API，共 20 个）
```
- getConnection()              // 连接池管理
- closeConnection()
- getEmbeddingSpaces()         // 多模型空间管理
- getCanonicalChunksForEmbeddingRebuild()
- planEmbeddingRebuild()       // 重建计划
- activatePlannedEmbeddingSpace()
- rebuildPlannedEmbeddingSpace()
- addChunks()                  // 文本块增删
- removeDocument()
- clearAll()
- search()                     // 检索入口（混合检索）
- searchWithScope()
- listDocuments()
- hashCanonicalChunkSet()      // 完整性校验
- getDocumentIntegrity()
- getStats()
- getChunksWithoutVectors()    // 向量回填
- getChunksForBackfill()
- updateChunkVectors()
- migrateFromJSON()            // 旧数据迁移
```

### 1.3 存储路径配置（**隔离风险源头**）
```typescript
// electron/vector-store.ts:123-131
function databasePath(projectPath: string): string {
  return path.join(projectPath, '.vela', 'lancedb')  // ⚠️ 共享目录
}

function registryPath(projectPath: string): string {
  return path.join(projectPath, '.vela', EMBEDDING_REGISTRY_FILE)
  // EMBEDDING_REGISTRY_FILE = 'embedding-spaces.json'
}

function legacyMigrationJournalPath(projectPath: string): string {
  return path.join(projectPath, '.vela', LEGACY_MIGRATION_JOURNAL_FILE)
  // LEGACY_MIGRATION_JOURNAL_FILE = 'vectors.json.migration-journal.json'
}
```

**表名常量**：
- `TABLE_NAME = 'chunks'`（文本块事实源，**始终可用**）
- `DOCS_TABLE_NAME = 'documents'`（文档聚合元信息）
- 每个 embedding space 有**独立物理表**（避免不同维度向量混写同一 Arrow `FixedSizeList`）

### 1.4 关键特性总结
| 特性 | 基线实现 | Tauri 替代方案 |
|---|---|---|
| **连接池** | ✓ Map<dbPath, Connection> | Rust `Arc<Mutex<Connection>>` 或单例 |
| **FTS 降级** | ✓ `table.createIndex('text', {config: lancedb.Index.fts()})` | SQLite FTS5 虚拟表（`rusqlite` bundled） |
| **多模型空间** | ✓ 独立表 + `embedding-spaces.json` 注册表 | **降级**：单空间 + 可选多模型支持（见 §2.3） |
| **旧数据迁移** | ✓ `migrateFromJSON()` 从 `vectors.json` 迁移 | **不迁移**：Tauri 侧为空启动（双栈隔离前提） |
| **混合检索** | ✓ `queryVector` 存在时走向量 ANN + FTS 回退 | **分阶段**：先 FTS only，后期实验混合 |

---

## 二、FTS vs 向量检索差异清单

### 2.1 检索语义差异
| 维度 | LanceDB 向量检索 | SQLite FTS5 | 影响 |
|---|---|---|---|
| 匹配原理 | 语义相似（L2 / cosine 距离） | 词元匹配（Porter stemming + trigram） | FTS 无法捕捉同义词、长尾问答 |
| 查询输入 | 需先调 embedding API 生成 `queryVector` | 直接传文本 | FTS **无需额外 HTTP 请求**，快 |
| 冷启动 | 需预先为所有 chunk 生成 vector | 仅需文本 | FTS 可立即检索，无回填等待 |
| 跨语言 | 天然支持（embedding 模型能力） | 依赖分词器（中文需 `simple` tokenizer） | FTS 中英混合效果差 |
| Top-K 精度 | 高（L2 距离排序） | 中（BM25 相关性） | 技术文档检索 FTS 够用，小说长文本可能需向量 |

### 2.2 混合检索策略对比
**基线（`electron/vector-store.ts:1547`）**：
```typescript
export async function search(
  projectPath: string,
  queryText: string,
  queryVector?: number[],  // ← 可选向量
  topK: number = 5,
  embeddingSpace?: EmbeddingSpaceIdentity,
  excludedCorpusKinds: readonly KnowledgeCorpusKind[] = [],
): Promise<SearchResult[]>
```
- `queryVector` **存在** → 向量检索（active generation 表）
- `queryVector` **缺失** → 安全降级至 FTS（`chunks` 表）
- 降级发生场景：embedding 模型未配置、API 调用失败、向量空间未激活

**Tauri 侧分阶段策略**：
1. **阶段 0（F2 最小化）**：**FTS only**，`queryVector` 参数接收但忽略；
2. **阶段 1（可选，F2+）**：Rust 侧向量存储 + 自研混合检索（权重可调）；
3. **阶段 2（实验）**：对接 `qdrant`/`milvus` 等外部向量库（需额外服务）。

### 2.3 多模型空间降级方案
**基线复杂度来源**：
- 每个 `(modelFingerprint, distanceMetric)` 组合一个 generation；
- 切换模型时需**重建所有向量**（2010 行封装的核心逻辑）；
- `embedding-spaces.json` 管理 active/inactive 状态。

**Tauri 侧降级（用户决策待定，默认单空间）**：
- **单一向量表**（一个 embedding 模型），维度固定；
- 切换模型 = **清空重建**（无多 generation 并存）；
- 省去 `embedding-spaces.json`，模型标识存入 `config.json`（现有配置层）。

**代价**：无法保留多模型历史向量（基线可保留 inactive generation）；
**收益**：实现简化 ~70%，与「减内存」目标一致。

---

## 三、向量存储路径改名方案（**隔离必需**）

### 3.1 问题根因
基线所有向量文件在 `<project>/.vela/` **共享根目录**下：
```
<project>/.vela/
├── vela.db                                  # Electron 基线 SQLite
├── lancedb/                                 # ⚠️ LanceDB 物理目录
│   ├── chunks.lance/
│   ├── documents.lance/
│   └── generation_<N>.lance/
├── embedding-spaces.json                    # ⚠️ 注册表
├── vectors.json.migration-journal.json      # ⚠️ 旧数据迁移日志
└── vectors.json                             # ⚠️ 遗留文件（已迁移后删除）
```

Tauri 侧若沿用 `lancedb/` 目录名 → 双栈**互相覆盖向量数据** → **违反隔离红线**。

### 3.2 改名方案（与 `lorekeeper.db` 命名风格一致）
| 基线路径 | Tauri 侧路径 | 常量名 |
|---|---|---|
| `.vela/lancedb/` | `.vela/lorekeeper-vectors/` | `LOREKEEPER_VECTOR_DIR` |
| `.vela/embedding-spaces.json` | `.vela/lorekeeper-embedding-spaces.json` | `LOREKEEPER_EMBEDDING_REGISTRY` |
| `.vela/vectors.json.migration-journal.json` | **删除**（不迁移旧数据） | — |
| `.vela/vectors.json` | **删除**（同上） | — |

**新目录结构**（Tauri 侧，F2 后）：
```
<project>/.vela/
├── lorekeeper.db                            # L2 隔离（已落地）
├── lorekeeper-vectors/                      # ← 本方案新增
│   ├── chunks.db                            # SQLite FTS5 虚拟表
│   └── vectors.bin                          # Rust 序列化向量（可选，阶段 1）
└── lorekeeper-embedding-spaces.json         # ← 若采纳多空间（可选）
```

**代价评估**：
- 路径字符串 **6 处修改**（`databasePath` / `registryPath` 等）；
- **无法读取基线已有向量**（但这是隔离前提，符合预期）；
- 用户若需保留旧向量 → 需手动**导出/重新导入**（文档级操作，非迁移自动化）。

---

## 四、改动量估算

### 4.1 模块拆解（按基线 2010 行功能占比）
| 模块 | 基线行数 | Tauri 侧复杂度 | 估算行数 | 说明 |
|---|---|---|---|---|
| 类型定义 | ~150 | 简化（单空间） | ~80 | `ChunkRecord` / `SearchResult` / `KBStats` |
| 连接池 / 路径配置 | ~50 | 等价（换 Rust API） | ~60 | `get_connection()` / `close_connection()` |
| Embedding space 管理 | **~600** | **降级或删除** | ~100 | 多 generation 逻辑 → 单表 |
| 文本块增删 | ~200 | 等价 | ~250 | `add_chunks()` / `remove_document()` / `clear_all()` |
| **FTS 索引** | ~80 | **SQLite FTS5** | ~150 | `CREATE VIRTUAL TABLE ... USING fts5` |
| **向量检索** | **~400** | **阶段 0 删除** | ~50 | 先 FTS only，后期可选实验 |
| 混合检索入口 | ~150 | 简化（无降级逻辑） | ~100 | `search()` / `searchWithScope()` |
| 文档列表 / 统计 | ~100 | 等价 | ~120 | `list_documents()` / `get_stats()` |
| 旧数据迁移 | **~280** | **删除** | 0 | `migrateFromJSON()`（不迁移） |

**总计（阶段 0 最小化）**：~910 行（削减 ~55%）  
**阶段 1（可选，加入 Rust 向量存储）**：+300–500 行（序列化 / ANN 检索）

### 4.2 新增文件清单
| 文件 | 内容 | 估算行数 |
|---|---|---|
| `src/vector_store/mod.rs` | 模块入口（pub use） | ~30 |
| `src/vector_store/fts.rs` | SQLite FTS5 封装（建表 / 索引） | ~200 |
| `src/vector_store/chunks.rs` | Chunk CRUD（对应基线 `addChunks` 等） | ~300 |
| `src/vector_store/search.rs` | 检索入口（FTS query + 排序） | ~250 |
| `src/vector_store/stats.rs` | 统计 / 列表（`list_documents` / `get_stats`） | ~150 |
| `src/commands/kb.rs` | 15 个 `#[command]` 路由 | ~400 |
| `src/repositories/kb_repository.rs`（可选） | 若抽离仓储层 | ~300 |

**合计（不含可选仓储层）**：~1330 行  
**含仓储层**：~1630 行

### 4.3 改动现有文件
| 文件 | 改动内容 | 行数变化 |
|---|---|---|
| `src/main.rs` | 注册 15 个 `kb:*` 命令 | +15 |
| `src/state.rs` | 添加 `vector_store_path` 字段（可选） | +5 |
| `src/db/mod.rs` | `CREATE TABLE` DDL（若 FTS 走独立 DB 则不需要） | 0 |
| `capabilities/default.json` | 追加权限（`fs:read` / `dialog:open`） | +8 |
| `tauri-app/src/services/ipc-client.ts` | `CHANNEL_ARG_NAMES` 登记 17 频道 | +17 |
| `tauri-app/src/shared/ipc-channels.ts` | 已有类型定义（无需改） | 0 |

### 4.4 依赖新增（候选）
| Crate | 版本 | 用途 | 必需性 |
|---|---|---|---|
| `rusqlite` | 已有 0.40.2 | FTS5 虚拟表 | ✅ 已满足（bundled feature 含 FTS5） |
| `serde_json` | 已有 1.0 | JSON 序列化 | ✅ 已满足 |
| `bincode`（可选） | 1.3 | 向量二进制序列化（阶段 1） | ⚠️ 新增 |
| `hnsw_rs`（可选） | 0.3 | 纯 Rust ANN 检索（阶段 1） | ⚠️ 新增 |

**阶段 0 结论**：**零新增依赖**（`rusqlite` bundled 已含 FTS5）。

---

## 五、FTS5 技术验证（rustc 1.99.0 + rusqlite 0.40.2）

### 5.1 FTS5 编译检查
```bash
# 在 tauri-app/src-tauri/ 下执行
cargo build --features rusqlite/bundled
# ✅ 通过（bundled feature 已启用）
```

### 5.2 最小可行示例（待实测）
```rust
use rusqlite::{Connection, Result};

fn create_fts_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts 
         USING fts5(id, text, content='');
         
         CREATE TABLE IF NOT EXISTS chunks (
             id TEXT PRIMARY KEY,
             doc_id TEXT NOT NULL,
             text TEXT NOT NULL,
             chunk_index INTEGER NOT NULL
         );
         
         CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
             INSERT INTO chunks_fts(id, text) VALUES (new.id, new.text);
         END;
         
         CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
             DELETE FROM chunks_fts WHERE id = old.id;
         END;"
    )?;
    Ok(())
}

fn search_fts(conn: &Connection, query: &str, limit: usize) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT c.text, rank 
         FROM chunks_fts 
         JOIN chunks c ON chunks_fts.id = c.id
         WHERE chunks_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2"
    )?;
    
    let rows = stmt.query_map([query, &limit.to_string()], |row| {
        row.get::<_, String>(0)
    })?;
    
    rows.collect()
}
```

**验证要点**：
1. FTS5 虚拟表 + 触发器同步（`content=''` 外部内容模式）；
2. `MATCH` 语法（支持 `AND` / `OR` / `NOT` / 短语 `"..."`）；
3. `rank` 排序（BM25 相关性）；
4. 中文分词（默认 `unicode61`，需测试效果；可考虑 `simple` tokenizer）。

---

## 六、迁移路径建议（分 3 阶段）

### 阶段 0：FTS only 最小化（F2 批次目标）
**范围**：15 个 `kb:*` 频道 + 2 个 `dialog:select-knowledge-*`  
**技术栈**：`rusqlite` FTS5 + `tauri-plugin-dialog` + `tauri-plugin-fs`  
**不包含**：向量生成、混合检索、多模型空间  
**交付物**：
- ✅ `kb:import-*` 可导入文本并切分 chunk（无向量）
- ✅ `kb:search` 走 FTS5 `MATCH` 查询
- ✅ `kb:stats` 返回 `vectorDimension: 0`（标识无向量）
- ✅ `kb:get-vector-rebuild-status` 返回 `embeddingConfigured: false`
- ⚠️ `kb:backfill-vectors` / `kb:get-vectorless-count` **空实现**（返回 0）

**工作量**：~1330 行新代码 + ~45 行改动 + 零新依赖  
**验收标准**：
1. `cargo test --lib` 新增 ~80 个测试（覆盖 FTS CRUD / 检索 / 统计）；
2. `check:channels` 未迁移清单 76 → **59**（`kb=15` + `dialog=2` 完成）；
3. GUI 冒烟：导入文档 → 搜索关键词 → 显示结果（无向量标识）。

### 阶段 1：Rust 向量存储 + 混合检索（可选，F2+）
**新增依赖**：`bincode` / `hnsw_rs`（或 `qdrant-client`）  
**新增文件**：`src/vector_store/vectors.rs`（序列化 / ANN）  
**改动行数**：+300–500 行  
**交付物**：
- `kb:backfill-vectors` 调用 embedding API 并存储向量
- `kb:search` 支持 `queryVector` 参数（混合检索）
- `kb:get-vector-rebuild-status` 返回 `embeddingConfigured: true`

### 阶段 2：外部向量库对接（实验）
**候选方案**：
- `qdrant`（Rust 实现，HTTP API，需独立服务）
- `milvus`（Go 实现，gRPC，企业级）
- `pgvector`（PostgreSQL 扩展，需切换主数据库）

**决策点**：是否值得引入**额外服务依赖**？（与「本地优先」理念冲突）

---

## 七、风险清单与缓解措施

| 风险 | 影响 | 缓解措施 | 优先级 |
|---|---|---|---|
| FTS5 中文分词效果差 | 搜索召回率低 | ① 实测对比（基线 vs FTS5）；② 可配置 tokenizer（`simple` / `porter`） | 🔴 高 |
| 无向量时无法复现基线语义检索 | 功能降级 | ① 文档明确标注「FTS only」；② 阶段 1 补齐 | 🟡 中 |
| 向量序列化格式不兼容基线 | 无法导入旧数据 | ① 隔离前提下**不需要兼容**；② 若需迁移，提供导出/重新导入脚本 | 🟢 低 |
| `kb:backfill-vectors` 空实现被 GUI 调用 | 前端显示「正在回填」但无效果 | ① 返回 `{success: true, processed: 0}`；② GUI 判断 `embeddingConfigured` 隐藏入口 | 🟡 中 |
| LanceDB 路径隔离不彻底 | 双栈互相覆盖 | ① **强制**改名 `lorekeeper-vectors/`；② 代码审查禁止 `.vela/lancedb` 字符串 | 🔴 高 |

---

## 八、后续行动（优先级排序）

### 立即执行（F2 开工前）
1. ✅ **本评估文档交付**（当前文件）
2. ⚠️ **FTS5 中文分词实测**（建 demo 项目，对比基线 LanceDB 召回率）
3. ⚠️ **路径改名 PR**（修改 6 处常量，锁定隔离边界）

### F2 批次内
4. 实现阶段 0（FTS only，~1330 行）
5. 前端适配（17 频道参数映射 + `CHANNEL_ARG_NAMES` 登记）
6. 测试覆盖（~80 个新用例）
7. GUI 验收（导入 → 搜索 → 列表 → 删除 → 清空）

### F2 之后（可选）
8. 阶段 1 实验（Rust 向量存储 + 混合检索）
9. 性能基准测试（FTS vs 向量 vs 混合，不同数据集）
10. 外部向量库调研（qdrant / milvus 可行性）

---

## 九、决策点总结（需用户确认）

| 编号 | 问题 | 推荐方案 | 风险 |
|---|---|---|---|
| D1 | 是否采纳「FTS only」阶段 0？ | ✅ 是（零新依赖，快速交付） | 无向量时功能降级 |
| D2 | 是否删除多模型空间逻辑？ | ✅ 是（简化 ~70%） | 无法保留多 generation 历史 |
| D3 | 是否强制改名向量目录？ | ✅ 是（`lorekeeper-vectors/`） | 无法读取基线旧数据 |
| D4 | 是否需要阶段 1（Rust 向量）？ | ⚠️ 待定（先验证 FTS5 效果） | 新增依赖 / +500 行 |
| D5 | FTS5 中文分词器选择？ | ⚠️ 待实测（默认 `unicode61` vs `simple`） | 影响召回率 |

**本评估完成后，等待用户答复 D1–D5，再正式开工 F2 批次实现。**

---

## 附录 A：基线关键代码片段

### A.1 混合检索入口
```typescript
// electron/vector-store.ts:1547
export async function search(
  projectPath: string,
  queryText: string,
  queryVector?: number[],
  topK: number = 5,
  embeddingSpace?: EmbeddingSpaceIdentity,
  excludedCorpusKinds: readonly KnowledgeCorpusKind[] = [],
): Promise<SearchResult[]> {
  return searchWithScope(
    projectPath,
    queryText,
    queryVector,
    undefined, // fromChapter
    undefined, // toChapter
    topK,
    embeddingSpace,
    excludedCorpusKinds,
  )
}
```

### A.2 FTS 降级逻辑
```typescript
// electron/vector-store.ts:1577（简化）
const registry = readRegistry(projectPath)
const activeSpace = registry.spaces.find(
  s => s.generation === registry.activeGeneration
)

if (queryVector && activeSpace && matches(activeSpace, embeddingSpace)) {
  // 向量检索路径
  const table = await db.openTable(activeSpace.tableName)
  return await table
    .search(queryVector)
    .distanceType(activeSpace.distanceMetric)
    .limit(topK)
    .execute()
} else {
  // FTS 降级路径
  const chunks = await db.openTable(TABLE_NAME)
  return await chunks
    .search(queryText)
    .limit(topK)
    .execute()
}
```

### A.3 Embedding space 注册表结构
```typescript
// electron/vector-store.ts:60-73
export interface EmbeddingSpaceRegistry {
  version: 1
  activeGeneration: number | null
  spaces: EmbeddingSpace[]
}

export interface EmbeddingSpace {
  generation: number        // 递增 ID
  tableName: string         // 'generation_1'
  modelFingerprint: string  // 'openai:text-embedding-3-small'
  vectorDimension: number   // 1536
  distanceMetric: string    // 'l2' | 'cosine'
  status: 'active' | 'building' | 'inactive'
  createdAt: string         // ISO 8601
}
```

---

**评估完成时间**：2026-10-08  
**下一步**：等待用户答复决策点 D1–D5，输出 F2 批次实施计划。
