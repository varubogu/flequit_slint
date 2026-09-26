use crate::infrastructure::collection::Collection;
use crate::infrastructure::document::Document;

use super::super::document_manager::{DocumentManager, DocumentType};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::weekday_condition::WeekdayCondition;
use flequit_model::types::id_types::{ProjectId, UserId, WeekdayConditionId};
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_repository::repositories::task_projects::weekday_condition_repository_trait::WeekdayConditionRepositoryTrait;
use flequit_types::errors::repository_error::RepositoryError;
use std::path::PathBuf;
use std::sync::Arc;

/// プロジェクトドキュメント内の曜日条件（キーは曜日条件 ID）
pub(crate) const WEEKDAY_CONDITIONS: Collection<WeekdayCondition> =
    Collection::new("weekday_conditions", |condition| condition.id.to_string());

/// Automerge実装の曜日条件リポジトリ
///
/// `Repository<WeekdayCondition>`と`WeekdayConditionRepositoryTrait`を実装し、
/// Automerge-Repoを使用した曜日条件管理を提供する。
///
/// # アーキテクチャ
///
/// ```text
/// WeekdayConditionLocalAutomergeRepository (このクラス)
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
pub struct WeekdayConditionLocalAutomergeRepository {
    document_manager: Arc<tokio::sync::RwLock<DocumentManager>>,
}

impl WeekdayConditionLocalAutomergeRepository {
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

    /// 指定されたプロジェクトの全曜日条件を取得
    pub async fn list_weekday_conditions(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<WeekdayCondition>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.load_collection(&WEEKDAY_CONDITIONS).await?)
    }

    /// IDで曜日条件を取得
    pub async fn get_weekday_condition(
        &self,
        project_id: &ProjectId,
        condition_id: &str,
    ) -> Result<Option<WeekdayCondition>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document
            .load_entry(&WEEKDAY_CONDITIONS, condition_id)
            .await?)
    }

    /// 曜日条件を作成または更新
    pub async fn set_weekday_condition(
        &self,
        project_id: &ProjectId,
        weekday_condition: &WeekdayCondition,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document
            .put_entry(&WEEKDAY_CONDITIONS, weekday_condition)
            .await?)
    }

    /// 曜日条件を削除
    pub async fn delete_weekday_condition(
        &self,
        project_id: &ProjectId,
        condition_id: &str,
    ) -> Result<bool, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document
            .delete_entry(&WEEKDAY_CONDITIONS, condition_id)
            .await?)
    }
}

// WeekdayConditionRepositoryTraitの実装
#[async_trait]
impl WeekdayConditionRepositoryTrait for WeekdayConditionLocalAutomergeRepository {}

#[async_trait]
impl ProjectRepository<WeekdayCondition, WeekdayConditionId>
    for WeekdayConditionLocalAutomergeRepository
{
    async fn save(
        &self,
        project_id: &ProjectId,
        entity: &WeekdayCondition,
        _user_id: &UserId,
        _timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        tracing::info!(
            "WeekdayConditionLocalAutomergeRepository::save - 開始: {:?}",
            entity.id
        );
        let result = self.set_weekday_condition(project_id, entity).await;
        if result.is_ok() {
            tracing::info!(
                "WeekdayConditionLocalAutomergeRepository::save - 完了: {:?}",
                entity.id
            );
        } else {
            tracing::error!(
                "WeekdayConditionLocalAutomergeRepository::save - エラー: {:?}",
                result
            );
        }
        result
    }

    async fn find_by_id(
        &self,
        project_id: &ProjectId,
        id: &WeekdayConditionId,
    ) -> Result<Option<WeekdayCondition>, RepositoryError> {
        self.get_weekday_condition(project_id, &id.to_string())
            .await
    }

    async fn find_all(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<WeekdayCondition>, RepositoryError> {
        self.list_weekday_conditions(project_id).await
    }

    async fn delete(
        &self,
        project_id: &ProjectId,
        id: &WeekdayConditionId,
    ) -> Result<(), RepositoryError> {
        let deleted = self
            .delete_weekday_condition(project_id, &id.to_string())
            .await?;
        if deleted {
            Ok(())
        } else {
            Err(RepositoryError::NotFound(format!(
                "WeekdayCondition not found: {}",
                id
            )))
        }
    }

    async fn exists(
        &self,
        project_id: &ProjectId,
        id: &WeekdayConditionId,
    ) -> Result<bool, RepositoryError> {
        let found = self.find_by_id(project_id, id).await?;
        Ok(found.is_some())
    }

    async fn count(&self, project_id: &ProjectId) -> Result<u64, RepositoryError> {
        let weekday_conditions = self.find_all(project_id).await?;
        Ok(weekday_conditions.len() as u64)
    }
}
