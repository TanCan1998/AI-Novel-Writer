//! 批次 H（H4-1/2）：MCP 连接管理器 —— 平移自
//! `electron/mcp/mcp-manager.ts`（500 行）。
//!
//! 架构（对齐基线 :100-117）：主进程持有两张内存表——
//! - `servers`：服务器运行时表（`id → McpServerRuntime`）
//! - `loaded_configs`：已加载配置表（`id → McpServerConfig`）
//!
//! 基线依赖 Electron 主线程 Event Loop 的串行性；Tauri 命令在多线程
//! 执行，故两张表以 `Mutex` 保护（kickoff §6 风险 #8）。
//!
//! **本轮（Task 1/2）范围**：类型镜像 + 管理器骨架 + 配置层。
//! **传输层尚未实现 —— Task 3 落地**：stdio 子进程
//! （`std::process::Command` + 自研守卫，零新增依赖）、JSON-RPC `\n`
//! 分帧、10s 超时、pending 请求表、`initializeSession`
//! （`clientInfo.name` 照抄基线 `'vela'`，D-H4-2）均不在本轮。
//! 本轮 `connect` / `call_tool` 等传输相关 API 返回占位错误
//! （如 `'MCP 传输尚未实现'`）；经命令层信封对齐 `mcp-ipc-bridge.ts`
//! 后，渲染层可见行为与基线一致（`mcp:connect` 失败一律
//! `'MCP 服务器连接失败'`，内部细节不透传）。
//!
//! 配置路径红线（D-H4-1）：`lorekeeper_home()/mcp_config.json`，
//! **绝不**落 `~/.vela` / `AI_NOVEL_VELA_HOME`。

mod config;
mod types;

pub use config::{
    get_config_path, get_config_path_at, load_config, load_config_at, read_config,
    McpConfigReadError, MCP_CONFIG_FILE,
};
pub use types::{
    MCPConfigLoadResponse, MCPConfigLoadResult, MCPConfigLoadStatus, MCPConnectionStatus,
    MCPResourceDescription, MCPServerStatus, MCPServerSummary, MCPToolCallResult,
    MCPToolDescription, McpConfig, McpServerConfig, McpTransport,
};

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

/// MCP 管理器（对齐基线 `MCPManager` 接口，mcp-manager.ts:96-117）。
///
/// 本轮为骨架：`servers` 表恒为空（无传输，Task 3 落地）；
/// `loaded_configs` 由 `load_config` 维护。
#[derive(Debug, Default)]
pub struct McpManager {
    /// 服务器运行时表（基线 `servers`，mcp-manager.ts:100）。
    servers: Mutex<BTreeMap<String, McpServerRuntime>>,
    /// 已加载配置表（基线 `loadedConfigs`，mcp-manager.ts:104）。
    loaded_configs: Mutex<BTreeMap<String, McpServerConfig>>,
}

impl McpManager {
    /// 新建空管理器（基线 `new MCPManager()`）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 默认配置文件路径（基线 `getDefaultConfigPath`，mcp-manager.ts:119；D-H4-1）。
    pub fn config_path(&self) -> PathBuf {
        config::get_config_path()
    }

    /// 已加载配置查询（基线 `loadedConfigs.get`，:114）。
    pub fn loaded_config(&self, server_id: &str) -> Option<McpServerConfig> {
        self.loaded_configs
            .lock()
            .ok()
            .and_then(|configs| configs.get(server_id).cloned())
    }

    /// 整体替换已加载配置表（基线 `loadedConfigs = ...`，:196）。
    pub fn set_loaded_configs(&self, configs: BTreeMap<String, McpServerConfig>) {
        if let Ok(mut current) = self.loaded_configs.lock() {
            *current = configs;
        }
    }

    /// 加载配置（基线 `loadConfig` 的完整编排，mcp-manager.ts:127-203）：
    ///
    /// 1. 先清空 `loaded_configs`（:129）；
    /// 2. 读取并校验文件（`config::read_config`）；
    /// 3. 成功：断开「未覆盖或配置不一致」的已连接服务器（:186-191），
    ///    写回 `loaded_configs`，返回 `loaded` 摘要；
    /// 4. 失败：`revoke` —— 先 `disconnect_all()` 再返回对应错误
    ///    （:201-214）。
    pub fn load_config(&self) -> MCPConfigLoadResult {
        // 基线 :129：`this.loadedConfigs.clear()`
        self.clear_loaded_configs();
        match config::read_config(&self.config_path()) {
            Ok(mcp_config) => {
                self.drop_stale_servers(&mcp_config.mcp_servers);
                let summaries: Vec<MCPServerSummary> = mcp_config
                    .mcp_servers
                    .values()
                    .map(|server| MCPServerSummary {
                        id: server.id.clone(),
                        name: server.name.clone(),
                        transport: server.transport,
                    })
                    .collect();
                self.set_loaded_configs(mcp_config.mcp_servers);
                MCPConfigLoadResult::loaded(summaries)
            }
            Err(McpConfigReadError::Missing) => {
                let _ = self.disconnect_all();
                MCPConfigLoadResult::missing()
            }
            Err(McpConfigReadError::Corrupt) => {
                let _ = self.disconnect_all();
                MCPConfigLoadResult::error("MCP 配置损坏，未加载任何服务器")
            }
            Err(McpConfigReadError::Unreadable) => {
                let _ = self.disconnect_all();
                MCPConfigLoadResult::error("MCP 配置损坏或无法读取，未加载任何服务器")
            }
        }
    }

    /// 连接服务器（基线 `connect`，:205-251）。
    ///
    /// **本轮占位（Task 3 落地）**：传输层未实现——
    /// - 未配置 → `Err('MCP 服务器未配置或配置尚未加载')`（基线 :210-212，保留）；
    /// - 已配置 → `Err('MCP 传输尚未实现')`（占位；Task 3 将实现
    ///   stdio 子进程 / JSON-RPC 分帧 / 10s 超时 / initializeSession，
    ///   基线 `connectStdio` :255-290）。
    ///
    /// 内部错误细节经命令层信封（对齐 `mcp-ipc-bridge.ts:33-41`）
    /// 统一为 `'MCP 服务器连接失败'`，不透传渲染层。
    pub fn connect(&self, server_id: &str) -> Result<(), String> {
        if self.loaded_config(server_id).is_none() {
            return Err("MCP 服务器未配置或配置尚未加载".to_string());
        }
        Err("MCP 传输尚未实现".to_string())
    }

    /// 断开服务器（基线 `disconnect`，:438-449）。
    ///
    /// 本轮为骨架：从未连接表移除（未命中为无操作成功）；
    /// Task 3 落地：kill 子进程、pending 请求 reject('连接已断开')。
    pub fn disconnect(&self, server_id: &str) -> Result<(), String> {
        if let Ok(mut servers) = self.servers.lock() {
            servers.remove(server_id);
        }
        Ok(())
    }

    /// 断开全部服务器（基线 `disconnectAll`，:451-456）。
    /// 本轮为骨架：清空运行时表；Task 3 落地：kill 全部子进程。
    pub fn disconnect_all(&self) -> Result<(), String> {
        if let Ok(mut servers) = self.servers.lock() {
            servers.clear();
        }
        Ok(())
    }

    /// 调用工具（基线 `callTool`，:410-437）。
    ///
    /// 本轮无传输（Task 3 落地），服务器恒未连接 → 恒返回基线
    /// :413-417 的未连接错误；`connected` 分支为占位
    /// `'MCP 传输尚未实现'`（本轮不可达，Task 3 替换为
    /// `tools/call` JSON-RPC 请求与 text 片段聚合 :429-437）。
    pub fn call_tool(
        &self,
        server_id: &str,
        _tool_name: &str,
        _tool_args: HashMap<String, serde_json::Value>,
    ) -> MCPToolCallResult {
        let connected = self
            .servers
            .lock()
            .ok()
            .and_then(|servers| {
                servers
                    .get(server_id)
                    .map(|runtime| runtime.status == MCPConnectionStatus::Connected)
            })
            .unwrap_or(false);
        if connected {
            MCPToolCallResult {
                success: false,
                content: String::new(),
                error: Some("MCP 传输尚未实现".to_string()),
            }
        } else {
            MCPToolCallResult {
                success: false,
                content: String::new(),
                error: Some(format!("服务器 {} 未连接", server_id)),
            }
        }
    }

    /// 聚合全部已连接服务器的工具（基线 `getAllTools`，:458-467）。
    /// 本轮无传输 → 恒为空。
    pub fn get_all_tools(&self) -> Vec<MCPToolDescription> {
        let Ok(servers) = self.servers.lock() else {
            return Vec::new();
        };
        servers
            .values()
            .filter(|runtime| runtime.status == MCPConnectionStatus::Connected)
            .flat_map(|runtime| runtime.tools.clone())
            .collect()
    }

    /// 聚合全部已连接服务器的资源（基线 `getAllResources`，:469-478）。
    /// 本轮无传输 → 恒为空。
    pub fn get_all_resources(&self) -> Vec<MCPResourceDescription> {
        let Ok(servers) = self.servers.lock() else {
            return Vec::new();
        };
        servers
            .values()
            .filter(|runtime| runtime.status == MCPConnectionStatus::Connected)
            .flat_map(|runtime| runtime.resources.clone())
            .collect()
    }

    /// 服务器状态快照（基线 `getServersStatus`，:480-493）。
    /// 本轮无传输 → 恒为空数组。
    pub fn get_servers_status(&self) -> Vec<MCPServerStatus> {
        let Ok(servers) = self.servers.lock() else {
            return Vec::new();
        };
        servers
            .values()
            .map(|runtime| MCPServerStatus {
                id: runtime.config.id.clone(),
                name: runtime.config.name.clone(),
                status: runtime.status,
                tool_count: runtime.tools.len(),
                error: runtime.error.clone(),
            })
            .collect()
    }

    /// 基线 :129：`loadedConfigs.clear()`。
    fn clear_loaded_configs(&self) {
        if let Ok(mut configs) = self.loaded_configs.lock() {
            configs.clear();
        }
    }

    /// 基线 :186-191：新配置未覆盖、或与新配置深度不一致的已连接
    /// 服务器断开（基线 `isDeepStrictEqual` ↔ `PartialEq` 结构等价）。
    ///
    /// 本轮 `servers` 表恒为空（无传输，Task 3 落地），本方法为无操作；
    /// 保留实现以对齐基线结构。
    fn drop_stale_servers(&self, configs: &BTreeMap<String, McpServerConfig>) {
        let Ok(mut servers) = self.servers.lock() else {
            return;
        };
        let stale: Vec<String> = servers
            .iter()
            .filter(|(server_id, runtime)| match configs.get(*server_id) {
                None => true,
                Some(config) => runtime.config != *config,
            })
            .map(|(server_id, _)| server_id.clone())
            .collect();
        for server_id in stale {
            servers.remove(&server_id);
        }
    }
}

/// 服务器运行时（对齐基线 `MCPServerRuntime`，mcp-manager.ts:80-94）。
///
/// 本轮为骨架：传输相关字段（子进程句柄 / 消息缓冲区 / pending 请求表）
/// 待 Task 3 落地；`status` 恒为 `Disconnected`。
#[derive(Debug, Clone)]
struct McpServerRuntime {
    config: McpServerConfig,
    status: MCPConnectionStatus,
    tools: Vec<MCPToolDescription>,
    resources: Vec<MCPResourceDescription>,
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> (String, McpServerConfig) {
        (
            "fs".to_string(),
            McpServerConfig {
                id: "fs".to_string(),
                name: "fs".to_string(),
                transport: McpTransport::Stdio,
                command: Some("npx".to_string()),
                args: Some(vec!["-y".to_string()]),
                env: Some(
                    [("HOME".to_string(), "/tmp".to_string())]
                        .into_iter()
                        .collect(),
                ),
                url: None,
            },
        )
    }

    #[test]
    fn set_and_query_loaded_configs_test() {
        let manager = McpManager::new();
        assert_eq!(manager.loaded_config("fs"), None);

        let (id, config) = sample_config();
        let mut configs = BTreeMap::new();
        configs.insert(id, config.clone());
        manager.set_loaded_configs(configs);

        assert_eq!(manager.loaded_config("fs"), Some(config));
        assert_eq!(manager.loaded_config("other"), None);
    }

    #[test]
    fn config_path_follows_lorekeeper_home_test() {
        // D-H4-1：管理器视角的默认路径与配置层一致。
        assert_eq!(McpManager::new().config_path(), config::get_config_path());
    }

    #[test]
    fn connect_placeholder_reports_internal_error_test() {
        let manager = McpManager::new();
        // 基线 :210-212：未配置 → 内部错误
        // （命令层信封统一为 'MCP 服务器连接失败'）
        assert_eq!(
            manager.connect("fs").unwrap_err(),
            "MCP 服务器未配置或配置尚未加载"
        );

        // 已配置但传输未实现（Task 3 落地）
        let (id, config) = sample_config();
        let mut configs = BTreeMap::new();
        configs.insert(id, config);
        manager.set_loaded_configs(configs);
        assert_eq!(manager.connect("fs").unwrap_err(), "MCP 传输尚未实现");
    }

    #[test]
    fn call_tool_reports_not_connected_test() {
        let manager = McpManager::new();
        let result = manager.call_tool("fs", "list", HashMap::new());
        assert!(!result.success);
        assert_eq!(result.content, "");
        assert_eq!(result.error.as_deref(), Some("服务器 fs 未连接"));
    }

    #[test]
    fn disconnect_is_noop_for_unknown_server_test() {
        let manager = McpManager::new();
        assert!(manager.disconnect("fs").is_ok());
        assert!(manager.disconnect_all().is_ok());
    }

    #[test]
    fn read_only_aggregates_are_empty_without_transport_test() {
        let manager = McpManager::new();
        assert!(manager.get_servers_status().is_empty());
        assert!(manager.get_all_tools().is_empty());
        assert!(manager.get_all_resources().is_empty());
    }
}
