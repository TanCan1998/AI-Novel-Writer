//! 更新后端 —— 平移自 `electron/services/github-release-update-backend.ts`（49 行）
//! 与 `update-service.ts:98-171` 的错误分类器。
//!
//! 本批唯一后端是 **GitHub Releases 元数据**（只读）：
//! `downloadUpdate()` 恒返回空数组、`quitAndInstall()` 为空操作 —— 与基线的
//! GitHub/macOS 后端逐字一致（不做就地下载与安装，见 B14 与 `runtime.rs` 的偏离说明）。
//!
//! **仓库指向**：fork `TanCan1998/Lorekeeper`（2026-10-09 用户确认；与 B12 官方主页一致）。
//! 基线常量指向上游 `EthanYoQ/AI-Novel-Writer`。

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::update::types::{
    make_update_error, update_configuration_missing_error, SafeUpdateTechnicalDetails,
    UpdateCheckResult, UpdateError, UpdateErrorCode, UpdateErrorPhase, UpdateErrorReason,
    UpdateReleaseInfo,
};

/// fork 仓库的 Releases 元数据 API（对齐基线 `GITHUB_LATEST_RELEASE_API` 的形态）
pub const GITHUB_LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/TanCan1998/Lorekeeper/releases/latest";
/// fork 仓库的 Releases 页（`update:open-release` 打开它）
pub const GITHUB_LATEST_RELEASE_PAGE: &str =
    "https://github.com/TanCan1998/Lorekeeper/releases/latest";
/// `User-Agent`（GitHub API 要求）
pub const GITHUB_USER_AGENT: &str = "Lorekeeper";

/// 后端抓取失败：只携带**可分类**的信息（绝不把原始错误对象跨 IPC 传递）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateFetchError {
    /// 供分类器匹配的小写提示串（原始错误文本 / 自定义标记）
    pub hints: String,
    /// HTTP 状态码（若已知）
    pub status: Option<u16>,
}

impl UpdateFetchError {
    pub fn new(hints: impl Into<String>, status: Option<u16>) -> Self {
        Self {
            hints: hints.into(),
            status,
        }
    }

    /// 对齐基线「更新配置缺失」判定（`app-update.yml` / updater config 文案）
    pub fn configuration_missing() -> Self {
        Self::new("app-update.yml updater configuration missing", None)
    }
}

/// 装箱后的后端 future（保持 trait 可 `dyn`，避免为 `AppState` 引入泛型）
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 可替换的更新后端（对齐基线 `UpdateBackend`）
pub trait UpdateBackend: Send + Sync {
    /// 返回 `Ok(None)` 表示后端不可用（对齐基线 `checkForUpdates(): UpdateCheckResult | null`）
    fn check_for_updates(
        &self,
    ) -> BackendFuture<'_, Result<Option<UpdateCheckResult>, UpdateFetchError>>;

    fn download_update(&self) -> BackendFuture<'_, Result<Vec<String>, UpdateFetchError>>;

    fn quit_and_install(&self) -> Result<(), String> {
        // 默认空操作（GitHub 后端语义）
        Ok(())
    }
}

/// 对齐基线 `classifyUpdateFailure`：把不可信的更新器失败映射为**固定的**渲染层安全分类。
///
/// 与基线的差异（刻意）：network 桶额外接受 `dns`，因为 reqwest 的 DNS 失败文案是
/// `dns error: failed to lookup address information`，而基线匹配的是 Node 的 `ENOTFOUND`。
pub fn classify_update_failure(phase: UpdateErrorPhase, error: &UpdateFetchError) -> UpdateError {
    let hints = error.hints.to_lowercase();
    let status = error.status;
    let code = match phase {
        UpdateErrorPhase::Download => UpdateErrorCode::DownloadFailed,
        _ => UpdateErrorCode::CheckFailed,
    };

    if regex_is_match(
        r"app-update\.ya?ml|update(?:r)? configuration|updater config",
        &hints,
    ) {
        return update_configuration_missing_error();
    }
    if phase == UpdateErrorPhase::Download
        && (status == Some(404)
            || regex_is_match(
                r"(?:asset|installer|update file).*(?:missing|not found|404)",
                &hints,
            ))
    {
        return make_update_error(
            code,
            phase,
            UpdateErrorReason::AssetMissing,
            false,
            SafeUpdateTechnicalDetails::UpdateAssetMissing,
        );
    }
    if regex_is_match(
        r"(?:invalid|malformed|parse).*(?:latest\.ya?ml|metadata|update)|(?:latest\.ya?ml|metadata).*(?:invalid|malformed|parse)",
        &hints,
    ) {
        return make_update_error(
            code,
            phase,
            UpdateErrorReason::MetadataInvalid,
            false,
            SafeUpdateTechnicalDetails::UpdateMetadataInvalid,
        );
    }
    match status {
        Some(403) => {
            return make_update_error(
                code,
                phase,
                UpdateErrorReason::HttpForbidden,
                false,
                SafeUpdateTechnicalDetails::Http403,
            )
        }
        Some(404) => {
            return make_update_error(
                code,
                phase,
                UpdateErrorReason::HttpNotFound,
                false,
                SafeUpdateTechnicalDetails::Http404,
            )
        }
        Some(429) => {
            return make_update_error(
                code,
                phase,
                UpdateErrorReason::HttpRateLimited,
                true,
                SafeUpdateTechnicalDetails::Http429,
            )
        }
        _ => {}
    }
    if regex_is_match(r"proxy|tunnel", &hints) {
        return make_update_error(
            code,
            phase,
            UpdateErrorReason::Proxy,
            true,
            SafeUpdateTechnicalDetails::ProxyConnectFailed,
        );
    }
    if regex_is_match(r"cert|tls|ssl|self signed|unable to verify", &hints) {
        return make_update_error(
            code,
            phase,
            UpdateErrorReason::Tls,
            true,
            SafeUpdateTechnicalDetails::TlsHandshakeFailed,
        );
    }
    if regex_is_match(
        r"enotfound|eai_again|enetunreach|econnrefused|econnreset|etimedout|enotconn|offline|network|dns",
        &hints,
    ) {
        return make_update_error(
            code,
            phase,
            UpdateErrorReason::Network,
            true,
            SafeUpdateTechnicalDetails::DnsOrOffline,
        );
    }
    make_update_error(
        code,
        phase,
        UpdateErrorReason::Unknown,
        true,
        SafeUpdateTechnicalDetails::UpdateOperationFailed,
    )
}

fn regex_is_match(pattern: &str, value: &str) -> bool {
    regex::Regex::new(pattern)
        .map(|regex| regex.is_match(value))
        .unwrap_or(false)
}

/// 对齐基线后端里对 `releases/latest` 响应的解析
pub fn parse_latest_release(release: &Value) -> Result<UpdateReleaseInfo, UpdateFetchError> {
    let Some(object) = release.as_object() else {
        return Err(UpdateFetchError::new(
            "invalid github release metadata",
            None,
        ));
    };
    let Some(tag_name) = object.get("tag_name").and_then(Value::as_str) else {
        return Err(UpdateFetchError::new(
            "invalid github release metadata",
            None,
        ));
    };
    let text_of = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_string);
    Ok(UpdateReleaseInfo {
        version: tag_name.strip_prefix('v').unwrap_or(tag_name).to_string(),
        release_name: text_of("name"),
        release_notes: text_of("body"),
        release_date: text_of("published_at"),
    })
}

/// 对齐基线 `createDisabledUpdateBackend`（门禁关闭 / 初始化失败时使用）
pub struct DisabledBackend;

impl UpdateBackend for DisabledBackend {
    fn check_for_updates(
        &self,
    ) -> BackendFuture<'_, Result<Option<UpdateCheckResult>, UpdateFetchError>> {
        Box::pin(async { Ok(None) })
    }

    fn download_update(&self) -> BackendFuture<'_, Result<Vec<String>, UpdateFetchError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// GitHub Releases 元数据后端（只读）
pub struct GithubReleaseBackend {
    client: reqwest::Client,
}

impl GithubReleaseBackend {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// 生产构造：复用全局配置的代理 + 10s 超时 + **禁止重定向**（对齐基线 `redirect: 'error'`）
    pub fn from_global_config() -> Result<Self, String> {
        let config = crate::json_store::read_json_value_or(
            &crate::app_paths::global_config_path(),
            crate::commands::default_global_config(),
        );
        let proxy = crate::llm::chat::proxy_from_config(&config);
        let client = crate::llm::chat::build_client_with_timeout(
            proxy.as_ref(),
            Some(std::time::Duration::from_secs(10)),
        )?;
        Ok(Self::new(client))
    }
}

impl UpdateBackend for GithubReleaseBackend {
    fn check_for_updates(
        &self,
    ) -> BackendFuture<'_, Result<Option<UpdateCheckResult>, UpdateFetchError>> {
        Box::pin(async move {
            let response = self
                .client
                .get(GITHUB_LATEST_RELEASE_API)
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", GITHUB_USER_AGENT)
                .header("X-GitHub-Api-Version", "2022-11-28")
                .send()
                .await
                .map_err(|error| UpdateFetchError::new(error.to_string(), None))?;
            let status = response.status();
            if !status.is_success() {
                return Err(UpdateFetchError::new(
                    format!("http {}", status.as_u16()),
                    Some(status.as_u16()),
                ));
            }
            let payload: Value = response.json().await.map_err(|error| {
                UpdateFetchError::new(format!("invalid metadata {error}"), None)
            })?;
            let update_info = parse_latest_release(&payload)?;
            Ok(Some(UpdateCheckResult {
                update_info: Some(update_info),
            }))
        })
    }

    fn download_update(&self) -> BackendFuture<'_, Result<Vec<String>, UpdateFetchError>> {
        // 对齐基线 GitHub 后端：不提供就地下载
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn classified(
        phase: UpdateErrorPhase,
        hints: &str,
        status: Option<u16>,
    ) -> (
        UpdateErrorCode,
        UpdateErrorReason,
        bool,
        SafeUpdateTechnicalDetails,
    ) {
        let error = classify_update_failure(phase, &UpdateFetchError::new(hints, status));
        (
            error.code,
            error.reason,
            error.retryable,
            error.safe_technical_details,
        )
    }

    #[test]
    fn classifier_maps_status_codes_and_transport_hints_test() {
        use SafeUpdateTechnicalDetails as Details;
        use UpdateErrorCode::*;
        use UpdateErrorReason::*;

        // 403 / 404 / 429
        assert_eq!(
            classified(UpdateErrorPhase::Check, "", Some(403)),
            (CheckFailed, HttpForbidden, false, Details::Http403)
        );
        assert_eq!(
            classified(UpdateErrorPhase::Check, "", Some(404)),
            (CheckFailed, HttpNotFound, false, Details::Http404)
        );
        assert_eq!(
            classified(UpdateErrorPhase::Check, "", Some(429)),
            (CheckFailed, HttpRateLimited, true, Details::Http429)
        );
        // 代理 / TLS / DNS / 兜底
        assert_eq!(
            classified(UpdateErrorPhase::Check, "connect via proxy failed", None),
            (CheckFailed, Proxy, true, Details::ProxyConnectFailed)
        );
        assert_eq!(
            classified(
                UpdateErrorPhase::Check,
                "tls handshake failed: self signed certificate",
                None
            ),
            (CheckFailed, Tls, true, Details::TlsHandshakeFailed)
        );
        assert_eq!(
            classified(
                UpdateErrorPhase::Check,
                "error trying to connect: dns error: failed to lookup address",
                None
            ),
            (CheckFailed, Network, true, Details::DnsOrOffline)
        );
        assert_eq!(
            classified(UpdateErrorPhase::Check, "something odd", None),
            (CheckFailed, Unknown, true, Details::UpdateOperationFailed)
        );
        // 下载阶段的 code 不同
        assert_eq!(
            classified(UpdateErrorPhase::Download, "network unreachable", None).0,
            DownloadFailed
        );
    }

    #[test]
    fn classifier_maps_configuration_and_metadata_and_asset_test() {
        // 配置缺失（任意阶段都先命中）
        let missing = classify_update_failure(
            UpdateErrorPhase::Check,
            &UpdateFetchError::configuration_missing(),
        );
        assert_eq!(missing.code, UpdateErrorCode::UpdateConfigurationMissing);
        assert_eq!(missing.reason, UpdateErrorReason::ConfigurationMissing);

        // 元数据无效
        let metadata = classify_update_failure(
            UpdateErrorPhase::Check,
            &UpdateFetchError::new("invalid github release metadata", None),
        );
        assert_eq!(metadata.reason, UpdateErrorReason::MetadataInvalid);
        assert_eq!(metadata.code, UpdateErrorCode::CheckFailed);

        // 下载阶段 404 → asset-missing（优先于 http-not-found）
        let asset = classify_update_failure(
            UpdateErrorPhase::Download,
            &UpdateFetchError::new("installer not found", Some(404)),
        );
        assert_eq!(asset.reason, UpdateErrorReason::AssetMissing);
        assert_eq!(asset.code, UpdateErrorCode::DownloadFailed);

        // 检查阶段 404 → http-not-found（不进入 asset 分支）
        let check_404 = classify_update_failure(
            UpdateErrorPhase::Check,
            &UpdateFetchError::new("installer not found", Some(404)),
        );
        assert_eq!(check_404.reason, UpdateErrorReason::HttpNotFound);
    }

    #[test]
    fn parse_latest_release_matches_baseline_mapping_test() {
        let release = parse_latest_release(&json!({
            "tag_name": "v1.2.0",
            "name": "Lorekeeper 1.2.0",
            "body": "修复若干问题",
            "published_at": "2026-10-01T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(release.version, "1.2.0", "tag_name 的单个前导 v 被剥离");
        assert_eq!(release.release_name.as_deref(), Some("Lorekeeper 1.2.0"));
        assert_eq!(release.release_notes.as_deref(), Some("修复若干问题"));
        assert_eq!(
            release.release_date.as_deref(),
            Some("2026-10-01T00:00:00Z")
        );

        // 可选字段缺省
        let minimal = parse_latest_release(&json!({ "tag_name": "1.0.0" })).unwrap();
        assert_eq!(minimal.version, "1.0.0");
        assert_eq!(minimal.release_name, None);
        assert_eq!(minimal.release_notes, None);
        assert_eq!(minimal.release_date, None);

        // 缺 tag_name / 非对象 → 元数据无效
        assert_eq!(
            parse_latest_release(&json!({ "name": "x" })).unwrap_err(),
            UpdateFetchError::new("invalid github release metadata", None)
        );
        assert!(parse_latest_release(&json!([1, 2, 3])).is_err());
    }

    #[test]
    fn release_urls_point_to_fork_test() {
        assert_eq!(
            GITHUB_LATEST_RELEASE_API,
            "https://api.github.com/repos/TanCan1998/Lorekeeper/releases/latest"
        );
        assert_eq!(
            GITHUB_LATEST_RELEASE_PAGE,
            "https://github.com/TanCan1998/Lorekeeper/releases/latest"
        );
        assert!(GITHUB_LATEST_RELEASE_PAGE.starts_with("https://"));
    }
}
