//! TaskAssignment用Automergeリポジトリ

use crate::infrastructure::collection::{Collection, relation_key};
use crate::infrastructure::document::Document;

use super::super::document_manager::{DocumentManager, DocumentType};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::task_assignment::TaskAssignment;
use flequit_model::types::id_types::{ProjectId, TaskId, UserId};
use flequit_repository::repositories::project_relation_repository_trait::ProjectRelationRepository;
use flequit_repository::repositories::task_projects::task_assignment_repository_trait::TaskAssignmentRepositoryTrait;
use flequit_types::errors::repository_error::RepositoryError;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TaskAssignmentRelation {
    task_id: String,
    user_id: String,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
    updated_by: String,
    deleted: bool,
}

/// プロジェクトドキュメント内のタスクとユーザーの割り当て（キーは `{task_id}:{user_id}`）
pub(crate) const TASK_ASSIGNMENTS: Collection<TaskAssignmentRelation> =
    Collection::new("task_assignments", |relation| {
        relation_key(&relation.task_id, &relation.user_id)
    });

#[derive(Debug)]
pub struct TaskAssignmentLocalAutomergeRepository {
    document_manager: Arc<Mutex<DocumentManager>>,
}

impl TaskAssignmentLocalAutomergeRepository {
    /// 新しいTaskAssignmentRepositoryを作成
    pub async fn new(base_path: PathBuf) -> Result<Self, RepositoryError> {
        let document_manager = DocumentManager::new(base_path)?;
        Ok(Self {
            document_manager: Arc::new(Mutex::new(document_manager)),
        })
    }

    /// 共有DocumentManagerを使用して新しいインスタンスを作成
    pub async fn new_with_manager(
        document_manager: Arc<Mutex<DocumentManager>>,
    ) -> Result<Self, RepositoryError> {
        Ok(Self { document_manager })
    }

    /// 指定されたプロジェクトのDocumentを取得または作成
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

    /// 指定タスクのユーザーIDリストを取得
    pub async fn find_user_ids_by_task_id(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
    ) -> Result<Vec<UserId>, RepositoryError> {
        let task_id = task_id.to_string();
        let assignments = self.load_all(project_id).await?;
        Ok(assignments
            .into_iter()
            .filter(|a| a.task_id == task_id)
            .map(|a| UserId::from(a.user_id))
            .collect())
    }

    /// 指定ユーザーに関連するタスクIDリストを取得
    pub async fn find_task_ids_by_user_id(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
    ) -> Result<Vec<TaskId>, RepositoryError> {
        let user_id = user_id.to_string();
        let assignments = self.load_all(project_id).await?;
        Ok(assignments
            .into_iter()
            .filter(|a| a.user_id == user_id)
            .map(|a| TaskId::from(a.task_id))
            .collect())
    }

    /// タスクとユーザーの割り当てを追加（既にあれば何もしない）
    pub async fn add_assignment(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
        user_id: &UserId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let key = relation_key(&task_id.to_string(), &user_id.to_string());
        let existing: Option<TaskAssignmentRelation> =
            document.load_entry(&TASK_ASSIGNMENTS, &key).await?;
        if existing.is_none() {
            document
                .put_entry(&TASK_ASSIGNMENTS, &new_assignment(task_id, user_id))
                .await?;
        }
        Ok(())
    }

    /// タスクとユーザーの割り当てを削除
    pub async fn remove_assignment(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
        user_id: &UserId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let key = relation_key(&task_id.to_string(), &user_id.to_string());
        document.delete_entry(&TASK_ASSIGNMENTS, &key).await?;
        Ok(())
    }

    /// 指定タスクの全ての割り当てを削除
    pub async fn remove_all_assignments_by_task_id(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let task_id = task_id.to_string();
        document
            .delete_entries_where(&TASK_ASSIGNMENTS, |a| a.task_id == task_id)
            .await?;
        Ok(())
    }

    /// 指定ユーザーの全ての割り当てを削除
    pub async fn remove_all_assignments_by_user_id(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let user_id = user_id.to_string();
        document
            .delete_entries_where(&TASK_ASSIGNMENTS, |a| a.user_id == user_id)
            .await?;
        Ok(())
    }

    /// タスクのユーザー割り当てを `user_ids` にする
    ///
    /// 外れたユーザーの割り当てだけを消し、新しいユーザーの割り当てだけを追加する。
    /// 続けて割り当てられているユーザーの割り当ては触らない。
    pub async fn update_task_assignments(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
        user_ids: &[UserId],
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let task_id_str = task_id.to_string();
        let wanted: Vec<String> = user_ids.iter().map(ToString::to_string).collect();
        let current: Vec<String> = self
            .load_all(project_id)
            .await?
            .into_iter()
            .filter(|a| a.task_id == task_id_str)
            .map(|a| a.user_id)
            .collect();

        let removed: Vec<String> = current
            .iter()
            .filter(|user_id| !wanted.contains(user_id))
            .map(|user_id| relation_key(&task_id_str, user_id))
            .collect();
        document.delete_entries(&TASK_ASSIGNMENTS, &removed).await?;

        let added: Vec<TaskAssignmentRelation> = user_ids
            .iter()
            .filter(|user_id| !current.contains(&user_id.to_string()))
            .map(|user_id| new_assignment(task_id, user_id))
            .collect();
        document.put_entries(&TASK_ASSIGNMENTS, &added).await?;
        Ok(())
    }

    async fn load_all(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<TaskAssignmentRelation>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.load_collection(&TASK_ASSIGNMENTS).await?)
    }
}

fn new_assignment(task_id: &TaskId, user_id: &UserId) -> TaskAssignmentRelation {
    TaskAssignmentRelation {
        task_id: task_id.to_string(),
        user_id: user_id.to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        updated_by: user_id.to_string(),
        deleted: false,
    }
}

// TaskAssignmentRepositoryTrait の実装
impl TaskAssignmentRepositoryTrait for TaskAssignmentLocalAutomergeRepository {}

#[async_trait]
impl ProjectRelationRepository<TaskAssignment, TaskId, UserId>
    for TaskAssignmentLocalAutomergeRepository
{
    async fn add(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
        child_id: &UserId,
        _user_id: &UserId,
        _timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.add_assignment(project_id, parent_id, child_id).await
    }

    async fn remove(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
        child_id: &UserId,
    ) -> Result<(), RepositoryError> {
        self.remove_assignment(project_id, parent_id, child_id)
            .await
    }

    async fn remove_all(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
    ) -> Result<(), RepositoryError> {
        self.remove_all_assignments_by_task_id(project_id, parent_id)
            .await
    }

    async fn find_relations(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
    ) -> Result<Vec<TaskAssignment>, RepositoryError> {
        let user_ids = self.find_user_ids_by_task_id(project_id, parent_id).await?;
        let mut relations = Vec::new();

        // 各ユーザーIDに対して TaskAssignment を作成
        for user_id in user_ids {
            let assignment = TaskAssignment {
                task_id: *parent_id,
                user_id,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                updated_by: user_id,
                deleted: false,
            };
            relations.push(assignment);
        }

        Ok(relations)
    }

    async fn exists(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
    ) -> Result<bool, RepositoryError> {
        let user_ids = self.find_user_ids_by_task_id(project_id, parent_id).await?;
        Ok(!user_ids.is_empty())
    }

    async fn count(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
    ) -> Result<u64, RepositoryError> {
        let user_ids = self.find_user_ids_by_task_id(project_id, parent_id).await?;
        Ok(user_ids.len() as u64)
    }

    async fn find_relation(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
        child_id: &UserId,
    ) -> Result<Option<TaskAssignment>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let key = relation_key(&parent_id.to_string(), &child_id.to_string());
        let assignment: Option<TaskAssignmentRelation> =
            document.load_entry(&TASK_ASSIGNMENTS, &key).await?;
        Ok(assignment.map(to_task_assignment))
    }

    async fn find_all(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<TaskAssignment>, RepositoryError> {
        let assignments = self.load_all(project_id).await?;
        Ok(assignments.into_iter().map(to_task_assignment).collect())
    }
}

fn to_task_assignment(relation: TaskAssignmentRelation) -> TaskAssignment {
    TaskAssignment {
        task_id: TaskId::from(relation.task_id),
        user_id: UserId::from(relation.user_id),
        created_at: relation.created_at,
        updated_at: relation.updated_at,
        updated_by: UserId::from(relation.updated_by),
        deleted: relation.deleted,
    }
}
