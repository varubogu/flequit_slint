//! Automerge 同期キューの統合テスト
//!
//! 実際の SQLite と Automerge を `.tmp/tests` 配下に作り、書き込みが
//! 「SQLite + キュー」で確定し、Automerge へはキュー経由でだけ届くことを確かめる。

use std::error::Error;
use std::time::Duration;

use chrono::{DateTime, Utc};
use flequit_core::ports::infrastructure_repositories::{
    SyncDiagnosticsPort, SyncQueueSummary, SyncRequeueReport, TagBookmarkRepositoryPort,
    TransactionalDeletionPort, TransactionalRestorePort,
};
use flequit_infrastructure::automerge_sync::{AutomergeChange, ProjectChange, TrashChange};
use flequit_infrastructure::{InfrastructureRepositories, UnifiedConfig};
use flequit_infrastructure_sqlite::infrastructure::automerge_sync_queue::NewSyncQueueEntry;
use flequit_infrastructure_sqlite::models::automerge_sync_queue::SyncQueueStatus;
use flequit_model::models::task_projects::{project::Project, task::Task, task_list::TaskList};
use flequit_model::models::user_preferences::tag_bookmark::TagBookmark;
use flequit_model::types::id_types::{ProjectId, TagBookmarkId, TagId, TaskId, TaskListId, UserId};
use flequit_model::types::project_types::ProjectStatus;
use flequit_model::types::task_types::TaskStatus;
use flequit_repository::project_repository_trait::ProjectRepository;
use flequit_repository::repositories::base_repository_trait::Repository;
use flequit_testing::TestPathGenerator;
use flequit_types::errors::repository_error::RepositoryError;

type TestResult = Result<(), Box<dyn Error>>;

async fn setup(case: &str) -> Result<InfrastructureRepositories, Box<dyn Error>> {
    let dir = TestPathGenerator::generate_test_dir(file!(), case);
    std::fs::create_dir_all(&dir)?;
    setup_in(&dir).await
}

/// `dir` のストレージを開く。同じ `dir` で開き直すと前回の内容が読める
async fn setup_in(dir: &std::path::Path) -> Result<InfrastructureRepositories, Box<dyn Error>> {
    let automerge_dir = TestPathGenerator::create_automerge_dir(dir)?;
    let config = UnifiedConfig::new(true, true, true)
        .with_storage_paths(dir.join("flequit.sqlite"), automerge_dir);
    InfrastructureRepositories::setup_with_sqlite_and_automerge(config).await
}

async fn count(repositories: &InfrastructureRepositories, status: SyncQueueStatus) -> u64 {
    repositories
        .automerge_sync()
        .expect("SQLite and Automerge are both enabled")
        .queue()
        .repository()
        .count_by_status(status)
        .await
        .expect("queue is readable")
}

struct Fixture {
    project: Project,
    task_list: TaskList,
    task: Task,
    user_id: UserId,
    timestamp: DateTime<Utc>,
}

fn fixture() -> Fixture {
    let user_id = UserId::new();
    let timestamp = Utc::now();
    let project_id = ProjectId::new();
    let task_list_id = TaskListId::new();
    Fixture {
        project: Project {
            id: project_id,
            name: "同期キュー".to_string(),
            description: None,
            color: None,
            order_index: 0,
            is_archived: false,
            status: Some(ProjectStatus::Active),
            owner_id: Some(user_id),
            created_at: timestamp,
            updated_at: timestamp,
            deleted: false,
            updated_by: user_id,
        },
        task_list: TaskList {
            id: task_list_id,
            project_id,
            name: "リスト".to_string(),
            description: None,
            color: None,
            order_index: 0,
            is_archived: false,
            created_at: timestamp,
            updated_at: timestamp,
            deleted: false,
            updated_by: user_id,
        },
        task: Task {
            id: TaskId::new(),
            project_id,
            list_id: Some(task_list_id),
            parent_task_id: None,
            title: "タスク".to_string(),
            reminders: vec![],
            description: None,
            status: TaskStatus::NotStarted,
            priority: 0,
            plan_start_date: None,
            plan_end_date: None,
            do_start_date: None,
            do_end_date: None,
            is_range_date: None,
            recurrence_rule: None,
            assigned_user_ids: vec![],
            tag_ids: vec![],
            order_index: 0,
            is_archived: false,
            created_at: timestamp,
            updated_at: timestamp,
            deleted: false,
            updated_by: user_id,
        },
        user_id,
        timestamp,
    }
}

async fn save_fixture(repositories: &InfrastructureRepositories, f: &Fixture) -> TestResult {
    let project_id = f.project.id;
    repositories
        .projects
        .save(&f.project, &f.user_id, &f.timestamp)
        .await?;
    repositories
        .task_lists
        .save(&project_id, &f.task_list, &f.user_id, &f.timestamp)
        .await?;
    repositories
        .tasks
        .save(&project_id, &f.task, &f.user_id, &f.timestamp)
        .await?;
    Ok(())
}

fn task_save(project_id: ProjectId) -> AutomergeChange {
    let mut task = fixture().task;
    task.project_id = project_id;
    AutomergeChange::Task(Box::new(ProjectChange::Save {
        project_id,
        entity: task,
        user_id: UserId::new(),
        timestamp: Utc::now(),
    }))
}

/// Automerge にプロジェクトが無いので、再試行しても当面は失敗し続ける変更
fn delete_in_missing_project(project_id: ProjectId) -> AutomergeChange {
    AutomergeChange::Trash(TrashChange::DeleteTask {
        project_id,
        task_id: TaskId::new(),
        user_id: UserId::new(),
        timestamp: Utc::now(),
    })
}

async fn enqueue(
    repositories: &InfrastructureRepositories,
    changes: Vec<AutomergeChange>,
) -> TestResult {
    let queue = repositories.automerge_sync().unwrap().queue();
    let txn = queue.begin(changes).await?;
    txn.finish(Ok(())).await?;
    Ok(())
}

#[tokio::test]
async fn a_write_commits_to_sqlite_and_reaches_automerge_only_through_the_queue() -> TestResult {
    let repositories = setup("a_write_commits_to_sqlite").await?;
    let f = fixture();
    let project_id = f.project.id;

    save_fixture(&repositories, &f).await?;

    // SQLite には確定している
    assert!(
        repositories
            .tasks
            .find_by_id(&project_id, &f.task.id)
            .await?
            .is_some()
    );
    // Automerge にはまだ無く、キューに 3 件（プロジェクト・リスト・タスク）
    let processor = repositories.automerge_sync().unwrap();
    assert!(
        processor
            .targets()
            .projects()
            .get_tasks(&project_id)
            .await?
            .is_empty()
    );
    assert_eq!(count(&repositories, SyncQueueStatus::Pending).await, 3);

    let report = processor.process_pending().await?;

    assert_eq!(report.processed, 3);
    assert_eq!(count(&repositories, SyncQueueStatus::Pending).await, 0);
    assert_eq!(count(&repositories, SyncQueueStatus::Processed).await, 3);
    let tasks = processor
        .targets()
        .projects()
        .get_tasks(&project_id)
        .await?;
    assert_eq!(
        tasks.iter().map(|task| task.id).collect::<Vec<_>>(),
        [f.task.id]
    );
    Ok(())
}

#[tokio::test]
async fn a_failed_write_rolls_back_its_queue_entry() -> TestResult {
    let repositories = setup("a_failed_write_rolls_back").await?;
    let now = Utc::now();
    let missing = TagBookmark {
        id: TagBookmarkId::new(),
        user_id: UserId::new(),
        project_id: ProjectId::new(),
        tag_id: TagId::new(),
        order_index: 0,
        created_at: now,
        updated_at: now,
    };

    // キューの行を入れた後で SQLite の更新が失敗する
    let result = repositories.tag_bookmarks.update(&missing).await;

    assert!(matches!(result, Err(RepositoryError::NotFound(_))));
    assert_eq!(count(&repositories, SyncQueueStatus::Pending).await, 0);
    Ok(())
}

#[tokio::test]
async fn a_failing_change_holds_back_its_own_document_only() -> TestResult {
    let repositories = setup("a_failing_change_holds_back").await?;
    let stuck = ProjectId::new();
    let other = ProjectId::new();
    enqueue(
        &repositories,
        vec![
            delete_in_missing_project(stuck),
            task_save(stuck),
            task_save(other),
        ],
    )
    .await?;
    let processor = repositories.automerge_sync().unwrap();

    let report = processor.process_pending().await?;

    // 別ドキュメントの変更は進み、失敗したドキュメントの後続は追い越さない
    assert_eq!(report.processed, 1);
    assert_eq!(report.retried, 1);
    assert!(
        report
            .pending_documents
            .contains(&format!("project:{stuck}"))
    );
    let pending = processor
        .queue()
        .repository()
        .find_pending_after(0, 10)
        .await?;
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].attempts, 1);
    assert!(pending[0].next_attempt_at.is_some_and(|at| at > Utc::now()));
    assert_eq!(pending[1].attempts, 0);

    // バックオフ中は再試行しない
    let report = processor.process_pending().await?;
    assert_eq!((report.processed, report.retried), (0, 0));

    // 読み取りバリアは、反映できていない変更があればエラーにする
    assert!(
        processor
            .flush_document(&format!("project:{stuck}"))
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn an_undecodable_row_is_parked_as_failed_without_blocking_its_document() -> TestResult {
    let repositories = setup("an_undecodable_row_is_parked").await?;
    let project_id = ProjectId::new();
    let queue = repositories.automerge_sync().unwrap().queue();
    let txn = queue.begin(Vec::new()).await?;
    let broken = NewSyncQueueEntry {
        document_key: format!("project:{project_id}"),
        change_kind: "task.save".to_string(),
        payload: "{not json".to_string(),
    };
    let stored = queue
        .repository()
        .enqueue_with_txn(txn.txn(), &[broken], Utc::now())
        .await;
    txn.finish(stored).await?;
    enqueue(&repositories, vec![task_save(project_id)]).await?;

    let report = repositories
        .automerge_sync()
        .unwrap()
        .process_pending()
        .await?;

    assert_eq!((report.failed, report.processed), (1, 1));
    assert_eq!(count(&repositories, SyncQueueStatus::Failed).await, 1);
    Ok(())
}

#[tokio::test]
async fn processed_rows_are_deleted_after_thirty_days() -> TestResult {
    let repositories = setup("processed_rows_are_deleted").await?;
    let project_id = ProjectId::new();
    enqueue(
        &repositories,
        vec![
            task_save(project_id),
            task_save(project_id),
            task_save(project_id),
        ],
    )
    .await?;
    let processor = repositories.automerge_sync().unwrap();
    let ids: Vec<i64> = processor
        .queue()
        .repository()
        .find_pending_after(0, 10)
        .await?
        .iter()
        .map(|entry| entry.id)
        .collect();
    processor.process_pending().await?;
    // 失敗し続ける行（pending のまま残る）
    enqueue(
        &repositories,
        vec![delete_in_missing_project(ProjectId::new())],
    )
    .await?;

    let now = Utc::now();
    let repository = processor.queue().repository();
    repository
        .mark_processed(ids[0], now - chrono::Duration::days(31))
        .await?;
    repository
        .mark_processed(ids[1], now - chrono::Duration::days(29))
        .await?;

    let deleted = processor.cleanup_processed(now).await?;

    assert_eq!(deleted, 1);
    assert!(repository.find_by_id(ids[0]).await?.is_none());
    assert!(repository.find_by_id(ids[1]).await?.is_some());
    assert_eq!(count(&repositories, SyncQueueStatus::Processed).await, 2);
    assert_eq!(count(&repositories, SyncQueueStatus::Pending).await, 1);
    Ok(())
}

#[tokio::test]
async fn the_worker_drains_the_queue_in_the_background() -> TestResult {
    let repositories = setup("the_worker_drains_the_queue").await?;
    let worker = repositories
        .start_automerge_sync(&tokio::runtime::Handle::current())
        .expect("SQLite and Automerge are both enabled");
    let f = fixture();

    save_fixture(&repositories, &f).await?;

    let drained = tokio::time::timeout(Duration::from_secs(10), async {
        while count(&repositories, SyncQueueStatus::Pending).await > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        drained.is_ok(),
        "the worker should apply the queued changes"
    );
    assert_eq!(count(&repositories, SyncQueueStatus::Processed).await, 3);
    assert!(worker.shutdown(Duration::from_secs(5)).await);
    Ok(())
}

#[tokio::test]
async fn restoring_a_task_first_applies_the_queued_deletion() -> TestResult {
    let repositories = setup("restoring_a_task").await?;
    let f = fixture();
    let project_id = f.project.id;
    save_fixture(&repositories, &f).await?;
    repositories
        .delete_task_transactionally(&project_id, &f.task.id, &f.user_id, &f.timestamp)
        .await?;
    assert!(
        repositories
            .tasks
            .find_by_id(&project_id, &f.task.id)
            .await?
            .is_none()
    );

    // 削除はまだ Automerge に届いていないが、復元が先に反映してから読む
    repositories
        .restore_task_transactionally(&project_id, &f.task.id, &f.user_id, &Utc::now())
        .await?;

    let restored = repositories
        .tasks
        .find_by_id(&project_id, &f.task.id)
        .await?;
    assert!(restored.is_some_and(|task| !task.deleted));
    let processor = repositories.automerge_sync().unwrap();
    processor.process_pending().await?;
    let active = processor
        .targets()
        .projects()
        .get_active_tasks(&project_id)
        .await?;
    assert_eq!(
        active.iter().map(|task| task.id).collect::<Vec<_>>(),
        [f.task.id]
    );
    assert_eq!(count(&repositories, SyncQueueStatus::Pending).await, 0);
    Ok(())
}

/// 未処理の行の id を古い順に返す
async fn pending_ids(
    repositories: &InfrastructureRepositories,
) -> Result<Vec<i64>, Box<dyn Error>> {
    Ok(repositories
        .automerge_sync()
        .unwrap()
        .queue()
        .repository()
        .find_pending_after(0, 100)
        .await?
        .iter()
        .map(|entry| entry.id)
        .collect())
}

#[tokio::test]
async fn cleanup_keeps_the_newest_processed_row_of_each_document() -> TestResult {
    let repositories = setup("cleanup_keeps_the_newest").await?;
    let first = ProjectId::new();
    let second = ProjectId::new();
    enqueue(
        &repositories,
        vec![task_save(first), task_save(first), task_save(second)],
    )
    .await?;
    let ids = pending_ids(&repositories).await?;
    let processor = repositories.automerge_sync().unwrap();
    processor.process_pending().await?;
    let now = Utc::now();
    let repository = processor.queue().repository();
    for &id in &ids {
        repository
            .mark_processed(id, now - chrono::Duration::days(31))
            .await?;
    }

    let deleted = processor.cleanup_processed(now).await?;

    // 各ドキュメントの最新の行は、failed の行を戻してよいかの判断に使うため残す
    assert_eq!(deleted, 1);
    assert!(repository.find_by_id(ids[0]).await?.is_none());
    assert!(repository.find_by_id(ids[1]).await?.is_some());
    assert!(repository.find_by_id(ids[2]).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn a_failed_change_is_requeued_only_when_nothing_newer_was_applied() -> TestResult {
    let repositories = setup("a_failed_change_is_requeued").await?;
    let alone = ProjectId::new();
    let overtaken = ProjectId::new();
    enqueue(
        &repositories,
        vec![task_save(alone), task_save(overtaken), task_save(overtaken)],
    )
    .await?;
    let ids = pending_ids(&repositories).await?;
    let processor = repositories.automerge_sync().unwrap();
    let repository = processor.queue().repository();
    repository.mark_failed(ids[0], "disk full").await?;
    repository.mark_failed(ids[1], "disk full").await?;
    processor.process_pending().await?;

    assert_eq!(
        repositories.sync_queue_summary().await?,
        Some(SyncQueueSummary {
            pending: 0,
            failed: 2
        })
    );
    let failed = repositories.failed_sync_changes(10).await?;
    assert_eq!(
        failed.iter().map(|change| change.id).collect::<Vec<_>>(),
        [ids[0], ids[1]]
    );
    assert_eq!(failed[0].last_error.as_deref(), Some("disk full"));
    assert_eq!(failed[0].change_kind, "task.save");

    // 後の行が反映済みの行を戻すと、Automerge を古い内容へ巻き戻してしまう
    let report = repositories
        .requeue_failed_sync_changes(&[ids[0], ids[1], ids[2], 9_999])
        .await?;

    assert_eq!(
        report,
        SyncRequeueReport {
            requeued: vec![ids[0]],
            superseded: vec![ids[1]],
            not_failed: vec![ids[2], 9_999],
        }
    );
    let requeued = repository.find_by_id(ids[0]).await?.unwrap();
    assert_eq!(
        (requeued.status.as_str(), requeued.attempts),
        ("pending", 0)
    );
    assert_eq!(requeued.last_error.as_deref(), Some("disk full"));
    let report = processor.process_pending().await?;
    assert_eq!(report.processed, 1);
    assert_eq!(
        repositories.sync_queue_summary().await?,
        Some(SyncQueueSummary {
            pending: 0,
            failed: 1
        })
    );
    Ok(())
}

#[tokio::test]
async fn a_superseded_change_stays_failed_after_cleanup() -> TestResult {
    let repositories = setup("a_superseded_change_stays_failed").await?;
    let project_id = ProjectId::new();
    enqueue(
        &repositories,
        vec![task_save(project_id), task_save(project_id)],
    )
    .await?;
    let ids = pending_ids(&repositories).await?;
    let processor = repositories.automerge_sync().unwrap();
    let repository = processor.queue().repository();
    repository.mark_failed(ids[0], "disk full").await?;
    processor.process_pending().await?;
    let now = Utc::now();
    repository
        .mark_processed(ids[1], now - chrono::Duration::days(31))
        .await?;

    processor.cleanup_processed(now).await?;
    let report = repositories.requeue_failed_sync_changes(&[ids[0]]).await?;

    assert_eq!(report.superseded, [ids[0]]);
    Ok(())
}

#[tokio::test]
async fn stopping_the_automerge_repo_leaves_the_applied_changes_on_disk() -> TestResult {
    let dir = TestPathGenerator::generate_test_dir(file!(), "stopping_the_automerge_repo");
    std::fs::create_dir_all(&dir)?;
    let f = fixture();
    let project_id = f.project.id;
    {
        let repositories = setup_in(&dir).await?;
        save_fixture(&repositories, &f).await?;
        repositories
            .automerge_sync()
            .unwrap()
            .process_pending()
            .await?;

        assert!(repositories.stop_automerge_repo(Duration::from_secs(10)));
    }

    let reopened = setup_in(&dir).await?;
    let tasks = reopened
        .automerge_sync()
        .unwrap()
        .targets()
        .projects()
        .get_tasks(&project_id)
        .await?;
    assert_eq!(
        tasks.iter().map(|task| task.id).collect::<Vec<_>>(),
        [f.task.id]
    );
    Ok(())
}

/// `parent` の子タスク（リストには属さない）
fn child_of(parent: &Task, title: &str) -> Task {
    Task {
        id: TaskId::new(),
        list_id: None,
        parent_task_id: Some(parent.id),
        title: title.to_string(),
        ..parent.clone()
    }
}

#[tokio::test]
async fn deleting_a_task_takes_its_subtree_and_restoring_brings_it_back() -> TestResult {
    let repositories = setup("deleting_a_task_takes_its_subtree").await?;
    let f = fixture();
    let project_id = f.project.id;
    save_fixture(&repositories, &f).await?;
    let child = child_of(&f.task, "子");
    let grandchild = child_of(&child, "孫");
    for task in [&child, &grandchild] {
        repositories
            .tasks
            .save(&project_id, task, &f.user_id, &f.timestamp)
            .await?;
    }

    repositories
        .delete_task_transactionally(&project_id, &f.task.id, &f.user_id, &Utc::now())
        .await?;
    for id in [f.task.id, child.id, grandchild.id] {
        assert!(
            repositories
                .tasks
                .find_by_id(&project_id, &id)
                .await?
                .is_none()
        );
    }

    repositories
        .restore_task_transactionally(&project_id, &f.task.id, &f.user_id, &Utc::now())
        .await?;

    let restored_child = repositories
        .tasks
        .find_by_id(&project_id, &child.id)
        .await?
        .expect("child is back");
    assert_eq!(restored_child.parent_task_id, Some(f.task.id));
    let restored_grandchild = repositories
        .tasks
        .find_by_id(&project_id, &grandchild.id)
        .await?
        .expect("grandchild is back");
    assert_eq!(restored_grandchild.parent_task_id, Some(child.id));
    let processor = repositories.automerge_sync().unwrap();
    processor.process_pending().await?;
    let mut active: Vec<TaskId> = processor
        .targets()
        .projects()
        .get_active_tasks(&project_id)
        .await?
        .iter()
        .map(|task| task.id)
        .collect();
    active.sort();
    let mut expected = vec![f.task.id, child.id, grandchild.id];
    expected.sort();
    assert_eq!(active, expected);
    Ok(())
}

/// アプリの更新前にキューへ入ったサブタスクの行は、親を持つタスクとして Automerge に入る
#[tokio::test]
async fn a_queued_legacy_subtask_lands_as_a_child_task() -> TestResult {
    let repositories = setup("a_queued_legacy_subtask").await?;
    let f = fixture();
    let project_id = f.project.id;
    save_fixture(&repositories, &f).await?;
    let subtask_id = TaskId::new();
    let legacy: AutomergeChange = serde_json::from_value(serde_json::json!({
        "target": "sub_task",
        "change": {
            "op": "save",
            "project_id": project_id,
            "entity": {
                "id": subtask_id,
                "task_id": f.task.id,
                "title": "旧サブタスク",
                "status": "not_started",
                "priority": null,
                "order_index": 0,
                "completed": true,
                "created_at": f.timestamp,
                "updated_at": f.timestamp,
                "deleted": false,
                "updated_by": f.user_id,
            },
            "user_id": f.user_id,
            "timestamp": f.timestamp,
        }
    }))?;
    enqueue(&repositories, vec![legacy]).await?;

    let processor = repositories.automerge_sync().unwrap();
    let report = processor.process_pending().await?;

    assert_eq!(report.processed, 4);
    let tasks = processor
        .targets()
        .projects()
        .get_tasks(&project_id)
        .await?;
    let child = tasks
        .iter()
        .find(|task| task.id == subtask_id)
        .expect("legacy subtask was applied");
    assert_eq!(child.parent_task_id, Some(f.task.id));
    assert_eq!(child.status, TaskStatus::Completed);
    Ok(())
}
