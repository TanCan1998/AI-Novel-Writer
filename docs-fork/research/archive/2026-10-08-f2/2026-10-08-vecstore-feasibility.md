# VecStore 可行性评估报告（2026-10-08）

> **评估目的**：确定 VecStore 是否可作为 F2 批次的向量存储方案。
> **评估时间**：2026-10-08
> **评估结论**：✅ **可行**（有条件推荐）

---

## 一、基本信息

### 1.1 Crate 元数据
| 项 | 值 | 评估 |
|---|---|---|
| 版本 | **1.0.0**（2025-10-21 发布） | ✅ 已达稳定版 |
| 总下载量 | **2,699** | ⚠️ 较低（新项目） |
| 近期下载量 | **1,280** | ⚠️ 社区采用度低 |
| 文档 | https://docs.rs/vecstore | ✅ 有官方文档 |
| 仓库 | https://github.com/PhilipJohnBasile/vecstore | ✅ 开源（MIT） |
| 关键字 | search, embedding, hnsw, vector-database, rag | ✅ 定位明确 |
| 分类 | data-structures, database, algorithms | ✅ 正确分类 |

**结论**：
- ✅ **已是 1.0 稳定版**（不是 0.1.0，搜索结果过时）
- ⚠️ **下载量较低**（2699 总下载，社区成熟度待验证）
- ✅ 有官方文档和活跃仓库

---

## 二、依赖分析

### 2.1 直接依赖清单（18 个）
```
anyhow          错误处理
bincode         二进制序列化（向量持久化）
chrono          时间戳
clap            命令行（可能用于工具）
directories     跨平台目录路径
getrandom       随机数生成
hnsw_rs         ✅ 核心向量索引（纯 Rust）
memmap2         内存映射文件
ordered-float   有序浮点数
rand            随机数
rayon           并行计算
regex           正则表达式
serde           序列化
serde_json      JSON 序列化
tempfile        临时文件
thiserror       错误类型
tracing         日志追踪
tracing-subscriber  日志订阅
```

### 2.2 传递依赖统计
- **总 crate 数**：172 个（cargo tree 输出的 "Locking 172 packages"）
- **关键传递依赖**：
  - `hnsw_rs` → `anndists` / `mmap-rs` / `indexmap` / `parking_lot`
  - `rayon` → 并行计算框架
  - `regex` → 正则引擎
  - `tracing` → 日志系统

**依赖树深度评估**：
- ⚠️ **172 个传递依赖**（比预期多，但仍在可控范围）
- ✅ **无 protobuf / cmake / bindgen 等复杂构建依赖**
- ✅ **无 tokio / async-runtime**（纯同步 API）
- ✅ **无 arrow / datafusion 等重型依赖**

---

## 三、构建复杂度分析

### 3.1 构建工具需求
**检查结果**：
- ✅ **无需 protoc**（无 protobuf 依赖）
- ✅ **无需 cmake**（无 C++ 绑定）
- ✅ **无需 NASM/Ninja**（无汇编优化）
- ✅ **纯 Rust 实现**（hnsw_rs 是纯 Rust）

**构建时间估算**：
- `cargo build --release`（首次）：约 3–5 分钟（172 crates）
- 增量构建：< 10 秒

### 3.2 Windows 兼容性
**依赖中的 Windows 特定 crate**：
- `windows-targets` v0.48.5
- `windows_x86_64_msvc` v0.48.5
- `windows-sys` v0.61.2
- `winapi` v0.3.9

**结论**：
- ✅ **明确支持 Windows**（有专门的 Windows 绑定）
- ✅ **MSVC 工具链支持**（与 Tauri 一致）

---

## 四、与原方案对比

### 4.1 方案对比表
| 对比项 | FTS5 only | FTS5 + hnsw_rs | **VecStore** | tantivy + jieba |
|---|---|---|---|---|
| 新增依赖（直接） | 0 | 2 | **18** | 30+ |
| 传递依赖总数 | 0 | ~30 | **172** | ~200 |
| 实现行数 | ~1980 | ~2280 | **~800** | ~2480 |
| 混合检索 | ❌ | 需自研 | **✅ 内置** | 需自研 |
| BM25 支持 | ❌ | ❌ | **✅ 内置** | ✅ |
| 元数据过滤 | ❌ | 需自研 | **✅ 内置** | ✅ |
| 持久化 | ✅（SQLite） | 需自研 | **✅ 内置** | ✅ |
| 快照备份 | ❌ | ❌ | **✅ 内置** | ❌ |
| 中文分词 | unicode61 | unicode61 | **BM25** | jieba |
| 构建复杂度 | 极低 | 低 | **低** | 中 |
| 社区成熟度 | ⭐⭐⭐⭐⭐ | ⭐⭐⭐⭐ | **⭐⭐** | ⭐⭐⭐⭐⭐ |

### 4.2 代码量对比（估算）
**VecStore 集成（~800 行）**：
```rust
// src/vector_store/mod.rs (~200 行)
use vecstore::VecStore;

pub struct VectorStoreService {
    store: VecStore,
}

impl VectorStoreService {
    pub fn open(path: &Path) -> Result<Self> {
        let store = VecStore::open(path)?;
        Ok(Self { store })
    }
    
    pub fn insert(&mut self, id: &str, vector: Vec<f32>, metadata: Metadata) -> Result<()> {
        self.store.upsert(id, vector, serde_json::to_value(metadata)?)?;
        Ok(())
    }
    
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        let results = self.store.query(query, k)?;
        // 转换为应用层类型
        Ok(results.into_iter().map(|r| SearchResult::from(r)).collect())
    }
    
    pub fn hybrid_search(&self, query: &[f32], text: &str, k: usize) -> Result<Vec<SearchResult>> {
        // VecStore 内置混合检索
        let results = self.store.hybrid_search(query, text, k)?;
        Ok(results.into_iter().map(|r| SearchResult::from(r)).collect())
    }
}

// src/commands/kb.rs (~400 行)
// 17 个命令的路由层，调用 VectorStoreService

// src/repositories/kb_repository.rs (~200 行)
// 文档元信息、统计等辅助功能
```

**FTS5 + hnsw_rs 集成（~2280 行）**：
- FTS5 封装：~200 行
- hnsw_rs 封装：~300 行
- 向量持久化（bincode）：~150 行
- **混合检索融合（RRF）**：~250 行（VecStore 省略）
- **BM25 实现**：~200 行（VecStore 省略）
- 元数据过滤：~180 行（VecStore 省略）
- 命令层 + 仓储层：~600 行
- 测试代码：~400 行

**节省工作量**：~1480 行（65%）

---

## 五、风险评估

### 5.1 主要风险
| 风险 | 严重度 | 概率 | 缓解措施 |
|---|---|---|---|
| **社区成熟度低**（2699 下载） | 🟡 中 | 🟡 中 | ① 深度代码审查；② 准备回退方案（hnsw_rs） |
| **依赖树较重**（172 crates） | 🟢 低 | ✅ 已确认 | 与「减内存」目标仍一致（无 datafusion 等重型依赖） |
| **API 稳定性**（1.0 刚发布） | 🟡 中 | 🟢 低 | 1.0 版本通常已稳定；关注后续 patch 版本 |
| **Windows 兼容性** | 🟢 低 | ✅ 已确认 | 有 Windows 特定依赖，MSVC 支持 |
| **混合检索质量未验证** | 🟡 中 | 🟡 中 | **必须实测**（与 FTS5 对比） |
| **中文 BM25 效果** | 🟡 中 | 🟡 中 | **必须实测**（15 个测试用例） |

### 5.2 回退方案
若 VecStore 实测不通过或出现严重问题：
1. **立即回退**至 FTS5 + hnsw_rs（原推荐方案）
2. **代价**：+1480 行实现，+2 天工期
3. **触发条件**：
   - 混合检索召回率 < 50%
   - 中文 BM25 效果显著差于 FTS5 unicode61
   - 发现严重 bug 且无法快速修复
   - 依赖冲突无法解决

---

## 六、实测计划（修订）

### 6.1 对比实测矩阵
| 方案 | 实测内容 | 预期时间 |
|---|---|---|
| **VecStore** | 混合检索（向量 + BM25） | 4 小时 |
| **FTS5 unicode61** | 纯关键词检索 | 4 小时 |
| **基线 LanceDB**（理论） | 理论上限（参考数据） | — |

### 6.2 测试用例（15 个，同原计划）
```rust
// 典型小说知识库查询
let test_cases = vec![
    // 人物查询
    ("主角名字", vec!["林晚", "林晚晚"]),
    ("配角身份", vec!["师兄", "青云宗"]),
    
    // 世界观设定
    ("修仙体系", vec!["筑基", "金丹", "元婴"]),
    ("地点描述", vec!["断魂崖", "迷雾森林"]),
    
    // 情节关键词
    ("战斗场景", vec!["斗法", "法宝", "灵力"]),
    ("感情线索", vec!["心动", "暗恋"]),
    
    // 长尾查询（测试语义相似）
    ("如何突破境界", vec!["突破", "瓶颈", "感悟"]),
    ("时间描述", vec!["三年后", "十年间"]),
    
    // 混合中英文
    ("功法名称", vec!["太玄心经", "剑法"]),
    
    // 专有名词
    ("门派组织", vec!["天机阁", "魔道联盟"]),
    
    // ... 共 15 个
];
```

### 6.3 评估指标
| 指标 | VecStore | FTS5 | 通过标准 |
|---|---|---|---|
| **召回率** | ? | ? | ≥ 60% |
| **精度** | ? | ? | ≥ 40% |
| **查询延迟** | ? | ? | < 50ms |
| **内存占用** | ? | ? | < 索引大小 1.5× |
| **易用性** | ? | ? | API 简洁度 |

---

## 七、评估结论

### ✅ 可行性：**有条件推荐**

**满足条件**：
1. ✅ 纯 Rust 实现，无复杂构建依赖
2. ✅ Windows MSVC 兼容
3. ✅ 依赖树可控（172 crates，无重型依赖）
4. ✅ API 简洁，集成工作量低（~800 行 vs ~2280 行）
5. ✅ 混合检索 / BM25 / 元数据过滤 / 快照全部内置

**不满足条件**：
1. ⚠️ 社区成熟度低（2699 下载，1.0 刚发布）
2. ⚠️ **混合检索质量未验证**（必须实测）
3. ⚠️ **中文 BM25 效果未知**（必须实测）

### 推荐决策流程

```
步骤 1：VecStore 实测（4 小时）
  ├─ 通过（召回率 ≥ 60%）→ ✅ 采用 VecStore（节省 1480 行）
  └─ 不通过 → 步骤 2

步骤 2：FTS5 实测（4 小时）
  ├─ 通过（召回率 ≥ 60%）→ ✅ 采用 FTS5 only（零依赖）
  └─ 不通过 → 步骤 3

步骤 3：回退至 FTS5 + hnsw_rs
  └─ ✅ 实现自研混合检索（+1480 行，+2 天）
```

---

## 八、后续行动

### 立即执行（1 天）
1. ✅ **可行性评估完成**（当前文件）
2. ⚠️ **创建实测代码**（VecStore + FTS5 对比）
   - `tauri-app/src-tauri/examples/vecstore_benchmark.rs`
   - `tauri-app/src-tauri/examples/fts5_benchmark.rs`
3. ⚠️ **执行实测**（8 小时）
4. ⚠️ **输出实测报告**（含决策建议）

### F2 批次开工
5. 根据实测结果，实施选定方案
6. 更新 F2 决策文档

---

**评估完成时间**：2026-10-08  
**评估结论**：✅ **VecStore 可行**，但必须通过实测验证混合检索质量  
**下一步**：创建 VecStore vs FTS5 对比实测代码
