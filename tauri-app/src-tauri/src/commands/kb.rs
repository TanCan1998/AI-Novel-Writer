//! 批次 F2-3：知识库命令（`kb:*` 15 频道 + `dialog:select-knowledge-*` 2 频道）。
//!
//! 平移自 `electron/controllers/kb-controller.ts` + `electron/knowledge-base.ts`
//! 的存储/检索编排；底层存储从 LanceDB 换为 **SQLite（`db/kb/store.rs`）+
//! HNSW（`db/vector.rs`）**（方案 B，见批次 F2 决策文档）。
//!
//! | 频道 | 命令 | 说明 |
//! |---|---|---|
//! | `kb:import-document` | [`kb_import_document`] | 授权文件 → 导入 |
//! | `kb:import-folder` | [`kb_import_folder`] | 授权目录递归导入 |
//! | `kb:import-text` | [`kb_import_text`] | 文本导入（可选向量） |
//! | `kb:import-planning-text` | [`kb_import_planning_text`] | 文本导入（强制 FTS-only） |
//! | `kb:import-reference-text` | [`kb_import_reference_text`] | **诚实化占位**（依赖批次 G） |
//! | `kb:search` | [`kb_search`] | 有向量配置走向量短路，否则文本支路 |
//! | `kb:search-writing-context` | [`kb_search_writing_context`] | 上述 + 排除 `reference` |
//! | `kb:search-with-scope` | [`kb_search_with_scope`] | 上述 + 章节范围 |
//! | `kb:list-documents` | [`kb_list_documents`] | 文档列表 |
//! | `kb:remove-document` | [`kb_remove_document`] | 删除文档（含向量） |
//! | `kb:clear-all` | [`kb_clear_all`] | 清空知识库（含向量快照） |
//! | `kb:stats` | [`kb_stats`] | 统计 |
//! | `kb:get-vectorless-count` | [`kb_get_vectorless_count`] | 无向量块计数 |
//! | `kb:get-vector-rebuild-status` | [`kb_get_vector_rebuild_status`] | 纯本地状态读 |
//! | `kb:backfill-vectors` | [`kb_backfill_vectors`] | 回填向量 |
//! | `dialog:select-knowledge-files` | [`dialog_select_knowledge_files`] | 多文件授权 |
//! | `dialog:select-knowledge-folder` | [`dialog_select_knowledge_folder`] | 目录授权 |
//!
//! # 已记录的刻意差异（F2 决策）
//!
//! 1. 文本支路由基线 LanceDB `LIKE` 子串扫描改为 **FTS5 + jieba 预分词**，
//!    降级分支 `score` 由恒定 0.5 改为**真实词命中度**（用户 2026-10-09 决定）；
//! 2. 嵌入空间距离度量统一为 **cosine**（`LocalVectorIndex` 用 `DistCosine`），
//!    基线 LanceDB 侧为 `l2`；
//! 3. `kb:import-reference-text` 依赖批次 G 的导入运行权威校验，**先注册但返回显式失败**；
//! 4. 基线的 `vectors.json` 旧数据迁移（`LEGACY_VECTOR_MIGRATION_BLOCKED`）在双栈隔离后
//!    不存在 `.vela` 项目目录互通，故本层不做迁移 barrier。

use std::path::Path;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::app_paths;
use crate::commands::db::guard_read;
use crate::commands::project::file_path_to_string;
use crate::commands::SimpleResult;
use crate::db::kb::{chunks, hybrid, store, vectors};
use crate::external_grant::{GrantOperation, KNOWLEDGE_BASE_GRANT_TTL};
use crate::llm::chat::{build_client, proxy_from_config};
use crate::llm::embedding;
use crate::security::ProjectSessionContext;
use crate::state::AppState;

/// 基线默认 `topK`
const DEFAULT_TOP_K: i64 = 5;
/// 目录递归导入的最大文件数（对齐基线 `MAX_SECURE_KNOWLEDGE_IMPORT_FILES`）
const MAX_IMPORT_FILES: usize = 16_384;
/// 目录递归导入的最大深度（对齐基线 `MAX_SECURE_KNOWLEDGE_IMPORT_DEPTH`）
const MAX_IMPORT_DEPTH: usize = 64;
/// 主窗口标签
const MAIN_WINDOW_LABEL: &str = "main";
/// 对话框等待上限（安全网）
const DIALOG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

// ==================== 返回类型 ====================

/// `kb:import-*` 返回（对齐契约 `{ success, docId?, chunkCount?, error?, errorCode? }`）
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KbImportResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

impl KbImportResult {
    fn ok(doc_id: String, chunk_count: i64) -> Self {
        Self {
            success: true,
            doc_id: Some(doc_id),
            chunk_count: Some(chunk_count),
            error: None,
            error_code: None,
        }
    }

    fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            doc_id: None,
            chunk_count: None,
            error: Some(message.into()),
            error_code: None,
        }
    }

    fn failure_code(message: impl Into<String>, code: &str) -> Self {
        Self {
            success: false,
            doc_id: None,
            chunk_count: None,
            error: Some(message.into()),
            error_code: Some(code.to_string()),
        }
    }
}

/// `kb:import-folder` 返回
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KbFolderImportResult {
    pub success: bool,
    pub imported_count: i64,
    pub failed_files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

/// `kb:backfill-vectors` 返回
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KbBackfillResult {
    pub success: bool,
    pub processed: i64,
    pub failed: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

/// `dialog:select-knowledge-*` 返回（只含展示名与不透明 grantId）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalGrantDto {
    pub grant_id: String,
    pub display_name: String,
}

/// `AppResult` 失败形态（`{ success:false, errorCode, error? }`）
fn app_failure(code: &str, error: impl Into<String>) -> Value {
    json!({ "success": false, "errorCode": code, "error": error.into() })
}

// ==================== Embedding 配置 ====================

/// 一次嵌入调用所需的身份与参数（从全局 `models.json` / `config.json` 解析）
#[derive(Debug, Clone)]
struct EmbeddingSetup {
    protocol: String,
    model: Value,
    chunk_size: usize,
    chunk_overlap: usize,
    batch_size: Option<usize>,
    fingerprint: String,
}

fn read_str<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// 读取全局配置 + 模型档案，解析默认嵌入模型（对齐基线 `getEmbeddingConfig`）。
///
/// 目标模型 id = `defaultEmbeddingModelId`（非空）否则 `defaultModelId`（非空）。
fn embedding_setup() -> Option<EmbeddingSetup> {
    let config = crate::commands::config::read_global_config_at(&app_paths::global_config_path());
    let target = read_str(&config, "defaultEmbeddingModelId");
    let target = if target.trim().is_empty() {
        read_str(&config, "defaultModelId")
    } else {
        target
    };
    if target.trim().is_empty() {
        return None;
    }
    let models = crate::commands::llm::read_models_at(&app_paths::models_config_path());
    let model = models
        .into_iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(target))?;
    let protocol = {
        let raw = read_str(&model, "protocol");
        if raw.is_empty() {
            "openai".to_string()
        } else {
            raw.to_string()
        }
    };
    Some(build_setup(&protocol, model))
}

fn build_setup(protocol: &str, model: Value) -> EmbeddingSetup {
    let (chunk_size, chunk_overlap, batch_size) = normalize_embedding_options(&model);
    let fingerprint = model_fingerprint(protocol, &model);
    EmbeddingSetup {
        protocol: protocol.to_string(),
        model,
        chunk_size,
        chunk_overlap,
        batch_size,
        fingerprint,
    }
}

/// `usable`：baseUrl 与 apiKey 均非空（对齐基线 `hasUsableEmbeddingConfig`）
fn usable(setup: &EmbeddingSetup) -> bool {
    !read_str(&setup.model, "baseUrl").trim().is_empty()
        && !read_str(&setup.model, "apiKey").trim().is_empty()
}

/// 对齐基线 `embeddingSpaceFor`：只纳入可公开比较的模型身份，绝不写 API key。
fn model_fingerprint(protocol: &str, model: &Value) -> String {
    let base = read_str(model, "baseUrl").trim().trim_end_matches('/');
    let name = {
        let raw = read_str(model, "modelName").trim();
        if raw.is_empty() {
            "default"
        } else {
            raw
        }
    };
    format!("{protocol}|{base}|{name}")
}

/// 对齐 `normalizeEmbeddingOptions`（含上下限与截断）
fn normalize_embedding_options(model: &Value) -> (usize, usize, Option<usize>) {
    let options = model.get("embeddingOptions");
    let read = |key: &str, fallback: i64| -> i64 {
        options
            .and_then(|value| value.get(key))
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_f64().map(|number| number.trunc() as i64))
            })
            .unwrap_or(fallback)
    };
    let chunk_size = read("chunkSize", 500).clamp(100, 4000) as usize;
    let chunk_overlap = read("chunkOverlap", 50).clamp(0, chunk_size as i64 - 1) as usize;
    let batch_size = read("batchSize", 50).clamp(1, 50) as usize;
    (chunk_size, chunk_overlap, Some(batch_size))
}

/// 调用远程 Embedding（含代理装配）。返回 `f32` 向量以直接喂给 HNSW。
async fn embed_texts(
    setup: &EmbeddingSetup,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, embedding::EmbeddingError> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let config = crate::commands::config::read_global_config_at(&app_paths::global_config_path());
    let proxy = proxy_from_config(&config);
    let client = build_client(proxy.as_ref()).map_err(embedding::EmbeddingError::Plain)?;
    let vectors = embedding::generate_embeddings(
        &client,
        texts,
        &setup.protocol,
        &setup.model,
        setup.batch_size,
    )
    .await?;
    Ok(vectors
        .into_iter()
        .map(|vector| vector.into_iter().map(|value| value as f32).collect())
        .collect())
}

// ==================== 存储预检（最小移植）====================

/// Windows 原生路径上限（对齐基线 `WINDOWS_NATIVE_PATH_LIMIT`）
const WINDOWS_NATIVE_PATH_LIMIT: usize = 259;
/// Tauri KB 派生的最长存储叶子（`.lore/kb/` 下的 HNSW 快照）
const KB_STORAGE_RELATIVE_PATHS: [&str; 3] = [
    r".lore\kb\index-2147483647.hnsw.data",
    r".lore\kb\index-2147483647.hnsw.graph",
    r".lore\kb\index-2147483647.idmap.json",
];

/// 知识库存储路径预检（对齐基线 `assertKnowledgeBaseStoragePathSupported`）。
///
/// 仅 Windows 生效：项目根 + 最长派生叶子不得超出 `MAX_PATH`。
/// 返回 `Some(中文错误文案)` 表示不可用。
fn kb_storage_path_unsupported(project_root: &str) -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let longest = KB_STORAGE_RELATIVE_PATHS
        .iter()
        .map(|path| path.chars().count())
        .max()
        .unwrap_or(0);
    let max_project_root_length = WINDOWS_NATIVE_PATH_LIMIT - 1 - longest;
    if project_root.chars().count() + 1 + longest <= WINDOWS_NATIVE_PATH_LIMIT {
        return None;
    }
    Some(format!(
        "项目路径过深，知识库暂不可用，但项目内容仍可打开和导出。请将整个项目文件夹移动到更靠近磁盘根目录的位置（例如 D:\\Novels），并将完整项目路径控制在 {max_project_root_length} 个字符以内。"
    ))
}

/// 统一处理预检失败（返回对应失败信封）
fn preflight_or_error(project_root: &str) -> Result<(), String> {
    match kb_storage_path_unsupported(project_root) {
        Some(message) => Err(message),
        None => Ok(()),
    }
}

// ==================== 导入编排 ====================

/// 空间计划
enum SpacePlan {
    Use { generation: i64, dimension: i64 },
    ReindexRequired { current: String },
}

/// 选定/新建嵌入空间（对齐基线 `addChunks` 的空间兼容性检查）
fn plan_vector_space(
    conn: &rusqlite::Connection,
    fingerprint: &str,
    dimension: i64,
) -> Result<SpacePlan, String> {
    let spaces = hybrid::list_spaces(conn).map_err(|error| error.to_string())?;
    if let Some(space) = spaces
        .iter()
        .find(|space| space.model_fingerprint == fingerprint && space.dimension == dimension)
    {
        return Ok(SpacePlan::Use {
            generation: space.generation,
            dimension: space.dimension,
        });
    }
    if !spaces.is_empty() {
        let active = hybrid::active_space(conn).map_err(|error| error.to_string())?;
        let current = active
            .map(|space| format!("{} / {} 维", space.model_fingerprint, space.dimension))
            .unwrap_or_else(|| "当前全文知识库".to_string());
        return Ok(SpacePlan::ReindexRequired { current });
    }
    let generation = hybrid::next_generation(conn).map_err(|error| error.to_string())?;
    hybrid::upsert_space(
        conn,
        &hybrid::EmbeddingSpace {
            generation,
            dimension,
            model_fingerprint: fingerprint.to_string(),
            distance_metric: "cosine".to_string(),
            status: "building".to_string(),
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(SpacePlan::Use {
        generation,
        dimension,
    })
}

/// 是否允许增量写向量（对齐基线 `canWriteIncrementalVectors` 的语义核心）
fn can_write_incremental_vectors(conn: &rusqlite::Connection) -> Result<bool, String> {
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM kb_chunks", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if total == 0 {
        return Ok(true);
    }
    Ok(hybrid::active_space(conn)
        .map_err(|error| error.to_string())?
        .is_some())
}

/// 把向量写入 HNSW 索引并落盘（成功返回 `Ok(())`）
async fn write_vectors(
    state: &AppState,
    project_root: &str,
    generation: i64,
    dimension: i64,
    ids: &[String],
    vectors: &[Vec<f32>],
) -> Result<(), String> {
    let index = state
        .kb_vector_index(project_root, generation, dimension as usize)
        .await?;
    for (id, vector) in ids.iter().zip(vectors.iter()) {
        index
            .insert_vector(id, vector)
            .await
            .map_err(|error| error.to_string())?;
    }
    index
        .file_dump(&vectors::snapshot_basename(generation))
        .await
        .map_err(|error| error.to_string())
}

/// 从向量索引移除一批 chunk id（尽力而为；索引不存在时忽略）
pub(crate) async fn purge_vectors(
    state: &AppState,
    project_root: &str,
    generation: i64,
    dimension: i64,
    ids: &[String],
) {
    if ids.is_empty() {
        return;
    }
    if let Ok(index) = state
        .kb_vector_index(project_root, generation, dimension as usize)
        .await
    {
        for id in ids {
            let _ = index.delete_vector(id).await;
        }
        let _ = index
            .file_dump(&vectors::snapshot_basename(generation))
            .await;
    }
}

/// 导入文本的核心编排（`kb:import-text` / `kb:import-planning-text` /
/// `kb:import-document` / `kb:import-folder` 共用）。
#[allow(clippy::too_many_arguments)]
async fn import_text_core(
    state: &AppState,
    text: &str,
    file_name: &str,
    project_root: &str,
    session: Option<&ProjectSessionContext>,
    fts_only: bool,
    corpus_kind: &str,
    chapter_number: Option<i64>,
    chapter_title: Option<&str>,
    file_path: &str,
) -> Result<KbImportResult, String> {
    guard_read(state, project_root, session)?;
    if let Err(message) = preflight_or_error(project_root) {
        return Ok(KbImportResult::failure_code(
            message,
            "PROJECT_STORAGE_PATH_UNSUPPORTED",
        ));
    }
    if text.trim().is_empty() {
        return Ok(KbImportResult::failure("文本内容为空"));
    }

    let setup = embedding_setup();
    let (chunk_size, chunk_overlap) = match setup.as_ref() {
        Some(setup) => (setup.chunk_size, setup.chunk_overlap),
        None => (500, 50),
    };
    let text_chunks = chunks::chunk_text(text, chunk_size, chunk_overlap);
    if text_chunks.is_empty() {
        return Ok(KbImportResult::failure("文本块为空或格式无效"));
    }

    let want_vectors = !fts_only && setup.as_ref().map(usable).unwrap_or(false);

    // 同步读：既有同名文档 + 是否允许增量写向量
    let (existing_doc_id, can_incremental) = state.with_project_db(|conn| {
        Ok((
            store::find_document_id_by_file_name(conn, file_name)?,
            can_write_incremental_vectors(conn)?,
        ))
    })?;

    // 可选：生成向量（失败且非响应校验错误时降级为 FTS-only，对齐基线）
    let mut generated: Option<Vec<Vec<f32>>> = None;
    if want_vectors && can_incremental {
        let setup = setup.as_ref().expect("want_vectors 蕴含 setup 存在");
        match embed_texts(setup, &text_chunks).await {
            Ok(vectors) => generated = Some(vectors),
            Err(embedding::EmbeddingError::InvalidResponse { .. }) => {
                return Ok(KbImportResult::failure("Embedding 响应无效，导入已中止"));
            }
            Err(_) => {
                // 网络/其他错误：降级 FTS-only，不影响导入
                generated = None;
            }
        }
    }

    // 同步：空间计划 + 写文档
    let doc_id = crate::project_access::random_uuid_v4();
    let (chunk_ids, plan) = {
        let setup_fp = setup.as_ref().map(|setup| setup.fingerprint.clone());
        let generated_dim = generated.as_ref().map(|vectors| {
            vectors
                .first()
                .map(|vector| vector.len() as i64)
                .unwrap_or(0)
        });
        state.with_project_db(|conn| {
            let mut plan: Option<(i64, i64)> = None;
            if let (Some(vectors), Some(fingerprint), Some(dimension)) =
                (generated.as_ref(), setup_fp.as_ref(), generated_dim)
            {
                if !vectors.is_empty() {
                    match plan_vector_space(conn, fingerprint, dimension)? {
                        SpacePlan::Use {
                            generation,
                            dimension,
                        } => plan = Some((generation, dimension)),
                        SpacePlan::ReindexRequired { current } => {
                            return Err(format!(
                                "reindex_required: 嵌入空间与{current}不兼容；请先执行显式向量回填/重建，旧代际未被修改"
                            ));
                        }
                    }
                }
            }
            let meta = store::DocumentMeta {
                file_path,
                chapter_number,
                chapter_title,
                corpus_kind,
            };
            let ids = store::insert_document(conn, &doc_id, file_name, &text_chunks, &meta)?;
            Ok((ids, plan))
        })?
    };

    // 异步：写向量 + 激活代际
    if let (Some(vectors), Some((generation, dimension))) = (generated.as_ref(), plan) {
        if !chunk_ids.is_empty() {
            write_vectors(
                state,
                project_root,
                generation,
                dimension,
                &chunk_ids,
                vectors,
            )
            .await?;
            state.with_project_db(|conn| {
                hybrid::activate_generation(conn, generation).map_err(|error| error.to_string())?;
                Ok(())
            })?;
        }
    }

    // 同步：删除被替换的同名旧文档（含其向量）
    if let Some(existing) = existing_doc_id {
        let removed = state.with_project_db(|conn| {
            let ids = store::document_chunk_ids(conn, &existing)?;
            let spaces = hybrid::list_spaces(conn).map_err(|error| error.to_string())?;
            let hit = store::remove_document(conn, &existing)?;
            Ok((hit, ids, spaces))
        })?;
        let (_hit, ids, spaces) = removed;
        for space in spaces {
            purge_vectors(state, project_root, space.generation, space.dimension, &ids).await;
        }
    }

    Ok(KbImportResult::ok(doc_id, text_chunks.len() as i64))
}

// ==================== 检索编排 ====================

/// 向量支路检索（active 空间 + 身份匹配时）
#[allow(clippy::too_many_arguments)]
async fn vector_search(
    state: &AppState,
    project_root: &str,
    generation: i64,
    dimension: i64,
    query_vector: &[f32],
    top_k: usize,
    filter: &store::SearchFilter<'_>,
) -> Result<Vec<store::SearchResult>, String> {
    let index = state
        .kb_vector_index(project_root, generation, dimension as usize)
        .await?;
    let hits = index
        .search_rag(query_vector, top_k)
        .await
        .map_err(|error| error.to_string())?;
    if hits.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = hits.iter().map(|(id, _)| id.clone()).collect();
    let rows = state.with_project_db(|conn| store::chunk_rows(conn, &ids))?;
    let by_id: std::collections::HashMap<&str, &store::ChunkRow> =
        rows.iter().map(|row| (row.id.as_str(), row)).collect();

    let mut out = Vec::new();
    for (id, distance) in &hits {
        let Some(row) = by_id.get(id.as_str()) else {
            continue;
        };
        if !store::matches_filter(row, filter) {
            continue;
        }
        out.push(store::SearchResult {
            text: row.text.clone(),
            score: hybrid::vector_distance_to_score(*distance as f64),
            file_name: row.file_name.clone(),
        });
        if out.len() >= top_k {
            break;
        }
    }
    Ok(out)
}

/// 检索核心（默认对齐基线「向量短路，否则文本」；返回 `AppResult` JSON）
#[allow(clippy::too_many_arguments)]
async fn search_core(
    state: &AppState,
    query: &str,
    top_k: Option<i64>,
    chapter_scope: Option<(i64, i64)>,
    excluded_corpus_kinds: &[String],
    project_root: &str,
    session: Option<&ProjectSessionContext>,
) -> Result<Value, String> {
    guard_read(state, project_root, session)?;
    if let Err(message) = preflight_or_error(project_root) {
        return Ok(app_failure("PROJECT_STORAGE_PATH_UNSUPPORTED", message));
    }
    let top_k = match top_k {
        Some(value) if value > 0 => value as usize,
        _ => DEFAULT_TOP_K as usize,
    };
    let filter = store::SearchFilter {
        chapter_scope,
        excluded_corpus_kinds,
    };

    // 可选：查询向量（有 apiKey 且查询非空）
    let setup = embedding_setup();
    let mut query_vector: Option<Vec<f32>> = None;
    if let Some(setup) = setup.as_ref() {
        if usable(setup) && !query.trim().is_empty() {
            if let Ok(vectors) = embed_texts(setup, &[query.to_string()]).await {
                if let Some(vector) = vectors.into_iter().next() {
                    if !vector.is_empty() {
                        query_vector = Some(vector);
                    }
                }
            }
        }
    }

    // 向量短路（仅当 active 空间身份匹配）
    if let Some(query_vector) = query_vector.as_ref() {
        let active = state.with_project_db(|conn| {
            hybrid::active_space(conn).map_err(|error| error.to_string())
        })?;
        if let Some(space) = active {
            let identity_matches = setup
                .as_ref()
                .map(|setup| setup.fingerprint == space.model_fingerprint)
                .unwrap_or(false);
            if identity_matches && query_vector.len() as i64 == space.dimension {
                if let Ok(results) = vector_search(
                    state,
                    project_root,
                    space.generation,
                    space.dimension,
                    query_vector,
                    top_k,
                    &filter,
                )
                .await
                {
                    if !results.is_empty() {
                        return serde_json::to_value(results).map_err(|error| error.to_string());
                    }
                }
            }
        }
    }

    // 文本支路
    let results = state.with_project_db(|conn| store::search_text(conn, query, top_k, &filter))?;
    serde_json::to_value(results).map_err(|error| error.to_string())
}

/// 计算无向量块数（使用 active 或首个 building 代际；无空间则全部计入）
async fn vectorless_count(state: &AppState, project_root: &str) -> Result<i64, String> {
    let (all_ids, space) = state.with_project_db(|conn| {
        let ids = store::all_chunk_ids(conn)?;
        let spaces = hybrid::list_spaces(conn).map_err(|error| error.to_string())?;
        let active = spaces
            .iter()
            .find(|space| space.status == "active")
            .or_else(|| spaces.iter().find(|space| space.status == "building"))
            .map(|space| (space.generation, space.dimension));
        Ok((ids, active))
    })?;
    let Some((generation, dimension)) = space else {
        return Ok(all_ids.len() as i64);
    };
    let index = state
        .kb_vector_index(project_root, generation, dimension as usize)
        .await?;
    let live: std::collections::HashSet<String> = index.live_doc_ids().await.into_iter().collect();
    Ok(all_ids
        .iter()
        .filter(|id| !live.contains(id.as_str()))
        .count() as i64)
}

// ==================== 命令：导入 ====================

/// `kb:import-text`
#[tauri::command]
pub async fn kb_import_text(
    state: State<'_, AppState>,
    text: String,
    file_name: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<KbImportResult, String> {
    import_text_core(
        state.inner(),
        &text,
        &file_name,
        &expected_project_path,
        project_session.as_ref(),
        false,
        "project-knowledge",
        None,
        None,
        "",
    )
    .await
}

/// `kb:import-planning-text`（强制 FTS-only）
#[tauri::command]
pub async fn kb_import_planning_text(
    state: State<'_, AppState>,
    text: String,
    file_name: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<KbImportResult, String> {
    import_text_core(
        state.inner(),
        &text,
        &file_name,
        &expected_project_path,
        project_session.as_ref(),
        true,
        "project-knowledge",
        None,
        None,
        "",
    )
    .await
}

/// 从文件名解析章节元数据（对齐基线 `parseChapterMetaFromFileName`）
fn parse_chapter_meta_from_file_name(file_name: &str) -> Option<(i64, String)> {
    let trimmed = file_name.strip_prefix('第')?;
    let (number, rest) = trimmed.split_once('章')?;
    let number: i64 = number.parse().ok()?;
    let rest = rest.strip_prefix(' ')?;
    for suffix in [" 正文.md", " 要点.md", " 蓝图.md"] {
        if let Some(title) = rest.strip_suffix(suffix) {
            if !title.is_empty() {
                return Some((number, title.to_string()));
            }
        }
    }
    None
}

/// `kb:import-document`：授权文件 → 读取 → 导入
#[tauri::command]
pub async fn kb_import_document(
    state: State<'_, AppState>,
    grant_id: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<KbImportResult, String> {
    let state = state.inner();
    guard_read(state, &expected_project_path, project_session.as_ref())?;

    let granted = {
        let mut registry = state
            .external_grants
            .lock()
            .map_err(|_| "外部文件授权状态被污染".to_string())?;
        registry.resolve(&grant_id, GrantOperation::Read)
    };
    let Ok(granted) = granted else {
        return Ok(KbImportResult::failure(
            crate::external_grant::INVALID_GRANT_MESSAGE,
        ));
    };

    let content = match std::fs::read_to_string(&granted.path) {
        Ok(content) => content,
        Err(error) => {
            return Ok(KbImportResult::failure(format!(
                "读取外部文件失败：{error}"
            )));
        }
    };
    let file_name = granted
        .path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let file_path = granted.path.to_string_lossy().to_string();
    let (chapter_number, chapter_title) = match parse_chapter_meta_from_file_name(&file_name) {
        Some((number, title)) => (Some(number), Some(title)),
        None => (None, None),
    };

    import_text_core(
        state,
        &content,
        &file_name,
        &expected_project_path,
        project_session.as_ref(),
        false,
        "project-knowledge",
        chapter_number,
        chapter_title.as_deref(),
        &file_path,
    )
    .await
}

/// 递归收集目录内 `.txt` / `.md` / `.markdown` 文件为 `(fileName, content)`
fn collect_folder_files(root: &Path) -> Result<Vec<(String, String)>, String> {
    let mut out: Vec<(String, String)> = Vec::new();
    fn visit(
        directory: &Path,
        depth: usize,
        out: &mut Vec<(String, String)>,
    ) -> Result<(), String> {
        if depth > MAX_IMPORT_DEPTH {
            return Err("SECURE_FS_DIRECTORY_TOO_DEEP".to_string());
        }
        let entries = std::fs::read_dir(directory)
            .map_err(|error| format!("读取目录失败（{}）：{error}", directory.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            // 跳过符号链接/junction，避免递归期间逃逸
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                visit(&path, depth + 1, out)?;
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let lower = name.to_lowercase();
            if !(lower.ends_with(".txt") || lower.ends_with(".md") || lower.ends_with(".markdown"))
            {
                continue;
            }
            if out.len() >= MAX_IMPORT_FILES {
                return Err("SECURE_FS_DIRECTORY_TOO_LARGE".to_string());
            }
            let content = std::fs::read_to_string(&path)
                .map_err(|error| format!("读取文件失败（{}）：{error}", path.display()))?;
            out.push((name, content));
        }
        Ok(())
    }
    visit(root, 0, &mut out)?;
    Ok(out)
}

/// `kb:import-folder`：授权目录递归导入
#[tauri::command]
pub async fn kb_import_folder(
    state: State<'_, AppState>,
    grant_id: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<KbFolderImportResult, String> {
    let state = state.inner();
    guard_read(state, &expected_project_path, project_session.as_ref())?;
    if let Err(message) = preflight_or_error(&expected_project_path) {
        return Ok(KbFolderImportResult {
            success: false,
            imported_count: 0,
            failed_files: Vec::new(),
            error: Some(message),
            error_code: Some("PROJECT_STORAGE_PATH_UNSUPPORTED".to_string()),
        });
    }

    // 枚举本身不授予读取权：先 revalidate(read)，再 resolve(list) 消费一次性授权
    let folder = {
        let mut registry = state
            .external_grants
            .lock()
            .map_err(|_| "外部文件授权状态被污染".to_string())?;
        if registry
            .revalidate(&grant_id, GrantOperation::Read)
            .is_err()
        {
            None
        } else {
            registry
                .resolve(&grant_id, GrantOperation::List)
                .ok()
                .map(|granted| granted.path)
        }
    };
    let Some(folder) = folder else {
        return Ok(KbFolderImportResult {
            success: false,
            imported_count: 0,
            failed_files: Vec::new(),
            error: Some(crate::external_grant::INVALID_GRANT_MESSAGE.to_string()),
            error_code: None,
        });
    };

    let files = match collect_folder_files(&folder) {
        Ok(files) => files,
        Err(error) => {
            return Ok(KbFolderImportResult {
                success: false,
                imported_count: 0,
                failed_files: Vec::new(),
                error: Some(error),
                error_code: None,
            });
        }
    };
    if files.is_empty() {
        return Ok(KbFolderImportResult {
            success: true,
            imported_count: 0,
            failed_files: Vec::new(),
            error: None,
            error_code: None,
        });
    }

    let mut imported_count = 0i64;
    let mut failed_files: Vec<String> = Vec::new();
    for (file_name, content) in files {
        let result = import_text_core(
            state,
            &content,
            &file_name,
            &expected_project_path,
            project_session.as_ref(),
            false,
            "project-knowledge",
            None,
            None,
            "",
        )
        .await?;
        if result.success {
            imported_count += 1;
        } else {
            failed_files.push(file_name);
        }
    }
    Ok(KbFolderImportResult {
        success: true,
        imported_count,
        failed_files,
        error: None,
        error_code: None,
    })
}

/// `kb:import-reference-text` —— **诚实化占位**（依赖批次 G 的导入运行权威校验）
#[tauri::command]
pub async fn kb_import_reference_text(
    state: State<'_, AppState>,
    _chapter_number: i64,
    _run_id: String,
    _execution_authority: Value,
    _project_session: Option<ProjectSessionContext>,
) -> Result<KbImportResult, String> {
    let _ = state;
    Ok(KbImportResult::failure(
        "参照知识导入尚未迁移（依赖批次 G 的导入运行权威校验），当前未写入任何内容。",
    ))
}

// ==================== 命令：检索 ====================

/// `kb:search`
#[tauri::command]
pub async fn kb_search(
    state: State<'_, AppState>,
    query: String,
    top_k: Option<i64>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    search_core(
        state.inner(),
        &query,
        top_k,
        None,
        &[],
        &expected_project_path,
        project_session.as_ref(),
    )
    .await
}

/// `kb:search-writing-context`（排除 `reference` 语料）
#[tauri::command]
pub async fn kb_search_writing_context(
    state: State<'_, AppState>,
    query: String,
    top_k: Option<i64>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    search_core(
        state.inner(),
        &query,
        top_k,
        None,
        &["reference".to_string()],
        &expected_project_path,
        project_session.as_ref(),
    )
    .await
}

/// `kb:search-with-scope`（章节范围限定）
#[tauri::command]
pub async fn kb_search_with_scope(
    state: State<'_, AppState>,
    query: String,
    from_chapter: i64,
    to_chapter: i64,
    top_k: Option<i64>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    search_core(
        state.inner(),
        &query,
        top_k,
        Some((from_chapter, to_chapter)),
        &[],
        &expected_project_path,
        project_session.as_ref(),
    )
    .await
}

// ==================== 命令：文档管理 ====================

/// `kb:list-documents`
#[tauri::command]
pub async fn kb_list_documents(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    guard_read(state, &expected_project_path, project_session.as_ref())?;
    if let Err(message) = preflight_or_error(&expected_project_path) {
        return Ok(app_failure("PROJECT_STORAGE_PATH_UNSUPPORTED", message));
    }
    let documents = state.with_project_db(store::list_documents)?;
    serde_json::to_value(documents).map_err(|error| error.to_string())
}

/// `kb:remove-document`（含向量清理）
#[tauri::command]
pub async fn kb_remove_document(
    state: State<'_, AppState>,
    doc_id: String,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<SimpleResult, String> {
    let state = state.inner();
    guard_read(state, &expected_project_path, project_session.as_ref())?;
    if let Err(message) = preflight_or_error(&expected_project_path) {
        return Ok(SimpleResult {
            success: false,
            error: Some(message),
        });
    }

    let (ids, spaces, hit) = state.with_project_db(|conn| {
        let ids = store::document_chunk_ids(conn, &doc_id)?;
        let spaces = hybrid::list_spaces(conn).map_err(|error| error.to_string())?;
        let hit = store::remove_document(conn, &doc_id)?;
        Ok((ids, spaces, hit))
    })?;
    for space in spaces {
        purge_vectors(
            state,
            &expected_project_path,
            space.generation,
            space.dimension,
            &ids,
        )
        .await;
    }
    Ok(if hit {
        SimpleResult {
            success: true,
            error: None,
        }
    } else {
        SimpleResult {
            success: false,
            error: Some("删除知识库文档失败".to_string()),
        }
    })
}

/// `kb:clear-all`（含向量索引与快照目录清理）
#[tauri::command]
pub async fn kb_clear_all(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<SimpleResult, String> {
    let state = state.inner();
    guard_read(state, &expected_project_path, project_session.as_ref())?;
    if let Err(message) = preflight_or_error(&expected_project_path) {
        return Ok(SimpleResult {
            success: false,
            error: Some(message),
        });
    }
    state.with_project_db(store::clear_all)?;
    state.drop_kb_vectors(&expected_project_path);
    let snapshot_dir = vectors::snapshot_dir(&expected_project_path);
    if snapshot_dir.exists() {
        let _ = std::fs::remove_dir_all(&snapshot_dir);
    }
    Ok(SimpleResult {
        success: true,
        error: None,
    })
}

/// `kb:stats`
#[tauri::command]
pub async fn kb_stats(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    guard_read(state, &expected_project_path, project_session.as_ref())?;
    if let Err(message) = preflight_or_error(&expected_project_path) {
        return Ok(app_failure("PROJECT_STORAGE_PATH_UNSUPPORTED", message));
    }
    let stats = state.with_project_db(store::stats)?;
    serde_json::to_value(stats).map_err(|error| error.to_string())
}

// ==================== 命令：向量维护 ====================

/// `kb:get-vectorless-count`
#[tauri::command]
pub async fn kb_get_vectorless_count(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    guard_read(state, &expected_project_path, project_session.as_ref())?;
    if let Err(message) = preflight_or_error(&expected_project_path) {
        return Ok(app_failure("PROJECT_STORAGE_PATH_UNSUPPORTED", message));
    }
    let count = vectorless_count(state, &expected_project_path).await?;
    Ok(json!({ "count": count }))
}

/// `kb:get-vector-rebuild-status` —— 纯本地状态读，绝不发起 embedding 请求
#[tauri::command]
pub async fn kb_get_vector_rebuild_status(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    guard_read(state, &expected_project_path, project_session.as_ref())?;
    if let Err(message) = preflight_or_error(&expected_project_path) {
        return Ok(app_failure("PROJECT_STORAGE_PATH_UNSUPPORTED", message));
    }

    let setup = embedding_setup();
    if !setup.as_ref().map(usable).unwrap_or(false) {
        return Ok(json!({
            "embeddingConfigured": false,
            "canRebuild": false,
            "totalChunks": 0,
            "vectorlessCount": 0,
            "activeVectorDimension": 0,
        }));
    }
    let stats = state.with_project_db(store::stats)?;
    let vectorless = vectorless_count(state, &expected_project_path).await?;
    Ok(json!({
        "embeddingConfigured": true,
        "canRebuild": stats.total_chunks > 0,
        "totalChunks": stats.total_chunks,
        "vectorlessCount": vectorless,
        "activeVectorDimension": stats.vector_dimension,
    }))
}

/// `kb:backfill-vectors`
#[tauri::command]
pub async fn kb_backfill_vectors(
    state: State<'_, AppState>,
    expected_project_path: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<KbBackfillResult, String> {
    let state = state.inner();
    guard_read(state, &expected_project_path, project_session.as_ref())?;
    if let Err(message) = preflight_or_error(&expected_project_path) {
        return Ok(KbBackfillResult {
            success: false,
            processed: 0,
            failed: 0,
            error: Some(message),
            error_code: Some("PROJECT_STORAGE_PATH_UNSUPPORTED".to_string()),
        });
    }
    let Some(setup) = embedding_setup().filter(usable) else {
        return Ok(KbBackfillResult {
            success: false,
            processed: 0,
            failed: 0,
            error: Some("未配置 Embedding 模型".to_string()),
            error_code: Some("EMBEDDING_MODEL_NOT_CONFIGURED".to_string()),
        });
    };

    // 全量 canonical 块
    let rows = state.with_project_db(|conn| {
        let ids = store::all_chunk_ids(conn)?;
        store::chunk_rows(conn, &ids)
    })?;
    if rows.is_empty() {
        return Ok(KbBackfillResult {
            success: true,
            processed: 0,
            failed: 0,
            error: None,
            error_code: None,
        });
    }

    // 探测维度（对齐基线：用首个块的响应建立唯一安全维度）
    let probe = match embed_texts(&setup, &[rows[0].text.clone()]).await {
        Ok(vectors) if vectors.len() == 1 => vectors[0].clone(),
        Ok(vectors) => {
            return Ok(KbBackfillResult {
                success: false,
                processed: 0,
                failed: rows.len() as i64,
                error: Some(format!(
                    "Embedding 探测返回数量不匹配：期望 1，实际 {}",
                    vectors.len()
                )),
                error_code: None,
            });
        }
        Err(error) => {
            return Ok(KbBackfillResult {
                success: false,
                processed: 0,
                failed: rows.len() as i64,
                error: Some(error.display_text()),
                error_code: None,
            });
        }
    };
    let dimension = probe.len() as i64;
    if dimension == 0 {
        return Ok(KbBackfillResult {
            success: false,
            processed: 0,
            failed: rows.len() as i64,
            error: Some("Embedding 返回空向量".to_string()),
            error_code: None,
        });
    }

    // 选定/新建代际
    let generation = {
        let fingerprint = setup.fingerprint.clone();
        state.with_project_db(move |conn| {
            let spaces = hybrid::list_spaces(conn).map_err(|error| error.to_string())?;
            if let Some(space) = spaces.iter().find(|space| {
                space.model_fingerprint == fingerprint && space.dimension == dimension
            }) {
                return Ok(space.generation);
            }
            let generation = hybrid::next_generation(conn).map_err(|error| error.to_string())?;
            hybrid::upsert_space(
                conn,
                &hybrid::EmbeddingSpace {
                    generation,
                    dimension,
                    model_fingerprint: fingerprint,
                    distance_metric: "cosine".to_string(),
                    status: "building".to_string(),
                },
            )
            .map_err(|error| error.to_string())?;
            Ok(generation)
        })?
    };

    let index = state
        .kb_vector_index(&expected_project_path, generation, dimension as usize)
        .await?;
    let live: std::collections::HashSet<String> = index.live_doc_ids().await.into_iter().collect();
    let missing: Vec<(String, String)> = rows
        .iter()
        .filter(|row| !live.contains(&row.id))
        .map(|row| (row.id.clone(), row.text.clone()))
        .collect();
    if missing.is_empty() {
        state.with_project_db(|conn| {
            hybrid::activate_generation(conn, generation).map_err(|error| error.to_string())?;
            Ok(())
        })?;
        return Ok(KbBackfillResult {
            success: true,
            processed: 0,
            failed: 0,
            error: None,
            error_code: None,
        });
    }

    let texts: Vec<String> = missing.iter().map(|(_, text)| text.clone()).collect();
    let generated = match embed_texts(&setup, &texts).await {
        Ok(vectors) if vectors.len() == missing.len() => vectors,
        Ok(vectors) => {
            return Ok(KbBackfillResult {
                success: false,
                processed: 0,
                failed: missing.len() as i64,
                error: Some(format!(
                    "Embedding 返回数量不匹配：期望 {}，实际 {}",
                    missing.len(),
                    vectors.len()
                )),
                error_code: None,
            });
        }
        Err(error) => {
            return Ok(KbBackfillResult {
                success: false,
                processed: 0,
                failed: missing.len() as i64,
                error: Some(error.display_text()),
                error_code: None,
            });
        }
    };

    let ids: Vec<String> = missing.iter().map(|(id, _)| id.clone()).collect();
    write_vectors(
        state,
        &expected_project_path,
        generation,
        dimension,
        &ids,
        &generated,
    )
    .await?;
    state.with_project_db(|conn| {
        hybrid::activate_generation(conn, generation).map_err(|error| error.to_string())?;
        Ok(())
    })?;

    Ok(KbBackfillResult {
        success: true,
        processed: ids.len() as i64,
        failed: 0,
        error: None,
        error_code: None,
    })
}

// ==================== 命令：外部文件选择 ====================

/// `dialog:select-knowledge-files`
#[tauri::command]
pub async fn dialog_select_knowledge_files(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<Vec<ExternalGrantDto>>, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut builder = app
        .dialog()
        .file()
        .set_title("选择要导入的文档")
        .add_filter("文本文件", &["txt", "md", "markdown"]);
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        builder = builder.set_parent(&window);
    }
    builder.pick_files(move |selection| {
        let _ = sender.send(selection);
    });
    let picked = tauri::async_runtime::spawn_blocking(move || {
        receiver.recv_timeout(DIALOG_TIMEOUT).ok().flatten()
    })
    .await
    .ok()
    .flatten();
    let Some(paths) = picked else {
        return Ok(None);
    };

    let mut registry = state
        .external_grants
        .lock()
        .map_err(|_| "外部文件授权状态被污染".to_string())?;
    let mut grants = Vec::new();
    for file in paths {
        let Some(path) = file_path_to_string(file) else {
            continue;
        };
        let display_name = Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        if let Ok(grant_id) =
            registry.issue_file(Path::new(&path), KNOWLEDGE_BASE_GRANT_TTL, Some(1))
        {
            grants.push(ExternalGrantDto {
                grant_id,
                display_name,
            });
        }
    }
    if grants.is_empty() {
        Ok(None)
    } else {
        Ok(Some(grants))
    }
}

/// `dialog:select-knowledge-folder`
#[tauri::command]
pub async fn dialog_select_knowledge_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<ExternalGrantDto>, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut builder = app.dialog().file().set_title("选择要批量导入的文件夹");
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        builder = builder.set_parent(&window);
    }
    builder.pick_folder(move |selection| {
        let _ = sender.send(selection);
    });
    let picked = tauri::async_runtime::spawn_blocking(move || {
        receiver.recv_timeout(DIALOG_TIMEOUT).ok().flatten()
    })
    .await
    .ok()
    .flatten();
    let Some(folder) = picked.and_then(file_path_to_string) else {
        return Ok(None);
    };

    let display_name = Path::new(&folder)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| folder.clone());
    let mut registry = state
        .external_grants
        .lock()
        .map_err(|_| "外部文件授权状态被污染".to_string())?;
    match registry.issue_directory(Path::new(&folder), KNOWLEDGE_BASE_GRANT_TTL, Some(1)) {
        Ok(grant_id) => Ok(Some(ExternalGrantDto {
            grant_id,
            display_name,
        })),
        Err(_) => Ok(None),
    }
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_embedding_options_matches_baseline_bounds_test() {
        let model = json!({});
        assert_eq!(normalize_embedding_options(&model), (500, 50, Some(50)));

        let model = json!({"embeddingOptions": {"chunkSize": 100000, "chunkOverlap": 99999, "batchSize": 999}});
        assert_eq!(normalize_embedding_options(&model), (4000, 3999, Some(50)));

        let model =
            json!({"embeddingOptions": {"chunkSize": 1, "chunkOverlap": -5, "batchSize": 0}});
        assert_eq!(normalize_embedding_options(&model), (100, 0, Some(1)));
    }

    #[test]
    fn model_fingerprint_matches_baseline_shape_test() {
        let model = json!({"baseUrl": "https://api.example.com/", "modelName": "  text-embedding-3-small "});
        assert_eq!(
            model_fingerprint("openai", &model),
            "openai|https://api.example.com|text-embedding-3-small"
        );
        let model = json!({"baseUrl": "https://api.example.com"});
        assert_eq!(
            model_fingerprint("gemini", &model),
            "gemini|https://api.example.com|default"
        );
    }

    #[test]
    fn parse_chapter_meta_from_file_name_test() {
        assert_eq!(
            parse_chapter_meta_from_file_name("第12章 风起 正文.md"),
            Some((12, "风起".to_string()))
        );
        assert_eq!(
            parse_chapter_meta_from_file_name("第3章 云涌 要点.md"),
            Some((3, "云涌".to_string()))
        );
        assert_eq!(parse_chapter_meta_from_file_name("随手笔记.md"), None);
        assert_eq!(
            parse_chapter_meta_from_file_name("第x章 风起 正文.md"),
            None
        );
    }

    #[test]
    fn collect_folder_files_skips_unsupported_and_reports_names_test() {
        let dir = std::env::temp_dir().join(format!("kb-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("子目录")).unwrap();
        std::fs::write(dir.join("a.txt"), "甲").unwrap();
        std::fs::write(dir.join("b.md"), "乙").unwrap();
        std::fs::write(dir.join("c.json"), "丙").unwrap();
        std::fs::write(dir.join("子目录").join("d.markdown"), "丁").unwrap();

        let mut files = collect_folder_files(&dir).unwrap();
        files.sort();
        let names: Vec<String> = files.into_iter().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            vec![
                "a.txt".to_string(),
                "b.md".to_string(),
                "d.markdown".to_string()
            ]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
