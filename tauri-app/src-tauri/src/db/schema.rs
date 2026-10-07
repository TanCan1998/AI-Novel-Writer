//! 项目数据库 schema —— 分批平移自 `electron/database.ts` 的 `createTables`
//!
//! 约束（与 Electron 基线双栈互通）：
//! 1. 建表必须幂等（`IF NOT EXISTS`），因为 Electron 与 Tauri 都会打开同一个 `.vela/vela.db`；
//! 2. 列定义必须与 Electron 侧的**最终列集**一致（含历史 ALTER 追加列），
//!    否则后打开的栈会读到不一致结构；
//! 3. 旧库（Electron 早期版本建的）通过 legacy 列补齐迁移收敛，迁移规则逐字对齐基线。
//!
//! 批次 C 按子域推进：本文件只含 project_core 域，后续子域建表陆续追加。

use rusqlite::{Connection, Result as SqlResult};
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

/// 建表入口（幂等）：所有分批 DDL 在此汇总执行后再跑迁移
pub fn create_tables(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(CREATE_PROJECT_CORE)?;
    migrate_project_core_legacy_columns(conn)?;
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
              worldbuilding TEXT DEFAULT ''
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
}