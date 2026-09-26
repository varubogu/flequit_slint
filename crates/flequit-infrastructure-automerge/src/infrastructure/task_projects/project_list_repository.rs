//! プロジェクト一覧管理用Automergeリポジトリ

use super::super::document_manager::{DocumentManager, DocumentType};
use crate::infrastructure::collection::Collection;
use crate::infrastructure::document::Document;
use flequit_model::models::task_projects::project::Project;
use flequit_model::traits::Trackable;
use flequit_model::types::id_types::ProjectId;
use flequit_types::errors::repository_error::RepositoryError;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Settings ドキュメント内のプロジェクト一覧（キーはプロジェクト ID）
pub(crate) const PROJECTS: Collection<Project> =
    Collection::new("projects", |project| project.id.to_string());

/// プロジェクト一覧管理のためのAutomergeリポジトリ
///
/// Settings文書を使用してプロジェクト一覧を管理する。
/// 個別プロジェクトの詳細は別途ProjectLocalAutomergeRepositoryで管理される。
#[derive(Debug)]
pub struct ProjectListLocalAutomergeRepository {
    document_manager: Arc<RwLock<DocumentManager>>,
}

impl ProjectListLocalAutomergeRepository {
    /// 新しいProjectListRepositoryを作成
    pub async fn new(base_path: PathBuf) -> Result<Self, RepositoryError> {
        let document_manager = DocumentManager::new(base_path)?;
        Ok(Self {
            document_manager: Arc::new(RwLock::new(document_manager)),
        })
    }

    /// Settings文書を取得または作成（プロジェクト一覧管理用）
    async fn get_or_create_settings_document(&self) -> Result<Document, RepositoryError> {
        let doc_type = DocumentType::Settings;
        let mut manager = self.document_manager.write().await;
        manager
            .get_or_create(&doc_type)
            .await
            .map_err(|e| RepositoryError::AutomergeError(e.to_string()))
    }

    /// 全プロジェクト一覧を取得（削除済み含む、内部処理用）
    async fn list_all_projects_raw(&self) -> Result<Vec<Project>, RepositoryError> {
        let document = self.get_or_create_settings_document().await?;
        Ok(document.load_collection(&PROJECTS).await?)
    }

    /// アクティブなプロジェクト一覧を取得（deleted=falseのみ）
    pub async fn list_projects(&self) -> Result<Vec<Project>, RepositoryError> {
        let projects = self.list_all_projects_raw().await?;
        Ok(projects.into_iter().filter(|p| !p.is_deleted()).collect())
    }

    /// プロジェクトをプロジェクト一覧に追加または更新
    pub async fn add_or_update_project(&self, project: &Project) -> Result<(), RepositoryError> {
        let document = self.get_or_create_settings_document().await?;
        Ok(document.put_entry(&PROJECTS, project).await?)
    }

    /// IDでプロジェクトを取得（一覧から、削除済みは除く）
    pub async fn get_project_from_list(
        &self,
        project_id: &str,
    ) -> Result<Option<Project>, RepositoryError> {
        let document = self.get_or_create_settings_document().await?;
        let project: Option<Project> = document.load_entry(&PROJECTS, project_id).await?;
        Ok(project.filter(|p| !p.is_deleted()))
    }

    /// プロジェクトをプロジェクト一覧から削除
    pub async fn remove_project_from_list(
        &self,
        project_id: &str,
    ) -> Result<bool, RepositoryError> {
        let document = self.get_or_create_settings_document().await?;
        Ok(document.delete_entry(&PROJECTS, project_id).await?)
    }

    /// プロジェクト数を取得
    pub async fn count_projects(&self) -> Result<u64, RepositoryError> {
        let projects = self.list_projects().await?;
        Ok(projects.len() as u64)
    }

    /// プロジェクトが一覧に存在するかチェック
    pub async fn project_exists_in_list(
        &self,
        project_id: &ProjectId,
    ) -> Result<bool, RepositoryError> {
        let found = self.get_project_from_list(&project_id.to_string()).await?;
        Ok(found.is_some())
    }

    /// 削除済みプロジェクト一覧を取得（deleted=trueのみ）
    pub async fn list_deleted_projects(&self) -> Result<Vec<Project>, RepositoryError> {
        let projects = self.list_all_projects_raw().await?;
        Ok(projects.into_iter().filter(|p| p.is_deleted()).collect())
    }
}
