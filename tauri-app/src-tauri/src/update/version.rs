//! 版本比较 —— 平移自 `electron/services/update-service.ts:61-69, 175-186`。
//!
//! 对应基线 `stableVersionParts` / `isHigherStableVersion`：
//! 只承认**稳定三段版本**（可选 `v` 前缀 + 可选 `+build` 元数据），预发布与非 SemVer 一律
//! 视为「不可比较」（返回 `false`，绝不把预发布当成可用更新）。

use regex::Regex;

/// 对齐基线 `stableVersionParts`：`^v?(\d+)\.(\d+)\.(\d+)(?:\+[...])?$`
pub fn stable_version_parts(version: &str) -> Option<[i64; 3]> {
    let pattern =
        Regex::new(r"^v?(\d+)\.(\d+)\.(\d+)(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$").unwrap();
    let captures = pattern.captures(version)?;
    Some([
        captures[1].parse().ok()?,
        captures[2].parse().ok()?,
        captures[3].parse().ok()?,
    ])
}

/// 对齐基线 `isHigherStableVersion`：按段比较，严格更高才为 `true`
pub fn is_higher_stable_version(candidate: &str, current: &str) -> bool {
    let Some(candidate_parts) = stable_version_parts(candidate) else {
        return false;
    };
    let Some(current_parts) = stable_version_parts(current) else {
        return false;
    };
    for index in 0..candidate_parts.len() {
        if candidate_parts[index] != current_parts[index] {
            return candidate_parts[index] > current_parts[index];
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_version_parts_accepts_stable_only_test() {
        assert_eq!(stable_version_parts("1.2.3"), Some([1, 2, 3]));
        assert_eq!(stable_version_parts("v1.2.3"), Some([1, 2, 3]));
        assert_eq!(stable_version_parts("1.2.3+build.7"), Some([1, 2, 3]));
        // 预发布 / 非三段 / 超长 → 不可比较
        assert_eq!(stable_version_parts("1.2.3-beta.1"), None);
        assert_eq!(stable_version_parts("1.2"), None);
        assert_eq!(stable_version_parts("1.2.3.4"), None);
        assert_eq!(stable_version_parts("latest"), None);
    }

    #[test]
    fn higher_stable_version_compares_numerically_test() {
        assert!(is_higher_stable_version("1.0.1", "1.0.0"));
        // 按段比较：1.10.0 高于 1.9.0（不是字符串比较）
        assert!(is_higher_stable_version("1.10.0", "1.9.0"));
        assert!(is_higher_stable_version("v2.0.0", "1.99.99"));
        assert!(!is_higher_stable_version("1.0.0", "1.0.0"));
        assert!(!is_higher_stable_version("1.0.0", "1.0.1"));
        assert!(!is_higher_stable_version("1.2.3-beta.1", "1.2.2"));
        assert!(!is_higher_stable_version("1.2.3", "not-a-version"));
        assert!(!is_higher_stable_version("not-a-version", "1.2.3"));
    }
}
