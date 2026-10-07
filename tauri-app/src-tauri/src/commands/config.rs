//! Config 频道的 Tauri 命令实现
//!
//! 对应 `src/shared/ipc-channels.ts` 的 `ConfigChannels` 与
//! `electron/controllers/config-controller.ts` 行为：
//! - `config:get` → `config_get`：返回完整全局配置（缺省时合并默认值）
//! - `config:set` → `config_set`：浅合并 `Partial<GlobalConfig>` 到现有配置

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::State;

/// 全局配置内存存储（骨架阶段；后续批次持久化到 app_data_dir 下 JSON 文件，
/// 对齐 electron/utils/config-utils.ts 的 `GLOBAL_CONFIG_PATH`）。
#[derive(Debug, Clone)]
pub struct ConfigStore {
    /// 扁平键值结构，字段名与 `GlobalConfig`（camelCase）一致。
    config: HashMap<String, serde_json::Value>,
}

impl ConfigStore {
    /// 默认值对齐 `electron/utils/config-utils.ts` 的 `DEFAULT_GLOBAL_CONFIG`。
    pub(crate) fn new() -> Self {
        let default_config: HashMap<String, serde_json::Value> = [
            ("theme".into(), serde_json::json!("dark")),
            ("defaultModelId".into(), serde_json::json!(null)),
            (
                "autoOpenNextChapterAfterFinalize".into(),
                serde_json::json!(false),
            ),
            ("editorFontSize".into(), serde_json::json!(16)),
            ("editorFontFamily".into(), serde_json::json!("Noto Serif SC")),
            ("autoSaveInterval".into(), serde_json::json!(30)),
            (
                "proxy".into(),
                serde_json::json!({
                    "enabled": false,
                    "type": "http",
                    "host": "",
                    "port": 7890
                }),
            ),
        ]
        .into_iter()
        .collect();

        ConfigStore { config: default_config }
    }

    fn get(&self) -> serde_json::Value {
        serde_json::to_value(&self.config).unwrap_or(serde_json::json!({}))
    }

    /// 浅合并写入（对齐 electron 侧 `{ ...existing, ...config }`）。
    fn set(&mut self, key: String, value: serde_json::Value) {
        self.config.insert(key, value);
    }
}

/// 纯函数核心：将 `Partial<GlobalConfig>` 浅合并进存储，便于单元测试。
fn apply_config_updates(
    store: &mut ConfigStore,
    config: Option<HashMap<String, serde_json::Value>>,
) -> ConfigSetResponse {
    if let Some(config) = config {
        for (key, value) in config {
            store.set(key, value);
        }
    }
    ConfigSetResponse { success: true, error: None }
}

/// Config:get 命令
#[tauri::command]
pub fn config_get(state: State<'_, crate::state::AppState>) -> serde_json::Value {
    state.config.lock().unwrap().get()
}

/// Config:set 命令（契约：args `[config: Partial<GlobalConfig>]`）
#[tauri::command]
pub fn config_set(
    state: State<'_, crate::state::AppState>,
    config: Option<HashMap<String, serde_json::Value>>,
) -> Result<ConfigSetResponse, String> {
    let mut store = state.config.lock().unwrap();
    Ok(apply_config_updates(&mut store, config))
}

/// Config:set 的响应
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSetResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_store_initializes_with_defaults() {
        let store = ConfigStore::new();
        assert!(store.config.contains_key("theme"));
        assert!(store.config.contains_key("editorFontSize"));
        assert_eq!(store.config.get("theme"), Some(&serde_json::json!("dark")));
        assert_eq!(store.config.get("editorFontSize"), Some(&serde_json::json!(16)));
    }

    #[test]
    fn config_set_returns_success() {
        let mut store = ConfigStore::new();
        let mut updates = HashMap::new();
        updates.insert("editorFontSize".to_string(), serde_json::json!(20));
        let result = apply_config_updates(&mut store, Some(updates));
        assert!(result.success);
        assert!(result.error.is_none());
        assert_eq!(store.config.get("editorFontSize"), Some(&serde_json::json!(20)));
    }

    #[test]
    fn config_set_without_updates_keeps_defaults() {
        let mut store = ConfigStore::new();
        let result = apply_config_updates(&mut store, None);
        assert!(result.success);
        assert!(store.config.contains_key("theme"));
    }
}
