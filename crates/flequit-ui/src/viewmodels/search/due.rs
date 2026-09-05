//! Due-date keywords accepted by the search box.
//!
//! The vocabulary is defined in `docs/ja/develop/design/ui/page/main/main.md`;
//! both the Japanese and the English spellings are accepted, and the sidebar
//! filter buttons write the same keywords into the search box, so the buttons
//! and typed queries cannot drift apart.

use chrono::{DateTime, TimeDelta, Utc};

use crate::adapters::datetime::{DisplayTimezone, end_of_day_after};

/// A due-date condition parsed from an `@keyword` token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DueKeyword {
    /// Past its due date and not finished yet.
    Overdue,
    /// Due within `days` calendar days, counting today as the first one.
    ///
    /// The boundary is the end of the last day, so the time of day is ignored:
    /// "due today" still matches a task due at 23:00 when it is 09:00 now.
    Within { days: u64 },
    /// Due within `minutes` from now, to the minute.
    ///
    /// Unlike [`Self::Within`] this does not round up to the end of a day:
    /// "@10minutes" is about the next ten minutes, not about today.
    WithinMinutes { minutes: u64 },
}

/// Words that map to a fixed horizon. Compared after lowercasing.
///
/// Numeric forms (`@10日`, `@3days`, `@2period`) are handled by [`parse_counted`]
/// instead, which is why `3days` and friends are absent here.
const KEYWORDS: &[(&str, DueKeyword)] = &[
    ("overdue", DueKeyword::Overdue),
    ("deadline", DueKeyword::Overdue),
    ("期限切れ", DueKeyword::Overdue),
    ("today", DueKeyword::Within { days: 1 }),
    ("今日", DueKeyword::Within { days: 1 }),
    ("本日", DueKeyword::Within { days: 1 }),
    ("tomorrow", DueKeyword::Within { days: 2 }),
    ("明日", DueKeyword::Within { days: 2 }),
    ("翌日", DueKeyword::Within { days: 2 }),
    ("明後日", DueKeyword::Within { days: 3 }),
    ("week", DueKeyword::Within { days: 7 }),
    ("weeks", DueKeyword::Within { days: 7 }),
    ("今週", DueKeyword::Within { days: 7 }),
    ("週", DueKeyword::Within { days: 7 }),
    ("month", DueKeyword::Within { days: 30 }),
    ("months", DueKeyword::Within { days: 30 }),
    ("今月", DueKeyword::Within { days: 30 }),
    ("月", DueKeyword::Within { days: 30 }),
    ("quarter", DueKeyword::Within { days: 90 }),
    ("period", DueKeyword::Within { days: 90 }),
    ("今期", DueKeyword::Within { days: 90 }),
    ("期", DueKeyword::Within { days: 90 }),
    ("year", DueKeyword::Within { days: 365 }),
    ("years", DueKeyword::Within { days: 365 }),
    ("今年", DueKeyword::Within { days: 365 }),
    ("fiscalyear", DueKeyword::Within { days: 365 }),
    ("fiscal-year", DueKeyword::Within { days: 365 }),
    ("今年度", DueKeyword::Within { days: 365 }),
    ("年度", DueKeyword::Within { days: 365 }),
];

/// Sub-day units accepted after a number, and how many minutes one unit covers.
///
/// Kept apart from [`UNITS`] because these horizons are measured from now
/// rather than rounded up to the end of a calendar day.
///
/// Longer spellings come first so that `時間` is not read as `時`.
const MINUTE_UNITS: &[(&str, u64)] = &[
    ("分", 1),
    ("minute", 1),
    ("minutes", 1),
    ("min", 1),
    ("時間", 60),
    ("時", 60),
    ("hour", 60),
    ("hours", 60),
    ("h", 60),
];

/// Units accepted after a number, and how many days one unit covers.
///
/// Longer spellings come first so that `年度` is not read as `年`.
const UNITS: &[(&str, u64)] = &[
    ("年度", 365),
    ("fiscalyear", 365),
    ("日", 1),
    ("day", 1),
    ("days", 1),
    ("週間", 7),
    ("週", 7),
    ("week", 7),
    ("weeks", 7),
    ("か月", 30),
    ("ヶ月", 30),
    ("ケ月", 30),
    ("カ月", 30),
    ("月", 30),
    ("month", 30),
    ("months", 30),
    ("期", 90),
    ("q", 90),
    ("period", 90),
    ("年", 365),
    ("year", 365),
    ("years", 365),
];

impl DueKeyword {
    /// Parses the text after the `@`, e.g. `today` or `10日`.
    ///
    /// Returns `None` for anything that is not a due keyword; the caller decides
    /// what an unrecognised keyword means.
    pub fn parse(keyword: &str) -> Option<Self> {
        let keyword = keyword.to_lowercase();
        KEYWORDS
            .iter()
            .find(|(word, _)| *word == keyword)
            .map(|(_, value)| *value)
            .or_else(|| parse_counted(&keyword))
    }

    /// Whether a task with this due date satisfies the keyword.
    ///
    /// A task without a due date never matches: the filters exist to narrow down
    /// what is coming up, and an undated task has nothing coming up.
    pub fn matches(
        self,
        due: Option<&DateTime<Utc>>,
        completed: bool,
        now: &DateTime<Utc>,
        timezone: DisplayTimezone,
    ) -> bool {
        let Some(due) = due else { return false };
        match self {
            // Mirrors `adapters::datetime::is_overdue`: a finished task is not late.
            Self::Overdue => !completed && due < now,
            Self::Within { days } => {
                *due <= end_of_day_after(now, days.saturating_sub(1), timezone)
            }
            Self::WithinMinutes { minutes } => {
                let Ok(minutes) = i64::try_from(minutes) else {
                    return true;
                };
                TimeDelta::try_minutes(minutes)
                    .and_then(|span| now.checked_add_signed(span))
                    .is_none_or(|boundary| *due <= boundary)
            }
        }
    }
}

/// Parses the `@<number><unit>` form, e.g. `10日`, `3days`, `2period`.
///
/// The count includes today, matching the fixed keywords: `@3days` covers today,
/// tomorrow and the day after, exactly like the "3 days" filter button.
fn parse_counted(keyword: &str) -> Option<DueKeyword> {
    let digits: String = keyword.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let unit = &keyword[digits.len()..];
    let count = digits.parse::<u64>().ok()?;

    if let Some((_, per_unit)) = MINUTE_UNITS.iter().find(|(name, _)| *name == unit) {
        // A zero horizon would match nothing at all; read it as one minute.
        let minutes = count.max(1).checked_mul(*per_unit)?;
        return Some(DueKeyword::WithinMinutes { minutes });
    }

    let per_unit = UNITS
        .iter()
        .find(|(name, _)| *name == unit)
        .map(|(_, days)| *days)?;

    // A count of zero is meaningless as a horizon; read it as "today".
    let days = count.max(1).checked_mul(per_unit)?;
    Some(DueKeyword::Within { days })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    #[test]
    fn japanese_and_english_spellings_agree() {
        assert_eq!(DueKeyword::parse("今日"), DueKeyword::parse("today"));
        assert_eq!(DueKeyword::parse("期限切れ"), Some(DueKeyword::Overdue));
        assert_eq!(
            DueKeyword::parse("今週"),
            Some(DueKeyword::Within { days: 7 })
        );
    }

    #[test]
    fn the_filter_buttons_all_map_to_a_keyword() {
        for query in [
            "overdue",
            "today",
            "tomorrow",
            "3days",
            "week",
            "month",
            "quarter",
            "year",
            "fiscalyear",
        ] {
            assert!(
                DueKeyword::parse(query).is_some(),
                "the sidebar button `@{query}` has no matching keyword"
            );
        }
    }

    #[test]
    fn a_custom_filter_button_maps_to_a_keyword() {
        use crate::viewmodels::settings::{CustomDueFilter, CustomDueUnit};

        // The sidebar counts a custom filter by parsing the query it writes into
        // the search box, so the two spellings must not drift apart.
        for (filter, expected) in [
            (
                CustomDueFilter::new(10, CustomDueUnit::Minute),
                DueKeyword::WithinMinutes { minutes: 10 },
            ),
            (
                CustomDueFilter::new(1, CustomDueUnit::Hour),
                DueKeyword::WithinMinutes { minutes: 60 },
            ),
            (
                CustomDueFilter::new(5, CustomDueUnit::Day),
                DueKeyword::Within { days: 5 },
            ),
        ] {
            let query = filter.query();
            assert_eq!(
                DueKeyword::parse(query.trim_start_matches('@')),
                Some(expected),
                "the custom filter `{query}` has no matching keyword"
            );
        }
    }

    #[test]
    fn a_counted_keyword_includes_today() {
        // "@3日" is documented as today, tomorrow and the day after.
        assert_eq!(
            DueKeyword::parse("3日"),
            Some(DueKeyword::Within { days: 3 })
        );
        assert_eq!(DueKeyword::parse("2日"), DueKeyword::parse("明日"));
        assert_eq!(
            DueKeyword::parse("2か月"),
            Some(DueKeyword::Within { days: 60 })
        );
    }

    #[test]
    fn a_sub_day_keyword_is_measured_from_now() {
        assert_eq!(
            DueKeyword::parse("10分"),
            Some(DueKeyword::WithinMinutes { minutes: 10 })
        );
        assert_eq!(DueKeyword::parse("10minutes"), DueKeyword::parse("10分"));
        assert_eq!(
            DueKeyword::parse("1時間"),
            Some(DueKeyword::WithinMinutes { minutes: 60 })
        );
        assert_eq!(DueKeyword::parse("2hours"), DueKeyword::parse("120min"));

        // The boundary is exactly now plus the horizon, not the end of the day.
        let now = utc(2026, 3, 1, 9);
        let keyword = DueKeyword::parse("1hour").expect("`1hour` is a keyword");
        for (due, expected) in [
            (Utc.with_ymd_and_hms(2026, 3, 1, 9, 59, 0).unwrap(), true),
            (Utc.with_ymd_and_hms(2026, 3, 1, 10, 1, 0).unwrap(), false),
        ] {
            assert_eq!(
                keyword.matches(Some(&due), false, &now, DisplayTimezone::Utc),
                expected,
                "due {due} against now {now}"
            );
        }
    }

    #[test]
    fn an_unknown_keyword_is_rejected() {
        assert_eq!(DueKeyword::parse("someone"), None);
        assert_eq!(DueKeyword::parse("10"), None);
        assert_eq!(DueKeyword::parse(""), None);
    }

    #[test]
    fn today_covers_the_whole_local_day_but_not_the_next() {
        let now = utc(2026, 3, 1, 9);
        let keyword = DueKeyword::parse("today").expect("`today` is a keyword");

        for (due, expected) in [
            (utc(2026, 3, 1, 23), true),
            (utc(2026, 2, 28, 8), true),
            (utc(2026, 3, 2, 0), false),
        ] {
            assert_eq!(
                keyword.matches(Some(&due), false, &now, DisplayTimezone::Utc),
                expected,
                "due {due} against now {now}"
            );
        }
    }

    #[test]
    fn a_finished_task_is_never_overdue() {
        let now = utc(2026, 3, 1, 9);
        let due = utc(2026, 2, 1, 9);

        assert!(DueKeyword::Overdue.matches(Some(&due), false, &now, DisplayTimezone::Utc));
        assert!(!DueKeyword::Overdue.matches(Some(&due), true, &now, DisplayTimezone::Utc));
    }

    #[test]
    fn a_task_without_a_due_date_matches_no_filter() {
        let now = utc(2026, 3, 1, 9);

        for keyword in [DueKeyword::Overdue, DueKeyword::Within { days: 365 }] {
            assert!(!keyword.matches(None, false, &now, DisplayTimezone::Utc));
        }
    }
}
