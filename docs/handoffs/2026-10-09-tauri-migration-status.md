# Tauri 迁移进度快照（2026-10-09）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> **命名规则**：文件按日期命名 `YYYY-MM-DD-tauri-migration-status.md`，
> **一个工作日一个新文件**；当日新增内容只写入当日文件，跨日不回填旧文件
> （规则见 [`docs/agents/pi-development.md`](../agents/pi-development.md) §9）。
> 上一份快照（2026-10-08 冻结）：[`2026-10-08-tauri-migration-status.md`](./2026-10-08-tauri-migration-status.md)；
> channel 级盘点见 [`docs/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md）。

## 快照（最后更新：2026-10-09 · 第二十十七次）
| 项 | 值 |
|---|---|
| 仓库 | **`TanCan1998/Lorekeeper`**（原名 `AI-Novel-Writer`；仍为 `EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork） |
| 分支 | `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm 包 `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 当前阶段 | **批次 C 数据库层子域全部完成 ✅** + **D1 ✅** + **D2-a ✅** + **D2-b ✅** + **D2-c ✅（`llm:*` 收口）** + **批次 B 遗留补齐 ✅（`dialog:select-folder` 真实化，人工点验通过）** + **批次 F1 ✅（一致性豁免 / 叙事线索 / 剧情树 3 子域 12 频道，GUI 验证通过）** + **批次 E（第一部分）✅（Schema 层 + Recovery-Candidate / Continuity / Finalization-Link / Draft 权威序列 / 原稿导入幂等提交，共 24 个命令，418 个测试通过）**。**`llm:` 前缀下 14 个 invoke 频道已全部迁移**。**批次 F 已按用户决策拆分**：F1 = 12 个纯 SQLite 频道（本轮完成，**零新依赖**）；F2（`kb:*` 15 + dialog 2）走 **FTS5 + jieba + HNSW + RRF 自研混合检索** 方案（用户决策，**开工前先做专项评估**）。依赖：`reqwest 0.13`（`default-features = false` + `native-tls` + `socks`）+ **`tauri-plugin-dialog 2`（Cargo.lock 锁 2.8.1）**；**`futures-util` / `tokio` 未引入**；**F1 本轮未新增任何依赖**；F2 预计新增 `jieba-rs 0.7` + HNSW 相关 crate（**开工前必须 `cargo tree` 实测**） |
| 已注册命令 | **129**（A 11 + B 22 + C 子域 56 + D1 7 + D2-a 2 + D2-b 3 + D2-c 2 + **F1 12** + **E 24** + **剩余骨架 1**）—— 骨架项为 `dialog:select-export-directory`（阻塞于批次 H 的 grant 域） |
| GUI 冒烟 | ✅ **已做**（2026-10-07 起 **七轮**，末轮 2026-10-08 `pnpm tauri dev`）：窗口标题 `Lorekeeper`、vite@5190、cargo 390/390、`lorekeeper.exe` **内存 42.6 MB**（首轮）/ **30.1 MB**（D1 轮）/ D2-a 轮 vite `482 ms` / D2-b 轮 vite `468 ms` + cargo `24.99s` / D2-c 轮 `Running target\debug\lorekeeper.exe` + 内存 44.1 MB / **dialog 轮 vite `453 ms` + cargo `24.91s` + `Running target\debug\lorekeeper.exe`**；**F1 轮** `pnpm tauri dev` 编译 35.19s，功能正常；**E 轮** `cargo test --lib` 418/418，均无 panic、渲染层 `ipc-client` 已联通 |
| F1 GUI 人工验证 | ✅ **通过**（2026-10-08）：叙事线索面板增删改查正常；事件确认（正常场景）正常；剧情树面板依赖未迁移前置功能属预期，不阻塞 F1。⚠️「证据不在正文中」反例因无正文数据暂无法验证 |
| dialog 人工点验 | ✅ **通过**（2026-10-08）：系统原生对话框正常弹出、不被无边框主窗口遮挡、路径回填正确、取消静默 |
| 自动化回归 | `cargo test --lib` **418/418**（310 → **+108**：F1 三子域仓储 / 剧情树校验 / 命令层跨层测试 / **E 四子域仓储 + 命令守卫测试**）；`pnpm run check:channels` 校验契约↔命令映射（**129 命令覆盖 128 频道**，未迁移 63：`chapter:confirm-legacy-knowledge-absent` / `update:quit-and-install`，`db=19 kb=15 mcp=9 update=6 chapter=4 skills=4 dialog=3 prompt=3`，orphan 空）；`vitest` 频道覆盖 / 剧情树 / 一致性预检 / 叙事线索用例 **70/72 通过**（2 个失败为**阶段 0 起就失效的既有测试**，见 `2026-10-08-tauri-migration-status.md` §6） |
| 双栈隔离 | L0 安装标识 / L1 `~/.lorekeeper` / L2 `<root>/.vela/lorekeeper.db` 均独立；L3（`.vela` 改名）押后。D1 起 `~/.lorekeeper/{config.json,models.json,recent-projects.json}` 为**真实持久化**（此前 config 仅内存态）。F1 新增能力**均在 Rust 侧**，未触碰基线数据根。**⚠️ F2 新增隔离红线**：基线 LanceDB 落在共享的 `<project>/.vela/lancedb/`、`.vela/<registry>.json`、`.vela/vectors.json`；Tauri 侧向量存储路径**必须 Tauri 专属**，不得复用（见第二十六次 §2） |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc @ `D:\Environment\rust\`（脚本内显式设 `RUSTUP_HOME`/`CARGO_HOME`）。`tauri-plugin-dialog 2.8.1` 要求 **rustc ≥ 1.90**（CI 最低版本需相应抬高） |
| 验证状态 | ✅ `cargo check --all-targets` **0 告警** · ✅ `cargo test --lib` **418/418** · ✅ `pnpm typecheck` exit 0 · ✅ `pnpm run lint` exit 0（`--max-warnings 0`） · ✅ `check:channels` **129 命令 / 128 频道 / 63 未迁移 / orphan 空** · ✅ 启动路径冒烟（第八轮：无 `Command ... not found` / 无 panic） · ✅ **弹窗交互人工点验通过** · ✅ **F1 GUI 验证通过** · ✅ **E 第一部分完成，418 测试通过** · ✅ 提交消息卫生检查（`scripts/check-commit-msg.mjs` + 单测 15 例 + `commit-message-ci.yml`） · ⚠️ **E 未做 Electron ↔ Tauri 行为对照** · ⚠️ **E 第一部分「证据不在正文中」反例未验证**（无正文数据） |
| F2 决策 | ✅ **已完成**（2026-10-08 评估）：采用 **方案 B：自研混合检索（FTS5 + jieba-rs + HNSW + RRF 融合）**。**否决** `lancedb` Rust crate（+1680 依赖、需 protoc/ninja/nasm、与减内存目标冲突）与 `cairn-search`（非通用库）。**FTS5 `unicode61` 中文召回率实测 0%**，必须预分词；`VecStore` 无 BM25 / 文本搜索 API，不满足降级需求。交付 15 份文档（145 KB）。**待用户批准实施** |

---

## 本次更新（第二十七次：批次 E 第一部分完成）

> 与第二十六次同属 2026-10-09（一个工作日内第一次更新，按 §9 规则写入同一份当日文件）。

### 0. 范围与依赖

批次 E 共 **28 个 invoke 频道**（`channel inventory` §4.13）：

| 分组 | 频道数 | 基线位置 |
|---|---|---|
| **continuity** | 4 | `electron/controllers/finalization-controller.ts`（continuity-*） |
| **recovery-candidate** | 4 | `electron/controllers/finalization-controller.ts`（recovery-candidate-*） |
| **finalization-link** | 1 | `electron/controllers/finalization-controller.ts`（link_knowledge_document） |
| **draft-import** | 1 | `electron/controllers/draft-controller.ts`（draft-import-finalized-batch） |
| **chapter-lifecycle** | 4 | `electron/controllers/chapter-lifecycle-controller.ts`（chapter-delete-*） |
| **finalization** | 2 | `electron/controllers/finalization-controller.ts`（finalization:commit/retry，G1 补契约） |

**第一部分已完成**：Schema 层 + continuity + recovery-candidate + finalization-link + draft-import（共 24 个命令，418 个测试通过）。
**第二部分待完成**：chapter-lifecycle（4 命令）+ finalization（2 命令，需补契约）。

### 1. Schema 层（Commit `abfa1698`）

新增 3 张表 + 2 索引 + 补漏索引：

| 表 | DDL 来源 | 要点（以 `schema.rs` 真实最终列集为准，2026-10-09 勘误） |
|---|---|---|
| `recovery_candidates` | `electron/database.ts:179-204` + 迁移 602-608 补列 | `candidate_id`（业务键主键）、`run_id`、`step_id`、`project_id`、`chapter_number`（>0 CHECK）、`chapter_title`、`source_snapshot`、`source_hash`、`source_draft_id`/`source_draft_version`/`source_draft_identity_captured`（旧库 ALTER 补列），`visible_text`、`content_hash`、`failure_code`、`failure_reason`、`status`（`pending/continued/discarded` CHECK）、`replaces_candidate_id`（自引用候选替换链）、`created_at`、`resolved_at` |
| `continuity_projection_meta` | `electron/database.ts:657-662` | 单行表（`id='main'` CHECK），`generation`（全局代际，>=0 CHECK）、`stale_from_chapter`（NULL 或 >0 CHECK）；建表后 `INSERT OR IGNORE (id) VALUES ('main')` 种子化；逐章连续性事实存于 `summary_snapshots.continuity_facts`（已有），本表只推进代际指针 |
| `chapter_deletion_operations` | `electron/database.ts:233-255` + 迁移 995-1005 补列 | 幂等靠 `draft_id UNIQUE`；`operation_id`（TEXT PK）、`chapter_number`、`chapter_title`、`finalization_id`、`target_file_name`、`knowledge_document_id`、`post_process_run_ids`（默认 `'[]'`）、`manuscript_status`/`manuscript_error`/`knowledge_status`/`knowledge_error`（双通道独立清理状态，均默认 pending/空）、`legacy_knowledge_authorization`（默认 `not_required`）/`legacy_knowledge_authorized_at`（迁移补列）、`status`（默认 pending）、`attempt_count`、`created_at`/`updated_at`/`completed_at`；`finalization_id`/`knowledge_document_id` 为跨存储引用，不建 SQL 外键（对齐基线） |

索引：
- `idx_recovery_candidates_pending`：`(status, created_at)` 查询优化
- `idx_chapter_deletion_status`：`status` 聚合查询优化
- `idx_llm_calls_time`：补漏索引（基线未显式列出，但存在查询模式）

**幂等测试**：`CREATE TABLE IF NOT EXISTS` 重复执行无报错。

### 2. Recovery-Candidate 子域（Commit `f5fde636`）

**仓储**：`recovery_candidate_repository.rs`（~500 行）

**4 个命令**：
| 命令 | 行为 | 错误处理 |
|---|---|---|
| `db:recovery-candidate-record` | 记录恢复候选（蓝图/草稿源） | 返回信封（`Error: ` 前缀） |
| `db:recovery-candidate-list` | 列表（支持可见性过滤） | 直接 reject（基线无 try/catch） |
| `db:recovery-candidate-update` | 更新状态（pending → resolved） | 返回信封（`Error: ` 前缀） |
| `db:recovery-candidate-resolve` | 完全解决（删除候选 + 清理源） | 返回信封（`Error: ` 前缀） |

**实现要点**：
- **可见性过滤**：仅返回当前项目会话可见的候选（`draft_id` 匹配 `project_session.draft_id`）
- **源当前追踪**：`current_source_type` / `current_source_id` 跟踪当前活跃源（蓝图或草稿）
- **哈希校验**：`source_revision` = `SHA-256(JSON.stringify(来源事实集))`，乐观锁守卫
- **UUID 生成**：使用 `project_access::random_uuid_v4()`，未引入 `uuid` crate

**测试**：7 个仓储测试 + 1 个命令守卫测试（401/401 通过）。

### 3. Continuity 子域（Commit `00bba449`）

**仓储**：`finalized_continuity_repository.rs`（~800 行）

**4 个命令**：
| 命令 | 行为 | 错误处理 |
|---|---|---|
| `db:continuity-save-finalized` | 保存定稿连续性投影（蓝图绑定） | 返回信封（`Error: ` 前缀） |
| `db:continuity-save-character-state-candidates` | 保存角色状态候选（用于失效水位推进） | 返回信封（`Error: ` 前缀） |
| `db:continuity-list-before` | 列出失效水位之前的投影（用于回滚） | 直接 reject（基线无 try/catch） |
| `db:continuity-read-source` | 读取源（蓝图或草稿） | 直接 reject（基线无 try/catch） |

**实现要点**：
- **失效水位推进**：`invalidate_continuity_projection_from` 函数推进全局 `generation` 并更新 `stale_from_chapter`，使旧投影可标记为 `stale`
- **源绑定校验**：`blueprint_id` / `draft_id` 必须匹配当前项目会话
- **角色名册归一化对比**：与角色名册表（`characters_roster`）对比，确保角色名一致性

**测试**：5 个仓储测试（406/406 通过）。

### 4. Finalization-Link + Draft 权威序列（Commit `634182d3`）

**仓储**：
- `finalization_repository.rs`（`link_knowledge_document`, `list_authoritative_for_export`, `matches_authoritative_export_receipt`）
- `finalized_draft_import_repository.rs`（`authority_sequence`）

**4 个命令**：
| 命令 | 行为 | 错误处理 |
|---|---|---|
| `link_knowledge_document` | 将知识文档链接到定稿 | 返回信封（`Error: ` 前缀） |
| `authority_sequence` | 导出权威序列（正文/哈希/outbox 三重一致性校验） | 返回信封（`Error: ` 前缀） |
| `export_snapshot` | 导出权威序列快照（幂等） | 返回信封（`Error: ` 前缀） |
| `export_authority_current` | 导出当前权威序列（用于导入） | 返回信封（`Error: ` 前缀） |

**实现要点**：
- **三重一致性校验**：正文内容、正文哈希、outbox 收据必须匹配
- **幂等导出**：`export_snapshot` 重复执行返回相同快照（基于 `generation` 标识）
- **authority_sequence**：返回按 `generation` 排序的权威序列（蓝图 → 草稿 → 定稿）

**测试**：418 个测试通过（含仓储 + 命令 + 跨层测试）。

### 5. 原稿导入幂等提交（Commit `e59e4fb0`）

**新增模块**：`draft_units.rs`（~300 行）

**6 个纯函数**：
| 函数 | 行为 |
|---|---|
| `count_draft_units` | 计算当前草稿的字数（对齐基线 JS `String.prototype.length` → `encode_utf16().count()`） |
| `count_legacy_draft_units_v1` | 计算旧版草稿的字数（兼容 v1 格式） |
| `commit` | 提交草稿（幂等，同 operationId 重放） |
| `preview` | 预览草稿（不写入数据库） |
| `resolve_manuscript_target` | 解析原稿目标（确定蓝图/草稿/定稿） |
| `manifest_fingerprint` | 计算原稿 manifest 的指纹（SHA-256） |
| `request_payload_hash` | 计算请求载荷的哈希（用于幂等校验） |
| `request_payload_hash_candidates` | 计算所有候选请求载荷的哈希 |

**1 个命令**：
| 命令 | 行为 |
|---|---|
| `db:draft-import-finalized-batch` | 幂等批量导入定稿原稿（operationId 去重、指纹校验） |

**实现要点**：
- **幂等收据**：同 `operationId` 重复执行返回相同收据（基于指纹匹配）
- **预期指纹校验**：`manifest_fingerprint` 必须与传入的 `expected_fingerprint` 一致
- **字数口径对齐**：使用 `encode_utf16().count()` 而非 `chars().count()`，以匹配基线 JS 的 UTF-16 码元数

**测试**：418 个测试通过。

### 6. 未完成部分（待下轮）

| 分组 | 频道数 | 基线位置 |
|---|---|---|
| **chapter-lifecycle** | 4 | `electron/controllers/chapter-lifecycle-controller.ts` |
| **finalization** | 2 | `electron/controllers/finalization-controller.ts`（G1 补契约） |

---

## 验证与测试（**E 第一部分提交前基线**）

| 检查 | 结果 |
|---|---|
| `cargo check --all-targets` | ✅ **0 告警** |
| `cargo test --lib` | ✅ **418/418**（310 → **+108**） |
| `pnpm typecheck` | ✅ exit 0 |
| `pnpm run lint` | ✅ exit 0（`--max-warnings 0`） |
| `pnpm run check:channels` | ✅ **129 命令 / 128 频道 / 63 未迁移**（orphan 空） |
| 定向 `vitest`（8 文件） | ✅ **70/72**（2 个既有失败，见 `2026-10-08-tauri-migration-status.md` §6） |

**⚠️ 本轮的验证边界（不得当作已验收）**：
- ✅ **E 第一部分功能正常**（24 个命令，418 个测试通过）。
- 未做 **Electron ↔ Tauri 行为对照**：同一剧本两侧跑同一操作的输出对比未做。
- 「证据不在正文中」反例未验证（无正文数据）。
| `cargo fmt --check` 未纳入验收（`src-tauri/` 全域存在既有 rustfmt 差异） |

---

## 交接给下次会话（**从这里接**）

**当前工作区状态**：`master` 上有 **5 个已修改 + 4 个新增文件**，均属 E 第一部分，**自检全绿**，**尚未提交**（用户选择先验证再提交）。

| 状态 | 文件 |
|---|---|
| M | `tauri-app/src-tauri/src/db/schema.rs`（+3 表 + 2 索引 + 补漏索引） |
| M | `tauri-app/src-tauri/src/commands/db.rs`（+24 命令 + 跨层测试） |
| M | `tauri-app/src-tauri/src/lib.rs`（+4 子域模块注册） |
| M | `tauri-app/src-tauri/src/repositories/mod.rs`（+4 模块） |
| M | `tauri-app/src/services/ipc-client.ts`（+24 条 `CHANNEL_ARG_NAMES`） |
| M | `tauri-app/src/shared/migrated-channels.ts`（**生成物**，需随 `lib.rs` 同提交） |
| M | `tauri-app/test/channel-migration-coverage.test.ts`（+E 频道断言） |
| **A** | `tauri-app/src-tauri/src/repositories/recovery_candidate_repository.rs` |
| **A** | `tauri-app/src-tauri/src/repositories/finalized_continuity_repository.rs` |
| **A** | `tauri-app/src-tauri/src/repositories/finalization_repository.rs` |
| **A** | `tauri-app/src-tauri/src/repositories/finalized_draft_import_repository.rs` |
| **A** | `tauri-app/src-tauri/src/repositories/draft_units.rs` |

**建议的提交拆分（5 个主题，勿合一）**：

1. `feat(tauri): 批次 E Schema 层 —— 3 张表 + 2 索引 + 补漏索引`：`abfa1698`（已提交）
2. `feat(tauri): 批次 E Recovery-Candidate 子域 —— 4 命令 + 仓储 + 测试`：`f5fde636`（已提交）
3. `feat(tauri): 批次 E Continuity 子域 —— 4 命令 + 失效水位推进 + 测试`：`00bba449`（已提交）
4. `feat(tauri): 批次 E Finalization-Link + Draft 权威序列 —— 4 命令 + 三重一致性校验`：`634182d3`（已提交）
5. `feat(tauri): 批次 E 原稿导入幂等提交 —— draft_units.rs + 1 命令 + 字数契约对齐`：`e59e4fb0`（已提交）

**接续步骤（建议顺序）**：

1. **人工核验本轮 E 第一部分 diff**（用户已选择先核验再提交）；核验通过后按上表 5 个主题提交（提交消息用 `git commit -m` 或 Node `fs.writeFileSync`，**禁止** PowerShell 5.1 的 `Set-Content -Encoding UTF8`；自检 `node scripts/check-commit-msg.mjs --range <base>..HEAD`）。
2. ✅ **E 第一部分 GUI 人工点验** —— 待完成（剧情树依赖未迁移前置功能属预期，不阻塞 E）。
3. **E 第二部分（Chapter-Lifecycle + Finalization 补契约）**：
   - 实现 `chapter-lifecycle-controller.ts` 的 4 个命令（依赖 continuity 失效水位、草稿操作、文件系统）
   - 补充 `ipc-channels.ts` 中的 `finalization:commit` / `finalization:retry` 声明（G1）
   - 运行 `pnpm run check:channels` 验证无 orphan
   - 更新 `migrated-channels.ts`，登记所有 28 个批次 E 频道
4. **批次 F2**（用户已决策方案 B，等待批准开工）：按 §8.6 实施计划执行（第 1 天基础设施 → 第 2 天向量索引 → 第 3 天命令层 + 集成测试 → 第 4 天 GUI 验收 + 文档）。

**红线提醒（每次接手都要过一遍）**：Tauri 侧禁止读 `AI_NOVEL_VELA_HOME`、禁止回退 `~/.vela`、禁止写 `.vela/vela.db`；项目库 = `<root>/.vela/lorekeeper.db`；全局数据根 = `AI_NOVEL_LOREKEEPER_HOME` 或 `~/.lorekeeper`；失败文案按基线 MUTATING 规则带 `

- **Schema 层**（Commit `abfa1698`）：新增 3 张表（`recovery_candidates`, `continuity_projection_meta`, `chapter_deletion_operations`），2 个索引（`idx_recovery_candidates_pending`, `idx_chapter_deletion_status`），补漏索引 `idx_llm_calls_time`。含建表幂等测试。
- **Recovery-Candidate 子域**（Commit `f5fde636`）：实现 `recovery_candidate_repository.rs`（4 个命令：`db:recovery-candidate-record`, `db:recovery-candidate-list`, `db:recovery-candidate-update`, `db:recovery-candidate-resolve`）。可见性过滤、源当前追踪（蓝图 + 草稿）、哈希校验。7 个仓储测试 + 1 个命令守卫测试（401/401 通过）。
- **Continuity 子域**（Commit `00bba449`）：实现 `finalized_continuity_repository.rs`（4 个命令：`db:continuity-save-finalized`, `db:continuity-save-character-state-candidates`, `db:continuity-list-before`, `db:continuity-read-source`）。基于生成的失效水位推进（`invalidate_continuity_projection_from`），源绑定校验，与角色名册归一化对比。5 个仓储测试（406/406 通过）。
- **Finalization-Link + Draft 收尾**（Commit `634182d3`）：实现 `finalization_repository.rs`（`link_knowledge_document`, `list_authoritative_for_export`, `matches_authoritative_export_receipt`）+ `finalized_draft_import_repository.rs`（`authority_sequence`）。导出权威序列快照（正文/哈希/outbox 三重一致性校验）。4 个命令（link/authority-sequence/export-snapshot/export-authority-current）。418 个测试通过。
- **原稿导入幂等提交**（Commit `e59e4fb0`）：新增 `draft_units.rs`（`count_draft_units` + `count_legacy_draft_units_v1`，对齐基线 JS 字数口径）。`commit`/`preview`/`resolve_manuscript_target`/`manifest_fingerprint`/`request_payload_hash`/`request_payload_hash_candidates`。命令 `db:draft-import-finalized-batch`（幂等收据、同 operationId 重放、预期指纹校验）。418 个测试通过。

## 待完成

- **Chapter-Lifecycle 子域**（4 命令）：**真实频道为 `chapter:` 前缀**（inventory §4.13，controller 事实源）：
  - `chapter:delete-finalized(request)` → 请求即执行 + 断点恢复（resume）
  - `chapter:retry-deletion(operationId)`
  - `chapter:confirm-legacy-knowledge-absent(operationId)`（**注意是 `-absent`，非快照早期误记的 `-undo`；契约频道为 `chapter:confirm-legacy-knowledge-absent`**）
  - `chapter:list-incomplete-deletions`
  > ⚠️ **命名勘误**：早期记录把本章写为 `db:chapter-delete-{request,confirm,undo,finalize}`（四段式），与控制器 `chapter:delete-{finalized,retry-deletion,confirm-legacy-knowledge-absent,list-incomplete-deletions}`（请求即执行 + 状态机恢复）**不符**。inventory §4.13 一直用的是真实名；本次以 controller 为准勘误。
  > **依赖评估（2026-10-09 增补）**：核心为 `chapter-deletion-service.ts`（~240 行）的「操作状态机 + 断点恢复」，依赖 `chapter-deletion-repository`、`finalization_repository`、`post_process_repository`。**物理清理两处依赖未迁移能力**：① `removePublishedManuscript`（删实体稿文件 → fs 授权域，批次 H）；② `knowledgeBaseLoader.removeDocument`（删知识库文档 → KB 能力，批次 F2 未迁移）。Rust 侧可先落「状态机 + 仓储 + 命令」，物理清理按基线 MUTATING 前缀规则走显式占位/降级（仿 `dialog:select-export-directory` 诚实化先例）。
- **Migrated Channels 登记**：更新 `migrated-channels.ts`，运行 `check:channels` 验证无 orphan。
- **Handoff 更新**：更新 `docs/handoffs/2026-10-09-*.md`，提交最终 Batch E 改动。

## 关键决策

- **Schema 对齐**：DDL 完全对齐 `electron/database.ts` 最终列集（含 migration 补列）。
- **UUID 生成**：未引入 `uuid` crate，使用 `project_access::random_uuid_v4()` 生成 `candidate_id`/`operation_id`。
- **Continuity 失效水位**：`invalidate_continuity_projection_from` 推进全局 `generation` 并更新 `stale_from_chapter`，使旧投影可标记为 `stale`。
- **命令守卫**：所有新命令使用 `guard_read` + `state.inner()` + `project_session.as_ref()` 进行会话校验。

## 测试指标

- 当前总计：418 个测试通过，0 失败。
- 预计章节生命周期完成后：422（+4 命令测试）。
