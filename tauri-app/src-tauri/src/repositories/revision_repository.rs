//! RevisionRepository —— 修稿（`revisions` 表 + `contents` 联动）
//!
//! 平移自 `electron/repositories/revision-repository.ts`：修稿是基于某一版草稿的探索分支，
//! 状态流转 `pending` → `merged` / `discarded`。
//! - 创建时冻结源稿快照（章号/版本/状态/正文），合并时据此拒绝与源稿不一致的覆盖；
//! - `create` / `replace_pending` 都要求 `expectedSource` 通过源守卫；
//! - `merge_into_draft` 以 `revisionId` 作为天然幂等身份，重复合并只回读结果；
//! - 修稿序号在事务内原子分配，不接受调用方传序号。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::draft_source_guard::{
    assert_expected_draft_source, ExpectedDraftSource, SOURCE_DRAFT_CHANGED_MESSAGE,
};
use crate::repositories::content_repository as contents;

/// 可修改草稿状态（合并目标必须是其中之一）
const MODIFIABLE_DRAFT_STATUSES: [&str; 3] = ["draft", "revised", "reviewed"];

/// 修稿元数据（不含正文）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionMeta {
    pub id: i64,
    pub base_draft_id: i64,
    pub revision_index: i64,
    pub revision_type: String,
    pub status: String,
    pub merged_to_draft_id: Option<i64>,
    pub user_prompt: String,
    pub review_source_id: Option<i64>,
    pub content_id: i64,
    pub word_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// 修稿完整数据（含正文与冻结的源稿）—— 扁平序列化（对齐 `RevisionFull extends RevisionMeta`）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionFull {
    #[serde(flatten)]
    pub meta: RevisionMeta,
    pub content: String,
    pub source_draft: Option<ExpectedDraftSource>,
}

/// 创建/替换修稿的参数
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionCreateParams {
    pub base_draft_id: i64,
    pub revision_type: String,
    #[serde(default)]
    pub user_prompt: Option<String>,
    #[serde(default)]
    pub review_source_id: Option<i64>,
    pub content: String,
    pub word_count: i64,
    /// 源守卫输入；缺失直接判为源稿变化（对齐基线 `if (!params.expectedSource) throw`）
    #[serde(default)]
    pub expected_source: Option<ExpectedDraftSource>,
}

/// 创建结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionCreated {
    pub id: i64,
    pub revision_index: i64,
}

/// 合并请求（对齐 `MergeRevisionRequest`）
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeRevisionRequest {
    pub revision_id: i64,
    pub target_draft_id: i64,
    pub expected_draft_content: String,
    pub merged_content: String,
    pub word_count: i64,
}

/// 合并回执（幂等身份 = `revisionId`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeRevisionReceipt {
    pub revision_id: i64,
    pub target_draft_id: i64,
    /// 恒为 `revised`（对齐基线字面量）
    pub status: String,
    pub word_count: i64,
    pub idempotent: bool,
}

struct RevisionRow {
    id: i64,
    base_draft_id: i64,
    revision_index: i64,
    revision_type: String,
    status: String,
    merged_to_draft_id: Option<i64>,
    user_prompt: String,
    review_source_id: Option<i64>,
    source_draft_chapter_number: Option<i64>,
    source_draft_version: Option<i64>,
    source_draft_status: Option<String>,
    source_content: Option<String>,
    content_id: i64,
    word_count: i64,
    created_at: String,
    updated_at: String,
}

fn revision_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RevisionRow> {
    Ok(RevisionRow {
        id: row.get("id")?,
        base_draft_id: row.get("base_draft_id")?,
        revision_index: row.get("revision_index")?,
        revision_type: row.get("revision_type")?,
        status: row.get::<_, Option<String>>("status")?.unwrap_or_default(),
        merged_to_draft_id: row.get("merged_to_draft_id")?,
        user_prompt: row
            .get::<_, Option<String>>("user_prompt")?
            .unwrap_or_default(),
        review_source_id: row.get("review_source_id")?,
        source_draft_chapter_number: row.get("source_draft_chapter_number")?,
        source_draft_version: row.get("source_draft_version")?,
        source_draft_status: row.get("source_draft_status")?,
        source_content: row.get("source_content")?,
        content_id: row.get("content_id")?,
        word_count: row.get::<_, Option<i64>>("word_count")?.unwrap_or_default(),
        created_at: row
            .get::<_, Option<String>>("created_at")?
            .unwrap_or_default(),
        updated_at: row
            .get::<_, Option<String>>("updated_at")?
            .unwrap_or_default(),
    })
}

fn row_to_meta(row: &RevisionRow) -> RevisionMeta {
    RevisionMeta {
        id: row.id,
        base_draft_id: row.base_draft_id,
        revision_index: row.revision_index,
        revision_type: row.revision_type.clone(),
        status: row.status.clone(),
        merged_to_draft_id: row.merged_to_draft_id,
        user_prompt: row.user_prompt.clone(),
        review_source_id: row.review_source_id,
        content_id: row.content_id,
        word_count: row.word_count,
        created_at: row.created_at.clone(),
        updated_at: row.updated_at.clone(),
    }
}

/// 冻结源稿快照 → `ExpectedDraftSource`（四列任一为 NULL 即视为缺失）
fn row_to_source_draft(row: &RevisionRow) -> Option<ExpectedDraftSource> {
    Some(ExpectedDraftSource {
        id: row.base_draft_id,
        chapter_number: row.source_draft_chapter_number?,
        version: row.source_draft_version?,
        status: row.source_draft_status.clone()?,
        content: row.source_content.clone()?,
    })
}

/// 分配下一个修稿序号（事务内调用）
fn next_revision_index(conn: &Connection, base_draft_id: i64) -> Result<i64, String> {
    let max_index: Option<i64> = conn
        .query_row(
            "SELECT MAX(revision_index) FROM revisions WHERE base_draft_id = ?1",
            [base_draft_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("读取修稿序号失败：{error}"))?;
    Ok(max_index.unwrap_or(0) + 1)
}

/// 插入修稿行（`create` 与 `replace_pending` 共用）
fn insert_revision(
    conn: &Connection,
    params: &RevisionCreateParams,
    expected: &ExpectedDraftSource,
    revision_index: i64,
    content_id: i64,
) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO revisions (
           base_draft_id, revision_index, revision_type,
           user_prompt, review_source_id,
           source_draft_chapter_number, source_draft_version, source_draft_status, source_content,
           content_id, word_count
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            params.base_draft_id,
            revision_index,
            params.revision_type,
            params.user_prompt.clone().unwrap_or_default(),
            params.review_source_id,
            expected.chapter_number,
            expected.version,
            expected.status,
            expected.content,
            content_id,
            params.word_count,
        ],
    )
    .map_err(|error| format!("写入修稿失败：{error}"))?;
    Ok(conn.last_insert_rowid())
}

/// 校验源守卫并取冻结源稿（缺失即判源稿变化）
fn guard_source<'a>(
    conn: &Connection,
    params: &'a RevisionCreateParams,
) -> Result<&'a ExpectedDraftSource, String> {
    let Some(expected) = params.expected_source.as_ref() else {
        return Err(SOURCE_DRAFT_CHANGED_MESSAGE.to_string());
    };
    assert_expected_draft_source(conn, params.base_draft_id, expected)?;
    Ok(expected)
}

/// 创建修稿（事务内原子分配 `revision_index`）
pub fn create(conn: &Connection, params: &RevisionCreateParams) -> Result<RevisionCreated, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;

    let expected = guard_source(&tx, params)?.clone();
    let revision_index = next_revision_index(&tx, params.base_draft_id)?;
    let content_id = contents::create(&tx, &params.content)?;
    let id = insert_revision(&tx, params, &expected, revision_index, content_id)?;

    tx.commit()
        .map_err(|error| format!("提交修稿失败：{error}"))?;
    Ok(RevisionCreated { id, revision_index })
}

/// 原子替换同一草稿的 pending 修稿：新修稿创建失败时，旧 pending 的状态与内容池分配
/// 会随整个事务一起回滚。
pub fn replace_pending(
    conn: &Connection,
    params: &RevisionCreateParams,
) -> Result<RevisionCreated, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;

    let expected = guard_source(&tx, params)?.clone();
    let revision_index = next_revision_index(&tx, params.base_draft_id)?;
    let content_id = contents::create(&tx, &params.content)?;
    let id = insert_revision(&tx, params, &expected, revision_index, content_id)?;

    tx.execute(
        "UPDATE revisions SET status = 'discarded', updated_at = datetime('now')
         WHERE base_draft_id = ?1 AND status = 'pending' AND id <> ?2",
        rusqlite::params![params.base_draft_id, id],
    )
    .map_err(|error| format!("弃用旧修稿失败：{error}"))?;

    tx.commit()
        .map_err(|error| format!("提交修稿失败：{error}"))?;
    Ok(RevisionCreated { id, revision_index })
}

fn read_metas(conn: &Connection, sql: &str, params: [i64; 1]) -> Result<Vec<RevisionMeta>, String> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|error| format!("读取修稿失败：{error}"))?;
    let rows = stmt
        .query_map(params, revision_row)
        .map_err(|error| format!("读取修稿失败：{error}"))?;
    let mut result = Vec::new();
    for row in rows {
        let row = row.map_err(|error| format!("读取修稿失败：{error}"))?;
        result.push(row_to_meta(&row));
    }
    Ok(result)
}

/// 列出某草稿的所有修稿（按序号升序）
pub fn list_by_draft(conn: &Connection, base_draft_id: i64) -> Result<Vec<RevisionMeta>, String> {
    read_metas(
        conn,
        "SELECT * FROM revisions WHERE base_draft_id = ?1 ORDER BY revision_index ASC",
        [base_draft_id],
    )
}

/// 列出某草稿的所有 pending 修稿
pub fn get_pending(conn: &Connection, base_draft_id: i64) -> Result<Vec<RevisionMeta>, String> {
    read_metas(
        conn,
        "SELECT * FROM revisions WHERE base_draft_id = ?1 AND status = 'pending'
         ORDER BY revision_index ASC",
        [base_draft_id],
    )
}

/// 获取修稿完整数据（含正文与冻结源稿）
pub fn get_full(conn: &Connection, id: i64) -> Result<Option<RevisionFull>, String> {
    let row = conn
        .query_row("SELECT * FROM revisions WHERE id = ?1", [id], revision_row)
        .optional()
        .map_err(|error| format!("读取修稿失败：{error}"))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let meta = row_to_meta(&row);
    let content = contents::get_body(conn, meta.content_id)?.unwrap_or_default();
    let source_draft = row_to_source_draft(&row);
    Ok(Some(RevisionFull {
        meta,
        content,
        source_draft,
    }))
}

/// 获取下一个修稿序号
pub fn get_next_index(conn: &Connection, base_draft_id: i64) -> Result<i64, String> {
    next_revision_index(conn, base_draft_id)
}

/// 将人工确认的合并正文、草稿状态与修订关系作为一个提交写入。
///
/// `revision_id` 同时是天然的幂等身份：已完成的同一合并只返回读回结果，
/// 不会再次覆盖之后产生的正文。
pub fn merge_into_draft(
    conn: &Connection,
    request: &MergeRevisionRequest,
) -> Result<MergeRevisionReceipt, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败：{error}"))?;

    let outcome = merge_into_draft_inner(&tx, request);
    match outcome {
        Ok(receipt) => {
            tx.commit()
                .map_err(|error| format!("提交合并失败：{error}"))?;
            Ok(receipt)
        }
        // `tx` drop 即回滚
        Err(error) => Err(error),
    }
}

fn merge_into_draft_inner(
    tx: &Connection,
    request: &MergeRevisionRequest,
) -> Result<MergeRevisionReceipt, String> {
    let target: Option<(String, i64, i64, i64, i64, String)> = tx
        .query_row(
            "SELECT drafts.chapter_number, drafts.version, drafts.status,
                    drafts.word_count, drafts.content_id, contents.body
             FROM drafts
             JOIN contents ON contents.id = drafts.content_id
             WHERE drafts.id = ?1",
            [request.target_draft_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    row.get(0)?,
                    row.get(1)?,
                    row.get::<_, Option<i64>>(3)?.unwrap_or_default(),
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("读取目标草稿失败：{error}"))?;
    let Some((status, chapter_number, version, word_count, content_id, body)) = target else {
        return Err(format!("草稿不存在：{}", request.target_draft_id));
    };

    let revision: Option<(
        i64,
        String,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<String>,
    )> = tx
        .query_row(
            "SELECT base_draft_id, status, merged_to_draft_id,
                    source_draft_chapter_number, source_draft_version,
                    source_draft_status, source_content
             FROM revisions WHERE id = ?1",
            [request.revision_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("读取修订稿失败：{error}"))?;
    let Some((
        base_draft_id,
        revision_status,
        merged_to_draft_id,
        source_chapter_number,
        source_version,
        source_status,
        source_content,
    )) = revision
    else {
        return Err(format!("修订稿不存在：{}", request.revision_id));
    };

    if base_draft_id != request.target_draft_id {
        return Err("修订稿不属于目标草稿，已拒绝合并".to_string());
    }

    if revision_status == "merged" {
        // 幂等分支：已完成合并必须与当前事实一致，否则拒绝再次覆盖
        if merged_to_draft_id != Some(request.target_draft_id)
            || body != request.merged_content
            || status != "revised"
            || word_count != request.word_count
        {
            return Err("修订稿已经合并，但目标草稿随后发生变化；已拒绝再次覆盖".to_string());
        }
        return Ok(MergeRevisionReceipt {
            revision_id: request.revision_id,
            target_draft_id: request.target_draft_id,
            status: "revised".to_string(),
            word_count: request.word_count,
            idempotent: true,
        });
    }
    if revision_status != "pending" {
        return Err("修订稿已失效，不能继续合并".to_string());
    }
    if !MODIFIABLE_DRAFT_STATUSES.contains(&status.as_str()) {
        return Err("目标草稿不是可修改状态，已拒绝合并".to_string());
    }
    if body != request.expected_draft_content {
        return Err("目标草稿正文已变化，请重新打开修订对比".to_string());
    }
    if source_chapter_number.is_none()
        || source_version.is_none()
        || source_status.is_none()
        || source_content.is_none()
    {
        return Err("旧修订稿缺少生成时源稿，仍可查看但不能合并；请重新生成修订稿".to_string());
    }
    if chapter_number != source_chapter_number.unwrap()
        || version != source_version.unwrap()
        || status != source_status.clone().unwrap()
        || body != source_content.clone().unwrap()
    {
        return Err("当前草稿与修订稿的生成时源稿不一致，已拒绝合并；请重新生成修订稿".to_string());
    }

    tx.execute(
        "UPDATE contents SET body = ?1 WHERE id = ?2",
        rusqlite::params![request.merged_content, content_id],
    )
    .map_err(|error| format!("更新正文失败：{error}"))?;

    let draft_update = tx
        .execute(
            "UPDATE drafts
             SET status = 'revised', word_count = ?1, updated_at = datetime('now')
             WHERE id = ?2 AND status IN ('draft', 'revised', 'reviewed')",
            rusqlite::params![request.word_count, request.target_draft_id],
        )
        .map_err(|error| format!("更新草稿状态失败：{error}"))?;
    if draft_update != 1 {
        return Err("目标草稿状态已变化，已拒绝合并".to_string());
    }

    let revision_update = tx
        .execute(
            "UPDATE revisions
             SET status = 'merged', merged_to_draft_id = ?1, updated_at = datetime('now')
             WHERE id = ?2 AND base_draft_id = ?3 AND status = 'pending'",
            rusqlite::params![
                request.target_draft_id,
                request.revision_id,
                request.target_draft_id
            ],
        )
        .map_err(|error| format!("更新修订稿状态失败：{error}"))?;
    if revision_update != 1 {
        return Err("修订稿状态已变化，已拒绝合并".to_string());
    }

    Ok(MergeRevisionReceipt {
        revision_id: request.revision_id,
        target_draft_id: request.target_draft_id,
        status: "revised".to_string(),
        word_count: request.word_count,
        idempotent: false,
    })
}

/// 标记为已合并（仅 `pending` → `merged`）
pub fn mark_merged(conn: &Connection, id: i64, merged_to_draft_id: i64) -> Result<(), String> {
    let changes = conn
        .execute(
            "UPDATE revisions
             SET status = 'merged', merged_to_draft_id = ?1, updated_at = datetime('now')
             WHERE id = ?2 AND status = 'pending'",
            rusqlite::params![merged_to_draft_id, id],
        )
        .map_err(|error| format!("标记修稿合并失败：{error}"))?;
    if changes == 0 {
        return Err(format!(
            "[RevisionRepository] 无法合并修稿 #{id}：不存在或非 pending 状态"
        ));
    }
    Ok(())
}

/// 标记为已弃用（仅 `pending` → `discarded`）
pub fn mark_discarded(conn: &Connection, id: i64) -> Result<(), String> {
    let changes = conn
        .execute(
            "UPDATE revisions SET status = 'discarded', updated_at = datetime('now')
             WHERE id = ?1 AND status = 'pending'",
            [id],
        )
        .map_err(|error| format!("标记修稿弃用失败：{error}"))?;
    if changes == 0 {
        return Err(format!(
            "[RevisionRepository] 无法弃用修稿 #{id}：不存在或非 pending 状态"
        ));
    }
    Ok(())
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

    /// 建一个草稿并返回其 ID 与冻结源稿
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
    ) -> RevisionCreateParams {
        RevisionCreateParams {
            base_draft_id,
            revision_type: "refine".to_string(),
            user_prompt: Some("请润色".to_string()),
            review_source_id: None,
            content: content.to_string(),
            word_count: content.chars().count() as i64,
            expected_source: source,
        }
    }

    #[test]
    fn create_assigns_index_and_freezes_source_test() {
        let conn = memory_db();
        let (draft_id, source) = seed_draft(&conn, BASE_BODY);

        let first = create(&conn, &params(draft_id, Some(source.clone()), "修稿一")).unwrap();
        assert_eq!(first.revision_index, 1);
        let second = create(&conn, &params(draft_id, Some(source.clone()), "修稿二")).unwrap();
        assert_eq!(second.revision_index, 2);

        let full = get_full(&conn, first.id).unwrap().unwrap();
        assert_eq!(full.content, "修稿一");
        assert_eq!(full.meta.status, "pending");
        assert_eq!(full.meta.user_prompt, "请润色");
        assert_eq!(full.meta.merged_to_draft_id, None);
        assert_eq!(full.meta.review_source_id, None);
        // 冻结的源稿快照必须与入参一致
        assert_eq!(full.source_draft, Some(source));

        assert_eq!(get_next_index(&conn, draft_id).unwrap(), 3);
        assert_eq!(list_by_draft(&conn, draft_id).unwrap().len(), 2);
        assert_eq!(get_pending(&conn, draft_id).unwrap().len(), 2);
        assert!(get_full(&conn, 9999).unwrap().is_none());
        assert!(list_by_draft(&conn, 9999).unwrap().is_empty());
        assert_eq!(get_next_index(&conn, 9999).unwrap(), 1);
    }

    #[test]
    fn create_requires_and_verifies_expected_source_test() {
        let conn = memory_db();
        let (draft_id, source) = seed_draft(&conn, BASE_BODY);

        // 缺 expectedSource → 直接判源稿变化
        assert_eq!(
            create(&conn, &params(draft_id, None, "修稿")).unwrap_err(),
            SOURCE_DRAFT_CHANGED_MESSAGE
        );

        // 冻结正文与库中不一致 → 拒绝
        let mut drifted = source.clone();
        drifted.content = "旧的正文".to_string();
        assert_eq!(
            create(&conn, &params(draft_id, Some(drifted), "修稿")).unwrap_err(),
            SOURCE_DRAFT_CHANGED_MESSAGE
        );

        // 源稿本体被改写后，旧冻结源稿不再可用
        drafts::update_content(&conn, draft_id, "改写后的正文", 6).unwrap();
        assert_eq!(
            create(&conn, &params(draft_id, Some(source), "修稿")).unwrap_err(),
            SOURCE_DRAFT_CHANGED_MESSAGE
        );
    }

    #[test]
    fn replace_pending_discards_previous_pending_test() {
        let conn = memory_db();
        let (draft_id, source) = seed_draft(&conn, BASE_BODY);

        let first = create(&conn, &params(draft_id, Some(source.clone()), "旧修稿")).unwrap();
        let merged = create(&conn, &params(draft_id, Some(source.clone()), "已合并")).unwrap();
        mark_merged(&conn, merged.id, draft_id).unwrap();

        let replaced =
            replace_pending(&conn, &params(draft_id, Some(source.clone()), "新修稿")).unwrap();
        assert_eq!(replaced.revision_index, 3);

        // 旧 pending 被弃用；已 merged 的不受影响
        let all = list_by_draft(&conn, draft_id).unwrap();
        let status_of = |id: i64| {
            all.iter()
                .find(|meta| meta.id == id)
                .unwrap()
                .status
                .clone()
        };
        assert_eq!(status_of(first.id), "discarded");
        assert_eq!(status_of(merged.id), "merged");
        assert_eq!(status_of(replaced.id), "pending");
        assert_eq!(get_pending(&conn, draft_id).unwrap().len(), 1);

        // 源守卫仍生效
        assert_eq!(
            replace_pending(&conn, &params(draft_id, None, "无效")).unwrap_err(),
            SOURCE_DRAFT_CHANGED_MESSAGE
        );
    }

    #[test]
    fn merge_writes_content_status_and_revision_test() {
        let conn = memory_db();
        let (draft_id, source) = seed_draft(&conn, BASE_BODY);
        let revision = create(&conn, &params(draft_id, Some(source), "修稿正文")).unwrap();

        let receipt = merge_into_draft(
            &conn,
            &MergeRevisionRequest {
                revision_id: revision.id,
                target_draft_id: draft_id,
                expected_draft_content: BASE_BODY.to_string(),
                merged_content: "合并后正文".to_string(),
                word_count: 5,
            },
        )
        .unwrap();
        assert_eq!(receipt.status, "revised");
        assert!(!receipt.idempotent);

        let draft = drafts::get_full(&conn, draft_id).unwrap().unwrap();
        assert_eq!(draft.content, "合并后正文");
        assert_eq!(draft.meta.status, "revised");
        assert_eq!(draft.meta.word_count, 5);
        let meta = get_full(&conn, revision.id).unwrap().unwrap().meta;
        assert_eq!(meta.status, "merged");
        assert_eq!(meta.merged_to_draft_id, Some(draft_id));
    }

    #[test]
    fn merge_is_idempotent_but_rejects_later_drift_test() {
        let conn = memory_db();
        let (draft_id, source) = seed_draft(&conn, BASE_BODY);
        let revision = create(&conn, &params(draft_id, Some(source), "修稿正文")).unwrap();
        let request = MergeRevisionRequest {
            revision_id: revision.id,
            target_draft_id: draft_id,
            expected_draft_content: BASE_BODY.to_string(),
            merged_content: "合并后正文".to_string(),
            word_count: 5,
        };
        merge_into_draft(&conn, &request).unwrap();

        // 重放同一请求：只回读结果
        let replay = merge_into_draft(&conn, &request).unwrap();
        assert!(replay.idempotent);
        assert_eq!(replay.word_count, 5);

        // 目标草稿随后被改写 → 幂等分支必须拒绝再次覆盖
        drafts::update_content(&conn, draft_id, "后续手改", 4).unwrap();
        assert_eq!(
            merge_into_draft(&conn, &request).unwrap_err(),
            "修订稿已经合并，但目标草稿随后发生变化；已拒绝再次覆盖"
        );
    }

    #[test]
    fn merge_rejects_all_guard_violations_test() {
        let conn = memory_db();
        let (draft_id, source) = seed_draft(&conn, BASE_BODY);
        let other_draft = drafts::create(
            &conn,
            &drafts::DraftCreateParams {
                chapter_number: 2,
                version: None,
                source: "write".to_string(),
                content: "第二草稿".to_string(),
                word_count: 4,
                source_dependencies: None,
            },
        )
        .unwrap();
        let revision = create(&conn, &params(draft_id, Some(source.clone()), "修稿正文")).unwrap();

        let request = |target: i64, expected: &str| MergeRevisionRequest {
            revision_id: revision.id,
            target_draft_id: target,
            expected_draft_content: expected.to_string(),
            merged_content: "合并后".to_string(),
            word_count: 3,
        };

        // 修稿不属于目标草稿
        assert_eq!(
            merge_into_draft(&conn, &request(other_draft, "第二草稿")).unwrap_err(),
            "修订稿不属于目标草稿，已拒绝合并"
        );
        // 目标草稿不存在 / 修订稿不存在
        assert_eq!(
            merge_into_draft(&conn, &request(9999, "")).unwrap_err(),
            "草稿不存在：9999"
        );
        let mut missing = request(draft_id, BASE_BODY);
        missing.revision_id = 9999;
        assert_eq!(
            merge_into_draft(&conn, &missing).unwrap_err(),
            "修订稿不存在：9999"
        );
        // 目标正文已变化
        assert_eq!(
            merge_into_draft(&conn, &request(draft_id, "不是当前正文")).unwrap_err(),
            "目标草稿正文已变化，请重新打开修订对比"
        );
        // 修订稿已失效
        mark_discarded(&conn, revision.id).unwrap();
        assert_eq!(
            merge_into_draft(&conn, &request(draft_id, BASE_BODY)).unwrap_err(),
            "修订稿已失效，不能继续合并"
        );
    }

    #[test]
    fn merge_rejects_stale_source_and_non_modifiable_target_test() {
        let conn = memory_db();
        let (draft_id, source) = seed_draft(&conn, BASE_BODY);
        let revision = create(&conn, &params(draft_id, Some(source.clone()), "修稿正文")).unwrap();

        // 目标草稿版本被后续提交推进，但正文未变 → 源稿不一致
        conn.execute("UPDATE drafts SET version = 99 WHERE id = ?1", [draft_id])
            .unwrap();
        assert_eq!(
            merge_into_draft(
                &conn,
                &MergeRevisionRequest {
                    revision_id: revision.id,
                    target_draft_id: draft_id,
                    expected_draft_content: BASE_BODY.to_string(),
                    merged_content: "合并后".to_string(),
                    word_count: 3,
                },
            )
            .unwrap_err(),
            "当前草稿与修订稿的生成时源稿不一致，已拒绝合并；请重新生成修订稿"
        );
        conn.execute("UPDATE drafts SET version = 1 WHERE id = ?1", [draft_id])
            .unwrap();

        // 目标草稿不是可修改状态
        conn.execute(
            "UPDATE drafts SET status = 'finalized' WHERE id = ?1",
            [draft_id],
        )
        .unwrap();
        assert_eq!(
            merge_into_draft(
                &conn,
                &MergeRevisionRequest {
                    revision_id: revision.id,
                    target_draft_id: draft_id,
                    expected_draft_content: BASE_BODY.to_string(),
                    merged_content: "合并后".to_string(),
                    word_count: 3,
                },
            )
            .unwrap_err(),
            "目标草稿不是可修改状态，已拒绝合并"
        );
    }

    #[test]
    fn merge_rejects_legacy_revision_without_frozen_source_test() {
        let conn = memory_db();
        let (draft_id, _) = seed_draft(&conn, BASE_BODY);
        // 旧版修订稿：无冻结源稿列
        conn.execute(
            "INSERT INTO revisions (base_draft_id, revision_index, revision_type, content_id, word_count)
             VALUES (?1, 1, 'refine', (SELECT content_id FROM drafts WHERE id = ?1), 4)",
            [draft_id],
        )
        .unwrap();
        let legacy_id = conn.last_insert_rowid();

        assert_eq!(
            merge_into_draft(
                &conn,
                &MergeRevisionRequest {
                    revision_id: legacy_id,
                    target_draft_id: draft_id,
                    expected_draft_content: BASE_BODY.to_string(),
                    merged_content: "合并后".to_string(),
                    word_count: 3,
                },
            )
            .unwrap_err(),
            "旧修订稿缺少生成时源稿，仍可查看但不能合并；请重新生成修订稿"
        );
        // 仍可读取（sourceDraft 为 null）
        assert!(get_full(&conn, legacy_id)
            .unwrap()
            .unwrap()
            .source_draft
            .is_none());
    }

    #[test]
    fn mark_merged_and_discarded_require_pending_test() {
        let conn = memory_db();
        let (draft_id, source) = seed_draft(&conn, BASE_BODY);
        let revision = create(&conn, &params(draft_id, Some(source.clone()), "修稿正文")).unwrap();

        mark_merged(&conn, revision.id, draft_id).unwrap();
        let meta = get_full(&conn, revision.id).unwrap().unwrap().meta;
        assert_eq!(meta.status, "merged");
        assert_eq!(meta.merged_to_draft_id, Some(draft_id));

        // 非 pending 不得再次标记
        assert_eq!(
            mark_discarded(&conn, revision.id).unwrap_err(),
            format!(
                "[RevisionRepository] 无法弃用修稿 #{}：不存在或非 pending 状态",
                revision.id
            )
        );
        assert_eq!(
            mark_merged(&conn, 9999, draft_id).unwrap_err(),
            "[RevisionRepository] 无法合并修稿 #9999：不存在或非 pending 状态"
        );

        let another = create(
            &conn,
            &params(
                draft_id,
                Some(ExpectedDraftSource {
                    id: draft_id,
                    chapter_number: source.chapter_number,
                    version: source.version,
                    status: source.status.clone(),
                    content: source.content.clone(),
                }),
                "另一个",
            ),
        )
        .unwrap();
        mark_discarded(&conn, another.id).unwrap();
        assert_eq!(
            get_full(&conn, another.id).unwrap().unwrap().meta.status,
            "discarded"
        );
    }

    #[test]
    fn revision_full_serialization_is_flat_test() {
        let conn = memory_db();
        let (draft_id, source) = seed_draft(&conn, BASE_BODY);
        let revision = create(&conn, &params(draft_id, Some(source), "修稿正文")).unwrap();
        let full = get_full(&conn, revision.id).unwrap().unwrap();
        let value = serde_json::to_value(&full).unwrap();

        // RevisionFull 必须扁平（对齐 `RevisionFull extends RevisionMeta`）
        assert!(value.get("meta").is_none(), "meta 不得嵌套");
        assert_eq!(value["id"], serde_json::json!(revision.id));
        assert_eq!(value["baseDraftId"], serde_json::json!(draft_id));
        assert_eq!(value["revisionIndex"], serde_json::json!(1));
        assert_eq!(value["content"], serde_json::json!("修稿正文"));
        // 可空字段序列化为 null（对齐基线 `number | null` / `?? null`）
        assert_eq!(value["mergedToDraftId"], serde_json::Value::Null);
        assert_eq!(value["reviewSourceId"], serde_json::Value::Null);
        assert_eq!(value["sourceDraft"]["chapterNumber"], serde_json::json!(1));
        assert_eq!(
            value["sourceDraft"]["content"],
            serde_json::json!(BASE_BODY)
        );
    }
}
