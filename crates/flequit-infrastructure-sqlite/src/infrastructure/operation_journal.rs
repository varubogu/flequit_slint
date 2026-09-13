use std::sync::Arc;

use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
};
use tokio::sync::RwLock;

use crate::errors::sqlite_error::SQLiteError;
use crate::infrastructure::database_manager::DatabaseManager;
use crate::models::operation_journal::{ActiveModel, Column, Entity, Model};
use flequit_types::errors::repository_error::RepositoryError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationStatus {
    Prepared,
    Committed,
    Failed,
}

impl OperationStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Committed => "committed",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Result<Self, RepositoryError> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "committed" => Ok(Self::Committed),
            "failed" => Ok(Self::Failed),
            _ => Err(RepositoryError::ConversionError(format!(
                "unknown operation status: {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOperation {
    pub operation_id: String,
    pub entity_kind: String,
    pub project_id: String,
    pub entity_id: String,
    pub base_revision: u64,
    pub forward_data: String,
    pub rollback_data: String,
    pub recovery_file_path: Option<String>,
    pub recovery_file_checksum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournaledOperation {
    pub operation_id: String,
    pub entity_kind: String,
    pub project_id: String,
    pub entity_id: String,
    pub base_revision: u64,
    pub forward_data: String,
    pub rollback_data: String,
    pub status: OperationStatus,
    pub recovery_file_path: Option<String>,
    pub recovery_file_checksum: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TryFrom<Model> for JournaledOperation {
    type Error = RepositoryError;

    fn try_from(model: Model) -> Result<Self, Self::Error> {
        Ok(Self {
            operation_id: model.operation_id,
            entity_kind: model.entity_kind,
            project_id: model.project_id,
            entity_id: model.entity_id,
            base_revision: u64::try_from(model.base_revision).map_err(|_| {
                RepositoryError::ConversionError("negative journal revision".to_string())
            })?,
            forward_data: model.forward_data,
            rollback_data: model.rollback_data,
            status: OperationStatus::parse(&model.status)?,
            recovery_file_path: model.recovery_file_path,
            recovery_file_checksum: model.recovery_file_checksum,
            created_at: model.created_at,
            updated_at: model.updated_at,
        })
    }
}

#[derive(Debug)]
pub struct OperationJournalRepository {
    db_manager: Arc<RwLock<DatabaseManager>>,
}

impl OperationJournalRepository {
    pub fn new(db_manager: Arc<RwLock<DatabaseManager>>) -> Self {
        Self { db_manager }
    }

    pub async fn prepare(&self, operation: NewOperation) -> Result<(), RepositoryError> {
        let base_revision = i64::try_from(operation.base_revision).map_err(|_| {
            RepositoryError::ConversionError("revision exceeds SQLite integer range".to_string())
        })?;
        let now = Utc::now();
        let active = ActiveModel {
            operation_id: Set(operation.operation_id),
            entity_kind: Set(operation.entity_kind),
            project_id: Set(operation.project_id),
            entity_id: Set(operation.entity_id),
            base_revision: Set(base_revision),
            forward_data: Set(operation.forward_data),
            rollback_data: Set(operation.rollback_data),
            status: Set(OperationStatus::Prepared.as_str().to_string()),
            recovery_file_path: Set(operation.recovery_file_path),
            recovery_file_checksum: Set(operation.recovery_file_checksum),
            created_at: Set(now),
            updated_at: Set(now),
        };

        let db_manager = self.db_manager.read().await;
        let db = db_manager.get_connection().await?;
        active
            .insert(db)
            .await
            .map_err(|error| RepositoryError::from(SQLiteError::from(error)))?;
        Ok(())
    }

    pub async fn mark_status(
        &self,
        operation_id: &str,
        status: OperationStatus,
    ) -> Result<(), RepositoryError> {
        let db_manager = self.db_manager.read().await;
        let db = db_manager.get_connection().await?;
        let model = Entity::find_by_id(operation_id.to_string())
            .one(db)
            .await
            .map_err(|error| RepositoryError::from(SQLiteError::from(error)))?
            .ok_or_else(|| RepositoryError::NotFound(operation_id.to_string()))?;
        let mut active = model.into_active_model();
        active.status = Set(status.as_str().to_string());
        active.updated_at = Set(Utc::now());
        active
            .update(db)
            .await
            .map_err(|error| RepositoryError::from(SQLiteError::from(error)))?;
        Ok(())
    }

    pub async fn prepared(&self) -> Result<Vec<JournaledOperation>, RepositoryError> {
        let db_manager = self.db_manager.read().await;
        let db = db_manager.get_connection().await?;
        Entity::find()
            .filter(Column::Status.eq(OperationStatus::Prepared.as_str()))
            .order_by_asc(Column::CreatedAt)
            .all(db)
            .await
            .map_err(|error| RepositoryError::from(SQLiteError::from(error)))?
            .into_iter()
            .map(JournaledOperation::try_from)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn prepared_operations_survive_until_their_terminal_status_is_recorded()
    -> Result<(), Box<dyn std::error::Error>> {
        let (_temp_dir, repository) = test_repository().await?;
        repository
            .prepare(NewOperation {
                operation_id: "operation".to_string(),
                entity_kind: "task".to_string(),
                project_id: "project".to_string(),
                entity_id: "task".to_string(),
                base_revision: 7,
                forward_data: "forward".to_string(),
                rollback_data: "rollback".to_string(),
                recovery_file_path: None,
                recovery_file_checksum: None,
            })
            .await?;

        let prepared = repository.prepared().await?;
        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].base_revision, 7);
        assert_eq!(prepared[0].status, OperationStatus::Prepared);

        repository
            .mark_status("operation", OperationStatus::Committed)
            .await?;
        assert!(repository.prepared().await?.is_empty());
        Ok(())
    }

    async fn test_repository()
    -> Result<(TempDir, OperationJournalRepository), Box<dyn std::error::Error>> {
        let temp_dir = tempfile::tempdir()?;
        let database_path = temp_dir.path().join("operation-journal.sqlite");
        let db_manager = Arc::new(RwLock::new(DatabaseManager::new_for_test(
            database_path.to_string_lossy(),
        )));
        db_manager.read().await.get_connection().await?;
        Ok((temp_dir, OperationJournalRepository::new(db_manager)))
    }
}
