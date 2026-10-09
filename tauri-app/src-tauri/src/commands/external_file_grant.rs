//! 批次 H：外部文件授权域 —— 平移自 `electron/controllers/external-file-grant-controller.ts`。
//!
//! ADR 0002：用户经系统对话框显式授权后才签发 grant；渲染进程只携带 **不透明 grantId**
//! 与**受限相对路径**，绝不传入/持有绝对路径。路径只存在于 `external_grant` 注册表内。
//!
//! 语义对齐（逐条对基线）：
//!
//! | 频道 | 基线行为 | 本实现 |
//! |---|---|---|
//! | `dialog:select-export-directory` | 目录选择 → `issueDirectory(['write','create'], TTL 10min, maxUses 4096)` → `{grantId, displayName: basename}` | 同（tauri-plugin-dialog + `issue_directory_with_operations`） |
//! | `fs:grant-read-file` | `resolve('read')`（消费 1 次）→ 读取文本 | 同 |
//! | `fs:grant-write-file` | `resolve('write')`（消费）→ 存在性 + `revalidate('create')` 探测 → 原子写入（ready 阶段复检）→ `mustAlreadyExist = !canCreate` | 同 |
//! | `fs:grant-mkdir` | `resolve('create')`（消费）→ 建目录 | 同 |
//!
//! **刻意偏离（已评估，均为更安全或不可观测）**：
//! 1. 基线把 `operations` 存入 grant 但 `resolveRequest` 并**未**校验操作集合（权限由
//!    webContents 归属 + 安全 fs 承担）；本实现在注册表内校验操作集合（`外部文件授权不允许X操作`）。
//! 2. 本实现**先做纯相对路径校验再消费配额**（基线在消费后才校验），避免无效输入烧掉一次授权；
//!    可观测差异仅限「无效相对路径是否消耗一次配额」。
//! 3. Tauri 无 `webContents` 归属语义（单窗口），故不做 `revokeWhenSenderIsDestroyed`。
//! 4. 基线依赖 Windows 句柄链安全 helper（`windows-safe-file-system`）；本实现用
//!    「词法归一 + 存在祖先 canonical 容器校验 + 同目录临时文件原 rename」达到等价目标
//!    （拒绝 junction/symlink 逃逸；失败提交态语义见 `write_text_atomically`）。

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::commands::project::file_path_to_string;
use crate::external_grant::{ExternalGrantRegistry, GrantOperation, EXPORT_GRANT_MAX_USES, EXPORT_GRANT_TTL};
use crate::state::AppState;

const MAIN_WINDOW_LABEL: &str = "main";
/// 对话框等待上限（安全网，与 `commands/kb.rs` 一致）
const DIALOG_TIMEOUT: Duration = Duration::from_secs(600);
const EXPORT_DIRECTORY_TITLE: &str = "选择导出目录";

// ===== 信封类型（与契约一致，勿改字段名）=====

/// 对齐契约 `ExternalFileGrant` / `ExternalDirectoryGrant`：只携带展示名与不透明能力标识。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalDirectoryGrant {
    pub grant_id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantReadResult {
    pub success: bool,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 契约判别联合：成功 `{success:true}`；失败 `{success:false, commitState, error?}`。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantWriteResult {
    pub success: bool,
    /// 成功侧不序列化（对齐契约的成功分支只有一个字段）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_state: Option<GrantFailureCommitState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl GrantWriteResult {
    fn ok() -> Self {
        Self { success: true, commit_state: None, error: None }
    }

    fn failed(commit_state: GrantFailureCommitState, error: String) -> Self {
        Self { success: false, commit_state: Some(commit_state), error: Some(error) }
    }
}

/// 失败侧提交态：与基线 `atomicWriteFailureCommitState` 的取值域一致（不含 `committed`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantFailureCommitState {
    NotCommitted,
    /// 已尝试原 rename：无法确定目标是否已被替换（保守上报）
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantSimpleResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ===== 错误文案分桶（对齐基线 `grantErrorText`，绝不回传绝对路径）=====

const AUTH_INVALID_TEXT: &str = "外部文件授权已失效，请重新选择。";
const PERMISSION_TEXT: &str = "当前窗口无权使用该外部文件授权。";
const PATH_INVALID_TEXT: &str = "外部文件授权路径无效，已拒绝操作。";
const FALLBACK_TEXT: &str = "外部文件授权操作失败。";

/// 文本读取上限（对齐基线 `MAX_TEXT_BYTES = 64 MiB`，超出对应 `SECURE_FS_FILE_TOO_LARGE`）
const MAX_TEXT_BYTES: u64 = 64 * 1024 * 1024;

fn grant_error_text(message: &str) -> String {
    if message.contains("无效")
        || message.contains("已失效")
        || message.contains("已过期")
        || message.contains("已用尽")
        || message.contains("已撤销")
        || message.contains("不存在")
    {
        return AUTH_INVALID_TEXT.to_string();
    }
    if message.contains("不允许") {
        return PERMISSION_TEXT.to_string();
    }
    if message.contains("相对路径")
        || message.contains("绝对路径")
        || message.contains("父目录遍历")
        || message.contains("超出授权范围")
        || message.contains("子路径")
        || message.contains("目标无效")
        || message.contains("目标已变化")
    {
        return PATH_INVALID_TEXT.to_string();
    }
    FALLBACK_TEXT.to_string()
}

/// IO 错误 → 文案（`NotFound` 归入「路径无效」桶，对齐基线 `SECURE_FS_NOT_FOUND`）
fn io_error_text(error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        PATH_INVALID_TEXT.to_string()
    } else {
        grant_error_text(&error.to_string())
    }
}

// ===== 原子写入（基线 `writeTextAtomically` 的等价实现）=====

struct AtomicWriteFailure {
    error: String,
    commit_state: GrantFailureCommitState,
}

impl AtomicWriteFailure {
    fn not_committed(error: String) -> Self {
        Self { error, commit_state: GrantFailureCommitState::NotCommitted }
    }
}

/// 同目录临时文件 → `ready` 阶段复检授权 → 原位 rename。
///
/// 提交点 = `rename`：此前任何失败均为 `not_committed`；rename 失败按 `unknown` 上报
/// （与基线 `atomicWriteFailureCommitState` 的保守语义一致）。
fn write_text_atomically(
    target: &Path,
    content: &str,
    must_already_exist: bool,
    mut ready_check: impl FnMut() -> Result<(), String>,
) -> Result<(), AtomicWriteFailure> {
    let parent = target
        .parent()
        .ok_or_else(|| AtomicWriteFailure::not_committed(PATH_INVALID_TEXT.to_string()))?;
    if !parent.is_dir() {
        return Err(AtomicWriteFailure::not_committed(PATH_INVALID_TEXT.to_string()));
    }
    if must_already_exist && !target.exists() {
        return Err(AtomicWriteFailure::not_committed(PATH_INVALID_TEXT.to_string()));
    }

    let file_name = target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| AtomicWriteFailure::not_committed(PATH_INVALID_TEXT.to_string()))?;
    let temporary = parent.join(format!(".{file_name}.{}.tmp", crate::project_access::random_uuid_v4()));

    let write_result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(content.as_bytes())?;
        file.flush()
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temporary);
        return Err(AtomicWriteFailure::not_committed(io_error_text(&error)));
    }

    // ready 阶段：临时文件已就绪，提交前复检授权（对齐基线 readyPhase 复验）
    if let Err(error) = ready_check() {
        let _ = std::fs::remove_file(&temporary);
        return Err(AtomicWriteFailure::not_committed(grant_error_text(&error)));
    }

    match std::fs::rename(&temporary, target) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            Err(AtomicWriteFailure {
                error: io_error_text(&error),
                commit_state: GrantFailureCommitState::Unknown,
            })
        }
    }
}

// ===== 逻辑层（不依赖 Tauri State，便于单测）=====

/// 按上限读取授权目标的文本（对齐基线 `readText` 的字节上限与严格 UTF-8 解码）
fn read_text_with_limit(path: &Path, limit: u64) -> Result<String, String> {
    let metadata = std::fs::metadata(path).map_err(|error| io_error_text(&error))?;
    if metadata.len() > limit {
        return Err(FALLBACK_TEXT.to_string());
    }
    std::fs::read_to_string(path).map_err(|error| io_error_text(&error))
}

/// `fs:grant-read-file` 逻辑：消费一次 `read` 授权并读取授权目标的文本内容。
pub fn read_granted_file(
    registry: &mut ExternalGrantRegistry,
    grant_id: &str,
    relative_path: Option<&str>,
) -> GrantReadResult {
    let target = match registry.resolve_target(grant_id, GrantOperation::Read, relative_path, true) {
        Ok(target) => target,
        Err(error) => {
            return GrantReadResult { success: false, content: String::new(), error: Some(grant_error_text(&error)) }
        }
    };
    match read_text_with_limit(&target.path, MAX_TEXT_BYTES) {
        Ok(content) => GrantReadResult { success: true, content, error: None },
        Err(error) => GrantReadResult { success: false, content: String::new(), error: Some(error) },
    }
}

/// `fs:grant-write-file` 逻辑（对齐基线三步：消费 `write` → 探测 `create` → 原子写入）。
pub fn write_granted_file(
    registry: &mut ExternalGrantRegistry,
    grant_id: &str,
    relative_path: &str,
    content: &str,
) -> GrantWriteResult {
    let target = match registry.resolve_target(
        grant_id,
        GrantOperation::Write,
        Some(relative_path),
        true,
    ) {
        Ok(target) => target,
        Err(error) => {
            return GrantWriteResult::failed(
                GrantFailureCommitState::NotCommitted,
                grant_error_text(&error),
            )
        }
    };

    let target_exists = target.path.exists();
    let create_probe = registry.revalidate(grant_id, GrantOperation::Create);
    let can_create = create_probe.is_ok();
    if !target_exists && !can_create {
        let error = create_probe.err().unwrap_or_else(|| PERMISSION_TEXT.to_string());
        return GrantWriteResult::failed(
            GrantFailureCommitState::NotCommitted,
            grant_error_text(&error),
        );
    }

    let outcome = write_text_atomically(&target.path, content, !can_create, || {
        registry.revalidate(grant_id, GrantOperation::Write)?;
        if can_create {
            registry.revalidate(grant_id, GrantOperation::Create)?;
        }
        Ok(())
    });
    match outcome {
        Ok(()) => GrantWriteResult::ok(),
        Err(failure) => GrantWriteResult::failed(failure.commit_state, failure.error),
    }
}

/// `fs:grant-mkdir` 逻辑：消费一次 `create` 授权并创建目录（含父级）。
pub fn mkdir_granted(
    registry: &mut ExternalGrantRegistry,
    grant_id: &str,
    relative_path: &str,
) -> GrantSimpleResult {
    let target = match registry.resolve_target(
        grant_id,
        GrantOperation::Create,
        Some(relative_path),
        true,
    ) {
        Ok(target) => target,
        Err(error) => {
            return GrantSimpleResult { success: false, error: Some(grant_error_text(&error)) }
        }
    };
    match std::fs::create_dir_all(&target.path) {
        Ok(()) => GrantSimpleResult { success: true, error: None },
        Err(error) => GrantSimpleResult { success: false, error: Some(io_error_text(&error)) },
    }
}

fn registry_lock(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, ExternalGrantRegistry>, String> {
    state
        .external_grants
        .lock()
        .map_err(|_| "外部文件授权状态被污染".to_string())
}

// ===== 命令 =====

/// `fs:grant-read-file`（能力域：不注入项目会话）
#[tauri::command]
pub fn fs_grant_read_file(
    state: State<'_, AppState>,
    grant_id: String,
    relative_path: Option<String>,
) -> GrantReadResult {
    match registry_lock(state.inner()) {
        Ok(mut registry) => read_granted_file(&mut registry, &grant_id, relative_path.as_deref()),
        Err(error) => GrantReadResult { success: false, content: String::new(), error: Some(error) },
    }
}

/// `fs:grant-write-file`
#[tauri::command]
pub fn fs_grant_write_file(
    state: State<'_, AppState>,
    grant_id: String,
    relative_path: String,
    content: String,
) -> GrantWriteResult {
    match registry_lock(state.inner()) {
        Ok(mut registry) => write_granted_file(&mut registry, &grant_id, &relative_path, &content),
        Err(error) => GrantWriteResult::failed(GrantFailureCommitState::NotCommitted, error),
    }
}

/// `fs:grant-mkdir`
#[tauri::command]
pub fn fs_grant_mkdir(
    state: State<'_, AppState>,
    grant_id: String,
    relative_path: String,
) -> GrantSimpleResult {
    match registry_lock(state.inner()) {
        Ok(mut registry) => mkdir_granted(&mut registry, &grant_id, &relative_path),
        Err(error) => GrantSimpleResult { success: false, error: Some(error) },
    }
}

/// `dialog:select-export-directory` —— 选择导出目录并签发 `write` + `create` 授权。
///
/// 返回 `grantId` + 展示名（绝回传绝对路径）。取消返回 `None`（与基线一致）；
/// 签发失败返回 `Err`（基线此处也是抛错，渲染层 `await` 会 reject）。
#[tauri::command]
pub async fn dialog_select_export_directory(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<ExternalDirectoryGrant>, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut builder = app.dialog().file().set_title(EXPORT_DIRECTORY_TITLE);
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        builder = builder.set_parent(&window);
    }
    builder.pick_folder(move |selection| {
        let _ = sender.send(selection);
    });
    let picked = tauri::async_runtime::spawn_blocking(move || {
        receiver.recv_timeout(DIALOG_TIMEOUT).ok().flatten()
    })
    .await
    .ok()
    .flatten();
    let Some(path) = picked else {
        return Ok(None);
    };
    let Some(directory) = file_path_to_string(path) else {
        return Ok(None);
    };

    let display_name = Path::new(&directory)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| directory.clone());
    let mut registry = registry_lock(state.inner())?;
    let grant_id = registry.issue_directory_with_operations(
        Path::new(&directory),
        vec![GrantOperation::Write, GrantOperation::Create],
        EXPORT_GRANT_TTL,
        Some(EXPORT_GRANT_MAX_USES),
    )?;
    Ok(Some(ExternalDirectoryGrant { grant_id, display_name }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_grant::KNOWLEDGE_BASE_GRANT_TTL;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lorekeeper-grant-cmd-{name}-{}",
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn export_registry(dir: &Path) -> (ExternalGrantRegistry, String) {
        let mut registry = ExternalGrantRegistry::default();
        let grant_id = registry
            .issue_directory_with_operations(
                dir,
                vec![GrantOperation::Write, GrantOperation::Create],
                EXPORT_GRANT_TTL,
                Some(EXPORT_GRANT_MAX_USES),
            )
            .unwrap();
        (registry, grant_id)
    }

    #[test]
    fn read_granted_file_returns_content_and_maps_errors_test() {
        let dir = temp_dir("read");
        let file = dir.join("资料.txt");
        std::fs::write(&file, "灯语与回声兽").unwrap();

        let mut registry = ExternalGrantRegistry::default();
        let grant_id = registry
            .issue_file(&file, KNOWLEDGE_BASE_GRANT_TTL, Some(1))
            .unwrap();

        let ok = read_granted_file(&mut registry, &grant_id, None);
        assert!(ok.success, "{:?}", ok.error);
        assert_eq!(ok.content, "灯语与回声兽");

        // 已消费 → 授权失效
        let exhausted = read_granted_file(&mut registry, &grant_id, None);
        assert!(!exhausted.success);
        assert_eq!(exhausted.error.as_deref(), Some("外部文件授权已失效，请重新选择。"));

        // 未知 grant → 同一文案（不泄露内部错误）
        let unknown = read_granted_file(&mut registry, "not-a-grant", None);
        assert_eq!(unknown.error.as_deref(), Some("外部文件授权已失效，请重新选择。"));

        // 文件授权不接受子路径 → 路径无效桶
        let grant_id = registry
            .issue_file(&file, KNOWLEDGE_BASE_GRANT_TTL, None)
            .unwrap();
        let sub = read_granted_file(&mut registry, &grant_id, Some("sub/a.txt"));
        assert_eq!(sub.error.as_deref(), Some("外部文件授权路径无效，已拒绝操作。"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_text_respects_byte_limit_and_missing_file_test() {
        let dir = temp_dir("read-limit");
        let file = dir.join("大文件.txt");
        std::fs::write(&file, "0123456789").unwrap();

        assert_eq!(read_text_with_limit(&file, 1024).unwrap(), "0123456789");
        assert_eq!(read_text_with_limit(&file, 4).unwrap_err(), FALLBACK_TEXT);
        assert_eq!(
            read_text_with_limit(&dir.join("不存在.txt"), 1024).unwrap_err(),
            PATH_INVALID_TEXT
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_granted_file_creates_and_replaces_atomically_test() {
        let dir = temp_dir("write");
        let (mut registry, grant_id) = export_registry(&dir);

        // 新建（依赖 create 权限）
        let created = write_granted_file(&mut registry, &grant_id, "合并.md", "# 正文");
        assert!(created.success, "{:?}", created.error);
        assert_eq!(std::fs::read_to_string(dir.join("合并.md")).unwrap(), "# 正文");
        assert!(created.commit_state.is_none(), "成功侧不序列化 commitState");

        // 覆盖既有文件（仅需 write 权限）
        let replaced = write_granted_file(&mut registry, &grant_id, "合并.md", "# 覆盖");
        assert!(replaced.success, "{:?}", replaced.error);
        assert_eq!(std::fs::read_to_string(dir.join("合并.md")).unwrap(), "# 覆盖");

        // 无临时文件残留
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "临时文件必须清理：{leftovers:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_requires_create_permission_for_new_files_test() {
        let dir = temp_dir("write-create");
        // 只给 write、不给 create 的目录授权
        let mut registry = ExternalGrantRegistry::default();
        let grant_id = registry
            .issue_directory_with_operations(
                &dir,
                vec![GrantOperation::Write],
                EXPORT_GRANT_TTL,
                Some(EXPORT_GRANT_MAX_USES),
            )
            .unwrap();

        let missing = write_granted_file(&mut registry, &grant_id, "新文件.md", "内容");
        assert!(!missing.success);
        assert_eq!(missing.commit_state, Some(GrantFailureCommitState::NotCommitted));
        assert_eq!(missing.error.as_deref(), Some("当前窗口无权使用该外部文件授权。"));
        assert!(!dir.join("新文件.md").exists(), "未授权创建时不得落盘");

        // 既有文件仍可写
        std::fs::write(dir.join("已存在.md"), "旧").unwrap();
        let existing = write_granted_file(&mut registry, &grant_id, "已存在.md", "新");
        assert!(existing.success, "{:?}", existing.error);
        assert_eq!(std::fs::read_to_string(dir.join("已存在.md")).unwrap(), "新");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_rejects_escape_and_expired_grant_test() {
        let dir = temp_dir("write-escape");
        let (mut registry, grant_id) = export_registry(&dir);

        let escape = write_granted_file(&mut registry, &grant_id, "../外面.md", "x");
        assert_eq!(escape.error.as_deref(), Some("外部文件授权路径无效，已拒绝操作。"));
        assert_eq!(escape.commit_state, Some(GrantFailureCommitState::NotCommitted));

        // 单次授权用尽后失效
        let mut registry = ExternalGrantRegistry::default();
        let single = registry
            .issue_directory_with_operations(
                &dir,
                vec![GrantOperation::Write, GrantOperation::Create],
                EXPORT_GRANT_TTL,
                Some(1),
            )
            .unwrap();
        assert!(write_granted_file(&mut registry, &single, "a.md", "1").success);
        let second = write_granted_file(&mut registry, &single, "b.md", "2");
        assert_eq!(second.error.as_deref(), Some("外部文件授权已失效，请重新选择。"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mkdir_granted_creates_nested_directories_test() {
        let dir = temp_dir("mkdir");
        let (mut registry, grant_id) = export_registry(&dir);

        let ok = mkdir_granted(&mut registry, &grant_id, "小说-11111111");
        assert!(ok.success, "{:?}", ok.error);
        assert!(dir.join("小说-11111111").is_dir());

        // 嵌套创建（导出分章目录下的子目录）
        let nested = mkdir_granted(&mut registry, &grant_id, "小说-22222222/子目录");
        assert!(nested.success, "{:?}", nested.error);
        assert!(dir.join("小说-22222222").join("子目录").is_dir());

        // 越界拒绝
        let escape = mkdir_granted(&mut registry, &grant_id, "../逃逸");
        assert_eq!(escape.error.as_deref(), Some("外部文件授权路径无效，已拒绝操作。"));

        // 只读目录授权（默认 list+read）没有 create 权限
        let mut registry = ExternalGrantRegistry::default();
        let read_only = registry
            .issue_directory(&dir, KNOWLEDGE_BASE_GRANT_TTL, Some(1))
            .unwrap();
        let denied = mkdir_granted(&mut registry, &read_only, "不允许");
        assert_eq!(denied.error.as_deref(), Some("当前窗口无权使用该外部文件授权。"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
