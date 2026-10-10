//! 批次 H（H4）：MCP 连接管理器 —— 平移自
//! `electron/mcp/mcp-manager.ts`（500 行）。//!
//!
//! 架构（对齐基线 :100-117）：主进程持有两张内存表——
//! - `servers`：服务器运行时表（`id → Arc<McpServerRuntime>`）
//! - `loaded_configs`：已加载配置表（`id → McpServerConfig`）
//!
//! 基线依赖 Electron 主线程 Event Loop 的串行性；Tauri 命令在多线程
//! 执行，故两张表以 `Mutex` 保护（kickoff §6 风险 #8）。`connect`
//! 的阻塞 I/O 段（spawn / 握手 / 发现，合计至多约 10s 超时）
//! **不持表锁**，提交结果时以 `Arc::ptr_eq` 校验运行时未被并发
//! disconnect / 重连替换（被替换则孤儿化，与基线语义一致）。
//!
//! **Task 3/4 已落地**：stdio 传输层（`transport.rs`：
//! `std::process::Command` argv 直传、`\n` 分帧、10s 超时、
//! pending 请求表；`initializeSession` 握手的 `clientInfo.name`
//! 照抄基线 `'vela'`，D-H4-2）、连接状态机
//! （connecting → connected / error）、`disconnect` /
//! `disconnect_all`（kill 子进程 + 拒绝在途请求
//! `'连接已断开'`）。SSE 不在范围（基线 :228）。
//!
//! 配置路径红线（D-H4-1）：`lorekeeper_home()/mcp_config.json`，
//! **绝不**落 `~/.vela` / `AI_NOVEL_VELA_HOME`。

mod config;
mod transport;
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
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

/// MCP 管理器（对齐基线 `MCPManager` 接口，mcp-manager.ts:96-117）。
///
/// `servers` 表持有 `Arc` 运行时：`call_tool` 等只读路径把
/// 传输句柄克隆出表后即可释放表锁，再阻塞等待 JSON-RPC 回应
/// （至多 10s），不阻塞其他命令。
#[derive(Debug, Default)]
pub struct McpManager {
    /// 服务器运行时表（基线 `servers`，mcp-manager.ts:100）。
    servers: Mutex<BTreeMap<String, Arc<McpServerRuntime>>>,
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
    /// 状态机：`loaded_configs` 未命中 → 内部错误
    /// `'MCP 服务器未配置或配置尚未加载'`（基线 :210-212）→
    /// 已连接则先断开再重连（:213-215）→ `connecting` 运行时
    /// 入表（:217-227）→ spawn + 握手 + 发现（:229-243）→
    /// `connected`；失败 → kill + 清理 pending + `error` 态
    /// （:246-250），返回 `'MCP 服务器连接失败'`（内部细节经
    /// 命令层信封不透传，对齐 `mcp-ipc-bridge.ts:33-41`）。
    ///
    /// 阻塞 I/O 段不持 `servers` 表锁（避免阻塞
    /// `get-servers-status` 等轮询命令）；提交结果时以
    /// `Arc::ptr_eq` 校验表中的运行时仍是本次连接的对象——
    /// 并发 disconnect / 重连替换后孤儿化（基线语义：初始化
    /// 完成但运行时已不在表中）。
    pub fn connect(&self, server_id: &str) -> Result<(), String> {
        // 基线 :210-212。
        let config = match self.loaded_config(server_id) {
            Some(config) => config,
            None => return Err("MCP 服务器未配置或配置尚未加载".to_string()),
        };
        // 基线 :213-215：已连接 → 先断开再重连。
        if self
            .servers
            .lock()
            .ok()
            .is_some_and(|servers| servers.contains_key(&config.id))
        {
            let _ = self.disconnect(&config.id);
        }
        // 基线 :217-227：connecting 运行时先入表。
        let connecting = Arc::new(McpServerRuntime {
            config: config.clone(),
            status: MCPConnectionStatus::Connecting,
            tools: Vec::new(),
            resources: Vec::new(),
            error: None,
            transport: None,
        });
        {
            let mut servers = self
                .servers
                .lock()
                .map_err(|_| "MCP 服务器连接失败".to_string())?;
            servers.insert(config.id.clone(), Arc::clone(&connecting));
        }
        // 基线 :229-243：传输 + 初始化 + 发现（不持表锁）。
        let established = establish_connection(&config);
        // 提交结果：仅当表中的运行时未被并发替换时。
        let mut servers = self
            .servers
            .lock()
            .map_err(|_| "MCP 服务器连接失败".to_string())?;
        let is_current = servers
            .get(&config.id)
            .is_some_and(|current| Arc::ptr_eq(current, &connecting));
        match established {
            Ok(established) => {
                if is_current {
                    servers.insert(config.id.clone(), Arc::new(established));
                }
                Ok(())
            }
            Err(_) => {
                // 基线 :246-250 catch：`error` 态入表 + 统一错误。
                if is_current {
                    servers.insert(
                        config.id.clone(),
                        Arc::new(McpServerRuntime {
                            config,
                            status: MCPConnectionStatus::Error,
                            tools: Vec::new(),
                            resources: Vec::new(),
                            error: Some("MCP 服务器连接失败".to_string()),
                            transport: None,
                        }),
                    );
                }
                Err("MCP 服务器连接失败".to_string())
            }
        }
    }
    /// 断开服务器（基线 `disconnect`，:438-449）：kill 子进程、
    /// 拒绝全部在途请求（`'连接已断开'`）、从运行时表移除。
    ///
    /// 基线该方法不抛异常 → 恒 `Ok(())`（未命中为无操作成功）。
    pub fn disconnect(&self, server_id: &str) -> Result<(), String> {
        let runtime = self
            .servers
            .lock()
            .ok()
            .and_then(|mut servers| servers.remove(server_id));
        if let Some(runtime) = runtime {
            if let Some(transport) = &runtime.transport {
                transport.shutdown();
            }
        }
        Ok(())
    }

    /// 断开全部服务器（基线 `disconnectAll`，:451-456）：
    /// 逐个 kill 子进程、拒绝在途请求、移除运行时。
    pub fn disconnect_all(&self) -> Result<(), String> {
        let server_ids: Vec<String> = self
            .servers
            .lock()
            .map(|servers| servers.keys().cloned().collect())
            .unwrap_or_default();
        for server_id in server_ids {
            let _ = self.disconnect(&server_id);
        }
        Ok(())
    }

    /// 调用工具（基线 `callTool`，:410-437）。
    ///
    /// 未连接（含传输缺失）→ `{success:false, content:'',
    /// error:'服务器 <id> 未连接'}`（基线 :412-417）；成功 →
    /// `content` 为结果中 `type==='text'` 片段的 `text`
    /// 以 `'\n'` 连接（基线 :425-431）；请求失败 →
    /// `{success:false, content:'', error: String(error)}`
    /// （基线 :434-436）。
    pub fn call_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        tool_args: HashMap<String, Value>,
    ) -> MCPToolCallResult {
        // 基线 :412-417：未连接 → 固定错误（`callTool` 自带的
        // 错误返回，bridge 无 try/catch）。
        let not_connected = || MCPToolCallResult {
            success: false,
            content: String::new(),
            error: Some(format!("服务器 {} 未连接", server_id)),
        };
        // 把传输句柄克隆出表后释放锁：JSON-RPC 请求至多阻塞
        // 10 秒，不应持锁阻塞其他命令。
        let transport = {
            let Ok(servers) = self.servers.lock() else {
                return not_connected();
            };
            let Some(runtime) = servers.get(server_id) else {
                return not_connected();
            };
            if !runtime.is_connected() {
                return not_connected();
            }
            match runtime.transport.as_ref() {
                Some(transport) => Arc::clone(transport),
                None => return not_connected(),
            }
        };
        // 基线 :419-431：`tools/call {name, arguments}`。
        match transport.send_request(
            "tools/call",
            Some(json!({
                "name": tool_name,
                "arguments": tool_args,
            })),
        ) {
            Ok(result) => MCPToolCallResult {
                success: true,
                content: extract_text_content(&result),
                error: None,
            },
            // 基线 :434-436。
            Err(error) => MCPToolCallResult {
                success: false,
                content: String::new(),
                error: Some(error),
            },
        }
    }

    /// 聚合全部已连接服务器的工具（基线 `getAllTools`，:458-467）。
    pub fn get_all_tools(&self) -> Vec<MCPToolDescription> {
        let Ok(servers) = self.servers.lock() else {
            return Vec::new();
        };
        servers
            .values()
            .filter(|runtime| runtime.is_connected())
            .flat_map(|runtime| runtime.tools.clone())
            .collect()
    }

    /// 聚合全部已连接服务器的资源（基线 `getAllResources`，:469-478）。
    pub fn get_all_resources(&self) -> Vec<MCPResourceDescription> {
        let Ok(servers) = self.servers.lock() else {
            return Vec::new();
        };
        servers
            .values()
            .filter(|runtime| runtime.is_connected())
            .flat_map(|runtime| runtime.resources.clone())
            .collect()
    }

    /// 服务器状态快照（基线 `getServersStatus`，:480-493）。
    /// 子进程意外退出后状态经 `effective_status` 反映为
    /// `disconnected`（基线 :278-284 的 `exit` 事件）。
    pub fn get_servers_status(&self) -> Vec<MCPServerStatus> {
        let Ok(servers) = self.servers.lock() else {
            return Vec::new();
        };
        servers
            .values()
            .map(|runtime| MCPServerStatus {
                id: runtime.config.id.clone(),
                name: runtime.config.name.clone(),
                status: runtime.effective_status(),
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

    /// 基线 :186-191：新配置未覆盖、或与新配置深度不一致
    /// （基线 `isDeepStrictEqual` ↔ `PartialEq` 结构等价）的
    /// 已连接服务器断开（kill 子进程 + 拒绝在途请求）。
    fn drop_stale_servers(&self, configs: &BTreeMap<String, McpServerConfig>) {
        let stale: Vec<String> = {
            let Ok(servers) = self.servers.lock() else {
                return;
            };
            servers
                .iter()
                .filter(|(server_id, runtime)| match configs.get(*server_id) {
                    None => true,
                    Some(config) => runtime.config != *config,
                })
                .map(|(server_id, _)| server_id.clone())
                .collect()
        };
        for server_id in stale {
            let _ = self.disconnect(&server_id);
        }
    }
}

/// 服务器运行时（对齐基线 `MCPServerRuntime`，mcp-manager.ts:80-94）。
///
/// `transport` 为 `Some` 当且仅当子进程已 spawn（基线
/// `runtime.process`）；进程意外退出后运行时记录的 `status`
/// 不变，但 `is_connected` / `effective_status` 动态反映
/// 基线 `exit` 事件的 `disconnected` 翻转（:278-284）。
#[derive(Debug)]
struct McpServerRuntime {
    config: McpServerConfig,
    status: MCPConnectionStatus,
    tools: Vec<MCPToolDescription>,
    resources: Vec<MCPResourceDescription>,
    error: Option<String>,
    transport: Option<Arc<transport::McpStdioTransport>>,
}

impl McpServerRuntime {
    /// 是否处于「已连接且子进程存活」状态。
    fn is_connected(&self) -> bool {
        self.status == MCPConnectionStatus::Connected
            && !self
                .transport
                .as_ref()
                .is_some_and(|transport| transport.is_closed())
    }

    /// 对外可见状态（基线 `getServersStatus` 读取的 `r.status`；
    /// 基线进程退出后由 `exit` 事件翻为 `disconnected`，
    /// :278-284——本实现动态反映，见 [`Self::is_connected`]）。
    fn effective_status(&self) -> MCPConnectionStatus {
        if self.is_connected() {
            MCPConnectionStatus::Connected
        } else if self.status == MCPConnectionStatus::Connected {
            MCPConnectionStatus::Disconnected
        } else {
            self.status
        }
    }
}

/// 建立连接（基线 `connect` :229-243 的 try 段）：
/// spawn stdio 子进程 → `initializeSession` 握手 →
/// `tools/list` / `resources/list` 发现。
///
/// 任何失败 → `Err`（`connect` 层 catch :246-250 统一为
/// `'MCP 服务器连接失败'`，内部细节不透传）。
fn establish_connection(config: &McpServerConfig) -> Result<McpServerRuntime, String> {
    // 基线 :227-230：仅 stdio；否则 throw 'SSE 传输暂未实现'
    //（经 catch 统一为 'MCP 服务器连接失败'）。
    if config.transport != McpTransport::Stdio {
        return Err("SSE 传输暂未实现".to_string());
    }
    // 基线 connectStdio :255-272。
    let transport = Arc::new(transport::McpStdioTransport::spawn(config)?);
    // 基线 initializeSession :363-380。
    initialize_session(&transport)?;
    // 基线 discoverTools / discoverResources :382-404。
    let tools = discover_tools(&transport, &config.id);
    let resources = discover_resources(&transport, &config.id);
    Ok(McpServerRuntime {
        config: config.clone(),
        status: MCPConnectionStatus::Connected,
        tools,
        resources,
        error: None,
        transport: Some(transport),
    })
}

/// 初始化 MCP 会话（基线 `initializeSession`，:363-380）。
fn initialize_session(transport: &transport::McpStdioTransport) -> Result<(), String> {
    // 基线 :364-373：`protocolVersion:'2024-11-05'`、
    // `capabilities:{}`、`clientInfo:{name:'vela', version:'1.0.0'}`
    //（D-H4-2：照抄基线 `'vela'`）。
    transport.send_request(
        "initialize",
        Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "vela",
                "version": "1.0.0",
            },
        })),
    )?;
    // 基线 :375-378：`notifications/initialized` 通知
    //（无 id、无 params）。
    transport.send_notification("notifications/initialized")
}

/// 发现可用工具（基线 `discoverTools`，:382-391）。
/// 失败 → 空数组（基线 :389-390 catch），不报错。
fn discover_tools(
    transport: &transport::McpStdioTransport,
    server_id: &str,
) -> Vec<MCPToolDescription> {
    match transport.send_request("tools/list", None) {
        Ok(result) => map_tools_result(&result, server_id),
        Err(_) => Vec::new(),
    }
}

/// 发现可用资源（基线 `discoverResources`，:395-404）。
/// 失败 → 空数组（基线 :402-403 catch），不报错。
fn discover_resources(
    transport: &transport::McpStdioTransport,
    server_id: &str,
) -> Vec<MCPResourceDescription> {
    match transport.send_request("resources/list", None) {
        Ok(result) => map_resources_result(&result, server_id),
        Err(_) => Vec::new(),
    }
}

/// 基线 `discoverTools` 的映射段（:385-388）：`tools` 数组
/// 逐项展开并附加 `serverId`（基线 `{...t, serverId}`；
/// 缺失字段保留空默认，与基线展开语义一致）。
fn map_tools_result(result: &Value, server_id: &str) -> Vec<MCPToolDescription> {
    result
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .map(|tool| MCPToolDescription {
                    name: tool
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    description: tool
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    input_schema: tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                    server_id: server_id.to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 基线 `discoverResources` 的映射段（:398-401）：`resources`
/// 数组逐项展开并附加 `serverId`。
fn map_resources_result(result: &Value, server_id: &str) -> Vec<MCPResourceDescription> {
    result
        .get("resources")
        .and_then(Value::as_array)
        .map(|resources| {
            resources
                .iter()
                .map(|resource| MCPResourceDescription {
                    uri: resource
                        .get("uri")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    name: resource
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    description: resource
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    mime_type: resource
                        .get("mimeType")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    server_id: server_id.to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 基线 `callTool` 的文本聚合段（:425-431）：结果 `content`
/// 数组中 `type === 'text'` 片段的 `text` 以 `'\n'` 连接
/// （`result?.content ?? []` → 空字符串）。
fn extract_text_content(result: &Value) -> String {
    result
        .get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .map(|part| part.get("text").and_then(Value::as_str).unwrap_or(""))
                .collect::<Vec<&str>>()
                .join("\n")
        })
        .unwrap_or_default()
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

    /// 指向不存在命令的配置（spawn 必失败，用于错误路径测试）。
    fn failing_config() -> McpServerConfig {
        McpServerConfig {
            id: "broken".to_string(),
            name: "broken".to_string(),
            transport: McpTransport::Stdio,
            command: Some("lorekeeper-mcp-nonexistent-command".to_string()),
            args: None,
            env: None,
            url: None,
        }
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
    fn connect_unconfigured_server_reports_internal_error_test() {
        let manager = McpManager::new();
        // 基线 :210-212：未配置 → 内部错误
        // （命令层信封统一为 'MCP 服务器连接失败'）。
        assert_eq!(
            manager.connect("fs").unwrap_err(),
            "MCP 服务器未配置或配置尚未加载"
        );
        assert!(manager.get_servers_status().is_empty());
    }

    #[test]
    fn connect_spawn_failure_sets_error_status_test() {
        let manager = McpManager::new();
        let mut configs = BTreeMap::new();
        configs.insert("broken".to_string(), failing_config());
        manager.set_loaded_configs(configs);

        // 基线 :246-250 catch：统一 'MCP 服务器连接失败'
        // （内部细节不透传）。
        assert_eq!(manager.connect("broken").unwrap_err(), "MCP 服务器连接失败");

        // 基线 :249：`error` 态入表（get-servers-status 可观察）。
        let status = manager.get_servers_status();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].id, "broken");
        assert_eq!(status[0].status, MCPConnectionStatus::Error);
        assert_eq!(status[0].tool_count, 0);
        assert_eq!(status[0].error.as_deref(), Some("MCP 服务器连接失败"));
    }

    #[test]
    fn connect_sse_transport_is_rejected_test() {
        let manager = McpManager::new();
        let mut configs = BTreeMap::new();
        configs.insert(
            "remote".to_string(),
            McpServerConfig {
                id: "remote".to_string(),
                name: "remote".to_string(),
                transport: McpTransport::Sse,
                command: None,
                args: None,
                env: None,
                url: Some("https://example.invalid/mcp".to_string()),
            },
        );
        manager.set_loaded_configs(configs);
        // 基线 :228 'SSE 传输暂未实现' → catch 统一为
        // 'MCP 服务器连接失败'。
        assert_eq!(manager.connect("remote").unwrap_err(), "MCP 服务器连接失败");
        assert_eq!(
            manager.get_servers_status()[0].status,
            MCPConnectionStatus::Error
        );
    }

    #[test]
    fn disconnect_clears_failed_server_from_status_test() {
        let manager = McpManager::new();
        let mut configs = BTreeMap::new();
        configs.insert("broken".to_string(), failing_config());
        manager.set_loaded_configs(configs);
        let _ = manager.connect("broken");
        assert_eq!(manager.get_servers_status().len(), 1);
        // 基线 :438-449：断开后状态表移除该服务器。
        assert!(manager.disconnect("broken").is_ok());
        assert!(manager.get_servers_status().is_empty());
        // 基线 :438：未命中为无操作成功。
        assert!(manager.disconnect("broken").is_ok());
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

    #[test]
    fn extract_text_content_joins_text_parts_test() {
        let result = json!({
            "content": [
                {"type": "text", "text": "第一行"},
                {"type": "image", "data": "..."},
                {"type": "text", "text": "第二行"},
            ]
        });
        assert_eq!(extract_text_content(&result), "第一行\n第二行");
        // 基线 :427 `result?.content ?? []` → 空字符串。
        assert_eq!(extract_text_content(&json!({})), "");
        assert_eq!(extract_text_content(&json!({"content": []})), "");
        // text 缺失 → 空片段（基线 :430 `c.text ?? ''`）。
        assert_eq!(
            extract_text_content(&json!({"content": [{"type": "text"}]})),
            ""
        );
    }

    #[test]
    fn map_tools_result_attaches_server_id_test() {
        let result = json!({
            "tools": [
                {"name": "read", "description": "读文件", "inputSchema": {"type": "object"}},
                {"name": "write"},
            ]
        });
        let tools = map_tools_result(&result, "fs");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "read");
        assert_eq!(tools[0].description, "读文件");
        assert_eq!(tools[0].input_schema, json!({"type": "object"}));
        assert_eq!(tools[0].server_id, "fs");
        // 基线 `{...t, serverId}`：缺失字段保留为空默认。
        assert_eq!(tools[1].description, "");
        assert_eq!(tools[1].input_schema, json!({}));
    }

    #[test]
    fn map_resources_result_attaches_server_id_test() {
        let result = json!({
            "resources": [
                {"uri": "file:///a.md", "name": "a", "description": "d", "mimeType": "text/markdown"},
                {"uri": "file:///b.md", "name": "b"},
            ]
        });
        let resources = map_resources_result(&result, "fs");
        assert_eq!(resources.len(), 2);
        assert_eq!(resources[0].uri, "file:///a.md");
        assert_eq!(resources[0].mime_type.as_deref(), Some("text/markdown"));
        assert_eq!(resources[0].server_id, "fs");
        assert_eq!(resources[1].description, None);
        assert_eq!(resources[1].mime_type, None);
    }
}
