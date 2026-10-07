//! 批次 D2：LLM 生成执行域命令 —— 迁移自 `electron/controllers/llm-controller.ts`
//! 的租约部分（生成 / 流式 / 发现 / 连通性见后续子批次）。
//!
//! | 频道 | 命令 | 基线行为 |
//! |---|---|---|
//! | `llm:begin-execution-lease` | `llm_begin_execution_lease` | 冻结模型快照并签发回执 |
//! | `llm:close-execution-lease` | `llm_close_execution_lease` | 幂等关闭（墓碑窗口内重复关闭仍成功） |
//!
//! 失败信封刻意与生成类命令不同（对齐基线）：
//! - 租约失败是**结构化**结果（`errorCode`），文案**不带** `Error: ` 前缀；
//! - 生成类命令走异常路径，文案带 `Error: ` 前缀（见 `commands::db::mutating_error`）。

use serde::Serialize;
use std::path::Path;

use crate::app_paths;
use crate::llm::lease::{
    LlmLeaseStoreCloseOutcome, ModelExecutionLeaseError, ModelExecutionLeaseReceipt,
};
use crate::state::AppState;

/// `errorCode`：模型不存在（基线 `MODEL_NOT_FOUND`）。
pub const MODEL_NOT_FOUND_CODE: &str = "MODEL_NOT_FOUND";

/// `errorCode`：租约创建失败（基线 `LEASE_BEGIN_FAILED`）。
pub const LEASE_BEGIN_FAILED_CODE: &str = "LEASE_BEGIN_FAILED";

/// 租约开启失败文案（对齐基线，**无** `Error: ` 前缀）。
pub const MODEL_NOT_FOUND_MESSAGE: &str = "指定的生成模型不存在或已被删除。";

/// 租约开启未知失败文案（对齐基线，**无** `Error: ` 前缀）。
pub const LEASE_BEGIN_FAILED_MESSAGE: &str = "无法创建模型执行租约。";

/// 关闭未知租约文案（对齐基线，**无** `Error: ` 前缀）。
pub const LEASE_CLOSE_FAILED_MESSAGE: &str = "模型执行租约无效或已关闭";

/// `llm:begin-execution-lease` 响应（成功带 `lease`，失败带 `errorCode`）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmBeginExecutionLeaseResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease: Option<ModelExecutionLeaseReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `llm:close-execution-lease` 响应。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmCloseExecutionLeaseResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 当前时间（毫秒）—— 与 `AppState.started_at_ms` 同一口径。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// 开启租约核心（路径与时钟注入，便于测试）。
pub(crate) fn begin_execution_lease_at(
    state: &AppState,
    models_path: &Path,
    model_id: &str,
    lease_id: String,
    now_ms: u64,
) -> LlmBeginExecutionLeaseResult {
    let Ok(mut store) = state.llm_leases.lock() else {
        return LlmBeginExecutionLeaseResult {
            success: false,
            lease: None,
            error_code: Some(LEASE_BEGIN_FAILED_CODE.to_string()),
            error: Some(LEASE_BEGIN_FAILED_MESSAGE.to_string()),
        };
    };
    match store.begin_at(models_path, model_id, lease_id, now_ms) {
        Ok(lease) => LlmBeginExecutionLeaseResult {
            success: true,
            lease: Some(lease),
            error_code: None,
            error: None,
        },
        Err(ModelExecutionLeaseError::ModelNotFound) => LlmBeginExecutionLeaseResult {
            success: false,
            lease: None,
            error_code: Some(MODEL_NOT_FOUND_CODE.to_string()),
            error: Some(MODEL_NOT_FOUND_MESSAGE.to_string()),
        },
        Err(_) => LlmBeginExecutionLeaseResult {
            success: false,
            lease: None,
            error_code: Some(LEASE_BEGIN_FAILED_CODE.to_string()),
            error: Some(LEASE_BEGIN_FAILED_MESSAGE.to_string()),
        },
    }
}

/// 关闭租约核心（`Closed` 与墓碑窗口内的 `AlreadyClosed` 都算成功）。
pub(crate) fn close_execution_lease_at(state: &AppState, lease_id: &str, now_ms: u64) -> LlmCloseExecutionLeaseResult {
    let Ok(mut store) = state.llm_leases.lock() else {
        return LlmCloseExecutionLeaseResult {
            success: false,
            error: Some(LEASE_CLOSE_FAILED_MESSAGE.to_string()),
        };
    };
    match store.close(lease_id, now_ms) {
        LlmLeaseStoreCloseOutcome::Closed | LlmLeaseStoreCloseOutcome::AlreadyClosed => {
            LlmCloseExecutionLeaseResult {
                success: true,
                error: None,
            }
        }
        LlmLeaseStoreCloseOutcome::Unknown => LlmCloseExecutionLeaseResult {
            success: false,
            error: Some(LEASE_CLOSE_FAILED_MESSAGE.to_string()),
        },
    }
}

#[tauri::command]
pub fn llm_begin_execution_lease(
    state: tauri::State<'_, AppState>,
    model_id: String,
) -> LlmBeginExecutionLeaseResult {
    begin_execution_lease_at(
        &state,
        &app_paths::models_config_path(),
        &model_id,
        crate::project_access::random_uuid_v4(),
        now_ms(),
    )
}

#[tauri::command]
pub fn llm_close_execution_lease(
    state: tauri::State<'_, AppState>,
    lease_id: String,
) -> LlmCloseExecutionLeaseResult {
    close_execution_lease_at(&state, &lease_id, now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lorekeeper-lease-cmd-{}-{}",
            tag,
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn models_path(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("models.json");
        let models = json!([{
            "id": "m-1",
            "name": "深寻",
            "provider": "deepseek",
            "protocol": "openai",
            "modelName": "deepseek-v4-flash",
            "apiKey": "sk-secret",
            "baseUrl": "https://api.deepseek.com",
            "temperature": 0.7,
            "maxTokens": 4096,
            "purposes": ["generation"],
        }]);
        std::fs::write(&path, serde_json::to_string(&models).unwrap()).unwrap();
        path
    }

    #[test]
    fn begin_then_close_round_trip_test() {
        let dir = temp_dir("round-trip");
        let path = models_path(&dir);
        let state = AppState::new();

        let begun = begin_execution_lease_at(&state, &path, "m-1", "lease-a".to_string(), 1_000);
        assert!(begun.success);
        let receipt = begun.lease.expect("成功必须带 lease");
        assert_eq!(receipt.lease_id, "lease-a");
        assert_eq!(receipt.model_id, "m-1");
        assert_eq!(receipt.provider, "deepseek");
        assert_eq!(receipt.model_name, "deepseek-v4-flash");
        assert_eq!(receipt.capability_evidence.max_output_tokens, 4_096);

        let first_close = close_execution_lease_at(&state, "lease-a", 1_100);
        assert!(first_close.success);
        assert!(first_close.error.is_none());

        let replayed = close_execution_lease_at(&state, "lease-a", 1_200);
        assert!(replayed.success, "墓碑窗口内重复关闭必须幂等成功");

        let failed = close_execution_lease_at(&state, "never-issued", 1_200);
        assert!(!failed.success);
        assert_eq!(failed.error.as_deref(), Some(LEASE_CLOSE_FAILED_MESSAGE));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn begin_reports_structured_model_not_found_test() {
        let dir = temp_dir("model-not-found");
        let path = models_path(&dir);
        let state = AppState::new();

        let result = begin_execution_lease_at(&state, &path, "missing", "lease-a".to_string(), 1_000);
        let serialized = serde_json::to_value(&result).unwrap();

        assert!(!result.success);
        assert_eq!(serialized["errorCode"], json!(MODEL_NOT_FOUND_CODE));
        assert_eq!(serialized["error"], json!(MODEL_NOT_FOUND_MESSAGE));
        assert!(serialized.get("lease").is_none(), "失败时不得出现 lease 键");
        assert!(
            !serialized["error"].as_str().unwrap().starts_with("Error: "),
            "租约失败文案不得带 Error: 前缀（对齐基线结构化返回）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn begin_reports_lease_begin_failed_for_invalid_capability_test() {
        let dir = temp_dir("capability-failed");
        let path = dir.join("models.json");
        // 既无 maxTokens 也无 capabilities → 无法推导输出上限
        std::fs::write(
            &path,
            serde_json::to_string(&json!([{
                "id": "m-1",
                "provider": "custom",
                "protocol": "openai",
                "baseUrl": "https://relay.example.com/v1",
                "modelName": "whatever",
                "temperature": 0.5,
            }]))
            .unwrap(),
        )
        .unwrap();
        let state = AppState::new();

        let result = begin_execution_lease_at(&state, &path, "m-1", "lease-a".to_string(), 1_000);

        assert!(!result.success);
        assert_eq!(
            result.error_code.as_deref(),
            Some(LEASE_BEGIN_FAILED_CODE),
            "非「模型不存在」的失败必须落到 LEASE_BEGIN_FAILED"
        );
        assert_eq!(result.error.as_deref(), Some(LEASE_BEGIN_FAILED_MESSAGE));
        assert!(result.lease.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn begin_records_full_receipt_shape_test() {
        let dir = temp_dir("receipt-shape");
        let path = models_path(&dir);
        let state = AppState::new();

        let receipt = begin_execution_lease_at(&state, &path, "m-1", "lease-a".to_string(), 1_000)
            .lease
            .unwrap();
        let serialized = serde_json::to_value(&receipt).unwrap();

        for key in [
            "leaseId",
            "modelId",
            "provider",
            "protocol",
            "modelName",
            "modelRevision",
            "endpointFingerprint",
            "capabilityEvidence",
            "createdAt",
            "expiresAt",
        ] {
            assert!(serialized.get(key).is_some(), "回执必须包含 {key}");
        }
        assert_eq!(serialized["createdAt"], json!(1_000));
        assert_eq!(
            serialized["expiresAt"],
            json!(1_000 + crate::llm::lease::DEFAULT_MODEL_EXECUTION_LEASE_TTL_MS)
        );
        assert_eq!(
            serialized["capabilityEvidence"]["source"]["featureFlags"],
            json!("verified-provider-preset")
        );
        assert!(
            !serde_json::to_string(&serialized).unwrap().contains("sk-secret"),
            "回执不得携带密钥"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
