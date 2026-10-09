//! 更新偏好读写 —— 平移自 `electron/services/update-preferences-store.ts`（45 行）。
//!
//! 偏好持久化在**全局配置**（`~/.lorekeeper/config.json`）的 `updatePreferences` 键，
//! 因为 `lastCheckedAt` / `lastAutomaticCheckDate` / 可用更新与延后提醒都是轻量状态。
//!
//! 关键纪律（对齐基线，**不得放宽**）：
//! - `read()` 宽容：文件缺失或损坏时返回 `{}`（不抛错）；
//! - `write()` 严格：文件存在但**不可安全解析**时返回 `false`，**绝不**用默认值覆盖用户
//!   已有的模型 / 语言 / 代理等设置；
//! - 写入采用「读原始 JSON → 只替换 `updatePreferences` 键 → 原子写回」，保留其余键与未知键。

use std::path::PathBuf;

use serde_json::Value;

use crate::app_paths;
use crate::json_store::{try_read_json_value, write_json_file, JsonFileReadResult};
use crate::update::types::UpdatePreferences;

/// 偏好存储抽象（便于单测注入内存实现）
pub trait UpdatePreferencesStore: Send + Sync {
    fn read(&self) -> UpdatePreferences;
    /// 返回 `false` 表示偏好未能被安全持久化
    fn write(&self, preferences: &UpdatePreferences) -> bool;
}

/// 全局配置里的偏好存储（`config.json` 的 `updatePreferences` 键）
#[derive(Debug, Clone)]
pub struct ConfigUpdatePreferencesStore {
    config_path: PathBuf,
}

impl ConfigUpdatePreferencesStore {
    /// 生产用：`~/.lorekeeper/config.json`
    pub fn new() -> Self {
        Self {
            config_path: app_paths::global_config_path(),
        }
    }

    /// 测试/注入用：指定配置文件路径
    pub fn at(config_path: PathBuf) -> Self {
        Self { config_path }
    }

    fn read_raw(&self) -> Option<Value> {
        match try_read_json_value(&self.config_path) {
            JsonFileReadResult::Ok(value) if value.is_object() => Some(value),
            _ => None,
        }
    }
}

impl Default for ConfigUpdatePreferencesStore {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdatePreferencesStore for ConfigUpdatePreferencesStore {
    fn read(&self) -> UpdatePreferences {
        let Some(value) = self.read_raw() else {
            return UpdatePreferences::default();
        };
        match value.get("updatePreferences") {
            Some(preferences) => serde_json::from_value(preferences.clone()).unwrap_or_default(),
            None => UpdatePreferences::default(),
        }
    }

    fn write(&self, preferences: &UpdatePreferences) -> bool {
        let Ok(serialized) = serde_json::to_value(preferences) else {
            return false;
        };
        match try_read_json_value(&self.config_path) {
            JsonFileReadResult::Missing => {
                let mut merged = crate::commands::default_global_config();
                if let Some(object) = merged.as_object_mut() {
                    object.insert("updatePreferences".to_string(), serialized);
                }
                write_json_file(&self.config_path, &merged).is_ok()
            }
            JsonFileReadResult::Ok(mut value) if value.is_object() => {
                if let Some(object) = value.as_object_mut() {
                    object.insert("updatePreferences".to_string(), serialized);
                }
                write_json_file(&self.config_path, &value).is_ok()
            }
            // 配置损坏（或形状不符）：**不覆盖**，交由调用方返回 REMINDER_SAVE_FAILED
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::types::UpdateReleaseInfo;

    fn temp_config(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "lorekeeper-update-prefs-{name}-{}",
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root.join("config.json")
    }

    #[test]
    fn read_is_lenient_and_write_is_strict_test() {
        let path = temp_config("strict");
        let store = ConfigUpdatePreferencesStore::at(path.clone());

        // 缺失 → read {} 且 write 成功（新建默认配置 + 偏好）
        assert_eq!(store.read(), UpdatePreferences::default());
        assert!(store.write(&UpdatePreferences {
            last_checked_at: Some("2026-10-09T12:00:00.000Z".to_string()),
            ..UpdatePreferences::default()
        }));
        let created = std::fs::read_to_string(&path).unwrap();
        assert!(created.contains("updatePreferences"));
        assert!(created.contains("2026-10-09T12:00:00.000Z"));
        assert_eq!(
            store.read().last_checked_at.as_deref(),
            Some("2026-10-09T12:00:00.000Z")
        );

        // 损坏 → read {} 且 write false，**文件不得被覆盖**
        let corrupt = temp_config("corrupt");
        std::fs::write(&corrupt, "{ 这不是 JSON").unwrap();
        let corrupt_store = ConfigUpdatePreferencesStore::at(corrupt.clone());
        assert_eq!(corrupt_store.read(), UpdatePreferences::default());
        assert!(!corrupt_store.write(&UpdatePreferences::default()));
        assert_eq!(std::fs::read_to_string(&corrupt).unwrap(), "{ 这不是 JSON");

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let _ = std::fs::remove_dir_all(corrupt.parent().unwrap());
    }

    #[test]
    fn write_preserves_other_config_keys_test() {
        let path = temp_config("preserve");
        std::fs::write(
            &path,
            r#"{ "theme": "dark", "proxy": { "enabled": true }, "defaultModelId": "m-1" }"#,
        )
        .unwrap();
        let store = ConfigUpdatePreferencesStore::at(path.clone());

        assert!(store.write(&UpdatePreferences {
            available_update: Some(UpdateReleaseInfo {
                version: "1.2.0".to_string(),
                release_name: Some("R".to_string()),
                release_notes: None,
                release_date: None,
            }),
            ..UpdatePreferences::default()
        }));

        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["theme"], "dark", "其他配置键必须保留");
        assert_eq!(written["proxy"]["enabled"], true);
        assert_eq!(written["defaultModelId"], "m-1");
        assert_eq!(
            written["updatePreferences"]["availableUpdate"]["version"],
            "1.2.0"
        );
        assert_eq!(store.read().available_update.unwrap().version, "1.2.0");

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn unknown_preference_keys_do_not_break_read_test() {
        let path = temp_config("unknown");
        std::fs::write(
            &path,
            r#"{ "updatePreferences": { "lastCheckedAt": "2026-10-09T00:00:00.000Z", "futureKey": 7 } }"#,
        )
        .unwrap();
        let store = ConfigUpdatePreferencesStore::at(path.clone());
        assert_eq!(
            store.read().last_checked_at.as_deref(),
            Some("2026-10-09T00:00:00.000Z")
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
