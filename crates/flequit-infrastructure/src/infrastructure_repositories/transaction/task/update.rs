use chrono::{DateTime, Utc};
use flequit_core::ports::infrastructure_repositories::{
    AutomergeRepositoriesPort, AutomergeTaskRepositoryPort, SqliteRepositoriesPort,
};
use flequit_model::models::task_projects::task::PartialTask;
use flequit_model::traits::Trackable;
use flequit_model::types::id_types::{ProjectId, TaskId, UserId};
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_types::errors::repository_error::RepositoryError;
use partially::Partial;
use uuid::Uuid;

use flequit_infrastructure_sqlite::infrastructure::operation_journal::{
    NewOperation, OperationStatus,
};

use super::{mark_status, prepared_recovery_error, result_summary};
use crate::infrastructure_repositories::InfrastructureRepositories;

pub(in crate::infrastructure_repositories::transaction) async fn update(
    repositories: &InfrastructureRepositories,
    project_id: &ProjectId,
    task_id: &TaskId,
    patch: &PartialTask,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<bool, RepositoryError> {
    let _write_guard = repositories.lock_task_write(project_id, task_id).await;
    let Some(mut task) = repositories.tasks.find_by_id(project_id, task_id).await? else {
        return Ok(false);
    };
    let rollback_task = task.clone();
    if !task.apply_some(patch.clone()) {
        return Ok(false);
    }
    task.mark_updated(*user_id, *timestamp);

    let operation_id = Uuid::new_v4().to_string();
    let sqlite_repositories = repositories
        .unified_manager
        .sqlite_repositories()
        .ok_or_else(|| {
            RepositoryError::ConfigurationError("SQLite repositories not initialized".to_string())
        })?;
    let automerge_repositories = repositories
        .unified_manager
        .automerge_repositories()
        .ok_or_else(|| {
            RepositoryError::ConfigurationError(
                "Automerge repositories not initialized".to_string(),
            )
        })?;
    let base_revision = {
        let sqlite_guard = sqlite_repositories.read().await;
        let base_revision = sqlite_guard
            .entity_revisions()
            .current("task", &project_id.to_string(), &task_id.to_string())
            .await?;
        sqlite_guard
            .operation_journal()
            .prepare(NewOperation {
                operation_id: operation_id.clone(),
                entity_kind: "task".to_string(),
                project_id: project_id.to_string(),
                entity_id: task_id.to_string(),
                base_revision,
                forward_data: serde_json::to_string(&task)
                    .map_err(|error| RepositoryError::SerializationError(error.to_string()))?,
                rollback_data: serde_json::to_string(&rollback_task)
                    .map_err(|error| RepositoryError::SerializationError(error.to_string()))?,
                recovery_file_path: None,
                recovery_file_checksum: None,
            })
            .await?;
        base_revision
    };

    let snapshot = {
        let automerge_guard = automerge_repositories.read().await;
        match automerge_guard
            .projects_repo()
            .create_snapshot(project_id)
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                mark_failed(repositories, &operation_id).await;
                return Err(error);
            }
        }
    };

    let transaction = match repositories.begin_transaction().await {
        Ok(transaction) => transaction,
        Err(error) => {
            mark_failed(repositories, &operation_id).await;
            return Err(error);
        }
    };
    let sqlite_guard = sqlite_repositories.read().await;
    let sqlite_result: Result<(), RepositoryError> = async {
        sqlite_guard
            .tasks_repo()
            .save_with_txn(&transaction, project_id, &task)
            .await?;
        sqlite_guard
            .entity_revisions()
            .advance_with_txn(
                &transaction,
                "task",
                &project_id.to_string(),
                &task_id.to_string(),
                base_revision,
            )
            .await?;
        Ok(())
    }
    .await;
    if let Err(error) = sqlite_result {
        drop(sqlite_guard);
        return match repositories.rollback_transaction(transaction).await {
            Ok(()) => {
                mark_failed(repositories, &operation_id).await;
                Err(error)
            }
            Err(rollback_error) => Err(prepared_recovery_error(
                &operation_id,
                &error,
                &rollback_error,
            )),
        };
    }
    drop(sqlite_guard);

    let automerge_guard = automerge_repositories.read().await;
    if let Err(error) = automerge_guard
        .tasks_repo()
        .save_task(project_id, &task, user_id, timestamp)
        .await
    {
        let restore_result = automerge_guard
            .projects_repo()
            .restore_from_snapshot(project_id, &snapshot)
            .await;
        drop(automerge_guard);
        let rollback_result = repositories.rollback_transaction(transaction).await;
        if rollback_result.is_ok() && restore_result.is_ok() {
            mark_failed(repositories, &operation_id).await;
            return Err(error);
        }
        return Err(prepared_recovery_error(
            &operation_id,
            &error,
            &RepositoryError::TransactionError(format!(
                "SQLite rollback: {}; Automerge restore: {}",
                result_summary(&rollback_result),
                result_summary(&restore_result)
            )),
        ));
    }
    drop(automerge_guard);

    if let Err(error) = repositories.commit_transaction(transaction).await {
        let restore_result = automerge_repositories
            .read()
            .await
            .projects_repo()
            .restore_from_snapshot(project_id, &snapshot)
            .await;
        return Err(prepared_recovery_error(
            &operation_id,
            &error,
            &RepositoryError::TransactionError(format!(
                "commit outcome is uncertain; Automerge restore: {}",
                result_summary(&restore_result)
            )),
        ));
    }

    if let Err(error) = mark_status(repositories, &operation_id, OperationStatus::Committed).await {
        tracing::error!(%error, %operation_id, "failed to mark operation committed");
    }

    Ok(true)
}

async fn mark_failed(repositories: &InfrastructureRepositories, operation_id: &str) {
    if let Err(error) = mark_status(repositories, operation_id, OperationStatus::Failed).await {
        tracing::error!(%error, %operation_id, "failed to mark operation failed");
    }
}
