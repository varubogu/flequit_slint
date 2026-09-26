//! SubtaskTag用Automergeリポジトリ

use crate::infrastructure::collection::{Collection, relation_key};
use crate::infrastructure::document::Document;

use super::super::document_manager::{DocumentManager, DocumentType};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::subtask_tag::SubTaskTag;
use flequit_model::types::id_types::{ProjectId, SubTaskId, TagId, UserId};
use flequit_repository::repositories::project_relation_repository_trait::ProjectRelationRepository;
use flequit_repository::repositories::task_projects::subtask_tag_repository_trait::SubTaskTagRepositoryTrait;
use flequit_types::errors::repository_error::RepositoryError;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SubtaskTagRelation {
    subtask_id: String,
    tag_id: String,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
    updated_by: String,
    deleted: bool,
}

/// プロジェクトドキュメント内のサブタスクとタグの関連（キーは `{subtask_id}:{tag_id}`）
pub(crate) const SUBTASK_TAGS: Collection<SubtaskTagRelation> =
    Collection::new("subtask_tags", |relation| {
        relation_key(&relation.subtask_id, &relation.tag_id)
    });

#[derive(Debug)]
pub struct SubtaskTagLocalAutomergeRepository {
    document_manager: Arc<Mutex<DocumentManager>>,
}

impl SubtaskTagLocalAutomergeRepository {
    /// 新しいSubtaskTagRepositoryを作成
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

    /// 指定サブタスクのタグIDリストを取得
    pub async fn find_tag_ids_by_subtask_id(
        &self,
        project_id: &ProjectId,
        subtask_id: &SubTaskId,
    ) -> Result<Vec<TagId>, RepositoryError> {
        let subtask_id = subtask_id.to_string();
        let relations = self.load_all(project_id).await?;
        Ok(relations
            .into_iter()
            .filter(|r| r.subtask_id == subtask_id)
            .map(|r| TagId::from(r.tag_id))
            .collect())
    }

    /// 指定タグに関連するサブタスクIDリストを取得
    pub async fn find_subtask_ids_by_tag_id(
        &self,
        project_id: &ProjectId,
        tag_id: &TagId,
    ) -> Result<Vec<SubTaskId>, RepositoryError> {
        let tag_id = tag_id.to_string();
        let relations = self.load_all(project_id).await?;
        Ok(relations
            .into_iter()
            .filter(|r| r.tag_id == tag_id)
            .map(|r| SubTaskId::from(r.subtask_id))
            .collect())
    }

    /// サブタスクとタグの関連付けを追加（既にあれば何もしない）
    pub async fn add_relation(
        &self,
        project_id: &ProjectId,
        subtask_id: &SubTaskId,
        tag_id: &TagId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let key = relation_key(&subtask_id.to_string(), &tag_id.to_string());
        let existing: Option<SubtaskTagRelation> = document.load_entry(&SUBTASK_TAGS, &key).await?;
        if existing.is_none() {
            document
                .put_entry(&SUBTASK_TAGS, &new_relation(subtask_id, tag_id))
                .await?;
        }
        Ok(())
    }

    /// サブタスクとタグの関連付けを削除
    pub async fn remove_relation(
        &self,
        project_id: &ProjectId,
        subtask_id: &SubTaskId,
        tag_id: &TagId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let key = relation_key(&subtask_id.to_string(), &tag_id.to_string());
        document.delete_entry(&SUBTASK_TAGS, &key).await?;
        Ok(())
    }

    /// 指定サブタスクの全ての関連付けを削除
    pub async fn remove_all_relations_by_subtask_id(
        &self,
        project_id: &ProjectId,
        subtask_id: &SubTaskId,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let subtask_id = subtask_id.to_string();
        document
            .delete_entries_where(&SUBTASK_TAGS, |r| r.subtask_id == subtask_id)
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
            .delete_entries_where(&SUBTASK_TAGS, |r| r.tag_id == tag_id)
            .await?;
        Ok(())
    }

    /// サブタスクのタグ関連付けを `tag_ids` にする
    ///
    /// 外れたタグの関連だけを消し、新しいタグの関連だけを追加する。続けて付いているタグの関連は触らない。
    pub async fn update_subtask_tag_relations(
        &self,
        project_id: &ProjectId,
        subtask_id: &SubTaskId,
        tag_ids: &[TagId],
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let subtask_id_str = subtask_id.to_string();
        let wanted: Vec<String> = tag_ids.iter().map(ToString::to_string).collect();
        let current: Vec<String> = self
            .load_all(project_id)
            .await?
            .into_iter()
            .filter(|r| r.subtask_id == subtask_id_str)
            .map(|r| r.tag_id)
            .collect();

        let removed: Vec<String> = current
            .iter()
            .filter(|tag_id| !wanted.contains(tag_id))
            .map(|tag_id| relation_key(&subtask_id_str, tag_id))
            .collect();
        document.delete_entries(&SUBTASK_TAGS, &removed).await?;

        let added: Vec<SubtaskTagRelation> = tag_ids
            .iter()
            .filter(|tag_id| !current.contains(&tag_id.to_string()))
            .map(|tag_id| new_relation(subtask_id, tag_id))
            .collect();
        document.put_entries(&SUBTASK_TAGS, &added).await?;
        Ok(())
    }

    async fn load_all(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<SubtaskTagRelation>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.load_collection(&SUBTASK_TAGS).await?)
    }
}

fn new_relation(subtask_id: &SubTaskId, tag_id: &TagId) -> SubtaskTagRelation {
    SubtaskTagRelation {
        subtask_id: subtask_id.to_string(),
        tag_id: tag_id.to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted: false,
        updated_by: UserId::from("system").to_string(),
    }
}

// SubTaskTagRepositoryTrait の実装
impl SubTaskTagRepositoryTrait for SubtaskTagLocalAutomergeRepository {}

#[async_trait]
impl ProjectRelationRepository<SubTaskTag, SubTaskId, TagId>
    for SubtaskTagLocalAutomergeRepository
{
    async fn add(
        &self,
        project_id: &ProjectId,
        parent_id: &SubTaskId,
        child_id: &TagId,
        _user_id: &UserId,
        _timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.add_relation(project_id, parent_id, child_id).await
    }

    async fn remove(
        &self,
        project_id: &ProjectId,
        parent_id: &SubTaskId,
        child_id: &TagId,
    ) -> Result<(), RepositoryError> {
        self.remove_relation(project_id, parent_id, child_id).await
    }

    async fn remove_all(
        &self,
        project_id: &ProjectId,
        parent_id: &SubTaskId,
    ) -> Result<(), RepositoryError> {
        self.remove_all_relations_by_subtask_id(project_id, parent_id)
            .await
    }

    async fn find_relations(
        &self,
        project_id: &ProjectId,
        parent_id: &SubTaskId,
    ) -> Result<Vec<SubTaskTag>, RepositoryError> {
        let tag_ids = self
            .find_tag_ids_by_subtask_id(project_id, parent_id)
            .await?;
        let mut relations = Vec::new();

        // 各タグIDに対して SubTaskTag を作成
        for tag_id in tag_ids {
            let tag_relation = SubTaskTag {
                subtask_id: *parent_id,
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
        parent_id: &SubTaskId,
    ) -> Result<bool, RepositoryError> {
        let tag_ids = self
            .find_tag_ids_by_subtask_id(project_id, parent_id)
            .await?;
        Ok(!tag_ids.is_empty())
    }

    async fn count(
        &self,
        project_id: &ProjectId,
        parent_id: &SubTaskId,
    ) -> Result<u64, RepositoryError> {
        let tag_ids = self
            .find_tag_ids_by_subtask_id(project_id, parent_id)
            .await?;
        Ok(tag_ids.len() as u64)
    }

    async fn find_relation(
        &self,
        project_id: &ProjectId,
        parent_id: &SubTaskId,
        child_id: &TagId,
    ) -> Result<Option<SubTaskTag>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let key = relation_key(&parent_id.to_string(), &child_id.to_string());
        let relation: Option<SubtaskTagRelation> = document.load_entry(&SUBTASK_TAGS, &key).await?;
        Ok(relation.map(to_subtask_tag))
    }

    async fn find_all(&self, project_id: &ProjectId) -> Result<Vec<SubTaskTag>, RepositoryError> {
        let relations = self.load_all(project_id).await?;
        Ok(relations.into_iter().map(to_subtask_tag).collect())
    }
}

fn to_subtask_tag(relation: SubtaskTagRelation) -> SubTaskTag {
    SubTaskTag {
        subtask_id: SubTaskId::from(relation.subtask_id),
        tag_id: TagId::from(relation.tag_id),
        created_at: relation.created_at,
        updated_at: relation.updated_at,
        updated_by: UserId::from(relation.updated_by),
        deleted: relation.deleted,
    }
}
