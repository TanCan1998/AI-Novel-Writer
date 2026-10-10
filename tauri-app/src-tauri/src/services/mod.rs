//! services —— 领域服务的纯 Rust 逻辑层（批次 B5 起）。
//!
//! 首批成员：`avatar_image`（自 `electron/services/avatar-image.ts` 移植的头像压缩）。
//! 约定：纯逻辑、无 Tauri 依赖，由 commands 层按需调用。

pub mod avatar_image;
