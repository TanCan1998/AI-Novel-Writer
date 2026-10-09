//! 混合检索编排层（批次 F2-2）。
//!
//! # 两条路径
//!
//! | 模式 | 语义 | 状态 |
//! |---|---|---|
//! | [`SearchMode::Baseline`] | **忠实对齐基线** `electron/vector-store.ts::searchWithScope` 的「二选一短路」：向量可用且召回非空 → 纯向量结果；否则 → 文本检索 | **默认** |
//! | [`SearchMode::Hybrid`] | **RRF 倒数排名融合**（FTS + 向量），召回更全 | 可选开关，**默认关** |
//!
//! # 基线真实行为（本轮读源码确认，与方案 B 文档不一致）
//!
//! 基线**没有融合**：向量分支命中即 `return`，文本分支是**降级路径**（`LIKE '%term%'` 子串扫描
//! + 词命中度 `Σ (n - index)` 仅用于排序）。故方案 B 文档 §8.5 的「RRF 融合」属**自研增强**，
//! 且该文档 §8.7 又把「+300 行 RRF」列为放弃理由 —— 自相矛盾。经用户 2026-10-09 决定：
//! **默认走基线语义，RRF 作为可选开关**。
//!
//! # 已记录的刻意差异
//!
//! 1. 文本分支由基线的 `LIKE` 子串扫描改为 **FTS5 + jieba 预分词**（LanceDB 移除后的既定替代，
//!    见 2026-10-08 快照 §1「差异须逐项记录」）；
//! 2. 基线降级分支的 `score` **恒为 0.5**（`relevance` 只排序）；本层按用户决定**返回真实相关性分**
//!    （[`text_relevance`]），便于前端展示与对照。

use regex::Regex;
use rusqlite::{params, Connection};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// RRF 融合的秩常数（Cormack et al. 2009 标准取值）
pub const DEFAULT_RRF_K: usize = 60;

/// 基线的默认 `topK`
pub const DEFAULT_TOP_K: usize = 5;

/// 检索模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    /// 默认：忠实对齐基线（向量命中即短路，否则走文本）
    #[default]
    Baseline,
    /// 可选：RRF 融合两路召回
    Hybrid,
}

/// 基线短路后的实际检索来源
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetrievalSource {
    Vector,
    Text,
}

/// 基线的「二选一」判定
///
/// 只有「向量可用 **且** 召回非空」才短路为向量结果 —— 与基线 `if (results.length > 0) return ...` 一致；
/// 向量可用但零召回时基线会继续走文本分支。
pub fn baseline_source(vector_usable: bool, vector_hits: usize) -> RetrievalSource {
    if vector_usable && vector_hits > 0 {
        RetrievalSource::Vector
    } else {
        RetrievalSource::Text
    }
}

/// 向量距离 → 基线分数 `1 / (1 + distance)`（距离越小越接近 1）
pub fn vector_distance_to_score(distance: f64) -> f64 {
    if !distance.is_finite() {
        return 0.0;
    }
    1.0 / (1.0 + distance)
}

/// 基线查询词项提取：`/[\p{L}\p{N}-]+/gu`
///
/// 优先保留**长度 ≥ 2 码点**的词项（全部不足 2 时回退为原始集合）；
/// 按小写键去重（保留首次出现的大小写形态）；最多 **8** 个。
pub fn extract_query_terms(query: &str) -> Vec<String> {
    fn word_regex() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"[\p{L}\p{N}-]+").expect("查询词项正则应可编译"))
    }

    let raw: Vec<&str> = word_regex()
        .find_iter(query)
        .map(|found| found.as_str())
        .collect();
    if raw.is_empty() {
        return Vec::new();
    }

    let meaningful: Vec<&str> = raw
        .iter()
        .copied()
        .filter(|term| term.chars().count() >= 2)
        .collect();
    let source = if meaningful.is_empty() { raw } else { meaningful };

    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for term in source {
        if seen.insert(term.to_lowercase()) {
            out.push(term.to_string());
        }
        if out.len() >= 8 {
            break;
        }
    }
    out
}

/// 基线降级分支的词命中度：`Σ (n - index)`（仅对**出现为子串**的词项累加）
///
/// `terms` 必须已是小写（`extract_query_terms` 的大小写形态由调用方决定，内部统一忽略大小写匹配）。
pub fn text_relevance(text: &str, terms: &[String]) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let haystack = text.to_lowercase();
    let count = terms.len() as i64;
    terms
        .iter()
        .enumerate()
        .filter(|(_, term)| haystack.contains(term.to_lowercase().as_str()))
        .map(|(index, _)| (count - index as i64) as f64)
        .sum()
}

/// RRF 倒数排名融合
///
/// `ranked_lists` 每项为一路**已按相关性降序**的 id 列表；
/// 融合分 `score(id) = Σ_lists 1 / (k + rank + 1)`。返回按分数降序、同分按 id 升序（确定性）。
pub fn reciprocal_rank_fusion(ranked_lists: &[Vec<String>], k: usize) -> Vec<(String, f64)> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    for list in ranked_lists {
        for (index, id) in list.iter().enumerate() {
            *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k + index + 1) as f64;
        }
    }
    let mut fused: Vec<(String, f64)> = scores.into_iter().collect();
    fused.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    fused
}

/// 去重：基线按 `JSON.stringify([fileName, text])` 作键，保留**首次出现**（即更高分名次）
pub fn dedupe_by_file_and_text(items: &[(String, String)]) -> Vec<(String, String)> {
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut out: Vec<(String, String)> = Vec::new();
    for (file_name, text) in items {
        if seen.insert((file_name.clone(), text.clone())) {
            out.push((file_name.clone(), text.clone()));
        }
    }
    out
}

// ==================== 嵌入空间注册表（`kb_embedding_spaces`）====================

/// 一条嵌入空间记录（对齐基线 `EmbeddingSpace` 的代际模型）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingSpace {
    pub generation: i64,
    pub dimension: i64,
    pub model_fingerprint: String,
    pub distance_metric: String,
    pub status: String,
}

impl EmbeddingSpace {
    /// 是否为当前激活代际
    pub fn is_active(&self) -> bool {
        self.status == "active"
    }
}

const SPACE_COLUMNS: &str = "generation, dimension, model_fingerprint, distance_metric, status";

fn map_space(row: &rusqlite::Row<'_>) -> rusqlite::Result<EmbeddingSpace> {
    Ok(EmbeddingSpace {
        generation: row.get(0)?,
        dimension: row.get(1)?,
        model_fingerprint: row.get(2)?,
        distance_metric: row.get(3)?,
        status: row.get(4)?,
    })
}

/// 列出全部代际（按 generation 升序）
pub fn list_spaces(conn: &Connection) -> rusqlite::Result<Vec<EmbeddingSpace>> {
    let mut statement = conn.prepare(&format!(
        "SELECT {SPACE_COLUMNS} FROM kb_embedding_spaces ORDER BY generation"
    ))?;
    let rows = statement.query_map([], map_space)?;
    rows.collect()
}

/// 当前激活代际（`status = 'active'`，取 generation 最大者）
pub fn active_space(conn: &Connection) -> rusqlite::Result<Option<EmbeddingSpace>> {
    let mut statement = conn.prepare(&format!(
        "SELECT {SPACE_COLUMNS} FROM kb_embedding_spaces
         WHERE status = 'active' ORDER BY generation DESC LIMIT 1"
    ))?;
    let mut rows = statement.query_map([], map_space)?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// 下一个可用代际号（空表为 1）
pub fn next_generation(conn: &Connection) -> rusqlite::Result<i64> {
    let max: Option<i64> = conn.query_row(
        "SELECT MAX(generation) FROM kb_embedding_spaces",
        [],
        |row| row.get(0),
    )?;
    Ok(max.unwrap_or(0) + 1)
}

/// 写入 / 覆盖一条代际记录（按 generation 主键）
pub fn upsert_space(conn: &Connection, space: &EmbeddingSpace) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO kb_embedding_spaces
           (generation, dimension, model_fingerprint, distance_metric, status)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(generation) DO UPDATE SET
           dimension = excluded.dimension,
           model_fingerprint = excluded.model_fingerprint,
           distance_metric = excluded.distance_metric,
           status = excluded.status",
        params![
            space.generation,
            space.dimension,
            space.model_fingerprint,
            space.distance_metric,
            space.status
        ],
    )?;
    Ok(())
}

/// 激活指定代际：同一事务内把其余代际降为 `inactive`，目标置 `active`
///
/// 目标不存在时返回 `Ok(false)`（不产生任何写入），与基线「激活前先校验物理表存在」的保守语义对齐。
pub fn activate_generation(conn: &Connection, generation: i64) -> rusqlite::Result<bool> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM kb_embedding_spaces WHERE generation = ?1)",
        params![generation],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(false);
    }

    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> rusqlite::Result<()> {
        conn.execute(
            "UPDATE kb_embedding_spaces SET status = 'inactive'\n             WHERE generation <> ?1 AND status = 'active'",
            params![generation],
        )?;
        conn.execute(
            "UPDATE kb_embedding_spaces SET status = 'active' WHERE generation = ?1",
            params![generation],
        )?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(true)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

// ==================== 回填计划 ====================

/// 尚未生成向量的文本块 id（`kb_chunks` 全量 − 向量索引活节点）
///
/// 对齐基线 `getChunksWithoutVectors` 的用途（`kb:get-vectorless-count` / `kb:backfill-vectors`）。
pub fn vectorless_chunk_ids(
    conn: &Connection,
    live_ids: &HashSet<String>,
) -> rusqlite::Result<Vec<String>> {
    let mut statement =
        conn.prepare("SELECT id FROM kb_chunks ORDER BY doc_id, chunk_index, id")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        let chunk_id = row?;
        if !live_ids.contains(&chunk_id) {
            out.push(chunk_id);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::{CREATE_KB_CHUNKS, CREATE_KB_DOCUMENTS, CREATE_KB_EMBEDDING_SPACES};

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(CREATE_KB_DOCUMENTS).unwrap();
        conn.execute_batch(CREATE_KB_CHUNKS).unwrap();
        conn.execute_batch(CREATE_KB_EMBEDDING_SPACES).unwrap();
        conn
    }

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn baseline_source_short_circuits_only_on_non_empty_vector_hits_test() {
        assert_eq!(baseline_source(true, 3), RetrievalSource::Vector);
        // 向量可用但零召回 → 基线会继续走文本分支
        assert_eq!(baseline_source(true, 0), RetrievalSource::Text);
        assert_eq!(baseline_source(false, 3), RetrievalSource::Text);
        assert_eq!(baseline_source(false, 0), RetrievalSource::Text);
    }

    #[test]
    fn vector_distance_to_score_matches_baseline_formula_test() {
        assert!((vector_distance_to_score(0.0) - 1.0).abs() < 1e-12);
        assert!((vector_distance_to_score(1.0) - 0.5).abs() < 1e-12);
        assert!((vector_distance_to_score(3.0) - 0.25).abs() < 1e-12);
        assert_eq!(vector_distance_to_score(f64::NAN), 0.0);
    }

    #[test]
    fn extract_query_terms_prefers_two_codepoint_terms_and_dedupes_test() {
        let terms = extract_query_terms("明月 明月 ab 江 x");
        assert_eq!(terms, vec!["明月".to_string(), "ab".to_string()]);
    }

    #[test]
    fn extract_query_terms_falls_back_when_all_terms_are_short_test() {
        let terms = extract_query_terms("a b");
        assert_eq!(terms, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn extract_query_terms_caps_at_eight_test() {
        let query = "a1 a2 a3 a4 a5 a6 a7 a8 a9 a10";
        assert_eq!(extract_query_terms(query).len(), 8);
    }

    #[test]
    fn text_relevance_weights_earlier_terms_higher_test() {
        let terms = extract_query_terms("明月 清泉");
        // 「明月」index 0 → +2；「清泉」index 1 → +1
        assert!((text_relevance("明月松间照，清泉石上流", &terms) - 3.0).abs() < 1e-12);
        assert!((text_relevance("明月松间照", &terms) - 2.0).abs() < 1e-12);
        assert!((text_relevance("大江东去", &terms)).abs() < 1e-12);
    }

    #[test]
    fn text_relevance_is_case_insensitive_test() {
        let terms = extract_query_terms("Vela");
        assert!((text_relevance("the VELA project", &terms) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn rrf_ranks_documents_found_by_both_lists_first_test() {
        let fused = reciprocal_rank_fusion(&[ids(&["a", "b"]), ids(&["b", "c"])], DEFAULT_RRF_K);
        assert_eq!(fused[0].0, "b", "两路都命中者应排第一");
        assert!(fused[0].1 > fused[1].1);
        assert_eq!(fused.len(), 3);
    }

    #[test]
    fn rrf_is_deterministic_on_ties_test() {
        let fused = reciprocal_rank_fusion(&[ids(&["b", "a"])], DEFAULT_RRF_K);
        // 反向输入应得到反向结果（名次决定分数）
        assert_eq!(fused[0].0, "b");
        let tied = reciprocal_rank_fusion(&[ids(&["z"]), ids(&["a"])], DEFAULT_RRF_K);
        assert_eq!(tied[0].0, "a", "同分时按 id 升序保证确定性");
    }

    #[test]
    fn dedupe_keeps_first_occurrence_test() {
        let items = vec![
            ("a.txt".to_string(), "正文".to_string()),
            ("a.txt".to_string(), "正文".to_string()),
            ("b.txt".to_string(), "正文".to_string()),
        ];
        assert_eq!(dedupe_by_file_and_text(&items).len(), 2);
    }

    #[test]
    fn space_registry_roundtrip_and_activation_test() {
        let conn = conn();
        assert_eq!(next_generation(&conn).unwrap(), 1);

        upsert_space(
            &conn,
            &EmbeddingSpace {
                generation: 1,
                dimension: 1536,
                model_fingerprint: "legacy:unknown".into(),
                distance_metric: "cosine".into(),
                status: "active".into(),
            },
        )
        .unwrap();
        upsert_space(
            &conn,
            &EmbeddingSpace {
                generation: 2,
                dimension: 1024,
                model_fingerprint: "fp-2".into(),
                distance_metric: "cosine".into(),
                status: "building".into(),
            },
        )
        .unwrap();

        assert_eq!(next_generation(&conn).unwrap(), 3);
        assert_eq!(list_spaces(&conn).unwrap().len(), 2);
        assert_eq!(active_space(&conn).unwrap().unwrap().generation, 1);

        activate_generation(&conn, 2).unwrap();
        let spaces = list_spaces(&conn).unwrap();
        assert_eq!(spaces.iter().filter(|s| s.is_active()).count(), 1, "只能有一个激活代际");
        assert_eq!(active_space(&conn).unwrap().unwrap().generation, 2);
        assert_eq!(spaces[0].status, "inactive");
    }

    #[test]
    fn activating_unknown_generation_writes_nothing_test() {
        let conn = conn();
        upsert_space(
            &conn,
            &EmbeddingSpace {
                generation: 1,
                dimension: 8,
                model_fingerprint: String::new(),
                distance_metric: "cosine".into(),
                status: "active".into(),
            },
        )
        .unwrap();
        assert!(!activate_generation(&conn, 99).unwrap());
        assert_eq!(active_space(&conn).unwrap().unwrap().generation, 1);
    }

    #[test]
    fn upsert_space_overwrites_same_generation_test() {
        let conn = conn();
        let mut space = EmbeddingSpace {
            generation: 1,
            dimension: 8,
            model_fingerprint: "a".into(),
            distance_metric: "cosine".into(),
            status: "building".into(),
        };
        upsert_space(&conn, &space).unwrap();
        space.status = "active".into();
        space.dimension = 16;
        upsert_space(&conn, &space).unwrap();

        let spaces = list_spaces(&conn).unwrap();
        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].dimension, 16);
        assert!(spaces[0].is_active());
    }

    #[test]
    fn vectorless_chunk_ids_is_set_difference_test() {
        let conn = conn();
        conn.execute(
            "INSERT INTO kb_documents (id, file_name, corpus_kind, imported_at, chunk_count)
             VALUES ('d1', 'a.txt', 'project-knowledge', '2026-10-09T00:00:00.000Z', 3)",
            [],
        )
        .unwrap();
        for (id, index) in [("c1", 0), ("c2", 1), ("c3", 2)] {
            conn.execute(
                "INSERT INTO kb_chunks
                   (id, doc_id, file_name, text, chunk_index, total_chunks, imported_at, corpus_kind)
                 VALUES (?1, 'd1', 'a.txt', '正文', ?2, 3, '2026-10-09T00:00:00.000Z', 'project-knowledge')",
                params![id, index],
            )
            .unwrap();
        }

        let live: HashSet<String> = ids(&["c1"]).into_iter().collect();
        assert_eq!(vectorless_chunk_ids(&conn, &live).unwrap(), vec!["c2", "c3"]);
        // 全覆盖时应为空
        let all: HashSet<String> = ids(&["c1", "c2", "c3"]).into_iter().collect();
        assert!(vectorless_chunk_ids(&conn, &all).unwrap().is_empty());
    }
}
