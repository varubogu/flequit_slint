//! Due-date keywords accepted by the search box.
//!
//! The vocabulary is defined in `docs/ja/develop/design/ui/page/main/search.md`.
//! Every language's spelling is accepted regardless of the UI language, and a
//! keyword is written back in the UI language's own spelling ([`DueSpec::spell`]).
//! The internal query stores the language-neutral [`DueSpec::key`] instead.

use chrono::{DateTime, TimeDelta, Utc};

use super::vocabulary::Lang;
use crate::adapters::datetime::{DisplayTimezone, end_of_day_after};

/// A due-date condition as matched against a task.
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

/// The named horizons, each with a word of its own in every language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DueName {
    Overdue,
    Today,
    Tomorrow,
    Week,
    Month,
    Quarter,
    Year,
    FiscalYear,
}

/// A due-date keyword as the user wrote it: a named horizon or a count.
///
/// Kept apart from [`DueKeyword`] so that `@今年度` is written back as
/// `@今年度` rather than as the 365 days it currently means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DueSpec {
    Named(DueName),
    /// `@3日`: the next `n` calendar days, today included.
    Days(u32),
    /// `@10分`: the next `n` minutes from now.
    Minutes(u32),
}

/// Words that map to a named horizon, compared after folding.
///
/// The first spelling per language is the one written back ([`DueSpec::spell`]).
const NAMES: &[(DueName, &[&str], &[&str])] = &[
    (DueName::Overdue, &["overdue", "deadline"], &["期限切れ"]),
    (DueName::Today, &["today"], &["今日", "本日"]),
    (DueName::Tomorrow, &["tomorrow"], &["明日", "翌日"]),
    (DueName::Week, &["week", "weeks"], &["今週", "週"]),
    (DueName::Month, &["month", "months"], &["今月", "月"]),
    (DueName::Quarter, &["quarter", "period"], &["今期", "期"]),
    (DueName::Year, &["year", "years"], &["今年"]),
    (
        DueName::FiscalYear,
        &["fiscalyear", "fiscal-year"],
        &["今年度", "年度"],
    ),
];

/// Words for a fixed count of days that have no [`DueName`] of their own.
const DAY_WORDS: &[(&str, u32)] = &[("明後日", 3)];

/// Sub-day units accepted after a number, and how many minutes one unit covers.
///
/// Kept apart from [`UNITS`] because these horizons are measured from now
/// rather than rounded up to the end of a calendar day.
///
/// Longer spellings come first so that `時間` is not read as `時`.
const MINUTE_UNITS: &[(&str, u32)] = &[
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
const UNITS: &[(&str, u32)] = &[
    ("年度", 365),
    ("fiscalyear", 365),
    ("日", 1),
    ("day", 1),
    ("days", 1),
    ("d", 1),
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

impl DueName {
    /// Every named horizon, in the order the sidebar and suggestions list them.
    pub const ALL: [Self; 8] = [
        Self::Overdue,
        Self::Today,
        Self::Tomorrow,
        Self::Week,
        Self::Month,
        Self::Quarter,
        Self::Year,
        Self::FiscalYear,
    ];

    fn keyword(self) -> DueKeyword {
        match self {
            Self::Overdue => DueKeyword::Overdue,
            Self::Today => DueKeyword::Within { days: 1 },
            Self::Tomorrow => DueKeyword::Within { days: 2 },
            Self::Week => DueKeyword::Within { days: 7 },
            Self::Month => DueKeyword::Within { days: 30 },
            // `@今期` / `@今年度` are approximated from today until a period
            // start and a fiscal-year start can be configured.
            Self::Quarter => DueKeyword::Within { days: 90 },
            Self::Year | Self::FiscalYear => DueKeyword::Within { days: 365 },
        }
    }

    fn spellings(self) -> (&'static [&'static str], &'static [&'static str]) {
        NAMES
            .iter()
            .find(|(name, _, _)| *name == self)
            .map(|(_, en, ja)| (*en, *ja))
            .expect("every due name has spellings")
    }

    fn key(self) -> &'static str {
        self.spellings().0[0]
    }
}

impl DueSpec {
    /// Parses the text after the `@`, e.g. `today`, `今日` or `10日`.
    ///
    /// The input is expected to be folded already (see `normalize::fold`), but
    /// ASCII case is folded here too so direct callers stay simple.
    pub fn parse(word: &str) -> Option<Self> {
        let word = word.to_lowercase();
        NAMES
            .iter()
            .find(|(_, en, ja)| en.contains(&word.as_str()) || ja.contains(&word.as_str()))
            .map(|(name, _, _)| Self::Named(*name))
            .or_else(|| {
                DAY_WORDS
                    .iter()
                    .find(|(spelling, _)| *spelling == word)
                    .map(|(_, days)| Self::Days(*days))
            })
            .or_else(|| parse_counted(&word))
    }

    /// Every spelling of this keyword in every language, for matching what is
    /// being typed against it.
    pub fn spellings(self) -> Vec<String> {
        match self {
            Self::Named(name) => {
                let (en, ja) = name.spellings();
                en.iter()
                    .chain(ja)
                    .map(|word| (*word).to_string())
                    .collect()
            }
            _ => vec![self.spell(Lang::En), self.spell(Lang::Ja)],
        }
    }

    /// The condition this keyword stands for.
    pub fn keyword(self) -> DueKeyword {
        match self {
            Self::Named(name) => name.keyword(),
            Self::Days(days) => DueKeyword::Within {
                days: u64::from(days.max(1)),
            },
            Self::Minutes(minutes) => DueKeyword::WithinMinutes {
                minutes: u64::from(minutes.max(1)),
            },
        }
    }

    /// The language-neutral spelling kept in the internal query.
    pub fn key(self) -> String {
        match self {
            Self::Named(name) => name.key().to_string(),
            Self::Days(days) => format!("{days}d"),
            Self::Minutes(minutes) => format!("{minutes}min"),
        }
    }

    /// Reads back what [`Self::key`] wrote.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::parse(key)
    }

    /// The spelling written into the search box for `lang`.
    pub fn spell(self, lang: Lang) -> String {
        match (self, lang) {
            (Self::Named(name), Lang::En) => name.spellings().0[0].to_string(),
            (Self::Named(name), Lang::Ja) => name.spellings().1[0].to_string(),
            (Self::Days(days), Lang::En) => format!("{days}days"),
            (Self::Days(days), Lang::Ja) => format!("{days}日"),
            (Self::Minutes(minutes), Lang::En) if minutes % 60 == 0 => {
                format!("{}hours", minutes / 60)
            }
            (Self::Minutes(minutes), Lang::Ja) if minutes % 60 == 0 => {
                format!("{}時間", minutes / 60)
            }
            (Self::Minutes(minutes), Lang::En) => format!("{minutes}min"),
            (Self::Minutes(minutes), Lang::Ja) => format!("{minutes}分"),
        }
    }
}

impl DueKeyword {
    /// Parses the text after the `@`, e.g. `today` or `10日`.
    ///
    /// Returns `None` for anything that is not a due keyword; the caller decides
    /// what an unrecognised keyword means.
    pub fn parse(keyword: &str) -> Option<Self> {
        DueSpec::parse(keyword).map(DueSpec::keyword)
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
fn parse_counted(keyword: &str) -> Option<DueSpec> {
    let digits: String = keyword.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let unit = &keyword[digits.len()..];
    let count = digits.parse::<u32>().ok()?;

    if let Some((_, per_unit)) = MINUTE_UNITS.iter().find(|(name, _)| *name == unit) {
        // A zero horizon would match nothing at all; read it as one minute.
        return count.max(1).checked_mul(*per_unit).map(DueSpec::Minutes);
    }

    let per_unit = UNITS
        .iter()
        .find(|(name, _)| *name == unit)
        .map(|(_, days)| *days)?;

    // A count of zero is meaningless as a horizon; read it as "today".
    count.max(1).checked_mul(per_unit).map(DueSpec::Days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    #[test]
    fn a_spec_round_trips_through_its_key_and_every_spelling() {
        let specs = DueName::ALL
            .iter()
            .map(|name| DueSpec::Named(*name))
            .chain([
                DueSpec::Days(3),
                DueSpec::Minutes(10),
                DueSpec::Minutes(120),
            ]);
        for spec in specs {
            assert_eq!(
                DueSpec::from_key(&spec.key()),
                Some(spec),
                "key of {spec:?}"
            );
            for lang in [Lang::En, Lang::Ja] {
                assert_eq!(
                    DueSpec::parse(&spec.spell(lang)),
                    Some(spec),
                    "{lang:?} {spec:?}"
                );
            }
        }
    }

    #[test]
    fn a_named_keyword_keeps_its_name_when_written_back() {
        assert_eq!(
            DueSpec::parse("今年度").map(|spec| spec.spell(Lang::En)),
            Some("fiscalyear".into())
        );
        assert_eq!(
            DueSpec::parse("today").map(|spec| spec.spell(Lang::Ja)),
            Some("今日".into())
        );
        assert_eq!(
            DueSpec::parse("10minutes").map(|spec| spec.spell(Lang::Ja)),
            Some("10分".into())
        );
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
