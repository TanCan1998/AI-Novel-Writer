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

## 快照（最后更新：2026-10-10 · 第三十八次）

> 本表只填**最新一次自检的实测值**。改表前必须重跑对应命令，不得沿用旧数字、不得估算。
> 本轮实测命令与输出见下方「[§5 自检记录](#5-自检记录2026-10-10-实测)」。

| 项 | 值 |
|---|---|
| 仓库 / 分支 | **`TanCan1998/Lorekeeper`**（`EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork）· `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 已注册命令 | **170** |
| 覆盖 invoke 频道 | **169**（契约总数 193，事件频道 4） |
| 未迁移 invoke 频道 | **24**（`db=15 mcp=9`） |
| orphan | **空** ✅ |
| `cargo test --lib` | **586/586** ✅（本轮修复窗口命令 +1） |
| `cargo fmt --check` | **干净（0 差异）** ✅ |
| `cargo check --all-targets` | **0 告警** ✅ |
| `pnpm typecheck` / `lint` | exit 0 / exit 0 ✅ |
| 定向 `vitest` | **7/7**（2 文件：契约覆盖 / 入参结构体契约）✅ |
| 已完成批次 | A ✅ / B ✅ / C ✅ / D1 ✅ / D2-a ✅ / D2-b ✅ / D2-c ✅ / **E ✅** / **F1 ✅** / **L3 ✅** / **F2 ✅** / **批次 E G1 ✅** / **H1 ✅** / **H2 ✅** / **H3 ✅** / **B12 ✅** / **批次 G 的 G1 ✅** / **批次 G2a ✅（本次）** |
| 当前阶段 | **批次 G2a 完成**（导入运行读面 3 频道）。下一步 **G2b（写面 2 频道 + 复活 `reference` 路径）→ G3（租约与批次推进，11 + effect receipts）→ G4（收口）**（未迁移 24 → 9）。其它待办：B13（导航防护）、B14（真 Windows 自更新）、H4（mcp，暂缓）、上游合并专项 |
| 依赖 | `reqwest 0.13`（`default-features=false` + `native-tls` + `socks`）、`tauri-plugin-dialog 2`（锁 **2.8.1**）、`tauri-plugin-opener 2.7.0`、`windows-sys 0.61`（`[target.'cfg(windows)'.dependencies]`，仅 lock 提级，**0 新下载**；Ask first 已批准 2026-10-10）。**G2a 零新依赖**。向量层 `hnsw_rs 0.3.4` / `jieba-rs 0.7.0` / `tokio`；FTS5 由 `libsqlite3-sys` bundled 提供 |
| GUI 冒烟 | ✅ 自 2026-10-07 起 **十三轮**。**第十三轮（2026-10-10，弹窗动画统一）**：设置弹窗明显变快（修复前实为“静置 400ms + 播 220ms”）、四类弹窗进出场一致、Radix 弹窗仍居中且尺寸正常 ✅。**第十二轮（2026-10-10，冒烟发现的三项缺陷修复）**：① 弹窗 **ESC 关闭**（根因：Radix `DismissableLayer` 仅在 `index === layers.length-1` 时注册 ESC，而 Radix 关闭后仍保留 `DialogContent` 挂载——实测 `layers.length=7`，可见弹窗永远不是最高层）；② **窗口命令真实化**（批次 A 四个命令原为假成功骨架）；③ **标题栏拖拽**（`-webkit-app-region` 在 WebView2 无效 → 补 `data-tauri-drag-region`）。4 项人工验证全部 ✅。近三轮：第十一轮（G2a）、第十轮（G1）、第九轮（H 前三项 + B12） |
| 双栈隔离 | **L0/L1/L2/L3 全部独立**：安装标识 / `~/.lorekeeper` / `<root>/.lore/`（库 `.lore/lorekeeper.db`、KB 向量 `.lore/kb/`）。基线为 `~/.vela` / `<root>/.vela/`。**两栈项目目录刻意不互通**（`ee40aaab`） |
| Rust 工具链 | rustc/cargo **1.99.0 stable-msvc** @ `D:\Environment\rust\`（脚本内须显式设 `RUSTUP_HOME` / `CARGO_HOME`）。`tauri-plugin-dialog 2.8.1` 要求 **rustc ≥ 1.90**（CI 最低版本需相应抬高） |

<sub>*命令与频道差额：**170 命令**中 1 个为阶段 0 骨架（`app_health_check`，不对应任何契约频道），其余 **169** 与 invoke 频道一一对应。</sub>

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

## 本次更新（第三十六次：批次 G2a —— 导入运行读面 3 频道）

> 承接同日的 G2 开工清单（[`docs-fork/plans/2026-10-10-g2-import-run-kickoff.md`](../plans/2026-10-10-g2-import-run-kickoff.md)）。
> G2a 只交付**读面**；写面（`beginParsing` / `finalizeParsing` / `prepare`）归 G2b。

### 1. 新增（本轮）

| 项 | 内容 |
|---|---|
| 模块 | `src/import/batch_checkpoint.rs`（新建：批次检查点 ID 的解析/构造纯函数）；`src/repositories/import_run_repository.rs`（新建：G2a 读面） |
| 命令 | `db:import-run-get`、`db:import-run-list-resumable`、`db:import-run-list-chapters`（`commands/db.rs` 追加 3 inner + 3 命令） |
| Schema | **无 Schema 变更**（G 的 9 张表已在 `cca792cd` 落地，本轮只读消费） |
| 接线 | `repositories/mod.rs` / `import/mod.rs` 增模块；`import/mod.rs` 增 `ImportPurpose::parse_contract` / `ImportRunLocale::parse_contract`；`lib.rs` 注册 3 命令 |

**实现要点**：

1. **`row_to_snapshot` 完整对齐契约 `ImportRunSnapshot`**（含 `manifest*` 仅在 author-manuscript 下发、
   `completedBatches` 宽检、`unfinishedSourceDisplay` 恒下发、`baseRunId`/`completedAt` 空则省略）；
2. **保留基线的「两条宽松度不同」**：`completed_batches()` 校验并抛错（“checkpoint 损坏”），
   `row_to_snapshot` 的输出字段走 `parse_json(…, {})` 兑底 —— **勿合并**；
3. **`persisted_progress` 的六条分支**逐字平移（parsing / prepared / knowledge / blueprints / global·style·refresh / 其余），
   knowledge 与 blueprints 的完成章数取 `completed_batches` × checkpoint 校验后的**去重章号集合**；
4. **`checkpoint_chapter_numbers` 的 knowledge 分支**额外校验
   `import_run_knowledge_receipts`（purpose / 来源归属 / 内容指纹 / documentId 为 64 hex / state=committed）
   与冻结章节逐条对齐，否则报「参照知识 receipt 未完成或与冻结章节不匹配」；
5. **`assert_frozen_chapter_snapshot`** 在读取时重算 sha256 与字节数（拒绝被篡改的冻结正文）；
6. **`list_chapter_batch` 的 `limit` 夹到 1..=100**（命令层收 `f64` 后 `floor()`，容忍 JS 小数）；
7. `ImportRunRow` / `ImportRunChapterRow` / `ImportRunSourceRow` **完整保留 schema 列集**，
   尚未消费的字段用 `#[allow(dead_code)]` + 注释标出（由 G2b/G3 消费，对齐 `security.rs` 先例）。

**⚠️ 诚实化占位 / 降级项**：本批无新增；G1 的 B18（`.epub` 依赖）与 G2b 范围内的 `reference` 诚实错误按计划保持。

### 2. 刻意偏离

本批无新偏离（沿用 G1 D1′–D10）；`batch_checkpoint.rs` 仅把 JS 的 `\d` 写作 `[0-9]`（Rust `regex` 默认含 Unicode 数字），语义不变。

### 3. 前端登记

- `src/services/ipc-client.ts`：+3 条（`db:import-run-get` / `-list-resumable` / `-list-chapters`）。
- `src/shared/migrated-channels.ts`：生成物，169 频道（须与 `lib.rs` 同提交）。
- `test/channel-migration-coverage.test.ts`：G2a 3 频道断言 `true`；`-prepare-inspection` / `-finalize-parsing` 保持 `false`。

**实际收益**：`ImportNovelDialog` 开启时调用的 `db:import-run-list-resumable` 从「尚未迁移」的友好报错变为**真实可恢复任务列表**。

### 4. 验证（本轮实测）

| 命令 | 输出 |
|---|---|
| `cargo test --lib` | `585 passed; 0 failed`（+13：batch_checkpoint 3 + 读面 10） |
| `cargo check --all-targets` | **0 告警** |
| `cargo fmt --check` | **0 行** |
| `node scripts/verify-channel-coverage.mjs` | 契约 **193** · 已注册 **170** → 覆盖 **169** · 未迁移 **24** `[db=15 mcp=9]` · orphan 空 |
| `pnpm typecheck` / `lint` | exit 0 / exit 0 |
| `npx vitest run`（2 文件） | `Test Files 2 passed`，`Tests 7 passed` |

⚠️ GUI 冒烟见下方「§5 GUI 冒烟（第十一轮）」—— **已通过**。

### 5. GUI 冒烟（第十一轮，2026-10-10）—— ✅ 通过

`pnpm tauri dev`：VITE v8.3.2 `ready in 812 ms` → cargo `Finished dev profile ... in 42.46s` → `Running target\debug\lorekeeper.exe`（**45 MB**）。

夹具（均在 `F:\Temp\`，**未污染用户目录**）：`F:\Temp\g2a-smoke\g2a-smoke.mjs`（可重复使用：`inspect` / `seed` / `clean`）、
`F:\Temp\g2a-smoke\files\`（G1 的 4 个素材）、测试项目 `F:\Temp\loretest\22\111`。

| # | 步骤 | 结果 |
|---|---|---|
| 1 | 项目「111」→ 导入小说（作者原稿 / 当前项目） | ✅ 出现黄色卡片「可继续的导入」：`第1章 开端.txt` / `2/2` / 「阶段：author-commit；进度：2/2」 |
| 2 | 点「继续导入」 | ✅ 对话框关闭；dev 日志错误为 `[ImportNovel] 导入失败: Error: [Tauri 适配] 频道 **db:import-run-prepare-inspection** 尚未迁移（参数名未登记）` |
| 3 | G1 回归：作者原稿 + 两个 `.txt` | ✅ 显示拆章/作者原稿预览 |
| 4 | G1 回归：参考语料 + `.md` | ✅ 显示指向 G2b 的诚实错误 |

**第 2 项为何是强证据**：`launchRun` 先 `await loadAuthorImportChapterNumbers(...)`（内部调 `db:import-run-list-chapters`，G2a），
**成功后才**`createImportWorkflow` 并触发 `db:import-run-prepare-inspection`。错误出现在后者，说明前者已真实完成
（若 `-list-chapters` 未迁移，错误会直接指向它）。

证据：dev 日志**无 panic / 无 Rust error**，仅两条已知 `setZoomFactor`（阶段 3 项）占位提示与 Windows `EBUSY` 文件监视器噪声。

**残留处置**：夹具注入的 `g2a-smoke-*` 运行已用 `clean` 清除（`import_runs` 回到 0 行）；
dev 进程树已 `taskkill /T` 结束，无残留进程；用户目录 `%TEMP%` 已无任何本轮产物。

### 6. 收尾

- 提交清单：`feat(tauri): 批次 G2a 导入运行读面（3 频道 + 批次检查点单源）`。
- 规则变更：无。

---

## 本次更新（第三十七次：冒烟发现的三项缺陷修复 —— 弹窗 ESC / 窗口命令真实化 / 标题栏拖拽）

> 由用户在同日第十一轮鼠标实测验出。三项均为**上游/骨架遗留缺陷**，与迁移本身无关，
> 但均为用户可感知的功能缺失，已在本批修复。

### 1. 🐞 弹窗不响应 ESC（上游既有缺陷，根因定位到 Radix 源码）

**现象**：应用内所有 `ui/Dialog` 弹窗按 ESC 无任何反应；而**系统原生**（选目录）对话框 ESC 正常。

**定位过程（证据链，均来自实测日志）**：

1. `ImportNovelDialog.tsx` / `ui/Dialog.tsx` 与基线 `src/` **逐字节相同** → 不是迁移回归；
2. 临时 `console.warn` 诊断证明 **ESC 确实到达 DOM**（`{"key":"Escape","target":"DIV","role":"dialog"}`）
   → 排除 webview 焦点问题；
3. 读 `@radix-ui/react-dismissable-layer@1.1.19` 源码找到真因：
   ```js
   const isHighestLayer = node ? index === layers.length - 1 : false
   useEffect(() => { if (!isHighestLayer) return; /* 才注册 document keydown(capture) */ })
   ```
4. 实测诊断：`{"isTop":false,"stackLen":7}` —— **Radix 在弹窗关闭后仍保留 `DialogContent` 挂载**
   （退出动画走 `Presence`），叠加本应用同时渲染多个弹窗，`layers.length` 达 **7**，
   真正可见的弹窗**几乎永远不是最高层** → Radix 自身的 ESC 通道永久失效。

**修复（fork 侧健壮化，2 文件）**：

| 文件 | 改动 |
|---|---|
| `src/components/ui/Dialog.tsx` | `Dialog` 包装 Radix `Root` 并下发 `{ open, requestClose }` 上下文；`DialogContent` 自建**仅登记「真正打开」弹窗**的栈 + `window` capture 阶段 ESC 兑底；尊重 `onEscapeKeyDown` 的 `preventDefault()` 选退（修稿合并弹窗） |
| `src/components/settings/SettingsModal.tsx` | 该弹窗是**手写全屏弹层**（不走 `ui/Dialog`），补 ESC（与 `ClearProjectDataDialog` 既有先例同型：冒泡阶段 + 尊重 `defaultPrevented`） |

### 2. 🐞 窗口最小化 / 最大化 / 关闭按钮全部无响应（批次 A 骨架）

**根因**：`commands/window.rs` 的四个命令此前均为**假成功占位**（`// TODO` + 直接 `Ok(success: true)`），
调用「成功」但不做任何事——正是盘点文档警告的「频道登记齐全但实现为骨架」。

**修复**：按 `electron/controllers/window-controller.ts` 逐条真实化，并复刻**两段式关窗协议**：
- `window:minimize` → `window.minimize()`；`window:toggle-maximize` → 切换后回读 `is_maximized()`；
  `window:close` → `window.close()`；
- 新增 `WindowCloseGuard`（`commands/window.rs`，常驻 `AppState`）+ `lib.rs` 的 `on_window_event`：
  拦截 `CloseRequested` → `prevent_close()` + 广播 `window:close-requested{requestId}`；
  `window:resolve-close` 校验 requestId/decision，`proceed` 才置 `approved` 并再次 `close()`。
- **刻意偏离**：基线守卫在 `webContents.isDestroyed() || isLoadingMainFrame()` 时放行，
  Tauri 无对应概念（`CloseRequested` 只对存活窗口触发），仅保留 `approved` 一条。

### 3. 🐞 标题栏无法拖动窗口

**根因**：基线用 Electron 专有的 `-webkit-app-region: drag`（**WebView2 不支持**）。
**修复**：`TitleBar.tsx` 标题栏根元素补 `data-tauri-drag-region`（保留原样式以维持与基线可对比）。

### 4. fork 侧与基线的偏离登记（本轮 3 文件）

| 文件 | 偏离类型 | 理由 |
|---|---|---|
| `src/components/ui/Dialog.tsx` | 行为增强 | Radix 上游缺陷兑底（见上）；若上游日后修复需收敛 |
| `src/components/settings/SettingsModal.tsx` | 行为增强 | 手写弹层缺 ESC；与 `ClearProjectDataDialog` 先例对齐 |
| `src/components/layout/TitleBar.tsx` | 平台适配 | WebView2 需 `data-tauri-drag-region` |

> 背景：`tauri-app/src` 与基线 `src/` 本已存在 ~20 个偏离文件
> （`git diff --no-index --name-only src tauri-app/src`），故上述偏离符合既有做法；同步上游时需逐项核对。

### 5. 验证（第十二轮 GUI 冒烟，用户人工）

| # | 项 | 结果 |
|---|---|---|
| 1 | 标题栏【最小化】/【最大化】 | ✅ |
| 2 | 按住标题栏空白拖动 / 双击最大化 | ✅ |
| 3 | 「设置」弹窗 ESC 关闭（且其余 `ui/Dialog` 弹窗 ESC 仍正常） | ✅ |
| 4 | 标题栏【关闭】按钮 | ✅ 有响应且可关闭（⚠️ **未保存内容确认流程未测** —— 需 dirty 状态） |

自检：`cargo test --lib 586/586` · `cargo check --all-targets` 0 告警 · `cargo fmt --check` 干净 ·
`check:channels` 193/170/169/24 · `typecheck`/`lint` exit 0 · 定向 `vitest` 7/7。

### 6. 收尾

- 提交清单：`fix(tauri): 修复弹窗 ESC、窗口命令真实化与标题栏拖拽`。
- 规则变更：无。

---

## 本次更新（第三十八次：弹窗动画统一 + B22 补验 + 闪烁登记）

### 1. B22 补验 ✅（两段式关窗协议）

构造未保存内容（改章节不保存，标签页出现未保存标记）→ 点标题栏【关闭】：
① 弹出「未保存修改」确认框（保存并退出 / 放弃修改退出 / 取消）✅；
② 点【取消】→ 窗口保留、确认框消失 ✅；
③ 点【放弃修改退出】→ 应用正常退出 ✅。B22 解除。

### 2. 弹窗动画统一 + 修掉 0.4s 隐形延迟（真 bug）

**根因**：`index.css` 的 `--transition-spring: 0.4s cubic-bezier(0.175, 0.885, 0.32, 1.275)`
**自身带时长**，却被当作缓动函数插进 `animation` 简写：

```css
animation: dialog-enter 0.22s var(--transition-spring) both;
/* 展开 → dialog-enter 0.22s 0.4s cubic-bezier(...) both
   多出来的 0.4s 落在 animation-delay 位 → 先静置 400ms 再播 220ms */
```

配合 `both` 填充（静置期保持 0% 关键帧 = 透明 + 缩小），实测观感：
**设置弹窗 0.62s、Confirm 0.65s、AlertDialog 0.70s**；而 Radix 的 `ui/Dialog` 走 Tailwind
`duration-300` 无此问题 → 四类弹窗三种节奏（用户报的「不统一 + 设置弹窗太长」）。

**修复**：

| 项 | 内容 |
|---|---|
| 单源变量 | `index.css` 新增 `--dialog-{enter,exit}-duration` / `--dialog-backdrop-{enter,exit}-duration` / `--dialog-{enter,exit}-ease`（**时长与缓动严格分开**，并加注释禁止重犯）；进场 **170ms**、退场 **120ms** |
| 唯一实现 | 新增 `.lk-dialog-backdrop` / `.lk-dialog-panel`（`[data-state='closed']` 触发退场、含 `prefers-reduced-motion` 降级）；新弹窗只加这两个 class |
| `ui/Dialog.tsx` | 改用上述 class；定位从 `left-1/2 top-1/2 -translate-x/y-1/2` 换成 **flex 居中包裹层**（包裹层 `pointer-events-none`、面板 `pointer-events-auto`），否则关键帧的 transform 会抵消居中位移 |
| `Confirm` / `AlertDialog` / `SettingsModal` | 删内联 `animation`，改 `class` + `data-state`；三者时长/缓动/关键帧至此与 Radix 弹窗完全一致 |

`SettingsModal` 的退场采用「渲染期同步调整派生状态」而非 effect 内 `setState`
（后者触发 `react-hooks/set-state-in-effect`）。

**验证（第十三轮 GUI 观感，用户人工）**：设置弹窗明显变快 ✅、四类弹窗一致 ✅、
Radix 弹窗仍居中且尺寸正常 ✅。

### 3. 自检（本轮）

仅前端改动：`pnpm typecheck` / `pnpm run lint` 均 exit 0（实测）；**Rust 未改动**，
`cargo test 586/586` / `check` 0 告警沿用上一轮实测；`check:channels` 193/170/169/24（无频道变化）。

### 4. 收尾

- 提交：`61ea32ad`（本节的代码改动）+ 本快照；均已推送 `origin/master`。
- 规则变更：无。

---

## 交接给下次会话（**从这里接**）

### 1. 当前工作区状态

**工作区干净**（G1 全链与 G2 开工清单均已提交并推送 `origin/master`；本份快照的 G2a 章节为待生成提交的一部分）：

| Commit | 说明 |
|---|---|
| `fbb88307` | `feat(tauri): 批次 G1 作者原稿导入（dialog:select-novel-files + db:import-run-author-preview）` |
| `75379748` | `docs(tauri): 第三十五次快照与频道盘点更新（批次 G1 收口）` |
| `e2bb92cc` | `docs(tauri): 记录第十轮 GUI 冒烟（批次 G1 验收通过）` |
| `8edd1b46` | `docs(tauri): G2 开工清单（状态机细分 G2a/G2b）与决策归档` |
| `1a523707` | `feat(tauri): 批次 G2a 导入运行读面（3 频道 + 批次检查点单源）` |
| `9d12514d` | `docs(tauri): 第三十六次快照与频道盘点更新（批次 G2a 收口）` |
| `63f255b8` | `fix(tauri): 修复弹窗 ESC、窗口命令真实化与标题栏拖拽` |
| `3f3486f5` | `docs(tauri): 第三十七次快照 —— 冒烟三项缺陷修复` |
| `61ea32ad` | `fix(tauri): 统一弹窗进出场动画并修掉 0.4s 隐形延迟` |
| 待生成 | `docs(tauri): 第三十八次快照 —— 弹窗动画统一与 B22/B23/B24 归档` |

- HEAD（写入本表时）：`9d12514d docs(tauri): 第三十六次快照与频道盘点更新（批次 G2a 收口）`
- ⚠️ 上一份快照（2026-10-09）中**已过期的交接描述**（防照旧操作）：
  1. 「未迁移 29（`db=19 mcp=9 dialog=1`）」「已注册命令 165」「`cargo test — 542/542`」→ 均已变为 **27 / 167 / 572**；
  2. 「下一步 1：批次 G 从 G1 开始」→ **G1 已完成**，下一步是 G2；
  3. 「G1 刻意偏离 4 项」→ 实际落地 **D1–D8 共 8 项**（新增 D7 排序、D8 大小写）；
  4. 上一份快照的 §5 自检已冻结，**不得据其回填**本份数字。

### 2. 下一步（1-2-3）

1. ~~GUI 冒烟（G1 验收剩余项）~~ ✅ **已完成**（第十轮，2026-10-10，3 项全部通过 —— 见「本次更新 §5」）。
2. ~~批次 G2 开工清单~~ ✅ **已完成**（`8edd1b46`，含 G2a/G2b 细分）。~~G2a（读面 3 频道）~~ ✅ **已完成**（见第三十六次）。
3. **批次 G2b（写面 2 频道 + 复活 `reference` 路径）**：按
   [`docs-fork/plans/2026-10-10-g2-import-run-kickoff.md`](../plans/2026-10-10-g2-import-run-kickoff.md) §3.2 执行 ——
   新建 `src/import/identity.rs`（无密钥版 `resolveEncodedSources`）+ `import_run_repository.rs` 续写
   `begin_parsing` / `commit_parsed_source` / `fail_parsed_source` / `finalize_parsing` / `prepare` + 2 命令 +
   2 前端登记 + 删除 `commands/import.rs` 的 `reference` 诚实错误早退。**开工前先读清单 §4 的 ~620 行**。
4. **G3 / G4**：执行租约与批次推进 + effect receipts + `db:import-global-facts-commit`；
   `kb:import-reference-text` 去占位 + 前端登记。
5. **其它待办**：B13（导航防护）、B14（真 Windows 自更新）、H4（mcp，暂缓）、上游合并专项。

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
| **B22** | **两段式关窗的未保存内容确认未验证** | ⚠️ **新增（本次）**：`window:close` 现在会拦截并广播 `window:close-requested`，渲染层无 dirty 时直接 `proceed`；**dirty 分支（确认框 + cancel / 再次关窗）尚未实测**，需构造未保存内容后再验 |
| **B23** | **手写弹层无退出动画** | ✅ **已解除（本次）**：`SettingsModal` 已改为延迟卸载 + 统一进出场（`.lk-dialog-backdrop` / `.lk-dialog-panel`），第十三轮实测有淡出 |
| **B24** | **窗口最小/最大化后整屏瞬黑（闪烁）** | ⚠️ **新增（本次）·用户决定暂缓到专门批次**。现象：最小/最大化后鼠标在窗口内移动时**整屏瞬黑**（偶发）；**浏览器打开同一页面拖动不闪** → 壳层问题。已排查且排除：透明/effect 配置、常驻 `backdrop-filter`、resize 重渲染风暴、`backgroundColor` 缺失、`shadow:false`（实测无效已回滚）；事件日志无 TDR/dxgkrnl/DWM 错误。机器：AMD Radeon(2021‑11‑30 驱动) + RTX 3060 Laptop 混合显卡、单屏 2560×1440@**165Hz**、**FreeSync/VRR 开启**。候选方案：M3 给 `lorekeeper.exe` 指定单一 GPU ／ M4 临时 60Hz ／ A2 WebView2 `--disable-direct-composition` ／ M1 关 MPO（注册表，需审批+重启）／ M2 更新 AMD 驱动 |
| **B25** | **`ClearProjectDataDialog` 未纳入统一动画** | ⚠️ **新增（本次）**：该弹窗是手写全屏弹层且**当前无任何进出场动画**（关闭是硬切）；纳入统一需把它 3 处 `onClose()` 包成 `requestClose` 并加延迟卸载（同 `SettingsModal` 做法，约 15 行） |

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
| `node scripts/verify-channel-coverage.mjs --quiet` | `tauri-app/` | 契约 invoke 频道 **193**（事件频道 4）· 已注册命令 **170** → 覆盖 **169** · 未迁移 **24** `[db=15 mcp=9]` · 命令名与契约频道一一对应 ✅ |
| `cargo test --lib` | `tauri-app/src-tauri/` | `test result: ok. 586 passed; 0 failed; 0 ignored; 0 measured` |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | `Finished dev profile ... `（**0 告警**） |
| `cargo fmt --check` | `tauri-app/src-tauri/` | 输出 **0 行**（干净） |
| `pnpm typecheck` / `pnpm run lint` | `tauri-app/` | exit 0 / exit 0 |
| `npx vitest run test/channel-migration-coverage.test.ts test/ipc-arg-struct-contract.test.ts` | `tauri-app/` | `Test Files 2 passed`，`Tests 7 passed` |
| `git status --porcelain` | 仓库根 | **空**（G1 三提交 + G2 开工清单 + G2a 两提交均已推送 `origin/master`；本轮冒烟只改文档） |
| `git log -1` | 仓库根 | `9d12514d docs(tauri): 第三十六次快照与频道盘点更新（批次 G2a 收口）` |
| `pnpm tauri dev`（第十轮冒烟，G1） | `tauri-app/` | VITE `ready in 441 ms` · cargo `Finished dev profile in 47.50s` · `lorekeeper.exe` **90 MB** · 3 项人工验证全部 ✅ |
| `pnpm tauri dev`（第十一轮冒烟，G2a） | `tauri-app/` | VITE `ready in 812 ms` · cargo `Finished dev profile in 42.46s` · `lorekeeper.exe` **45 MB** · 4 项人工验证全部 ✅（唯一 console.error 为预期的 G2b 频道未迁移） |
| `pnpm tauri dev`（第十二轮冒烟，ESC/窗口修复） | `tauri-app/` | VITE `ready` · cargo 增量重建 · `lorekeeper.exe` **32 MB** · 4 项人工验证全部 ✅（窗口最小/最大化、标题栏拖拽、设置弹窗 ESC、关闭按钮） |

---

## 6. 历史指针（不复制正文）

| 日期 | 文件 | 内容 |
|---|---|---|
| 2026-10-09 | [`2026-10-09-tauri-migration-status.md`](./2026-10-09-tauri-migration-status.md) | 第三十四次（H3 update 域）及以前（G schema、L3、E、F2、H1/H2/B12…） |
| 2026-10-08 | [`2026-10-08-tauri-migration-status.md`](./2026-10-08-tauri-migration-status.md) | 第二十三～二十六次（F1 / F2 决策 / 批次 B 遗留补齐 / D2-b·D2-c） |
| 2026-10-07 | [`2026-10-07-tauri-migration-status.md`](./2026-10-07-tauri-migration-status.md) | 批次 D2-a 及以前 |
| 2026-10-06 | [`2026-10-06-tauri-migration-status.md`](./2026-10-06-tauri-migration-status.md) | 批次 A–C 细节 |
