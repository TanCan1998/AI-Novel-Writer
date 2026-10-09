//! ReviewRepository —— 审稿（`reviews` 表 + `contents` 联动）
//!
//! 平移自 `electron/repositories/review-repository.ts`：审稿是对某版草稿的评审反馈报告。
//! - 创建时冻结源稿快照，且要求 `expectedSource` 通过源守卫（同样回 `SOURCE_DRAFT_CHANGED`）；
//! - `review_index` 在事务内原子分配，不接受调用方传序号；
//! - 审稿本身没有状态流转（不像修稿有 pending/merged/discarded）。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::draft_source_guard::{
    assert_expected_draft_source, ExpectedDraftSource, SOURCE_DRAFT_CHANGED_MESSAGE,
};
use crate::repositories::content_repository as contents;

/// 审稿元数据（不含报告正文）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewMeta {
    pub id: i64,
    pub base_draft_id: i64,
    pub review_index: i64,
    pub content_id: i64,
    pub created_at: String,
}

/// 审稿完整数据（含报告正文与冻结源稿）—— 扁平序列化（对齐 `ReviewFull extends ReviewMeta`）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFull {
    #[serde(flatten)]
    pub meta: ReviewMeta,
    pub content: String,
    pub source_draft: Option<ExpectedDraftSource>,
}

/// 创建审稿的参数
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCreateParams {
    pub base_draft_id: i64,
    /// 契约允许传入，但基线在事务内重新分配 `review_index`，故忽略。
    #[serde(default)]
    #[allow(dead_code)]
    pub review_index: Option<i64>,
    pub content: String,
    /// 源守卫输入；缺失直接判为源稿变化（对齐基线 `if (!params.expectedSource) throw`）
    #[serde(default)]
    pub expected_source: Option<ExpectedDraftSource>,
}

/// 创建结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCreated {
    pub id: i64,
    pub review_index: i64,
}

struct ReviewRow {
    id: i64,
    base_draft_id: i64,
    review_index: i64,
    source_draft_chapter_number: Option<i64>,
    source_draft_version: Option<i64>,
    source_draft_status: Option<String>,
    source_content: Option<String>,
    content_id: i64,
    created_at: String,
}

fn review_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewRow> {
    Ok(ReviewRow {
        id: row.get("id")?,
        base_draft_id: row.get("base_draft_id")?,
        review_index: row.get("review_index")?,
        source_draft_chapter_number: row.get("source_draft_chapter_number")?,
        source_draft_version: row.get("source_draft_version")?,
        source_draft_status: row.get("source_draft_status")?,
        source_content: row.get("source_content")?,
        content_id: row.get("content_id")?,
        created_at: row
            .get::<_, Option<String>>("created_at")?
            .unwrap_or_default(),
    })
}

fn row_to_meta(row: &ReviewRow) -> ReviewMeta {
    ReviewMeta {
        id: row.id,
        base_draft_id: row.base_draft_id,
        review_index: row.review_index,
        content_id: row.content_id,
        created_at: row.created_at.clone(),
    }
}

/// 冻结源稿快照 → `ExpectedDraftSource`（四列任一为 NULL 即视为缺失）
fn row_to_source_draft(row: &ReviewRow) -> Option<ExpectedDraftSource> {
    Some(ExpectedDraftSource {
        id: row.base_draft_id,
        chapter_number: row.source_draft_chapter_number?,
        version: row.source_draft_version?,
        status: row.source_draft_status.clone()?,
        content: row.source_content.clone()?,
    })
}

fn next_review_index(conn: &Connection, base_draft_id: i64) -> Result<i64, String> {
    let max_index: Option<i64> = conn
        .query_row(
            "SELECT MAX(review_index) FROM reviews WHERE base_draft_id = ?1",
            [base_draft_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("读取审稿序号失败：{error}"))?;
    Ok(max_index.unwrap_or(0) + 1)
}

/// 创建审稿（事务内原子分配 `review_index` + 校验源稿）
pub fn create(conn: &Connection, params: &ReviewCreateParams) -> Result<ReviewCreated, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;

    let Some(expected) = params.expected_source.as_ref() else {
        return Err(SOURCE_DRAFT_CHANGED_MESSAGE.to_string());
    };
    assert_expected_draft_source(&tx, params.base_draft_id, expected)?;

    let review_index = next_review_index(&tx, params.base_draft_id)?;
    let content_id = contents::create(&tx, &params.content)?;
    tx.execute(
        "INSERT INTO reviews (
           base_draft_id, review_index,
           source_draft_chapter_number, source_draft_version, source_draft_status, source_content,
           content_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            params.base_draft_id,
            review_index,
            expected.chapter_number,
            expected.version,
            expected.status,
            expected.content,
            content_id,
        ],
    )
    .map_err(|error| format!("写入审稿失败：{error}"))?;
    let id = tx.last_insert_rowid();

    tx.commit()
        .map_err(|error| format!("提交审稿失败：{error}"))?;
    Ok(ReviewCreated { id, review_index })
}

/// 列出某草稿的所有审稿（按序号升序）
pub fn list_by_draft(conn: &Connection, base_draft_id: i64) -> Result<Vec<ReviewMeta>, String> {
    let mut stmt = conn
        .prepare("SELECT * FROM reviews WHERE base_draft_id = ?1 ORDER BY review_index ASC")
        .map_err(|error| format!("读取审稿失败：{error}"))?;
    let rows = stmt
        .query_map([base_draft_id], review_row)
        .map_err(|error| format!("读取审稿失败：{error}"))?;
    let mut result = Vec::new();
    for row in rows {
        let row = row.map_err(|error| format!("读取审稿失败：{error}"))?;
        result.push(row_to_meta(&row));
    }
    Ok(result)
}

fn build_full(conn: &Connection, row: ReviewRow) -> Result<ReviewFull, String> {
    let meta = row_to_meta(&row);
    let content = contents::get_body(conn, meta.content_id)?.unwrap_or_default();
    let source_draft = row_to_source_draft(&row);
    Ok(ReviewFull {
        meta,
        content,
        source_draft,
    })
}

/// 获取某草稿的最新审稿（含报告正文）
pub fn get_latest_by_draft(
    conn: &Connection,
    base_draft_id: i64,
) -> Result<Option<ReviewFull>, String> {
    let row = conn
        .query_row(
            "SELECT * FROM reviews WHERE base_draft_id = ?1
             ORDER BY review_index DESC LIMIT 1",
            [base_draft_id],
            review_row,
        )
        .optional()
        .map_err(|error| format!("读取审稿失败：{error}"))?;
    match row {
        Some(row) => build_full(conn, row).map(Some),
        None => Ok(None),
    }
}

/// 获取审稿完整数据（含报告正文与冻结源稿）
pub fn get_full(conn: &Connection, id: i64) -> Result<Option<ReviewFull>, String> {
    let row = conn
        .query_row("SELECT * FROM reviews WHERE id = ?1", [id], review_row)
        .optional()
        .map_err(|error| format!("读取审稿失败：{error}"))?;
    match row {
        Some(row) => build_full(conn, row).map(Some),
        None => Ok(None),
    }
}

/// 获取下一个审稿序号
pub fn get_next_index(conn: &Connection, base_draft_id: i64) -> Result<i64, String> {
    next_review_index(conn, base_draft_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;
    use crate::repositories::draft_repository as drafts;

    const BASE_BODY: &str = "基础正文";

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        create_tables(&conn).expect("建表失败");
        conn
    }

    fn seed_draft(conn: &Connection, body: &str) -> (i64, ExpectedDraftSource) {
        let id = drafts::create(
            conn,
            &drafts::DraftCreateParams {
                chapter_number: 1,
                version: None,
                source: "write".to_string(),
                content: body.to_string(),
                word_count: body.chars().count() as i64,
                source_dependencies: None,
            },
        )
        .expect("建草稿失败");
        (
            id,
            ExpectedDraftSource {
                id,
                chapter_number: 1,
                version: 1,
                status: "draft".to_string(),
                content: body.to_string(),
            },
        )
    }

    fn params(
        base_draft_id: i64,
        source: Option<ExpectedDraftSource>,
        content: &str,
    ) -> ReviewCreateParams {
        ReviewCreateParams {
            base_draft_id,
            // 契约允许传入，但基线忽略该值
            review_index: Some(99),
            content: content.to_string(),
            expected_source: source,
        }
    }

    #[test]
    fn create_assigns_index_and_freezes_source_test() {
        let conn = memory_db();
        let (draft_id, source) = seed_draft(&conn, BASE_BODY);

        let first = create(&conn, &params(draft_id, Some(source.clone()), "审稿报告一")).unwrap();
        assert_eq!(first.review_index, 1, "序号必须由事务分配，而非入参 99");
        let second = create(&conn, &params(draft_id, Some(source.clone()), "审稿报告二")).unwrap();
        assert_eq!(second.review_index, 2);

        let full = get_full(&conn, first.id).unwrap().unwrap();
        assert_eq!(full.content, "审稿报告一");
        assert_eq!(full.meta.base_draft_id, draft_id);
        // 冻结的源稿快照必须与入参一致
        assert_eq!(full.source_draft, Some(source));

        assert_eq!(get_next_index(&conn, draft_id).unwrap(), 3);
        assert_eq!(list_by_draft(&conn, draft_id).unwrap().len(), 2);
        assert!(get_full(&conn, 9999).unwrap().is_none());
        assert!(get_latest_by_draft(&conn, 9999).unwrap().is_none());
        assert!(list_by_draft(&conn, 9999).unwrap().is_empty());
        assert_eq!(get_next_index(&conn, 9999).unwrap(), 1);
    }

    #[test]
    fn get_latest_returns_highest_index_test() {
        let conn = memory_db();
        let (draft_id, source) = seed_draft(&conn, BASE_BODY);
        create(&conn, &params(draft_id, Some(source.clone()), "第一份")).unwrap();
        let latest = create(&conn, &params(draft_id, Some(source.clone()), "第二份")).unwrap();

        let full = get_latest_by_draft(&conn, draft_id).unwrap().unwrap();
        assert_eq!(full.meta.id, latest.id);
        assert_eq!(full.content, "第二份");
        assert_eq!(full.source_draft, Some(source));
    }

    #[test]
    fn create_requires_and_verifies_expected_source_test() {
        let conn = memory_db();
        let (draft_id, source) = seed_draft(&conn, BASE_BODY);

        // 缺 expectedSource → 直接判源稿变化
        assert_eq!(
            create(&conn, &params(draft_id, None, "报告")).unwrap_err(),
            SOURCE_DRAFT_CHANGED_MESSAGE
        );

        // 冻结正文与库中不一致 → 拒绝
        let mut drifted = source.clone();
        drifted.content = "旧的正文".to_string();
        assert_eq!(
            create(&conn, &params(draft_id, Some(drifted), "报告")).unwrap_err(),
            SOURCE_DRAFT_CHANGED_MESSAGE
        );

        // 源稿本体被改写后，旧冻结源稿不再可用
        drafts::update_content(&conn, draft_id, "改写后的正文", 6).unwrap();
        assert_eq!(
            create(&conn, &params(draft_id, Some(source), "报告")).unwrap_err(),
            SOURCE_DRAFT_CHANGED_MESSAGE
        );
    }

    #[test]
    fn review_full_serialization_is_flat_test() {
        let conn = memory_db();
        let (draft_id, source) = seed_draft(&conn, BASE_BODY);
        let review = create(&conn, &params(draft_id, Some(source), "审稿报告")).unwrap();
        let full = get_full(&conn, review.id).unwrap().unwrap();
        let value = serde_json::to_value(&full).unwrap();

        // ReviewFull 必须扁平（对齐 `ReviewFull extends ReviewMeta`）
        assert!(value.get("meta").is_none(), "meta 不得嵌套");
        assert_eq!(value["id"], serde_json::json!(review.id));
        assert_eq!(value["baseDraftId"], serde_json::json!(draft_id));
        assert_eq!(value["reviewIndex"], serde_json::json!(1));
        assert_eq!(value["content"], serde_json::json!("审稿报告"));
        // 冻结源稿字段名为 camelCase
        assert_eq!(value["sourceDraft"]["chapterNumber"], serde_json::json!(1));
        assert_eq!(
            value["sourceDraft"]["content"],
            serde_json::json!(BASE_BODY)
        );

        // 旧审稿无冻结源稿时序列化为 null
        conn.execute(
            "INSERT INTO reviews (base_draft_id, review_index, content_id)
             VALUES (?1, 2, (SELECT content_id FROM drafts WHERE id = ?1))",
            [draft_id],
        )
        .unwrap();
        let legacy_id = conn.last_insert_rowid();
        let legacy = get_full(&conn, legacy_id).unwrap().unwrap();
        let legacy_value = serde_json::to_value(&legacy).unwrap();
        assert_eq!(legacy_value["sourceDraft"], serde_json::Value::Null);
    }
}
