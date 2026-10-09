//! 知识库 FTS5 关键词检索支路（批次 F2 / 方案 B 的第一路）。
//!
//! # 为什么预分词
//!
//! FTS5 内置 `unicode61` 分词器按空白切词，中文整句会成为**单一 token**（实测召回率 0%，
//! 见 `docs-fork/research/archive/2026-10-08-f2/2026-10-08-fts5-tokenizer-findings.md`）。
//! 故本层在**写入前**用 `jieba` 切词、以空格连接后存入 `tokens` 列；**查询时**同样先切词，
//! 再把 token 用 `OR` 拼成 FTS5 查询串。虚拟表因此声明 `tokenize = 'unicode61'`
//! （只承担按空白切已分好的词），而非 `porter`（英文词干器，会改写 ASCII token 语义）。
//!
//! # 评分口径
//!
//! SQLite `bm25()` 返回**负值**（越负越相关）。本层统一取 `-bm25()`，对外承诺
//! **分数越大越相关**（`SearchResult.score` 的语义）。RRF 融合只使用**名次**，不依赖绝对值。
//!
//! # FTS5 查询串转义
//!
//! 每个 token 用双引号包裹（内部 `"` 翻倍），避免 `-` / `:` / `*` 等被当作 FTS5 语法。

use jieba_rs::Jieba;
use rusqlite::{params, Connection};
use std::sync::OnceLock;

/// 进程级单例分词器（构造需加载词典，代价高）
fn jieba() -> &'static Jieba {
    static JIEBA: OnceLock<Jieba> = OnceLock::new();
    JIEBA.get_or_init(Jieba::new)
}

/// 中文分词：切出「至少含一个字母/数字/汉字」的 token
///
/// 纯标点 token（如 `，`）会被 `unicode61` 丢弃、在查询串里也会变成空短语，故在此前置过滤。
pub fn tokenize(text: &str) -> Vec<String> {
    jieba()
        .cut(text, true)
        .into_iter()
        .map(str::trim)
        .filter(|token| !token.is_empty() && token.chars().any(char::is_alphanumeric))
        .map(str::to_string)
        .collect()
}

/// 写入 `kb_fts.tokens` 列的形式：空格连接的预分词串
pub fn to_index_tokens(text: &str) -> String {
    tokenize(text).join(" ")
}

/// 将单个 token 包装为 FTS5 短语（内部 `"` 翻倍），避免 `-` / `:` / `*` 等被当作语法
pub fn quote_token(token: &str) -> String {
    format!("\"{}\"", token.replace('"', "\"\""))
}

/// 构造 FTS5 `MATCH` 查询串；无可检索 token 时返回 `None`
pub fn build_match_query(query: &str) -> Option<String> {
    let tokens = tokenize(query);
    if tokens.is_empty() {
        return None;
    }
    Some(
        tokens
            .iter()
            .map(|token| quote_token(token))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

/// 一条待写入 FTS 的文本块
#[derive(Debug, Clone)]
pub struct FtsChunk<'a> {
    pub chunk_id: &'a str,
    pub doc_id: &'a str,
    pub file_name: &'a str,
    pub corpus_kind: &'a str,
    /// 原始正文（写入前由本层分词）
    pub text: &'a str,
}

/// 命中项
#[derive(Debug, Clone, PartialEq)]
pub struct FtsHit {
    pub chunk_id: String,
    pub doc_id: String,
    /// `-bm25()`：越大越相关
    pub score: f64,
}

/// 写入 / 覆盖一条文本块的 FTS 索引（FTS5 无 UPSERT，先删后插）
pub fn replace_chunk(conn: &Connection, chunk: &FtsChunk<'_>) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM kb_fts WHERE chunk_id = ?1", params![chunk.chunk_id])?;
    conn.execute(
        "INSERT INTO kb_fts (chunk_id, doc_id, file_name, corpus_kind, tokens)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            chunk.chunk_id,
            chunk.doc_id,
            chunk.file_name,
            chunk.corpus_kind,
            to_index_tokens(chunk.text)
        ],
    )?;
    Ok(())
}

/// 按 `chunk_id` 删除（返回被删行数）
pub fn delete_chunk(conn: &Connection, chunk_id: &str) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM kb_fts WHERE chunk_id = ?1", params![chunk_id])
}

/// 按 `doc_id` 删除该文档的全部块（返回被删行数）
pub fn delete_document(conn: &Connection, doc_id: &str) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM kb_fts WHERE doc_id = ?1", params![doc_id])
}

/// 清空整张 FTS 索引（返回被删行数）
pub fn clear(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM kb_fts", [])
}

/// FTS5 关键词检索，按 `-bm25()` 降序返回前 `limit` 条
pub fn search(conn: &Connection, query: &str, limit: usize) -> rusqlite::Result<Vec<FtsHit>> {
    let Some(match_query) = build_match_query(query) else {
        return Ok(Vec::new());
    };
    let mut statement = conn.prepare(
        "SELECT chunk_id, doc_id, -bm25(kb_fts) AS score
         FROM kb_fts
         WHERE kb_fts MATCH ?1
         ORDER BY score DESC
         LIMIT ?2",
    )?;
    let rows = statement.query_map(params![match_query, limit as i64], |row| {
        Ok(FtsHit {
            chunk_id: row.get(0)?,
            doc_id: row.get(1)?,
            score: row.get(2)?,
        })
    })?;
    rows.collect()
}

/// 知识库 FTS 层错误
#[derive(Debug)]
pub enum FtsError {
    /// SQLite 层失败
    Sqlite(rusqlite::Error),
}

impl std::fmt::Display for FtsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "知识库 FTS 操作失败：{error}"),
        }
    }
}

impl std::error::Error for FtsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
        }
    }
}

impl From<rusqlite::Error> for FtsError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::CREATE_KB_FTS;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(CREATE_KB_FTS).unwrap();
        conn
    }

    fn chunk<'a>(id: &'a str, doc: &'a str, text: &'a str) -> FtsChunk<'a> {
        FtsChunk {
            chunk_id: id,
            doc_id: doc,
            file_name: "素材.txt",
            corpus_kind: "project-knowledge",
            text,
        }
    }

    #[test]
    fn tokenize_drops_pure_punctuation_test() {
        let tokens = tokenize("，。！ 春江 ");
        assert!(tokens.contains(&"春江".to_string()));
        assert!(!tokens.iter().any(|t| t == "，" || t == "。"));
    }

    #[test]
    fn match_query_quotes_and_joins_with_or_test() {
        // jieba 可能把整句切成单个 token；多词查询才能观察到 OR 拼接
        let query = build_match_query("明月 清泉").unwrap();
        assert_eq!(query, "\"明月\" OR \"清泉\"");
        assert!(query.starts_with('"'));
        assert_eq!(build_match_query("，。！"), None);
    }

    #[test]
    fn single_token_query_has_no_or_test() {
        let query = build_match_query("春江花月夜").unwrap();
        assert!(query.starts_with('"') && query.ends_with('"'));
        assert!(!query.contains(" OR "), "单 token 不应拼接 OR: {query}");
    }

    #[test]
    fn quote_token_doubles_inner_quotes_test() {
        assert_eq!(quote_token("plain"), "\"plain\"");
        assert_eq!(quote_token("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_token("-"), "\"-\"");
    }

    #[test]
    fn chinese_keyword_hits_pre_tokenized_row_test() {
        let conn = conn();
        replace_chunk(&conn, &chunk("c1", "d1", "春江潮水连海平，海上明月共潮生。")).unwrap();
        replace_chunk(&conn, &chunk("c2", "d2", "大江东去，浪淘尽千古风流人物。")).unwrap();

        let hits = search(&conn, "明月", 5).unwrap();
        assert_eq!(hits.len(), 1, "应只命中含「明月」的块");
        assert_eq!(hits[0].chunk_id, "c1");
        assert!(hits[0].score > 0.0, "对外分数应为正（-bm25）");
    }

    #[test]
    fn replace_chunk_is_idempotent_and_updates_text_test() {
        let conn = conn();
        replace_chunk(&conn, &chunk("c1", "d1", "旧文本 甲乙")).unwrap();
        replace_chunk(&conn, &chunk("c1", "d1", "新文本 丙丁")).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM kb_fts WHERE chunk_id = 'c1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "覆盖写不得留下重复行");
        assert!(search(&conn, "丙丁", 5).unwrap().len() == 1);
        assert!(search(&conn, "甲乙", 5).unwrap().is_empty());
    }

    #[test]
    fn delete_document_removes_all_chunks_of_doc_test() {
        let conn = conn();
        replace_chunk(&conn, &chunk("c1", "d1", "明月松间照")).unwrap();
        replace_chunk(&conn, &chunk("c2", "d1", "清泉石上流")).unwrap();
        replace_chunk(&conn, &chunk("c3", "d2", "明月别枝惊鹊")).unwrap();

        assert_eq!(delete_document(&conn, "d1").unwrap(), 2);
        assert!(search(&conn, "松间照", 5).unwrap().is_empty());
        assert_eq!(search(&conn, "明月", 5).unwrap().len(), 1, "d2 不受影响");
    }

    #[test]
    fn search_with_no_keyword_returns_empty_test() {
        let conn = conn();
        replace_chunk(&conn, &chunk("c1", "d1", "正文")).unwrap();
        assert!(search(&conn, "，。！", 5).unwrap().is_empty());
    }

    #[test]
    fn search_respects_limit_test() {
        let conn = conn();
        for i in 0..5 {
            replace_chunk(&conn, &chunk(&format!("c{i}"), "d1", "明月 明月 明月")).unwrap();
        }
        assert_eq!(search(&conn, "明月", 3).unwrap().len(), 3);
    }

    #[test]
    fn clear_empties_index_test() {
        let conn = conn();
        replace_chunk(&conn, &chunk("c1", "d1", "正文")).unwrap();
        assert_eq!(clear(&conn).unwrap(), 1);
        assert!(search(&conn, "正文", 5).unwrap().is_empty());
    }
}
