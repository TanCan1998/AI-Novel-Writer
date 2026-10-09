//! Tauri 2 后端入口 —— 迁移自 `electron/main.ts` + `electron/ipc-handlers.ts`。
//!
//! 迁移纪律（docs/agents/pi-development.md）：
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
mod json_store;
// 批次 F1：剧情树快照的**结构校验**（`src/shared/plot-tree.ts` 的 Rust 单源，
// 纯函数、不依赖数据库，故置于 crate 根而非 repositories）。
mod plot_tree;
// 批次 D2：LLM 生成执行域（预设目录 / 推理策略 / 生成参数 / 执行租约 / HTTP 生成链
// + 模型发现 / 远程 Embedding）。
mod llm;
mod project_access;
mod repositories;
mod security;
mod state;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(state::AppState::new())
        // 批次 B 遗留补齐（2026-10-08）：原生目录/文件选择对话框。
        // 仅供自研命令在 Rust 侧调用；webview 不直调插件的 `plugin:dialog|*` 命令，
        // 故 `capabilities/default.json` 维持最小权限（不追加 `dialog:*`）。
        .plugin(tauri_plugin_dialog::init())
        // 对齐基线 `ensureVelaHome()`：启动即保证 `~/.lorekeeper/{prompts,logs}` 存在。
        // 失败不阻断启动（首次写入时会再次建目录并给出可读错误）。
        .setup(|_app| {
            if let Err(error) = app_paths::ensure_lorekeeper_home() {
                eprintln!("[Lorekeeper] {error}");
            }
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
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 应用启动失败");
}
