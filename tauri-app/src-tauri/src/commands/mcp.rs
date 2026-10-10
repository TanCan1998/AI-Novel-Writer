//! 批次 H（H4-2）：MCP 频道 —— `mcp:*` 9 个 invoke 频道。
//!
//! 平移自 `electron/mcp/mcp-ipc-bridge.ts`（87 行）：
//! - 9 个频道一一对应；命令名 = 频道名机械映射（`:` / `-` → `_`）；
//! - 错误信封逐条对齐基线：
//!   - `mcp:load-config`：`status === 'error'` → `success:false`；
//!     外层异常 → `{success:false, status:'error', servers:[], error:'MCP 配置加载失败'}`；
//!   - `mcp:connect`：catch 一律 `'MCP 服务器连接失败'`（内部细节不透传）；
//!   - `mcp:disconnect` / `mcp:disconnect-all`：catch → `String(error)`（细节透传）；
//!   - `list-tools` / `list-resources` / `get-servers-status` / `get-config-path`：无信封直返。
//!
//! 管理器由 `AppState` 常驻装配（`state.rs` 的 `mcp` 字段），
//! 命令一律经 `state.mcp_manager()` 获取。
//!
//! 9 个命令已在 `lib.rs` 的 `generate_handler!` 注册（Task 6 收口），
//! `verify-channel-coverage` 口径「未迁移 invoke 频道」至此归零。

use std::collections::HashMap;

use serde_json::Value;
use tauri::State;

use super::SimpleResult;
use crate::mcp::get_config_path;
use crate::mcp::{
    MCPConfigLoadResponse, MCPResourceDescription, MCPServerStatus, MCPToolCallResult,
    MCPToolDescription,
};
use crate::state::AppState;

/// `mcp:load-config` —— 加载 MCP 配置（真实实现，配置层）。
///
/// 对齐 `mcp-ipc-bridge.ts:17-30`：`status === 'error'` → `success:false`
/// （经 `MCPConfigLoadResponse` 的 `From` 转换）；基线外层 catch 的固定
/// 信封 `{success:false, status:'error', servers:[], error:'MCP 配置加载失败'}`
/// 在同步实现中无触发点（`load_config` 不抛异常），见
/// `MCPConfigLoadResponse::load_failed`。
#[tauri::command]
pub fn mcp_load_config(state: State<'_, AppState>) -> MCPConfigLoadResponse {
    state.mcp_manager().load_config().into()
}

/// `mcp:connect` —— 连接服务器。
///
/// 对齐 `mcp-ipc-bridge.ts:33-41`：内部错误细节一律不透传，
/// 统一返回 `'MCP 服务器连接失败'`。
#[tauri::command]
pub fn mcp_connect(server_id: String, state: State<'_, AppState>) -> SimpleResult {
    match state.mcp_manager().connect(&server_id) {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(_) => SimpleResult {
            success: false,
            error: Some("MCP 服务器连接失败".to_string()),
        },
    }
}

/// `mcp:disconnect` —— 断开服务器（终止子进程 + 清理在途请求）。
///
/// 对齐 `mcp-ipc-bridge.ts:43-51`：catch → `String(error)`（细节透传）。
#[tauri::command]
pub fn mcp_disconnect(server_id: String, state: State<'_, AppState>) -> SimpleResult {
    match state.mcp_manager().disconnect(&server_id) {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => SimpleResult {
            success: false,
            error: Some(error),
        },
    }
}

/// `mcp:disconnect-all` —— 断开全部服务器（逐个 disconnect）。
#[tauri::command]
pub fn mcp_disconnect_all(state: State<'_, AppState>) -> SimpleResult {
    match state.mcp_manager().disconnect_all() {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => SimpleResult {
            success: false,
            error: Some(error),
        },
    }
}

/// `mcp:list-tools` —— 聚合工具列表（无信封直返）。
#[tauri::command]
pub fn mcp_list_tools(state: State<'_, AppState>) -> Vec<MCPToolDescription> {
    state.mcp_manager().get_all_tools()
}

/// `mcp:list-resources` —— 聚合资源列表（无信封直返）。
#[tauri::command]
pub fn mcp_list_resources(state: State<'_, AppState>) -> Vec<MCPResourceDescription> {
    state.mcp_manager().get_all_resources()
}

/// `mcp:call-tool` —— 调用工具。
///
/// 基线 `callTool` 自带错误返回（bridge 无 try/catch）：未连接 →
/// `{success:false, content:'', error:'服务器 <id> 未连接'}`。
#[tauri::command]
pub fn mcp_call_tool(
    server_id: String,
    tool_name: String,
    args: HashMap<String, Value>,
    state: State<'_, AppState>,
) -> MCPToolCallResult {
    state.mcp_manager().call_tool(&server_id, &tool_name, args)
}

/// `mcp:get-servers-status` —— 服务器状态快照（无信封直返）。
#[tauri::command]
pub fn mcp_get_servers_status(state: State<'_, AppState>) -> Vec<MCPServerStatus> {
    state.mcp_manager().get_servers_status()
}

/// `mcp:get-config-path` —— 默认配置文件路径（无信封直返，真实实现）。
///
/// D-H4-1：`lorekeeper_home()/mcp_config.json`。
#[tauri::command]
pub fn mcp_get_config_path() -> String {
    get_config_path().to_string_lossy().into_owned()
}
