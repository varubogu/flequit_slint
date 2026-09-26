use crate::infrastructure::collection::Collection;
use crate::infrastructure::document::Document;

use super::super::document_manager::{DocumentManager, DocumentType};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::tag::Tag;
use flequit_model::traits::Trackable;
use flequit_model::types::id_types::{ProjectId, TagId, UserId};
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_repository::repositories::task_projects::tag_repository_trait::TagRepositoryTrait;
use flequit_types::errors::repository_error::RepositoryError;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// プロジェクトドキュメント内のタグ（キーはタグ ID）
pub(crate) const TAGS: Collection<Tag> = Collection::new("tags", |tag| tag.id.to_string());

/// Automerge実装のタグリポジトリ
///
/// `Repository<Tag>`と`TagRepositoryTrait`を実装し、
/// Automerge-Repoを使用したタグ管理を提供する。
///
/// # アーキテクチャ
///
/// ```text
/// TagLocalAutomergeRepository (このクラス)
///   | 委譲
/// DocumentManager (プロジェクトごとのドキュメント管理)
///   | データアクセス
/// Automerge Documents
/// ```
///
/// # 特徴
///
/// - **分散同期**: CRDTによる競合解決機能
/// - **履歴管理**: すべての変更履歴を保持
/// - **オフライン対応**: ローカル優先で同期可能
/// - **JSON互換**: 構造化データの効率的な管理
/// - **ステートレス**: プロジェクトIDは必要時に引数で受け取る
#[derive(Debug)]
pub struct TagLocalAutomergeRepository {
    document_manager: Arc<Mutex<DocumentManager>>,
}

impl TagLocalAutomergeRepository {
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
        let doc_type = DocumentType::Project(*project_id);
        let mut manager = self.document_manager.lock().await;
        manager
            .get_or_create(&doc_type)
            .await
            .map_err(|e| RepositoryError::AutomergeError(e.to_string()))
    }

    /// 指定されたプロジェクトの全タグを取得（削除済みを含む）
    async fn list_all_tags_raw(&self, project_id: &ProjectId) -> Result<Vec<Tag>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.load_collection(&TAGS).await?)
    }

    pub async fn list_tags(&self, project_id: &ProjectId) -> Result<Vec<Tag>, RepositoryError> {
        let tags = self.list_all_tags_raw(project_id).await?;
        Ok(tags.into_iter().filter(|t| !t.is_deleted()).collect())
    }

    /// IDでタグを取得（削除済みは除く）
    pub async fn get_tag(
        &self,
        project_id: &ProjectId,
        tag_id: &str,
    ) -> Result<Option<Tag>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let tag: Option<Tag> = document.load_entry(&TAGS, tag_id).await?;
        Ok(tag.filter(|t| !t.is_deleted()))
    }

    /// タグを作成または更新
    pub async fn set_tag(&self, project_id: &ProjectId, tag: &Tag) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.put_entry(&TAGS, tag).await?)
    }

    /// タグを削除
    pub async fn delete_tag(
        &self,
        project_id: &ProjectId,
        tag_id: &str,
    ) -> Result<bool, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.delete_entry(&TAGS, tag_id).await?)
    }
}

// TagRepositoryTraitの実装
#[async_trait]
impl TagRepositoryTrait for TagLocalAutomergeRepository {}

#[async_trait]
impl ProjectRepository<Tag, TagId> for TagLocalAutomergeRepository {
    async fn save(
        &self,
        project_id: &ProjectId,
        entity: &Tag,
        _user_id: &UserId,
        _timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        tracing::info!("TagLocalAutomergeRepository::save - 開始: {:?}", entity.id);
        let result = self.set_tag(project_id, entity).await;
        if result.is_ok() {
            tracing::info!("TagLocalAutomergeRepository::save - 完了: {:?}", entity.id);
        } else {
            tracing::error!("TagLocalAutomergeRepository::save - エラー: {:?}", result);
        }
        result
    }

    async fn find_by_id(
        &self,
        project_id: &ProjectId,
        id: &TagId,
    ) -> Result<Option<Tag>, RepositoryError> {
        self.get_tag(project_id, &id.to_string()).await
    }

    async fn find_all(&self, project_id: &ProjectId) -> Result<Vec<Tag>, RepositoryError> {
        self.list_tags(project_id).await
    }

    async fn delete(&self, project_id: &ProjectId, id: &TagId) -> Result<(), RepositoryError> {
        let deleted = self.delete_tag(project_id, &id.to_string()).await?;
        if deleted {
            Ok(())
        } else {
            Err(RepositoryError::NotFound(format!("Tag not found: {}", id)))
        }
    }

    async fn exists(&self, project_id: &ProjectId, id: &TagId) -> Result<bool, RepositoryError> {
        let found = self.find_by_id(project_id, id).await?;
        Ok(found.is_some())
    }

    async fn count(&self, project_id: &ProjectId) -> Result<u64, RepositoryError> {
        let tags = self.find_all(project_id).await?;
        Ok(tags.len() as u64)
    }
}
