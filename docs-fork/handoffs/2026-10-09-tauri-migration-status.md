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

## 快照（最后更新：2026-10-09 · 第二十八次）

> 本表只填**最新一次自检的实测值**。改表前必须重跑对应命令，不得沿用旧数字、不得估算。
> 本轮实测命令与输出见下方「[§5 自检记录](#5-自检记录2026-10-09-实测)」。

| 项 | 值 |
|---|---|
| 仓库 / 分支 | **`TanCan1998/Lorekeeper`**（`EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork）· `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 已注册命令 | **133** |
| 覆盖 invoke 频道 | **132**（契约总数 191，事件频道 4） |
| 未迁移 invoke 频道 | **59**（`db=19 kb=15 mcp=9 update=6 skills=4 dialog=3 prompt=3`） |
| orphan | **空** ✅ |
| `cargo test --lib` | **423/423** ✅ |
| `cargo check --all-targets` | **0 告警** ✅ |
| `pnpm typecheck` / `lint` | exit 0 / exit 0 ✅ |
| 定向 `vitest` | **70/72**（2 个失败为阶段 0 起就失效的既有测试，见 10-08 快照 §6） |
| 已完成批次 | A ✅ / B ✅（含遗留补齐） / C ✅ / D1 ✅ / D2-a ✅ / D2-b ✅ / D2-c ✅ / **E ✅（两部分全部完成）** / **F1 ✅** |
| 当前阶段 | **批次 E 全部完成**（28 频道）。下一里程碑为 **F2**（已评估定案，待批准实施） |
| 依赖 | `reqwest 0.13`（`default-features=false` + `native-tls` + `socks`）、`tauri-plugin-dialog 2`（Cargo.lock 锁 **2.8.1**）。**E / F1 均未新增依赖**；`futures-util` / `tokio` 未引入。F2 预计新增 `jieba-rs 0.7` + HNSW 相关 crate（**开工前必须 `cargo tree` 实测**） |
| GUI 冒烟 | ✅ 自 2026-10-07 起 **七轮**（末轮 2026-10-08）。近三轮记录：F1 轮 `pnpm tauri dev` 编译 **35.19s** 功能正常；E 轮 `cargo test --lib` **418/418** 无 panic；dialog 轮 vite `453 ms` + cargo `24.91s`。渲染层 `ipc-client` 已联通。**⚠️ E 第二部分的 GUI 冒烟未做**（chapter-lifecycle 依赖含正文项目） |
| 双栈隔离 | L0 安装标识 / L1 `~/.lorekeeper` / L2 `<root>/.vela/lorekeeper.db` 均独立；**L3（`.vela` 改名）押后** |
| Rust 工具链 | rustc/cargo **1.99.0 stable-msvc** @ `D:\Environment\rust\`（脚本内须显式设 `RUSTUP_HOME` / `CARGO_HOME`）。`tauri-plugin-dialog 2.8.1` 要求 **rustc ≥ 1.90**（CI 最低版本需相应抬高） |

<sub>*命令与频道差额：**133 命令**中 1 个为骨架（`dialog:select-export-directory`），不产生独立频道覆盖；其余 **132** 与 invoke 频道一一对应。</sub>

---

## 本次更新（第二十八次：批次 E 第二部分完成 —— chapter-lifecycle 收口）

> **本文件同日含两份内容**：第二十八次（本正文）+ 第二十七次（批次 E 第一部分，见下方摘要）。
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

**`master` 上工作区干净**（`git status --porcelain` 无输出）；第二十七 / 二十八次改动**均已提交**，
HEAD = `0c2dffc7`。第二十七次快照里「5 个已修改 + 4 个新增文件尚未提交」的描述**已过期**，勿再照它操作。

> 历史备注：第二十七次的 5 个 commit message 里附带过「建议提交拆分（5 主题）」表；实际提交已按该表完成，**无需再拆**。

### 2. 下一步（1-2-3）

1. **补 E 第二部分 GUI 冒烟**（唯一未做的验收项）：需要**含正文的真实项目**，跑 `pnpm tauri dev` 验证
   `chapter:delete-finalized` → `chapter:list-incomplete-deletions` → `chapter:retry-deletion` 流程，
   并确认两处 `*_cleanup_unavailable` 占位返回**可读失败**而非静默成功。
2. **G1 补契约（`finalization:commit` / `finalization:retry`）** —— 属**定稿不可逆核心**，
   **Ask first：需用户批准**后方可平移 `electron/services/finalization-service.ts`。这是批次 E 唯一剩余缺口。
3. **批次 F2**（方案 B 已评估定案，**待用户批准实施**）：`kb:*` 15 频道 + dialog 2；
   实施计划见 2026-10-08 快照 §8。**开工前必做两件事**：① `cargo tree` 实测新增依赖；
   ② 解决下述 L2 向量存储隔离红线。落地后把 `knowledge_cleanup_unavailable` 占位替换为真实 `removeDocument`。

### 3. 阻塞项与待授权项（不得删除，须逐条确认后更新）

| # | 项 | 状态 |
|---|---|---|
| B1 | `finalization:commit` / `finalization:retry`（G1） | ⛔ **阻塞于用户授权**（定稿不可逆核心） |
| B2 | `chapter:*` 物理清理（删稿件 / 删 KB 文档） | ⛔ **阻塞于批次 H + F2**，当前为显式占位 |
| B3 | `dialog:select-export-directory` | ⛔ 仍返回 `None`，阻塞于批次 H 的 grant 域 |
| B4 | L3 双栈隔离（`.vela` 改名） | ⏸️ 主动押后 |
| B5 | `cargo fmt --check` | ⚠️ 未纳入验收（`src-tauri/` 全域存在既有 rustfmt 差异） |
| B6 | `tauri-app` 全量 `pnpm test` | ⚠️ 暂超时（见 10-08 快照遗留项） |
| B7 | 「证据不在正文中」反例 | ⚠️ 未验证（无正文数据） |
| B8 | 2 个既有 `vitest` 失败 | ⚠️ 阶段 0 起就失效，**非本轮引入**（10-08 快照 §6） |
| B9 | `tauri-plugin-dialog 2.8.1` 要求 rustc ≥ 1.90 | ⚠️ CI 最低版本需相应抬高 |

### 4. 红线提醒（每次接手都要过一遍）

- 🚫 Tauri 侧**禁止**读 `AI_NOVEL_VELA_HOME`、**禁止**回退 `~/.vela`、**禁止**写 `.vela/vela.db`。
- 项目库 = `<root>/.vela/lorekeeper.db`；全局数据根 = `AI_NOVEL_LOREKEEPER_HOME` 或 `~/.lorekeeper`。
- 失败文案按基线 MUTATING 规则带 `"Error: "` 前缀（`commands/db.rs::mutating_error`）。
- ⚠️ **F2 新增隔离红线**：基线 LanceDB 落在共享的 `<project>/.vela/lancedb/`、`.vela/<registry>.json`、
  `.vela/vectors.json`；**Tauri 侧向量存储路径必须 Tauri 专属，不得复用**（详见 2026-10-08 快照 §2）。
- 定稿不可逆：`finalization:` 相关改动一律 **Ask first**。
- 提交消息**无 BOM / 无 CRLF / 无行尾空白**（2026-10-08 出过 BOM 事故，见 `pi-development.md` §10.1）。

---

## 5. 自检记录（2026-10-09 实测）

> 生成本表快照时在本机实跑，命令与输出如下。**下次更新快照表必须先重跑这些命令。**

| 命令 | 工作目录 | 实测输出 |
|---|---|---|
| `pnpm run check:channels` | `tauri-app/` | 契约 invoke 频道 **191**（事件频道 4）· 已注册命令 **133** → 覆盖 **132** · 未迁移 **59** `[db=19 kb=15 mcp=9 update=6 skills=4 dialog=3 prompt=3]` · 命令名与契约频道一一对应 ✅ |
| `cargo test --lib` | `tauri-app/src-tauri/` | `test result: ok. 423 passed; 0 failed; 0 ignored` |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | `Finished dev profile ... in 4.21s`（0 告警） |
| `git status --porcelain` | 仓库根 | 空（工作区干净） |
| `git log -1` | 仓库根 | `0c2dffc7 docs(tauri): 补录 chapter-lifecycle Electron 基线行为对照结果（六项指标完全一致）` |