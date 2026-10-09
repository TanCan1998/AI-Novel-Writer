//! 更新域的时间辅助 —— 不引入 `chrono`/`time` 依赖，沿用 Howard Hinnant 历法算法。
//!
//! 对齐基线两处语义：
//! - `localCalendarDate(now)`（`update-service.ts:71-76`）：自动检查「每天最多一次」的日历日；
//! - `Date.parse(reminder.until)`（`update-service.ts:495`）：解析已持久化的延后截止时间。
//!
//! ⚠️ **刻意偏离**：基线的日历日取自**本地时区**（`getFullYear/getMonth/getDate`）；
//! 本实现为免引入时区依赖，使用 **UTC 日历日**。可观测差异仅限「本地跨零点」与
//! 「UTC 跨零点」之间最多一天的边界偏移，不影响「每天最多一次」的节流语义。
//! 时间戳形态与 `commands::project::iso8601_utc_from_millis`（`toISOString()` 等价）一致。

use regex::Regex;

/// `days`（自 1970-01-01 起的天数）→ `(year, month, day)`（对齐 `civil_from_days`）
pub fn civil_date_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month as i64, day as i64)
}

/// `days_from_civil`：`(year, month, day)` → 自 1970-01-01 起的天数（`civil_date_from_days` 的逆）
pub fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 }.div_euclid(400);
    let yoe = (year - era * 400) as u64;
    let mp = if month > 2 { month - 3 } else { month + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

/// 毫秒时间戳 → UTC 日历日 `YYYY-MM-DD`
pub fn utc_calendar_date(millis: u64) -> String {
    let days = (millis / 1000).div_euclid(86_400) as i64;
    let (year, month, day) = civil_date_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// ISO 8601 → 毫秒时间戳；不可解析返回 `None`（调用方按「未延后」处理）。
///
/// 接受形态：`YYYY-MM-DDTHH:MM:SS[.sss]Z`（`Z` 大小写不敏感）与等价的 `+00:00` 后缀。
/// 带非零时区偏移的形态一律返回 `None` —— 本仓库自己写出的 `until` 恒为 `...Z`。
pub fn parse_iso8601_millis(value: &str) -> Option<i64> {
    let pattern = Regex::new(
        r"(?i)^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(?:\.(\d{1,3}))?(?:[Zz]|\+00:00)$",
    )
    .unwrap();
    let captures = pattern.captures(value.trim())?;
    let number = |index: usize| -> Option<i64> { captures[index].parse().ok() };
    let (year, month, day) = (number(1)?, number(2)?, number(3)?);
    let (hour, minute, second) = (number(4)?, number(5)?, number(6)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let millis = captures.get(7).map(|item| {
        let digits = item.as_str();
        let padded = format!("{digits:0<3}");
        padded[..3].parse::<i64>().unwrap_or(0)
    });
    let days = days_from_civil(year, month, day);
    Some(days * 86_400_000 + (hour * 3600 + minute * 60 + second) * 1000 + millis.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::project::iso8601_utc_from_millis;

    #[test]
    fn civil_date_roundtrip_matches_iso_helpers_test() {
        for millis in [0u64, 1_700_000_000_000, 1_800_000_000_000] {
            let iso = iso8601_utc_from_millis(millis);
            let days = (millis / 1000).div_euclid(86_400) as i64;
            let (year, month, day) = civil_date_from_days(days);
            assert_eq!(
                utc_calendar_date(millis),
                format!("{year:04}-{month:02}-{day:02}")
            );
            assert!(iso.starts_with(&utc_calendar_date(millis)));
            assert_eq!(
                days_from_civil(year, month, day),
                days,
                "days_from_civil 必须可逆"
            );
        }
        assert_eq!(utc_calendar_date(0), "1970-01-01");
        assert_eq!(utc_calendar_date(1_700_000_000_000), "2023-11-14");
    }

    #[test]
    fn parse_iso8601_accepts_own_output_and_rejects_noise_test() {
        let millis = 1_700_000_000_123u64;
        let iso = iso8601_utc_from_millis(millis);
        assert_eq!(parse_iso8601_millis(&iso), Some(millis as i64));
        assert_eq!(parse_iso8601_millis("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(parse_iso8601_millis("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601_millis("1970-01-01T00:00:00+00:00"), Some(0));
        assert_eq!(parse_iso8601_millis("1970-01-01T00:00:01.5Z"), Some(1500));
        // 非法输入 → None（调用方按「未延后」处理）
        assert_eq!(parse_iso8601_millis("昨天"), None);
        assert_eq!(parse_iso8601_millis("2026-10-16T12:00:00+08:00"), None);
        assert_eq!(parse_iso8601_millis("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_iso8601_millis(""), None);
    }
}
