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

## 快照（最后更新：2026-10-10 · 第三十九次）

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

## 本次更新（第三十九次：B25 收口 + G2b 解析写入面）

### 1. B25 ✅ `ClearProjectDataDialog` 纳入统一动画

该弹窗是手写全屏弹层，**此前零动画**（硬切）。不改各处调用点，而在组件内把
`onClose` 收口为带延迟卸载的包装（`onCloseProp` → `isExiting` → 200ms 后调真 onClose，
`exitTimerRef` 幂等），因此 ESC / 遮罩点击 / 清除成功后 / 取消按钮 四处入口共同获得动画。
**至此五类弹窗进出场实现完全一致**（Radix Dialog / Confirm / AlertDialog /
SettingsModal / ClearProjectDataDialog）。GUI 已验：淡入 + 淡出 ✅。

### 2. G2b 推进

| 步 | 内容 | 提交 |
|---|---|---|
| G2b-1 | `src/import/identity.rs`：无密钥版 `resolveEncodedSources`（D1′ / D3′），含 5 条测试 | `215b4520` |
| G2b-2 | `import_run_repository.rs` 解析写入面：`canonical_manifest` / `hash_manifest`（键序对齐 `JSON.stringify`）、`normalize_display/source_ids/source_fingerprints/chapters`、`parsed_source_status`、`begin_parsing`（三分支）、`commit_parsed_source`、`fail_parsed_source`，含 6 条测试 | 本次 |

**顺带**：`lib.rs` 的 `mod repositories;` → `pub mod repositories;`（与 `pub mod db` 一致）——
否则尚未被命令层消费的新仓储 API 会持续触发 `dead_code` 告警（之前靠逐项 `#[allow]` 缓解）。

**尚未做的 G2b 剩余**：`finalize_parsing`、`prepare`（author/reference 两分支）、
`matching_resumable_run` / `latest_completed_run` / `overlapping_resumable_source_run` /
`discard_provisional_parsing_run` / `fence_uncommitted_author_run` / `assign_stable_chapter_numbers` /
`completed_chapter_manifest` / `create_preparation_inspection`（均已在清单 §4 标注行号）
+ `db:import-run-prepare-inspection` / `-finalize-parsing` 两频道 + 前端登记 + 复活 `reference` 分支。

### 3. 自检（本轮）

`cargo test --lib` **597/597**（+6）· `cargo check --all-targets` **0 告警** ·
`cargo fmt --check` 干净 · `pnpm typecheck` / `lint` exit 0（B25 改动时实测）·
`check:channels` 193/170/169/24（无频道变化）。

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

## 历史指针（不复制正文）

| 日期 | 文件 | 内容 |
|---|---|---|
| 2026-10-09 | [`2026-10-09-tauri-migration-status.md`](./2026-10-09-tauri-migration-status.md) | 第三十四次（H3 update 域）及以前（G schema、L3、E、F2、H1/H2/B12…） |
| 2026-10-08 | [`2026-10-08-tauri-migration-status.md`](./2026-10-08-tauri-migration-status.md) | 第二十三～二十六次（F1 / F2 决策 / 批次 B 遗留补齐 / D2-b·D2-c） |
| 2026-10-07 | [`2026-10-07-tauri-migration-status.md`](./2026-10-07-tauri-migration-status.md) | 批次 D2-a 及以前 |
| 2026-10-06 | [`2026-10-06-tauri-migration-status.md`](./2026-10-06-tauri-migration-status.md) | 批次 A–C 细节 |
