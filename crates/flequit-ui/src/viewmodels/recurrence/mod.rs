//! The writes behind the repeat-schedule editor.
//!
//! Pure: the dialog sends its working copy and these functions turn it into the
//! domain values a facade expects. The editor covers the patterns it can draw —
//! period, interval, weekdays, day of the month, and how the series ends —
//! while the business-day corrections a rule may carry are preserved untouched
//! rather than dropped, since nothing in this UI can rebuild them.

pub mod occurrence;

use chrono::Utc;
use flequit_model::models::task_projects::recurrence_details::RecurrenceDetails;
use flequit_model::models::task_projects::recurrence_rule::RecurrenceRule;
use flequit_model::types::datetime_calendar_types::{DayOfWeek, RecurrenceUnit as DomainUnit};
use flequit_model::types::id_types::{RecurrenceRuleId, UserId};

use crate::adapters::datetime::{DateTimeParts, DisplayTimezone, from_display_parts};
use crate::adapters::recurrence::{from_unit, from_week_of_month, from_weekday_index};
use crate::bindings::{RecurrenceEnd, RecurrenceMonthlyMode, RecurrenceState, RecurrenceUnit};

/// The largest interval the editor accepts, matching the spin box.
const MAX_INTERVAL: i32 = 999;

/// The rule the editor's working copy describes.
///
/// `None` when the task should not repeat, which the caller turns into a
/// deletion rather than a write. `existing` supplies the identity of a rule
/// being edited so that the association with the task survives the save.
pub fn rule_from_state(
    state: &RecurrenceState,
    existing: Option<&RecurrenceRule>,
    timezone: DisplayTimezone,
    user_id: UserId,
) -> Option<RecurrenceRule> {
    if !state.enabled {
        return None;
    }

    let now = Utc::now();
    let unit = from_unit(state.unit);
    let interval = state.interval.clamp(1, MAX_INTERVAL);

    Some(RecurrenceRule {
        id: existing.map_or_else(RecurrenceRuleId::new, |rule| rule.id),
        unit,
        interval,
        days_of_week: selected_weekdays(state),
        details: details(state, existing, user_id, now),
        // No editor draws the business-day corrections, so a rule that has them
        // keeps them: saving an interval must not quietly change when the task
        // actually lands.
        adjustment: existing.and_then(|rule| rule.adjustment.clone()),
        end_date: match state.end_kind {
            RecurrenceEnd::OnDate => from_display_parts(
                DateTimeParts {
                    year: state.end_year,
                    month: state.end_month,
                    day: state.end_day,
                    // The last day counts in full, like the due-date filters.
                    hour: 23,
                    minute: 59,
                },
                timezone,
            ),
            _ => None,
        },
        max_occurrences: match state.end_kind {
            RecurrenceEnd::AfterCount => Some(state.max_occurrences.max(1)),
            _ => None,
        },
        created_at: existing.map_or(now, |rule| rule.created_at),
        updated_at: now,
        deleted: false,
        updated_by: user_id,
    })
}

/// The weekdays a weekly rule fires on, or `None` when the period is not a week
/// or no weekday is ticked — in which case the rule follows its anchor's day.
fn selected_weekdays(state: &RecurrenceState) -> Option<Vec<DayOfWeek>> {
    if state.unit != RecurrenceUnit::Week {
        return None;
    }

    let days: Vec<DayOfWeek> = [
        state.sunday,
        state.monday,
        state.tuesday,
        state.wednesday,
        state.thursday,
        state.friday,
        state.saturday,
    ]
    .iter()
    .enumerate()
    .filter(|(_, selected)| **selected)
    .map(|(index, _)| from_weekday_index(index as i32))
    .collect();

    (!days.is_empty()).then_some(days)
}

/// The day-of-period pattern, for the periods that have one.
///
/// Any extra date conditions the rule already carried are kept: like the
/// corrections, they have no editor and would otherwise be lost on every save.
fn details(
    state: &RecurrenceState,
    existing: Option<&RecurrenceRule>,
    user_id: UserId,
    now: chrono::DateTime<Utc>,
) -> Option<RecurrenceDetails> {
    let previous = existing.and_then(|rule| rule.details.as_ref());
    let date_conditions = previous.and_then(|details| details.date_conditions.clone());

    let (specific_date, week_of_period, weekday_of_week) = match state.monthly_mode {
        _ if !is_period_of_months(&from_unit(state.unit)) => (None, None, None),
        RecurrenceMonthlyMode::DayOfMonth => (Some(state.day_of_month.clamp(1, 31)), None, None),
        RecurrenceMonthlyMode::WeekdayOfMonth => (
            None,
            Some(from_week_of_month(state.week_of_month)),
            Some(from_weekday_index(state.weekday_of_month)),
        ),
        RecurrenceMonthlyMode::SameDay => (None, None, None),
    };

    if specific_date.is_none() && week_of_period.is_none() && date_conditions.is_none() {
        return None;
    }

    Some(RecurrenceDetails {
        specific_date,
        week_of_period,
        weekday_of_week,
        date_conditions,
        created_at: previous.map_or(now, |details| details.created_at),
        updated_at: now,
        deleted: false,
        updated_by: user_id,
    })
}

/// Whether the period is long enough to pick a day inside it.
fn is_period_of_months(unit: &DomainUnit) -> bool {
    matches!(
        unit,
        DomainUnit::Month | DomainUnit::Quarter | DomainUnit::HalfYear | DomainUnit::Year
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::recurrence::to_recurrence_state;
    use chrono::TimeZone;
    use slint::SharedString;

    fn state() -> RecurrenceState {
        let anchor = Utc.with_ymd_and_hms(2026, 9, 6, 9, 0, 0).unwrap();
        RecurrenceState {
            enabled: true,
            ..to_recurrence_state("t1", None, &anchor, DisplayTimezone::Utc)
        }
    }

    #[test]
    fn a_disabled_draft_describes_no_rule() {
        let draft = RecurrenceState {
            enabled: false,
            ..state()
        };

        assert!(rule_from_state(&draft, None, DisplayTimezone::Utc, UserId::new()).is_none());
    }

    #[test]
    fn weekday_flags_only_reach_a_weekly_rule() {
        let mut draft = state();
        draft.tuesday = true;
        draft.unit = RecurrenceUnit::Day;

        let daily = rule_from_state(&draft, None, DisplayTimezone::Utc, UserId::new()).unwrap();
        assert!(daily.days_of_week.is_none());

        draft.unit = RecurrenceUnit::Week;
        let weekly = rule_from_state(&draft, None, DisplayTimezone::Utc, UserId::new()).unwrap();
        assert_eq!(weekly.days_of_week.map(|days| days.len()), Some(1));
    }

    #[test]
    fn a_day_of_the_month_is_only_recorded_for_periods_of_months() {
        let mut draft = state();
        draft.monthly_mode = RecurrenceMonthlyMode::DayOfMonth;
        draft.day_of_month = 15;

        draft.unit = RecurrenceUnit::Week;
        let weekly = rule_from_state(&draft, None, DisplayTimezone::Utc, UserId::new()).unwrap();
        assert!(weekly.details.is_none());

        draft.unit = RecurrenceUnit::Month;
        let monthly = rule_from_state(&draft, None, DisplayTimezone::Utc, UserId::new()).unwrap();
        assert_eq!(
            monthly.details.and_then(|details| details.specific_date),
            Some(15)
        );
    }

    #[test]
    fn each_way_of_ending_excludes_the_other() {
        let mut draft = state();
        draft.end_kind = RecurrenceEnd::AfterCount;
        draft.max_occurrences = 4;

        let counted = rule_from_state(&draft, None, DisplayTimezone::Utc, UserId::new()).unwrap();
        assert_eq!(counted.max_occurrences, Some(4));
        assert!(counted.end_date.is_none());

        draft.end_kind = RecurrenceEnd::OnDate;
        draft.end_year = 2026;
        draft.end_month = 12;
        draft.end_day = 24;

        let dated = rule_from_state(&draft, None, DisplayTimezone::Utc, UserId::new()).unwrap();
        assert!(dated.max_occurrences.is_none());
        assert_eq!(
            dated.end_date,
            Some(Utc.with_ymd_and_hms(2026, 12, 24, 23, 59, 0).unwrap())
        );
    }

    #[test]
    fn an_interval_below_one_is_raised_rather_than_saved() {
        let mut draft = state();
        draft.interval = 0;

        let rule = rule_from_state(&draft, None, DisplayTimezone::Utc, UserId::new()).unwrap();

        assert_eq!(rule.interval, 1);
    }

    #[test]
    fn editing_a_rule_keeps_its_identity_and_its_corrections() {
        let user_id = UserId::new();
        let created = rule_from_state(&state(), None, DisplayTimezone::Utc, user_id).unwrap();

        let mut draft = state();
        draft.task_id = SharedString::from("t1");
        draft.interval = 5;
        let updated =
            rule_from_state(&draft, Some(&created), DisplayTimezone::Utc, user_id).unwrap();

        assert_eq!(updated.id, created.id);
        assert_eq!(updated.created_at, created.created_at);
        assert_eq!(updated.interval, 5);
    }
}
