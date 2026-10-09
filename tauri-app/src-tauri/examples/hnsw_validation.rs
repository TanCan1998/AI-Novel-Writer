//! examples/hnsw_validation.rs
//! 最终修复结论：全量补齐截断的类型与测试向量数据
//! 运行命令：cargo run --example hnsw_validation

use hnsw_rs::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

// 1. 定义距离度量（这里使用余弦相似度 Cosine）
type Metric = DistCosine;

// 2. 内存层封装的知识库向量索引管理器（显式标注生命周期 'a）
pub struct LocalVectorIndex<'a> {
    pub hnsw: Hnsw<'a, f32, Metric>,
    pub tombstones: HashSet<usize>,
    pub storage_dir: String,
}

impl<'a> LocalVectorIndex<'a> {
    pub fn new(dim: usize, max_elements: usize, storage_dir: &str) -> Self {
        // 精确对齐 0.3.4 参数顺序：每个节点最大边数 M=16，最大容量，最大层数=16，ef_construction=200
        let hnsw = Hnsw::new(16, max_elements, 16, 200, DistCosine {});
        let _ = dim; // 标记使用变量
        Self {
            hnsw,
            tombstones: HashSet::new(),
            storage_dir: storage_dir.to_string(),
        }
    }

    /// 增量插入
    pub fn insert_vector(&mut self, id: usize, vector: &[f32]) {
        self.tombstones.remove(&id);
        self.hnsw.insert((vector, id));
    }

    /// 逻辑删除（实现墓碑机制）
    pub fn delete_vector(&mut self, id: usize) {
        self.tombstones.insert(id);
    }

    /// 满血 RAG 语义搜索（带墓碑过滤）
    pub fn search_rag(&self, query: &[f32], top_k: usize) -> Vec<(usize, f32)> {
        let knn_results = self.hnsw.search(query, top_k, 64);

        knn_results
            .into_iter()
            .filter(|neighbour| !self.tombstones.contains(&neighbour.d_id))
            .map(|neighbour| (neighbour.d_id, neighbour.distance))
            .collect()
    }

    /// 极速持久化 Dump（已对齐 get_nb_point 与多参 file_dump）
    pub fn dump_to_disk(&self, prefix: &str) -> std::io::Result<()> {
        let path = Path::new(&self.storage_dir);
        if !path.exists() {
            fs::create_dir_all(path)?;
        }

        if self.hnsw.get_nb_point() == 0 {
            return Ok(());
        }

        self.hnsw.file_dump(path, prefix).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, format!("HNSW Dump 失败: {}", e))
        })?;

        println!(
            "✅ 极速持久化完成！已在 {} 目录下生成二进制图快照：{}.hnsw.graph/.data",
            self.storage_dir, prefix
        );
        Ok(())
    }
}

fn main() {
    println!("=== 开始验证全新的 Hnsw 生命周期与签名绑定 ===");
    let mock_dir = "./target/mock_hnsw_db";
    let dimension = 4;

    let mut index_manager = LocalVectorIndex::new(dimension, 1000, mock_dir);

    // 补齐模拟高维向量数据
    let vec_doc_0 = vec![1.0, 0.0, 0.0, 0.0];
    let vec_doc_1 = vec![0.0, 1.0, 0.0, 0.0];
    let vec_doc_2 = vec![0.0, 0.9, 0.1, 0.0];

    index_manager.insert_vector(100, &vec_doc_0);
    index_manager.insert_vector(200, &vec_doc_1);
    index_manager.insert_vector(300, &vec_doc_2);

    // 验证检索
    let query_vector = vec![1.0, 0.0, 0.0, 0.0];
    println!("\n[测试 1] 正在执行语义检索...");
    let matches = index_manager.search_rag(&query_vector, 2);
    for (id, dist) in &matches {
        println!(" -> 匹配 ID: {}, 距离评分: {}", id, dist);
    }
    assert_eq!(matches[0].0, 100, "最高匹配度应该是文档 100");

    // 验证逻辑删除
    println!("\n[测试 2] 执行物理隔离（加入内存墓碑表）文档 100...");
    index_manager.delete_vector(100);

    let matches_after_delete = index_manager.search_rag(&query_vector, 2);
    println!(" -> 过滤后剩余有效结果数: {}", matches_after_delete.len());
    for (id, dist) in &matches_after_delete {
        println!(" -> 有效匹配 ID: {}, 距离评分: {}", id, dist);
    }
    if !matches_after_delete.is_empty() {
        assert_ne!(matches_after_delete[0].0, 100, "被墓碑截断的文档不应召回");
    }

    // 验证持久化 Dump 契约
    println!("\n[测试 3] 测试修补后的多参 file_dump 落盘性能...");
    index_manager.dump_to_disk("kb_batch_f2").unwrap();

    println!("\n🎉 结论：修复版全部通过验证！生命周期契约与多参持久化契约完美闭环！");

    // 清理沙盒测试文件
    let _ = fs::remove_dir_all(mock_dir);
}
