//! 批次 H（H4-1）：MCP 契约类型镜像。
//!
//! 逐字平移契约 `MCP` 命名空间（`src/shared/ipc-channels.ts:1096-1151`；
//! tauri-app 副本 `src/shared/ipc-channels.ts:1142-1195`）：字段名 /
//! 枚举字面量与契约一一对应（`#[serde(rename_all = "camelCase")]` +
//! 可选字段 `skip_serializing_if = "Option::is_none"`，对齐 `SimpleResult` 范式）。
//!
//! 配置侧内部类型（[`McpServerConfig`] / [`McpConfig`]，兼容 Claude
//! Desktop 配置文件格式，对齐基线 `electron/mcp/mcp-manager.ts:30` / `:48`）
//! **不上前端**——仅主进程解析配置文件时使用
//! （`mcpServers` 映射的逐字段校验见 `config.rs`）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 连接状态（契约 `MCPConnectionStatus`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MCPConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

/// 传输方式（契约 `'stdio' | 'sse'`）。
/// 内部类型，不上前端。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Sse,
}

/// 服务器摘要（契约 `MCPServerSummary`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPServerSummary {
    pub id: String,
    pub name: String,
    pub transport: McpTransport,
}

/// 服务器状态（契约 `MCPServerStatus`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPServerStatus {
    pub id: String,
    pub name: String,
    pub status: MCPConnectionStatus,
    pub tool_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 工具描述（契约 `MCPToolDescription`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPToolDescription {
    pub name: String,
    pub description: String,
    /// `inputSchema`：契约 `Record<string, unknown>`。
    pub input_schema: serde_json::Value,
    pub server_id: String,
}

/// 资源描述（契约 `MCPResourceDescription`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPResourceDescription {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub server_id: String,
}

/// 配置加载状态（契约 `MCPConfigLoadResult` 的判别字段 `status`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MCPConfigLoadStatus {
    Missing,
    Loaded,
    Error,
}

/// 配置加载结果（契约 `MCPConfigLoadResult` 三态联合的镜像）。
///
/// 契约中 `missing` / `loaded` 两态无 `error` 字段、`error` 态必带
/// `error: string`——以 `skip_serializing_if` 在序列化层面精确对齐。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPConfigLoadResult {
    pub status: MCPConfigLoadStatus,
    pub servers: Vec<MCPServerSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl MCPConfigLoadResult {
    /// `{status:'missing', servers:[]}`（基线 ENOENT 分支）。
    pub fn missing() -> Self {
        Self {
            status: MCPConfigLoadStatus::Missing,
            servers: Vec::new(),
            error: None,
        }
    }

    /// `{status:'loaded', servers}`。
    pub fn loaded(servers: Vec<MCPServerSummary>) -> Self {
        Self {
            status: MCPConfigLoadStatus::Loaded,
            servers,
            error: None,
        }
    }

    /// `{status:'error', servers:[], error}`。
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            status: MCPConfigLoadStatus::Error,
            servers: Vec::new(),
            error: Some(message.into()),
        }
    }
}

/// 配置加载响应（契约 `MCPConfigLoadResponse` = `MCPConfigLoadResult` + `success`）。
///
/// 对齐 `mcp-ipc-bridge.ts:17-30`：`status === 'error'` → `success:false`，
/// 其余两态 → `success:true`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPConfigLoadResponse {
    pub success: bool,
    pub status: MCPConfigLoadStatus,
    pub servers: Vec<MCPServerSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<MCPConfigLoadResult> for MCPConfigLoadResponse {
    fn from(result: MCPConfigLoadResult) -> Self {
        let success = result.status != MCPConfigLoadStatus::Error;
        Self {
            success,
            status: result.status,
            servers: result.servers,
            error: result.error,
        }
    }
}

impl MCPConfigLoadResponse {
    /// 基线 `mcp-ipc-bridge.ts:27-29` 的外层 catch 信封：
    /// `{success:false, status:'error', servers:[], error:'MCP 配置加载失败'}`。
    ///
    /// 同步实现中 `load_config` 不抛异常，该信封无触发点；保留构造器
    /// 使桥接契约完整可表达。
    pub fn load_failed() -> Self {
        Self {
            success: false,
            status: MCPConfigLoadStatus::Error,
            servers: Vec::new(),
            error: Some("MCP 配置加载失败".to_string()),
        }
    }
}

/// 工具调用结果（契约 `mcp:call-tool` 返回类型 `{success, content, error?}`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPToolCallResult {
    pub success: bool,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 单个服务器配置（基线 `MCPServerConfig`，mcp-manager.ts:30）。
/// 内部类型，不上前端。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    pub transport: McpTransport,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<BTreeMap<String, String>>,
    pub url: Option<String>,
}

/// 配置文件格式（基线 `MCPConfig`，mcp-manager.ts:48；兼容 Claude Desktop）。
/// 内部类型，不上前端。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConfig {
    /// `mcpServers` 映射（键 = 服务器 id）。
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 契约字段名逐字对齐：camelCase 字段、枚举小写字面量、
    /// 可选字段缺省（`error: None` 不出现在 JSON 中）。
    #[test]
    fn contract_wire_format_test() {
        let status = MCPServerStatus {
            id: "fs".to_string(),
            name: "fs".to_string(),
            status: MCPConnectionStatus::Connected,
            tool_count: 2,
            error: None,
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            serde_json::json!({"id": "fs", "name": "fs", "status": "connected", "toolCount": 2})
        );

        let result = MCPConfigLoadResult::error("MCP 配置损坏，未加载任何服务器");
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            serde_json::json!({
                "status": "error",
                "servers": [],
                "error": "MCP 配置损坏，未加载任何服务器"
            })
        );

        // `loaded` 态序列化不含 `error` 键（契约联合类型无此字段）。
        let response = MCPConfigLoadResponse::from(MCPConfigLoadResult::loaded(Vec::new()));
        assert_eq!(
            serde_json::to_value(&response).unwrap(),
            serde_json::json!({"success": true, "status": "loaded", "servers": []})
        );

        // 外层 catch 信封（桥接防御，同步实现中无触发点）。
        assert_eq!(
            serde_json::to_value(&MCPConfigLoadResponse::load_failed()).unwrap(),
            serde_json::json!({
                "success": false,
                "status": "error",
                "servers": [],
                "error": "MCP 配置加载失败"
            })
        );
    }
}
