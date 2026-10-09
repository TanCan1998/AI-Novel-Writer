# F2 批次最终决策：向量检索迁移方案

> **⚠️ 已过时（Superseded）**
> 本文件是 F2 决策的早期版本（FTS5 + jieba，纯关键词）。
> 最终决策见 [`2026-10-08-final-decision-plan-b.md`](../../2026-10-08-final-decision-plan-b.md)：
> 方案 B（FTS5 + jieba-rs + HNSW + RRF 自研混合检索）。
> 本文仅保留决策演进记录，不得作为实施依据。

**日期**：2026-10-08  
**决策人**：用户 + AI Agent  
**状态**：✅ 已决策，等待实施  

---

## 执行摘要

经过完整的技术评估、实测验证和网络调研，**批次 F2 的向量检索迁移方案确定为：FTS5 + jieba 分词器**。

**核心理由**：
1. ❌ **VecStore 不满足需求**：无 BM25 文本搜索，仅支持向量检索
2. ❌ **FTS5 unicode61 不可用**：中文召回率 0%（整句作为一个词元）
3. ✅ **jieba 分词成熟**：中文分词事实标准，Rust 绑定稳定
4. ✅ **零向量依赖**：延迟向量存储至阶段 1（可选）

---

## 决策过程回顾

### 阶段 1：基线行为对照（已完成）
- 分析 `electron/vector-store.ts`（2010 行）
- 识别核心特性：FTS 降级 / 连接池 / 多模型空间 / 混合检索
- **关键发现**：基线在无 embedding 时**自动降级 FTS**，证明 FTS 是必需路径

### 阶段 2：Rust 生态调研（已完成）
- 候选方案：VecStore / FTS5 only / FTS5 + hnsw_rs / tantivy + jieba
- 初步推荐：VecStore（误以为有 BM25）

### 阶段 3：网络搜索（已完成）
- 发现 VecStore（GitHub 16 stars，MIT）
- 误读特性列表，以为有混合检索（实际只有向量）

### 阶段 4：可行性评估（已完成）
- 确认 VecStore 依赖树可控（172 crates）
- 确认 Windows MSVC 兼容
- **发现真相**：README 无 `text_search` / `bm25` API

### 阶段 5：FTS5 实测（已完成）
- 测试 unicode61 tokenizer
- **结果**：15/15 用例召回率 0%
- **根因**：unicode61 将"林晚在筑基期"作为一个词元，"林晚"查询失败
- **验证**：通配符 `林*` 能匹配（前缀匹配）

### 阶段 6：最终决策（当前）
- VecStore 排除（无文本搜索）
- FTS5 unicode61 排除（中文不可用）
- **确定方案**：FTS5 + jieba 分词器

---

## 最终方案：FTS5 + jieba

### 技术栈
```toml
[dependencies]
rusqlite = { version = "0.40.2", features = ["bundled"] }  # 已有
jieba-rs = "0.7"  # 新增（唯一新依赖）
```

### 架构设计

#### 1. 分词层（`vector_store/tokenizer.rs`）
```rust
use jieba_rs::Jieba;

pub struct ChineseTokenizer {
    jieba: Jieba,
}

impl ChineseTokenizer {
    pub fn tokenize(&self, text: &str) -> Vec<String> {
        self.jieba.cut(text, false)  // 精确模式
            .into_iter()
            .filter(|t| t.len() > 1)  // 过滤单字（可选）
            .map(|s| s.to_string())
            .collect()
    }
}
```

#### 2. FTS5 虚拟表（改进版）
```sql
-- 改用 porter tokenizer（西文词干）+ 手动分词（中文）
CREATE VIRTUAL TABLE kb_fts USING fts5(
    doc_id,
    title_tokens,      -- 预分词后的标题
    content_tokens,    -- 预分词后的正文
    tokenize = 'porter'
);
```

**关键变化**：
- 不再依赖 FTS5 的 unicode61 自动分词
- 在写入前调用 `ChineseTokenizer::tokenize()`
- 将分词结果以空格连接后存入 `*_tokens` 列
- 查询时也先分词，转为 FTS5 查询语法

#### 3. 查询流程
```rust
pub fn search_fts(&self, query: &str, limit: i64) -> Result<Vec<SearchResult>> {
    // 1. 对查询分词
    let tokens = self.tokenizer.tokenize(query);
    
    // 2. 构造 FTS5 查询（OR 连接）
    let fts_query = tokens.join(" OR ");
    
    // 3. 执行查询
    let mut stmt = self.conn.prepare(
        "SELECT doc_id, rank FROM kb_fts WHERE kb_fts MATCH ? ORDER BY rank LIMIT ?"
    )?;
    
    // 4. 返回结果
    // ...
}
```

### 依赖分析

#### jieba-rs 传递依赖
```bash
$ cargo tree -p jieba-rs
jieba-rs v0.7.0
├── cedarwood v0.4.6
├── fnv v1.0.7
├── lazy_static v1.5.0
└── phf v0.11.2
    └── phf_shared v0.11.2
        └── siphasher v1.0.1
```

**总计**：6 个传递依赖，全部纯 Rust，无构建工具要求。

### 工作量估算

| 模块 | 文件 | 新增行数 | 说明 |
|---|---|---|---|
| 分词器 | `vector_store/tokenizer.rs` | ~120 | jieba 封装 + 停用词 |
| FTS5 封装 | `vector_store/fts.rs` | ~350 | 改用预分词模式 |
| 分块逻辑 | `vector_store/chunks.rs` | ~280 | 与基线对齐 |
| 搜索接口 | `vector_store/search.rs` | ~200 | 查询 + 排序 + 分页 |
| 统计接口 | `vector_store/stats.rs` | ~80 | count / 存储大小 |
| 命令注册 | `commands/kb.rs` | ~450 | 17 个频道映射 |
| 单元测试 | `vector_store/*.rs` | ~500 | 覆盖率 ≥ 80% |
| **总计** | | **~1980** | 对比基线 2010 行 |

**减少的复杂度**（相比基线）：
- ❌ 删除：LanceDB 连接池（~150 行）
- ❌ 删除：多 generation 向量空间（~200 行）
- ❌ 删除：旧数据迁移逻辑（~180 行）
- ❌ 删除：向量检索逻辑（~300 行）
- ✅ 新增：jieba 分词封装（~120 行）
- ✅ 简化：FTS5 预分词模式（~200 行）

**净变化**：~1980 行（简化 ~50 行）

### 性能预期

| 指标 | 预期值 | 依据 |
|---|---|---|
| 中文召回率 | **70–85%** | jieba 精确模式 + FTS5 BM25 排序 |
| 查询延迟 | **5–20ms** | FTS5 索引 + 无向量计算 |
| 内存占用 | **+5–10MB** | jieba 词典（~5MB）+ FTS5 索引 |
| 冷启动 | **无需 embedding** | 直接基于文本，立即可用 |

### 风险与缓解

| 风险 | 优先级 | 缓解措施 |
|---|---|---|
| jieba 分词质量不足 | 🟡 中 | 实测对比基线 LanceDB；支持自定义词典 |
| FTS5 索引膨胀 | 🟡 中 | 监控 `.vela/lorekeeper.db` 大小；`VACUUM` 优化 |
| 无法复现语义检索 | 🟢 低 | 文档标注「FTS + 分词」；阶段 1 可加向量 |
| 停用词误伤 | 🟢 低 | 默认不启用停用词；可配置 |

---

## 路径隔离（强制）

**必须改名**，避免与基线 `.vela/lancedb/` 冲突：

```rust
// tauri-app/src-tauri/src/vector_store/mod.rs

/// FTS5 索引直接存储在项目 DB 中（.vela/lorekeeper.db），
/// 不再需要独立的向量目录。
/// 
/// 若将来引入向量存储（阶段 1），使用：
/// .vela/lorekeeper-vectors/  ← 改名，避免与基线 .vela/lancedb/ 冲突
const VECTOR_DIR_NAME: &str = "lorekeeper-vectors";  // 预留
```

**影响**：
- ✅ 无需迁移基线 `.vela/lancedb/` 旧数据（双栈隔离要求）
- ✅ FTS5 索引在 SQLite 主库内，无额外目录
- ✅ 将来可选引入向量时，路径已预留

---

## 对比：不采用的方案

### ❌ VecStore
- **原因**：无 BM25 / 文本搜索 API，仅支持向量检索
- **优点**：依赖树可控、HNSW 优化、快照备份
- **致命缺陷**：基线在无 embedding 时**必须降级 FTS**，VecStore 不提供此路径

### ❌ FTS5 unicode61
- **原因**：中文召回率 0%（整句作为一个词元）
- **优点**：零依赖、最简实现
- **致命缺陷**：实测 15/15 用例全失败

### ❌ tantivy + cang-jie
- **原因**：依赖树 30+ crates，与「减内存」目标冲突
- **优点**：全文搜索引擎级特性（高亮、分面、聚合）
- **过度设计**：知识库检索不需要 Elasticsearch 级能力

### ❌ hnsw_rs + 自研 BM25
- **原因**：需自研 RRF 混合检索（+300 行），jieba 方案更成熟
- **优点**：向量 + BM25 混合
- **劣势**：工作量 +15%，无现成中文分词

---

## 阶段 1（可选）：引入向量存储

**触发条件**（以下任一）：
1. 用户明确要求语义检索
2. FTS + jieba 实测召回率 < 60%
3. 跨语言检索需求（同义词 / 翻译）

**方案**：
- 依赖：`hnsw_rs`（纯 Rust HNSW）
- 位置：`.vela/lorekeeper-vectors/`（已改名隔离）
- 混合检索：RRF 融合（向量 top-K + FTS top-K）
- 工作量：+500 行

**不在 F2 批次范围内**，延后至用户需求明确时。

---

## 实施计划

### 第 1 步：依赖准备（10 分钟）
```bash
cd tauri-app/src-tauri
# 编辑 Cargo.toml，添加 jieba-rs = "0.7"
cargo build --release
```

### 第 2 步：模块骨架（2 小时）
- 创建 `src/vector_store/{mod,tokenizer,fts,chunks,search,stats}.rs`
- 实现 `ChineseTokenizer::tokenize()`
- 创建 FTS5 虚拟表（预分词模式）

### 第 3 步：命令映射（4 小时）
- 创建 `src/commands/kb.rs`
- 映射 17 个 `kb:*` 频道
- 对齐基线参数签名

### 第 4 步：单元测试（3 小时）
- 覆盖分词 / 插入 / 查询 / 删除
- 中文测试用例（与 `fts5_benchmark.rs` 复用）
- 目标：`cargo test --lib` 80% 覆盖率

### 第 5 步：前端适配（1 小时）
- `tauri-app/src/shared/ipc-channels.ts` 参数映射
- 无需改 GUI（17 频道已在基线使用）

### 第 6 步：GUI 验收（1 小时）
- 导入知识库 → 搜索 → 查看结果 → 删除
- 对比基线 LanceDB 召回率

### 第 7 步：文档与提交（1 小时）
- 更新 `docs-fork/handoffs/2026-10-08-tauri-migration-status.md`
- 更新 `docs-fork/plans/tauri-migration-channel-inventory.md`
- Git 提交：`feat(tauri): 批次 F2 - FTS5 + jieba 知识库检索（17 频道）`

**总工期**：~12 小时（1.5 个工作日）

---

## 成功标准

| 指标 | 目标 | 验证方式 |
|---|---|---|
| 命令注册 | 17 个 `kb:*` 频道 | `check:channels` orphan 空 |
| 单元测试 | `cargo test --lib` 全绿 | CI 通过 |
| 类型检查 | `pnpm typecheck` 零错误 | 本地 + CI |
| 中文召回率 | ≥ 70% | 复用 `fts5_benchmark.rs` 用例 |
| GUI 功能 | 导入 / 搜索 / 删除正常 | 人工验收 |

---

## 决策签署

- ✅ **技术可行性**：jieba-rs 成熟，FTS5 已在 schema.rs
- ✅ **依赖可控**：+6 个传递依赖，全部纯 Rust
- ✅ **工期可控**：~1980 行，1.5 天
- ✅ **风险可控**：降级路径（FTS only）+ 升级路径（阶段 1 向量）

**批准人**：用户  
**执行人**：AI Agent  
**预计开工**：用户确认后立即  
**预计完成**：开工后 1.5 个工作日  

---

## 附录：实测数据

### A. FTS5 unicode61 实测（失败）
- **召回率**：0.0%（15/15 用例）
- **根因**：`"林晚在筑基期"` 作为一个词元，`"林晚"` 查询失败
- **通配符验证**：`"林*"` 能匹配（证明分词问题）

### B. VecStore 调研（排除）
- **发现**：README 误导，仅支持向量检索
- **API**：`upsert(id, vector, metadata)` / `query(vector, k)`
- **缺失**：`text_search()` / `bm25()` / `hybrid_search()`

### C. jieba-rs 调研（采用）
- **版本**：0.7.0（稳定）
- **依赖**：6 个纯 Rust crate
- **性能**：~1MB/s 分词速度（Rust 绑定）
- **词典**：~5MB（可自定义）

---

**文档版本**：v1.0  
**最后更新**：2026-10-08 20:30  
**下一步**：等待用户批准，开工实施
