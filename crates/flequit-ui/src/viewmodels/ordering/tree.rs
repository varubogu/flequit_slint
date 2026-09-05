//! Reordering inside the cached project tree.
//!
//! The tree is the ViewModel's copy of what storage holds, so a drag has to
//! change it and storage together. These functions apply the move to the tree
//! and hand back the patches that make storage agree; they perform no I/O.

use flequit_model::models::task_projects::project::ProjectTree;
use flequit_model::models::task_projects::task::{PartialTask, TaskTree};
use flequit_model::types::id_types::{ProjectId, TaskId, TaskListId};

use super::insertion_index;

/// The writes needed to persist a move, and the project they belong to.
pub type OrderUpdates = (ProjectId, Vec<(TaskId, PartialTask)>);

/// Puts every list in stored order.
///
/// Storage returns tasks in no particular order, and the manual ordering is
/// exactly `order_index`, so the tree is normalised once on load rather than
/// sorted again on every refresh.
pub fn normalize(trees: &mut [ProjectTree]) {
    for tree in trees {
        for list in &mut tree.task_lists {
            list.tasks.sort_by_key(|task| task.order_index);
        }
    }
}

/// Moves a task between the two tasks it was dropped between.
///
/// The neighbours are visible rows, which may be a filtered subset of the list;
/// see [`insertion_index`] for how the position is resolved. Returns `None` when
/// the task is not in the tree.
pub fn reorder_task(
    trees: &mut [ProjectTree],
    task_id: &str,
    previous: Option<TaskId>,
    next: Option<TaskId>,
) -> Option<OrderUpdates> {
    let (project_id, list) = list_of_task(trees, task_id)?;

    let from = list.iter().position(|task| task.id.as_str() == task_id)?;
    let task = list.remove(from);
    let ids: Vec<TaskId> = list.iter().map(|task| task.id).collect();
    let at = insertion_index(&ids, previous, next);
    list.insert(at, task);

    Some((project_id, renumber(list)))
}

/// Moves a task to the end of another list in the same project.
///
/// Returns `None` when either end of the move cannot be resolved, including a
/// target in a different project: tasks are stored per project, so that is a
/// different operation rather than a longer move.
pub fn move_task_to_list(
    trees: &mut [ProjectTree],
    task_id: &str,
    target_list_id: &str,
) -> Option<OrderUpdates> {
    let tree = trees.iter_mut().find(|tree| {
        tree.task_lists
            .iter()
            .any(|list| list.tasks.iter().any(|task| task.id.as_str() == task_id))
    })?;
    let project_id = tree.id;

    let target_id = TaskListId::try_from_str(target_list_id).ok()?;
    if !tree.task_lists.iter().any(|list| list.id == target_id) {
        return None;
    }

    let source = tree
        .task_lists
        .iter_mut()
        .find(|list| list.tasks.iter().any(|task| task.id.as_str() == task_id))?;
    if source.id == target_id {
        return None;
    }
    let from = source
        .tasks
        .iter()
        .position(|task| task.id.as_str() == task_id)?;
    let mut task = source.tasks.remove(from);
    // The source keeps the gap the task leaves behind: the remaining order is
    // still correct, and renumbering it would double the writes.
    task.list_id = target_id;
    let moved_id = task.id;

    let target = tree
        .task_lists
        .iter_mut()
        .find(|list| list.id == target_id)?;
    target.tasks.push(task);

    let mut updates = renumber(&mut target.tasks);
    // The list itself changed, and only for the task that moved.
    match updates.iter_mut().find(|(id, _)| *id == moved_id) {
        Some((_, patch)) => patch.list_id = Some(target_id),
        None => updates.push((
            moved_id,
            PartialTask {
                list_id: Some(target_id),
                ..Default::default()
            },
        )),
    }

    Some((project_id, updates))
}

/// The tasks of the list holding `task_id`, with the project that owns it.
fn list_of_task<'a>(
    trees: &'a mut [ProjectTree],
    task_id: &str,
) -> Option<(ProjectId, &'a mut Vec<TaskTree>)> {
    trees.iter_mut().find_map(|tree| {
        let project_id = tree.id;
        tree.task_lists
            .iter_mut()
            .find(|list| list.tasks.iter().any(|task| task.id.as_str() == task_id))
            .map(|list| (project_id, &mut list.tasks))
    })
}

/// Renumbers `order_index` from zero, returning a patch per task that moved.
fn renumber(tasks: &mut [TaskTree]) -> Vec<(TaskId, PartialTask)> {
    tasks
        .iter_mut()
        .enumerate()
        .filter_map(|(index, task)| {
            let order = index as i32;
            if task.order_index == order {
                return None;
            }
            task.order_index = order;
            Some((
                task.id,
                PartialTask {
                    order_index: Some(order),
                    ..Default::default()
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};
    use flequit_model::models::task_projects::task_list::TaskListTree;
    use flequit_model::types::id_types::UserId;
    use flequit_model::types::task_types::TaskStatus;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn task(project_id: ProjectId, list_id: TaskListId, title: &str, order: i32) -> TaskTree {
        TaskTree {
            id: TaskId::new(),
            project_id,
            list_id,
            title: title.to_string(),
            description: None,
            status: TaskStatus::NotStarted,
            priority: 0,
            plan_start_date: None,
            plan_end_date: None,
            do_start_date: None,
            do_end_date: None,
            is_range_date: None,
            recurrence_rule: None,
            reminders: Vec::new(),
            assigned_user_ids: Vec::new(),
            order_index: order,
            is_archived: false,
            created_at: now(),
            updated_at: now(),
            deleted: false,
            updated_by: UserId::new(),
            sub_tasks: Vec::new(),
            tag_ids: Vec::new(),
        }
    }

    fn list(project_id: ProjectId, name: &str, tasks: Vec<TaskTree>) -> TaskListTree {
        TaskListTree {
            id: tasks.first().map(|task| task.list_id).unwrap_or_default(),
            project_id,
            name: name.to_string(),
            description: None,
            color: None,
            order_index: 0,
            is_archived: false,
            created_at: now(),
            updated_at: now(),
            deleted: false,
            updated_by: UserId::new(),
            tasks,
        }
    }

    fn project(lists: Vec<TaskListTree>) -> ProjectTree {
        ProjectTree {
            id: lists
                .first()
                .map(|list| list.project_id)
                .unwrap_or_default(),
            name: "Home".to_string(),
            description: None,
            color: None,
            order_index: 0,
            is_archived: false,
            status: None,
            owner_id: None,
            created_at: now(),
            updated_at: now(),
            deleted: false,
            updated_by: UserId::new(),
            task_lists: lists,
        }
    }

    /// One project, one list of three tasks named a, b, c.
    fn one_list() -> Vec<ProjectTree> {
        let project_id = ProjectId::new();
        let list_id = TaskListId::new();
        let tasks = vec![
            task(project_id, list_id, "a", 0),
            task(project_id, list_id, "b", 1),
            task(project_id, list_id, "c", 2),
        ];
        vec![project(vec![list(project_id, "Inbox", tasks)])]
    }

    fn titles(trees: &[ProjectTree], list_index: usize) -> Vec<&str> {
        trees[0].task_lists[list_index]
            .tasks
            .iter()
            .map(|task| task.title.as_str())
            .collect()
    }

    fn id_of(trees: &[ProjectTree], list_index: usize, title: &str) -> TaskId {
        trees[0].task_lists[list_index]
            .tasks
            .iter()
            .find(|task| task.title == title)
            .expect("the task exists")
            .id
    }

    #[test]
    fn normalizing_puts_a_list_back_into_stored_order() {
        let mut trees = one_list();
        trees[0].task_lists[0].tasks.reverse();

        normalize(&mut trees);

        assert_eq!(titles(&trees, 0), ["a", "b", "c"]);
    }

    #[test]
    fn a_task_dropped_below_another_lands_after_it() {
        let mut trees = one_list();
        let a = id_of(&trees, 0, "a");
        let c_id = id_of(&trees, 0, "c").as_str();

        let (_, updates) = reorder_task(&mut trees, &c_id, Some(a), None).expect("the task moved");

        assert_eq!(titles(&trees, 0), ["a", "c", "b"]);
        // Only the two tasks that changed places are written back.
        assert_eq!(updates.len(), 2);
        assert!(updates.iter().all(|(_, patch)| patch.order_index.is_some()));
    }

    #[test]
    fn a_task_dropped_at_the_top_lands_first() {
        let mut trees = one_list();
        let a = id_of(&trees, 0, "a");
        let c_id = id_of(&trees, 0, "c").as_str();

        reorder_task(&mut trees, &c_id, None, Some(a)).expect("the task moved");

        assert_eq!(titles(&trees, 0), ["c", "a", "b"]);
        let orders: Vec<i32> = trees[0].task_lists[0]
            .tasks
            .iter()
            .map(|task| task.order_index)
            .collect();
        assert_eq!(orders, [0, 1, 2]);
    }

    #[test]
    fn reordering_an_unknown_task_changes_nothing() {
        let mut trees = one_list();

        assert!(reorder_task(&mut trees, &TaskId::new().as_str(), None, None).is_none());
        assert_eq!(titles(&trees, 0), ["a", "b", "c"]);
    }

    #[test]
    fn moving_to_another_list_appends_and_records_the_list() {
        let project_id = ProjectId::new();
        let source_id = TaskListId::new();
        let target_id = TaskListId::new();
        let mut trees = vec![project(vec![
            list(
                project_id,
                "Inbox",
                vec![
                    task(project_id, source_id, "a", 0),
                    task(project_id, source_id, "b", 1),
                ],
            ),
            list(
                project_id,
                "Later",
                vec![task(project_id, target_id, "z", 0)],
            ),
        ])];
        let moved = id_of(&trees, 0, "a").as_str();

        let (_, updates) =
            move_task_to_list(&mut trees, &moved, &target_id.as_str()).expect("the task moved");

        assert_eq!(titles(&trees, 0), ["b"]);
        assert_eq!(titles(&trees, 1), ["z", "a"]);
        let (_, patch) = updates
            .iter()
            .find(|(_, patch)| patch.list_id.is_some())
            .expect("the move is recorded");
        assert_eq!(patch.list_id, Some(target_id));
        assert_eq!(patch.order_index, Some(1));
    }

    #[test]
    fn moving_to_the_list_it_is_already_in_changes_nothing() {
        let mut trees = one_list();
        let list_id = trees[0].task_lists[0].id;
        let moved = id_of(&trees, 0, "a").as_str();

        assert!(move_task_to_list(&mut trees, &moved, &list_id.as_str()).is_none());
        assert_eq!(titles(&trees, 0), ["a", "b", "c"]);
    }

    #[test]
    fn a_list_in_another_project_is_not_a_valid_target() {
        let mut trees = one_list();
        let moved = id_of(&trees, 0, "a").as_str();

        assert!(move_task_to_list(&mut trees, &moved, &TaskListId::new().as_str()).is_none());
        assert_eq!(titles(&trees, 0), ["a", "b", "c"]);
    }
}
