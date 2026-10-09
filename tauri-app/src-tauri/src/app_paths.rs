//! 全局数据根与配置文件路径 —— 迁移自 `electron/utils/config-utils.ts`
//! 的路径常量部分（`VELA_HOME` / `GLOBAL_CONFIG_PATH` / `MODELS_CONFIG_PATH` /
//! `RECENT_PROJECTS_PATH` / `ensureVelaHome`）。
//!
//! **双栈隔离（AGENTS.md L1 决策，不可回退）**：Tauri 版（Lorekeeper）只认
//! `AI_NOVEL_LOREKEEPER_HOME`，缺省 `~/.lorekeeper`；**不读**基线的
//! `AI_NOVEL_VELA_HOME`、**不回退** `~/.vela`。两个应用可以在同一台机器上
//! 同时运行，否则 `config.json` / `models.json` / `recent-projects.json` /
//! `prompts/` 会被两个进程同时读写。

use std::path::{Path, PathBuf};

/// 环境变量名（Lorekeeper 专用，与基线的 `AI_NOVEL_VELA_HOME` 刻意不同）。
pub const LOREKEEPER_HOME_ENV: &str = "AI_NOVEL_LOREKEEPER_HOME";

/// 缺省数据根目录名（`~/.lorekeeper`）。
const LOREKEEPER_HOME_DIR: &str = ".lorekeeper";

/// 纯函数核心：环境变量优先（去空白后非空即生效），否则 `<用户主目录>/.lorekeeper`。
///
/// 基线等价物为 `process.env.AI_NOVEL_VELA_HOME?.trim() || path.join(os.homedir(), '.vela')`；
/// 这里把两个输入显式传入，便于在**不污染进程环境**的前提下测试。
fn resolve_home(env_home: Option<&str>, user_home: Option<&str>) -> PathBuf {
    if let Some(home) = env_home {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    let user_home = user_home.unwrap_or_default();
    PathBuf::from(user_home.trim()).join(LOREKEEPER_HOME_DIR)
}

/// Lorekeeper 全局数据根（`AI_NOVEL_LOREKEEPER_HOME` 或 `~/.lorekeeper`）。
pub fn lorekeeper_home() -> PathBuf {
    let env_home = std::env::var(LOREKEEPER_HOME_ENV).ok();
    let user_home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok();
    resolve_home(env_home.as_deref(), user_home.as_deref())
}

/// `config.json` 路径（注入数据根，便于测试与将来的迁移导入）。
pub fn global_config_path_at(home: &Path) -> PathBuf {
    home.join("config.json")
}

/// `models.json` 路径（注入数据根）。
pub fn models_config_path_at(home: &Path) -> PathBuf {
    home.join("models.json")
}

/// `recent-projects.json` 路径（注入数据根）。
pub fn recent_projects_path_at(home: &Path) -> PathBuf {
    home.join("recent-projects.json")
}

/// 全局配置路径（真实数据根）。
pub fn global_config_path() -> PathBuf {
    global_config_path_at(&lorekeeper_home())
}

/// 模型配置路径（真实数据根）。
pub fn models_config_path() -> PathBuf {
    models_config_path_at(&lorekeeper_home())
}

/// 最近项目路径（真实数据根）。
pub fn recent_projects_path() -> PathBuf {
    recent_projects_path_at(&lorekeeper_home())
}

/// 对齐基线 `ensureVelaHome()`：保证数据根与 `prompts/`、`logs/` 目录存在。
///
/// 基线在应用启动时调用一次；这里同样在 Tauri `setup` 中调用，使
/// `~/.lorekeeper` 在首次启动即存在（而不是等到第一次写配置）。
pub fn ensure_lorekeeper_home() -> Result<(), String> {
    ensure_lorekeeper_home_at(&lorekeeper_home())
}

/// 注入数据根版本，便于测试。
pub fn ensure_lorekeeper_home_at(home: &Path) -> Result<(), String> {
    for directory in [home.to_path_buf(), home.join("prompts"), home.join("logs")] {
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("无法创建数据目录 {}：{}", directory.display(), error))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lorekeeper-app-paths-{}-{}",
            tag,
            crate::project_access::random_uuid_v4()
        ));
        dir
    }

    #[test]
    fn env_home_wins_and_is_trimmed_test() {
        assert_eq!(
            resolve_home(Some("  F:/novel-home  "), Some("C:/Users/demo")),
            PathBuf::from("F:/novel-home")
        );
    }

    #[test]
    fn blank_env_home_falls_back_to_user_home_test() {
        // 对齐基线 `?.trim() || ...`：空白字符串视为未设置。
        assert_eq!(
            resolve_home(Some("   "), Some("C:/Users/demo")),
            PathBuf::from("C:/Users/demo").join(".lorekeeper")
        );
        assert_eq!(
            resolve_home(None, Some("C:/Users/demo")),
            PathBuf::from("C:/Users/demo").join(".lorekeeper")
        );
    }

    #[test]
    fn missing_user_home_still_yields_relative_lorekeeper_dir_test() {
        // 与基线不同：基线 os.homedir() 恒非空；这里退化为相对路径，仍以 `.lorekeeper` 结尾。
        assert_eq!(resolve_home(None, None), PathBuf::from(".lorekeeper"));
    }

    #[test]
    fn path_helpers_are_derived_from_home_test() {
        let home = PathBuf::from("D:/data/lorekeeper");
        assert_eq!(
            global_config_path_at(&home),
            PathBuf::from("D:/data/lorekeeper/config.json")
        );
        assert_eq!(
            models_config_path_at(&home),
            PathBuf::from("D:/data/lorekeeper/models.json")
        );
        assert_eq!(
            recent_projects_path_at(&home),
            PathBuf::from("D:/data/lorekeeper/recent-projects.json")
        );
    }

    #[test]
    fn ensure_lorekeeper_home_creates_expected_dirs_test() {
        let home = temp_home("ensure");
        let _ = std::fs::remove_dir_all(&home);

        ensure_lorekeeper_home_at(&home).unwrap();

        assert!(home.is_dir());
        assert!(home.join("prompts").is_dir());
        assert!(home.join("logs").is_dir());

        // 幂等：重复调用不报错
        ensure_lorekeeper_home_at(&home).unwrap();

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn ensure_lorekeeper_home_reports_unusable_parent_test() {
        let home = temp_home("unusable");
        let _ = std::fs::remove_dir_all(&home);
        // 用同名文件占位，使 create_dir_all 必然失败
        std::fs::write(&home, b"not a directory").unwrap();

        let error = ensure_lorekeeper_home_at(&home).unwrap_err();
        assert!(error.contains("无法创建数据目录"), "实际文案：{error}");

        let _ = std::fs::remove_file(&home);
    }
}
