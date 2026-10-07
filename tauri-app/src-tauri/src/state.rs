use std::sync::Mutex;

#[derive(Debug)]
pub struct AppState {
    /// 启动时间戳（毫秒）。阶段 0 用于健康检查命令验证状态注入链路。
    pub(crate) started_at_ms: u64,
    /// 批次 A：配置存储
    pub(crate) config: Mutex<crate::commands::ConfigStore>,
    /// 批次 A：皮肤命令存储
    pub(crate) skin: Mutex<crate::commands::SkinCommandStore>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            started_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            config: Mutex::new(crate::commands::ConfigStore::new()),
            skin: Mutex::new(crate::commands::SkinCommandStore::new()),
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
