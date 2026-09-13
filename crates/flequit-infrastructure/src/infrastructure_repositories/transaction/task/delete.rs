use chrono::{DateTime, Utc};
use flequit_model::types::id_types::{ProjectId, TaskId, UserId};
use flequit_types::errors::repository_error::RepositoryError;

use crate::infrastructure_repositories::InfrastructureRepositories;

pub(in crate::infrastructure_repositories::transaction) async fn delete(
    repositories: &InfrastructureRepositories,
    project_id: &ProjectId,
    task_id: &TaskId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<(), RepositoryError> {
    let snapshot = if let Some(automerge) = repositories.unified_manager.automerge_repositories() {
        let automerge_guard = automerge.read().await;
        match automerge_guard.projects().create_snapshot(project_id).await {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                tracing::warn!(%error, "failed to create Automerge snapshot");
                None
            }
        }
    } else {
        None
    };

    let transaction = repositories.begin_transaction().await?;
    let sqlite_repositories = repositories
        .unified_manager
        .sqlite_repositories()
        .expect("SQLite repositories exist after transaction start");
    let sqlite_guard = sqlite_repositories.read().await;

    let sqlite_result: Result<(), RepositoryError> = async {
        sqlite_guard
            .sub_tasks()
            .remove_all_by_task_id_with_txn(&transaction, project_id, &task_id.to_string())
            .await?;
        sqlite_guard
            .task_tags
            .remove_all_by_task_id_with_txn(&transaction, project_id, task_id)
            .await?;
        sqlite_guard
            .task_assignments()
            .remove_all_by_task_id_with_txn(&transaction, task_id)
            .await?;
        sqlite_guard
            .task_recurrences
            .remove_all_with_txn(&transaction, project_id, task_id)
            .await?;
        sqlite_guard
            .tasks()
            .delete_with_txn(&transaction, project_id, task_id)
            .await?;
        Ok(())
    }
    .await;
    drop(sqlite_guard);

    if let Err(error) = sqlite_result {
        return Err(repositories.rollback_with_error(transaction, error).await);
    }

    if let Some(automerge) = repositories.unified_manager.automerge_repositories() {
        let automerge_guard = automerge.read().await;
        if let Err(error) = automerge_guard
            .projects()
            .mark_task_deleted(project_id, task_id, user_id, timestamp)
            .await
        {
            if let Some(snapshot) = snapshot.as_ref()
                && let Err(restore_error) = automerge_guard
                    .projects()
                    .restore_from_snapshot(project_id, snapshot)
                    .await
            {
                tracing::error!(%restore_error, "failed to restore Automerge snapshot");
            }
            drop(automerge_guard);
            return Err(repositories.rollback_with_error(transaction, error).await);
        }
    }

    if let Err(error) = repositories.commit_transaction(transaction).await {
        if let (Some(snapshot), Some(automerge)) = (
            snapshot.as_ref(),
            repositories.unified_manager.automerge_repositories(),
        ) {
            let automerge_guard = automerge.read().await;
            if let Err(restore_error) = automerge_guard
                .projects()
                .restore_from_snapshot(project_id, snapshot)
                .await
            {
                tracing::error!(%restore_error, "failed to restore Automerge snapshot");
            }
        }
        return Err(error);
    }

    Ok(())
}
