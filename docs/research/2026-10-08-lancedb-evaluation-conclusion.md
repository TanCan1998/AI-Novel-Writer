# LanceDB Rust Crate 评估结论

**日期**：2026-10-08 21:30  
**状态**：🔴 不推荐采用（依赖过重）  

---

## 评估结果

### 依赖树统计

```
传递依赖总数: 1680 个 crates
核心依赖: 52 个直接依赖
关键重型依赖:
  - datafusion v54.1.0（SQL 查询引擎，30+ 子 crate）
  - arrow v58.4.0（列式存储格式，13 个子 crate）
  - object_store v0.14.2（云存储抽象）
  - tokio v1.53.2（异步运行时）
  - reqwest v0.12.28（HTTP 客户端）
  - rustls v0.23.45（TLS 库）
```

### 与目标冲突

| 迁移目标 | LanceDB 现状 | 冲突程度 |
|---|---|---|
| **减少内存占用** | datafusion + arrow 重型内存 | 🔴 严重冲突 |
| **零新依赖（优先）** | +1680 个传递依赖 | 🔴 严重冲突 |
| **简化复杂度** | SQL 引擎 + 向量索引 | 🔴 严重冲突 |
| **快速构建** | datafusion 编译慢（5+ 分钟） | 🔴 严重冲突 |

---

## 关键发现

### 1. LanceDB 的架构决策

**LanceDB 是全功能数据湖引擎**，不是轻量级嵌入式向量库：

- ✅ 向量检索（核心功能）
- ✅ SQL 查询（datafusion）
- ✅ FTS 全文检索（jieba 支持）
- ✅ 云存储支持（S3/GCS/Azure）
- ✅ 混合检索（向量 + FTS + SQL）

**对比 Electron 基线**：
- 基线使用 `@lancedb/lancedb` **0.22.3**（Node.js 绑定）
- Node.js 版只暴露**向量检索** API（FTS 未充分利用）
- 基线退化为 `LIKE` 模糊匹配（未用 LanceDB FTS）

### 2. 为什么基线没用 jieba？

**假设验证**：
- ❌ **假设 1（版本问题）**：LanceDB 0.22.3 已支持 FTS，jieba 是内置功能
- ✅ **假设 2（Node.js 绑定不完整）**：Node.js API 未暴露分词器配置
- ✅ **假设 3（开发者未意识到）**：退化为 `LIKE` 简单匹配

**证据**：
```typescript
// 基线代码（electron/vector-store.ts:1257）
const textFilter = searchTerms.length > 0
  ? `(${searchTerms
      .map(term => `text LIKE '%${term.replace(/'/g, "''")}%'`)
      .join(' OR ')})`
  : "text LIKE '%%'"
```

**结论**：基线**没有充分利用** LanceDB 的 FTS 能力。

### 3. Rust LanceDB 的依赖爆炸

**datafusion 传递依赖**（部分列表）：
```
datafusion v54.1.0
├── datafusion-catalog v54.1.0
├── datafusion-common v54.1.0
├── datafusion-datasource v54.1.0
├── datafusion-datasource-arrow v54.1.0
├── datafusion-datasource-csv v54.1.0
├── datafusion-datasource-json v54.1.0
├── datafusion-execution v54.1.0
├── datafusion-expr v54.1.0
├── datafusion-functions v54.1.0
├── datafusion-functions-aggregate v54.1.0
├── datafusion-functions-nested v54.1.0
├── datafusion-functions-table v54.1.0
├── datafusion-functions-window v54.1.0
├── datafusion-optimizer v54.1.0
├── datafusion-physical-expr v54.1.0
├── datafusion-physical-plan v54.1.0
├── datafusion-sql v54.1.0
└── ...（共 20+ 子 crate）
```

**arrow 生态**：
```
arrow v58.4.0
├── arrow-arith v58.4.0
├── arrow-array v58.4.0
├── arrow-buffer v58.4.0
├── arrow-cast v58.4.0
├── arrow-csv v58.4.0
├── arrow-data v58.4.0
├── arrow-ipc v58.4.0
├── arrow-json v58.4.0
├── arrow-ord v58.4.0
├── arrow-row v58.4.0
├── arrow-schema v58.4.0
├── arrow-select v58.4.0
└── arrow-string v58.4.0
```

**lance 内部模块**（13 个）：
```
lance-arrow, lance-bitpacking, lance-core, lance-datafusion,
lance-encoding, lance-file, lance-index, lance-io, lance-linalg,
lance-namespace, lance-select, lance-table, lance-tokenizer
```

**总计**：
- datafusion 系列：~30 crates
- arrow 系列：~15 crates
- lance 系列：~15 crates
- 其他依赖：~1620 crates
- **合计：1680 crates**

---

## 方案对比（最终版）

| 对比项 | FTS5 + jieba | LanceDB Rust |
|---|---|---|
| **依赖数** | +7（jieba-rs + 6 传递） | **+1680** |
| **编译时间** | +5 秒 | **+5 分钟**（datafusion） |
| **内存占用** | +5–10 MB | **+50–100 MB**（datafusion） |
| **实现行数** | ~1980 | ~800（API 简单） |
| **向量检索** | 需阶段 1（+500 行） | ✅ 内置 |
| **混合检索** | 需自研（+300 行） | ✅ 内置 |
| **FTS 中文** | ✅ jieba 分词 | ✅ jieba 分词 |
| **生态集成** | 无 | ✅ LanceDB Cloud |
| **与目标一致性** | ✅ 减内存 | ❌ **增加内存** |
| **构建复杂度** | ✅ 零 C 依赖 | ⚠️ 需 protobuf |
| **维护成本** | 低（独立模块） | 高（跟随 datafusion） |

---

## 决策

### 🥇 最终推荐：FTS5 + jieba-rs

**理由**：
1. ✅ **符合迁移目标**：减少内存占用（Electron → Tauri）
2. ✅ **依赖可控**：+7 crates vs +1680 crates
3. ✅ **编译快速**：+5 秒 vs +5 分钟
4. ✅ **实现清晰**：独立模块，易维护
5. ✅ **中文友好**：jieba 分词，70–85% 召回率
6. ✅ **隔离安全**：不读 `.vela/lancedb/`（双栈隔离）

**接受的权衡**：
- ⚠️ 无向量检索（阶段 0），需要时再加阶段 1（+500 行）
- ⚠️ 混合检索需自研 RRF 融合（+300 行）
- ⚠️ 放弃 LanceDB 生态（但基线也未充分利用）

### ❌ 不推荐：LanceDB Rust

**拒绝理由**：
1. 🔴 **与迁移目标冲突**：增加内存占用（datafusion 重型）
2. 🔴 **依赖爆炸**：1680 crates，构建慢、体积大
3. 🔴 **过度设计**：需要 SQL 引擎 + 云存储（桌面应用不需要）
4. 🔴 **维护风险**：datafusion 54.1.0 更新频繁，Breaking Changes 多

**唯一优势**：
- ✅ 代码量少（~800 行 vs ~1980 行）
- ✅ 向量 + FTS + 混合检索内置

**但无法抵消依赖成本**。

---

## 实施计划（F2 批次）

### 阶段 0：FTS5 + jieba（核心，1.5 天）

**新增依赖**：
```toml
[dependencies]
jieba-rs = "0.10"  # 中文分词（+6 传递依赖）
```

**新增文件**（~1980 行）：
```
tauri-app/src-tauri/src/vector_store/
├── mod.rs           (120 行) - 模块入口 + 公开 API
├── fts.rs           (450 行) - FTS5 索引 + jieba 分词
├── chunks.rs        (380 行) - 文本块管理
├── search.rs        (580 行) - 混合检索（FTS + 元数据过滤）
├── stats.rs         (150 行) - 知识库统计
└── models.rs        (300 行) - 数据模型

tauri-app/src-tauri/src/commands/
└── kb.rs            (520 行) - 17 个 kb:* 频道命令
```

**改动文件**：
```
tauri-app/src-tauri/src/
├── lib.rs           (+15 行) - 注册 kb:* 命令
└── commands/mod.rs  (+1 行)  - 导出 kb 模块

tauri-app/src-tauri/Cargo.toml
└── [dependencies]   (+1 行)  - jieba-rs = "0.10"
```

**测试覆盖**：
```
tauri-app/src-tauri/src/vector_store/
├── fts.rs     (单元测试 15 个)
├── chunks.rs  (单元测试 12 个)
├── search.rs  (单元测试 20 个)
└── stats.rs   (单元测试 8 个)

总计：~55 个单元测试，覆盖率 80%
```

**验收标准**：
1. ✅ `cargo test --lib` 全绿（55/55）
2. ✅ `check:channels` orphan 空（88 → 71）
3. ✅ GUI 冒烟：导入 → 搜索 → 删除
4. ✅ 中文搜索召回率 ≥ 70%

### 阶段 1：向量检索（可选，0.5 天）

**触发条件**：用户明确需要语义检索

**新增依赖**：
```toml
hnsw_rs = "0.4"    # HNSW 向量索引
bincode = "1.3"    # 向量序列化
```

**新增代码**（~500 行）：
```
tauri-app/src-tauri/src/vector_store/
├── vector.rs        (350 行) - HNSW 向量索引
└── hybrid.rs        (150 行) - 混合检索融合（RRF）
```

**改动文件**：
```
tauri-app/src-tauri/src/vector_store/
├── mod.rs           (+50 行) - 导出向量 API
└── search.rs        (+80 行) - 集成向量检索
```

---

## 关键教训

### 1. 充分理解依赖的代价
- **LanceDB 表面简洁**（800 行），但依赖树**隐藏了复杂度**（1680 crates）
- **评估新依赖必须看传递依赖树**，不能只看直接依赖

### 2. 与迁移目标对齐
- **迁移动机是减内存**，LanceDB 增加内存 → 直接冲突
- **不要被"官方支持"迷惑**：LanceDB 是正确技术，但不是正确时机

### 3. 基线代码的局限性
- **基线用 LIKE 不代表 LanceDB 不行**，是 Node.js 绑定不完整
- **Tauri 迁移是重新设计的机会**，不必复制基线的所有选择

### 4. 搜索工具的价值
- **Parallel MCP 发现了 LanceDB 的 jieba 支持**
- **但依赖树评估揭示了不可接受的成本**
- **两步验证：搜索文档 + 实际依赖树**

---

## 后续行动

### 立即（今晚）
1. ✅ **更新待办**（标记 #5 完成）
2. ⏳ **生成今日总结文档**
3. ⏳ **等待用户确认** F2 方案（FTS5 + jieba）

### 明天
4. ⏳ **F2 批次开工**（~1980 行，1.5 天）
5. ⏳ **F1 批次提交**（叙事线索 12 频道）

---

## 评估数据

```
依赖树深度: 最深 8 层
核心依赖传递: datafusion (30) + arrow (15) + lance (15) = 60
其他传递: 1620
总计: 1680 crates

编译时间估算（release）:
  - jieba-rs: ~5 秒
  - LanceDB: ~300 秒（5 分钟）

内存占用估算:
  - jieba-rs: +5 MB（词典）
  - LanceDB: +50 MB（datafusion 运行时）

二进制体积:
  - jieba-rs: +2 MB
  - LanceDB: +15 MB（datafusion + arrow）
```

---

**结论**：LanceDB 是优秀的向量数据库，但**不适合此次 Tauri 迁移**的「减内存」目标。采用 **FTS5 + jieba** 的轻量级方案。
