//! Transactional deletion and restore for the integrated infrastructure repositories.
//!
//! SQLite への書き込みと Automerge 同期キューへの登録を 1 トランザクションで行う。
//! Automerge へはワーカーが後から反映するので、Automerge 側のロールバックは要らない。

mod project;
mod restore;
mod tag;
mod task;
mod task_list;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flequit_core::ports::infrastructure_repositories::{
    TransactionalDeletionPort, TransactionalRestorePort,
};
use flequit_infrastructure_sqlite::infrastructure::local_sqlite_repositories::LocalSqliteRepositories;
use flequit_model::traits::TransactionManager;
use flequit_model::types::id_types::{ProjectId, TagId, TaskId, TaskListId, UserId};
use flequit_types::errors::repository_error::RepositoryError;
use sea_orm::DatabaseTransaction;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::InfrastructureRepositories;
use crate::automerge_sync::{AutomergeSyncProcessor, AutomergeSyncQueue};

impl InfrastructureRepositories {
    pub(super) fn sqlite(&self) -> Result<&Arc<RwLock<LocalSqliteRepositories>>, RepositoryError> {
        self.unified_manager.sqlite_repositories().ok_or_else(|| {
            RepositoryError::ConfigurationError("SQLite repositories not initialized".to_string())
        })
    }

    /// 書き込みトランザクションの入口。Automerge が無効ならキューへは入れない
    pub(super) fn sync_queue(&self) -> Result<&AutomergeSyncQueue, RepositoryError> {
        self.unified_manager.sync_queue_ref().ok_or_else(|| {
            RepositoryError::ConfigurationError("SQLite repositories not initialized".to_string())
        })
    }

    /// 復元は Automerge にしか残っていない削除済みデータを読むため、Automerge が必要
    pub(super) fn automerge_sync_or_error(
        &self,
    ) -> Result<&Arc<AutomergeSyncProcessor>, RepositoryError> {
        self.unified_manager.automerge_sync().ok_or_else(|| {
            RepositoryError::ConfigurationError(
                "restoring deleted items requires both SQLite and Automerge storage".to_string(),
            )
        })
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

#[async_trait]
impl TransactionalRestorePort for InfrastructureRepositories {
    async fn restore_project_transactionally(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        restore::restore_project(self, project_id, user_id, timestamp).await
    }

    async fn restore_task_transactionally(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        restore::restore_task(self, project_id, task_id, user_id, timestamp).await
    }

    async fn restore_task_list_transactionally(
        &self,
        project_id: &ProjectId,
        task_list_id: &TaskListId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        restore::restore_task_list(self, project_id, task_list_id, user_id, timestamp).await
    }

    async fn restore_tag_transactionally(
        &self,
        project_id: &ProjectId,
        tag_id: &TagId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        restore::restore_tag(self, project_id, tag_id, user_id, timestamp).await
    }
}
