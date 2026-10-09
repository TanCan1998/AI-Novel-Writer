//! 导入运行批次检查点 ID —— `src/shared/import-run.ts:89-138` 的 Rust 单源（纯函数）。
//!
//! 契约形态：`` `${startChapter}-${endChapter}-${prefix(8).join('.')}` ``，
//! 例如 `3-5-a1b2c3d4.11223344.55667788`。它是「批次推进」与「effect receipt」
//! 的**规范身份**（同一批次可跨进程重算得到同一 ID），因此必须逐字节对齐基线。
//!
//! G2a 只消费 [`parse_import_run_chapter_batch_checkpoint_id`]（knowledge / blueprints
//! 批次的完成章数投影）；本模块同时落地配对的 [`create_import_run_chapter_batch_checkpoint_id`]，
//! 供 G3 的 `completeBatch` / effect receipt 复用，避免二次编辑同一文件。
//!
//! 正则口径：基线的 `\d` 为 ASCII，故此处写 `[0-9]`（Rust `regex` 的 `\d` 默认含 Unicode 数字）。

use std::sync::OnceLock;

use regex::Regex;

/// 参照知识批次大小（对齐 `IMPORT_RUN_KNOWLEDGE_BATCH_SIZE`）
pub const IMPORT_RUN_KNOWLEDGE_BATCH_SIZE: usize = 10;
/// 章节蓝图批次大小（对齐 `IMPORT_RUN_BLUEPRINT_BATCH_SIZE`）
pub const IMPORT_RUN_BLUEPRINT_BATCH_SIZE: usize = 5;

/// 内容指纹格式（小写 sha256）
fn is_content_fingerprint(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// 对齐契约 `ImportRunChapterBatchCheckpoint`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRunChapterBatchCheckpoint {
    pub start_chapter: i64,
    pub end_chapter: i64,
    pub content_fingerprint_prefixes: Vec<String>,
}

fn checkpoint_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"^([1-9][0-9]*)-([1-9][0-9]*)-([a-f0-9]{8}(?:\.[a-f0-9]{8})*)$")
            .expect("批次检查点正则")
    })
}

/// 对齐 `createImportRunChapterBatchCheckpointId`（要求章号连续、指纹合法）。
pub fn create_import_run_chapter_batch_checkpoint_id(
    chapters: &[(i64, String)],
) -> Result<String, String> {
    let Some(first) = chapters.first() else {
        return Err("导入章节批次不能为空".to_string());
    };
    for (index, (number, fingerprint)) in chapters.iter().enumerate() {
        if *number < 1 {
            return Err("导入章节批次章号无效".to_string());
        }
        if !is_content_fingerprint(fingerprint) {
            return Err("导入章节批次内容指纹无效".to_string());
        }
        if index > 0 && *number != chapters[index - 1].0 + 1 {
            return Err("导入章节批次必须连续".to_string());
        }
    }
    let last = chapters[chapters.len() - 1].0;
    let prefixes = chapters
        .iter()
        .map(|(_, fingerprint)| fingerprint[..8].to_string())
        .collect::<Vec<_>>()
        .join(".");
    Ok(format!("{}-{}-{}", first.0, last, prefixes))
}

/// 对齐 `parseImportRunChapterBatchCheckpointId`（不合法返回 `None`）。
pub fn parse_import_run_chapter_batch_checkpoint_id(
    checkpoint_id: &str,
) -> Option<ImportRunChapterBatchCheckpoint> {
    let captures = checkpoint_pattern().captures(checkpoint_id)?;
    let start_chapter: i64 = captures.get(1)?.as_str().parse().ok()?;
    let end_chapter: i64 = captures.get(2)?.as_str().parse().ok()?;
    let content_fingerprint_prefixes: Vec<String> = captures
        .get(3)?
        .as_str()
        .split('.')
        .map(str::to_string)
        .collect();
    if end_chapter < start_chapter
        || content_fingerprint_prefixes.len() as i64 != end_chapter - start_chapter + 1
    {
        return None;
    }
    Some(ImportRunChapterBatchCheckpoint {
        start_chapter,
        end_chapter,
        content_fingerprint_prefixes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(prefix: &str) -> String {
        format!("{prefix}{}", "0".repeat(64 - prefix.len()))
    }

    #[test]
    fn create_and_parse_roundtrip_test() {
        let chapters = vec![
            (3, fingerprint("a1b2c3d4")),
            (4, fingerprint("11223344")),
            (5, fingerprint("55667788")),
        ];
        let id = create_import_run_chapter_batch_checkpoint_id(&chapters).unwrap();
        assert_eq!(id, "3-5-a1b2c3d4.11223344.55667788");
        let parsed = parse_import_run_chapter_batch_checkpoint_id(&id).unwrap();
        assert_eq!(parsed.start_chapter, 3);
        assert_eq!(parsed.end_chapter, 5);
        assert_eq!(
            parsed.content_fingerprint_prefixes,
            vec!["a1b2c3d4", "11223344", "55667788"]
        );
    }

    #[test]
    fn create_rejects_invalid_input_test() {
        assert!(create_import_run_chapter_batch_checkpoint_id(&[]).is_err());
        assert!(
            create_import_run_chapter_batch_checkpoint_id(&[(0, fingerprint("a1b2c3d4"))]).is_err()
        );
        assert!(create_import_run_chapter_batch_checkpoint_id(&[(1, "zz".to_string())]).is_err());
        assert!(create_import_run_chapter_batch_checkpoint_id(&[
            (1, fingerprint("a1b2c3d4")),
            (3, fingerprint("11223344")),
        ])
        .is_err());
    }

    #[test]
    fn parse_rejects_malformed_ids_test() {
        assert!(parse_import_run_chapter_batch_checkpoint_id("").is_none());
        assert!(parse_import_run_chapter_batch_checkpoint_id("done").is_none());
        assert!(parse_import_run_chapter_batch_checkpoint_id("chapter:3").is_none());
        // 前缀个数与区间不匹配
        assert!(parse_import_run_chapter_batch_checkpoint_id("3-5-a1b2c3d4").is_none());
        // 起始章号为 0（基线正则要求 [1-9] 开头）
        assert!(parse_import_run_chapter_batch_checkpoint_id("0-1-a1b2c3d4").is_none());
        // 区间倒置
        assert!(
            parse_import_run_chapter_batch_checkpoint_id("5-3-a1b2c3d4.11223344.55667788")
                .is_none()
        );
        // 前缀长度不足 8
        assert!(parse_import_run_chapter_batch_checkpoint_id("1-1-a1b2").is_none());
    }
}
