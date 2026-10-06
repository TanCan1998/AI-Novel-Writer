// 发布构建下隐藏 Windows 控制台窗口；Tauri 官方模板标准配置。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ai_novel_writer_lib::run()
}
