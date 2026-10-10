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
//! **本轮（Task 1/2）**：仅 `mcp:load-config` 与 `mcp:get-config-path`
//! 真实实现（配置层）；其余 7 个频道为契约类型占位（传输层 Task 3 落地）。
//! 管理器尚未在 `lib.rs` 的 `setup` 装配（装配随 Task 4/6 收口），
//! `state.mcp_manager()` 恒为 `None`，命令经 `manager_or_transient`
//! 回退临时实例——只读频道（`load-config`）行为与装配后一致。
//!
//! 注意：本轮**不注册** `lib.rs` 的 `generate_handler!`（注册与
//! `migrated-channels.ts` 重生成统一收口于 Task 6/7），故
//! `verify-channel-coverage` 口径「未迁移 mcp=9」保持不变。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use super::SimpleResult;
use crate::mcp::get_config_path;
use crate::mcp::McpManager;
use crate::mcp::{
    MCPConfigLoadResponse, MCPResourceDescription, MCPServerStatus, MCPToolCallResult,
    MCPToolDescription,
};
use crate::state::AppState;

/// 取 `AppState` 装配的 MCP 管理器；未装配时回退临时实例。
///
/// 本轮 `lib.rs` 的 `setup` 尚未装配管理器（装配随 Task 4/6 收口），
/// 临时实例保证 `mcp:load-config` 等频道的行为正确。
fn manager_or_transient(state: &AppState) -> Arc<McpManager> {
    state
        .mcp_manager()
        .unwrap_or_else(|| Arc::new(McpManager::new()))
}

/// `mcp:load-config` —— 加载 MCP 配置（真实实现，配置层）。
///
/// 对齐 `mcp-ipc-bridge.ts:17-30`：`status === 'error'` → `success:false`
/// （经 `MCPConfigLoadResponse` 的 `From` 转换）；基线外层 catch 的固定
/// 信封 `{success:false, status:'error', servers:[], error:'MCP 配置加载失败'}`
/// 在同步实现中无触发点（`load_config` 不抛异常），见
/// `MCPConfigLoadResponse::load_failed`。
#[tauri::command]
pub fn mcp_load_config(state: State<'_, AppState>) -> MCPConfigLoadResponse {
    manager_or_transient(state.inner()).load_config().into()
}

/// `mcp:connect` —— 连接服务器（占位，Task 3 落地传输层）。
///
/// 对齐 `mcp-ipc-bridge.ts:33-41`：内部错误细节一律不透传，
/// 统一返回 `'MCP 服务器连接失败'`。
#[tauri::command]
pub fn mcp_connect(server_id: String, state: State<'_, AppState>) -> SimpleResult {
    match manager_or_transient(state.inner()).connect(&server_id) {
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

/// `mcp:disconnect` —— 断开服务器（骨架：运行时表移除，Task 3 完善）。
///
/// 对齐 `mcp-ipc-bridge.ts:43-51`：catch → `String(error)`（细节透传）。
#[tauri::command]
pub fn mcp_disconnect(server_id: String, state: State<'_, AppState>) -> SimpleResult {
    match manager_or_transient(state.inner()).disconnect(&server_id) {
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

/// `mcp:disconnect-all` —— 断开全部服务器（骨架：清空运行时表，Task 3 完善）。
#[tauri::command]
pub fn mcp_disconnect_all(state: State<'_, AppState>) -> SimpleResult {
    match manager_or_transient(state.inner()).disconnect_all() {
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

/// `mcp:list-tools` —— 聚合工具列表（无信封直返；本轮无传输 → 空数组）。
#[tauri::command]
pub fn mcp_list_tools(state: State<'_, AppState>) -> Vec<MCPToolDescription> {
    manager_or_transient(state.inner()).get_all_tools()
}

/// `mcp:list-resources` —— 聚合资源列表（无信封直返；本轮无传输 → 空数组）。
#[tauri::command]
pub fn mcp_list_resources(state: State<'_, AppState>) -> Vec<MCPResourceDescription> {
    manager_or_transient(state.inner()).get_all_resources()
}

/// `mcp:call-tool` —— 调用工具（占位，Task 3 落地传输层）。
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
    manager_or_transient(state.inner()).call_tool(&server_id, &tool_name, args)
}

/// `mcp:get-servers-status` —— 服务器状态快照（无信封直返；本轮无传输 → 空数组）。
#[tauri::command]
pub fn mcp_get_servers_status(state: State<'_, AppState>) -> Vec<MCPServerStatus> {
    manager_or_transient(state.inner()).get_servers_status()
}

/// `mcp:get-config-path` —— 默认配置文件路径（无信封直返，真实实现）。
///
/// D-H4-1：`lorekeeper_home()/mcp_config.json`。
#[tauri::command]
pub fn mcp_get_config_path() -> String {
    get_config_path().to_string_lossy().into_owned()
}
