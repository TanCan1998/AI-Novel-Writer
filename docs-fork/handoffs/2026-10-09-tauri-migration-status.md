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

## 快照（最后更新：2026-10-10 · 第三十四次）

> 本表只填**最新一次自检的实测值**。改表前必须重跑对应命令，不得沿用旧数字、不得估算。
> 本轮实测命令与输出见下方「[§5 自检记录](#5-自检记录2026-10-09-实测)」。

| 项 | 值 |
|---|---|
| 仓库 / 分支 | **`TanCan1998/Lorekeeper`**（`EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork）· `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 已注册命令 | **165** |
| 覆盖 invoke 频道 | **164**（契约总数 193，事件频道 4） |
| 未迁移 invoke 频道 | **29**（`db=19 mcp=9 dialog=1`） |
| orphan | **空** ✅ |
| `cargo test --lib` | **539/539** ✅ |
| `cargo fmt --check` | **干净（0 差异）** ✅ 已纳入验收（第三十三次全量格式化） |
| `cargo check --all-targets` | **0 告警** ✅ |
| `pnpm typecheck` / `lint` | exit 0 / exit 0 ✅ |
| 定向 `vitest` | **33/33**（8 文件：契约覆盖 / 入参结构体契约 / 源码契约 / locale / ipc-client 会话 / finalization-client / finalization-snapshot / writing-skills）✅（既有 2 个失败已修） |
| 已完成批次 | A ✅ / B ✅（含遗留补齐） / C ✅ / D1 ✅ / D2-a ✅ / D2-b ✅ / D2-c ✅ / **E ✅** / **F1 ✅** / **L3 ✅** / **F2 ✅** / **G1 ✅** / **H1 ✅** / **H2 ✅** / **H3 ✅（update:*）** / **B12 ✅** |
| 当前阶段 | **批次 H 基本收口**：H1（fs:grant-* / 导出目录）、H2（prompt:* / skills:*）、**H3（update:* 6 频道 + `update:state`）**、B12（打开外部链接）已完成；**H4（mcp）暂缓**（用户决定）。下一步：① **批次 G**（import-run 19 + `dialog:select-novel-files`，20 频道 → 收口 `kb:import-reference-text`）；② H4 或上游合并专项（502 提交）；③ B13（渲染层导航防护）、B14（真 Windows 自更新） |
| 依赖 | `reqwest 0.13`（`default-features=false` + `native-tls` + `socks`）、`tauri-plugin-dialog 2`（锁 **2.8.1**）、**`tauri-plugin-opener 2.7.0`**（第三十三次 B12 新增，连带 `open 5.4.4`；Ask first 已批准）。**F2 无新增依赖**：`hnsw_rs 0.3.4` / `jieba-rs 0.7.0` / `tokio` 已在 `Cargo.toml`；**FTS5 由 `libsqlite3-sys` bundled 提供** |
| GUI 冒烟 | ✅ 自 2026-10-07 起 **九轮**。**第九轮（2026-10-09，批次 H 前三项 + B12）**：vite `502 ms` + cargo `47.95s` → `lorekeeper.exe`（90 MB），**KB 界面导入→搜索→stats→删除全链路 ✅、导出成稿（合并 md + 分章 md）✅、官方主页/模型资源链接真实打开 ✅、提示词保存/删除与技能列表 ✅**（用户人工逐项验证，事后库/磁盘状态已复核）。近两轮：dialog 轮 vite `453 ms` + cargo `24.91s`；**第八轮（2026-10-09，G1）**：vite `533 ms` + cargo `1.33s`（增量）→ G1 定稿（两章 outbox `published` + `.txt` 落盘且标题剥离）+ B2 删除（两章实体稿真实删除、KB 文档真实清理）全部通过，过程中发现并修复 1 个入参契约缺陷（见第三十二次 §2） |
| 双栈隔离 | **L0/L1/L2/L3 全部独立**：安装标识 / `~/.lorekeeper` / `<root>/.lore/`（库 `.lore/lorekeeper.db`、KB 向量 `.lore/kb/`）。基线为 `~/.vela` / `<root>/.vela/`。**两栈项目目录刻意不互通**（`ee40aaab`） |
| Rust 工具链 | rustc/cargo **1.99.0 stable-msvc** @ `D:\Environment\rust\`（脚本内须显式设 `RUSTUP_HOME` / `CARGO_HOME`）。`tauri-plugin-dialog 2.8.1` 要求 **rustc ≥ 1.90**（CI 最低版本需相应抬高） |

<sub>*命令与频道差额：**165 命令**中 1 个为阶段 0 骨架（`app_health_check`，不对应任何契约频道），其余 **164** 与 invoke 频道一一对应。</sub>

---

## 本次更新（第三十四次：批次 H3 —— `update:*` 6 频道 + `update:state` 事件）

> 承接第三十三次，跨日到 **2026-10-10**。本批按用户确认的**零依赖方案**落地更新域；
> **未迁移 35 → 29**（`update=6` 全部收口），批次 H 基本收尾（仅剩 H4 暂缓）。

### 0. 决策（用户确认）

| 决策 | 选择 |
|---|---|
| **更新源** | **fork `TanCan1998/Lorekeeper`**（与 B12 官方主页一致；基线指向上游 `EthanYoQ/AI-Novel-Writer`） |
| **Windows 打包版策略** | **全平台 `updateAction = 'open-release'`**：Windows 也能查到新版本并打开 Release 页（不引入 `tauri-plugin-updater`） |

### 1. 交付（提交 `70d114d8`）

新增 `src/update/`（9 文件，与基线文件一一对应）+ `commands/update.rs` + 前端登记。

| 模块 | 平移自 | 要点 |
|---|---|---|
| `update/types.rs` | `src/shared/update-types.ts`（111 行） | 类型全集镜像（8 态 / 11 错误码 / 6 阶段 / 17 reason / 18 项脱敏细节），serde 重命名逐字对齐 |
| `update/version.rs` | `update-service.ts:61-69,175-186` | 稳定三段版本比较（`v` 前缀 / `+build` / 预发布一律不可比较） |
| `update/time.rs` | `update-service.ts:71-76,495` | 日历日 + ISO8601 解析（复用 Hinnant 算法）；project.rs 的 `iso8601_utc_from_millis` 改为复用同一 `civil_date_from_days`（消除重复） |
| `update/service.rs` | `update-service.ts`（585 行） | 状态机：严格更高版本才算可用、自动检查每天一次、手动可绕过、自动失败只留主进程、偏好不可写则**不自动联网**、可用更新/延后提醒叠加；锁纪律（状态锁不跨 `.await`，检查/下载用 `tokio::sync::Mutex` 串行化 = 基线 `checkQueue`） |
| `update/backend.rs` | `github-release-update-backend.ts` + 错误分类器 | GitHub Releases 元数据（只读）+ `DisabledBackend`；分类器 403/404/429/proxy/tls/network/unknown（network 桶补充 `dns` 以适配 reqwest 文案） |
| `update/preferences.rs` | `update-preferences-store.ts`（45 行） | 写全局配置 `updatePreferences`：**读宽容 / 写严格**，配置损坏时绝不覆盖用户其他设置 |
| `update/runtime.rs` | `update-runtime.ts` | 复刻 `isPackaged + devServerUrl` 门禁；集中记录两处刻意偏离（`UPDATE_ACTION` / `update_configuration_available`） |
| `update/startup.rs` | `update-startup.ts`（73 行） | 装配与降级：初始化失败→「更新不可用」；启动后 fire-and-forget 自动检查，**绝不阻塞启动** |
| `commands/update.rs` | `update-controller.ts`（54 行） | 6 频道信封（失败不 reject）；`days` 宽容强制（非 7/30 → `INVALID_REMINDER_DELAY`） |

**事件**：每次状态变更经注入的 `publish` 闭包 `app.emit("update:state", …)`（对应基线 `subscribe → publish`）。

### 2. 刻意偏离（已评估，需在 B14 复活基线行为时逐条改回）

1. **不引入 `tauri-plugin-updater`** → 后端恒为 GitHub-Release 只读元数据；
2. **`updateAction` 全平台 `open-release`** → `update:download` / `update:quit-and-install`
   诚实返回 `DOWNLOAD_NOT_READY` / `INSTALL_NOT_READY`（**不伪造已下载**）；
3. **更新源指向 fork**（基线指向上游）；
4. **无 `app-update.yml` 等价物** → `updateConfiguration` 恒 `available`（我们确实有可用的 GitHub-Release 配置）；
5. **日历日节流用 UTC**（基线为本地时区）—— 仅跨零点边界偏移，节流语义不变。

### 3. 验证（本轮实测）

`cargo test --lib` **539/539**（510 → +29）；`cargo check --all-targets` **0 告警**；`cargo fmt --check` **干净**（452 → 0）；
`pnpm typecheck` / `lint` **exit 0**；`check:channels` **193 契约 / 165 命令 / 164 覆盖 / 29 未迁移**，orphan 空；
定向 vitest **8 文件 / 33 测试全过**。

新增单测覆盖：类型序列化与枚举重命名、版本比较边界、ISO8601 互转、错误分类六桶、
偏好读写（缺失/损坏/保留其他键）、平台门禁、状态机（disabled / 配置缺失 / 可用更新+延后提醒 /
不可用清理 / 自动节流 / 分类失败的分模式差异 / 偏好不可写阻断联网 / 下载与安装诚实未就绪 /
open-release 注入闭包 / 每次变更都发布快照）。

### 4. 未做 / 后续

- **GUI 冒烟未做**：dev（`tauri::is_dev()` 为真）下门禁关闭 → `update:get-state` 应为 `disabled`、
  `update:check` 返回 `UPDATES_DISABLED`（与基线一致）；真正的检查/打开 Release 页需**打包版**验证。
- **B14**（真正 Windows 自更新）与 **H4**（mcp）仍待做；**批次 G** 为下一批。

---

## 本次更新（第三十三次：批次 H 前三项 —— H1 外部文件授权真实化 / H2 应用数据域 / B12 打开外部链接）

> 承接第三十二次，同属 **2026-10-09**。本批完成 **H1 + H2 + B12 + 遗留项 L**，
> 并产出 H3/H4 依赖评估；**未迁移 42 → 35**，多条旧阻塞项（B3/B5/B8/B10/B12）全部解除。

### 0. 决策（用户确认）

| 决策 | 选择 |
|---|---|
| 本轮范围 | 批次 H 全部 + 遗留项；依赖策略 = 先评估再单独确认 |
| **H3（update）** | **零依赖 GitHub-Release 版**（基线自带可替换后端：Windows 已安装包走 electron-updater，其余平台走 GitHub Release 元数据；`downloadUpdate()` 返回空、`quitAndInstall` 为 no-op）；真正的 Windows 自更新另立专项（需签名密钥） |
| **B12（打开外部链接）** | **引入 `tauri-plugin-opener`**（一次修好 official-homepage / model-provider-resource / update:open-release） |
| **H4（mcp）** | **本轮暂缓**（基线仅 stdio、SSE 明确未实现；用户决定改做批次 G） |

### 1. H1：外部文件授权域真实化（提交 `2f055519`）

解除 **B10**（KB 界面导入 / 导出成稿 / 角色卡导入全部不可用）与 **B3**。

- `external_grant.rs`：+`EXPORT_GRANT_TTL/MAX_USES`、`issue_directory_with_operations`（导出目录需 `write`+`create`）、
  `resolve_target`（相对路径纯校验 + 词法归一 + 存在祖先 canonical 双容器校验，防 junction/symlink 逃逸）、
  `assert_safe_relative_path`（拒绝 NUL / 绝对路径 / 盘符 / 父目录遍历）。
- **语义修正**：授权用尽由「归零即删除」改为「保留记录、后续消费报已用尽」——对齐基线 `resolveRequest`，
  且是 `fs:grant-write-file` 消费 `write` 后仍需 `revalidate('create')` 的前提。
- `commands/external_file_grant.rs`：read（消费 read + **64 MiB 上限** + 严格 UTF-8）、write（消费 write → 探测 create →
  同目录临时文件 + ready 阶段复检 + rename 提交点，失败提交态 `not_committed`/`unknown`）、mkdir、
  导出目录选择（tauri-plugin-dialog `pick_folder` → 签发 `write`+`create` 授权 → `{grantId, displayName}`）；
  错误文案四桶分桶，绝不回传绝对路径。

### 2. H2：应用数据域（提交 `099c757b`）

- 新增 `commands/app_data.rs`（**7 频道**）与 `writing_skills.rs`（`src/shared/writing-skills.ts` 的 Rust 单源）。
- `prompt:load/save/delete-global`：`~/.lorekeeper/prompts/<key>[.<lang>].json`；文件名与内部标识一致性校验、
  逐文件诊断收集、zh-CN 保存/删除同步清理 legacy 无后缀文件。
- `skills:list-user` / `uninstall-user`：受信根校验（拒绝符号链接根/目标）、SKILL.md 容器校验、单条无效不阻断其余。
- `skills:inspect-github` / `install-github`：先检查后安装（内存一次性确认缓存 + 安装前**重新抓取核对**
  `contentSha256`/`resolvedUrl`）、仅接受自包含提示词、同名拒覆盖；地址解析支持 `github.com` 的 repo/tree/blob
  与 `raw.githubusercontent.com`，路径段 percent-decode 后须为安全段且落到 `SKILL.md`。
- `writing_skills.rs`：frontmatter 解析、语言/阶段归一、建议阶段启发式、**六类不兼容原因**、GitHub raw URL 编码。
- 失败信封口径：`prompt:save/delete` 带 `Error: ` 前缀；`skills:*` 与 `prompt:load-global` 的 diagnostics 不带前缀。

### 3. L：遗留项（提交 `3397f23c` + `2b45c707`）

- **B8 解除**：`ipc-client-project-session.test.ts` 改为注入 `window.__TAURI_INTERNALS__.invoke` 桩
  （而非已废弃的 `window.velaAPI`），真实覆盖频道映射 / 命名参数 / 会话注入；断言只比前两位参数
  （真实 core.invoke 会透传第三个 `options`）。
- **B5 解除**：`cargo fmt` 全量格式化 **66 文件**（`cargo fmt --check` 现已干净，纳入验收）。
  ⚠️ **未改 CI**：把 fmt 加入 CI 属「改 CI/发布配置」（Ask first），需单独批准。

### 4. B12 + 依赖评估（提交 `ee4a3f07` + `df9ceb75`）

- 新增评估文档 `docs-fork/research/2026-10-09-h3-h4-dependency-evaluation.md`。
- **B12（新发现）**：`official-homepage:open` 与 `model-provider-resource:open` 是**假成功占位**
  （返回 `success:true` 却不打开任何 URL，代码内为 TODO），且此前**未登记**为骨架。
- 新增 `tauri-plugin-opener 2.7.0` + `external_link.rs`（仅接受 https 常量链接，仅由 Rust 侧调用；
  渲染层未引入 `@tauri-apps/plugin-opener`，故 **capabilities 未开放 opener 权限**——权限只门禁渲染层→插件命令）。
- 官方主页指向 fork 仓库（`TanCan1998/Lorekeeper`）——**刻意偏离基线**（基线指向上游），因本 fork 产品身份为 Lorekeeper。
- 后续项（新记）：基线还有「拒绝渲染层导航替换主框架」（`preventRendererNavigation`）的等价防护，
  Tauri 侧 `on_navigation` / 新窗口拦截**尚未接入**（见 B13）。

### 5. 验证（本轮实测）

`cargo test --lib` **510/510**（491 → +19）；`cargo check --all-targets` **0 告警**；`cargo fmt --check` **干净**；
`pnpm typecheck` / `lint` **exit 0**；`check:channels` **193 契约 / 159 命令 / 158 覆盖 / 35 未迁移**，orphan 空；
定向 vitest **8 文件 / 33 测试全过**。

### 6. GUI 冒烟（第九轮，2026-10-09）—— ✅ 通过

测试项目 `F:\Temp\loretest\lore-smoke`（清单 + 播种的**已定稿**章「雾港的灯语」+ 真实 `.txt`）；
参考文本 `F:\Temp\loretest\reference\雾港设定.txt`。用户逐项人工验证：

| 项 | 结果 |
|---|---|
| **A. KB 界面导入全链路（B10 核心）** | ✅ `dialog:select-knowledge-files` → **`fs:grant-read-file`**（此前卡此处）→ 材料列表 → 导入 → 搜索「灯语/回声兽/林晚」召回 → `kb:stats`/vectorless 计数 > 0 → 删除文档 |
| **B. 导出成稿（H1 的 `fs:grant-write-file` / `mkdir`）** | ✅ 「选择导出目录」弹出**真实系统目录选择框**（此前恒为取消）；合并 Markdown 与分章 Markdown（新建 `<项目名>-<uuid>/` 目录 + 逐章写入）均成功 |
| **C. 打开外部链接（B12）** | ✅ 官方主页真实打开 `https://github.com/TanCan1998/Lorekeeper`；设置里 SiliconFlow 资源链接可用（此前为假成功） |
| **D. 应用数据域（H2）** | ✅ 全局提示词列表加载 + 保存/删除；技能列表（`skills:list-user`）正常 |

事后库/磁盘复核：定稿章与 outbox `published` 完整；`kb_documents` / `kb_chunks` 归零（对应步骤 6 的删除）；
`.lore/prompts` 已被应用创建；`~/.lorekeeper/{prompts,skills}` 与预期一致（已删除/为空）。

### 7. 本轮未做 / 下一步

- **H3（update 6 频道 + `update:state` 事件）** —— 下一步（零依赖 GitHub-Release 后端）。
- **批次 G（import-run 19 + `dialog:select-novel-files`，20 频道）** —— 收口 `kb:import-reference-text`。
- **GUI 冒烟已做（第九轮 ✅）**：H1 解开的 KB 导入 / 导出成稿 / 链接打开与 H2 的应用数据域均已实测通过。

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

### 6. GUI 冒烟夹具与残留处置（2026-10-09 收尾，用户确认「保留可复用的并记入文档，其余删除」）

**保留（下次 GUI 冒烟可直接复用，零 API）**：

| 保留物 | 位置 | 用途 |
|---|---|---|
| 现成 `.lore/` 测试项目 | `F:\Temp\loretest\lore-smoke\` | 含 `.lore/project.json` + 已建库（当前 0 草稿）；可直接打开 |
| KB 导入样本 | `F:\Temp\loretest\reference\雾港设定.txt` | 知识库导入/检索用参考文本 |
| 播种与取证脚本 | `F:\Temp\loretest\seed_draft.py` / `seed_ch2.py` / `inspect*.py`（4 个） | Python 3 + sqlite3 直接读写 `.lore/lorekeeper.db`：播种草稿、只读取证（无 API 也能造出可定稿的草稿） |
| 假生成模型「222」 | `C:\Users\tanca\.lorekeeper\models.json` | 内置 OpenAI 预设（`gpt-4o-mini` + 未连通 baseUrl + 占位 key），仅用于通过生成运行时的「默认模型」前置校验 |
| 最近项目条目 | `C:\Users\tanca\.lorekeeper\recent-projects.json` | 保留 `lore-smoke`，方便一键重开 |

⚠️ **`config.json` 的 `defaultModelId` 已恢复为 `null`**（不把假模型留在默认位，避免正常使用时误发请求得 401）；下次冒烟时在「设置 → 模型配置」把「222」设为默认即可。若已配置真实模型，建议直接删掉「222」。

**已删除**：`tauri-dev*.log` / `tauri-dev*.err.log`（含 ANSI 的 dev 日志，4 个）、`msg1..3.txt`（提交消息中间文件）。

**进程**：`pnpm tauri dev` 进程树已全部停止（含 vite / cargo-watch / `lorekeeper.exe`），释放 `.lore/lorekeeper.db` 文件锁。

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

**工作区干净**。最近两轮（第三十三～三十四次）的 9 个提交（均在 `master`）：

```
70d114d8 feat(tauri): 迁移应用更新域（update:* 6 频道 + update:state 事件）
d8d54832 docs(tauri): 新增 H3（update）开工清单
06b282d4 docs(tauri): 记录第九轮 GUI 冒烟（批次 H 前三项 + B12 全部实测通过）
b77bfee4 docs(tauri): 第三十三次快照（批次 H 前三项 + 遗留项 L）
df9ceb75 feat(tauri): 真实化「打开外部链接」能力（tauri-plugin-opener）
ee4a3f07 docs(tauri): H3/H4 依赖评估与决策（零新依赖 + B12）
2b45c707 style(tauri): cargo fmt 全量格式化 src-tauri（66 文件）
3397f23c test(tauri): 修复 2 个过期 vitest（ipc-client 会话传输）
099c757b feat(tauri): 迁移应用数据域（prompt:* 3 + skills:* 4）
2f055519 feat(tauri): 真实化外部文件授权域（fs:grant-* 三命令 + 导出目录授权）
```

> 历史备注：第二十七次的 5 个 commit message 里附带过「建议提交拆分（5 主题）」表；实际提交已按该表完成。

### 2. 下一步（1-2-3）

1. **批次 G**（当前最高优先）：import-run 19 频道 + `dialog:select-novel-files`（共 **20** 频道 →
   未迁移 29 → 9）；落实后收口 `kb:import-reference-text`（当前为显式占位失败）。
   注意执行租约 `ImportRunExecutionLease` 与断点恢复语义。
2. **H4（mcp 9 频道）**：已暂缓（用户决定）；方案已评估——基线仅 stdio（SSE 明确未实现），
   `std::process` + 自研守卫即可零依赖（见
   [`docs-fork/research/2026-10-09-h3-h4-dependency-evaluation.md`](../research/2026-10-09-h3-h4-dependency-evaluation.md)）。
3. **其它待办**：① **B14** 真正的 Windows 自动更新（需 `tauri-plugin-updater` + 签名公钥 + 打包链路）；
   ② **B13** 渲染层导航防护（`on_navigation` / 新窗口拦截）；③ 上游合并专项（502 提交，冲突面仅根目录 4 个文件）。
   GUI 冒烟（可选）：H3 在 dev 下应为 `disabled`（基线语义），真正的检查/打开 Release 页需**打包版**验证。

### 3. 阻塞项与待授权项（不得删除，须逐条确认后更新）

| # | 项 | 状态 |
|---|---|---|
| B1 | G1 定稿频道 | ✅ 完成（第三十二次） |
| B2 | `chapter:*` 物理清理 | ✅ 全部解除（F2-3 删 KB 文档 + 第三十二次删实体稿） |
| B3 | `dialog:select-export-directory` | ✅ **已真实化**（第三十三次 H1） |
| B4 | L3 双栈隔离 | ✅ 已执行 |
| B5 | `cargo fmt --check` | ✅ **已解除**（第三十三次全量格式化；⚠️ 但**未加入 CI**，改 CI 仍需 Ask first） |
| B6 | `tauri-app` 全量 `pnpm test` | ⚠️ 仍超时；已有结论：凡未 mock `ipc-client`/未注入 `__TAURI_INTERNALS__` 的流程测试在 node 下会失败（既有，非本轮） |
| B7 | 「证据不在正文中」反例 | ⚠️ 未验证（无正文数据） |
| B8 | 2 个既有 `vitest` 失败 | ✅ **已解除**（第三十三次：改为注入 `__TAURI_INTERNALS__` 桩） |
| B9 | `tauri-plugin-dialog 2.8.1` 要求 rustc ≥ 1.90 | ⚠️ CI 最低版本需相应抬高 |
| B10 | `fs:grant-*` 占位阻塞 KB 导入/导出 | ✅ **已解除**（第三十三次 H1） |
| B11 | `chapter:delete-finalized` 入参缺 camelCase | ✅ 已修复（第三十二次）+ 源码扫描防线 |
| B12 | 「打开外部链接」缺失（两个假成功占位） | ✅ **已解除**（第三十三次：`tauri-plugin-opener`） |
| B13 | 渲染层导航防护未接入 | ⚠️ 基线有 `preventRendererNavigation` + 新窗口拦截；Tauri 侧 `on_navigation` / 新窗口拦截**尚未接入**（第三十三次记录） |
| B14 | 真正的 Windows 自动更新 | ⚠️ 需 `tauri-plugin-updater` + 签名公钥 + 打包链路；本轮 H3 只做 GitHub-Release 元数据方案 |
| B15 | H4（mcp 9 频道） | ⚠️ **用户决定暂缓**；方案已评估（仅 stdio，`std::process` + 自研守卫） |

### 4. 红线提醒（每次接手都要过一遍）

- 🚫 Tauri 侧**禁止**读 `AI_NOVEL_VELA_HOME`、**禁止**回退 `~/.vela`、**禁止**写 `.vela/vela.db`。
- 项目库 = `<root>/.lore/lorekeeper.db`；KB 向量 = `<root>/.lore/kb/`；全局数据根 = `AI_NOVEL_LOREKEEPER_HOME` 或 `~/.lorekeeper`。
- 失败文案按基线 MUTATING 规则带 `"Error: "` 前缀（`commands/db.rs::mutating_error`）；但 `skills:*` 与 `prompt:load-global` 的 diagnostics 按基线**不带**前缀。
- 定稿不可逆：`finalization:` 相关改动一律 **Ask first**。
- 兜底约定：新增命令入参结构体必须带 `#[serde(rename_all = "camelCase")]`（`pi-development.md` §2.5 + 源码扫描测试）。
- ⚠️ **F2 隔离红线已解除（`ee40aaab` L3）**：Tauri 项目目录 `.lore/`、向量快照 `.lore/kb/`，不再有互覆可能。
- **更新域约定**：`update:*` 的后端恒为 GitHub-Release 只读元数据（B14 前），`updateAction` 恒 `open-release`；
  不得伪造「已下载/已安装」，`defer-reminder` 写入全局配置时**必须保留**用户其他设置。
- 提交消息**无 BOM / 无 CRLF / 无行尾空白**（2026-10-08 出过 BOM 事故）。

---

### 5. 自检记录（2026-10-10 实测，第三十四次）

> 生成本表快照时在本机实跑，命令与输出如下。**下次更新快照表必须先重跑这些命令。**

| 命令 | 工作目录 | 实测输出 |
|---|---|---|
| `pnpm run check:channels` | `tauri-app/` | 契约 invoke 频道 **193**（事件频道 4）· 已注册命令 **165** → 覆盖 **164** · 未迁移 **29** `[db=19 mcp=9 dialog=1]` · 命令名与契约频道一一对应 ✅ |
| `cargo test --lib` | `tauri-app/src-tauri/` | `test result: ok. 539 passed; 0 failed; 0 ignored` |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | `Finished dev profile ... in 4.20s`（0 告警） |
| `cargo fmt --check` | `tauri-app/src-tauri/` | 输出 **0 行**（干净） |
| `pnpm typecheck` / `lint` | `tauri-app/` | exit 0 / exit 0 |
| `npx vitest run`（8 文件定向） | `tauri-app/` | `Test Files 8 passed`，`Tests 33 passed` |