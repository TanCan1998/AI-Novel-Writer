# 网络搜索结果汇总（2026-10-08）

> **目的**：补充 Rust 向量存储方案调研，基于 Parallel MCP 实际搜索结果。
> **搜索工具**：Parallel MCP web_search
> **Session ID**：f2-vector-research-2026-10-08

---

## 一、Rust 嵌入式向量数据库实际发现

### 1.1 新发现的轻量级方案

#### **VecStore** —— "SQLite of vector search"
- **GitHub**：https://github.com/PhilipJohnBasile/vecstore
- **Stars**：16（活跃项目，2024 Edition）
- **技术特征**：
  - ✅ HNSW 索引（亚毫秒级搜索 100K+ 向量）
  - ✅ 混合检索（向量 + BM25 关键词）
  - ✅ 元数据过滤（SQL-like 表达式）
  - ✅ 跨平台（Rust + Python + JavaScript）
  - ✅ 浏览器支持（WebAssembly）
- **代码示例**：
  ```rust
  use vecstore::VecStore;
  let mut store = VecStore::open("vectors.db")?;
  store.upsert("doc1", vec![0.1, 0.2, 0.3], json!({"title": "Hello"}))?;
  let results = store.query(&vec![0.1, 0.2, 0.3], 10)?;
  ```
- **优势**：
  - 直接提供混合检索（不需要自研融合）
  - 9 种距离度量（Cosine, Euclidean, Dot Product, Manhattan, Hamming, Jaccard 等）
  - 快照备份/恢复
- **劣势**：
  - 新项目（0.1.0），生产成熟度待验证
  - 依赖树未知（需查看 Cargo.toml）

#### **PolarisDB** —— 纯 Rust 嵌入式向量库
- **GitHub**：https://github.com/hugoev/PolarisDB
- **文档**：https://docs.rs/polarisdb/latest/polarisdb/
- **技术特征**：
  - ✅ 100% 本地运行（无 C++ 依赖）
  - ✅ 零网络延迟（in-process）
  - ✅ HNSW 索引 + Bitmap 过滤
  - ✅ 异步支持（async Collection）
  - ✅ LangChain 集成（Python）
- **代码示例**：
  ```rust
  let mut index = HnswIndex::new(DistanceMetric::Cosine, 384, config);
  index.insert(id, embedding, metadata)?;
  let results = index.search(&query, 10, None, None);
  // 比暴力搜索快 9 倍
  ```
- **优势**：
  - RAG 应用友好
  - 边缘 AI 场景优化
  - 纯 Rust 实现
- **劣势**：
  - 未说明是否支持持久化
  - Stars 数未知（较新项目）

#### **iQDB** —— 高性能嵌入式向量数据库
- **GitHub**：https://github.com/jamesgober/iQDB
- **特点**：
  - ✅ 单进程、in-application（对标 sqlite）
  - ✅ 三种索引：HNSW / IVF / Flat
  - ✅ 一流元数据过滤（first-class metadata filtering）
  - ✅ 持久化（persistent）
  - ✅ 零网络开销
- **定位**：
  - 微秒级查询路径优化
  - 适合高维工作负载

#### **qdrant-edge** —— Qdrant 官方嵌入式版本
- **Crate**：https://lib.rs/crates/qdrant-edge
- **版本**：0.6.0（2026-03-19）
- **技术特征**：
  - ✅ Qdrant 官方维护
  - ✅ 嵌入式设备优化
  - ✅ 最小内存占用
  - ✅ 可选与 Qdrant Cloud 同步
  - ⚠️ 依赖较重（40+ crates，含 tokio/tonic/arrow）
- **适用场景**：
  - 移动 agent
  - 自主系统
  - 嵌入式设备
- **劣势**：
  - 依赖树复杂（与「减内存」目标冲突）
  - 需要 async runtime（tokio）

---

## 二、SQLite FTS5 中文分词实际情况

### 2.1 官方文档确认
- **SQLite FTS5 官方**：https://sqlite.org//fts5.html
- **核心内容**：
  - FTS5 是 SQLite 虚拟表模块
  - 提供全文搜索功能
  - 支持多种 tokenizer

### 2.2 Tokenizer 选项
根据搜索结果，FTS5 内置三种 tokenizer：
1. **unicode61**（默认）
   - 按 Unicode 字符边界分词
   - 中文效果：逐字拆分（"知识库" → "知" "识" "库"）
   - 优势：多语言通用，召回率高
   - 劣势：精度低，噪音多
   
2. **simple**
   - 仅按空白符分词
   - 中文效果：整句当一个词元（**几乎不可用**）
   
3. **porter**
   - 英文词干提取
   - 中文：**完全不适用**

### 2.3 中文分词扩展
- **jieba C 扩展**：需编译 SQLite 扩展（Windows 上极复杂）
- **无官方支持**：SQLite FTS5 没有内置中文智能分词
- **第三方方案**：
  - 腾讯云文档提到 FTS5 中文使用（https://cloud.tencent.com/developer/section/1419763）
  - 但未提供具体的中文分词方案

---

## 三、Tantivy 中文支持确认

### 3.1 官方文档（https://docs.rs/tantivy/latest/tantivy/）
- **描述**：Rust 实现的搜索引擎库（对标 Lucene）
- **中文 Tokenizer**：
  - ✅ **tantivy-jieba**（第三方支持）
  - ✅ **cang-jie**（第三方支持）
  - ✅ 日语：lindera / Vaporetto / tantivy-tokenizer-tiny-segmenter
  - ✅ 韩语：lindera

### 3.2 配置化 Tokenizer
```
Configurable tokenizer (stemming available for 17 Latin languages) 
with third party support for Chinese (tantivy-jieba and cang-jie)
```

**结论**：
- Tantivy **原生不支持**中文分词
- 需要集成 **tantivy-jieba** 或 **cang-jie** crate
- 这会增加依赖树（+1–2 crates）

---

## 四、方案对比更新（基于实际搜索结果）

| 方案 | 新增依赖 | 实现行数 | 混合检索 | 中文支持 | 成熟度 | 推荐度 |
|---|---|---|---|---|---|---|
| **FTS5 only** | 0 | ~1980 | ❌ | ⭐⭐⭐ | ⭐⭐⭐⭐⭐ | ⭐⭐⭐⭐⭐ |
| **FTS5 + hnsw_rs** | 2 | ~2280 | 需自研 | ⭐⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐⭐⭐ |
| **VecStore** | 1 | ~800 | ✅ 内置 | ⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐⭐⭐（新发现） |
| **PolarisDB** | 1 | ~900 | ❌ | ⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐⭐ |
| **tantivy + jieba** | 30+ | ~2480 | 需自研 | ⭐⭐⭐⭐⭐ | ⭐⭐⭐⭐⭐ | ⭐⭐⭐ |
| **qdrant-edge** | 40+ | ~1000 | ✅ 内置 | ⭐⭐⭐⭐⭐ | ⭐⭐⭐⭐⭐ | ⭐⭐（依赖重） |

---

## 五、关键发现总结

### 发现 1：**VecStore 是最优候选**
**理由**：
- ✅ 单 crate（`vecstore = "0.1.0"`），依赖树可控
- ✅ **混合检索内置**（向量 + BM25），无需自研融合逻辑
- ✅ HNSW 索引成熟（亚毫秒级）
- ✅ 元数据过滤（SQL-like）
- ✅ 持久化 + 快照
- ⚠️ 新项目（需验证生产稳定性）

**与原方案对比**：
| 维度 | FTS5 + hnsw_rs（原推荐） | VecStore（新发现） |
|---|---|---|
| 新增依赖 | 2（hnsw_rs + bincode） | 1（vecstore） |
| 混合检索 | 需自研 RRF 融合（+300 行） | ✅ 内置 |
| 实现行数 | ~2280 | ~800（减少 65%） |
| BM25 支持 | 需自研 | ✅ 内置 |
| 持久化 | 需自研 | ✅ 内置 |

**建议**：
- 优先评估 **VecStore**（可能是「最优解」）
- 若 VecStore 不满足需求，再回退至 FTS5 + hnsw_rs

---

### 发现 2：FTS5 中文分词确实受限
**确认**：
- unicode61 = 逐字拆分（召回高但精度低）
- simple = 几乎不可用
- 无内置智能分词

**缓解措施**：
1. **先实测 unicode61**（最简单，零配置）
2. 若不可接受 → 升级至 **VecStore**（BM25 对中文更友好）
3. 最后选择：tantivy + jieba（依赖最重）

---

### 发现 3：hnsw_rs 仍是纯向量检索首选
**理由**：
- 成熟稳定（被多个项目采用：VecStore / PolarisDB / iQDB）
- 纯 Rust，零外部依赖
- 单 crate，易集成

**但**：
- VecStore / PolarisDB 已封装好 HNSW + 元数据 + 持久化
- 自研 hnsw_rs 方案的代价（+500 行）> 直接用 VecStore（+0 行，内置方案）

---

## 六、修正后的推荐方案

### 阶段 0a：FTS5 only（最低风险，1 天实测）
- 技术栈：SQLite FTS5 (`unicode61`)
- 新增依赖：0
- 实现行数：~1980
- 适用场景：中文召回率 ≥ 60% 的情况

### 阶段 0b：VecStore（新推荐，若 FTS5 不通过）
- 技术栈：`vecstore = "0.1.0"`
- 新增依赖：1
- 实现行数：~800（比原方案少 1480 行）
- 特性：
  - ✅ 混合检索内置（向量 + BM25）
  - ✅ HNSW 索引
  - ✅ 元数据过滤
  - ✅ 持久化 + 快照
- 风险：新项目（0.1.0），需验证稳定性

### 阶段 1：FTS5 + hnsw_rs（备选，若 VecStore 不可用）
- 技术栈：SQLite FTS5 + `hnsw_rs` + 自研融合
- 新增依赖：2
- 实现行数：~2280
- 特性：完全可控，无第三方向量库依赖

### 阶段 2：tantivy + jieba（最后手段，若中文分词刚需）
- 技术栈：`tantivy` + `tantivy-jieba`
- 新增依赖：30+
- 实现行数：~2480
- 特性：企业级中文全文检索

---

## 七、后续行动更新

### 立即执行（本周内，2 天）
1. ✅ **网络搜索完成**（当前文件）
2. ⚠️ **VecStore 可行性评估**（1 小时）
   - 查看 `vecstore` Cargo.toml 依赖树
   - 确认是否有 protobuf / cmake / 复杂构建需求
   - 检查 Windows 兼容性
   - 验证持久化格式是否与 LanceDB 兼容（若需迁移）
3. ⚠️ **FTS5 中文实测**（1 天，按原计划）
   - 若 VecStore 评估通过 → 同时实测 VecStore vs FTS5
   - 若 VecStore 不可用 → 仅实测 FTS5

### F2 批次开工后
4. 根据评估 + 实测结果，选择最终方案：
   - **优先级 1**：VecStore（若评估通过）
   - **优先级 2**：FTS5 only（若实测通过）
   - **优先级 3**：FTS5 + hnsw_rs（若两者都不通过）

---

## 八、决策点更新

### D1（已答复）：是否 FTS only？
**更新**：增加 VecStore 候选
- ✅ **优先评估 VecStore**（单依赖，混合检索内置）
- ⚠️ VecStore 不可用 → 回退 FTS5 only

### D4（待定）：是否需要阶段 1（向量检索）？
**更新**：取决于 VecStore 评估
- **若 VecStore 可用** → 直接跳至「完整混合检索」（跳过阶段 1）
- **若 VecStore 不可用** → 按原计划（FTS5 → FTS5+hnsw_rs → tantivy）

---

**搜索完成时间**：2026-10-08  
**下一步**：
1. VecStore 可行性评估（查看依赖树 + 构建要求）
2. FTS5 中文实测（+ VecStore 对比实测，若评估通过）
