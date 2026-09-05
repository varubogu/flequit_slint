//! `TaskTree` / `SubTaskTree` → Slint UI types.

use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::subtask::SubTaskTree;
use flequit_model::models::task_projects::task::TaskTree;
use flequit_model::types::task_types::TaskStatus as DomainStatus;
use slint::{ModelRc, SharedString, VecModel};

use super::datetime::{DateTimeDisplaySettings, format_due, is_overdue, to_display_parts};
use super::recurrence::to_unit;
use crate::bindings::{
    RecurrenceUnit, ReminderItem, SubTaskItem, TaskItem, TaskPriority, TaskStatus,
};

/// Maps the domain status enum to its UI counterpart.
pub fn to_status(status: &DomainStatus) -> TaskStatus {
    match status {
        DomainStatus::NotStarted => TaskStatus::NotStarted,
        DomainStatus::InProgress => TaskStatus::InProgress,
        DomainStatus::Waiting => TaskStatus::Waiting,
        DomainStatus::Completed => TaskStatus::Completed,
        DomainStatus::Cancelled => TaskStatus::Cancelled,
    }
}

/// Maps a UI status selection back to the domain enum.
pub fn from_status(status: TaskStatus) -> DomainStatus {
    match status {
        TaskStatus::NotStarted => DomainStatus::NotStarted,
        TaskStatus::InProgress => DomainStatus::InProgress,
        TaskStatus::Waiting => DomainStatus::Waiting,
        TaskStatus::Completed => DomainStatus::Completed,
        TaskStatus::Cancelled => DomainStatus::Cancelled,
    }
}

/// Buckets the numeric domain priority into the three levels the UI renders.
///
/// The domain stores an open-ended `i32`; the UI only distinguishes "needs
/// attention", "elevated" and "normal", so the mapping is deliberately coarse.
///
/// # Examples
///
/// ```
/// use flequit_ui::adapters::task::to_priority;
/// use flequit_ui::bindings::TaskPriority;
///
/// assert_eq!(to_priority(0), TaskPriority::None);
/// assert_eq!(to_priority(1), TaskPriority::Low);
/// assert_eq!(to_priority(9), TaskPriority::High);
/// ```
pub fn to_priority(priority: i32) -> TaskPriority {
    match priority {
        i32::MIN..=0 => TaskPriority::None,
        1..=2 => TaskPriority::Low,
        3..=5 => TaskPriority::Medium,
        _ => TaskPriority::High,
    }
}

/// Converts a subtask for display.
///
/// A subtask carries the same schedule fields as its parent, so its due date is
/// resolved exactly like `to_task_item` does: pre-formatted for the label, and
/// broken into local calendar parts for the picker.
pub fn to_subtask_item(
    sub: &SubTaskTree,
    now: &DateTime<Utc>,
    display: &DateTimeDisplaySettings,
) -> SubTaskItem {
    let due = sub.plan_end_date.as_ref();
    let due_parts = to_display_parts(due.unwrap_or(now), display.timezone);

    SubTaskItem {
        id: SharedString::from(sub.id.as_str()),
        task_id: SharedString::from(sub.task_id.as_str()),
        title: SharedString::from(sub.title.clone()),
        status: to_status(&sub.status),
        priority: to_priority(sub.priority.unwrap_or(0)),
        completed: sub.completed,
        notes: SharedString::from(sub.description.clone().unwrap_or_default()),
        due_label: due
            .map(|d| SharedString::from(format_due(d, display)))
            .unwrap_or_default(),
        has_due: due.is_some(),
        overdue: is_overdue(due, sub.completed, now),
        due_year: due_parts.year,
        due_month: due_parts.month,
        due_day: due_parts.day,
        due_hour: due_parts.hour,
        due_minute: due_parts.minute,
    }
}

/// Converts a task and its subtasks for display.
///
/// `expanded` comes from the UI-state ViewModel rather than the domain, since
/// accordion state is not persisted (yet) and is not part of the task.
///
/// `tags` holds the already-resolved tag names: the task carries only tag ids,
/// and resolving them needs the project's tag list, which an adapter may not
/// load.
pub fn to_task_item(
    task: &TaskTree,
    expanded: bool,
    now: &DateTime<Utc>,
    display: &DateTimeDisplaySettings,
    tags: &[&str],
) -> TaskItem {
    let completed = matches!(task.status, DomainStatus::Completed);
    let due = task.plan_end_date.as_ref();
    let due_parts = to_display_parts(due.unwrap_or(now), display.timezone);
    let start = task.plan_start_date.as_ref();
    let start_parts = to_display_parts(start.or(due).unwrap_or(now), display.timezone);

    let subtasks: Vec<SubTaskItem> = task
        .sub_tasks
        .iter()
        .map(|sub| to_subtask_item(sub, now, display))
        .collect();
    let done_count = subtasks.iter().filter(|s| s.completed).count();
    let mut reminder_dates = task.reminders.clone();
    reminder_dates.sort_unstable();
    let reminders = reminder_dates
        .iter()
        .map(|reminder| ReminderItem {
            key: SharedString::from(reminder.to_rfc3339()),
            label: SharedString::from(format_due(reminder, display)),
        })
        .collect::<Vec<_>>();

    TaskItem {
        id: SharedString::from(task.id.as_str()),
        project_id: SharedString::from(task.project_id.as_str()),
        list_id: SharedString::from(task.list_id.as_str()),
        title: SharedString::from(task.title.clone()),
        status: to_status(&task.status),
        priority: to_priority(task.priority),
        completed,
        due_label: due
            .map(|d| SharedString::from(format_due(d, display)))
            .unwrap_or_default(),
        has_due: due.is_some(),
        overdue: is_overdue(due, completed, now),
        due_year: due_parts.year,
        due_month: due_parts.month,
        due_day: due_parts.day,
        due_hour: due_parts.hour,
        due_minute: due_parts.minute,
        // A stored start date makes the task a range even if the flag is unset:
        // another client may have written one without it.
        is_range_date: task.is_range_date.unwrap_or(false) || start.is_some(),
        start_label: start
            .map(|d| SharedString::from(format_due(d, display)))
            .unwrap_or_default(),
        has_start: start.is_some(),
        start_year: start_parts.year,
        start_month: start_parts.month,
        start_day: start_parts.day,
        start_hour: start_parts.hour,
        start_minute: start_parts.minute,
        notes: SharedString::from(task.description.clone().unwrap_or_default()),
        tag_labels: ModelRc::new(VecModel::from(
            tags.iter()
                .map(|tag| SharedString::from(*tag))
                .collect::<Vec<_>>(),
        )),
        subtask_count: subtasks.len() as i32,
        subtask_done_count: done_count as i32,
        subtasks: ModelRc::new(VecModel::from(subtasks)),
        has_recurrence: task.recurrence_rule.is_some(),
        recurrence_unit: task
            .recurrence_rule
            .as_ref()
            .map_or(RecurrenceUnit::Day, |rule| to_unit(&rule.unit)),
        recurrence_interval: task
            .recurrence_rule
            .as_ref()
            .map_or(1, |rule| rule.interval.max(1)),
        reminders: ModelRc::new(VecModel::from(reminders)),
        expanded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_round_trips_every_variant() {
        for status in [
            DomainStatus::NotStarted,
            DomainStatus::InProgress,
            DomainStatus::Waiting,
            DomainStatus::Completed,
            DomainStatus::Cancelled,
        ] {
            assert_eq!(from_status(to_status(&status)), status);
        }
    }

    #[test]
    fn priority_buckets_cover_the_whole_range() {
        assert_eq!(to_priority(-5), TaskPriority::None);
        assert_eq!(to_priority(2), TaskPriority::Low);
        assert_eq!(to_priority(4), TaskPriority::Medium);
        assert_eq!(to_priority(100), TaskPriority::High);
    }
}
