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

## 快照（最后更新：2026-10-10 · 第三十五次）

> 本表只填**最新一次自检的实测值**。改表前必须重跑对应命令，不得沿用旧数字、不得估算。
> 本轮实测命令与输出见下方「[§5 自检记录](#5-自检记录2026-10-10-实测)」。

| 项 | 值 |
|---|---|
| 仓库 / 分支 | **`TanCan1998/Lorekeeper`**（`EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork）· `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 已注册命令 | **167** |
| 覆盖 invoke 频道 | **166**（契约总数 193，事件频道 4） |
| 未迁移 invoke 频道 | **27**（`db=18 mcp=9`） |
| orphan | **空** ✅ |
| `cargo test --lib` | **572/572** ✅（本轮新增 30 条） |
| `cargo fmt --check` | **干净（0 差异）** ✅ |
| `cargo check --all-targets` | **0 告警** ✅ |
| `pnpm typecheck` / `lint` | exit 0 / exit 0 ✅ |
| 定向 `vitest` | **7/7**（2 文件：契约覆盖 / 入参结构体契约）✅ |
| 已完成批次 | A ✅ / B ✅ / C ✅ / D1 ✅ / D2-a ✅ / D2-b ✅ / D2-c ✅ / **E ✅** / **F1 ✅** / **L3 ✅** / **F2 ✅** / **批次 E G1 ✅** / **H1 ✅** / **H2 ✅** / **H3 ✅** / **B12 ✅** / **批次 G 的 G1 ✅（本次）** |
| 当前阶段 | **批次 G1 完成**（作者原稿导入完整链路）。下一步 **G2（状态机主体 + `reference` 路径，5 频道）→ G3（租约与批次推进，11）→ G4（收口）**（未迁移 27 → 9）。其它待办：B13（导航防护）、B14（真 Windows 自更新）、H4（mcp，暂缓）、上游合并专项 |
| 依赖 | `reqwest 0.13`（`default-features=false` + `native-tls` + `socks`）、`tauri-plugin-dialog 2`（锁 **2.8.1**）、`tauri-plugin-opener 2.7.0`。**本轮新增**：`windows-sys 0.61`（`[target.'cfg(windows)'.dependencies]`，仅在 lock 中提级为直接依赖，**0 新下载**；Ask first 已批准 2026-10-10）。向量层 `hnsw_rs 0.3.4` / `jieba-rs 0.7.0` / `tokio`；FTS5 由 `libsqlite3-sys` bundled 提供 |
| GUI 冒烟 | ✅ 自 2026-10-07 起 **十轮**。**第十轮（2026-10-10，批次 G1）**：vite `441 ms` + cargo `47.50s` → `lorekeeper.exe`（90 MB）；用户人工验证 3 项全部通过 —— 作者原稿（2 个 `.txt`）显示拆章/预览、参考语料（`.md`）显示指向 G2 的诚实错误、`.epub` 显示「导出尚未迁移」诚实错误；dev 日志**无 error/panic/失败**输出（仅两条已知 `setZoomFactor` 占位提示 + Windows EBUSY 文件监视器噪声）。近两轮：第九轮（2026-10-09，批次 H 前三项 + B12）；**第八轮（2026-10-09，批次 E G1）** |
| 双栈隔离 | **L0/L1/L2/L3 全部独立**：安装标识 / `~/.lorekeeper` / `<root>/.lore/`（库 `.lore/lorekeeper.db`、KB 向量 `.lore/kb/`）。基线为 `~/.vela` / `<root>/.vela/`。**两栈项目目录刻意不互通**（`ee40aaab`） |
| Rust 工具链 | rustc/cargo **1.99.0 stable-msvc** @ `D:\Environment\rust\`（脚本内须显式设 `RUSTUP_HOME` / `CARGO_HOME`）。`tauri-plugin-dialog 2.8.1` 要求 **rustc ≥ 1.90**（CI 最低版本需相应抬高） |

<sub>*命令与频道差额：**167 命令**中 1 个为阶段 0 骨架（`app_health_check`，不对应任何契约频道），其余 **166** 与 invoke 频道一一对应。</sub>

---

## 本次更新（第三十五次：批次 G1 —— 作者原稿导入检视面）

> 按用户 2026-10-10 确认的开工清单执行：
> [`docs-fork/plans/2026-10-10-g1-import-select-kickoff.md`](../plans/2026-10-10-g1-import-select-kickoff.md)。
> 交付**「导入作者原稿」完整可用链路**（`reference` 与 `.epub` 返回诚实错误，属 G2 / 待批依赖）。

### 1. 新增（本轮）

| 项 | 内容 |
|---|---|
| 模块 | `src/import/{mod,limits,inspection_store,parsing}.rs`（新建）；`src/commands/import.rs`（新建） |
| 命令 | `dialog:select-novel-files` → `dialog_select_novel_files`；`db:import-run-author-preview` → `db_import_run_author_preview`（`commands/db.rs` 追加 inner + 命令） |
| Schema | **无 Schema 变更**（G 的 9 张表已在 `cca792cd` 落地，本轮只读消费） |
| 接线 | `state.rs` 增 `import_inspections: Mutex<ImportInspectionStore>`；`lib.rs` 增 `pub mod import;` + 注册 2 命令；`commands/mod.rs` 增 `pub mod import;` |

**实现要点**：

1. **`inspection_store` 逐字平移**基线 `import-inspection-store.ts`（章节 1..=5000、来源 1..=5000、sha256 摘要格式、
   displayName ≤255 且不含 `/ \ NUL`、mediaType ≤100、单章 ≤16 MiB、总量 ≤128 MiB、`(sourceIndex, sourceChapterNumber)`
   唯一、同会话重选即替换、TTL 10 分钟、`consume`/`peek(purpose)`/`clear`/`active_count`）；**时钟可注入**以便 TTL 测试。
2. **解析纯函数逐字平移** `import-controller.ts:73-296`（三条正则池 + `chinese_num_to_arabic` + `extract_chapter_number`
   + `extract_title` + `has_chapter_headings` + `split_single_file_content`，字数走既有 `draft_units::count_draft_units`）。
   仅把 JS 的 `\d` 写成 `[0-9]`（Rust `regex` 默认 Unicode 数字），语义不变。
3. **门禁形态刻意区分**：项目租约/`expectedProjectPath` 失败 → **invoke reject**（与全部 `db:*`/`fs:*` 一致）；
   业务失败（上限/空文件/重号/epub/reference）→ `{success:false,error}` 信封（对齐基线 `importSelectionErrorMessage` 文案逐字）。
4. **`db:import-run-author-preview` 复用批次 E 已迁移的 `finalized_draft_import_repository::preview`**，以 `peek`（只读）取检视，
   故渲染层可重复预览同一令牌（对齐基线；消费归 G2 的 `prepare-inspection`）。
5. **`windows-sys` 取文件标识**：std 的 `MetadataExt::file_index/volume_serial_number` 仍是不稳定 feature
   （`windows_by_handle`，rustc 1.99 实测 E0658），改用 Win32 `GetFileInformationByHandle`（唯一一处 `unsafe`，
   已附 SAFETY 注释）。

**⚠️ 诚实化占位 / 降级项（本轮必须显式列出）**：

| 项 | 阻塞于 | 表现形式 |
|---|---|---|
| `.epub` 导入 | 需 `zip` 类 crate（**Ask first**） | 对话框仍列出 epub（与基线筛选器一致），选中后返回 D4 诚实错误 |
| `reference`（参考语料）路径 | 批次 G2 的 `beginParsing`/`prepare`/`finalizeParsing` | 选中文件后返回诚实错误，指向 G2 |
| `MAX_ACTIVE_INSPECTIONS` / `maxAggregateBytes` | D2 移除 `webContentsId` 后无「他人会话」 | 校验保留但**结构不可达**（单窗口单逻辑会话的直接后果，见模块文档） |

### 2. 刻意偏离（本轮，须在复活基线行为时逐条改回）

| # | 偏离 | 理由 |
|---|---|---|
| D1 | 来源身份摘要 = **无密钥 sha256**（基线 `HMAC(applicationSecret, …)`） | 身份从不进渲染层；零新依赖（避免 `hmac` 另批审批） |
| D2 | 不迁移 `webContentsId` 归属校验；`clear()` 退化为「清空全部待处理检视」 | Tauri 单窗口，与 `external_grant.rs` 既有决定一致 |
| D3 | 不建 `import_legacy_identity_bridge`；legacy 解析路径整体跳过 | L3 两栈项目目录刻意不互通，无遗留身份可桥 |
| D4 | `.epub` 诚实错误 | 解包需新增依赖（Ask first） |
| D5 | 文件读取直接 `std::fs` + 字节上限（无基线句柄链） | 路径从不回传渲染层；上限与基线一致 |
| D6 | 对话框用 `tauri-plugin-dialog`（`pick_files` 多选 + 筛选器） | 与 `dialog:select-knowledge-files` 同路径 |
| **D7（本轮新增）** | 来源文件按**数字感知自然序**排序，逼近但不等价于 `localeCompare(…, 'zh-CN', {numeric:true})` | 完整 zh-CN 拼音排序需 ICU 类新依赖（Ask first）；排序只影响来源处理顺序，章号最终由正文/文件名决定 |
| **D8（本轮新增）** | `alias_digest` 未做 NFC 之外的 `toLocaleLowerCase('en-US')` 对等（Rust 用 `to_lowercase()`），Windows 下已小写化 | 差异仅存在于土耳其语等特殊大小写规则，路径场景无影响 |

### 3. 前端登记

- `src/services/ipc-client.ts`：新增 2 条 —— `'dialog:select-novel-files': ['request', 'projectSession']`
  （⚠️ 开工清单称「已有登记」**不实**，实际为本次补登；该频道属能力域不自动注入会话，故 `projectSession` 为显式第 2 定位参数）、
  `'db:import-run-author-preview': ['inspectionId', 'expectedProjectPath']`。
- `src/shared/migrated-channels.ts`：**生成物**，166 频道（须与 `lib.rs` 同提交）。
- `test/channel-migration-coverage.test.ts`：`dialog:select-novel-files` 断言 `false → true`、新增
  `db:import-run-author-preview` 为 `true` 与 `db:import-run-prepare-inspection`/`db:import-run-get` 为 `false`。

### 4. 行为对照（Electron ↔ Tauri）

⚠️ **未做双栈 A/B 对照**（本轮只做源码级逐字平移 + Rust 单测；对照边界为「基线源码字符串 → Rust 实现」，
未覆盖真机行为）。真机行为由下方第十轮 GUI 冒烟覆盖。

### 5. GUI 冒烟（第十轮，2026-10-10）—— ✅ 通过

`pnpm tauri dev`：VITE v8.3.2 `ready in 441 ms` → cargo `Finished dev profile ... in 47.50s` → `Running target\debug\lorekeeper.exe`（工作集 90 MB）。
用户人工逐项验证（夹具 `%TEMP%\g1-smoke\`）：

| # | 步骤 | 结果 |
|---|---|---|
| 1 | 打开项目 → 导入小说 → 用途「作者原稿」→ 选中 `第1章 开端.txt` + `第2章 发展.txt` | ✅ 显示拆章/作者原稿预览（走 `db:import-run-author-preview`） |
| 2 | 用途切「参考语料」→ 选 `参考语料.md` | ✅ 显示指向批次 G2 的诚实错误（非通用失败、非静默） |
| 3 | 用途切回「作者原稿」→ 选 `样本.epub` | ✅ 显示 EPUB 未迁移的诚实错误（D4） |

证据：dev 日志**无 error / panic / 失败**输出，仅两条已知 `setZoomFactor`（阶段 3 迁移项）占位提示与
Windows `EBUSY` 文件监视器噪声（目标产物被占用，属正常）。渲染层 `console.log` 不经 Vite 转发，
故 `[ipc-client.invoke]` 日志未入 dev 终端；结论以用户逐项确认为准。

### 6. 收尾

- 提交清单：

| Commit | 说明 |
|---|---|
| `fbb88307` | `feat(tauri): 批次 G1 作者原稿导入（dialog:select-novel-files + db:import-run-author-preview）`（14 文件，+2120/‑2） |
| `75379748` | `docs(tauri): 第三十五次快照与频道盘点更新（批次 G1 收口）` |
| 本文件冒烟追加 | `docs(tauri): 记录第十轮 GUI 冒烟（批次 G1 验收通过）` |
- 规则变更：无。

---

## 交接给下次会话（**从这里接**）

### 1. 当前工作区状态

**工作区干净**（本轮的 2 个提交已完成；本文件因追加第十轮冒烟记录产生第 3 个提交）：

| Commit | 说明 |
|---|---|
| `fbb88307` | `feat(tauri): 批次 G1 作者原稿导入（dialog:select-novel-files + db:import-run-author-preview）` |
| `75379748` | `docs(tauri): 第三十五次快照与频道盘点更新（批次 G1 收口）` |
| 待生成 | `docs(tauri): 记录第十轮 GUI 冒烟（批次 G1 验收通过）`（即本文件的追加） |

- HEAD（写入本表时）：`75379748 docs(tauri): 第三十五次快照与频道盘点更新（批次 G1 收口）`
- ⚠️ 上一份快照（2026-10-09）中**已过期的交接描述**（防照旧操作）：
  1. 「未迁移 29（`db=19 mcp=9 dialog=1`）」「已注册命令 165」「`cargo test — 542/542`」→ 均已变为 **27 / 167 / 572**；
  2. 「下一步 1：批次 G 从 G1 开始」→ **G1 已完成**，下一步是 G2；
  3. 「G1 刻意偏离 4 项」→ 实际落地 **D1–D8 共 8 项**（新增 D7 排序、D8 大小写）；
  4. 上一份快照的 §5 自检已冻结，**不得据其回填**本份数字。

### 2. 下一步（1-2-3）

1. ~~GUI 冒烟（G1 验收剩余项）~~ ✅ **已完成**（第十轮，2026-10-10，3 项全部通过 —— 见「本次更新 §5」）。
2. **批次 G2（状态机主体 + `reference` 路径，5 频道）**：`db:import-run-prepare-inspection` /
   `-finalize-parsing` / `-get` / `-list-resumable` / `-list-chapters`。核心是 `ImportRunRepository` 的
   `beginParsing`（基线 2551 行状态机，**新模块，需先读 `electron/repositories/import-run-repository.ts`**）+
   `ImportSourceIdentityRepository.resolveEncodedSources` 的**无密钥 sha256 版**（G1 已铺 `location_alias_digest`/`file_alias_digest`，
   G2 需补「别名表 upsert + sourceId 复用 + sourceFingerprint」）。
   ⚠️ **开工前仍需按老规矩先出开工清单**（读剩余规范 → 分批 → 决策入档）。
3. **G3 / G4**：执行租约与批次推进 + effect receipts + `db:import-global-facts-commit`；
   `kb:import-reference-text` 去占位 + 前端登记。
4. **其它待办**：B13（导航防护）、B14（真 Windows 自更新）、H4（mcp，暂缓）、上游合并专项。

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
| **B19** | **`reference`（参考语料）路径** | ⚠️ **新增（本次）**：依赖 G2 状态机；G1 返回诚实错误并登记为临时缺口 |
| **B20** | **D7 zh-CN 排序不等价** | ⚠️ **新增（本次）**：无 ICU 依赖，来源文件名排序用数字感知自然序近似；如需逐字对齐须 Ask first 引 ICU |
| **B21** | **G1 GUI 冒烟** | ✅ **已解除（本次）**：第十轮冒烟 3 项全部通过（作者原稿预览 / reference 诚实错误 / epub 诚实错误），dev 日志无 error/panic |

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
| `node scripts/verify-channel-coverage.mjs --quiet` | `tauri-app/` | 契约 invoke 频道 **193**（事件频道 4）· 已注册命令 **167** → 覆盖 **166** · 未迁移 **27** `[db=18 mcp=9]` · 命令名与契约频道一一对应 ✅ |
| `cargo test --lib` | `tauri-app/src-tauri/` | `test result: ok. 572 passed; 0 failed; 0 ignored; 0 measured` |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | `Finished dev profile ... `（**0 告警**） |
| `cargo fmt --check` | `tauri-app/src-tauri/` | 输出 **0 行**（干净） |
| `pnpm typecheck` / `pnpm run lint` | `tauri-app/` | exit 0 / exit 0 |
| `npx vitest run test/channel-migration-coverage.test.ts test/ipc-arg-struct-contract.test.ts` | `tauri-app/` | `Test Files 2 passed`，`Tests 7 passed` |
| `git status --porcelain` | 仓库根 | **空**（提交 `fbb88307` + `75379748` 后；本文件的冒烟追加为第 3 个提交） |
| `git log -1` | 仓库根 | `75379748 docs(tauri): 第三十五次快照与频道盘点更新（批次 G1 收口）` |
| `pnpm tauri dev`（第十轮冒烟） | `tauri-app/` | VITE `ready in 441 ms` · cargo `Finished dev profile in 47.50s` · `lorekeeper.exe` 工作集 **90 MB** · 3 项人工验证全部 ✅ |

---

## 6. 历史指针（不复制正文）

| 日期 | 文件 | 内容 |
|---|---|---|
| 2026-10-09 | [`2026-10-09-tauri-migration-status.md`](./2026-10-09-tauri-migration-status.md) | 第三十四次（H3 update 域）及以前（G schema、L3、E、F2、H1/H2/B12…） |
| 2026-10-08 | [`2026-10-08-tauri-migration-status.md`](./2026-10-08-tauri-migration-status.md) | 第二十三～二十六次（F1 / F2 决策 / 批次 B 遗留补齐 / D2-b·D2-c） |
| 2026-10-07 | [`2026-10-07-tauri-migration-status.md`](./2026-10-07-tauri-migration-status.md) | 批次 D2-a 及以前 |
| 2026-10-06 | [`2026-10-06-tauri-migration-status.md`](./2026-10-06-tauri-migration-status.md) | 批次 A–C 细节 |
