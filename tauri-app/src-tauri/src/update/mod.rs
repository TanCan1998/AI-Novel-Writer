//! 更新域（批次 H / H3）—— 平移自 `electron/services/update-*.ts` 与
//! `electron/controllers/update-controller.ts`。
//!
//! 模块划分（与基线文件一一对应）：
//!
//! | 基线 | 本模块 |
//! |---|---|
//! | `src/shared/update-types.ts` | [`types`] |
//! | `update-service.ts` 的版本比较 | [`version`] |
//! | `update-service.ts` 的状态机 | [`service`] |
//! | `github-release-update-backend.ts` + 错误分类器 | [`backend`] |
//! | `update-preferences-store.ts` | [`preferences`] |
//! | `update-runtime.ts` | [`runtime`] |
//! | `update-startup.ts`（装配与降级） | [`startup`] |
//! | `update-controller.ts`（6 频道 + 事件） | `commands/update.rs` |
//!
//! **本批刻意偏离基线**（用户已确认，理由见
//! `docs-fork/research/2026-10-09-h3-h4-dependency-evaluation.md`）：
//! 1. 不引入 `tauri-plugin-updater`：后端恒为 GitHub-Release 元数据（只读）；
//! 2. `updateAction` 全平台为 `open-release`（基线 Windows 已安装包为 `download`），
//!    因此 `update:download` / `update:quit-and-install` 诚实返回 `DOWNLOAD_NOT_READY` /
//!    `INSTALL_NOT_READY`；
//! 3. 更新源指向 fork `TanCan1998/Lorekeeper`（基线指向上游）；
//! 4. 无 `app-update.yml` 等价物 → `updateConfiguration` 恒为 `available`（我们确实有可用的
//!    GitHub-Release 配置）；
//! 5. 日历日节流使用 UTC 日历日（基线为本地时区），仅边界偏移差异。

pub mod backend;
pub mod preferences;
pub mod runtime;
pub mod service;
pub mod startup;
pub mod time;
pub mod types;
pub mod version;

pub use backend::{
    GithubReleaseBackend, UpdateBackend, GITHUB_LATEST_RELEASE_API, GITHUB_LATEST_RELEASE_PAGE,
};
pub use preferences::{ConfigUpdatePreferencesStore, UpdatePreferencesStore};
pub use service::{CheckMode, UpdateService, UpdateServiceOptions};
pub use types::*;
pub use version::is_higher_stable_version;
