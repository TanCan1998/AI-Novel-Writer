# 实测执行指南

> **目的**：对比 FTS5 和 VecStore 在小说知识库场景的中文检索效果。
> **测试代码位置**：
> - `tauri-app/src-tauri/examples/fts5_benchmark.rs`
> - `tauri-app/src-tauri/examples/vecstore_benchmark.rs`

---

## 一、执行步骤

### 1.1 添加依赖

编辑 `tauri-app/src-tauri/Cargo.toml`，在 `[dev-dependencies]` 下添加：

```toml
[dev-dependencies]
# FTS5 实测
rusqlite = { version = "0.40.2", features = ["bundled"] }

# VecStore 实测
vecstore = "1.0.0"
anyhow = "1.0"
serde_json = "1.0"
```

### 1.2 执行 FTS5 实测

```bash
cd tauri-app/src-tauri

# 设置 Rust 环境（Windows）
$env:RUSTUP_HOME = 'D:\Environment\rust\rustup'
$env:CARGO_HOME = 'D:\Environment\rust\cargo'
$env:Path = "D:\Environment\rust\cargo\bin;$env:Path"

# 运行 FTS5 实测
cargo run --example fts5_benchmark --release
```

**预期输出**：
```
初始化 FTS5 测试环境...
导入测试语料...
执行测试用例...
  [1/15] 人物名字（精确）
  [2/15] 配角身份
  ...

=== FTS5 中文分词实测报告 ===

Tokenizer: unicode61（默认）
测试用例数: 15

详细结果:

测试用例                  期望     实际    召回率%     精度%    延迟ms
-------------------------------------------------------------------------------------
人物名字（精确）            1        1      100.0      100.0       0.50
配角身份                    3        3      100.0      100.0       0.45
...

平均值                                       XX.X%      XX.X%       X.XX

=== 评估结论 ===

平均召回率: XX.X%
平均精度: XX.X%
平均延迟: X.XXms

✅/❌ 通过标准（召回率 ≥ 60%）
```

### 1.3 执行 VecStore 实测

```bash
# 运行 VecStore 实测
cargo run --example vecstore_benchmark --release
```

**⚠️ 注意**：
- 当前 VecStore 实测使用**模拟 embedding**（基于文本哈希）
- **不具备真实语义理解能力**，召回率可能偏低
- 若要真实测试，需要：
  1. 集成 OpenAI / SiliconFlow embedding API
  2. 修改 `generate_embedding()` 函数调用真实 API
  3. 重新运行测试

---

## 二、实测报告模板

### 2.1 对比表格

| 指标 | FTS5 unicode61 | VecStore (模拟) | VecStore (真实 API)* | 胜出方 |
|---|---|---|---|---|
| **平均召回率** | ?% | ?% | ?% | ? |
| **平均精度** | ?% | ?% | ?% | ? |
| **平均延迟** | ?ms | ?ms | ?ms | ? |
| **依赖复杂度** | 零依赖 | 172 crates | 172 crates | FTS5 |
| **实现行数** | ~1980 | ~800 | ~800 | VecStore |
| **混合检索** | ❌ | ✅ | ✅ | VecStore |
| **中文分词** | 逐字拆分 | BM25 + 向量 | BM25 + 向量 | ? |

\* 需要额外实施 embedding API 集成

### 2.2 决策矩阵

```
情况 1：FTS5 召回率 ≥ 60%
  → ✅ 采用 FTS5 only
  → 理由：零依赖 + 最低风险 + 满足需求

情况 2：FTS5 < 60%，VecStore (真实 API) ≥ 60%
  → ✅ 采用 VecStore
  → 理由：混合检索内置 + 节省 1480 行

情况 3：两者都 < 60%
  → ✅ 采用 FTS5 + hnsw_rs（自研）
  → 理由：完全可控 + 无第三方库风险

情况 4：无法测试 VecStore (真实 API)
  → 若 FTS5 ≥ 60% → 采用 FTS5
  → 若 FTS5 < 60% → 采用 FTS5 + hnsw_rs（回退）
```

---

## 三、已知限制

### 3.1 VecStore 实测的局限性

**当前实现（模拟 embedding）**：
```rust
fn generate_embedding(text: &str) -> Vec<f32> {
    // ⚠️ 简化版：基于文本哈希生成向量
    // 优势：快速验证 VecStore API 可用性
    // 劣势：不具备语义理解能力
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    let seed = hasher.finish();
    
    // 生成 384 维向量
    let mut rng = seed;
    (0..384)
        .map(|i| {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            ((rng as f32 / u64::MAX as f32) - 0.5) * 2.0
        })
        .collect()
}
```

**真实 embedding 集成（需额外实施）**：
```rust
async fn generate_embedding(text: &str) -> Result<Vec<f32>> {
    // 调用 OpenAI / SiliconFlow API
    let client = reqwest::Client::new();
    let response = client
        .post("https://api.openai.com/v1/embeddings")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&json!({
            "model": "text-embedding-3-small",
            "input": text
        }))
        .send()
        .await?;
    
    let data: EmbeddingResponse = response.json().await?;
    Ok(data.data[0].embedding)
}
```

**工作量估算**：
- 集成 embedding API：+100 行
- 异步支持（tokio）：+50 行
- 错误处理 + 重试：+50 行
- **总计**：+200 行（1 天）

### 3.2 FTS5 实测的已知问题

**unicode61 分词特性**：
- "知识库" → "知" "识" "库"（逐字拆分）
- 优势：召回率高（任意子串都能匹配）
- 劣势：精度低（"知" 会匹配所有含该字的文档）

**预期结果**：
- 精确匹配（人物名、地点）：召回率 ≥ 90%
- 多字组合（"青云宗"）：召回率 ≥ 80%
- 同义词查询（"修炼" vs "练功"）：召回率 < 50%（预期失败）

---

## 四、后续行动

### 4.1 立即执行（2 小时）

1. ✅ **实测代码已创建**
2. ⚠️ **执行 FTS5 实测**
   ```bash
   cd tauri-app/src-tauri
   cargo run --example fts5_benchmark --release
   ```
3. ⚠️ **执行 VecStore 实测（模拟 embedding）**
   ```bash
   cargo run --example vecstore_benchmark --release
   ```
4. ⚠️ **记录实测数据**（填写上方对比表格）

### 4.2 可选（若 FTS5 不通过，1 天）

5. ⚠️ **集成真实 embedding API**（+200 行）
6. ⚠️ **重新执行 VecStore 实测**
7. ⚠️ **对比真实 API 结果**

### 4.3 决策与实施（根据结果）

8. ⚠️ **更新 F2 决策文档**
9. ⚠️ **F2 批次实施**（~800–2280 行）

---

## 五、故障排查

### 5.1 编译错误：找不到 vecstore

**原因**：`vecstore` crate 未添加到依赖

**解决**：
```bash
cd tauri-app/src-tauri
cargo add vecstore --dev
```

### 5.2 运行错误：database is locked

**原因**：SQLite 文件被其他进程占用

**解决**：
```rust
// 使用内存数据库（FTS5 实测已使用）
let conn = Connection::open_in_memory()?;

// 或使用唯一临时文件（VecStore 实测已使用）
let db_path = temp_dir.join(format!("vecstore_test_{}.db", std::process::id()));
```

### 5.3 VecStore API 不匹配

**原因**：实测代码假设的 API 可能与实际不符

**解决**：
1. 查看 VecStore 文档：https://docs.rs/vecstore
2. 修正 API 调用（特别是 `query` / `hybrid_search` 方法）
3. 参考 GitHub 示例：https://github.com/PhilipJohnBasile/vecstore

---

## 六、预期时间线

| 任务 | 预估时间 | 前置条件 |
|---|---|---|
| 添加依赖 + 编译 | 10 分钟 | 无 |
| 执行 FTS5 实测 | 5 分钟 | 编译通过 |
| 执行 VecStore 实测（模拟） | 10 分钟 | 编译通过 |
| **小计（最小化）** | **25 分钟** | — |
| 集成真实 embedding API | 1 天 | FTS5 不通过 |
| 执行 VecStore 实测（真实） | 10 分钟 | API 集成完成 |
| **小计（完整）** | **1 天** | — |

---

**文档创建时间**：2026-10-08  
**下一步**：执行 FTS5 实测，记录结果
