# Tauri 迁移进度快照（2026-10-09）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> **规则**：按日期命名，一个工作日一个新文件；当日新增只写当日文件，跨日不回填旧文件；
> 旧文件冻结后不允许修改（见 [`docs-fork/agents/pi-development.md`](../agents/pi-development.md) §9）。
> **模板**：[`_TEMPLATE-tauri-migration-status.md`](./_TEMPLATE-tauri-migration-status.md)。
>
> 全部历史快照见 `docs-fork/handoffs/` 目录（按日期命名）。
>
> - channel 级盘点：[`docs-fork/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-09 · 第三十二次）

> 本表只填**最新一次自检的实测值**。改表前必须重跑对应命令，不得沿用旧数字、不得估算。
> 本轮实测命令与输出见下方「[§5 自检记录](#5-自检记录2026-10-09-实测)」。

| 项 | 值 |
|---|---|
| 仓库 / 分支 | **`TanCan1998/Lorekeeper`**（`EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork）· `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 已注册命令 | **152** |
| 覆盖 invoke 频道 | **151**（契约总数 193，事件频道 4） |
| 未迁移 invoke 频道 | **42**（`db=19 mcp=9 update=6 skills=4 prompt=3 dialog=1`） |
| orphan | **空** ✅ |
| `cargo test --lib` | **491/491** ✅ |
| `cargo check --all-targets` | **0 告警** ✅ |
| `pnpm typecheck` / `lint` | exit 0 / exit 0 ✅ |
| 定向 `vitest` | **16/16**（6 文件：契约覆盖 / 入参结构体契约 / 源码契约 / locale / finalization-client / finalization-snapshot）✅ |
| 已完成批次 | A ✅ / B ✅（含遗留补齐） / C ✅ / D1 ✅ / D2-a ✅ / D2-b ✅ / D2-c ✅ / **E ✅（两部分全部完成）** / **F1 ✅** / **L3 ✅** / **F2 ✅（F2-1/F2-2/F2-3 全部完成）** / **G1 ✅（批次 E 收口）** |
| 当前阶段 | **G1 ✅ 完成（批次 E 彻底收口）**：`finalization:commit` / `finalization:retry` + `manuscript_publisher.rs` 落地，`chapter:*` 删实体稿**真实化**（B2 全部解除）；契约补在 **tauri-app 副本**（基线 `src/` 未动）。下一步：① **批次 H**（`fs:grant-*` 三命令 → 解开 KB 界面导入 / 导出 / 角色卡导入，见 B10；+ update/mcp/prompt/skills）；② 批次 G（import-run + `kb:import-reference-text` 收口）；③ 上游 502 提交合并专项（本轮已评估，见第三十二次 §0） |
| 依赖 | `reqwest 0.13`（`default-features=false` + `native-tls` + `socks`）、`tauri-plugin-dialog 2`（Cargo.lock 锁 **2.8.1**）。**F2 无新增依赖**：`hnsw_rs 0.3.4` / `jieba-rs 0.7.0` / `tokio` 已在 `Cargo.toml`（`4aff3f65`）；**FTS5 由 `libsqlite3-sys` bundled 提供**（`-DSQLITE_ENABLE_FTS5` 实测） |
| GUI 冒烟 | ✅ 自 2026-10-07 起 **八轮**。近两轮：dialog 轮 vite `453 ms` + cargo `24.91s`；**第八轮（2026-10-09，G1）**：vite `533 ms` + cargo `1.33s`（增量）→ `lorekeeper.exe` 运行正常，**G1 定稿（两章 outbox `published` + `.txt` 落盘且标题剥离）+ B2 删除（两章实体稿真实删除、KB 文档真实清理）全部通过**；本轮共 3 次 cargo-watch 自动重建（19.59s / 19.92s / 增量），过程中发现并修复 1 个入参契约缺陷（见第三十二次 §2） |
| 双栈隔离 | **L0/L1/L2/L3 全部独立**：安装标识 / `~/.lorekeeper` / `<root>/.lore/`（库 `.lore/lorekeeper.db`、KB 向量 `.lore/kb/`）。基线为 `~/.vela` / `<root>/.vela/`。**两栈项目目录刻意不互通**（`ee40aaab`） |
| Rust 工具链 | rustc/cargo **1.99.0 stable-msvc** @ `D:\Environment\rust\`（脚本内须显式设 `RUSTUP_HOME` / `CARGO_HOME`）。`tauri-plugin-dialog 2.8.1` 要求 **rustc ≥ 1.90**（CI 最低版本需相应抬高） |

<sub>*命令与频道差额：**152 命令**中 1 个为阶段 0 骨架（`app_health_check`，不对应任何契约频道），其余 **151** 与 invoke 频道一一对应。`dialog:select-export-directory` 仍是诚实化占位（返回 `None`，批次 H）。</sub>

---

## 本次更新（第三十二次：批次 E 收口 G1 —— 定稿提交/重试 + 实体稿发布与清理真实化；GUI 冒烟发现并修复入参契约缺陷）

> 承接第三十一次，同属 **2026-10-09**。本轮把批次 E 唯一剩余缺口 **G1** 落地，
> 并借此**真实化** `chapter:*` 的删实体稿投影（解除 B2），随后完成**第八轮 GUI 冒烟**。

### 0. 决策（用户确认，本轮）

| 决策 | 选择 |
|---|---|
| **上游策略** | **暂不合并 upstream**。核查（2026-10-09）：上游领先 **502 提交**（合并基点 `992b3f5f`，2026-10-01）；invoke 频道 191 → **205**；上游**仍未声明** `FinalizationChannels`。**利好**：已迁移的 149 频道**零删除/零改名**（上游只新增）。继续冻结 10-01 基线，上游合并另立专项 |
| **G1 契约归属** | **只补 tauri-app 副本**（`tauri-app/src/shared/ipc-channels.ts`），并把 `scripts/verify-channel-coverage.mjs` 与 `test/channel-migration-coverage.test.ts` 的契约读取路径改为 tauri-app 副本 → **基线 `src/` 保持逐字节不变**，上游可合并。代价：两份契约分叉（已记入盘点文档 §1） |
| **B2 范围** | G1 一并真实化「删实体稿文件」，**不再等批次 H**（核对基线 `chapter-deletion-service.ts`：删实体稿仅 `unlink` 项目根内冻结文件名，路径受约束，不需 ADR 0002 外部授权） |
| **冒烟方式** | GUI 加一个「假」生成模型（内置 OpenAI 预设 + 任意 key，仅用于通过生成运行时前置校验）。**定稿的提交/发布在命令体最前**（`finalize-chapter.command.ts:708`），随后的 AI 后处理失败不影响已提交事实 |

### 1. 交付

**Rust**
- `repositories/finalization_repository.rs`：+`FinalizationCommitInput` / `commit`（正文·字数·定稿状态·outbox **同事务**）/ `get` / `get_by_draft_id` / `mark_publication_pending` / `mark_published`；抽取 `map_record` 收敛列投影
- **新增** `manuscript_publisher.rs`：`resolve_manuscript_target`（直接子文件守卫 + 碰撞后缀）/ `publish_manuscript`（临时文件 `create_new` + 原位 rename + 同内容幂等）/ `remove_published_manuscript` / `serialize_manuscript`（章节头 + 剥离首个 Markdown 标题行）。**删除了 `finalized_draft_import_repository` 里的重复实现并改为复用此模块**（消除两份 `resolveManuscriptTarget` 漂移）
- **新增** `commands/finalization.rs`：`finalization_commit` / `finalization_retry`，逐字对齐 `finalization-controller.ts` + `finalization-service.ts` 的**三层失败语义**（controller throw → `"Error: "`；service 明文返回分支不前缀；`publishCommitted` catch → `定稿已提交、实体稿待发布：…`）
- `commands/chapter_lifecycle.rs`：`manuscript_cleanup_unavailable()` 占位 → **真实** `manuscript_publisher::remove_published_manuscript`

**前端 / 契约**
- `src/shared/ipc-channels.ts`：+`FinalizationChannels`（2 频道）+ `FinalizationResult` / `FinalizationSnapshot` / `FinalizationPublicationStatus`（`finalization-snapshot.ts` 改为再导出，单源）
- `src/services/finalization-client.ts`：`getVelaApi()`（读 `window.velaAPI`）→ `ipc.invoke`；`projectSession` 仍**显式**尾参（保持基线 `(snapshot, context)` 签名）
- `src/services/ipc-client.ts`：+2 条 `CHANNEL_ARG_NAMES`；`migrated-channels.ts` 重新生成（149 → **151**）

### 2. 🐞 GUI 冒烟发现并修复的真实缺陷（本轮最大收获）

**现象**：侧边栏删除已定稿章节 → 确认框弹出、点确认后「什么都没发生」（无 toast、库中零记录）。

**根因**：`chapter:delete-finalized` 的入参结构体 `DeleteFinalizedChapterRequest` **缺 `#[serde(rename_all = "camelCase")]`**。渲染层发 `{ draftId, chapterNumber }`，Tauri 在**参数反序列化**阶段即失败并 **reject promise**：

```
Unknown Error: invalid args `request` for command `chapter_delete_finalized`: missing field `draft_id`
```

调用点未 catch → 未处理拒绝 → 静默失败；后端连一行也不会落。**Rust 单测永远抓不到**（测试直接构造结构体）。

**修复与防线**：
1. ✅ 补属性（`repositories/chapter_deletion_repository.rs`）
2. ✅ Rust 契约测试：从 `{"draftId":7,"chapterNumber":3}` 反序列化必须成功（旧 snake_case 载荷必须失败）
3. ✅ **新增源码扫描测试** `test/ipc-arg-struct-contract.test.ts`：任何派生 `Deserialize` 且含下划线字段的结构体都必须显式 camelCase（内部结构体走白名单）。已实测**「注入回归 → 立刻报错」**（去掉属性即报 `DeleteFinalizedChapterRequest [draft_id,chapter_number]`），当前 0 违规
4. 📋 全仓审计：82 个 `Deserialize` 结构体中**仅此 1 个**是 IPC 入参且缺 rename（其余为单词字段或纯内部）；规则已写入 `docs-fork/agents/pi-development.md` §2 第 5 条

### 3. GUI 冒烟（第八轮，2026-10-09）—— ✅ 通过

测试项目 `F:\Temp\loretest\lore-smoke`（手写 `.lore/project.json` 清单；草稿由外部脚本播种，**零 API**）。

| 项 | 结果 |
|---|---|
| 项目打开 | ✅ 自动建 `.lore/lorekeeper.db` + 清单校验通过 |
| **G1 定稿（第 1 章）** | ✅ `drafts.status=finalized`、outbox `published` @ `15:07:23`、落盘 `第1章 # 标题.txt`；内容 `第1章 # 标题\n\n正文…`，**首行 `# 第一章 雾港的灯语` 已剥离** |
| **G1 定稿（第 2 章）** | ✅ outbox `published` @ `15:22:48`、`第2章 潮汐的裂口.txt` |
| **B2 删除（第 1 章）** | ✅ 走 **legacy 人工确认**分支（`required → consumed`）→ `manuscript=completed` / `knowledge=not_required` / `status=completed`，`.txt` **真实删除** |
| **B2 删除（第 2 章）** | ✅ `knowledge_status=completed`（**KB 文档被真实清理**）+ `.txt` 真实删除；`drafts` 归零、项目根无残留 |
| F2 KB 界面导入 | ⛔ 阻塞于 `fs:grant-read-file`（批次 H 占位，见 §4 B10）。服务端内核已被第 2 章后处理间接验证（真实创建 KB 文档并回链，删除时真实清理） |

> 顺带确认：无向量（vectorless 导入）时 `.lore/kb/` 不落 HNSW 快照 —— 属预期。

### 4. 阻塞项变化

- **B1 ✅ 解除**：G1 落地；`finalization-client.ts` 底层已切 `ipc.invoke`。
- **B2 ✅ 全部解除**：删 KB 文档（F2-3）+ 删实体稿文件（本轮）均真实化。
- **新增 B10**：`fs:grant-read-file` / `write-file` / `mkdir` 仍是批次 H 占位（`commands/external_file_grant.rs:65`）。影响面：**KB 界面导入**、**导出成稿**、**角色卡导入**三条前端路径；且 `kb:import-document` / `kb:import-folder` **无任何 UI 调用点**（唯一 UI 导入入口是 `KnowledgeOverview` 的「导入参考资料」→ `selectPlanningMaterials()`）。

### 5. 验证（本轮实测）

`cargo test --lib` **491/491**（478 → +13）；`cargo check --all-targets` **0 告警**；`pnpm typecheck` / `lint` **exit 0**；
`check:channels` **193 契约 / 152 命令 / 151 覆盖 / 42 未迁移**，orphan 空；定向 vitest **6 文件 / 16 测试全过**。

---

## 本次更新（第三十一次：F2-3 收口 —— kb 命令层 + 外部授权注册表）

> 承接第三十次，同属 **2026-10-09**。实现细节见 `419076db` / `20d26f42` / `0e74971e` 的 commit message。
> 本批**无新依赖**（HNSW / jieba / reqwest / tauri-plugin-dialog 均已在列）。

### 1. 交付（3 个提交）

| Commit | 内容 |
|---|---|
| `419076db` | 基础设施：`external_grant.rs`（内存态外部文件授权注册表）+ `db/kb/vectors.rs`（`KbVectorManager`，按项目/代际懒加载 HNSW）+ `db/kb/store.rs`（SQLite 存储与文本检索编排）+ `AppState` 新字段 |
| `20d26f42` | 命令层：`commands/kb.rs`（15 频道 + 2 dialog）+ `lib.rs` 注册 17 命令 + `chapter_lifecycle` 知识库清理真实化 |
| `0e74971e` | 前端登记：`ipc-client` 参数名 15 条、`migrated-channels` 重生（149）、coverage 断言更新 |

### 2. 语义要点

- **默认对齐基线**：`kb:search*` 向量可用且召回非空 → 短路；否则文本支路。嵌入空间按「指纹 + 维度」匹配；换模型触发 `reindex_required`（旧代际不破坏）。
- **用户 2026-10-09 决定**：文本支路 score 返回**真实词命中度**（基线恒 0.5）；RRF 仍为可选开关（本批命令层未暴露开关，默认关）。
- **距离度量差异**：`LocalVectorIndex` 用 cosine，基线 LanceDB 侧为 `l2`（已在 `commands/kb.rs` 模块文档记录）。
- **外部授权注册表**：`dialog:select-knowledge-*` 签发一次性 grant，`kb:import-{document,folder}` 解析消费；**批次 H 的 `fs:grant-*` 复用同一注册表**（模块头已注明）。
- **章节清理收口**：`chapter:*` 的知识库物理清理由占位改为真实 `removeDocument`（SQLite 事实 + HNSW 向量）；实体稿清理仍占位（批次 H）。相关命令改为 async（Tauri async 命令含引用入参须返回 `Result`）。

### 3. 已知缺口（待后续批次）

- `kb:import-reference-text`：依赖**批次 G**（import-run 权威），先注册但**显式占位失败**。
- 存储预检：仅最小移植（Windows MAX_PATH），基线 `vectors.json` 迁移 barrier 在双栈隔离后无适用路径。
- 向量持久化：HNSW 快照落 `<project>/.lore/kb/index-<generation>.hnsw.*`（需真实项目 GUI 验证）。

### 4. 验证（本轮实测）

`cargo test --lib` **478/478**（F2-3 +20）；`cargo check --all-targets` **0 告警**；`pnpm typecheck` / `lint` **exit 0**；`check:channels` **150 命令 / 149 覆盖 / 42 未迁移**，orphan 空；coverage 测试 6/6 通过。

---

## 本次更新（第三十次：F2-1 分块/FTS5 层 + F2-2 混合编排）

> 承接第二十九次，同属 **2026-10-09**。实现细节见 `99efceaf` / `d0538819` 的 commit message。
> 本批**无新依赖**（`jieba-rs` / `hnsw_rs` / `tokio` / `rusqlite [bundled]` 早于 `4aff3f65` 已声明）。

### 0. 关键决策与发现（本轮）

**读基线源码确认：基线并无融合。** `electron/vector-store.ts::searchWithScope` 实为**二选一短路** ——
向量可用且召回非空则 `return` 纯向量结果（`score = 1/(1+distance)`）；否则降级为 `LIKE '%term%'` 子串扫描，
且**降级分支 `score` 恒为 0.5**（`relevance = Σ(n-index)` 只参与排序、不进结果）。
→ 方案 B 文档 §8.5（核心技术栈列 **RRF 融合**）与 §8.7（以「**+300 行 RRF 混合检索**」为由**不采用**）**自相矛盾**。

**用户决定（2026-10-09）**：`kb:search` 等检索频道 ——
1. **默认忠实对齐基线**（二选一短路）；
2. **RRF 作为可选开关，默认关**；
3. 降级分支**改为返回真实相关性分**（刻意差异，已在 `hybrid.rs` 模块文档记录）。

### 1. F2-1（`99efceaf`）

| 产出 | 要点 |
|---|---|
| `db/schema.rs` | 四条已获批 DDL：`kb_documents` / `kb_chunks` / `kb_embedding_spaces` / `kb_fts`（FTS5 虚拟表，`tokenize='unicode61'`，修正方案 B 草案的 `porter` 笔误） |
| `db/kb/chunks.rs` | `chunkText` 逐字移植：`\n\s*\n` **贪婪回溯**切段、`(?<=[。！？.!?])\s*` **零宽后视**切句、硬切步进 `max(start+1, end-overlap)`。长度口径 = **UTF-16 码元**；`is_js_whitespace` 显式枚举 ECMAScript 集合。**唯一刻意偏离**：切片边界吸附到码点（Rust 无孤立代理项，宁可偏移 ≤1 码元也不产 `U+FFFD`） |
| `db/kb/fts.rs` | jieba 预分词（滤除纯标点 token）+ FTS5 CRUD + `-bm25()` 检索（分数越大越相关）+ `quote_token` 转义 |

### 2. F2-2（`d0538819`）

`db/kb/hybrid.rs`：`SearchMode::{Baseline,Hybrid}` / `baseline_source` / `vector_distance_to_score` /
`extract_query_terms`（`\p{L}\p{N}-` 提取 → 优先 ≥2 码点 → 小写去重 → **上限 8**）/ `text_relevance`（`Σ(n-index)`）/
`reciprocal_rank_fusion`（`k=60`，同分按 id 升序）/ `dedupe_by_file_and_text`；
嵌入空间注册表 `list_spaces` / `active_space` / `next_generation` / `upsert_space` / `activate_generation`（事务化，**只降 `active`、不干扰 `building`**）；
回填计划 `vectorless_chunk_ids`。
另为 `db/vector.rs` 补只读 API `live_doc_ids` / `contains`。

### 3. 验证（本轮实测）

`cargo test --lib` **458/458**（F2-1 +20、F2-2 +15）；`cargo check --all-targets` **0 告警**。
`check:channels` 仍 **133 / 132 / 59**（F2-1/F2-2 不注册频道，符合预期）。

### 4. 遗留

- **F2-3 未开工**：15 个 `kb:*` + 2 个 dialog 频道 + 前端登记。已知依赖缺口：`kb:import-reference-text` → 批次 G（占位失败）；`kb:import-{document,folder}` → 需安全文件系统读取面；存储预检待最小移植；回填需 embedding 调用。
- `.lore` 改名后，**基线项目文件夹不再能被 Lorekeeper 打开**（项目目录刻意不互通）——E 第二部分 GUI 冒烟需用 `.lore` 项目。

---

## 第二十九次（L3 项目目录改名 + F2 知识库 schema 申报，同日）—— 正文

> 承接第二十八次，同属 **2026-10-09**。本轮以 **L3 双栈隔离**为主，兼落 F2 首个交付物（schema 申报书）。
> 实现细节见 `ee40aaab` 的 commit message；本批**无 Schema 变更**（F2 申报书尚未实施）。

### 0. 决策（用户确认）

- **L3 解锁**：原先因「两栈共享 `.vela` 项目目录」而押后；用户明确「两栈项目本就互不相通」，故 **Tauri 项目目录改用 `.lore`**，两栈项目目录**刻意不互通**。
- **F2 schema 获批**：申报书四条 DDL 获批；向量快照目录定为 **`<project>/.lore/kb/`**（原提案 `.vela/lorekeeper-kb/` 被否决后随 L3 定名）。
- **F2 本批范围**：先做 **F2-1**（`chunks` + `fts`）；`kb:import-reference-text` 依赖未迁移的 import-run 域 → 先注册但 **占位失败**（沿用批次 E 诚实化占位先例，待批次 G 收口）。

### 1. L3 改名实现（commit `ee40aaab`，36 文件）

| 层 | 文件 | 改动 |
|---|---|---|
| Rust | `db/mod.rs` | `PROJECT_DIR_NAME` `".vela"` → `".lore"` |
| Rust | `project_access.rs` | `PROJECT_MANIFEST_DIR` → `.lore`；`DIR_PROMPTS_RELATIVE` → `.lore/prompts` |
| Rust | `project_clear_repository.rs` | 回收站 `.vela/trash` → `.lore/trash` |
| Rust | `disk_e2e.rs` | 断言与文档字符串 |
| 前端 | `shared/project-paths.ts` | `DIR_VELA_INTERNAL` → **`DIR_LORE_INTERNAL`**（值 `'.lore'`）；`DIR_PROMPTS = '.lore/prompts'` |
| 前端 | 其余 30 文件 | `prompt-catalog` / `agent/skill-registry` / `writing-skill-bindings` / `read-file.tool` / `architecture.command` / `WorldBuildingEditor` / `ChapterCreationDialog` / i18n 文案 + 相应用例 |

**刻意保留（非项目目录，不得改）**：`vela://` 应用内伪协议、`window.velaAPI` 测试桥、`.vela-editor-content` CSS 类、`app_paths.rs` 中对基线的描述。

### 2. F2 知识库 schema 申报（本轮未实施）

`docs-fork/research/2026-10-09-f2-kb-schema-proposal.md`：四条 DDL（`kb_documents` / `kb_chunks` / `kb_embedding_spaces` / `kb_fts`）+ 向量快照目录 + 三增量切分（F2-1 chunks+fts / F2-2 hybrid / F2-3 命令层 17 频道）。**只申报，未写入 `db/schema.rs`。**

### 3. 事实核实（本轮实测）

- **F2 无需新增依赖**（见上表「依赖」行）。
- **FTS5 可用**：`libsqlite3-sys` bundled 构建带 `-DSQLITE_ENABLE_FTS5`。
- **基线无 SQLite kb 表**：知识库全在 LanceDB + `.vela/*.json`，故 Tauri 侧为**全新设计**（非列集对齐）。

### 4. 验证与自检

`cargo test --lib` **423/423**；`cargo check --all-targets` **0 告警**；`pnpm typecheck` / `lint` **exit 0 / exit 0**；`check:channels` 仍 **133 / 132 / 59**。
**vitest 回归判定**：受影响的 9 个测试文件，失败数 HEAD 与改动后**完全一致（46 failed / 28 passed）** → **零回归**（详见 §5）。

### 5. 遗留

- 既有 vitest 失败根因：node 模式下 `@tauri-apps/api` 的 `invoke` 未被 mock（该批测试需浏览器 runner），**非本轮引入**。
- `kb:import-reference-text` 待批次 G（import-run 域）；`chapter:*` 物理清理待批次 H + F2。

---

## 第二十八次（批次 E 第二部分完成 —— chapter-lifecycle 收口，同日）—— 正文

> **本文件同日含三份内容**：第二十九次（L3 + F2 申报）+ 第二十八次（本正文）+ 第二十七次（批次 E 第一部分，见下方摘要）。
> 实现细节见 `a554f76a` 的 commit message；状态机与基线逐字对齐，423 单测覆盖。
> 承接第二十七次，两次更新同属 **2026-10-09** 一个工作日。

### 1. 新增（本轮）

`repositories/chapter_deletion_repository.rs`（平移基线 `electron/repositories/chapter-deletion-repository.ts`）+
`commands/chapter_lifecycle.rs` —— **4 频道**（真实名以 inventory §4.13 / controller 为准）。
Schema 复用第二十七次落地的 `chapter_deletion_operations` 表，**本轮无 Schema 变更**。

| 命令 | 频道 | 说明 |
|---|---|---|
| `chapter_delete_finalized` | `chapter:delete-finalized` | 幂等 delete + resume |
| `chapter_retry_deletion` | `chapter:retry-deletion` | 未找到 / 授权必需分支逐字对齐 |
| `chapter_confirm_legacy_knowledge_absent` | `chapter:confirm-legacy-knowledge-absent` | 一次性授权 `required → consumed` 后 resume |
| `chapter_list_incomplete_deletions` | `chapter:list-incomplete-deletions` | 读信封 |

**⚠️ 诚实化占位（仿 `dialog:select-export-directory` 先例，非完成态）**：
`resume` 的两个**物理清理投影**当前一律显式返回 `failed`（附可读原因），状态机流转与基线逐字对齐；
SQLite 事实删除**已真实提交**（`committed: true`）。

| 投影 | 阻塞于 | 占位错误码 |
|---|---|---|
| 删实体稿文件 | 批次 H（fs 授权域） | `manuscript_cleanup_unavailable` |
| 删知识库文档 | 批次 F2（kb 能力） | `knowledge_cleanup_unavailable` |

批次 H / F2 落地后替换为真实 cleaner 即恢复完整断点恢复。**在此之前该 4 频道的 delete 操作不会真正清理磁盘稿件与 KB 文档。**

### 2. 前端登记补齐

`ipc-client.ts` +4 条 chapter 频道、并**补登记第二十七次的 13 条**（recovery / continuity / finalization-link / draft 导出与导入 —— 第二十七次快照曾称已登记但实际缺失，由 `channel-migration-coverage` 测试暴露）；`migrated-channels.ts` 重新生成（**生成物**，须随 `lib.rs` 同提交）；`channel-migration-coverage.test.ts` +E 频道断言。

### 3. 行为对照（Electron ↔ Tauri，2026-10-09 补验）

素材为用户真实项目「武林秘事」**副本**（73 定稿章节 + 73 outbox + 73 后处理 run + 1 continuity meta，已删），**原项目零改动**；两侧对副本执行同一操作（删除第 10 章定稿，`draft_id=10`，带 `knowledge_document_id`），Electron 走 `initProjectDatabase` + `ChapterDeletionRepository.begin`、Tauri 走 ignored 测试直连 `&Connection`（临时脚本均已清理）。
结果 **六项指标完全一致**（`status` / `manuscript_status` / `knowledge_status` / `legacy_knowledge_authorization` / `drafts·contents·runs` = `73/74/72` / `continuity` = `10/1`），**仓储层行为与基线逐字节一致，无差异需修复**。
边界：对照**只在仓储层**（命令层为薄壳，状态机已由 423 单测覆盖）；副本 `lorekeeper.db` 为旧 schema，测试前用 `db::schema::create_tables` 幂等补齐。

### 4. 提交清单（第二十八次，均已提交）

| Commit | 类型: 主题词 |
|---|---|
| `a554f76a` | `feat`: 批次 E 章节生命周期子域 |
| `23aaa700` | `fix`: 补登记批次 E 频道参数名 |
| `17ab44ce` | `docs`: 第二十八次快照与指标勘误 |
| `c3378723` | `docs`: 快照勘误与 inventory 更新 |
| `0c2dffc7` | `docs`: 补录基线行为对照结果 |

第二十七次的 5 个提交见下方 §2。

---

## 第二十七次（批次 E 第一部分，同日）—— 摘要

> 详细正文已精简；完整明细见提交 `abfa1698` / `f5fde636` / `00bba449` / `634182d3` / `e59e4fb0` 的 commit message。
> 逐条测试覆盖、基线对照细节见各 commit message 与 [`docs/research/`](../research/) 及 [`docs-fork/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md) §4.13。

**范围**：批次 E 共 **28 个 invoke 频道**，本轮完成前 **24 个**（Schema 层 + 4 子域）。

| 分组 | 频道数 | 基线位置 | 本轮 |
|---|---|---|---|
| continuity | 4 | `finalization-controller.ts` | ✅ |
| recovery-candidate | 4 | `finalization-controller.ts` | ✅ |
| finalization-link | 1 | `finalization-controller.ts` | ✅ |
| draft-import | 1 | `draft-controller.ts` | ✅ |
| draft 权威序列 / 快照 | 3 | `finalization-controller.ts` | ✅ |
| chapter-lifecycle | 4 | `chapter-lifecycle-controller.ts` | ⏭ 第二十八次 |
| finalization（G1 补契约） | 2 | `finalization-controller.ts` | ❌ 未做 |

**Schema 层（`abfa1698`）**：新增 3 表（`recovery_candidates` / `continuity_projection_meta` / `chapter_deletion_operations`）+ 3 索引（`idx_recovery_candidates_pending` / `idx_chapter_deletion_status` / `idx_llm_calls_time`），DDL 对齐 `electron/database.ts` 最终列集（含 migration 补列），幂等测试已过。

**4 子域**（每个含仓储 + 命令 + 测试）：`recovery_candidate_repository.rs`（7 仓储 + 1 命令守卫测试）、`finalized_continuity_repository.rs`（5 测试，含 `invalidate_continuity_projection_from` 失效水位推进）、`finalization_repository.rs` + `finalized_draft_import_repository.rs`（导出权威序列，正文 / 哈希 / outbox 三重一致性校验）、`draft_units.rs`（字数口径用 `encode_utf16().count()` 对齐基线 JS）。

**UUID**：未引入 `uuid` crate，用 `project_access::random_uuid_v4()`。

---

## 交接给下次会话（**从这里接**）

### 1. 当前工作区状态

⚠️ **第三十二次（G1）改动尚未提交**（第二十七 ～ 第三十一次均已提交，上次 HEAD = `0e74971e`）。
待提交文件（`git status --porcelain`）：

```
docs-fork/agents/pi-development.md            (¶2 新增第 5 条 camelCase 规则)
docs-fork/handoffs/2026-10-09-...md           (本快照)
docs-fork/plans/tauri-migration-channel-inventory.md
tauri-app/scripts/verify-channel-coverage.mjs
tauri-app/src-tauri/src/commands/chapter_lifecycle.rs
tauri-app/src-tauri/src/commands/finalization.rs      (新增)
tauri-app/src-tauri/src/commands/mod.rs
tauri-app/src-tauri/src/disk_e2e.rs
tauri-app/src-tauri/src/lib.rs
tauri-app/src-tauri/src/manuscript_publisher.rs       (新增)
tauri-app/src-tauri/src/repositories/chapter_deletion_repository.rs
tauri-app/src-tauri/src/repositories/finalization_repository.rs
tauri-app/src-tauri/src/repositories/finalized_draft_import_repository.rs
tauri-app/src/services/__tests__/finalization-client-session.test.ts
tauri-app/src/services/finalization-client.ts
tauri-app/src/services/finalization-snapshot.ts
tauri-app/src/services/ipc-client.ts
tauri-app/src/shared/ipc-channels.ts
tauri-app/src/shared/migrated-channels.ts
tauri-app/test/channel-migration-coverage.test.ts
tauri-app/test/ipc-arg-struct-contract.test.ts        (新增)
```

**建议提交拆分（3 个，未执行）**：
1. `feat(tauri): 迁移 G1 定稿频道并真实化实体稿发布与清理`（Rust + 前端 + 契约 + 脚本）
2. `fix(tauri): 修复章节删除入参缺 camelCase 导致 GUI 静默失败`（含源码扫描防线）
3. `docs(tauri): 第三十二次快照与上游/契约决策记录`

> 历史备注：第二十七次的 5 个 commit message 里附带过「建议提交拆分（5 主题）」表；实际提交已按该表完成。

### 2. 下一步（1-2-3）

1. **批次 H（当前最高优先）**：`fs:grant-read-file` / `write-file` / `mkdir` 三命令（接入 F2-3 已落地的
   `external_grant.rs` 注册表）—— 可直接解开 **KB 界面导入**、**导出成稿**、**角色卡导入**三条前端路径（B10）；
   顺带 `dialog:select-export-directory`（B3）+ update / mcp / prompt / skills。
2. **批次 G**：import-run 18 频道 + `dialog:select-novel-files`，落地后收口 `kb:import-reference-text`。
3. **上游合并专项**（本轮已评估、**未执行**）：上游领先 502 提交（`src/` +39k/−10.8k、`electron/` +47k、`docs/` +81k），
   但**共享目录零冲突**（本 fork 从未改过 `src/`/`electron/`/`docs/`），且已迁移 149 频道**零删除/零改名**；
   冲突面仅根目录 `.github` / `.gitignore` / `README.md` / `scripts/check-commit-msg.mjs`。
   合并后需重建盘点（191 → ~205 invoke）并评估仓库层语义漂移。

### 3. 阻塞项与待授权项（不得删除，须逐条确认后更新）

| # | 项 | 状态 |
|---|---|---|
| B1 | `finalization:commit` / `finalization:retry`（G1） | ✅ **已完成（第三十二次）**：命令 + `manuscript_publisher.rs` 落地；`finalization-client.ts` 底层已切 `ipc.invoke`；契约补在 tauri-app 副本 |
| B2 | `chapter:*` 物理清理（删稿件 / 删 KB 文档） | ✅ **全部解除**：删 KB 文档（F2-3）+ 删实体稿文件（第三十二次，`manuscript_publisher::remove_published_manuscript`）；GUI 双章删除已验（含 legacy 授权与 KB 清理两分支） |
| B3 | `dialog:select-export-directory` | ⛔ 仍返回 `None`，阻塞于批次 H 的 grant 域 |
| B4 | L3 双栈隔离（`.vela` 改名） | ✅ **已执行**（`ee40aaab`） |
| B5 | `cargo fmt --check` | ⚠️ 未纳入验收（`src-tauri/` 全域存在既有 rustfmt 差异） |
| B6 | `tauri-app` 全量 `pnpm test` | ⚠️ 暂超时（见 10-08 快照遗留项）；另：凡未 mock `ipc-client` 的流程测试在 node 下必因未 mock 的 `@tauri-apps/api` invoke 失败（**既有**，非本轮） |
| B7 | 「证据不在正文中」反例 | ⚠️ 未验证（无正文数据） |
| B8 | 2 个既有 `vitest` 失败（`ipc-client-project-session.test.ts`） | ⚠️ 阶段 0 起就失效，**非本轮引入**（10-08 快照 §6） |
| B9 | `tauri-plugin-dialog 2.8.1` 要求 rustc ≥ 1.90 | ⚠️ CI 最低版本需相应抬高 |
| B10 | **`fs:grant-*` 三命令仍为批次 H 占位** | ⛔ `commands/external_file_grant.rs:65` → **KB 界面导入 / 导出成稿 / 角色卡导入全部不可用**；`kb:import-document`/`folder` 无 UI 调用点（2026-10-09 GUI 冒烟发现，推荐随批次 H 解决） |
| B11 | **`chapter:delete-finalized` 的 `request` 入参曾缺 camelCase** | ✅ **已修复（第三十二次）** + Rust 契约测试 + 全仓源码扫描防线；规则写入 `pi-development.md` §2.5 |

### 4. 红线提醒（每次接手都要过一遍）

- 🚫 Tauri 侧**禁止**读 `AI_NOVEL_VELA_HOME`、**禁止**回退 `~/.vela`、**禁止**写 `.vela/vela.db`。
- 项目库 = `<root>/.lore/lorekeeper.db`；KB 向量 = `<root>/.lore/kb/`；全局数据根 = `AI_NOVEL_LOREKEEPER_HOME` 或 `~/.lorekeeper`。
- 失败文案按基线 MUTATING 规则带 `"Error: "` 前缀（`commands/db.rs::mutating_error`）。
- ⚠️ **F2 隔离红线**（2026-10-08 快照 §2）：基线 LanceDB 落在 `.vela/lancedb/`、`.vela/<registry>.json`、
  `.vela/vectors.json`，Tauri 侧向量路径须 Tauri 专属。**✅ 已解除（`ee40aaab` L3）**：Tauri 项目目录改为 `.lore/`，
  向量快照定为 `.lore/kb/`，不再有互覆可能。
- 定稿不可逆：`finalization:` 相关改动一律 **Ask first**。
- 提交消息**无 BOM / 无 CRLF / 无行尾空白**（2026-10-08 出过 BOM 事故，见 `pi-development.md` §10.1）。

---

## 5. 自检记录（2026-10-09 实测，第三十二次）

> 生成本表快照时在本机实跑，命令与输出如下。**下次更新快照表必须先重跑这些命令。**

| 命令 | 工作目录 | 实测输出 |
|---|---|---|
| `pnpm run check:channels` | `tauri-app/` | 契约 invoke 频道 **193**（事件频道 4）· 已注册命令 **152** → 覆盖 **151** · 未迁移 **42** `[db=19 mcp=9 update=6 skills=4 prompt=3 dialog=1]` · 命令名与契约频道一一对应 ✅ |
| `cargo test --lib` | `tauri-app/src-tauri/` | `test result: ok. 491 passed; 0 failed; 0 ignored` |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | `Finished dev profile ... in 5.62s`（0 告警） |
| `pnpm typecheck` / `lint` | `tauri-app/` | exit 0 / exit 0 |
| `npx vitest run`（6 文件定向） | `tauri-app/` | `Test Files 6 passed`，`Tests 16 passed`（契约覆盖 / 入参结构体契约 / 源码契约 / locale / finalization-client / finalization-snapshot） |
| 磁盘级 E2E | `cargo test --lib` 内 | `disk_e2e::real_project_finalize_publish_then_delete_removes_manuscript_test` ✅（真实目录 + WAL 库 + 真实 `.txt` 删除、陪跑文件保留、幂等、重开持久） |
| GUI 冒烟 | `pnpm tauri dev` | vite `533 ms` + cargo `1.33s` → `lorekeeper.exe`；G1 两章 `published` + `.txt` 落盘；B2 两章真实删除。详见本快照第三十二次 §3 |