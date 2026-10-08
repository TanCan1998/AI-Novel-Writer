//! PlotTreeRepository —— 剧情树派生快照（`project_core.plot_tree_snapshot` 列）
//!
//! 平移自 `electron/repositories/plot-tree-repository.ts`（批次 F1）。领域要点：
//! - **快照是派生投影，绝不反向覆盖作者事实**：只读写 `project_core.plot_tree_snapshot`
//!   这一个缓存列，来源（梗概 / 蓝图 / 定稿章节 / 叙事线索）始终是只读事实源；
//! - **乐观锁**：`sourceRevision` = `JSON.stringify(来源事实集)` 的 SHA-256。
//!   保存时若与调用方持有的版本不一致，说明「生成期间剧情资料已更新」，
//!   拒绝写入（命令层翻译为 `errorCode: 'sources-changed'`）；
//! - **损坏快照隔离而非删除**：存量快照解析或结构校验失败时，
//!   只回传 `storedSnapshotInvalid` 标记 + `snapshot: null`，**不修改库内容**。
//!
//! `sourceRevision` 的字节口径必须与基线 `JSON.stringify` 一致 ——
//! 事实集结构体的字段顺序即 JSON 键顺序，故**顺序不可调整**；
//! `source_revision_golden_test` 用基线实测输出锁定该口径。

use rusqlite::Connection;
use serde::Serialize;

use crate::plot_tree::{
    assert_plot_tree_snapshot, assert_stored_plot_tree_snapshot, PlotTreeBlueprintFact,
    PlotTreeFinalizedChapterFact, PlotTreeNarrativeThreadEventFact, PlotTreeNarrativeThreadFact,
    PlotTreeSnapshot, PlotTreeSourceFacts, PlotTreeSynopsisFact,
};
use crate::repositories::character_roster_repository::hash_text;
use crate::repositories::narrative_thread_repository as threads;
use crate::repositories::project_core_repository::resolve_writing_language;

/// 剧情树来源包（对齐 `PlotTreeSourceBundle`）
///
/// 序列化形状 = 事实集（展平）+ `sourceRevision` + `snapshot` + 可选
/// `storedSnapshotInvalid`，与基线的对象展开顺序一致。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotTreeSourceBundle {
    #[serde(flatten)]
    pub facts: PlotTreeSourceFacts,
    pub source_revision: String,
    /// 无快照时为 `null`（基线恒含该键，故不使用 `skip_serializing_if`）
    pub snapshot: Option<PlotTreeSnapshot>,
    /// 存量快照损坏时为 `Some(true)`（仅在为真时出现）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_snapshot_invalid: Option<bool>,
}

/// 来源事实集的 `sourceRevision`：`JSON.stringify(facts)` 的 SHA-256 小写 hex
pub fn source_revision(facts: &PlotTreeSourceFacts) -> Result<String, String> {
    let json = serde_json::to_string(facts)
        .map_err(|error| format!("序列化剧情树来源失败：{error}"))?;
    Ok(hash_text(&json))
}

/// 组装来源事实集（`project_core` 缺失即报错，对齐基线 `项目配置不存在`）
fn read_source_facts(conn: &Connection) -> Result<PlotTreeSourceFacts, String> {
    let core = conn
        .query_row(
            "SELECT writing_language, synopsis, plot_tree_snapshot
             FROM project_core WHERE id = 'main'",
            [],
            |row| {
                Ok((
                    row.get::<_, Option<String>>("writing_language")?.unwrap_or_default(),
                    row.get::<_, Option<String>>("synopsis")?.unwrap_or_default(),
                    row.get::<_, Option<String>>("plot_tree_snapshot")?.unwrap_or_default(),
                ))
            },
        )
        .ok();
    let Some((writing_language, synopsis, _)) = core else {
        return Err("项目配置不存在".to_string());
    };

    let mut blueprint_statement = conn
        .prepare(
            "SELECT chapter_number, title, purpose, key_events
             FROM blueprints ORDER BY chapter_number ASC",
        )
        .map_err(|error| format!("读取章节蓝图失败：{error}"))?;
    let blueprints = blueprint_statement
        .query_map([], |row| {
            Ok(PlotTreeBlueprintFact {
                chapter_number: row.get("chapter_number")?,
                title: row.get::<_, Option<String>>("title")?.unwrap_or_default(),
                purpose: row.get::<_, Option<String>>("purpose")?.unwrap_or_default(),
                key_events: row
                    .get::<_, Option<String>>("key_events")?
                    .unwrap_or_default(),
            })
        })
        .map_err(|error| format!("读取章节蓝图失败：{error}"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("读取章节蓝图失败：{error}"))?;

    // 「每章最新定稿」：同章取更高版本；版本相同取更大 id（对齐基线 NOT EXISTS 子查询）
    let mut finalized_statement = conn
        .prepare(
            "SELECT drafts.id AS draft_id,
                    drafts.chapter_number,
                    COALESCE(finalization_outbox.chapter_title, blueprints.title, '') AS title,
                    COALESCE(
                      NULLIF(summary_snapshots.chapter_notes, ''),
                      CASE WHEN TRIM(COALESCE(blueprints.notes_updated_at, '')) <> ''
                        THEN blueprints.notes ELSE '' END
                    ) AS summary
             FROM drafts
             LEFT JOIN finalization_outbox ON finalization_outbox.draft_id = drafts.id
             LEFT JOIN summary_snapshots ON summary_snapshots.draft_id = drafts.id
             LEFT JOIN blueprints ON blueprints.chapter_number = drafts.chapter_number
             WHERE drafts.status = 'finalized'
               AND NOT EXISTS (
                 SELECT 1 FROM drafts newer
                 WHERE newer.chapter_number = drafts.chapter_number
                   AND newer.status = 'finalized'
                   AND (newer.version > drafts.version
                     OR (newer.version = drafts.version AND newer.id > drafts.id))
               )
             ORDER BY drafts.chapter_number ASC",
        )
        .map_err(|error| format!("读取已定稿章节失败：{error}"))?;
    let finalized_chapters = finalized_statement
        .query_map([], |row| {
            Ok(PlotTreeFinalizedChapterFact {
                draft_id: row.get("draft_id")?,
                chapter_number: row.get("chapter_number")?,
                title: row.get::<_, Option<String>>("title")?.unwrap_or_default(),
                summary: row.get::<_, Option<String>>("summary")?.unwrap_or_default(),
            })
        })
        .map_err(|error| format!("读取已定稿章节失败：{error}"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("读取已定稿章节失败：{error}"))?;

    // 叙事线索：只取基线 `read()` 用到的 8 个字段（状态为派生值，事件只留 5 键）
    let narrative_threads = threads::list(conn)?
        .into_iter()
        .map(|thread| PlotTreeNarrativeThreadFact {
            id: thread.id,
            title: thread.title,
            thread_type: thread.thread_type,
            target_start_chapter: thread.target_start_chapter,
            target_end_chapter: thread.target_end_chapter,
            author_intent: thread.author_intent,
            status: thread.status,
            events: thread
                .events
                .into_iter()
                .map(|event| PlotTreeNarrativeThreadEventFact {
                    id: event.id,
                    chapter_number: event.chapter_number,
                    event_type: event.event_type,
                    evidence: event.evidence,
                    reason: event.reason,
                })
                .collect(),
        })
        .collect();

    Ok(PlotTreeSourceFacts {
        writing_language: resolve_writing_language(&writing_language),
        synopsis: PlotTreeSynopsisFact { content: synopsis },
        blueprints,
        finalized_chapters,
        narrative_threads,
    })
}

/// 读取剧情树来源包（含存量快照；损坏快照只标记不删除）
pub fn read(conn: &Connection) -> Result<PlotTreeSourceBundle, String> {
    let facts = read_source_facts(conn)?;
    let revision = source_revision(&facts)?;

    let stored: String = conn
        .query_row(
            "SELECT plot_tree_snapshot FROM project_core WHERE id = 'main'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|error| format!("读取剧情树快照失败：{error}"))?
        .unwrap_or_default();

    let mut snapshot = None;
    let mut stored_snapshot_invalid = None;
    if !stored.is_empty() {
        match serde_json::from_str::<serde_json::Value>(&stored)
            .ok()
            .and_then(|value| assert_stored_plot_tree_snapshot(&value).ok())
        {
            Some(parsed) => snapshot = Some(parsed),
            None => stored_snapshot_invalid = Some(true),
        }
    }

    Ok(PlotTreeSourceBundle {
        facts,
        source_revision: revision,
        snapshot,
        stored_snapshot_invalid,
    })
}

/// 保存快照（乐观锁）：来源版本不匹配时报 `剧情资料在生成期间已更新`
pub fn save(
    conn: &Connection,
    snapshot: &serde_json::Value,
    expected_source_revision: &str,
) -> Result<PlotTreeSnapshot, String> {
    let bundle = read(conn)?;
    if bundle.source_revision != expected_source_revision {
        return Err("剧情资料在生成期间已更新".to_string());
    }
    let validated = assert_plot_tree_snapshot(snapshot, &bundle.facts, &bundle.source_revision)?;
    let payload = serde_json::to_string(&validated)
        .map_err(|error| format!("序列化剧情树快照失败：{error}"))?;
    let changes = conn
        .execute(
            "UPDATE project_core SET plot_tree_snapshot = ?1 WHERE id = 'main'",
            [payload],
        )
        .map_err(|error| format!("保存剧情树快照失败：{error}"))?;
    if changes != 1 {
        return Err("剧情树快照保存失败".to_string());
    }
    Ok(validated)
}

/// 清除快照（只清缓存列，不动来源事实）
pub fn clear(conn: &Connection) -> Result<(), String> {
    let changes = conn
        .execute(
            "UPDATE project_core SET plot_tree_snapshot = '' WHERE id = 'main'",
            [],
        )
        .map_err(|error| format!("清除剧情树快照失败：{error}"))?;
    if changes != 1 {
        return Err("剧情树快照清除失败".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn memory_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库应可打开");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("应启用外键");
        schema::create_tables(&conn).expect("建表应成功");
        conn
    }

    /// 与 `plot_tree.rs` 测试夹具一致的事实集（用于黄金口径校验）
    fn golden_facts() -> PlotTreeSourceFacts {
        PlotTreeSourceFacts {
            writing_language: "zh-CN".to_string(),
            synopsis: PlotTreeSynopsisFact {
                content: "梗概".to_string(),
            },
            blueprints: vec![PlotTreeBlueprintFact {
                chapter_number: 1,
                title: "标题".to_string(),
                purpose: "目的".to_string(),
                key_events: "事件".to_string(),
            }],
            finalized_chapters: vec![PlotTreeFinalizedChapterFact {
                draft_id: 7,
                chapter_number: 2,
                title: "第二章".to_string(),
                summary: "摘要".to_string(),
            }],
            narrative_threads: vec![PlotTreeNarrativeThreadFact {
                id: 3,
                title: "线索".to_string(),
                thread_type: "foreshadow".to_string(),
                target_start_chapter: 1,
                target_end_chapter: 5,
                author_intent: "意图".to_string(),
                status: "planted".to_string(),
                events: vec![PlotTreeNarrativeThreadEventFact {
                    id: 9,
                    chapter_number: 2,
                    event_type: "planted".to_string(),
                    evidence: "证据".to_string(),
                    reason: "理由".to_string(),
                }],
            }],
        }
    }

    /// **黄金测试**：`serde_json` 的序列化字节必须与 Node `JSON.stringify` 完全一致。
    ///
    /// 期望串由基线算法实测得到（`JSON.stringify` 后逐字节记录），
    /// 锁死「键顺序 + 转义规则 + 非 ASCII 不转义」三项口径 ——
    /// 任何一项漂移都会让 `sourceRevision` 与 Electron 侧不再同值。
    #[test]
    fn source_revision_golden_test() {
        let json = serde_json::to_string(&golden_facts()).expect("应可序列化");
        assert_eq!(
            json,
            r#"{"writingLanguage":"zh-CN","synopsis":{"content":"梗概"},"blueprints":[{"chapterNumber":1,"title":"标题","purpose":"目的","keyEvents":"事件"}],"finalizedChapters":[{"draftId":7,"chapterNumber":2,"title":"第二章","summary":"摘要"}],"narrativeThreads":[{"id":3,"title":"线索","type":"foreshadow","targetStartChapter":1,"targetEndChapter":5,"authorIntent":"意图","status":"planted","events":[{"id":9,"chapterNumber":2,"type":"planted","evidence":"证据","reason":"理由"}]}]}"#
        );
        let revision = source_revision(&golden_facts()).unwrap();
        assert_eq!(revision.len(), 64);
        assert!(revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    }

    /// 转义口径逐项核对（对应 Node `JSON.stringify` 的实测行为）
    #[test]
    fn json_escaping_matches_json_stringify_test() {
        let tricky = "\"quote\" \\back\\ \n\t\u{8}\u{c}\r \u{1}\u{1f} 😀 \u{FEFF} \u{2028}\u{2029} \u{85} /slash/";
        let facts = PlotTreeSourceFacts {
            writing_language: "zh-CN".to_string(),
            synopsis: PlotTreeSynopsisFact {
                content: tricky.to_string(),
            },
            blueprints: vec![],
            finalized_chapters: vec![],
            narrative_threads: vec![],
        };
        let json = serde_json::to_string(&facts).unwrap();
        // 控制字符使用短转义（与 JSON.stringify 一致）
        assert!(json.contains(r#"\n"#), "换行应为 \\n");
        assert!(json.contains(r#"\t"#), "制表应为 \\t");
        assert!(json.contains(r#"\b"#), "退格应为 \\b");
        assert!(json.contains(r#"\f"#), "换页应为 \\f");
        assert!(json.contains(r#"\r"#), "回车应为 \\r");
        // 其余控制字符用 \uXXXX 小写四位
        assert!(json.contains(r#"\u0001"#), "U+0001 应为 \\u0001");
        assert!(json.contains(r#"\u001f"#), "U+001F 应为 \\u001f");
        // 引号与反斜杠转义
        assert!(json.contains(r#"\"quote\""#));
        assert!(json.contains(r#"\\back\\"#));
        // 非 ASCII 一律原样输出（含增补平面、BOM、U+2028/2029、U+0085）
        assert!(json.contains('😀'), "增补平面字符不得转义");
        assert!(json.contains('\u{FEFF}'), "BOM 不得转义");
        assert!(json.contains('\u{2028}'), "U+2028 不得转义");
        assert!(json.contains('\u{2029}'), "U+2029 不得转义");
        assert!(json.contains('\u{85}'), "U+0085 不得转义");
        assert!(!json.contains(r#"\u2028"#));
        // 正斜杠不转义
        assert!(json.contains("/slash/"));
        assert!(!json.contains(r#"\/"#));
    }

    fn seed_project(conn: &Connection) {
        conn.execute(
            "INSERT INTO project_core (id, project_name, writing_language, synopsis)
             VALUES ('main', '项目', 'zh-CN', '梗概')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn read_without_project_core_is_rejected_test() {
        let conn = memory_conn();
        assert_eq!(read(&conn).unwrap_err(), "项目配置不存在");
    }

    #[test]
    fn read_empty_project_has_no_snapshot_test() {
        let conn = memory_conn();
        seed_project(&conn);
        let bundle = read(&conn).unwrap();
        assert!(bundle.snapshot.is_none());
        assert_eq!(bundle.stored_snapshot_invalid, None);
        assert_eq!(bundle.source_revision.len(), 64);
        assert!(bundle.facts.blueprints.is_empty());
        assert!(bundle.facts.finalized_chapters.is_empty());
        assert!(bundle.facts.narrative_threads.is_empty());
        // 无任何来源 → 无法支撑事件
        assert!(!crate::plot_tree::has_usable_plot_tree_event_source(
            &bundle.facts
        ));
    }

    #[test]
    fn read_normalizes_invalid_writing_language_test() {
        let conn = memory_conn();
        conn.execute(
            "INSERT INTO project_core (id, project_name, writing_language, synopsis)
             VALUES ('main', '项目', 'fr-FR', '梗概')",
            [],
        )
        .unwrap();
        let bundle = read(&conn).unwrap();
        assert_eq!(bundle.facts.writing_language, "zh-CN", "非法语言回落默认");
    }

    #[test]
    fn read_collects_sources_and_matches_revision_test() {
        let conn = memory_conn();
        seed_project(&conn);
        conn.execute(
            "INSERT INTO blueprints (chapter_number, title, purpose, key_events)
             VALUES (1, '第一章', '开端', '事件')",
            [],
        )
        .unwrap();
        let content_id = crate::repositories::content_repository::create(&conn, "正文").unwrap();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id, word_count)
             VALUES (2, 1, 'finalized', ?1, 2)",
            [content_id],
        )
        .unwrap();
        let draft_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO finalization_outbox (
               finalization_id, draft_id, chapter_number, chapter_title, content_hash,
               content_revision, content_snapshot, target_file_name, publication_status
             ) VALUES ('fin-1', ?1, 2, '第二章', 'hash', 1, '正文', 'f.md', 'published')",
            [draft_id],
        )
        .unwrap();

        let bundle = read(&conn).unwrap();
        assert_eq!(bundle.facts.blueprints.len(), 1);
        assert_eq!(bundle.facts.blueprints[0].chapter_number, 1);
        assert_eq!(bundle.facts.finalized_chapters.len(), 1);
        assert_eq!(bundle.facts.finalized_chapters[0].draft_id, draft_id);
        assert_eq!(bundle.facts.finalized_chapters[0].title, "第二章");
        // 版本号可自洽复算
        assert_eq!(
            bundle.source_revision,
            source_revision(&bundle.facts).unwrap()
        );
    }

    #[test]
    fn read_uses_latest_finalized_per_chapter_test() {
        let conn = memory_conn();
        seed_project(&conn);
        for (version, body) in [(1, "旧稿"), (2, "新稿")] {
            let content_id = crate::repositories::content_repository::create(&conn, body).unwrap();
            conn.execute(
                "INSERT INTO drafts (chapter_number, version, status, content_id, word_count)
                 VALUES (3, ?1, 'finalized', ?2, 2)",
                rusqlite::params![version, content_id],
            )
            .unwrap();
        }
        let bundle = read(&conn).unwrap();
        assert_eq!(
            bundle.facts.finalized_chapters.len(),
            1,
            "同章多版本定稿只取最新一条"
        );
    }

    #[test]
    fn read_flags_corrupt_stored_snapshot_without_deleting_test() {
        let conn = memory_conn();
        seed_project(&conn);
        conn.execute(
            "UPDATE project_core SET plot_tree_snapshot = 'not-json' WHERE id = 'main'",
            [],
        )
        .unwrap();
        let bundle = read(&conn).unwrap();
        assert!(bundle.snapshot.is_none());
        assert_eq!(bundle.stored_snapshot_invalid, Some(true));
        let stored: String = conn
            .query_row(
                "SELECT plot_tree_snapshot FROM project_core WHERE id = 'main'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "not-json", "损坏快照必须原样保留，不得清除");
    }

    #[test]
    fn read_flags_structurally_invalid_snapshot_test() {
        let conn = memory_conn();
        seed_project(&conn);
        conn.execute(
            "UPDATE project_core SET plot_tree_snapshot = '{\"version\":2}' WHERE id = 'main'",
            [],
        )
        .unwrap();
        let bundle = read(&conn).unwrap();
        assert!(bundle.snapshot.is_none());
        assert_eq!(bundle.stored_snapshot_invalid, Some(true));
    }

    #[test]
    fn save_rejects_stale_source_revision_test() {
        let conn = memory_conn();
        seed_project(&conn);
        let error = save(&conn, &serde_json::json!({}), &"a".repeat(64)).unwrap_err();
        assert_eq!(error, "剧情资料在生成期间已更新");
    }

    #[test]
    fn save_rejects_invalid_expected_revision_shape_test() {
        let conn = memory_conn();
        seed_project(&conn);
        // 形状非法由命令层先行拒绝；仓储层只做版本比较
        let error = save(&conn, &serde_json::json!({}), "not-a-hash").unwrap_err();
        assert_eq!(error, "剧情资料在生成期间已更新");
    }

    #[test]
    fn clear_removes_snapshot_only_test() {
        let conn = memory_conn();
        seed_project(&conn);
        conn.execute(
            "UPDATE project_core SET plot_tree_snapshot = '{}' WHERE id = 'main'",
            [],
        )
        .unwrap();
        clear(&conn).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT plot_tree_snapshot FROM project_core WHERE id = 'main'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(stored.is_empty());
        // 来源事实（梗概）不受影响
        let synopsis: String = conn
            .query_row("SELECT synopsis FROM project_core WHERE id = 'main'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(synopsis, "梗概");
    }

    #[test]
    fn clear_without_project_core_reports_failure_test() {
        let conn = memory_conn();
        assert_eq!(clear(&conn).unwrap_err(), "剧情树快照清除失败");
    }

    #[test]
    fn bundle_serialization_shape_test() {
        let conn = memory_conn();
        seed_project(&conn);
        let bundle = read(&conn).unwrap();
        // ⚠️ 必须用 `to_string` 断言键顺序：`to_value` 会按字母序重排（BTreeMap），
        // 而 `#[serde(flatten)]` 的真实输出顺序只有 `to_string` 才能校验。
        let json = serde_json::to_string(&bundle).unwrap();
        assert!(
            json.starts_with(
                concat!(
                    r#"{"writingLanguage":"zh-CN","synopsis":{"content":"梗概"},"#,
                    r#""blueprints":[],"finalizedChapters":[],"narrativeThreads":[],"#,
                    r#""sourceRevision":"#
                )
            ),
            "键顺序须与基线对象展开一致：{json}"
        );
        assert!(
            json.ends_with(r#","snapshot":null}"#),
            "`snapshot` 恒存在且未设置时为 null：{json}"
        );
        assert!(
            !json.contains("storedSnapshotInvalid"),
            "未损坏时不得出现该键"
        );

        // 损坏标记出现的位置与形状
        conn.execute(
            "UPDATE project_core SET plot_tree_snapshot = 'bad' WHERE id = 'main'",
            [],
        )
        .unwrap();
        let bundle = read(&conn).unwrap();
        let json = serde_json::to_string(&bundle).unwrap();
        assert!(
            json.ends_with(r#","snapshot":null,"storedSnapshotInvalid":true}"#),
            "损坏标记须在 snapshot 之后：{json}"
        );
    }
}
