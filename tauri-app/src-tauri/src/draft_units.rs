//! draft_units —— 可见正文字数契约（对齐 `src/shared/draft-units.ts`）。
//!
//! `countDraftUnits`（v3，现行写入/UI 口径）：
//! - 每个 Han 码点计 1（含扩展区）；
//! - 任意 Unicode 文字的「词」计 1（重音拉丁文不再被拆成 ASCII 词 + 散字符）；
//! - 数字与其他非词可见码点保留逐码点计数；
//! - 空白、标点、符号（含 emoji）不计入。
//! `countLegacyDraftUnitsV1`（v0.9.0）仅为持久化幂等兼容保留。
//!
//! 归一化：v3 先做 NFC（unicode-normalization，Ask first 已批准）。

use unicode_normalization::UnicodeNormalization;

/// 对齐基线 `countDraftUnits`
pub fn count_draft_units(text: &str) -> i64 {
    let normalized: String = text.nfc().collect();
    let re_han = regex::Regex::new(r"\p{Han}").unwrap();
    let re_word = regex::Regex::new(r"\p{L}[\p{L}\p{M}]*(?:['’]\p{L}[\p{L}\p{M}]*)*").unwrap();
    let re_skip = regex::Regex::new(r"[\s\p{P}\p{S}]").unwrap();

    let han_characters = re_han.find_iter(&normalized).count() as i64;
    // JS `replace(HAN, ' ')`：Han 区段替换为单个空格（长度不变语义仅影响后续切词边界）
    let without_han = re_han.replace_all(&normalized, " ").into_owned();
    let words = re_word.find_iter(&without_han).count() as i64;
    let after_words = re_word.replace_all(&without_han, "").into_owned();
    let other_visible = after_words.chars().filter(|c| !re_skip.is_match(&c.to_string())).count() as i64;
    han_characters + words + other_visible
}

/// 对齐基线 `countLegacyDraftUnitsV1`（v0.9.0 幂等兼容口径）
pub fn count_legacy_draft_units_v1(text: &str) -> i64 {
    let re_english = regex::Regex::new(r"[A-Za-z]+(?:['’][A-Za-z]+)*").unwrap();
    let re_legacy_han = regex::Regex::new(r"[\u{3400}-\u{4DBF}\u{4E00}-\u{9FFF}\u{F900}-\u{FAFF}]").unwrap();
    let re_skip = regex::Regex::new(r"[\s\p{P}\p{S}]").unwrap();

    let english_words = re_english.find_iter(text).count() as i64;
    let without_english = re_english.replace_all(text, "").into_owned();
    let chinese_characters = re_legacy_han.find_iter(&without_english).count() as i64;
    let other = without_english
        .chars()
        .filter(|c| !re_legacy_han.is_match(&c.to_string()) && !re_skip.is_match(&c.to_string()))
        .count() as i64;
    chinese_characters + english_words + other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_han_words_and_visible_code_points_test() {
        // 纯中文：逐字计数
        assert_eq!(count_draft_units("林决在城门"), 5);
        // 中英混排：英文词计 1
        assert_eq!(count_draft_units("林决 uses 灵力"), 5);
        // 标点与空白不计
        assert_eq!(count_draft_units("林决，城门！"), 4);
        // emoji（符号）不计，但 VS16 变体选择符为 Mn 类，基线同样逐码点计入 → 3
        assert_eq!(count_draft_units("林决🗡️"), 3);
        // 数字逐码点
        assert_eq!(count_draft_units("第3章"), 3);
        // 重音拉丁词整词计数
        assert_eq!(count_draft_units("café café"), 2);
        // NFC 归一化：中 + VS16 → Han 计 1，VS16（Mn）逐码点计入 → 2（与基线一致）
        assert_eq!(count_draft_units("\u{4e2d}\u{fe0f}"), 2);
    }

    #[test]
    fn legacy_v1_counter_keeps_old_semantics_test() {
        // 英文词 + 汉字 + 其他可见字符
        assert_eq!(count_legacy_draft_units_v1("林决 uses 灵力"), 5);
        assert_eq!(count_legacy_draft_units_v1("，！"), 0);
        assert_eq!(count_legacy_draft_units_v1("abc"), 1);
    }

    #[test]
    fn empty_and_whitespace_only_test() {
        assert_eq!(count_draft_units(""), 0);
        assert_eq!(count_draft_units("  \n\t"), 0);
        assert_eq!(count_legacy_draft_units_v1(""), 0);
    }
}
