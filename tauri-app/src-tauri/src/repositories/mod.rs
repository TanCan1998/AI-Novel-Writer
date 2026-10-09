//! 仓储层 —— 一域一仓，平移自 `electron/repositories/*`。
//!
//! 约定：仓储只接收 `&Connection`，不持有全局状态；命令层负责会话门禁与错误形状。

pub mod blueprint_repository;
pub mod chapter_deletion_repository;
pub mod character_repository;
pub mod character_roster_repository;
pub mod consistency_exemption_repository;
pub mod content_repository;
pub mod draft_repository;
pub mod finalization_repository;
pub mod finalized_continuity_repository;
pub mod finalized_draft_import_repository;
pub mod llm_repository;
pub mod narrative_thread_repository;
pub mod plot_tree_repository;
pub mod post_process_repository;
pub mod project_clear_repository;
pub mod project_core_repository;
pub mod recovery_candidate_repository;
pub mod review_repository;
pub mod revision_repository;
