//! ゴミ箱（論理削除済み）からの復元
//!
//! 削除済みのデータは Automerge にだけ残っている。まず対象ドキュメントの
//! 未反映のキューを反映し（読み取りバリア）、旧形式のサブタスクをタスクへ移してから
//! 削除済みデータを読み、「SQLite への再作成 + 復元のキュー登録」を 1 トランザクションで確定する。

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::task::Task;
use flequit_model::traits::Trackable;
use flequit_model::types::id_types::{ProjectId, TagId, TaskId, TaskListId, UserId};
use flequit_types::errors::repository_error::RepositoryError;

use super::super::InfrastructureRepositories;
use crate::automerge_sync::change::project_document_key;
use crate::automerge_sync::{AutomergeChange, AutomergeSyncProcessor, TrashChange};

/// 対象ドキュメントへの未反映の変更を反映し、旧形式のサブタスクを移す
async fn prepare_document(
    processor: &AutomergeSyncProcessor,
    project_id: &ProjectId,
) -> Result<(), RepositoryError> {
    processor
        .flush_document(&project_document_key(project_id))
        .await?;
    processor
        .targets()
        .projects()
        .migrate_legacy_subtasks(project_id)
        .await?;
    Ok(())
}

/// 親を先に並べる（SQLite は親の無い子を外部キーで拒む）
///
/// 親が並びの中にも SQLite にも無いタスクは、親を外して最上位に戻す。
/// 木の組み立ては親の見つからないタスクを最上位として扱うので、見え方は変わらない。
fn parents_first(tasks: Vec<Task>, live: &HashSet<TaskId>) -> Vec<Task> {
    let restoring: HashSet<_> = tasks.iter().map(|task| task.id).collect();
    let mut placed: HashSet<_> = live.clone();
    let mut pending: Vec<Task> = tasks;
    let mut ordered = Vec::with_capacity(pending.len());
    while !pending.is_empty() {
        let before = pending.len();
        pending.retain(|task| {
            let ready = match task.parent_task_id {
                None => true,
                Some(parent) => placed.contains(&parent) || !restoring.contains(&parent),
            };
            if ready {
                let mut task = task.clone();
                if let Some(parent) = task.parent_task_id
                    && !placed.contains(&parent)
                {
                    tracing::warn!(task_id = %task.id, %parent, "restoring a task whose parent is gone; it becomes top-level");
                    task.parent_task_id = None;
                }
                placed.insert(task.id);
                ordered.push(task);
            }
            !ready
        });
        if pending.len() == before {
            // 親子が輪になっている。残りは親を外して戻す
            for mut task in pending.drain(..) {
                tracing::warn!(task_id = %task.id, "restoring a task caught in a parent cycle; it becomes top-level");
                task.parent_task_id = None;
                ordered.push(task);
            }
        }
    }
    ordered
}

pub(super) async fn restore_task(
    repositories: &InfrastructureRepositories,
    project_id: &ProjectId,
    task_id: &TaskId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<(), RepositoryError> {
    let processor = repositories.automerge_sync_or_error()?;
    prepare_document(processor, project_id).await?;
    // SQLite は削除時に子孫も消しているので、同じ削除で消えた子孫も一緒に戻す
    let mut subtree = processor
        .targets()
        .projects()
        .get_deleted_task_subtree(project_id, task_id)
        .await?
        .ok_or_else(|| {
            RepositoryError::NotFound(format!("Task not found or not deleted: {task_id}"))
        })?;
    for task in &mut subtree {
        if task.is_deleted() {
            task.mark_restored(*user_id, *timestamp);
        }
    }
    let live: HashSet<_> = repositories
        .sqlite()?
        .read()
        .await
        .tasks()
        .find_ids_by_project_id(project_id)
        .await?
        .into_iter()
        .collect();
    let subtree = parents_first(subtree, &live);

    let sqlite_repositories = repositories.sqlite()?;
    let sqlite = sqlite_repositories.read().await;
    let txn = processor
        .queue()
        .begin(vec![AutomergeChange::Trash(TrashChange::RestoreTask {
            project_id: *project_id,
            task_id: *task_id,
            user_id: *user_id,
            timestamp: *timestamp,
        })])
        .await?;
    let result: Result<(), RepositoryError> = async {
        for task in &subtree {
            sqlite
                .tasks()
                .save_with_txn(txn.txn(), project_id, task, user_id, timestamp)
                .await?;
        }
        Ok(())
    }
    .await;
    txn.finish(result).await
}

pub(super) async fn restore_tag(
    repositories: &InfrastructureRepositories,
    project_id: &ProjectId,
    tag_id: &TagId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<(), RepositoryError> {
    let processor = repositories.automerge_sync_or_error()?;
    prepare_document(processor, project_id).await?;
    let mut tag = processor
        .targets()
        .projects()
        .get_deleted_tag_by_id(project_id, tag_id)
        .await?
        .ok_or_else(|| {
            RepositoryError::NotFound(format!("Tag not found or not deleted: {tag_id}"))
        })?;
    tag.mark_restored(*user_id, *timestamp);

    let sqlite_repositories = repositories.sqlite()?;
    let sqlite = sqlite_repositories.read().await;
    let txn = processor
        .queue()
        .begin(vec![AutomergeChange::Trash(TrashChange::RestoreTag {
            project_id: *project_id,
            tag_id: *tag_id,
            user_id: *user_id,
            timestamp: *timestamp,
        })])
        .await?;
    let result = sqlite
        .tags()
        .save_with_txn(txn.txn(), project_id, &tag, user_id, timestamp)
        .await;
    txn.finish(result).await
}

pub(super) async fn restore_task_list(
    repositories: &InfrastructureRepositories,
    project_id: &ProjectId,
    task_list_id: &TaskListId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<(), RepositoryError> {
    let processor = repositories.automerge_sync_or_error()?;
    prepare_document(processor, project_id).await?;
    let mut task_list = processor
        .targets()
        .projects()
        .get_deleted_task_list_by_id(project_id, task_list_id)
        .await?
        .ok_or_else(|| {
            RepositoryError::NotFound(format!(
                "Task list not found or not deleted: {task_list_id}"
            ))
        })?;
    task_list.mark_restored(*user_id, *timestamp);

    let sqlite_repositories = repositories.sqlite()?;
    let sqlite = sqlite_repositories.read().await;
    let txn = processor
        .queue()
        .begin(vec![AutomergeChange::Trash(TrashChange::RestoreTaskList {
            project_id: *project_id,
            task_list_id: *task_list_id,
            user_id: *user_id,
            timestamp: *timestamp,
        })])
        .await?;
    let result = sqlite
        .task_lists()
        .save_with_txn(txn.txn(), project_id, &task_list, user_id, timestamp)
        .await;
    txn.finish(result).await
}

/// プロジェクトと、その中の削除済みのタスクリスト・タグ・タスクを戻す
pub(super) async fn restore_project(
    repositories: &InfrastructureRepositories,
    project_id: &ProjectId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<(), RepositoryError> {
    let processor = repositories.automerge_sync_or_error()?;
    prepare_document(processor, project_id).await?;
    let automerge = processor.targets().projects();
    let mut project = automerge
        .get_deleted_project(project_id)
        .await?
        .ok_or_else(|| {
            RepositoryError::NotFound(format!("Project not found or not deleted: {project_id}"))
        })?;
    project.mark_restored(*user_id, *timestamp);
    let mut task_lists = automerge.get_deleted_task_lists(project_id).await?;
    let mut tags = automerge.get_deleted_tags(project_id).await?;
    // プロジェクトを消すと SQLite からはタスクがすべて消える。Automerge で削除済みに
    // なっていないタスク（旧形式から移したサブタスク）も含めて戻す
    let mut tasks: Vec<Task> = automerge
        .get_project_document(project_id)
        .await?
        .map(|document| document.tasks)
        .unwrap_or_default();
    for task_list in &mut task_lists {
        task_list.mark_restored(*user_id, *timestamp);
    }
    for tag in &mut tags {
        tag.mark_restored(*user_id, *timestamp);
    }
    for task in &mut tasks {
        if task.is_deleted() {
            task.mark_restored(*user_id, *timestamp);
        }
    }
    let tasks = parents_first(tasks, &HashSet::new());

    let sqlite_repositories = repositories.sqlite()?;
    let sqlite = sqlite_repositories.read().await;
    let txn = processor
        .queue()
        .begin(vec![AutomergeChange::Trash(TrashChange::RestoreProject {
            project_id: *project_id,
            user_id: *user_id,
            timestamp: *timestamp,
        })])
        .await?;
    // タスクはタスクリストとタグを参照するので最後に戻す
    let result: Result<(), RepositoryError> = async {
        sqlite
            .projects()
            .save_with_txn(txn.txn(), &project, user_id, timestamp)
            .await?;
        for task_list in &task_lists {
            sqlite
                .task_lists()
                .save_with_txn(txn.txn(), project_id, task_list, user_id, timestamp)
                .await?;
        }
        for tag in &tags {
            sqlite
                .tags()
                .save_with_txn(txn.txn(), project_id, tag, user_id, timestamp)
                .await?;
        }
        for task in &tasks {
            sqlite
                .tasks()
                .save_with_txn(txn.txn(), project_id, task, user_id, timestamp)
                .await?;
        }
        Ok(())
    }
    .await;
    txn.finish(result).await
}
