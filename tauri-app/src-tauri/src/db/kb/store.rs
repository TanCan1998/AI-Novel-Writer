//! 知识库存储与文本检索编排层（批次 F2-3）。
//!
//! 本层只做 **SQLite（`kb_documents` / `kb_chunks` / `kb_fts`）** 的读写与**文本支路**
//! 检索编排，不触碰向量索引（向量支路由 `commands/kb.rs` 协调
//! [`crate::db::vector::LocalVectorIndex`] 与 [`super::hybrid`]）。
//!
//! # 与基线的对照
//!
//! | 基线（`electron/knowledge-base.ts` + `vector-store.ts`） | 本层 |
//! |---|---|
//! | `addChunks`（写 canonical 表 + 文档表 + FTS 索引） | [`insert_document`] |
//! | `removeDocument` | [`remove_document`] |
//! | `clearAll` | [`clear_all`] |
//! | `listDocuments` | [`list_documents`] |
//! | `getStats` | [`stats`] |
//! | `searchWithScope` 的文本降级支路 | [`search_text`] |
//! | `hashCanonicalChunkSet` / `getDocumentIntegrity` | [`hash_canonical_chunk_set`] / [`document_integrity`] |
//! | `removeDocument`（stable-id 语义） | [`remove_document_stable_id`] / [`replace_document`] |
//!
//! # 已记录的刻意差异
//!
//! 基线文本支路是 LanceDB 上的 `LIKE` 子串扫描；本层按既定替代方案改为
//! **FTS5（jieba 预分词）召回 + 真实相关性分**（见 `hybrid.rs` 模块文档），
//! 排序键仍为基线的词命中度 `Σ(n - index)`。

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashSet;

use super::{fts, hybrid};

/// 文档元信息（对齐基线 `DocumentInfo`；`corpusKind` 为 Tauri 侧扩展语义）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInfo {
    pub id: String,
    pub file_name: String,
    pub imported_at: String,
    pub chunk_count: i64,
    pub file_path: String,
    pub corpus_kind: String,
}

/// 知识库统计（对齐基线 `KBStats` 的对外三列）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KbStats {
    pub document_count: i64,
    pub total_chunks: i64,
    pub vector_dimension: i64,
}

/// 检索结果（对齐契约 `{ text, score, fileName }`）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub text: String,
    pub score: f64,
    pub file_name: String,
}

/// 一条文本块（检索/回填共用）
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkRow {
    pub id: String,
    pub doc_id: String,
    pub file_name: String,
    pub text: String,
    pub chapter_number: Option<i64>,
    pub corpus_kind: String,
}

/// 写入分块时携带的元数据
#[derive(Debug, Clone, Default)]
pub struct DocumentMeta<'a> {
    pub file_path: &'a str,
    pub chapter_number: Option<i64>,
    pub chapter_title: Option<&'a str>,
    pub corpus_kind: &'a str,
}

/// 检索过滤条件（章节范围 + 排除语料类型）
#[derive(Debug, Clone, Default)]
pub struct SearchFilter<'a> {
    pub chapter_scope: Option<(i64, i64)>,
    pub excluded_corpus_kinds: &'a [String],
}

// ==================== 时间戳 ====================

/// ISO8601 UTC 时间戳（毫秒精度），对齐基线 `new Date().toISOString()`
pub fn now_iso() -> String {
    crate::commands::iso8601_utc_from_millis(crate::commands::epoch_millis_now())
}

// ==================== 写入 ====================

/// 写入文档 + 分块 + FTS 索引（单事务）。返回新建的块 id 列表。
///
/// 与基线 `addChunks` 的差异：向量不在本层写入；调用方在成功提交后再写向量索引。
pub fn insert_document(
    conn: &Connection,
    doc_id: &str,
    file_name: &str,
    chunks: &[String],
    meta: &DocumentMeta<'_>,
) -> Result<Vec<String>, String> {
    if chunks.is_empty() {
        return Err("文本块为空或格式无效".to_string());
    }
    let now = now_iso();
    let corpus_kind = if meta.corpus_kind.is_empty() {
        "unknown"
    } else {
        meta.corpus_kind
    };

    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| error.to_string())?;
    let result = (|| -> Result<Vec<String>, String> {
        conn.execute(
            "INSERT INTO kb_documents
               (id, file_name, file_path, corpus_kind, imported_at, chunk_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                doc_id,
                file_name,
                meta.file_path,
                corpus_kind,
                now,
                chunks.len() as i64
            ],
        )
        .map_err(|error| error.to_string())?;

        let total = chunks.len() as i64;
        let mut ids = Vec::with_capacity(chunks.len());
        for (index, text) in chunks.iter().enumerate() {
            let chunk_id = crate::project_access::random_uuid_v4();
            conn.execute(
                "INSERT INTO kb_chunks
                   (id, doc_id, file_name, chapter_number, chapter_title, text,
                    chunk_index, total_chunks, imported_at, corpus_kind)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    chunk_id,
                    doc_id,
                    file_name,
                    meta.chapter_number,
                    meta.chapter_title,
                    text,
                    index as i64,
                    total,
                    now,
                    corpus_kind
                ],
            )
            .map_err(|error| error.to_string())?;
            fts::replace_chunk(
                conn,
                &fts::FtsChunk {
                    chunk_id: &chunk_id,
                    doc_id,
                    file_name,
                    corpus_kind,
                    text,
                },
            )
            .map_err(|error| error.to_string())?;
            ids.push(chunk_id);
        }
        Ok(ids)
    })();

    match result {
        Ok(ids) => {
            conn.execute_batch("COMMIT")
                .map_err(|error| error.to_string())?;
            Ok(ids)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

// ==================== 读取 ====================

/// 列出全部文档（按导入时间升序，对齐基线数组顺序）
pub fn list_documents(conn: &Connection) -> Result<Vec<DocumentInfo>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, file_name, imported_at, chunk_count, file_path, corpus_kind
             FROM kb_documents ORDER BY imported_at, file_name",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(DocumentInfo {
                id: row.get(0)?,
                file_name: row.get(1)?,
                imported_at: row.get(2)?,
                chunk_count: row.get(3)?,
                file_path: row.get(4)?,
                corpus_kind: row.get(5)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

/// 按同名文件查找文档 id（用于导入同名文档时的替换语义）
pub fn find_document_id_by_file_name(
    conn: &Connection,
    file_name: &str,
) -> Result<Option<String>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id FROM kb_documents WHERE file_name = ?1 ORDER BY imported_at DESC LIMIT 1",
        )
        .map_err(|error| error.to_string())?;
    let mut rows = statement
        .query_map([file_name], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    match rows.next() {
        Some(row) => row.map(Some).map_err(|error| error.to_string()),
        None => Ok(None),
    }
}

/// 删除文档及其分块与 FTS 行。返回是否命中（幂等）。
pub fn remove_document(conn: &Connection, doc_id: &str) -> Result<bool, String> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| error.to_string())?;
    let result = (|| -> Result<bool, String> {
        fts::delete_document(conn, doc_id).map_err(|error| error.to_string())?;
        conn.execute("DELETE FROM kb_chunks WHERE doc_id = ?1", [doc_id])
            .map_err(|error| error.to_string())?;
        let removed = conn
            .execute("DELETE FROM kb_documents WHERE id = ?1", [doc_id])
            .map_err(|error| error.to_string())?;
        Ok(removed > 0)
    })();
    match result {
        Ok(hit) => {
            conn.execute_batch("COMMIT")
                .map_err(|error| error.to_string())?;
            Ok(hit)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// 清空整张知识库（文档 / 分块 / FTS / 嵌入空间注册表）
pub fn clear_all(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| error.to_string())?;
    let result = (|| -> Result<(), String> {
        fts::clear(conn).map_err(|error| error.to_string())?;
        conn.execute("DELETE FROM kb_chunks", [])
            .map_err(|error| error.to_string())?;
        conn.execute("DELETE FROM kb_documents", [])
            .map_err(|error| error.to_string())?;
        conn.execute("DELETE FROM kb_embedding_spaces", [])
            .map_err(|error| error.to_string())?;
        Ok(())
    })();
    match result {
        Ok(()) => conn
            .execute_batch("COMMIT")
            .map_err(|error| error.to_string()),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// 知识库统计（`vectorDimension` 取当前 active 嵌入代际）
pub fn stats(conn: &Connection) -> Result<KbStats, String> {
    let document_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM kb_documents", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let total_chunks: i64 = conn
        .query_row("SELECT COUNT(*) FROM kb_chunks", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let vector_dimension = hybrid::active_space(conn)
        .map_err(|error| error.to_string())?
        .map(|space| space.dimension)
        .unwrap_or(0);
    Ok(KbStats {
        document_count,
        total_chunks,
        vector_dimension,
    })
}

/// 某文档全部分块 id（删除文档时同步清理向量索引）
pub fn document_chunk_ids(conn: &Connection, doc_id: &str) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare("SELECT id FROM kb_chunks WHERE doc_id = ?1 ORDER BY chunk_index, id")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([doc_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

/// 全部块 id（用于 vectorless 计算）
pub fn all_chunk_ids(conn: &Connection) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare("SELECT id FROM kb_chunks ORDER BY doc_id, chunk_index, id")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

/// 按 id 批量读取块正文（与输入顺序无关，返回命中项）
pub fn chunk_rows(conn: &Connection, ids: &[String]) -> Result<Vec<ChunkRow>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, doc_id, file_name, text, chapter_number, corpus_kind
             FROM kb_chunks WHERE id = ?1",
        )
        .map_err(|error| error.to_string())?;
    let mut out = Vec::new();
    for id in ids {
        let row = statement
            .query_row([id], |row| {
                Ok(ChunkRow {
                    id: row.get(0)?,
                    doc_id: row.get(1)?,
                    file_name: row.get(2)?,
                    text: row.get(3)?,
                    chapter_number: row.get(4)?,
                    corpus_kind: row.get(5)?,
                })
            })
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other.to_string()),
            })?;
        if let Some(row) = row {
            out.push(row);
        }
    }
    Ok(out)
}

/// 过滤判定：章节范围（NULL 不落入任何范围，对齐基线过滤语义）+ 排除语料类型
pub fn matches_filter(row: &ChunkRow, filter: &SearchFilter<'_>) -> bool {
    if let Some((from, to)) = filter.chapter_scope {
        match row.chapter_number {
            Some(number) if number >= from && number <= to => {}
            _ => return false,
        }
    }
    if filter
        .excluded_corpus_kinds
        .iter()
        .any(|kind| kind == &row.corpus_kind)
    {
        return false;
    }
    true
}

/// 文本支路检索：FTS5 预分词召回 → 过滤 → 词命中度排序 → 按 (fileName, text) 去重 → topK。
///
/// `score` 为基线的词命中度 `Σ(n - index)`（用户 2026-10-09 决定：以真实相关性分
/// 取代基线的恒 0.5）。FTS5 自身分数仅作同分兜底。
pub fn search_text(
    conn: &Connection,
    query: &str,
    top_k: usize,
    filter: &SearchFilter<'_>,
) -> Result<Vec<SearchResult>, String> {
    if top_k == 0 {
        return Ok(Vec::new());
    }
    let terms = hybrid::extract_query_terms(query);
    // FTS5 OR 召回上限：足以覆盖后续按相关性重排的候选集。
    let recall_limit = std::cmp::max(top_k.saturating_mul(20), 200);
    let hits = fts::search(conn, query, recall_limit).map_err(|error| error.to_string())?;
    if hits.is_empty() {
        return Ok(Vec::new());
    }

    let ids: Vec<String> = hits.iter().map(|hit| hit.chunk_id.clone()).collect();
    let rows = chunk_rows(conn, &ids)?;
    let by_id: std::collections::HashMap<&str, &ChunkRow> =
        rows.iter().map(|row| (row.id.as_str(), row)).collect();

    let mut candidates: Vec<(f64, f64, &ChunkRow)> = Vec::new();
    for hit in &hits {
        let Some(row) = by_id.get(hit.chunk_id.as_str()) else {
            continue;
        };
        if !matches_filter(row, filter) {
            continue;
        }
        let relevance = hybrid::text_relevance(&row.text, &terms);
        candidates.push((relevance, hit.score, row));
    }
    candidates.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                right
                    .1
                    .partial_cmp(&left.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.2.id.cmp(&right.2.id))
    });

    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut out = Vec::new();
    for (relevance, _, row) in candidates {
        if !seen.insert((row.file_name.clone(), row.text.clone())) {
            continue;
        }
        out.push(SearchResult {
            text: row.text.clone(),
            score: relevance,
            file_name: row.file_name.clone(),
        });
        if out.len() >= top_k {
            break;
        }
    }
    Ok(out)
}

// ==================== 参照文档幂等（批次 G4） ====================

/// 参照导入的规范化块集哈希（对齐基线 `hashCanonicalChunkSet`：
/// `sha256(JSON.stringify(chunks))`）。
///
/// `serde_json` 对字符串数组的编码与基线 `JSON.stringify` 逐字节一致
/// （短控制字符转义、非 ASCII 原样输出、`/` 不转义），故哈希输入与
/// 基线相同——**不得改变输入顺序或分隔符**。
pub fn hash_canonical_chunk_set(chunks: &[String]) -> String {
    let canonical = serde_json::to_string(chunks).expect("字符串数组的 JSON 序列化不可失败");
    crate::import::parsing::sha256_hex(&canonical)
}

/// 参照文档幂等收据（对齐基线 `import_reference_documents` 的 7 列业务
/// 投影；`created_at` / `updated_at` 由 SQLite 列默认值维护，见
/// `db/schema.rs` 的建表语句——只读，不在此层修改）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceDocumentReceipt {
    pub document_id: String,
    pub idempotency_key_hash: String,
    pub content_hash: String,
    pub chunk_set_hash: String,
    pub expected_chunk_count: i64,
    pub corpus_kind: String,
    pub state: String,
}

/// 按 `document_id` 或幂等键哈希读取参照文档收据（对齐基线
/// `SELECT ... FROM import_reference_documents
///        WHERE document_id = ? OR idempotency_key_hash = ?`）。
pub fn read_reference_document(
    conn: &Connection,
    document_id: &str,
    idempotency_key_hash: &str,
) -> Result<Option<ReferenceDocumentReceipt>, String> {
    conn.query_row(
        "SELECT document_id, idempotency_key_hash, content_hash, chunk_set_hash,
                expected_chunk_count, corpus_kind, state
         FROM import_reference_documents
         WHERE document_id = ?1 OR idempotency_key_hash = ?2",
        params![document_id, idempotency_key_hash],
        |row| {
            Ok(ReferenceDocumentReceipt {
                document_id: row.get(0)?,
                idempotency_key_hash: row.get(1)?,
                content_hash: row.get(2)?,
                chunk_set_hash: row.get(3)?,
                expected_chunk_count: row.get(4)?,
                corpus_kind: row.get(5)?,
                state: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())
}

/// 写入参照文档幂等收据。对齐基线「不存在则插 `state='prepared'`」：
/// 仅当 `document_id`（主键）与 `idempotency_key_hash`（唯一键）均无
/// 冲突时插入；任一冲突即保持既有行不动（基线仅在无收据时插入，
/// 后续状态推进走 [`mark_reference_document_state`]）。
pub fn upsert_reference_document(
    conn: &Connection,
    receipt: &ReferenceDocumentReceipt,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO import_reference_documents (
           document_id, idempotency_key_hash, content_hash, chunk_set_hash,
           expected_chunk_count, corpus_kind, state
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT DO NOTHING",
        params![
            receipt.document_id,
            receipt.idempotency_key_hash,
            receipt.content_hash,
            receipt.chunk_set_hash,
            receipt.expected_chunk_count,
            receipt.corpus_kind,
            receipt.state,
        ],
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

/// 推进参照文档状态机（对齐基线
/// `UPDATE import_reference_documents
///        SET state = ?, updated_at = datetime('now') WHERE document_id = ?`）。
pub fn mark_reference_document_state(
    conn: &Connection,
    document_id: &str,
    state: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE import_reference_documents
         SET state = ?1, updated_at = datetime('now')
         WHERE document_id = ?2",
        params![state, document_id],
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

/// 文档完整性（对齐基线 `getDocumentIntegrity` 的 DB 侧判定口径）。
///
/// ⚠️ 架构差异：基线的 `complete` 还要求各嵌入代际（`embeddingGenerations`）
/// 完整；Tauri 侧向量由文件型 [`crate::db::vector::LocalVectorIndex`] 持有、
/// 不入 SQLite，故 `complete` 只反映 **canonical 完整性**（文档行唯一 +
/// 块序列严格等于 `0..n` 且每块 `total_chunks` / `corpus_kind` 自洽）。
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentIntegrity {
    /// 文档行与全部 canonical 块是否构成完整提交（基线口径，剔除向量代际）
    pub complete: bool,
    /// 语料类型：文档行优先，其次首块，最后 `'unknown'`（对齐基线）
    pub corpus_kind: String,
    /// 块数量（按 `chunk_index` 排序后的块集大小）
    pub chunk_count: i64,
    /// 按 `chunk_index` 排序后的块集哈希（[`hash_canonical_chunk_set`]）
    pub chunk_set_hash: String,
}

/// 校验文档提交行与全部 canonical 块的完整性（对齐基线
/// `getDocumentIntegrity`：文档与块均不存在时返回 `None`）。
pub fn document_integrity(
    conn: &Connection,
    doc_id: &str,
) -> Result<Option<DocumentIntegrity>, String> {
    /// 块投影（按 `chunk_index` 排序；同 index 时按 `id` 保证确定性次序）
    struct ChunkProjection {
        id: String,
        chunk_index: i64,
        total_chunks: i64,
        text: String,
        corpus_kind: String,
    }

    let mut statement = conn
        .prepare(
            "SELECT id, chunk_index, total_chunks, text, corpus_kind
             FROM kb_chunks WHERE doc_id = ?1
             ORDER BY chunk_index, id",
        )
        .map_err(|error| error.to_string())?;
    let ordered: Vec<ChunkProjection> = statement
        .query_map([doc_id], |row| {
            Ok(ChunkProjection {
                id: row.get(0)?,
                chunk_index: row.get(1)?,
                total_chunks: row.get(2)?,
                text: row.get(3)?,
                corpus_kind: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;

    let document = conn
        .query_row(
            "SELECT chunk_count, corpus_kind FROM kb_documents WHERE id = ?1",
            [doc_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    // 基线：文档行与块行均无记录（且各嵌入代际无块）时返回 null。
    if document.is_none() && ordered.is_empty() {
        return Ok(None);
    }

    let corpus_kind = document
        .as_ref()
        .map(|(_, kind)| kind.clone())
        .or_else(|| ordered.first().map(|row| row.corpus_kind.clone()))
        .unwrap_or_else(|| "unknown".to_string());
    let chunk_count = ordered.len() as i64;
    let mut seen_ids: HashSet<String> = HashSet::new();
    let canonical_complete = document.is_some()
        && chunk_count > 0
        && document.is_some_and(|(count, _)| count == chunk_count)
        && ordered.iter().all(|row| seen_ids.insert(row.id.clone()))
        && ordered.iter().enumerate().all(|(index, row)| {
            row.chunk_index == index as i64
                && row.total_chunks == chunk_count
                && row.corpus_kind == corpus_kind
        });
    let texts: Vec<String> = ordered.iter().map(|row| row.text.clone()).collect();
    Ok(Some(DocumentIntegrity {
        complete: canonical_complete,
        corpus_kind,
        chunk_count,
        chunk_set_hash: hash_canonical_chunk_set(&texts),
    }))
}

/// 删除参照文档（stable-id 语义）——对齐基线 `removeDocFromStore`
/// （`vector-store.ts::removeDocument`）的**数据库侧语义**：删
/// `kb_documents` / `kb_chunks` / FTS 索引行，并验证零行后条件。
///
/// ⚠️ 向量**不在 SQLite**：Tauri 侧向量由文件型
/// [`crate::db::vector::LocalVectorIndex`]（`<project>/.lore/kb/` 下的
/// HNSW 图快照 + ID 映射 sidecar）持有，**物理索引由上层清理**
/// （命令层在 DB 清理成功后另行调用 `LocalVectorIndex::delete_vector`）。
///
/// 返回 `Ok(true)` 表示清理完成（含「原本不存在」的幂等路径，对齐基线
/// 零行后条件验证语义）；`Ok(false)` 表示删除后仍有残留（事务内理论上
/// 不会发生，保留该返回值以对齐基线「无法安全清理」的失败形状）。
pub fn remove_document_stable_id(conn: &Connection, doc_id: &str) -> Result<bool, String> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| error.to_string())?;
    let result = (|| -> Result<bool, String> {
        fts::delete_document(conn, doc_id).map_err(|error| error.to_string())?;
        conn.execute("DELETE FROM kb_chunks WHERE doc_id = ?1", [doc_id])
            .map_err(|error| error.to_string())?;
        conn.execute("DELETE FROM kb_documents WHERE id = ?1", [doc_id])
            .map_err(|error| error.to_string())?;
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kb_chunks WHERE doc_id = ?1",
                [doc_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        Ok(remaining == 0)
    })();
    match result {
        Ok(clean) => {
            conn.execute_batch("COMMIT")
                .map_err(|error| error.to_string())?;
            Ok(clean)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// 按 stable-id 重写文档（对齐基线 `replacementMode: 'stable-id'`）：
/// 先 [`remove_document_stable_id`]（存在才删，幂等），再复用
/// [`insert_document`] 写入新块，返回新块 id 列表。
///
/// 与基线的差异：基线由 `performReferenceTextImport` 先
/// `removeDocFromStore` 再 `addChunks`；本函数把「先删后插」收口为
/// 单点，供参照导入幂等路径使用。
pub fn replace_document(
    conn: &Connection,
    doc_id: &str,
    file_name: &str,
    chunks: &[String],
    meta: &DocumentMeta<'_>,
) -> Result<Vec<String>, String> {
    // 「存在才删」守卫对齐基线 `if (existingIntegrity && ...)`：
    // 文档行或任一 canonical 块存在即视为已存在（含残缺态）。
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM kb_documents WHERE id = ?1)
                     OR EXISTS(SELECT 1 FROM kb_chunks WHERE doc_id = ?1)",
            [doc_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if exists && !remove_document_stable_id(conn, doc_id)? {
        return Err("残缺参照文档无法安全清理".to_string());
    }
    insert_document(conn, doc_id, file_name, chunks, meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::{
        CREATE_IMPORT_RUN, CREATE_KB_CHUNKS, CREATE_KB_DOCUMENTS, CREATE_KB_EMBEDDING_SPACES,
        CREATE_KB_FTS,
    };

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(CREATE_KB_DOCUMENTS).unwrap();
        conn.execute_batch(CREATE_KB_CHUNKS).unwrap();
        conn.execute_batch(CREATE_KB_EMBEDDING_SPACES).unwrap();
        conn.execute_batch(CREATE_KB_FTS).unwrap();
        // 批次 G4：参照文档幂等收据（import_reference_documents）
        conn.execute_batch(CREATE_IMPORT_RUN).unwrap();
        conn
    }

    fn sample_meta() -> DocumentMeta<'static> {
        DocumentMeta {
            file_path: "",
            chapter_number: None,
            chapter_title: None,
            corpus_kind: "project-knowledge",
        }
    }

    #[test]
    fn insert_list_and_stats_roundtrip_test() {
        let conn = conn();
        let ids = insert_document(
            &conn,
            "d1",
            "甲.txt",
            &["春江潮水连海平".to_string(), "海上明月共潮生".to_string()],
            &sample_meta(),
        )
        .unwrap();
        assert_eq!(ids.len(), 2);

        let docs = list_documents(&conn).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].chunk_count, 2);
        assert_eq!(docs[0].corpus_kind, "project-knowledge");

        let stats = stats(&conn).unwrap();
        assert_eq!(stats.document_count, 1);
        assert_eq!(stats.total_chunks, 2);
        assert_eq!(stats.vector_dimension, 0);
    }

    #[test]
    fn insert_rejects_empty_chunks_test() {
        let conn = conn();
        assert!(insert_document(&conn, "d1", "a.txt", &[], &sample_meta()).is_err());
    }

    #[test]
    fn remove_document_cascades_chunks_and_fts_test() {
        let conn = conn();
        insert_document(
            &conn,
            "d1",
            "甲.txt",
            &["明月松间照".to_string()],
            &sample_meta(),
        )
        .unwrap();
        assert!(remove_document(&conn, "d1").unwrap());
        assert!(!remove_document(&conn, "d1").unwrap(), "重复删除为未命中");
        assert!(stats(&conn).unwrap().total_chunks == 0);
        assert!(search_text(&conn, "明月", 5, &SearchFilter::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn search_text_reranks_by_relevance_and_dedupes_test() {
        let conn = conn();
        insert_document(&conn, "d1", "甲.txt", &["明月".to_string()], &sample_meta()).unwrap();
        insert_document(
            &conn,
            "d2",
            "乙.txt",
            &["明月松间照，清泉石上流".to_string()],
            &sample_meta(),
        )
        .unwrap();
        let results = search_text(&conn, "明月 清泉", 5, &SearchFilter::default()).unwrap();
        assert_eq!(results.len(), 2);
        // 命中的词项更多（含清泉）应排前
        assert_eq!(results[0].file_name, "乙.txt");
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn search_text_respects_chapter_scope_test() {
        let conn = conn();
        insert_document(
            &conn,
            "d1",
            "第一章.md",
            &["明月".to_string()],
            &DocumentMeta {
                chapter_number: Some(1),
                corpus_kind: "reference",
                ..sample_meta()
            },
        )
        .unwrap();
        insert_document(
            &conn,
            "d2",
            "第九章.md",
            &["明月".to_string()],
            &DocumentMeta {
                chapter_number: Some(9),
                corpus_kind: "reference",
                ..sample_meta()
            },
        )
        .unwrap();

        let filter = SearchFilter {
            chapter_scope: Some((1, 3)),
            excluded_corpus_kinds: &[],
        };
        let results = search_text(&conn, "明月", 5, &filter).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].file_name, "第一章.md");
    }

    #[test]
    fn search_text_excludes_corpus_kinds_test() {
        let conn = conn();
        insert_document(
            &conn,
            "d1",
            "项目素材.md",
            &["明月".to_string()],
            &DocumentMeta {
                corpus_kind: "project-knowledge",
                ..sample_meta()
            },
        )
        .unwrap();
        insert_document(
            &conn,
            "d2",
            "参照.md",
            &["明月".to_string()],
            &DocumentMeta {
                corpus_kind: "reference",
                ..sample_meta()
            },
        )
        .unwrap();

        let excluded = vec!["project-knowledge".to_string()];
        let filter = SearchFilter {
            chapter_scope: None,
            excluded_corpus_kinds: &excluded,
        };
        let results = search_text(&conn, "明月", 5, &filter).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].file_name, "参照.md");
    }

    #[test]
    fn find_document_id_by_file_name_prefers_latest_test() {
        let conn = conn();
        insert_document(&conn, "d1", "同名.txt", &["旧".to_string()], &sample_meta()).unwrap();
        insert_document(&conn, "d2", "同名.txt", &["新".to_string()], &sample_meta()).unwrap();
        assert_eq!(
            find_document_id_by_file_name(&conn, "同名.txt")
                .unwrap()
                .as_deref(),
            Some("d2")
        );
        assert!(find_document_id_by_file_name(&conn, "不存在.txt")
            .unwrap()
            .is_none());
    }

    #[test]
    fn clear_all_empties_everything_test() {
        let conn = conn();
        insert_document(&conn, "d1", "甲.txt", &["正文".to_string()], &sample_meta()).unwrap();
        clear_all(&conn).unwrap();
        let stats = stats(&conn).unwrap();
        assert_eq!(stats.document_count, 0);
        assert_eq!(stats.total_chunks, 0);
        assert!(all_chunk_ids(&conn).unwrap().is_empty());
    }

    fn reference_meta() -> DocumentMeta<'static> {
        DocumentMeta {
            corpus_kind: "reference",
            ..sample_meta()
        }
    }

    #[test]
    fn hash_canonical_chunk_set_is_order_sensitive_and_stable_test() {
        let chunks = vec!["春江潮水".to_string(), "连海平".to_string()];
        // 相同输入稳定
        assert_eq!(
            hash_canonical_chunk_set(&chunks),
            hash_canonical_chunk_set(&chunks)
        );
        // 块序敏感
        let mut reversed = chunks.clone();
        reversed.reverse();
        assert_ne!(
            hash_canonical_chunk_set(&chunks),
            hash_canonical_chunk_set(&reversed)
        );
        // 逐字节对齐基线：sha256(JSON.stringify(chunks))
        let baseline_json = "[\"春江潮水\",\"连海平\"]";
        assert_eq!(
            hash_canonical_chunk_set(&chunks),
            crate::import::parsing::sha256_hex(baseline_json)
        );
    }

    #[test]
    fn document_integrity_reports_complete_partial_and_mismatched_states_test() {
        let chunks = vec!["甲".to_string(), "乙".to_string()];

        // 完整：文档行 + 严格 0..n 块序列
        let complete = conn();
        insert_document(&complete, "d1", "参照.md", &chunks, &reference_meta()).unwrap();
        let integrity = document_integrity(&complete, "d1")
            .unwrap()
            .expect("完整文档应有完整性记录");
        assert!(integrity.complete);
        assert_eq!(integrity.corpus_kind, "reference");
        assert_eq!(integrity.chunk_count, 2);
        assert_eq!(integrity.chunk_set_hash, hash_canonical_chunk_set(&chunks));

        // 残缺：文档行声称 2 块、实际只余 1 块
        let partial = conn();
        insert_document(&partial, "d2", "参照.md", &chunks, &reference_meta()).unwrap();
        partial
            .execute(
                "DELETE FROM kb_chunks WHERE doc_id = 'd2' AND chunk_index = 1",
                [],
            )
            .unwrap();
        let integrity = document_integrity(&partial, "d2")
            .unwrap()
            .expect("残缺文档仍有记录");
        assert!(!integrity.complete);
        assert_eq!(integrity.chunk_count, 1);

        // 块数不符：文档行 chunk_count 与实际块数不一致
        let mismatched = conn();
        insert_document(&mismatched, "d3", "参照.md", &chunks, &reference_meta()).unwrap();
        mismatched
            .execute(
                "UPDATE kb_documents SET chunk_count = 1 WHERE id = 'd3'",
                [],
            )
            .unwrap();
        let integrity = document_integrity(&mismatched, "d3")
            .unwrap()
            .expect("块数不符文档仍有记录");
        assert!(!integrity.complete);
        assert_eq!(integrity.chunk_count, 2);

        // 文档与块均不存在 → None（对齐基线 null 语义）
        assert!(document_integrity(&complete, "missing").unwrap().is_none());
    }

    #[test]
    fn replace_document_rewrites_chunks_without_residue_test() {
        let conn = conn();
        let old = vec!["旧块一".to_string(), "旧块二".to_string()];
        let old_ids = insert_document(&conn, "d1", "参照.md", &old, &reference_meta()).unwrap();
        assert_eq!(old_ids.len(), 2);

        let new = vec![
            "新块一".to_string(),
            "新块二".to_string(),
            "新块三".to_string(),
        ];
        let new_ids = replace_document(&conn, "d1", "参照.md", &new, &reference_meta()).unwrap();
        assert_eq!(new_ids.len(), 3);

        // kb_chunks 数量正确
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kb_chunks WHERE doc_id = 'd1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
        // 无残留：旧块在 kb_chunks 与 kb_fts 均不存活
        for old_id in &old_ids {
            let remaining: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM kb_chunks WHERE id = ?1",
                    [old_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(remaining, 0, "旧块 {old_id} 在 kb_chunks 应无残留");
            let fts_remaining: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM kb_fts WHERE chunk_id = ?1",
                    [old_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(fts_remaining, 0, "FTS 旧块 {old_id} 应无残留");
        }
        // 重写后完整性复核
        let integrity = document_integrity(&conn, "d1")
            .unwrap()
            .expect("重写后应完整");
        assert!(integrity.complete);
        assert_eq!(integrity.chunk_set_hash, hash_canonical_chunk_set(&new));
    }

    #[test]
    fn reference_document_receipt_roundtrip_test() {
        let conn = conn();
        let receipt = ReferenceDocumentReceipt {
            document_id: "doc-1".to_string(),
            idempotency_key_hash: "key-hash".to_string(),
            content_hash: "content-hash".to_string(),
            chunk_set_hash: "chunk-set-hash".to_string(),
            expected_chunk_count: 2,
            corpus_kind: "reference".to_string(),
            state: "prepared".to_string(),
        };
        assert!(read_reference_document(&conn, "doc-1", "key-hash")
            .unwrap()
            .is_none());

        upsert_reference_document(&conn, &receipt).unwrap();
        let read = read_reference_document(&conn, "doc-1", "key-hash")
            .unwrap()
            .expect("应可读回");
        assert_eq!(read, receipt);

        // 幂等：重复插入不覆盖既有行（对齐基线「不存在则插」）
        let mut rewritten = receipt.clone();
        rewritten.state = "committed".to_string();
        upsert_reference_document(&conn, &rewritten).unwrap();
        assert_eq!(
            read_reference_document(&conn, "doc-1", "key-hash")
                .unwrap()
                .unwrap()
                .state,
            "prepared"
        );

        // 状态机推进
        mark_reference_document_state(&conn, "doc-1", "committed").unwrap();
        assert_eq!(
            read_reference_document(&conn, "doc-1", "key-hash")
                .unwrap()
                .unwrap()
                .state,
            "committed"
        );

        // 幂等键哈希亦可定位（基线 WHERE document_id = ? OR idempotency_key_hash = ?）
        let by_key = read_reference_document(&conn, "unknown-doc", "key-hash")
            .unwrap()
            .expect("应按幂等键命中");
        assert_eq!(by_key.document_id, "doc-1");
    }
}
