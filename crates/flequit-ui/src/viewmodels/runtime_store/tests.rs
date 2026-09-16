use chrono::Utc;
use flequit_model::models::task_projects::project::ProjectTree;
use flequit_model::models::task_projects::task::{PartialTask, TaskTree};
use flequit_model::models::task_projects::task_list::TaskListTree;
use flequit_model::types::id_types::{ProjectId, TaskId, TaskListId, UserId};
use flequit_model::types::task_types::TaskStatus;

use super::{RuntimeStore, find_task};

#[test]
fn failed_mutation_does_not_erase_a_later_committed_mutation() {
    let (mut store, task_id) = store_with_one_task();
    let failed = store
        .begin_task_mutation(
            &task_id,
            PartialTask {
                status: Some(TaskStatus::Completed),
                ..PartialTask::default()
            },
        )
        .expect("task exists");
    let committed = store
        .begin_task_mutation(
            &task_id,
            PartialTask {
                title: Some("new title".to_string()),
                ..PartialTask::default()
            },
        )
        .expect("task exists");

    assert!(store.resolve_task_mutation(&task_id, committed, true));
    assert!(store.resolve_task_mutation(&task_id, failed, false));

    let task = find_task(&store, &task_id).expect("task remains visible");
    assert_eq!(task.status, TaskStatus::NotStarted);
    assert_eq!(task.title, "new title");
}

#[test]
fn later_change_to_the_same_field_wins_after_an_older_failure() {
    let (mut store, task_id) = store_with_one_task();
    let failed = store
        .begin_task_mutation(
            &task_id,
            PartialTask {
                title: Some("discarded".to_string()),
                ..PartialTask::default()
            },
        )
        .expect("task exists");
    let committed = store
        .begin_task_mutation(
            &task_id,
            PartialTask {
                title: Some("kept".to_string()),
                ..PartialTask::default()
            },
        )
        .expect("task exists");

    assert!(store.resolve_task_mutation(&task_id, committed, true));
    assert!(store.resolve_task_mutation(&task_id, failed, false));

    assert_eq!(
        find_task(&store, &task_id).map(|task| task.title.as_str()),
        Some("kept")
    );
}

#[test]
fn a_later_failure_is_hidden_while_an_older_mutation_is_still_pending() {
    let (mut store, task_id) = store_with_one_task();
    let pending = store
        .begin_task_mutation(
            &task_id,
            PartialTask {
                status: Some(TaskStatus::Completed),
                ..PartialTask::default()
            },
        )
        .expect("task exists");
    let failed = store
        .begin_task_mutation(
            &task_id,
            PartialTask {
                title: Some("discarded".to_string()),
                ..PartialTask::default()
            },
        )
        .expect("task exists");

    assert!(store.resolve_task_mutation(&task_id, failed, false));
    let task = find_task(&store, &task_id).expect("task remains visible");
    assert_eq!(task.status, TaskStatus::Completed);
    assert_eq!(task.title, "original");

    assert!(store.resolve_task_mutation(&task_id, pending, true));
}

#[test]
fn reload_reapplies_pending_mutations_over_persisted_data() {
    let (mut store, task_id) = store_with_one_task();
    let reloaded = store.trees.clone();
    store
        .begin_task_mutation(
            &task_id,
            PartialTask {
                status: Some(TaskStatus::Completed),
                ..PartialTask::default()
            },
        )
        .expect("task exists");

    store.replace_trees(reloaded);

    assert_eq!(
        find_task(&store, &task_id).map(|task| &task.status),
        Some(&TaskStatus::Completed)
    );
}

fn store_with_one_task() -> (RuntimeStore, TaskId) {
    let now = Utc::now();
    let user_id = UserId::new();
    let project_id = ProjectId::new();
    let list_id = TaskListId::new();
    let task_id = TaskId::new();
    let task = TaskTree {
        id: task_id,
        project_id,
        list_id,
        previous_task_id: None,
        title: "original".to_string(),
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
        order_index: 0,
        is_archived: false,
        created_at: now,
        updated_at: now,
        deleted: false,
        updated_by: user_id,
        sub_tasks: Vec::new(),
        tag_ids: Vec::new(),
    };
    let list = TaskListTree {
        id: list_id,
        project_id,
        name: "Inbox".to_string(),
        description: None,
        color: None,
        order_index: 0,
        is_archived: false,
        created_at: now,
        updated_at: now,
        deleted: false,
        updated_by: user_id,
        tasks: vec![task],
    };
    let project = ProjectTree {
        id: project_id,
        name: "Project".to_string(),
        description: None,
        color: None,
        order_index: 0,
        is_archived: false,
        status: None,
        owner_id: Some(user_id),
        created_at: now,
        updated_at: now,
        deleted: false,
        updated_by: user_id,
        task_lists: vec![list],
    };
    (RuntimeStore::from(vec![project]), task_id)
}
