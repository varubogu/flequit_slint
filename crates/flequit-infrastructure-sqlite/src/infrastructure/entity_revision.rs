use std::sync::Arc;

use chrono::Utc;
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
use tokio::sync::RwLock;

use crate::errors::sqlite_error::SQLiteError;
use crate::infrastructure::database_manager::DatabaseManager;
use crate::models::entity_revision::{ActiveModel, Entity};
use flequit_types::errors::repository_error::RepositoryError;

#[derive(Debug)]
pub struct EntityRevisionRepository {
    db_manager: Arc<RwLock<DatabaseManager>>,
}

impl EntityRevisionRepository {
    pub fn new(db_manager: Arc<RwLock<DatabaseManager>>) -> Self {
        Self { db_manager }
    }

    pub async fn current(
        &self,
        entity_kind: &str,
        project_id: &str,
        entity_id: &str,
    ) -> Result<u64, RepositoryError> {
        let db_manager = self.db_manager.read().await;
        let db = db_manager.get_connection().await?;
        let revision = Entity::find_by_id((
            entity_kind.to_string(),
            project_id.to_string(),
            entity_id.to_string(),
        ))
        .one(db)
        .await
        .map_err(|error| RepositoryError::from(SQLiteError::from(error)))?
        .map_or(0, |model| model.revision);
        u64::try_from(revision).map_err(|_| {
            RepositoryError::ConversionError(format!(
                "negative revision for {entity_kind}:{project_id}:{entity_id}"
            ))
        })
    }

    pub async fn advance_with_txn(
        &self,
        txn: &sea_orm::DatabaseTransaction,
        entity_kind: &str,
        project_id: &str,
        entity_id: &str,
        expected: u64,
    ) -> Result<u64, RepositoryError> {
        let key = (
            entity_kind.to_string(),
            project_id.to_string(),
            entity_id.to_string(),
        );
        let existing = Entity::find_by_id(key.clone())
            .one(txn)
            .await
            .map_err(|error| RepositoryError::from(SQLiteError::from(error)))?;
        let actual = existing.as_ref().map_or(0, |model| model.revision);
        let expected = i64::try_from(expected).map_err(|_| {
            RepositoryError::ConversionError("revision exceeds SQLite integer range".to_string())
        })?;
        if actual != expected {
            return Err(RepositoryError::ConstraintViolation(format!(
                "revision conflict for {entity_kind}:{project_id}:{entity_id}: expected {expected}, actual {actual}"
            )));
        }

        let next = actual
            .checked_add(1)
            .ok_or_else(|| RepositoryError::ConstraintViolation("revision overflow".to_string()))?;
        if let Some(model) = existing {
            let mut active = model.into_active_model();
            active.revision = Set(next);
            active.updated_at = Set(Utc::now());
            active
                .update(txn)
                .await
                .map_err(|error| RepositoryError::from(SQLiteError::from(error)))?;
        } else {
            ActiveModel {
                entity_kind: Set(key.0),
                project_id: Set(key.1),
                entity_id: Set(key.2),
                revision: Set(next),
                updated_at: Set(Utc::now()),
            }
            .insert(txn)
            .await
            .map_err(|error| RepositoryError::from(SQLiteError::from(error)))?;
        }

        u64::try_from(next)
            .map_err(|_| RepositoryError::ConversionError("revision became negative".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::TransactionTrait;
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn revision_advance_participates_in_the_callers_transaction()
    -> Result<(), Box<dyn std::error::Error>> {
        let (_temp_dir, db_manager, repository) = test_repository().await?;
        let db_guard = db_manager.read().await;
        let connection = db_guard.get_connection().await?;

        let rolled_back = connection.begin().await?;
        assert_eq!(
            repository
                .advance_with_txn(&rolled_back, "task", "project", "task", 0)
                .await?,
            1
        );
        rolled_back.rollback().await?;
        assert_eq!(repository.current("task", "project", "task").await?, 0);

        let committed = connection.begin().await?;
        repository
            .advance_with_txn(&committed, "task", "project", "task", 0)
            .await?;
        committed.commit().await?;
        assert_eq!(repository.current("task", "project", "task").await?, 1);

        let conflicted = connection.begin().await?;
        assert!(
            repository
                .advance_with_txn(&conflicted, "task", "project", "task", 0)
                .await
                .is_err()
        );
        conflicted.rollback().await?;
        Ok(())
    }

    async fn test_repository() -> Result<
        (
            TempDir,
            Arc<RwLock<DatabaseManager>>,
            EntityRevisionRepository,
        ),
        Box<dyn std::error::Error>,
    > {
        let temp_dir = tempfile::tempdir()?;
        let database_path = temp_dir.path().join("entity-revision.sqlite");
        let db_manager = Arc::new(RwLock::new(DatabaseManager::new_for_test(
            database_path.to_string_lossy(),
        )));
        db_manager.read().await.get_connection().await?;
        let repository = EntityRevisionRepository::new(Arc::clone(&db_manager));
        Ok((temp_dir, db_manager, repository))
    }
}
