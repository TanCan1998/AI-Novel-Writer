//! recovery_candidates —— 生成失败恢复候选仓储（批次 E）。
//!
//! 平移自 `electron/repositories/recovery-candidate-repository.ts`：
//! - `candidate_id` 为业务键主键，基线用 `randomUUID()` 生成，此处复用项目访问层同语义工具；
//! - 哈希口径与基线一致：`source_hash = sha256(源章节快照 JSON)`、
//!   `content_hash = sha256(visible_text)`，读取时逐字节校验完整性；
//! - `source_is_current` 复刻基线对「蓝图 + 最新草稿身份」的双重时效校验；
//! - 错误文案与基线逐字对齐（MUTATING 包装由 `commands::db::mutating_error` 统一处理）。

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::blueprint_repository as blueprints;
use super::draft_repository as drafts;

pub const MAX_IDENTITY_TEXT: usize = 160;
pub const MAX_FAILURE_REASON: usize = 2_000;

/// v4 格式标识生成（基线 `randomUUID()` 的等价物），复用项目访问层既有工具。
pub fn new_candidate_id() -> String {
    crate::project_access::random_uuid_v4()
}

// ===== 共享类型（对齐 `src/shared/recovery-candidate.ts`） =====

pub type RecoveryCandidateStatus = &'static str;
#[allow(dead_code)]
pub const STATUS_PENDING: RecoveryCandidateStatus = "pending";
pub const STATUS_CONTINUED: RecoveryCandidateStatus = "continued";
pub const STATUS_DISCARDED: RecoveryCandidateStatus = "discarded";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryChapterSource {
    pub chapter_number: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub key_events: String,
    #[serde(default)]
    pub characters: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspense_hook: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_guidance: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryDraftSource {
    pub id: i64,
    pub version: i64,
}

/// `db:recovery-candidate-record` 入参（`projectId` 由租约注入，不经渲染层）
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCandidateRecordRequest {
    pub run_id: String,
    pub step_id: String,
    pub chapter_number: i64,
    #[serde(default)]
    pub chapter_title: String,
    pub source: RecoveryChapterSource,
    #[serde(default)]
    pub source_draft: Option<RecoveryDraftSource>,
    pub visible_text: String,
    #[serde(default)]
    pub failure_code: String,
    #[serde(default)]
    pub failure_reason: String,
    #[serde(default)]
    pub replaces_candidate_id: Option<String>,
}

/// 对齐契约 `RecoveryCandidate`（`sourceCurrent` 由读取时校验得出）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCandidate {
    pub candidate_id: String,
    pub run_id: String,
    pub step_id: String,
    pub project_id: String,
    pub chapter_number: i64,
    pub chapter_title: String,
    pub source_hash: String,
    pub visible_text: String,
    pub content_hash: String,
    pub failure_code: String,
    pub failure_reason: String,
    pub status: String,
    pub replaces_candidate_id: Option<String>,
    pub source_current: bool,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

// ===== 内部行结构 =====

#[derive(Debug, Clone)]
struct CandidateRow {
    candidate_id: String,
    run_id: String,
    step_id: String,
    project_id: String,
    chapter_number: i64,
    chapter_title: String,
    source_snapshot: String,
    source_hash: String,
    source_draft_id: Option<i64>,
    source_draft_version: Option<i64>,
    source_draft_identity_captured: i64,
    visible_text: String,
    content_hash: String,
    failure_code: String,
    failure_reason: String,
    status: String,
    replaces_candidate_id: Option<String>,
    created_at: String,
    resolved_at: Option<String>,
}

const ROW_SELECT: &str = "SELECT candidate_id, run_id, step_id, project_id, chapter_number, \
     chapter_title, source_snapshot, source_hash, source_draft_id, source_draft_version, \
     source_draft_identity_captured, visible_text, content_hash, failure_code, failure_reason, \
     status, replaces_candidate_id, created_at, resolved_at FROM recovery_candidates";

fn row_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<CandidateRow> {
    Ok(CandidateRow {
        candidate_id: row.get(0)?,
        run_id: row.get(1)?,
        step_id: row.get(2)?,
        project_id: row.get(3)?,
        chapter_number: row.get(4)?,
        chapter_title: row.get(5)?,
        source_snapshot: row.get(6)?,
        source_hash: row.get(7)?,
        source_draft_id: row.get(8)?,
        source_draft_version: row.get(9)?,
        source_draft_identity_captured: row.get(10)?,
        visible_text: row.get(11)?,
        content_hash: row.get(12)?,
        failure_code: row.get(13)?,
        failure_reason: row.get(14)?,
        status: row.get(15)?,
        replaces_candidate_id: row.get(16)?,
        created_at: row.get(17)?,
        resolved_at: row.get(18)?,
    })
}

// ===== 工具函数（对齐基线同名列） =====

pub(crate) fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 对齐基线 `assertText`：trim 后非空且不超过 `max`，否则报「{label}无效」
fn assert_text(value: &str, label: &str, max: usize) -> Result<String, String> {
    let normalized = value.trim();
    if normalized.is_empty() || normalized.chars().count() > max {
        return Err(format!("{label}无效"));
    }
    Ok(normalized.to_string())
}

/// 源章节快照的规范化（对齐基线 `sourceSnapshot` + `serializedSource`）：
/// 固定键序 `chapterNumber → title → role → purpose → keyEvents → characters
/// → suspenseHook → userGuidance`（JS 对象插入序，手工拼串保证逐字节稳定）。
fn serialized_source(source: &RecoveryChapterSource) -> String {
    let esc = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string());
    let arr = serde_json::to_string(&source.characters).unwrap_or_else(|_| "[]".to_string());
    format!(
        "{{\"chapterNumber\":{},\"title\":{},\"role\":{},\"purpose\":{},\"keyEvents\":{},\"characters\":{},\"suspenseHook\":{},\"userGuidance\":{}}}",
        source.chapter_number,
        esc(&source.title),
        esc(&source.role),
        esc(&source.purpose),
        esc(&source.key_events),
        arr,
        esc(source.suspense_hook.as_deref().unwrap_or("")),
        esc(source.user_guidance.as_deref().unwrap_or("")),
    )
}

/// 对齐基线 `visibleOnly`：剥除 `<think>` 思考链，只留可见正文。
///
/// 基线三条规则：
/// 1. 成对 `<think>…</think>`（或未闭合直到结尾）整体移除；
/// 2. 若残留**孤儿闭合标签** `</think>`，只保留其后的内容；
/// 3. 剩余孤立 `<think>` / `</think>` 标签全部移除后 trim。
pub(crate) fn visible_only(text: &str) -> String {
    let re_block = regex::Regex::new(r"(?is)<think>[\s\S]*?(?:</think>|$)").unwrap();
    let without_paired_thinking = re_block.replace_all(text, "").to_string();
    let re_orphan = regex::Regex::new(r"(?i)</think>").unwrap();
    let re_tags = regex::Regex::new(r"(?i)</?think>").unwrap();
    match re_orphan.find(&without_paired_thinking) {
        None => re_tags
            .replace_all(&without_paired_thinking, "")
            .trim()
            .to_string(),
        Some(m) => re_tags
            .replace_all(&without_paired_thinking[m.end()..], "")
            .trim()
            .to_string(),
    }
}

/// 对齐基线 `sourceIsCurrent`：蓝图未变 且 草稿身份未被推进
fn source_is_current(conn: &Connection, row: &CandidateRow) -> Result<bool, String> {
    if row.source_draft_identity_captured != 1 {
        return Ok(false);
    }
    let current = blueprints::get_by_chapter(conn, row.chapter_number)?;
    let Some(current) = current else {
        return Ok(false);
    };
    let blueprint_current = sha256_hex(&serialized_source(&RecoveryChapterSource {
        chapter_number: current.chapter_number,
        title: current.title.clone(),
        role: current.role.clone(),
        purpose: current.purpose.clone(),
        key_events: current.key_events.clone(),
        characters: current.characters.clone(),
        suspense_hook: Some(current.suspense_hook.clone()),
        user_guidance: Some(current.user_guidance.clone()),
    })) == row.source_hash;
    if !blueprint_current {
        return Ok(false);
    }

    let current_draft = drafts::get_latest_by_chapter(conn, row.chapter_number)?;
    if row.source_draft_id.is_none() && row.source_draft_version.is_none() {
        return Ok(current_draft.is_none());
    }
    Ok(match current_draft {
        Some(d) => {
            Some((d.id, d.version))
                == row
                    .source_draft_id
                    .zip(row.source_draft_version)
                    .map(|(id, v)| (id, v))
        }
        None => false,
    })
}

/// 对齐基线 `toCandidate`：读取时强制完整性校验（快照 / 正文哈希逐字节比对）
fn to_candidate(row: &CandidateRow) -> Result<RecoveryCandidate, String> {
    if sha256_hex(&row.source_snapshot) != row.source_hash
        || sha256_hex(&row.visible_text) != row.content_hash
    {
        return Err("恢复候选完整性校验失败".to_string());
    }
    Ok(RecoveryCandidate {
        candidate_id: row.candidate_id.clone(),
        run_id: row.run_id.clone(),
        step_id: row.step_id.clone(),
        project_id: row.project_id.clone(),
        chapter_number: row.chapter_number,
        chapter_title: row.chapter_title.clone(),
        source_hash: row.source_hash.clone(),
        visible_text: row.visible_text.clone(),
        content_hash: row.content_hash.clone(),
        failure_code: row.failure_code.clone(),
        failure_reason: row.failure_reason.clone(),
        status: row.status.clone(),
        replaces_candidate_id: row.replaces_candidate_id.clone(),
        // 由带连接的包装覆盖（时效校验需要查蓝图/草稿）
        source_current: false,
        created_at: row.created_at.clone(),
        resolved_at: row.resolved_at.clone(),
    })
}

fn to_candidate_with_conn(
    conn: &Connection,
    row: &CandidateRow,
) -> Result<RecoveryCandidate, String> {
    let mut candidate = to_candidate(row)?;
    candidate.source_current = source_is_current(conn, row)?;
    Ok(candidate)
}

fn get_row(conn: &Connection, candidate_id: &str) -> Result<Option<CandidateRow>, String> {
    let sql = format!("{ROW_SELECT} WHERE candidate_id = ?1");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map([candidate_id], row_from)
        .map_err(|e| e.to_string())?;
    rows.next().transpose().map_err(|e| e.to_string())
}

fn get_pending_row(conn: &Connection, candidate_id: &str) -> Result<Option<CandidateRow>, String> {
    let sql = format!("{ROW_SELECT} WHERE candidate_id = ?1 AND status = 'pending'");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map([candidate_id], row_from)
        .map_err(|e| e.to_string())?;
    rows.next().transpose().map_err(|e| e.to_string())
}

// ===== 领域操作（对齐基线四个静态方法） =====

/// 基线 `RecoveryCandidateRepository.record`
pub fn record(
    conn: &Connection,
    project_id: &str,
    request: &RecoveryCandidateRecordRequest,
) -> Result<RecoveryCandidate, String> {
    let run_id = assert_text(&request.run_id, "候选任务身份", MAX_IDENTITY_TEXT)?;
    let step_id = assert_text(&request.step_id, "候选步骤身份", MAX_IDENTITY_TEXT)?;
    let project_id = assert_text(project_id, "候选项目身份", MAX_IDENTITY_TEXT)?;
    if request.chapter_number < 1 {
        return Err("候选章节身份无效".to_string());
    }
    if request.source.chapter_number != request.chapter_number {
        return Err("候选源章节身份不一致".to_string());
    }
    if let Some(draft) = &request.source_draft {
        if draft.id < 1 || draft.version < 1 {
            return Err("候选源草稿身份无效".to_string());
        }
    }
    let visible_text = visible_only(&request.visible_text);
    if visible_text.is_empty() {
        return Err("恢复候选没有可见正文".to_string());
    }
    let serialized = serialized_source(&request.source);
    let candidate_id = new_candidate_id();

    if let Some(replaces_id) = &request.replaces_candidate_id {
        let replaced = get_row(conn, replaces_id)?;
        let valid = replaced.as_ref().is_some_and(|r| {
            r.run_id == run_id
                && r.project_id == project_id
                && r.chapter_number == request.chapter_number
        });
        if !valid {
            return Err("替代候选关系无效".to_string());
        }
    }

    conn.execute(
        "INSERT INTO recovery_candidates (
            candidate_id, run_id, step_id, project_id, chapter_number,
            chapter_title, source_snapshot, source_hash, source_draft_id,
            source_draft_version, source_draft_identity_captured, visible_text,
            content_hash, failure_code, failure_reason, replaces_candidate_id
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            candidate_id,
            run_id,
            step_id,
            project_id,
            request.chapter_number,
            request.chapter_title.trim(),
            serialized,
            sha256_hex(&serialized),
            request.source_draft.as_ref().map(|d| d.id),
            request.source_draft.as_ref().map(|d| d.version),
            visible_text,
            sha256_hex(&visible_text),
            request
                .failure_code
                .trim()
                .chars()
                .take(MAX_IDENTITY_TEXT)
                .collect::<String>(),
            request
                .failure_reason
                .trim()
                .chars()
                .take(MAX_FAILURE_REASON)
                .collect::<String>(),
            request.replaces_candidate_id,
        ],
    )
    .map_err(|e| e.to_string())?;

    let row = get_row(conn, &candidate_id)?.ok_or("恢复候选写入后读取失败")?;
    to_candidate_with_conn(conn, &row)
}

/// 基线 `RecoveryCandidateRepository.listPending`
pub fn list_pending(conn: &Connection) -> Result<Vec<RecoveryCandidate>, String> {
    let sql = format!("{ROW_SELECT} WHERE status = 'pending' ORDER BY rowid ASC");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_from)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    rows.iter()
        .map(|r| to_candidate_with_conn(conn, r))
        .collect()
}

/// 基线 `RecoveryCandidateRepository.updatePending`
pub fn update_pending(
    conn: &Connection,
    candidate_id: &str,
    text: &str,
) -> Result<RecoveryCandidate, String> {
    let id = assert_text(candidate_id, "恢复候选身份", MAX_IDENTITY_TEXT)?;
    let row = get_pending_row(conn, &id)?.ok_or("恢复候选不存在或已处理")?;
    to_candidate_with_conn(conn, &row)?;
    if !source_is_current(conn, &row)? {
        return Err("恢复候选的源章节已变化，已拒绝保存".to_string());
    }
    let visible_text = visible_only(text);
    if visible_text.is_empty() {
        return Err("恢复候选没有可见正文".to_string());
    }
    conn.execute(
        "UPDATE recovery_candidates SET visible_text = ?1, content_hash = ?2
         WHERE candidate_id = ?3 AND status = 'pending'",
        rusqlite::params![visible_text, sha256_hex(&visible_text), id],
    )
    .map_err(|e| e.to_string())?;

    let row = get_row(conn, &id)?.ok_or("恢复候选不存在或已处理")?;
    to_candidate_with_conn(conn, &row)
}

/// 基线 `RecoveryCandidateRepository.resolve`（status 仅接受 continued / discarded）
pub fn resolve(conn: &Connection, candidate_id: &str, status: &str) -> Result<(), String> {
    if status != STATUS_CONTINUED && status != STATUS_DISCARDED {
        return Err("恢复候选动作无效".to_string());
    }
    if status == STATUS_CONTINUED {
        let row = get_pending_row(conn, candidate_id)?.ok_or("恢复候选不存在或已处理")?;
        if !source_is_current(conn, &row)? {
            return Err("恢复候选的源章节已变化，已拒绝继续".to_string());
        }
    }
    let changed = conn
        .execute(
            "UPDATE recovery_candidates
             SET status = ?1, resolved_at = datetime('now')
             WHERE candidate_id = ?2 AND status = 'pending'",
            rusqlite::params![status, candidate_id],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("恢复候选不存在或已处理".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        conn
    }

    fn request(chapter: i64) -> RecoveryCandidateRecordRequest {
        RecoveryCandidateRecordRequest {
            run_id: "run-1".to_string(),
            step_id: "step-1".to_string(),
            chapter_number: chapter,
            chapter_title: "第一章".to_string(),
            source: RecoveryChapterSource {
                chapter_number: chapter,
                title: "标题".to_string(),
                role: "main".to_string(),
                purpose: "推进".to_string(),
                key_events: "事件A".to_string(),
                characters: vec!["甲".to_string()],
                suspense_hook: Some("钩子".to_string()),
                user_guidance: None,
            },
            source_draft: None,
            visible_text: "<think>思考链</think>正文内容".to_string(),
            failure_code: "TIMEOUT".to_string(),
            failure_reason: "生成超时".to_string(),
            replaces_candidate_id: None,
        }
    }

    fn matching_blueprint(chapter: i64) -> blueprints::BlueprintData {
        blueprints::BlueprintData {
            chapter_number: chapter,
            title: "标题".to_string(),
            role: "main".to_string(),
            purpose: "推进".to_string(),
            key_events: "事件A".to_string(),
            characters: vec!["甲".to_string()],
            new_character_candidates: None,
            relationship_hints: None,
            suspense_hook: "钩子".to_string(),
            user_guidance: String::new(),
            notes: String::new(),
            notes_updated_at: String::new(),
        }
    }

    #[test]
    fn record_strips_thinking_and_verifies_hash_test() {
        let conn = memory_db();
        let candidate = record(&conn, "proj-1", &request(1)).unwrap();
        assert_eq!(candidate.visible_text, "正文内容");
        assert_eq!(candidate.content_hash, sha256_hex("正文内容"));
        assert_eq!(candidate.status, "pending");
        assert_eq!(candidate.project_id, "proj-1");
        assert!(!candidate.source_current, "无蓝图时源不时效");

        // 列表回读时哈希完整性校验 + sourceCurrent 仍为 false
        let listed = list_pending(&conn).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].candidate_id, candidate.candidate_id);
    }

    #[test]
    fn source_current_tracks_blueprint_test() {
        let conn = memory_db();
        blueprints::upsert(&conn, &matching_blueprint(1)).unwrap();
        let candidate = record(&conn, "proj-1", &request(1)).unwrap();
        assert!(candidate.source_current, "蓝图一致时应为时效");

        // 蓝图漂移 → 不再时效（title 参与时效哈希；notes 不参与）
        let mut drifted = matching_blueprint(1);
        drifted.title = "标题改".to_string();
        blueprints::upsert(&conn, &drifted).unwrap();
        let listed = list_pending(&conn).unwrap();
        assert!(!listed[0].source_current, "蓝图漂移后源不时效");
    }

    #[test]
    fn update_and_resolve_state_machine_test() {
        let conn = memory_db();
        blueprints::upsert(&conn, &matching_blueprint(1)).unwrap();
        let candidate = record(&conn, "proj-1", &request(1)).unwrap();

        // 更新可见正文（哈希同步重算）
        let updated = update_pending(&conn, &candidate.candidate_id, "修订后正文").unwrap();
        assert_eq!(updated.visible_text, "修订后正文");
        assert_eq!(updated.content_hash, sha256_hex("修订后正文"));

        // continued 需要源时效：蓝图未被推动 → 可继续
        resolve(&conn, &candidate.candidate_id, STATUS_CONTINUED).unwrap();

        // 终态后不可再更新/再处理
        assert_eq!(
            update_pending(&conn, &candidate.candidate_id, "再改").unwrap_err(),
            "恢复候选不存在或已处理"
        );
        assert_eq!(
            resolve(&conn, &candidate.candidate_id, STATUS_DISCARDED).unwrap_err(),
            "恢复候选不存在或已处理"
        );
    }

    #[test]
    fn resolve_rejects_stale_source_for_continued_test() {
        let conn = memory_db();
        let candidate = record(&conn, "proj-1", &request(2)).unwrap();
        // 无蓝图 → 源不时效，continued 拒绝；discarded 不要求时效
        assert_eq!(
            resolve(&conn, &candidate.candidate_id, STATUS_CONTINUED).unwrap_err(),
            "恢复候选的源章节已变化，已拒绝继续"
        );
        resolve(&conn, &candidate.candidate_id, STATUS_DISCARDED).unwrap();
    }

    #[test]
    fn replace_relation_and_input_validation_test() {
        let conn = memory_db();
        // 替代不存在的候选 → 拒绝
        let mut req = request(1);
        req.replaces_candidate_id = Some("no-such-id".to_string());
        assert_eq!(
            record(&conn, "proj-1", &req).unwrap_err(),
            "替代候选关系无效"
        );

        // 合法替代链：同 run/project/章节
        let first = record(&conn, "proj-1", &request(1)).unwrap();
        let mut second = request(1);
        second.replaces_candidate_id = Some(first.candidate_id.clone());
        let replaced = record(&conn, "proj-1", &second).unwrap();
        assert_eq!(
            replaced.replaces_candidate_id.as_deref(),
            Some(first.candidate_id.as_str())
        );

        // 身份校验
        let mut bad = request(1);
        bad.run_id = "  ".to_string();
        assert_eq!(
            record(&conn, "proj-1", &bad).unwrap_err(),
            "候选任务身份无效"
        );
        let mut mismatch = request(1);
        mismatch.source.chapter_number = 9;
        assert_eq!(
            record(&conn, "proj-1", &mismatch).unwrap_err(),
            "候选源章节身份不一致"
        );
        let mut empty_text = request(1);
        empty_text.visible_text = "<think>只有思考</think>".to_string();
        assert_eq!(
            record(&conn, "proj-1", &empty_text).unwrap_err(),
            "恢复候选没有可见正文"
        );
        assert_eq!(resolve(&conn, "x", "nope").unwrap_err(), "恢复候选动作无效");
    }
}
