//! 批次 G1：导入文件选择 —— 平移自 `electron/controllers/import-controller.ts:297-671`。
//!
//! 频道：`dialog:select-novel-files`（[`dialog_select_novel_files`]）。
//!
//! # 交付范围（G1）
//!
//! 「导入作者原稿」完整链路：**选择 → 受限读取 → 拆章 → 检视存储**；
//! 渲染层只拿到 [`ImportInspectionSummary`]。`reference`（参考语料）路径依赖
//! 批次 G2 的导入运行状态机，G1 返回**诚实错误**。`.epub` 同理（D4）。
//!
//! # 失败的两种形态（刻意区分）
//!
//! | 形态 | 触发 | 前端表现 |
//! |---|---|---|
//! | `Err(String)`（invoke reject） | 项目租约 / `expectedProjectPath` 门禁失败 | 直接展示守卫文案 |
//! | `Ok({success:false, error})` | 业务失败（上限 / 空文件 / 重号 / epub / reference） | 展示映射后的基线文案 |
//!
//! 基线把**两者**都吞进 `{success:false,error}`。迁移侧按 Tauri 既有约定把门禁
//! 失败提升为 reject（与 `db:*` / `fs:*` 全部项目域命令一致），业务失败保持信封。

use std::io::Read;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::commands::db::guard_read;
use crate::commands::project::file_path_to_string;
use crate::draft_units::count_draft_units;
use crate::import::inspection_store::{
    ImportInspectionStore, ImportInspectionSummary, InspectedImportChapter, InspectedImportSource,
};
use crate::import::limits::{MAX_IMPORT_CHAPTERS, MAX_IMPORT_SOURCE_FILES, MAX_IMPORT_TOTAL_BYTES};
use crate::import::parsing::{
    self, default_file_identity, file_alias_digest, location_alias_digest, ParsedChapter,
};
use crate::import::{ImportPurpose, ImportRunLocale};
use crate::security::ProjectSessionContext;
use crate::state::AppState;

/// 主窗口标签（对话框父窗口绑定）
const MAIN_WINDOW_LABEL: &str = "main";
/// 对话框等待上限（安全网，与 `dialog:select-knowledge-*` 一致）
const DIALOG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// 契约 `dialog:select-novel-files` 的返回类型（`| null` 由 `Option` 表达）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NovelFileSelectionResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inspection: Option<ImportInspectionSummary>,
    /// G1 恒为 `None`（`reference` 路径依赖 G2 状态机）；保留字段以对齐契约。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preparation: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl NovelFileSelectionResult {
    fn ok(inspection: ImportInspectionSummary) -> Self {
        Self {
            success: true,
            inspection: Some(inspection),
            preparation: None,
            error: None,
        }
    }

    fn failure(error: String) -> Self {
        Self {
            success: false,
            inspection: None,
            preparation: None,
            error: Some(error),
        }
    }
}

/// 结构化请求已解析出的冻结字段（对齐基线 `structuredRequest`）
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedSelectionRequest {
    pub purpose: ImportPurpose,
    pub locale: Option<ImportRunLocale>,
    /// 结构化请求才做项目门禁；`None` = 基线 `expectedProjectPath` 为 undefined
    pub expected_project_path: Option<String>,
    pub structured: bool,
}

/// 平移自基线 `typeof request === 'string' ? request : (structured?.purpose ?? 'reference')`。
///
/// 只有**字符串**请求里出现未知用途才报 `IMPORT_PURPOSE_INVALID`；对象缺 `purpose`
/// 或非对象请求一律落回 `reference`（与基线可选链一致）。
pub(crate) fn parse_selection_request(
    request: &Option<Value>,
) -> Result<ParsedSelectionRequest, String> {
    let object = match request {
        Some(Value::Object(map)) => Some(map),
        _ => None,
    };
    let purpose = match request {
        Some(Value::String(raw)) => match raw.as_str() {
            "reference" => ImportPurpose::Reference,
            "author-manuscript" => ImportPurpose::AuthorManuscript,
            _ => return Err("IMPORT_PURPOSE_INVALID".to_string()),
        },
        Some(Value::Object(map)) => map
            .get("purpose")
            .and_then(|value| serde_json::from_value::<ImportPurpose>(value.clone()).ok())
            .unwrap_or(ImportPurpose::Reference),
        _ => ImportPurpose::Reference,
    };
    let locale = object
        .and_then(|map| map.get("locale"))
        .and_then(|value| serde_json::from_value::<ImportRunLocale>(value.clone()).ok());
    let expected_project_path = object
        .and_then(|map| map.get("expectedProjectPath"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(ParsedSelectionRequest {
        purpose,
        locale,
        expected_project_path,
        structured: object.is_some(),
    })
}

/// 语言选择（对齐基线 `importText(locale, zh, en)`）
pub(crate) fn import_text(
    locale: ImportRunLocale,
    zh_cn: impl Into<String>,
    en_us: impl Into<String>,
) -> String {
    match locale {
        ImportRunLocale::EnUs => en_us.into(),
        ImportRunLocale::ZhCn => zh_cn.into(),
    }
}

/// 平移自 `importSelectionErrorMessage`（错误码 → 基线文案；未知码 → 通用文案）。
pub(crate) fn import_selection_error_message(
    error_message: &str,
    locale: ImportRunLocale,
) -> String {
    match error_message {
        "IMPORT_SOURCE_COUNT_EXCEEDED" => import_text(
            locale,
            format!("所选文件数量超过导入上限（最多 {MAX_IMPORT_SOURCE_FILES} 个）。"),
            format!(
                "The import source-file limit was exceeded (limit: {MAX_IMPORT_SOURCE_FILES})."
            ),
        ),
        "IMPORT_SOURCE_BYTES_EXCEEDED" | "SECURE_FS_FILE_TOO_LARGE" => import_text(
            locale,
            format!("所选文件总大小超过导入上限（最多 {MAX_IMPORT_TOTAL_BYTES} 字节）。"),
            format!(
                "The import source-size limit was exceeded (limit: {MAX_IMPORT_TOTAL_BYTES} bytes)."
            ),
        ),
        "IMPORT_CHAPTER_COUNT_EXCEEDED" => import_text(
            locale,
            format!("拆分后的章节数超过导入上限（最多 {MAX_IMPORT_CHAPTERS} 章）。"),
            format!("The import chapter limit was exceeded (limit: {MAX_IMPORT_CHAPTERS})."),
        ),
        _ => import_text(
            locale,
            "无法读取所选文件；请重新选择后再试。",
            "Could not read the selected files. Please choose them again.",
        ),
    }
}

/// D4：`.epub` 诚实错误
fn epub_import_unmigrated_message(locale: ImportRunLocale) -> String {
    import_text(
        locale,
        "EPUB 导入尚未迁移：需要新增解包依赖（已登记待批）。请改用 txt / md 文件。",
        "EPUB import is not migrated yet: it requires a new unpacking dependency (registered for approval). Please use txt / md files instead.",
    )
}

/// G2 依赖：`reference` 诚实错误
fn reference_import_unmigrated_message(locale: ImportRunLocale) -> String {
    import_text(
        locale,
        "参考语料导入尚未迁移：它依赖批次 G2 的导入运行状态机。本次仅支持「作者原稿」导入。",
        "Reference-corpus import is not migrated yet: it depends on the batch G2 import-run state machine. Only author-manuscript import is available for now.",
    )
}

/// 空来源文案（对齐基线两条专用错误）
fn empty_source_message(locale: ImportRunLocale) -> String {
    import_text(
        locale,
        "一个或多个所选文件为空。请补充小说正文后，重新选择未完成的文件。",
        "One or more selected files are empty. Add novel text and choose the unfinished files again.",
    )
}

/// 仅标题来源文案
fn title_only_source_message(locale: ImportRunLocale) -> String {
    import_text(
        locale,
        "一个或多个所选文件只有章节标题，没有可导入的正文。请补充小说正文后，重新选择未完成的文件。",
        "One or more selected files contain chapter headings but no body text. Add novel text and choose the unfinished files again.",
    )
}

/// 作者原稿重号文案（对齐基线 `AUTHOR_MANUSCRIPT_DUPLICATE_CHAPTER:<n>` 的展示映射）
fn author_manuscript_duplicate_message(locale: ImportRunLocale, chapter: i64) -> String {
    import_text(
        locale,
        format!("作者原稿包含重复的第 {chapter} 章；请修正章节号后重新选择。"),
        format!(
            "The author manuscript contains duplicate Chapter {chapter}. Correct the chapter numbers and choose the files again."
        ),
    )
}

/// 数字感知的自然序比较（D7：逼近 `localeCompare(…, 'zh-CN', {numeric: true})`）。
pub(crate) fn compare_display_names(left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let mut left_chars = left.chars().peekable();
    let mut right_chars = right.chars().peekable();
    loop {
        match (left_chars.peek().copied(), right_chars.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left_char), Some(right_char)) => {
                if left_char.is_ascii_digit() && right_char.is_ascii_digit() {
                    let left_run = take_digit_run(&mut left_chars);
                    let right_run = take_digit_run(&mut right_chars);
                    let order = compare_numeric_runs(&left_run, &right_run);
                    if order != Ordering::Equal {
                        return order;
                    }
                } else {
                    left_chars.next();
                    right_chars.next();
                    let folded_left: String = left_char.to_lowercase().collect();
                    let folded_right: String = right_char.to_lowercase().collect();
                    let order = folded_left.cmp(&folded_right);
                    if order != Ordering::Equal {
                        return order;
                    }
                }
            }
        }
    }
}

fn take_digit_run(iterator: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut run = String::new();
    while let Some(character) = iterator.peek().copied() {
        if character.is_ascii_digit() {
            run.push(character);
            iterator.next();
        } else {
            break;
        }
    }
    run
}

fn compare_numeric_runs(left: &str, right: &str) -> std::cmp::Ordering {
    let trimmed_left = left.trim_start_matches('0');
    let trimmed_right = right.trim_start_matches('0');
    if trimmed_left.len() != trimmed_right.len() {
        return trimmed_left.len().cmp(&trimmed_right.len());
    }
    trimmed_left.cmp(trimmed_right)
}

/// 受限文本读取（D5：直接 `std::fs`，字节上限与基线一致）。
///
/// 超限返回基线的 `SECURE_FS_FILE_TOO_LARGE` 错误码，由文案映射落回
/// 「总大小超过导入上限」。
fn read_text_bounded(path: &Path, limit: usize) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut buffer = Vec::new();
    let mut limited = (&file).take(limit as u64 + 1);
    limited
        .read_to_end(&mut buffer)
        .map_err(|error| error.to_string())?;
    if buffer.len() > limit {
        return Err("SECURE_FS_FILE_TOO_LARGE".to_string());
    }
    // 对齐 Node `readFile(path, 'utf8')` 的宽松解码语义
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

/// 预检通过的一个来源
struct SelectedSource {
    identity: parsing::ImportSourceFileIdentity,
    display_name: String,
}

/// 平移自控制器主体（对话框与项目门禁之外的全部纯逻辑）。
///
/// 失败时**不自行清理**存储：调用方负责 `clear()`（命令层在失败路径统一清理）。
pub(crate) fn inspect_selected_novel_files(
    store: &mut ImportInspectionStore,
    file_paths: Vec<String>,
    purpose: ImportPurpose,
    locale: ImportRunLocale,
) -> Result<ImportInspectionSummary, String> {
    if purpose == ImportPurpose::Reference {
        return Err(reference_import_unmigrated_message(locale));
    }
    if file_paths.is_empty() {
        return Err(import_selection_error_message("", locale));
    }
    if file_paths.len() > MAX_IMPORT_SOURCE_FILES {
        return Err(import_selection_error_message(
            "IMPORT_SOURCE_COUNT_EXCEEDED",
            locale,
        ));
    }

    // 预检：身份 + 单文件大小 + 累计上限（在读取任何字节前完成）
    let mut selected_bytes: usize = 0;
    let mut selected: Vec<SelectedSource> = Vec::new();
    for file_path in &file_paths {
        let identity = default_file_identity(file_path)
            .map_err(|error| import_selection_error_message(&error, locale))?;
        let metadata = std::fs::metadata(&identity.canonical_location)
            .map_err(|error| import_selection_error_message(&error.to_string(), locale))?;
        let raw_size = metadata.len();
        if raw_size > (1u64 << 53) - 1 {
            return Err(import_selection_error_message(
                "IMPORT_SOURCE_SIZE_INVALID",
                locale,
            ));
        }
        let selected_size = raw_size as usize;
        if selected_size > MAX_IMPORT_TOTAL_BYTES - selected_bytes {
            return Err(import_selection_error_message(
                "IMPORT_SOURCE_BYTES_EXCEEDED",
                locale,
            ));
        }
        selected_bytes += selected_size;
        let display_name = Path::new(file_path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| file_path.clone());
        selected.push(SelectedSource {
            identity,
            display_name,
        });
    }
    // D7：数字感知自然序（逼近基线 zh-CN numeric collation）
    selected.sort_by(|left, right| compare_display_names(&left.display_name, &right.display_name));

    let mut consumed_bytes: usize = 0;
    let mut chapter_count: usize = 0;
    let mut sources: Vec<InspectedImportSource> = Vec::new();
    let mut inspected_chapters: Vec<InspectedImportChapter> = Vec::new();
    let mut empty_source_found = false;
    let mut title_only_source_found = false;

    for (source_index, source) in selected.into_iter().enumerate() {
        if parsing::is_epub_name(&source.display_name) {
            return Err(epub_import_unmigrated_message(locale));
        }
        let remaining = MAX_IMPORT_TOTAL_BYTES - consumed_bytes;
        let raw = read_text_bounded(Path::new(&source.identity.canonical_location), remaining)
            .map_err(|code| import_selection_error_message(&code, locale))?;
        let content_bytes = raw.len();
        if content_bytes > MAX_IMPORT_TOTAL_BYTES - consumed_bytes {
            return Err(import_selection_error_message(
                "IMPORT_SOURCE_BYTES_EXCEEDED",
                locale,
            ));
        }
        consumed_bytes += content_bytes;
        let content = raw.trim();
        if content.is_empty() {
            empty_source_found = true;
            continue;
        }

        let parsed = if parsing::has_chapter_headings(content) {
            parsing::split_single_file_content(content, MAX_IMPORT_CHAPTERS - chapter_count)
                .map_err(|code| import_selection_error_message(&code, locale))?
        } else {
            let stem = Path::new(&source.display_name)
                .file_stem()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| source.display_name.clone());
            let extracted = parsing::extract_chapter_number(&stem);
            vec![ParsedChapter {
                number: if extracted != 0 { extracted } else { 1 },
                title: stem,
                content: content.to_string(),
                word_count: count_draft_units(content),
            }]
        };

        let location_digest = location_alias_digest(&source.identity.canonical_location)
            .map_err(|error| import_selection_error_message(&error, locale))?;
        let file_digest = match &source.identity.file_identity {
            Some(file_identity) => Some(
                file_alias_digest(file_identity)
                    .map_err(|error| import_selection_error_message(&error, locale))?,
            ),
            None => None,
        };
        sources.push(InspectedImportSource {
            location_alias_digest: location_digest,
            file_alias_digest: file_digest,
            display_name: source.display_name.clone(),
            media_type: parsing::source_media_type(&source.display_name),
            size: content_bytes,
        });

        if parsed.is_empty() {
            title_only_source_found = true;
            continue;
        }
        if parsed.len() > MAX_IMPORT_CHAPTERS - chapter_count {
            return Err(import_selection_error_message(
                "IMPORT_CHAPTER_COUNT_EXCEEDED",
                locale,
            ));
        }
        chapter_count += parsed.len();

        let mut used_local_numbers: std::collections::HashSet<i64> =
            std::collections::HashSet::new();
        let mut next_local_number: i64 = 1;
        for chapter in parsed {
            let mut source_chapter_number = chapter.number;
            if source_chapter_number < 1 || used_local_numbers.contains(&source_chapter_number) {
                while used_local_numbers.contains(&next_local_number) {
                    next_local_number += 1;
                }
                source_chapter_number = next_local_number;
            }
            used_local_numbers.insert(source_chapter_number);
            let ParsedChapter {
                number,
                title,
                content,
                word_count,
            } = chapter;
            let content_fingerprint = parsing::sha256_hex(&content);
            let content_size = content.len();
            inspected_chapters.push(InspectedImportChapter {
                number,
                source_index,
                source_chapter_number,
                title,
                content,
                word_count,
                content_fingerprint,
                content_size,
            });
        }
    }

    if empty_source_found {
        return Err(empty_source_message(locale));
    }
    if title_only_source_found {
        return Err(title_only_source_message(locale));
    }

    // author-manuscript：章号必须唯一（`reference` 在上方已提前拒绝）
    let mut seen_numbers: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for chapter in &inspected_chapters {
        if chapter.number < 1 || !seen_numbers.insert(chapter.number) {
            return Err(author_manuscript_duplicate_message(locale, chapter.number));
        }
    }

    store
        .create(purpose, sources, inspected_chapters)
        .map_err(|error| import_selection_error_message(&error, locale))
}

/// `dialog:select-novel-files` —— 选择作者原稿文件并生成检视（G1 正式频道）。
#[tauri::command]
pub async fn dialog_select_novel_files(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: Option<Value>,
    project_session: Option<ProjectSessionContext>,
) -> Result<Option<NovelFileSelectionResult>, String> {
    let parsed = match parse_selection_request(&request) {
        Ok(parsed) => parsed,
        Err(code) => {
            clear_inspections(&state);
            return Ok(Some(NovelFileSelectionResult::failure(
                import_selection_error_message(&code, ImportRunLocale::ZhCn),
            )));
        }
    };
    let locale = parsed.locale.unwrap_or(ImportRunLocale::ZhCn);

    // 结构化请求：冻结项目门禁（会话 + expectedProjectPath 同源校验）
    if parsed.structured {
        let expected = parsed.expected_project_path.as_deref().unwrap_or("");
        if let Err(error) = guard_read(state.inner(), expected, project_session.as_ref()) {
            clear_inspections(&state);
            return Err(error);
        }
    }

    let title = match parsed.purpose {
        ImportPurpose::AuthorManuscript => {
            import_text(locale, "选择作者原稿文件", "Choose author manuscript files")
        }
        ImportPurpose::Reference => {
            import_text(locale, "选择参考小说文件", "Choose reference novel files")
        }
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut builder = app
        .dialog()
        .file()
        .set_title(title)
        .add_filter("小说文件", &["txt", "md", "text", "epub"])
        .add_filter("所有文件", &["*"]);
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        builder = builder.set_parent(&window);
    }
    builder.pick_files(move |selection| {
        let _ = sender.send(selection);
    });
    let picked = tauri::async_runtime::spawn_blocking(move || {
        receiver.recv_timeout(DIALOG_TIMEOUT).ok().flatten()
    })
    .await
    .ok()
    .flatten();
    let Some(paths) = picked else {
        return Ok(None);
    };

    // 读取前再次确认项目仍是冻结那一个（基线 `assertFrozenProject()`）
    if parsed.structured {
        let expected = parsed.expected_project_path.as_deref().unwrap_or("");
        if let Err(error) = guard_read(state.inner(), expected, project_session.as_ref()) {
            clear_inspections(&state);
            return Err(error);
        }
    }

    let mut file_paths: Vec<String> = Vec::new();
    for file in paths {
        if let Some(path) = file_path_to_string(file) {
            file_paths.push(path);
        }
    }
    if file_paths.is_empty() {
        return Ok(None);
    }

    let outcome = {
        let mut store = state
            .import_inspections
            .lock()
            .map_err(|_| "导入检查状态被污染".to_string())?;
        // 同会话重选即替换（D2：清空全部待处理检视）
        store.clear();
        inspect_selected_novel_files(&mut store, file_paths, parsed.purpose, locale)
    };
    match outcome {
        Ok(summary) => Ok(Some(NovelFileSelectionResult::ok(summary))),
        Err(message) => {
            clear_inspections(&state);
            Ok(Some(NovelFileSelectionResult::failure(message)))
        }
    }
}

fn clear_inspections(state: &State<'_, AppState>) {
    if let Ok(mut store) = state.import_inspections.lock() {
        store.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_access::random_uuid_v4;
    use crate::state::ActiveProject;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lorekeeper-g1-import-{name}-{}", random_uuid_v4()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(dir: &Path, name: &str, content: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path.to_string_lossy().to_string()
    }

    #[test]
    fn parse_selection_request_defaults_and_rejects_unknown_test() {
        // 缺省 / null / 非对象 → reference
        for request in [None, Some(Value::Null), Some(Value::from(7))] {
            let parsed = parse_selection_request(&request).unwrap();
            assert_eq!(parsed.purpose, ImportPurpose::Reference);
            assert!(!parsed.structured);
            assert!(parsed.locale.is_none());
        }
        // 字符串用途
        let parsed = parse_selection_request(&Some(Value::from("author-manuscript"))).unwrap();
        assert_eq!(parsed.purpose, ImportPurpose::AuthorManuscript);
        assert!(!parsed.structured);
        // 未知字符串用途 → IMPORT_PURPOSE_INVALID
        assert_eq!(
            parse_selection_request(&Some(Value::from("bogus"))).unwrap_err(),
            "IMPORT_PURPOSE_INVALID"
        );
        // 结构化请求
        let request = Some(serde_json::json!({
            "runId": "r-1",
            "purpose": "author-manuscript",
            "locale": "en-US",
            "expectedProjectPath": "F:\\Novel",
        }));
        let parsed = parse_selection_request(&request).unwrap();
        assert_eq!(parsed.purpose, ImportPurpose::AuthorManuscript);
        assert_eq!(parsed.locale, Some(ImportRunLocale::EnUs));
        assert!(parsed.structured);
        assert_eq!(parsed.expected_project_path.as_deref(), Some("F:\\Novel"));
        // 对象缺 purpose → reference（对齐基线可选链）
        let sparse = Some(serde_json::json!({ "expectedProjectPath": "F:\\Novel" }));
        let parsed = parse_selection_request(&sparse).unwrap();
        assert_eq!(parsed.purpose, ImportPurpose::Reference);
    }

    #[test]
    fn import_selection_error_message_maps_baseline_copy_test() {
        assert_eq!(
            import_selection_error_message("IMPORT_SOURCE_COUNT_EXCEEDED", ImportRunLocale::ZhCn),
            "所选文件数量超过导入上限（最多 5000 个）。"
        );
        assert_eq!(
            import_selection_error_message("SECURE_FS_FILE_TOO_LARGE", ImportRunLocale::ZhCn),
            "所选文件总大小超过导入上限（最多 134217728 字节）。"
        );
        assert_eq!(
            import_selection_error_message("IMPORT_SOURCE_BYTES_EXCEEDED", ImportRunLocale::ZhCn),
            "所选文件总大小超过导入上限（最多 134217728 字节）。"
        );
        assert_eq!(
            import_selection_error_message("IMPORT_CHAPTER_COUNT_EXCEEDED", ImportRunLocale::ZhCn),
            "拆分后的章节数超过导入上限（最多 5000 章）。"
        );
        // 未知码 / 空串 → 通用文案
        assert_eq!(
            import_selection_error_message("SOMETHING_ELSE", ImportRunLocale::ZhCn),
            "无法读取所选文件；请重新选择后再试。"
        );
        assert_eq!(
            import_selection_error_message("", ImportRunLocale::EnUs),
            "Could not read the selected files. Please choose them again."
        );
    }

    #[test]
    fn compare_display_names_orders_numerically_test() {
        use std::cmp::Ordering;
        assert_eq!(
            compare_display_names("第2章.txt", "第10章.txt"),
            Ordering::Less
        );
        assert_eq!(
            compare_display_names("第10章.txt", "第2章.txt"),
            Ordering::Greater
        );
        assert_eq!(compare_display_names("A.txt", "a.txt"), Ordering::Equal);
        assert_eq!(
            compare_display_names("book.txt", "book2.txt"),
            Ordering::Less
        );
    }

    #[test]
    fn inspect_author_manuscript_creates_summary_test() {
        let dir = temp_dir("happy");
        let first = write_file(&dir, "第2章.txt", "第二章 发展\n正文乙");
        let second = write_file(&dir, "第1章.txt", "第一章 开端\n正文甲");
        let mut store = ImportInspectionStore::new();
        // 传入顺序颠倒，验证按 displayName 数值排序后 source 顺序一致
        let summary = inspect_selected_novel_files(
            &mut store,
            vec![first, second],
            ImportPurpose::AuthorManuscript,
            ImportRunLocale::ZhCn,
        )
        .unwrap();
        assert_eq!(summary.source_count, 2);
        assert_eq!(
            summary.source_display_names,
            vec!["第1章.txt".to_string(), "第2章.txt".to_string()]
        );
        assert_eq!(summary.chapter_count, 2);
        assert_eq!(summary.total_words, 6);
        assert_eq!(summary.preview.len(), 2);
        assert_eq!(summary.preview[0].title, "开端");
        assert_eq!(summary.preview[1].title, "发展");
        assert_eq!(store.active_count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspect_rejects_reference_and_epub_honestly_test() {
        let dir = temp_dir("honest");
        let text = write_file(&dir, "a.md", "正文");
        let epub = write_file(&dir, "book.epub", "not-a-real-epub");

        let mut store = ImportInspectionStore::new();
        let error = inspect_selected_novel_files(
            &mut store,
            vec![text.clone()],
            ImportPurpose::Reference,
            ImportRunLocale::ZhCn,
        )
        .unwrap_err();
        assert!(error.contains("G2"), "reference 错误应指向 G2：{error}");

        let error = inspect_selected_novel_files(
            &mut store,
            vec![epub],
            ImportPurpose::AuthorManuscript,
            ImportRunLocale::ZhCn,
        )
        .unwrap_err();
        assert!(error.contains("EPUB"), "epub 错误应说明未迁移：{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspect_rejects_empty_and_duplicate_and_over_limit_test() {
        let dir = temp_dir("failures");
        let mut store = ImportInspectionStore::new();

        // 空文件
        let empty = write_file(&dir, "empty.txt", "   \n \n  ");
        let error = inspect_selected_novel_files(
            &mut store,
            vec![empty],
            ImportPurpose::AuthorManuscript,
            ImportRunLocale::ZhCn,
        )
        .unwrap_err();
        assert_eq!(
            error,
            "一个或多个所选文件为空。请补充小说正文后，重新选择未完成的文件。"
        );

        // 重复章号
        let a = write_file(&dir, "a.txt", "第一章 甲\n正文甲");
        let b = write_file(&dir, "b.txt", "第一章 乙\n正文乙");
        let error = inspect_selected_novel_files(
            &mut store,
            vec![a, b],
            ImportPurpose::AuthorManuscript,
            ImportRunLocale::ZhCn,
        )
        .unwrap_err();
        assert_eq!(error, "作者原稿包含重复的第 1 章；请修正章节号后重新选择。");

        // 章节数超限 → 基线文案
        let mut content = String::new();
        for number in 1..=(MAX_IMPORT_CHAPTERS + 1) {
            content.push_str(&format!("第{number}章 标题\n正文\n"));
        }
        let many = write_file(&dir, "many.txt", &content);
        let error = inspect_selected_novel_files(
            &mut store,
            vec![many],
            ImportPurpose::AuthorManuscript,
            ImportRunLocale::ZhCn,
        )
        .unwrap_err();
        assert_eq!(error, "拆分后的章节数超过导入上限（最多 5000 章）。");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn structured_request_guard_enforces_session_and_path_test() {
        let root = temp_dir("guard");
        let root_text = root.to_string_lossy().to_string();
        let state = AppState::new();
        let lease = ActiveProject {
            project_id: random_uuid_v4(),
            lease_id: random_uuid_v4(),
            root_path: root_text.clone(),
        };
        let session = ProjectSessionContext {
            project_id: lease.project_id.clone(),
            lease_id: lease.lease_id.clone(),
            project_path: lease.root_path.clone(),
        };
        state.activate_project(lease).unwrap();

        let matching = parse_selection_request(&Some(serde_json::json!({
            "runId": "r-1",
            "purpose": "author-manuscript",
            "locale": "zh-CN",
            "expectedProjectPath": root_text,
        })))
        .unwrap();
        assert!(matching.structured);
        guard_read(
            &state,
            matching.expected_project_path.as_deref().unwrap_or(""),
            Some(&session),
        )
        .expect("同源会话 + 路径应通过");

        let foreign = parse_selection_request(&Some(serde_json::json!({
            "runId": "r-1",
            "purpose": "author-manuscript",
            "locale": "zh-CN",
            "expectedProjectPath": "C:\\Other",
        })))
        .unwrap();
        assert!(guard_read(
            &state,
            foreign.expected_project_path.as_deref().unwrap_or(""),
            Some(&session),
        )
        .is_err());

        // 缺会话
        assert!(guard_read(
            &state,
            matching.expected_project_path.as_deref().unwrap_or(""),
            None,
        )
        .is_err());
        state.invalidate_current_session();
        let _ = std::fs::remove_dir_all(&root);
    }
}
