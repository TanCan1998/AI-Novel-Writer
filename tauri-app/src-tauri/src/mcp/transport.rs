//! 批次 H（H4 Task 3）：MCP stdio 传输层 —— 平移自基线
//! `electron/mcp/mcp-manager.ts`：
//! - `connectStdio`（:255-290）：`std::process::Command` argv 直传
//!   （不经 shell）、env 继承当前进程环境后叠加用户配置、
//!   stdin/stdout/stderr 全管道、Windows `CREATE_NO_WINDOW`
//!   （D-H4-3，对齐基线 `windowsHide: true`，:264）；
//! - `processBuffer`（:301-318）/ `handleMessage`（:320-336）：
//!   stdout 读线程按 `\n` 分帧、忽略非 JSON 行与通知、按 `id`
//!   匹配 pending 请求表；
//! - `sendRequest`（:338-361）：请求体
//!   `{jsonrpc:'2.0', id, method, params: params ?? {}}`、
//!   **10 秒超时** → `MCP 请求超时: <method>`。
//!
//! 握手参数（`initializeSession`，:363-380）见 `mcp/mod.rs`。
//!
//! 零新依赖：仅 std（线程 / 通道 / 进程）+ 已在依赖中的 serde_json。
//!
//! 与基线的差异（实施登记，理由均为同步模型等价替换）：
//! - 基线靠 Node 事件循环并发；本实现为同步阻塞模型，
//!   `send_request` 在调用线程上阻塞至多 10 秒（语义等价，
//!   仅并发度不同）。
//! - 基线对 stdin 写失败不处理（请求挂起到 10 秒超时）；
//!   本实现立即以 `连接已断开` 失败——管道关闭即进程已退出，
//!   避免无谓悬挂（基线最终也只能得到超时错误）。
//! - 基线进程退出后由 `exit` 事件把运行时状态翻为
//!   `disconnected`（:278-284）；本实现以传输层关闭标志动态
//!   反映（见 `mcp/mod.rs` 的 `McpServerRuntime::effective_status`）。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use super::types::McpServerConfig;

/// 请求超时（基线 :352-358：10 秒）。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Windows 子进程创建标志 `CREATE_NO_WINDOW`（D-H4-3，
/// 对齐基线 spawn 的 `windowsHide: true`，:264）。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 单个 pending 请求的回应通道（一次性，
/// 基线 `pendingRequests` 表项的 `{resolve, reject}`，:323）。
type PendingSender = mpsc::Sender<Result<Value, String>>;
type PendingReceiver = mpsc::Receiver<Result<Value, String>>;

/// 传输层共享状态（stdout 读线程与请求线程之间）。
///
/// 对应基线 `MCPServerRuntime` 的 `nextRequestId` /
/// `pendingRequests`（:80-94）与进程退出后的断开语义。
#[derive(Debug)]
struct TransportShared {
    /// pending 请求表（基线 `runtime.pendingRequests`）。
    pending: Mutex<HashMap<i64, PendingSender>>,
    /// 下一个请求 id（基线 `runtime.nextRequestId`，自 1 起，:84）。
    next_request_id: AtomicI64,
    /// 传输已关闭（子进程退出 / 主动断开）。
    closed: AtomicBool,
}

impl TransportShared {
    fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            next_request_id: AtomicI64::new(1),
            closed: AtomicBool::new(false),
        }
    }

    /// 分配下一个请求 id（基线 `runtime.nextRequestId++`，:341）。
    fn next_id(&self) -> i64 {
        self.next_request_id.fetch_add(1, Ordering::SeqCst)
    }

    /// 注册 pending 请求并返回回应接收端（基线 :342-343）。
    fn register_pending(&self, id: i64) -> PendingReceiver {
        let (sender, receiver) = mpsc::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, sender);
        }
        receiver
    }

    /// 取出并移除 pending 请求（基线 :323-324 `get` + `delete`）。
    fn take_pending(&self, id: i64) -> Option<PendingSender> {
        self.pending.lock().ok()?.remove(&id)
    }

    /// 传输是否已关闭。
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// 关闭传输：置位关闭标志并以 `message` 拒绝全部在途请求
    /// （基线 `disconnect` 的 `reject('连接已断开')`，:440-441；
    /// `connect` catch 的 `reject('MCP 服务器连接失败')`，:247）。
    fn close(&self, message: &str) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let drained = self
            .pending
            .lock()
            .map(|mut pending| std::mem::take(&mut *pending))
            .unwrap_or_default();
        for (_, sender) in drained {
            let _ = sender.send(Err(message.to_string()));
        }
    }
}

/// 构造 JSON-RPC 请求帧（基线 :345-350：
/// `{jsonrpc:'2.0', id, method, params: params ?? {}}` + `\n`）。
fn build_request_frame(id: i64, method: &str, params: Option<Value>) -> String {
    let message = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params.unwrap_or_else(|| json!({})),
    });
    format!("{}\n", message)
}

/// 构造 JSON-RPC 通知帧（基线 :375-378：
/// `{jsonrpc:'2.0', method}` + `\n`，无 id、无 params）。
fn build_notification_frame(method: &str) -> String {
    let message = json!({
        "jsonrpc": "2.0",
        "method": method,
    });
    format!("{}\n", message)
}

/// 处理一行 stdout 输出（基线 `processBuffer` 的单行语义
/// :306-317 + `handleMessage` :320-336）。
///
/// 空行 / 非 JSON 行 / 无 `id` 的通知消息 / 未知 `id` 的响应
/// 一律忽略（不 panic、不影响其他 pending 请求）。
fn handle_line(line: &str, shared: &TransportShared) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    let message: Value = match serde_json::from_str(trimmed) {
        Ok(message) => message,
        Err(_) => return, // 基线 :314-316：非 JSON 行忽略
    };
    // 基线 :321：仅处理带非 null `id` 的响应消息。
    let Some(id) = message.get("id").and_then(Value::as_i64) else {
        return;
    };
    let Some(sender) = shared.take_pending(id) else {
        return;
    };
    // 基线 :325-332：`error` → `err?.message ?? 'MCP error'`；
    // 否则 resolve `msg.result`。
    let response = match message.get("error") {
        Some(error) => {
            let detail = error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| "MCP error".to_string());
            Err(detail)
        }
        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
    };
    let _ = sender.send(response);
}

/// stdout 读线程主循环（基线 `processBuffer` :301-318 的逐行语义）。
///
/// 按 `\n` 分帧（CRLF 容忍：行尾 `\r` 一并去掉）；**非 UTF-8 行仅损失
/// 该行、连接继续存活**——基线对非法字节不敏感（替换字符不视为错误），
/// 而 `BufReader::lines()` 遇非法 UTF-8 会返回 `Err`，故改按字节读行
/// （`read_until`）。EOF / 读错误 → 退出循环，由调用方走既有 `close`
/// 清理路径（对齐基线 `proc.on('exit')`）。

fn read_dispatch_loop<R: BufRead>(reader: &mut R, shared: &TransportShared) {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                    if buf.last() == Some(&b'\r') {
                        buf.pop();
                    }
                }
                let line = String::from_utf8_lossy(&buf);
                handle_line(&line, shared);
            }
            Err(_) => break,
        }
    }
}
/// 等待请求回应（基线 :353-359 的 10 秒超时分支）。
///
/// 超时 → 从 pending 表移除并拒绝 `MCP 请求超时: <method>`。
fn wait_response(
    receiver: PendingReceiver,
    shared: &TransportShared,
    id: i64,
    timeout: Duration,
    method: &str,
) -> Result<Value, String> {
    match receiver.recv_timeout(timeout) {
        Ok(response) => response,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // 基线 :355-357：仅在请求仍挂起时拒绝（避免重复拒绝）。
            shared.take_pending(id);
            Err(format!("MCP 请求超时: {}", method))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            // 回应通道已断开（请求在等待期间被移除）——按超时口径拒绝。
            shared.take_pending(id);
            Err(format!("MCP 请求超时: {}", method))
        }
    }
}

/// stdio 传输层：持有 MCP 子进程及其双向管道。
///
/// 对应基线 `MCPServerRuntime.process` + stdout 读线程
/// （`connectStdio` :255-290 的 spawn 与事件监听）。
#[derive(Debug)]
pub struct McpStdioTransport {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    shared: Arc<TransportShared>,
}

impl McpStdioTransport {
    /// 启动 stdio 子进程（基线 `connectStdio` :255-272）。
    ///
    /// - `command` 缺失 / 为空 → `Err('stdio 模式需要 command 参数')`
    ///   （基线 :258-260）；
    /// - spawn 失败 → `Err('MCP 服务器连接失败')`（基线 :285-289
    ///   `proc.on('error')`：`connecting` 态的 pending 统一以
    ///   `'MCP 服务器连接失败'` 拒绝）。
    pub fn spawn(config: &McpServerConfig) -> Result<Self, String> {
        let command = config.command.clone().unwrap_or_default();
        if command.is_empty() {
            return Err("stdio 模式需要 command 参数".to_string());
        }

        let mut command = Command::new(&command);
        // 基线 :261：`spawn(command, args)` —— argv 直传、不经 shell。
        command.args(config.args.clone().unwrap_or_default());
        // 基线 :262：`env: { ...process.env, ...env }` —— 继承当前
        // 进程环境后叠加用户配置。
        if let Some(env) = &config.env {
            command.envs(env.iter());
        }
        // 基线 :263：`stdio: ['pipe','pipe','pipe']`。
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // 基线 :264：`windowsHide: true` → `CREATE_NO_WINDOW`（D-H4-3）。
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);

        let mut child = command
            .spawn()
            .map_err(|_| "MCP 服务器连接失败".to_string())?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "MCP 服务器连接失败".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "MCP 服务器连接失败".to_string())?;
        let stderr = child.stderr.take();

        let shared = Arc::new(TransportShared::new());

        // 基线 :268-270：stdout 数据 → 缓冲区 → processBuffer。
        // 本实现以读线程逐行处理（`\n` 分帧），等价于事件回调。
        {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                read_dispatch_loop(&mut reader, &shared);
                // 基线 :278-284 `proc.on('exit')`：进程退出 / 管道
                // 关闭 → 拒绝全部在途请求（`连接已断开`）。
                shared.close("连接已断开");
            });
        }

        // 基线 :272-274：stderr 只记日志。本实现仅排水（防止管道
        // 缓冲区阻塞子进程），内容丢弃。
        if let Some(stderr) = stderr {
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines() {
                    let _ = line;
                }
            });
        }

        Ok(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            shared,
        })
    }

    /// 传输是否已关闭（子进程退出或主动断开）。
    ///
    /// 供管理器动态反映基线 `exit` 事件的状态翻转
    /// （:278-284，见 `mcp/mod.rs` 的 `effective_status`）。
    pub fn is_closed(&self) -> bool {
        self.shared.is_closed()
    }

    /// 发送 JSON-RPC 请求并等待回应（基线 `sendRequest` :338-361）。
    ///
    /// 传输已关闭 → `Err('连接已断开')`；10 秒无回应 →
    /// `Err('MCP 请求超时: <method>')`；JSON-RPC error →
    /// `Err(error.message ?? 'MCP error')`。
    pub fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value, String> {
        // 传输已关闭 → 在途 / 新请求报 `连接已断开`。
        if self.shared.is_closed() {
            return Err("连接已断开".to_string());
        }
        let id = self.shared.next_id();
        let receiver = self.shared.register_pending(id);
        // 注册与关闭的竞态兜底：注册后若传输已关闭，立即失败。
        if self.shared.is_closed() {
            self.shared.take_pending(id);
            return Err("连接已断开".to_string());
        }
        let frame = build_request_frame(id, method, params);
        if self.write_frame(&frame).is_err() {
            // stdin 写失败 = 管道关闭 = 进程已退出（差异登记见模块文档）。
            self.shared.close("连接已断开");
            return Err("连接已断开".to_string());
        }
        wait_response(receiver, &self.shared, id, REQUEST_TIMEOUT, method)
    }

    /// 发送 JSON-RPC 通知（无 id、无 params，基线 :375-378 的
    /// `notifications/initialized` 写法）。
    pub fn send_notification(&self, method: &str) -> Result<(), String> {
        if self.shared.is_closed() {
            return Err("连接已断开".to_string());
        }
        let frame = build_notification_frame(method);
        if self.write_frame(&frame).is_err() {
            self.shared.close("连接已断开");
            return Err("连接已断开".to_string());
        }
        Ok(())
    }

    /// 写一帧到子进程 stdin。
    fn write_frame(&self, frame: &str) -> std::io::Result<()> {
        let mut stdin = self
            .stdin
            .lock()
            .map_err(|_| std::io::Error::other("stdin lock poisoned"))?;
        stdin.write_all(frame.as_bytes())?;
        stdin.flush()
    }

    /// 终止子进程并拒绝全部在途请求（`连接已断开`）。
    ///
    /// 基线 `disconnect` :439-442 的 `kill()` +
    /// `reject('连接已断开')`。
    pub fn shutdown(&self) {
        self.kill_child();
        self.shared.close("连接已断开");
    }

    fn kill_child(&self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for McpStdioTransport {
    fn drop(&mut self) {
        // 兜底：运行时被移除 / 连接失败时确保子进程不泄漏
        // （kickoff §6 风险 #2）。
        self.kill_child();
        self.shared.close("连接已断开");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册一个 pending 请求，返回共享状态与回应接收端。
    fn shared_with_pending(id: i64) -> (Arc<TransportShared>, PendingReceiver) {
        let shared = Arc::new(TransportShared::new());
        let receiver = shared.register_pending(id);
        (shared, receiver)
    }

    #[test]
    fn request_frame_format_test() {
        // 基线 :345-350：`{jsonrpc:'2.0', id, method, params}`。
        let frame = build_request_frame(1, "initialize", Some(json!({"k": "v"})));
        assert_eq!(
            serde_json::from_str::<Value>(frame.trim_end()).unwrap(),
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"k": "v"}})
        );
        // params 缺省 → `{}`（基线 `params ?? {}`）。
        let frame = build_request_frame(2, "tools/list", None);
        assert_eq!(
            serde_json::from_str::<Value>(frame.trim_end()).unwrap(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        );
    }

    #[test]
    fn notification_frame_format_test() {
        // 基线 :375-378：通知无 id、无 params。
        let frame = build_notification_frame("notifications/initialized");
        assert_eq!(
            serde_json::from_str::<Value>(frame.trim_end()).unwrap(),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        );
    }

    #[test]
    fn handle_line_resolves_pending_request_test() {
        let (shared, receiver) = shared_with_pending(1);
        handle_line(r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#, &shared);
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(Ok(json!({"tools": []})))
        );
        // pending 已移除（基线 :323-324 `delete`）。
        assert!(shared.take_pending(1).is_none());
    }

    #[test]
    fn handle_line_maps_rpc_error_test() {
        // 基线 :326-329：`err?.message ?? 'MCP error'`。
        let (shared, receiver) = shared_with_pending(7);
        handle_line(
            r#"{"jsonrpc":"2.0","id":7,"error":{"code":-32000,"message":"boom"}}"#,
            &shared,
        );
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(Err("boom".to_string()))
        );

        // error 无 message 字段 → 'MCP error'。
        let (shared, receiver) = shared_with_pending(8);
        handle_line(
            r#"{"jsonrpc":"2.0","id":8,"error":{"code":-32000}}"#,
            &shared,
        );
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(Err("MCP error".to_string()))
        );
    }

    #[test]
    fn handle_line_ignores_noise_test() {
        let (shared, receiver) = shared_with_pending(1);
        // 空行 / 非 JSON 行（基线 :309-316）/ 无 id 通知（:321）/
        // 未知 id 响应 → 均不解析、不 panic。
        handle_line("", &shared);
        handle_line("   ", &shared);
        handle_line("not json at all", &shared);
        handle_line(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            &shared,
        );
        handle_line(r#"{"jsonrpc":"2.0","id":999,"result":null}"#, &shared);
        // 原 pending 不受影响：sender 仍在表内，
        // 短时间内不会有消息送达。
        assert_eq!(
            receiver.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        assert!(shared.take_pending(1).is_some());
    }

    #[test]
    fn close_drains_pending_with_message_test() {
        let (shared, receiver_a) = shared_with_pending(1);
        let receiver_b = shared.register_pending(2);
        shared.close("连接已断开");
        assert!(shared.is_closed());
        assert_eq!(
            receiver_a.recv_timeout(Duration::from_secs(1)),
            Ok(Err("连接已断开".to_string()))
        );
        assert_eq!(
            receiver_b.recv_timeout(Duration::from_secs(1)),
            Ok(Err("连接已断开".to_string()))
        );
        // 关闭幂等：重复 close 不再拒绝（不 panic）。
        shared.close("MCP 服务器连接失败");
    }

    #[test]
    fn wait_response_timeout_removes_pending_test() {
        let (shared, receiver) = shared_with_pending(5);
        // 超时路径（基线 :353-359）：注入短超时，验证错误文案与
        // pending 清理。
        let error = wait_response(
            receiver,
            &shared,
            5,
            Duration::from_millis(10),
            "tools/list",
        )
        .unwrap_err();
        assert_eq!(error, "MCP 请求超时: tools/list");
        assert!(shared.take_pending(5).is_none());
    }

    #[test]
    fn spawn_rejects_missing_command_test() {
        // 基线 :258-260：`stdio 模式需要 command 参数`。
        let config = McpServerConfig {
            id: "fs".to_string(),
            name: "fs".to_string(),
            transport: crate::mcp::types::McpTransport::Stdio,
            command: None,
            args: None,
            env: None,
            url: None,
        };
        assert_eq!(
            McpStdioTransport::spawn(&config).unwrap_err(),
            "stdio 模式需要 command 参数"
        );
        let config = McpServerConfig {
            command: Some(String::new()),
            ..config
        };
        assert_eq!(
            McpStdioTransport::spawn(&config).unwrap_err(),
            "stdio 模式需要 command 参数"
        );
    }

    #[test]
    fn read_loop_frames_crlf_lines_test() {
        // CRLF 容忍：行尾 `\r\n` 与 `\n` 同样分帧解析。
        let (shared, receiver) = shared_with_pending(1);
        let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\r\n";
        let mut cursor = std::io::Cursor::new(&input[..]);
        read_dispatch_loop(&mut cursor, &shared);
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(Ok(json!({"ok": true})))
        );
        assert!(shared.take_pending(1).is_none());
    }

    #[test]
    fn read_loop_skips_invalid_utf8_line_test() {
        // 非 UTF-8 行仅损失该行、连接存活：其后合法响应仍能送达。
        let (shared, receiver) = shared_with_pending(1);
        let mut input: Vec<u8> = b"bad\xff\xfe line\n".to_vec();
        input.extend_from_slice(
            b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"after\":\"lossy\"}}\n",
        );
        let mut cursor = std::io::Cursor::new(input);
        read_dispatch_loop(&mut cursor, &shared);
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(Ok(json!({"after": "lossy"})))
        );
        assert!(shared.take_pending(1).is_none());
    }

    #[test]
    fn wait_response_returns_disconnected_on_close_test() {
        // 等待期间 close() → 在途请求以 `连接已断开` 拒绝。
        let (shared, receiver) = shared_with_pending(9);
        let closer = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                shared.close("连接已断开");
            })
        };
        let error =
            wait_response(receiver, &shared, 9, Duration::from_secs(5), "tools/list").unwrap_err();
        closer.join().unwrap();
        assert_eq!(error, "连接已断开");
        assert!(shared.take_pending(9).is_none());
    }

    #[test]
    fn close_is_idempotent_test() {
        // 幂等：连续两次 close 不 panic、不重复清理、
        // 首条拒绝文案不被后一次覆盖。
        let (shared, receiver) = shared_with_pending(3);
        shared.close("连接已断开");
        shared.close("MCP 服务器连接失败");
        assert!(shared.is_closed());
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(Err("连接已断开".to_string()))
        );
        // pending 已在首次 close 时清空；第二次 close 未再处置任何请求。
        assert!(shared.pending.lock().unwrap().is_empty());
        assert!(shared.take_pending(3).is_none());
    }

    #[test]
    fn spawn_node_subprocess_e2e_test() {
        // 真实子进程 e2e：node 起一行假 MCP server，逐行应答
        // initialize / tools/list / resources/list。
        let script = r#"const rl=require('readline').createInterface({input:process.stdin});rl.on('line',l=>{let m;try{m=JSON.parse(l)}catch(e){return}let res=null;if(m.method==='initialize')res={serverInfo:{name:'fake'}};else if(m.method==='tools/list')res={tools:[{name:'fake_tool'}]};else if(m.method==='resources/list')res={resources:[{uri:'fake://res'}]};else return;process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:m.id,result:res})+'\n')})"#;
        let config = McpServerConfig {
            id: "fake".to_string(),
            name: "fake".to_string(),
            transport: crate::mcp::types::McpTransport::Stdio,
            command: Some("node".to_string()),
            args: Some(vec!["-e".to_string(), script.to_string()]),
            env: None,
            url: None,
        };
        let transport = match McpStdioTransport::spawn(&config) {
            Ok(transport) => transport,
            Err(_) => {
                // 环境无 node → 跳过（不允许因缺 node 而红）。
                println!("跳过：环境无 node，子进程 e2e 测试未运行");
                return;
            }
        };
        // 握手：initialize → result.serverInfo。
        let init = transport
            .send_request("initialize", Some(json!({})))
            .unwrap();
        assert_eq!(init["serverInfo"]["name"], json!("fake"));
        // tools/list → 一个工具。
        let tools = transport.send_request("tools/list", None).unwrap();
        assert_eq!(tools["tools"][0]["name"], json!("fake_tool"));
        // resources/list → 一个资源。
        let resources = transport.send_request("resources/list", None).unwrap();
        assert_eq!(resources["resources"][0]["uri"], json!("fake://res"));
    }
}
