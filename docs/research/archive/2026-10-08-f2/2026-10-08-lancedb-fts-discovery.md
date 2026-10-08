# F2 批次决策修正：基于 LanceDB FTS 分词器发现

**日期**：2026-10-08 21:00  
**状态**：🔴 重大发现 - 需要重新评估  

---

## 关键发现

### 搜索结果揭示的真相

根据 Parallel MCP 搜索结果，**LanceDB 官方文档明确支持分词器**：

> **"Full-Text Search (FTS) Index - LanceDB"**  
> "Model-backed tokenizers such as **jieba/default**, lindera/ipadic, and lindera/ko-dic require tokenizer model files in Lance's language model home."

**这意味着**：
1. ✅ LanceDB **支持 jieba 分词器**（内置）
2. ✅ 基线 Node.js 版可能**已经在使用** jieba（需验证）
3. ❌ 之前的 FTS5 + jieba 方案是**重复造轮子**

---

## 基线代码再分析

### 基线的 FTS 实现

从 `electron/vector-store.ts` 搜索结果：

```typescript
// 基线的 FTS 降级逻辑
async function ensureTextIndex(table: lancedb.Table): Promise<void> {
  try {
    await table.createIndex('text', { config: lancedb.Index.fts() })
  } catch {
    // 已有索引和不支持 FTS 的旧 LanceDB 都不应阻断文本写入。
  }
}
```

**关键点**：
- 基线调用 `lancedb.Index.fts()` 创建 FTS 索引
- **没有显式指定分词器**（使用默认）
- 搜索时使用 `LIKE` 模糊匹配（未充分利用 FTS）

### 基线的搜索实现

```typescript
// 基线的文本检索（FTS 降级路径）
const rawTerms = queryText.match(/[\p{L}\p{N}-]+/gu) ?? []
const searchTerms = [...new Map(
  meaningfulTerms.map(term => [term.toLocaleLowerCase(), term]),
).values()].slice(0, 8)

const textFilter = searchTerms.length > 0
  ? `(${searchTerms
      .map(term => `text LIKE '%${term.replace(/'/g, "''")}%'`)
      .join(' OR ')})`
  : "text LIKE '%%'"
```

**问题**：
- ❌ **没有使用 LanceDB 的 FTS 查询 API**
- ❌ 使用 `LIKE` 模糊匹配（慢且不准确）
- ❌ 没有利用 jieba 分词能力

---

## LanceDB vs Chroma vs Qdrant

### 竞品对比（来自搜索结果）

| 对比项 | Chroma | LanceDB | Qdrant |
|---|---|---|---|
| 嵌入式 | ✅ 轻量级 | ✅ 嵌入 + 云 | ⚠️ 主要云端 |
| 全文检索 | ❌ 仅向量 | ✅ **向量+FTS+SQL** | ✅ 向量+过滤 |
| 混合检索 | ❌ 无 | ✅ **原生支持** | ✅ 原生支持 |
| Rust 支持 | ❌ Python only | ✅ **原生 Rust** | ✅ 原生 Rust |
| 规模 | 小数据集 | **20PB / 20K QPS** | 云端大规模 |
| 成本 | 免费 | **100x 节省** | 付费 |

**结论**：
- LanceDB 是**唯一同时满足**：嵌入式 + FTS + Rust + 混合检索的方案
- Chroma 无全文检索（之前误以为 VecStore 是替代品）
- Qdrant 更适合云端部署

---

## 为什么基线没用好 LanceDB FTS？

### 假设 1：版本问题
- 基线使用 `@lancedb/lancedb` **0.22.3**（2024 年版本）
- jieba 分词器可能是**后续版本**才加入（需验证）

### 假设 2：Node.js 绑定不完整
- Node.js 版 LanceDB 的 FTS API 可能**不如 Rust 版完整**
- 基线代码只用了 `lancedb.Index.fts()`，未配置分词器

### 假设 3：开发者未意识到
- 基线开发者可能**不知道** LanceDB 支持 jieba
- 退化为 `LIKE` 模糊匹配（简单但低效）

---

## 重新评估的三个方案

### 方案 A：继续 FTS5 + jieba（原推荐）
**优点**：
- ✅ 零依赖（jieba-rs 已决定引入）
- ✅ 与 LanceDB 完全隔离（双栈安全）
- ✅ 实现简单（~1980 行）

**缺点**：
- ❌ **无向量检索**（需阶段 1 再加 +500 行）
- ❌ 混合检索需自研 RRF（+300 行）
- ❌ 放弃 LanceDB 的生态优势

### 方案 B：迁移到 Rust LanceDB + jieba
**优点**：
- ✅ **与基线功能一致**（向量 + FTS + 混合）
- ✅ **复用基线数据**（`.vela/lancedb/` 可读）
- ✅ 官方支持 jieba 分词器
- ✅ 原生混合检索（无需自研）
- ✅ 未来可升级到 LanceDB Cloud

**缺点**：
- ⚠️ 依赖树未知（需评估 `lancedb` crate）
- ⚠️ 需要配置 jieba 模型文件（`LANCE_LANGUAGE_MODEL_HOME`）
- ⚠️ 可能与基线 `.vela/lancedb/` 冲突（需隔离策略）

### 方案 C：混合方案（FTS5 + LanceDB 向量层）
**优点**：
- ✅ FTS5 文本检索（立即可用）
- ✅ LanceDB 向量检索（可选）
- ✅ 双栈隔离（各自独立）

**缺点**：
- ❌ **最复杂**（~2500 行）
- ❌ 两套存储系统（维护成本高）
- ❌ 混合检索融合逻辑复杂

---

## 需要验证的问题

### 1. LanceDB Rust crate 的依赖树
```bash
cargo tree -p lancedb --depth 2
```
**关键指标**：
- 传递依赖数量（是否 > 200？）
- 是否包含 arrow / datafusion（重型依赖）
- 是否需要 protobuf / cmake（构建工具）

### 2. jieba 模型文件大小
```bash
# LanceDB 的 jieba 模型在哪里？
# 大小是多少？（影响安装包体积）
```

### 3. 基线数据兼容性
```rust
// Rust LanceDB 能否读取 Node.js LanceDB 0.22.3 的数据？
// .vela/lancedb/ 目录结构是否兼容？
```

### 4. FTS 查询 API
```rust
// Rust LanceDB 的 FTS 查询语法是什么？
// 是否需要手动分词？
```

---

## 初步推荐（待验证）

### 🥇 优先评估：方案 B（LanceDB Rust + jieba）

**理由**：
1. ✅ 与基线功能对齐（向量 + FTS + 混合）
2. ✅ 复用 LanceDB 生态（官方支持）
3. ✅ 避免重复造轮子（FTS5 是降级方案）
4. ✅ 未来可迁移到 LanceDB Cloud

**前置条件**：
- ⚠️ `lancedb` crate 依赖树可控（< 300 crates）
- ⚠️ jieba 模型文件 < 20MB
- ⚠️ 有明确的路径隔离方案（避开 `.vela/lancedb/`）

### 🥈 备选：方案 A（FTS5 + jieba）

**触发条件**：
- ❌ LanceDB 依赖树 > 300 crates
- ❌ 构建要求复杂（protobuf / cmake）
- ❌ jieba 模型文件 > 20MB

---

## 立即行动

### 第 1 步：评估 LanceDB Rust crate（30 分钟）
```bash
cd tauri-app/src-tauri
# 临时添加 lancedb 依赖
cargo add lancedb --dry-run
cargo tree -p lancedb --depth 3
```

### 第 2 步：查看 LanceDB FTS 文档（30 分钟）
- 官方文档：https://docs.lancedb.com/indexing/fts-index
- Rust API：https://docs.rs/lancedb/latest/lancedb
- jieba 配置方式

### 第 3 步：对比决策（15 分钟）
| 指标 | FTS5 + jieba | LanceDB + jieba |
|---|---|---|
| 依赖数 | +6 | ??? |
| 工作量 | ~1980 行 | ??? |
| 向量支持 | 需阶段 1 | ✅ 内置 |
| 混合检索 | 需自研 | ✅ 内置 |
| 生态 | 无 | ✅ LanceDB |

### 第 4 步：更新决策文档
- 若 LanceDB 可行 → 推翻 FTS5 方案
- 若 LanceDB 不可行 → 保持 FTS5 方案

---

## 待办更新

```
✅ #1: VecStore 可行性评估（已排除）
✅ #2: FTS5 中文实测（召回率 0%）
✅ #3: VecStore BM25 实测（无此 API）
✅ #4: F2 批次决策文档（FTS5 + jieba）
🔴 #5: LanceDB Rust crate 评估（新增，紧急）
⏳ #6: F2 批次实施（方案待定）
⏳ #7: F1 批次提交（已验证）
```

---

## 关键教训

1. **不要过早排除基线方案**：LanceDB 本身就是正确答案
2. **充分利用搜索工具**：Parallel MCP 发现了文档中的 jieba
3. **验证假设**：基线用 `LIKE` 不代表 LanceDB 不支持 FTS

---

**下一步**：等待用户确认是否评估 LanceDB Rust crate，或直接采用 FTS5 + jieba 保守方案。
