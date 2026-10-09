//! 角色定位领域常量与归一化 —— 平移自 `src/shared/character-role.ts`
//!
//! 持久化或外部输入的角色定位必须先经 [`normalize_character_role`] 归一，
//! 未知的历史取值一律回落到最不意外的可编辑定位（`supporting`）。

use serde::{Deserialize, Serialize};

/// 角色定位（对齐 `CHARACTER_ROLES`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CharacterRole {
    Protagonist,
    Antagonist,
    Supporting,
    Minor,
}

/// 角色卡列表排序权重（对齐基线 `ORDER BY CASE role` 与 roster 的 `ROLE_ORDER`）。
///
/// 它是排序权的**单一事实源**：[`CharacterRole::sort_weight`] 直接由此推导。
pub const CHARACTER_ROLE_ORDER: [CharacterRole; 4] = [
    CharacterRole::Protagonist,
    CharacterRole::Supporting,
    CharacterRole::Antagonist,
    CharacterRole::Minor,
];

impl CharacterRole {
    /// 持久化标识（小写英文，与 characters.role 列取值一致）。
    pub fn as_str(self) -> &'static str {
        match self {
            CharacterRole::Protagonist => "protagonist",
            CharacterRole::Antagonist => "antagonist",
            CharacterRole::Supporting => "supporting",
            CharacterRole::Minor => "minor",
        }
    }

    /// 列表/投影排序权重（主角 → 配角 → 反派 → 龙套）
    pub fn sort_weight(self) -> u8 {
        CHARACTER_ROLE_ORDER
            .iter()
            .position(|role| *role == self)
            .map(|index| index as u8)
            .unwrap_or(u8::MAX)
    }

    /// 语言相关展示名（对齐 `CHARACTER_ROLE_LABELS`）。
    pub fn label(self, english: bool) -> &'static str {
        match (self, english) {
            (CharacterRole::Protagonist, false) => "主角",
            (CharacterRole::Protagonist, true) => "Protagonist",
            (CharacterRole::Antagonist, false) => "反派",
            (CharacterRole::Antagonist, true) => "Antagonist",
            (CharacterRole::Supporting, false) => "配角",
            (CharacterRole::Supporting, true) => "Supporting character",
            (CharacterRole::Minor, false) => "龙套",
            (CharacterRole::Minor, true) => "Minor character",
        }
    }
}

impl Serialize for CharacterRole {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CharacterRole {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // 非字符串输入一律回落 supporting（对齐基线 typeof value !== 'string' 分支）。
        let raw = serde_json::Value::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            Some(value) => normalize_character_role(value),
            None => CharacterRole::Supporting,
        })
    }
}

/// 历史/中英文别名表（逐条对齐 `CHARACTER_ROLE_ALIASES`）。
fn character_role_alias(value: &str) -> Option<CharacterRole> {
    Some(match value {
        "protagonist" | "main" | "主角" | "男主" | "女主" | "核心主角" => {
            CharacterRole::Protagonist
        }
        "antagonist" | "villain" | "反派" | "对手" | "敌人" => CharacterRole::Antagonist,
        "supporting" | "support" | "配角" | "重要配角" | "核心配角" => {
            CharacterRole::Supporting
        }
        "minor" | "龙套" | "次要角色" => CharacterRole::Minor,
        _ => return None,
    })
}

/// 归一化角色定位：先查原样，再查小写形式，最后回落 `supporting`。
pub fn normalize_character_role(value: &str) -> CharacterRole {
    let candidate = value.trim();
    if let Some(role) = character_role_alias(candidate) {
        return role;
    }
    let lowered = candidate.to_lowercase();
    character_role_alias(&lowered).unwrap_or(CharacterRole::Supporting)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_character_role_matches_baseline_aliases_test() {
        assert_eq!(normalize_character_role("主角"), CharacterRole::Protagonist);
        assert_eq!(normalize_character_role("MAIN"), CharacterRole::Protagonist);
        assert_eq!(
            normalize_character_role(" villain "),
            CharacterRole::Antagonist
        );
        assert_eq!(
            normalize_character_role("核心配角"),
            CharacterRole::Supporting
        );
        assert_eq!(normalize_character_role("次要角色"), CharacterRole::Minor);
        // 未知取值回落 supporting
        assert_eq!(
            normalize_character_role("未知定位"),
            CharacterRole::Supporting
        );
        assert_eq!(normalize_character_role(""), CharacterRole::Supporting);
    }

    #[test]
    fn character_role_serialization_roundtrip_test() {
        for role in CHARACTER_ROLE_ORDER {
            let json = serde_json::to_string(&role).unwrap();
            assert_eq!(json, format!("\"{}\"", role.as_str()));
            let parsed: CharacterRole = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, role);
        }
        // 别名与非字符串输入均回落 supporting
        let aliased: CharacterRole = serde_json::from_str("\"主角\"").unwrap();
        assert_eq!(aliased, CharacterRole::Protagonist);
        let fallback: CharacterRole = serde_json::from_str("42").unwrap();
        assert_eq!(fallback, CharacterRole::Supporting);
    }

    #[test]
    fn role_labels_match_baseline_test() {
        assert_eq!(CharacterRole::Protagonist.label(false), "主角");
        assert_eq!(
            CharacterRole::Supporting.label(true),
            "Supporting character"
        );
        assert_eq!(CharacterRole::Minor.label(false), "龙套");
        assert_eq!(CharacterRole::Antagonist.sort_weight(), 2);
    }
}
