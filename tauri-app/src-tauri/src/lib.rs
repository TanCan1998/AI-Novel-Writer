//! Tauri 2 后端入口 —— 迁移自 `electron/main.ts` + `electron/ipc-handlers.ts`。
//!
//! 迁移纪律（docs/agents/pi-development.md）：
//! - 每个 `src/shared/ipc-channels.ts` 频道对应一个 `#[tauri::command]`，契约唯一事实源不变；
//! - 项目域命令预留尾参 `project_session`（渲染层 ipc-client.ts 自动注入）并校验租约（ADR 0001）；
//! - 仅新增，不改 Electron 代码；每批次迁移配 Rust 单元测试并通过 `cargo test`。

mod character_role;
mod commands;
mod db;
mod draft_source_guard;
mod project_access;
mod repositories;
mod security;
mod state;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(state::AppState::new())
        .invoke_handler(tauri::generate_handler![
            // 阶段 0 骨架联调命令（非业务频道，验证 invoke/状态注入链路）
            commands::app_health_check,
            // 批次 A：基础配置
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
            // 批次 B：目录选择（骨架：插件接入前返回取消语义）
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
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 应用启动失败");
}
