//! Transactional deletion for the integrated infrastructure repositories.

mod project;
mod tag;
mod task;
mod task_list;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flequit_core::ports::infrastructure_repositories::TransactionalDeletionPort;
use flequit_model::traits::TransactionManager;
use flequit_model::types::id_types::{ProjectId, TagId, TaskId, TaskListId, UserId};
use flequit_types::errors::repository_error::RepositoryError;
use sea_orm::DatabaseTransaction;

use super::InfrastructureRepositories;

impl InfrastructureRepositories {
    pub(super) async fn rollback_with_error(
        &self,
        transaction: DatabaseTransaction,
        operation_error: RepositoryError,
    ) -> RepositoryError {
        match self.rollback_transaction(transaction).await {
            Ok(()) => operation_error,
            Err(rollback_error) => RepositoryError::TransactionError(format!(
                "{operation_error}; rollback failed: {rollback_error}"
            )),
        }
    }

    pub(super) async fn begin_transaction(&self) -> Result<DatabaseTransaction, RepositoryError> {
        let sqlite_repositories = self.unified_manager.sqlite_repositories().ok_or_else(|| {
            RepositoryError::ConfigurationError("SQLite repositories not initialized".to_string())
        })?;
        let sqlite_guard = sqlite_repositories.read().await;
        let database_manager = sqlite_guard.database_manager();
        let database_guard = database_manager.read().await;
        database_guard.begin().await
    }

    pub(super) async fn commit_transaction(
        &self,
        transaction: DatabaseTransaction,
    ) -> Result<(), RepositoryError> {
        transaction.commit().await.map_err(|error| {
            RepositoryError::TransactionError(format!("failed to commit transaction: {error}"))
        })
    }

    async fn rollback_transaction(
        &self,
        transaction: DatabaseTransaction,
    ) -> Result<(), RepositoryError> {
        transaction.rollback().await.map_err(|error| {
            RepositoryError::TransactionError(format!("failed to roll back transaction: {error}"))
        })
    }
}

#[async_trait]
impl TransactionManager for InfrastructureRepositories {
    type Transaction = DatabaseTransaction;

    async fn begin(&self) -> Result<Self::Transaction, RepositoryError> {
        self.begin_transaction().await
    }

    async fn commit(&self, transaction: Self::Transaction) -> Result<(), RepositoryError> {
        self.commit_transaction(transaction).await
    }

    async fn rollback(&self, transaction: Self::Transaction) -> Result<(), RepositoryError> {
        self.rollback_transaction(transaction).await
    }
}

#[async_trait]
impl TransactionalDeletionPort for InfrastructureRepositories {
    async fn delete_project_transactionally(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        project::delete(self, project_id, user_id, timestamp).await
    }

    async fn delete_task_transactionally(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        task::delete(self, project_id, task_id, user_id, timestamp).await
    }

    async fn delete_task_list_transactionally(
        &self,
        project_id: &ProjectId,
        task_list_id: &TaskListId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        task_list::delete(self, project_id, task_list_id, user_id, timestamp).await
    }

    async fn delete_tag_transactionally(
        &self,
        project_id: &ProjectId,
        tag_id: &TagId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        tag::delete(self, project_id, tag_id, user_id, timestamp).await
    }
}
