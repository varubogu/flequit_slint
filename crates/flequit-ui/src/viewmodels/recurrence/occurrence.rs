//! Expanding a recurrence rule into the dates it would actually fire on.
//!
//! Reading a rule is a domain judgement, not layout, so it happens here and the
//! view only renders the resulting strings
//! (`docs/ja/develop/design/ui/slint-patterns.md`).
//!
//! Everything is computed on the calendar the user sees: the anchor is moved
//! into the display timezone, stepped there, and converted back to UTC at the
//! end. Stepping in UTC would drift an "every day at 09:00" rule by an hour
//! across a daylight-saving boundary.

use chrono::{
    DateTime, Datelike, Days, Months, NaiveDate, NaiveDateTime, TimeDelta, Timelike, Utc, Weekday,
};
use flequit_model::models::task_projects::recurrence_rule::RecurrenceRule;
use flequit_model::models::task_projects::weekday_condition::WeekdayCondition;
use flequit_model::types::datetime_calendar_types::{
    AdjustmentDirection, AdjustmentTarget, DateRelation, DayOfWeek, RecurrenceUnit, WeekOfMonth,
};

use crate::adapters::datetime::{
    DateTimeParts, DisplayTimezone, from_display_parts, to_display_parts,
};

/// How many periods are examined before giving up.
///
/// A rule can describe a date that never comes round — "the 31st, every month,
/// moved to the previous Tuesday" filtered by a date condition that excludes it
/// — and the preview must still return rather than spin.
const MAX_PERIODS: usize = 500;

/// How far a weekday adjustment will look for a day that fits.
const ADJUSTMENT_SPAN: u64 = 14;

/// The next dates `rule` fires on after `anchor`, at most `limit` of them.
///
/// `anchor` is treated as the occurrence the rule counts from — the task's due
/// date when it has one. It is never included in the result: the preview
/// answers "and then when?".
pub fn next_occurrences(
    rule: &RecurrenceRule,
    anchor: &DateTime<Utc>,
    timezone: DisplayTimezone,
    limit: usize,
) -> Vec<DateTime<Utc>> {
    if limit == 0 || rule.interval <= 0 {
        return Vec::new();
    }
    let Some(anchor_local) = to_local(anchor, timezone) else {
        return Vec::new();
    };

    // `max_occurrences` counts the whole series and the anchor is its first
    // occurrence, so only the remainder is still ahead.
    let wanted = match rule.max_occurrences {
        Some(max) => limit.min((max - 1).max(0) as usize),
        None => limit,
    };
    if wanted == 0 {
        return Vec::new();
    }

    let end = rule
        .end_date
        .as_ref()
        .and_then(|end| to_local(end, timezone));
    let conditions = date_conditions(rule, timezone);

    let mut found: Vec<NaiveDateTime> = Vec::new();
    'periods: for period in 0..MAX_PERIODS {
        for base in period_candidates(rule, anchor_local, period) {
            let candidate = adjusted(rule, base);
            if candidate <= anchor_local {
                continue;
            }
            if end.is_some_and(|end| candidate > end) {
                break 'periods;
            }
            if !conditions
                .iter()
                .all(|(relation, reference)| satisfies(candidate, relation, *reference))
            {
                continue;
            }
            if found.contains(&candidate) {
                continue;
            }
            found.push(candidate);
            if found.len() >= wanted {
                break 'periods;
            }
        }
    }

    found
        .into_iter()
        .filter_map(|value| to_utc(value, timezone))
        .collect()
}

/// The candidate dates the `period`-th repetition offers, in ascending order.
///
/// A period yields more than one date only for a weekly rule that names several
/// weekdays. Periods start at zero so that a monthly rule can still fire later
/// in the anchor's own month; the anchor itself is filtered out by the caller.
fn period_candidates(
    rule: &RecurrenceRule,
    anchor: NaiveDateTime,
    period: usize,
) -> Vec<NaiveDateTime> {
    let interval = rule.interval.max(1) as i64;
    let steps = interval * period as i64;

    match rule.unit {
        RecurrenceUnit::Minute => anchor
            .checked_add_signed(TimeDelta::minutes(steps))
            .into_iter()
            .collect(),
        RecurrenceUnit::Hour => anchor
            .checked_add_signed(TimeDelta::hours(steps))
            .into_iter()
            .collect(),
        RecurrenceUnit::Day => anchor
            .checked_add_signed(TimeDelta::days(steps))
            .into_iter()
            .collect(),
        RecurrenceUnit::Week => week_candidates(rule, anchor, steps),
        RecurrenceUnit::Month => month_candidates(rule, anchor, steps),
        RecurrenceUnit::Quarter => month_candidates(rule, anchor, steps * 3),
        RecurrenceUnit::HalfYear => month_candidates(rule, anchor, steps * 6),
        RecurrenceUnit::Year => month_candidates(rule, anchor, steps * 12),
    }
}

/// A weekly rule fires on the weekdays it names, or on the anchor's own weekday
/// when it names none.
///
/// Weeks are counted from Sunday, matching the order of [`DayOfWeek`], so that
/// "every two weeks on Monday and Friday" keeps both days in the same week.
fn week_candidates(rule: &RecurrenceRule, anchor: NaiveDateTime, steps: i64) -> Vec<NaiveDateTime> {
    let selected: Vec<Weekday> = rule
        .days_of_week
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(weekday_of)
        .collect();

    let Ok(offset) = u64::try_from(steps * 7) else {
        return Vec::new();
    };
    if selected.is_empty() {
        return anchor
            .checked_add_days(Days::new(offset))
            .into_iter()
            .collect();
    }

    let date = anchor.date();
    let week_start = date - Days::new(u64::from(date.weekday().num_days_from_sunday()));
    let Some(base) = week_start.checked_add_days(Days::new(offset)) else {
        return Vec::new();
    };

    let mut days: Vec<u32> = selected
        .iter()
        .map(|weekday| weekday.num_days_from_sunday())
        .collect();
    days.sort_unstable();
    days.dedup();

    days.into_iter()
        .filter_map(|day| base.checked_add_days(Days::new(u64::from(day))))
        .map(|date| date.and_time(anchor.time()))
        .collect()
}

/// A monthly (or quarterly, half-yearly, yearly) rule fires once per period.
///
/// A period that cannot supply the requested day — the 31st of a short month,
/// or a fifth Wednesday that does not exist — yields nothing and is skipped,
/// rather than being clamped to a day the user did not ask for.
fn month_candidates(
    rule: &RecurrenceRule,
    anchor: NaiveDateTime,
    months: i64,
) -> Vec<NaiveDateTime> {
    let Ok(months) = u32::try_from(months) else {
        return Vec::new();
    };
    let Some(first) = anchor
        .date()
        .with_day(1)
        .and_then(|date| date.checked_add_months(Months::new(months)))
    else {
        return Vec::new();
    };

    let details = rule.details.as_ref();
    let date = match details {
        Some(details) if details.specific_date.is_some() => details
            .specific_date
            .and_then(|day| u32::try_from(day).ok())
            .and_then(|day| first.with_day(day)),
        Some(details) => match (
            details.week_of_period.as_ref(),
            details.weekday_of_week.as_ref(),
        ) {
            (Some(week), Some(weekday)) => nth_weekday(first, week, weekday_of(weekday)),
            _ => first.with_day(anchor.day()),
        },
        None => first.with_day(anchor.day()),
    };

    date.map(|date| date.and_time(anchor.time()))
        .into_iter()
        .collect()
}

/// The date of the `week`-th `weekday` in the month `first` starts.
fn nth_weekday(first: NaiveDate, week: &WeekOfMonth, weekday: Weekday) -> Option<NaiveDate> {
    let shift = (7 + weekday.num_days_from_sunday() - first.weekday().num_days_from_sunday()) % 7;
    let earliest = first.checked_add_days(Days::new(u64::from(shift)))?;

    match week {
        WeekOfMonth::First => Some(earliest),
        WeekOfMonth::Second => in_same_month(earliest, 1),
        WeekOfMonth::Third => in_same_month(earliest, 2),
        WeekOfMonth::Fourth => in_same_month(earliest, 3),
        WeekOfMonth::Last => {
            let mut last = earliest;
            while let Some(next) = last.checked_add_days(Days::new(7)) {
                if next.month() != earliest.month() {
                    break;
                }
                last = next;
            }
            Some(last)
        }
    }
}

fn in_same_month(from: NaiveDate, weeks: u64) -> Option<NaiveDate> {
    let date = from.checked_add_days(Days::new(weeks * 7))?;
    (date.month() == from.month()).then_some(date)
}

/// Applies the rule's weekday corrections, such as "move a Saturday backwards
/// to the previous working day".
fn adjusted(rule: &RecurrenceRule, base: NaiveDateTime) -> NaiveDateTime {
    let Some(adjustment) = rule.adjustment.as_ref() else {
        return base;
    };

    let mut date = base.date();
    for condition in &adjustment.weekday_conditions {
        if condition.deleted || date.weekday() != weekday_of(&condition.if_weekday) {
            continue;
        }
        date = shift(date, condition).unwrap_or(date);
    }
    date.and_time(base.time())
}

fn shift(date: NaiveDate, condition: &WeekdayCondition) -> Option<NaiveDate> {
    match condition.then_target {
        AdjustmentTarget::Days => {
            let days = Days::new(u64::from(condition.then_days.unwrap_or(0).unsigned_abs()));
            match condition.then_direction {
                AdjustmentDirection::Previous => date.checked_sub_days(days),
                _ => date.checked_add_days(days),
            }
        }
        AdjustmentTarget::SpecificWeekday => {
            let target = weekday_of(condition.then_weekday.as_ref()?);
            search(date, &condition.then_direction, |candidate| {
                candidate.weekday() == target
            })
        }
        AdjustmentTarget::Weekday | AdjustmentTarget::NonWeekend => {
            search(date, &condition.then_direction, |candidate| {
                !is_weekend(candidate)
            })
        }
        AdjustmentTarget::Weekend | AdjustmentTarget::WeekendOnly => {
            search(date, &condition.then_direction, is_weekend)
        }
        // The holiday-aware targets need a holiday calendar, which the app does
        // not have. Leaving the date where the rule put it keeps the preview
        // honest about the part that is understood.
        _ => Some(date),
    }
}

/// The nearest day in `direction` that `matches`, within [`ADJUSTMENT_SPAN`].
///
/// `Nearest` looks backwards first, so a date equally far from both is moved
/// earlier — a deadline is better met early than late.
fn search(
    date: NaiveDate,
    direction: &AdjustmentDirection,
    matches: impl Fn(&NaiveDate) -> bool,
) -> Option<NaiveDate> {
    (1..=ADJUSTMENT_SPAN).find_map(|offset| {
        let previous = || date.checked_sub_days(Days::new(offset)).filter(&matches);
        let next = || date.checked_add_days(Days::new(offset)).filter(&matches);
        match direction {
            AdjustmentDirection::Previous => previous(),
            AdjustmentDirection::Next => next(),
            AdjustmentDirection::Nearest => previous().or_else(next),
        }
    })
}

fn is_weekend(date: &NaiveDate) -> bool {
    matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

/// The date conditions a candidate has to satisfy, from both the pattern and
/// the correction block, resolved into the display timezone once.
fn date_conditions(
    rule: &RecurrenceRule,
    timezone: DisplayTimezone,
) -> Vec<(DateRelation, NaiveDateTime)> {
    let from_details = rule
        .details
        .as_ref()
        .and_then(|details| details.date_conditions.as_deref())
        .unwrap_or_default();
    let from_adjustment = rule
        .adjustment
        .as_ref()
        .map(|adjustment| adjustment.date_conditions.as_slice())
        .unwrap_or_default();

    from_details
        .iter()
        .chain(from_adjustment)
        .filter(|condition| !condition.deleted)
        .filter_map(|condition| {
            to_local(&condition.reference_date, timezone)
                .map(|reference| (condition.relation.clone(), reference))
        })
        .collect()
}

fn satisfies(candidate: NaiveDateTime, relation: &DateRelation, reference: NaiveDateTime) -> bool {
    match relation {
        DateRelation::Before => candidate < reference,
        DateRelation::OnOrBefore => candidate <= reference,
        DateRelation::Same => candidate.date() == reference.date(),
        DateRelation::OnOrAfter => candidate >= reference,
        DateRelation::After => candidate > reference,
    }
}

fn weekday_of(day: &DayOfWeek) -> Weekday {
    match day {
        DayOfWeek::Sunday => Weekday::Sun,
        DayOfWeek::Monday => Weekday::Mon,
        DayOfWeek::Tuesday => Weekday::Tue,
        DayOfWeek::Wednesday => Weekday::Wed,
        DayOfWeek::Thursday => Weekday::Thu,
        DayOfWeek::Friday => Weekday::Fri,
        DayOfWeek::Saturday => Weekday::Sat,
    }
}

fn to_local(value: &DateTime<Utc>, timezone: DisplayTimezone) -> Option<NaiveDateTime> {
    let parts = to_display_parts(value, timezone);
    NaiveDate::from_ymd_opt(
        parts.year,
        u32::try_from(parts.month).ok()?,
        u32::try_from(parts.day).ok()?,
    )?
    .and_hms_opt(
        u32::try_from(parts.hour).ok()?,
        u32::try_from(parts.minute).ok()?,
        0,
    )
}

fn to_utc(value: NaiveDateTime, timezone: DisplayTimezone) -> Option<DateTime<Utc>> {
    from_display_parts(
        DateTimeParts {
            year: value.year(),
            month: value.month() as i32,
            day: value.day() as i32,
            hour: value.hour() as i32,
            minute: value.minute() as i32,
        },
        timezone,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use flequit_model::models::task_projects::recurrence_adjustment::RecurrenceAdjustment;
    use flequit_model::models::task_projects::recurrence_details::RecurrenceDetails;
    use flequit_model::types::datetime_calendar_types::{AdjustmentDirection, AdjustmentTarget};
    use flequit_model::types::id_types::{
        RecurrenceAdjustmentId, RecurrenceRuleId, UserId, WeekdayConditionId,
    };

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .unwrap()
    }

    fn rule(unit: RecurrenceUnit, interval: i32) -> RecurrenceRule {
        let now = at(2026, 1, 1, 0, 0);
        RecurrenceRule {
            id: RecurrenceRuleId::new(),
            unit,
            interval,
            days_of_week: None,
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

    fn details() -> RecurrenceDetails {
        let now = at(2026, 1, 1, 0, 0);
        RecurrenceDetails {
            specific_date: None,
            week_of_period: None,
            weekday_of_week: None,
            date_conditions: None,
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
        }
    }

    fn weekday_adjustment(condition: WeekdayCondition) -> RecurrenceAdjustment {
        let now = at(2026, 1, 1, 0, 0);
        RecurrenceAdjustment {
            id: RecurrenceAdjustmentId::new(),
            recurrence_rule_id: RecurrenceRuleId::new(),
            date_conditions: vec![],
            weekday_conditions: vec![condition],
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
        }
    }

    fn condition(
        if_weekday: DayOfWeek,
        direction: AdjustmentDirection,
        target: AdjustmentTarget,
    ) -> WeekdayCondition {
        let now = at(2026, 1, 1, 0, 0);
        WeekdayCondition {
            id: WeekdayConditionId::new(),
            if_weekday,
            then_direction: direction,
            then_target: target,
            then_weekday: None,
            then_days: None,
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
        }
    }

    fn preview(rule: &RecurrenceRule, anchor: DateTime<Utc>, limit: usize) -> Vec<DateTime<Utc>> {
        next_occurrences(rule, &anchor, DisplayTimezone::Utc, limit)
    }

    #[test]
    fn a_daily_rule_steps_by_its_interval_and_never_returns_the_anchor() {
        let anchor = at(2026, 9, 6, 9, 0);

        let dates = preview(&rule(RecurrenceUnit::Day, 3), anchor, 3);

        assert_eq!(
            dates,
            vec![
                at(2026, 9, 9, 9, 0),
                at(2026, 9, 12, 9, 0),
                at(2026, 9, 15, 9, 0)
            ]
        );
    }

    #[test]
    fn a_weekly_rule_fires_on_every_named_weekday_of_the_week() {
        // 2026-09-06 is a Sunday, so Tuesday and Thursday are still ahead of it
        // in the same week.
        let mut weekly = rule(RecurrenceUnit::Week, 2);
        weekly.days_of_week = Some(vec![DayOfWeek::Tuesday, DayOfWeek::Thursday]);

        let dates = preview(&weekly, at(2026, 9, 6, 9, 0), 4);

        assert_eq!(
            dates,
            vec![
                at(2026, 9, 8, 9, 0),
                at(2026, 9, 10, 9, 0),
                at(2026, 9, 22, 9, 0),
                at(2026, 9, 24, 9, 0),
            ]
        );
    }

    #[test]
    fn a_monthly_rule_keeps_the_anchors_day_of_month() {
        let dates = preview(&rule(RecurrenceUnit::Month, 1), at(2026, 1, 15, 9, 0), 2);

        assert_eq!(dates, vec![at(2026, 2, 15, 9, 0), at(2026, 3, 15, 9, 0)]);
    }

    #[test]
    fn a_month_that_cannot_supply_the_day_is_skipped_rather_than_clamped() {
        let mut monthly = rule(RecurrenceUnit::Month, 1);
        monthly.details = Some(RecurrenceDetails {
            specific_date: Some(31),
            ..details()
        });

        let dates = preview(&monthly, at(2026, 1, 31, 9, 0), 3);

        // February and April have no 31st.
        assert_eq!(
            dates,
            vec![
                at(2026, 3, 31, 9, 0),
                at(2026, 5, 31, 9, 0),
                at(2026, 7, 31, 9, 0)
            ]
        );
    }

    #[test]
    fn the_second_tuesday_of_each_month_is_resolved_from_the_pattern() {
        let mut monthly = rule(RecurrenceUnit::Month, 1);
        monthly.details = Some(RecurrenceDetails {
            week_of_period: Some(WeekOfMonth::Second),
            weekday_of_week: Some(DayOfWeek::Tuesday),
            ..details()
        });

        let dates = preview(&monthly, at(2026, 9, 1, 9, 0), 3);

        assert_eq!(
            dates,
            vec![
                at(2026, 9, 8, 9, 0),
                at(2026, 10, 13, 9, 0),
                at(2026, 11, 10, 9, 0)
            ]
        );
    }

    #[test]
    fn the_last_friday_is_the_last_one_that_still_falls_in_the_month() {
        let mut monthly = rule(RecurrenceUnit::Month, 1);
        monthly.details = Some(RecurrenceDetails {
            week_of_period: Some(WeekOfMonth::Last),
            weekday_of_week: Some(DayOfWeek::Friday),
            ..details()
        });

        let dates = preview(&monthly, at(2026, 1, 1, 9, 0), 2);

        assert_eq!(dates, vec![at(2026, 1, 30, 9, 0), at(2026, 2, 27, 9, 0)]);
    }

    #[test]
    fn a_quarterly_rule_steps_three_months_at_a_time() {
        let dates = preview(&rule(RecurrenceUnit::Quarter, 1), at(2026, 1, 10, 9, 0), 2);

        assert_eq!(dates, vec![at(2026, 4, 10, 9, 0), at(2026, 7, 10, 9, 0)]);
    }

    #[test]
    fn an_end_date_stops_the_series() {
        let mut daily = rule(RecurrenceUnit::Day, 1);
        daily.end_date = Some(at(2026, 9, 8, 23, 59));

        let dates = preview(&daily, at(2026, 9, 6, 9, 0), 5);

        assert_eq!(dates, vec![at(2026, 9, 7, 9, 0), at(2026, 9, 8, 9, 0)]);
    }

    #[test]
    fn a_count_limit_counts_the_anchor_as_the_first_occurrence() {
        let mut daily = rule(RecurrenceUnit::Day, 1);
        daily.max_occurrences = Some(3);

        let dates = preview(&daily, at(2026, 9, 6, 9, 0), 5);

        assert_eq!(dates, vec![at(2026, 9, 7, 9, 0), at(2026, 9, 8, 9, 0)]);
    }

    #[test]
    fn a_rule_that_repeats_only_once_has_nothing_ahead_of_it() {
        let mut daily = rule(RecurrenceUnit::Day, 1);
        daily.max_occurrences = Some(1);

        assert!(preview(&daily, at(2026, 9, 6, 9, 0), 5).is_empty());
    }

    #[test]
    fn an_interval_of_zero_produces_nothing_rather_than_looping() {
        assert!(preview(&rule(RecurrenceUnit::Day, 0), at(2026, 9, 6, 9, 0), 5).is_empty());
    }

    #[test]
    fn a_weekend_occurrence_is_moved_to_the_previous_working_day() {
        let mut weekly = rule(RecurrenceUnit::Week, 1);
        // 2026-09-05 is a Saturday.
        weekly.adjustment = Some(weekday_adjustment(condition(
            DayOfWeek::Saturday,
            AdjustmentDirection::Previous,
            AdjustmentTarget::Weekday,
        )));

        let dates = preview(&weekly, at(2026, 9, 5, 9, 0), 2);

        assert_eq!(dates, vec![at(2026, 9, 11, 9, 0), at(2026, 9, 18, 9, 0)]);
    }

    #[test]
    fn a_fixed_shift_moves_the_occurrence_by_the_requested_days() {
        let mut weekly = rule(RecurrenceUnit::Week, 1);
        let mut shift = condition(
            DayOfWeek::Saturday,
            AdjustmentDirection::Next,
            AdjustmentTarget::Days,
        );
        shift.then_days = Some(2);
        weekly.adjustment = Some(weekday_adjustment(shift));

        let dates = preview(&weekly, at(2026, 9, 5, 9, 0), 2);

        // The anchor's own Saturday still counts: shifted forward it lands
        // after the anchor, so it is the first date ahead.
        assert_eq!(dates, vec![at(2026, 9, 7, 9, 0), at(2026, 9, 14, 9, 0)]);
    }
}
