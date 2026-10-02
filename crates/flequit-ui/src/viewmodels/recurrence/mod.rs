//! The writes behind the repeat-schedule editor.
//!
//! Pure: the dialog sends its working copy and these functions turn it into the
//! domain values a facade expects. The editor covers the patterns it can draw —
//! period, interval, weekdays, day of the month, and how the series ends —
//! while the business-day corrections a rule may carry are preserved untouched
//! rather than dropped, since nothing in this UI can rebuild them.

pub mod occurrence;

use chrono::{DateTime, TimeDelta, Utc};
use flequit_model::models::task_projects::recurrence_details::RecurrenceDetails;
use flequit_model::models::task_projects::recurrence_rule::RecurrenceRule;
use flequit_model::models::task_projects::task::{Task, TaskTree};
use flequit_model::types::datetime_calendar_types::{DayOfWeek, RecurrenceUnit as DomainUnit};
use flequit_model::types::id_types::{RecurrenceRuleId, TaskId, UserId};
use flequit_model::types::task_types::TaskStatus;

use crate::adapters::datetime::{DateTimeParts, DisplayTimezone, from_display_parts};
use crate::adapters::recurrence::{from_unit, from_week_of_month, from_weekday_index};
use crate::bindings::{RecurrenceEnd, RecurrenceMonthlyMode, RecurrenceState, RecurrenceUnit};

/// The largest interval the editor accepts, matching the spin box.
const MAX_INTERVAL: i32 = 999;

/// The preview count uses the same bounds as the editor's spin box.
const MAX_PREVIEW_COUNT: i32 = 999;

/// Chooses how many dates the occurrence expander may return.
///
/// Endless rules use the explicit preview count. A finite rule has its own
/// natural bound, so it is allowed to return every date through that bound.
pub fn preview_limit(rule: &RecurrenceRule, requested: i32) -> usize {
    if rule.end_date.is_some() || rule.max_occurrences.is_some() {
        usize::MAX
    } else {
        requested.clamp(1, MAX_PREVIEW_COUNT) as usize
    }
}

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

/// The task and subtasks that take over a repeating task once it is completed.
#[derive(Debug, Clone)]
pub struct NextOccurrence {
    /// The successor task, carrying the handed-over rule.
    pub task: Task,
    /// Fresh copies of the completed task's subtasks at every depth, parents
    /// before their children, so they can be created in this order.
    pub sub_tasks: Vec<Task>,
    /// The rule as it stands once it has moved to the successor.
    pub rule: RecurrenceRule,
}

/// The occurrence that takes over a repeating task once it is completed.
///
/// The next task is due on the rule's next date after the completed one's due
/// date (or after `now` when it had none), and keeps its title, notes,
/// priority, tags and assignees. A span or reminders move by the same distance
/// as the due date, and so do the subtasks: the gap the user arranged between
/// a subtask and its task is what decides when the next subtask is due. `None`
/// when the series has nothing left ahead of it.
///
/// The rule is handed over rather than copied: it returns with one fewer
/// occurrence left, so a limited series still ends where it was set to end.
/// Only the task carries it — a subtask never repeats on its own.
pub fn next_task(
    task: &TaskTree,
    now: DateTime<Utc>,
    timezone: DisplayTimezone,
    order_index: i32,
    user_id: UserId,
) -> Option<NextOccurrence> {
    let rule = task.recurrence_rule.as_ref()?;
    let anchor = task.plan_end_date.unwrap_or(now);
    let next_due = *occurrence::next_occurrences(rule, &anchor, timezone, 1).first()?;
    let offset = next_due - anchor;
    let shift = |date: DateTime<Utc>| date + offset;

    let mut handed_over = rule.clone();
    handed_over.max_occurrences = rule.max_occurrences.map(|max| (max - 1).max(1));
    handed_over.updated_at = now;
    handed_over.updated_by = user_id;

    let next_id = TaskId::new();
    let next = Task {
        id: next_id,
        project_id: task.project_id,
        list_id: task.list_id,
        parent_task_id: task.parent_task_id,
        title: task.title.clone(),
        description: task.description.clone(),
        status: TaskStatus::NotStarted,
        priority: task.priority,
        plan_start_date: task.plan_start_date.map(shift),
        plan_end_date: Some(next_due),
        do_start_date: None,
        do_end_date: None,
        is_range_date: task.is_range_date,
        recurrence_rule: Some(handed_over.clone()),
        reminders: if task.plan_end_date.is_some() {
            task.reminders.iter().copied().map(shift).collect()
        } else {
            Vec::new()
        },
        order_index,
        is_archived: false,
        assigned_user_ids: task.assigned_user_ids.clone(),
        tag_ids: task.tag_ids.clone(),
        created_at: now,
        updated_at: now,
        deleted: false,
        updated_by: user_id,
    };
    let mut sub_tasks = Vec::new();
    let copy = SubTaskCopy {
        offset,
        // Without a due date the offset counts from now, which says nothing
        // about when a reminder should come round again.
        keep_reminders: task.plan_end_date.is_some(),
        now,
        user_id,
    };
    copy_sub_tasks(task, next_id, &copy, &mut sub_tasks);
    Some(NextOccurrence {
        task: next,
        sub_tasks,
        rule: handed_over,
    })
}

/// Fresh copies of `task`'s subtasks at every depth, for its successor.
///
/// Each copy starts unstarted, and its planned dates move by the same distance
/// as the task's due date: a subtask due two days before its task is again due
/// two days before it in the next occurrence. Subtasks already deleted are
/// left behind with everything below them, and no copy carries a rule — the
/// series belongs to the task. A copy belongs to the copy of its parent.
fn copy_sub_tasks(task: &TaskTree, parent_id: TaskId, copy: &SubTaskCopy, copies: &mut Vec<Task>) {
    let SubTaskCopy {
        offset,
        keep_reminders,
        now,
        user_id,
    } = *copy;
    for sub_task in task.sub_tasks.iter().filter(|sub_task| !sub_task.deleted) {
        let copy_id = TaskId::new();
        copies.push(Task {
            id: copy_id,
            project_id: sub_task.project_id,
            list_id: None,
            parent_task_id: Some(parent_id),
            title: sub_task.title.clone(),
            description: sub_task.description.clone(),
            status: TaskStatus::NotStarted,
            priority: sub_task.priority,
            plan_start_date: sub_task.plan_start_date.map(|date| date + offset),
            plan_end_date: sub_task.plan_end_date.map(|date| date + offset),
            do_start_date: None,
            do_end_date: None,
            is_range_date: sub_task.is_range_date,
            recurrence_rule: None,
            reminders: if keep_reminders {
                sub_task
                    .reminders
                    .iter()
                    .map(|date| *date + offset)
                    .collect()
            } else {
                Vec::new()
            },
            order_index: sub_task.order_index,
            is_archived: false,
            assigned_user_ids: sub_task.assigned_user_ids.clone(),
            tag_ids: sub_task.tag_ids.clone(),
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: user_id,
        });
        copy_sub_tasks(sub_task, copy_id, copy, copies);
    }
}

/// How [`copy_sub_tasks`] shifts and stamps each copy.
#[derive(Clone, Copy)]
struct SubTaskCopy {
    offset: TimeDelta,
    keep_reminders: bool,
    now: DateTime<Utc>,
    user_id: UserId,
}

/// Whether the occurrence `next_task` created is still exactly as it was made.
///
/// Undoing a completion takes the successor back only in this case: once the
/// user has edited it, completed it, changed its subtasks or changed its
/// schedule, it is their task and stays.
pub fn is_untouched(created: &NextOccurrence, current: &TaskTree) -> bool {
    let task = &created.task;
    current.id == task.id
        && !current.deleted
        && !current.is_archived
        && current.list_id == task.list_id
        && current.parent_task_id == task.parent_task_id
        && current.reminders == task.reminders
        && fields_are_untouched(task, current)
        && sub_tasks_are_untouched(&created.sub_tasks, current)
        && schedule_of(current.recurrence_rule.as_ref())
            == schedule_of(task.recurrence_rule.as_ref())
}

/// Whether the successor still has exactly the subtasks it was created with.
///
/// Subtasks added, removed, moved or edited since make the successor the
/// user's, so the set has to match one for one; a subtask deleted in the
/// meantime counts as a change rather than as absent.
fn sub_tasks_are_untouched(created: &[Task], current: &TaskTree) -> bool {
    let mut live = Vec::new();
    collect_live(current, &mut live);

    live.len() == created.len()
        && created.iter().all(|created| {
            live.iter()
                .find(|current| current.id == created.id)
                .is_some_and(|current| {
                    current.parent_task_id == created.parent_task_id
                        && current.order_index == created.order_index
                        && current.reminders == created.reminders
                        && fields_are_untouched(created, current)
                })
        })
}

/// The live subtasks of `task` at every depth.
fn collect_live<'a>(task: &'a TaskTree, live: &mut Vec<&'a TaskTree>) {
    for sub_task in task.sub_tasks.iter().filter(|sub_task| !sub_task.deleted) {
        live.push(sub_task);
        collect_live(sub_task, live);
    }
}

/// Whether what the user can edit on a task is still as it was created.
fn fields_are_untouched(created: &Task, current: &TaskTree) -> bool {
    let mut created_tags = created.tag_ids.clone();
    let mut current_tags = current.tag_ids.clone();
    created_tags.sort();
    current_tags.sort();

    current.title == created.title
        && current.description == created.description
        && current.status == created.status
        && current.priority == created.priority
        && current.plan_start_date == created.plan_start_date
        && current.plan_end_date == created.plan_end_date
        && current.do_start_date == created.do_start_date
        && current.do_end_date == created.do_end_date
        && current.is_range_date == created.is_range_date
        && current_tags == created_tags
}

/// What a rule does, without when it was last written.
///
/// The domain types carry no `PartialEq`, so the schedule is compared through
/// its debug form; the bookkeeping fields are blanked first because saving
/// the rule rewrites them without changing what it does.
fn schedule_of(rule: Option<&RecurrenceRule>) -> Option<String> {
    rule.map(|rule| {
        let epoch = DateTime::<Utc>::UNIX_EPOCH;
        let normalized = RecurrenceRule {
            created_at: epoch,
            updated_at: epoch,
            updated_by: UserId::from(uuid::Uuid::nil()),
            ..rule.clone()
        };
        format!("{normalized:?}")
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
    use flequit_model::types::id_types::{ProjectId, TagId, TaskListId};
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
    fn only_an_endless_rule_uses_the_requested_preview_count() {
        let user_id = UserId::new();
        let endless = rule_from_state(&state(), None, DisplayTimezone::Utc, user_id).unwrap();
        assert_eq!(preview_limit(&endless, 7), 7);

        let mut counted_state = state();
        counted_state.end_kind = RecurrenceEnd::AfterCount;
        counted_state.max_occurrences = 12;
        let counted = rule_from_state(&counted_state, None, DisplayTimezone::Utc, user_id).unwrap();
        assert_eq!(preview_limit(&counted, 7), usize::MAX);

        let mut dated_state = state();
        dated_state.end_kind = RecurrenceEnd::OnDate;
        dated_state.end_year = 2026;
        dated_state.end_month = 12;
        dated_state.end_day = 31;
        let dated = rule_from_state(&dated_state, None, DisplayTimezone::Utc, user_id).unwrap();
        assert_eq!(preview_limit(&dated, 7), usize::MAX);
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

    fn repeating_task(unit: DomainUnit, due: Option<DateTime<Utc>>) -> TaskTree {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        TaskTree {
            id: TaskId::new(),
            project_id: ProjectId::new(),
            list_id: Some(TaskListId::new()),
            parent_task_id: None,
            title: "Water the plants".to_string(),
            description: Some("both balconies".to_string()),
            status: TaskStatus::Completed,
            priority: 2,
            plan_start_date: None,
            plan_end_date: due,
            do_start_date: None,
            do_end_date: None,
            is_range_date: None,
            recurrence_rule: Some(RecurrenceRule {
                id: RecurrenceRuleId::new(),
                unit,
                interval: 1,
                days_of_week: None,
                details: None,
                adjustment: None,
                end_date: None,
                max_occurrences: None,
                created_at: now,
                updated_at: now,
                deleted: false,
                updated_by: UserId::new(),
            }),
            reminders: Vec::new(),
            assigned_user_ids: Vec::new(),
            order_index: 0,
            is_archived: false,
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
            sub_tasks: Vec::new(),
            tag_ids: vec![TagId::new()],
        }
    }

    #[test]
    fn completing_a_repeating_task_schedules_the_next_one() {
        let due = Utc.with_ymd_and_hms(2026, 9, 19, 9, 0, 0).unwrap();
        let mut task = repeating_task(DomainUnit::Day, Some(due));
        task.plan_start_date = Some(due - chrono::TimeDelta::hours(2));
        task.reminders = vec![due - chrono::TimeDelta::minutes(30)];
        let user = UserId::new();

        let created = next_task(&task, due, DisplayTimezone::Utc, 3, user).unwrap();
        let (next, rule) = (created.task, created.rule);

        let next_due = Utc.with_ymd_and_hms(2026, 9, 20, 9, 0, 0).unwrap();
        assert_ne!(next.id, task.id);
        assert_eq!(next.status, TaskStatus::NotStarted);
        assert_eq!(next.title, task.title);
        assert_eq!(next.description, task.description);
        assert_eq!(next.priority, task.priority);
        assert_eq!(next.list_id, task.list_id);
        assert_eq!(next.tag_ids, task.tag_ids);
        assert_eq!(next.order_index, 3);
        assert_eq!(next.plan_end_date, Some(next_due));
        assert_eq!(
            next.plan_start_date,
            Some(next_due - chrono::TimeDelta::hours(2))
        );
        assert_eq!(
            next.reminders,
            vec![next_due - chrono::TimeDelta::minutes(30)]
        );
        assert_eq!(rule.id, task.recurrence_rule.as_ref().unwrap().id);
        assert_eq!(next.recurrence_rule.map(|r| r.id), Some(rule.id));
    }

    #[test]
    fn a_task_without_a_due_date_repeats_from_now() {
        let task = repeating_task(DomainUnit::Week, None);
        let now = Utc.with_ymd_and_hms(2026, 9, 19, 8, 0, 0).unwrap();

        let created = next_task(&task, now, DisplayTimezone::Utc, 0, UserId::new()).unwrap();

        assert_eq!(
            created.task.plan_end_date,
            Some(Utc.with_ymd_and_hms(2026, 9, 26, 8, 0, 0).unwrap())
        );
    }

    #[test]
    fn a_limited_series_counts_down_and_then_stops() {
        let due = Utc.with_ymd_and_hms(2026, 9, 19, 9, 0, 0).unwrap();
        let mut task = repeating_task(DomainUnit::Day, Some(due));
        task.recurrence_rule.as_mut().unwrap().max_occurrences = Some(2);

        let created = next_task(&task, due, DisplayTimezone::Utc, 0, UserId::new()).unwrap();
        assert_eq!(created.rule.max_occurrences, Some(1));

        task.recurrence_rule = Some(created.rule);
        assert!(next_task(&task, due, DisplayTimezone::Utc, 0, UserId::new()).is_none());
    }

    #[test]
    fn a_task_without_a_rule_has_no_successor() {
        let mut task = repeating_task(DomainUnit::Day, None);
        task.recurrence_rule = None;

        assert!(next_task(&task, Utc::now(), DisplayTimezone::Utc, 0, UserId::new()).is_none());
    }

    #[test]
    fn an_untouched_successor_is_recognised() {
        let due = Utc.with_ymd_and_hms(2026, 9, 19, 9, 0, 0).unwrap();
        let mut task = repeating_task(DomainUnit::Day, Some(due));
        task.sub_tasks = vec![sub_task_of(&task, "Fill the can", Some(due))];
        let created = next_task(&task, due, DisplayTimezone::Utc, 0, UserId::new()).unwrap();

        let mut stored = as_tree(&created);
        // Saving rewrites the rule's bookkeeping without changing the schedule.
        stored.recurrence_rule = Some(RecurrenceRule {
            updated_at: due + chrono::TimeDelta::seconds(1),
            ..created.rule.clone()
        });
        assert!(is_untouched(&created, &stored));
    }

    #[test]
    fn any_edit_makes_the_successor_the_users() {
        let due = Utc.with_ymd_and_hms(2026, 9, 19, 9, 0, 0).unwrap();
        let mut task = repeating_task(DomainUnit::Day, Some(due));
        let mut child = sub_task_of(&task, "Fill the can", Some(due));
        child.sub_tasks = vec![sub_task_of(&child, "Find the can", None)];
        task.sub_tasks = vec![child];
        let created = next_task(&task, due, DisplayTimezone::Utc, 0, UserId::new()).unwrap();

        let edits: Vec<fn(&mut TaskTree)> = vec![
            |t| t.title.push('!'),
            |t| t.status = TaskStatus::Completed,
            |t| t.priority += 1,
            |t| t.plan_end_date = None,
            |t| t.tag_ids.clear(),
            |t| t.recurrence_rule.as_mut().unwrap().interval = 2,
            |t| t.recurrence_rule = None,
            |t| t.deleted = true,
            |t| t.sub_tasks[0].title.push('!'),
            |t| t.sub_tasks[0].plan_end_date = None,
            |t| t.sub_tasks[0].status = TaskStatus::Completed,
            |t| t.sub_tasks[0].deleted = true,
            |t| t.sub_tasks.clear(),
            |t| {
                let extra = t.sub_tasks[0].clone();
                t.sub_tasks.push(TaskTree {
                    id: TaskId::new(),
                    ..extra
                });
            },
            |t| t.sub_tasks[0].sub_tasks[0].title.push('!'),
            |t| t.sub_tasks[0].sub_tasks[0].deleted = true,
            |t| {
                let mut grandchild = t.sub_tasks[0].sub_tasks.remove(0);
                grandchild.parent_task_id = Some(t.id);
                t.sub_tasks.push(grandchild);
            },
        ];
        for edit in edits {
            let mut stored = as_tree(&created);
            edit(&mut stored);
            assert!(!is_untouched(&created, &stored));
        }
    }

    #[test]
    fn a_subtask_keeps_its_distance_from_the_task_it_belongs_to() {
        // A weekly task due on the 5th with a subtask due on the 3rd repeats
        // as a task due on the 12th with its subtask due on the 10th.
        let due = Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap();
        let sub_due = Utc.with_ymd_and_hms(2026, 1, 3, 9, 0, 0).unwrap();
        let mut task = repeating_task(DomainUnit::Week, Some(due));
        task.sub_tasks = vec![sub_task_of(&task, "Buy the soil", Some(sub_due))];

        let created = next_task(&task, due, DisplayTimezone::Utc, 0, UserId::new()).unwrap();

        assert_eq!(
            created.task.plan_end_date,
            Some(Utc.with_ymd_and_hms(2026, 1, 12, 9, 0, 0).unwrap())
        );
        assert_eq!(created.sub_tasks.len(), 1);
        assert_eq!(
            created.sub_tasks[0].plan_end_date,
            Some(Utc.with_ymd_and_hms(2026, 1, 10, 9, 0, 0).unwrap())
        );
    }

    #[test]
    fn a_copied_subtask_starts_over_and_never_repeats_on_its_own() {
        let due = Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap();
        let mut task = repeating_task(DomainUnit::Week, Some(due));
        let mut source = sub_task_of(&task, "Buy the soil", Some(due));
        source.plan_start_date = Some(due - chrono::TimeDelta::hours(3));
        source.do_start_date = Some(due - chrono::TimeDelta::hours(2));
        source.do_end_date = Some(due);
        source.status = TaskStatus::Completed;
        source.order_index = 4;
        task.sub_tasks = vec![source.clone()];

        let created = next_task(&task, due, DisplayTimezone::Utc, 0, UserId::new()).unwrap();
        let copy = &created.sub_tasks[0];

        assert_ne!(copy.id, source.id);
        assert_eq!(copy.parent_task_id, Some(created.task.id));
        assert_eq!(copy.list_id, None);
        assert_eq!(copy.title, source.title);
        assert_eq!(copy.status, TaskStatus::NotStarted);
        assert_eq!(copy.order_index, source.order_index);
        assert_eq!(copy.tag_ids, source.tag_ids);
        assert!(copy.recurrence_rule.is_none());
        assert!(copy.do_start_date.is_none());
        assert!(copy.do_end_date.is_none());
        assert_eq!(
            copy.plan_start_date,
            Some(Utc.with_ymd_and_hms(2026, 1, 12, 6, 0, 0).unwrap())
        );
    }

    #[test]
    fn a_subtask_without_a_due_date_stays_without_one() {
        let due = Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap();
        let mut task = repeating_task(DomainUnit::Week, Some(due));
        task.sub_tasks = vec![sub_task_of(&task, "Ask the neighbour", None)];

        let created = next_task(&task, due, DisplayTimezone::Utc, 0, UserId::new()).unwrap();

        assert!(created.sub_tasks[0].plan_end_date.is_none());
    }

    #[test]
    fn a_deleted_subtask_is_left_behind() {
        let due = Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap();
        let mut task = repeating_task(DomainUnit::Week, Some(due));
        let mut gone = sub_task_of(&task, "No longer needed", Some(due));
        gone.deleted = true;
        task.sub_tasks = vec![gone, sub_task_of(&task, "Buy the soil", Some(due))];

        let created = next_task(&task, due, DisplayTimezone::Utc, 0, UserId::new()).unwrap();

        assert_eq!(created.sub_tasks.len(), 1);
        assert_eq!(created.sub_tasks[0].title, "Buy the soil");
    }

    #[test]
    fn subtasks_are_copied_at_every_depth_under_their_own_copies() {
        let due = Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap();
        let mut task = repeating_task(DomainUnit::Week, Some(due));
        let mut child = sub_task_of(&task, "Prepare", None);
        let mut grandchild = sub_task_of(&child, "Buy the soil", Some(due));
        let mut gone = sub_task_of(&grandchild, "Never mind", None);
        gone.deleted = true;
        let under_gone = sub_task_of(&gone, "Lost with it", None);
        gone.sub_tasks = vec![under_gone];
        grandchild.sub_tasks = vec![gone];
        child.sub_tasks = vec![grandchild];
        task.sub_tasks = vec![child];

        let created = next_task(&task, due, DisplayTimezone::Utc, 0, UserId::new()).unwrap();

        let titles: Vec<&str> = created.sub_tasks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Prepare", "Buy the soil"]);
        assert_eq!(created.sub_tasks[0].parent_task_id, Some(created.task.id));
        assert_eq!(
            created.sub_tasks[1].parent_task_id,
            Some(created.sub_tasks[0].id)
        );
        assert!(is_untouched(&created, &as_tree(&created)));
    }

    fn sub_task_of(parent: &TaskTree, title: &str, due: Option<DateTime<Utc>>) -> TaskTree {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        TaskTree {
            id: TaskId::new(),
            project_id: parent.project_id,
            list_id: None,
            parent_task_id: Some(parent.id),
            title: title.to_string(),
            description: Some("from the shed".to_string()),
            status: TaskStatus::NotStarted,
            priority: 1,
            plan_start_date: None,
            plan_end_date: due,
            do_start_date: None,
            do_end_date: None,
            is_range_date: None,
            recurrence_rule: None,
            reminders: Vec::new(),
            assigned_user_ids: Vec::new(),
            order_index: 0,
            is_archived: false,
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
            sub_tasks: Vec::new(),
            tag_ids: vec![TagId::new()],
        }
    }

    /// The successor as storage would load it: the copies nested under their
    /// parents.
    fn as_tree(created: &NextOccurrence) -> TaskTree {
        let mut root = tree_of(&created.task);
        for copy in &created.sub_tasks {
            let parent = copy.parent_task_id.expect("a copy has a parent");
            root.find_mut(&parent)
                .expect("parents come first")
                .sub_tasks
                .push(tree_of(copy));
        }
        root
    }

    fn tree_of(task: &Task) -> TaskTree {
        TaskTree {
            id: task.id,
            project_id: task.project_id,
            list_id: task.list_id,
            parent_task_id: task.parent_task_id,
            title: task.title.clone(),
            description: task.description.clone(),
            status: task.status.clone(),
            priority: task.priority,
            plan_start_date: task.plan_start_date,
            plan_end_date: task.plan_end_date,
            do_start_date: task.do_start_date,
            do_end_date: task.do_end_date,
            is_range_date: task.is_range_date,
            recurrence_rule: task.recurrence_rule.clone(),
            reminders: task.reminders.clone(),
            assigned_user_ids: task.assigned_user_ids.clone(),
            order_index: task.order_index,
            is_archived: task.is_archived,
            created_at: task.created_at,
            updated_at: task.updated_at,
            deleted: task.deleted,
            updated_by: task.updated_by,
            sub_tasks: Vec::new(),
            tag_ids: task.tag_ids.clone(),
        }
    }
}
