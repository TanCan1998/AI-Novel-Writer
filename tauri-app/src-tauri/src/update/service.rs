//! 更新服务状态机 —— 平移自 `electron/services/update-service.ts`（585 行）。
//!
//! 并发纪律（与基线 `checkQueue` + 单线程 JS 等价）：
//! - `inner`（`std::sync::Mutex<Inner>`）只在**同步临界区**内加锁，**绝不在 `.await` 前后持有**；
//! - `check_queue`（`tokio::sync::Mutex<()>`）串行化「检查 / 下载」全过程，避免并发检查互相覆盖状态；
//! - 状态每次变更都调用 `publish`（Tauri 侧注入 `app.emit("update:state", …)`）。
//!
//! 行为要点（逐条对齐基线，**不得放宽**）：
//! 1. 未打包 → `disabled`；`updateConfiguration === 'missing'` → 手动检查返回可行动的
//!    `UPDATE_CONFIGURATION_MISSING`；
//! 2. 成功自动检查**每天最多一次**（日历日节流），手动检查始终可绕过；
//! 3. 自动检查失败只留在主进程（状态回到 `idle`/`available`，不向渲染层暴露错误）；
//! 4. 偏好写入失败不抛错，但自动检查会因此**不联网**（避免每次重启绕过节流）；
//! 5. 只有严格更高的稳定版本才算可用更新（预发布/非 SemVer 一律忽略）。

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::update::backend::UpdateBackend;
use crate::update::preferences::UpdatePreferencesStore;
use crate::update::time::{parse_iso8601_millis, utc_calendar_date};
use crate::update::types::{
    download_not_ready_error, make_update_error, update_configuration_missing_error,
    updates_disabled_error, SafeUpdateTechnicalDetails, UpdateAction, UpdateActionResponse,
    UpdateCheckResponse, UpdateDownloadProgress, UpdateError, UpdateErrorCode, UpdateErrorPhase,
    UpdateErrorReason, UpdatePreferences, UpdateReleaseInfo, UpdateReminder, UpdateState,
    UpdateStatus,
};
use crate::update::version::is_higher_stable_version;

/// 打开 Release 页的注入闭包（Tauri 侧复用 `external_link::open_external_url`）
pub type OpenReleaseFn = Arc<
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
        + Send
        + Sync,
>;
/// 状态发布闭包（Tauri 侧 `app.emit("update:state", state)`）
pub type PublishFn = Arc<dyn Fn(&UpdateState) + Send + Sync>;
/// 时钟注入（毫秒时间戳；便于单测确定性）
pub type NowFn = Arc<dyn Fn() -> u64 + Send + Sync>;

/// 对齐基线 `UpdateServiceOptions`
pub struct UpdateServiceOptions {
    pub updater: Arc<dyn UpdateBackend>,
    pub current_version: String,
    pub is_packaged: bool,
    /// 对齐基线 `updateConfiguration === 'missing'`
    pub update_configuration_missing: bool,
    pub update_action: UpdateAction,
    pub open_release: Option<OpenReleaseFn>,
    pub preferences: Arc<dyn UpdatePreferencesStore>,
    pub now_millis: Option<NowFn>,
    pub publish: Option<PublishFn>,
}

struct Inner {
    state: UpdateState,
    reminder: Option<UpdateReminder>,
    downloaded_version: Option<String>,
    checked_available_version: Option<String>,
}

pub struct UpdateService {
    options: UpdateServiceOptions,
    now_millis: NowFn,
    inner: Mutex<Inner>,
    check_queue: tokio::sync::Mutex<()>,
}

impl UpdateService {
    pub fn new(options: UpdateServiceOptions) -> Self {
        let now_millis: NowFn = options
            .now_millis
            .clone()
            .unwrap_or_else(|| Arc::new(crate::commands::project::epoch_millis_now));
        let preferences = options.preferences.read();
        let reminder = preferences.reminder.clone();
        let available_update = match preferences.available_update.clone() {
            Some(update) if is_higher_stable_version(&update.version, &options.current_version) => {
                Some(update)
            }
            _ => None,
        };
        let (reminder_until, is_reminder_deferred) = match (&available_update, &reminder) {
            (Some(update), Some(reminder)) => {
                let state = Self::reminder_state_for(reminder, &update.version, now_millis());
                (state.0, state.1)
            }
            _ => (None, false),
        };
        let status = if options.is_packaged {
            if available_update.is_some() {
                UpdateStatus::Available
            } else {
                UpdateStatus::Idle
            }
        } else {
            UpdateStatus::Disabled
        };
        let state = UpdateState {
            status,
            current_version: options.current_version.clone(),
            available_version: available_update
                .as_ref()
                .map(|update| update.version.clone()),
            update_action: available_update.as_ref().map(|_| options.update_action),
            release_name: available_update
                .as_ref()
                .and_then(|update| update.release_name.clone()),
            release_notes: available_update
                .as_ref()
                .and_then(|update| update.release_notes.clone()),
            release_date: available_update
                .as_ref()
                .and_then(|update| update.release_date.clone()),
            last_checked_at: preferences.last_checked_at.clone(),
            reminder_until,
            is_reminder_deferred,
            download_progress: None,
            error: None,
        };
        Self {
            options,
            now_millis,
            inner: Mutex::new(Inner {
                state,
                reminder,
                downloaded_version: None,
                checked_available_version: None,
            }),
            check_queue: tokio::sync::Mutex::new(()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // 状态锁不参与跨线程不变式，污染时直接取回内层值（避免一次 panic 让更新域永久失效）
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn now(&self) -> u64 {
        (self.now_millis)()
    }

    /// 对齐基线 `getState`：叠加当前生效的延后提醒
    pub fn get_state(&self) -> UpdateState {
        let now = self.now();
        let inner = self.lock();
        Self::snapshot(&inner, now)
    }

    fn snapshot(inner: &Inner, now: u64) -> UpdateState {
        let reminder = match (&inner.state.available_version, &inner.reminder) {
            (Some(version), Some(reminder)) => Self::reminder_for(reminder, version, now),
            _ => None,
        };
        UpdateState {
            reminder_until: reminder.as_ref().map(|item| item.until.clone()),
            is_reminder_deferred: reminder.is_some(),
            ..inner.state.clone()
        }
    }

    fn reminder_for(reminder: &UpdateReminder, version: &str, now: u64) -> Option<UpdateReminder> {
        if reminder.version != version {
            return None;
        }
        let until = parse_iso8601_millis(&reminder.until)?;
        if until <= now as i64 {
            return None;
        }
        Some(reminder.clone())
    }

    fn reminder_state_for(
        reminder: &UpdateReminder,
        version: &str,
        now: u64,
    ) -> (Option<String>, bool) {
        match Self::reminder_for(reminder, version, now) {
            Some(active) => (Some(active.until), true),
            None => (None, false),
        }
    }

    fn publish(&self, state: &UpdateState) {
        if let Some(publish) = &self.options.publish {
            publish(state);
        }
    }

    /// 对齐基线 `setState`
    fn set_state(&self, next: UpdateState) {
        let now = self.now();
        let snapshot = {
            let mut inner = self.lock();
            inner.state = next;
            Self::snapshot(&inner, now)
        };
        self.publish(&snapshot);
    }

    fn read_preferences(&self) -> UpdatePreferences {
        self.options.preferences.read()
    }

    fn write_preferences(&self, preferences: &UpdatePreferences) -> bool {
        self.options.preferences.write(preferences)
    }

    fn response(
        &self,
        success: bool,
        checked: bool,
        update_available: Option<bool>,
        error: Option<UpdateError>,
    ) -> UpdateCheckResponse {
        UpdateCheckResponse {
            success,
            checked,
            update_available: update_available
                .unwrap_or_else(|| self.lock().state.available_version.is_some()),
            state: self.get_state(),
            error,
        }
    }

    fn action_response(&self, success: bool, error: Option<UpdateError>) -> UpdateActionResponse {
        UpdateActionResponse {
            success,
            state: self.get_state(),
            error,
        }
    }

    fn disabled_response(&self) -> UpdateCheckResponse {
        let mut next = self.get_state();
        next.status = UpdateStatus::Disabled;
        self.set_state(next);
        self.response(false, false, None, Some(updates_disabled_error()))
    }

    fn handle_failure(
        &self,
        mode: CheckMode,
        error: UpdateError,
        automatic_status: UpdateStatus,
        checked: bool,
    ) -> UpdateCheckResponse {
        if mode == CheckMode::Manual {
            let mut next = self.get_state();
            next.status = UpdateStatus::Error;
            next.error = Some(error.clone());
            self.set_state(next);
            return self.response(false, checked, None, Some(error));
        }
        // 自动检查失败只留在主进程：不向渲染进程提供打扰性错误状态
        let mut next = self.get_state();
        next.status = automatic_status;
        next.error = None;
        self.set_state(next);
        self.response(false, checked, None, None)
    }

    /// 当前可用版本对应的回退状态（`available` / `idle`）
    fn fallback_status(&self) -> UpdateStatus {
        if self.lock().state.available_version.is_some() {
            UpdateStatus::Available
        } else {
            UpdateStatus::Idle
        }
    }

    /// 对齐基线 `checkAutomatically`
    pub async fn check_automatically(&self) -> UpdateCheckResponse {
        if !self.options.is_packaged {
            return self.disabled_response();
        }
        if self.options.update_configuration_missing {
            return self.handle_failure(
                CheckMode::Automatic,
                update_configuration_missing_error(),
                self.fallback_status(),
                false,
            );
        }
        if self.lock().downloaded_version.is_some() {
            return self.response(true, false, Some(true), None);
        }

        let now = self.now();
        let today = utc_calendar_date(now);
        let preferences = self.read_preferences();
        if preferences.last_automatic_check_date.as_deref() == Some(today.as_str()) {
            return self.response(true, false, None, None);
        }
        if !self.write_preferences(&preferences) {
            return self.response(false, false, None, None);
        }
        let service = self;
        let today_for_write = today.clone();
        self.enqueue_check(move || async move {
            let result = service.perform_check(CheckMode::Automatic, now).await;
            if result.success && result.checked {
                let mut updated = service.read_preferences();
                updated.last_checked_at =
                    Some(crate::commands::project::iso8601_utc_from_millis(now));
                updated.last_automatic_check_date = Some(today_for_write);
                service.write_preferences(&updated);
            }
            result
        })
        .await
    }

    /// 对齐基线 `checkManually`
    pub async fn check_manually(&self) -> UpdateCheckResponse {
        if !self.options.is_packaged {
            return self.disabled_response();
        }
        if self.options.update_configuration_missing {
            return self.handle_failure(
                CheckMode::Manual,
                update_configuration_missing_error(),
                self.fallback_status(),
                false,
            );
        }
        if self.lock().downloaded_version.is_some() {
            return self.response(true, false, Some(true), None);
        }

        let now = self.now();
        let mut preferences = self.read_preferences();
        preferences.last_checked_at = Some(crate::commands::project::iso8601_utc_from_millis(now));
        self.write_preferences(&preferences);
        let service = self;
        self.enqueue_check(
            move || async move { service.perform_check(CheckMode::Manual, now).await },
        )
        .await
    }

    /// 对齐基线 `enqueueCheck`：串行化检查/下载，且**不**因前一个任务失败而拒绝后续
    async fn enqueue_check<F, Fut>(&self, operation: F) -> UpdateCheckResponse
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = UpdateCheckResponse>,
    {
        let _guard = self.check_queue.lock().await;
        operation().await
    }

    /// 对齐基线 `performCheck`
    async fn perform_check(&self, mode: CheckMode, now: u64) -> UpdateCheckResponse {
        // 队列内复检：先前的检查可能已启动或完成下载
        {
            let inner = self.lock();
            if inner.state.status == UpdateStatus::Downloading || inner.downloaded_version.is_some()
            {
                drop(inner);
                return self.response(true, false, Some(true), None);
            }
        }
        let mut next = self.get_state();
        next.status = UpdateStatus::Checking;
        next.last_checked_at = Some(crate::commands::project::iso8601_utc_from_millis(now));
        next.error = None;
        self.set_state(next);

        let result = match self.options.updater.check_for_updates().await {
            Ok(result) => result,
            Err(error) => {
                let classified = crate::update::backend::classify_update_failure(
                    UpdateErrorPhase::Check,
                    &error,
                );
                let fallback = self.fallback_status();
                return self.handle_failure(mode, classified, fallback, true);
            }
        };

        let update = result.and_then(|item| item.update_info);
        let is_newer = update
            .as_ref()
            .map(|item| is_higher_stable_version(&item.version, &self.options.current_version))
            .unwrap_or(false);
        if !is_newer {
            {
                let mut inner = self.lock();
                inner.checked_available_version = None;
            }
            self.forget_available_update();
            let mut next = self.get_state();
            next.status = UpdateStatus::NotAvailable;
            next.available_version = None;
            next.update_action = None;
            next.release_name = None;
            next.release_notes = None;
            next.release_date = None;
            next.download_progress = None;
            next.reminder_until = None;
            next.is_reminder_deferred = false;
            self.set_state(next);
            return self.response(true, true, None, None);
        }

        let update = update.expect("is_newer 蕴含 update 存在");
        {
            let mut inner = self.lock();
            inner.checked_available_version = Some(update.version.clone());
        }
        self.remember_available_update(&update);

        let (reminder_until, is_reminder_deferred) = {
            let inner = self.lock();
            match (&inner.reminder, update.version.as_str()) {
                (Some(reminder), version) => {
                    Self::reminder_state_for(reminder, version, self.now())
                }
                _ => (None, false),
            }
        };
        let mut next = self.get_state();
        next.status = UpdateStatus::Available;
        next.available_version = Some(update.version.clone());
        next.update_action = Some(self.options.update_action);
        next.release_name = update.release_name.clone();
        next.release_notes = update.release_notes.clone();
        next.release_date = update.release_date.clone();
        next.reminder_until = reminder_until;
        next.is_reminder_deferred = is_reminder_deferred;
        next.download_progress = None;
        self.set_state(next);
        self.response(true, true, Some(true), None)
    }

    /// 对齐基线 `downloadUpdate`（Windows 就地更新路径；本批 `updateAction` 恒为
    /// `open-release`，故实际总是诚实返回 `DOWNLOAD_NOT_READY`，B14 落地后启用）
    pub async fn download_update(&self) -> UpdateActionResponse {
        let (status, update_action, available_version, downloaded) = {
            let inner = self.lock();
            (
                inner.state.status,
                inner.state.update_action,
                inner.state.available_version.clone(),
                inner.downloaded_version.clone(),
            )
        };
        if !self.options.is_packaged
            || self.options.update_configuration_missing
            || update_action != Some(UpdateAction::Download)
            || available_version.is_none()
        {
            return self.action_response(false, Some(download_not_ready_error()));
        }
        let available_version = available_version.expect("上方已判空");
        if downloaded.as_deref() == Some(available_version.as_str()) {
            return self.action_response(true, None);
        }
        if status != UpdateStatus::Available {
            return self.action_response(false, Some(download_not_ready_error()));
        }

        let displayed_version = available_version;
        let needs_recheck =
            self.lock().checked_available_version.as_deref() != Some(displayed_version.as_str());
        if needs_recheck {
            let mut next = self.get_state();
            next.status = UpdateStatus::Checking;
            next.error = None;
            self.set_state(next);
            let now = self.now();
            let service = self;
            let checked = self
                .enqueue_check(move || async move {
                    service.perform_check(CheckMode::Manual, now).await
                })
                .await;
            let checked_version = self.lock().checked_available_version.clone();
            let accept =
                checked.success && checked_version.as_deref() == Some(displayed_version.as_str());
            let acceptable_superseding = checked_version
                .as_deref()
                .map(|version| is_higher_stable_version(version, &displayed_version))
                .unwrap_or(false);
            if !accept && !acceptable_superseding {
                return self.action_response(
                    false,
                    Some(checked.error.unwrap_or_else(download_not_ready_error)),
                );
            }
        }

        let (state_status, state_version) = {
            let inner = self.lock();
            (inner.state.status, inner.state.available_version.clone())
        };
        if state_status != UpdateStatus::Available || state_version.is_none() {
            return self.action_response(false, Some(download_not_ready_error()));
        }
        let version = state_version.expect("上方已判空");
        let mut next = self.get_state();
        next.status = UpdateStatus::Downloading;
        next.download_progress = None;
        next.error = None;
        self.set_state(next);

        if let Err(error) = self.options.updater.download_update().await {
            let failure =
                crate::update::backend::classify_update_failure(UpdateErrorPhase::Download, &error);
            let mut next = self.get_state();
            next.status = UpdateStatus::Available;
            next.error = None;
            self.set_state(next);
            return self.action_response(false, Some(failure));
        }
        {
            let mut inner = self.lock();
            inner.downloaded_version = Some(version);
        }
        let mut next = self.get_state();
        next.status = UpdateStatus::Downloaded;
        self.set_state(next);
        self.action_response(true, None)
    }

    /// 对齐基线 `openRelease`（macOS/本批全平台策略：打开固定的 Releases 页）
    pub async fn open_release(&self) -> UpdateActionResponse {
        let (update_action, available_version) = {
            let inner = self.lock();
            (
                inner.state.update_action,
                inner.state.available_version.clone(),
            )
        };
        if !self.options.is_packaged
            || update_action != Some(UpdateAction::OpenRelease)
            || available_version.is_none()
            || self.options.open_release.is_none()
        {
            return self.action_response(
                false,
                Some(make_update_error(
                    UpdateErrorCode::OpenReleaseFailed,
                    UpdateErrorPhase::Navigation,
                    UpdateErrorReason::NotReady,
                    false,
                    SafeUpdateTechnicalDetails::OpenReleaseFailed,
                )),
            );
        }
        let open_release = self.options.open_release.clone().expect("上方已判空");
        match (open_release)().await {
            Ok(()) => self.action_response(true, None),
            Err(_) => self.action_response(
                false,
                Some(make_update_error(
                    UpdateErrorCode::OpenReleaseFailed,
                    UpdateErrorPhase::Navigation,
                    UpdateErrorReason::OpenReleaseFailed,
                    true,
                    SafeUpdateTechnicalDetails::OpenReleaseFailed,
                )),
            ),
        }
    }

    /// 对齐基线 `deferReminder`
    pub async fn defer_reminder(&self, days: u32) -> UpdateActionResponse {
        if !self.options.is_packaged {
            return self.action_response(false, Some(updates_disabled_error()));
        }
        if days != 7 && days != 30 {
            return self.action_response(
                false,
                Some(make_update_error(
                    UpdateErrorCode::InvalidReminderDelay,
                    UpdateErrorPhase::Reminder,
                    UpdateErrorReason::InvalidReminderDelay,
                    false,
                    SafeUpdateTechnicalDetails::InvalidReminderDelay,
                )),
            );
        }
        let available_version = self.lock().state.available_version.clone();
        let Some(version) = available_version else {
            return self.action_response(
                false,
                Some(make_update_error(
                    UpdateErrorCode::ReminderNotAvailable,
                    UpdateErrorPhase::Reminder,
                    UpdateErrorReason::ReminderUnavailable,
                    false,
                    SafeUpdateTechnicalDetails::ReminderNotAvailable,
                )),
            );
        };

        let until = crate::commands::project::iso8601_utc_from_millis(
            self.now() + u64::from(days) * 24 * 60 * 60 * 1000,
        );
        let reminder = UpdateReminder {
            version: version.clone(),
            until: until.clone(),
        };
        let mut preferences = self.read_preferences();
        preferences.reminder = Some(reminder.clone());
        if !self.write_preferences(&preferences) {
            return self.action_response(
                false,
                Some(make_update_error(
                    UpdateErrorCode::ReminderSaveFailed,
                    UpdateErrorPhase::Reminder,
                    UpdateErrorReason::ReminderSaveFailed,
                    true,
                    SafeUpdateTechnicalDetails::ReminderSaveFailed,
                )),
            );
        }
        {
            let mut inner = self.lock();
            inner.reminder = Some(reminder);
        }
        let mut next = self.get_state();
        next.reminder_until = Some(until);
        next.is_reminder_deferred = true;
        self.set_state(next);
        self.action_response(true, None)
    }

    /// 对齐基线 `requestInstall`：只响应渲染进程明确发出的安装请求
    pub async fn request_install(&self) -> UpdateActionResponse {
        if !self.options.is_packaged {
            return self.action_response(false, Some(updates_disabled_error()));
        }
        let (downloaded, available) = {
            let inner = self.lock();
            (
                inner.downloaded_version.clone(),
                inner.state.available_version.clone(),
            )
        };
        let ready = match (&downloaded, &available) {
            (Some(downloaded), Some(available)) => downloaded == available,
            _ => false,
        };
        if !ready {
            return self.action_response(
                false,
                Some(make_update_error(
                    UpdateErrorCode::InstallNotReady,
                    UpdateErrorPhase::Install,
                    UpdateErrorReason::NotReady,
                    true,
                    SafeUpdateTechnicalDetails::InstallNotReady,
                )),
            );
        }
        match self.options.updater.quit_and_install() {
            Ok(()) => self.action_response(true, None),
            Err(_) => self.action_response(
                false,
                Some(make_update_error(
                    UpdateErrorCode::InstallFailed,
                    UpdateErrorPhase::Install,
                    UpdateErrorReason::InstallFailed,
                    true,
                    SafeUpdateTechnicalDetails::InstallFailed,
                )),
            ),
        }
    }

    fn remember_available_update(&self, update: &UpdateReleaseInfo) {
        let mut preferences = self.read_preferences();
        preferences.available_update = Some(update.clone());
        self.write_preferences(&preferences);
    }

    fn forget_available_update(&self) {
        let mut preferences = self.read_preferences();
        if preferences.available_update.is_none() {
            return;
        }
        preferences.available_update = None;
        self.write_preferences(&preferences);
    }
}

/// 后台下载进度（对齐基线 `bindUpdaterEvents` 的 `download-progress`）
///
/// 本批后端不产生进度事件，但保留入口以便接入 `tauri-plugin-updater` 后复用。
impl UpdateService {
    pub fn apply_download_progress(&self, progress: UpdateDownloadProgress) {
        let mut next = self.get_state();
        next.status = if next.status == UpdateStatus::Downloaded {
            UpdateStatus::Downloaded
        } else {
            UpdateStatus::Downloading
        };
        next.download_progress = Some(progress);
        self.set_state(next);
    }
}

/// 对齐基线 `UpdateService`（`AppState` 派生 `Debug` 需要的手写实现）
impl std::fmt::Debug for UpdateService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        formatter
            .debug_struct("UpdateService")
            .field("status", &inner.state.status)
            .field("current_version", &inner.state.current_version)
            .field("available_version", &inner.state.available_version)
            .finish_non_exhaustive()
    }
}

/// 检查模式（对齐基线 `'automatic' | 'manual'`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckMode {
    Automatic,
    Manual,
}

/// 基线的默认 `checkAutomatically` 启动延迟（Tauri 侧由调用方在 `setup` 中直接触发）
pub const STARTUP_AUTO_CHECK_DELAY: Duration = Duration::from_secs(0);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::backend::{BackendFuture, UpdateFetchError};
    use std::sync::Mutex as StdMutex;

    const NOW: u64 = 1_700_000_000_000; // 2023-11-14T22:13:20.000Z

    struct StubBackend {
        result: StdMutex<Result<Option<crate::update::types::UpdateCheckResult>, UpdateFetchError>>,
        downloads: StdMutex<usize>,
        fail_install: bool,
    }

    impl StubBackend {
        fn with_result(
            result: Result<Option<crate::update::types::UpdateCheckResult>, UpdateFetchError>,
        ) -> Arc<Self> {
            Arc::new(Self {
                result: StdMutex::new(result),
                downloads: StdMutex::new(0),
                fail_install: false,
            })
        }
    }

    impl UpdateBackend for StubBackend {
        fn check_for_updates(
            &self,
        ) -> BackendFuture<
            '_,
            Result<Option<crate::update::types::UpdateCheckResult>, UpdateFetchError>,
        > {
            let value = self.result.lock().unwrap().clone();
            Box::pin(async move { value })
        }

        fn download_update(&self) -> BackendFuture<'_, Result<Vec<String>, UpdateFetchError>> {
            *self.downloads.lock().unwrap() += 1;
            Box::pin(async { Ok(Vec::new()) })
        }

        fn quit_and_install(&self) -> Result<(), String> {
            if self.fail_install {
                Err("install failed".to_string())
            } else {
                Ok(())
            }
        }
    }

    struct MemoryPreferences {
        value: StdMutex<UpdatePreferences>,
        writable: bool,
        writes: StdMutex<usize>,
    }

    impl MemoryPreferences {
        fn new(writable: bool) -> Arc<Self> {
            Arc::new(Self {
                value: StdMutex::new(UpdatePreferences::default()),
                writable,
                writes: StdMutex::new(0),
            })
        }
    }

    impl UpdatePreferencesStore for MemoryPreferences {
        fn read(&self) -> UpdatePreferences {
            self.value.lock().unwrap().clone()
        }

        fn write(&self, preferences: &UpdatePreferences) -> bool {
            if !self.writable {
                return false;
            }
            *self.writes.lock().unwrap() += 1;
            *self.value.lock().unwrap() = preferences.clone();
            true
        }
    }

    fn release(version: &str) -> crate::update::types::UpdateCheckResult {
        crate::update::types::UpdateCheckResult {
            update_info: Some(UpdateReleaseInfo {
                version: version.to_string(),
                release_name: Some(format!("Release {version}")),
                release_notes: Some("notes".to_string()),
                release_date: Some("2026-10-01T00:00:00Z".to_string()),
            }),
        }
    }

    fn service(
        backend: Arc<dyn UpdateBackend>,
        preferences: Arc<dyn UpdatePreferencesStore>,
        is_packaged: bool,
        configuration_missing: bool,
        current_version: &str,
    ) -> UpdateService {
        UpdateService::new(UpdateServiceOptions {
            updater: backend,
            current_version: current_version.to_string(),
            is_packaged,
            update_configuration_missing: configuration_missing,
            update_action: UpdateAction::OpenRelease,
            open_release: None,
            preferences,
            now_millis: Some(Arc::new(|| NOW)),
            publish: None,
        })
    }

    #[tokio::test]
    async fn unpackaged_runtime_is_disabled_test() {
        let backend = StubBackend::with_result(Ok(Some(release("9.9.9"))));
        let prefs = MemoryPreferences::new(true);
        let service = service(backend, prefs, false, false, "1.1.0");

        assert_eq!(service.get_state().status, UpdateStatus::Disabled);
        let checked = service.check_manually().await;
        assert!(!checked.success);
        assert!(!checked.checked);
        assert_eq!(
            checked.error.as_ref().map(|error| error.code),
            Some(UpdateErrorCode::UpdatesDisabled)
        );
        assert_eq!(checked.state.status, UpdateStatus::Disabled);
    }

    #[tokio::test]
    async fn missing_configuration_reports_actionable_error_test() {
        let backend = StubBackend::with_result(Ok(Some(release("9.9.9"))));
        let prefs = MemoryPreferences::new(true);
        let service = service(backend, prefs, true, true, "1.1.0");

        let checked = service.check_manually().await;
        assert!(!checked.success);
        assert!(!checked.checked);
        assert_eq!(
            checked.error.as_ref().map(|error| error.code),
            Some(UpdateErrorCode::UpdateConfigurationMissing)
        );
        // 手动检查失败进入 error 状态（自动检查则只回退，不起错误）
        assert_eq!(checked.state.status, UpdateStatus::Error);
    }

    #[tokio::test]
    async fn available_update_flow_and_reminder_test() {
        let backend = StubBackend::with_result(Ok(Some(release("1.2.0"))));
        let prefs = MemoryPreferences::new(true);
        let service = service(backend, prefs.clone(), true, false, "1.1.0");

        let checked = service.check_manually().await;
        assert!(checked.success && checked.checked && checked.update_available);
        assert_eq!(checked.state.status, UpdateStatus::Available);
        assert_eq!(checked.state.available_version.as_deref(), Some("1.2.0"));
        assert_eq!(checked.state.update_action, Some(UpdateAction::OpenRelease));
        assert_eq!(checked.state.release_name.as_deref(), Some("Release 1.2.0"));
        // 可用更新被记住
        assert_eq!(
            prefs
                .read()
                .available_update
                .as_ref()
                .map(|item| item.version.as_str()),
            Some("1.2.0")
        );
        assert_eq!(
            prefs.read().last_checked_at.as_deref(),
            Some("2023-11-14T22:13:20.000Z")
        );

        // 延后提醒：仅 7/30
        let invalid = service.defer_reminder(3).await;
        assert_eq!(
            invalid.error.as_ref().map(|error| error.code),
            Some(UpdateErrorCode::InvalidReminderDelay)
        );
        let deferred = service.defer_reminder(7).await;
        assert!(deferred.success, "{:?}", deferred.error);
        assert!(deferred.state.is_reminder_deferred);
        let until = deferred.state.reminder_until.clone().unwrap();
        assert_eq!(
            parse_iso8601_millis(&until),
            Some(NOW as i64 + 7 * 86_400_000)
        );
        assert_eq!(
            prefs
                .read()
                .reminder
                .as_ref()
                .map(|item| item.until.as_str()),
            Some(until.as_str())
        );
    }

    #[tokio::test]
    async fn not_available_clears_state_and_preferences_test() {
        let backend = StubBackend::with_result(Ok(None));
        let prefs = MemoryPreferences::new(true);
        // 预置一个可用更新（应被清除）
        let mut initial = UpdatePreferences::default();
        initial.available_update = Some(UpdateReleaseInfo {
            version: "9.0.0".to_string(),
            release_name: None,
            release_notes: None,
            release_date: None,
        });
        *prefs.value.lock().unwrap() = initial;
        let service = service(backend, prefs.clone(), true, false, "1.1.0");

        // 构造时恢复「可继续准备」而不是伪造已下载
        assert_eq!(service.get_state().status, UpdateStatus::Available);
        let checked = service.check_manually().await;
        assert!(checked.success && checked.checked && !checked.update_available);
        assert_eq!(checked.state.status, UpdateStatus::NotAvailable);
        assert_eq!(checked.state.available_version, None);
        assert!(
            prefs.read().available_update.is_none(),
            "不可用更新必须从偏好中清除"
        );
    }

    #[tokio::test]
    async fn automatic_check_is_throttled_per_calendar_day_test() {
        let backend = StubBackend::with_result(Ok(Some(release("2.0.0"))));
        let prefs = MemoryPreferences::new(true);
        let service = service(backend, prefs.clone(), true, false, "1.1.0");

        let first = service.check_automatically().await;
        assert!(first.success && first.checked);
        assert_eq!(
            prefs.read().last_automatic_check_date.as_deref(),
            Some("2023-11-14")
        );

        // 同一天再次自动检查：跳过（checked=false），但仍算成功
        let second = service.check_automatically().await;
        assert!(second.success && !second.checked);
    }

    #[tokio::test]
    async fn failed_classification_reaches_manual_response_but_not_automatic_test() {
        let backend = StubBackend::with_result(Err(UpdateFetchError::new(
            "error trying to connect: dns error: failed to lookup address",
            None,
        )));
        let prefs = MemoryPreferences::new(true);
        let service = service(backend, prefs, true, false, "1.1.0");

        let automatic = service.check_automatically().await;
        assert!(!automatic.success);
        assert!(automatic.error.is_none(), "自动检查失败不向渲染层暴露错误");
        assert_eq!(automatic.state.status, UpdateStatus::Idle);
        assert!(automatic.state.error.is_none());

        let manual = service.check_manually().await;
        assert!(!manual.success && manual.checked);
        let error = manual.error.expect("手动检查必须返回分类错误");
        assert_eq!(error.code, UpdateErrorCode::CheckFailed);
        assert_eq!(error.reason, UpdateErrorReason::Network);
        assert!(error.retryable);
        assert_eq!(manual.state.status, UpdateStatus::Error);
    }

    #[tokio::test]
    async fn preference_write_failure_blocks_automatic_network_check_test() {
        let backend = StubBackend::with_result(Ok(Some(release("2.0.0"))));
        let prefs = MemoryPreferences::new(false); // 不可写（对齐「配置损坏」）
        let service = service(backend, prefs, true, false, "1.1.0");

        let checked = service.check_automatically().await;
        assert!(!checked.success);
        assert!(!checked.checked, "偏好不可写时不得自动联网");
        assert_eq!(checked.state.status, UpdateStatus::Idle);
    }

    #[tokio::test]
    async fn download_and_install_are_honestly_not_ready_test() {
        let backend = StubBackend::with_result(Ok(Some(release("1.2.0"))));
        let prefs = MemoryPreferences::new(true);
        let service = service(backend, prefs, true, false, "1.1.0");
        service.check_manually().await;

        // 本批 updateAction 恒为 open-release → 下载诚实返回 DOWNLOAD_NOT_READY
        let download = service.download_update().await;
        assert!(!download.success);
        assert_eq!(
            download.error.as_ref().map(|error| error.code),
            Some(UpdateErrorCode::DownloadNotReady)
        );
        // 未下载 → 安装同样诚实返回 INSTALL_NOT_READY
        let install = service.request_install().await;
        assert_eq!(
            install.error.as_ref().map(|error| error.code),
            Some(UpdateErrorCode::InstallNotReady)
        );
    }

    #[tokio::test]
    async fn open_release_succeeds_only_with_injected_closure_test() {
        let backend = StubBackend::with_result(Ok(Some(release("1.2.0"))));
        let prefs = MemoryPreferences::new(true);
        let opened = Arc::new(StdMutex::new(0usize));
        let flag = opened.clone();
        let service = UpdateService::new(UpdateServiceOptions {
            updater: backend,
            current_version: "1.1.0".to_string(),
            is_packaged: true,
            update_configuration_missing: false,
            update_action: UpdateAction::OpenRelease,
            open_release: Some(Arc::new(move || {
                let flag = flag.clone();
                Box::pin(async move {
                    *flag.lock().unwrap() += 1;
                    Ok(())
                })
            })),
            preferences: prefs,
            now_millis: Some(Arc::new(|| NOW)),
            publish: Some(Arc::new(|_| {})),
        });

        // 尚无可用更新 → not-ready
        let premature = service.open_release().await;
        assert_eq!(
            premature.error.as_ref().map(|error| error.code),
            Some(UpdateErrorCode::OpenReleaseFailed)
        );

        service.check_manually().await;
        let ok = service.open_release().await;
        assert!(ok.success, "{:?}", ok.error);
        assert_eq!(*opened.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn publish_receives_each_state_snapshot_test() {
        let backend = StubBackend::with_result(Ok(Some(release("1.2.0"))));
        let prefs = MemoryPreferences::new(true);
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let sink = seen.clone();
        let service = UpdateService::new(UpdateServiceOptions {
            updater: backend,
            current_version: "1.1.0".to_string(),
            is_packaged: true,
            update_configuration_missing: false,
            update_action: UpdateAction::OpenRelease,
            open_release: None,
            preferences: prefs,
            now_millis: Some(Arc::new(|| NOW)),
            publish: Some(Arc::new(move |state: &UpdateState| {
                sink.lock().unwrap().push(state.status);
            })),
        });

        service.check_manually().await;
        let statuses = seen.lock().unwrap().clone();
        assert!(
            statuses.contains(&UpdateStatus::Checking)
                && statuses.contains(&UpdateStatus::Available),
            "每次状态变更都必须发布快照：{statuses:?}"
        );
    }
}
