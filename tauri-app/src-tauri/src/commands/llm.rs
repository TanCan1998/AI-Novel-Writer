//! 批次 D1：LLM 模型管理（7 频道）—— 迁移自 `electron/controllers/llm-controller.ts`
//! 的配置读写部分（生成 / 流式 / 租约 / 发现 / 连通性属批次 D2）。
//!
//! | 频道 | 命令 | 基线行为 |
//! |---|---|---|
//! | `llm:list-models` | `llm_list_models` | `readJsonFile(models.json, [])` |
//! | `llm:save-model` | `llm_save_model` | 按 `id` 就地更新或追加；配置损坏则拒绝覆盖 |
//! | `llm:delete-model` | `llm_delete_model` | **先清引用再删对象**，第二步失败时回滚配置 |
//! | `llm:set-default-model` | `llm_set_default_model` | 写 `config.json` 的 `defaultModelId` |
//! | `llm:get-default-model` | `llm_get_default_model` | 读 `defaultModelId` |
//! | `llm:set-default-embedding-model` | `llm_set_default_embedding_model` | 写 `defaultEmbeddingModelId` |
//! | `llm:get-default-embedding-model` | `llm_get_default_embedding_model` | 读 `defaultEmbeddingModelId` |
//!
//! 模型条目保持 `serde_json::Value` **原样透传**：`ModelProfile` 的字段集由渲染层
//! 契约定义（`capabilities` / `reasoningOverride` / `embeddingOptions` 等可选字段），
//! Rust 侧只解释 `id`，避免漏字段导致配置被静默裁剪。

use serde::Serialize;
use std::path::Path;

use crate::app_paths;
use crate::commands::config::{self, ConfigSetResponse};

/// 模型配置损坏时的拒绝覆盖文案（对齐基线 `loadModelConfigsForUpdate()`）。
pub const MODELS_CONFIG_CORRUPTED_MESSAGE: &str = "模型配置损坏，已拒绝覆盖";

/// 非对象模型条目的拒绝文案（基线会抛 `TypeError`，这里给出可读文案）。
pub const MODEL_MUST_BE_OBJECT_MESSAGE: &str = "模型配置必须是对象";

/// 对齐基线失败形态 `{ success: false, error: String(error) }`（统一 `Error: ` 前缀）。
fn failure(message: String) -> LlmSaveModelResult {
    LlmSaveModelResult {
        success: false,
        error: Some(format!("Error: {message}")),
    }
}

/// `llm:save-model` 响应（契约 `{ success: boolean }`；失败附 `error`）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmSaveModelResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `llm:delete-model` 响应。
///
/// 字段存在性对齐基线：**成功**时同时给出 `defaultModelId` 与
/// `defaultEmbeddingModelId`（可为 `null` = 已清空）；**失败**时两者整体缺省。
/// 故用 `Option<Value>` 区分「缺省」与「显式 null」。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmDeleteModelResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model_id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_embedding_model_id: Option<serde_json::Value>,
}

/// 组合「删除失败且回滚也失败」的文案。
///
/// 基线为 `throw new Error(\`${String(error)}；恢复默认模型配置失败：${String(rollbackError)}\`)`，
/// 外层 `catch` 再 `String(error)` 一次 —— 因此最终文案里会出现**两处** `Error: `
/// 前缀（`String(new Error(...))` 的链式结果）。此处刻意逐字复刻。
fn rollback_failure_message(error: &str, rollback_error: &str) -> String {
    format!("Error: {error}；恢复默认模型配置失败：Error: {rollback_error}")
}

/// 读模型列表：缺失 / 损坏 / 形状不符一律返回空列表（对齐 `readJsonFile(path, [])`）。
pub fn read_models_at(path: &Path) -> Vec<serde_json::Value> {
    crate::json_store::read_json_file(path, Vec::new())
}

/// 读模型列表用于**增量更新**：损坏时拒绝覆盖。
pub fn models_for_update_at(path: &Path) -> Result<Vec<serde_json::Value>, String> {
    use crate::json_store::JsonFileReadResult;
    match crate::json_store::try_read_json_value(path) {
        JsonFileReadResult::Missing => Ok(Vec::new()),
        JsonFileReadResult::Ok(serde_json::Value::Array(items)) => Ok(items),
        JsonFileReadResult::Ok(_) => Err(MODELS_CONFIG_CORRUPTED_MESSAGE.to_string()),
        JsonFileReadResult::Error(_) => Err(MODELS_CONFIG_CORRUPTED_MESSAGE.to_string()),
    }
}

/// 条目身份判定（对齐 `m.id === model.id` 的严格相等语义）：
/// 两侧都缺 `id` 视为同一身份（JS `undefined === undefined`）。
fn same_model_identity(existing: &serde_json::Value, incoming: &serde_json::Value) -> bool {
    match (existing.get("id"), incoming.get("id")) {
        (None, None) => true,
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// 条目是否属于指定模型 id（对齐 `model.id !== modelId`）。
fn has_model_id(entry: &serde_json::Value, model_id: &str) -> bool {
    entry.get("id").and_then(|value| value.as_str()) == Some(model_id)
}

/// `llm:save-model` 核心：按 `id` 就地替换，否则追加。
pub fn save_model_at(models_path: &Path, model: &serde_json::Value) -> LlmSaveModelResult {
    if !model.is_object() {
        return failure(MODEL_MUST_BE_OBJECT_MESSAGE.to_string());
    }
    let mut models = match models_for_update_at(models_path) {
        Ok(models) => models,
        Err(message) => return failure(message),
    };
    match models
        .iter()
        .position(|existing| same_model_identity(existing, model))
    {
        Some(index) => models[index] = model.clone(),
        None => models.push(model.clone()),
    }
    match crate::json_store::write_json_file(models_path, &serde_json::Value::Array(models)) {
        Ok(()) => LlmSaveModelResult {
            success: true,
            error: None,
        },
        Err(error) => failure(error),
    }
}

/// `llm:delete-model` 核心：先清除默认引用，再删除模型；第二步失败时回滚配置。
///
/// 回滚目标是「默认模型不再悬空指向已删除模型」，即使回滚也失败，最坏情况只是
/// 缺省值被清空，绝不会悬空引用。
pub fn delete_model_at(
    models_path: &Path,
    config_path: &Path,
    model_id: &str,
) -> LlmDeleteModelResult {
    let fail = |message: String| LlmDeleteModelResult {
        success: false,
        error: Some(format!("Error: {message}")),
        default_model_id: None,
        default_embedding_model_id: None,
    };

    let original_models = match models_for_update_at(models_path) {
        Ok(models) => models,
        Err(message) => return fail(message),
    };
    let original_config = match config::global_config_for_update_at(config_path) {
        Ok(value) => value,
        Err(message) => return fail(message),
    };

    let mut next_config = original_config.clone();
    let mut config_changed = false;
    if next_config
        .get("defaultModelId")
        .and_then(|value| value.as_str())
        == Some(model_id)
    {
        next_config["defaultModelId"] = serde_json::Value::Null;
        config_changed = true;
    }
    if next_config
        .get("defaultEmbeddingModelId")
        .and_then(|value| value.as_str())
        == Some(model_id)
    {
        next_config["defaultEmbeddingModelId"] = serde_json::Value::Null;
        config_changed = true;
    }

    if config_changed {
        // 先清除引用（此步失败不回滚：配置尚未被改写为不一致状态）
        if let Err(error) = crate::json_store::write_json_file(config_path, &next_config) {
            return fail(error);
        }
    }

    let remaining: Vec<serde_json::Value> = original_models
        .into_iter()
        .filter(|entry| !has_model_id(entry, model_id))
        .collect();
    if let Err(error) =
        crate::json_store::write_json_file(models_path, &serde_json::Value::Array(remaining))
    {
        if config_changed {
            if let Err(rollback_error) =
                crate::json_store::write_json_file(config_path, &original_config)
            {
                return fail(rollback_failure_message(&error, &rollback_error));
            }
        }
        return fail(error);
    }

    LlmDeleteModelResult {
        success: true,
        error: None,
        // 对齐基线：成功时给出 `defaultModelId`（缺键则整体缺省）与
        // `defaultEmbeddingModelId ?? null`（恒存在）
        default_model_id: next_config.get("defaultModelId").cloned(),
        default_embedding_model_id: Some(
            next_config
                .get("defaultEmbeddingModelId")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        ),
    }
}

/// `llm:get-default-model` 核心。
pub fn get_default_model_at(config_path: &Path) -> Option<String> {
    config::read_global_config_at(config_path)
        .get("defaultModelId")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

/// `llm:get-default-embedding-model` 核心。
pub fn get_default_embedding_model_at(config_path: &Path) -> Option<String> {
    config::read_global_config_at(config_path)
        .get("defaultEmbeddingModelId")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

// ===== 命令面（路径全部取自 `app_paths`，逻辑在 `*_at` 中） =====

#[tauri::command]
pub fn llm_list_models() -> Vec<serde_json::Value> {
    read_models_at(&app_paths::models_config_path())
}

#[tauri::command]
pub fn llm_save_model(model: serde_json::Value) -> LlmSaveModelResult {
    save_model_at(&app_paths::models_config_path(), &model)
}

#[tauri::command]
pub fn llm_delete_model(model_id: String) -> LlmDeleteModelResult {
    delete_model_at(
        &app_paths::models_config_path(),
        &app_paths::global_config_path(),
        &model_id,
    )
}

#[tauri::command]
pub fn llm_get_default_model() -> Option<String> {
    get_default_model_at(&app_paths::global_config_path())
}

#[tauri::command]
pub fn llm_set_default_model(model_id: Option<String>) -> ConfigSetResponse {
    config::set_config_key_at(
        &app_paths::global_config_path(),
        "defaultModelId",
        model_id
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    )
}

#[tauri::command]
pub fn llm_get_default_embedding_model() -> Option<String> {
    get_default_embedding_model_at(&app_paths::global_config_path())
}

#[tauri::command]
pub fn llm_set_default_embedding_model(model_id: Option<String>) -> ConfigSetResponse {
    config::set_config_key_at(
        &app_paths::global_config_path(),
        "defaultEmbeddingModelId",
        model_id
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lorekeeper-llm-{}-{}",
            tag,
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn paths(dir: &Path) -> (PathBuf, PathBuf) {
        (dir.join("models.json"), dir.join("config.json"))
    }

    fn read(path: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn model(id: &str, name: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "name": name,
            "provider": "openai",
            "protocol": "openai",
            "modelName": "gpt-4o-mini",
            "apiKey": "sk-test",
            "baseUrl": "https://api.example.com/v1",
            "temperature": 0.7,
            "maxTokens": 4096,
            "purposes": ["generation"],
        })
    }

    #[test]
    fn list_models_returns_empty_when_missing_or_corrupt_test() {
        let dir = temp_dir("list");
        let (models_path, _) = paths(&dir);

        assert!(read_models_at(&models_path).is_empty(), "缺失 → 空列表");

        std::fs::write(&models_path, b"{ broken").unwrap();
        assert!(read_models_at(&models_path).is_empty(), "损坏 → 回落空列表");

        std::fs::write(&models_path, br#"{"id":"not-an-array"}"#).unwrap();
        assert!(
            read_models_at(&models_path).is_empty(),
            "形状不符 → 回落空列表"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_models_preserves_unknown_fields_test() {
        let dir = temp_dir("list-fields");
        let (models_path, _) = paths(&dir);
        let stored = serde_json::json!([{
            "id": "m-1",
            "capabilities": { "contextWindowTokens": 128000 },
            "embeddingOptions": { "dimensions": 1024 },
            "futureField": "kept",
        }]);
        std::fs::write(&models_path, serde_json::to_string(&stored).unwrap()).unwrap();

        let listed = read_models_at(&models_path);
        assert_eq!(
            listed[0]["capabilities"]["contextWindowTokens"],
            serde_json::json!(128000)
        );
        assert_eq!(
            listed[0]["embeddingOptions"]["dimensions"],
            serde_json::json!(1024)
        );
        assert_eq!(listed[0]["futureField"], serde_json::json!("kept"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_model_inserts_then_updates_in_place_test() {
        let dir = temp_dir("save");
        let (models_path, _) = paths(&dir);

        assert!(save_model_at(&models_path, &model("m-1", "甲")).success);
        assert!(save_model_at(&models_path, &model("m-2", "乙")).success);
        assert!(save_model_at(&models_path, &model("m-1", "甲改")).success);

        let stored = read(&models_path);
        let items = stored.as_array().unwrap();
        assert_eq!(items.len(), 2, "同 id 必须就地更新而不是追加");
        assert_eq!(items[0]["id"], serde_json::json!("m-1"));
        assert_eq!(items[0]["name"], serde_json::json!("甲改"));
        assert_eq!(items[1]["id"], serde_json::json!("m-2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_model_refuses_corrupt_or_non_object_test() {
        let dir = temp_dir("save-corrupt");
        let (models_path, _) = paths(&dir);
        std::fs::write(&models_path, b"{ broken").unwrap();

        let result = save_model_at(&models_path, &model("m-1", "甲"));
        assert!(!result.success);
        assert_eq!(
            result.error.as_deref(),
            Some("Error: 模型配置损坏，已拒绝覆盖")
        );
        assert_eq!(std::fs::read_to_string(&models_path).unwrap(), "{ broken");

        std::fs::write(&models_path, b"[]").unwrap();
        let non_object = save_model_at(&models_path, &serde_json::json!("not-an-object"));
        assert!(!non_object.success);
        assert_eq!(
            non_object.error.as_deref(),
            Some("Error: 模型配置必须是对象")
        );
        assert_eq!(read(&models_path), serde_json::json!([]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_model_clears_both_defaults_and_reports_fields_test() {
        let dir = temp_dir("delete-defaults");
        let (models_path, config_path) = paths(&dir);
        save_model_at(&models_path, &model("m-1", "甲"));
        save_model_at(&models_path, &model("m-2", "乙"));
        std::fs::write(
            &config_path,
            br#"{"theme":"dark","defaultModelId":"m-1","defaultEmbeddingModelId":"m-1"}"#,
        )
        .unwrap();

        let result = delete_model_at(&models_path, &config_path, "m-1");

        assert!(result.success);
        assert!(result.error.is_none());
        assert_eq!(result.default_model_id, Some(serde_json::Value::Null));
        assert_eq!(
            result.default_embedding_model_id,
            Some(serde_json::Value::Null)
        );
        let stored = read(&models_path);
        assert_eq!(stored.as_array().unwrap().len(), 1);
        assert_eq!(stored[0]["id"], serde_json::json!("m-2"));
        let stored_config = read(&config_path);
        assert_eq!(stored_config["defaultModelId"], serde_json::Value::Null);
        assert_eq!(
            stored_config["defaultEmbeddingModelId"],
            serde_json::Value::Null
        );
        assert_eq!(
            stored_config["theme"],
            serde_json::json!("dark"),
            "其它键保留"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_model_keeps_unrelated_defaults_test() {
        let dir = temp_dir("delete-unrelated");
        let (models_path, config_path) = paths(&dir);
        save_model_at(&models_path, &model("m-1", "甲"));
        save_model_at(&models_path, &model("m-2", "乙"));
        std::fs::write(
            &config_path,
            br#"{"defaultModelId":"m-1","defaultEmbeddingModelId":"m-2"}"#,
        )
        .unwrap();

        let result = delete_model_at(&models_path, &config_path, "m-2");

        assert!(result.success);
        assert_eq!(
            result.default_model_id,
            Some(serde_json::json!("m-1")),
            "非目标默认模型必须保留"
        );
        assert_eq!(
            result.default_embedding_model_id,
            Some(serde_json::Value::Null)
        );
        let stored_config = read(&config_path);
        assert_eq!(stored_config["defaultModelId"], serde_json::json!("m-1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_model_omits_default_fields_on_failure_test() {
        let dir = temp_dir("delete-fail-shape");
        let (models_path, config_path) = paths(&dir);
        std::fs::write(&models_path, b"{ broken").unwrap();
        std::fs::write(&config_path, br#"{"defaultModelId":"m-1"}"#).unwrap();

        let result = delete_model_at(&models_path, &config_path, "m-1");
        let serialized = serde_json::to_value(&result).unwrap();

        assert!(!result.success);
        assert_eq!(
            serialized["error"],
            serde_json::json!("Error: 模型配置损坏，已拒绝覆盖")
        );
        assert!(
            serialized.get("defaultModelId").is_none(),
            "失败时不得出现该键"
        );
        assert!(serialized.get("defaultEmbeddingModelId").is_none());
        assert_eq!(
            read(&config_path)["defaultModelId"],
            serde_json::json!("m-1"),
            "配置不得被改动"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_model_rolls_back_config_when_models_write_fails_test() {
        let dir = temp_dir("delete-rollback");
        let (models_path, config_path) = paths(&dir);
        // models.json 用同名目录占位 → 写入必然失败
        std::fs::create_dir(&models_path).unwrap();
        let original_config = serde_json::json!({
            "theme": "light",
            "defaultModelId": "m-1",
        });
        std::fs::write(
            &config_path,
            serde_json::to_string(&original_config).unwrap(),
        )
        .unwrap();

        let result = delete_model_at(&models_path, &config_path, "m-1");

        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().starts_with("Error: "));
        assert_eq!(
            read(&config_path),
            original_config,
            "删除失败必须把默认模型引用回滚为原值"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_failure_message_matches_baseline_double_prefix_test() {
        // 基线：`throw new Error(`${String(e1)}；恢复默认模型配置失败：${String(e2)}`)`
        // 之后又被外层 `String(error)` 包一次 → 最终文案首部出现两处 `Error: `。
        // 传入的是**裸**错误描述（与 Rust `io::Error::to_string()` 同层）。
        assert_eq!(
            rollback_failure_message("EACCES 写失败", "EPERM 回滚失败"),
            "Error: EACCES 写失败；恢复默认模型配置失败：Error: EPERM 回滚失败"
        );
        assert_eq!(
            failure(rollback_failure_message("EACCES 写失败", "EPERM 回滚失败"))
                .error
                .unwrap(),
            "Error: Error: EACCES 写失败；恢复默认模型配置失败：Error: EPERM 回滚失败"
        );
    }

    #[test]
    fn default_model_get_set_round_trip_test() {
        let dir = temp_dir("defaults");
        let (_, config_path) = paths(&dir);

        assert_eq!(get_default_model_at(&config_path), None, "缺文件 → null");
        assert!(
            config::set_config_key_at(&config_path, "defaultModelId", serde_json::json!("m-1"))
                .success
        );
        assert_eq!(get_default_model_at(&config_path).as_deref(), Some("m-1"));

        assert!(
            config::set_config_key_at(&config_path, "defaultModelId", serde_json::Value::Null)
                .success
        );
        assert_eq!(get_default_model_at(&config_path), None, "显式 null → null");

        assert_eq!(get_default_embedding_model_at(&config_path), None);
        assert!(
            config::set_config_key_at(
                &config_path,
                "defaultEmbeddingModelId",
                serde_json::json!("e-1")
            )
            .success
        );
        assert_eq!(
            get_default_embedding_model_at(&config_path).as_deref(),
            Some("e-1")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_default_refuses_corrupt_config_test() {
        let dir = temp_dir("defaults-corrupt");
        let (_, config_path) = paths(&dir);
        std::fs::write(&config_path, b"{ broken").unwrap();

        let result =
            config::set_config_key_at(&config_path, "defaultModelId", serde_json::json!("m-1"));
        assert!(!result.success);
        assert_eq!(
            result.error.as_deref(),
            Some("Error: 全局配置损坏，已拒绝覆盖")
        );
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), "{ broken");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 命令层接线验证：把数据根指到临时目录后调用真实命令。
    ///
    /// 注意：`AI_NOVEL_LOREKEEPER_HOME` 是**进程级**环境变量，本测试会保存并恢复它；
    /// 其它测试都不读取该变量（`app_paths` 的解析逻辑用注入式纯函数测试），因此
    /// 这里串行化风险可接受。
    #[test]
    fn command_wrappers_use_lorekeeper_home_test() {
        let dir = temp_dir("wrappers");
        let previous = std::env::var(app_paths::LOREKEEPER_HOME_ENV).ok();
        std::env::set_var(app_paths::LOREKEEPER_HOME_ENV, &dir);

        assert!(llm_list_models().is_empty());
        assert!(llm_save_model(model("m-1", "甲")).success);
        assert_eq!(llm_list_models().len(), 1);
        assert!(llm_set_default_model(Some("m-1".to_string())).success);
        assert_eq!(llm_get_default_model().as_deref(), Some("m-1"));
        assert!(llm_set_default_embedding_model(Some("m-1".to_string())).success);
        assert_eq!(llm_get_default_embedding_model().as_deref(), Some("m-1"));

        let deleted = llm_delete_model("m-1".to_string());
        assert!(deleted.success);
        assert!(llm_list_models().is_empty());
        assert_eq!(llm_get_default_model(), None);

        match previous {
            Some(value) => std::env::set_var(app_paths::LOREKEEPER_HOME_ENV, value),
            None => std::env::remove_var(app_paths::LOREKEEPER_HOME_ENV),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
