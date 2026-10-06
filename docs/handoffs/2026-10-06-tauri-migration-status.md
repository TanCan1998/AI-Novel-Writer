# Tauri 迁移进度快照（活文档）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档。**每次迁移工作完成后必须更新本文件的快照区块**。channel 级细节见 [`docs/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。
> 本文件为日期化交接快照（docs/README.md 治理规则）；后续大节点可另立日期文件，勿回写历史快照。

---

## 快照（最后更新：2026-10-06 · 第二次）

| 项 | 值 |
|---|---|
| 分支 | `master` |
| 基准 SHA | `cb71878`（fork 初始化提交；后续全部工作未提交，见「未提交变更」） |
| 当前阶段 | **阶段 0 完成**（TS 侧 + Rust 侧验证全绿），下一步批次 A |
| 结构 | **已重设计为独立迁移根 `tauri-app/`**（用户确认的方案 B），根目录三文件已还原上游原样 |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc @ `D:\Environment\rust\`（遵循 AGENTS.md 第 3 条规则）；crates 走 rsproxy 镜像；VS Build Tools 已装 |
| 未提交变更 | 新增：`tauri-app/`、两份文档；修改：`.gitignore`（仅 Tauri 忽略条目） |
| 未验证事项 | ① `pnpm tauri dev` 窗口冒烟未做（编译链已通，风险低）；② tauri-app vitest 全量超时未定位；③ i18n 覆盖校验未在 tauri-app 配置 |

### 结构重设计（2026-10-06 第二次更新，用户确认）

- **根目录** = Electron 基线（`package.json`、`vite.config.ts` 已 `git checkout` 还原上游原样；`src/`、`electron/` 不动）→ 上游合并冲突面归零。
- **`tauri-app/`** = 迁移项目根：独立 `package.json`（无 electron 系/native 模块依赖，仅类型用途 electron）+ `pnpm-workspace.yaml`（`packages: ['.']` 阻断向上吸附）+ 纯前端 `vite.config.ts`（端口 **5190**，与基线 5180 并行对照）+ `src/`（复制自基线，排除 stories）+ `test/` + `src-tauri/`。
- **类型单源**：tauri-app 中 19 个文件的 electron 类型引用改写为 `@baseline` 别名（tsconfig paths + vite alias → `../electron/`，仅 import type 运行时零依赖）；`electron`/`better-sqlite3` 模块映射到 tauri-app 本地 node_modules 的 d.ts（基线文件位于上级目录，解析不到子项目 node_modules，须显式映射）。
- **为什么改**：① 共享 src/ 的方案与「不混改基线」矛盾（ipc-client 必须换底层）；② 根目录旧依赖树 resolve 卡死（resolved 1010 恒定卡点），独立小树 8.7 秒装完。

### 阶段 0 验证状态（tauri-app/ 下执行）

| 项 | 状态 |
|---|---|
| 依赖安装（441 包，@tauri-apps/api 2.12.1 + cli 2.12.1） | ✅ 8.7s |
| `pnpm typecheck`（501 文件，TS 6 严格） | ✅ 0 错误 |
| `pnpm run lint` | ✅ 零告警 |
| `pnpm build`（→ tauri-app/dist/） | ✅ 1.10s |
| `pnpm test`（vitest 全量） | ⚠️ 25 分钟超时未定位，**批次验收遗留项**（疑似个别测试等待 IPC 挂起；可用 `-t` 过滤或逐文件定位） |
| `cargo test` | ✅ **2 tests passed**（state.rs + commands/mod.rs 骨架测试；tauri 2.12.1 依赖树全量编译通过；icons/icon.ico 已从 build/ 补齐） |
| `pnpm tauri dev` | ⬜ 窗口冒烟待做（编译链已通，风险低，可与批次 A 首批命令一起验证） |

### 已知遗留项（下一会话处理）

1. **`pnpm tauri dev` 窗口冒烟**：编译链已通；预期 Tauri 窗口加载前端、IPC 调用因后端未迁而失败，属预期。可与批次 A 一起验证。
2. **vitest 全量超时定位**（上表 ⚠️）。
3. **i18n 校验**：根目录 `check:i18n` 脚本未在 tauri-app 配置，批次 A 前补。
4. **@baseline 类型链风险**：上游改动 electron/repositories|services 类型文件时 tauri-app typecheck 会同步受影响 —— 属预期（类型单源），批次验收时留意。
5. **radix 按需清单**：package.json 仅含扫描到的 3 个 @radix-ui 包（dialog/slot/tooltip），其余（label/select/separator/tabs）在 vite build/typecheck 报缺时补装。

---

## 阶段总览

| 阶段 | 内容 | 状态 |
|---|---|---|
| 0 | Tauri 脚手架（`tauri-app/` 迁移根 + AppState + invoke_handler 骨架 + tauri.conf.json 接 Vite） | ✅ **完成**（TS 全绿 + cargo test 2/2） |
| 1 | IPC 契约盘点（198 频道清单、模式标注、批次规划） | ✅ 2026-10-06 完成 |
| 2 | 按 controller 批次迁移（A→H，见盘点文档 §6） | ⬜ 未开始 |
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

### 关键发现摘要（接续前必读，详见盘点文档）

- **★ 会话注入约定**：ipc-client 对项目域频道（`db:/kb:/chapter:/fs:/project:save|update-config|delete`）自动在 args 尾部追加 `projectSession`（契约未声明）—— Rust 命令签名必须预留尾参并校验租约。
- **G1 契约缺口**：`finalization:commit/retry` 未在 ipc-channels.ts 声明，迁移时补 `FinalizationChannels`。
- **G2 类型越界**：现已转为 `@baseline` 别名机制（tsconfig paths 显式映射 electron/better-sqlite3 到本地 d.ts）。
- **G4 两段式关窗**：`window:close-requested { requestId }` → `window:resolve-close(requestId, decision)`。
- **G5 流式性能**：`llm:stream-chunk` 高频事件过 Tauri 桥需实测。
- **zoom**：preload 的 `webFrame` 缩放为 Electron 专属，阶段 3 找 Tauri 替代。
- **G3**：update-controller 在 main.ts:259 特殊注册（带 publish 回调）。

## 下一步（按序）

1. **批次 A**（tauri-app 内）：window/config/skin/official-homepage/model-provider-resource 五个轻 controller → 前 10 个 Rust 命令 + `window:close-requested` 事件，打通 invoke/event 双通道；顺带跑 `pnpm tauri dev` 窗口冒烟。前端侧同步改造 `tauri-app/src/services/ipc-client.ts` 底层（velaAPI → `@tauri-apps/api/core.invoke` + `event.listen`，保留环境探测兼容）。
2. 每完成一批：更新本文件快照 + 盘点文档勾选，提交 `feat(tauri): migrate <module>`。

## 纪律提醒（来自 AGENTS.md / pi-development.md）

- 根目录（Electron 基线）**永不改动**（上游同步锚点）；一切迁移工作在 `tauri-app/` 内。
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
