//! Tauri 命令注册表 —— 对应 `src/shared/ipc-channels.ts` 各频道组的迁移落点。
//!
//! 约定（docs/agents/pi-development.md §2/§6 + docs/plans/tauri-migration-channel-inventory.md）：
//! - 命令名 snake_case，与频道名一一对应（`config:get` → `config_get`）；
//! - 入参/返回值与契约文件的 `args`/`return` 类型一一对应，结构体派生
//!   `Serialize`/`Deserialize` 且 `#[serde(rename_all = "camelCase")]` 对齐前端字段；
//! - 项目域命令尾部必须接收 `project_session`（渲染层自动注入）；
//! - 错误 `Result<T, String>` 起步，复杂域再自定义可序列化 Error。
//!
//! 批次规划：A 窗口/配置/皮肤 → B 项目/文件/授权 → C 数据库 → D LLM
//! → E 定稿链 → F 知识库 → G 导入 → H 更新/MCP/prompt。

use serde::Serialize;

mod config; // 批次 A：配置管理
mod window; // 批次 A：窗口管理
mod skin; // 批次 A：皮肤管理
mod official_homepage; // 批次 A：官方主页
mod model_provider_resource; // 批次 A：模型资源
mod fs; // 批次 B：项目文件系统
mod project; // 批次 B：项目生命周期
mod external_file_grant; // 批次 B：外部文件授权
mod db; // 批次 C：项目数据库

/// 再导出各批次模块的全部公开项（含 Tauri 命令宏 `__cmd__*`），
/// 供 `lib.rs` 的 `generate_handler![commands::xxx]` 与 `state.rs` 引用。
pub use config::*;
pub use db::*;
pub use external_file_grant::*;
pub use fs::*;
pub use model_provider_resource::*;
pub use official_homepage::*;
pub use project::*;
pub use skin::*;
pub use window::*;

/// 通用「成功/失败」返回 —— 对齐契约 `{ success: boolean; error?: string }`。
/// 批次 B 的 fs / project 两模块共用，避免 glob 再导出同名歧义。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimpleResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 阶段 0 骨架健康检查报告。
/// 不对应任何 Electron IPC 频道，仅供 Tauri 联调验证 invoke 链路。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckReport {
    pub ok: bool,
    pub started_at_ms: u64,
}

/// 纯函数核心，便于单元测试（命令本体保持薄封装）。
pub fn health_check_report(started_at_ms: u64) -> HealthCheckReport {
    HealthCheckReport {
        ok: true,
        started_at_ms,
    }
}

/// 阶段 0 骨架健康检查命令。前端可用 `invoke('app_health_check')` 验证联通。
#[tauri::command]
pub fn app_health_check(state: tauri::State<'_, crate::state::AppState>) -> HealthCheckReport {
    health_check_report(state.started_at_ms)
}

// 批次 A：窗口管理命令

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_check_report_reflects_started_state_test() {
        let report = health_check_report(123_456);
        assert!(report.ok);
        assert_eq!(report.started_at_ms, 123_456);
    }
}
