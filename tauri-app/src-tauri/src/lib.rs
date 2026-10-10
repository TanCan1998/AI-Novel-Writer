//! Tauri 2 后端入口 —— 迁移自 `electron/main.ts` + `electron/ipc-handlers.ts`。
//!
//! 迁移纪律（docs-fork/agents/pi-development.md）：
//! - 每个 `src/shared/ipc-channels.ts` 频道对应一个 `#[tauri::command]`，契约唯一事实源不变；
//! - 项目域命令预留尾参 `project_session`（渲染层 ipc-client.ts 自动注入）并校验租约（ADR 0001）；
//! - 仅新增，不改 Electron 代码；每批次迁移配 Rust 单元测试并通过 `cargo test`。

mod app_paths;
mod character_role;
mod commands;
pub mod db;
#[cfg(test)]
mod disk_e2e;
mod draft_source_guard;
// 批次 F2-3：外部文件授权注册表（内存态，知识库选择/导入与批次 H 共用）。
pub mod external_grant;
// 批次 H（B12）：打开受信常量外部链接（tauri-plugin-opener）。
pub mod external_link;
// 批次 E：定稿导入的字数契约（`src/shared/draft-units.ts` 的 Rust 单源，纯函数）。
pub mod draft_units;
mod json_store;
// 批次 F1：剧情树快照的**结构校验**（`src/shared/plot-tree.ts` 的 Rust 单源，
// 纯函数、不依赖数据库，故置于 crate 根而非 repositories）。
mod plot_tree;
// 批次 D2：LLM 生成执行域（预设目录 / 推理策略 / 生成参数 / 执行租约 / HTTP 生成链
// + 模型发现 / 远程 Embedding）。
mod llm;
// 批次 G1：作者原稿导入（检视存储 + 章节解析；`dialog:select-novel-files`）。
pub mod import;
// 批次 E（G1）：实体稿发布 / 清理投影（`electron/services/manuscript-publisher.ts` 的平移）。
mod manuscript_publisher;
// 批次 A 收口（B13）：渲染层导航防护（对齐基线 `preventRendererNavigation`
// + 官方主页弹窗拦截）。主窗口由 tauri.conf.json 声明（非 builder 创建），
// 故以自有插件注册，经插件 store 的 on_navigation 钩子覆盖所有 webview。
mod navigation_guard;
mod project_access;
// 批次 G2a/G2b：仓储层公开（与 `pub mod db` 一致）——
// 各仓储的 `pub fn` 是 crate 的公开 API，命令层在后续批次逐步接入；
// 若保持私有，尚未被命令层消费的新 API 会触发 dead_code 告警。
pub mod repositories;
mod security;
mod state;
// 批次 H：Writing Skill 检查与 GitHub 地址解析（`src/shared/writing-skills.ts` 的 Rust 单源）。
pub mod writing_skills;

// 批次 H（H3）：应用更新域（update:* 6 频道 + update:state 事件）。
pub mod update;

// 批次 H（H4-2）：MCP 连接管理器（`mcp:*` 9 频道——类型镜像 +
// 管理器 + 传输层均已落地，本轮在 `generate_handler!` 完成
// 全部 9 个命令的注册）。
pub mod mcp;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(state::AppState::new())
        // 批次 B 遗留补齐（2026-10-08）：原生目录/文件选择对话框。
        // 仅供自研命令在 Rust 侧调用；webview 不直调插件的 `plugin:dialog|*` 命令，
        // 故 `capabilities/default.json` 维持最小权限（不追加 `dialog:*`）。
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        // 批次 A 收口（B13）：渲染层导航防护。
        // 拒绝渲染层导航替换主框架；URL 精确等于官方主页时
        // 交给系统浏览器打开，但导航仍被拒绝（对齐基线「永远 deny」）。
        .plugin(navigation_guard::init())
        // 拦截 `CloseRequested` → 广播 `window:close-requested` 让渲染层确认未保存内容，
        // 待 `window:resolve-close(proceed)` 将 `approved` 置位后才真正放行。
        .on_window_event(|window, event| {
            use tauri::{Emitter, Manager};
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let (allow_close, request_id) = window
                    .app_handle()
                    .state::<state::AppState>()
                    .request_window_close(window.label());
                if allow_close {
                    return;
                }
                api.prevent_close();
                if let Some(request_id) = request_id {
                    let _ = window.emit(
                        "window:close-requested",
                        serde_json::json!({ "requestId": request_id }),
                    );
                }
            }
        })
        // 对齐基线 `ensureVelaHome()`：启动即保证 `~/.lorekeeper/{prompts,logs}` 存在。
        // 失败不阻断启动（首次写入时会再次建目录并给出可读错误）。
        .setup(|app| {
            use tauri::Manager;
            if let Err(error) = app_paths::ensure_lorekeeper_home() {
                eprintln!("[Lorekeeper] {error}");
            }
            // 批次 H（H3）：装配更新运行时（失败降级为「更新不可用」，绝不阻断启动）
            let outcome = update::startup::start_update_runtime(app.handle());
            app.state::<state::AppState>()
                .install_update_service(outcome.service);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // 阶段 0 骨架联调命令（非业务频道，验证 invoke/状态注入链路）
            commands::app_health_check,
            // 批次 A：基础配置（批次 D1 补齐真实文件持久化）
            commands::config_get,
            commands::config_set,
            // 批次 A：窗口管理
            commands::window_minimize,
            commands::window_toggle_maximize,
            commands::window_close,
            commands::window_resolve_close,
            // 批次 A：皮肤管理
            commands::skin_get_state,
            commands::skin_execute,
            commands::skin_read_custom_asset,
            // 批次 A：官方主页
            commands::official_homepage_open,
            // 批次 A：模型资源
            commands::model_provider_resource_open,
            // 批次 B：项目文件系统（fs 基础 7 频道）
            commands::fs_read_file,
            commands::fs_write_file,
            commands::fs_list_dir,
            commands::fs_mkdir,
            commands::fs_check_exists,
            commands::fs_read_json,
            commands::fs_write_json,
            // 批次 B：项目生命周期（project 10 频道）
            commands::project_get_runtime_context,
            commands::project_create,
            commands::project_open,
            commands::project_save,
            commands::project_update_config,
            commands::project_recent_list,
            commands::project_recent_remove,
            commands::project_delete,
            commands::project_smoke_open_request,
            commands::project_smoke_open_confirm,
            // 批次 B：目录选择（`select-folder` 已真实化；`select-export-directory`
            // 弹窗能力已就绪，仍需批次 H 的 grant 签发，暂为取消骨架）
            commands::dialog_select_folder,
            commands::dialog_select_export_directory,
            // 批次 B：外部文件授权（骨架：等 tauri-plugin-dialog）
            commands::fs_grant_read_file,
            commands::fs_grant_write_file,
            commands::fs_grant_mkdir,
            // 批次 C：项目数据库（project_core 子域）
            commands::db_close,
            commands::db_project_core_get,
            commands::db_project_core_update,
            commands::db_project_core_synopsis_commit,
            // 批次 C：角色与角色名单（characters 子域）
            commands::db_character_get_all,
            commands::db_character_roster_read,
            commands::db_character_roster_commit,
            commands::db_blueprint_get_all,
            commands::db_blueprint_get,
            commands::db_blueprint_upsert,
            commands::db_blueprint_upsert_many,
            commands::db_blueprint_update_notes,
            commands::db_blueprint_delete,
            commands::db_blueprint_clear_all,
            commands::db_blueprint_commit_range,
            // 批次 C：蓝图角色同步（blueprints 子域 S2-c）
            commands::db_blueprint_character_sync_list_pending,
            commands::db_blueprint_character_sync_get,
            commands::db_blueprint_character_sync_complete,
            // 批次 C：草稿（drafts 子域 S3-a）
            commands::db_draft_create,
            commands::db_draft_list,
            commands::db_draft_list_all,
            commands::db_draft_get_meta,
            commands::db_draft_get_full,
            commands::db_draft_get_latest,
            commands::db_draft_get_finalized,
            commands::db_draft_get_max_finalized_chapter,
            commands::db_draft_next_version,
            commands::db_draft_update_status,
            commands::db_draft_update_content,
            commands::db_draft_delete,
            // 批次 C：修稿（revisions 子域 S3-b）
            commands::db_revision_create,
            commands::db_revision_replace_pending,
            commands::db_revision_list,
            commands::db_revision_get_pending,
            commands::db_revision_get_full,
            commands::db_revision_next_index,
            commands::db_revision_merge,
            commands::db_revision_mark_merged,
            commands::db_revision_mark_discarded,
            // 批次 C：审稿（reviews 子域 S3-c）
            commands::db_review_create,
            commands::db_review_list,
            commands::db_review_get_latest,
            commands::db_review_get_full,
            commands::db_review_next_index,
            // 批次 C：后处理（post-process 子域 S3-d）
            commands::db_post_process_create_run,
            commands::db_post_process_get_latest_run,
            commands::db_post_process_get_steps,
            commands::db_post_process_mark_step_ok,
            commands::db_post_process_mark_step_failed,
            commands::db_post_process_is_all_passed,
            // 批次 C：LLM 日志与摘要
            commands::db_log_llm_call,
            commands::db_get_llm_stats,
            commands::db_get_llm_history,
            commands::db_save_summary_snapshot,
            commands::db_get_latest_summary,
            // 批次 C：项目生成数据清理
            commands::db_project_clear_generated_data,
            // 批次 F1：一致性豁免（consistency-exemption 子域，3 频道）
            commands::db_consistency_exemption_list,
            commands::db_consistency_exemption_save,
            commands::db_consistency_exemption_revoke,
            // 批次 F1：叙事线索（narrative-thread 子域，6 频道）
            commands::db_narrative_thread_list,
            commands::db_narrative_thread_list_relevant,
            commands::db_narrative_thread_plan_create,
            commands::db_narrative_thread_plan_update,
            commands::db_narrative_thread_plan_delete,
            commands::db_narrative_thread_event_confirm,
            // 批次 F1：剧情树（plot-tree 子域，3 频道）
            commands::db_plot_tree_read,
            commands::db_plot_tree_save,
            commands::db_plot_tree_clear,
            // 批次 E：生成失败恢复候选（recovery-candidate 子域，4 频道）
            commands::db_recovery_candidate_record,
            commands::db_recovery_candidate_list,
            commands::db_recovery_candidate_update,
            commands::db_recovery_candidate_resolve,
            // 批次 E：定稿连续性投影（continuity 子域，4 频道）
            commands::db_continuity_save_finalized,
            commands::db_continuity_save_character_state_candidates,
            commands::db_continuity_list_before,
            commands::db_continuity_read_source,
            // 批次 E：定稿回链 + 导出权威（finalization-link + draft 收尾前 3 频道）
            commands::db_finalization_link_knowledge_document,
            commands::db_draft_authority_sequence,
            commands::db_draft_export_snapshot,
            commands::db_draft_export_authority_current,
            // 批次 E：原稿导入幂等提交（draft 收尾最后一频道）
            commands::db_draft_import_finalized_batch,
            // 批次 E 第二部分：章节生命周期（chapter-lifecycle，4 频道）
            commands::chapter_delete_finalized,
            commands::chapter_retry_deletion,
            commands::chapter_confirm_legacy_knowledge_absent,
            commands::chapter_list_incomplete_deletions,
            // 批次 E（G1）：定稿提交 / 实体稿重试（finalization:* 2 频道）
            commands::finalization_commit,
            commands::finalization_retry,
            // 批次 H：应用数据域（prompt:* 3 + skills:* 4）
            commands::prompt_load_global,
            commands::prompt_save_global,
            commands::prompt_delete_global,
            commands::skills_list_user,
            commands::skills_inspect_github,
            commands::skills_install_github,
            commands::skills_uninstall_user,
            // 批次 H（H3）：应用更新（update:* 6 频道 + update:state 事件）
            commands::update_get_state,
            commands::update_check,
            commands::update_download,
            commands::update_open_release,
            commands::update_defer_reminder,
            commands::update_quit_and_install,
            // 批次 H（H4-2）：MCP（mcp:* 9 频道）
            commands::mcp_load_config,
            commands::mcp_connect,
            commands::mcp_disconnect,
            commands::mcp_disconnect_all,
            commands::mcp_list_tools,
            commands::mcp_list_resources,
            commands::mcp_call_tool,
            commands::mcp_get_servers_status,
            commands::mcp_get_config_path,
            // 批次 D1：LLM 模型管理（7 频道）
            commands::llm_list_models,
            commands::llm_save_model,
            commands::llm_delete_model,
            commands::llm_get_default_model,
            commands::llm_set_default_model,
            commands::llm_get_default_embedding_model,
            commands::llm_set_default_embedding_model,
            // 批次 D2：LLM 生成执行（租约 2 频道）
            commands::llm_begin_execution_lease,
            commands::llm_close_execution_lease,
            // 批次 D2-b：LLM 生成 / 流式 / 取消（3 频道 + 3 事件）
            commands::llm_generate,
            commands::llm_generate_stream,
            commands::llm_cancel,
            // 批次 D2-c：llm:* 收口（连通性探测 + 模型发现）
            commands::llm_test_connection,
            commands::llm_discover_models,
            // 批次 F2-3：知识库（kb:* 15 频道 + dialog:select-knowledge-* 2 频道）
            commands::kb_import_document,
            commands::kb_import_folder,
            commands::kb_import_text,
            commands::kb_import_planning_text,
            commands::kb_import_reference_text,
            commands::kb_search,
            commands::kb_search_writing_context,
            commands::kb_search_with_scope,
            commands::kb_list_documents,
            commands::kb_remove_document,
            commands::kb_clear_all,
            commands::kb_stats,
            commands::kb_get_vectorless_count,
            commands::kb_get_vector_rebuild_status,
            commands::kb_backfill_vectors,
            commands::dialog_select_knowledge_files,
            commands::dialog_select_knowledge_folder,
            // 批次 G1：作者原稿导入（dialog:select-novel-files + db:import-run-author-preview）
            commands::dialog_select_novel_files,
            commands::db_import_run_author_preview,
            // 批次 G2a：导入运行读面（3 频道）
            commands::db_import_run_get,
            commands::db_import_run_list_resumable,
            commands::db_import_run_list_chapters,
            // 批次 G2b-5：导入运行准备 / 解析收口（2 频道）
            commands::db_import_run_prepare_inspection,
            commands::db_import_run_finalize_parsing,
            // 批次 G3b：导入运行执行租约 / 批次推进 / effect receipts（12 频道）
            commands::db_import_run_start_resume,
            commands::db_import_run_renew_execution,
            commands::db_import_run_restart,
            commands::db_import_run_request_cancel,
            commands::db_import_run_cancel_at_boundary,
            commands::db_import_run_complete_batch,
            commands::db_import_run_advance_stage,
            commands::db_import_run_fail,
            commands::db_import_run_complete,
            commands::db_import_run_effect_receipt_get,
            commands::db_import_run_effect_receipt_prepare,
            commands::db_import_run_effect_receipt_commit,
            // 批次 G3b：全局事实提交
            commands::db_import_global_facts_commit,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 应用启动失败");
}
