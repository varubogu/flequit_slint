//! ゴミ箱（論理削除済み）からの復元
//!
//! 削除済みのデータは Automerge にだけ残っている。まず対象ドキュメントの
//! 未反映のキューを反映し（読み取りバリア）、削除済みデータを読んでから
//! 「SQLite への再作成 + 復元のキュー登録」を 1 トランザクションで確定する。

use chrono::{DateTime, Utc};
use flequit_model::traits::Trackable;
use flequit_model::types::id_types::{ProjectId, TagId, TaskId, TaskListId, UserId};
use flequit_types::errors::repository_error::RepositoryError;

use super::super::InfrastructureRepositories;
use crate::automerge_sync::change::project_document_key;
use crate::automerge_sync::{AutomergeChange, TrashChange};

pub(super) async fn restore_task(
    repositories: &InfrastructureRepositories,
    project_id: &ProjectId,
    task_id: &TaskId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<(), RepositoryError> {
    let processor = repositories.automerge_sync_or_error()?;
    processor
        .flush_document(&project_document_key(project_id))
        .await?;
    let mut task = processor
        .targets()
        .projects()
        .get_deleted_task_by_id(project_id, task_id)
        .await?
        .ok_or_else(|| {
            RepositoryError::NotFound(format!("Task not found or not deleted: {task_id}"))
        })?;
    task.mark_restored(*user_id, *timestamp);

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
    let result = sqlite
        .tasks()
        .save_with_txn(txn.txn(), project_id, &task, user_id, timestamp)
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
    processor
        .flush_document(&project_document_key(project_id))
        .await?;
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
    processor
        .flush_document(&project_document_key(project_id))
        .await?;
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
    processor
        .flush_document(&project_document_key(project_id))
        .await?;
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
    let mut tasks = automerge.get_deleted_tasks(project_id).await?;
    for task_list in &mut task_lists {
        task_list.mark_restored(*user_id, *timestamp);
    }
    for tag in &mut tags {
        tag.mark_restored(*user_id, *timestamp);
    }
    for task in &mut tasks {
        task.mark_restored(*user_id, *timestamp);
    }

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
