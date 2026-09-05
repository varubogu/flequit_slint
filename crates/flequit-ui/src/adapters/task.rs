//! `TaskTree` / `SubTaskTree` → Slint UI types.

use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::subtask::SubTaskTree;
use flequit_model::models::task_projects::task::TaskTree;
use flequit_model::types::task_types::TaskStatus as DomainStatus;
use slint::{ModelRc, SharedString, VecModel};

use super::datetime::{DisplayTimezone, format_due, is_overdue};
use crate::bindings::{SubTaskItem, TaskItem, TaskPriority, TaskStatus};

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
pub fn to_subtask_item(sub: &SubTaskTree) -> SubTaskItem {
    SubTaskItem {
        id: SharedString::from(sub.id.as_str()),
        task_id: SharedString::from(sub.task_id.as_str()),
        title: SharedString::from(sub.title.clone()),
        status: to_status(&sub.status),
        completed: sub.completed,
    }
}

/// Converts a task and its subtasks for display.
///
/// `expanded` comes from the UI-state ViewModel rather than the domain, since
/// accordion state is not persisted (yet) and is not part of the task.
pub fn to_task_item(
    task: &TaskTree,
    expanded: bool,
    now: &DateTime<Utc>,
    timezone: DisplayTimezone,
) -> TaskItem {
    let completed = matches!(task.status, DomainStatus::Completed);
    let due = task.plan_end_date.as_ref();

    let subtasks: Vec<SubTaskItem> = task.sub_tasks.iter().map(to_subtask_item).collect();
    let done_count = subtasks.iter().filter(|s| s.completed).count();

    TaskItem {
        id: SharedString::from(task.id.as_str()),
        list_id: SharedString::from(task.list_id.as_str()),
        title: SharedString::from(task.title.clone()),
        status: to_status(&task.status),
        priority: to_priority(task.priority),
        completed,
        due_label: due
            .map(|d| SharedString::from(format_due(d, timezone)))
            .unwrap_or_default(),
        has_due: due.is_some(),
        overdue: is_overdue(due, completed, now),
        notes: SharedString::from(task.description.clone().unwrap_or_default()),
        // Tag names require a separate lookup; the ViewModel fills this in when
        // the tag cache is available.
        tag_labels: ModelRc::new(VecModel::<SharedString>::default()),
        subtask_count: subtasks.len() as i32,
        subtask_done_count: done_count as i32,
        subtasks: ModelRc::new(VecModel::from(subtasks)),
        expanded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_buckets_cover_the_whole_range() {
        assert_eq!(to_priority(-5), TaskPriority::None);
        assert_eq!(to_priority(2), TaskPriority::Low);
        assert_eq!(to_priority(4), TaskPriority::Medium);
        assert_eq!(to_priority(100), TaskPriority::High);
    }
}
