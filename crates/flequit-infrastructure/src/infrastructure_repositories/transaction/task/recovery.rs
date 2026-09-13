use flequit_core::ports::infrastructure_repositories::{
    AutomergeRepositoriesPort, AutomergeTaskRepositoryPort, SqliteRepositoriesPort,
};
use flequit_model::models::task_projects::task::Task;
use flequit_model::types::id_types::{ProjectId, TaskId};
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_types::errors::repository_error::RepositoryError;

use flequit_infrastructure_sqlite::infrastructure::operation_journal::{
    JournaledOperation, OperationStatus,
};

use super::mark_status;
use crate::infrastructure_repositories::InfrastructureRepositories;

pub(in crate::infrastructure_repositories::transaction) async fn recover(
    repositories: &InfrastructureRepositories,
    operation: &JournaledOperation,
) -> Result<(), RepositoryError> {
    let forward_task = serde_json::from_str(&operation.forward_data)
        .map_err(|error| RepositoryError::SerializationError(error.to_string()))?;
    let rollback_task = serde_json::from_str(&operation.rollback_data)
        .map_err(|error| RepositoryError::SerializationError(error.to_string()))?;
    let project_id = ProjectId::from(operation.project_id.clone());
    let task_id = TaskId::from(operation.entity_id.clone());
    let sqlite = repositories
        .unified_manager
        .sqlite_repositories()
        .expect("prepared operations require SQLite");
    let automerge = repositories
        .unified_manager
        .automerge_repositories()
        .ok_or_else(|| {
            RepositoryError::ConfigurationError("prepared operations require Automerge".to_string())
        })?;
    let current_revision = sqlite
        .read()
        .await
        .entity_revisions()
        .current("task", &operation.project_id, &operation.entity_id)
        .await?;

    if current_revision > operation.base_revision {
        let current_task = sqlite
            .read()
            .await
            .tasks_repo()
            .find_by_id(&project_id, &task_id)
            .await?
            .ok_or_else(|| RepositoryError::NotFound(operation.entity_id.clone()))?;
        automerge
            .read()
            .await
            .tasks_repo()
            .save_task(
                &project_id,
                &current_task,
                &current_task.updated_by,
                &current_task.updated_at,
            )
            .await?;
        mark_status(
            repositories,
            &operation.operation_id,
            OperationStatus::Committed,
        )
        .await?;
        return Ok(());
    }
    if current_revision < operation.base_revision {
        return Err(RepositoryError::ConstraintViolation(format!(
            "journal revision {} is ahead of stored revision {current_revision}",
            operation.base_revision
        )));
    }

    let transaction = match repositories.begin_transaction().await {
        Ok(transaction) => transaction,
        Err(error) => {
            compensate(repositories, operation, &project_id, &rollback_task).await?;
            tracing::warn!(%error, operation_id = %operation.operation_id, "rolled back prepared task operation");
            return Ok(());
        }
    };
    let sqlite_guard = sqlite.read().await;
    let sqlite_result: Result<(), RepositoryError> = async {
        sqlite_guard
            .tasks_repo()
            .save_with_txn(&transaction, &project_id, &forward_task)
            .await?;
        sqlite_guard
            .entity_revisions()
            .advance_with_txn(
                &transaction,
                "task",
                &operation.project_id,
                &operation.entity_id,
                operation.base_revision,
            )
            .await?;
        Ok(())
    }
    .await;
    drop(sqlite_guard);

    if let Err(error) = sqlite_result {
        repositories.rollback_transaction(transaction).await?;
        compensate(repositories, operation, &project_id, &rollback_task).await?;
        tracing::warn!(%error, operation_id = %operation.operation_id, "rolled back prepared task operation");
        return Ok(());
    }

    if let Err(error) = automerge
        .read()
        .await
        .tasks_repo()
        .save_task(
            &project_id,
            &forward_task,
            &forward_task.updated_by,
            &forward_task.updated_at,
        )
        .await
    {
        repositories.rollback_transaction(transaction).await?;
        compensate(repositories, operation, &project_id, &rollback_task).await?;
        tracing::warn!(%error, operation_id = %operation.operation_id, "rolled back prepared task operation");
        return Ok(());
    }

    if let Err(error) = repositories.commit_transaction(transaction).await {
        restore_rollback_task(repositories, &project_id, &rollback_task).await?;
        return Err(RepositoryError::TransactionError(format!(
            "{error}; commit outcome is uncertain; operation {} remains prepared",
            operation.operation_id
        )));
    }

    mark_status(
        repositories,
        &operation.operation_id,
        OperationStatus::Committed,
    )
    .await
}

async fn compensate(
    repositories: &InfrastructureRepositories,
    operation: &JournaledOperation,
    project_id: &ProjectId,
    rollback_task: &Task,
) -> Result<(), RepositoryError> {
    restore_rollback_task(repositories, project_id, rollback_task).await?;
    mark_status(
        repositories,
        &operation.operation_id,
        OperationStatus::Failed,
    )
    .await
}

async fn restore_rollback_task(
    repositories: &InfrastructureRepositories,
    project_id: &ProjectId,
    rollback_task: &Task,
) -> Result<(), RepositoryError> {
    let automerge = repositories
        .unified_manager
        .automerge_repositories()
        .ok_or_else(|| {
            RepositoryError::ConfigurationError("task compensation requires Automerge".to_string())
        })?;
    automerge
        .read()
        .await
        .tasks_repo()
        .save_task(
            project_id,
            rollback_task,
            &rollback_task.updated_by,
            &rollback_task.updated_at,
        )
        .await
}
