//! TaskTag用Automergeリポジトリ

use super::super::document_manager::{DocumentManager, DocumentType};
use crate::infrastructure::collection::{Collection, relation_key};
use crate::infrastructure::document::Document;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::task_tag::TaskTag;
use flequit_model::types::id_types::{ProjectId, TagId, TaskId, UserId};
use flequit_repository::repositories::project_relation_repository_trait::ProjectRelationRepository;
use flequit_repository::repositories::task_projects::task_tag_repository_trait::TaskTagRepositoryTrait;
use flequit_types::errors::repository_error::RepositoryError;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TaskTagRelation {
    pub(crate) task_id: String,
    pub(crate) tag_id: String,
    pub(crate) created_at: chrono::DateTime<Utc>,
    pub(crate) updated_at: chrono::DateTime<Utc>,
    pub(crate) deleted: bool,
    pub(crate) updated_by: String,
}

/// プロジェクトドキュメント内のタスクとタグの関連（キーは `{task_id}:{tag_id}`）
pub(crate) const TASK_TAGS: Collection<TaskTagRelation> =
    Collection::new("task_tags", |relation| {
        relation_key(&relation.task_id, &relation.tag_id)
    });

#[derive(Debug)]
pub struct TaskTagLocalAutomergeRepository {
    document_manager: Arc<Mutex<DocumentManager>>,
}

impl TaskTagLocalAutomergeRepository {
    /// 新しいTaskTagRepositoryを作成
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

    /// 指定タスクのタグIDリストを取得
    pub async fn find_tag_ids_by_task_id(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
    ) -> Result<Vec<TagId>, RepositoryError> {
        let task_id = task_id.to_string();
        let relations = self.load_all(project_id).await?;
        Ok(relations
            .into_iter()
            .filter(|r| r.task_id == task_id)
            .map(|r| TagId::from(r.tag_id))
            .collect())
    }

    /// 指定タグに関連するタスクIDリストを取得
    pub async fn find_task_ids_by_tag_id(
        &self,
        project_id: &ProjectId,
        tag_id: &TagId,
    ) -> Result<Vec<TaskId>, RepositoryError> {
        let tag_id = tag_id.to_string();
        let relations = self.load_all(project_id).await?;
        Ok(relations
            .into_iter()
            .filter(|r| r.tag_id == tag_id)
            .map(|r| TaskId::from(r.task_id))
            .collect())
    }

    /// タスクとタグの関連付けを追加（既にあれば何もしない）
    pub async fn add_relation(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
        tag_id: &TagId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let key = relation_key(&task_id.to_string(), &tag_id.to_string());
        let existing: Option<TaskTagRelation> = document.load_entry(&TASK_TAGS, &key).await?;
        if existing.is_none() {
            document
                .put_entry(&TASK_TAGS, &new_relation(task_id, tag_id))
                .await?;
        }
        Ok(())
    }

    /// タスクとタグの関連付けを削除
    pub async fn remove_relation(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
        tag_id: &TagId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let key = relation_key(&task_id.to_string(), &tag_id.to_string());
        document.delete_entry(&TASK_TAGS, &key).await?;
        Ok(())
    }

    /// 指定タスクの全ての関連付けを削除
    pub async fn remove_all_relations_by_task_id(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let task_id = task_id.to_string();
        document
            .delete_entries_where(&TASK_TAGS, |r| r.task_id == task_id)
            .await?;
        Ok(())
    }

    /// 指定タグの全ての関連付けを削除
    pub async fn remove_all_relations_by_tag_id(
        &self,
        project_id: &ProjectId,
        tag_id: &TagId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let tag_id = tag_id.to_string();
        document
            .delete_entries_where(&TASK_TAGS, |r| r.tag_id == tag_id)
            .await?;
        Ok(())
    }

    /// タスクのタグ関連付けを `tag_ids` にする
    ///
    /// 外れたタグの関連だけを消し、新しいタグの関連だけを追加する。続けて付いているタグの関連は触らない。
    pub async fn update_task_tag_relations(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
        tag_ids: &[TagId],
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let task_id_str = task_id.to_string();
        let wanted: Vec<String> = tag_ids.iter().map(ToString::to_string).collect();
        let current: Vec<String> = self
            .load_all(project_id)
            .await?
            .into_iter()
            .filter(|r| r.task_id == task_id_str)
            .map(|r| r.tag_id)
            .collect();

        let removed: Vec<String> = current
            .iter()
            .filter(|tag_id| !wanted.contains(tag_id))
            .map(|tag_id| relation_key(&task_id_str, tag_id))
            .collect();
        document.delete_entries(&TASK_TAGS, &removed).await?;

        let added: Vec<TaskTagRelation> = tag_ids
            .iter()
            .filter(|tag_id| !current.contains(&tag_id.to_string()))
            .map(|tag_id| new_relation(task_id, tag_id))
            .collect();
        document.put_entries(&TASK_TAGS, &added).await?;
        Ok(())
    }

    async fn load_all(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<TaskTagRelation>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.load_collection(&TASK_TAGS).await?)
    }
}

fn new_relation(task_id: &TaskId, tag_id: &TagId) -> TaskTagRelation {
    TaskTagRelation {
        task_id: task_id.to_string(),
        tag_id: tag_id.to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted: false,
        updated_by: UserId::from("system").to_string(),
    }
}

// TaskTagRepositoryTrait の実装
impl TaskTagRepositoryTrait for TaskTagLocalAutomergeRepository {}

#[async_trait]
impl ProjectRelationRepository<TaskTag, TaskId, TagId> for TaskTagLocalAutomergeRepository {
    async fn add(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
        child_id: &TagId,
        _user_id: &UserId,
        _timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.add_relation(project_id, parent_id, child_id).await
    }

    async fn remove(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
        child_id: &TagId,
    ) -> Result<(), RepositoryError> {
        self.remove_relation(project_id, parent_id, child_id).await
    }

    async fn remove_all(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
    ) -> Result<(), RepositoryError> {
        self.remove_all_relations_by_task_id(project_id, parent_id)
            .await
    }

    async fn find_relations(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
    ) -> Result<Vec<TaskTag>, RepositoryError> {
        let tag_ids = self.find_tag_ids_by_task_id(project_id, parent_id).await?;
        let mut relations = Vec::new();

        // 各タグIDに対して TaskTag を作成
        for tag_id in tag_ids {
            let tag_relation = TaskTag {
                task_id: *parent_id,
                tag_id,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                updated_by: UserId::from("system"),
                deleted: false,
            };
            relations.push(tag_relation);
        }

        Ok(relations)
    }

    async fn exists(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
    ) -> Result<bool, RepositoryError> {
        let tag_ids = self.find_tag_ids_by_task_id(project_id, parent_id).await?;
        Ok(!tag_ids.is_empty())
    }

    async fn count(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
    ) -> Result<u64, RepositoryError> {
        let tag_ids = self.find_tag_ids_by_task_id(project_id, parent_id).await?;
        Ok(tag_ids.len() as u64)
    }

    async fn find_relation(
        &self,
        project_id: &ProjectId,
        parent_id: &TaskId,
        child_id: &TagId,
    ) -> Result<Option<TaskTag>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let key = relation_key(&parent_id.to_string(), &child_id.to_string());
        let relation: Option<TaskTagRelation> = document.load_entry(&TASK_TAGS, &key).await?;
        Ok(relation.map(to_task_tag))
    }

    async fn find_all(&self, project_id: &ProjectId) -> Result<Vec<TaskTag>, RepositoryError> {
        let relations = self.load_all(project_id).await?;
        Ok(relations.into_iter().map(to_task_tag).collect())
    }
}

fn to_task_tag(relation: TaskTagRelation) -> TaskTag {
    TaskTag {
        task_id: TaskId::from(relation.task_id),
        tag_id: TagId::from(relation.tag_id),
        created_at: relation.created_at,
        updated_at: relation.updated_at,
        updated_by: UserId::from(relation.updated_by),
        deleted: relation.deleted,
    }
}
