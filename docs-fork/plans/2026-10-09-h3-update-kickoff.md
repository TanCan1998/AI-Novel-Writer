# H3 开工清单：`update`（6 频道 + `update:state` 事件）

> 2026-10-09 产出（第三十三次会话收尾）。**用户已确认方案：零依赖 GitHub-Release 后端**
> （见 [`2026-10-09-h3-h4-dependency-evaluation.md`](../research/2026-10-09-h3-h4-dependency-evaluation.md)）。
> 本清单目标：下次会话可以**不再调研**，直接按 §4 的顺序实现。

## 1. 范围与预期结果

| 项 | 值 |
|---|---|
| 频道 | `update:get-state` / `update:check` / `update:download` / `update:open-release` / `update:defer-reminder` / `update:quit-and-install`（6 个） |
| 事件 | `update:state`（主 → 渲染，每状态变更推全量 state） |
| 预期指标变化 | 未迁移 **35 → 29**；已注册命令 **159 → 165**；覆盖 **158 → 164**（`update=6` 从清单消失） |
| 不做什么 | **不引入 `tauri-plugin-updater`**（B14 另立专项）；不移植 `electron-updater-adapter.ts`；不做真正的静默安装 |

## 2. 契约与类型（逐条对齐，勿改名）

`UpdateChannels`（`tauri-app/src/shared/ipc-channels.ts:86-112`）：

| 频道 | args | return |
|---|---|---|
| `update:get-state` | `[]` | `UpdateState` |
| `update:check` | `[]` | `UpdateCheckResponse` |
| `update:download` | `[]` | `UpdateActionResponse` |
| `update:open-release` | `[]` | `UpdateActionResponse` |
| `update:defer-reminder` | `[days: UpdateReminderDelay]` | `UpdateActionResponse` |
| `update:quit-and-install` | `[]` | `UpdateActionResponse` |

`UpdateStateEvents['update:state']: UpdateState`（同文件 `:113-116`）。

类型全集在 **`src/shared/update-types.ts`（111 行）**，Rust 侧需按 camelCase 逐字段镜像：

- `UpdateStatus`（8 态）：`idle | checking | not-available | available | downloading | downloaded | error | disabled`
- `UpdateErrorCode`（11 个）：`UPDATES_DISABLED | UPDATE_CONFIGURATION_MISSING | CHECK_FAILED | DOWNLOAD_NOT_READY | DOWNLOAD_FAILED | INSTALL_NOT_READY | INSTALL_FAILED | OPEN_RELEASE_FAILED | REMINDER_NOT_AVAILABLE | REMINDER_SAVE_FAILED | INVALID_REMINDER_DELAY`
- `UpdateErrorPhase`：`configuration | check | download | install | navigation | reminder`
- `UpdateErrorReason`、`SafeUpdateTechnicalDetails`（脱敏细节联合）、`UpdateError`、`UpdateDownloadProgress`、`UpdateState`、`UpdateCheckResponse`、`UpdateActionResponse`、`UpdatePreferences`、`UpdateReminder`、`UpdateReminderDelay = 7 | 30`、`UpdateReleaseInfo`、`UpdateCheckResult`、`UpdateAction = 'download' | 'open-release'`

> ⚠️ 该文件是**上游镜像**（`src/`），Rust 侧只做镜像、**不改它**。

## 3. 基线文件清单（含行数，实现时逐行对照）

| 文件 | 行数 | 本批处理 |
|---|---|---|
| `electron/services/update-service.ts` | 585 | **核心：完整移植**（状态机 + 版本比较 + 错误分类 + 6 方法） |
| `electron/services/github-release-update-backend.ts` | 49 | **移植**（本批唯一后端） |
| `electron/services/update-preferences-store.ts` | 45 | **移植**（写全局配置的 `updatePreferences`，须保留其他字段） |
| `electron/services/update-startup.ts` | 73 | **移植装配逻辑**（后端选择 + 降级 + 自动检查不阻塞启动） |
| `electron/services/update-runtime.ts` | 30 | **移植门禁**：`isWindowsUpdateRuntimeEnabled` = `win32 && isPackaged && !devServerUrl`；`isMacUpdateReminderEnabled` = `darwin && isPackaged`；`hasWindowsUpdateConfiguration` 检查 `resources/app-update.yml` |
| `electron/controllers/update-controller.ts` | 54 | **频道映射参考**（6 个 handle + `subscribe` → `publish(toRendererState(state))`） |
| `electron/main.ts:245-260` | — | **装配点参考**：`openRelease: shell.openExternal(GITHUB_LATEST_RELEASE_PAGE)`、`createBackend` 平台二选一、`createPreferences`、`registerController`、`reportFailure` |
| `electron/services/electron-updater-adapter.ts` | 65 | **不移植**（B14 专项） |

关键语义（实现时必须保留）：

1. **错误分类器**（`update-service.ts:145-171`）：`asset-missing` / `metadata-invalid` / `HTTP_403`（不可重试）/ `HTTP_404`（不可重试）/ `HTTP_429`（可重试）/ `proxy|tunnel` → `PROXY_CONNECT_FAILED`（可重试）/ `cert|tls|ssl|self signed|unable to verify` → `TLS_HANDSHAKE_FAILED`（可重试）/ 其余 → `DNS_OR_OFFLINE`（可重试）/ 兜底 `UPDATE_OPERATION_FAILED`（可重试）。**细节必须脱敏**（只回传 `SafeUpdateTechnicalDetails`）。
2. **版本比较** `isHigherStableVersion`（`:175-186`）：按 `.` 分段比较；任一侧不可解析 → `false`。
3. **后端契约**：`checkForUpdates()` 返回 `UpdateCheckResult | null`（`null` = 不可用）；`downloadUpdate()` 返回 `string[]`（GitHub 后端恒 `[]`）；`quitAndInstall()` 为 `void`。
4. **禁用态**：`updateConfiguration === 'missing'` 或门禁关闭时用 **disabled backend**（`checkForUpdates → null`），但 `isPackaged` 保持 true，以便手动检查能返回可行动的 `UPDATE_CONFIGURATION_MISSING`。
5. **启动降级**：任何初始化/自动检查失败都必须**降级为更新不可用**，绝不阻塞应用启动（`update-startup.ts:32-73`）。
6. **偏好写入**：写 `~/.lorekeeper/config.json` 的 `updatePreferences`；配置缺失 → 用默认配置 + 偏好新建；**配置损坏 → 返回 `false` 并保留原文件**（`REMINDER_SAVE_FAILED`），绝不用默认值覆盖用户的模型/语言/代理设置。

## 4. 实现顺序（建议）

1. **模块骨架** `src-tauri/src/update/mod.rs`（或单文件 `update_service.rs`）+ `Cargo.toml` **无新依赖**。
2. **类型镜像**（`update-types.ts` 全集，`#[serde(rename_all = "camelCase")]`；⚠️ 注意 `SafeUpdateTechnicalDetails` 是联合类型 → Rust 侧建议 `serde_json::Value` 或 tagged enum，**先核对基线实际取值集合**）。
3. **门禁函数**（3 个纯函数，可直接单测）。
4. **`is_higher_stable_version`**（纯函数，单测边界）。
5. **GitHub-Release 后端**：`reqwest` GET `https://api.github.com/repos/<owner>/<repo>/releases/latest`
   （`Accept: application/vnd.github+json`、`X-GitHub-Api-Version: 2022-11-28`、`User-Agent`；未经代理时也沿用全局 `proxy` 配置 → 复用 `llm::chat::{proxy_from_config, build_client_with_timeout}`）。
   **更新源决策（实现时确认）**：建议指向 **fork** `TanCan1998/Lorekeeper`（与 B12 官方主页一致）；基线常量指向上游 `EthanYoQ/AI-Novel-Writer`（`github-release-update-backend.ts:3-4`）——**这是本轮唯一需要用户复述确认的点**。
6. **偏好存储**：复用 `json_store::read_json_value_or` / `write_json_file`，写前读合并（`updatePreferences` 之外的键全部保留）。
7. **`UpdateService`**：`state` + `listeners`；6 个方法；状态流转与 `checkAutomatically`（含 `lastCheckedAt` / `reminder` 语义，逐行对照基线）。
8. **命令层** `commands/update.rs`：6 个 `#[tauri::command]`，均返回信封（不 reject）；`update:defer-reminder` 校验 `days ∈ {7,30}` 后转 `UpdateReminderDelay`。
9. **事件**：状态变更后 `app.emit("update:state", state)`。建议把 `AppHandle` 显式传给服务（`Arc<dyn Fn(UpdateState)>` 回调注入），或让命令层在每次变更后 emit——**二选一，需在实现时定**（基线是 `subscribe` → controller `publish`）。
10. **启动装配**：`lib.rs` 的 `.setup(...)` 中按 §3.4/3.5 创建服务并触发自动检查（失败仅 `log`，不影响启动）。
11. **前端登记**：
    - `src/services/ipc-client.ts`：`'update:defer-reminder': ['days']`（其余 5 个无参，无需登记）；
    - 重新生成 `src/shared/migrated-channels.ts`（`node scripts/verify-channel-coverage.mjs --emit`）；
    - `test/channel-migration-coverage.test.ts` 增加 6 个 `MIGRATED_CHANNELS.has(...)` 断言（可选）。
12. **渲染层核对**：确认 `update:state` 的既有消费点（`ipc.on('update:state', ...)`）与 `UpdateState` 字段一致；`ipc-client.ts` 的 `EventChannel` 已含该事件。

## 5. 测试计划（Rust 单测，目标 +12～18）

| 组 | 用例 |
|---|---|
| 版本比较 | `1.0.0 < 1.0.1`；`1.2.0` vs `1.10.0`（按段比较，不是字符串）；相等 → false；`v1.2.3` 前缀（GitHub `tag_name` 剥 `v`）；非数字段 → false |
| 错误分类 | 403/404/429、`proxy`/`tunnel`、`cert`/`tls`、其他网络、兜底；断言 `code`/`phase`/`retryable`/`safeTechnicalDetails` 四元组与基线一致 |
| 后端解析 | 缺 `tag_name` → `METADATA_INVALID`；`name`/`body`/`published_at` 可缺省；非 2xx → 对应分类 |
| 状态机 | `idle → checking → not-available`；`→ available`；`downloadUpdate` 未就绪 → `DOWNLOAD_NOT_READY`（可重试）；disabled 后端 → `disabled` 且手动检查返回 `UPDATE_CONFIGURATION_MISSING` |
| 偏好 | `deferReminder(7/30)` 写 `updatePreferences` 且**保留**其他配置键；非法值（非 7/30）→ `INVALID_REMINDER_DELAY`；配置损坏 → 写失败 → `REMINDER_SAVE_FAILED` 且**原文件不被覆盖** |
| 门禁 | `win32 && packaged && !devServerUrl`；`darwin && packaged`；`hasWindowsUpdateConfiguration` 用注入的 `existsAt` 桩 |
| open-release | 复用 `external_link::open_external_url`；打开失败 → `OPEN_RELEASE_FAILED`（可重试） |

## 6. 验收标准

```bash
# tauri-app/src-tauri
cargo test --lib          # 期望 ≥ 522 通过（510 + 新增 ≥12）
cargo check --all-targets # 0 告警
cargo fmt --check         # 干净
# tauri-app
pnpm typecheck && pnpm run lint
node scripts/verify-channel-coverage.mjs --quiet   # 193 契约 / 165 命令 / 164 覆盖 / 29 未迁移
npx vitest run test/channel-migration-coverage.test.ts test/ipc-arg-struct-contract.test.ts
```

GUI 冒烟（可复用 `F:\Temp\loretest\lore-smoke` 夹具）：`update:get-state` 初始化、手动检查（dev 非打包 → 期望 `disabled`/`UPDATE_CONFIGURATION_MISSING` 语义）、`update:open-release` 真实打开 Release 页、`update:defer-reminder` 写入 `~/.lorekeeper/config.json`。

## 7. 风险与开放问题

| # | 项 | 说明 |
|---|---|---|
| 1 | **更新源 owner/repo** | 基线指向上游；建议 fork。**实现前请向用户复述确认** |
| 2 | `SafeUpdateTechnicalDetails` 联合类型 | 需先枚举基线取值再决定 Rust 表示（`Value` 最稳） |
| 3 | 事件发射点 | 服务内注入 `AppHandle` 回调 vs 命令层 emit；建议前者（自动检查也能推送） |
| 4 | 自动检查时机 | 基线为启动后 fire-and-forget（`update-startup.ts:71`）；Tauri 侧在 `.setup` 中同样不得阻塞 |
| 5 | `update:download` 语义 | GitHub 后端恒返回空数组（基线行为）；**不得**伪造成「已下载」 |
| 6 | `update:quit-and-install` | GitHub 后端下状态永不为 `downloaded` → 恒 `INSTALL_NOT_READY`（对齐基线 macOS 策略）；Windows 真正安装见 B14 |
| 7 | 与 `config:set` 并发写 `config.json` | 复用 `json_store` 原子写 + 读合并；避免覆盖用户其他设置 |
| 8 | rustc 版本 | `tauri-plugin-dialog 2.8.1` 已要求 rustc ≥ 1.90（B9）；本批无新增约束 |
