use crate::infrastructure::collection::Collection;
use crate::infrastructure::document::Document;

use super::super::document_manager::{DocumentManager, DocumentType};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::member::Member;
use flequit_model::types::id_types::{ProjectId, UserId};
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_repository::repositories::task_projects::member_repository_trait::MemberRepositoryTrait;
use flequit_types::errors::repository_error::RepositoryError;
use std::path::PathBuf;
use std::sync::Arc;

/// プロジェクトドキュメント内のメンバー（キーはユーザー ID。1 ユーザーにつき 1 件）
pub(crate) const MEMBERS: Collection<Member> =
    Collection::new("members", |member| member.user_id.to_string());

/// Automerge実装のメンバーリポジトリ
///
/// `Repository<Member>`と`MemberRepositoryTrait`を実装し、
/// Automerge-Repoを使用したメンバー管理を提供する。
///
/// # アーキテクチャ
///
/// ```text
/// MemberLocalAutomergeRepository (このクラス)
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
pub struct MemberLocalAutomergeRepository {
    document_manager: Arc<tokio::sync::RwLock<DocumentManager>>,
}

impl MemberLocalAutomergeRepository {
    pub async fn new(base_path: PathBuf) -> Result<Self, RepositoryError> {
        let document_manager = DocumentManager::new(base_path)?;
        Ok(Self {
            document_manager: Arc::new(tokio::sync::RwLock::new(document_manager)),
        })
    }

    /// 指定されたプロジェクトのDocumentを取得または作成
    async fn get_or_create_document(
        &self,
        project_id: &ProjectId,
    ) -> Result<Document, RepositoryError> {
        let doc_type = DocumentType::Project(*project_id);
        let mut manager = self.document_manager.write().await;
        manager
            .get_or_create(&doc_type)
            .await
            .map_err(|e| RepositoryError::AutomergeError(e.to_string()))
    }

    /// 指定されたプロジェクトの全メンバーを取得
    pub async fn list_members(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<Member>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.load_collection(&MEMBERS).await?)
    }

    /// ユーザーIDでメンバーを取得
    pub async fn get_member(
        &self,
        project_id: &ProjectId,
        user_id: &str,
    ) -> Result<Option<Member>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.load_entry(&MEMBERS, user_id).await?)
    }

    /// メンバーを作成または更新
    pub async fn set_member(
        &self,
        project_id: &ProjectId,
        member: &Member,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.put_entry(&MEMBERS, member).await?)
    }

    /// ユーザーIDでメンバーを削除
    pub async fn delete_member(
        &self,
        project_id: &ProjectId,
        user_id: &str,
    ) -> Result<bool, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.delete_entry(&MEMBERS, user_id).await?)
    }
}

// MemberRepositoryTraitの実装
#[async_trait]
impl MemberRepositoryTrait for MemberLocalAutomergeRepository {}

#[async_trait]
impl ProjectRepository<Member, UserId> for MemberLocalAutomergeRepository {
    async fn save(
        &self,
        project_id: &ProjectId,
        entity: &Member,
        _user_id: &UserId,
        _timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        tracing::info!(
            "MemberLocalAutomergeRepository::save - 開始: {:?}",
            entity.id
        );
        let result = self.set_member(project_id, entity).await;
        if result.is_ok() {
            tracing::info!(
                "MemberLocalAutomergeRepository::save - 完了: {:?}",
                entity.id
            );
        } else {
            tracing::error!(
                "MemberLocalAutomergeRepository::save - エラー: {:?}",
                result
            );
        }
        result
    }

    async fn find_by_id(
        &self,
        project_id: &ProjectId,
        id: &UserId,
    ) -> Result<Option<Member>, RepositoryError> {
        self.get_member(project_id, &id.to_string()).await
    }

    async fn find_all(&self, project_id: &ProjectId) -> Result<Vec<Member>, RepositoryError> {
        self.list_members(project_id).await
    }

    async fn delete(&self, project_id: &ProjectId, id: &UserId) -> Result<(), RepositoryError> {
        let deleted = self.delete_member(project_id, &id.to_string()).await?;
        if deleted {
            Ok(())
        } else {
            Err(RepositoryError::NotFound(format!(
                "Member not found: {}",
                id
            )))
        }
    }

    async fn exists(&self, project_id: &ProjectId, id: &UserId) -> Result<bool, RepositoryError> {
        let found = self.find_by_id(project_id, id).await?;
        Ok(found.is_some())
    }

    async fn count(&self, project_id: &ProjectId) -> Result<u64, RepositoryError> {
        let members = self.find_all(project_id).await?;
        Ok(members.len() as u64)
    }
}
