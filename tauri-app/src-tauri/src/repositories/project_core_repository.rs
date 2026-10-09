//! ProjectCoreRepository —— 项目主台账（`project_core` 表）
//!
//! 平移自 `electron/repositories/project-core-repository.ts`：合并 NovelConfig 与架构四大件的
//! 统一读写，表中恒只有一行（`id = 'main'`）。
//!
//! 语义对齐要点：
//! - `characters_arch` 是角色名单的派生投影，**拒绝**通过本仓储写入（基线同样拒绝）；
//! - `writing_language` / `creative_strategy` 读取时做白名单收敛，非法值回落默认；
//! - 休眠阈值读取时收敛到 [1, 50]，写入时先归一化；
//! - `commit_synopsis` 是带前置条件（expected）的乐观并发提交，仅当全部字段未变才写入。

use rusqlite::{Connection, Row};
use serde::Deserialize;
use serde_json::{Map, Value};

/// 默认写作语言
pub const DEFAULT_WRITING_LANGUAGE: &str = "zh-CN";
/// 写作语言白名单（对齐 `src/shared/writing-language.ts`）
pub const WRITING_LANGUAGES: [&str; 2] = ["zh-CN", "en-US"];
/// 创作策略白名单（对齐 `src/shared/reasoning-types.ts`）
pub const CREATIVE_STRATEGIES: [&str; 4] = [
    "auto",
    "fluent-drafting",
    "consistency-first",
    "deep-planning",
];
/// 休眠阈值默认值（对齐 `src/shared/narrative-thread.ts`）
pub const DEFAULT_NARRATIVE_THREAD_DORMANT_THRESHOLD: i64 = 3;
/// 休眠阈值下界
pub const MIN_NARRATIVE_THREAD_DORMANT_THRESHOLD: i64 = 1;
/// 休眠阈值上界
pub const MAX_NARRATIVE_THREAD_DORMANT_THRESHOLD: i64 = 50;

/// `characters_arch` 写入拒绝文案（与基线逐字一致）
pub const CHARACTERS_ARCH_READONLY_MESSAGE: &str =
    "角色图谱由角色名单自动生成；请通过角色管理修改角色资料";

/// 前端使用的驼峰命名数据结构（字段顺序与基线接口一致）
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCoreData {
    pub project_name: String,
    pub genre: String,
    pub sub_genre: String,
    pub target_audience: String,
    pub total_chapters: i64,
    pub words_per_chapter: i64,
    pub writing_language: String,
    pub creative_strategy: String,
    pub narrative_thread_dormant_chapter_threshold: i64,
    pub plot_structure: String,
    pub narrative_pov: String,
    pub writing_style: String,
    pub reference_works: String,
    pub global_guidance: String,
    pub golden_finger: String,
    pub core_outline: String,
    pub world_setting: String,
    pub protagonist_profile: String,
    pub premise: String,
    pub worldbuilding: String,
    pub characters_arch: String,
    pub synopsis: String,
    pub character_states: String,
}

/// 乐观并发提交的前置快照（对齐基线 `ProjectCoreSynopsisExpected`）
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCoreSynopsisExpected {
    pub synopsis: String,
    pub premise: String,
    pub characters_arch: String,
    pub worldbuilding: String,
    pub genre: String,
    pub total_chapters: i64,
    pub words_per_chapter: i64,
    pub writing_language: String,
    pub plot_structure: String,
    pub narrative_pov: String,
    pub global_guidance: String,
}

/// 情节大纲提交请求
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCoreSynopsisCommitRequest {
    pub synopsis: String,
    pub expected: ProjectCoreSynopsisExpected,
}

/// 字段映射：前端驼峰名 → 数据库列名
const FIELD_MAP: [(&str, &str); 22] = [
    ("projectName", "project_name"),
    ("genre", "genre"),
    ("subGenre", "sub_genre"),
    ("targetAudience", "target_audience"),
    ("totalChapters", "total_chapters"),
    ("wordsPerChapter", "words_per_chapter"),
    ("writingLanguage", "writing_language"),
    ("creativeStrategy", "creative_strategy"),
    (
        "narrativeThreadDormantChapterThreshold",
        "narrative_thread_dormant_threshold",
    ),
    ("plotStructure", "plot_structure"),
    ("narrativePov", "narrative_pov"),
    ("writingStyle", "writing_style"),
    ("referenceWorks", "reference_works"),
    ("globalGuidance", "global_guidance"),
    ("goldenFinger", "golden_finger"),
    ("coreOutline", "core_outline"),
    ("worldSetting", "world_setting"),
    ("protagonistProfile", "protagonist_profile"),
    ("premise", "premise"),
    ("worldbuilding", "worldbuilding"),
    ("synopsis", "synopsis"),
    ("characterStates", "character_states"),
];

/// 写入时被拒绝的派生投影字段
const READONLY_FIELDS: [&str; 1] = ["charactersArch"];

/// 写作语言收敛（非法值回落默认）
pub fn resolve_writing_language(value: &str) -> String {
    if WRITING_LANGUAGES.contains(&value) {
        value.to_string()
    } else {
        DEFAULT_WRITING_LANGUAGE.to_string()
    }
}

/// 创作策略收敛（非法值回落 auto）
pub fn resolve_creative_strategy(value: &str) -> String {
    if CREATIVE_STRATEGIES.contains(&value) {
        value.to_string()
    } else {
        "auto".to_string()
    }
}

/// 休眠阈值收敛：非整数或非有限值回落默认，超界截断到 [1, 50]
pub fn resolve_narrative_thread_dormant_threshold(value: &Value) -> i64 {
    let Some(number) = value.as_f64() else {
        return DEFAULT_NARRATIVE_THREAD_DORMANT_THRESHOLD;
    };
    if !number.is_finite() {
        return DEFAULT_NARRATIVE_THREAD_DORMANT_THRESHOLD;
    }
    (number.trunc() as i64).clamp(
        MIN_NARRATIVE_THREAD_DORMANT_THRESHOLD,
        MAX_NARRATIVE_THREAD_DORMANT_THRESHOLD,
    )
}

/// JSON 值 → SQL 绑定值（`None` 表示该 JSON 类型无法绑定）
fn json_to_sql_value(value: &Value) -> Result<rusqlite::types::Value, String> {
    use rusqlite::types::Value as SqlValue;
    match value {
        Value::Null => Ok(SqlValue::Null),
        Value::Bool(flag) => Ok(SqlValue::Integer(i64::from(*flag))),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                Ok(SqlValue::Integer(integer))
            } else if let Some(real) = number.as_f64() {
                Ok(SqlValue::Real(real))
            } else {
                Err(format!("不支持的数字值：{number}"))
            }
        }
        Value::String(text) => Ok(SqlValue::Text(text.clone())),
        other => Err(format!(
            "项目配置字段只接受字符串、数字或空值，收到：{other}"
        )),
    }
}

/// 数据库行 → 前端数据结构
///
/// 历史列允许 NULL（基线 DDL 多为 `DEFAULT ''` 而非 `NOT NULL`），这里与基线保持一致地
/// 给出空串/默认数值，避免旧库缺省值导致整条读取失败。
fn row_to_data(row: &Row<'_>) -> rusqlite::Result<ProjectCoreData> {
    let text = |column: &str| -> rusqlite::Result<String> {
        Ok(row.get::<_, Option<String>>(column)?.unwrap_or_default())
    };
    Ok(ProjectCoreData {
        project_name: text("project_name")?,
        genre: text("genre")?,
        sub_genre: text("sub_genre")?,
        target_audience: text("target_audience")?,
        total_chapters: row.get::<_, Option<i64>>("total_chapters")?.unwrap_or(100),
        words_per_chapter: row
            .get::<_, Option<i64>>("words_per_chapter")?
            .unwrap_or(3000),
        writing_language: resolve_writing_language(&text("writing_language")?),
        creative_strategy: resolve_creative_strategy(&text("creative_strategy")?),
        narrative_thread_dormant_chapter_threshold: resolve_narrative_thread_dormant_threshold(
            &Value::from(
                row.get::<_, Option<i64>>("narrative_thread_dormant_threshold")?
                    .unwrap_or(DEFAULT_NARRATIVE_THREAD_DORMANT_THRESHOLD),
            ),
        ),
        plot_structure: text("plot_structure")?,
        narrative_pov: text("narrative_pov")?,
        writing_style: text("writing_style")?,
        reference_works: text("reference_works")?,
        global_guidance: text("global_guidance")?,
        golden_finger: text("golden_finger")?,
        core_outline: text("core_outline")?,
        world_setting: text("world_setting")?,
        protagonist_profile: text("protagonist_profile")?,
        premise: text("premise")?,
        worldbuilding: text("worldbuilding")?,
        characters_arch: text("characters_arch")?,
        synopsis: text("synopsis")?,
        character_states: text("character_states")?,
    })
}

const SELECT_ALL_COLUMNS: &str = "SELECT id, project_name, genre, sub_genre, target_audience, \
     total_chapters, words_per_chapter, writing_language, creative_strategy, \
     narrative_thread_dormant_threshold, plot_structure, narrative_pov, writing_style, \
     reference_works, global_guidance, golden_finger, core_outline, world_setting, \
     protagonist_profile, premise, worldbuilding, characters_arch, synopsis, character_states \
     FROM project_core WHERE id = ?1";

/// 获取项目配置（不存在则 `None`）
pub fn get(conn: &Connection) -> Result<Option<ProjectCoreData>, String> {
    let mut stmt = conn
        .prepare(SELECT_ALL_COLUMNS)
        .map_err(|error| format!("读取项目配置失败：{error}"))?;
    let mut rows = stmt
        .query(["main"])
        .map_err(|error| format!("读取项目配置失败：{error}"))?;
    match rows
        .next()
        .map_err(|error| format!("读取项目配置失败：{error}"))?
    {
        Some(row) => Ok(Some(row_to_data(row).map_err(|error| error.to_string())?)),
        None => Ok(None),
    }
}

/// 初始化项目配置（创建项目时调用，幂等）
pub fn init(conn: &Connection, project_name: &str, writing_language: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO project_core (id, project_name, writing_language) VALUES ('main', ?1, ?2)",
        rusqlite::params![project_name, resolve_writing_language(writing_language)],
    )
    .map_err(|error| format!("初始化项目配置失败：{error}"))?;
    Ok(())
}

/// 更新项目配置（传入部分字段即可）
///
/// 参数为前端原始 JSON 对象，键为驼峰字段名；未知键忽略（与基线一致，基线按映射表遍历）。
pub fn update(conn: &Connection, data: &Map<String, Value>) -> Result<(), String> {
    for field in READONLY_FIELDS {
        if data.contains_key(field) {
            return Err(CHARACTERS_ARCH_READONLY_MESSAGE.to_string());
        }
    }

    let mut set_clauses: Vec<String> = Vec::new();
    let mut values: Vec<rusqlite::types::Value> = Vec::new();

    for (field, column) in FIELD_MAP {
        let Some(value) = data.get(field) else {
            continue;
        };
        set_clauses.push(format!("{column} = ?"));
        if field == "narrativeThreadDormantChapterThreshold" {
            values.push(rusqlite::types::Value::Integer(
                resolve_narrative_thread_dormant_threshold(value),
            ));
        } else {
            values.push(json_to_sql_value(value)?);
        }
    }

    if set_clauses.is_empty() {
        return Ok(());
    }

    set_clauses.push("updated_at = datetime('now')".to_string());
    values.push(rusqlite::types::Value::Text("main".to_string()));

    let sql = format!(
        "UPDATE project_core SET {} WHERE id = ?",
        set_clauses.join(", ")
    );
    conn.execute(&sql, rusqlite::params_from_iter(values))
        .map_err(|error| format!("更新项目配置失败：{error}"))?;
    Ok(())
}

/// 乐观并发提交情节大纲：仅当 `expected` 快照与库中当前值完全一致才写入
///
/// 返回是否写入成功（`false` = 项目数据已变化）。
pub fn commit_synopsis(
    conn: &Connection,
    request: &ProjectCoreSynopsisCommitRequest,
) -> Result<bool, String> {
    let expected = &request.expected;
    let changed = conn
        .execute(
            "UPDATE project_core \
             SET synopsis = ?1, updated_at = datetime('now') \
             WHERE id = 'main' \
               AND synopsis = ?2 \
               AND premise = ?3 \
               AND characters_arch = ?4 \
               AND worldbuilding = ?5 \
               AND genre = ?6 \
               AND total_chapters = ?7 \
               AND words_per_chapter = ?8 \
               AND CASE WHEN writing_language = 'en-US' THEN 'en-US' ELSE 'zh-CN' END = ?9 \
               AND plot_structure = ?10 \
               AND narrative_pov = ?11 \
               AND global_guidance = ?12",
            rusqlite::params![
                request.synopsis,
                expected.synopsis,
                expected.premise,
                expected.characters_arch,
                expected.worldbuilding,
                expected.genre,
                expected.total_chapters,
                expected.words_per_chapter,
                resolve_writing_language(&expected.writing_language),
                expected.plot_structure,
                expected.narrative_pov,
                expected.global_guidance,
            ],
        )
        .map_err(|error| format!("提交情节大纲失败：{error}"))?;
    Ok(changed == 1)
}

/// 清空由架构/导入/AI 分析生成的创作字段，保留项目名、章节规模与基础偏好
///
/// 对应 `db:project-clear-generated-data`（批次 C 后续子域）；仓储先行落地以保证
/// 行为与基线一致，暂抑制死代码警告。
#[allow(dead_code)]
pub fn reset_creative_fields(conn: &Connection) -> Result<(), String> {
    let mut data = Map::new();
    for field in [
        "writingStyle",
        "referenceWorks",
        "globalGuidance",
        "goldenFinger",
        "coreOutline",
        "worldSetting",
        "protagonistProfile",
        "premise",
        "worldbuilding",
        "synopsis",
        "characterStates",
    ] {
        data.insert(field.to_string(), Value::String(String::new()));
    }
    update(conn, &data)
}

/// 情节大纲被并发修改时的拒绝文案（与基线逐字一致）
pub const SYNOPSIS_CONFLICT_MESSAGE: &str = "项目数据已变化，已拒绝覆盖情节大纲";

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        conn
    }

    fn update_from_pairs(pairs: &[(&str, Value)]) -> Map<String, Value> {
        let mut data = Map::new();
        for (key, value) in pairs {
            data.insert((*key).to_string(), value.clone());
        }
        data
    }

    #[test]
    fn get_returns_none_before_init_test() {
        let conn = seeded_db();
        assert!(get(&conn).unwrap().is_none(), "未初始化时应返回 None");
    }

    #[test]
    fn init_is_idempotent_and_applies_defaults_test() {
        let conn = seeded_db();
        init(&conn, "首名", DEFAULT_WRITING_LANGUAGE).unwrap();
        init(&conn, "改名无效", DEFAULT_WRITING_LANGUAGE).unwrap();

        let data = get(&conn).unwrap().expect("初始化后应可读取");
        assert_eq!(data.project_name, "首名", "INSERT OR IGNORE 不应覆盖已有行");
        assert_eq!(data.writing_language, "zh-CN");
        assert_eq!(data.creative_strategy, "auto");
        assert_eq!(data.narrative_thread_dormant_chapter_threshold, 3);
        assert_eq!(data.total_chapters, 100);
        assert_eq!(data.words_per_chapter, 3000);
    }

    #[test]
    fn init_normalizes_invalid_writing_language_test() {
        let conn = seeded_db();
        init(&conn, "项目", "fr-FR").unwrap();
        assert_eq!(get(&conn).unwrap().unwrap().writing_language, "zh-CN");
    }

    #[test]
    fn update_only_touches_provided_fields_test() {
        let conn = seeded_db();
        init(&conn, "项目", "zh-CN").unwrap();

        update(
            &conn,
            &update_from_pairs(&[
                ("genre", Value::String("仙侠".into())),
                ("totalChapters", Value::from(240)),
                ("narrativeThreadDormantChapterThreshold", Value::from(99)),
            ]),
        )
        .unwrap();

        let data = get(&conn).unwrap().unwrap();
        assert_eq!(data.genre, "仙侠");
        assert_eq!(data.total_chapters, 240);
        // 阈值收敛到上界 50
        assert_eq!(data.narrative_thread_dormant_chapter_threshold, 50);
        // 未传字段保持默认
        assert_eq!(data.sub_genre, "");
        assert_eq!(data.project_name, "项目");
    }

    #[test]
    fn update_with_empty_payload_is_noop_test() {
        let conn = seeded_db();
        init(&conn, "项目", "zh-CN").unwrap();
        update(&conn, &Map::new()).unwrap();
        assert_eq!(get(&conn).unwrap().unwrap().project_name, "项目");
    }

    #[test]
    fn update_rejects_characters_arch_test() {
        let conn = seeded_db();
        init(&conn, "项目", "zh-CN").unwrap();
        let error = update(
            &conn,
            &update_from_pairs(&[("charactersArch", Value::String("派生投影".into()))]),
        )
        .unwrap_err();
        assert_eq!(error, CHARACTERS_ARCH_READONLY_MESSAGE);
    }

    #[test]
    fn update_rejects_non_scalar_values_test() {
        let conn = seeded_db();
        init(&conn, "项目", "zh-CN").unwrap();
        let error = update(
            &conn,
            &update_from_pairs(&[("genre", Value::from(vec![1, 2]))]),
        )
        .unwrap_err();
        assert!(
            error.contains("只接受字符串、数字或空值"),
            "错误文案不符：{error}"
        );
    }

    #[test]
    fn update_ignores_unknown_fields_test() {
        let conn = seeded_db();
        init(&conn, "项目", "zh-CN").unwrap();
        update(
            &conn,
            &update_from_pairs(&[("unknownField", Value::String("x".into()))]),
        )
        .unwrap();
        assert_eq!(get(&conn).unwrap().unwrap().project_name, "项目");
    }

    #[test]
    fn resolve_dormant_threshold_matches_baseline_test() {
        // 非数字 → 默认 3；越界截断；小数截断
        assert_eq!(
            resolve_narrative_thread_dormant_threshold(&Value::String("5".into())),
            3
        );
        assert_eq!(resolve_narrative_thread_dormant_threshold(&Value::Null), 3);
        assert_eq!(
            resolve_narrative_thread_dormant_threshold(&Value::from(0)),
            1
        );
        assert_eq!(
            resolve_narrative_thread_dormant_threshold(&Value::from(-7)),
            1
        );
        assert_eq!(
            resolve_narrative_thread_dormant_threshold(&Value::from(51)),
            50
        );
        assert_eq!(
            resolve_narrative_thread_dormant_threshold(&Value::from(7.9)),
            7
        );
    }

    fn synopsis_request(synopsis: &str) -> ProjectCoreSynopsisCommitRequest {
        ProjectCoreSynopsisCommitRequest {
            synopsis: synopsis.to_string(),
            expected: ProjectCoreSynopsisExpected {
                synopsis: String::new(),
                premise: String::new(),
                characters_arch: String::new(),
                worldbuilding: String::new(),
                genre: String::new(),
                total_chapters: 100,
                words_per_chapter: 3000,
                writing_language: "zh-CN".to_string(),
                plot_structure: "three_act".to_string(),
                narrative_pov: "third_limited".to_string(),
                global_guidance: String::new(),
            },
        }
    }

    #[test]
    fn commit_synopsis_writes_when_expected_matches_test() {
        let conn = seeded_db();
        init(&conn, "项目", "zh-CN").unwrap();
        let request = synopsis_request("新大纲");
        assert!(
            commit_synopsis(&conn, &request).unwrap(),
            "快照一致时应写入"
        );
        assert_eq!(get(&conn).unwrap().unwrap().synopsis, "新大纲");
    }

    #[test]
    fn commit_synopsis_refuses_when_expected_stale_test() {
        let conn = seeded_db();
        init(&conn, "项目", "zh-CN").unwrap();
        update(
            &conn,
            &update_from_pairs(&[("genre", Value::String("科幻".into()))]),
        )
        .unwrap();

        // expected 中 genre 仍为空 → 与库中不符，拒绝覆盖
        let request = synopsis_request("不得写入");
        assert!(!commit_synopsis(&conn, &request).unwrap(), "快照过期应拒绝");
        assert_eq!(get(&conn).unwrap().unwrap().synopsis, "");
    }

    #[test]
    fn commit_synopsis_normalizes_expected_writing_language_test() {
        let conn = seeded_db();
        init(&conn, "项目", "zh-CN").unwrap();
        let mut request = synopsis_request("大纲");
        // 基线用 CASE 把非 en-US 归一为 zh-CN，故 expected 传 zh-CN 亦可匹配
        request.expected.writing_language = "zh-CN".to_string();
        assert!(commit_synopsis(&conn, &request).unwrap());
    }

    #[test]
    fn reset_creative_fields_keeps_identity_and_scale_test() {
        let conn = seeded_db();
        init(&conn, "项目", "zh-CN").unwrap();
        update(
            &conn,
            &update_from_pairs(&[
                ("genre", Value::String("仙侠".into())),
                ("totalChapters", Value::from(180)),
                ("writingStyle", Value::String("冷峻白描".into())),
                ("premise", Value::String("前提".into())),
                ("synopsis", Value::String("大纲".into())),
                ("worldbuilding", Value::String("世界观".into())),
            ]),
        )
        .unwrap();

        reset_creative_fields(&conn).unwrap();

        let data = get(&conn).unwrap().unwrap();
        // 创作字段清空
        assert_eq!(data.writing_style, "");
        assert_eq!(data.premise, "");
        assert_eq!(data.synopsis, "");
        assert_eq!(data.worldbuilding, "");
        // 项目名、章节规模、流派与基础偏好保留
        assert_eq!(data.project_name, "项目");
        assert_eq!(data.genre, "仙侠");
        assert_eq!(data.total_chapters, 180);
        assert_eq!(data.writing_language, "zh-CN");
        assert_eq!(data.plot_structure, "three_act");
    }
}
