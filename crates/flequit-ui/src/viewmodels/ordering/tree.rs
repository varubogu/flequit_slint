//! Reordering inside the cached project tree.
//!
//! The tree is the ViewModel's copy of what storage holds, so a drag has to
//! change it and storage together. These functions apply the move to the tree
//! and hand back the patches that make storage agree; they perform no I/O.
//!
//! A top-level task lives in one of its project's lists or directly under the
//! project; each of those keeps its own order. Subtasks move with their
//! top-level task and are not moved on their own here.

use flequit_model::models::task_projects::project::ProjectTree;
use flequit_model::models::task_projects::task::{PartialTask, TaskTree};
use flequit_model::types::id_types::{ProjectId, TaskId, TaskListId};

use super::insertion_index;

/// The writes needed to persist a move, and the project they belong to.
pub type OrderUpdates = (ProjectId, Vec<(TaskId, PartialTask)>);

/// Puts every list, the project's own tasks and every task's subtasks in
/// stored order.
///
/// Storage returns tasks in no particular order, and the manual ordering is
/// exactly `order_index`, so the tree is normalised once on load rather than
/// sorted again on every refresh.
pub fn normalize(trees: &mut [ProjectTree]) {
    fn sort(tasks: &mut [TaskTree]) {
        tasks.sort_by_key(|task| task.order_index);
        for task in tasks {
            sort(&mut task.sub_tasks);
        }
    }
    for tree in trees {
        for list in &mut tree.task_lists {
            sort(&mut list.tasks);
        }
        sort(&mut tree.tasks);
    }
}

/// Moves a top-level task between the two tasks it was dropped between.
///
/// The neighbours are visible top-level rows, which may be a filtered subset
/// of where the task lives; see [`insertion_index`] for how the position is
/// resolved. Returns `None` when the task is not a top-level task in the tree.
pub fn reorder_task(
    trees: &mut [ProjectTree],
    task_id: &str,
    previous: Option<TaskId>,
    next: Option<TaskId>,
) -> Option<OrderUpdates> {
    let (project_id, siblings) = home_of(trees, task_id)?;

    let from = siblings
        .iter()
        .position(|task| task.id.as_str() == task_id)?;
    let task = siblings.remove(from);
    let ids: Vec<TaskId> = siblings.iter().map(|task| task.id).collect();
    let at = insertion_index(&ids, previous, next);
    siblings.insert(at, task);

    Some((project_id, renumber(siblings)))
}

/// Moves a top-level task to the end of a list in the same project.
///
/// Returns `None` when either end of the move cannot be resolved, including a
/// target in a different project: tasks are stored per project, so that is a
/// different operation rather than a longer move.
pub fn move_task_to_list(
    trees: &mut [ProjectTree],
    task_id: &str,
    target_list_id: &str,
) -> Option<OrderUpdates> {
    let target_id = TaskListId::try_from_str(target_list_id).ok()?;
    move_task(trees, task_id, Some(target_id))
}

/// Takes a top-level task out of its list, to the end of its project's own
/// tasks. Returns `None` when it is not in a list.
pub fn move_task_to_project(trees: &mut [ProjectTree], task_id: &str) -> Option<OrderUpdates> {
    move_task(trees, task_id, None)
}

/// Moves a top-level task to the end of `target` (a list, or the project's own
/// tasks for `None`) in its own project.
fn move_task(
    trees: &mut [ProjectTree],
    task_id: &str,
    target: Option<TaskListId>,
) -> Option<OrderUpdates> {
    let tree = trees
        .iter_mut()
        .find(|tree| tree.root_tasks().any(|task| task.id.as_str() == task_id))?;
    let project_id = tree.id;
    if let Some(target_id) = target
        && !tree.task_lists.iter().any(|list| list.id == target_id)
    {
        return None;
    }

    let source = home_in(tree, task_id)?;
    let from = source.iter().position(|task| task.id.as_str() == task_id)?;
    if source[from].list_id == target {
        return None;
    }
    // The source keeps the gap the task leaves behind: the remaining order is
    // still correct, and renumbering it would double the writes.
    let mut task = source.remove(from);
    task.list_id = target;
    let moved_id = task.id;

    let destination = match target {
        Some(target_id) => {
            &mut tree
                .task_lists
                .iter_mut()
                .find(|list| list.id == target_id)?
                .tasks
        }
        None => &mut tree.tasks,
    };
    destination.push(task);

    let mut updates = renumber(destination);
    // Where it lives changed, and only for the task that moved.
    match updates.iter_mut().find(|(id, _)| *id == moved_id) {
        Some((_, patch)) => patch.list_id = Some(target),
        None => updates.push((
            moved_id,
            PartialTask {
                list_id: Some(target),
                ..Default::default()
            },
        )),
    }

    Some((project_id, updates))
}

/// The top-level tasks living alongside `task_id`, with their project.
fn home_of<'a>(
    trees: &'a mut [ProjectTree],
    task_id: &str,
) -> Option<(ProjectId, &'a mut Vec<TaskTree>)> {
    trees.iter_mut().find_map(|tree| {
        let project_id = tree.id;
        home_in(tree, task_id).map(|siblings| (project_id, siblings))
    })
}

/// The list (or the project's own tasks) holding the top-level task `task_id`.
fn home_in<'a>(tree: &'a mut ProjectTree, task_id: &str) -> Option<&'a mut Vec<TaskTree>> {
    let holds = |tasks: &Vec<TaskTree>| tasks.iter().any(|task| task.id.as_str() == task_id);
    if holds(&tree.tasks) {
        return Some(&mut tree.tasks);
    }
    tree.task_lists
        .iter_mut()
        .find(|list| holds(&list.tasks))
        .map(|list| &mut list.tasks)
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
            list_id: Some(list_id),
            parent_task_id: None,
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
            id: tasks
                .first()
                .and_then(|task| task.list_id)
                .unwrap_or_default(),
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
            tasks: Vec::new(),
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
        assert_eq!(patch.list_id, Some(Some(target_id)));
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

    #[test]
    fn a_task_taken_out_of_its_list_goes_to_the_end_of_the_project() {
        let mut trees = one_list();
        let project_id = trees[0].id;
        let mut loose = task(project_id, TaskListId::new(), "loose", 0);
        loose.list_id = None;
        trees[0].tasks.push(loose);
        let moved = id_of(&trees, 0, "b").as_str();

        let (_, updates) = move_task_to_project(&mut trees, &moved).expect("the task moved");

        assert_eq!(titles(&trees, 0), ["a", "c"]);
        let own: Vec<&str> = trees[0]
            .tasks
            .iter()
            .map(|task| task.title.as_str())
            .collect();
        assert_eq!(own, ["loose", "b"]);
        assert_eq!(trees[0].tasks[1].list_id, None);
        let (_, patch) = updates
            .iter()
            .find(|(_, patch)| patch.list_id.is_some())
            .expect("the move is recorded");
        assert_eq!(patch.list_id, Some(None));
        // It already had the order it takes there, so only the place is written.
        assert_eq!(trees[0].tasks[1].order_index, 1);
        assert_eq!(patch.order_index, None);
    }

    #[test]
    fn a_task_of_the_project_itself_can_go_into_a_list() {
        let mut trees = one_list();
        let project_id = trees[0].id;
        let list_id = trees[0].task_lists[0].id;
        let mut loose = task(project_id, list_id, "loose", 0);
        loose.list_id = None;
        let loose_id = loose.id.as_str();
        trees[0].tasks.push(loose);

        move_task_to_list(&mut trees, &loose_id, &list_id.as_str()).expect("the task moved");

        assert!(trees[0].tasks.is_empty());
        assert_eq!(titles(&trees, 0), ["a", "b", "c", "loose"]);
        let a = id_of(&trees, 0, "a").as_str();
        assert!(move_task_to_project(&mut trees, &a).is_some());
    }

    #[test]
    fn tasks_of_the_project_itself_reorder_among_themselves() {
        let mut trees = one_list();
        let project_id = trees[0].id;
        for (order, title) in ["x", "y"].into_iter().enumerate() {
            let mut loose = task(project_id, TaskListId::new(), title, order as i32);
            loose.list_id = None;
            trees[0].tasks.push(loose);
        }
        let y = trees[0].tasks[1].id.as_str();
        let x = trees[0].tasks[0].id;

        reorder_task(&mut trees, &y, None, Some(x)).expect("the task moved");

        let own: Vec<&str> = trees[0]
            .tasks
            .iter()
            .map(|task| task.title.as_str())
            .collect();
        assert_eq!(own, ["y", "x"]);
    }

    #[test]
    fn a_subtask_is_not_moved_on_its_own() {
        let mut trees = one_list();
        let parent = &mut trees[0].task_lists[0].tasks[0];
        let mut child = parent.clone();
        child.id = TaskId::new();
        child.list_id = None;
        child.parent_task_id = Some(parent.id);
        let child_id = child.id.as_str();
        parent.sub_tasks.push(child);
        let list_id = trees[0].task_lists[0].id;

        assert!(reorder_task(&mut trees, &child_id, None, None).is_none());
        assert!(move_task_to_project(&mut trees, &child_id).is_none());
        assert!(move_task_to_list(&mut trees, &child_id, &list_id.as_str()).is_none());
    }
}
