//! `TaskTree` → Slint UI types.

use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::task::TaskTree;
use flequit_model::types::task_types::TaskStatus as DomainStatus;
use slint::{ModelRc, SharedString, VecModel};

use super::datetime::{DateTimeDisplaySettings, format_due, is_overdue, to_display_parts};
use super::recurrence::to_unit;
use crate::bindings::{
    AncestorItem, RecurrenceUnit, ReminderItem, SubTaskItem, TaskItem, TaskPriority, TaskStatus,
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

/// The subtasks a task shows: deleted ones are gone from every view.
pub fn live_children(task: &TaskTree) -> impl Iterator<Item = &TaskTree> {
    task.sub_tasks.iter().filter(|child| !child.deleted)
}

/// Converts a direct child for the list of subtasks in its parent's pane.
pub fn to_subtask_item(
    child: &TaskTree,
    now: &DateTime<Utc>,
    display: &DateTimeDisplaySettings,
) -> SubTaskItem {
    let completed = matches!(child.status, DomainStatus::Completed);
    let due = child.plan_end_date.as_ref();
    SubTaskItem {
        id: SharedString::from(child.id.as_str()),
        title: SharedString::from(child.title.clone()),
        completed,
        due_label: due
            .map(|d| SharedString::from(format_due(d, display)))
            .unwrap_or_default(),
        has_due: due.is_some(),
        overdue: is_overdue(due, completed, now),
        subtask_count: live_children(child).count() as i32,
    }
}

/// Where a task sits in the tree.
#[derive(Debug, Clone, Copy, Default)]
pub struct Placement<'a> {
    /// The list of the task's top-level task, `None` directly under the project.
    pub list_id: Option<&'a str>,
    /// The tasks above it, top-level first. Empty for a top-level task.
    pub ancestors: &'a [&'a TaskTree],
}

/// Converts a task at any depth for display.
///
/// `expanded` comes from the UI-state ViewModel rather than the domain, since
/// accordion state is not persisted (yet) and is not part of the task.
///
/// `tags` holds the already-resolved tag names: the task carries only tag ids,
/// and resolving them needs the project's tag list, which an adapter may not
/// load.
pub fn to_task_item(
    task: &TaskTree,
    placement: Placement<'_>,
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

    let subtasks: Vec<SubTaskItem> = live_children(task)
        .map(|child| to_subtask_item(child, now, display))
        .collect();
    let done_count = subtasks.iter().filter(|s| s.completed).count();
    let ancestors: Vec<AncestorItem> = placement
        .ancestors
        .iter()
        .map(|ancestor| AncestorItem {
            id: SharedString::from(ancestor.id.as_str()),
            title: SharedString::from(ancestor.title.clone()),
        })
        .collect();
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
        list_id: SharedString::from(placement.list_id.unwrap_or_default()),
        parent_id: placement
            .ancestors
            .last()
            .map(|parent| SharedString::from(parent.id.as_str()))
            .unwrap_or_default(),
        depth: placement.ancestors.len() as i32,
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
        ancestors: ModelRc::new(VecModel::from(ancestors)),
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
        search_dimmed: false,
        matched_subtask_count: 0,
        search_match: false,
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
