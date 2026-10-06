//! 全局应用状态 —— 对应 Electron 主进程的模块级单例。
//!
//! 后续批次迁移时统一收敛到这里（经 `tauri::State` 注入命令）：
//! - 批次 B：项目会话租约表（`leaseId → projectPath`，ADR 0001）、外部文件授权（ADR 0002）
//! - 批次 C：rusqlite 连接池 / 每项目数据库句柄
//! - 批次 D：LLM 执行租约与流式请求注册表
//!
//! 注意：rusqlite 连接非 `Sync`，届时使用 `Mutex<Connection>` 或按项目句柄表封装，
//! 本地 SQL 保持同步调用（docs/agents/pi-development.md §6）。

#[derive(Debug)]
pub struct AppState {
    /// 启动时间戳（毫秒）。阶段 0 用于健康检查命令验证状态注入链路。
    pub(crate) started_at_ms: u64,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            started_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_state_new_records_started_at_test() {
        let state = AppState::new();
        assert!(state.started_at_ms > 0, "启动时间戳必须大于 0");
    }
}
