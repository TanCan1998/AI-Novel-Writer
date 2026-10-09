//! 知识库子层（批次 F2 / 方案 B：自研混合检索）。
//!
//! 两条检索支路在此分层实现，命令层（`commands/kb.rs`）只做编排：
//!
//! - [`chunks`]：文本分块（移植基线 `electron/embedding.ts::chunkText`）；
//! - [`fts`]：FTS5 预分词关键检索（`jieba` 切词 + `unicode61` 虚拟表）；
//! - [`hybrid`]：混合编排（默认忠实对齐基线二选一短路；RRF 融合为可选开关）+ 嵌入空间注册表 + 回填计划。
//!
//! 语义支路（HNSW 向量）在 [`crate::db::vector`] 的 `LocalVectorIndex`。
//!
//! 存储约定（L3 后）：项目库 `<project>/.lore/lorekeeper.db`，
//! 向量快照 `<project>/.lore/kb/`。**绝不触碰**基线的 `.vela/lancedb/` 与 `.vela/vectors.json`。

pub mod chunks;
pub mod fts;
pub mod hybrid;
pub mod store;
pub mod vectors;
