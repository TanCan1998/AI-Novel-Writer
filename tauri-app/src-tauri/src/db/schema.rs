//! 项目数据库 schema —— 分批平移自 `electron/database.ts` 的 `createTables`
//!
//! 约束（Tauri 侧**独立库**，与原项目不共享数据库文件）：
//! 1. 建表必须幂等（`IF NOT EXISTS`）：`read` / `commit` 等路径每次打开都会复跑迁移；
//! 2. 列定义对齐 Electron 侧的**最终列集**（含历史 ALTER 追加列），以保留将来
//!    一次性导入原项目数据的能力；
//! 3. 旧库通过 legacy 列补齐迁移收敛，迁移规则逐字对齐基线。
//!
//! 批次 C 按子域推进：建表与迁移在此汇总，仓储实现在 `repositories/`。

use rusqlite::{Connection, OptionalExtension, Result as SqlResult};
use std::collections::HashSet;

/// project_core —— 项目主台账（NovelConfig + 架构四大件），恒为单行（`id = 'main'`）
pub const CREATE_PROJECT_CORE: &str = r#"
CREATE TABLE IF NOT EXISTS project_core (
  id TEXT PRIMARY KEY DEFAULT 'main',
  project_name TEXT NOT NULL DEFAULT '',
  -- [基础定位]
  genre TEXT DEFAULT '',
  sub_genre TEXT DEFAULT '',
  target_audience TEXT DEFAULT '',
  total_chapters INTEGER DEFAULT 100,
  words_per_chapter INTEGER DEFAULT 3000,
  writing_language TEXT NOT NULL DEFAULT 'zh-CN',
  creative_strategy TEXT NOT NULL DEFAULT 'auto',
  narrative_thread_dormant_threshold INTEGER NOT NULL DEFAULT 3,
  -- [写作技法]
  plot_structure TEXT DEFAULT 'three_act',
  narrative_pov TEXT DEFAULT 'third_limited',
  writing_style TEXT DEFAULT '',
  reference_works TEXT DEFAULT '',
  global_guidance TEXT DEFAULT '',
  golden_finger TEXT DEFAULT '',
  core_outline TEXT DEFAULT '',
  world_setting TEXT DEFAULT '',
  protagonist_profile TEXT DEFAULT '',
  -- [架构四大件]
  premise TEXT DEFAULT '',
  worldbuilding TEXT DEFAULT '',
  characters_arch TEXT DEFAULT '',
  synopsis TEXT DEFAULT '',
  -- [系统缓存]
  character_states TEXT DEFAULT '',
  plot_tree_snapshot TEXT NOT NULL DEFAULT '',
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);
"#;

/// blueprints —— 章节蓝图（`characters` 列表为 JSON 数组，角色改名需同步维护）
pub const CREATE_BLUEPRINTS: &str = r#"
CREATE TABLE IF NOT EXISTS blueprints (
  chapter_number INTEGER PRIMARY KEY,
  title TEXT NOT NULL DEFAULT '',
  role TEXT DEFAULT '',
  purpose TEXT DEFAULT '',
  key_events TEXT DEFAULT '',
  characters TEXT DEFAULT '[]',
  suspense_hook TEXT DEFAULT '',
  user_guidance TEXT DEFAULT '',
  notes TEXT DEFAULT '',
  notes_updated_at TEXT DEFAULT '',
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);
"#;

/// blueprint_commit_operations —— 蓝图表提交幂等日志（对齐基线 `ensureBlueprintCommitSchema`）
pub const CREATE_BLUEPRINT_COMMIT_OPERATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS blueprint_commit_operations (
  operation_id TEXT PRIMARY KEY,
  payload_hash TEXT NOT NULL,
  mode TEXT NOT NULL CHECK(mode IN ('full', 'replace-range')),
  start_chapter INTEGER NOT NULL,
  end_chapter INTEGER NOT NULL,
  character_sync_input TEXT NOT NULL,
  committed_at TEXT NOT NULL DEFAULT (datetime('now'))
);
"#;

/// blueprint_character_sync_operations —— 角色同步操作持久化（含基线外键与索引）
pub const CREATE_BLUEPRINT_CHARACTER_SYNC_OPERATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS blueprint_character_sync_operations (
  operation_id TEXT PRIMARY KEY,
  blueprint_commit_operation_id TEXT NOT NULL UNIQUE,
  blueprint_commit_payload_hash TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending', 'completed')),
  start_chapter INTEGER NOT NULL,
  end_chapter INTEGER NOT NULL,
  character_sync_input TEXT NOT NULL,
  completion_receipt TEXT DEFAULT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  completed_at TEXT DEFAULT NULL,
  FOREIGN KEY (blueprint_commit_operation_id)
    REFERENCES blueprint_commit_operations(operation_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_blueprint_character_sync_status
  ON blueprint_character_sync_operations(status, created_at);
"#;

/// 蓝图提交/角色同步表的幂等收敛（对齐基线 `ensureBlueprintCommitSchema`）
///
/// 建表 + 索引 + 回填：把已存在的提交操作补建对应的待处理角色同步操作。
/// Tauri 侧在每次 `create_tables`（即每次打开项目库）调用，效果等价于基线在
/// 各 blueprint 写路径前的惰性收敛。
pub fn ensure_blueprint_commit_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(CREATE_BLUEPRINT_COMMIT_OPERATIONS)?;
    conn.execute_batch(CREATE_BLUEPRINT_CHARACTER_SYNC_OPERATIONS)?;
    conn.execute_batch(
        r#"
        INSERT OR IGNORE INTO blueprint_character_sync_operations (
          operation_id,
          blueprint_commit_operation_id,
          blueprint_commit_payload_hash,
          start_chapter,
          end_chapter,
          character_sync_input
        )
        SELECT
          'blueprint-sync-' || operation_id,
          operation_id,
          payload_hash,
          start_chapter,
          end_chapter,
          character_sync_input
        FROM blueprint_commit_operations;
        "#,
    )?;
    Ok(())
}

/// characters —— 角色卡（`currentState` 拍平为 `cs_*` 列，杜绝 JSON 大字段）
pub const CREATE_CHARACTERS: &str = r#"
CREATE TABLE IF NOT EXISTS characters (
  name TEXT PRIMARY KEY,
  role TEXT DEFAULT 'supporting',
  gender TEXT DEFAULT '',
  age TEXT DEFAULT '',
  appearance TEXT DEFAULT '',
  personality TEXT DEFAULT '',
  background TEXT DEFAULT '',
  abilities TEXT DEFAULT '',
  motivation TEXT DEFAULT '',
  relationships TEXT DEFAULT '',
  arc TEXT DEFAULT '',
  notes TEXT DEFAULT '',
  cs_location TEXT DEFAULT '',
  cs_power_level TEXT DEFAULT '',
  cs_physical_state TEXT DEFAULT '',
  cs_mental_state TEXT DEFAULT '',
  cs_key_items TEXT DEFAULT '',
  cs_recent_events TEXT DEFAULT '',
  cs_updated_at_chapter INTEGER DEFAULT NULL,
  cs_provenance TEXT NOT NULL DEFAULT '{}',
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);
"#;

/// contents —— 文本内容池（正文与元数据分离）
pub const CREATE_CONTENTS: &str = r#"
CREATE TABLE IF NOT EXISTS contents (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  body TEXT NOT NULL DEFAULT '',
  created_at TEXT DEFAULT (datetime('now'))
);
"#;

/// drafts —— 草稿主线（`finalized` 即定稿）
///
/// 本子域只建表：角色名单的 `chapter_progress` 来源校验需要读 `drafts` 与
/// `finalization_outbox`（对齐基线 `assertCurrentFinalizedSource`）。drafts 子域
/// 迁移时不再重复建表，只补仓储与命令。
pub const CREATE_DRAFTS: &str = r#"
CREATE TABLE IF NOT EXISTS drafts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  chapter_number INTEGER NOT NULL,
  version INTEGER NOT NULL,
  status TEXT DEFAULT 'draft',
  source TEXT DEFAULT 'write',
  content_id INTEGER NOT NULL,
  word_count INTEGER DEFAULT 0,
  source_dependencies TEXT NOT NULL DEFAULT '[]',
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now')),
  FOREIGN KEY (content_id) REFERENCES contents(id) ON DELETE RESTRICT
);
CREATE INDEX IF NOT EXISTS idx_drafts_chapter ON drafts(chapter_number);
CREATE UNIQUE INDEX IF NOT EXISTS idx_drafts_chapter_version ON drafts(chapter_number, version);
"#;

/// finalization_outbox —— 定稿实体稿发布投影（SQLite 事实先提交，根目录实体稿异步发布）
pub const CREATE_FINALIZATION_OUTBOX: &str = r#"
CREATE TABLE IF NOT EXISTS finalization_outbox (
  finalization_id TEXT PRIMARY KEY,
  draft_id INTEGER NOT NULL UNIQUE,
  chapter_number INTEGER NOT NULL,
  chapter_title TEXT NOT NULL DEFAULT '',
  content_hash TEXT NOT NULL,
  content_revision INTEGER NOT NULL,
  content_snapshot TEXT NOT NULL DEFAULT '',
  target_file_name TEXT NOT NULL,
  knowledge_document_id TEXT NOT NULL DEFAULT '',
  publication_status TEXT NOT NULL DEFAULT 'pending',
  last_error TEXT NOT NULL DEFAULT '',
  published_at TEXT DEFAULT NULL,
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now')),
  FOREIGN KEY (draft_id) REFERENCES drafts(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_finalization_outbox_status ON finalization_outbox(publication_status);
"#;

/// revisions —— 修稿（基于某版草稿的探索分支；`pending` → `merged` / `discarded`）
///
/// 生成时会冻结源稿快照（章号/版本/状态/正文），合并时据此拒绝与源稿不一致的覆盖。
pub const CREATE_REVISIONS: &str = r#"
CREATE TABLE IF NOT EXISTS revisions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  base_draft_id INTEGER NOT NULL,
  revision_index INTEGER NOT NULL,
  revision_type TEXT NOT NULL,
  status TEXT DEFAULT 'pending',
  merged_to_draft_id INTEGER,
  user_prompt TEXT DEFAULT '',
  review_source_id INTEGER,
  source_draft_chapter_number INTEGER,
  source_draft_version INTEGER,
  source_draft_status TEXT,
  source_content TEXT,
  content_id INTEGER NOT NULL,
  word_count INTEGER DEFAULT 0,
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now')),
  FOREIGN KEY (base_draft_id) REFERENCES drafts(id) ON DELETE CASCADE,
  FOREIGN KEY (content_id) REFERENCES contents(id) ON DELETE RESTRICT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_revisions_draft_index
  ON revisions(base_draft_id, revision_index);
"#;

/// reviews —— 审稿（对某版草稿的评审反馈报告；无状态流转）
///
/// 审稿时也冻结源稿快照（章号/版本/状态/正文），保证报告可追溯到当时的稿本。
pub const CREATE_REVIEWS: &str = r#"
CREATE TABLE IF NOT EXISTS reviews (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  base_draft_id INTEGER NOT NULL,
  review_index INTEGER NOT NULL,
  source_draft_chapter_number INTEGER,
  source_draft_version INTEGER,
  source_draft_status TEXT,
  source_content TEXT,
  content_id INTEGER NOT NULL,
  created_at TEXT DEFAULT (datetime('now')),
  FOREIGN KEY (base_draft_id) REFERENCES drafts(id) ON DELETE CASCADE,
  FOREIGN KEY (content_id) REFERENCES contents(id) ON DELETE RESTRICT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_reviews_draft_index
  ON reviews(base_draft_id, review_index);
"#;

/// post_process_runs / post_process_steps —— 后处理跑批与步骤明细
///
/// 每次后处理产生一个 Run（UUID 主键），下属多个 Step；`all_critical_passed` 是
/// 由步骤收据派生的汇总标志（“无失败关键步骤”）。
pub const CREATE_POST_PROCESS: &str = r#"
CREATE TABLE IF NOT EXISTS post_process_runs (
  id TEXT PRIMARY KEY,
  trigger_source_type TEXT NOT NULL,
  trigger_source_id TEXT NOT NULL,
  source_label TEXT DEFAULT '',
  all_critical_passed INTEGER DEFAULT 0,
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_post_runs_source
  ON post_process_runs(trigger_source_type, trigger_source_id);

CREATE TABLE IF NOT EXISTS post_process_steps (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id TEXT NOT NULL,
  step_key TEXT NOT NULL,
  label TEXT DEFAULT '',
  critical INTEGER DEFAULT 0,
  ok INTEGER DEFAULT 0,
  error_msg TEXT DEFAULT '',
  attempt_count INTEGER DEFAULT 0,
  completed_at TEXT DEFAULT '',
  last_attempt_at TEXT DEFAULT '',
  FOREIGN KEY (run_id) REFERENCES post_process_runs(id) ON DELETE CASCADE
);
"#;

/// llm_calls —— LLM 调用日志（统计与历史面板的事实源）
pub const CREATE_LLM_CALLS: &str = r#"
CREATE TABLE IF NOT EXISTS llm_calls (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  model_id TEXT NOT NULL,
  model_name TEXT DEFAULT '',
  purpose TEXT DEFAULT '',
  prompt_tokens INTEGER DEFAULT 0,
  completion_tokens INTEGER DEFAULT 0,
  total_tokens INTEGER DEFAULT 0,
  duration_ms INTEGER DEFAULT 0,
  success INTEGER DEFAULT 1,
  error_message TEXT DEFAULT '',
  created_at TEXT DEFAULT (datetime('now'))
);
"#;

/// summary_snapshots —— 跨用表：角色状态快照（含定稿连续性投影列）
///
/// `draft_id IS NULL` 的行是旧式快照（`getLatestSnapshot` 只看这些行）；
/// 绑定定稿的行由连续性投影写入（批次 E）。
pub const CREATE_SUMMARY_SNAPSHOTS: &str = r#"
CREATE TABLE IF NOT EXISTS summary_snapshots (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  draft_id INTEGER DEFAULT NULL,
  chapter_number INTEGER NOT NULL,
  character_states TEXT DEFAULT '',
  chapter_notes TEXT NOT NULL DEFAULT '',
  continuity_facts TEXT NOT NULL DEFAULT '[]',
  character_state_candidates TEXT NOT NULL DEFAULT '[]',
  source_finalization_id TEXT NOT NULL DEFAULT '',
  source_content_hash TEXT NOT NULL DEFAULT '',
  projection_generation INTEGER NOT NULL DEFAULT 0,
  created_at TEXT DEFAULT (datetime('now')),
  FOREIGN KEY (draft_id) REFERENCES drafts(id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_summary_snapshots_draft
  ON summary_snapshots(draft_id) WHERE draft_id IS NOT NULL;
"#;

/// character_roster —— 结构化角色名单的并发控制/迁移/投影元数据
///
/// 角色条目本体始终留在 `characters` 表，这里绝不建并列 JSON 事实源。
pub const CREATE_CHARACTER_ROSTER: &str = r#"
CREATE TABLE IF NOT EXISTS character_roster_meta (
  id TEXT PRIMARY KEY CHECK (id = 'main'),
  schema_version INTEGER NOT NULL CHECK (schema_version = 1),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  migration_state TEXT NOT NULL CHECK (
    migration_state IN (
      'empty',
      'legacy_cards_preserved',
      'legacy_markdown_pending',
      'ready'
    )
  ),
  legacy_markdown TEXT NOT NULL DEFAULT '',
  projection_hash TEXT NOT NULL DEFAULT '',
  fact_hash TEXT NOT NULL DEFAULT '',
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS character_roster_operations (
  operation_id TEXT PRIMARY KEY,
  payload_hash TEXT NOT NULL,
  committed_revision INTEGER NOT NULL,
  projection_hash TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
"#;

/// 建表入口（幂等）：所有分批 DDL 在此汇总执行后再跑迁移
pub fn create_tables(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(CREATE_PROJECT_CORE)?;
    conn.execute_batch(CREATE_BLUEPRINTS)?;
    ensure_blueprint_commit_schema(conn)?;
    conn.execute_batch(CREATE_CHARACTERS)?;
    conn.execute_batch(CREATE_CONTENTS)?;
    conn.execute_batch(CREATE_DRAFTS)?;
    conn.execute_batch(CREATE_REVISIONS)?;
    conn.execute_batch(CREATE_REVIEWS)?;
    conn.execute_batch(CREATE_POST_PROCESS)?;
    conn.execute_batch(CREATE_LLM_CALLS)?;
    conn.execute_batch(CREATE_SUMMARY_SNAPSHOTS)?;
    conn.execute_batch(CREATE_FINALIZATION_OUTBOX)?;
    migrate_project_core_legacy_columns(conn)?;
    migrate_character_roster_schema(conn)?;
    Ok(())
}

/// 读取表的现有列名集合（`PRAGMA table_info`）
fn table_columns(conn: &Connection, table: &str) -> SqlResult<HashSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = HashSet::new();
    for row in rows {
        columns.insert(row?);
    }
    Ok(columns)
}

/// project_core 旧库列补齐（对齐 `electron/database.ts` 的 `addProjectCoreTextColumn` 段）
///
/// 旧项目把作者配置字段映射到架构字段，导致重开漂移。新列保持独立事实：
/// 大纲与世界设定可从旧显示来源无损继承，主角档案绝不复制 `characters_arch`
/// （后者是结构化角色名单的派生投影）。
fn migrate_project_core_legacy_columns(conn: &Connection) -> SqlResult<()> {
    let mut columns = table_columns(conn, "project_core")?;

    // (列名, 旧数据来源列)
    let text_columns: [(&str, Option<&str>); 4] = [
        ("core_outline", Some("synopsis")),
        ("world_setting", Some("worldbuilding")),
        ("protagonist_profile", None),
        ("plot_tree_snapshot", None),
    ];
    for (column, legacy_source) in text_columns {
        if columns.contains(column) {
            continue;
        }
        conn.execute_batch(&format!(
            "ALTER TABLE project_core ADD COLUMN {column} TEXT NOT NULL DEFAULT ''"
        ))?;
        if let Some(source) = legacy_source {
            conn.execute_batch(&format!(
                "UPDATE project_core SET {column} = COALESCE({source}, '')"
            ))?;
        }
        columns.insert(column.to_string());
    }

    let typed_columns: [(&str, &str); 3] = [
        (
            "writing_language",
            "ALTER TABLE project_core ADD COLUMN writing_language TEXT NOT NULL DEFAULT 'zh-CN'",
        ),
        (
            "creative_strategy",
            "ALTER TABLE project_core ADD COLUMN creative_strategy TEXT NOT NULL DEFAULT 'auto'",
        ),
        (
            "narrative_thread_dormant_threshold",
            "ALTER TABLE project_core ADD COLUMN narrative_thread_dormant_threshold INTEGER NOT NULL DEFAULT 3",
        ),
    ];
    for (column, statement) in typed_columns {
        if columns.contains(column) {
            continue;
        }
        conn.execute_batch(statement)?;
        columns.insert(column.to_string());
    }

    Ok(())
}

/// 角色名单 schema 收敛（逐条对齐 `character-roster-schema.ts: ensureCharacterRosterSchema`）
///
/// `read` / `commit` 每次都会调用（与基线一致），因此列补齐与首次建档必须幂等：
/// 1. 建 `character_roster_meta` / `character_roster_operations`；
/// 2. 为旧库补 `characters.cs_provenance` 与 `character_roster_meta.fact_hash`；
/// 3. 首次建档时按「已有角色卡 → 旧图谱原文 → 空」判定迁移状态，原文只归档不解析。
pub fn migrate_character_roster_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(CREATE_CHARACTER_ROSTER)?;

    let character_columns = table_columns(conn, "characters")?;
    if !character_columns.contains("cs_provenance") {
        conn.execute_batch(
            "ALTER TABLE characters ADD COLUMN cs_provenance TEXT NOT NULL DEFAULT '{}'",
        )?;
    }

    let meta_columns = table_columns(conn, "character_roster_meta")?;
    if !meta_columns.contains("fact_hash") {
        conn.execute_batch(
            "ALTER TABLE character_roster_meta ADD COLUMN fact_hash TEXT NOT NULL DEFAULT ''",
        )?;
    }

    let has_meta: Option<i64> = conn
        .query_row("SELECT 1 FROM character_roster_meta WHERE id = 'main'", [], |row| {
            row.get(0)
        })
        .optional()?;
    if has_meta.is_some() {
        return Ok(());
    }

    let character_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM characters", [], |row| row.get(0))?;
    let legacy_markdown: String = conn
        .query_row(
            "SELECT COALESCE(characters_arch, '') FROM project_core WHERE id = 'main'",
            [],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_default();

    let migration_state = if character_count > 0 {
        "legacy_cards_preserved"
    } else if !legacy_markdown.trim().is_empty() {
        "legacy_markdown_pending"
    } else {
        "empty"
    };

    conn.execute(
        "INSERT INTO character_roster_meta (id, schema_version, revision, migration_state, legacy_markdown, projection_hash, fact_hash) VALUES ('main', 1, 0, ?1, ?2, '', '')",
        rusqlite::params![migration_state, legacy_markdown],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        Connection::open_in_memory().expect("内存库打开失败")
    }

    #[test]
    fn create_tables_is_idempotent_test() {
        let conn = memory_db();
        create_tables(&conn).expect("首次建表失败");
        create_tables(&conn).expect("重复建表必须幂等");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'project_core'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn project_core_has_full_final_column_set_test() {
        let conn = memory_db();
        create_tables(&conn).unwrap();
        let columns = table_columns(&conn, "project_core").unwrap();
        for expected in [
            "id",
            "project_name",
            "genre",
            "sub_genre",
            "target_audience",
            "total_chapters",
            "words_per_chapter",
            "writing_language",
            "creative_strategy",
            "narrative_thread_dormant_threshold",
            "plot_structure",
            "narrative_pov",
            "writing_style",
            "reference_works",
            "global_guidance",
            "golden_finger",
            "core_outline",
            "world_setting",
            "protagonist_profile",
            "premise",
            "worldbuilding",
            "characters_arch",
            "synopsis",
            "character_states",
            "plot_tree_snapshot",
            "created_at",
            "updated_at",
        ] {
            assert!(columns.contains(expected), "project_core 缺列: {expected}");
        }
    }

    #[test]
    fn legacy_project_core_gains_columns_and_inherits_legacy_sources_test() {
        let conn = memory_db();
        // 模拟 Electron 早期库：缺少 core_outline/world_setting/protagonist_profile 等列
        conn.execute_batch(
            r#"
            CREATE TABLE project_core (
              id TEXT PRIMARY KEY DEFAULT 'main',
              project_name TEXT NOT NULL DEFAULT '',
              synopsis TEXT DEFAULT '',
              worldbuilding TEXT DEFAULT '',
              characters_arch TEXT DEFAULT ''
            );
            INSERT INTO project_core (id, project_name, synopsis, worldbuilding)
            VALUES ('main', '旧项目', '旧大纲', '旧世界观');
            "#,
        )
        .unwrap();

        create_tables(&conn).unwrap();

        let columns = table_columns(&conn, "project_core").unwrap();
        for expected in [
            "core_outline",
            "world_setting",
            "protagonist_profile",
            "plot_tree_snapshot",
            "writing_language",
            "creative_strategy",
            "narrative_thread_dormant_threshold",
        ] {
            assert!(columns.contains(expected), "legacy 迁移缺列: {expected}");
        }

        let (core_outline, world_setting, protagonist_profile, writing_language, threshold): (
            String,
            String,
            String,
            String,
            i64,
        ) = conn
            .query_row(
                "SELECT core_outline, world_setting, protagonist_profile, writing_language, narrative_thread_dormant_threshold FROM project_core WHERE id = 'main'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        // 旧显示来源无损继承；主角档案不复制 characters_arch（保持空）
        assert_eq!(core_outline, "旧大纲");
        assert_eq!(world_setting, "旧世界观");
        assert_eq!(protagonist_profile, "");
        assert_eq!(writing_language, "zh-CN");
        assert_eq!(threshold, 3);
    }

    #[test]
    fn character_domain_tables_exist_test() {
        let conn = memory_db();
        create_tables(&conn).unwrap();
        for table in [
            "blueprints",
            "characters",
            "contents",
            "drafts",
            "finalization_outbox",
            "character_roster_meta",
            "character_roster_operations",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "缺少表: {table}");
        }
    }

    #[test]
    fn character_roster_meta_first_build_state_test() {
        // 空项目 → empty
        let empty = memory_db();
        create_tables(&empty).unwrap();
        let state: String = empty
            .query_row(
                "SELECT migration_state FROM character_roster_meta WHERE id = 'main'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "empty");

        // 旧图谱原文非空 → legacy_markdown_pending，且原文只归档不解析
        let legacy = memory_db();
        legacy.execute_batch(CREATE_PROJECT_CORE).unwrap();
        legacy
            .execute_batch(
                "INSERT INTO project_core (id, characters_arch) VALUES ('main', '# 旧角色图谱');",
            )
            .unwrap();
        create_tables(&legacy).unwrap();
        let (state, markdown): (String, String) = legacy
            .query_row(
                "SELECT migration_state, legacy_markdown FROM character_roster_meta WHERE id = 'main'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "legacy_markdown_pending");
        assert_eq!(markdown, "# 旧角色图谱");

        // 已有结构化角色卡 → legacy_cards_preserved
        let cards = memory_db();
        cards.execute_batch(CREATE_PROJECT_CORE).unwrap();
        cards.execute_batch(CREATE_CHARACTERS).unwrap();
        cards
            .execute("INSERT INTO characters (name) VALUES ('林清玄')", [])
            .unwrap();
        create_tables(&cards).unwrap();
        let state: String = cards
            .query_row(
                "SELECT migration_state FROM character_roster_meta WHERE id = 'main'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "legacy_cards_preserved");
    }

    #[test]
    fn legacy_roster_gains_columns_without_resetting_meta_test() {
        let conn = memory_db();
        // 模拟第一版 roster 元数据：characters 无 cs_provenance，meta 无 fact_hash
        conn.execute_batch(
            r#"
            CREATE TABLE characters (
              name TEXT PRIMARY KEY,
              role TEXT DEFAULT 'supporting'
            );
            CREATE TABLE character_roster_meta (
              id TEXT PRIMARY KEY CHECK (id = 'main'),
              schema_version INTEGER NOT NULL CHECK (schema_version = 1),
              revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
              migration_state TEXT NOT NULL CHECK (
                migration_state IN ('empty','legacy_cards_preserved','legacy_markdown_pending','ready')
              ),
              legacy_markdown TEXT NOT NULL DEFAULT '',
              projection_hash TEXT NOT NULL DEFAULT '',
              updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            INSERT INTO character_roster_meta (id, schema_version, revision, migration_state)
            VALUES ('main', 1, 7, 'ready');
            "#,
        )
        .unwrap();

        create_tables(&conn).unwrap();

        assert!(table_columns(&conn, "characters").unwrap().contains("cs_provenance"));
        assert!(table_columns(&conn, "character_roster_meta").unwrap().contains("fact_hash"));
        let (revision, state, fact_hash): (i64, String, String) = conn
            .query_row(
                "SELECT revision, migration_state, fact_hash FROM character_roster_meta WHERE id = 'main'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        // 既有元数据不被重置，只补列
        assert_eq!(revision, 7);
        assert_eq!(state, "ready");
        assert_eq!(fact_hash, "");
    }
}