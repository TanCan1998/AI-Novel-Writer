# Tauri 迁移进度快照（2026-10-08）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> **命名规则**：文件按日期命名 `YYYY-MM-DD-tauri-migration-status.md`，
> **一个工作日一个新文件**；当日新增内容只写入当日文件，跨日不回填旧文件
> （规则见 [`docs/agents/pi-development.md`](../agents/pi-development.md) §9）。
> 上一份快照（2026-10-07 冻结）：
> [`2026-10-07-tauri-migration-status.md`](./2026-10-07-tauri-migration-status.md)；
> 批次 A–C `project_core` / `characters` 的历史细节见
> [`2026-10-06-tauri-migration-status.md`](./2026-10-06-tauri-migration-status.md)；
> channel 级盘点见 [`docs/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-08 · 第二十五次）
| 项 | 值 |
|---|---|
| 仓库 | **`TanCan1998/Lorekeeper`**（原名 `AI-Novel-Writer`；仍为 `EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork） |
| 分支 | `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm 包 `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 当前阶段 | **批次 C 数据库层子域全部完成 ✅** + **批次 D1 ✅** + **批次 D2-a ✅** + **批次 D2-b ✅** + **批次 D2-c ✅（`llm:*` 收口）** + **批次 B 遗留补齐 ✅（`dialog:select-folder` 真实化）**：`project_core` / `characters` / `blueprints` / `drafts`（12/16）/ `revisions` / `reviews` / `post-process` / `llm 日志与摘要` / `project 清理` / **D1 模型管理** / **D2-a 租约** / **D2-b HTTP 生成链** / **D2-c 模型发现 + 连通性探测** / **目录选择弹窗**。**`llm:` 前缀下 14 个 invoke 频道已全部迁移**（不再有 `llm=` 未迁项）。依赖：`reqwest 0.13`（`default-features = false` + `native-tls` + `socks`）+ **`tauri-plugin-dialog 2`（本轮新增，Cargo.lock 锁 2.8.1）**；**`futures-util` / `tokio` 未引入** |
| 已注册命令 | **104**（A 11 + B 22 + C 子域 56 + D1 7 + D2-a 2 + D2-b 3 + D2-c 2 + **剩余骨架 1**）—— 骨架项由 `dialog:select-folder` **换为** `dialog:select-export-directory`（阻塞于批次 H 的 grant 域，见下） |
| GUI 冒烟 | ✅ **已做**（2026-10-07 起 **七轮**，末轮 2026-10-08 `pnpm tauri dev`）：窗口标题 `Lorekeeper`、vite@5190、cargo 390/390、`lorekeeper.exe` **内存 42.6 MB**（首轮）/ **30.1 MB**（D1 轮）/ D2-a 轮 vite `482 ms` / D2-b 轮 vite `468 ms` + cargo `24.99s` / D2-c 轮 `Running target\debug\lorekeeper.exe` + 内存 44.1 MB / **本轮（dialog）vite `453 ms` + cargo `24.91s` + `Running target\debug\lorekeeper.exe`**，均无 panic、渲染层 `ipc-client` 已联通 |
| 自动化回归 | `cargo test --lib` **310/310**（279 → +27 → **+4**；含 3 个**磁盘级**端到端：真实 `.vela/lorekeeper.db` + WAL + 外键 + 跨重开持久化）；`pnpm run check:channels` 校验契约↔命令映射（104 命令覆盖 103 invoke 频道，未迁移 90 → **88** 频道，`llm=` **已清零**，orphan 空）；`vitest` 相关用例 7/7 |
| 双栈隔离 | L0 安装标识 / L1 `~/.lorekeeper` / L2 `<root>/.vela/lorekeeper.db` 均独立；L3（`.vela` 改名）押后。D1 起 `~/.lorekeeper/{config.json,models.json,recent-projects.json}` 为**真实持久化**（此前 config 仅内存态）。本轮新增能力**均在 Rust 侧**，未触碰基线数据根 |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc @ `D:\Environment\rust\`（脚本内显式设 `RUSTUP_HOME`/`CARGO_HOME`）。`tauri-plugin-dialog 2.8.1` 要求 **rustc ≥ 1.90**（CI 最低版本需相应抬高） |
| 验证状态 | ✅ `cargo check --all-targets` **0 告警** · ✅ `cargo test`（全目标）**310/310** · ✅ `pnpm typecheck` exit 0 · ✅ `pnpm run lint` exit 0 · ✅ `check:channels` orphan 空 · ✅ 启动路径冒烟（本轮第七轮：无 `Command ... not found` / 无 panic） · ✅ **弹窗交互人工点验通过**（系统原生对话框正常弹出、不被遮挡、路径回填正确、取消静默） · ➕ 新增提交消息卫生检查（`scripts/check-commit-msg.mjs` + 单测 15 例 + `commit-message-ci.yml`） |

---

## 本次更新（第二十五次：批次 B 遗留补齐 —— `tauri-plugin-dialog` 接入，`dialog:select-folder` 真实化）

> 与第二十四次同属 2026-10-08（一个工作日内两次更新，按 §9 规则写入同一份当日文件）。

### 0. 缺口来源（**补录**：第二十四次快照漏记）

批次 B 交付时留下了一处**自洽但在 UI 上表现为「点了没反应」**的缺口：

- `dialog:select-folder` / `dialog:select-export-directory` 两频道在
  `src/shared/migrated-channels.ts` 中**已登记为「已迁移」**，`lib.rs` 中也已注册同名命令
  → 因此**不报「未迁移」提示**；但两个 Rust 命令当时是**恒返回 `None` 的骨架**，
  而 `None` 与基线的 `result.canceled` 同义 → 用户观感是「弹窗没出现 / 选了等于取消」。
- 同族的 `dialog:select-novel-files`（`ImportNovelDialog.tsx:198`）与
  `dialog:select-knowledge-files`（`knowledge-service.ts:137`）**既未登记也未注册**，
  属**批次 F / G**（需带 `ImportPurpose` 与 `projectSession` 语义），本轮**不动**。
- 定性：这**不是新功能开发**，而是「契约已声称迁移、实现却是占位」的**诚实性缺口**，
  故优先补齐，并借此把 dialog 能力一次性接好，供批次 F/G/H 复用。

### 1. 依赖决策（Ask first —— 已获用户授权）

| 方案 | 内容 | 结论 |
|---|---|---|
| **A（采纳）** | 只引 Rust crate `tauri-plugin-dialog`，前端**不加** `@tauri-apps/plugin-dialog` | ✅ 采纳 |
| B | 同时加 npm 包，前端 `import { open } from '@tauri-apps/plugin-dialog'` 直调 | ❌ 否决：需在 `capabilities/default.json` 放开 `dialog:allow-open` 等 ACL，**扩大 webview 攻击面**，且破坏「前端只经自研命令、不直调插件」的既有约定 |

**本机实测增量**（`cargo tree` 对照，Windows x64）：

| 项 | 值 |
|---|---|
| 版本 | `tauri-plugin-dialog 2.8.1`（`Cargo.toml` 写 `"2"`，`Cargo.lock` 锁 2.8.1） |
| 新增编译单元 | **8 个**（`tauri-plugin`、`tauri-plugin-fs`、`rfd 0.16.0`、`serde_repr`、`windows-sys 0.60.2`、`windows-targets` + 2 个 arch 包） |
| `Cargo.lock` 新条目 | **14 个**（含 aarch64/i686/gnu 等**其他目标平台**的 `windows_*` 变体，本机不编译） |
| 下载量 | ≈ 3.5 MB（**本轮为 0 下载**：crate 已在 `CARGO_HOME` 缓存） |
| release 二进制约 | +0.1–0.3 MB（社区实测 plugins-workspace#1085 为 +0.13 MB） |
| 常驻内存 | ≈ 0（无后台线程/轮询） |
| 编译耗时 | 增量首次约 **+20–40 s**（本轮 `cargo check` 全量 10.2 s，dev 轮 24.91 s） |
| 唯一真实成本 | `rfd 0.16` 把 `windows-sys` 锁在 **0.60**，与项目既有 `windows 0.62` **版本分叉**（各编一份）——由上游锁定，换方案也躲不掉 |
| 连带收益 | 硬依赖 **`tauri-plugin-fs 2.6.0`** 被一并引入 → **批次 H 的 fs 授权域将来直接复用，勿重复引入**（注意届时保持版本一致） |
| MSRV | 插件要求 **rustc ≥ 1.90**（本机 1.99.0 满足）；**CI 最低版本需相应抬高** |

`Cargo.toml` 保留插件默认 features（`gtk3`）：该 feature 只影响 **Linux/BSD** 的 rfd 后端，
Windows / macOS 无差异。

### 2. Rust 侧四处改动

| 文件 | 改动 |
|---|---|
| `src-tauri/Cargo.toml` | 新增 `tauri-plugin-dialog = "2"`（附决策注释）；按需清单中把 `fs` 从「待引入」移除 |
| `src-tauri/src/lib.rs` | `.plugin(tauri_plugin_dialog::init())`；命令注册注释更新（`select-folder` 真实化 / `select-export-directory` 仍骨架） |
| `src-tauri/src/commands/project.rs` | `dialog_select_folder()` **骨架 → 真实实现**；新增可复用纯函数 `file_path_to_string()`；新增 4 个单测 |
| `src-tauri/src/commands/external_file_grant.rs` | **不改行为**（仍返回 `None`），仅把「为何此刻仍不实现」的**理由写进模块头与函数文档** |

**`dialog_select_folder` 的关键实现取舍**（迁移自 `electron/controllers/project-controller.ts:801`
的 `showOpenDialog({ properties: ['openDirectory','createDirectory'] })`）：

1. **必须 `async`**：非 async 的 `#[tauri::command]` 在**主线程**执行，而插件文档明确
   `blocking_*` 族**禁止在主线程调用**（会与事件循环死锁）。改 `async` 后命令跑在
   `tauri::async_runtime` 线程上，主线程保持自由以驱动对话框消息循环。
2. **`spawn_blocking` + `recv_timeout(600s)`**：等待放在阻塞线程池；超时按「取消」(`None`)
   返回 —— 与基线 `result.canceled` 同义。之所以自己加超时：插件 `run_on_main_thread`
   的结果被 `let _ =` 丢弃，极端情形（主线程已退出）下其 `blocking_*` 会**永久阻塞**。
3. **显式父窗口**：基线的 `showOpenDialog` 默认以调用窗口为父；Tauri 侧需手动
   `set_parent(&window)`（标签 `main`，对齐 `tauri.conf.json`），否则对话框会被
   `decorations: false` 的无边框主窗口**遮挡**。
4. **返回值净化**：`file_path_to_string()` 把 `FilePath` 归一为字符串 ——
   `into_path()` 失败（如 Android `content://`）或结果为空串时一律按「取消」返回 `None`，
   **绝不回传无法使用的值**。该函数为 `pub`，供批次 F/G 的文件选择复用。

### 3. `capabilities/default.json` **未改动**（保持 `["core:default"]`）

Rust 侧内部调用插件 API **不经过 webview ACL**，故**无需**追加 `dialog:default` / `dialog:allow-open`
（这一判断修正了本轮开工前的预判方案）。同理 `migrated-channels.ts` 与前端 **零改动**：
频道集合未变（`dialog:select-folder` 本就是已登记频道），命令签名也未变（无参、返回 `Option<String>`）。

### 4. 为何 `dialog:select-export-directory` **仍**返回 `None`

其返回类型是 **`ExternalDirectoryGrant`（grantId + 展示名，绝对路径不得越界回传，ADR 0002）**，
而非路径 —— 签发 grant 需要 **grant 注册表**，该表与 `fs:grant-*` 三命令同在**批次 H**。
若此刻就地签发一个**假 grantId**，界面会显示「已选择导出目录」而后续写入必然失败，
**反而不如当前语义诚实**（`None` 与基线「用户取消」严格同义，`ExportDialog.tsx` 对 `null`
即静默返回，无契约偏差）。**弹窗能力本轮已就绪**：待批次 H 落地 grant 域后，
只需把选择结果喂给 grant 签发，插件侧无需再改。

### 5. 验证与测试

| 检查 | 结果 |
|---|---|
| `cargo check --all-targets` | **0 告警**（10.20 s） |
| `cargo test --lib` | **310/310**（306 → +4） |
| `pnpm typecheck` | exit 0 |
| `pnpm run lint` | exit 0（`--max-warnings 0`） |
| `pnpm run check:channels` | 104 命令、未迁移 **88**（不变）、orphan 空 |
| GUI 冒烟（第七轮） | vite `453 ms` @5190 → cargo `Finished dev profile in 24.91s` → `Running target\debug\lorekeeper.exe`；**无 panic**、无编译告警；仅历史 `[ipc-client] setZoomFactor … Tauri …` 警告；进程树已清理 |

**新增 4 个单测**（`commands/project.rs`，全部为纯函数、无 GUI 依赖）：
Windows 绝对路径透传、**空路径按取消处理**、`file:///F:/…%20…` **百分号解码**、
`content://` 等非 `file://` URI **拒绝转为路径**。

> ✅ **人工点验已完成**（2026-10-08 会话末尾）：在 `pnpm tauri dev` 中验证了系统原生对话框正常弹出、不被无边框主窗口遮挡、选择后路径正确回填表单、点击取消静默无报错。自动化部分（编译通过 + 命令能注册 + 纯函数语义正确 + 单测 4/4）也已全绿。

### 6. 交接给下次会话（**从这里接**）

**当前工作区状态**：`master` 上有 **5 个未提交的已修改文件**（`Cargo.lock` / `Cargo.toml` /
`lib.rs` / `commands/project.rs` / `commands/external_file_grant.rs`），全部属于本轮 dialog 接入，
**自检全绿**（见 §5），**尚未提交**。

**接续步骤（建议顺序）**：

1. ✅ **提交本轮** —— 已完成（`f892e47` / `e74ce13` / `b0fbf89` / `7e47b5a` / `dc0ce27`）。
2. ✅ **人工点验弹窗** —— 已完成（见上）。
3. **再决定下一步批次**（二选一，与原建议一致）：
   - **批次 E**（定稿不可逆 + 删除生命周期）—— 需**先获批**在 `schema.rs` 新增 3 张表：
     `recovery_candidates`、`continuity_projection_meta`、`chapter_deletion_operations`；
   - **或批次 F**（16 频道，零新依赖）。
4. **`dialog:select-export-directory` 真实化随批次 H 一起做**（不要单独提前做，理由见 §4）。

**⚠️ 待授权的决策点（阻塞项）**：`recovery-candidate`（4 频道）需在 `schema.rs` 新增
`recovery_candidates` 表（+ 状态索引）—— **此授权用户尚未答复**（此前的「按照建议」只针对 dialog）。

**红线提醒（每次接手都要过一遍）**：Tauri 侧禁止读 `AI_NOVEL_VELA_HOME`、禁止回退 `~/.vela`、
禁止写 `.vela/vela.db`；项目库 = `<root>/.vela/lorekeeper.db`；全局数据根 =
`AI_NOVEL_LOREKEEPER_HOME` 或 `~/.lorekeeper`；失败文案按基线 MUTATING 规则带 `"Error: "` 前缀
（`commands/db.rs::mutating_error`）；前端只经自研命令、**不直调插件 API**。

### 7. 收尾：提交、历史修正与规则变更

**提交清单**（已推送 `origin/master`）：

| 提交 | 内容 |
|---|---|
| `867cbba` | `feat(tauri)`: 批次 D2-c — 模型发现 / 连通性探测，`llm:*` 收口 |
| `d4a9ff3` | `docs(tauri)`: 第二十四次快照 — 批次 D2-c 完成 |
| `f892e47` | `feat(tauri)`: 接入 `tauri-plugin-dialog`，`dialog:select-folder` 返回真实目录 |
| `e74ce13` | `docs(tauri)`: 第二十五次快照 — 批次 B 遗留补齐 |
| 本轮追加 | `ci`: 提交消息卫生检查（脚本 + 单测 + workflow）；`docs`: 快照收尾与规范记录 |

**历史修正（BOM 事故）**：`867cbba` / `d4a9ff3` 的前身（`7105b13` / `612002b`）
提交消息首 3 字节带 UTF-8 BOM（`EF BB BF`），在**推送前**用 `git commit-tree` 重放剥除
（`tree` / 作者 / 提交者 / 时间戳逐字节保留，仅 SHA 变化），因此本轮先本地重写再一次性推送。
事故成因、后果与正确写法见 [`docs/agents/pi-development.md`](../agents/pi-development.md) §10.1。

**规则变更**：

- 新增 [`scripts/check-commit-msg.mjs`](../../scripts/check-commit-msg.mjs)：BOM / CRLF / 行尾空白 /
  语义化前缀检查，支持 hook 单文件模式与 CI `--range` 模式（后者读 `git cat-file commit`
  **原始字节**，而非 `git log` 的渲染结果）；配 15 例单测
  [`scripts/__tests__/check-commit-msg.test.ts`](../../scripts/__tests__/check-commit-msg.test.ts)。
- 新增 [`.github/workflows/commit-message-ci.yml`](../../.github/workflows/commit-message-ci.yml)：
  push 检查 `before..after`、pull_request 检查 `base..HEAD`，**刻意不设 `paths-ignore`** ——
  本次事故正好发生在 docs-only 提交上，若挂在 `pr-ci.yml` 下会被其 `paths-ignore` 漏掉。
- `docs/agents/pi-development.md` 新增 §10「Git 提交消息规范与提交前自检」。
- 根 `AGENTS.md`「Git 工作流」补充 BOM 硬约束与自检命令
  （该文件被 `.gitignore` 忽略，属本地 agent 指引，不入库）。

**工作区清理**：脚本无残留产物；测试仓库用 `mkdtemp` 创建并在 `afterEach` 删除。

---

## 上一次更新（第二十四次：批次 D2-c — `llm:*` 收口：模型发现 + 连通性探测 2 频道）

### 0. 范围与依赖

`llm:*` 前缀下最后 2 个未迁频道：

| 频道 | 命令 | 基线来源 |
|---|---|---|
| `llm:discover-models` | `llm_discover_models` | `electron/services/model-discovery-service.ts`（204 行） |
| `llm:test-connection` | `llm_test_connection` | `llm-controller.ts` handler + `electron/embedding.ts`（调用面） |

**零新增依赖**：D2-b 引入的 `reqwest` 直接承担全部网络职责；基线的
`AbortController` 15s 总超时改由 `reqwest::Client::timeout` 表达（同样是
「含 body 读取」的总时长语义），因此**不需要** `tokio` 直接依赖，也无需
`futures-util`。测试亦不依赖异步运行时（见 §3 的纯函数切分）。

### 1. `llm/discovery.rs`（新，204 行基线的逐支复刻）

| 项 | 对齐基线 |
|---|---|
| `resolve_openai_models_url()` | 剥 `/chat/completions` → 剥 `/chat` → 空路径回落 `/v1` → 拼 `/models`；清 query/fragment/凭据 |
| `resolve_gemini_models_url()` | `/v1beta/models` 原样 / `/v1beta` 补 `/models` / 其余补 `/v1beta/models` |
| `url_contains_credential()` | 原文 → `encodeURIComponent` 形态 → `decodeURIComponent` 后形态三档命中；空凭据恒假；解码失败按未命中 |
| `safe_provider_text()` | ≤512 **字节**（`Buffer.byteLength` = UTF-8 字节）、无控制字符（`<=0x1f` 与 `0x7f..=0x9f`）、`trim` 后非空、不含凭据 |
| `parse_openai_models()` / `parse_gemini_models()` | 条目上限 500；**任一条目非法 → 整体 `invalid_response`**（不是跳过该条）；`id` 去重后保序 |
| `classify_http_failure()` | 401/403 → `auth`；408/425/429/5xx → `network`；其余 → `unsupported` |
| `interpret_discovery_response()` | 空列表 → `empty`；载荷坏 → `invalid_response`；body 读取失败分「超时 → network / 否则 invalid_response」 |

**两处刻意的环境差异**（均已注释）：

1. **重定向语义**：基线 `redirect: 'error'` 会让 fetch **reject**（→ `network`）；
   Rust 侧用 `redirect::Policy::none()`，若直接落进 `classify_http_failure(302)`
   会得到 `unsupported`（**语义错误**）—— 故在 `fetch_models()` 中**先**判
   `status.is_redirection()` 并强制归为 `network`。这条差异是最容易写错的一处。
2. **URL 规范化**：`URL.toString()` vs `url::Url::to_string()` 在极端形态上可能
   有百分号编码差异；端点语义一致，已在测试中固定关键形态。

### 2. `llm/embedding.rs`（新，**刻意收窄**的迁移范围）

`electron/embedding.ts` 共 360 行，本批次**只迁移调用面**：

| 基线片段 | 迁移 | 理由 |
|---|---|---|
| `embedOpenAI` / `embedGemini` / `generateEmbeddings` | ✅ | `llm:test-connection` 的 embedding 分支 |
| `validate*Embeddings` / `validateEmbeddingVectors` | ✅ | 响应合法性就是「连接可用」判据 |
| `ollamaOpenAIEmbeddingBaseUrl` / `buildOpenAIEmbeddingUrl` | ✅ | URL 构造属调用语义 |
| `releaseSmokeEmbeddings` | ❌ | Electron 安装包冒烟专用（需环境变量 + 唯一 argv 令牌） |
| `chunkText` | ❌ | 文本分块属 RAG 侧（批次 F），本批次无调用面 |
| `normalizeEmbeddingOptions` 的 `batchSize` 归一 | ❌ | 仅在显式传入 `configuredBatchSize` 时生效，当前无调用面 |

**`String(error)` 的三档前缀逐字复刻**（含一个此前未注意到的细节）：

- `new Error(msg)` → `Error: msg`；
- `EmbeddingResponseValidationError` → **`EmbeddingResponseValidationError: `**（不是 `Error: `！
  该类在构造器里把 `this.name` 改成了类名，`String(error)` 走 `name: message`）；
- `fetch` 失败的 `TypeError` → Rust 侧无法逐字复刻异常类名，退化为 `Error: …`（已注明）。

同时复刻了两个**容易被简化掉**的细节：

1. Gemini 侧基地址只去**一个**尾斜杠（`replace(/\/$/, '')`），OpenAI 侧去**全部**
   （`replace(/\/+$/, '')`）—— 两者不可混用，已各自单测；
2. OpenAI 响应校验在「条目缺 `embedding`」时 `continue` 后**仍会汇总**
   「`index` 覆盖不完整」错误，最终文案是 `A；B` 拼接（测试逐条固定该拼接结果）。

### 3. `commands/llm_management.rs`（新，2 频道）

| 频道 | 行为 |
|---|---|
| `llm:test-connection` | 先同步决议探测参数（失败即返回 `Error: ` 文案）→ 构建客户端 → `purposes` 含 `embedding` 走 Embedding 端点，否则发一条 `Say "hello" and nothing else.` 探测 |
| `llm:discover-models` | 构建 `Policy::none()` + 15s 超时的客户端 → 交 `llm::discovery::discover_models` |

**探测与生成链共用同一策略缝**：`maxTokens = 1024`（`CONNECTION_TEST_MAX_TOKENS`，
推理模型需要足够预算才不会被误判为截断失败）、`reasoningStage = 'general'`、
`conversationId` **恒为 `None`**（探测不是创作会话，不得共享网关粘性）——
三条都有测试锁定。

**测试可离线运行的关键切分**：把「请求前同步判定」抽成纯函数，避免为单测引入
异步运行时：

- `discovery::resolve_discovery_request()`（端点解析 + 凭据回显守卫）；
- `discovery::interpret_discovery_response()`（传输结果 → 结果档位）；
- `llm_management::resolve_connection_options()`（参数决议，含 Kimi 温度校验）；
- `embedding::embedding_batches()`（批量切分）。

### 4. 小重构（消除第二份定义）

- `is_gemini()` 上移到 `llm/chat.rs` 作为单源；`commands/llm_generation.rs` 的私有
  副本删除并改为引用（其测试保留，注释标注实现单源位置）；
- `llm/chat.rs` 新增 `build_client_with_timeout()`，`build_client()` 改为它的
  `None` 特例（行为不变，仍是「无总超时」）。

### 5. 接线

- `llm/mod.rs`：新增 `pub mod discovery; pub mod embedding;`；
- `commands/mod.rs`：新增 `mod llm_management;` + glob 再导出；
- `lib.rs`：注册 2 命令（102 → **104**）；
- `src/shared/migrated-channels.ts`：由 `pnpm run check:channels:emit` 重新生成（103 频道）；
- `src/services/ipc-client.ts`：登记 `llm:test-connection` → `['model','creativeStrategy']`、
  `llm:discover-models` → `['request']`（渲染层实际恒传 2 个实参，`creativeStrategy` 有 `?? 'auto'` 兜底）；
- `test/channel-migration-coverage.test.ts`：未迁移样本改指 `kb:search` / `update:get-state`，
  并断言 2 个新频道已迁移。

### 6. 验证与测试

| 检查 | 结果 |
|---|---|
| `cargo check --all-targets` | **0 告警** |
| `cargo test --lib` | **306/306**（279 → +27） |
| `pnpm typecheck` | exit 0 |
| `pnpm run lint` | exit 0（`--max-warnings 0`） |
| `pnpm run check:channels` | 104 命令、未迁移 **88**（90 → -2，`llm=` 清零）、orphan 空 |
| `vitest`（频道覆盖 + 发现边界） | 7/7 |
| GUI 冒烟（第六轮） | `Running BeforeDevCommand (pnpm dev)` → `Running DevCommand (cargo run …)` → `Running target\debug\lorekeeper.exe`，内存 44.1 MB，无 panic / 无编译告警；进程树已清理 |

新增单测覆盖：端点解析全分支（含 `/chat/completions` / `/chat` / 空路径 / query+fragment+凭据清除 / 非法 URL）、
凭据回显三档形态、`safe_provider_text` 六类拒绝、OpenAI/Gemini 载荷严格解析（含条目上限与去重）、
HTTP 分类五档、传输解释六分支、结果序列化的键存在性（成功不含 `errorCode`、失败不含 `models`）、
Ollama 原生 `/api` 九类不命中形态、Embedding URL 推断七例、Gemini 单斜杠剥除、
向量维度/有限性校验、OpenAI `index` 覆盖性与拼接文案、Gemini 顺序语义、
错误文案三档前缀、批量切分、探测参数意图、探测温度（档案唯一来源 / Kimi 固定采样省略）、
Kimi 越界文案、探测不带会话粘性、探测复用已验证推理指令。

---

## 更早更新（第二十三次：批次 D2-b — LLM 生成 / 流式 / 取消 3 频道 + 3 事件）

### 0. 依赖决策（推翻了「`default-tls` = native-tls」的假设）

抽 `reqwest 0.13.5` 权威 feature 表后确认：`default = ["default-tls", "charset", "http2", "system-proxy"]`，
而 **`default-tls = ["rustls"]`** —— 0.13 起默认 TLS 后端已是 rustls，不再是 native-tls。
rustls 路径必拉 `aws-lc-rs`，本机无 VS/CMake 构建链 → 必然编译失败。

最终声明：

```toml
reqwest = { version = "0.13", default-features = false, features = ["json", "stream", "native-tls", "socks"] }
```

- `Cargo.lock` **+18 个包**（TLS 家族），Windows 走 `schannel`（纯 FFI，**无** `aws-lc-rs` / `ring`）；
- 首次引入 `reqwest` 触发真实 crates 下载并编译成功；
- 刻意**不**启用：`charset`（基线 Node `res.text()` 恒按 UTF-8 解码）、`http2`
  （Node fetch/undici 为 HTTP/1.1）、`system-proxy`（代理在代码内显式装配）；
- **未引入 `futures-util`**（原计划项）：取消改用 `tauri::async_runtime::JoinHandle::abort()`，
  零新依赖达成基线 `AbortController` 的效果（见 §4）。

### 1. `src-tauri/src/llm/chat.rs`（新，共享层）

| 项 | 对齐基线 |
|---|---|
| `strip_thinking()` | `OpenAIProvider.stripThinking()`；三条规则的**顺序**与**首匹配/全局**语义逐字同构（规则 1 全局 `replace_all`、规则 2/3 只替换首个），大小写不敏感、跨多行 |
| `proxy_from_config()` | `applyProxyConfig()` 的判定顺序：`enabled === true` **且** host 非空；`type === 'socks5'` 走 socks5、其余 http；配置读取失败静默 `None` |
| `build_client()` | 基线 `fetch` **无总超时**（长文生成可跑数分钟），故不设 timeout；仅装配显式代理 |
| `provider_error_text()` | 复刻 `String(error)` 的 `"Error: "` 前缀，**复用** `commands::db::mutating_error`，不造第二份格式定义 |
| 共享类型 | `LlmFinishReason`（6 档 snake_case）、`TokenUsage`（三项 `Option`）、`LlmResponse`（`camelCase` + `skip_serializing_if`）、`LlmGenerateOptions`、`ChatMessage`、`StreamFailure`（`Cancelled` 与 `Message` 分开） |

### 2. `src-tauri/src/llm/openai.rs`（新）

- `build_request_body()`：`temperature` 为 `None` 时**整键省略**（不得回退 `model.temperature`）；
  NovelAI 走窄兼容载荷（不吃 `reasoning_effort` / `response_format` / `stream_options`）；
  DeepSeek 专有 `thinking: { type }` + `enabled` 时才带 `reasoning_effort`；
  `stream && !isNovelAI` 才带 `stream_options.include_usage`；`max_tokens` 在两侧都缺时**不出现** `null` 占位。
- `build_request_headers()`：opencode Go 判据（`https` + `opencode.ai` + 路径 `/zen/go` 前缀）成立时补
  `x-opencode-session`（缺失会话退化为 **单请求 UUID**，绝不与创作运行共享粘性）与 `User-Agent`。
- `OpenAiSseDecoder`：纯状态机（无网络依赖，可单测）—— CRLF 切断与尾缓冲、`data:` 多行拼接、
  注释行忽略、`[DONE]` 终止、`reasoning_content` → `<think>\n…` 包裹、`content` 到达时补 `\n</think>\n\n`、
  仅三字段皆为数才收 `usage`、7 类 typed fatal error（损坏 JSON / 非对象 / `error` 载荷 / choices / choice / finish_reason / delta / content 类型）。
- `generate()`：常返回信封（不 `Err`）；非 2xx → 文案 `API 调用失败 ({status}): {text}`；
  非 `stop` 终态 → `success: false` + `API 返回的文本未正常完成`。
- `generate_stream()`：三态回调；`!sawDone` → `响应流在完成标记前结束，生成结果不完整`；
  推理块未闭合时补 `\n</think>\n\n`；失败一律走 `fail()`，把**已交付的可见候选**（空则不给）交回调用方。

### 3. `src-tauri/src/llm/gemini.rs`（新）

`system` 消息提升为 `systemInstruction`、`assistant` → `model` 重命名、
URL 用字符串拼接（`{base}/v1beta/models/{name}:generateContent` / `…:streamGenerateContent?alt=sse`，
与基线模板字面量一致，不做 URL 规范化）；SSE 解码器**宽松解析**（坏行跳过而非 fatal，CRLF 尾缓冲、
`null` finish_reason → `unknown`）；**Gemini 不剥离推理块**（prompt 不走 `<think>`，与 OpenAI 分道）。

### 4. `commands/llm_generation.rs`（新，3 频道 + 3 事件）

| 频道 | 行为 |
|---|---|
| `llm:generate` | **总是**返回信封（不 reject）；两档失败语义**不可统一** |
| `llm:generate-stream` | 立即 `{requestId, started}`，后台任务把三回调翻译为事件 |
| `llm:cancel` | 记录 `cancelled` 统计 → 交付终态事件 → `abort()` 任务 |

**两档失败语义**（基线字面量分支，刻意不统一）：

- **模型不存在**：`llm:generate` 文案 `未找到模型配置` / `llm:generate-stream` **不带** `error` 字段，均**无** `Error: ` 前缀；
- **异常路径**（租约解析失败 / Kimi 温度越界 / messages 非法 / 客户端构建失败）：走 `provider_error_text`（带前缀）。

**调用统计**（`llm_calls` 表）三重守卫，逐条对齐 `recordProviderOutcome`：
无会话上下文 → 丢弃；会话与当前活跃项目不匹配 → 丢弃；落库失败 → **吞掉**
（诊断信息绝不改变生成结果）。且统计上下文只在**模型快照解析之后**才产生 ——
对齐基线 `catch` 里的 `if (model) recordProviderOutcome(...)`（租约解析失败时 `model` 仍为 null，不留痕）。

**取消的语义补齐（本轮真实缺口）**：基线靠 `AbortSignal` 让 provider 抛 `AbortError` 后自行 `fail()`，
渲染层 `llm-store` 正是靠 `llm:stream-done` / `llm:stream-error` 才执行 `cleanup()`；
而 Rust 任务被 `abort()` **硬终止、不走任何回调** —— 若只置探针，UI 会永久停在 `running`。
故本轮引入：

- `StreamSnapshot`（`Mutex<String>` 已交付全文 + `Mutex<Option<TokenUsage>>`）：
  取消时用它派生 `stripThinking(fullText)` 与 `usage`，与基线 `fail()` 的入参同源；
- `emitted` 终态事件一次性闸门：取消方先占位再 `abort()`，任务侧回调不得重复推送；
- `task: Arc<Mutex<Option<StreamTask>>>`：`llm_cancel` 借此 `abort()` 掉挂起中的
  `chunk().await`（基线由 `AbortController` 关闭连接达成同等效果），避免连接与任务永久残留。

### 5. 与基线的**刻意差异**（均已在代码注释注明）

| # | 基线 | Tauri 侧 | 影响 |
|---|---|---|---|
| 1 | `llm:generate-stream` 的参数决议在 `try` 外 → handler **reject** | 返回 `{started:false, error:"Error: …"}` 信封 | 前端两条路径都会 `cleanup()` + `onError` + throw，**行为等价**且更贴契约 |
| 2 | 同 `requestId` 重发直接覆盖 `activeStreams`（旧流不中止） | 三闸门置位 + `abort()` 旧任务 | 修掉旧流继续推送事件/记账的隐患 |
| 3 | `messages` 无运行时校验（TS 类型） | serde 校验，非法结构记失败统计并返回信封 | 更严格 |
| 4 | 取消 → `AbortError` → `onError('已取消生成')` | `abort()` + 命令层主动补发同名终态事件 | 等价，且保证 `cleanup()` 执行 |
| 5 | `usage` 中间态为 `undefined`（structured clone 保留） | 序列化为 `null` | `?? 0` 类消费无差别 |

### 6. 接线

- `commands/mod.rs`：`mod llm_generation` + glob 再导出；
- `lib.rs`：注册 3 命令（99 → **102**），并**移除** D2 期遗留的 `#[allow(dead_code)] mod llm;`
  （D2-b 接通后推理映射/生成参数的调用面已真实使用，注解按原计划撤除，仍 0 告警）；
- `src/shared/migrated-channels.ts`：登记 3 频道；
- `src/services/ipc-client.ts`：`CHANNEL_ARG_NAMES` 补登 `llm:generate` / `llm:generate-stream` / `llm:cancel`；
- `test/channel-migration-coverage.test.ts`：前置拦截用例改指 `llm:discover-models`，并断言 3 个新频道已迁移。

事件无需前端登记：`ipc.on(channel, …)` 直通 `tauriListen`，与 Electron 版语义一致。

### 7. 验证与测试

| 检查 | 结果 |
|---|---|
| `cargo check --all-targets` | **0 告警** |
| `cargo test --lib` | **279/279**（243 → +36） |
| `pnpm typecheck` | exit 0 |
| `pnpm run lint` | exit 0（`--max-warnings 0`） |
| `pnpm run check:channels` | 102 命令、未迁移 **90**（93 → -3，`llm=` 剩 2）、orphan 空 |
| GUI 冒烟（第五轮） | vite `468 ms` @5190 → cargo `24.99s` → `Running target\debug\lorekeeper.exe`，无 panic，渲染层 `ipc-client` 已联通；进程树已清理 |

新增单测覆盖：`<think>` 三规则的顺序/首匹配语义、代理四重守卫、URL 分支、opencode Go 判据、
`normalize_finish_reason` 词表映射、`temperature` 整键省略、SSE 解码器（CRLF / 多行 data / 坏 JSON /
类型错误 / `[DONE]` / usage 收集 / 多字节切断）、Gemini 载荷形状与宽松解码、两档失败信封、
统计上下文产生时机、事件载荷 `camelCase` 契约字段、`StreamSnapshot` 派生可见候选、取消文案无 `Error: ` 前缀。

---

### 8. 收尾（工作区卫生 + 快照命名规则）

- **提交** `1bcae45`（`chore: 忽略 pi 会话数据目录（.pi/）`）：`.pi/` 新增忽略
  （内含 `agent/auth.json` 凭据、`tmp/reqwest.index` 临时索引近 1 MB）；目录**保留在磁盘**，
  仅忽略 —— 删除会导致登录态与缓存丢失，该点已在 `.gitignore` 注释中写明。
- **清理**：删除 5 个上一轮会话调试产物（`tauri-app/{chk,emit}.txt`、
  `src-tauri/{check_err,test2,test_err}.txt`）与 2 个早期冒烟轮次遗留日志
  （`tauri-app/dev.{out,err}.log`）；清理后 `git status` 干净。
- **快照命名规则确立**（**本文件即首个按新规则产生的快照**）：交接快照按日期命名
  `docs/handoffs/YYYY-MM-DD-tauri-migration-status.md`，**一个工作日一个新文件**；
  当日章节只写当日文件，跨日**不得回填**旧文件（旧文件视为该日期的冻结快照）。
  规则出处：[`docs/agents/pi-development.md`](../agents/pi-development.md) §9。
  随之完成一次**结构迁移**：本文件承接 2026-10-08 产生的全部内容（快照表 + 第二十三次
  章节 + 滚动的收口/下一步/遗留项），`2026-10-07-*.md` 回退为 `7f0ecc1` 的冻结状态
  并加上前向指针。

## 批次 C 收口状态（2026-10-07）

**已完成子域**（命令累计 **90**，其中 C 子域 56）：

| 子域 | 频道 | 最新 commit |
|---|---|---|
| project_core | 4 | `ff7fbd8` `a8d742a` |
| characters / roster | 3 | `69fc50c` |
| blueprints | 11 | `50c0fe8` `1332216` `84a3100` |
| drafts | 12/16 | `6351b2d` |
| revisions | 9 | `5e79ea7` |
| reviews | 5 | `fa07875` |
| post-process | 6 | `39ee0c2` |
| llm 日志 / 摘要 | 5 | `e0e1004` |
| project 清理 | 1/2 | 本轮 |
| **合计** | **56** | |

**剩余 `db:*` 频道全部归属后续批次**（不是 C 的欠账）：

- **批次 E**：continuity（4）、finalization-link（1）、drafts 余 4
  （`authority-sequence` / `export-snapshot` / `export-authority-current` / `import-finalized-batch`）；
- **批次 F**：一致性豁免（3）、叙事线程（6）、派生树（3）、恢复候选（3）；
- **批次 G**：import-run（18）、`db:import-global-facts-commit`。

---

## 建议的下一步

1. **🖱️ 先做弹窗人工点验**（本轮唯一未完成的验收项，见第二十五次 §5）：
   `pnpm tauri dev` → 新建项目 → 「选择文件夹」，验三条（弹出/回填/取消静默），
   结果补回快照。顺手提交本轮的 5 个改动文件。
2. **⚙️ 再补 G5 压测**：`llm:stream-chunk` 已接通但「高频 chunk 跨 webview 桥的吞吐/延迟」
   仍无实测数据。建议写一个不带网络的本地回放（例如直接循环 `emit` N 次）测出 chunk/s 上限，
   再决定是否需要批量合并（inventory §G5 已预留该选项）。
3. **双栈同库行为对照**：同目录下 Electron（`vela.db`）与 Tauri（`lorekeeper.db`）各写各库，
   确认互不影响；顺带对照 `~/.vela/config.json` 与 `~/.lorekeeper/config.json` 的读写形态差异。
4. **`vitest` 全量超时定位**（遗留项 7）。
5. **批次 E**（定稿不可逆 + 删除生命周期，ADR 0003/0011 等量测试）：`finalization`（2，需补契约 G1）、
   `chapter-lifecycle`（4）、`continuity`（4）、`recovery-candidate`（4）、
   `drafts` 余 4（`authority-sequence` / `export-snapshot` / `export-authority-current` /
   `import-finalized-batch`）、`finalization-link`（1）。
   **⚠️ 阻塞**：需先获批在 `schema.rs` 新增 `recovery_candidates` / `continuity_projection_meta` /
   `chapter_deletion_operations` 三张表。
6. **或批次 F**（16 频道，零新依赖：一致性豁免 3 / 叙事线程 6 / 派生树 3 / 恢复候选 3 +
   `kb:*` 需先定 LanceDB 取舍）。该批次可顺带用本轮新增的 `file_path_to_string()`
   落地两个文件选择频道（`dialog:select-novel-files` / `dialog:select-knowledge-files`）。
7. **`dialog:select-export-directory` 真实化** 与 **`fs:grant-*` 三命令** 一并归入**批次 H**
   （`tauri-plugin-fs 2.6.0` 已随本轮连带引入，届时**勿重复添加**）。

---

## 遗留项（沿用 2026-10-06 快照）

- ✅ **批次 B 的 dialog 骨架缺口已补齐（第二十五次）**：`dialog:select-folder` 由「恒返回 `None`
  的占位」改为**真实原生目录选择**（`tauri-plugin-dialog 2`，纯 Rust 侧，capabilities 未放开），
  **人工点验已通过**（系统对话框正常弹出、不被遮挡、路径回填正确、取消静默）。
  **补录**：该缺口此前**未被记录**在任何快照里 —— 因为频道登记齐全、命令已注册，
  表面无异常，仅表现为「点了没反应」；排查此类问题时须**核对命令实现是否为骨架**，
  而非只看 `check:channels` 的 orphan 是否为空。
- ⚠️ **`dialog:select-export-directory` 仍为取消骨架**（**有意为之**，非欠账）：弹窗能力已就绪，
  阻塞点是**批次 H 的 grant 签发**（须回传 `grantId` 而非绝对路径，ADR 0002）。
  详见第二十五次 §4。
- ⚠️ **`dialog:select-novel-files`（`ImportNovelDialog.tsx:198`）与
  `dialog:select-knowledge-files`（`knowledge-service.ts:137`）既未登记也未注册** → 批次 F/G
  （需带 `ImportPurpose` 与 `projectSession` 语义），本期未动。
- ⚠️ **CI 最低 Rust 版本需抬高到 ≥ 1.90**（`tauri-plugin-dialog 2.8.1` 的 MSRV）；
  本机 1.99.0 不受影响。
- ✅ **批次 C GUI 实机验证（自动化部分已完成）**：三轮 `pnpm tauri dev` 冒烟通过；
  核心读写链路已由 `disk_e2e.rs` 用真实 `.vela/lorekeeper.db` 断言覆盖。
  **仍未人工验证**：界面交互本身（按钮触发、表单回显、错误提示的 UI 形式）。
- ✅ **遗留项 12 已修复**（`24c008f`）：未迁移频道给出统一友好提示 + 生成物一致性测试。
- ✅ **config 持久化缺口已补齐**（第二十一次，批次 D1）：`config:get/set` 由内存态改为
  `~/.lorekeeper/config.json` 真实持久化；`models.json` / `recent-projects.json` 同步走原子写。
- ✅ **批次 D2 依赖已批准并落地**（第二十三次，批次 D2-b）：`reqwest 0.13`
  （`default-features = false` + `native-tls` + `socks`）；**`futures-util` 未引入**（改用 `JoinHandle::abort()`）。
- ⚠️ **G5（流式事件经 webview 桥的性能实测）仍未做**：D2-b 已把 3 个事件接通，
  但「高频 chunk 下跨 IPC 桥的吞吐/延迟」尚无实测数据；D2-c 也未触及该缝
  （探测与发现均为低频调用）—— 建议在批次 E 前补一次压测。
- ⚠️ **D2-c 刻意未移植的基线片段**（均无调用面，非欠账）：`embedding.ts` 的
  `releaseSmokeEmbeddings`（Electron 安装包冒烟专用）、`chunkText`（属 RAG 侧）与
  `normalizeEmbeddingOptions` 的 `batchSize` 归一；若批次 F 接入知识库，
  `chunkText` 与 `batchSize` 归一需随该批次一并落地。
- 未验证：双栈同库行为对照、`vitest` 全量超时定位、`cargo fmt --check` 未纳入验收
  （`src-tauri/` 全域存在 rustfmt 差异，需单独提交）。
- 押后：L3（`.vela` → `.lorekeeper`）、可见品牌（`brand.ts` / i18n 标题）。
- 批次 G 需要的 `BlueprintRepository.getCommittedRangeOperation`（无 IPC 频道，被
  `import-run-repository.ts` 使用）尚未移植，随批次 G 一并落地。
- 无 IPC 频道的基线辅助方法未移植：`post_process_repository.get_failed_step_labels`、
  `DraftRepository.clearAll`（后者功能已被 `project_clear` 覆盖）。
- 已知取舍（D1）：`models.json` 内容非数组对象（如合法 JSON 对象）时，基线会把它当数组
  误用并写出怪异结果，Rust 侧改为**拒绝覆盖**（更严格，语义差别只在文件被外部破坏时出现）；
  `delete-model` 遇损坏文件时基线报 JS `SyntaxError` 原文，Rust 侧统一为
  `Error: 模型配置损坏，已拒绝覆盖`。
