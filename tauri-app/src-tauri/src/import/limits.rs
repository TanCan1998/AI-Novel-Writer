//! 导入资源上限 —— 对齐 `src/shared/import-limits.ts`（G1）。
//!
//! 数值必须与基线逐字一致：Rust 侧不引入可配置项，上限直接用常量，
//! 避免「Tauri 版悄悄放宽基线安全边界」。基线另有 `ImportResourceLimits`
//! 的可覆盖构造（仅供测试注入），迁移侧不需要该通道。

/// 拆分后的章节数上限（对齐 `MAX_IMPORT_CHAPTERS`）
pub const MAX_IMPORT_CHAPTERS: usize = 5_000;
/// 单次选择的来源文件数上限（对齐 `MAX_IMPORT_SOURCE_FILES`）
pub const MAX_IMPORT_SOURCE_FILES: usize = 5_000;
/// 单次导入正文总字节上限（对齐 `MAX_IMPORT_TOTAL_BYTES` = 128 MiB）
pub const MAX_IMPORT_TOTAL_BYTES: usize = 128 * 1024 * 1024;
/// 单章正文字节上限（对齐 `import-inspection-store.ts` 的 `MAX_IMPORT_CHAPTER_BYTES` = 16 MiB）
pub const MAX_IMPORT_CHAPTER_BYTES: usize = 16 * 1024 * 1024;
/// 待处理检视的存活时长（对齐 `IMPORT_INSPECTION_TTL_MS` = 10 分钟）
pub const IMPORT_INSPECTION_TTL_MS: u64 = 10 * 60 * 1_000;
/// 并发待处理检视上限（对齐 `ImportInspectionStore` 默认 `maxActive`）。
///
/// ⚠️ G1 的 D2 决策移除了 `webContentsId` 归属维度（Tauri 单窗口单逻辑会话），
/// 因此「他人活跃检视」恒为空集合，该上限在当前结构下**结构不可达**；
/// 保留常量与校验作为将来多窗口扩展的护栏，并维持与基线的失败文案口径。
pub const MAX_ACTIVE_INSPECTIONS: usize = 2;
