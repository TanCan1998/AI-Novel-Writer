//! Skin 频道的 Tauri 命令实现
//!
//! 对应 `src/shared/ipc-channels.ts` 的 `SkinChannels`：
//! - `skin:get-state` → `skin_get_state`
//! - `skin:execute` → `skin_execute`
//! - `skin:read-custom-asset` → `skin_read_custom_asset`

use serde::{Deserialize, Serialize};
use tauri::State;

/// 皮肤状态
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinState {
    pub active_skin: String,
    pub custom_skin: Option<CustomSkin>,
}

/// 自定义皮肤信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomSkin {
    pub mime: String,
    pub revision: String,
    pub width: u32,
    pub height: u32,
}

/// 皮肤执行命令（tag 对齐契约字面量：activate / import-custom / remove-custom）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SkinCommand {
    #[serde(rename_all = "camelCase")]
    Activate {
        skin_id: String,
    },
    ImportCustom,
    RemoveCustom,
}

/// 皮肤执行响应
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SkinExecuteResponse {
    Success {
        success: bool,
        state: SkinState,
        cancelled: bool,
    },
    Failure {
        success: bool,
        state: SkinState,
        error: SkinError,
    },
}

/// 皮肤错误
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkinError {
    pub code: String,
    pub message: String,
}

/// 皮肤命令执行器（内存存储）
#[derive(Debug, Clone)]
pub struct SkinCommandStore {
    state: SkinState,
}

impl SkinCommandStore {
    pub(crate) fn new() -> Self {
        SkinCommandStore {
            state: SkinState {
                active_skin: "classic".to_string(),
                custom_skin: None,
            },
        }
    }

    fn execute(&mut self, command: SkinCommand) -> SkinExecuteResponse {
        match command {
            SkinCommand::Activate { skin_id } => {
                self.state.active_skin = skin_id.clone();
                SkinExecuteResponse::Success {
                    success: true,
                    state: self.state.clone(),
                    cancelled: false,
                }
            }
            SkinCommand::ImportCustom => {
                // TODO: 实际导入逻辑（读取文件）
                SkinExecuteResponse::Success {
                    success: true,
                    state: self.state.clone(),
                    cancelled: false,
                }
            }
            SkinCommand::RemoveCustom => {
                self.state.custom_skin = None;
                SkinExecuteResponse::Success {
                    success: true,
                    state: self.state.clone(),
                    cancelled: false,
                }
            }
        }
    }

    fn read_custom_asset(&self) -> Result<SkinAsset, SkinError> {
        match &self.state.custom_skin {
            Some(custom) => Ok(SkinAsset {
                mime: custom.mime.clone(),
                revision: custom.revision.clone(),
                bytes: vec![],
            }),
            None => Err(SkinError {
                code: "CUSTOM_ASSET_UNAVAILABLE".to_string(),
                message: "未找到自定义皮肤资产".to_string(),
            }),
        }
    }
}

/// 皮肤资产（用于读取自定义皮肤）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkinAsset {
    pub mime: String,
    pub revision: String,
    pub bytes: Vec<u8>,
}

/// Skin:read-custom-asset 的响应（对齐契约 SkinReadCustomAssetResponse 判别联合；
/// 失败作为成功响应返回，而不是 invoke reject）
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum SkinReadCustomAssetResponse {
    /// 成功：展开 SkinAsset 字段并附 success: true
    Success {
        success: bool,
        mime: String,
        revision: String,
        bytes: Vec<u8>,
    },
    /// 失败：附当前状态与错误
    Failure {
        success: bool,
        state: SkinState,
        error: SkinError,
    },
}

/// Skin:get-state 命令
#[tauri::command]
pub fn skin_get_state(state: State<'_, crate::state::AppState>) -> SkinState {
    state.skin.lock().unwrap().state.clone()
}

/// Skin:execute 命令
#[tauri::command]
pub fn skin_execute(
    state: State<'_, crate::state::AppState>,
    command: SkinCommand,
) -> SkinExecuteResponse {
    state.skin.lock().unwrap().execute(command)
}

/// Skin:read-custom-asset 命令（失败走契约判别联合而非 invoke reject）
#[tauri::command]
pub fn skin_read_custom_asset(
    state: State<'_, crate::state::AppState>,
) -> SkinReadCustomAssetResponse {
    let guard = state.skin.lock().unwrap();
    match guard.read_custom_asset() {
        Ok(asset) => SkinReadCustomAssetResponse::Success {
            success: true,
            mime: asset.mime,
            revision: asset.revision,
            bytes: asset.bytes,
        },
        Err(error) => SkinReadCustomAssetResponse::Failure {
            success: false,
            state: guard.state.clone(),
            error,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_state_initializes_correctly() {
        let store = SkinCommandStore::new();
        assert_eq!(store.state.active_skin, "classic");
        assert!(store.state.custom_skin.is_none());
    }

    #[test]
    fn skin_command_deserializes_kebab_tags_from_frontend() {
        // 前端传 `{ type: 'import-custom' }`（kebab-case tag），Rust 侧必须可反序列化
        let cmd: SkinCommand = serde_json::from_str(r#"{"type":"import-custom"}"#).unwrap();
        assert!(matches!(cmd, SkinCommand::ImportCustom));
        let cmd: SkinCommand =
            serde_json::from_str(r#"{"type":"activate","skinId":"anime"}"#).unwrap();
        assert!(matches!(cmd, SkinCommand::Activate { .. }));
        let cmd: SkinCommand = serde_json::from_str(r#"{"type":"remove-custom"}"#).unwrap();
        assert!(matches!(cmd, SkinCommand::RemoveCustom));
    }

    #[test]
    fn resolve_close_decision_accepts_lowercase_literals() {
        let d: crate::commands::window::ResolveCloseDecision =
            serde_json::from_str(r#""proceed""#).unwrap();
        assert!(matches!(
            d,
            crate::commands::window::ResolveCloseDecision::Proceed
        ));
    }

    #[test]
    fn skin_execute_activate_succeeds() {
        let mut store = SkinCommandStore::new();
        let command = SkinCommand::Activate {
            skin_id: "anime".to_string(),
        };
        let response = store.execute(command);
        match response {
            SkinExecuteResponse::Success {
                success: true,
                state,
                ..
            } => {
                assert_eq!(state.active_skin, "anime");
            }
            _ => panic!("Expected Success response"),
        }
    }

    #[test]
    fn skin_execute_remove_custom_succeeds() {
        let mut store = SkinCommandStore::new();
        store.state.custom_skin = Some(CustomSkin {
            mime: "image/png".to_string(),
            revision: "1.0".to_string(),
            width: 800,
            height: 600,
        });
        let command = SkinCommand::RemoveCustom;
        let response = store.execute(command);
        match response {
            SkinExecuteResponse::Success {
                success: true,
                state,
                ..
            } => {
                assert!(state.custom_skin.is_none());
            }
            _ => panic!("Expected Success response"),
        }
    }
}
