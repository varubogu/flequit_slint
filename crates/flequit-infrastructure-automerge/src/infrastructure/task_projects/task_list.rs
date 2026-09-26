use super::super::document_manager::{DocumentManager, DocumentType};
use crate::infrastructure::collection::Collection;
use crate::infrastructure::document::Document;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::task_list::TaskList;
use flequit_model::traits::Trackable;
use flequit_model::types::id_types::{ProjectId, TaskListId, UserId};
use flequit_repository::repositories::project_patchable_trait::ProjectPatchable;
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_repository::repositories::task_projects::task_list_repository_trait::TaskListRepositoryTrait;
use flequit_types::errors::repository_error::RepositoryError;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// プロジェクトドキュメント内のタスクリスト（キーはタスクリスト ID）
pub(crate) const TASK_LISTS: Collection<TaskList> =
    Collection::new("task_lists", |task_list| task_list.id.to_string());

/// TaskList用のAutomerge-Repoリポジトリ
///
/// ステートレス設計でDocumentManagerを保持し、project_idは動的取得する
#[derive(Debug)]
pub struct TaskListLocalAutomergeRepository {
    document_manager: Arc<Mutex<DocumentManager>>,
}

impl TaskListLocalAutomergeRepository {
    /// 新しいTaskListRepositoryを作成
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

    /// 指定されたプロジェクトの全タスクリストを取得（削除済みを含む）
    async fn list_all_task_lists_raw(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<TaskList>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.load_collection(&TASK_LISTS).await?)
    }

    pub async fn list_task_lists(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<TaskList>, RepositoryError> {
        let task_lists = self.list_all_task_lists_raw(project_id).await?;
        Ok(task_lists
            .into_iter()
            .filter(|tl| !tl.is_deleted())
            .collect())
    }

    /// IDでタスクリストを取得（削除済みは除く）
    pub async fn get_task_list(
        &self,
        project_id: &ProjectId,
        task_list_id: &str,
    ) -> Result<Option<TaskList>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let task_list: Option<TaskList> = document.load_entry(&TASK_LISTS, task_list_id).await?;
        Ok(task_list.filter(|tl| !tl.is_deleted()))
    }

    /// タスクリストを作成または更新
    pub async fn set_task_list(
        &self,
        project_id: &ProjectId,
        task_list: &TaskList,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.put_entry(&TASK_LISTS, task_list).await?)
    }

    /// タスクリストを削除
    pub async fn delete_task_list(
        &self,
        project_id: &ProjectId,
        task_list_id: &str,
    ) -> Result<bool, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        Ok(document.delete_entry(&TASK_LISTS, task_list_id).await?)
    }
}

// Repository<TaskList, TaskListId> トレイトの実装
impl TaskListRepositoryTrait for TaskListLocalAutomergeRepository {}

impl ProjectPatchable<TaskList, TaskListId> for TaskListLocalAutomergeRepository {}

#[async_trait]
impl ProjectRepository<TaskList, TaskListId> for TaskListLocalAutomergeRepository {
    async fn save(
        &self,
        project_id: &ProjectId,
        entity: &TaskList,
        _user_id: &UserId,
        _timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.set_task_list(project_id, entity).await
    }

    async fn find_by_id(
        &self,
        project_id: &ProjectId,
        id: &TaskListId,
    ) -> Result<Option<TaskList>, RepositoryError> {
        self.get_task_list(project_id, &id.to_string()).await
    }

    async fn find_all(&self, project_id: &ProjectId) -> Result<Vec<TaskList>, RepositoryError> {
        self.list_task_lists(project_id).await
    }

    async fn delete(&self, project_id: &ProjectId, id: &TaskListId) -> Result<(), RepositoryError> {
        // 削除操作は冪等性を保証するため、存在しない場合でも成功とする
        self.delete_task_list(project_id, &id.to_string()).await?;
        Ok(())
    }

    async fn exists(
        &self,
        project_id: &ProjectId,
        id: &TaskListId,
    ) -> Result<bool, RepositoryError> {
        let found = self.find_by_id(project_id, id).await?;
        Ok(found.is_some())
    }

    async fn count(&self, project_id: &ProjectId) -> Result<u64, RepositoryError> {
        let task_lists = self.find_all(project_id).await?;
        Ok(task_lists.len() as u64)
    }
}

impl TaskListLocalAutomergeRepository {
    /// Automergeドキュメントの変更履歴を段階的にJSONで出力
    pub async fn export_task_list_changes_history<P: AsRef<std::path::Path>>(
        &self,
        project_id: &ProjectId,
        output_dir: P,
        description: Option<&str>,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        document
            .export_document_changes_history(&output_dir, description)
            .await
            .map_err(|e| RepositoryError::Export(e.to_string()))
    }

    /// JSON出力機能：現在のタスクリスト状態をファイルにエクスポート
    pub async fn export_task_list_state<P: AsRef<std::path::Path>>(
        &self,
        project_id: &ProjectId,
        output_path: P,
        description: Option<&str>,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        document
            .export_json(&output_path, description)
            .await
            .map_err(|e| RepositoryError::Export(e.to_string()))
    }
}
