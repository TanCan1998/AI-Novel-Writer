use std::sync::Mutex;

/// 当前活跃项目（批次 B 骨架）。数据库与租约未迁移（批次 C），
/// 具备 root_path 供路径边界与会话一致校验使用。
#[derive(Debug, Clone)]
pub struct ActiveProject {
    pub root_path: String,
}

#[derive(Debug)]
pub struct AppState {
    /// 启动时间戳（毫秒）。阶段 0 用于健康检查命令验证状态注入链路。
    pub(crate) started_at_ms: u64,
    /// 批次 A：配置存储
    pub(crate) config: Mutex<crate::commands::ConfigStore>,
    /// 批次 A：皮肤命令存储
    pub(crate) skin: Mutex<crate::commands::SkinCommandStore>,
    /// 批次 B：活跃项目（None = 未打开项目；项目域命令一律拒绝）
    pub(crate) active_project: Mutex<Option<ActiveProject>>,
    /// 批次 B：项目文件操作串行锁（骨架阶段全局互斥，安全语义同基线 per-path 队列）
    pub(crate) fs_lock: Mutex<()>,
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
            active_project: Mutex::new(None),
            fs_lock: Mutex::new(()),
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
