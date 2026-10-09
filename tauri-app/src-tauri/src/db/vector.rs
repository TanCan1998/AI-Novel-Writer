//! 纯 Rust 内存向量检索层（批次 F2 / 方案 B：自研混合检索的语义支路）
//!
//! 以 [`hnsw_rs`] 的 HNSW 图为底座，封装成可常驻 Tauri State 的文档级索引：
//!
//! - **并发安全闭环**：内部为 `Arc<tokio::sync::RwLock<VectorIndexCore>>`，
//!   以 `&self` 即可安全并发——检索走读锁，增量写入 / 图恢复走写锁；
//! - **双向 ID 映射**：`String` 业务 `doc_id` ↔ `usize` 图内 ID 透明转换，
//!   调用方永远只看到 `doc_id`，无需关心 HNSW 的 `usize` 主键模型；
//! - **内存墓碑**：逻辑删除（`delete`）与「更新即退休旧点」统一走
//!   [`HashSet`] 墓碑，检索拿到 `Neighbour` 列表后立即在内存中 `.filter()`
//!   拦截，绝不召回已退休节点；
//! - **二进制图快照**：[`LocalVectorIndex::file_dump`] / [`LocalVectorIndex::load_hnsw`]
//!   对接 `Hnsw::file_dump` 与 `HnswIo::load_hnsw`，极速落盘 / 恢复；并附带一份
//!   小而关键的 sidecar（`.idmap.json`）保存 ID 映射与墓碑，保证重启后
//!   删除语义不会「复活」。
//!
//! # 生命周期 `'static` 的成立依据（重要）
//!
//! `Hnsw<'a, T, D>` 带有生命周期参数，而常驻 State 要求字段为
//! `Hnsw<'static, f32, DistCosine>`。此处**不需要**对每个向量 `Box::leak`：
//! `hnsw_rs` 0.3.4 的 `insert` 在内部执行
//! `Point::new(data.to_vec(), origin_id, p_id)`（`hnsw.rs:508`），
//! 即把调用方切片**复制**进图自持的 `Vec<T>` 堆内存；图不会借用调用方的
//! 临时切片。因此 `Hnsw::new(...)` 产出的图天然满足 `'static`，插入临时
//! `&[f32]` 是安全且无额外泄漏的。
//!（反过来说，对每个向量 `Box::leak` 会造成随插入量线性增长的真实内存泄漏，
//! 与「减少内存占用」的迁移目标相悖，故不采用。）
//!
//! 唯一真正需要 `Box::leak` 的是 [`LocalVectorIndex::load_hnsw`] 中的
//! `HnswIo`：`HnswIo::load_hnsw` 的借用签名要求 `'a: 'b`，而「结构体同时持有
//! `HnswIo` 与其派生的 `Hnsw`」是自引用结构，安全 Rust 无法表达。由于默认
//! `ReloadOptions` **不做 mmap**（图与向量完全由 `Hnsw` 拥有），泄漏体量仅为
//! 元数据级，且本方法按**冷启动一次性加载**设计。
//!
//! # 存储路径红线（双栈隔离）
//!
//! `storage_dir` 由调用方提供，**必须是 Tauri 专属目录**，严禁复用 Electron
//! 基线的 `.vela/lancedb/`、`.vela/vectors.json` 等（见
//! `docs-fork/handoffs/2026-10-08-tauri-migration-status.md` 第二十六次快照 §2）。

use hnsw_rs::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 图内点 ID（HNSW 的 `usize` 主键）
pub type GraphIdx = usize;

/// 距离度量：余弦距离（`hnsw_rs` 输出 `1 - cos` 的距离值，越小越相似）
type Metric = DistCosine;

/// 每个节点的最大邻居边数（构造参数 `M`）
const DEFAULT_MAX_CONNECTION: usize = 16;
/// 最大层数
const DEFAULT_MAX_LAYER: usize = 16;
/// 构造期候选集大小
const DEFAULT_EF_CONSTRUCTION: usize = 200;
/// 检索期候选集大小下限（必要时按 `top_k` / 墓碑数量抬升）
const DEFAULT_SEARCH_EF: usize = 64;

/// 向量层错误
#[derive(Debug)]
pub enum VectorIndexError {
    /// 查询 / 插入向量维度与索引声明维度不一致
    DimensionMismatch { expected: usize, got: usize },
    /// 参数非法（空 `doc_id` / 空 basename / 维度为 0 等）
    InvalidArgument(String),
    /// 底层文件 IO 失败
    Io(std::io::Error),
    /// 图快照落盘或恢复失败
    Snapshot(String),
}

impl fmt::Display for VectorIndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionMismatch { expected, got } => {
                write!(f, "向量维度不匹配：索引期望 {expected} 维，实际 {got} 维")
            }
            Self::InvalidArgument(message) => write!(f, "向量索引参数非法：{message}"),
            Self::Io(error) => write!(f, "向量索引文件操作失败：{error}"),
            Self::Snapshot(message) => write!(f, "向量索引快照失败：{message}"),
        }
    }
}

impl std::error::Error for VectorIndexError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for VectorIndexError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// ID 映射 sidecar 的序列化载荷
///
/// 只保存「活节点」映射与墓碑集合；恢复时据此重建双向映射与墓碑。
#[derive(Debug, Serialize, Deserialize)]
struct IdMapSnapshot {
    /// 索引声明维度（恢复时用于一致性校验）
    dimension: usize,
    /// 下一个可分配的图内 ID（保证恢复后 ID 不重复）
    next_idx: GraphIdx,
    /// `doc_id -> 图内 ID`（仅活节点），排序后落盘保证字节稳定
    entries: Vec<(String, GraphIdx)>,
    /// 已退休（墓碑化）的图内 ID
    tombstones: Vec<GraphIdx>,
}

/// 被读写锁保护的向量索引内核
///
/// 所有字段都只在持有 [`LocalVectorIndex`] 的写锁时变更，读锁下只读。
struct VectorIndexCore {
    /// HNSW 图（`&self` 插入，内部自带锁；生命周期见模块文档）
    hnsw: Hnsw<'static, f32, Metric>,
    /// 活节点：业务 `doc_id` → 图内 ID
    doc_id_to_idx: HashMap<String, GraphIdx>,
    /// 活节点：图内 ID → 业务 `doc_id`
    idx_to_doc_id: HashMap<GraphIdx, String>,
    /// 内存墓碑：已退休（删除 / 被更新替换）的图内 ID，检索时过滤
    tombstones: HashSet<GraphIdx>,
    /// 单调递增的图内 ID 分配器
    next_idx: GraphIdx,
    /// 向量维度
    dimension: usize,
    /// 二进制快照存放目录（必须为 Tauri 专属目录）
    storage_dir: PathBuf,
}

/// 内存向量索引：面向业务 `doc_id` 的 HNSW 封装（可克隆，常驻 Tauri State）
///
/// 克隆仅复制 `Arc`，指向同一份索引内核；所有公开方法均为 `&self` 的异步方法。
#[derive(Clone)]
pub struct LocalVectorIndex {
    inner: Arc<RwLock<VectorIndexCore>>,
}

impl LocalVectorIndex {
    /// 创建空索引
    ///
    /// - `dimension`：向量维度（须与写入 / 查询向量一致，0 视为非法）
    /// - `max_elements`：HNSW 预分配容量提示（可略大于实际，不影响正确性）
    /// - `storage_dir`：快照目录，须为 Tauri 专属目录；不存在时自动创建
    pub fn new(
        dimension: usize,
        max_elements: usize,
        storage_dir: impl AsRef<Path>,
    ) -> Result<Self, VectorIndexError> {
        if dimension == 0 {
            return Err(VectorIndexError::InvalidArgument(
                "dimension 必须大于 0".to_string(),
            ));
        }

        let storage_dir = storage_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&storage_dir)?;

        // 精确对齐 hnsw_rs 0.3.4 的参数位置与语义：
        // (max_nb_connection = M, max_elements, max_layer, ef_construction, distance)
        let hnsw = Hnsw::new(
            DEFAULT_MAX_CONNECTION,
            max_elements,
            DEFAULT_MAX_LAYER,
            DEFAULT_EF_CONSTRUCTION,
            DistCosine {},
        );

        let core = VectorIndexCore {
            hnsw,
            doc_id_to_idx: HashMap::new(),
            idx_to_doc_id: HashMap::new(),
            tombstones: HashSet::new(),
            next_idx: 0,
            dimension,
            storage_dir,
        };

        Ok(Self {
            inner: Arc::new(RwLock::new(core)),
        })
    }

    /// 索引声明维度
    pub async fn dimension(&self) -> usize {
        self.inner.read().await.dimension
    }

    /// 活节点数量（已扣除墓碑）
    pub async fn len(&self) -> usize {
        let core = self.inner.read().await;
        core.hnsw
            .get_nb_point()
            .saturating_sub(core.tombstones.len())
    }

    /// 是否没有活节点
    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }

    /// 当前活节点的 `doc_id` 快照（读锁）
    ///
    /// 供「回填计划 / vectorless 计数」使用：调用方拿它与本项目 `kb_chunks` 求差集。
    /// 不含墓碑节点（墓碑只在检索时用于过滤）。
    pub async fn live_doc_ids(&self) -> Vec<String> {
        self.inner
            .read()
            .await
            .doc_id_to_idx
            .keys()
            .cloned()
            .collect()
    }

    /// 是否已登记该 `doc_id` 的向量（读锁）
    pub async fn contains(&self, doc_id: &str) -> bool {
        self.inner.read().await.doc_id_to_idx.contains_key(doc_id)
    }

    /// 登记向量（插入或更新）—— 写锁
    ///
    /// - 新 `doc_id`：分配新的图内 ID 后写入；
    /// - 已存在的 `doc_id`：把旧图内 ID 打入墓碑（HNSW 不支持原地更新），
    ///   再以新 ID 写入新向量，检索时旧点被过滤。
    ///
    /// 返回本次写入的图内 ID。
    ///
    /// # `'static` 说明
    /// 这里传入的 `vector: &[f32]` 是临时切片，但 `hnsw_rs::insert` 会在内部
    /// `Point::new(data.to_vec(), ...)` 把数据复制为图自持的堆 `Vec`，
    /// 图不持有对 `vector` 的引用，故无需 `Box::leak`（见模块文档）。
    pub async fn insert_vector(
        &self,
        doc_id: &str,
        vector: &[f32],
    ) -> Result<GraphIdx, VectorIndexError> {
        if doc_id.is_empty() {
            return Err(VectorIndexError::InvalidArgument(
                "doc_id 不能为空".to_string(),
            ));
        }

        let mut core = self.inner.write().await;
        core.check_dimension(vector)?;

        // 更新：退休旧点（保留墓碑以便检索过滤）
        if let Some(old_idx) = core.doc_id_to_idx.remove(doc_id) {
            core.idx_to_doc_id.remove(&old_idx);
            core.tombstones.insert(old_idx);
        }

        let idx = core.next_idx;
        core.next_idx += 1;
        core.doc_id_to_idx.insert(doc_id.to_string(), idx);
        core.idx_to_doc_id.insert(idx, doc_id.to_string());

        // 所有权转移：hnsw_rs 内部复制为自持 Vec（见方法文档）
        core.hnsw.insert((vector, idx));
        Ok(idx)
    }

    /// 逻辑删除：把 `doc_id` 对应节点打入内存墓碑 —— 写锁
    ///
    /// 返回是否命中活节点（重复删除 / 未知 `doc_id` 返回 `false`）。
    /// 映射表同步移除，保持「双向映射只描述活节点」的不变式。
    pub async fn delete_vector(&self, doc_id: &str) -> bool {
        let mut core = self.inner.write().await;
        match core.doc_id_to_idx.remove(doc_id) {
            Some(idx) => {
                core.idx_to_doc_id.remove(&idx);
                core.tombstones.insert(idx);
                true
            }
            None => false,
        }
    }

    /// RAG 语义检索（带墓碑 `.filter()` 拦截）—— 读锁
    ///
    /// 返回 `(doc_id, 余弦距离)`，按距离升序（越靠前越相似）。
    /// `top_k == 0` 或无活节点时返回空列表。
    pub async fn search_rag(
        &self,
        query: &[f32],
        top_k: usize,
    ) -> Result<Vec<(String, f32)>, VectorIndexError> {
        let core = self.inner.read().await;
        core.check_dimension(query)?;

        if top_k == 0 || core.hnsw.get_nb_point() <= core.tombstones.len() {
            return Ok(Vec::new());
        }

        // 墓碑会挤占召回名额，按墓碑数量放大候选，最后再截断到 top_k。
        let raw_points = core.hnsw.get_nb_point();
        let fetch = top_k
            .saturating_add(core.tombstones.len())
            .min(raw_points)
            .max(1);
        let ef = fetch.max(DEFAULT_SEARCH_EF);

        let neighbours = core.hnsw.search(query, fetch, ef);

        // 拿到 Neighbour 列表后，瞬间用 .filter() 在内存剔除墓碑节点。
        let results: Vec<(String, f32)> = neighbours
            .into_iter()
            .filter(|neighbour| !core.tombstones.contains(&neighbour.d_id))
            .filter_map(|neighbour| {
                core.idx_to_doc_id
                    .get(&neighbour.d_id)
                    .map(|doc_id| (doc_id.clone(), neighbour.distance))
            })
            .take(top_k)
            .collect();

        Ok(results)
    }

    /// 把 HNSW 图落盘为二进制快照（极速持久化）—— 读锁
    ///
    /// 对齐 `hnsw_rs` 0.3.4 契约：`file_dump(目录: &Path, 文件名前缀: &str)`，
    /// 生成 `<basename>.hnsw.graph` / `<basename>.hnsw.data`，另写本层
    /// `<basename>.idmap.json`（ID 映射 + 墓碑 sidecar）。
    ///
    /// **空图安全守卫**：`hnsw.get_nb_point() == 0` 时直接拦截返回 `Ok(())`，
    /// 绝不调用底层 `file_dump`，防止空图快照导致后续恢复崩溃。
    pub async fn file_dump(&self, basename: &str) -> Result<(), VectorIndexError> {
        if basename.is_empty() {
            return Err(VectorIndexError::InvalidArgument(
                "basename 不能为空".to_string(),
            ));
        }

        let core = self.inner.read().await;

        // 空图安全守卫：没有任何图内点时不落盘
        if core.hnsw.get_nb_point() == 0 {
            return Ok(());
        }

        core.hnsw
            .file_dump(core.storage_dir.as_path(), basename)
            .map_err(|error| VectorIndexError::Snapshot(format!("HNSW 图落盘失败：{error}")))?;

        core.write_id_map(basename)
    }

    /// 从二进制快照恢复索引（替换当前内存图）—— 写锁
    ///
    /// 先读 sidecar 重建双向映射与墓碑，再经 `HnswIo::load_hnsw` 载入图。
    /// 返回载入的图内点总数（含墓碑）。
    ///
    /// ⚠️ 该方法按冷启动一次性加载设计：`HnswIo` 被 `Box::leak` 为 `'static`
    ///（原因见模块文档），请勿在热路径反复调用。
    pub async fn load_hnsw(&self, basename: &str) -> Result<usize, VectorIndexError> {
        if basename.is_empty() {
            return Err(VectorIndexError::InvalidArgument(
                "basename 不能为空".to_string(),
            ));
        }

        let mut core = self.inner.write().await;

        let snapshot = core.read_id_map(basename)?;
        if snapshot.dimension != core.dimension {
            return Err(VectorIndexError::DimensionMismatch {
                expected: core.dimension,
                got: snapshot.dimension,
            });
        }

        // 默认 ReloadOptions 不做 mmap：图与向量数据由 Hnsw 完全拥有。
        // 为满足 `HnswIo::load_hnsw` 的 `'a: 'b` 自引用借用签名，将 io 泄漏为
        // 'static（冷启动一次，泄漏体量仅元数据级）。
        let hnsw_io: &'static mut HnswIo =
            Box::leak(Box::new(HnswIo::new(core.storage_dir.as_path(), basename)));
        let hnsw: Hnsw<'static, f32, Metric> = hnsw_io
            .load_hnsw()
            .map_err(|error| VectorIndexError::Snapshot(format!("HNSW 图恢复失败：{error}")))?;

        core.hnsw = hnsw;
        core.tombstones = snapshot.tombstones.into_iter().collect();
        core.doc_id_to_idx.clear();
        core.idx_to_doc_id.clear();
        for (doc_id, idx) in snapshot.entries {
            core.idx_to_doc_id.insert(idx, doc_id.clone());
            core.doc_id_to_idx.insert(doc_id, idx);
        }

        let max_live_idx = core.idx_to_doc_id.keys().copied().max().unwrap_or(0);
        let max_tomb_idx = core.tombstones.iter().copied().max().unwrap_or(0);
        core.next_idx = snapshot
            .next_idx
            .max(max_live_idx + 1)
            .max(max_tomb_idx + 1);

        Ok(core.hnsw.get_nb_point())
    }
}

impl VectorIndexCore {
    /// 校验向量维度
    fn check_dimension(&self, vector: &[f32]) -> Result<(), VectorIndexError> {
        if vector.len() != self.dimension {
            return Err(VectorIndexError::DimensionMismatch {
                expected: self.dimension,
                got: vector.len(),
            });
        }
        Ok(())
    }

    /// 写出 ID 映射 sidecar
    fn write_id_map(&self, basename: &str) -> Result<(), VectorIndexError> {
        let mut entries: Vec<(String, GraphIdx)> = self
            .doc_id_to_idx
            .iter()
            .map(|(doc_id, idx)| (doc_id.clone(), *idx))
            .collect();
        entries.sort();

        let mut tombstones: Vec<GraphIdx> = self.tombstones.iter().copied().collect();
        tombstones.sort_unstable();

        let snapshot = IdMapSnapshot {
            dimension: self.dimension,
            next_idx: self.next_idx,
            entries,
            tombstones,
        };
        let payload = serde_json::to_string(&snapshot)
            .map_err(|error| VectorIndexError::Snapshot(format!("ID 映射序列化失败：{error}")))?;
        std::fs::write(id_map_path(&self.storage_dir, basename), payload)?;
        Ok(())
    }

    /// 读取 ID 映射 sidecar
    fn read_id_map(&self, basename: &str) -> Result<IdMapSnapshot, VectorIndexError> {
        let path = id_map_path(&self.storage_dir, basename);
        let payload = std::fs::read_to_string(&path).map_err(|error| {
            VectorIndexError::Snapshot(format!("读取 ID 映射失败（{}）：{error}", path.display()))
        })?;
        serde_json::from_str(&payload)
            .map_err(|error| VectorIndexError::Snapshot(format!("ID 映射反序列化失败：{error}")))
    }
}

/// ID 映射 sidecar 文件路径：`<dir>/<basename>.idmap.json`
fn id_map_path(dir: &Path, basename: &str) -> PathBuf {
    dir.join(format!("{basename}.idmap.json"))
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个用例独立的临时目录（进程级 + 用例名，避免并行冲突）
    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lorekeeper-vector-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn index_is_send_sync_static_test() {
        // 编译期断言：必须能常驻 `tauri::State`（Send + Sync + 'static）
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<LocalVectorIndex>();
    }

    #[tokio::test]
    async fn insert_and_search_maps_doc_id_back_test() {
        let dir = temp_dir("insert-search");
        let index = LocalVectorIndex::new(4, 128, &dir).unwrap();

        index
            .insert_vector("doc-x", &[1.0, 0.0, 0.0, 0.0])
            .await
            .unwrap();
        index
            .insert_vector("doc-y", &[0.0, 1.0, 0.0, 0.0])
            .await
            .unwrap();
        index
            .insert_vector("doc-z", &[0.0, 0.9, 0.1, 0.0])
            .await
            .unwrap();

        let hits = index.search_rag(&[1.0, 0.0, 0.0, 0.0], 2).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, "doc-x", "最相似应映射回 doc_id");
        assert_eq!(index.len().await, 3);
        assert!(!index.is_empty().await);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn insert_same_doc_id_retires_old_point_test() {
        let dir = temp_dir("update");
        let index = LocalVectorIndex::new(4, 128, &dir).unwrap();

        let first = index
            .insert_vector("doc", &[1.0, 0.0, 0.0, 0.0])
            .await
            .unwrap();
        // 更新为另一个方向：旧点应进墓碑并分配新图内 ID
        let second = index
            .insert_vector("doc", &[0.0, 1.0, 0.0, 0.0])
            .await
            .unwrap();
        assert_ne!(first, second, "更新必须分配新的图内 ID");
        assert_eq!(index.len().await, 1, "活节点仅 1 个");

        let hits = index.search_rag(&[1.0, 0.0, 0.0, 0.0], 5).await.unwrap();
        // 真正的不变量是「已墓碑化的旧点绝不被召回」：
        // HNSW 是**近似**检索，且这里的查询向量恰等于旧点向量，取回的候选可能只包含
        // 旧点（过滤后为 0），甚至可能在取满 top_k 前就停（召回不足）——
        // 二者都是近似检索的可接受结果，但旧点绝不能出现在结果里。
        assert!(hits.len() <= 1, "旧点不得被召回：{hits:?}");
        assert!(
            hits.iter().all(|(doc_id, _)| doc_id == "doc"),
            "只允许返回活点 doc：{hits:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn delete_vector_filters_from_search_test() {
        let dir = temp_dir("delete");
        let index = LocalVectorIndex::new(4, 128, &dir).unwrap();

        index
            .insert_vector("keep", &[1.0, 0.0, 0.0, 0.0])
            .await
            .unwrap();
        index
            .insert_vector("drop", &[0.0, 1.0, 0.0, 0.0])
            .await
            .unwrap();

        assert!(index.delete_vector("drop").await);
        assert!(!index.delete_vector("drop").await, "重复删除应为未命中");
        assert!(!index.delete_vector("missing").await);
        assert_eq!(index.len().await, 1);

        let hits = index.search_rag(&[0.0, 1.0, 0.0, 0.0], 5).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "keep");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dimension_mismatch_is_rejected_test() {
        let dir = temp_dir("dimension");
        let index = LocalVectorIndex::new(4, 128, &dir).unwrap();

        let insert_err = index.insert_vector("doc", &[1.0, 0.0]).await.unwrap_err();
        assert!(matches!(
            insert_err,
            VectorIndexError::DimensionMismatch {
                expected: 4,
                got: 2
            }
        ));

        index
            .insert_vector("doc", &[1.0, 0.0, 0.0, 0.0])
            .await
            .unwrap();
        let search_err = index.search_rag(&[1.0, 0.0, 0.0], 1).await.unwrap_err();
        assert!(matches!(
            search_err,
            VectorIndexError::DimensionMismatch {
                expected: 4,
                got: 3
            }
        ));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn invalid_arguments_are_rejected_test() {
        let dir = temp_dir("invalid");
        assert!(LocalVectorIndex::new(0, 16, &dir).is_err());

        let index = LocalVectorIndex::new(4, 16, &dir).unwrap();
        assert!(index
            .insert_vector("", &[1.0, 0.0, 0.0, 0.0])
            .await
            .is_err());
        assert!(index.file_dump("").await.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn empty_graph_dump_is_guarded_test() {
        let dir = temp_dir("empty-dump");
        let index = LocalVectorIndex::new(4, 16, &dir).unwrap();

        // 空图守卫：返回 Ok(())，不落盘、不产生 sidecar
        index.file_dump("empty").await.unwrap();
        assert!(!id_map_path(&dir, "empty").exists());
        assert!(!dir.join("empty.hnsw.graph").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dump_and_load_roundtrip_keeps_mapping_and_tombstones_test() {
        let dir = temp_dir("roundtrip");
        {
            let index = LocalVectorIndex::new(4, 128, &dir).unwrap();
            index
                .insert_vector("doc-a", &[1.0, 0.0, 0.0, 0.0])
                .await
                .unwrap();
            index
                .insert_vector("doc-b", &[0.0, 1.0, 0.0, 0.0])
                .await
                .unwrap();
            assert!(index.delete_vector("doc-b").await);

            index.file_dump("kb").await.unwrap();
            assert!(dir.join("kb.hnsw.graph").exists());
            assert!(dir.join("kb.hnsw.data").exists());
            assert!(dir.join("kb.idmap.json").exists());
        }

        let restored = LocalVectorIndex::new(4, 128, &dir).unwrap();
        let loaded_points = restored.load_hnsw("kb").await.unwrap();
        assert_eq!(loaded_points, 2, "图内点总数含墓碑");
        assert_eq!(restored.len().await, 1, "墓碑随 sidecar 一并恢复");

        let hits = restored.search_rag(&[0.0, 1.0, 0.0, 0.0], 5).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "doc-a", "删除语义不得因重启复活");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn live_doc_ids_and_contains_reflect_active_nodes_only_test() {
        let dir = temp_dir("live-ids");
        let index = LocalVectorIndex::new(4, 64, &dir).unwrap();

        index
            .insert_vector("a", &[1.0, 0.0, 0.0, 0.0])
            .await
            .unwrap();
        index
            .insert_vector("b", &[0.0, 1.0, 0.0, 0.0])
            .await
            .unwrap();
        assert!(index.contains("a").await);
        assert!(!index.contains("missing").await);

        index.delete_vector("b").await;
        let mut ids = index.live_doc_ids().await;
        ids.sort();
        assert_eq!(ids, vec!["a"]);
        assert!(!index.contains("b").await, "墓碑节点不计入活集合");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn load_without_snapshot_is_rejected_test() {
        let dir = temp_dir("missing-snapshot");
        let index = LocalVectorIndex::new(4, 64, &dir).unwrap();
        assert!(index.load_hnsw("nope").await.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
