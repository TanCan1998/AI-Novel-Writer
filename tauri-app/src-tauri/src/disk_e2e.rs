//! 磁盘级端到端回归 —— 真实项目目录 + 真实 `.lore/lorekeeper.db`。
//!
//! 与各仓储的单元测试（内存库）不同，本模块覆盖「真机等价」路径：
//! 文件库 + WAL + `foreign_keys=ON` + 真实目录上的物理文件 + **跨连接重开后的持久化**。
//! 它是 GUI 冒烟（`pnpm tauri dev` 打开真实项目）的可自动化对应物：命令层本身
//! 只在参数/信封上做机械转换，真正的持久化语义由这些仓储决定。
//!
//! 覆盖链路（单次会话顺序执行，模拟创作流程）：
//! 建项目主台账 → 蓝图范围提交 → 蓝图角色同步 → 草稿 → 修稿合并 → 审稿
//! → 后处理跑批 → LLM 日志与统计 → 生成数据清理。

use std::path::PathBuf;

use crate::db::{project_database_path, ProjectDatabase};
use crate::draft_source_guard::ExpectedDraftSource;
use crate::repositories::{
    blueprint_repository as blueprints, character_roster_repository as roster,
    draft_repository as drafts, llm_repository as llm, post_process_repository as post_process,
    project_clear_repository as project_clear, project_core_repository as project_core,
    review_repository as reviews, revision_repository as revisions,
};

fn temp_project(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "anw-disk-e2e-{name}-{}",
        crate::project_access::random_uuid_v4()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("建临时项目目录失败");
    root
}

fn blueprint(chapter: i64) -> blueprints::BlueprintData {
    blueprints::BlueprintData {
        chapter_number: chapter,
        title: format!("第 {chapter} 章"),
        role: "起".to_string(),
        purpose: "推进主线".to_string(),
        key_events: "关键事件".to_string(),
        characters: vec!["林清玄".to_string(), "苏晚".to_string()],
        new_character_candidates: None,
        relationship_hints: None,
        suspense_hook: "钩子".to_string(),
        user_guidance: "指导".to_string(),
        notes: String::new(),
        notes_updated_at: String::new(),
    }
}

fn draft_params(chapter: i64, content: &str) -> drafts::DraftCreateParams {
    drafts::DraftCreateParams {
        chapter_number: chapter,
        version: None,
        source: "write".to_string(),
        content: content.to_string(),
        word_count: content.chars().count() as i64,
        source_dependencies: None,
    }
}

#[test]
fn real_project_creation_lifecycle_persists_across_reopen_test() {
    let root = temp_project("lifecycle");

    // ===== 第一次会话：真实文件库 =====
    {
        let db = ProjectDatabase::open(&root).expect("打开真实项目库失败");
        assert!(
            project_database_path(&root).exists(),
            "应在 <root>/.lore/lorekeeper.db 建库"
        );
        let conn = db.connection();

        // 1) 项目主台账
        project_core::init(conn, "磁盘回归项目", project_core::DEFAULT_WRITING_LANGUAGE).unwrap();
        let base = project_core::get(conn).unwrap().expect("init 后可读");
        assert_eq!(base.project_name, "磁盘回归项目");

        // 2) 蓝图范围提交（full 1..=3）
        let request = blueprints::BlueprintCommitRangeRequest {
            mode: "full".to_string(),
            operation_id: "disk-op-1".to_string(),
            start_chapter: 1,
            end_chapter: 3,
            blueprints: (1..=3).map(blueprint).collect(),
        };
        let receipt = blueprints::commit_range(conn, &request).expect("蓝图提交失败");
        assert!(!receipt.idempotent);
        assert_eq!(receipt.chapter_numbers, vec![1, 2, 3]);
        assert_eq!(
            blueprints::get_all(conn).unwrap().len(),
            3,
            "蓝图应真实落盘"
        );

        // 3) 蓝图角色同步：pending → complete（无名单证据 → already-satisfied）
        let pending = blueprints::list_pending_character_sync_operations(conn).unwrap();
        assert_eq!(pending.len(), 1);
        let completed =
            blueprints::complete_character_sync_operation(conn, &pending[0].operation_id).unwrap();
        assert_eq!(completed.status, "completed");
        assert_eq!(
            completed.completion_receipt.map(|receipt| receipt.status),
            Some("already-satisfied".to_string())
        );

        // 4) 草稿创建 + 改正文
        let draft_id = drafts::create(conn, &draft_params(1, "第一章初稿正文")).unwrap();
        drafts::update_content(conn, draft_id, "第一章修改后的正文", 10).unwrap();
        let draft = drafts::get_full(conn, draft_id).unwrap().unwrap();
        assert_eq!(draft.content, "第一章修改后的正文");
        assert_eq!(draft.meta.version, 1);

        // 5) 修稿：冻结源 → 合并（草稿变 revised）
        let source = ExpectedDraftSource {
            id: draft_id,
            chapter_number: 1,
            version: 1,
            status: "draft".to_string(),
            content: "第一章修改后的正文".to_string(),
        };
        let revision = revisions::create(
            conn,
            &revisions::RevisionCreateParams {
                base_draft_id: draft_id,
                revision_type: "refine".to_string(),
                user_prompt: Some("润色".to_string()),
                review_source_id: None,
                content: "第一章修稿后正文".to_string(),
                word_count: 8,
                expected_source: Some(source),
            },
        )
        .expect("修稿创建失败");

        let merge_receipt = revisions::merge_into_draft(
            conn,
            &revisions::MergeRevisionRequest {
                revision_id: revision.id,
                target_draft_id: draft_id,
                expected_draft_content: "第一章修改后的正文".to_string(),
                merged_content: "第一章合并后正文".to_string(),
                word_count: 8,
            },
        )
        .expect("合并失败");
        assert!(!merge_receipt.idempotent);
        let merged_draft = drafts::get_full(conn, draft_id).unwrap().unwrap();
        assert_eq!(merged_draft.content, "第一章合并后正文");
        assert_eq!(merged_draft.meta.status, "revised");

        // 6) 审稿：基于合并后的草稿事实
        let review = reviews::create(
            conn,
            &reviews::ReviewCreateParams {
                base_draft_id: draft_id,
                review_index: None,
                content: "审稿报告正文".to_string(),
                expected_source: Some(ExpectedDraftSource {
                    id: draft_id,
                    chapter_number: 1,
                    version: 1,
                    status: "revised".to_string(),
                    content: "第一章合并后正文".to_string(),
                }),
            },
        )
        .expect("审稿创建失败");
        assert_eq!(review.review_index, 1);
        let latest_review = reviews::get_latest_by_draft(conn, draft_id).unwrap().unwrap();
        assert_eq!(latest_review.content, "审稿报告正文");

        // 7) 后处理跑批：两个关键步骤 → 汇总切换
        let run_id = post_process::create_run(
            conn,
            &post_process::PostProcessCreateParams {
                trigger_source_type: "chapter_finalize".to_string(),
                trigger_source_id: "1".to_string(),
                source_label: "第 1 章".to_string(),
                steps: vec![
                    post_process::PostProcessStepInput {
                        key: "extract".to_string(),
                        label: "提取要点".to_string(),
                        critical: true,
                    },
                    post_process::PostProcessStepInput {
                        key: "summary".to_string(),
                        label: "生成摘要".to_string(),
                        critical: true,
                    },
                ],
            },
        )
        .expect("创建跑批失败");
        post_process::mark_step_ok(conn, &run_id, "extract").unwrap();
        assert!(!post_process::is_all_critical_passed(conn, "chapter_finalize", "1").unwrap());
        post_process::mark_step_ok(conn, &run_id, "summary").unwrap();
        assert!(post_process::is_all_critical_passed(conn, "chapter_finalize", "1").unwrap());
        post_process::mark_step_failed(conn, &run_id, "extract", "重跑超时").unwrap();
        assert!(
            !post_process::is_all_critical_passed(conn, "chapter_finalize", "1").unwrap(),
            "失败重跑必须把汇总重算为 false"
        );

        // 8) LLM 日志与统计
        llm::log_call(
            conn,
            &serde_json::json!({
                "modelId": "deepseek-chat",
                "modelName": "DeepSeek Chat",
                "purpose": "write",
                "promptTokens": 100,
                "completionTokens": 200,
                "totalTokens": 300,
                "durationMs": 1500,
                "success": true
            }),
        )
        .unwrap();
        llm::log_call(
            conn,
            &serde_json::json!({
                "modelId": "deepseek-chat",
                "purpose": "write",
                "success": false,
                "errorMessage": "finish:length"
            }),
        )
        .unwrap();
        let stats = llm::get_stats(conn).unwrap();
        assert_eq!((stats.total_calls, stats.successful_calls, stats.failed_calls), (2, 1, 1));
        assert_eq!(stats.total_tokens, Some(300));
        assert_eq!(llm::get_history(conn, 10).unwrap().len(), 2);
    } // 连接在此释放（模拟关闭应用）

    // ===== 第二次会话：重开同一库，验证全部事实仍在 =====
    {
        let db = ProjectDatabase::open(&root).expect("重开项目库失败");
        let conn = db.connection();

        let core = project_core::get(conn).unwrap().expect("主台账应仍在");
        assert_eq!(core.project_name, "磁盘回归项目");

        assert_eq!(blueprints::get_all(conn).unwrap().len(), 3, "蓝图应跨重开保留");
        assert!(
            blueprints::list_pending_character_sync_operations(conn)
                .unwrap()
                .is_empty(),
            "已完成的角色同步不得重回 pending"
        );

        let all_drafts = drafts::list_all(conn).unwrap();
        assert_eq!(all_drafts.len(), 1);
        assert_eq!(all_drafts[0].status, "revised");
        let persisted = drafts::get_full(conn, all_drafts[0].id).unwrap().unwrap();
        assert_eq!(persisted.content, "第一章合并后正文");

        assert_eq!(revisions::list_by_draft(conn, all_drafts[0].id).unwrap().len(), 1);
        assert_eq!(
            revisions::get_full(conn, 1).unwrap().unwrap().meta.status,
            "merged"
        );
        assert_eq!(reviews::list_by_draft(conn, all_drafts[0].id).unwrap().len(), 1);

        let steps = post_process::get_steps(conn, &post_process::get_latest_run(
            conn,
            "chapter_finalize",
            "1",
        )
        .unwrap()
        .unwrap()
        .id)
        .unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].error_msg, "重跑超时", "步骤失败原因应保留");

        assert_eq!(llm::get_stats(conn).unwrap().total_calls, 2);
        // 角色名单元数据在首次 read/commit 前不存在；此处只验证读取不报错
        let snapshot = roster::read(conn).unwrap();
        assert_eq!(snapshot.revision, 0);
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn real_project_sqlite_pragmas_are_effective_on_disk_test() {
    let root = temp_project("pragma");
    let db = ProjectDatabase::open(&root).unwrap();
    let conn = db.connection();

    // WAL 在文件库上真实生效（内存库无法反映这一点）
    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode.to_lowercase(), "wal");

    let foreign_keys: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 1, "外键约束必须开启");

    // 外键 RESTRICT 真实生效：仍被草稿引用的正文不可删除
    let draft_id = drafts::create(conn, &draft_params(1, "正文")).unwrap();
    let content_id = drafts::get_meta(conn, draft_id).unwrap().unwrap().content_id;
    let delete_result = conn.execute("DELETE FROM contents WHERE id = ?1", [content_id]);
    assert!(delete_result.is_err(), "ON DELETE RESTRICT 应阻止删除被引用的正文");

    // 级联真实生效：删除草稿会带走其修稿/审稿
    let source = ExpectedDraftSource {
        id: draft_id,
        chapter_number: 1,
        version: 1,
        status: "draft".to_string(),
        content: "正文".to_string(),
    };
    revisions::create(
        conn,
        &revisions::RevisionCreateParams {
            base_draft_id: draft_id,
            revision_type: "refine".to_string(),
            user_prompt: None,
            review_source_id: None,
            content: "修稿".to_string(),
            word_count: 2,
            expected_source: Some(source.clone()),
        },
    )
    .unwrap();
    reviews::create(
        conn,
        &reviews::ReviewCreateParams {
            base_draft_id: draft_id,
            review_index: None,
            content: "审稿".to_string(),
            expected_source: Some(source),
        },
    )
    .unwrap();
    assert_eq!(revisions::list_by_draft(conn, draft_id).unwrap().len(), 1);
    assert_eq!(reviews::list_by_draft(conn, draft_id).unwrap().len(), 1);

    conn.execute("DELETE FROM drafts WHERE id = ?1", [draft_id]).unwrap();
    assert!(
        revisions::list_by_draft(conn, draft_id).unwrap().is_empty(),
        "删除草稿应级联删除修稿"
    );
    assert!(
        reviews::list_by_draft(conn, draft_id).unwrap().is_empty(),
        "删除草稿应级联删除审稿"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn real_project_clear_moves_physical_files_and_keeps_library_test() {
    let root = temp_project("clear");
    let db = ProjectDatabase::open(&root).unwrap();
    let conn = db.connection();

    project_core::init(conn, "清理回归", project_core::DEFAULT_WRITING_LANGUAGE).unwrap();
    blueprints::upsert(conn, &blueprint(1)).unwrap();
    drafts::create(conn, &draft_params(1, "待清理正文")).unwrap();

    // 真实物理文件：成稿实体稿 + 一份非成稿文件
    let finalized_file = root.join("第1章 起始.txt");
    std::fs::write(&finalized_file, "定稿正文").unwrap();
    let notes_file = root.join("写作笔记.txt");
    std::fs::write(&notes_file, "笔记").unwrap();

    let result = project_clear::clear_generated_data(
        conn,
        Some(&root),
        &project_clear::ProjectClearOptions {
            generated_text: Some(true),
            ..Default::default()
        },
    )
    .expect("清理失败");

    assert_eq!(
        result.cleared,
        vec![project_clear::ProjectClearScope::GeneratedText]
    );
    assert_eq!(result.physical_files_deleted, 1);
    assert!(!finalized_file.exists(), "成稿实体稿应移出项目根");
    assert!(notes_file.exists(), "非成稿文件不得被触碰");
    assert!(drafts::list_all(conn).unwrap().is_empty(), "草稿应被清空");
    assert_eq!(
        blueprints::get_all(conn).unwrap().len(),
        1,
        "未勾选的范围（蓝图）必须保留"
    );

    // 库文件本身仍可用（清理不是删除库）
    assert!(project_database_path(&root).exists());
    drop(db);

    let reopened = ProjectDatabase::open(&root).unwrap();
    assert_eq!(
        blueprints::get_all(reopened.connection()).unwrap().len(),
        1,
        "重开后未清理范围的事实仍在"
    );
    assert!(drafts::list_all(reopened.connection()).unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

/// G1 + B2 磁盘级回归：**定稿事务 → 实体稿落盘 → 章节删除真实移除实体稿**。
///
/// 对应 GUI 冒烟里「点定稿得到 `.txt`」与「删除章节后 `.txt` 消失」两条断言。
/// 命令层只做门禁与信封转换，故在此直接驱动仓储 + `manuscript_publisher`。
#[test]
fn real_project_finalize_publish_then_delete_removes_manuscript_test() {
    use crate::manuscript_publisher::{publish_manuscript, remove_published_manuscript};
    use crate::repositories::chapter_deletion_repository as chapter_deletion;
    use crate::repositories::finalization_repository as finalization;

    let root = temp_project("finalize-delete");
    let root_text = root.to_string_lossy().to_string();
    let database = ProjectDatabase::open(&root).expect("打开项目库失败");
    let conn = database.connection();
    project_core::init(conn, "定稿删除回归", project_core::DEFAULT_WRITING_LANGUAGE).unwrap();

    // 1) 待定稿草稿
    conn.execute("INSERT INTO contents (body) VALUES ('草稿正文')", [])
        .unwrap();
    let content_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO drafts (chapter_number, version, status, content_id) VALUES (1, 1, 'draft', ?1)",
        [content_id],
    )
    .unwrap();
    let draft_id = conn.last_insert_rowid();

    // 2) 定稿提交（G1）：事务内冻结正文 / 状态 / 字数 / outbox
    let body = "# 第一章 起点\n\n正文第一段。\n";
    let record = finalization::commit(
        conn,
        &finalization::FinalizationCommitInput {
            finalization_id: "fin-disk-1".to_string(),
            draft_id,
            chapter_number: 1,
            chapter_title: "起点".to_string(),
            content: body.to_string(),
            content_hash: crate::repositories::finalized_continuity_repository::sha256_hex(body),
            content_revision: 1,
            target_file_name: "第1章 起点.txt".to_string(),
        },
    )
    .expect("定稿提交失败");
    assert_eq!(record.publication_status, "pending");
    assert_eq!(record.content_snapshot, body, "outbox 必须冻结不可变正文");
    assert_eq!(
        conn.query_row("SELECT body FROM contents WHERE id = ?1", [content_id], |row| row
            .get::<_, String>(0))
            .unwrap(),
        body,
        "定稿后 contents.body 应等于冻结正文"
    );

    // 3) 发布实体稿（真实文件，标题行被剥离）
    publish_manuscript(
        &root_text,
        &record.target_file_name,
        1,
        "起点",
        &record.content_snapshot,
    )
    .expect("发布实体稿失败");
    let manuscript = root.join("第1章 起点.txt");
    assert!(manuscript.exists(), "定稿后应落盘实体稿");
    assert_eq!(
        std::fs::read_to_string(&manuscript).unwrap(),
        "第1章 起点\n\n正文第一段。\n"
    );
    assert_eq!(
        finalization::mark_published(conn, &record.finalization_id)
            .unwrap()
            .publication_status,
        "published"
    );

    // 陪跑文件：非冻结目标，删除时不得被波及
    let decoy = root.join("其他稿件.txt");
    std::fs::write(&decoy, "无关内容").unwrap();

    // 4) 章节删除（B2）：冻结收据 + 同事务删除 SQLite 事实
    let request = chapter_deletion::DeleteFinalizedChapterRequest {
        draft_id,
        chapter_number: 1,
    };
    let operation = chapter_deletion::begin(conn, "op-disk-1", &request, false)
        .expect("冻结删除收据失败");
    assert_eq!(operation.manuscript_status, "pending");
    assert_eq!(operation.knowledge_status, "not_required");
    assert_eq!(operation.target_file_name, "第1章 起点.txt");
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM drafts WHERE id = ?1", [draft_id], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(remaining, 0, "定稿事实应随删除事务移除");

    // 5) 断点恢复的实体稿投影：真实删文件（缺失视为幂等成功）
    chapter_deletion::start_attempt(conn, "op-disk-1").unwrap();
    remove_published_manuscript(&root_text, &operation.target_file_name).expect("移除实体稿失败");
    chapter_deletion::mark_projection(conn, "op-disk-1", "manuscript", "completed", "").unwrap();

    assert!(!manuscript.exists(), "删除章节后实体稿文件应被真实移除");
    assert!(decoy.exists(), "非冻结目标的陪跑文件不得被删除");
    let finished = chapter_deletion::get(conn, "op-disk-1").unwrap().unwrap();
    assert_eq!(finished.manuscript_status, "completed");
    assert_eq!(finished.status, "completed", "双通道完成后聚合状态应为 completed");

    // 6) 幂等：同一目标再次清理不报错
    remove_published_manuscript(&root_text, &operation.target_file_name).unwrap();

    // 7) 跨连接重开仍持久
    let db_path = project_database_path(&root);
    drop(database);
    let reopened = ProjectDatabase::open(&root).unwrap();
    assert!(db_path.exists());
    assert_eq!(
        chapter_deletion::get(reopened.connection(), "op-disk-1")
            .unwrap()
            .unwrap()
            .status,
        "completed"
    );
    assert!(!manuscript.exists());

    let _ = std::fs::remove_dir_all(&root);
}
