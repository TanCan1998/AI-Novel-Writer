//! 批次 H（H4-1）：MCP 配置层。
//!
//! 平移基线 `electron/mcp/mcp-manager.ts` 的 `loadConfig`（:127-203）
//! 解析段：读取 `mcp_config.json`（D-H4-1：`lorekeeper_home()` 下，
//! **绝不**落 `~/.vela` / `AI_NOVEL_VELA_HOME`），按「兼容 Claude
//! Desktop」格式（`mcpServers` 映射）逐字段校验。
//!
//! 读取经 `json_store::try_read_json_value` 的三态结果映射基线 catch：
//! - `Missing`（ENOENT）→ `{status:'missing', servers:[]}`；
//! - `Error`（IO / JSON 解析失败）→ `'MCP 配置损坏或无法读取，未加载任何服务器'`；
//! - 结构 / 字段校验失败 → `'MCP 配置损坏，未加载任何服务器'`。
//!
//! 纯函数：不触碰管理器运行时状态；基线 `revoke`（先 `disconnectAll()`
//! 再返回错误）由 `McpManager::load_config` 编排。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::types::{
    MCPConfigLoadResult, MCPServerSummary, McpConfig, McpServerConfig, McpTransport,
};
use crate::app_paths;
use crate::json_store;

/// 配置文件名（D-H4-1：位于 `lorekeeper_home()` 下）。
pub const MCP_CONFIG_FILE: &str = "mcp_config.json";

/// 配置读取错误分类（基线 `loadConfig` catch 的三态语义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpConfigReadError {
    /// 文件不存在（基线 ENOENT → `status:'missing'`）。
    Missing,
    /// 结构 / 字段校验失败（基线 → `'MCP 配置损坏，未加载任何服务器'`）。
    Corrupt,
    /// 读取或 JSON 解析异常（基线 catch 非 ENOENT 分支
    /// → `'MCP 配置损坏或无法读取，未加载任何服务器'`）。
    Unreadable,
}

/// 默认配置文件路径（基线 `getDefaultConfigPath`，mcp-manager.ts:119）。
///
/// D-H4-1：`lorekeeper_home()/mcp_config.json`。
pub fn get_config_path() -> PathBuf {
    get_config_path_at(&app_paths::lorekeeper_home())
}

/// 注入数据根的纯函数版本（测试与可注入性范式，对齐 `app_paths::resolve_home`）。
pub fn get_config_path_at(home: &Path) -> PathBuf {
    home.join(MCP_CONFIG_FILE)
}

/// 读取并校验配置文件（基线 `loadConfig` 解析段，mcp-manager.ts:134-193）。
pub fn read_config(path: &Path) -> Result<McpConfig, McpConfigReadError> {
    match json_store::try_read_json_value(path) {
        json_store::JsonFileReadResult::Missing => Err(McpConfigReadError::Missing),
        // 基线：`JSON.parse` 抛错 / 读取异常 → catch 非 ENOENT 分支。
        json_store::JsonFileReadResult::Error(_) => Err(McpConfigReadError::Unreadable),
        json_store::JsonFileReadResult::Ok(raw) => {
            parse_config(&raw).ok_or(McpConfigReadError::Corrupt)
        }
    }
}

/// 读取并校验，返回契约结果（基线 `loadConfig` 返回段）。
pub fn load_config_at(path: &Path) -> MCPConfigLoadResult {
    match read_config(path) {
        Ok(config) => MCPConfigLoadResult::loaded(summarize(&config.mcp_servers)),
        Err(McpConfigReadError::Missing) => MCPConfigLoadResult::missing(),
        Err(McpConfigReadError::Corrupt) => {
            MCPConfigLoadResult::error("MCP 配置损坏，未加载任何服务器")
        }
        Err(McpConfigReadError::Unreadable) => {
            MCPConfigLoadResult::error("MCP 配置损坏或无法读取，未加载任何服务器")
        }
    }
}

/// `load_config_at` 的默认路径版本。
pub fn load_config() -> MCPConfigLoadResult {
    load_config_at(&get_config_path())
}

/// 基线 :134-141：顶层结构校验（`mcpServers` 必须是非数组对象）。
fn parse_config(raw: &Value) -> Option<McpConfig> {
    // `!parsed || typeof parsed !== 'object' || !('mcpServers' in parsed)`
    let servers_value = raw.get("mcpServers")?;
    // `!parsed.mcpServers || typeof parsed.mcpServers !== 'object'
    //  || Array.isArray(parsed.mcpServers)`
    let servers = servers_value.as_object()?;

    let mut mcp_servers = BTreeMap::new();
    for (id, entry) in servers {
        let config = parse_server_entry(id, entry)?;
        mcp_servers.insert(id.clone(), config);
    }
    Some(McpConfig { mcp_servers })
}

/// 基线 :149-184：单条目类型校验 + `hasCommand === hasUrl` 判定。
///
/// 任一字段类型不符、或 `command` 与 `url` 同有同缺（含仅空白字符）
/// → `None`（调用方映射为 `Corrupt`）。
fn parse_server_entry(id: &str, entry: &Value) -> Option<McpServerConfig> {
    // `!value || typeof value !== 'object' || Array.isArray(value)`
    let entry = entry.as_object()?;

    if let Some(command) = entry.get("command") {
        if !command.is_string() {
            return None;
        }
    }
    if let Some(args) = entry.get("args") {
        match args.as_array() {
            Some(items) if items.iter().all(|item| item.is_string()) => {}
            _ => return None,
        }
    }
    if let Some(env) = entry.get("env") {
        match env.as_object() {
            Some(map) if map.values().all(|value| value.is_string()) => {}
            _ => return None,
        }
    }
    if let Some(url) = entry.get("url") {
        if !url.is_string() {
            return None;
        }
    }

    // `hasCommand` / `hasUrl`：字段为 string 且 `trim()` 非空。
    let has_command = entry
        .get("command")
        .and_then(Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let has_url = entry
        .get("url")
        .and_then(Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    // `hasCommand === hasUrl`（同有同缺）→ 契约拒绝。
    if has_command == has_url {
        return None;
    }

    Some(McpServerConfig {
        id: id.to_string(),
        name: id.to_string(),
        transport: if has_url {
            McpTransport::Sse
        } else {
            McpTransport::Stdio
        },
        command: entry
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_string),
        args: entry.get("args").and_then(Value::as_array).map(|items| {
            items
                .iter()
                .map(|item| item.as_str().unwrap_or_default().to_string())
                .collect()
        }),
        env: entry.get("env").and_then(Value::as_object).map(|map| {
            map.iter()
                .map(|(key, value)| (key.clone(), value.as_str().unwrap_or_default().to_string()))
                .collect()
        }),
        url: entry.get("url").and_then(Value::as_str).map(str::to_string),
    })
}

/// 基线 :180-183：服务器摘要（`id` / `name` / `transport`）。
fn summarize(mcp_servers: &BTreeMap<String, McpServerConfig>) -> Vec<MCPServerSummary> {
    mcp_servers
        .values()
        .map(|config| MCPServerSummary {
            id: config.id.clone(),
            name: config.name.clone(),
            transport: config.transport,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::types::MCPConfigLoadStatus;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lorekeeper-mcp-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_returns_missing_status_test() {
        let dir = temp_dir("missing");
        let result = load_config_at(&get_config_path_at(&dir));
        assert_eq!(result.status, MCPConfigLoadStatus::Missing);
        assert!(result.servers.is_empty());
        assert_eq!(result.error, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn valid_stdio_config_returns_loaded_summary_test() {
        let dir = temp_dir("valid-stdio");
        let config_path = get_config_path_at(&dir);
        std::fs::write(
            &config_path,
            r#"{"mcpServers":{"fs":{"command":"npx","args":["-y","@modelcontextprotocol/server-filesystem"],"env":{"HOME":"/tmp"}}}}"#,
        )
        .unwrap();

        let result = load_config_at(&config_path);
        assert_eq!(result.status, MCPConfigLoadStatus::Loaded);
        assert_eq!(result.error, None);
        assert_eq!(result.servers.len(), 1);
        assert_eq!(result.servers[0].id, "fs");
        assert_eq!(result.servers[0].name, "fs");
        assert_eq!(result.servers[0].transport, McpTransport::Stdio);

        // 解析后的内部配置保留全部字段（Task 3 传输层消费）。
        let config = read_config(&config_path).unwrap();
        let server = config.mcp_servers.get("fs").unwrap();
        assert_eq!(server.command.as_deref(), Some("npx"));
        assert_eq!(
            server.args.as_deref(),
            Some(
                [
                    "-y".to_string(),
                    "@modelcontextprotocol/server-filesystem".to_string()
                ]
                .as_slice()
            )
        );
        assert_eq!(
            server
                .env
                .as_ref()
                .map(|env| env.get("HOME").map(String::as_str)),
            Some(Some("/tmp"))
        );
        assert_eq!(server.url, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn url_entry_maps_to_sse_transport_test() {
        let dir = temp_dir("valid-sse");
        let config_path = get_config_path_at(&dir);
        std::fs::write(
            &config_path,
            r#"{"mcpServers":{"web":{"url":"https://example.com/mcp"}}}"#,
        )
        .unwrap();

        let result = load_config_at(&config_path);
        assert_eq!(result.status, MCPConfigLoadStatus::Loaded);
        assert_eq!(result.servers[0].transport, McpTransport::Sse);

        let config = read_config(&config_path).unwrap();
        let server = config.mcp_servers.get("web").unwrap();
        assert_eq!(server.transport, McpTransport::Sse);
        assert_eq!(server.url.as_deref(), Some("https://example.com/mcp"));
        assert_eq!(server.command, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_servers_map_returns_loaded_empty_test() {
        let dir = temp_dir("empty-map");
        let config_path = get_config_path_at(&dir);
        std::fs::write(&config_path, r#"{"mcpServers":{}}"#).unwrap();

        let result = load_config_at(&config_path);
        assert_eq!(result.status, MCPConfigLoadStatus::Loaded);
        assert!(result.servers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_json_returns_unreadable_error_test() {
        // 基线：`JSON.parse` 抛错 → catch 非 ENOENT 分支。
        let dir = temp_dir("bad-json");
        let config_path = get_config_path_at(&dir);
        std::fs::write(&config_path, "{ broken").unwrap();

        let result = load_config_at(&config_path);
        assert_eq!(result.status, MCPConfigLoadStatus::Error);
        assert!(result.servers.is_empty());
        assert_eq!(
            result.error.as_deref(),
            Some("MCP 配置损坏或无法读取，未加载任何服务器")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_field_type_returns_corrupt_error_test() {
        let cases = [
            // `command` 非 string
            r#"{"mcpServers":{"a":{"command":123}}}"#,
            // `args` 非 string[]
            r#"{"mcpServers":{"a":{"command":"x","args":"-y"}}}"#,
            // `args` 含非 string 元素
            r#"{"mcpServers":{"a":{"command":"x","args":["-y",1]}}}"#,
            // `env` 值非 string
            r#"{"mcpServers":{"a":{"command":"x","env":{"A":1}}}}"#,
            // `env` 为数组
            r#"{"mcpServers":{"a":{"command":"x","env":[]}}}"#,
            // `url` 非 string
            r#"{"mcpServers":{"a":{"url":42}}}"#,
            // 条目非对象
            r#"{"mcpServers":{"a":"not-an-object"}}"#,
            // 顶层无 `mcpServers` 键
            r#"{"other":{}}"#,
            // `mcpServers` 为数组
            r#"{"mcpServers":[]}"#,
            // 顶层非对象
            r#"42"#,
        ];
        for (index, case) in cases.iter().enumerate() {
            let dir = temp_dir(&format!("corrupt-{index}"));
            let config_path = get_config_path_at(&dir);
            std::fs::write(&config_path, case).unwrap();

            let result = load_config_at(&config_path);
            assert_eq!(result.status, MCPConfigLoadStatus::Error, "用例 {index}");
            assert!(result.servers.is_empty(), "用例 {index}");
            assert_eq!(
                result.error.as_deref(),
                Some("MCP 配置损坏，未加载任何服务器"),
                "用例 {index}"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn command_and_url_both_present_or_absent_returns_corrupt_error_test() {
        let cases = [
            // 同有
            r#"{"mcpServers":{"a":{"command":"x","url":"https://example.com/mcp"}}}"#,
            // 同缺
            r#"{"mcpServers":{"a":{}}}"#,
            // `command` 仅空白字符 → `hasCommand=false`，与 `hasUrl=false` 同缺
            r#"{"mcpServers":{"a":{"command":"   "}}}"#,
        ];
        for (index, case) in cases.iter().enumerate() {
            let dir = temp_dir(&format!("has-command-url-{index}"));
            let config_path = get_config_path_at(&dir);
            std::fs::write(&config_path, case).unwrap();

            let result = load_config_at(&config_path);
            assert_eq!(result.status, MCPConfigLoadStatus::Error, "用例 {index}");
            assert_eq!(
                result.error.as_deref(),
                Some("MCP 配置损坏，未加载任何服务器"),
                "用例 {index}"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// 路径红线（D-H4-1）：`AI_NOVEL_LOREKEEPER_HOME` 指向临时目录时，
    /// 默认配置路径必须落在该目录下的 `mcp_config.json`，且不含 `.vela`。
    ///
    /// 注意：`AI_NOVEL_LOREKEEPER_HOME` 是**进程级**环境变量，本测试会
    /// 保存并恢复它（对齐 `commands/llm.rs` 的
    /// `command_wrappers_use_lorekeeper_home_test` 范式）；其它测试都
    /// 不读取该变量（`app_paths` 的解析逻辑用注入式纯函数测试），
    /// 因此这里串行化风险可接受。
    #[test]
    fn config_path_follows_lorekeeper_home_env_test() {
        let dir = temp_dir("path-redline");
        let previous = std::env::var(app_paths::LOREKEEPER_HOME_ENV).ok();
        std::env::set_var(app_paths::LOREKEEPER_HOME_ENV, &dir);

        let config_path = get_config_path();
        assert_eq!(config_path, dir.join("mcp_config.json"));
        assert!(
            !config_path.to_string_lossy().contains(".vela"),
            "配置路径不得落 .vela（D-H4-1）：{}",
            config_path.display()
        );

        match previous {
            Some(value) => std::env::set_var(app_paths::LOREKEEPER_HOME_ENV, value),
            None => std::env::remove_var(app_paths::LOREKEEPER_HOME_ENV),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
