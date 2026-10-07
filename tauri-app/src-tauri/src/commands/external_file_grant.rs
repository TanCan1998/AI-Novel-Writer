//! 批次 B：外部文件授权域 —— 迁移自 `electron/controllers/external-file-grant-controller.ts`。
//!
//! ADR 0002：用户经系统目录选择对话框显式授权后签发 grant；渲染进程只能携带
//! grantId 与受限相对路径，绝不暴露绝对路径。Tauri 映射为 tauri-plugin-dialog
//!（选择入口）+ 自研 grant 校验。
//!
//! 本批次为骨架占位：`dialog:select-export-directory` 与三个 `fs:grant-*` 命令
//! 依赖 tauri-plugin-dialog（新增插件依赖，AGENTS.md Ask first），接入前返回
//! 契约形状的失败/取消结果；grant 校验框架批次后续填充。

use serde::Serialize;

/// 对齐契约 `ExternalFileGrant`：只携带展示名与不透明能力标识，不暴露绝对路径。
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

/// 契约判别联合（序列化形态）：成功 `{success:true}`；
/// 失败 `{success:false, commitState, error?}`（失败侧排除 'committed'）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantWriteResult {
    pub success: bool,
    pub commit_state: GrantFailureCommitState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 失败侧提交态：骨架阶段不产生 `Unknown`（等 tauri-plugin-dialog 接入后的
/// 真实写路径启用），暂抑制死代码警告。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantFailureCommitState {
    NotCommitted,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantSimpleResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

const GRANT_SKELETON_MESSAGE: &str =
    "外部文件授权功能将在接入 tauri-plugin-dialog（目录选择插件）后启用。";

// ===== fs:grant-read-file =====

#[tauri::command]
pub fn fs_grant_read_file(_grant_id: String, _relative_path: Option<String>) -> GrantReadResult {
    GrantReadResult {
        success: false,
        content: String::new(),
        error: Some(GRANT_SKELETON_MESSAGE.to_string()),
    }
}

// ===== fs:grant-write-file =====

#[tauri::command]
pub fn fs_grant_write_file(
    _grant_id: String,
    _relative_path: String,
    _content: String,
) -> GrantWriteResult {
    GrantWriteResult {
        success: false,
        commit_state: GrantFailureCommitState::NotCommitted,
        error: Some(GRANT_SKELETON_MESSAGE.to_string()),
    }
}

// ===== fs:grant-mkdir =====

#[tauri::command]
pub fn fs_grant_mkdir(_grant_id: String, _relative_path: String) -> GrantSimpleResult {
    GrantSimpleResult { success: false, error: Some(GRANT_SKELETON_MESSAGE.to_string()) }
}

/// `dialog:select-export-directory` 骨架：返回 None（取消语义），等
/// tauri-plugin-dialog 接入后返回真实 ExternalDirectoryGrant。
#[tauri::command]
pub fn dialog_select_export_directory() -> Option<ExternalDirectoryGrant> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_commands_return_structured_failure_before_dialog_plugin() {
        let read = fs_grant_read_file("g-1".into(), Some("a.txt".to_string()));
        assert!(!read.success);
        assert_eq!(read.content, "");
        assert!(read.error.is_some());

        let write = fs_grant_write_file("g-1".into(), "out.txt".into(), "x".into());
        assert!(!write.success);
        assert_eq!(write.commit_state, GrantFailureCommitState::NotCommitted);

        let mkdir = fs_grant_mkdir("g-1".into(), "sub".into());
        assert!(!mkdir.success);
        assert!(dialog_select_export_directory().is_none(), "插件未接入前对齐取消语义");
    }
}
