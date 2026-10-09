//! 导入解析纯函数 —— 逐字平移自 `electron/controllers/import-controller.ts:73-296`。
//!
//! 全部为无 I/O 的纯函数（`default_file_identity` 除外，它只做一次
//! `canonicalize` + `metadata`），便于单元测试覆盖拆章边界。
//!
//! # 正则口径
//!
//! 基线的三条候选正则原样保留，仅把 JS 的 `\d`（ASCII）改写为 `[0-9]`
//! （Rust `regex` 的 `\d` 默认含 Unicode 数字），语义不变。

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::draft_units::count_draft_units;

/// 中文「第X章」格式（支持中文数字与阿拉伯数字，冒号可有可无）
const CN_CHAPTER_BODY: &str = r"第[一二三四五六七八九十百千零0-9]+章";
/// 英文「Chapter X」格式
const EN_CHAPTER_BODY: &str = r"Chapter\s+[0-9]+";

/// 候选正则池（惰性编译一次）：中文 → 英文 → Markdown 标题。
fn chapter_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            Regex::new(&format!(r"^{CN_CHAPTER_BODY}[\s：:·—-]*(.*)")).expect("中文章标题正则"),
            Regex::new(r"(?i)^Chapter\s+([0-9]+)[\s：:·—-]*(.*)").expect("英文章标题正则"),
            Regex::new(&format!(
                r"(?i)^#{{1,3}}\s+(?:{CN_CHAPTER_BODY}|{EN_CHAPTER_BODY})[\s：:·—-]*(.*)"
            ))
            .expect("Markdown 章标题正则"),
        ]
    })
}

/// SHA-256 小写 hex（对齐基线 `sha256()`）。
pub fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 基线 `ImportSourceFileIdentity`（仅主进程内存态，绝不回传渲染层）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSourceFileIdentity {
    pub canonical_location: String,
    pub file_identity: Option<String>,
}

/// 平移自 `defaultFileIdentity`：`realpathSync.native` + `statSync`。
pub fn default_file_identity(file_path: &str) -> Result<ImportSourceFileIdentity, String> {
    let canonical = std::fs::canonicalize(file_path).map_err(|error| error.to_string())?;
    // 对齐基线 `statSync(canonicalLocation)` 的存在性校验（大小在预检阶段单独读取）
    std::fs::metadata(&canonical).map_err(|error| error.to_string())?;
    Ok(ImportSourceFileIdentity {
        canonical_location: canonical.to_string_lossy().to_string(),
        file_identity: platform_file_identity(&canonical),
    })
}

/// Windows：`dev:<volumeSerial>:ino:<fileIndex>`（对齐基线 `stats.dev` / `stats.ino`）。
///
/// std 的 `MetadataExt::volume_serial_number/file_index` 仍属 unstable
/// （`windows_by_handle`），故直接调 Win32 `GetFileInformationByHandle`。
#[cfg(windows)]
fn platform_file_identity(path: &Path) -> Option<String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let file = std::fs::File::open(path).ok()?;
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `file` 持有有效句柄，`information` 为栈上可写结构；
    // 调用失败时 Win32 返回 0（不做任何写越界/悬垂访问）。
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) };
    if succeeded == 0 {
        return None;
    }
    let index = ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64;
    if index == 0 {
        // 对齐基线 `stats.ino === 0n ? {} : { fileIdentity }`
        return None;
    }
    Some(format!(
        "dev:{}:ino:{index}",
        information.dwVolumeSerialNumber
    ))
}

#[cfg(not(windows))]
fn platform_file_identity(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).ok()?;
    let index = metadata.ino();
    if index == 0 {
        // 对齐基线 `stats.ino === 0n ? {} : { fileIdentity }`
        None
    } else {
        Some(format!("dev:{}:ino:{index}", metadata.dev()))
    }
}

/// 平移自 `normalizedLocation`：trim + NFC + Windows 小写化。
///
/// ⚠️ 基线用 `toLocaleLowerCase('en-US')`，Rust 侧为 `to_lowercase()`；
/// 差异仅出现在土耳其语等特殊 locale 的大小写规则上（Windows 路径不受影响）。
pub fn normalized_location(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 32_000 || trimmed.contains('\0') {
        return Err("导入来源位置身份无效".to_string());
    }
    let normalized: String = trimmed.nfc().collect();
    Ok(if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    })
}

/// 平移自 `normalizedFileIdentity`。
pub fn normalized_file_identity(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 1_024 || trimmed.contains('\0') {
        return Err("导入来源文件身份无效".to_string());
    }
    Ok(trimmed.to_string())
}

/// D1：`locationAliasDigest = sha256(规范化位置)`（基线为 `HMAC(secret, …)`）。
pub fn location_alias_digest(canonical_location: &str) -> Result<String, String> {
    Ok(sha256_hex(&normalized_location(canonical_location)?))
}

/// D1：`fileAliasDigest = sha256(规范化文件身份)`（基线为 `HMAC(secret, …)`）。
pub fn file_alias_digest(file_identity: &str) -> Result<String, String> {
    Ok(sha256_hex(&normalized_file_identity(file_identity)?))
}

/// 平移自 `sourceMediaType`。
pub fn source_media_type(display_name: &str) -> String {
    let extension = Path::new(display_name)
        .extension()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "epub" => "application/epub+zip".to_string(),
        "md" => "text/markdown".to_string(),
        _ => "text/plain".to_string(),
    }
}

/// 文件名是否以 `.epub` 结尾（D4 诚实错误的判别入口）。
pub fn is_epub_name(display_name: &str) -> bool {
    Path::new(display_name)
        .extension()
        .map(|value| value.to_string_lossy().to_lowercase() == "epub")
        .unwrap_or(false)
}

/// 平移自 `chineseNumToArabic`（含 JS `parseInt` 的前缀数值语义）。
pub fn chinese_num_to_arabic(value: &str) -> i64 {
    let leading: String = value.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if !leading.is_empty() {
        if let Ok(parsed) = leading.parse::<i64>() {
            return parsed;
        }
        if let Ok(parsed) = leading.parse::<f64>() {
            return parsed as i64;
        }
    }
    let digit_value = |ch: char| -> Option<i64> {
        Some(match ch {
            '零' => 0,
            '一' => 1,
            '二' => 2,
            '三' => 3,
            '四' => 4,
            '五' => 5,
            '六' => 6,
            '七' => 7,
            '八' => 8,
            '九' => 9,
            '十' => 10,
            '百' => 100,
            '千' => 1000,
            _ => return None,
        })
    };
    let mut result: i64 = 0;
    let mut current: i64 = 0;
    for ch in value.chars() {
        let Some(found) = digit_value(ch) else {
            continue;
        };
        if found >= 10 {
            if current == 0 {
                current = 1;
            }
            current *= found;
            result += current;
            current = 0;
        } else {
            current = found;
        }
    }
    result + current
}

/// 平移自 `extractChapterNumber`：只认「第X章」与「Chapter X」，其余返回 0。
pub fn extract_chapter_number(line: &str) -> i64 {
    static CN: OnceLock<Regex> = OnceLock::new();
    static EN: OnceLock<Regex> = OnceLock::new();
    let cn = CN.get_or_init(|| {
        Regex::new(r"第([一二三四五六七八九十百千零0-9]+)章").expect("中文章号正则")
    });
    let en = EN.get_or_init(|| Regex::new(r"(?i)Chapter\s+([0-9]+)").expect("英文章号正则"));
    if let Some(captures) = cn.captures(line) {
        return chinese_num_to_arabic(&captures[1]);
    }
    if let Some(captures) = en.captures(line) {
        return captures[1].parse::<i64>().unwrap_or(0);
    }
    0
}

/// 平移自 `isChapterHeading`。
pub fn is_chapter_heading(line: &str) -> bool {
    let trimmed = line.trim();
    chapter_patterns()
        .iter()
        .any(|pattern| pattern.is_match(trimmed))
}

/// 平移自 `extractTitle`：取最后一个捕获组（标题部分），空则回退整行。
pub fn extract_title(line: &str) -> String {
    let trimmed = line.trim();
    for pattern in chapter_patterns() {
        if let Some(captures) = pattern.captures(trimmed) {
            if let Some(last) = captures.get(captures.len() - 1) {
                let title = last.as_str().trim();
                if !title.is_empty() {
                    return title.to_string();
                }
            }
            return trimmed.to_string();
        }
    }
    trimmed.to_string()
}

/// 平移自 `hasChapterHeadings`。
pub fn has_chapter_headings(content: &str) -> bool {
    content.split('\n').any(is_chapter_heading)
}

/// 拆章产物（对齐基线内部 `ParsedChapter`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedChapter {
    pub number: i64,
    pub title: String,
    pub content: String,
    pub word_count: i64,
}

/// 章节数超限的统一错误码（对齐基线 `IMPORT_CHAPTER_COUNT_EXCEEDED`）。
pub const IMPORT_CHAPTER_COUNT_EXCEEDED: &str = "IMPORT_CHAPTER_COUNT_EXCEEDED";

fn flush_chapter(
    header_line: &str,
    lines: &[String],
    auto_number: &mut i64,
    max_chapters: usize,
    chapters: &mut Vec<ParsedChapter>,
) -> Result<(), String> {
    *auto_number += 1;
    let extracted = extract_chapter_number(header_line);
    let number = if extracted != 0 {
        extracted
    } else {
        *auto_number
    };
    let body = lines.join("\n");
    let body = body.trim();
    if body.is_empty() {
        return Ok(());
    }
    if chapters.len() >= max_chapters {
        return Err(IMPORT_CHAPTER_COUNT_EXCEEDED.to_string());
    }
    chapters.push(ParsedChapter {
        number,
        title: extract_title(header_line),
        content: body.to_string(),
        word_count: count_draft_units(body),
    });
    Ok(())
}

/// 平移自 `splitSingleFileContent`（含 maxChapters 截断与「无标题兜底」）。
pub fn split_single_file_content(
    content: &str,
    max_chapters: usize,
) -> Result<Vec<ParsedChapter>, String> {
    let mut chapters: Vec<ParsedChapter> = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    let mut auto_number: i64 = 0;

    for line in content.split('\n') {
        if is_chapter_heading(line) {
            if let Some((header, lines)) = current.take() {
                flush_chapter(
                    &header,
                    &lines,
                    &mut auto_number,
                    max_chapters,
                    &mut chapters,
                )?;
            }
            current = Some((line.to_string(), Vec::new()));
        } else if let Some((_, lines)) = current.as_mut() {
            lines.push(line.to_string());
        } else {
            // 第一个章节标题之前的内容 → 前言/序章（首行充当标题行）
            current = Some((line.to_string(), Vec::new()));
        }
    }
    if let Some((header, lines)) = current.take() {
        flush_chapter(
            &header,
            &lines,
            &mut auto_number,
            max_chapters,
            &mut chapters,
        )?;
    }
    Ok(chapters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_num_to_arabic_matches_baseline_test() {
        assert_eq!(chinese_num_to_arabic("一"), 1);
        assert_eq!(chinese_num_to_arabic("十"), 10);
        assert_eq!(chinese_num_to_arabic("十一"), 11);
        assert_eq!(chinese_num_to_arabic("二十三"), 23);
        assert_eq!(chinese_num_to_arabic("一百零五"), 105);
        assert_eq!(chinese_num_to_arabic("12"), 12);
        // 无任何可识别字符 → 0（对齐基线 `parseInt` NaN 后循环无命中）
        assert_eq!(chinese_num_to_arabic("abc"), 0);
    }

    #[test]
    fn extract_chapter_number_matches_baseline_test() {
        assert_eq!(extract_chapter_number("第3章 开端"), 3);
        assert_eq!(extract_chapter_number("第十章"), 10);
        assert_eq!(extract_chapter_number("Chapter 12: The Road"), 12);
        // 「第X节」不是章标题 → 0
        assert_eq!(extract_chapter_number("第3节"), 0);
        assert_eq!(extract_chapter_number("3"), 0);
        assert_eq!(extract_chapter_number("普通正文"), 0);
    }

    #[test]
    fn is_chapter_heading_and_title_matches_baseline_test() {
        assert!(is_chapter_heading("第一章 开端"));
        assert!(is_chapter_heading("# 第三章"));
        assert!(is_chapter_heading("Chapter 5: Intro"));
        assert!(!is_chapter_heading("普通正文"));
        assert_eq!(extract_title("第一章 开端"), "开端");
        assert_eq!(extract_title("第一章"), "第一章");
        assert_eq!(extract_title("# 第三章"), "# 第三章");
        assert_eq!(extract_title("Chapter 5: Intro"), "Intro");
    }

    #[test]
    fn has_chapter_headings_scans_all_lines_test() {
        assert!(has_chapter_headings("开场白\n第二章 继续"));
        assert!(!has_chapter_headings("开场白\n正文"));
    }

    #[test]
    fn split_single_file_content_splits_multiple_chapters_test() {
        let content = "第一章 开端\n正文甲\n第二章 发展\n正文乙";
        let chapters = split_single_file_content(content, 5_000).unwrap();
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].number, 1);
        assert_eq!(chapters[0].title, "开端");
        assert_eq!(chapters[0].content, "正文甲");
        assert_eq!(chapters[0].word_count, 3);
        assert_eq!(chapters[1].number, 2);
        assert_eq!(chapters[1].content, "正文乙");
    }

    #[test]
    fn split_single_file_content_truncates_at_max_chapters_test() {
        let content = "第一章 甲\n正文\n第二章 乙\n正文\n第三章 丙\n正文";
        let error = split_single_file_content(content, 2).unwrap_err();
        assert_eq!(error, IMPORT_CHAPTER_COUNT_EXCEEDED);
    }

    #[test]
    fn split_single_file_content_treats_preamble_as_chapter_test() {
        // 基线：无标题时整段视作一章，首行充当标题行
        let chapters = split_single_file_content("书名\n正文一\n正文二", 5_000).unwrap();
        assert_eq!(chapters.len(), 1);
        // 首行充当标题行，不进入正文（与基线一致）
        assert_eq!(chapters[0].title, "书名");
        assert_eq!(chapters[0].content, "正文一\n正文二");
        assert_eq!(chapters[0].number, 1); // 无章号 → 自动编号 1
    }

    #[test]
    fn source_media_type_matches_baseline_test() {
        assert_eq!(source_media_type("a.txt"), "text/plain");
        assert_eq!(source_media_type("a.md"), "text/markdown");
        assert_eq!(source_media_type("A.MD"), "text/markdown");
        assert_eq!(source_media_type("a.epub"), "application/epub+zip");
        assert_eq!(source_media_type("noext"), "text/plain");
        assert!(is_epub_name("a.EPUB"));
        assert!(!is_epub_name("a.txt"));
    }

    #[test]
    fn normalized_location_lowercases_on_windows_test() {
        let normalized = normalized_location("  C:\\Novel\\Book.TXT  ").unwrap();
        if cfg!(windows) {
            assert_eq!(normalized, "c:\\novel\\book.txt");
        } else {
            assert_eq!(normalized, "C:\\Novel\\Book.TXT");
        }
        assert!(normalized_location("   ").is_err());
        assert!(normalized_location("a\0b").is_err());
    }

    #[test]
    fn alias_digests_are_lowercase_sha256_test() {
        let digest = location_alias_digest("C:\\Novel\\Book.txt").unwrap();
        assert_eq!(digest.len(), 64);
        assert!(digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        let file_digest = file_alias_digest("dev:1:ino:2").unwrap();
        assert_eq!(file_digest.len(), 64);
    }
}
