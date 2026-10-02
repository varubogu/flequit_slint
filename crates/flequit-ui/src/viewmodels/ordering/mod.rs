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

use crate::bindings::{TaskItem, TaskSort};
pub use tree::{move_task_to_list, move_task_to_project, normalize, reorder_task};

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

/// The top-level position a drop at `target_index` of the visible rows means.
///
/// Rows are the flat list the view shows, subtasks included; a drop on a
/// subtask's row counts as a drop on its top-level task. `None` when there are
/// no top-level rows.
pub fn root_position(rows: &[TaskItem], target_index: i32) -> Option<usize> {
    let roots = rows.iter().filter(|row| row.depth == 0).count();
    if roots == 0 {
        return None;
    }
    let target = usize::try_from(target_index)
        .unwrap_or(0)
        .min(rows.len().saturating_sub(1));
    let before = rows[..=target].iter().filter(|row| row.depth == 0).count();
    Some(before.saturating_sub(1).min(roots - 1))
}

/// The visible top-level tasks that would surround `task_id` once it moves
/// from its position among them to the one `to` picks.
///
/// Only tasks living in the same place (the same list, or the same project
/// directly) can anchor the move: the visible list may span several. `None`
/// when the task is not a visible top-level row or would not move.
pub fn neighbours_after_move(
    rows: &[TaskItem],
    task_id: &str,
    to: impl FnOnce(usize) -> Option<usize>,
) -> Option<(Option<TaskId>, Option<TaskId>)> {
    let mut roots: Vec<&TaskItem> = rows.iter().filter(|row| row.depth == 0).collect();
    let from = roots.iter().position(|row| row.id == task_id)?;
    let to = to(from)?.min(roots.len() - 1);
    if to == from {
        return None;
    }
    let moved = roots.remove(from);
    roots.insert(to, moved);

    let same_home =
        |row: &&&TaskItem| row.project_id == moved.project_id && row.list_id == moved.list_id;
    let id = |row: &&TaskItem| TaskId::try_from_str(row.id.as_str()).ok();
    let previous = roots[..to].iter().rev().find(same_home).and_then(id);
    let next = roots[to + 1..].iter().find(same_home).and_then(id);
    Some((previous, next))
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
            list_id: Some(TaskListId::new()),
            parent_task_id: None,
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

    fn row(id: &TaskId, list: &str, depth: i32) -> TaskItem {
        TaskItem {
            id: id.as_str().into(),
            project_id: "p".into(),
            list_id: list.into(),
            depth,
            ..TaskItem::default()
        }
    }

    #[test]
    fn a_drop_on_a_subtask_row_counts_as_a_drop_on_its_task() {
        let (a, b) = (TaskId::new(), TaskId::new());
        let rows = [row(&a, "l", 0), row(&TaskId::new(), "", 1), row(&b, "l", 0)];

        assert_eq!(root_position(&rows, 0), Some(0));
        assert_eq!(root_position(&rows, 1), Some(0));
        assert_eq!(root_position(&rows, 2), Some(1));
        assert_eq!(root_position(&rows, 9), Some(1));
        assert_eq!(root_position(&[], 0), None);
    }

    #[test]
    fn a_task_moves_among_its_own_place_and_skips_its_subtasks() {
        let (a, b, c, other) = (TaskId::new(), TaskId::new(), TaskId::new(), TaskId::new());
        let rows = [
            row(&a, "l", 0),
            row(&TaskId::new(), "", 1),
            row(&other, "", 0),
            row(&b, "l", 0),
            row(&c, "l", 0),
        ];

        // a goes after b: the task of the project itself is not an anchor.
        assert_eq!(
            neighbours_after_move(&rows, &a.as_str(), |_| Some(2)),
            Some((Some(b), Some(c)))
        );
        // One step down from the top passes only the other place's task.
        assert_eq!(
            neighbours_after_move(&rows, &a.as_str(), |from| from.checked_add(1)),
            Some((None, Some(b)))
        );
        assert_eq!(
            neighbours_after_move(&rows, &a.as_str(), |from| from.checked_sub(1)),
            None
        );
        assert_eq!(
            neighbours_after_move(&rows, &TaskId::new().as_str(), |_| Some(0)),
            None
        );
    }
}
