//! タスク繰り返しルール用Automergeリポジトリ

use super::super::document_manager::{DocumentManager, DocumentType};
use crate::infrastructure::collection::Collection;
use crate::infrastructure::document::Document;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::task_recurrence::TaskRecurrence;
use flequit_model::types::id_types::{
    ProjectId, RecurrenceRuleId, TaskId, TaskRecurrenceId, UserId,
};
use flequit_repository::repositories::base_repository_trait::Repository;
use flequit_repository::repositories::project_relation_repository_trait::ProjectRelationRepository;
use flequit_repository::repositories::task_projects::task_recurrence_repository_trait::TaskRecurrenceRepositoryTrait;
use flequit_types::errors::repository_error::RepositoryError;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TaskRecurrenceRelation {
    task_id: String,
    recurrence_rule_id: String,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
    updated_by: String,
    deleted: bool,
}

/// プロジェクトドキュメント内のタスクと繰り返しルールの関連（キーはタスク ID。1 タスクに 1 ルール）
pub(crate) const TASK_RECURRENCES: Collection<TaskRecurrenceRelation> =
    Collection::new("task_recurrences", |relation| relation.task_id.clone());

#[derive(Debug)]
pub struct TaskRecurrenceLocalAutomergeRepository {
    document_manager: Arc<Mutex<DocumentManager>>,
}

impl TaskRecurrenceLocalAutomergeRepository {
    pub async fn new(base_path: PathBuf) -> Result<Self, RepositoryError> {
        let document_manager = DocumentManager::new(base_path)
            .map_err(|e| RepositoryError::AutomergeError(e.to_string()))?;
        Ok(Self {
            document_manager: Arc::new(Mutex::new(document_manager)),
        })
    }

    pub async fn new_with_manager(
        document_manager: Arc<Mutex<DocumentManager>>,
    ) -> Result<Self, RepositoryError> {
        Ok(Self { document_manager })
    }

    async fn get_or_create_document(
        &self,
        project_id: &ProjectId,
    ) -> Result<Document, RepositoryError> {
        let document_type = DocumentType::Project(*project_id);
        let mut manager = self.document_manager.lock().await;
        manager
            .get_or_create(&document_type)
            .await
            .map_err(|e| RepositoryError::AutomergeError(e.to_string()))
    }

    async fn load_all(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<TaskRecurrenceRelation>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.load_collection(&TASK_RECURRENCES).await?)
    }

    async fn load(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
    ) -> Result<Option<TaskRecurrenceRelation>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document
            .load_entry(&TASK_RECURRENCES, &task_id.to_string())
            .await?)
    }
}

impl TaskRecurrenceRepositoryTrait for TaskRecurrenceLocalAutomergeRepository {}

#[async_trait]
impl Repository<TaskRecurrence, TaskRecurrenceId> for TaskRecurrenceLocalAutomergeRepository {
    async fn save(
        &self,
        _entity: &TaskRecurrence,
        _user_id: &UserId,
        _timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        Err(RepositoryError::InvalidOperation(
            "Use ProjectRelationRepository::add instead".into(),
        ))
    }

    async fn find_by_id(
        &self,
        _id: &TaskRecurrenceId,
    ) -> Result<Option<TaskRecurrence>, RepositoryError> {
        Err(RepositoryError::InvalidOperation(
            "Identified by (project_id, task_id, recurrence_rule_id) on Automerge".into(),
        ))
    }

    async fn find_all(&self) -> Result<Vec<TaskRecurrence>, RepositoryError> {
        Err(RepositoryError::InvalidOperation(
            "Use ProjectRelationRepository::find_all(project_id) instead".into(),
        ))
    }

    async fn delete(&self, _id: &TaskRecurrenceId) -> Result<(), RepositoryError> {
        Err(RepositoryError::InvalidOperation(
            "Use ProjectRelationRepository::remove instead".into(),
        ))
    }

    async fn exists(&self, _id: &TaskRecurrenceId) -> Result<bool, RepositoryError> {
        Err(RepositoryError::InvalidOperation(
            "Use ProjectRelationRepository::exists instead".into(),
        ))
    }

    async fn count(&self) -> Result<u64, RepositoryError> {
        Err(RepositoryError::InvalidOperation(
            "Use ProjectRelationRepository::count instead".into(),
        ))
    }
}

#[async_trait]
impl ProjectRelationRepository<TaskRecurrence, TaskId, RecurrenceRuleId>
    for TaskRecurrenceLocalAutomergeRepository
{
    async fn add(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
        child_id: &RecurrenceRuleId,
        _user_id: &UserId,
        _timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        // タスクごとに1つのルールのみ許可（キーがタスク ID なので既存を置き換える）
        let relation = TaskRecurrenceRelation {
            task_id: parent_id.to_string(),
            recurrence_rule_id: child_id.to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            updated_by: UserId::from("system").to_string(),
            deleted: false,
        };
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.put_entry(&TASK_RECURRENCES, &relation).await?)
    }

    async fn remove(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
        child_id: &RecurrenceRuleId,
    ) -> Result<(), RepositoryError> {
        let (t, r) = (parent_id.to_string(), child_id.to_string());
        let document = self.get_or_create_document(project_id).await?;
        document
            .delete_entries_where(&TASK_RECURRENCES, |x| {
                x.task_id == t && x.recurrence_rule_id == r
            })
            .await?;
        Ok(())
    }

    async fn remove_all(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        document
            .delete_entry(&TASK_RECURRENCES, &parent_id.to_string())
            .await?;
        Ok(())
    }

    async fn find_relations(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
    ) -> Result<Vec<TaskRecurrence>, RepositoryError> {
        let found = self.load(project_id, parent_id).await?;
        Ok(found.into_iter().map(to_task_recurrence).collect())
    }

    async fn exists(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
    ) -> Result<bool, RepositoryError> {
        Ok(self.load(project_id, parent_id).await?.is_some())
    }

    async fn count(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
    ) -> Result<u64, RepositoryError> {
        Ok(u64::from(self.load(project_id, parent_id).await?.is_some()))
    }

    async fn find_all(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<TaskRecurrence>, RepositoryError> {
        let list = self.load_all(project_id).await?;
        Ok(list.into_iter().map(to_task_recurrence).collect())
    }

    async fn find_relation(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
        child_id: &RecurrenceRuleId,
    ) -> Result<Option<TaskRecurrence>, RepositoryError> {
        let r = child_id.to_string();
        let found = self.load(project_id, parent_id).await?;
        Ok(found
            .filter(|x| x.recurrence_rule_id == r)
            .map(to_task_recurrence))
    }
}

fn to_task_recurrence(relation: TaskRecurrenceRelation) -> TaskRecurrence {
    TaskRecurrence {
        task_id: TaskId::from(relation.task_id),
        recurrence_rule_id: RecurrenceRuleId::from(relation.recurrence_rule_id),
        created_at: relation.created_at,
        updated_at: relation.updated_at,
        updated_by: UserId::from(relation.updated_by),
        deleted: relation.deleted,
    }
}
