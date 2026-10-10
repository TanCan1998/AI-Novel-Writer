# 上游合并专项评估（只读评估，**绝不实际合并**）

- 评估日期：2026-10-10
- 分支：`master`，HEAD = `bdd9d3c3ebcde495dc3c1c6d48f3bdd920df30e0`
- remote：`origin` = `https://github.com/TanCan1998/Lorekeeper.git`（本 fork）；`upstream` = `https://github.com/EthanYoQ/AI-Novel-Writer.git`（上游原仓库）
- 性质：全程只执行了 `git fetch upstream` 与只读查询（`log` / `diff` / `show` / `rev-list` / `name-only` / `merge-base` / `status`），**未执行任何 merge / rebase / cherry-pick / checkout / switch / reset / pull / push / stash / add / commit**，未改动工作树任何已有文件。唯一写入文件即本报告。

## 0. 实测执行记录与原始输出

1. `git fetch upstream` → **成功**（exit 0）。fetch 前本地跟踪的 `upstream/master` 停在 `464aa953`，fetch 增量输出 `464aa953..21d67211  master -> upstream/master`，并新增 8 个上游分支与 2 个 tag（`v1.2.1-Preview`、`v1.2.0-Preview`）。fetch 后 `upstream/master` tip = `21d67211`（2026-10-10 10:39:06 +0800，`fix(startup): restore installed startup and prepare 1.2.1-Preview (#343)`）。
2. `git rev-list --left-right --count upstream/master...HEAD` **原始输出**：
   ```
   509	150
   ```
   （左列 509 = `upstream/master` 独有提交数；右列 150 = 本仓 HEAD 独有提交数。）
3. 共同祖先：`git merge-base HEAD upstream/master` → `992b3f5f40165b59be7f1c126166235e7a7f807e`。
4. 时间窗：上游 509 提交 = 2026-09-13 02:08:44 → 2026-10-10 10:39（+0800）；本仓 150 提交 = 2026-10-06 16:37:19 → 2026-10-10 17:39（+0800）。

## 一、分叉量化数据

- **分叉规模**：`upstream/master` 领先 **509** 提交；本仓领先 **150** 提交；共同祖先 `992b3f5f`。
- **上游侧改动规模**（`git diff --stat HEAD...upstream/master`，三点语法 = 上游自共同祖先以来的改动）：**1018 个文件，+209,574 / −13,350 行**。
- **上游侧按目录汇总**（`git diff --name-only HEAD...upstream/master` 按首段分组计数）：

  | 目录 | 文件数 | 备注 |
  |---|---|---|
  | `src/**` | 421 | 渲染层大改：新增 `services/builtin-prompt-templates.ts`（+1506）、`shared/*` 大量新契约文件；删除 `src/services/vela-protocol.ts`（−133），新增 `src/services/resource-protocol.ts`；`src/shared/ipc-channels.ts` +187 行 |
  | `electron/**` | 264 | 主进程大改：新增 `migrations/`（m00–m06 + registry/runner）、`services/main-generation-owner.ts`（+1401）、`services/project-archive-service.ts`（+1216）、`services/webdav-backup-service.ts`（+684）、`repositories/review-cycle-repository.ts`（+789）等；`electron/database.ts` ±1069；`electron/knowledge-base.ts` 142 行变化；`electron/controllers/kb-controller.ts` 18 行；`electron/repositories/import-run-repository.ts` 174 行 |
  | `docs/**` | 180 | `docs/plans/novel-quality-program-v3-2026-09-13/**`（audits/specs/evidence/checks 全套）、`docs/research/novel-quality-modernization/**`（约 60 文件）、`docs/product-domain.md` |
  | `scripts/**` | 128 | 新增 `quality-modernization-*.mjs` 系列（`__tests__/quality-modernization-run.test.mjs` 单文件 **+7,683 行**）、`f04-v3-*` / `f05-u*` journey 脚本、startup-acceptance 系列；含大二进制夹具：`fixtures/s14c-official-old-sources/v100/.vela/vela.db`（368,640 B）、`v110/.vela/vela.db`（380,928 B）、lancedb 切片若干、`s14a-baseline-native-source.json.gz`（230,552 B）、2 个中文名 txt fixture |
  | `.github/**` | 5 | 改 5 个**既有** workflow：`cross-platform-runtime-artifact-promotion.yml`、`macos-arm64-cloud-build.yml`、`macos-x64-cloud-build.yml`、`pr-ci.yml`、`windows-cloud-build-test.yml` |
  | `test/**` | 4 | `test/desktop/project-archive.fixture.ts`（+480）、`test/fixtures/novel-quality-modernization/semantic-source.json`、`test/helpers/*` |
  | `plugins/dsh-ai-novel-writer/**` | 3 | `AGENTS.md`（**删除 4 行**）、`README.md`（163 行变化）、`tests/package.spec.ts` |
  | `.release/**` | 3 | 上游发布元数据 |
  | 根级单文件 | 8 | `package.json`（5 行变化）、`pnpm-lock.yaml`（77 行变化，净减依赖）、`pnpm-workspace.yaml`（−1）、`vite.config.ts`（+3）、`vitest.browser.config.ts`（16 行变化）、`README.md`（22 行：12+/10−）、`README_en.md`（16 行）、`.gitattributes`（+7 新增） |

- **本仓侧改动规模**（`git diff --name-only 992b3f5f...HEAD`）：**688 个文件**，按目录：`tauri-app/**` **638**（Tauri 迁移主体）、`docs-fork/**` **42**、`.github/**` **3**（**新增** `commit-message-ci.yml`、`gitleaks.toml`、`gitleaks.yml`）、`scripts/**` **2**（**新增** `check-commit-msg.mjs` + `__tests__/check-commit-msg.test.ts`）、`README.md` **1**（+10 行）、`.gitleaks.toml` **1**（新增）、`.gitignore` **1**（+8 行：`tauri-app/src-tauri/target`、`tauri-app/src-tauri/gen/schemas`、`.pi/` 及注释）。
- **上游最近 30 条提交**（共 509 条，按时间倒序）：
  ```
  21d67211 fix(startup): restore installed startup and prepare 1.2.1-Preview (#343)
  1094c3f8 Update README.md (#341)
  8adea7c0 docs(readme): restore previous layout and shorten release summary (#340)
  0d850373 feat(release): prepare 1.2.0-Preview and rewrite user guides (#339)
  78f18fd7 test: remove unused batch browser screenshot captures (#338)
  1f8f22d4 test: keep batch chapter waits inside React act (#337)
  dc74cdeb test(browser): bound relationship graph fixture dimensions (#336)
  464aa953 fix(qualification): reuse protocol snapshot per ledger update (#335)
  8c934037 Merge pull request #331 from EthanYoQ/codex/intel-unit-budget
  9c8aa6b4 ci: budget Intel serial unit qualification
  2c05dcfd Merge pull request #330 from EthanYoQ/codex/planning-retry-ready
  8d795b0c test: wait for planning retry button readiness
  b5459bef Merge pull request #329 from EthanYoQ/codex/batch-test-ready
  e8afc7ff test: synchronize batch draft readiness
  d44c7300 Merge pull request #328 from EthanYoQ/codex/continuation-full-context
  40f92038 fix: supply full predecessor chapters to draft generation
  5d418f72 test: require complete predecessor prose in draft requests
  13e3d348 fix: keep full manuscript context in draft continuations
  f54dd524 test: cover full accumulated draft in continuation prompts
  791439ea Merge pull request #230 from EthanYoQ/codex/novel-quality-program-v3
  a74f4be6 fix: validate archive freeze sidecar before opening snapshot
  b7e22f82 test: reproduce archive staging leak on invalid freeze sidecar
  23734ab1 fix(R006): treat character cards as cleared data in the renderer
  c2ff27f3 fix(R006): also clear upgraded legacy character architecture text
  cde36e8b fix(R245): bound whole-archive WebDAV reads by the transfer deadline
  645b7869 fix(R006): clear character cards with story architecture
  8974a740 test: restore the generation controller fixture root
  4d41efa6 test: make Windows browser CI independent of locale and fixed port
  6a7cdf4f fix: close review gaps missed or regressed in 60865eec
  59cdce8c test: align stale tests with contracts changed in 60865eec
  ```

## 二、冲突热点表

方法：上游侧改动文件集（`git diff --name-only HEAD...upstream/master`）与本仓侧改动文件集（`git diff --name-only 992b3f5f...HEAD`）**求交集** = 双方都改过的文件。

**实测交集 = 1 个文件：`README.md`。** 即「高风险冲突文件数 = 1」，最高危文件 = **`README.md`**（上游 1.2.0-Preview 重写版 12+/10− vs 本仓 +10 行）。

| 文件/路径 | 上游侧 | 本仓侧 | 冲突判定与处理口径 |
|---|---|---|---|
| `README.md` | 改（22 行变化，1.2.0-Preview 发布重写） | 改（+10 行） | ⚠️ **唯一真冲突**，需人工消解（取上游 / 保留本仓 / 手工合并三选一，见 §4 决策点 6） |
| `docs/**`（180 文件） | 改 | **未碰** | 本仓承诺 `docs/` 是上游镜像（逐字节一致、永不修改）→ **整体同步上游版本**；实测零冲突 |
| 根 `package.json` / `vite.config.ts` / `pnpm-lock.yaml` | 改（5 / 3 / 77 行） | **未碰** | 本仓刻意不动 → **取上游**；零冲突。注意 `pnpm-lock.yaml` 净减依赖，合并后需 `pnpm install` 验证 |
| `.gitignore` | **未碰** | 改（+8 行） | **实测上游未改 `.gitignore`** → 无冲突，本仓新增行直接保留；Task C-1 并行任务同期可能再追加忽略行，仅趋势判断（若 C-1 先落，仍不与上游冲突） |
| `electron/**`（264）/ `src/**`（421） | 改 | **未碰** | 本仓保留上游原样 → **取上游**；零冲突（但见 §5 基线漂移风险） |
| `tauri-app/**`（638） | **未碰**（实测确认） | 改 | 本仓独有，上游不会碰 → 本仓侧全部保留；实测零冲突 |
| `.github/**` | 改 5 个既有 workflow | 新增 3 个不同名文件 | 无路径冲突：上游 5 个既有 workflow 取上游；本仓新增 `commit-message-ci.yml` / `gitleaks.toml` / `gitleaks.yml` 保留，两侧并存 |
| `scripts/**` | 改 128 | 新增 2（`check-commit-msg.mjs` + 测试） | 本仓 2 个新增文件不在上游改动集 → 零冲突；上游 128 取上游 |
| `plugins/**`（3）/ `test/**`（4）/ `.release/**`（3）/ `.gitattributes` / `README_en.md` | 改 | **未碰** | 取上游；零冲突 |
| `docs-fork/**`（42）/ `.gitleaks.toml` | **未碰** | 新增 | 本仓独有，保留；零冲突 |

## 三、建议分批策略

**预期冲突：仅 `README.md` 1 个**，其余路径双方无交集，理论上 merge 应「自动合并成功 + 1 个冲突文件」。建议分 7 步（每步收口后再进下一步；合并操作本身须由用户执行，本评估不执行）：

- **批次 0｜预检与基线留档**（合并前）：确认 `git status` 干净；记录当前 HEAD `bdd9d3c3`；跑全量基线自检留档——根仓 `pnpm typecheck` / `pnpm test`（**已知既存失败 `scripts/__tests__/public-repository-hygiene.test.ts`，因本机被忽略的 `AGENTS.md` 触发，判回归时要排除**）；`tauri-app/` 的 `pnpm typecheck` / `pnpm run lint` / `node scripts/verify-channel-coverage.mjs`（当前期望：契约 invoke 频道 **193**（事件频道 5）/ 已注册命令 **185** → 覆盖 invoke 频道 **184** / 未迁移 invoke 频道 **9 [mcp=9]**）；`tauri-app/src-tauri/` 的 `cargo fmt --check` / `cargo check --all-targets` / `cargo test --lib`（当前 **655 passed**，须显式设 `RUSTUP_HOME=D:\Environment\rust\rustup`、`CARGO_HOME=D:\Environment\rust\cargo`）。
- **批次 1｜`docs/` 镜像同步**（180 文件，零冲突）：整体取上游版本（镜像承诺）。验证：`git diff upstream/master -- docs/` 应为空；`git status --porcelain -- docs` 0 行。
- **批次 2｜工程配置层**（根 `package.json` / `pnpm-lock.yaml` / `pnpm-workspace.yaml` / `vite.config.ts` / `vitest.browser.config.ts` / `.gitattributes` / `README_en.md`，零冲突）：取上游。验证：根仓 `pnpm install`（lockfile 有 77 行变化）→ `pnpm typecheck`。
- **批次 3｜主代码层**（`electron/**` 264 + `src/**` 421 + `scripts/**` 128 + `test/**` 4 + `plugins/**` 3 + `.release/**` 3，零冲突但体量最大）：取上游。验证：根仓 `pnpm typecheck` / `pnpm test`（排除已知失败）；**重点复核 `src/shared/ipc-channels.ts`（上游 +187 行）对 tauri-app 频道契约的影响**——合并后立即跑 `node scripts/verify-channel-coverage.mjs`，若 193/185/184/9 基数变化须先对齐再进批次 4（见 §4 决策点 7）。
- **批次 4｜CI 层**（`.github/**`）：上游 5 个既有 workflow 取上游；本仓新增 3 个（Gitleaks ×2 + 提交信息检查）保留。验证：YAML 语法检查；确认两侧 workflow 在 GitHub Actions 上并发策略无重复构建。
- **批次 5｜`README.md` 冲突消解**（唯一真冲突）：按用户决策取向上游重写版 / 保留本仓 +10 行版 / 手工合并。验证：`git diff --check` 无冲突标记。
- **批次 6｜本仓保留层确认 + 收口全量自检**：`tauri-app/**`（638）/ `docs-fork/**`（42）/ `.gitignore` / `.gitleaks.toml` 上游未触碰，天然保留；收口重跑批次 0 全部自检命令，逐项对比基线（`verify-channel-coverage` 期望仍为 193/185/184/未迁移 9 [mcp=9]；`cargo test --lib` 期望 655 passed 无回归），确认 `git status` 干净后由用户完成合并提交。

## 四、待用户决策点

1. **是否合并**：体量 509 提交 / 1018 文件 / +209,574−13,350 行，是否在当前阶段合并。
2. **合并时机**：H4（mcp 9 频道）批次进行中 vs 批次收口后；建议在当前批次收口、全量自检绿后再合。
3. **`docs/` 是否整体覆盖**：按镜像承诺应整体取上游 180 文件（本仓从未改 `docs/`，实测零冲突）；如需保留 fork 专属文档须先迁入 `docs-fork/`。
4. **`.gitignore` 冲突如何解**：实测上游未改 `.gitignore` → 无冲突，本仓 +8 行直接保留；但需知悉并行 Task C-1 可能再追加忽略行，合并时以「保留本仓全部新增行」为口径。
5. **是否接受因上游改动而重跑全量自检**：根仓 + `tauri-app/` + `tauri-app/src-tauri/` 三套自检（含 `cargo test --lib` 约 1 分钟量级）需全部重跑并对比基线。
6. **`README.md` 取舍**：取上游 1.2.0-Preview 重写版 / 保留本仓 +10 行版 / 手工合并。
7. **频道契约对齐口径**：上游 `src/shared/ipc-channels.ts` +187 行后，`tauri-app/src/shared/ipc-channels.ts`（fork 契约副本）是否同步对齐；若不同步，`verify-channel-coverage.mjs` 的 193/185/184/9 基数可能变化。
8. **合并方式**：`git merge` / `rebase` / 逐批 cherry-pick 三选一（本评估不执行，仅列出）。

## 五、未决风险

1. **基线漂移（最高优先级复核项）**：上游演进了 `electron/knowledge-base.ts`（142 行）、`electron/controllers/kb-controller.ts`（18 行）、`electron/repositories/import-run-repository.ts`（174 行）——正是 G4 批次（`kb:import-reference-text` 真实化）所端口的基线；合并后需逐条复核 tauri-app 侧端口是否仍与上游新基线行为一致。
2. **频道契约基数漂移**：`src/shared/ipc-channels.ts` 上游 +187 行，可能改变契约 invoke 频道总数（当前 193）与未迁移计数（当前 9 [mcp=9]），合并后必须复测 `verify-channel-coverage.mjs`。
3. **协议重构**：上游删除 `src/services/vela-protocol.ts`、新增 `src/services/resource-protocol.ts`，渲染层协议面变化，tauri-app 侧 `ipc-client` / resource 相关端口需复核。
4. **仓库体积膨胀**：上游新增大二进制夹具（`vela.db` ×2 约 730 KB、lancedb 切片、`json.gz` 230 KB）于 `scripts/fixtures/`。
5. **测试体量**：`scripts/__tests__/quality-modernization-run.test.mjs` 单文件 +7,683 行，根仓 `pnpm test` 耗时可能显著上升。
6. **已知既存失败**：`scripts/__tests__/public-repository-hygiene.test.ts` 因本机被忽略的 `AGENTS.md` 触发，判回归时必须排除，不可误判为合并引入。
7. **数字持续漂移**：本次 fetch 已把 `upstream/master` 从 `464aa953` 推进到 `21d67211`；若合并继续搁置，509/150 与热点结论需重新实测。
8. **CI 并存**：合并后 `.github/workflows` 为上游 5 改 + 本仓 3 新增并存，需确认 Gitleaks 与提交信息检查不会与上游 CI 重复或互相阻塞。
9. **依赖面变化**：`pnpm-lock.yaml` 77 行变化（净减），合并后首次 `pnpm install` 需验证解析一致。

---

**自检**：本文件存在且「分叉量化数据 / 冲突热点表 / 建议分批策略 / 待用户决策点 / 未决风险」五部分齐全；`git rev-list --left-right --count upstream/master...HEAD` 原始输出为 `509	150`（见 §0）；除本新增文档与 `docs-fork/todo.md` 本条目更新外未改动任何文件。
