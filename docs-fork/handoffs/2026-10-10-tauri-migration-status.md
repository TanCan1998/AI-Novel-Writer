# Tauri 迁移进度快照（2026-10-10）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> **规则**：按日期命名，一个工作日一个新文件；当日新增只写当日文件，跨日不回填旧文件；
> 旧文件冻结后不允许修改（见 [`docs-fork/agents/pi-development.md`](../agents/pi-development.md) §9）。
> **模板**：[`_TEMPLATE-tauri-migration-status.md`](./_TEMPLATE-tauri-migration-status.md)。
>
> 全部历史快照见 `docs-fork/handoffs/` 目录（按日期命名）。
>
> - channel 级盘点：[`docs-fork/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-10 · 第四十次）

> 本表只填**最新一次自检的实测值**。改表前必须重跑对应命令，不得沿用旧数字、不得估算。
> 本轮实测命令与输出见下方「[§5 自检记录](#5-自检记录2026-10-10-实测)」。

| 项 | 值 |
|---|---|
| 仓库 / 分支 | **`TanCan1998/Lorekeeper`**（`EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork）· `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 已注册命令 | **185** |
| 覆盖 invoke 频道 | **184**（契约总数 193，事件频道 4） |
| 未迁移 invoke 频道 | **9**（`db=0 mcp=9`） |
| orphan | **空** ✅ |
| `cargo test --lib` | **645/645** ✅ |
| `cargo fmt --check` | **干净（0 差异）** ✅ |
| `cargo check --all-targets` | **0 告警** ✅ |
| `pnpm typecheck` / `lint` | exit 0 / exit 0 ✅ |
| 定向 `vitest` | **7/7**（2 文件：契约覆盖 / 入参结构体契约）✅ |
| 已完成批次 | A ✅ / B ✅ / C ✅ / D1 ✅ / D2-a ✅ / D2-b ✅ / D2-c ✅ / **E ✅** / **F1 ✅** / **L3 ✅** / **F2 ✅** / **批次 E G1 ✅** / **H1 ✅** / **H2 ✅** / **H3 ✅** / **B12 ✅** / **批次 G 的 G1 ✅** / **批次 G2a ✅（本次）** / **批次 G2b ✅（1–6 步全部完成）** / **批次 G3a ✅** / **批次 G3b ✅（本次）** |
| 当前阶段 | **批次 G4 代码已收口**（`kb:import-reference-text` 去占位 + 前端登记，提交 `94a1fe6a` + `5c4fd26e`；未迁移仍为 **9**，仅剩 mcp 9）——G3b 遗留的 13 频道状态机（执行租约 5 / 批次推进 4 / effect receipts 3 / 全局事实提交 1）亦已落地。**⚠️ G4 的 GUI 冒烟（导入参照章节链路）待做**（用户决定稍后）→ 之后仅剩 H4（mcp 9，暂缓）。其它待办：B13（导航防护）、B14（真 Windows 自更新）、上游合并专项、B24（整屏瞬黑，暂缓） |
| 依赖 | `reqwest 0.13`（`default-features=false` + `native-tls` + `socks`）、`tauri-plugin-dialog 2`（锁 **2.8.1**）、`tauri-plugin-opener 2.7.0`、`windows-sys 0.61`（`[target.'cfg(windows)'.dependencies]`，仅 lock 提级，**0 新下载**；Ask first 已批准 2026-10-10）。**G2a 零新依赖**，**G3a 零新依赖**（仓储层平移，无 Cargo.toml 改动），**G3b 零新依赖**（命令层平移，无 Cargo.toml 改动）。向量层 `hnsw_rs 0.3.4` / `jieba-rs 0.7.0` / `tokio`；FTS5 由 `libsqlite3-sys` bundled 提供 |
| GUI 冒烟 | ✅ 自 2026-10-07 起 **十三轮**。⏳ **第十四轮（G4 参照章节链路）待做**：① 作者原稿全链路回归；② 参考语料选 `.md` → 应得 `preparation` 而非诚实错误，重选应得 `exact-duplicate`；③ `importReference` 成功写库并记「参照章节 N 已进入知识库」（重复显示「（已存在）」）。**第十三轮（2026-10-10，弹窗动画统一）**：设置弹窗明显变快（修复前实为“静置 400ms + 播 220ms”）、四类弹窗进出场一致、Radix 弹窗仍居中且尺寸正常 ✅。**第十二轮（2026-10-10，冒烟发现的三项缺陷修复）**：① 弹窗 **ESC 关闭**（根因：Radix `DismissableLayer` 仅在 `index === layers.length-1` 时注册 ESC，而 Radix 关闭后仍保留 `DialogContent` 挂载——实测 `layers.length=7`，可见弹窗永远不是最高层）；② **窗口命令真实化**（批次 A 四个命令原为假成功骨架）；③ **标题栏拖拽**（`-webkit-app-region` 在 WebView2 无效 → 补 `data-tauri-drag-region`）。4 项人工验证全部 ✅。近三轮：第十一轮（G2a）、第十轮（G1）、第九轮（H 前三项 + B12） |
| 双栈隔离 | **L0/L1/L2/L3 全部独立**：安装标识 / `~/.lorekeeper` / `<root>/.lore/`（库 `.lore/lorekeeper.db`、KB 向量 `.lore/kb/`）。基线为 `~/.vela` / `<root>/.vela/`。**两栈项目目录刻意不互通**（`ee40aaab`） |
| Rust 工具链 | rustc/cargo **1.99.0 stable-msvc** @ `D:\Environment\rust\`（脚本内须显式设 `RUSTUP_HOME` / `CARGO_HOME`）。`tauri-plugin-dialog 2.8.1` 要求 **rustc ≥ 1.90**（CI 最低版本需相应抬高） |

<sub>*命令与频道差额：**185 命令**中 1 个为阶段 0 骨架（`app_health_check`，不对应任何契约频道），其余 **184** 与 invoke 频道一一对应。</sub>

---

## 本次更新（第四十次：批次 G3b 收口）

### 1. G3b ✅ 频道注册（13 频道，未迁移 22 → 9）

提交 `f31446ec`（5 文件，+770/−3）按开工清单 §4
完成命令层注册。仓储层（G3a，`c8c56ad9`）提供的全部
seam 在本层接上 IPC：

- **执行租约 5 频道**：`db:import-run-start-resume` /
  `renew-execution` / `restart` / `request-cancel` /
  `cancel-at-boundary`——`start_or_resume` / `renew_execution`
  传 `now_ms()` + `DEFAULT_EXECUTION_LEASE_MS`（15 min）；
  `restart` 传 `now_ms()`；`request_cancel` /
  `cancel_at_boundary` 无时间参数（仓储内部取当前时间）
- **批次推进 4 频道**：`complete-batch` / `advance-stage` /
  `fail` / `complete`——仓储内部取当前时间，命令层不传
- **effect receipts 3 频道**：`effect-receipt-get`（**读频道**，
  失败直接 reject，无 `{success}` 信封，对齐基线 handler）/
  `effect-receipt-prepare`（传 `now_ms()`）/
  `effect-receipt-commit`（`project_root` 取
  `expected_project_path`，对齐基线仓储内部
  `getCurrentProjectPath()`；跨仓原子事务）
- **全局事实 1 频道**：`db:import-global-facts-commit`——
  `ImportGlobalFactsRepository::commit`（事务：幂等重放或
  核心台账 + roster 提交）
- **D4 落地**：`completeBatch` 的 direct-stage 前置校验
  （`is_import_run_direct_checkpoint_stage`：仅 `knowledge` /
  `author-publish` / `author-postprocess` / `refresh`）在
  命令层实现并先于仓储调用，对齐基线 db-controller handler
  前置断言；`'parsing'` 等非 direct stage 返回
  `该导入阶段不接受直接 checkpoint`（经 `mutating_error` 包装）
- **命令层自算时间**：仓储 `now_ms()` 为私有，命令层自带
  `import_run_now_ms()` helper（SystemTime 毫秒）
- **信封结构**：7 个 camelCase `#[serde(rename_all =
  "camelCase")]` 响应结构体（`ImportRunStartResumeResult` /
  `ImportRunRenewExecutionResult` / `ImportRunRunMutationResult`
  / `ImportRunCompleteBatchResult`（含 `newlyCompleted` /
  `cancelApplied`）/ `ImportRunEffectReceiptPrepareResult` /
  `ImportRunEffectReceiptCommitResult` / `ImportGlobalFactsCommitResult`），
  `Option` 字段全部 `skip_serializing_if`，与契约返回结构一致
- **每命令** `command + _inner` 双函数模式：命令函数供
  `invoke_handler` 注册（`map_err(mutating_error)`，写面带
  `Error: ` 前缀），`_inner` 供测试直调（错误无前缀）
- **前端门禁**：`ipc-client.ts` 的 `CHANNEL_ARG_NAMES` 登记
  13 频道参数名（如 `complete-batch` =
  `['runId','stage','batchId','execution','expectedProjectPath']`）；
  `migrated-channels.ts` 生成物经 `check:channels:emit` 重新
  生成（184 频道）；`channel-migration-coverage.test.ts` 的
  迁移状态断言同步更新（G3 频道由「仍未迁移」改为已迁移）
- **新增命令层测试 2 例**：D4 拒绝（非 direct stage +
  MUTATING 包装断言 + 缺会话拒绝）、start-resume 签发租约 →
  running + 同一执行器 renew 顺延 + 缺会话拒绝（经 reference
  通道 `prepare-inspection` 落地 'prepared' 可恢复运行）

**GUI 冒烟**：本轮无新增冒烟轮次——13 频道均为导入执行器
内部状态机 seam，前端调用方（导入执行 UI）属后续批次，当前无
用户可触达路径；覆盖由单元测试（仓储 36 例 + 命令层 2 例）
与频道契约测试保证。

### 2. 前置：G3a ✅ 仓储层（已收口，提交 `c8c56ad9`）

（第四十次快照 G3a 章节原文保留——13 频道的全部仓储 seam，
8 文件 +5202/−861，移植测试 36 例。）

### 3. G3a 仓储层细节（13 频道的全部仓储 seam）

提交 `c8c56ad9`（8 文件，+5202/−861）平移基线
`electron/repositories/import-run-repository.ts`（2551 行）与
`import-global-facts-repository.ts`，落地开工清单 §3 全部内容：

- **执行租约族**：`assert_execution_authority` / `assert_execution` /
  `start_or_resume` / `renew_execution` / `restart` / `request_cancel` /
  `cancel_at_boundary`（`DEFAULT_EXECUTION_LEASE_MS = 15 * 60_000` 对齐）
- **批次推进族**：`next_stage_for_run` / `author_checkpoint_chapter_number` /
  `assert_checkpoint_can_apply` / `assert_stage_checkpoint_complete` /
  `apply_batch_checkpoint` / `complete_batch` / `advance_stage` / `fail` / `complete`
- **effect receipts**：`canonicalize` / `canonical_payload` / `exact_keys` /
  `assert_effect_payload_schema` / `assert_effect_payload_binding` /
  `assert_author_effect_run_binding` / `assert_committed_effect_schema` /
  `assert_completed_blueprint_sync_operation` / `assert_blueprint_effect_authority` /
  `assert_committed_effect_authority` / `validate_effect_stage` /
  `row_to_effect_receipt` / `get_effect_receipt` / `prepare_effect_receipt` /
  `commit_effect_receipt`（跨仓原子事务，复用下游 `_in_transaction` 抽取件；
  `MAX_EFFECT_RECEIPT_PAYLOAD_BYTES = 16 MiB`）
- **新模块** `import_global_facts_repository`：`normalized_request`（13 文本字段
  trim + 枚举校验 + 总章数/章节字数校验）/ `hash_request` / `core_snapshot` /
  `parse_receipt` / `ensure_ledger`（D2 懒建表）/ `get_committed_operation`
  （核心台账 + roster factHash 比对，拒绝历史操作冒充当前事实）/ `commit`
  （事务：幂等重放或 `ProjectCoreRepository.update` + `CharacterRosterRepository.commit`）
- **下游读回 helper**：`blueprint_repository::get_committed_range_operation`、
  `finalized_draft_import_repository::get_committed_operation`、
  `character_roster_repository` 事务体抽取
- **会话边界围栏**：`create_tables` 末尾 `fence_import_run_execution_leases`
  （对齐基线 `createTables` 的「打开项目即围栏上一会话租约」迁移）

**测试**：移植基线三个测试文件，净增 **36 例**（execution-lease 11 /
state-machine 5 / receipt 12 例 22 用例，含 committed 回放、离线伪造拒绝、
失败关闭、键-载荷绑定、过期租约围栏、全局事实台账哈希校验）。

### 4. 刻意偏离（D1–D4，见开工清单 §2）

`adoptLegacyCompletedRun` 不移植（双栈隔离下 `.lore` 无 legacy 运行）；全局事实
台账懒建表（不改 `db/schema.rs`，G schema 刻意固定 9 表）；`canonicalize` 键排序
用字节序（哈希仅 Tauri 内部自洽）；`completeBatch` 的 direct-stage 前置校验
（`isImportRunDirectCheckpointStage`）留在命令层 G3b（对齐基线 db-controller
handler 前置断言，D4）。恢复 D1–D3 需用户确认。

### 5. 自检（本轮，G3b）

`cargo test --lib` **645/645**（G3a +36 后再 +2 命令层）·
`cargo check --all-targets` **0 告警** · `cargo fmt --check` 干净 ·
`pnpm typecheck` / `lint` exit 0 · 定向 `vitest` **7/7** ·
`check:channels` 193 契约 / **185 命令 / 184 覆盖 / 未迁移 9**
（仅剩 `mcp=9`）· 提交消息检查 + pre-commit gitleaks 无命中。

（G3a 轮自检：643/643、193/172/171/22，无频道变化——纯仓储层。）

---

## 交接给下次会话（**从这里接**）

### 1. 当前工作区状态

**跟踪文件干净**（唯一未跟踪项为 `docs-fork/todo.md`，按约定永不入库）；`docs-fork/research/2026-10-10-b24-black-flash-investigation.md`（B24 暂缓中的排查记录）已补提交 `2f28d4a9` 入库。G3a / G3b / G4 均以单提交落地：

| Commit | 说明 |
|---|---|
| `b4530e50` | `fix(tauri): ClearProjectDataDialog 纳入统一弹窗动画（B25 收口）` |
| `1e0c1ce3` | `feat(tauri): G2b-6 复活 dialog:select-novel-files 的 reference 分支` |
| `237f838a` | `feat(tauri): G2b-5 导入运行两频道（prepare-inspection / finalize-parsing）` |
| `720337db` | `feat(tauri): G2b-4 prepare 两分支（author / reference）` |
| `adb0e74e` | `feat(tauri): G2b-3 finalizeParsing 四态分类与准备检视` |
| `ad7a77ea` | `fix(tauri): 导入弹窗重开或切换项目时复位会话状态` |
| `a91d9f5c` | `chore(tauri): 对齐浏览器测试依赖并补浏览器测试配置` |
| `c8c56ad9` | `feat(tauri): G3a 导入运行执行租约/批次推进/effect receipts 仓储层` |
| `461c087c` | `docs(tauri): 第四十次快照（批次 G3a 收口）` |
| `f31446ec` | `feat(tauri): G3b 导入运行执行租约/批次推进/effect receipts（13 频道注册）` |
| `40123891` | `docs(tauri): 扩写第四十次快照（批次 G3b 收口）` |
| `94a1fe6a` | `feat(tauri): G4-1 参照文档幂等存储层（哈希/完整性/stable-id 重写）` |
| `5c4fd26e` | `feat(tauri): G4-2 kb:import-reference-text 去占位真实化` |
| `7eeea498` | `docs(tauri): G4 收口登记与刻意偏离 D-G4-1` |
| `c0a0c5c2` | `docs(fork): 子代理编排验证开工清单（PiDeck 子代理面板）` |
| `2f28d4a9` | `docs(fork): B24 整屏瞬黑排查记录` |

- HEAD（写入本行时）：`2f28d4a9 docs(fork): B24 整屏瞬黑排查记录`
- **推送状态（2026-10-10）**：`origin/master` 曾落后 10 个提交（末个远端为 `03c9f230`），本轮补提交后**已全部推送**（`03c9f230..2f28d4a9`，`ahead 0 / behind 0`）。
- ⚠️ 旧快照中**已过期的交接描述**（防照旧操作）：
  1. 第三十九次「`cargo test --lib` **607/607**」→ G3a 移植 36 例后为 643，G3b 再 +2 命令层测试后实测为 **645/645**；
  2. 第三十九次「下一步 G3（执行租约 / 批次推进 / effect receipts 11 频道 + `db:import-global-facts-commit`）」→ **G3a（仓储层）与 G3b（频道注册）均已完成**，下一步是 **G4**；
  3. 第四十次 G3a 版「下一步 **G3b**（13 频道注册）」→ **G3b 已完成**（提交 `f31446ec`，未迁移 22 → 9）；
  4. 第三十九次快照的 §5 自检已冻结，**不得据其回填**本份数字。

### 2. 下一步（1-2-3）

1. **批次 G4 ✅ 代码已完成**（2026-10-10，提交 `94a1fe6a` + `5c4fd26e`）：`kb:import-reference-text` 去占位真实化（G4-1 存储层 `db/kb/store.rs` 参照文档幂等 seam + G4-2 命令层 `commands/kb.rs`）；为占位频道真实化，未改变未迁移计数（仍为 9，仅剩 mcp）。**⚠️ G4 的 GUI 冒烟（导入参照章节链路）仍未做**（用户决定稍后）：清单为 ① 作者原稿全链路回归；② 参考语料选 `.md` → 应得 `preparation`（classification + 预览）而非诚实错误，同一文件重选应得 `exact-duplicate`；③ 工作流跑到 `importReference` 时应成功写库并打日志「参照章节 N 已进入知识库」，重复运行显示「（已存在）」。
2. **子代理编排通道已验**（开工清单 [`docs-fork/plans/2026-10-10-subagent-pideck-kickoff.md`](../plans/2026-10-10-subagent-pideck-kickoff.md) §8）：`lorekeeper-task` + `bash` 端到端可用（子代理返回 `c0a0c5c2` 与编排者一致）；PiDeck 面板出条目已确认；`toolUses`/`tokens` 实时跳动**未取得界面证据**。
3. **其它待办**：B13（导航防护）、B14（真 Windows 自更新）、H4（mcp，暂缓）、上游合并专项、B24（整屏瞬黑，暂缓）。

**刻意偏离登记（批次 G4）**：**D-G4-1** —— `tauri-app/src-tauri/src/db/kb/store.rs::document_integrity` 的 `complete` **只反映 canonical 完整性**（文档行唯一 + 块序列严格 `0..n` + `total_chunks`/`corpus_kind` 自洽），**不含基线的 `embeddingGenerations` 检查**（基线 `electron/vector-store.ts:1464-1470`：`complete = canonicalComplete && embeddingGenerations.every(g => g.status === 'building' || g.complete)`）。原因：Tauri 侧向量由文件型 `LocalVectorIndex` 持有、不入 SQLite，无法在 SQL 层复现代际检查。恢复需用户确认。

### 3. 阻塞项与待授权项（**不得删除，须逐条确认后更新**）

| # | 项 | 状态 |
|---|---|---|
| B1 | 批次 E 定稿频道 | ✅ 完成（第三十二次） |
| B2 | `chapter:*` 物理清理 | ✅ 全部解除（F2-3 删 KB 文档 + 第三十二次删实体稿） |
| B3 | `dialog:select-export-directory` | ✅ 已真实化（第三十三次 H1） |
| B4 | L3 双栈隔离 | ✅ 已执行 |
| B5 | `cargo fmt --check` | ✅ 已解除；⚠️ **未加入 CI**，改 CI 仍需 Ask first |
| B6 | `tauri-app` 全量 `pnpm test` | ⚠️ 仍超时；凡未 mock `ipc-client`/未注入 `__TAURI_INTERNALS__` 的流程测试在 node 下会失败（既有，非本轮） |
| B7 | 「证据不在正文中」反例 | ⚠️ 未验证（无正文数据） |
| B8 | 2 个既有 `vitest` 失败 | ✅ 已解除（第三十三次） |
| B9 | `tauri-plugin-dialog 2.8.1` 要求 rustc ≥ 1.90 | ⚠️ CI 最低版本需相应抬高 |
| B10 | `fs:grant-*` 占位阻塞 KB 导入/导出 | ✅ 已解除（第三十三次 H1） |
| B11 | `chapter:delete-finalized` 入参缺 camelCase | ✅ 已修复 + 源码扫描防线 |
| B12 | 「打开外部链接」缺失 | ✅ 已解除（第三十三次） |
| B13 | 渲染层导航防护未接入 | ⚠️ 基线有 `preventRendererNavigation` + 新窗口拦截；Tauri 侧未接入 |
| B14 | 真正的 Windows 自动更新 | ⚠️ 需 `tauri-plugin-updater` + 签名公钥 + 打包链路 |
| B15 | H4（mcp 9 频道） | ⚠️ 用户决定暂缓；方案已评估（仅 stdio，`std::process` + 自研守卫） |
| B16 | `db/vector.rs` HNSW 墓碑测试偶发失败 | ✅ 已修（第三十四次） |
| B17 | 批次 G schema | ✅ 已获批并落地 9 张表（`cca792cd`） |
| **B18** | **`.epub` 导入依赖** | ⚠️ **新增（本次）**：需 `zip` 类 crate 解包，属 Ask first；G1 返回 D4 诚实错误，对话框仍列出 epub |
| **B19** | **`reference`（参考语料）路径** | ✅ **已解除**（G2b-6 `1e0c1ce3`：`dialog:select-novel-files` 的 reference 分支复活，选择期即落地解析运行） |
| **B20** | **D7 zh-CN 排序不等价** | ⚠️ **新增（本次）**：无 ICU 依赖，来源文件名排序用数字感知自然序近似；如需逐字对齐须 Ask first 引 ICU |
| **B21** | **G1 GUI 冒烟** | ✅ **已解除（本次）**：第十轮冒烟 3 项全部通过（作者原稿预览 / reference 诚实错误 / epub 诚实错误），dev 日志无 error/panic |
| **B22** | **两段式关窗的未保存内容确认未验证** | ⚠️ **新增（本次）**：`window:close` 现在会拦截并广播 `window:close-requested`，渲染层无 dirty 时直接 `proceed`；**dirty 分支（确认框 + cancel / 再次关窗）尚未实测**，需构造未保存内容后再验 |
| **B23** | **手写弹层无退出动画** | ✅ **已解除（本次）**：`SettingsModal` 已改为延迟卸载 + 统一进出场（`.lk-dialog-backdrop` / `.lk-dialog-panel`），第十三轮实测有淡出 |
| **B24** | **窗口最小/最大化后整屏瞬黑（闪烁）** | ⚠️ **新增（本次）·用户决定暂缓到专门批次**。现象：最小/最大化后鼠标在窗口内移动时**整屏瞬黑**（偶发）；**浏览器打开同一页面拖动不闪** → 壳层问题。已排查且排除：透明/effect 配置、常驻 `backdrop-filter`、resize 重渲染风暴、`backgroundColor` 缺失、`shadow:false`（实测无效已回滚）；事件日志无 TDR/dxgkrnl/DWM 错误。机器：AMD Radeon(2021‑11‑30 驱动) + RTX 3060 Laptop 混合显卡、单屏 2560×1440@**165Hz**、**FreeSync/VRR 开启**。候选方案：M3 给 `lorekeeper.exe` 指定单一 GPU ／ M4 临时 60Hz ／ A2 WebView2 `--disable-direct-composition` ／ M1 关 MPO（注册表，需审批+重启）／ M2 更新 AMD 驱动 |
| **B25** | **`ClearProjectDataDialog` 未纳入统一动画** | ✅ **已解除**（`b4530e50`：手写全屏弹层纳入统一进出场，GUI 已验淡入淡出） |

### 4. 红线提醒（每次接手都要过一遍）

- 🚫 Tauri 侧**禁止**读 `AI_NOVEL_VELA_HOME`、**禁止**回退 `~/.vela`、**禁止**写 `.vela/vela.db`。
- 项目库 = `<root>/.lore/lorekeeper.db`；KB 向量 = `<root>/.lore/kb/`；全局数据根 = `AI_NOVEL_LOREKEEPER_HOME` 或 `~/.lorekeeper`。
- 失败文案按基线 MUTATING 规则带 `"Error: "` 前缀（`commands/db.rs::mutating_error`）；但 `skills:*` 与 `prompt:load-global` 的 diagnostics 按基线**不带**前缀。
- 定稿不可逆：`finalization:` 相关改动一律 **Ask first**。
- 兜底约定：新增命令入参结构体必须带 `#[serde(rename_all = "camelCase")]`（`pi-development.md` §2.5 + 源码扫描测试）。
- ⚠️ **新增红线（本次，来源 B18/D1–D8）**：`src/import/` 的 8 项刻意偏离**不得在未改动基线对照的情况下静默去除**；
  尤其 D1（无密钥 sha256）与 D2（无 `webContentsId`）是**安全相关**决定，恢复需用户确认。
- 🚫 **导入路径安全**：来源绝对路径**永久禁止**回传渲染层；检视存储只暴露 `ImportInspectionSummary`（令牌 + 展示事实 + 前 8 章预览）。
- **更新域约定**：`update:*` 后端恒为 GitHub-Release 只读元数据（B14 前），`updateAction` 恒 `open-release`；不得伪造「已下载/已安装」，`defer-reminder` 写入全局配置时**必须保留**用户其他设置。
- 提交消息**无 BOM / 无 CRLF / 无行尾空白**（2026-10-08 出过 BOM 事故）。

---

### 5. 自检记录（2026-10-10 实测）

> 生成本表快照时在本机实跑，命令与输出如下。**下次更新快照表必须先重跑这些命令。**

| 命令 | 工作目录 | 实测输出 |
|---|---|---|
| `node scripts/verify-channel-coverage.mjs --quiet` | `tauri-app/` | 契约 invoke 频道 **193**（事件频道 4）· 已注册命令 **185** → 覆盖 **184** · 未迁移 **9** `[db=0 mcp=9]` · 命令名与契约频道一一对应 ✅ |
| `cargo test --lib` | `tauri-app/src-tauri/` | `test result: ok. 645 passed; 0 failed; 0 ignored; 0 measured` |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | `Finished dev profile ... `（**0 告警**） |
| `cargo fmt --check` | `tauri-app/src-tauri/` | 输出 **0 行**（干净） |
| `pnpm typecheck` / `pnpm run lint` | `tauri-app/` | exit 0 / exit 0 |
| `npx vitest run test/channel-migration-coverage.test.ts test/ipc-arg-struct-contract.test.ts` | `tauri-app/` | `Test Files 2 passed`，`Tests 7 passed` |
| `git status --porcelain` | 仓库根 | 仅 1 份未跟踪文档 `docs-fork/research/2026-10-10-b24-black-flash-investigation.md`（B24 暂缓中的排查记录）；跟踪文件干净 |
| `git log -1` | 仓库根 | `f31446ec feat(tauri): G3b 导入运行执行租约/批次推进/effect receipts（13 频道注册）` |
| `pnpm tauri dev`（第十轮冒烟，G1） | `tauri-app/` | VITE `ready in 441 ms` · cargo `Finished dev profile in 47.50s` · `lorekeeper.exe` **90 MB** · 3 项人工验证全部 ✅ |
| `pnpm tauri dev`（第十一轮冒烟，G2a） | `tauri-app/` | VITE `ready in 812 ms` · cargo `Finished dev profile in 42.46s` · `lorekeeper.exe` **45 MB** · 4 项人工验证全部 ✅（唯一 console.error 为预期的 G2b 频道未迁移） |
| `pnpm tauri dev`（第十二轮冒烟，ESC/窗口修复） | `tauri-app/` | VITE `ready` · cargo 增量重建 · `lorekeeper.exe` **32 MB** · 4 项人工验证全部 ✅（窗口最小/最大化、标题栏拖拽、设置弹窗 ESC、关闭按钮） |

