//! 知识库文本分块 —— 逐字移植基线 `electron/embedding.ts::chunkText`。
//!
//! # 长度口径：UTF-16 码元（不是码点）
//!
//! 基线全部长度判断走 JS `String.prototype.length`，即 **UTF-16 码元数**；
//! `slice` 也按码元切。故本模块统一用 `encode_utf16().count()` 计量，
//! 与批次 E 的 `draft_units.rs` 同一口径（见 `pi-development.md` 的 JS/Rust 语义对齐提示）。
//!
//! **唯一刻意偏离**：JS `slice` 允许把代理对（astral 字符，如 emoji）切成孤立代理项；
//! Rust `String` 必须是合法 UTF-8，无法表达孤立代理项。故本模块的切片边界**吸附到码点边界**
//! （宁可让边界偏移 1 个码元，也不把字符替换成 U+FFFD —— 知识库正文保真优先于长度精确到码元）。
//!
//! # 空白口径：ECMAScript `\s`
//!
//! JS 的 `\s` 含 `U+FEFF`（ZWNBSP）而不含 `U+0085`（NEL）；Rust `char::is_whitespace`
//! 反之。此处显式枚举 ECMAScript 集合（Zs 类用 `is_whitespace` 近似，故仍会多含 NEL）。

/// 判定 ECMAScript `\s`（WhiteSpace ∪ LineTerminator）
///
/// 与 JS 的唯一已知残差：Rust 的 `char::is_whitespace()` 含 `U+0085`（NEL），JS 不含。
pub fn is_js_whitespace(c: char) -> bool {
    c != '\u{85}' && (c.is_whitespace() || c == '\u{FEFF}')
}

/// ECMAScript `String.prototype.trim`
fn trim_js(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

/// UTF-16 码元长度（对齐 JS `String.prototype.length`）
fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// 把「UTF-16 码元偏移」换算为字节偏移，并吸附到码点边界（向下取整）
fn byte_index_at_utf16(s: &str, units: usize) -> usize {
    let mut counted = 0usize;
    for (byte_index, ch) in s.char_indices() {
        if counted >= units {
            return byte_index;
        }
        counted += ch.len_utf16();
    }
    s.len()
}

/// JS `String.prototype.slice(start, end)`（按 UTF-16 码元；边界不切断码点）
fn utf16_slice(s: &str, start: usize, end: usize) -> &str {
    let begin = byte_index_at_utf16(s, start);
    let stop = byte_index_at_utf16(s, end).max(begin);
    &s[begin..stop]
}

/// JS `String.prototype.slice(-n)`：取末尾 `n` 个 UTF-16 码元
fn tail_utf16(s: &str, n: usize) -> &str {
    let total = utf16_len(s);
    utf16_slice(s, total.saturating_sub(n), total)
}

/// 按 `/\n\s*\n/` 切段（JS 语义：贪婪回溯 → 匹配到空白运行中的**最后一个** LF）
fn split_paragraphs(text: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out: Vec<&str> = Vec::new();
    let mut segment_start = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i].1 == '\n' {
            let mut j = i + 1;
            let mut last_lf: Option<usize> = None;
            while j < chars.len() && is_js_whitespace(chars[j].1) {
                if chars[j].1 == '\n' {
                    last_lf = Some(j);
                }
                j += 1;
            }
            if let Some(lf) = last_lf {
                out.push(&text[segment_start..chars[i].0]);
                segment_start = chars[lf].0 + 1;
                i = lf + 1;
                continue;
            }
        }
        i += 1;
    }
    out.push(&text[segment_start..]);
    out
}

/// 按 `/(?<=[。！？.!?])\s*/` 切句（零宽后视 + 吞掉后随空白）
fn split_sentences(paragraph: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = paragraph.char_indices().collect();
    let mut out: Vec<&str> = Vec::new();
    let mut segment_start = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let (byte_index, ch) = chars[i];
        if matches!(ch, '。' | '！' | '？' | '.' | '!' | '?') {
            let mut end = byte_index + ch.len_utf8();
            let mut j = i + 1;
            while j < chars.len() && is_js_whitespace(chars[j].1) {
                end = chars[j].0 + chars[j].1.len_utf8();
                j += 1;
            }
            out.push(&paragraph[segment_start..end]);
            segment_start = end;
            i = j;
        } else {
            i += 1;
        }
    }
    if segment_start < paragraph.len() {
        out.push(&paragraph[segment_start..]);
    }
    out
}

/// `pushWithinLimit`：trim 后若超限则硬切（步进 = maxChars - overlap，至少前进 1）
fn push_within_limit(chunks: &mut Vec<String>, value: &str, max_chars: usize, overlap: usize) {
    let normalized = trim_js(value);
    if normalized.is_empty() {
        return;
    }
    let total = utf16_len(normalized);
    if total <= max_chars {
        chunks.push(normalized.to_string());
        return;
    }

    let mut start = 0usize;
    loop {
        let end = (start + max_chars).min(total);
        chunks.push(utf16_slice(normalized, start, end).to_string());
        if end == total {
            break;
        }
        start = (start + 1).max(end.saturating_sub(overlap));
    }
}

/// 将文本按段落分块，每块约 `max_chars` 个 UTF-16 码元。
///
/// 与基线 `chunkText(text, maxChars = 500, overlap = 50)` 行为一致；
/// 返回值至少含一项（空输入回退为 `[text.trim()]`）。
pub fn chunk_text(text: &str, max_chars: usize, overlap: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for paragraph in split_paragraphs(text) {
        if trim_js(paragraph).is_empty() {
            continue;
        }

        // 段落本身超限：按句切分累积
        if utf16_len(paragraph) > max_chars {
            if !current.is_empty() {
                push_within_limit(&mut chunks, &current, max_chars, overlap);
                current.clear();
            }
            let mut sentence_chunk = String::new();
            for sentence in split_sentences(paragraph) {
                if !sentence_chunk.is_empty()
                    && utf16_len(&sentence_chunk) + utf16_len(sentence) > max_chars
                {
                    push_within_limit(&mut chunks, &sentence_chunk, max_chars, overlap);
                    sentence_chunk = format!("{}{}", tail_utf16(&sentence_chunk, overlap), sentence);
                } else {
                    sentence_chunk.push_str(sentence);
                }
            }
            if !trim_js(&sentence_chunk).is_empty() {
                current = sentence_chunk;
            }
            continue;
        }

        // 累积段落
        if !current.is_empty() && utf16_len(&current) + utf16_len(paragraph) > max_chars {
            push_within_limit(&mut chunks, &current, max_chars, overlap);
            current = format!("{}\n\n{}", tail_utf16(&current, overlap), paragraph);
        } else {
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(paragraph);
        }
    }

    if !trim_js(&current).is_empty() {
        push_within_limit(&mut chunks, &current, max_chars, overlap);
    }

    if chunks.is_empty() {
        vec![trim_js(text).to_string()]
    } else {
        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_one_chunk_test() {
        assert_eq!(chunk_text("春来江水绿如蓝。", 500, 50), vec!["春来江水绿如蓝。"]);
    }

    #[test]
    fn empty_text_falls_back_to_single_empty_chunk_test() {
        assert_eq!(chunk_text("   \n\n  ", 500, 50), vec![""]);
    }

    #[test]
    fn paragraphs_join_with_double_newline_test() {
        let chunks = chunk_text("第一段。\n\n第二段。", 500, 50);
        assert_eq!(chunks, vec!["第一段。\n\n第二段。"]);
    }

    #[test]
    fn paragraph_split_matches_js_alternation_test() {
        // 多个空行（含空白）只算一次切分，且切点落在空白运行中的最后一个 LF
        let text = "甲。\n \n \n乙。";
        assert_eq!(chunk_text(text, 500, 50), vec!["甲。\n\n乙。"]);
    }

    #[test]
    fn overlong_paragraph_is_hard_split_by_utf16_units_test() {
        // 200 个汉字（BMP，1 码元/字）→ 上限 80，步进 80-0 = 80
        let text = "春".repeat(200);
        let chunks = chunk_text(&text, 80, 0);
        assert_eq!(chunks.len(), 3);
        assert_eq!(utf16_len(&chunks[0]), 80);
        assert_eq!(utf16_len(&chunks[1]), 80);
        assert_eq!(utf16_len(&chunks[2]), 40);
    }

    #[test]
    fn hard_split_uses_overlap_stride_test() {
        // 上限 10、overlap 4 → 步进 6
        let text = "甲".repeat(25);
        let chunks = chunk_text(&text, 10, 4);
        assert_eq!(chunks.len(), 4); // 0..10 / 6..16 / 12..22 / 18..25
        assert_eq!(utf16_len(&chunks[0]), 10);
        assert_eq!(utf16_len(&chunks[1]), 10);
        assert_eq!(utf16_len(&chunks[2]), 10);
        assert_eq!(utf16_len(&chunks[3]), 7);
    }

    #[test]
    fn long_paragraph_splits_at_cjk_sentence_punctuation_test() {
        // 单段 60 字：5 句各 11 字 + 1 句 5 字；上限 30
        let paragraph = "一二三四五六七八九十。".repeat(5) + "甲乙丙丁戊。";
        let chunks = chunk_text(&paragraph, 30, 0);
        assert!(chunks.len() >= 2, "长段必须被句点切分: {chunks:?}");
        for chunk in &chunks {
            assert!(utf16_len(chunk) <= 30, "每块不得超限: {chunk:?}");
        }
        assert_eq!(chunks.concat(), paragraph.trim());
    }

    #[test]
    fn emoji_boundary_never_produces_replacement_char_test() {
        // astral 字符（4 字节 / 2 码元）：边界吸附到码点，绝不产生 U+FFFD
        let text = "😀".repeat(20);
        let chunks = chunk_text(&text, 5, 0);
        for chunk in &chunks {
            assert!(!chunk.contains('\u{FFFD}'), "不得出现替换字符: {chunk:?}");
            assert!(chunk.chars().all(|c| c == '😀'));
        }
        // 吸附是「取整到完整码点」：5 码元预算下整取 3 个 emoji（6 码元，超限 < 1 码点）
        assert_eq!(chunks[0], "😀😀😀");
        assert!(utf16_len(&chunks[0]) <= 5 + 1, "超限不得超过 1 个码点");
    }

    #[test]
    fn js_whitespace_set_matches_ecmascript_test() {
        assert!(is_js_whitespace('\u{FEFF}')); // ZWNBSP：JS 含，Rust 默认不含
        assert!(!is_js_whitespace('\u{85}')); // NEL：Rust 含，JS 不含
        assert!(is_js_whitespace('\u{2028}')); // LS
        assert!(is_js_whitespace('\u{3000}')); // 全角空格（Zs）
    }

    #[test]
    fn zero_max_chars_terminates_test() {
        // 退化参数不得死循环（与 JS 一致：一直前进到结尾）
        let chunks = chunk_text("甲乙丙", 0, 0);
        assert!(!chunks.is_empty());
    }
}
