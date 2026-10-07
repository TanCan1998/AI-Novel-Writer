//! CharacterRepository —— 角色卡（`characters` 表）
//!
//! 平移自 `electron/repositories/character-repository.ts`：
//! - `currentState` 子结构已拍平为 `cs_*` 前缀列，杜绝 JSON 大字段；
//! - `cs_updated_at_chapter` 为 NULL 表示没有 `currentState`（第 0 章是合法状态值）；
//! - `getAll` 按角色定位排序（主角 → 配角 → 反派 → 龙套）；
//! - `saveAll` 承担改名的一致性校验与蓝图引用同步（两阶段临时键防主键冲突）。

use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::character_role::{normalize_character_role, CharacterRole};

/// 定稿来源收据 —— 对齐 `src/shared/finalized-continuity.ts` 的 `FinalizedSourceIdentity`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedSourceIdentity {
    pub draft_id: i64,
    pub finalization_id: String,
    pub chapter_number: i64,
    pub content_hash: String,
}

/// `currentState` 字段级来源（对齐 `CharacterStateFieldProvenance`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CharacterStateFieldProvenance {
    /// 作者手改；记下生效章节号
    Author {
        #[serde(rename = "chapterNumber")]
        chapter_number: i64,
    },
    /// 后处理从定稿派生；携带冻结来源收据
    Derived { source: FinalizedSourceIdentity },
    /// 早期数据迁移而来，无来源可考
    Legacy,
}

/// `currentState` 的六个文本字段（对齐 `CHARACTER_STATE_TEXT_FIELDS`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CharacterStateTextField {
    Location,
    PowerLevel,
    PhysicalState,
    MentalState,
    KeyItems,
    RecentEvents,
}

/// 字段级来源映射（键为字段名；未知历史键原样保留以维持哈希一致）。
///
/// 用 `BTreeMap` 而非 `HashMap`：该映射会参与角色名单的规范 JSON 哈希，
/// 键序必须可复现，否则同一事实每次读回的 `fact_hash` 都会漂移。
pub type CharacterStateProvenance = BTreeMap<String, CharacterStateFieldProvenance>;

/// 角色卡动态状态（对齐 `CharacterRosterCharacterState`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterStateData {
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub power_level: String,
    #[serde(default)]
    pub physical_state: String,
    #[serde(default)]
    pub mental_state: String,
    #[serde(default)]
    pub key_items: String,
    #[serde(default)]
    pub recent_events: String,
    pub updated_at_chapter: i64,
    /// 空映射必须省略键（对齐基线：空 provenance 不序列化）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<CharacterStateProvenance>,
}

/// 角色卡完整数据（前端驼峰接口）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterData {
    pub name: String,
    #[serde(default = "default_character_role")]
    pub role: CharacterRole,
    #[serde(default)]
    pub gender: String,
    #[serde(default)]
    pub age: String,
    #[serde(default)]
    pub appearance: String,
    #[serde(default)]
    pub personality: String,
    #[serde(default)]
    pub background: String,
    #[serde(default)]
    pub abilities: String,
    #[serde(default)]
    pub motivation: String,
    #[serde(default)]
    pub relationships: String,
    #[serde(default)]
    pub arc: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_state: Option<CharacterStateData>,
}

fn default_character_role() -> CharacterRole {
    CharacterRole::Supporting
}

// 已平移但暂无调用点：由后续批次（drafts / reviews / post-process / import）接线。
#[allow(dead_code)]
/// 角色改名意图（对齐 `CharacterRenameData`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterRenameData {
    pub original_name: String,
    pub new_name: String,
}

/// 解析 `cs_provenance`：非法/空/非对象一律按空映射处理（不影响其它列回读）。
fn parse_provenance(value: Option<String>) -> Option<CharacterStateProvenance> {
    let text = value.unwrap_or_default();
    if text.trim().is_empty() {
        return None;
    }
    let parsed: serde_json::Value = match serde_json::from_str(&text) {
        Ok(parsed) => parsed,
        Err(_) => return None,
    };
    if !parsed.is_object() {
        return None;
    }
    let map: CharacterStateProvenance = match serde_json::from_value(parsed) {
        Ok(map) => map,
        Err(_) => return None,
    };
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

/// provenance → 落库文本：空映射写 `{}`（对齐基线 `JSON.stringify(state.provenance ?? {})`）。
pub fn provenance_to_text(provenance: Option<&CharacterStateProvenance>) -> String {
    match provenance {
        Some(map) if !map.is_empty() => serde_json::to_string(map).unwrap_or_else(|_| "{}".to_string()),
        _ => "{}".to_string(),
    }
}

fn row_to_data(row: &rusqlite::Row) -> rusqlite::Result<CharacterData> {
    let updated_at_chapter: Option<i64> = row.get("cs_updated_at_chapter")?;
    let current_state = match updated_at_chapter {
        Some(updated_at_chapter) => Some(CharacterStateData {
            location: row.get::<_, Option<String>>("cs_location")?.unwrap_or_default(),
            power_level: row.get::<_, Option<String>>("cs_power_level")?.unwrap_or_default(),
            physical_state: row.get::<_, Option<String>>("cs_physical_state")?.unwrap_or_default(),
            mental_state: row.get::<_, Option<String>>("cs_mental_state")?.unwrap_or_default(),
            key_items: row.get::<_, Option<String>>("cs_key_items")?.unwrap_or_default(),
            recent_events: row.get::<_, Option<String>>("cs_recent_events")?.unwrap_or_default(),
            updated_at_chapter,
            provenance: parse_provenance(row.get::<_, Option<String>>("cs_provenance")?),
        }),
        None => None,
    };

    Ok(CharacterData {
        name: row.get("name")?,
        role: normalize_character_role(&row.get::<_, Option<String>>("role")?.unwrap_or_default()),
        gender: row.get::<_, Option<String>>("gender")?.unwrap_or_default(),
        age: row.get::<_, Option<String>>("age")?.unwrap_or_default(),
        appearance: row.get::<_, Option<String>>("appearance")?.unwrap_or_default(),
        personality: row.get::<_, Option<String>>("personality")?.unwrap_or_default(),
        background: row.get::<_, Option<String>>("background")?.unwrap_or_default(),
        abilities: row.get::<_, Option<String>>("abilities")?.unwrap_or_default(),
        motivation: row.get::<_, Option<String>>("motivation")?.unwrap_or_default(),
        relationships: row.get::<_, Option<String>>("relationships")?.unwrap_or_default(),
        arc: row.get::<_, Option<String>>("arc")?.unwrap_or_default(),
        notes: row.get::<_, Option<String>>("notes")?.unwrap_or_default(),
        current_state,
    })
}

/// 读取全部角色，按角色定位排序（对齐基线 `ORDER BY CASE role`）。
pub fn get_all(conn: &Connection) -> Result<Vec<CharacterData>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT * FROM characters ORDER BY \
             CASE role \
               WHEN 'protagonist' THEN 0 \
               WHEN 'supporting' THEN 1 \
               WHEN 'antagonist' THEN 2 \
               WHEN 'minor' THEN 3 \
               ELSE 9 \
             END ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map([], |row| row_to_data(row))
        .map_err(|error| error.to_string())?;
    let mut characters = Vec::new();
    for row in rows {
        characters.push(row.map_err(|error| error.to_string())?);
    }
    Ok(characters)
}

// 已平移但暂无调用点：由后续批次（drafts / reviews / post-process / import）接线。
#[allow(dead_code)]
/// 读取单个角色。
pub fn get_by_name(conn: &Connection, name: &str) -> Result<Option<CharacterData>, String> {
    conn.query_row("SELECT * FROM characters WHERE name = ?1", [name], |row| {
        row_to_data(row)
    })
    .optional()
    .map_err(|error| error.to_string())
}

// 已平移但暂无调用点：由后续批次（drafts / reviews / post-process / import）接线。
#[allow(dead_code)]
/// 角色数量。
pub fn count(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COUNT(*) FROM characters", [], |row| row.get(0))
        .map_err(|error| error.to_string())
}

/// 插入或更新角色（`currentState` 缺失即清空 `cs_*` 列，与基线 upsert 语义一致）。
pub fn upsert(conn: &Connection, data: &CharacterData) -> Result<(), String> {
    let current = data.current_state.clone().unwrap_or(CharacterStateData {
        location: String::new(),
        power_level: String::new(),
        physical_state: String::new(),
        mental_state: String::new(),
        key_items: String::new(),
        recent_events: String::new(),
        updated_at_chapter: 0,
        provenance: None,
    });
    let updated_at_chapter = data
        .current_state
        .as_ref()
        .map(|state| state.updated_at_chapter);
    let provenance = data
        .current_state
        .as_ref()
        .and_then(|state| state.provenance.as_ref());

    conn.execute(
        "INSERT INTO characters (\
           name, role, gender, age, appearance, personality, background, \
           abilities, motivation, relationships, arc, notes, \
           cs_location, cs_power_level, cs_physical_state, cs_mental_state, \
           cs_key_items, cs_recent_events, cs_updated_at_chapter, cs_provenance \
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20) \
         ON CONFLICT(name) DO UPDATE SET \
           role = excluded.role, \
           gender = excluded.gender, \
           age = excluded.age, \
           appearance = excluded.appearance, \
           personality = excluded.personality, \
           background = excluded.background, \
           abilities = excluded.abilities, \
           motivation = excluded.motivation, \
           relationships = excluded.relationships, \
           arc = excluded.arc, \
           notes = excluded.notes, \
           cs_location = excluded.cs_location, \
           cs_power_level = excluded.cs_power_level, \
           cs_physical_state = excluded.cs_physical_state, \
           cs_mental_state = excluded.cs_mental_state, \
           cs_key_items = excluded.cs_key_items, \
           cs_recent_events = excluded.cs_recent_events, \
           cs_updated_at_chapter = excluded.cs_updated_at_chapter, \
           cs_provenance = excluded.cs_provenance, \
           updated_at = datetime('now')",
        rusqlite::params![
            data.name,
            data.role.as_str(),
            data.gender,
            data.age,
            data.appearance,
            data.personality,
            data.background,
            data.abilities,
            data.motivation,
            data.relationships,
            data.arc,
            data.notes,
            current.location,
            current.power_level,
            current.physical_state,
            current.mental_state,
            current.key_items,
            current.recent_events,
            updated_at_chapter,
            provenance_to_text(provenance),
        ],
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

// 已平移但暂无调用点：由后续批次（drafts / reviews / post-process / import）接线。
#[allow(dead_code)]
/// 批量保存角色（事务）：名称归一/唯一、改名一致性校验、两阶段改名 + 蓝图引用同步。
pub fn save_all(
    conn: &Connection,
    characters: &[CharacterData],
    renames: &[CharacterRenameData],
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;

    let normalized_characters: Vec<CharacterData> = characters
        .iter()
        .map(|character| CharacterData {
            name: character.name.trim().to_string(),
            ..character.clone()
        })
        .collect();
    let names: Vec<String> = normalized_characters
        .iter()
        .map(|character| character.name.clone())
        .collect();
    if names.iter().any(|name| name.is_empty()) {
        return Err("角色名不能为空".to_string());
    }
    if names.iter().collect::<std::collections::HashSet<_>>().len() != names.len() {
        return Err("角色名必须唯一".to_string());
    }

    let normalized_renames: Vec<CharacterRenameData> = renames
        .iter()
        .map(|rename| CharacterRenameData {
            original_name: rename.original_name.clone(),
            new_name: rename.new_name.trim().to_string(),
        })
        .filter(|rename| rename.original_name != rename.new_name)
        .collect();
    if normalized_renames
        .iter()
        .any(|rename| rename.original_name.is_empty() || rename.new_name.is_empty())
    {
        return Err("角色改名的原名和新名不能为空".to_string());
    }
    let original_names: Vec<String> = normalized_renames
        .iter()
        .map(|rename| rename.original_name.clone())
        .collect();
    let target_names: Vec<String> = normalized_renames
        .iter()
        .map(|rename| rename.new_name.clone())
        .collect();
    if original_names.iter().collect::<std::collections::HashSet<_>>().len() != original_names.len()
        || target_names.iter().collect::<std::collections::HashSet<_>>().len() != target_names.len()
    {
        return Err("角色改名目标必须唯一".to_string());
    }

    for rename in &normalized_renames {
        let original_exists = tx
            .query_row(
                "SELECT 1 FROM characters WHERE name = ?1",
                [rename.original_name.as_str()],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .is_some();
        if !original_exists {
            return Err(format!("角色「{}」不存在，无法改名", rename.original_name));
        }
        let conflict = tx
            .query_row(
                "SELECT 1 FROM characters WHERE name = ?1",
                [rename.new_name.as_str()],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .is_some();
        if conflict && !original_names.contains(&rename.new_name) {
            return Err(format!("角色名「{}」已存在", rename.new_name));
        }
        if !names.contains(&rename.new_name)
            || (!target_names.contains(&rename.original_name) && names.contains(&rename.original_name))
        {
            return Err(format!(
                "角色改名「{} → {}」与保存内容不一致",
                rename.original_name, rename.new_name
            ));
        }
    }

    // 两阶段改名：先把全部原名移到事务内临时键，允许 A↔B 交换与链式改名，
    // 同时避免 SQLite 主键唯一约束中途冲突。
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let mut temporary_renames: Vec<(String, String)> = Vec::new();
    for (index, rename) in normalized_renames.iter().enumerate() {
        let mut temporary_name = format!("__vela_rename_{stamp}_{index}__");
        loop {
            let collision = names.contains(&temporary_name)
                || target_names.contains(&temporary_name)
                || tx
                    .query_row(
                        "SELECT 1 FROM characters WHERE name = ?1",
                        [temporary_name.as_str()],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(|error| error.to_string())?
                    .is_some();
            if !collision {
                break;
            }
            temporary_name.push('_');
        }
        temporary_renames.push((rename.original_name.clone(), temporary_name));
    }
    for (original_name, temporary_name) in &temporary_renames {
        tx.execute(
            "UPDATE characters SET name = ?1, updated_at = datetime('now') WHERE name = ?2",
            rusqlite::params![temporary_name, original_name],
        )
        .map_err(|error| error.to_string())?;
    }
    for (index, (_, temporary_name)) in temporary_renames.iter().enumerate() {
        tx.execute(
            "UPDATE characters SET name = ?1, updated_at = datetime('now') WHERE name = ?2",
            rusqlite::params![normalized_renames[index].new_name, temporary_name],
        )
        .map_err(|error| error.to_string())?;
    }

    if !normalized_renames.is_empty() {
        let rename_by_original: HashMap<String, String> = normalized_renames
            .iter()
            .map(|rename| (rename.original_name.clone(), rename.new_name.clone()))
            .collect();
        let blueprints: Vec<(i64, String)> = {
            let mut stmt = tx
                .prepare("SELECT chapter_number, characters FROM blueprints")
                .map_err(|error| error.to_string())?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    ))
                })
                .map_err(|error| error.to_string())?;
            let mut collected = Vec::new();
            for row in rows {
                collected.push(row.map_err(|error| error.to_string())?);
            }
            collected
        };
        for (chapter_number, characters_json) in blueprints {
            let parsed: serde_json::Value = serde_json::from_str(&characters_json)
                .map_err(|_| format!("第 {chapter_number} 章蓝图角色列表损坏"))?;
            let Some(items) = parsed.as_array() else {
                return Err(format!("第 {chapter_number} 章蓝图角色列表格式错误"));
            };
            let mut renamed_names: Vec<String> = Vec::with_capacity(items.len());
            for item in items {
                let Some(name) = item.as_str() else {
                    return Err(format!("第 {chapter_number} 章蓝图角色列表格式错误"));
                };
                renamed_names.push(rename_by_original.get(name).cloned().unwrap_or_else(|| name.to_string()));
            }
            let changed = items
                .iter()
                .zip(renamed_names.iter())
                .any(|(item, renamed)| item.as_str() != Some(renamed.as_str()));
            if changed {
                let payload = serde_json::to_string(&renamed_names)
                    .map_err(|error| error.to_string())?;
                tx.execute(
                    "UPDATE blueprints SET characters = ?1, updated_at = datetime('now') WHERE chapter_number = ?2",
                    rusqlite::params![payload, chapter_number],
                )
                .map_err(|error| error.to_string())?;
            }
        }
    }

    for character in &normalized_characters {
        upsert(&tx, character)?;
    }

    tx.commit().map_err(|error| error.to_string())
}

// 已平移但暂无调用点：由后续批次（drafts / reviews / post-process / import）接线。
#[allow(dead_code)]
/// 删除角色。
pub fn delete(conn: &Connection, name: &str) -> Result<(), String> {
    conn.execute("DELETE FROM characters WHERE name = ?1", [name])
        .map(|_| ())
        .map_err(|error| error.to_string())
}

// 已平移但暂无调用点：由后续批次（drafts / reviews / post-process / import）接线。
#[allow(dead_code)]
/// 仅更新角色动态状态（后处理使用，不动角色卡其它字段）。
pub fn update_state(conn: &Connection, name: &str, state: &CharacterStateData) -> Result<(), String> {
    conn.execute(
        "UPDATE characters SET \
           cs_location = ?1, cs_power_level = ?2, cs_physical_state = ?3, \
           cs_mental_state = ?4, cs_key_items = ?5, cs_recent_events = ?6, \
           cs_updated_at_chapter = ?7, cs_provenance = ?8, updated_at = datetime('now') \
         WHERE name = ?9",
        rusqlite::params![
            state.location,
            state.power_level,
            state.physical_state,
            state.mental_state,
            state.key_items,
            state.recent_events,
            state.updated_at_chapter,
            provenance_to_text(state.provenance.as_ref()),
            name,
        ],
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        create_tables(&conn).expect("建表失败");
        conn
    }

    fn character(name: &str, role: CharacterRole) -> CharacterData {
        CharacterData {
            name: name.to_string(),
            role,
            gender: String::new(),
            age: String::new(),
            appearance: String::new(),
            personality: String::new(),
            background: String::new(),
            abilities: String::new(),
            motivation: String::new(),
            relationships: String::new(),
            arc: String::new(),
            notes: String::new(),
            current_state: None,
        }
    }

    fn state(chapter: i64) -> CharacterStateData {
        CharacterStateData {
            location: "青云宗".to_string(),
            power_level: "筑基".to_string(),
            physical_state: "良好".to_string(),
            mental_state: "沉稳".to_string(),
            key_items: "青锋剑".to_string(),
            recent_events: "突破".to_string(),
            updated_at_chapter: chapter,
            provenance: None,
        }
    }

    #[test]
    fn upsert_and_read_roundtrip_test() {
        let conn = memory_db();
        let mut data = character("林清玄", CharacterRole::Protagonist);
        data.current_state = Some(state(0));
        upsert(&conn, &data).unwrap();

        let loaded = get_by_name(&conn, "林清玄").unwrap().expect("角色应存在");
        assert_eq!(loaded.role, CharacterRole::Protagonist);
        let loaded_state = loaded.current_state.expect("第 0 章也是合法状态");
        assert_eq!(loaded_state.updated_at_chapter, 0);
        assert_eq!(loaded_state.location, "青云宗");
        assert_eq!(loaded_state.provenance, None);
    }

    #[test]
    fn upsert_without_current_state_clears_state_columns_test() {
        let conn = memory_db();
        let mut data = character("苏白", CharacterRole::Supporting);
        data.current_state = Some(state(3));
        upsert(&conn, &data).unwrap();
        upsert(&conn, &character("苏白", CharacterRole::Supporting)).unwrap();

        let loaded = get_by_name(&conn, "苏白").unwrap().unwrap();
        assert!(loaded.current_state.is_none(), "currentState 缺失应清空 cs_* 列");
    }

    #[test]
    fn get_all_sorts_by_role_weight_test() {
        let conn = memory_db();
        upsert(&conn, &character("龙套甲", CharacterRole::Minor)).unwrap();
        upsert(&conn, &character("反派乙", CharacterRole::Antagonist)).unwrap();
        upsert(&conn, &character("主角丙", CharacterRole::Protagonist)).unwrap();
        upsert(&conn, &character("配角丁", CharacterRole::Supporting)).unwrap();

        let names: Vec<String> = get_all(&conn)
            .unwrap()
            .into_iter()
            .map(|character| character.name)
            .collect();
        assert_eq!(names, vec!["主角丙", "配角丁", "反派乙", "龙套甲"]);
    }

    #[test]
    fn provenance_roundtrip_and_serialization_shape_test() {
        let conn = memory_db();
        let mut provenance = CharacterStateProvenance::new();
        provenance.insert(
            "location".to_string(),
            CharacterStateFieldProvenance::Derived {
                source: FinalizedSourceIdentity {
                    draft_id: 12,
                    finalization_id: "fin-12".to_string(),
                    chapter_number: 5,
                    content_hash: "abc".to_string(),
                },
            },
        );
        provenance.insert(
            "powerLevel".to_string(),
            CharacterStateFieldProvenance::Author { chapter_number: 6 },
        );
        let mut data = character("林清玄", CharacterRole::Protagonist);
        let mut current = state(5);
        current.provenance = Some(provenance.clone());
        data.current_state = Some(current);
        upsert(&conn, &data).unwrap();

        let loaded = get_by_name(&conn, "林清玄").unwrap().unwrap();
        assert_eq!(loaded.current_state.unwrap().provenance, Some(provenance));

        let json = serde_json::to_value(&data).unwrap();
        let provenance_json = &json["currentState"]["provenance"];
        assert_eq!(provenance_json["location"]["kind"], "derived");
        assert_eq!(provenance_json["location"]["source"]["finalizationId"], "fin-12");
        assert_eq!(provenance_json["powerLevel"]["kind"], "author");
        assert_eq!(provenance_json["powerLevel"]["chapterNumber"], 6);
    }

    #[test]
    fn legacy_provenance_text_falls_back_to_empty_test() {
        let conn = memory_db();
        conn.execute(
            "INSERT INTO characters (name, role, cs_updated_at_chapter, cs_provenance) VALUES ('旧角色', 'minor', 2, 'not-json')",
            [],
        )
        .unwrap();
        let loaded = get_by_name(&conn, "旧角色").unwrap().unwrap();
        assert_eq!(loaded.current_state.unwrap().provenance, None);
    }

    #[test]
    fn save_all_renames_characters_and_blueprint_references_test() {
        let conn = memory_db();
        upsert(&conn, &character("旧名", CharacterRole::Protagonist)).unwrap();
        conn.execute(
            "INSERT INTO blueprints (chapter_number, characters) VALUES (1, ?1), (2, ?2)",
            rusqlite::params![
                serde_json::to_string(&vec!["旧名", "路人"]).unwrap(),
                serde_json::to_string(&vec!["路人"]).unwrap()
            ],
        )
        .unwrap();

        save_all(
            &conn,
            &[character("新名", CharacterRole::Protagonist)],
            &[CharacterRenameData {
                original_name: "旧名".to_string(),
                new_name: "新名".to_string(),
            }],
        )
        .unwrap();

        assert!(get_by_name(&conn, "旧名").unwrap().is_none());
        assert!(get_by_name(&conn, "新名").unwrap().is_some());
        let renamed: String = conn
            .query_row(
                "SELECT characters FROM blueprints WHERE chapter_number = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(renamed, serde_json::to_string(&vec!["新名", "路人"]).unwrap());
        // 未命中的蓝图不写回（updated_at 保持默认空值语义不变）
        let untouched: String = conn
            .query_row(
                "SELECT characters FROM blueprints WHERE chapter_number = 2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(untouched, serde_json::to_string(&vec!["路人"]).unwrap());
    }

    #[test]
    fn save_all_rename_swap_is_atomic_test() {
        let conn = memory_db();
        upsert(&conn, &character("甲", CharacterRole::Protagonist)).unwrap();
        upsert(&conn, &character("乙", CharacterRole::Supporting)).unwrap();

        save_all(
            &conn,
            &[
                character("乙", CharacterRole::Protagonist),
                character("甲", CharacterRole::Supporting),
            ],
            &[
                CharacterRenameData {
                    original_name: "甲".to_string(),
                    new_name: "乙".to_string(),
                },
                CharacterRenameData {
                    original_name: "乙".to_string(),
                    new_name: "甲".to_string(),
                },
            ],
        )
        .unwrap();

        assert_eq!(
            get_by_name(&conn, "乙").unwrap().unwrap().role,
            CharacterRole::Protagonist
        );
        assert_eq!(
            get_by_name(&conn, "甲").unwrap().unwrap().role,
            CharacterRole::Supporting
        );
    }

    #[test]
    fn save_all_rejects_invalid_inputs_test() {
        let conn = memory_db();
        upsert(&conn, &character("甲", CharacterRole::Protagonist)).unwrap();

        let duplicate = save_all(
            &conn,
            &[
                character("甲", CharacterRole::Protagonist),
                character("甲", CharacterRole::Supporting),
            ],
            &[],
        );
        assert_eq!(duplicate.unwrap_err(), "角色名必须唯一");

        let empty = save_all(&conn, &[character("  ", CharacterRole::Minor)], &[]);
        assert_eq!(empty.unwrap_err(), "角色名不能为空");

        let missing = save_all(
            &conn,
            &[character("乙", CharacterRole::Minor)],
            &[CharacterRenameData {
                original_name: "不存在".to_string(),
                new_name: "乙".to_string(),
            }],
        );
        assert_eq!(missing.unwrap_err(), "角色「不存在」不存在，无法改名");

        let conflict = save_all(
            &conn,
            &[character("甲", CharacterRole::Protagonist)],
            &[CharacterRenameData {
                original_name: "甲".to_string(),
                new_name: "甲".to_string(),
            }],
        );
        // 原名==新名会被过滤，等价于无改名
        assert!(conflict.is_ok());
    }

    #[test]
    fn save_all_rejects_broken_blueprint_characters_test() {
        let conn = memory_db();
        upsert(&conn, &character("旧名", CharacterRole::Protagonist)).unwrap();
        conn.execute(
            "INSERT INTO blueprints (chapter_number, characters) VALUES (3, 'not-json')",
            [],
        )
        .unwrap();

        let error = save_all(
            &conn,
            &[character("新名", CharacterRole::Protagonist)],
            &[CharacterRenameData {
                original_name: "旧名".to_string(),
                new_name: "新名".to_string(),
            }],
        )
        .unwrap_err();
        assert_eq!(error, "第 3 章蓝图角色列表损坏");

        // 事务整体回滚：改名不落地
        assert!(get_by_name(&conn, "旧名").unwrap().is_some());
    }

    #[test]
    fn update_state_writes_only_state_columns_test() {
        let conn = memory_db();
        let mut data = character("苏白", CharacterRole::Supporting);
        data.background = "世家子弟".to_string();
        upsert(&conn, &data).unwrap();

        update_state(&conn, "苏白", &state(4)).unwrap();

        let loaded = get_by_name(&conn, "苏白").unwrap().unwrap();
        assert_eq!(loaded.background, "世家子弟");
        assert_eq!(loaded.current_state.unwrap().updated_at_chapter, 4);
    }

    #[test]
    fn delete_removes_character_test() {
        let conn = memory_db();
        upsert(&conn, &character("甲", CharacterRole::Minor)).unwrap();
        assert_eq!(count(&conn).unwrap(), 1);
        delete(&conn, "甲").unwrap();
        assert_eq!(count(&conn).unwrap(), 0);
    }
}