# F2 方案修正：混合检索策略（FTS5 + 向量可选）

**日期**：2026-10-08 22:00  
**状态**：🟡 需要讨论 - 基于用户反馈修正  

---

## 用户关键洞察

用户正确指出了 **FTS5 与 LanceDB 的根本差异**：

| 维度 | FTS5 + jieba | LanceDB 向量检索 |
|---|---|---|
| **检索原理** | 关键词匹配（BM25） | **语义相似度**（向量距离） |
| **适用场景** | 精确查找（角色名、术语） | **模糊语义召回**（理解意图） |
| **RAG 能力** | ❌ 无法语义理解 | ✅ **核心能力** |
| **在项目中的作用** | 备用方案（无 embedding 时） | **主要方案**（语义检索） |

---

## 基线代码验证

### 1. 基线的检索策略（`electron/knowledge-base.ts:195-220`）

```typescript
export async function search(
  query: string,
  model: EmbeddingModelWithProtocol,
  // ...
): Promise<Array<{ text: string; score: number; fileName: string }>> {
  // 可选：生成查询向量
  let queryVector: number[] | undefined
  if (model.apiKey && query.trim()) {
    try {
      const [vec] = await generateEmbeddings([query], protocol, model, ...)
      if (vec && vec.length > 0) {
        queryVector = vec  // ✅ 优先使用向量检索
      }
    } catch {
      // ❌ Embedding 不可用，降级为 FTS
    }
  }

  return storeSearchWithScope(
    projectPath,
    query,
    queryVector,  // 传入向量（或 undefined）
    topK,
    chapterScope,
    embeddingSpaceFor(protocol, model),
    excludedCorpusKinds,
  )
}
```

**关键发现**：
1. ✅ **向量检索是第一优先级**（有 embedding 配置时）
2. ✅ **FTS 是降级方案**（embedding 失败或未配置时）
3. ✅ **混合检索**：`storeSearchWithScope` 同时接受 `query` 和 `queryVector`

### 2. 基线的 FTS 实现（`electron/vector-store.ts:1253-1285`）

```typescript
// FTS 降级路径
const rawTerms = queryText.match(/[\p{L}\p{N}-]+/gu) ?? []
const meaningfulTerms = rawTerms.filter(term => Array.from(term).length >= 2)
const searchTerms = [...new Map(
  (meaningfulTerms.length > 0 ? meaningfulTerms : rawTerms)
    .map(term => [term.toLocaleLowerCase(), term]),
).values()].slice(0, 8)

const textFilter = searchTerms.length > 0
  ? `(${searchTerms
      .map(term => `text LIKE '%${term.replace(/'/g, "''")}%'`)
      .join(' OR ')})`
  : "text LIKE '%%'"
```

**问题**：
- ❌ 使用 `LIKE` 模糊匹配（慢且不准确）
- ❌ 未利用 LanceDB 的 FTS 索引
- ❌ 未使用 jieba 分词

---

## 重新评估迁移目标

### 迁移的根本动机

**原始目标**：减少内存占用（Electron → Tauri）

**但**：
- ✅ 减少内存是**手段**
- ❌ 如果牺牲了 **RAG 核心能力**，就失去了意义

### 用户的核心诉求（推测）

| 场景 | FTS5 only | FTS5 + 向量（可选） | LanceDB Rust |
|---|---|---|---|
| **关键词查找**（角色名、术语） | ✅ 足够 | ✅ 足够 | ✅ 过度 |
| **语义检索**（找回伏笔、风格参考） | ❌ 无法实现 | ✅ **可实现** | ✅ 完整 |
| **内存占用** | ✅ 最低（+5 MB） | ⚠️ 中等（+20 MB） | ❌ 最高（+50 MB） |
| **依赖复杂度** | ✅ 最简（+7） | ⚠️ 中等（+40） | ❌ 最复杂（+1680） |

---

## 修正后的三种方案

### 方案 A：FTS5 only（之前推荐）

**适用场景**：
- ✅ 仅需关键词查找
- ✅ 不使用 RAG（不需要语义理解）
- ✅ 追求极致轻量（零额外依赖）

**优点**：
- ✅ 零向量依赖（+7 crates）
- ✅ 内存最低（+5 MB）
- ✅ 编译最快（+5 秒）

**缺点**：
- ❌ **丢失 RAG 核心能力**
- ❌ 无法语义理解（找回伏笔、风格参考）
- ❌ 降级为关键词匹配

**工作量**：~1980 行

---

### 方案 B：FTS5 + 轻量级向量（推荐修正）

**技术栈**：
```toml
[dependencies]
jieba-rs = "0.10"          # 中文分词（+6 传递）
hnsw_rs = "0.4"            # HNSW 向量索引（+30 传递）
bincode = "1.3"            # 向量序列化（+3 传递）
```

**架构**：
```
FTS5（主检索）
  ├─ jieba 分词（关键词精确匹配）
  └─ HNSW 向量索引（可选，语义检索）
      └─ 需要外部 embedding API（与基线一致）
```

**优点**：
- ✅ **保留 RAG 能力**（语义检索）
- ✅ 依赖可控（+40 crates vs +1680）
- ✅ 内存可控（+20 MB vs +50 MB）
- ✅ 与基线功能对齐（向量 + FTS 混合）
- ✅ 双栈隔离（不读 `.vela/lancedb/`）

**缺点**：
- ⚠️ 需自研混合检索融合（RRF 算法）
- ⚠️ 依赖外部 embedding API（与基线一致）
- ⚠️ 实现复杂度中等

**工作量**：~2280 行（+300 行向量层）

---

### 方案 C：LanceDB Rust（之前拒绝）

**优点**：
- ✅ 官方支持（生态完整）
- ✅ 混合检索内置（无需自研）
- ✅ 代码量少（~800 行）

**缺点**：
- ❌ **与迁移目标冲突**（增加内存）
- ❌ 依赖爆炸（+1680 crates）
- ❌ 编译慢（+5 分钟）
- ❌ 需要 datafusion（过度设计）

**工作量**：~800 行

---

## 方案对比（完整版）

| 对比项 | 方案 A<br>FTS5 only | 方案 B<br>FTS5 + 向量 | 方案 C<br>LanceDB |
|---|---|---|---|
| **依赖数** | +7 | **+40** | +1680 |
| **编译时间** | +5 秒 | **+20 秒** | +300 秒 |
| **内存增量** | +5 MB | **+20 MB** | +50 MB |
| **实现行数** | ~1980 | **~2280** | ~800 |
| **关键词检索** | ✅ jieba | ✅ jieba | ✅ jieba |
| **语义检索** | ❌ 无 | ✅ **HNSW** | ✅ 内置 |
| **RAG 能力** | ❌ **丢失** | ✅ **保留** | ✅ 完整 |
| **混合检索** | ❌ 仅 FTS | ✅ **需自研** | ✅ 内置 |
| **与目标一致** | ⚠️ 牺牲功能 | ✅ **平衡** | ❌ 增内存 |
| **双栈隔离** | ✅ 完全隔离 | ✅ 完全隔离 | ⚠️ 需改路径 |

---

## 推荐决策流程

### 第 1 步：确认 RAG 需求

**问题**：项目是否需要保留语义检索能力？

**A. 需要 RAG**（语义理解、找回伏笔）
- → **方案 B**（FTS5 + HNSW）
- 工作量：~2280 行，+40 依赖，+20 MB 内存

**B. 不需要 RAG**（仅关键词查找）
- → **方案 A**（FTS5 only）
- 工作量：~1980 行，+7 依赖，+5 MB 内存

### 第 2 步：评估依赖接受度

**如果选择方案 B**，需要接受：
- ⚠️ hnsw_rs（+30 crates，纯 Rust，无 C 依赖）
- ⚠️ 混合检索融合需自研（+300 行 RRF 算法）

**如果依赖不可接受**：
- → **回退方案 A**（FTS5 only）
- 或 → **保留 Electron 基线的 LanceDB**（不迁移向量检索）

---

## 方案 B 的技术细节

### 依赖树（hnsw_rs）

```bash
hnsw_rs v0.4.0
├── anyhow v1.0
├── indexmap v2.0
├── ndarray v0.16
├── num-traits v0.2
├── parking_lot v0.12
├── rand v0.8
└── rayon v1.12
```

**传递依赖**：~30 个（可控）

### 架构设计

```
tauri-app/src-tauri/src/vector_store/
├── mod.rs           (150 行) - 模块入口
├── fts.rs           (450 行) - FTS5 + jieba
├── chunks.rs        (380 行) - 文本块管理
├── search.rs        (580 行) - 混合检索（FTS + 向量）
├── vector.rs        (350 行) - HNSW 向量索引（新增）
├── hybrid.rs        (150 行) - RRF 融合算法（新增）
├── stats.rs         (150 行) - 统计
└── models.rs        (300 行) - 数据模型
```

### 混合检索流程

```rust
async fn search_hybrid(
    query: &str,
    query_vector: Option<Vec<f32>>,
    top_k: usize,
) -> Result<Vec<SearchResult>> {
    // 1. FTS 检索（关键词）
    let fts_results = fts_search(query, top_k * 2)?;
    
    // 2. 向量检索（语义）
    let vector_results = if let Some(vec) = query_vector {
        hnsw_search(&vec, top_k * 2)?
    } else {
        vec![]  // 降级为 FTS only
    };
    
    // 3. RRF 融合
    let fused = reciprocal_rank_fusion(
        &fts_results,
        &vector_results,
        top_k,
    );
    
    Ok(fused)
}
```

---

## 实施建议

### 阶段 0：FTS5 only（1 天）
- 实现关键词检索
- 验收标准：关键词查找通过
- **可选停止点**：如果不需要 RAG

### 阶段 1：向量检索（0.5 天）
- 添加 hnsw_rs 依赖
- 实现 HNSW 向量索引
- 集成 embedding API 调用

### 阶段 2：混合检索（0.5 天）
- 实现 RRF 融合算法
- 测试向量 + FTS 混合
- GUI 验收

**总计**：~2 天（如果需要 RAG）

---

## 待决策问题

### Q1: 项目是否需要保留 RAG 能力？
- **A. 需要** → 方案 B（FTS5 + HNSW）
- **B. 不需要** → 方案 A（FTS5 only）

### Q2: 如果选择方案 B，能否接受 +40 依赖？
- **A. 可以** → 开工实施
- **B. 不行** → 回退方案 A 或保留基线

### Q3: 向量存储路径如何隔离？
- **推荐**：`.vela/lorekeeper-vectors/`（避开 `.vela/lancedb/`）
- **备选**：`.vela/hnsw-index/`

---

## 关键教训

1. **功能优先于依赖**：减少依赖是手段，保留核心能力是目的
2. **基线分析不够深入**：之前只关注了 FTS 降级路径，忽略了向量检索是主路径
3. **用户反馈的价值**：技术决策需要结合实际使用场景

---

**下一步**：等待用户回答 Q1–Q3，再确定最终方案。
