# G2 开工清单：导入运行状态机（读面 + 解析写入 + author prepare）

> 2026-10-10 产出（G1 交付并推送后立即调研）。**分批已按调研结果细化为 G2a / G2b**（见 §1），
> 下一次会话可直接按 §3 编码，**无需重新调研**；§4 列出尚未读的剩余规范（约 620 行）。
>
> 前置：G1 已交付并推送（`fbb88307` / `75379748` / `e2bb92cc`）。G1 已铺好
> `src/import/{mod,limits,inspection_store,parsing}.rs`、`commands/import.rs`、
> `db:import-run-author-preview` 与**无密钥 sha256 别名**（`location_alias_digest` / `file_alias_digest`）。

## 1. 分批修订（为何原 G2 拆成 G2a / G2b）

调研 `electron/repositories/import-run-repository.ts`（2552 行）后发现：

| 发现 | 影响 |
|---|---|
| **读面与写面完全解耦**：`get` / `listResumable` / `listChapterBatch` 只依赖 `rowToSnapshot` / `persistedProgress` / `chapterRowToSnapshot` 投影，不触碰状态机 | G2 的「读面」可独立交付且可先验收 → 拆出 **G2a** |
| `beginParsing` 显式拒绝 `author-manuscript`（`'当前版本不支持作者手稿导入'`），作者路径走 `prepare` | 两条写入路径都要落在 G2b |
| `prepare` 的 **reference 分支在桌面流程中不可达**（`db:import-run-prepare-inspection` 的 reference 分支走 `beginParsing`；`prepare` 的 reference 分支只被基线单测与 `kb:import-reference-text` 间接使用） | 仍**整函数平移**（共享 prologue + normalize，拆函数反而制造返工），配 Rust 单测覆盖 |
| `adoptLegacyCompletedRun` 在 **Tauri 项目内恒为空转**（依赖 `legacy_source_fingerprint` / `legacy:<fingerprint>` 的 source_id，D3 下永不出现） | 保留平移（约 70 行纯 SQL，无副作用）或按 D3 直接省略——**建议保留**，避免将来复活基线行为时再补 |
| `resolveReferenceImportAuthority` / `commitReferenceImportReceipt`（`:1297-1412`）只服务 `kb:import-reference-text` | **仍属 G4**，G2 不做 |

**修订后的分批**（G1 清单原 G2/G3/G4 不变，仅 G2 细分）：

| 分批 | 范围 | 频道 |
|---|---|---|
| **G2a（读面）** | `repositories/import_run_repository.rs` 投影层：`row_to_snapshot` / `persisted_progress` / `source_progress` / `unfinished_source_display` / `chapter_row_to_snapshot` / `assert_frozen_chapter_snapshot` + `get` / `list_resumable` / `list_chapter_batch` | `db:import-run-get`、`-list-resumable`、`-list-chapters`（3） |
| **G2b（写面）** | 平移 `beginParsing` / `commitParsedSource` / `failParsedSource` / `finalizeParsing` / `prepare` + `normalize{Display,SourceIds,SourceFingerprints,Chapters}` / `assignStableChapterNumbers` / `completedChapterManifest` / `createPreparationInspection` / `hashManifest` / `adoptLegacyCompletedRun` + **身份解析（新模块）** `import/identity.rs`（无密钥版 `resolveEncodedSources`） | `db:import-run-prepare-inspection`、`-finalize-parsing`（2）+ **复活 `dialog:select-novel-files` 的 `reference` 分支**（G1 的诚实错误改为真实链路） |
| **G3** | 执行租约与批次推进（`startOrResume` / `renewExecution` / `restart` / `requestCancel` / `cancelAtBoundary` / `completeBatch` / `advanceStage` / `fail` / `complete`）+ effect receipts（`prepareEffectReceipt` / `commitEffectReceipt` / `getEffectReceipt`）+ `assertExecution` / checkpoint 族 + `db:import-global-facts-commit` | 11 + 1 |
| **G4** | `kb:import-reference-text` 去占位（`resolveReferenceImportAuthority` / `commitReferenceImportReceipt`）+ 前端登记 + GUI 冒烟 | 1 |

## 2. 已确认的决策与刻意偏离（G2 相关）

| # | 决策/偏离 | 理由 |
|---|---|---|
| D3′ | **`legacy_source_fingerprint` / `legacy_collection_fingerprint` 恒为空**（延续 G1 D3）：G2 里 `resolveEncodedSources` 不产出 `legacyStableIdentity`，`beginParsing` / `prepare` 收到的 legacy 入参恒为 `''` | L3 双栈项目目录刻意不互通，无遗留身份可桥；`import_legacy_identity_bridge` 表刻意不建 |
| D1′ | **`sourceFingerprint` / `sourceFingerprints` 同样改为无密钥 `sha256`**（基线为 `HMAC(applicationSecret, JSON({version:1,purpose,sourceIds:[sorted]}))`） | 与 G1 D1 同源：`sourceId` 是本地新生成的 UUID，指纹只在本项目库内自洽；避免 `hmac` 新依赖 |
| D9 | **`db:import-run-prepare-inspection` 的失败形态**：业务失败 → `{success:false, errorCode?}`／`{success:false, error}` 信封；项目门禁失败 → invoke reject（与 G1 一致） | 与 G1 §3.1-4 的门禁/业务分层保持同构 |
| D10 | **`AuthorImportPreviewStaleError` 用 `errorCode: 'AUTHOR_IMPORT_PREVIEW_STALE'` 表达**（Rust 侧无异常类，用结构化错误码） | 契约 `ImportRunPrepareFromInspectionResult` 已有该分支；渲染层 `isAuthorImportPreviewStaleError` 按 `code` 判别 |
| D2′ | 继续不迁移 `webContentsId`；`consume(inspectionId)` 不带归属参数 | 延续 G1 D2（单窗口单逻辑会话） |

## 3. G2 需要落地的清单

### 3.1 G2a（读面）

1. **`src-tauri/src/repositories/import_run_repository.rs`**（新建）：
   - 行结构体 `ImportRunRow` / `ImportRunChapterRow`（列集见 §5 既有件）
   - `row_to_snapshot(conn, row) -> ImportRunSnapshot`（对齐契约 `ImportRunSnapshot`，含 `completedBatches` / `resumable` / `cancelRequested` / `manifest*` / `unfinishedSourceDisplay`）
   - `persisted_progress(conn, row)`、`source_progress(conn, run_id)`、`unfinished_source_display(conn, run_id)`
   - `chapter_row_to_snapshot` + `assert_frozen_chapter_snapshot`（正文哈希校验）
   - `get(run_id)`、`list_resumable()`、`list_chapter_batch(run_id, after, limit)`（`MAX_PAGE_SIZE = 100` 截断）
2. **`commands/db.rs` 追加 3 命令**：`db_import_run_get`、`db_import_run_list_resumable`、`db_import_run_list_chapters`
   （读频道：失败 reject；均带 `expected_project_path` + `project_session` 门禁）。
3. **前端登记**：`ipc-client.ts` 增 `'db:import-run-get': ['runId','expectedProjectPath']`、
   `'db:import-run-list-resumable': ['expectedProjectPath']`、
   `'db:import-run-list-chapters': ['runId','afterChapterNumber','limit','expectedProjectPath']`；
   重新生成 `migrated-channels.ts`。

### 3.2 G2b（写面）

1. **`src-tauri/src/import/identity.rs`**（新建，平移 `import-source-identity-repository.ts:71-192` 的 `resolveEncodedSources`）：
   - 入参：G1 产出的 `(location_alias_digest, file_alias_digest?)` 列表 + `purpose`
   - 事务内：按 `location_alias_digest` 查 `import_source_aliases` → 命中复用 `source_id`；
     否则按 `file_alias_digest` 查 → 命中复用；否则 `random_uuid_v4()`；
     upsert 两条别名（`alias_kind` = `location` / `file`）
   - 产出：`source_ids`、`source_fingerprint`（**sha256** 版）、`source_fingerprints`（逐 sourceId）
   - 校验：两个摘要均须 `^[a-f0-9]{64}$`；列表长度 1..=5000（否则「导入来源别名无效」）
2. **`src-tauri/src/repositories/import_run_repository.rs` 续写**：
   - `begin_parsing`（`:1413-1567`）：三条分支（显式 run 重新授权 / 命中既有 parsing run / 新建），
     含 `source_index + MAX_IMPORT_SOURCE_FILES` 的**避让写法**（绕开 `UNIQUE(run_id, source_index)`）
   - `commit_parsed_source`（`:1568-1640`）、`fail_parsed_source`（`:1641-1661`）、`parsed_source_status`
   - `finalize_parsing`（`:1662-1847`）：manifest 组装 → `assignStableChapterNumbers` → 四态分类
     （`new` / `resumable` / `conflict` / `exact-duplicate`）+ 各分支的删表/丢弃语义
   - `prepare`（`:1848-2139`）：author 分支（复用已迁移的 `finalized_draft_import_repository::preview`
     比对 `manifestFingerprint` / `authorityFingerprint`；`matchingResumableRun` / `fenceUncommittedAuthorRun` /
     `hasCommittedAuthorFinalizationReceipt`）+ reference 分支
   - 辅助：`normalize_display` / `normalize_source_ids` / `normalize_source_fingerprints` / `normalize_chapters` /
     `create_preparation_inspection` / `source_chapter_key` / `completed_chapter_manifest` /
     `assign_stable_chapter_numbers` / `hash_manifest` / `canonical_manifest` / `matching_resumable_run` /
     `latest_completed_run` / `overlapping_resumable_source_run` / `discard_provisional_parsing_run` /
     `adopt_legacy_completed_run`（恒空转）/ `read_run_row`
3. **`commands/db.rs` 追加 2 命令**：
   - `db_import_run_prepare_inspection(request, expected_project_path, project_session)`：
     `peek(inspectionId, purpose)` → author 分支调 `preview` 做 `AUTHOR_IMPORT_PREVIEW_STALE` 预检 →
     `consume(inspectionId)` → `identity::resolve` → `prepare`（author）/ `begin_parsing` + 逐来源
     `commit_parsed_source`（失败落 `fail_parsed_source`）+ `finalize_parsing`（reference）；
     **idempotency 保留**：`peek` 失败 → 基线文案「导入检查已失效，请重新选择文件」
   - `db_import_run_finalize_parsing(run_id, expected_project_path, project_session)`
4. **`commands/import.rs` 复活 reference 分支**：删掉 `reference_import_unmigrated_message` 的早退，
   改为移植 `import-controller.ts:395-470` 的 `parsingContext`（`begin_parsing`）+ 逐来源
   `commitParsedSource` / `failParsedSource` + 末尾返回 `{ success:true, preparation }`；
   **同时删除 `.epub` 之外的 D4 说明**（`reference` 的诚实错误不再是 G2 契约）。
5. **前端登记**：`ipc-client.ts` 增 `'db:import-run-prepare-inspection': ['request','expectedProjectPath']`、
   `'db:import-run-finalize-parsing': ['runId','expectedProjectPath']`；重新生成 `migrated-channels.ts`。

### 3.3 测试（目标 +25～35）

| 组 | 用例 |
|---|---|
| G2a 投影 | `row_to_snapshot` 全字段形状（含 `unfinishedSourceDisplay`）；`persisted_progress` 对 `knowledge`/`blueprints` checkpoint 的完成章数；`list_chapter_batch` 分页 + `MAX_PAGE_SIZE` 截断；`assert_frozen_chapter_snapshot` 拒绝被篡改的正文 |
| identity | 首次解析生成新 sourceId 并落两条别名；同位置二次解析复用 sourceId；**改名（location 变、file 同）复用 sourceId**；同位置但 file 变仍复用 location 命中；摘要非法后缀拒 |
| beginParsing | 新建 run（stage=parsing、effectNamespace=`import:<purpose>:<runId>`）；显式 runId 重授权；命中既有 parsing run 的 `source_index + 5000` 避让；author-manuscript → 明确报错 |
| commitParsedSource / failParsedSource | 状态迁移 + `chapter_count`/`content_size`/`word_count` 汇总；失败态文案；`parsed_source_status` |
| finalizeParsing | `new`（写入 `import_run_chapters` + `import_source_chapter_map` 映射）；`exact-duplicate`（删 run）；`conflict`（丢弃临时 run + 冲突章号）；`resumable`（命中既有可恢复 run）；章号稳定性（同一 `(source_id, source_chapter_number)` 跨 run 恒得同章号） |
| prepare（author） | 预览过期 → `AUTHOR_IMPORT_PREVIEW_STALE`；`exact-duplicate`；连续追加 `new`（`total_chapters` = 新增章数）；`conflict` 三种文案；`resumable` 命中 |
| prepare（reference） | 与基线 `knowledge-base-reference-receipt.test.ts` 等价的准备结果形状 |
| 命令层 | `prepare-inspection` 的 peek/consume 语义（成功后检视失效、失败保留）；门禁拒绝；`finalize-parsing` 的入参校验 |

### 3.4 验收

```bash
# tauri-app/src-tauri
cargo test --lib && cargo check --all-targets && cargo fmt --check
# tauri-app
pnpm typecheck && pnpm run lint
node scripts/verify-channel-coverage.mjs --quiet   # G2a 后 193/170/169/24；G2b 后 193/172/171/22
npx vitest run test/channel-migration-coverage.test.ts test/ipc-arg-struct-contract.test.ts
```
GUI 冒烟：① 作者原稿全链路（G1 已过，回归确认）；② **参考语料**：选 `.md` → 应得到 `preparation`
（`classification` 与预览）而非诚实错误；再选同一文件 → 应得 `exact-duplicate`。

## 4. 尚未读的剩余规范（下次开工先读，约 620 行）

| 文件:行 | 行数 | 用途 |
|---|---|---|
| `electron/repositories/import-run-repository.ts:1568-1661` | 94 | `commitParsedSource` / `failParsedSource` 逐字实现（§3.2-2） |
| `electron/repositories/import-run-repository.ts:1988-2139` | 152 | `prepare` 的 reference 分支逐字实现 |
| `electron/repositories/import-run-repository.ts:570-720` | 151 | `sourceProgress` / `unfinishedSourceDisplay` / `completedCheckpointChapters` / `persistedProgress` / `rowToSnapshot` / `chapterRowToSnapshot` / `assertFrozenChapterSnapshot`（G2a 主体） |
| `electron/repositories/import-run-repository.ts:206-259` | 54 | `canonicalManifest` / `hashManifest` / `parseJson` / `canonicalize` / `canonicalPayload`（黄金哈希口径） |
| `electron/repositories/import-run-repository.ts:1-96` | 96 | `ImportRunRow` / `ImportRunChapterRow` / `NormalizedImportRunChapter` 列与类型（含 §5 的行列名） |
| `src/shared/import-run.ts:101-185` | 85 | `ImportRunChapterBatchCheckpoint` / receipt 类型（G3 才用，G2 可略读） |
| `electron/repositories/import-source-identity-repository.ts:121-160` | 40 | `resolveEncodedSources` 的 sourceId 复用与 fingerprint 组装（**已读**，此处只作复核） |

> 已读并可直接照做：`import-run-repository.ts:96-135`（`createPreparationInspection`）、`:721-806`（`normalize*`）、
> `:1057-1126`（`sourceChapterKey` / `completedChapterManifest` / `assignStableChapterNumbers`）、
> `:1196-1280`（`matchingResumableRun` / `latestCompletedRun` / `overlappingResumableSourceRun` /
> `discardProvisionalParsingRun` / `fenceUncommittedAuthorRun`）、`:1282-1288`（`parsedSourceStatus`）、
> `:1413-1567`（`beginParsing`）、`:1662-1847`（`finalizeParsing`）、`:1848-1987`（`prepare` 前半）、
> `:2140-2168`（`get` / `listResumable` / `listChapterBatch`）、
> `electron/controllers/db-controller.ts:213-350`（5 频道入参组装）、
> `electron/controllers/import-controller.ts:297-671`（G1 已平移，reference 分支待复活）。

## 5. 已确认的既有可复用件

- **schema 已落地**（`cca792cd`，9 张表 + 7 索引）：`import_runs`(28 列) / `import_run_chapters`(8) /
  `import_run_sources`(13) / `import_run_source_chapters`(8) / `import_source_aliases`(4) /
  `import_source_chapter_map`(4) / `import_run_receipts`(12) / `import_run_knowledge_receipts`(10) /
  `import_reference_documents`(9)。**列集与基线一致，无需改 schema**。
- `finalized_draft_import_repository::{preview, AuthorManuscriptImportPreview}` —— E 批次已迁移，`prepare`
  author 分支直接复用（含 `manifest_fingerprint` / `authority_fingerprint` / `classification` / `newChapterNumbers` / …）。
- G1 件：`import/parsing.rs::{sha256_hex, location_alias_digest, file_alias_digest}`、
  `import/inspection_store.rs::{peek, consume, ImportInspection, ImportInspectionSummary}`、
  `import/limits.rs::{MAX_IMPORT_CHAPTERS, MAX_IMPORT_SOURCE_FILES, MAX_IMPORT_TOTAL_BYTES}`。
- `project_access::random_uuid_v4`（sourceId / runId）、`crate::commands::db::{guard_read, mutating_error}`。
- `db_controller` 侧错误码信封：`{success, preparation?, error?, errorCode?}`（对齐 `ImportRunPrepareFromInspectionResult`）。

## 6. 已知风险 / 需注意

1. **`source_index + 5000` 避让写法**（`beginParsing`）会**临时**把 index 顶到 5000+，
   随后逐条写回真实 index；Rust 侧必须放在同一事务内，否则 `UNIQUE(run_id, source_index)` 会炸。
2. **`finalizeParsing` 的删表语义**：`exact-duplicate` 直接 `DELETE FROM import_runs WHERE id = ?`（无 stage 守卫），
   而 `resumable` / `conflict` 走 `discardProvisionalParsingRun`（有 `stage='parsing' AND status IN ('ready','failed')` 守卫）。
   两者语义不同，不要合并。
3. **`prepare` 的 `chaptersToPersist` 过滤**：author 分支只落 `preview.newChapterNumbers` 的章
   （duplicate 不落库），但 `total_chapters` 也只为新增章数；`manifest_chapter_count` 才是全量。
4. **`normalizeChapters` 的 `bytes === 0` 拒绝**：空正文在 store 层已被拦，但 repo 层仍须复刻。
5. **`title.length > 500`** 的上限在 `normalizeChapters`（G1 的 store 未校验 title 长度）——不要漏。
