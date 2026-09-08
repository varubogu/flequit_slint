//! `RecurrenceRule` → the Slint types the repeat editor works with.
//!
//! The domain enums carry more units than the editor advertises, so every
//! variant is mapped rather than folded into a default: a rule written by
//! another Flequit client has to survive being opened and saved here.

use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::recurrence_rule::RecurrenceRule;
use flequit_model::types::datetime_calendar_types::{
    DayOfWeek, RecurrenceUnit as DomainUnit, WeekOfMonth as DomainWeekOfMonth,
};
use slint::SharedString;

use super::datetime::{DateTimeParts, DisplayTimezone, to_display_parts};
use crate::bindings::{
    RecurrenceEnd, RecurrenceMonthlyMode, RecurrenceState, RecurrenceUnit, RecurrenceWeekOfMonth,
};

/// How many times a new rule repeats before the editor's count field kicks in.
const DEFAULT_MAX_OCCURRENCES: i32 = 10;

/// How many future dates an endless rule initially shows.
const DEFAULT_PREVIEW_COUNT: i32 = 5;

/// Maps the domain period to its UI counterpart.
pub fn to_unit(unit: &DomainUnit) -> RecurrenceUnit {
    match unit {
        DomainUnit::Minute => RecurrenceUnit::Minute,
        DomainUnit::Hour => RecurrenceUnit::Hour,
        DomainUnit::Day => RecurrenceUnit::Day,
        DomainUnit::Week => RecurrenceUnit::Week,
        DomainUnit::Month => RecurrenceUnit::Month,
        DomainUnit::Quarter => RecurrenceUnit::Quarter,
        DomainUnit::HalfYear => RecurrenceUnit::HalfYear,
        DomainUnit::Year => RecurrenceUnit::Year,
    }
}

/// Maps a period chosen in the editor back to the domain enum.
pub fn from_unit(unit: RecurrenceUnit) -> DomainUnit {
    match unit {
        RecurrenceUnit::Minute => DomainUnit::Minute,
        RecurrenceUnit::Hour => DomainUnit::Hour,
        RecurrenceUnit::Day => DomainUnit::Day,
        RecurrenceUnit::Week => DomainUnit::Week,
        RecurrenceUnit::Month => DomainUnit::Month,
        RecurrenceUnit::Quarter => DomainUnit::Quarter,
        RecurrenceUnit::HalfYear => DomainUnit::HalfYear,
        RecurrenceUnit::Year => DomainUnit::Year,
    }
}

pub fn to_week_of_month(week: &DomainWeekOfMonth) -> RecurrenceWeekOfMonth {
    match week {
        DomainWeekOfMonth::First => RecurrenceWeekOfMonth::First,
        DomainWeekOfMonth::Second => RecurrenceWeekOfMonth::Second,
        DomainWeekOfMonth::Third => RecurrenceWeekOfMonth::Third,
        DomainWeekOfMonth::Fourth => RecurrenceWeekOfMonth::Fourth,
        DomainWeekOfMonth::Last => RecurrenceWeekOfMonth::Last,
    }
}

pub fn from_week_of_month(week: RecurrenceWeekOfMonth) -> DomainWeekOfMonth {
    match week {
        RecurrenceWeekOfMonth::First => DomainWeekOfMonth::First,
        RecurrenceWeekOfMonth::Second => DomainWeekOfMonth::Second,
        RecurrenceWeekOfMonth::Third => DomainWeekOfMonth::Third,
        RecurrenceWeekOfMonth::Fourth => DomainWeekOfMonth::Fourth,
        RecurrenceWeekOfMonth::Last => DomainWeekOfMonth::Last,
    }
}

/// Weekday as the zero-based index the UI counts with, Sunday first.
///
/// # Examples
///
/// ```
/// use flequit_model::types::datetime_calendar_types::DayOfWeek;
/// use flequit_ui::adapters::recurrence::{to_weekday_index, from_weekday_index};
///
/// assert_eq!(to_weekday_index(&DayOfWeek::Sunday), 0);
/// assert_eq!(to_weekday_index(&DayOfWeek::Saturday), 6);
/// assert_eq!(to_weekday_index(&from_weekday_index(3)), 3);
/// ```
pub fn to_weekday_index(day: &DayOfWeek) -> i32 {
    match day {
        DayOfWeek::Sunday => 0,
        DayOfWeek::Monday => 1,
        DayOfWeek::Tuesday => 2,
        DayOfWeek::Wednesday => 3,
        DayOfWeek::Thursday => 4,
        DayOfWeek::Friday => 5,
        DayOfWeek::Saturday => 6,
    }
}

/// The weekday an index stands for. Anything outside 0–6 wraps to Sunday.
pub fn from_weekday_index(index: i32) -> DayOfWeek {
    match index.rem_euclid(7) {
        1 => DayOfWeek::Monday,
        2 => DayOfWeek::Tuesday,
        3 => DayOfWeek::Wednesday,
        4 => DayOfWeek::Thursday,
        5 => DayOfWeek::Friday,
        6 => DayOfWeek::Saturday,
        _ => DayOfWeek::Sunday,
    }
}

/// The editor's working copy of a task's rule.
///
/// `rule` is `None` for a task that does not repeat yet; the state is still
/// filled in, so switching the checkbox on shows a usable draft rather than a
/// form of zeroes. `anchor` supplies those defaults — it is the task's due date
/// when it has one, and the current time otherwise.
pub fn to_recurrence_state(
    task_id: &str,
    rule: Option<&RecurrenceRule>,
    anchor: &DateTime<Utc>,
    timezone: DisplayTimezone,
) -> RecurrenceState {
    let anchor_parts = to_display_parts(anchor, timezone);
    let details = rule.and_then(|rule| rule.details.as_ref());
    let selected_days = rule.and_then(|rule| rule.days_of_week.as_ref());
    let weekday = |day: DayOfWeek| {
        selected_days.is_some_and(|days| {
            days.iter()
                .any(|candidate| to_weekday_index(candidate) == to_weekday_index(&day))
        })
    };

    let end_parts = rule
        .and_then(|rule| rule.end_date.as_ref())
        .map_or(anchor_parts, |end| to_display_parts(end, timezone));

    RecurrenceState {
        task_id: SharedString::from(task_id),
        enabled: rule.is_some(),
        unit: rule.map_or(RecurrenceUnit::Day, |rule| to_unit(&rule.unit)),
        interval: rule.map_or(1, |rule| rule.interval.max(1)),
        sunday: weekday(DayOfWeek::Sunday),
        monday: weekday(DayOfWeek::Monday),
        tuesday: weekday(DayOfWeek::Tuesday),
        wednesday: weekday(DayOfWeek::Wednesday),
        thursday: weekday(DayOfWeek::Thursday),
        friday: weekday(DayOfWeek::Friday),
        saturday: weekday(DayOfWeek::Saturday),
        monthly_mode: monthly_mode(details),
        day_of_month: details
            .and_then(|details| details.specific_date)
            .unwrap_or(anchor_parts.day)
            .clamp(1, 31),
        week_of_month: details
            .and_then(|details| details.week_of_period.as_ref())
            .map_or(RecurrenceWeekOfMonth::First, to_week_of_month),
        weekday_of_month: details
            .and_then(|details| details.weekday_of_week.as_ref())
            .map_or_else(|| weekday_index_of(anchor_parts), to_weekday_index),
        end_kind: end_kind(rule),
        end_year: end_parts.year,
        end_month: end_parts.month,
        end_day: end_parts.day,
        max_occurrences: rule
            .and_then(|rule| rule.max_occurrences)
            .unwrap_or(DEFAULT_MAX_OCCURRENCES)
            .max(1),
        preview_count: DEFAULT_PREVIEW_COUNT,
    }
}

fn monthly_mode(
    details: Option<&flequit_model::models::task_projects::recurrence_details::RecurrenceDetails>,
) -> RecurrenceMonthlyMode {
    match details {
        Some(details) if details.specific_date.is_some() => RecurrenceMonthlyMode::DayOfMonth,
        Some(details) if details.week_of_period.is_some() && details.weekday_of_week.is_some() => {
            RecurrenceMonthlyMode::WeekdayOfMonth
        }
        _ => RecurrenceMonthlyMode::SameDay,
    }
}

/// A rule with both an end date and a count is shown as ending on the date:
/// the editor offers one termination, and the date is the more visible of the
/// two. Neither value is dropped until the rule is saved again.
fn end_kind(rule: Option<&RecurrenceRule>) -> RecurrenceEnd {
    match rule {
        Some(rule) if rule.end_date.is_some() => RecurrenceEnd::OnDate,
        Some(rule) if rule.max_occurrences.is_some() => RecurrenceEnd::AfterCount,
        _ => RecurrenceEnd::Never,
    }
}

/// Weekday index of a calendar date, without going back through chrono's
/// timezone machinery: the parts are already in the display timezone.
fn weekday_index_of(parts: DateTimeParts) -> i32 {
    chrono::NaiveDate::from_ymd_opt(parts.year, parts.month as u32, parts.day as u32)
        .map_or(0, |date| {
            chrono::Datelike::weekday(&date).num_days_from_sunday() as i32
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use flequit_model::types::id_types::{RecurrenceRuleId, UserId};

    fn rule() -> RecurrenceRule {
        let now = Utc.with_ymd_and_hms(2026, 9, 6, 12, 0, 0).unwrap();
        RecurrenceRule {
            id: RecurrenceRuleId::new(),
            unit: DomainUnit::Week,
            interval: 2,
            days_of_week: Some(vec![DayOfWeek::Tuesday, DayOfWeek::Thursday]),
            details: None,
            adjustment: None,
            end_date: None,
            max_occurrences: None,
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
        }
    }

    #[test]
    fn every_unit_survives_a_round_trip() {
        for unit in [
            DomainUnit::Minute,
            DomainUnit::Hour,
            DomainUnit::Day,
            DomainUnit::Week,
            DomainUnit::Month,
            DomainUnit::Quarter,
            DomainUnit::HalfYear,
            DomainUnit::Year,
        ] {
            let mapped = from_unit(to_unit(&unit));
            assert_eq!(to_unit(&mapped), to_unit(&unit));
        }
    }

    #[test]
    fn a_task_without_a_rule_still_gets_a_usable_draft() {
        let anchor = Utc.with_ymd_and_hms(2026, 9, 6, 12, 0, 0).unwrap();

        let state = to_recurrence_state("t1", None, &anchor, DisplayTimezone::Utc);

        assert!(!state.enabled);
        assert_eq!(state.unit, RecurrenceUnit::Day);
        assert_eq!(state.interval, 1);
        assert_eq!(state.day_of_month, 6);
        assert_eq!(state.max_occurrences, DEFAULT_MAX_OCCURRENCES);
        assert_eq!(state.preview_count, DEFAULT_PREVIEW_COUNT);
        assert_eq!(state.end_kind, RecurrenceEnd::Never);
    }

    #[test]
    fn the_selected_weekdays_come_back_as_flags() {
        let anchor = Utc.with_ymd_and_hms(2026, 9, 6, 12, 0, 0).unwrap();

        let state = to_recurrence_state("t1", Some(&rule()), &anchor, DisplayTimezone::Utc);

        assert!(state.enabled);
        assert!(state.tuesday && state.thursday);
        assert!(!state.sunday && !state.monday && !state.saturday);
        assert_eq!(state.interval, 2);
    }

    #[test]
    fn an_end_date_wins_over_a_count_in_the_editor() {
        let anchor = Utc.with_ymd_and_hms(2026, 9, 6, 12, 0, 0).unwrap();
        let mut rule = rule();
        rule.end_date = Some(Utc.with_ymd_and_hms(2026, 12, 24, 23, 59, 0).unwrap());
        rule.max_occurrences = Some(4);

        let state = to_recurrence_state("t1", Some(&rule), &anchor, DisplayTimezone::Utc);

        assert_eq!(state.end_kind, RecurrenceEnd::OnDate);
        assert_eq!(
            (state.end_year, state.end_month, state.end_day),
            (2026, 12, 24)
        );
        assert_eq!(state.max_occurrences, 4);
    }
}
