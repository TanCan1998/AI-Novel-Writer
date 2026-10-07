# Tauri 迁移进度快照（活文档）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档。**每次迁移工作完成后必须更新本文件的快照区块**。channel 级细节见 [`docs/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。
> 本文件为日期化交接快照（docs/README.md 治理规则）；后续大节点可另立日期文件，勿回写历史快照。

---

## 快照（最后更新：2026-10-06 · 第六次）

| 项 | 值 |
|---|---|
| 分支 | `master` |
| 基准 SHA | `a0fd2f4`（阶段 0 锚点提交；后续批次 A–H 以此为起点） |
| 当前阶段 | **批次 B 完成（接线 + 验证全绿）**；批次 A 已提交 |
| 结构 | **独立迁移根 `tauri-app/`**（用户确认的方案 B），根目录三文件已还原上游原样 |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc @ `D:\Environment\rust\`（遵循 AGENTS.md 第 3 条规则）；crates 走 rsproxy 镜像；VS Build Tools 已装；**脚本内需显式设 RUSTUP_HOME/CARGO_HOME** |
| 未提交变更 | ✅ **批次 B 已完成待提交**：4 个新 Rust 文件 + state/lib/mod/ipc-client 四处接线；`cargo test --lib` 28/28、`cargo check --all-targets` 0 告警、typecheck/lint/build 全绿 |
| 残留死文件 | ✅ 已删除（上会话残留 `state/commands.rs`、`commands/config/internal_exports.rs`） |
| 未验证事项 | ① `pnpm tauri dev` 窗口冒烟未做（GUI 需手动验证）；② 批次 B 与 Electron 版行为对照未做；③ vitest 全量超时未定位；④ `cargo fmt --check` 未达标（批次 A/B 均未过 rustfmt，未纳入验收） |

### 结构重设计（2026-10-06 第二次更新，用户确认）

- **根目录** = Electron 基线（`package.json`、`vite.config.ts` 已 `git checkout` 还原上游原样；`src/`、`electron/` 不动）→ 上游合并冲突面归零。
- **`tauri-app/`** = 迁移项目根：独立 `package.json`（无 electron 系/native 模块依赖，仅类型用途 electron）+ `pnpm-workspace.yaml`（`packages: ['.']` 阻断向上吸附）+ 纯前端 `vite.config.ts`（端口 **5190**，与基线 5180 并行对照）+ `src/`（复制自基线，排除 stories）+ `test/` + `src-tauri/`。
- **类型单源**：tauri-app 中 19 个文件的 electron 类型引用改写为 `@baseline` 别名（tsconfig paths + vite alias → `../electron/`，仅 import type 运行时零依赖）；`electron`/`better-sqlite3` 模块映射到 tauri-app 本地 node_modules 的 d.ts（基线文件位于上级目录，解析不到子项目 node_modules，须显式映射）。
- **为什么改**：① 共享 src/ 的方案与「不混改基线」矛盾（ipc-client 必须换底层）；② 根目录旧依赖树 resolve 卡死（resolved 1010 恒定卡点），独立小树 8.7 秒装完。

### 阶段 0 验证状态（tauri-app/ 下执行）

| 项 | 状态 |
|---|---|
| 依赖安装（441 包，@tauri-apps/api 2.12.1 + cli 2.12.1） | ✅ 8.7s |
| `pnpm typecheck`（501 文件，TS 6 严格） | ✅ 0 错误（ipc-client 适配后复验） |
| `pnpm run lint` | ✅ 零告警（适配后复验） |
| `pnpm build`（→ tauri-app/dist/） | ✅ 2.11s（适配后复验） |
| `pnpm test`（vitest 全量） | ⚠️ 25 分钟超时未定位，**批次验收遗留项**（setup-locale + source-contract 单文件 5/5 通过） |
| `cargo test --lib` | ✅ **28/28 全绿**（2026-10-06 第六次更新验证：批次 A 17 + 批次 B 11，含 fs/security/project 契约用例） |
| `cargo check --all-targets` | ✅ **0 告警**（第六次更新：消除 dead_code 告警 6 处） |
| `pnpm tauri dev` | ⬜ 窗口冒烟待做（Rust 编译链已验证通过，风险低） |

### 批次 A 迁移完成详情（2026-10-06 第三次更新）

**状态**：✅ **Rust 后端验证通过（17/17），前端 ipc-client 底层已切 Tauri**

#### 已完成工作

**1. 创建命令模块文件（5 个文件）**
- ✅ `src-tauri/src/commands/config.rs` - 配置管理（config_get, config_set）
- ✅ `src-tauri/src/commands/window.rs` - 窗口管理（minimize, toggle_maximize, close, resolve_close）
- ✅ `src-tauri/src/commands/skin.rs` - 皮肤管理（get_state, execute, read_custom_asset）
- ✅ `src-tauri/src/commands/official_homepage.rs` - 官方主页（open）
- ✅ `src-tauri/src/commands/model_provider_resource.rs` - 模型资源（open）

**2. 更新模块注册**
- ✅ `src-tauri/src/commands/mod.rs` - 移除重复模块声明，添加所有批次 A 模块
- ✅ `src-tauri/src/state.rs` - 添加 AppState 状态管理（config、skin Mutex）
- ✅ `src-tauri/src/lib.rs` - 注册所有 11 个 Tauri 命令

**3. 修复编译错误**
- ✅ 移除重复模块声明（mod.rs 中 config 定义重复）
- ✅ 修复类型错误（config.get()、config.set() 类型匹配）
- ✅ 修复导入问题（AppState 引用路径）
- ✅ 添加缺失字段（OfficialHomepageOpenResponse、ModelProviderResourceOpenResponse 的 error 字段）

**4. 代码检查结果**
- ✅ TypeScript 类型检查：`pnpm typecheck` ✅
- ✅ 代码规范：`pnpm run lint` ✅

#### 批次 A 频道映射

| 频道组 | 频道名 | 命令数 | 命令列表 | 状态 |
|--------|--------|--------|--------|-----|
| Config | `config` | 2 | config_get, config_set | ✅ 完成 |
| Window | `window` | 4 | window_minimize, window_toggle_maximize, window_close, window_resolve_close | ✅ 完成 |
| Skin | `skin` | 3 | skin_get_state, skin_execute, skin_read_custom_asset | ✅ 完成 |
| Official Homepage | `official-homepage` | 1 | official_homepage_open | ✅ 完成 |
| Model Provider Resource | `model-provider-resource` | 1 | model_provider_resource_open | ✅ 完成 |

**总计：11 个命令**

#### Rust 命令签名约定

遵循以下约定（docs/agents/pi-development.md）：
- 命令名：snake_case，与频道名一一对应（`config:get` → `config_get`）
- 入参/返回值：与ipc-channels.ts 的 `args`/`return` 类型一一对应，派生 `Serialize`/`Deserialize` 且 `#[serde(rename_all = "camelCase")]` 对齐前端字段
- 项目域命令：尾部必须接收 `project_session`（渲染层 ipc-client.ts 自动注入）
- 错误返回：`Result<T, String>` 起步，复杂域再自定义可序列化 Error
- 状态访问：通过 `Mutex` 锁访问，如 `state.config.lock().unwrap()`

#### 下一步工作

1. ~~验证阶段~~ → ✅ 已完成（2026-10-06 第四次更新：cargo test 17/17）

2. ~~前端适配~~ → ✅ 已完成（ipc-client.ts 已切 Tauri 底层，见下方适配详情）

3. **批次 B 迁移准备**：
   - 阅读批次 B 盘点清单（docs/plans/tauri-migration-channel-inventory.md）
   - 创建新的命令模块文件

4. **窗口冒烟**：`pnpm tauri dev`（需 GUI，建议用户手动验证）

5. ~~提交批次 A~~ → ✅ 已完成（`e3d388f` Rust 后端 / `00e1b73` ipc-client 适配）

#### 前端适配详情（第四次更新新增）

**改造文件：`tauri-app/src/services/ipc-client.ts`（唯一 invoke 调用面，`window.velaAPI` → `@tauri-apps/api/core.invoke`）**

- 频道→命令名映射：机械规则 `channel:seg-name` → `channel_seg_name`（`:`/`-` → `_`）。
- 位置参数 → 命名参数：新增 `CHANNEL_ARG_NAMES` 登记表（频道 → Rust 命令参数名 camelCase 序列），
  批次迁移时登记；未登记的非空参频道立即抛错，防止静默错配。
- 项目域频道沿用「尾部注入 projectSession」→ Tauri 命名参数 `projectSession`（对应 Rust 尾参 `project_session`）。
- 事件：`ipc.on/once` → `@tauri-apps/api/event` 的 `listen/once`（回调取 `event.payload`；
  异步注册包装为同步取消函数，对齐 Electron 版语义）。
- `isElectron` getter 保留原属性名（减少上游同源调用面漂移），语义改为「存在 IPC 后端」（Tauri 判定）。
- zoom 三方法（setZoomLevel/setZoomFactor/getZoomLevel）暂为 no-op + warn（阶段 3 迁移项）。
- `finalization-client.ts` 仍走 velaAPI（批次 E 迁移时统一处理，Tauri 下会抛「不在 Electron 环境」属预期）。

**契约对齐修复（批次 A 命令与 ipc-channels.ts 逐项核对）**

| 项 | 修复 |
|---|---|
| `config:set` 参数名 | Rust `updates` → `config`（契约 args: [config: Partial<GlobalConfig>]） |
| `config` 存储结构 | `HashMap<String, HashMap<..>>` → 扁平 `HashMap<String, Value>`，默认值对齐 `DEFAULT_GLOBAL_CONFIG`（theme=dark, editorFontSize=16, autoSaveInterval=30, proxy{...}）；set 改浅合并语义 |
| `model-provider-resource:open` 参数名 | Rust `resource_id` → `resource`（契约 args 命名） |
| `SkinCommand` tag | `rename_all = "lowercase"` → `"kebab-case"`（TS 传 `import-custom`）；variant 字段 camelCase |
| `window:resolve-close` decision | enum 加 `rename_all = "lowercase"`（契约字面量 'proceed'/'cancel'） |
| `skin:read-custom-asset` 失败形状 | `Result` reject → 契约判别联合（`SkinReadCustomAssetResponse`：Success 展开 asset 字段 / Failure 带 state+error） |
| 测试 | 新增 kebab-tag 反序列化、lowercase decision 反序列化用例（共 17 测试） |

### 批次 B 完成（2026-10-06 第六次更新）

批次 B = 项目/文件/授权 22 频道（`fs:read/write/list/mkdir/check-exists/read-json/write-json` 7 + `project:*` 10 + `dialog:select-folder` 1 + `fs:grant-*` 3 + `dialog:select-export-directory` 1）。**第六次更新已完成接线、编译验证与前端参数登记**：

- **已创建（4 文件，已编译验证）**：
  - `src-tauri/src/security.rs` —— 路径安全边界（迁移 `electron/utils/project-context.ts` + `project-session-context.ts`）：`ProjectSessionContext`（serde camelCase 三字段）、`GuardKind`（缺会话/越界/跨项目/租约失效，中文文案对齐基线）、`lexically_normalize`（消 `.`/`..`/统一分隔符）、`lexically_contained`、`canonical_writable_target`（存在祖先 realpath + 拼回缺失段）、`assert_project_file_path`（词法→realpath 双重检查防 symlink/junction 越界）、`assert_current_project_context` 骨架租约校验（活跃项目路径一致；完整租约签发/失效校验待批次 C）+ 6 单元测试；**已写入并字节级验证**。
  - `src-tauri/src/commands/fs.rs` —— fs 基础 7 命令真实实现：统一守卫链（会话→expectedProjectPath→路径边界）+ 全局文件锁骨架（基线为 per-path mutex 队列，安全语义不变，并发优化批次 C 后评估）+ 原子写（同目录临时文件 + rename，`commitState` 两态，`unknown` 结构保留）+ `FileNode` 递归列目录（过滤 `.` 开头、目录优先）+ 错误文案对齐（read-file ENOENT 特例文案含 read_architecture 引导）；async 命令内同步 IO（不阻塞 UI 线程）；附带 2 个单元测试（原子写/列目录）。
  - `src-tauri/src/commands/project.rs` —— 真实实现 5：`project:get-runtime-context`（读 AppState.active_project，db_ready 恒 false 诚实反映未迁）、`project:recent-list` / `project:recent-remove`（`~/.vela/recent-projects.json`，词法归一删除，对齐 `readJsonFile(path, [])` 缺省空）、`project:smoke-open-request` / `project:smoke-open-confirm`（烟测 env 校验 + marker 写入）；骨架 5：`project:create/open/save/update-config/delete` 返回契约形状完整结构化失败（`SKELETON_DB_MESSAGE`）；`dialog:select-folder` 返回 null。**⚠️ 写入时出现重复定义污染已手工清理（RuntimeContext/ProjectDeleteResult 末尾重复块已删），仍需 cargo 编译确认**。
  - `src-tauri/src/commands/external_file_grant.rs` —— grant 域骨架：`fs:grant-read-file/write-file/mkdir` 返回契约形状失败 + `dialog:select-export-directory` 返回 None（取消语义）；`ExternalDirectoryGrant { grantId, displayName }` 契约结构已就位；等 tauri-plugin-dialog（新插件依赖，AGENTS.md Ask first）接入；附 1 个单元测试。
- **接线已完成（第六次更新，四处）**：
  1. ✅ `state.rs`：新增 `ActiveProject { root_path }` + `active_project: Mutex<Option<ActiveProject>>` + `fs_lock: Mutex<()>`
  2. ✅ `commands/mod.rs`：声明 `mod fs; mod project; mod external_file_grant;` + `pub use`；新增共享 `SimpleResult`（fs/project 两模块共用，避免 glob 再导出同名歧义）
  3. ✅ `lib.rs`：声明 `mod security;` + `generate_handler!` 注册批次 B 22 命令（共 34 命令）
  4. ✅ `ipc-client.ts`：`CHANNEL_ARG_NAMES` 登记 **17 个**带参频道（fs 7 + grant 3 + project 7：create/open/save/update-config/recent-remove/delete/smoke-confirm；第五次快照预估的 15 个漏计了 `project:save`/`update-config`）
- **验证结果（第六次更新 全绿）**：`cargo test --lib` **28/28**（批次 A 17 + 批次 B 11）、`cargo check --all-targets` **0 告警**、`pnpm typecheck` ✅、`pnpm run lint` ✅（零告警）、`pnpm build` ✅（1.40s）
- **编译期修复清单（本次）**：
  1. `project.rs` 重复 `RuntimeContext` 定义（写入污染残留）→ 删除第二处
  2. `epoch_millis_to_string` 未定义 → 改为 `iso8601_utc_from_millis(ms)`（对齐基线 `new Date().toISOString()`，手写 civil_from_days，不引入 chrono）+ `epoch_millis_now()`
  3. 烟测回执写入目标错误：原写 `project_path`（会把 marker 写到项目目录）→ 按基线改为写 `markerPath`，回执体改 `to_string`（对齐 `JSON.stringify` 单行）
  4. 缺 `dialog:select-folder` 命令 → 在 `project.rs` 补 `dialog_select_folder() -> Option<String>`（None 取消语义）
  5. `fs.rs` 五处 `async` 命令返回非 `Result`（Tauri 报 `AsyncCommandMustReturnResult`）→ 改 `Result<T, String>`，结构化失败用 `Ok(...)` 包裹
  6. `FsError` 缺 `From<GuardKind>` → 补 impl（或 `fs_error_message` 映射）；`guard_message(kind)` 误接 `FsError` 的两处改 `fs_error_message`
  7. 测试断言错误 2 处：`lexically_contained` 测试须先 `lexically_normalize`；`read_dir_recursive` 过滤后应为 2 项（目录优先）
  8. dead_code 告警 6 处（骨架预留态：`FileWriteCommitState::Unknown`、`FsError::Io` 字段、`GrantFailureCommitState::Unknown`、`GuardKind::MissingSessionContext`、`GuardResult`、`OFFICIAL_HOMEPAGE_URL`）→ 加 `#[allow(dead_code)]` + 注释说明启用批次
- **Tauri 参数名规则已核实（源码级）**：`tauri-macros 2.7.1` 对每个参数 key 做 `to_lower_camel_case()`（`wrapper.rs:501`），故 Rust `expected_project_path` 与 `_expected_project_path` 均对应前端 `expectedProjectPath`；`Option<T>` 参数在 key 缺失时 `visit_none()`（`tauri-2.12.1/src/ipc/command.rs:146-156`），故 `fs:grant-read-file` 的 `relativePath` 可省略。
- **骨架行为（诚实反映未迁能力）**：`project:create/open/save/update-config/delete` 返回契约形状结构化失败（`SKELETON_DB_MESSAGE`，不 reject）；`fs:*` 因无活跃项目（`project:open` 未真实化）一律返回「项目租约已失效，已拒绝操作」（属预期，批次 C 接数据库后解开）。
- **两个 Ask first 决策点仍待用户确认**：① `tauri-plugin-dialog`（`dialog:select-folder`/`select-export-directory` 真实目录选择，新增插件依赖）；② `rusqlite`（批次 C 数据库层，Rust crate 依赖）——均需用户点头后再加。
- **写入污染教训**：长文件 `write` 易产生重复定义/残缺片段（本批次出现 `RuntimeContext` 重定义、缺失函数引用、字段名悬空）。**每写完一个长文件先跑顶层声明去重 + 括号配平扫描（`ctx_execute` 脚本）再接线**，本次已在接线前发现并清理 2 处。

### 已知遗留项（下一会话处理）

1. **提交批次 B**（第六次更新验收全绿，待提交）：`feat(tauri): migrate batch-b fs/project/grant`（4 新文件 + state/mod/lib/ipc-client 四处接线 + 快照更新）
2. **Ask first 待确认**：`tauri-plugin-dialog`（dialog 2 频道真实化）与 `rusqlite`（批次 C 前置），均需用户点头后再加
3. **批次 B 验证**：与 Electron 版行为对照（recent-projects.json 双栈互通、错误文案、commitState 两态）
4. **`pnpm tauri dev` 窗口冒烟**：批次 A+B 一起验证，预期未迁频道抛「尚未迁移」
5. **批次 C（数据库层）**：rusqlite 真实实现 + project:create/open/save/delete 租约签发启用（会话租约完整校验补入 security.rs 骨架处）
6. **持续验证**：提交前 `cd tauri-app && pnpm typecheck && pnpm run lint`；改 Rust 追加 `cargo test --lib`；双栈并行（5180/5190）
7. **vitest 全量超时定位**（上表 ⚠️）
8. **i18n 校验**：`check:i18n` 未在 tauri-app 配置，批次 E 收口前补
9. **@baseline 类型链风险**：上游改动 electron/repositories|services 类型文件时 tauri-app typecheck 会同步受影响 —— 属预期（类型单源），批次验收时留意
10. **radix 按需清单**：package.json 仅含扫描到的 3 个 @radix-ui 包（dialog/slot/tooltip），其余（label/select/separator/tabs）在 vite build/typecheck 报缺时补装
11. **rustfmt 未纳入验收**：`cargo fmt --check` 在 `src-tauri/` 全域有差异（批次 A/B 文件均未过 rustfmt）；如需统一，应单独提交 `chore(tauri): cargo fmt src-tauri`，勿混入功能批次

---

## 阶段总览

| 阶段 | 内容 | 状态 |
|---|---|---|
| 0 | Tauri 脚手架（`tauri-app/` 迁移根 + AppState + invoke_handler 骨架 + tauri.conf.json 接 Vite） | ✅ **完成**（TS 全绿 + cargo test 待验证） |
| 1 | IPC 契约盘点（198 频道清单、模式标注、批次规划） | ✅ 2026-10-06 完成 |
| 2 | 按 controller 批次迁移（A→H，见盘点文档 §6） | 🟡 **批次 A 已提交；批次 B 验证全绿待提交** |
| 3 | 原生绑定收尾（窗口/菜单/通知/更新插件、zoom 替代、CI tauri-action） | ⬜ 未开始 |
| 4 | 双栈并存验证 + 上游同步策略执行 | ⬜ 未开始 |

---

## 历史工作记录

### 2026-10-06 第一次更新（阶段 1 盘点 + 阶段 0 启动）

1. 全量通读 `src/shared/ipc-channels.ts`（1154 行）：15 个 invoke 接口组 + 3 个事件接口组，**193 invoke + 5 event = 198 频道**；盘点 17 个 controller 对应关系。
2. **前端调用面收敛**：`window.velaAPI` 仅被 `src/services/ipc-client.ts` 与 `src/services/finalization-client.ts` 直接访问 —— 迁移时只换这两个文件底层。
3. 产出盘点文档：`docs/plans/tauri-migration-channel-inventory.md`（缺口 G1–G6、传输约定、批次 A–H）。
4. 初始 `src-tauri/` 骨架建于仓库根（后被结构重设计移入 `tauri-app/src-tauri/`）。
5. pnpm 9.6.0 → 11.11.0 升级（standalone 自装结构，`pnpm add -g pnpm@11.11.0`）；根目录依赖树卡死 → 交用户手动 → 转向结构重设计。

### 2026-10-06 第二次更新（阶段 0 验证 + 结构重设计）

1. 确定**独立迁移根**方案：根目录还原上游，新建 `tauri-app/` 隔离迁移工作。
2. 产出结构重设计文档，用户确认后执行。
3. 阶段 0 验证全绿：依赖安装 8.7s、typecheck 0 错误、lint 零告警、build 1.10s。
4. 识别 vitest 全量超时问题，标记为批次验收遗留项。
5. 识别 `cargo test` 骨架测试通过（2/2），但正式测试未执行。

### 2026-10-06 第三次更新（批次 A 完成）

1. **Rust 后端全量创建**：5 个命令模块文件，共 11 个命令覆盖批次 A 所有频道。
2. **修复编译错误**：
   - mod.rs：移除 config 模块重复定义
   - state.rs：修复重复 import Mutex
   - config.rs、skin.rs：添加 crate::state::AppState 引用
   - official_homepage.rs、model_provider_resource.rs：添加缺失的 error 字段
3. **代码检查结果**：TypeScript 类型检查通过，lint 零告警。
4. **标识未验证**：`cargo test --lib` 需手动执行（环境不可用）。

### 2026-10-06 第四次更新（批次 A 验证 + 前端适配）

1. **cargo test --lib 验证通过**：修复 28→4→0 编译错误（mod.rs 未再导出命令宏、state.rs 路径解析、config 默认值类型、测试引用未声明模块），最终 17/17 全绿。
2. **残留死文件确认**：`src-tauri/src/state/commands.rs`、`src-tauri/src/commands/config/internal_exports.rs` 为上会话残留（未被 mod 树引用，内容与正式实现重复且过时），待用户确认后删除。
3. **前端适配完成**：`ipc-client.ts` 切换 Tauri 底层（invoke/listen/once + CHANNEL_ARG_NAMES 登记表 + projectSession 命名参数注入）；zoom 三方法标记为阶段 3 no-op。
4. **契约对齐修复 6 项**（config:set 参数名、config 扁平存储+默认值、resource 参数名、SkinCommand kebab tag、decision 小写字面量、skin 资产读取判别联合），见快照区「契约对齐修复」表。
5. **验证状态**：cargo test 17/17 + typecheck 0 错误 + lint 零告警 + build 2.11s + vitest 相关单文件 5/5。

### 2026-10-06 第五次更新（批次 B 开工，未完成）

- 创建 `security.rs`（路径安全边界 + 6 测试）、`commands/fs.rs`（fs 基础 7 命令真实实现）、`commands/project.rs`（真实 5 + 骨架 5）、`commands/external_file_grant.rs`（grant 骨架 4）；均已写入但**未接线未编译**（state/mod/lib/ipc-client 四处接线未做，详见批次 B 章节）。
- 长文件写入多次出现重复定义污染，已手工清理 project.rs 末尾重复块；教训与检查脚本已写入快照。

### 2026-10-06 第六次更新（批次 B 完成）

1. **接线四处落地**：`state.rs`（`ActiveProject` + `active_project` + `fs_lock`）、`commands/mod.rs`（3 模块 + 共享 `SimpleResult`）、`lib.rs`（`mod security;` + 22 命令注册）、`ipc-client.ts`（17 带参频道登记）。
2. **先扫描后接线**：接线前用 `ctx_execute` 脚本扫 4 新文件的顶层重复定义与括号配平，发现 `project.rs` `RuntimeContext` 重定义、`epoch_millis_to_string` 缺失、`dialog_select_folder` 缺失三处硬伤，先清后接。
3. **编译修复 8 类**（详见批次 B 章节清单）：async 命令返 `Result`、`From<GuardKind>`、错误映射类型、烟测 marker 写入目标、ISO8601 实现等。
4. **契约校验强化**：源码级确认 Tauri 参数 key = `to_lower_camel_case`（含下划线前缀归并）、`Option<T>` 缺省安全；烟测回执对齐 `new Date().toISOString()` 与 `JSON.stringify` 形态，并补固定时间戳单测（含闰年）。
5. **验证**：`cargo test --lib` 28/28 · `cargo check --all-targets` 0 告警 · `pnpm typecheck`/`lint`/`build` 均绿。

### 关键发现摘要（接续前必读，详见盘点文档）

- **★ 会话注入约定**：ipc-client 对项目域频道（`db:/kb:/chapter:/fs:/project:save|update-config|delete`）自动在 args 尾部追加 `projectSession`（契约未声明）—— Rust 命令签名必须预留尾参并校验租约。
- **G1 契约缺口**：`finalization:commit/retry` 未在 ipc-channels.ts 声明，迁移时补 `FinalizationChannels`。
- **G2 类型越界**：现已转为 `@baseline` 别名机制（tsconfig paths 显式映射 electron/better-sqlite3 到本地 d.ts）。
- **G4 两段式关窗**：`window:close-requested { requestId }` → `window:resolve-close(requestId, decision)`。
- **G5 流式性能**：`llm:stream-chunk` 高频事件过 Tauri 桥需实测。
- **zoom**：preload 的 `webFrame` 缩放为 Electron 专属，阶段 3 找 Tauri 替代。
- **G3**：update-controller 在 main.ts:259 特殊注册（带 publish 回调）。

## 下一步（按序）

1. ~~批次 A 验证~~ → ✅ 已完成（2026-10-06 第四次：cargo test 17/17 全绿）

2. ~~前端适配~~ → ✅ 已完成（ipc-client.ts 已切 Tauri invoke；契约对齐修复 6 项，见上方适配详情）

3. ~~批次 B 迁移~~ → ✅ 已完成（2026-10-06 第六次：22 命令注册 + 接线 + 验证 28/28 + 前端登记 17 频道）；**下一步提交批次 B**

4. **窗口冒烟**（需 GUI，建议用户手动验证）：
   - 运行 `pnpm tauri dev`
   - 预期：Tauri 窗口加载前端、批次 A 已迁频道可 invoke 成功、未迁频道抛「尚未迁移」、`fs:*` 因无活跃项目返回租约失效文案

5. ~~提交批次 A~~ → ✅ 已完成（两个 commit：Rust 后端 + ipc-client 适配；2 个残留死文件已删）

6. **批次 C（数据库层）**：需用户先确认 `rusqlite` 依赖（Ask first）；接入后启用 `project:create/open/save/update-config/delete` 真实实现 + 租约签发，法兰 `security.rs` 的 `assert_current_project_context` 完整校验

7. **批次 B 双栈行为对照**：recent-projects.json 双栈互通、错误文案、`commitState` 两态（与 Electron 5180 并行验证）

6. **持续验证**：
   - 每次提交前：`cd tauri-app && pnpm typecheck && pnpm run lint`；改 Rust 追加 `cargo test --lib`
   - 每次提交后：运行 `pnpm tauri dev` 验证双栈兼容（5180/5190）

## 纪律提醒（来自 AGENTS.md / pi-development.md）

- 根目录**（Electron 基线）永不改动**（上游同步锚点）；一切迁移工作在 `tauri-app/` 内。
- 每批次验收：`cargo test` + `pnpm typecheck` + `pnpm run lint` + 与 Electron 版行为对照（两栈可并行：5180/5190）。
- 定稿不可逆（ADR 0003/0011）、会话租约（ADR 0001）、外部文件授权（ADR 0002）、提示词合同（ADR 0008/0015）不得在迁移中弱化。
- 勿提交 API Key、小说正文、构建产物；`tauri-app/src-tauri/target/`、`dist*/`、`.release/` 不可编辑。
- Issue-first：迁移任务按批次开 Issue（`docs/agents/issue-tracker.md`），PR 引用 Issue。

## 文档索引

| 文档 | 用途 |
|---|---|
| `docs/plans/tauri-migration-channel-inventory.md` | **阶段 1 产出**：198 频道盘点、传输约定、缺口 G1–G6、批次 A–H |
| `docs/agents/pi-development.md` | 迁移总纲：分层映射、Rust 规范、实施步骤（⚠️ 其目录结构段仍为旧 src-tauri 布局，以本文与 AGENTS.md 为准） |
| `docs/research/2026-10-04-tauri-migration-evaluation.md` | 迁移决策来源（工时评估） |
| `docs/product-domain.md` | 领域词汇与事实源边界（迁移不得偏离） |
| `docs/adr/0001,0002,0008,0011,0013,0015,0017` | 迁移涉足的关键架构决定 |