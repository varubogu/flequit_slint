//! Ordering rules for the task list.
//!
//! Two things live here, both pure: the comparison behind each [`TaskSort`]
//! mode, and the arithmetic that turns a drop position into stored
//! `order_index` values. Keeping them out of the callback wiring is what makes
//! them testable without a window or a database.

pub mod tree;

use std::cmp::Ordering;

use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::task::TaskTree;
use flequit_model::types::id_types::TaskId;

use crate::bindings::TaskSort;
pub use tree::{move_task_to_list, normalize, reorder_task};

/// Compares two tasks under `sort`.
///
/// `TaskSort::Manual` treats every task as equal so that a *stable* sort leaves
/// the stored order untouched. The derived orderings do the same for their
/// ties, which is why the stored order remains the tiebreaker everywhere.
pub fn compare(a: &TaskTree, b: &TaskTree, sort: TaskSort) -> Ordering {
    match sort {
        TaskSort::Manual => Ordering::Equal,
        // Undated tasks sort last: they are not upcoming, and pushing them to
        // the top would bury the ones that are.
        TaskSort::Due => compare_due(a.plan_end_date, b.plan_end_date),
        // The domain priority grows with importance, so the order is reversed.
        TaskSort::Priority => b.priority.cmp(&a.priority),
        TaskSort::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
    }
}

fn compare_due(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Where a task belongs in its list after being dropped.
///
/// The drop is expressed as the visible neighbours the task ended up between,
/// not as an index, because the visible list can be filtered by a search and
/// can span several lists. `previous` wins when both are known: it is the row
/// the task was dropped below.
///
/// `full` must not contain the moved task.
pub fn insertion_index(full: &[TaskId], previous: Option<TaskId>, next: Option<TaskId>) -> usize {
    if let Some(previous) = previous
        && let Some(index) = full.iter().position(|id| *id == previous)
    {
        return index + 1;
    }
    if let Some(next) = next
        && let Some(index) = full.iter().position(|id| *id == next)
    {
        return index;
    }
    full.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use flequit_model::models::task_projects::task::TaskTree;
    use flequit_model::types::id_types::{ProjectId, TaskListId, UserId};
    use flequit_model::types::task_types::TaskStatus;

    fn task(title: &str, priority: i32, due: Option<DateTime<Utc>>) -> TaskTree {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        TaskTree {
            id: TaskId::new(),
            project_id: ProjectId::new(),
            list_id: TaskListId::new(),
            title: title.to_string(),
            description: None,
            status: TaskStatus::NotStarted,
            priority,
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
            tag_ids: Vec::new(),
        }
    }

    fn due(day: u32) -> Option<DateTime<Utc>> {
        Some(Utc.with_ymd_and_hms(2026, 4, day, 12, 0, 0).unwrap())
    }

    fn sorted(mut tasks: Vec<TaskTree>, sort: TaskSort) -> Vec<String> {
        tasks.sort_by(|a, b| compare(a, b, sort));
        tasks.into_iter().map(|task| task.title).collect()
    }

    #[test]
    fn the_manual_order_is_left_alone() {
        let tasks = vec![
            task("c", 9, due(1)),
            task("a", 0, None),
            task("b", 3, due(2)),
        ];

        assert_eq!(sorted(tasks, TaskSort::Manual), ["c", "a", "b"]);
    }

    #[test]
    fn due_dates_come_first_and_undated_tasks_last() {
        let tasks = vec![
            task("none", 0, None),
            task("late", 0, due(9)),
            task("soon", 0, due(2)),
        ];

        assert_eq!(sorted(tasks, TaskSort::Due), ["soon", "late", "none"]);
    }

    #[test]
    fn the_highest_priority_comes_first() {
        let tasks = vec![
            task("low", 1, None),
            task("high", 9, None),
            task("mid", 4, None),
        ];

        assert_eq!(sorted(tasks, TaskSort::Priority), ["high", "mid", "low"]);
    }

    #[test]
    fn titles_are_compared_without_case() {
        let tasks = vec![task("banana", 0, None), task("Apple", 0, None)];

        assert_eq!(sorted(tasks, TaskSort::Title), ["Apple", "banana"]);
    }

    #[test]
    fn equal_tasks_keep_their_stored_order() {
        let tasks = vec![task("first", 5, due(1)), task("second", 5, due(1))];

        assert_eq!(
            sorted(tasks.clone(), TaskSort::Priority),
            ["first", "second"]
        );
        assert_eq!(sorted(tasks, TaskSort::Due), ["first", "second"]);
    }

    #[test]
    fn a_drop_lands_after_the_row_above_it() {
        let ids: Vec<TaskId> = (0..3).map(|_| TaskId::new()).collect();

        assert_eq!(insertion_index(&ids, Some(ids[0]), Some(ids[1])), 1);
        assert_eq!(insertion_index(&ids, Some(ids[2]), None), 3);
    }

    #[test]
    fn a_drop_at_the_top_lands_before_the_row_below_it() {
        let ids: Vec<TaskId> = (0..3).map(|_| TaskId::new()).collect();

        assert_eq!(insertion_index(&ids, None, Some(ids[0])), 0);
    }

    #[test]
    fn a_drop_with_no_visible_neighbour_lands_at_the_end() {
        let ids: Vec<TaskId> = (0..3).map(|_| TaskId::new()).collect();
        let hidden = TaskId::new();

        assert_eq!(insertion_index(&ids, Some(hidden), Some(hidden)), 3);
        assert_eq!(insertion_index(&ids, None, None), 3);
    }
}
