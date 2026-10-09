//! 更新域类型镜像 —— 平移自 `src/shared/update-types.ts`（111 行）。
//!
//! 该文件只镜像**跨进程数据契约**，不含任何逻辑；所有字段名/取值与 TS 侧逐字对齐
//! （serde 重命名规则见各类型注解）。⚠️ `src/shared/update-types.ts` 是上游镜像文件，
//! **不得修改**；契约变化时两侧同步。

use serde::{Deserialize, Serialize};

/// 对齐契约 `UpdateReleaseInfo`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReleaseInfo {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
}

/// 对齐契约 `UpdateCheckResult`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_info: Option<UpdateReleaseInfo>,
}

/// 对齐契约 `UpdateReminder`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReminder {
    pub version: String,
    pub until: String,
}

/// 对齐契约 `UpdatePreferences`（持久化在全局配置的 `updatePreferences` 键）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePreferences {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_automatic_check_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_update: Option<UpdateReleaseInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder: Option<UpdateReminder>,
}

/// 对齐契约 `UpdateReminderDelay`（仅 7 / 30）
pub type UpdateReminderDelay = u32;

/// 对齐契约 `UpdateAction`（kebab-case）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateAction {
    Download,
    OpenRelease,
}

/// 对齐契约 `UpdateStatus`（kebab-case）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateStatus {
    Idle,
    Checking,
    NotAvailable,
    Available,
    Downloading,
    Downloaded,
    Error,
    Disabled,
}

/// 对齐契约 `UpdateErrorCode`（SCREAMING_SNAKE_CASE）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UpdateErrorCode {
    UpdatesDisabled,
    UpdateConfigurationMissing,
    CheckFailed,
    DownloadNotReady,
    DownloadFailed,
    InstallNotReady,
    InstallFailed,
    OpenReleaseFailed,
    ReminderNotAvailable,
    ReminderSaveFailed,
    InvalidReminderDelay,
}

/// 对齐契约 `UpdateErrorPhase`（kebab-case）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateErrorPhase {
    Configuration,
    Check,
    Download,
    Install,
    Navigation,
    Reminder,
}

/// 对齐契约 `UpdateErrorReason`（kebab-case）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateErrorReason {
    NotInstalled,
    ConfigurationMissing,
    Network,
    Proxy,
    Tls,
    HttpForbidden,
    HttpNotFound,
    HttpRateLimited,
    MetadataInvalid,
    AssetMissing,
    NotReady,
    ReminderUnavailable,
    ReminderSaveFailed,
    InvalidReminderDelay,
    InstallFailed,
    OpenReleaseFailed,
    Unknown,
}

/// 对齐契约 `SafeUpdateTechnicalDetails`（SCREAMING_SNAKE_CASE，脱敏白名单）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SafeUpdateTechnicalDetails {
    UpdatesDisabled,
    UpdateConfigurationMissing,
    DnsOrOffline,
    ProxyConnectFailed,
    TlsHandshakeFailed,
    Http403,
    Http404,
    Http429,
    UpdateMetadataInvalid,
    UpdateAssetMissing,
    UpdateOperationFailed,
    DownloadNotReady,
    InstallNotReady,
    InstallFailed,
    OpenReleaseFailed,
    ReminderNotAvailable,
    ReminderSaveFailed,
    InvalidReminderDelay,
}

/// 对齐契约 `UpdateError`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateError {
    pub code: UpdateErrorCode,
    pub phase: UpdateErrorPhase,
    pub reason: UpdateErrorReason,
    pub retryable: bool,
    pub safe_technical_details: SafeUpdateTechnicalDetails,
}

/// 对齐契约 `UpdateDownloadProgress`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDownloadProgress {
    pub percent: f64,
    pub transferred: f64,
    pub total: f64,
    pub bytes_per_second: f64,
}

/// 对齐契约 `UpdateState`（只含可安全发给渲染进程的值）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub status: UpdateStatus,
    pub current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_action: Option<UpdateAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder_until: Option<String>,
    pub is_reminder_deferred: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_progress: Option<UpdateDownloadProgress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<UpdateError>,
}

/// 对齐契约 `UpdateCheckResponse`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResponse {
    pub success: bool,
    pub checked: bool,
    pub update_available: bool,
    pub state: UpdateState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<UpdateError>,
}

/// 对齐契约 `UpdateActionResponse`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateActionResponse {
    pub success: bool,
    pub state: UpdateState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<UpdateError>,
}

/// 对齐基线 `makeUpdateError`
pub fn make_update_error(
    code: UpdateErrorCode,
    phase: UpdateErrorPhase,
    reason: UpdateErrorReason,
    retryable: bool,
    safe_technical_details: SafeUpdateTechnicalDetails,
) -> UpdateError {
    UpdateError {
        code,
        phase,
        reason,
        retryable,
        safe_technical_details,
    }
}

/// 对齐基线 `updateConfigurationMissingError`
pub fn update_configuration_missing_error() -> UpdateError {
    make_update_error(
        UpdateErrorCode::UpdateConfigurationMissing,
        UpdateErrorPhase::Configuration,
        UpdateErrorReason::ConfigurationMissing,
        false,
        SafeUpdateTechnicalDetails::UpdateConfigurationMissing,
    )
}

/// 对齐基线 `UPDATES_DISABLED` 信封
pub fn updates_disabled_error() -> UpdateError {
    make_update_error(
        UpdateErrorCode::UpdatesDisabled,
        UpdateErrorPhase::Configuration,
        UpdateErrorReason::NotInstalled,
        false,
        SafeUpdateTechnicalDetails::UpdatesDisabled,
    )
}

/// 对齐基线 `DOWNLOAD_NOT_READY`
pub fn download_not_ready_error() -> UpdateError {
    make_update_error(
        UpdateErrorCode::DownloadNotReady,
        UpdateErrorPhase::Download,
        UpdateErrorReason::NotReady,
        true,
        SafeUpdateTechnicalDetails::DownloadNotReady,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_serializes_with_camel_case_and_omits_absent_optionals_test() {
        let state = UpdateState {
            status: UpdateStatus::Disabled,
            current_version: "1.1.0".to_string(),
            available_version: None,
            update_action: None,
            release_name: None,
            release_notes: None,
            release_date: None,
            last_checked_at: None,
            reminder_until: None,
            is_reminder_deferred: false,
            download_progress: None,
            error: Some(updates_disabled_error()),
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["status"], "disabled");
        assert_eq!(json["currentVersion"], "1.1.0");
        assert_eq!(json["isReminderDeferred"], false);
        assert!(json.get("availableVersion").is_none(), "缺省字段不得序列化");
        assert_eq!(json["error"]["code"], "UPDATES_DISABLED");
        assert_eq!(json["error"]["phase"], "configuration");
        assert_eq!(json["error"]["reason"], "not-installed");
        assert_eq!(json["error"]["safeTechnicalDetails"], "UPDATES_DISABLED");
    }

    #[test]
    fn enum_renames_match_contract_test() {
        assert_eq!(
            serde_json::to_value(UpdateStatus::NotAvailable).unwrap(),
            "not-available"
        );
        assert_eq!(
            serde_json::to_value(UpdateAction::OpenRelease).unwrap(),
            "open-release"
        );
        assert_eq!(
            serde_json::to_value(UpdateErrorReason::HttpRateLimited).unwrap(),
            "http-rate-limited"
        );
        assert_eq!(
            serde_json::to_value(SafeUpdateTechnicalDetails::DnsOrOffline).unwrap(),
            "DNS_OR_OFFLINE"
        );
        assert_eq!(
            serde_json::to_value(UpdateErrorCode::ReminderSaveFailed).unwrap(),
            "REMINDER_SAVE_FAILED"
        );
    }

    #[test]
    fn preferences_roundtrip_and_unknown_fields_ignored_test() {
        let raw = r#"{
            "lastCheckedAt": "2026-10-09T12:00:00.000Z",
            "lastAutomaticCheckDate": "2026-10-09",
            "availableUpdate": { "version": "1.2.0", "releaseName": "R" },
            "reminder": { "version": "1.2.0", "until": "2026-10-16T12:00:00.000Z" },
            "legacyUnknownKey": 1
        }"#;
        let preferences: UpdatePreferences = serde_json::from_str(raw).unwrap();
        assert_eq!(
            preferences.last_automatic_check_date.as_deref(),
            Some("2026-10-09")
        );
        assert_eq!(
            preferences
                .available_update
                .as_ref()
                .map(|item| item.version.as_str()),
            Some("1.2.0")
        );
        assert_eq!(
            preferences
                .reminder
                .as_ref()
                .map(|item| item.until.as_str()),
            Some("2026-10-16T12:00:00.000Z")
        );
        // 写回时字段名保持 camelCase
        let json = serde_json::to_value(&preferences).unwrap();
        assert_eq!(json["lastAutomaticCheckDate"], "2026-10-09");
    }
}
