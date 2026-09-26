//! SubtaskAssignment用Automergeリポジトリ

use crate::infrastructure::collection::{Collection, relation_key};
use crate::infrastructure::document::Document;

use super::super::document_manager::{DocumentManager, DocumentType};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::subtask_assignment::SubTaskAssignment;
use flequit_model::types::id_types::{ProjectId, SubTaskId, UserId};
use flequit_repository::repositories::project_relation_repository_trait::ProjectRelationRepository;
use flequit_repository::repositories::task_projects::subtask_assignment_repository_trait::SubTaskAssignmentRepositoryTrait;
use flequit_types::errors::repository_error::RepositoryError;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SubtaskAssignmentRelation {
    subtask_id: String,
    user_id: String,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
    updated_by: String,
    deleted: bool,
}

/// プロジェクトドキュメント内のサブタスクとユーザーの割り当て（キーは `{subtask_id}:{user_id}`）
pub(crate) const SUBTASK_ASSIGNMENTS: Collection<SubtaskAssignmentRelation> =
    Collection::new("subtask_assignments", |relation| {
        relation_key(&relation.subtask_id, &relation.user_id)
    });

#[derive(Debug)]
pub struct SubtaskAssignmentLocalAutomergeRepository {
    document_manager: Arc<Mutex<DocumentManager>>,
}

impl SubtaskAssignmentLocalAutomergeRepository {
    /// 新しいSubtaskAssignmentRepositoryを作成
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

    /// 指定サブタスクのユーザーIDリストを取得
    pub async fn find_user_ids_by_subtask_id(
        &self,
        project_id: &ProjectId,
        subtask_id: &SubTaskId,
    ) -> Result<Vec<UserId>, RepositoryError> {
        let subtask_id = subtask_id.to_string();
        let assignments = self.load_all(project_id).await?;
        Ok(assignments
            .into_iter()
            .filter(|a| a.subtask_id == subtask_id)
            .map(|a| UserId::from(a.user_id))
            .collect())
    }

    /// 指定ユーザーに関連するサブタスクIDリストを取得
    pub async fn find_subtask_ids_by_user_id(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
    ) -> Result<Vec<SubTaskId>, RepositoryError> {
        let user_id = user_id.to_string();
        let assignments = self.load_all(project_id).await?;
        Ok(assignments
            .into_iter()
            .filter(|a| a.user_id == user_id)
            .map(|a| SubTaskId::from(a.subtask_id))
            .collect())
    }

    /// サブタスクとユーザーの割り当てを追加（既にあれば何もしない）
    pub async fn add_assignment(
        &self,
        project_id: &ProjectId,
        subtask_id: &SubTaskId,
        user_id: &UserId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let key = relation_key(&subtask_id.to_string(), &user_id.to_string());
        let existing: Option<SubtaskAssignmentRelation> =
            document.load_entry(&SUBTASK_ASSIGNMENTS, &key).await?;
        if existing.is_none() {
            document
                .put_entry(&SUBTASK_ASSIGNMENTS, &new_assignment(subtask_id, user_id))
                .await?;
        }
        Ok(())
    }

    /// サブタスクとユーザーの割り当てを削除
    pub async fn remove_assignment(
        &self,
        project_id: &ProjectId,
        subtask_id: &SubTaskId,
        user_id: &UserId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let key = relation_key(&subtask_id.to_string(), &user_id.to_string());
        document.delete_entry(&SUBTASK_ASSIGNMENTS, &key).await?;
        Ok(())
    }

    /// 指定サブタスクの全ての割り当てを削除
    pub async fn remove_all_assignments_by_subtask_id(
        &self,
        project_id: &ProjectId,
        subtask_id: &SubTaskId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let subtask_id = subtask_id.to_string();
        document
            .delete_entries_where(&SUBTASK_ASSIGNMENTS, |a| a.subtask_id == subtask_id)
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
            .delete_entries_where(&SUBTASK_ASSIGNMENTS, |a| a.user_id == user_id)
            .await?;
        Ok(())
    }

    /// サブタスクのユーザー割り当てを `user_ids` にする
    ///
    /// 外れたユーザーの割り当てだけを消し、新しいユーザーの割り当てだけを追加する。
    /// 続けて割り当てられているユーザーの割り当ては触らない。
    pub async fn update_subtask_assignments(
        &self,
        project_id: &ProjectId,
        subtask_id: &SubTaskId,
        user_ids: &[UserId],
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let subtask_id_str = subtask_id.to_string();
        let wanted: Vec<String> = user_ids.iter().map(ToString::to_string).collect();
        let current: Vec<String> = self
            .load_all(project_id)
            .await?
            .into_iter()
            .filter(|a| a.subtask_id == subtask_id_str)
            .map(|a| a.user_id)
            .collect();

        let removed: Vec<String> = current
            .iter()
            .filter(|user_id| !wanted.contains(user_id))
            .map(|user_id| relation_key(&subtask_id_str, user_id))
            .collect();
        document
            .delete_entries(&SUBTASK_ASSIGNMENTS, &removed)
            .await?;

        let added: Vec<SubtaskAssignmentRelation> = user_ids
            .iter()
            .filter(|user_id| !current.contains(&user_id.to_string()))
            .map(|user_id| new_assignment(subtask_id, user_id))
            .collect();
        document.put_entries(&SUBTASK_ASSIGNMENTS, &added).await?;
        Ok(())
    }

    async fn load_all(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<SubtaskAssignmentRelation>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.load_collection(&SUBTASK_ASSIGNMENTS).await?)
    }
}

fn new_assignment(subtask_id: &SubTaskId, user_id: &UserId) -> SubtaskAssignmentRelation {
    SubtaskAssignmentRelation {
        subtask_id: subtask_id.to_string(),
        user_id: user_id.to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        updated_by: user_id.to_string(),
        deleted: false,
    }
}

// SubTaskAssignmentRepositoryTrait の実装
impl SubTaskAssignmentRepositoryTrait for SubtaskAssignmentLocalAutomergeRepository {}

#[async_trait]
impl ProjectRelationRepository<SubTaskAssignment, SubTaskId, UserId>
    for SubtaskAssignmentLocalAutomergeRepository
{
    async fn add(
        &self,
        project_id: &ProjectId,
        parent_id: &SubTaskId,
        child_id: &UserId,
        _user_id: &UserId,
        _timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.add_assignment(project_id, parent_id, child_id).await
    }

    async fn remove(
        &self,
        project_id: &ProjectId,
        parent_id: &SubTaskId,
        child_id: &UserId,
    ) -> Result<(), RepositoryError> {
        self.remove_assignment(project_id, parent_id, child_id)
            .await
    }

    async fn remove_all(
        &self,
        project_id: &ProjectId,
        parent_id: &SubTaskId,
    ) -> Result<(), RepositoryError> {
        self.remove_all_assignments_by_subtask_id(project_id, parent_id)
            .await
    }

    async fn find_relations(
        &self,
        project_id: &ProjectId,
        parent_id: &SubTaskId,
    ) -> Result<Vec<SubTaskAssignment>, RepositoryError> {
        let user_ids = self
            .find_user_ids_by_subtask_id(project_id, parent_id)
            .await?;
        let mut relations = Vec::new();

        // 各ユーザーIDに対して SubTaskAssignment を作成
        for user_id in user_ids {
            let assignment = SubTaskAssignment {
                subtask_id: *parent_id,
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
        parent_id: &SubTaskId,
    ) -> Result<bool, RepositoryError> {
        let user_ids = self
            .find_user_ids_by_subtask_id(project_id, parent_id)
            .await?;
        Ok(!user_ids.is_empty())
    }

    async fn count(
        &self,
        project_id: &ProjectId,
        parent_id: &SubTaskId,
    ) -> Result<u64, RepositoryError> {
        let user_ids = self
            .find_user_ids_by_subtask_id(project_id, parent_id)
            .await?;
        Ok(user_ids.len() as u64)
    }

    async fn find_relation(
        &self,
        project_id: &ProjectId,
        parent_id: &SubTaskId,
        child_id: &UserId,
    ) -> Result<Option<SubTaskAssignment>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let key = relation_key(&parent_id.to_string(), &child_id.to_string());
        let assignment: Option<SubtaskAssignmentRelation> =
            document.load_entry(&SUBTASK_ASSIGNMENTS, &key).await?;
        Ok(assignment.map(to_subtask_assignment))
    }

    async fn find_all(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<SubTaskAssignment>, RepositoryError> {
        let assignments = self.load_all(project_id).await?;
        Ok(assignments.into_iter().map(to_subtask_assignment).collect())
    }
}

fn to_subtask_assignment(relation: SubtaskAssignmentRelation) -> SubTaskAssignment {
    SubTaskAssignment {
        subtask_id: SubTaskId::from(relation.subtask_id),
        user_id: UserId::from(relation.user_id),
        created_at: relation.created_at,
        updated_at: relation.updated_at,
        updated_by: UserId::from(relation.updated_by),
        deleted: relation.deleted,
    }
}
