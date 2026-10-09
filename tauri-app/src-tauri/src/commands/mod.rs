//! Tauri 命令注册表 —— 对应 `src/shared/ipc-channels.ts` 各频道组的迁移落点。
//!
//! 约定（docs-fork/agents/pi-development.md §2/§6 + docs-fork/plans/tauri-migration-channel-inventory.md）：
//! - 命令名 snake_case，与频道名一一对应（`config:get` → `config_get`）；
//! - 入参/返回值与契约文件的 `args`/`return` 类型一一对应，结构体派生
//!   `Serialize`/`Deserialize` 且 `#[serde(rename_all = "camelCase")]` 对齐前端字段；
//! - 项目域命令尾部必须接收 `project_session`（渲染层自动注入）；
//! - 错误 `Result<T, String>` 起步，复杂域再自定义可序列化 Error。
//!
//! 批次规划：A 窗口/配置/皮肤 → B 项目/文件/授权 → C 数据库 → D LLM
//! → E 定稿链 → F 知识库 → G 导入 → H 更新/MCP/prompt。

use serde::Serialize;

pub mod app_data; // 批次 H：应用数据域（prompt:* 3 + skills:* 4）
pub mod chapter_lifecycle; // 批次 E 第二部分：章节生命周期（4 频道）
mod config; // 批次 A：配置管理
mod db; // 批次 C：项目数据库
mod external_file_grant; // 批次 B：外部文件授权
pub mod finalization; // 批次 E（G1）：定稿提交 / 实体稿重试（2 频道）
mod fs; // 批次 B：项目文件系统
pub mod import; // 批次 G1：导入文件选择（dialog:select-novel-files）
pub mod kb; // 批次 F2-3：知识库（kb:* 15 频道 + dialog 2）
mod llm; // 批次 D1：LLM 模型管理（配置读写 7 频道）
mod llm_execution; // 批次 D2：LLM 生成执行（租约 2 频道）
mod llm_generation; // 批次 D2-b：LLM 生成 / 流式 / 取消（3 频道 + 3 事件）
mod llm_management; // 批次 D2-c：连通性探测 + 模型发现（llm:* 收口 2 频道）
mod model_provider_resource; // 批次 A：模型资源
mod official_homepage; // 批次 A：官方主页
pub mod project; // 批次 B：项目生命周期
mod skin; // 批次 A：皮肤管理
pub mod update; // 批次 H（H3）：应用更新（update:* 6 频道 + update:state 事件）
mod window; // 批次 A：窗口管理

/// 再导出各批次模块的全部公开项（含 Tauri 命令宏 `__cmd__*`），
/// 供 `lib.rs` 的 `generate_handler![commands::xxx]` 与 `state.rs` 引用。
pub use app_data::*;
pub use chapter_lifecycle::*;
pub use config::*;
pub use db::*;
pub use external_file_grant::*;
pub use finalization::*;
pub use fs::*;
pub use import::*;
pub use kb::*;
pub use llm::*;
pub use llm_execution::*;
pub use llm_generation::*;
pub use llm_management::*;
pub use model_provider_resource::*;
pub use official_homepage::*;
pub use project::*;
pub use skin::*;
pub use update::*;
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
