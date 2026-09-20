//! タスクリスト用統合リポジトリ

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tracing::info;

use flequit_infrastructure_automerge::infrastructure::task_projects::task_list::TaskListLocalAutomergeRepository;
use flequit_infrastructure_sqlite::infrastructure::task_projects::task_list::TaskListLocalSqliteRepository;
use flequit_model::models::task_projects::task_list::TaskList;
use flequit_model::types::id_types::{ProjectId, TaskListId, UserId};
use flequit_repository::repositories::project_patchable_trait::ProjectPatchable;
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_repository::repositories::task_projects::task_list_repository_trait::TaskListRepositoryTrait;
use flequit_types::errors::repository_error::RepositoryError;

use crate::automerge_sync::{AutomergeChange, ProjectChange, QueuedSqlite};

#[derive(Debug)]
pub enum TaskListRepositoryVariant {
    LocalSqlite(TaskListLocalSqliteRepository),
    LocalAutomerge(TaskListLocalAutomergeRepository),
}

impl TaskListRepositoryTrait for TaskListRepositoryVariant {}

impl ProjectPatchable<TaskList, TaskListId> for TaskListRepositoryVariant {}

#[async_trait]
impl ProjectRepository<TaskList, TaskListId> for TaskListRepositoryVariant {
    async fn save(
        &self,
        project_id: &ProjectId,
        entity: &TaskList,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        match self {
            Self::LocalSqlite(repo) => repo.save(project_id, entity, user_id, timestamp).await,
            Self::LocalAutomerge(repo) => repo.save(project_id, entity, user_id, timestamp).await,
        }
    }

    async fn find_by_id(
        &self,
        project_id: &ProjectId,
        id: &TaskListId,
    ) -> Result<Option<TaskList>, RepositoryError> {
        match self {
            Self::LocalSqlite(repo) => repo.find_by_id(project_id, id).await,
            Self::LocalAutomerge(repo) => repo.find_by_id(project_id, id).await,
        }
    }

    async fn find_all(&self, project_id: &ProjectId) -> Result<Vec<TaskList>, RepositoryError> {
        match self {
            Self::LocalSqlite(repo) => repo.find_all(project_id).await,
            Self::LocalAutomerge(repo) => repo.find_all(project_id).await,
        }
    }

    async fn delete(&self, project_id: &ProjectId, id: &TaskListId) -> Result<(), RepositoryError> {
        match self {
            Self::LocalSqlite(repo) => repo.delete(project_id, id).await,
            Self::LocalAutomerge(repo) => repo.delete(project_id, id).await,
        }
    }

    async fn exists(
        &self,
        project_id: &ProjectId,
        id: &TaskListId,
    ) -> Result<bool, RepositoryError> {
        match self {
            Self::LocalSqlite(repo) => repo.exists(project_id, id).await,
            Self::LocalAutomerge(repo) => repo.exists(project_id, id).await,
        }
    }

    async fn count(&self, project_id: &ProjectId) -> Result<u64, RepositoryError> {
        match self {
            Self::LocalSqlite(repo) => repo.count(project_id).await,
            Self::LocalAutomerge(repo) => repo.count(project_id).await,
        }
    }
}

#[derive(Debug)]
pub struct TaskListUnifiedRepository {
    save_repositories: Vec<TaskListRepositoryVariant>,
    search_repositories: Vec<TaskListRepositoryVariant>,
    /// SQLite への書き込み先と Automerge 同期キュー。SQLite ストレージが有効なときにある
    queued_sqlite: Option<QueuedSqlite<TaskListLocalSqliteRepository>>,
}

impl Default for TaskListUnifiedRepository {
    fn default() -> Self {
        Self::new(vec![], vec![])
    }
}

impl TaskListUnifiedRepository {
    pub fn new(
        save_repositories: Vec<TaskListRepositoryVariant>,
        search_repositories: Vec<TaskListRepositoryVariant>,
    ) -> Self {
        Self {
            save_repositories,
            search_repositories,
            queued_sqlite: None,
        }
    }

    pub fn add_sqlite_for_save(&mut self, sqlite_repo: TaskListLocalSqliteRepository) {
        self.save_repositories
            .push(TaskListRepositoryVariant::LocalSqlite(sqlite_repo));
    }

    /// 書き込みを「Automerge 同期キューへの登録 + SQLite への書き込み」の
    /// 1 トランザクションにする。Automerge へはワーカーが後から反映する
    pub fn set_queued_sqlite(&mut self, queued: QueuedSqlite<TaskListLocalSqliteRepository>) {
        self.queued_sqlite = Some(queued);
    }

    pub fn add_automerge_for_save(&mut self, automerge_repo: TaskListLocalAutomergeRepository) {
        self.save_repositories
            .push(TaskListRepositoryVariant::LocalAutomerge(automerge_repo));
    }

    pub fn add_sqlite_for_search(&mut self, sqlite_repo: TaskListLocalSqliteRepository) {
        self.search_repositories
            .push(TaskListRepositoryVariant::LocalSqlite(sqlite_repo));
    }

    pub fn add_automerge_for_search(&mut self, automerge_repo: TaskListLocalAutomergeRepository) {
        self.search_repositories
            .push(TaskListRepositoryVariant::LocalAutomerge(automerge_repo));
    }

    pub fn add_web_for_save(&mut self, _web_repo: impl std::fmt::Debug + Send + Sync + 'static) {
        // 将来のWeb実装用の拡張ポイント
        // self.save_repositories.push(TaskListRepositoryVariant::Web(web_repo));
    }

    pub fn add_web_for_search(&mut self, _web_repo: impl std::fmt::Debug + Send + Sync + 'static) {
        // 将来のWeb実装用の拡張ポイント
        // self.search_repositories.push(TaskListRepositoryVariant::Web(web_repo));
    }

    /// 保存用リポジトリの数を取得
    pub fn save_repositories_count(&self) -> usize {
        self.save_repositories.len() + usize::from(self.queued_sqlite.is_some())
    }

    /// 検索用リポジトリの数を取得
    pub fn search_repositories_count(&self) -> usize {
        self.search_repositories.len()
    }
}

impl TaskListRepositoryTrait for TaskListUnifiedRepository {}

impl ProjectPatchable<TaskList, TaskListId> for TaskListUnifiedRepository {}

#[async_trait]
impl ProjectRepository<TaskList, TaskListId> for TaskListUnifiedRepository {
    async fn save(
        &self,
        project_id: &ProjectId,
        entity: &TaskList,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        info!(
            "Saving task list entity with ID: {} in project: {}",
            entity.id, project_id
        );
        if let Some(queued) = &self.queued_sqlite {
            let txn = queued
                .queue
                .begin(vec![AutomergeChange::TaskList(ProjectChange::Save {
                    project_id: *project_id,
                    entity: entity.clone(),
                    user_id: *user_id,
                    timestamp: *timestamp,
                })])
                .await?;
            let result = queued
                .sqlite
                .save_with_txn(txn.txn(), project_id, entity, user_id, timestamp)
                .await;
            return txn.finish(result).await;
        }

        for repository in &self.save_repositories {
            repository
                .save(project_id, entity, user_id, timestamp)
                .await?;
        }

        Ok(())
    }

    async fn find_by_id(
        &self,
        project_id: &ProjectId,
        id: &TaskListId,
    ) -> Result<Option<TaskList>, RepositoryError> {
        info!("Finding task list by ID: {} in project: {}", id, project_id);

        for repository in &self.search_repositories {
            if let Some(entity) = repository.find_by_id(project_id, id).await? {
                return Ok(Some(entity));
            }
        }

        Ok(None)
    }

    async fn find_all(&self, project_id: &ProjectId) -> Result<Vec<TaskList>, RepositoryError> {
        info!("Finding all task lists in project: {}", project_id);

        if let Some(repository) = self.search_repositories.first() {
            repository.find_all(project_id).await
        } else {
            Ok(vec![])
        }
    }

    async fn delete(&self, project_id: &ProjectId, id: &TaskListId) -> Result<(), RepositoryError> {
        info!(
            "Deleting task list with ID: {} in project: {}",
            id, project_id
        );
        if let Some(queued) = &self.queued_sqlite {
            let txn = queued
                .queue
                .begin(vec![AutomergeChange::TaskList(ProjectChange::Delete {
                    project_id: *project_id,
                    id: *id,
                })])
                .await?;
            let result = queued
                .sqlite
                .delete_with_txn(txn.txn(), project_id, id)
                .await;
            return txn.finish(result).await;
        }

        for repository in &self.save_repositories {
            repository.delete(project_id, id).await?;
        }

        Ok(())
    }

    async fn exists(
        &self,
        project_id: &ProjectId,
        id: &TaskListId,
    ) -> Result<bool, RepositoryError> {
        info!(
            "Checking if task list exists with ID: {} in project: {}",
            id, project_id
        );

        for repository in &self.search_repositories {
            if repository.exists(project_id, id).await? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    async fn count(&self, project_id: &ProjectId) -> Result<u64, RepositoryError> {
        info!("Counting task lists in project: {}", project_id);

        if let Some(repository) = self.search_repositories.first() {
            repository.count(project_id).await
        } else {
            Ok(0)
        }
    }
}
