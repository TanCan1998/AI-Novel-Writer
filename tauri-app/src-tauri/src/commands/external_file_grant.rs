//! 批次 B：外部文件授权域 —— 迁移自 `electron/controllers/external-file-grant-controller.ts`。
//!
//! ADR 0002：用户经系统目录选择对话框显式授权后签发 grant；渲染进程只能携带
//! grantId 与受限相对路径，绝不暴露绝对路径。Tauri 映射为 tauri-plugin-dialog
//!（选择入口）+ 自研 grant 校验。
//!
//! 状态（2026-10-08）：`tauri-plugin-dialog` 已于批次 B 遗留补齐中获批接入，
//! 原生目录选择能力**已就绪**。但 `dialog:select-export-directory` 的返回值不是路径，
//! 而是 `ExternalDirectoryGrant`（grantId + 展示名，绝对路径不得越界回传，ADR 0002），
//! 其签发需要 grant 注册表 —— 该注册表与 `fs:grant-*` 三命令同在批次 H。
//!
//! 因此本命令**继续返回 `None`（取消语义）**：若此时就地签发一个假 grantId，
//! 界面会显示「已选择导出目录」而后续写入必然失败，反而不如当前语义诚实。
//! 待批次 H 落地 grant 域后，只需把下面的选择结果喂给 grant 签发，弹窗侧无需再改。

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

/// `dialog:select-export-directory` —— 骨架：返回 None（取消语义）。
///
/// 原生目录选择已可用（见模块头状态说明），阻塞点是**批次 H 的 grant 签发**：
/// 本命令必须回传 `grantId`（而非绝对路径）才能让渲染层后续经 `fs:grant-write-file`
/// 写入导出目录，而 grant 注册表尚未迁移。当前返回 `None` 与基线的「用户取消」同义，
/// 且 `ExportDialog.tsx` 对 `null` 的处理就是静默返回，无契约偏差。
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
