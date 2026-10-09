# G1 开工清单（修订版）：`dialog:select-novel-files` + `db:import-run-author-preview`

> 2026-10-10 产出。**分批方案已按调研结果修订**（见 §1），两个决策已获用户确认（§2）。
> 下次会话可直接按 §4 编码，**无需重新调研**；§5 列出尚未读的剩余规范（约 370 行）。

## 1. 分批修订（为何与原计划不同）

调研 `electron/controllers/import-controller.ts:297-671` 后发现：

| 发现 | 影响 |
|---|---|
| 渲染层**只用结构化请求**路径（`ImportNovelDialog.tsx:198` 传 `{runId, purpose, locale, expectedProjectPath}`） | 原「G1 只做检视面」无法支撑 UI |
| `author-manuscript` 分支只需 `result.inspection` | ✅ 不依赖状态机 |
| `reference` 分支需要 `result.preparation` → 依赖 **`ImportRunRepository.beginParsing`**（2551 行状态机核心，原属 G2） | ❌ 属 G2 |

**修订后的分批**（用户已确认）：

| 分批 | 范围 |
|---|---|
| **G1（本清单）** | 交付**「导入作者原稿」完整可用链路**：`dialog:select-novel-files`（author-manuscript 路径）+ inspection store + 章节解析 + `db:import-run-author-preview`（**复用已迁移**的 `finalized_draft_import_repository::preview`）。`reference` 路径返回**诚实错误**（依赖 G2 状态机），并登记为临时缺口 |
| **G2** | 状态机主体 + `reference` 路径（`beginParsing` / `prepare` / `finalizeParsing` / get / listResumable / listChapterBatch） |
| **G3** | 执行租约与批次推进 + effect receipts + `db:import-global-facts-commit` |
| **G4** | `kb:import-reference-text` 去占位 + 前端登记 + GUI 冒烟 |

## 2. 已确认的决策与刻意偏离（G1 相关）

| # | 决策/偏离 | 理由 |
|---|---|---|
| D1 | **来源身份摘要改为无密钥 `sha256`**（`alias_digest = sha256(规范化位置)`，文件标识同理） | 基线用 `aliasDigest(applicationSecret, kind, value)`（密钥化）。Tauri 侧身份**从不进入渲染层**，且项目库已存 `display_json`（文件名）与 `content_snapshot`（正文）——「隐藏路径」的边际安全价值极低；**零新依赖**（`hmac` 需另批审批）。须在快照登记为刻意偏离 |
| D2 | **不迁移 `webContentsId` 归属校验** | 与 `external_grant.rs` 既有决定一致（Tauri 单窗口，无多 webContents 语义）。`revokeForWebContents(id)` → 退化为「清空全部待处理检查」 |
| D3 | **不建 `import_legacy_identity_bridge`**（schema 已获批）→ `ImportSourceIdentityRepository` 的 **legacy 解析路径整体跳过** | L3 两栈项目目录刻意不互通，无遗留身份可桥；故 `legacySourceFingerprints` / `legacyCollectionFingerprint` 恒为空 |
| D4 | **`.epub` 暂不支持**：对话框仍列出 epub（与基线筛选器一致），但选中后返回**诚实错误**（如「EPUB 导入尚未迁移：需要解包依赖，已登记待批」） | Rust 解包需新增 `zip` 类依赖（Ask first）；txt / md / text 已覆盖主要用法 |
| D5 | **文件读取直接 `std::fs`（带字节上限）**，不引入 base 线 `windowsSafeFileSystem` 句柄链 | 命令在 Rust 内完成「选择 → 受限读取 → 解析」，路径从不回传渲染层；上限与基线一致（单章 16 MiB / 总量 128 MiB） |
| D6 | 对话框用 **`tauri-plugin-dialog`**（已接入）：`pick_files` + 筛选器（小说文件 `txt/md/text/epub` + 所有文件）+ 多选 | 与 `dialog:select-knowledge-files` 同路径，父窗口绑定已有先例 |

## 3. G1 需要落地的清单

### 3.1 Rust

1. **`src/import/limits.rs`**：`MAX_IMPORT_CHAPTERS = 5_000`、`MAX_IMPORT_SOURCE_FILES = 5_000`、
   `MAX_IMPORT_TOTAL_BYTES = 128 MiB`、`MAX_IMPORT_CHAPTER_BYTES = 16 MiB`、
   `IMPORT_INSPECTION_TTL = 10 min`、`MAX_ACTIVE_INSPECTIONS = 2`（对齐 `src/shared/import-limits.ts`）
2. **`src/import/inspection_store.rs`**：平移 `import-inspection-store.ts`（192 行）——`create` 的全部校验
   （章节数 1..=5000、来源数、SHA256 摘要格式、displayName ≤255 且不含 `/ \ NUL`、mediaType ≤100、
   size 安全整数、章节号唯一/来源内 `(sourceIndex, sourceChapterNumber)` 唯一、单章 ≤16 MiB、
   总量 ≤128 MiB、保留他人合计 ≤maxAggregateBytes、他人活跃 < 2）、**同会话重选即替换**、
   `consume` / `peek(purpose)` / `clear()` / `active_count()`、TTL 过期清理。
   ⚠️ 删除 `webContentsId`（D2）；错误文案逐字对齐基线。
3. **`src/import/parsing.rs`**：平移 `import-controller.ts:73-296` 的纯函数——
   `default_file_identity`、`source_media_type`、`sha256_hex`、`chinese_num_to_arabic`、
   `extract_chapter_number`、`is_chapter_heading`、`extract_title`、`has_chapter_headings`、
   `split_single_file_content`，以及字数口径 `crate::draft_units::count_draft_units`（已存在）。
4. **`src/commands/import.rs`**：`dialog_select_novel_files`（async + `app` + `state` + `project_session`）
   - 参数：`request: Option<Value>`（字符串 purpose 或结构化对象）、`project_session: Option<ProjectSessionContext>`
   - 结构化请求：`guard_read` 同源校验（会话 + `expectedProjectPath`）→ 冻结项目
   - 对话框 → 取消返回 `null`
   - 预检：`stat` 每个文件 → 计数/单文件/累计上限 → 按 `displayName` 的 zh-CN 数值排序（对齐 `localeCompare(…, 'zh-CN', {numeric:true})`）
   - 逐文件：`.epub` → D4 诚实错误；否则受限读取 → `trim()` → 空文件/仅标题 → 对应错误文案
   - 章节组装（含本地 `sourceChapterNumber` 去重）→ author-manuscript 重复章号 → `AUTHOR_MANUSCRIPT_DUPLICATE_CHAPTER:<n>` → 文案
   - `purpose === 'reference'` → 返回诚实错误（依赖 G2）；`author-manuscript` → `inspection.store.create(...)` → `{ success: true, inspection }`
   - 失败路径：清空待处理检查 + 文案映射（`IMPORT_SOURCE_COUNT_EXCEEDED` / `IMPORT_SOURCE_SIZE_INVALID` /
     `IMPORT_SOURCE_BYTES_EXCEEDED` / `IMPORT_CHAPTER_COUNT_EXCEEDED` / `IMPORT_PURPOSE_INVALID` → 基线 `importSelectionErrorMessage`，见 §5）
5. **`src/commands/db.rs` 追加 `db_import_run_author_preview`**：`peek(inspectionId, purpose='author-manuscript')`
   → 取章节 → 调 **已迁移**的 `finalized_draft_import_repository::preview(...)` → 返回 `AuthorManuscriptImportPreview`
   （须先读 `db-controller.ts:311` 确认入参组装与错误口径，见 §5）
6. **`state.rs`**：`import_inspections: Mutex<ImportInspectionStore>`
7. **`lib.rs`**：`pub mod import;` + 注册 2 个命令

### 3.2 前端

- `ipc-client.ts`：`'dialog:select-novel-files'` **已有** `CHANNEL_ARG_NAMES` 登记（无需改）；
  `db:import-run-author-preview` 需新增 `['inspectionId','expectedProjectPath']`
- `node scripts/verify-channel-coverage.mjs --emit` 重新生成 `migrated-channels.ts`（映射新增 2 频道怪）
- `test/channel-migration-coverage.test.ts`：加 2 条 `MIGRATED_CHANNELS.has(...)` 断言

### 3.3 测试（目标 +12～16）

| 组 | 用例 |
|---|---|
| inspection store | 正常 create（summary 形状/预览前 8 章）；章节数 0 / >5000；来源数 0 / >5000；displayName 含 `/`；摘要非 64 位 hex；单章 >16 MiB；总量 >128 MiB；同会话重选替换；TTL 过期；`peek` purpose 不匹配；`consume` 后失效 |
| parsing | `chinese_num_to_arabic`（一/十/十一/二十三/一百零五/无效）；`extract_chapter_number`（第X章/第X节/数字/无）；`is_chapter_heading`；`extract_title`；`has_chapter_headings`；`split_single_file_content`（多章切分、maxChapters 截断、无标题兜底） |
| 命令层 | 非结构化 purpose 校验；结构化请求的会话/路径门禁；上限映射到基线文案；epub 诚实错误；reference 诚实错误；重复章号文案 |

### 3.4 验收

```bash
# tauri-app/src-tauri
cargo test --lib && cargo check --all-targets && cargo fmt --check
# tauri-app
pnpm typecheck && pnpm run lint
node scripts/verify-channel-coverage.mjs --quiet   # 预期 193 契约 / 167 命令 / 166 覆盖 / 27 未迁移
npx vitest run test/channel-migration-coverage.test.ts test/ipc-arg-struct-contract.test.ts
```
GUI 冒烟：`ImportNovelDialog` → 选 2 个 .txt → 作者原稿预览（`db:import-run-author-preview`）；
再选 1 个 .md 作用「参考语料」→ 应显示 G2 依赖的诚实错误。

## 4. 尚未读的剩余规范（下次开工先读，约 370 行）

| 文件:行 | 行数 | 用途 |
|---|---|---|
| `electron/controllers/import-controller.ts:73-296` | 224 | **解析纯函数逐字实现**（§3.1-3） |
| `electron/controllers/import-controller.ts:92-147` | 56 | `importSelectionErrorMessage`（错误码 → 文案映射） |
| `electron/controllers/db-controller.ts:200-340` | 140 | `db:import-run-prepare-inspection` + `db:import-run-author-preview` 的确切入参组装 |
| `src/shared/import-run.ts` | 352 | `ImportInspectionSummary` / `ImportRunPreparationResult` / `AuthorManuscriptImportPreview` 等类型（按需） |

## 5. 已确认的既有可复用件

- `finalized_draft_import_repository::{preview, AuthorManuscriptImportPreview}` —— **E 批次已迁移**，
  `db:import-run-author-preview` 直接复用（含 `//! 对齐契约 AuthorManuscriptImportPreview` 注释）。
- `crate::draft_units::count_draft_units`（字数口径）与 `project_access::random_uuid_v4`（inspectionId）。
- `tauri-plugin-dialog` 的多选文件对话框（`dialog_select_knowledge_files` 已有同型实现可抄）。
- schema：`import_runs` / `import_run_chapters` / `import_run_sources` / `import_run_source_chapters` /
  `import_source_chapter_map` / `import_run_receipts` / `import_run_knowledge_receipts` /
  `import_reference_documents` / `import_source_aliases`（**已落地**）。
