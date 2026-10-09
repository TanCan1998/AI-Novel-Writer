use std::sync::Mutex;

/// 当前活跃项目会话（租约）。
///
/// 安全语义：路径不构成授权。项目域命令必须同时匹配 `project_id`、`lease_id`
/// 与 `root_path`（见 `security::assert_current_project_context`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveProject {
    pub project_id: String,
    pub lease_id: String,
    pub root_path: String,
}

impl ActiveProject {
    /// 转换为可传递的会话租约
    pub fn to_lease(&self) -> crate::project_access::ProjectSessionLease {
        crate::project_access::ProjectSessionLease {
            project_id: self.project_id.clone(),
            root_path: self.root_path.clone(),
            lease_id: self.lease_id.clone(),
        }
    }
}

/// 项目数据库状态快照（对齐基线 `ProjectDatabaseState`）
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDatabaseState {
    pub database_restored: bool,
    pub db_ready: bool,
    pub active_project_path: Option<String>,
}

#[derive(Debug)]
pub struct AppState {
    /// 启动时间戳（毫秒）。阶段 0 用于健康检查命令验证状态注入链路。
    pub(crate) started_at_ms: u64,
    /// 批次 A：皮肤命令存储
    pub(crate) skin: Mutex<crate::commands::SkinCommandStore>,
    /// 批次 B/C：活跃项目会话（None = 未打开项目；项目域命令一律拒绝）
    pub(crate) active_project: Mutex<Option<ActiveProject>>,
    /// 批次 C：项目数据库连接，生命周期与 `active_project` 一致
    pub(crate) project_db: Mutex<Option<crate::db::ProjectDatabase>>,
    /// 批次 C：最近一次项目打开/创建请求令牌（陈旧请求必须让位于最新请求）
    pub(crate) latest_open_token: Mutex<Option<String>>,
    /// 批次 B：项目文件操作串行锁（骨架阶段全局互斥，安全语义同基线 per-path 队列）
    pub(crate) fs_lock: Mutex<()>,
    /// 批次 D2：模型执行租约注册表（进程内存态，重启即失效）
    pub(crate) llm_leases: Mutex<crate::llm::lease::LlmLeaseStore>,
    /// 批次 D2-b：活跃流式生成任务（`requestId` → 取消句柄）。
    /// 进程内存态；重启即失效（对齐基线 `activeStreams`）。
    pub(crate) llm_streams:
        Mutex<std::collections::HashMap<String, crate::commands::LlmStreamHandle>>,
    /// 批次 F2-3：外部文件授权注册表（进程内存态）。
    /// 知识库选择/导入与批次 H `fs:grant-*` 共用；渲染层只持有不透明 grantId。
    pub(crate) external_grants: Mutex<crate::external_grant::ExternalGrantRegistry>,
    /// 批次 F2-3：知识库向量索引管理器（按项目懒加载 HNSW 图）。
    pub(crate) kb_vectors: Mutex<crate::db::kb::vectors::KbVectorManager>,
    /// 批次 H：`skills:inspect-github` 的「先检查后安装」一次性确认缓存
    /// （`sourceUrl` → `(contentSha256, resolvedUrl)`；进程内存态，重启即失效）。
    pub(crate) writing_skill_inspections:
        Mutex<std::collections::HashMap<String, (String, String)>>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            started_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            skin: Mutex::new(crate::commands::SkinCommandStore::new()),
            active_project: Mutex::new(None),
            project_db: Mutex::new(None),
            latest_open_token: Mutex::new(None),
            fs_lock: Mutex::new(()),
            llm_leases: Mutex::new(crate::llm::lease::LlmLeaseStore::new()),
            llm_streams: Mutex::new(std::collections::HashMap::new()),
            external_grants: Mutex::new(crate::external_grant::ExternalGrantRegistry::default()),
            kb_vectors: Mutex::new(crate::db::kb::vectors::KbVectorManager::default()),
            writing_skill_inspections: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// 当前活跃项目快照
    pub(crate) fn active_project_snapshot(&self) -> Option<ActiveProject> {
        self.active_project
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// 当前已打开项目根路径（等价基线 `getCurrentProjectPath`）
    pub(crate) fn current_project_path(&self) -> Option<String> {
        self.active_project
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|project| project.root_path.clone()))
    }

    /// 登记最新打开/创建请求令牌
    pub(crate) fn set_latest_open_token(&self, token: &str) -> Result<(), String> {
        let mut guard = self
            .latest_open_token
            .lock()
            .map_err(|_| "项目打开令牌状态被污染".to_string())?;
        *guard = Some(token.to_string());
        Ok(())
    }

    /// 请求令牌是否仍是最新的（用于拒绝陈旧请求）
    pub(crate) fn is_latest_open_token(&self, token: &str) -> bool {
        self.latest_open_token
            .lock()
            .map(|guard| guard.as_deref() == Some(token))
            .unwrap_or(false)
    }

    /// 打开项目数据库并登记会话。
    ///
    /// 先关闭旧连接再打开新库，避免半切换状态；任一步失败都不会留下「有会话无库」
    /// 或「有库无会话」的组合。
    pub(crate) fn activate_project(&self, lease: ActiveProject) -> Result<(), String> {
        self.close_project_database();
        let database = crate::db::ProjectDatabase::open(std::path::Path::new(&lease.root_path))?;
        {
            let mut guard = self
                .project_db
                .lock()
                .map_err(|_| "项目数据库状态被污染".to_string())?;
            *guard = Some(database);
        }
        let mut session = self
            .active_project
            .lock()
            .map_err(|_| "项目会话状态被污染".to_string())?;
        *session = Some(lease);
        Ok(())
    }

    /// 关闭项目数据库（连接随所有权释放而关闭）
    pub(crate) fn close_project_database(&self) {
        if let Ok(mut guard) = self.project_db.lock() {
            *guard = None;
        }
    }

    /// 使当前会话失效（关闭数据库 + 清空租约）
    pub(crate) fn invalidate_current_session(&self) {
        self.close_project_database();
        if let Ok(mut guard) = self.active_project.lock() {
            *guard = None;
        }
    }

    /// 项目数据库是否已打开（用于 `project:get-runtime-context` 的中立态语义）
    pub(crate) fn project_database_open(&self) -> bool {
        self.project_db
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    /// 在项目数据库连接上执行操作（未打开数据库时返回结构化错误）
    pub(crate) fn with_project_db<T>(
        &self,
        operation: impl FnOnce(&rusqlite::Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        let guard = self
            .project_db
            .lock()
            .map_err(|_| "项目数据库状态被污染".to_string())?;
        match guard.as_ref() {
            Some(database) => operation(database.connection()),
            None => Err("项目数据库未打开".to_string()),
        }
    }

    /// 项目数据库状态快照（对齐基线 `databaseStateFor`）
    pub(crate) fn database_state(
        &self,
        expected_project_path: Option<&str>,
    ) -> ProjectDatabaseState {
        let active_project_path = self.current_project_path();
        let db_open = self
            .project_db
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false);
        let db_ready = db_open
            && match (active_project_path.as_deref(), expected_project_path) {
                (Some(current), Some(expected)) => {
                    crate::project_access::project_path_key(current)
                        == crate::project_access::project_path_key(expected)
                }
                (Some(_), None) => true,
                (None, None) => false,
                (None, Some(_)) => false,
            };
        ProjectDatabaseState {
            database_restored: db_ready,
            db_ready,
            active_project_path,
        }
    }

    /// 中立态（无项目、无数据库、无会话）
    pub(crate) fn neutral_database_state(&self) -> ProjectDatabaseState {
        let active_project_path = self.current_project_path();
        ProjectDatabaseState {
            database_restored: active_project_path.is_none(),
            db_ready: active_project_path.is_none(),
            active_project_path,
        }
    }

    /// 批次 F2-3：按「项目根 + 嵌入代际」懒加载知识库向量索引。
    ///
    /// 首次调用时在 `<project>/.lore/kb/` 建目录，并尝试从 `index-<generation>.hnsw.*`
    /// 快照恢复（快照缺失或损坏时按空图起步）。索引实例常驻 `AppState`，
    /// 后续调用直接命中缓存。
    pub(crate) async fn kb_vector_index(
        &self,
        project_root: &str,
        generation: i64,
        dimension: usize,
    ) -> Result<crate::db::vector::LocalVectorIndex, String> {
        use crate::db::kb::vectors as kbv;
        let key = kbv::index_key(project_root, generation);
        if let Some(index) = self
            .kb_vectors
            .lock()
            .ok()
            .and_then(|guard| guard.get(&key))
        {
            return Ok(index);
        }

        let directory = kbv::snapshot_dir(project_root);
        let index = crate::db::vector::LocalVectorIndex::new(
            dimension,
            kbv::DEFAULT_KB_MAX_ELEMENTS,
            &directory,
        )
        .map_err(|error| error.to_string())?;
        // 已有快照则恢复；不存在视为首次构建，按空图起步。
        let _ = index.load_hnsw(&kbv::snapshot_basename(generation)).await;

        let mut guard = self
            .kb_vectors
            .lock()
            .map_err(|_| "知识库向量索引状态被污染".to_string())?;
        Ok(guard.insert_if_absent(key, index))
    }

    /// 批次 F2-3：丢弃某项目已加载的全部代际向量索引（关闭项目 / 清空知识库）。
    pub(crate) fn drop_kb_vectors(&self, project_root: &str) {
        if let Ok(mut guard) = self.kb_vectors.lock() {
            guard.remove_project(project_root);
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

    #[test]
    fn new_state_has_no_project_and_no_database_test() {
        let state = AppState::new();
        assert!(state.current_project_path().is_none());
        assert_eq!(state.database_state(None).db_ready, false);
        assert!(state.neutral_database_state().database_restored);
        let error = state.with_project_db(|_| Ok(())).unwrap_err();
        assert_eq!(error, "项目数据库未打开");
    }

    #[test]
    fn activate_and_invalidate_project_test() {
        let root = std::env::temp_dir().join(format!("anw-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let state = AppState::new();
        let lease = ActiveProject {
            project_id: crate::project_access::random_uuid_v4(),
            lease_id: crate::project_access::random_uuid_v4(),
            root_path: root.to_string_lossy().to_string(),
        };
        state.activate_project(lease.clone()).unwrap();

        assert_eq!(
            state.current_project_path().as_deref(),
            Some(lease.root_path.as_str())
        );
        assert!(state.database_state(Some(&lease.root_path)).db_ready);
        assert!(
            !state.database_state(Some("C:\\other")).db_ready,
            "路径不符时不得视为就绪"
        );
        state
            .with_project_db(|conn| {
                let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE name = 'project_core'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(|error| error.to_string())?;
                assert_eq!(count, 1);
                Ok(())
            })
            .unwrap();

        state.invalidate_current_session();
        assert!(state.current_project_path().is_none());
        assert!(state.with_project_db(|_| Ok(())).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn latest_open_token_rejects_stale_request_test() {
        let state = AppState::new();
        state.set_latest_open_token("token-2").unwrap();
        assert!(state.is_latest_open_token("token-2"));
        assert!(!state.is_latest_open_token("token-1"), "旧令牌必须判为陈旧");
    }

    #[test]
    fn new_state_has_no_live_model_leases_test() {
        let state = AppState::new();
        assert_eq!(state.llm_leases.lock().unwrap().live_lease_count(), 0);
    }
}
