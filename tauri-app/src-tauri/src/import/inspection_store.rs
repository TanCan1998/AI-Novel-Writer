//! 导入检视存储（进程内存态）—— 平移自 `electron/services/import-inspection-store.ts`。
//!
//! 语义：一次「选择 → 受限读取 → 解析」在**主进程内**完成，渲染层只拿到
//! [`ImportInspectionSummary`]（检视令牌 + 安全展示事实）；章节正文与来源身份
//! 始终留在后端内存，直到 `db:import-run-*-preview/-prepare` 消费它。
//!
//! # 与基线的对齐 / 偏离
//!
//! - ✅ 全部 `create` 校验逐字平移（章节数 / 来源数 / 摘要格式 / displayName /
//!   mediaType / size / 章号唯一性 / 单章与总量上限 / 同会话重选即替换）；
//! - ✅ `consume` / `peek` / `clear` / `active_count` / TTL 过期清理；
//! - ⚠️ **D2 刻意偏离**：删除 `webContentsId` 归属维度（Tauri 单窗口单逻辑会话），
//!   `revokeForWebContents(id)` 退化为 [`ImportInspectionStore::clear`]（清空全部）。
//!   直接后果：基线中「他人活跃检视 < `maxActive`」与「他人合计 + 本次 ≤
//!   `maxAggregateBytes`」两条校验在当前结构下结构不可达（常量与校验保留，
//!   作为将来多窗口扩展的护栏）。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Serialize;

use crate::import::limits::{
    IMPORT_INSPECTION_TTL_MS, MAX_ACTIVE_INSPECTIONS, MAX_IMPORT_CHAPTERS,
    MAX_IMPORT_CHAPTER_BYTES, MAX_IMPORT_SOURCE_FILES, MAX_IMPORT_TOTAL_BYTES,
};
use crate::import::ImportPurpose;
use crate::project_access::random_uuid_v4;

/// sha256 摘要格式（小写 hex，64 位）
fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// 已检视来源（对齐基线 `InspectedImportSource`；D1 下摘要为无密钥 sha256）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectedImportSource {
    pub location_alias_digest: String,
    pub file_alias_digest: Option<String>,
    pub display_name: String,
    pub media_type: String,
    pub size: usize,
}

/// 已检视章节（对齐基线 `InspectedImportChapter`）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectedImportChapter {
    pub number: i64,
    /// 来源在本次选择中的下标（主进程内存态，永不回传渲染层）
    pub source_index: usize,
    /// 来源内稳定章号（同来源内唯一）
    pub source_chapter_number: i64,
    pub title: String,
    pub content: String,
    pub word_count: i64,
    pub content_fingerprint: String,
    pub content_size: usize,
}

/// 完整检视（含正文），仅在进程内流通
#[derive(Debug, Clone)]
pub struct ImportInspection {
    pub inspection_id: String,
    pub purpose: ImportPurpose,
    pub sources: Vec<InspectedImportSource>,
    pub chapters: Vec<InspectedImportChapter>,
    pub total_words: i64,
    pub total_bytes: usize,
    pub expires_at: u64,
}

/// 对齐契约 `ImportChapterPreview`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportChapterPreview {
    pub number: i64,
    pub title: String,
    pub word_count: i64,
}

/// 对齐契约 `ImportInspectionSummary`（渲染层可见的安全子集）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportInspectionSummary {
    pub inspection_id: String,
    pub purpose: ImportPurpose,
    pub source_count: usize,
    pub source_display_names: Vec<String>,
    pub chapter_count: usize,
    pub total_words: i64,
    pub total_bytes: usize,
    /// 前 8 章预览（对齐基线 `chapters.slice(0, 8)`）
    pub preview: Vec<ImportChapterPreview>,
}

/// 时钟注入（毫秒时间戳），便于 TTL 过期测试。
type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// 系统当前毫秒（对齐 JS `Date.now()`）
fn system_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 存储参数（对齐基线 `ImportInspectionStoreOptions`，除 `now` 走构造器注入）
#[derive(Debug, Clone)]
pub struct ImportInspectionStoreOptions {
    pub ttl_ms: u64,
    pub max_active: usize,
    pub max_aggregate_bytes: usize,
}

impl Default for ImportInspectionStoreOptions {
    fn default() -> Self {
        Self {
            ttl_ms: IMPORT_INSPECTION_TTL_MS,
            max_active: MAX_ACTIVE_INSPECTIONS,
            max_aggregate_bytes: MAX_IMPORT_TOTAL_BYTES,
        }
    }
}

/// 待处理导入检视存储（常驻 `AppState`，进程重启即失效）
pub struct ImportInspectionStore {
    inspections: HashMap<String, ImportInspection>,
    now: Clock,
    options: ImportInspectionStoreOptions,
}

impl std::fmt::Debug for ImportInspectionStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ImportInspectionStore")
            .field("active", &self.inspections.len())
            .field("options", &self.options)
            .finish()
    }
}

impl Default for ImportInspectionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportInspectionStore {
    pub fn new() -> Self {
        Self::with_clock(
            Arc::new(system_now_ms),
            ImportInspectionStoreOptions::default(),
        )
    }

    /// 测试用构造器（注入时钟 / 上限）
    pub fn with_clock(now: Clock, options: ImportInspectionStoreOptions) -> Self {
        Self {
            inspections: HashMap::new(),
            now,
            options,
        }
    }

    fn now_ms(&self) -> u64 {
        (self.now)()
    }

    fn remove_expired(&mut self) {
        let now = self.now_ms();
        self.inspections
            .retain(|_, inspection| inspection.expires_at > now);
    }

    /// 平移自 `create`：全部校验 + 同会话重选替换 + 摘要返回。
    pub fn create(
        &mut self,
        purpose: ImportPurpose,
        sources: Vec<InspectedImportSource>,
        chapters: Vec<InspectedImportChapter>,
    ) -> Result<ImportInspectionSummary, String> {
        self.remove_expired();
        if chapters.is_empty() || chapters.len() > MAX_IMPORT_CHAPTERS {
            return Err(format!("导入章节数必须在 1–{MAX_IMPORT_CHAPTERS} 之间"));
        }
        if sources.is_empty() || sources.len() > MAX_IMPORT_SOURCE_FILES {
            return Err("导入来源数量无效".to_string());
        }
        for source in &sources {
            let location_ok = is_sha256(&source.location_alias_digest);
            let file_ok = source.file_alias_digest.as_deref().map_or(true, is_sha256);
            if !location_ok
                || !file_ok
                || source.display_name.is_empty()
                || source.display_name.chars().count() > 255
                || source.display_name.contains('/')
                || source.display_name.contains('\\')
                || source.display_name.contains('\0')
                || source.media_type.is_empty()
                || source.media_type.chars().count() > 100
                || source.size > MAX_IMPORT_TOTAL_BYTES
            {
                return Err("导入来源检查数据无效".to_string());
            }
        }

        let mut chapter_numbers: HashSet<i64> = HashSet::new();
        let mut source_chapters: HashSet<(usize, i64)> = HashSet::new();
        let mut total_bytes: usize = 0;
        for chapter in &chapters {
            let bytes = chapter.content.len();
            if chapter.number < 1
                || chapter_numbers.contains(&chapter.number)
                || chapter.source_index >= sources.len()
                || chapter.source_chapter_number < 1
                || source_chapters.contains(&(chapter.source_index, chapter.source_chapter_number))
                || !is_sha256(&chapter.content_fingerprint)
                || chapter.content_size != bytes
                || bytes > MAX_IMPORT_CHAPTER_BYTES
            {
                return Err("导入章节检查数据无效".to_string());
            }
            chapter_numbers.insert(chapter.number);
            source_chapters.insert((chapter.source_index, chapter.source_chapter_number));
            total_bytes += bytes;
            if total_bytes > MAX_IMPORT_TOTAL_BYTES {
                return Err("导入正文总字节数超过安全上限".to_string());
            }
        }

        // D2：单窗口只有一个逻辑会话 → 「他人待处理检视」恒为空集合。
        // 两条校验因此结构不可达，保留以维持基线的失败口径（见模块文档）。
        let retained_count = 0usize;
        let retained_bytes = 0usize;
        if retained_bytes + total_bytes > self.options.max_aggregate_bytes {
            return Err("导入正文总字节数超过安全上限".to_string());
        }
        if retained_count >= self.options.max_active {
            return Err("待处理导入检查过多，请先完成或取消现有检查".to_string());
        }

        // 同一会话重选即替换（D2：退化为清空全部待处理检视）
        self.clear();

        let inspection_id = random_uuid_v4();
        let total_words: i64 = chapters.iter().map(|chapter| chapter.word_count).sum();
        let source_display_names = sources
            .iter()
            .map(|source| source.display_name.clone())
            .collect();
        let summary = ImportInspectionSummary {
            inspection_id: inspection_id.clone(),
            purpose,
            source_count: sources.len(),
            source_display_names,
            chapter_count: chapters.len(),
            total_words,
            total_bytes,
            preview: chapters
                .iter()
                .take(8)
                .map(|chapter| ImportChapterPreview {
                    number: chapter.number,
                    title: chapter.title.clone(),
                    word_count: chapter.word_count,
                })
                .collect(),
        };
        let expires_at = self.now_ms() + self.options.ttl_ms;
        self.inspections.insert(
            inspection_id.clone(),
            ImportInspection {
                inspection_id,
                purpose,
                sources,
                chapters,
                total_words,
                total_bytes,
                expires_at,
            },
        );
        Ok(summary)
    }

    /// 平移自 `consume`（消费后立即失效）
    pub fn consume(&mut self, inspection_id: &str) -> Result<ImportInspection, String> {
        self.remove_expired();
        self.inspections
            .remove(inspection_id)
            .ok_or_else(|| "导入检查已失效，请重新选择文件".to_string())
    }

    /// 平移自 `peek`（只读；`purpose` 不匹配视为失效）
    pub fn peek(
        &mut self,
        inspection_id: &str,
        purpose: Option<ImportPurpose>,
    ) -> Result<&ImportInspection, String> {
        self.remove_expired();
        match self.inspections.get(inspection_id) {
            Some(inspection) if purpose.map_or(true, |expected| inspection.purpose == expected) => {
                Ok(inspection)
            }
            _ => Err("导入检查已失效，请重新选择文件".to_string()),
        }
    }

    /// 平移自 `revokeForWebContents`（D2：退化为「清空全部待处理检视」）
    pub fn clear(&mut self) {
        self.inspections.clear();
    }

    /// 平移自 `activeCount`（先做 TTL 清理）
    pub fn active_count(&mut self) -> usize {
        self.remove_expired();
        self.inspections.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn source(display_name: &str) -> InspectedImportSource {
        InspectedImportSource {
            location_alias_digest: HEX.to_string(),
            file_alias_digest: Some(HEX.to_string()),
            display_name: display_name.to_string(),
            media_type: "text/plain".to_string(),
            size: 10,
        }
    }

    fn chapter(number: i64, content: &str) -> InspectedImportChapter {
        InspectedImportChapter {
            number,
            source_index: 0,
            source_chapter_number: number,
            title: format!("第 {number} 章"),
            content: content.to_string(),
            word_count: crate::draft_units::count_draft_units(content),
            content_fingerprint: crate::import::parsing::sha256_hex(content),
            content_size: content.len(),
        }
    }

    fn create(
        store: &mut ImportInspectionStore,
        source_count: usize,
        chapters: Vec<InspectedImportChapter>,
    ) -> Result<ImportInspectionSummary, String> {
        let sources = (0..source_count)
            .map(|index| source(&format!("book{index}.txt")))
            .collect();
        store.create(ImportPurpose::AuthorManuscript, sources, chapters)
    }

    #[test]
    fn create_returns_summary_with_first_eight_preview_test() {
        let mut store = ImportInspectionStore::new();
        let chapters = (1..=10).map(|number| chapter(number, "正文")).collect();
        let summary = create(&mut store, 1, chapters).unwrap();
        assert_eq!(summary.purpose, ImportPurpose::AuthorManuscript);
        assert_eq!(summary.source_count, 1);
        assert_eq!(summary.source_display_names, vec!["book0.txt".to_string()]);
        assert_eq!(summary.chapter_count, 10);
        assert_eq!(summary.total_words, 20);
        assert_eq!(summary.preview.len(), 8);
        assert_eq!(summary.preview[0].number, 1);
        assert_eq!(store.active_count(), 1);
    }

    #[test]
    fn create_rejects_chapter_count_bounds_test() {
        let mut store = ImportInspectionStore::new();
        let error = create(&mut store, 1, vec![]).unwrap_err();
        assert_eq!(error, "导入章节数必须在 1–5000 之间");

        let too_many = (1..=(MAX_IMPORT_CHAPTERS as i64 + 1))
            .map(|number| chapter(number, "正文"))
            .collect();
        let error = create(&mut store, 1, too_many).unwrap_err();
        assert_eq!(error, "导入章节数必须在 1–5000 之间");
        assert_eq!(store.active_count(), 0);
    }

    #[test]
    fn create_rejects_source_count_bounds_test() {
        let mut store = ImportInspectionStore::new();
        let error = create(&mut store, 0, vec![chapter(1, "正文")]).unwrap_err();
        assert_eq!(error, "导入来源数量无效");

        let error = create(
            &mut store,
            MAX_IMPORT_SOURCE_FILES + 1,
            vec![chapter(1, "正文")],
        )
        .unwrap_err();
        assert_eq!(error, "导入来源数量无效");
    }

    #[test]
    fn create_rejects_invalid_source_metadata_test() {
        let mut store = ImportInspectionStore::new();
        let mut bad_name = source("a/b.txt");
        bad_name.location_alias_digest = HEX.to_string();
        let error = store
            .create(
                ImportPurpose::AuthorManuscript,
                vec![bad_name],
                vec![chapter(1, "正文")],
            )
            .unwrap_err();
        assert_eq!(error, "导入来源检查数据无效");

        let mut bad_digest = source("a.txt");
        bad_digest.location_alias_digest = "not-a-digest".to_string();
        let error = store
            .create(
                ImportPurpose::AuthorManuscript,
                vec![bad_digest],
                vec![chapter(1, "正文")],
            )
            .unwrap_err();
        assert_eq!(error, "导入来源检查数据无效");
    }

    #[test]
    fn create_rejects_invalid_chapter_metadata_test() {
        let mut store = ImportInspectionStore::new();

        let mut bad_fingerprint = chapter(1, "正文");
        bad_fingerprint.content_fingerprint = "zzz".to_string();
        let error = create(&mut store, 1, vec![bad_fingerprint]).unwrap_err();
        assert_eq!(error, "导入章节检查数据无效");

        // content_size 与实际字节数不符
        let mut bad_size = chapter(1, "正文");
        bad_size.content_size = 999;
        let error = create(&mut store, 1, vec![bad_size]).unwrap_err();
        assert_eq!(error, "导入章节检查数据无效");

        // 章号重复
        let error = create(&mut store, 1, vec![chapter(1, "甲"), chapter(1, "乙")]).unwrap_err();
        assert_eq!(error, "导入章节检查数据无效");

        // 单章超过 16 MiB
        let huge = "a".repeat(MAX_IMPORT_CHAPTER_BYTES + 1);
        let error = create(&mut store, 1, vec![chapter(1, &huge)]).unwrap_err();
        assert_eq!(error, "导入章节检查数据无效");
    }

    #[test]
    fn create_rejects_total_bytes_over_limit_test() {
        let mut store = ImportInspectionStore::new();
        let chunk = "a".repeat(MAX_IMPORT_CHAPTER_BYTES);
        let mut chapters: Vec<InspectedImportChapter> =
            (1..=8).map(|number| chapter(number, &chunk)).collect();
        chapters.push(chapter(9, "b"));
        let error = create(&mut store, 1, chapters).unwrap_err();
        assert_eq!(error, "导入正文总字节数超过安全上限");
    }

    #[test]
    fn create_replaces_previous_inspection_test() {
        let mut store = ImportInspectionStore::new();
        let first = create(&mut store, 1, vec![chapter(1, "甲")]).unwrap();
        let second = create(&mut store, 1, vec![chapter(2, "乙")]).unwrap();
        assert_eq!(store.active_count(), 1);
        assert!(store.peek(&first.inspection_id, None).is_err());
        assert!(store.peek(&second.inspection_id, None).is_ok());
    }

    #[test]
    fn peek_enforces_purpose_test() {
        let mut store = ImportInspectionStore::new();
        let summary = create(&mut store, 1, vec![chapter(1, "正文")]).unwrap();
        assert!(store
            .peek(
                &summary.inspection_id,
                Some(ImportPurpose::AuthorManuscript)
            )
            .is_ok());
        assert!(store
            .peek(&summary.inspection_id, Some(ImportPurpose::Reference))
            .is_err());
    }

    #[test]
    fn consume_invalidates_inspection_test() {
        let mut store = ImportInspectionStore::new();
        let summary = create(&mut store, 1, vec![chapter(1, "正文")]).unwrap();
        let consumed = store.consume(&summary.inspection_id).unwrap();
        assert_eq!(consumed.chapters.len(), 1);
        assert_eq!(store.active_count(), 0);
        assert_eq!(
            store.peek(&summary.inspection_id, None).unwrap_err(),
            "导入检查已失效，请重新选择文件"
        );
        assert_eq!(
            store.consume(&summary.inspection_id).unwrap_err(),
            "导入检查已失效，请重新选择文件"
        );
    }

    #[test]
    fn ttl_expiry_removes_inspection_test() {
        use std::sync::atomic::{AtomicU64, Ordering};

        let now = Arc::new(AtomicU64::new(1_000));
        let clock_cell = now.clone();
        let store_clock: Clock = Arc::new(move || clock_cell.load(Ordering::SeqCst));
        let mut store = ImportInspectionStore::with_clock(
            store_clock,
            ImportInspectionStoreOptions {
                ttl_ms: 100,
                ..ImportInspectionStoreOptions::default()
            },
        );
        let summary = create(&mut store, 1, vec![chapter(1, "正文")]).unwrap();
        assert_eq!(store.active_count(), 1);

        // expires_at = 1000 + 100 = 1100；`expires_at <= now` 即过期
        now.store(1_100, Ordering::SeqCst);
        assert_eq!(store.active_count(), 0);
        assert_eq!(
            store.peek(&summary.inspection_id, None).unwrap_err(),
            "导入检查已失效，请重新选择文件"
        );
    }
}
