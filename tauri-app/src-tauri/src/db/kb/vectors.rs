//! 知识库向量索引管理器（批次 F2-3）。
//!
//! 把 [`crate::db::vector::LocalVectorIndex`] 按「项目根 + 嵌入代际」懒加载并常驻
//! `AppState`。索引本身是内存态（HNSW 图 + ID 映射 + 墓碑），持久化依赖
//! `LocalVectorIndex::file_dump` 落在 **Tauri 专属** 目录 `<project>/.lore/kb/`。
//!
//! # 代际隔离（对齐基线嵌入空间注册表）
//!
//! 每个 `generation` 对应一份独立快照 `index-<generation>.hnsw.*`；检索只使用
//! `kb_embedding_spaces` 中 `status = 'active'` 的代际。换模型时新代际以
//! `building` 起步，显式回填完成后才切换 active（基线语义）。
//!
//! # 存储红线（双栈隔离）
//!
//! 快照目录固定 `<project>/.lore/kb/`，**绝不触碰** 基线的 `.vela/lancedb/`、
//! `.vela/vectors.json`（见 `docs-fork/handoffs/2026-10-09-tauri-migration-status.md`）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::db::vector::LocalVectorIndex;

/// HNSW 容量提示（可略大于实际；知识库分块规模）
pub const DEFAULT_KB_MAX_ELEMENTS: usize = 100_000;

/// 项目根 → 规范化键（大小写折叠，避免同一项目多种写法重复建索引）
pub fn project_key(project_root: &str) -> String {
    crate::security::normalized_project_path(project_root)
}

/// 索引注册键：`<normRoot>#<generation>`
pub fn index_key(project_root: &str, generation: i64) -> String {
    format!("{}#{generation}", project_key(project_root))
}

/// 向量快照目录：`<project>/.lore/kb/`
pub fn snapshot_dir(project_root: &str) -> PathBuf {
    Path::new(project_root)
        .join(crate::db::PROJECT_DIR_NAME)
        .join("kb")
}

/// 快照 basename：`index-<generation>`
pub fn snapshot_basename(generation: i64) -> String {
    format!("index-{generation}")
}

/// 索引管理器（常驻 `AppState`；克隆索引仅复制 `Arc`）
#[derive(Default)]
pub struct KbVectorManager {
    indexes: HashMap<String, LocalVectorIndex>,
}

impl std::fmt::Debug for KbVectorManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KbVectorManager")
            .field("loaded_indexes", &self.indexes.len())
            .finish()
    }
}

impl KbVectorManager {
    /// 取出已加载索引（未加载返回 `None`）
    pub fn get(&self, key: &str) -> Option<LocalVectorIndex> {
        self.indexes.get(key).cloned()
    }

    /// 插入并返回既有/新建索引（并发下优先复用已有实例）
    pub fn insert_if_absent(&mut self, key: String, index: LocalVectorIndex) -> LocalVectorIndex {
        self.indexes.entry(key).or_insert(index).clone()
    }

    /// 移除某项目的全部代际索引（关闭项目 / 清空知识库时）
    pub fn remove_project(&mut self, project_root: &str) {
        let prefix = format!("{}#", project_key(project_root));
        self.indexes.retain(|key, _| !key.starts_with(&prefix));
    }

    /// 已加载索引数（测试/诊断用）
    pub fn len(&self) -> usize {
        self.indexes.len()
    }

    /// 是否未加载任何索引
    pub fn is_empty(&self) -> bool {
        self.indexes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_key_folds_case_and_separators_test() {
        assert_eq!(
            project_key("F:\\Workplace\\Novel"),
            project_key("f:/workplace/novel")
        );
    }

    #[test]
    fn index_key_and_snapshot_paths_are_project_scoped_test() {
        let key = index_key("F:/Novel", 2);
        assert!(key.ends_with("#2"));
        assert_eq!(
            snapshot_dir("F:/Novel"),
            PathBuf::from("F:/Novel").join(".lore").join("kb")
        );
        assert_eq!(snapshot_basename(2), "index-2");
    }

    #[tokio::test]
    async fn manager_get_and_remove_project_test() {
        let dir = std::env::temp_dir().join(format!("kb-vec-mgr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut manager = KbVectorManager::default();
        let index = LocalVectorIndex::new(4, 32, &dir).unwrap();
        let key = index_key("F:/Novel", 1);
        assert!(manager.get(&key).is_none());
        manager.insert_if_absent(key.clone(), index.clone());
        assert!(manager.get(&key).is_some());

        manager.remove_project("F:/Novel");
        assert!(manager.get(&key).is_none());
        assert!(manager.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
