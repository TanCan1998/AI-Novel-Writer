//! PlotTree —— 剧情树快照的**结构校验**（`src/shared/plot-tree.ts` 的 Rust 单源）
//!
//! 剧情树是**派生投影**：来源（项目梗概 + 章节蓝图 + 已定稿章节 + 叙事线索）是作者事实，
//! 快照只是模型对它们的编排视图。因此这里做两件事：
//! 1. **持久化校验**（`assert_stored_plot_tree_snapshot`）—— 只认结构自洽，
//!    不要求历史来源仍存在（章节可能已被删除，但快照不得因此变成「非法」）；
//! 2. **生成校验**（`assert_plot_tree_snapshot`）—— 在 1 之上要求
//!    每条来源引用都真实存在、且章节范围不越出当前来源域。
//!
//! 校验顺序、错误文案与基线**逐字对齐**（`剧情树…无效` 系列），
//! 以便前端提示与 Electron 版一致。**唯一有意近似**是 `generatedAt` 的时间可解析性：
//! 基线用 JS `Date.parse`，这里用 ISO-8601 词法校验（见 `is_parseable_datetime`）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 快照章节号安全上限（与项目当前来源范围无关的不可变护栏）
pub const MAX_PLOT_TREE_CHAPTER_NUMBER: i64 = 4_294_967_295;

/// JS `Number.isSafeInteger` 上界（2^53 - 1）
const JS_MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// 轨道角色白名单
const TRACK_ROLES: [&str; 2] = ["main", "subplot"];
/// 事件状态白名单
const EVENT_STATUSES: [&str; 2] = ["planned", "occurred"];
/// 事件类型白名单（叙事线索确认事件）
const THREAD_EVENT_TYPES: [&str; 4] = ["planted", "progressing", "resolved", "abandoned"];

/// 来源引用（对齐 `PlotTreeSourceReference`）
///
/// 序列化用内部标签 `type`（kebab-case），字段 camelCase ——
/// 与基线对象字面量 `{ type, chapterNumber }` / `{ type, draftId, chapterNumber }` /
/// `{ type, planId, eventId?, chapterNumber? }` 的形状与键顺序一致。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PlotTreeSourceReference {
    #[serde(rename_all = "camelCase")]
    Blueprint { chapter_number: i64 },
    #[serde(rename_all = "camelCase")]
    FinalizedChapter { draft_id: i64, chapter_number: i64 },
    #[serde(rename_all = "camelCase")]
    NarrativeThread {
        plan_id: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        event_id: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        chapter_number: Option<i64>,
    },
}

/// 剧情事件（对齐 `PlotTreeEvent`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotTreeEvent {
    pub status: String,
    pub chapter_number: i64,
    pub summary: String,
    pub sources: Vec<PlotTreeSourceReference>,
}

/// 剧情轨道（对齐 `PlotTreeTrack`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotTreeTrack {
    pub id: String,
    pub title: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_track_id: Option<String>,
    pub start_chapter: i64,
    pub end_chapter: i64,
    pub summary: String,
    pub events: Vec<PlotTreeEvent>,
}

/// 剧情树快照（对齐 `PlotTreeSnapshot`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotTreeSnapshot {
    pub version: i64,
    pub generated_at: String,
    pub writing_language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_revision: Option<String>,
    pub tracks: Vec<PlotTreeTrack>,
}

/// 章节蓝图来源事实（对齐 `PlotTreeSourceBundle['blueprints'][number]`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotTreeBlueprintFact {
    pub chapter_number: i64,
    pub title: String,
    pub purpose: String,
    pub key_events: String,
}

/// 已定稿章节来源事实（对齐 `PlotTreeSourceBundle['finalizedChapters'][number]`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotTreeFinalizedChapterFact {
    pub draft_id: i64,
    pub chapter_number: i64,
    pub title: String,
    pub summary: String,
}

/// 叙事线索事件事实（对齐 `PlotTreeNarrativeThreadSource['events'][number]`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotTreeNarrativeThreadEventFact {
    pub id: i64,
    pub chapter_number: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub evidence: String,
    pub reason: String,
}

/// 叙事线索来源事实（对齐 `PlotTreeNarrativeThreadSource`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotTreeNarrativeThreadFact {
    pub id: i64,
    pub title: String,
    #[serde(rename = "type")]
    pub thread_type: String,
    pub target_start_chapter: i64,
    pub target_end_chapter: i64,
    pub author_intent: String,
    pub status: String,
    pub events: Vec<PlotTreeNarrativeThreadEventFact>,
}

/// 项目梗概（对齐 `synopsis: { content }`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotTreeSynopsisFact {
    pub content: String,
}

/// 剧情树来源事实集（对齐 `Omit<PlotTreeSourceBundle, 'sourceRevision' | 'snapshot'>`）
///
/// **字段顺序即 `JSON.stringify` 的键顺序**，`sourceRevision` 的 SHA-256 由本结构
/// 序列化后的字节算出，故顺序不可随意调整（见 `plot_tree_repository` 的黄金测试）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotTreeSourceFacts {
    pub writing_language: String,
    pub synopsis: PlotTreeSynopsisFact,
    pub blueprints: Vec<PlotTreeBlueprintFact>,
    pub finalized_chapters: Vec<PlotTreeFinalizedChapterFact>,
    pub narrative_threads: Vec<PlotTreeNarrativeThreadFact>,
}

/// `isPlotTreeSourceRevision` —— 64 位小写十六进制
pub fn is_plot_tree_source_revision(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// 对齐基线 `record(value)`：仅普通对象算记录（数组与 `null` 不算）
fn as_record(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value.as_object()
}

/// 对齐基线 `text(value, label)`：非空字符串（`trim` 后）才有效，返回 `trim` 结果
fn required_text(value: Option<&Value>, label: &str) -> Result<String, String> {
    let raw = value.and_then(|value| value.as_str());
    match raw {
        Some(text) if !text.trim().is_empty() => Ok(text.trim().to_string()),
        _ => Err(format!("剧情树{label}无效")),
    }
}

/// 对齐基线 `positiveInteger(value, label)`：JS 安全整数且 ≥ 1
fn positive_integer(value: Option<&Value>, label: &str) -> Result<i64, String> {
    match value.and_then(|value| value.as_i64()) {
        Some(number) if (1..=JS_MAX_SAFE_INTEGER).contains(&number) => Ok(number),
        _ => Err(format!("剧情树{label}无效")),
    }
}

/// 对齐基线 `validChapterNumber(value)`
fn valid_chapter_number(value: Option<&Value>) -> bool {
    matches!(
        value.and_then(|value| value.as_i64()),
        Some(number) if (1..=MAX_PLOT_TREE_CHAPTER_NUMBER).contains(&number)
    )
}

/// 对齐基线 `chapterInteger(value, label)`（内部复用 `valid_chapter_number`）
fn chapter_integer(value: Option<&Value>, label: &str) -> Result<i64, String> {
    if valid_chapter_number(value) {
        // `valid_chapter_number` 已确保可取 `i64`
        Ok(value.and_then(|value| value.as_i64()).unwrap_or_default())
    } else {
        Err(format!("剧情树{label}无效"))
    }
}

/// 对齐基线 `nonEmptyString(value)`
fn non_empty_string(value: &str) -> bool {
    !value.trim().is_empty()
}

/// 对齐基线 `validChapterRange(start, end)`
fn valid_chapter_range(start: i64, end: i64) -> bool {
    (1..=MAX_PLOT_TREE_CHAPTER_NUMBER).contains(&start)
        && (1..=MAX_PLOT_TREE_CHAPTER_NUMBER).contains(&end)
        && end >= start
}

/// 来源域章节边界（对齐基线 `plotTreeSourceChapterBounds`）
///
/// 只统计**有内容**的来源：空标题/空摘要的蓝图或定稿章节不参与边界计算，
/// 避免把「占位行」当成有效来源从而放宽模型可编排的范围。
fn plot_tree_source_chapter_bounds(sources: &PlotTreeSourceFacts) -> Option<(i64, i64)> {
    let mut bounds: Option<(i64, i64)> = None;
    let mut extend = |chapter: i64| {
        bounds = Some(match bounds {
            None => (chapter, chapter),
            Some((start, end)) => (start.min(chapter), end.max(chapter)),
        });
    };

    for blueprint in &sources.blueprints {
        let has_content = [&blueprint.title, &blueprint.purpose, &blueprint.key_events]
            .iter()
            .any(|text| non_empty_string(text));
        if (1..=MAX_PLOT_TREE_CHAPTER_NUMBER).contains(&blueprint.chapter_number) && has_content {
            extend(blueprint.chapter_number);
        }
    }
    for chapter in &sources.finalized_chapters {
        let has_content = [&chapter.title, &chapter.summary]
            .iter()
            .any(|text| non_empty_string(text));
        if (1..=JS_MAX_SAFE_INTEGER).contains(&chapter.draft_id)
            && (1..=MAX_PLOT_TREE_CHAPTER_NUMBER).contains(&chapter.chapter_number)
            && has_content
        {
            extend(chapter.chapter_number);
        }
    }
    for thread in &sources.narrative_threads {
        let has_identity = [&thread.title, &thread.thread_type, &thread.author_intent]
            .iter()
            .all(|text| non_empty_string(text));
        if (1..=JS_MAX_SAFE_INTEGER).contains(&thread.id)
            && valid_chapter_range(thread.target_start_chapter, thread.target_end_chapter)
            && has_identity
        {
            extend(thread.target_start_chapter);
            extend(thread.target_end_chapter);
            for event in &thread.events {
                let has_evidence = [&event.evidence, &event.reason]
                    .iter()
                    .all(|text| non_empty_string(text));
                if (1..=JS_MAX_SAFE_INTEGER).contains(&event.id)
                    && (1..=MAX_PLOT_TREE_CHAPTER_NUMBER).contains(&event.chapter_number)
                    && THREAD_EVENT_TYPES.contains(&event.event_type.as_str())
                    && has_evidence
                {
                    extend(event.chapter_number);
                }
            }
        }
    }
    bounds
}

/// 对齐基线 `hasUsablePlotTreeEventSource`：至少有一条来源可合法支撑事件
///
/// 基线中该谓词被**渲染层**用于判定「能否生成剧情树」（`plot-tree-generator.ts`）；
/// Rust 侧的保存路径已由 `assert_plot_tree_snapshot_chapter_bounds` 覆盖同一判定，
/// 故此处暂无 crate 内调用方 —— 保留为对外谓词的 Rust 对应物，避免两栈语义分叉。
#[allow(dead_code)]
pub fn has_usable_plot_tree_event_source(sources: &PlotTreeSourceFacts) -> bool {
    plot_tree_source_chapter_bounds(sources).is_some()
}

/// 对齐基线 `assertPlotTreeSnapshotChapterBounds`：轨道范围不得越出当前来源域
pub fn assert_plot_tree_snapshot_chapter_bounds(
    snapshot: &PlotTreeSnapshot,
    sources: &PlotTreeSourceFacts,
) -> Result<(), String> {
    let Some((start_chapter, end_chapter)) = plot_tree_source_chapter_bounds(sources) else {
        return Err("剧情树缺少有效事件来源".to_string());
    };
    if snapshot
        .tracks
        .iter()
        .any(|track| track.start_chapter < start_chapter || track.end_chapter > end_chapter)
    {
        return Err("剧情树轨道章节范围超出当前剧情来源".to_string());
    }
    Ok(())
}

/// 对齐基线 `sourceReference(value)` —— 解析一条来源引用
fn parse_source_reference(value: &Value) -> Result<PlotTreeSourceReference, String> {
    let Some(record) = as_record(value) else {
        return Err("剧情树来源引用无效".to_string());
    };
    match record.get("type").and_then(|value| value.as_str()) {
        Some("blueprint") => Ok(PlotTreeSourceReference::Blueprint {
            chapter_number: chapter_integer(record.get("chapterNumber"), "来源章节")?,
        }),
        Some("finalized-chapter") => Ok(PlotTreeSourceReference::FinalizedChapter {
            draft_id: positive_integer(record.get("draftId"), "来源定稿")?,
            chapter_number: chapter_integer(record.get("chapterNumber"), "来源章节")?,
        }),
        Some("narrative-thread") => {
            let plan_id = positive_integer(record.get("planId"), "来源叙事计划")?;
            let event_id = match record.get("eventId") {
                None | Some(Value::Null) => None,
                value => Some(positive_integer(value, "来源叙事事件")?),
            };
            let chapter_number = match record.get("chapterNumber") {
                None | Some(Value::Null) => None,
                value => Some(chapter_integer(value, "来源章节")?),
            };
            if event_id.is_some() != chapter_number.is_some() {
                return Err("剧情树叙事来源引用无效".to_string());
            }
            Ok(PlotTreeSourceReference::NarrativeThread {
                plan_id,
                event_id,
                chapter_number,
            })
        }
        _ => Err("剧情树来源引用无效".to_string()),
    }
}

/// 对齐基线 `sourceSupportsEvent` —— 事件状态是否被该来源支撑
///
/// `sources` 为 `None` 时对应基线「持久化校验」路径：叙事线索的**计划区间**约束
/// 无法校验（历史来源可能已不存在），此时只要求状态为 `planned`。
fn source_supports_event(
    source: &PlotTreeSourceReference,
    status: &str,
    chapter_number: i64,
    sources: Option<&PlotTreeSourceFacts>,
) -> bool {
    match source {
        PlotTreeSourceReference::Blueprint {
            chapter_number: source_chapter,
        } => status == "planned" && *source_chapter == chapter_number,
        PlotTreeSourceReference::FinalizedChapter {
            chapter_number: source_chapter,
            ..
        } => status == "occurred" && *source_chapter == chapter_number,
        PlotTreeSourceReference::NarrativeThread {
            plan_id,
            event_id,
            chapter_number: source_chapter,
            ..
        } => {
            if event_id.is_some() {
                return status == "occurred" && *source_chapter == Some(chapter_number);
            }
            if status != "planned" {
                return false;
            }
            let Some(sources) = sources else {
                return true;
            };
            sources
                .narrative_threads
                .iter()
                .find(|thread| thread.id == *plan_id)
                .map(|thread| {
                    chapter_number >= thread.target_start_chapter
                        && chapter_number <= thread.target_end_chapter
                })
                .unwrap_or(false)
        }
    }
}

/// 对齐基线 `assertSourceExists` —— 生成校验时来源必须真实存在
fn assert_source_exists(
    source: &PlotTreeSourceReference,
    sources: &PlotTreeSourceFacts,
) -> Result<(), String> {
    match source {
        PlotTreeSourceReference::Blueprint { chapter_number } => {
            if !sources
                .blueprints
                .iter()
                .any(|blueprint| blueprint.chapter_number == *chapter_number)
            {
                return Err("剧情树来源引用不存在".to_string());
            }
        }
        PlotTreeSourceReference::FinalizedChapter {
            draft_id,
            chapter_number,
        } => {
            if !sources.finalized_chapters.iter().any(|chapter| {
                chapter.draft_id == *draft_id && chapter.chapter_number == *chapter_number
            }) {
                return Err("剧情树来源引用不存在".to_string());
            }
        }
        PlotTreeSourceReference::NarrativeThread {
            plan_id,
            event_id,
            chapter_number,
            ..
        } => {
            let Some(plan) = sources
                .narrative_threads
                .iter()
                .find(|thread| thread.id == *plan_id)
            else {
                return Err("剧情树来源引用不存在".to_string());
            };
            if let Some(event_id) = event_id {
                let found = plan.events.iter().any(|event| {
                    event.id == *event_id && Some(event.chapter_number) == *chapter_number
                });
                if !found {
                    return Err("剧情树来源引用不存在".to_string());
                }
            }
        }
    }
    Ok(())
}

/// `Date.parse` 的**词法近似**：接受 ISO-8601 日期（含仅日期形式）与常见时间部分。
///
/// 与 JS 的差异（均已在模块文档注明）：不做时区换算、不接受 RFC 2822 等历史格式、
/// 允许 `2024-02-31` 这类「月份内越界但会被 JS 归一化」的日期（与 `Date.parse` 一致）。
fn is_parseable_datetime(value: &str) -> bool {
    /// 在**给定切片**内按相对区间取纯数字（越界或非字符边界均返回 `None`，不 panic）
    fn digits_in(text: &str, range: std::ops::Range<usize>) -> Option<i64> {
        let slice = text.get(range)?;
        if slice.is_empty() || !slice.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        slice.parse::<i64>().ok()
    }

    let bytes = value.as_bytes();
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let (Some(_year), Some(month), Some(day)) = (
        digits_in(value, 0..4),
        digits_in(value, 5..7),
        digits_in(value, 8..10),
    ) else {
        return false;
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return false;
    }
    // 仅日期形式（`YYYY-MM-DD`）已被 JS `Date.parse` 接受
    if bytes.len() == 10 {
        return true;
    }

    let rest = &value[10..];
    match rest.chars().next() {
        Some('T') | Some('t') | Some(' ') => {}
        _ => return false,
    }
    let time = &rest[1..];
    let time_bytes = time.as_bytes();
    if time_bytes.len() < 5 || time_bytes[2] != b':' {
        return false;
    }
    if digits_in(time, 0..2).is_none() || digits_in(time, 3..5).is_none() {
        return false;
    }

    let mut index = 5usize;
    if time_bytes.get(index) == Some(&b':') {
        if digits_in(time, index + 1..index + 3).is_none() {
            return false;
        }
        index += 3;
        if time_bytes.get(index) == Some(&b'.') {
            let start = index + 1;
            let mut end = start;
            while end < time_bytes.len() && time_bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end == start {
                return false;
            }
            index = end;
        }
    }

    let timezone = &time[index..];
    if timezone.is_empty() || timezone == "Z" || timezone == "z" {
        // 无时区：JS `Date.parse` 按本地时区解释 ISO 日期时间，仍属合法
        return true;
    }
    let timezone_bytes = timezone.as_bytes();
    if timezone_bytes.len() != 6
        || (timezone_bytes[0] != b'+' && timezone_bytes[0] != b'-')
        || timezone_bytes[3] != b':'
    {
        return false;
    }
    digits_in(timezone, 1..3).is_some() && digits_in(timezone, 4..6).is_some()
}

/// 对齐基线 `assertStoredPlotTreeSnapshot` —— 只校验持久化结构自洽
pub fn assert_stored_plot_tree_snapshot(value: &Value) -> Result<PlotTreeSnapshot, String> {
    let Some(input) = as_record(value) else {
        return Err("剧情树快照版本或写作语言无效".to_string());
    };
    let version = input.get("version").and_then(|value| value.as_i64());
    let writing_language = input
        .get("writingLanguage")
        .and_then(|value| value.as_str())
        .filter(|language| {
            crate::repositories::project_core_repository::WRITING_LANGUAGES.contains(language)
        })
        .map(|language| language.to_string());
    let (Some(1), Some(writing_language)) = (version, writing_language) else {
        return Err("剧情树快照版本或写作语言无效".to_string());
    };

    let generated_at = required_text(input.get("generatedAt"), "生成时间")?;
    if !is_parseable_datetime(&generated_at) {
        return Err("剧情树生成时间无效".to_string());
    }

    let source_revision = match input.get("sourceRevision") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if is_plot_tree_source_revision(text) => Some(text.clone()),
        Some(_) => return Err("剧情树来源版本无效".to_string()),
    };

    let Some(Value::Array(track_values)) = input.get("tracks") else {
        return Err("剧情树轨道无效".to_string());
    };
    if track_values.is_empty() {
        return Err("剧情树轨道无效".to_string());
    }

    let mut tracks = Vec::with_capacity(track_values.len());
    for candidate in track_values {
        let Some(track) = as_record(candidate) else {
            return Err("剧情树轨道无效".to_string());
        };
        let role = track
            .get("role")
            .and_then(|value| value.as_str())
            .filter(|role| TRACK_ROLES.contains(role))
            .map(|role| role.to_string());
        let Some(role) = role else {
            return Err("剧情树轨道无效".to_string());
        };
        let id = required_text(track.get("id"), "轨道 ID")?;
        let title = required_text(track.get("title"), "轨道标题")?;
        let summary = required_text(track.get("summary"), "轨道摘要")?;
        let start_chapter = chapter_integer(track.get("startChapter"), "轨道章节范围")?;
        let end_chapter = chapter_integer(track.get("endChapter"), "轨道章节范围")?;
        if end_chapter < start_chapter {
            return Err("剧情树轨道章节范围无效".to_string());
        }

        let Some(Value::Array(event_values)) = track.get("events") else {
            return Err("剧情树轨道事件无效".to_string());
        };
        if event_values.is_empty() {
            return Err("剧情树轨道事件无效".to_string());
        }
        let mut events = Vec::with_capacity(event_values.len());
        for candidate_event in event_values {
            let Some(event) = as_record(candidate_event) else {
                return Err("剧情树事件无效".to_string());
            };
            let status = event
                .get("status")
                .and_then(|value| value.as_str())
                .filter(|status| EVENT_STATUSES.contains(status))
                .map(|status| status.to_string());
            let Some(status) = status else {
                return Err("剧情树事件无效".to_string());
            };
            let Some(Value::Array(source_values)) = event.get("sources") else {
                return Err("剧情树事件缺少来源引用".to_string());
            };
            if source_values.is_empty() {
                return Err("剧情树事件缺少来源引用".to_string());
            }
            let chapter_number = chapter_integer(event.get("chapterNumber"), "事件章节")?;
            let mut sources = Vec::with_capacity(source_values.len());
            for candidate_source in source_values {
                sources.push(parse_source_reference(candidate_source)?);
            }
            if !sources
                .iter()
                .all(|source| source_supports_event(source, &status, chapter_number, None))
            {
                return Err("剧情树事件状态与来源不匹配".to_string());
            }
            events.push(PlotTreeEvent {
                status,
                chapter_number,
                summary: required_text(event.get("summary"), "事件摘要")?,
                sources,
            });
        }
        if events
            .iter()
            .any(|event| event.chapter_number < start_chapter || event.chapter_number > end_chapter)
        {
            return Err("剧情树事件超出轨道章节范围".to_string());
        }
        let parent_track_id = match track.get("parentTrackId") {
            None | Some(Value::Null) => None,
            _ => Some(required_text(track.get("parentTrackId"), "父轨道 ID")?),
        };
        tracks.push(PlotTreeTrack {
            id,
            title,
            role,
            parent_track_id,
            start_chapter,
            end_chapter,
            summary,
            events,
        });
    }

    let mut by_id = std::collections::HashMap::new();
    for track in &tracks {
        if by_id.insert(track.id.clone(), track).is_some() {
            return Err("剧情树轨道 ID 重复".to_string());
        }
    }
    for track in &tracks {
        if track.role == "main" && track.parent_track_id.is_some() {
            return Err("剧情树主线不能有父轨道".to_string());
        }
        if track.role == "subplot" && track.parent_track_id.is_none() {
            return Err("剧情树支线必须归属一条主线".to_string());
        }
        if let Some(parent) = &track.parent_track_id {
            let is_main = by_id
                .get(parent)
                .map(|track| track.role == "main")
                .unwrap_or(false);
            if !is_main {
                return Err("剧情树支线父轨道必须是现有主线".to_string());
            }
        }
    }

    Ok(PlotTreeSnapshot {
        version: 1,
        generated_at,
        writing_language,
        source_revision,
        tracks,
    })
}

/// 对齐基线 `assertPlotTreeSnapshot` —— 新生成快照须与当前来源完全对齐
///
/// `expected_revision` 由调用方（`plot_tree_repository`）按 `JSON.stringify(facts)` 的
/// SHA-256 算出后传入 —— 校验模块因此不依赖数据库与哈希工具。
pub fn assert_plot_tree_snapshot(
    value: &Value,
    sources: &PlotTreeSourceFacts,
    expected_revision: &str,
) -> Result<PlotTreeSnapshot, String> {
    let snapshot = assert_stored_plot_tree_snapshot(value)?;
    if snapshot.writing_language != sources.writing_language {
        return Err("剧情树快照写作语言与当前项目不匹配".to_string());
    }
    if snapshot.source_revision.as_deref() != Some(expected_revision) {
        return Err("剧情树来源版本不匹配".to_string());
    }
    for track in &snapshot.tracks {
        for event in &track.events {
            for source in &event.sources {
                assert_source_exists(source, sources)?;
            }
            if !event.sources.iter().all(|source| {
                source_supports_event(source, &event.status, event.chapter_number, Some(sources))
            }) {
                return Err("剧情树事件状态与来源不匹配".to_string());
            }
        }
    }
    assert_plot_tree_snapshot_chapter_bounds(&snapshot, sources)?;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn facts() -> PlotTreeSourceFacts {
        PlotTreeSourceFacts {
            writing_language: "zh-CN".to_string(),
            synopsis: PlotTreeSynopsisFact {
                content: "梗概".to_string(),
            },
            blueprints: vec![PlotTreeBlueprintFact {
                chapter_number: 1,
                title: "第一章".to_string(),
                purpose: "开端".to_string(),
                key_events: "事件".to_string(),
            }],
            finalized_chapters: vec![PlotTreeFinalizedChapterFact {
                draft_id: 7,
                chapter_number: 2,
                title: "第二章".to_string(),
                summary: "摘要".to_string(),
            }],
            narrative_threads: vec![PlotTreeNarrativeThreadFact {
                id: 3,
                title: "线索".to_string(),
                thread_type: "foreshadow".to_string(),
                target_start_chapter: 1,
                target_end_chapter: 5,
                author_intent: "意图".to_string(),
                status: "planted".to_string(),
                events: vec![PlotTreeNarrativeThreadEventFact {
                    id: 9,
                    chapter_number: 2,
                    event_type: "planted".to_string(),
                    evidence: "证据".to_string(),
                    reason: "理由".to_string(),
                }],
            }],
        }
    }

    /// 测试用 `sourceRevision`：`JSON.stringify(facts)` 的 SHA-256 小写 hex
    ///
    /// 生产路径由 `plot_tree_repository` 提供同一算法（共享
    /// `character_roster_repository::hash_text`），此处复制以便校验模块单测自洽。
    fn revision_of(sources: &PlotTreeSourceFacts) -> String {
        use sha2::{Digest, Sha256};
        let json = serde_json::to_string(sources).expect("事实集应可序列化");
        let digest = Sha256::digest(json.as_bytes());
        let mut out = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write;
            let _ = write!(out, "{byte:02x}");
        }
        out
    }

    /// 把合法快照 JSON 解析为强类型快照（供边界函数直测）
    fn payload_snapshot(revision: &str) -> PlotTreeSnapshot {
        assert_stored_plot_tree_snapshot(&valid_snapshot_json(revision)).expect("夹具快照应合法")
    }

    fn valid_snapshot_json(revision: &str) -> Value {
        json!({
            "version": 1,
            "generatedAt": "2026-10-08T10:00:00.000Z",
            "writingLanguage": "zh-CN",
            "sourceRevision": revision,
            "tracks": [{
                "id": "main-1",
                "title": "主线",
                "role": "main",
                "startChapter": 1,
                "endChapter": 2,
                "summary": "主线摘要",
                "events": [
                    { "status": "planned", "chapterNumber": 1, "summary": "规划", "sources": [{ "type": "blueprint", "chapterNumber": 1 }] },
                    { "status": "occurred", "chapterNumber": 2, "summary": "已发生", "sources": [{ "type": "finalized-chapter", "draftId": 7, "chapterNumber": 2 }] }
                ]
            }]
        })
    }

    #[test]
    fn source_revision_requires_64_lowercase_hex_test() {
        assert!(is_plot_tree_source_revision(&"a".repeat(64)));
        assert!(is_plot_tree_source_revision(&"0123456789abcdef".repeat(4)));
        assert!(!is_plot_tree_source_revision(&"a".repeat(63)));
        assert!(!is_plot_tree_source_revision(&"a".repeat(65)));
        assert!(!is_plot_tree_source_revision(&"A".repeat(64)), "大写不接受");
        assert!(!is_plot_tree_source_revision(&"g".repeat(64)));
    }

    #[test]
    fn source_bounds_ignore_empty_sources_test() {
        let mut sources = facts();
        sources.blueprints = vec![PlotTreeBlueprintFact {
            chapter_number: 99,
            title: "".to_string(),
            purpose: "".to_string(),
            key_events: "".to_string(),
        }];
        sources.finalized_chapters = vec![];
        sources.narrative_threads = vec![];
        assert!(
            !has_usable_plot_tree_event_source(&sources),
            "空内容蓝图不得放宽来源域"
        );
    }

    #[test]
    fn source_bounds_cover_thread_range_and_events_test() {
        let sources = facts();
        assert_eq!(plot_tree_source_chapter_bounds(&sources), Some((1, 5)));
    }

    #[test]
    fn source_bounds_reject_thread_without_identity_test() {
        let mut sources = facts();
        sources.blueprints = vec![];
        sources.finalized_chapters = vec![];
        sources.narrative_threads[0].title = "  ".to_string();
        assert_eq!(plot_tree_source_chapter_bounds(&sources), None);
    }

    #[test]
    fn stored_snapshot_accepts_valid_payload_test() {
        let revision = "b".repeat(64);
        let snapshot = assert_stored_plot_tree_snapshot(&valid_snapshot_json(&revision)).unwrap();
        assert_eq!(snapshot.version, 1);
        assert_eq!(snapshot.writing_language, "zh-CN");
        assert_eq!(snapshot.tracks.len(), 1);
        assert_eq!(snapshot.tracks[0].events.len(), 2);
        assert_eq!(snapshot.tracks[0].parent_track_id, None);
    }

    #[test]
    fn stored_snapshot_rejects_bad_version_or_language_test() {
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["version"] = json!(2);
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树快照版本或写作语言无效"
        );

        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["writingLanguage"] = json!("fr-FR");
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树快照版本或写作语言无效"
        );

        assert_eq!(
            assert_stored_plot_tree_snapshot(&json!([])).unwrap_err(),
            "剧情树快照版本或写作语言无效"
        );
    }

    #[test]
    fn stored_snapshot_validates_generated_at_test() {
        for bad in ["", "   ", "not-a-date", "2026/10/08", "2026-13-01"] {
            let mut payload = valid_snapshot_json(&"b".repeat(64));
            payload["generatedAt"] = json!(bad);
            let error = assert_stored_plot_tree_snapshot(&payload).unwrap_err();
            assert_eq!(error, "剧情树生成时间无效", "应报生成时间无效：{bad}");
        }
        // 合法形态
        for good in [
            "2026-10-08",
            "2026-10-08T10:00:00Z",
            "2026-10-08T10:00:00.123+08:00",
            "2026-10-08T10:00",
        ] {
            let mut payload = valid_snapshot_json(&"b".repeat(64));
            payload["generatedAt"] = json!(good);
            assert!(
                assert_stored_plot_tree_snapshot(&payload).is_ok(),
                "应接受：{good}"
            );
        }
    }

    #[test]
    fn stored_snapshot_keeps_trimmed_generated_at_test() {
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["generatedAt"] = json!("  2026-10-08T10:00:00Z  ");
        let snapshot = assert_stored_plot_tree_snapshot(&payload).unwrap();
        assert_eq!(snapshot.generated_at, "2026-10-08T10:00:00Z");
    }

    #[test]
    fn stored_snapshot_validates_source_revision_shape_test() {
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["sourceRevision"] = json!("zz");
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树来源版本无效"
        );
        // 缺省即合法（可选字段）
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload.as_object_mut().unwrap().remove("sourceRevision");
        assert!(assert_stored_plot_tree_snapshot(&payload).is_ok());
    }

    #[test]
    fn stored_snapshot_requires_non_empty_tracks_test() {
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"] = json!([]);
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树轨道无效"
        );
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"] = json!("nope");
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树轨道无效"
        );
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["role"] = json!("side");
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树轨道无效"
        );
    }

    #[test]
    fn stored_snapshot_requires_event_state_matches_source_test() {
        // planned 事件引用已定稿章节 → 状态不匹配
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["events"][0]["sources"] =
            json!([{ "type": "finalized-chapter", "draftId": 7, "chapterNumber": 1 }]);
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树事件状态与来源不匹配"
        );

        // occurred 事件引用蓝图 → 状态不匹配
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["events"][1]["sources"] =
            json!([{ "type": "blueprint", "chapterNumber": 2 }]);
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树事件状态与来源不匹配"
        );
    }

    #[test]
    fn stored_snapshot_requires_event_within_track_range_test() {
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["events"][1]["chapterNumber"] = json!(9);
        payload["tracks"][0]["events"][1]["sources"] =
            json!([{ "type": "finalized-chapter", "draftId": 7, "chapterNumber": 9 }]);
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树事件超出轨道章节范围"
        );
    }

    #[test]
    fn stored_snapshot_requires_sources_present_test() {
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["events"][0]["sources"] = json!([]);
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树事件缺少来源引用"
        );
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["events"] = json!([]);
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树轨道事件无效"
        );
    }

    #[test]
    fn source_reference_rejects_invalid_shapes_test() {
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["events"][0]["sources"] = json!([{ "type": "unknown", "chapterNumber": 1 }]);
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树来源引用无效"
        );

        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["events"][0]["sources"] = json!([{ "type": "blueprint", "chapterNumber": 0 }]);
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树来源章节无效"
        );

        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["events"][0]["sources"] =
            json!([{ "type": "narrative-thread" }]);
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树来源叙事计划无效"
        );

        // planId 必须 ≥ 1
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["events"][0]["sources"] =
            json!([{ "type": "narrative-thread", "planId": 0 }]);
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树来源叙事计划无效"
        );

        // 仅给 planId 是**合法**的：planned 事件引用「计划区间」而非具体事件，
        // 持久化校验不要求历史来源仍存在（与基线一致）。
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["events"][0]["sources"] =
            json!([{ "type": "narrative-thread", "planId": 3 }]);
        assert!(assert_stored_plot_tree_snapshot(&payload).is_ok());

        // eventId 与 chapterNumber 必须同时给出
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["events"][0]["sources"] =
            json!([{ "type": "narrative-thread", "planId": 3, "eventId": 9 }]);
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树叙事来源引用无效"
        );
    }

    #[test]
    fn narrative_thread_planned_event_needs_no_sources_in_stored_check_test() {
        // 持久化校验（sources = None）：planned + 计划区间引用即合法，无需来源仍存在
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["events"][0]["sources"] =
            json!([{ "type": "narrative-thread", "planId": 999 }]);
        assert!(assert_stored_plot_tree_snapshot(&payload).is_ok());

        // occurred + eventId 引用
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["events"][1]["sources"] =
            json!([{ "type": "narrative-thread", "planId": 999, "eventId": 9, "chapterNumber": 2 }]);
        assert!(assert_stored_plot_tree_snapshot(&payload).is_ok());
    }

    #[test]
    fn track_hierarchy_rules_test() {
        // 重复 ID
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        let track = payload["tracks"][0].clone();
        payload["tracks"] = json!([track.clone(), track]);
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树轨道 ID 重复"
        );

        // 主线带父轨道
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["parentTrackId"] = json!("main-1");
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树主线不能有父轨道"
        );

        // 支线无父轨道
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        payload["tracks"][0]["role"] = json!("subplot");
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树支线必须归属一条主线"
        );

        // 支线父轨道不存在
        let mut payload = valid_snapshot_json(&"b".repeat(64));
        let subplot = json!({
            "id": "sub-1", "title": "支线", "role": "subplot", "parentTrackId": "missing",
            "startChapter": 1, "endChapter": 2, "summary": "支线摘要",
            "events": [{ "status": "planned", "chapterNumber": 1, "summary": "规划",
                         "sources": [{ "type": "blueprint", "chapterNumber": 1 }] }]
        });
        let mut tracks = payload["tracks"].clone();
        tracks.as_array_mut().unwrap().push(subplot);
        payload["tracks"] = tracks;
        assert_eq!(
            assert_stored_plot_tree_snapshot(&payload).unwrap_err(),
            "剧情树支线父轨道必须是现有主线"
        );
    }

    #[test]
    fn generation_check_requires_revision_match_test() {
        let sources = facts();
        let revision = revision_of(&sources);
        let payload = valid_snapshot_json(&revision);
        assert!(assert_plot_tree_snapshot(&payload, &sources, &revision).is_ok());

        let wrong = valid_snapshot_json(&"c".repeat(64));
        assert_eq!(
            assert_plot_tree_snapshot(&wrong, &sources, &revision).unwrap_err(),
            "剧情树来源版本不匹配"
        );

        let mut missing = valid_snapshot_json(&revision);
        missing.as_object_mut().unwrap().remove("sourceRevision");
        assert_eq!(
            assert_plot_tree_snapshot(&missing, &sources, &revision).unwrap_err(),
            "剧情树来源版本不匹配"
        );
    }

    #[test]
    fn generation_check_requires_language_match_test() {
        let sources = facts();
        let revision = revision_of(&sources);
        let mut payload = valid_snapshot_json(&revision);
        payload["writingLanguage"] = json!("en-US");
        assert_eq!(
            assert_plot_tree_snapshot(&payload, &sources, &revision).unwrap_err(),
            "剧情树快照写作语言与当前项目不匹配"
        );
    }

    #[test]
    fn generation_check_requires_sources_exist_test() {
        let sources = facts();
        let revision = revision_of(&sources);

        // 蓝图来源引用一个不存在的章号（章号与事件章号一致，先绕过状态匹配）
        let mut payload = valid_snapshot_json(&revision);
        payload["tracks"][0]["events"][0]["chapterNumber"] = json!(2);
        payload["tracks"][0]["events"][0]["sources"] =
            json!([{ "type": "blueprint", "chapterNumber": 2 }]);
        assert_eq!(
            assert_plot_tree_snapshot(&payload, &sources, &revision).unwrap_err(),
            "剧情树来源引用不存在"
        );

        // 定稿来源 draftId 不匹配
        let mut payload = valid_snapshot_json(&revision);
        payload["tracks"][0]["events"][1]["sources"] =
            json!([{ "type": "finalized-chapter", "draftId": 8, "chapterNumber": 2 }]);
        assert_eq!(
            assert_plot_tree_snapshot(&payload, &sources, &revision).unwrap_err(),
            "剧情树来源引用不存在"
        );

        // 叙事线索计划不存在
        let mut payload = valid_snapshot_json(&revision);
        payload["tracks"][0]["events"][0]["sources"] =
            json!([{ "type": "narrative-thread", "planId": 999 }]);
        assert_eq!(
            assert_plot_tree_snapshot(&payload, &sources, &revision).unwrap_err(),
            "剧情树来源引用不存在"
        );
    }

    #[test]
    fn generation_check_thread_event_uses_plan_range_test() {
        let sources = facts(); // 线索 3 的目标区间为 1..=5
        let revision = revision_of(&sources);

        // 计划区间内 planned 事件：合法
        let mut payload = valid_snapshot_json(&revision);
        payload["tracks"][0]["events"][0]["sources"] =
            json!([{ "type": "narrative-thread", "planId": 3 }]);
        assert!(assert_plot_tree_snapshot(&payload, &sources, &revision).is_ok());

        // 计划区间外 planned 事件：状态与来源不匹配（先把来源域扩到第 9 章，避免边界检查先报）
        let mut widened = sources.clone();
        widened.blueprints.push(PlotTreeBlueprintFact {
            chapter_number: 9,
            title: "第九章".to_string(),
            purpose: "终局".to_string(),
            key_events: "收束".to_string(),
        });
        let widened_revision = revision_of(&widened);
        let mut payload = valid_snapshot_json(&widened_revision);
        payload["tracks"][0]["endChapter"] = json!(9);
        payload["tracks"][0]["events"][0]["chapterNumber"] = json!(9);
        payload["tracks"][0]["events"][0]["sources"] =
            json!([{ "type": "narrative-thread", "planId": 3 }]);
        assert_eq!(
            assert_plot_tree_snapshot(&payload, &widened, &widened_revision).unwrap_err(),
            "剧情树事件状态与来源不匹配"
        );

        // eventId 引用：章号不匹配 → 来源不存在
        let mut payload = valid_snapshot_json(&widened_revision);
        payload["tracks"][0]["endChapter"] = json!(9);
        payload["tracks"][0]["events"][1]["chapterNumber"] = json!(9);
        payload["tracks"][0]["events"][1]["sources"] = json!([{
            "type": "narrative-thread", "planId": 3, "eventId": 9, "chapterNumber": 9
        }]);
        assert_eq!(
            assert_plot_tree_snapshot(&payload, &widened, &widened_revision).unwrap_err(),
            "剧情树来源引用不存在"
        );

        // eventId 引用：章号匹配 → 合法
        let mut payload = valid_snapshot_json(&revision);
        payload["tracks"][0]["events"][1]["sources"] = json!([{
            "type": "narrative-thread", "planId": 3, "eventId": 9, "chapterNumber": 2
        }]);
        assert!(assert_plot_tree_snapshot(&payload, &sources, &revision).is_ok());
    }

    #[test]
    fn generation_check_rejects_out_of_bounds_tracks_test() {
        let sources = facts(); // 边界 1..=5
        let revision = revision_of(&sources);
        let mut payload = valid_snapshot_json(&revision);
        payload["tracks"][0]["startChapter"] = json!(1);
        payload["tracks"][0]["endChapter"] = json!(6);
        assert_eq!(
            assert_plot_tree_snapshot(&payload, &sources, &revision).unwrap_err(),
            "剧情树轨道章节范围超出当前剧情来源"
        );
    }

    #[test]
    fn generation_check_requires_usable_sources_test() {
        let mut sources = facts();
        sources.blueprints = vec![];
        sources.finalized_chapters = vec![];
        sources.narrative_threads = vec![];
        let revision = revision_of(&sources);
        let payload = valid_snapshot_json(&revision);

        // ⚠️ 校验顺序与基线一致：`assertSourceExists` 先于章节边界检查执行，
        // 故 `assert_plot_tree_snapshot` 在无来源时先报「来源引用不存在」，
        // 「缺少有效事件来源」分支只能由边界函数自身触发。
        assert_eq!(
            assert_plot_tree_snapshot(&payload, &sources, &revision).unwrap_err(),
            "剧情树来源引用不存在"
        );
        assert_eq!(
            assert_plot_tree_snapshot_chapter_bounds(&payload_snapshot(&revision), &sources).unwrap_err(),
            "剧情树缺少有效事件来源"
        );
    }

    #[test]
    fn snapshot_serialization_omits_optional_fields_test() {
        let snapshot = assert_stored_plot_tree_snapshot(&valid_snapshot_json(&"b".repeat(64)))
            .unwrap();
        // ⚠️ 必须用 `to_string` 断言键顺序：`to_value` 会按字母序重排（BTreeMap），
        // 而真实序列化路径（及黄金哈希）走的是 `to_string`。
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains(r#""sourceRevision":"#), "已设置时须输出版本号");
        assert!(
            !json.contains("parentTrackId"),
            "未设置父轨道时不得输出 null 占位"
        );
        // 来源引用：blueprint 只有 type + chapterNumber，且键顺序与基线对象字面量一致
        assert!(
            json.contains(r#"{"type":"blueprint","chapterNumber":1}"#),
            "blueprint 来源引用形状与键顺序须一致：{json}"
        );
        // 快照顶层键顺序
        assert!(
            json.starts_with(
                r#"{"version":1,"generatedAt":"2026-10-08T10:00:00.000Z","writingLanguage":"zh-CN","sourceRevision":"#
            ),
            "快照顶层键顺序须与基线一致：{json}"
        );
    }

    #[test]
    fn narrative_thread_reference_serialization_shape_test() {
        let reference = PlotTreeSourceReference::NarrativeThread {
            plan_id: 3,
            event_id: Some(9),
            chapter_number: Some(2),
        };
        // 用 `to_string` 逐字节锁定键顺序（`to_value` 会按字母序重排）
        assert_eq!(
            serde_json::to_string(&reference).unwrap(),
            r#"{"type":"narrative-thread","planId":3,"eventId":9,"chapterNumber":2}"#
        );

        // 无 eventId / chapterNumber 时不输出这两个键
        let reference = PlotTreeSourceReference::NarrativeThread {
            plan_id: 3,
            event_id: None,
            chapter_number: None,
        };
        assert_eq!(
            serde_json::to_string(&reference).unwrap(),
            r#"{"type":"narrative-thread","planId":3}"#
        );

        // 另两种来源引用的形状
        assert_eq!(
            serde_json::to_string(&PlotTreeSourceReference::Blueprint { chapter_number: 1 }).unwrap(),
            r#"{"type":"blueprint","chapterNumber":1}"#
        );
        assert_eq!(
            serde_json::to_string(&PlotTreeSourceReference::FinalizedChapter {
                draft_id: 7,
                chapter_number: 2
            })
            .unwrap(),
            r#"{"type":"finalized-chapter","draftId":7,"chapterNumber":2}"#
        );
    }
}
