//! Tauri 2 后端入口 —— 迁移自 `electron/main.ts` + `electron/ipc-handlers.ts`。
//!
//! 迁移纪律（docs/agents/pi-development.md）：
//! - 每个 `src/shared/ipc-channels.ts` 频道对应一个 `#[tauri::command]`，契约唯一事实源不变；
//! - 项目域命令预留尾参 `project_session`（渲染层 ipc-client.ts 自动注入）并校验租约（ADR 0001）；
//! - 仅新增，不改 Electron 代码；每批次迁移配 Rust 单元测试并通过 `cargo test`。

mod commands;
mod state;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(state::AppState::new())
        .invoke_handler(tauri::generate_handler![
            // 阶段 0 骨架联调命令（非业务频道，验证 invoke/状态注入链路）
            commands::app_health_check,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 应用启动失败");
}
