//! 批次 B：项目文件系统命令 —— 迁移自 `electron/controllers/fs-controller.ts`（基础 7 频道）。
//!
//! 行为对齐要点：
//! - 统一守卫：`project_session` 尾参必填（渲染层自动注入）→ 会话租约校验 →
//!   `expectedProjectPath` 匹配 → 词法包含 + realpath 边界（security 模块）；
//! - 文件互斥：骨架阶段全局串行锁（基线为 per-path mutex 队列；安全语义不变，
//!   仅并发度收窄，per-path 优化批次 C 后评估）；
//! - 原子写：同目录临时文件 + rename 提交，`commitState` 两态
//!   （基线第三态 `unknown` 保留结构，等 atomicWriteFailureCommitState 细化）；
//! - 错误不跨 IPC 泄露内部路径/原始 IO 错误（`fs_error_message` 统一映射）；
//! - `fs:list-dir` 失败走 invoke reject（不伪装空目录），其余命令结构化失败。

use serde::Serialize;
use tauri::State;

use crate::commands::SimpleResult;
use crate::security::{
    assert_current_project_context, assert_project_file_path, guard_message, GuardKind,
    PathCheckMode, ProjectSessionContext,
};
use crate::state::AppState;

/// 迁移自契约 `FileWriteCommitState`。
/// 骨架阶段不产生 `Unknown`（无 fsync 后段），批次 C 细化
/// atomicWriteFailureCommitState 后启用，暂抑制死代码警告。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileWriteCommitState {
    NotCommitted,
    Committed,
    Unknown,
}

/// 迁移自契约 `FileNode`。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<FileNode>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextReadResponse {
    pub success: bool,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteFileResponse {
    pub success: bool,
    pub commit_state: FileWriteCommitState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonReadResponse {
    pub success: bool,
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 内部错误 —— 守卫类错误映射为统一文案，IO 类错误按频道映射默认文案。
/// `Io` 携带的原始消息仅用于后续日志/诊断（当前不外泄到前端），
/// 骨架阶段无读取点，暂抑制死代码警告。
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum FsError {
    Guard(GuardKind),
    /// 目标不存在（对齐 ENOENT / SECURE_FS_NOT_FOUND）。
    MissingFile,
    Io(String),
}

impl From<GuardKind> for FsError {
    fn from(kind: GuardKind) -> Self {
        FsError::Guard(kind)
    }
}

type FsOutcome<T> = Result<T, FsError>;

/// 对齐 `projectFilesystemFailure`：不暴露文件系统路径或原始错误。
/// TODO(i18n)：文案随批次 E 与 i18n 收口统一接入 `mainText` 等价物。
fn fs_error_message(channel: &str, error: &FsError) -> String {
    if let FsError::Guard(kind) = error {
        return guard_message(*kind);
    }
    match channel {
        "fs:read-file" => {
            if matches!(error, FsError::MissingFile) {
                // 对齐基线 ENOENT 特例文案（引导使用 read_architecture 工具）。
                return "项目文件不存在。请检查文件路径；故事架构保存在项目数据中，请使用 read_architecture 工具读取。".to_string();
            }
            "无法读取项目文件。".to_string()
        }
        "fs:read-json" => "无法读取项目数据。".to_string(),
        "fs:write-file" | "fs:mkdir" | "fs:write-json" => "无法写入项目文件。".to_string(),
        _ => "项目文件操作失败。".to_string(),
    }
}

/// 统一守卫链 —— 对齐 `assertProjectFileOperation`：
/// 会话租约 → expectedProjectPath → 路径边界（词法 + realpath）。
fn assert_project_file_operation(
    state: &AppState,
    context: &ProjectSessionContext,
    target_path: &str,
    expected_project_path: &str,
    mode: PathCheckMode,
) -> FsOutcome<()> {
    let active_root: Option<String> = {
        let active = state.active_project.lock().expect("active_project 锁中毒");
        active.as_ref().map(|p| p.root_path.clone())
    };
    assert_current_project_context(context, active_root.as_deref())?;
    crate::security::assert_required_expected_project_path(active_root.as_deref(), Some(expected_project_path))
        .map_err(FsError::Guard)?;
    assert_project_file_path(target_path, active_root.as_deref().unwrap_or_default(), mode)
        .map_err(FsError::Guard)?;
    Ok(())
}

/// 原子写：先建父目录，再写同目录临时文件，rename 提交。
/// 返回提交前失败（NotCommitted）；rename 自身失败同样视为 NotCommitted
///（骨架无 fsync 后阶段，`Unknown` 暂不产生）。
fn write_text_atomically(target: &std::path::Path, content: &str) -> std::io::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "无父目录"))?;
    std::fs::create_dir_all(parent)?;
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let tmp = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let write_result = std::fs::write(&tmp, content);
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    match std::fs::rename(&tmp, target) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

/// 递归列目录：过滤 `.` 开头、目录优先、名称排序（TODO locale 对齐 zh-CN）。
fn read_dir_recursive(root_display: &std::path::Path, dir: &std::path::Path) -> FsOutcome<Vec<FileNode>> {
    let entries = std::fs::read_dir(dir).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            FsError::MissingFile
        } else {
            FsError::Io(err.to_string())
        }
    })?;
    let mut visible: Vec<(String, bool, std::path::PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| FsError::Io(err.to_string()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = entry.file_type().map_err(|err| FsError::Io(err.to_string()))?.is_dir();
        visible.push((name, is_dir, entry.path()));
    }
    visible.sort_by(|a, b| match (a.1, b.1) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.0.cmp(&b.0),
    });
    let mut nodes = Vec::with_capacity(visible.len());
    for (name, is_dir, path) in visible {
        let display_path = root_display.join(path.strip_prefix(dir).unwrap_or(&path));
        let node = if is_dir {
            FileNode {
                name,
                path: display_path.to_string_lossy().to_string(),
                is_dir: true,
                children: Some(read_dir_recursive(root_display, &path)?),
            }
        } else {
            FileNode {
                name,
                path: display_path.to_string_lossy().to_string(),
                is_dir: false,
                children: None,
            }
        };
        nodes.push(node);
    }
    Ok(nodes)
}

// ===== fs:read-file =====

#[tauri::command]
pub async fn fs_read_file(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    file_path: String,
    expected_project_path: String,
) -> Result<TextReadResponse, String> {
    let _file_lock = state.fs_lock.lock().expect("fs_lock 锁中毒");
    Ok(
        match read_file_inner(&state, &project_session, &file_path, &expected_project_path) {
            Ok(content) => TextReadResponse { success: true, content, error: None },
            Err(error) => TextReadResponse {
                success: false,
                content: String::new(),
                error: Some(fs_error_message("fs:read-file", &error)),
            },
        },
    )
}

fn read_file_inner(
    state: &State<'_, AppState>,
    context: &ProjectSessionContext,
    file_path: &str,
    expected_project_path: &str,
) -> FsOutcome<String> {
    assert_project_file_operation(state, context, file_path, expected_project_path, PathCheckMode::Existing)?;
    std::fs::read_to_string(file_path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            FsError::MissingFile
        } else {
            FsError::Io(err.to_string())
        }
    })
}

// ===== fs:write-file =====

#[tauri::command]
pub async fn fs_write_file(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    file_path: String,
    content: String,
    expected_project_path: String,
) -> Result<WriteFileResponse, String> {
    let _file_lock = state.fs_lock.lock().expect("fs_lock 锁中毒");
    Ok(
        match write_file_inner(&state, &project_session, &file_path, &content, &expected_project_path) {
            Ok(()) => WriteFileResponse {
                success: true,
                commit_state: FileWriteCommitState::Committed,
                error: None,
            },
            Err(error) => WriteFileResponse {
                success: false,
                commit_state: FileWriteCommitState::NotCommitted,
                error: Some(fs_error_message("fs:write-file", &error)),
            },
        },
    )
}

fn write_file_inner(
    state: &State<'_, AppState>,
    context: &ProjectSessionContext,
    file_path: &str,
    content: &str,
    expected_project_path: &str,
) -> FsOutcome<()> {
    assert_project_file_operation(state, context, file_path, expected_project_path, PathCheckMode::Writable)?;
    write_text_atomically(std::path::Path::new(file_path), content).map_err(|err| FsError::Io(err.to_string()))
}

// ===== fs:list-dir =====

#[tauri::command]
pub async fn fs_list_dir(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    dir_path: String,
    expected_project_path: String,
) -> Result<Vec<FileNode>, String> {
    let _file_lock = state.fs_lock.lock().expect("fs_lock 锁中毒");
    assert_project_file_operation(&state, &project_session, &dir_path, &expected_project_path, PathCheckMode::Existing)
        .map_err(|error| fs_error_message("fs:list-dir", &error))?;
    let root_display = crate::security::lexically_normalize(&dir_path);
    read_dir_recursive(&root_display, std::path::Path::new(&dir_path))
        .map_err(|error| fs_error_message("fs:list-dir", &error))
}

// ===== fs:mkdir =====

#[tauri::command]
pub async fn fs_mkdir(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    dir_path: String,
    expected_project_path: String,
) -> Result<SimpleResult, String> {
    let _file_lock = state.fs_lock.lock().expect("fs_lock 锁中毒");
    Ok(
        match mkdir_inner(&state, &project_session, &dir_path, &expected_project_path) {
            Ok(()) => SimpleResult { success: true, error: None },
            Err(_) => SimpleResult {
                success: false,
                error: Some("无法创建项目目录。".to_string()),
            },
        },
    )
}

fn mkdir_inner(
    state: &State<'_, AppState>,
    context: &ProjectSessionContext,
    dir_path: &str,
    expected_project_path: &str,
) -> FsOutcome<()> {
    assert_project_file_operation(state, context, dir_path, expected_project_path, PathCheckMode::Writable)?;
    std::fs::create_dir_all(dir_path).map_err(|err| FsError::Io(err.to_string()))
}

// ===== fs:check-exists =====

#[tauri::command]
pub async fn fs_check_exists(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    file_path: String,
    expected_project_path: String,
) -> Result<bool, String> {
    let _file_lock = state.fs_lock.lock().expect("fs_lock 锁中毒");
    assert_project_file_operation(&state, &project_session, &file_path, &expected_project_path, PathCheckMode::Writable)
        .map_err(|error| fs_error_message("fs:check-exists", &error))?;
    Ok(std::path::Path::new(&file_path).exists())
}

// ===== fs:read-json =====

#[tauri::command]
pub async fn fs_read_json(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    file_path: String,
    expected_project_path: String,
) -> Result<JsonReadResponse, String> {
    let _file_lock = state.fs_lock.lock().expect("fs_lock 锁中毒");
    let outcome = (|| -> FsOutcome<serde_json::Value> {
        assert_project_file_operation(&state, &project_session, &file_path, &expected_project_path, PathCheckMode::Existing)?;
        let content = std::fs::read_to_string(file_path).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                FsError::MissingFile
            } else {
                FsError::Io(err.to_string())
            }
        })?;
        serde_json::from_str(&content).map_err(|err| FsError::Io(err.to_string()))
    })();
    match outcome {
        Ok(data) => Ok(JsonReadResponse { success: true, data: Some(data), error: None }),
        Err(_) => Ok(JsonReadResponse {
            success: false,
            data: None,
            error: Some("无法读取项目数据。".to_string()),
        }),
    }
}

// ===== fs:write-json =====

#[tauri::command]
pub async fn fs_write_json(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    file_path: String,
    data: serde_json::Value,
    expected_project_path: String,
) -> Result<SimpleResult, String> {
    let _file_lock = state.fs_lock.lock().expect("fs_lock 锁中毒");
    let outcome = (|| -> FsOutcome<()> {
        assert_project_file_operation(&state, &project_session, &file_path, &expected_project_path, PathCheckMode::Writable)?;
        let pretty = serde_json::to_string_pretty(&data).map_err(|err| FsError::Io(err.to_string()))?;
        write_text_atomically(std::path::Path::new(&file_path), &pretty)
            .map_err(|err| FsError::Io(err.to_string()))
    })();
    match outcome {
        Ok(()) => Ok(SimpleResult { success: true, error: None }),
        Err(_) => Ok(SimpleResult { success: false, error: Some("无法写入项目数据。".to_string()) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project_root(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vela-fs-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn atomic_write_creates_parent_and_content_test() {
        let dir = temp_project_root("atomic");
        let target = dir.join("sub").join("chapter.md");
        write_text_atomically(&target, "正文").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "正文");
        // 覆盖写
        write_text_atomically(&target, "正文v2").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "正文v2");
        // 临时文件不留残留
        let leftovers: Vec<_> = std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "临时文件必须清理");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_dir_recursive_filters_hidden_and_sorts_dirs_first_test() {
        let dir = temp_project_root("list");
        std::fs::create_dir_all(dir.join("chapters")).unwrap();
        std::fs::write(dir.join("chapters").join("1.md"), "a").unwrap();
        std::fs::write(dir.join(".hidden"), "x").unwrap();
        std::fs::write(dir.join("readme.md"), "r").unwrap();
        let tree = read_dir_recursive(&dir, &dir).unwrap();
        assert_eq!(tree.len(), 2, "隐藏文件必须被过滤，目录优先排序");
        let chapters = &tree[0];
        assert!(chapters.is_dir, "目录优先于文件");
        assert_eq!(tree[1].name, "readme.md");
        let children = chapters.children.as_ref().unwrap();
        assert_eq!(children.len(), 1);
    }
}
