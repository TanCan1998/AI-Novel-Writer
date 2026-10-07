//! 项目数据库连接层 —— 等价于 `electron/database.ts` 的 init/close/get
//!
//! 一个进程同一时刻只持有一个项目库连接（与 Electron 基线一致），
//! 库文件位于 `<projectRoot>/.vela/vela.db`，采用 WAL + 外键约束。

pub mod schema;

use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// 项目库目录名（相对项目根）
pub const PROJECT_DIR_NAME: &str = ".vela";
/// 项目库文件名
pub const PROJECT_DB_FILE_NAME: &str = "vela.db";

/// 已打开的项目数据库（连接 + 归属项目根）
#[derive(Debug)]
pub struct ProjectDatabase {
    /// 归属项目根路径（错误文案与后续子域用；当前调用点均直接持有路径，
    /// 故读取点未启用，暂抑制死代码警告）。
    #[allow(dead_code)]
    root_path: PathBuf,
    conn: Connection,
}

impl ProjectDatabase {
    /// 打开（或创建）项目库并确保表结构就绪
    ///
    /// 与基线 `initProjectDatabase` 行为对齐：建目录 → 打开 → WAL → 外键 → 建表。
    pub fn open(project_root: &Path) -> Result<Self, String> {
        let db_path = project_database_path(project_root);
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("创建项目数据库目录失败：{error}"))?;
        }

        let conn = Connection::open(&db_path)
            .map_err(|error| format!("打开项目数据库失败（{}）：{error}", db_path.display()))?;

        // WAL：query_row 读取返回的模式值（execute_batch 无法接收 PRAGMA 结果）
        let _mode: String = conn
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .map_err(|error| format!("启用 WAL 失败：{error}"))?;
        conn.execute_batch("PRAGMA foreign_keys = ON")
            .map_err(|error| format!("启用外键约束失败：{error}"))?;

        schema::create_tables(&conn).map_err(|error| format!("建表失败：{error}"))?;

        Ok(Self {
            root_path: project_root.to_path_buf(),
            conn,
        })
    }

    /// 项目根路径（供后续子域校验归属，当前暂无调用点）
    #[allow(dead_code)]
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    /// 数据库连接（仓储层使用）
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

/// 项目库文件绝对路径
pub fn project_database_path(project_root: &Path) -> PathBuf {
    project_root.join(PROJECT_DIR_NAME).join(PROJECT_DB_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("anw-db-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn open_creates_vela_database_file_and_tables_test() {
        let root = temp_project_root("open");
        let db = ProjectDatabase::open(&root).expect("打开项目库失败");

        let db_path = project_database_path(&root);
        assert!(db_path.exists(), "应在 .vela/vela.db 建库");

        // project_core 表已建好
        let count: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'project_core'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reopen_keeps_existing_rows_test() {
        let root = temp_project_root("reopen");
        {
            let db = ProjectDatabase::open(&root).unwrap();
            db.connection()
                .execute(
                    "INSERT INTO project_core (id, project_name) VALUES ('main', '复用项目')",
                    [],
                )
                .unwrap();
        }

        let db = ProjectDatabase::open(&root).unwrap();
        let name: String = db
            .connection()
            .query_row("SELECT project_name FROM project_core WHERE id = 'main'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(name, "复用项目");

        let _ = std::fs::remove_dir_all(&root);
    }
}