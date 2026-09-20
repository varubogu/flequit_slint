//! 繰り返しルール用統合リポジトリ

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tracing::info;

// RecurrenceRuleリポジトリ実装をインポート
use flequit_infrastructure_automerge::infrastructure::task_projects::recurrence_rule::RecurrenceRuleLocalAutomergeRepository;
use flequit_infrastructure_sqlite::infrastructure::task_projects::recurrence_rule::RecurrenceRuleLocalSqliteRepository;
use flequit_model::{
    models::task_projects::recurrence_rule::RecurrenceRule,
    types::id_types::{ProjectId, RecurrenceRuleId, UserId},
};
use flequit_repository::project_repository_trait::ProjectRepository;
use flequit_repository::repositories::project_patchable_trait::ProjectPatchable;
use flequit_repository::repositories::task_projects::recurrence_rule_repository_trait::RecurrenceRuleRepositoryTrait;
use flequit_types::errors::repository_error::RepositoryError;

use crate::automerge_sync::{AutomergeChange, ProjectChange, QueuedSqlite};

#[derive(Debug)]
pub enum RecurrenceRuleRepositoryVariant {
    LocalSqlite(RecurrenceRuleLocalSqliteRepository),
    LocalAutomerge(RecurrenceRuleLocalAutomergeRepository),
}

impl RecurrenceRuleRepositoryTrait for RecurrenceRuleRepositoryVariant {}

impl ProjectPatchable<RecurrenceRule, RecurrenceRuleId> for RecurrenceRuleRepositoryVariant {}

#[async_trait]
impl ProjectRepository<RecurrenceRule, RecurrenceRuleId> for RecurrenceRuleRepositoryVariant {
    async fn save(
        &self,
        project_id: &ProjectId,
        entity: &RecurrenceRule,
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
        id: &RecurrenceRuleId,
    ) -> Result<Option<RecurrenceRule>, RepositoryError> {
        match self {
            Self::LocalSqlite(repo) => repo.find_by_id(project_id, id).await,
            Self::LocalAutomerge(repo) => repo.find_by_id(project_id, id).await,
        }
    }

    async fn find_all(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<RecurrenceRule>, RepositoryError> {
        match self {
            Self::LocalSqlite(repo) => repo.find_all(project_id).await,
            Self::LocalAutomerge(repo) => repo.find_all(project_id).await,
        }
    }

    async fn delete(
        &self,
        project_id: &ProjectId,
        id: &RecurrenceRuleId,
    ) -> Result<(), RepositoryError> {
        match self {
            Self::LocalSqlite(repo) => repo.delete(project_id, id).await,
            Self::LocalAutomerge(repo) => repo.delete(project_id, id).await,
        }
    }

    async fn exists(
        &self,
        project_id: &ProjectId,
        id: &RecurrenceRuleId,
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

/// 繰り返しルール統合リポジトリ
///
/// 複数のデータソース（SQLite、AutoMerge等）を統合管理し、
/// 保存と検索を分離したリポジトリパターンを実装する。
#[derive(Debug)]
pub struct RecurrenceRuleUnifiedRepository {
    save_repositories: Vec<RecurrenceRuleRepositoryVariant>,
    search_repositories: Vec<RecurrenceRuleRepositoryVariant>,
    /// SQLite への書き込み先と Automerge 同期キュー。SQLite ストレージが有効なときにある
    queued_sqlite: Option<QueuedSqlite<RecurrenceRuleLocalSqliteRepository>>,
}

impl Default for RecurrenceRuleUnifiedRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl RecurrenceRuleUnifiedRepository {
    /// 新しいRecurrenceRuleUnifiedRepositoryを作成（空のリポジトリで初期化）
    pub fn new() -> Self {
        Self {
            save_repositories: Vec::new(),
            search_repositories: Vec::new(),
            queued_sqlite: None,
        }
    }

    /// 指定したリポジトリで新しいRecurrenceRuleUnifiedRepositoryを作成
    ///
    /// # Arguments
    /// * `save_repositories` - データ保存用リポジトリ群
    /// * `search_repositories` - データ検索用リポジトリ群
    pub fn with_repositories(
        save_repositories: Vec<RecurrenceRuleRepositoryVariant>,
        search_repositories: Vec<RecurrenceRuleRepositoryVariant>,
    ) -> Self {
        Self {
            save_repositories,
            search_repositories,
            queued_sqlite: None,
        }
    }

    /// SQLiteリポジトリを保存用として追加
    pub fn add_sqlite_for_save(&mut self, sqlite_repo: RecurrenceRuleLocalSqliteRepository) {
        self.save_repositories
            .push(RecurrenceRuleRepositoryVariant::LocalSqlite(sqlite_repo));
    }

    /// 書き込みを「Automerge 同期キューへの登録 + SQLite への書き込み」の
    /// 1 トランザクションにする。Automerge へはワーカーが後から反映する
    pub fn set_queued_sqlite(&mut self, queued: QueuedSqlite<RecurrenceRuleLocalSqliteRepository>) {
        self.queued_sqlite = Some(queued);
    }

    /// AutoMergeリポジトリを保存用として追加
    pub fn add_automerge_for_save(
        &mut self,
        automerge_repo: RecurrenceRuleLocalAutomergeRepository,
    ) {
        self.save_repositories
            .push(RecurrenceRuleRepositoryVariant::LocalAutomerge(
                automerge_repo,
            ));
    }

    /// SQLiteリポジトリを検索用として追加
    pub fn add_sqlite_for_search(&mut self, sqlite_repo: RecurrenceRuleLocalSqliteRepository) {
        self.search_repositories
            .push(RecurrenceRuleRepositoryVariant::LocalSqlite(sqlite_repo));
    }

    /// AutoMergeリポジトリを検索用として追加
    pub fn add_automerge_for_search(
        &mut self,
        automerge_repo: RecurrenceRuleLocalAutomergeRepository,
    ) {
        self.search_repositories
            .push(RecurrenceRuleRepositoryVariant::LocalAutomerge(
                automerge_repo,
            ));
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

impl RecurrenceRuleRepositoryTrait for RecurrenceRuleUnifiedRepository {}

impl ProjectPatchable<RecurrenceRule, RecurrenceRuleId> for RecurrenceRuleUnifiedRepository {}

#[async_trait]
impl ProjectRepository<RecurrenceRule, RecurrenceRuleId> for RecurrenceRuleUnifiedRepository {
    async fn save(
        &self,
        project_id: &ProjectId,
        entity: &RecurrenceRule,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        info!(
            "Saving recurrence rule entity with ID: {} in project: {}",
            entity.id, project_id
        );
        if let Some(queued) = &self.queued_sqlite {
            let txn = queued
                .queue
                .begin(vec![AutomergeChange::RecurrenceRule(ProjectChange::Save {
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
        id: &RecurrenceRuleId,
    ) -> Result<Option<RecurrenceRule>, RepositoryError> {
        info!(
            "Finding recurrence rule by ID: {} in project: {}",
            id, project_id
        );

        for repository in &self.search_repositories {
            if let Some(entity) = repository.find_by_id(project_id, id).await? {
                return Ok(Some(entity));
            }
        }

        Ok(None)
    }

    async fn find_all(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<RecurrenceRule>, RepositoryError> {
        info!("Finding all recurrence rules in project: {}", project_id);

        if let Some(repository) = self.search_repositories.first() {
            repository.find_all(project_id).await
        } else {
            Ok(vec![])
        }
    }

    async fn delete(
        &self,
        project_id: &ProjectId,
        id: &RecurrenceRuleId,
    ) -> Result<(), RepositoryError> {
        info!(
            "Deleting recurrence rule with ID: {} in project: {}",
            id, project_id
        );
        if let Some(queued) = &self.queued_sqlite {
            let txn = queued
                .queue
                .begin(vec![AutomergeChange::RecurrenceRule(
                    ProjectChange::Delete {
                        project_id: *project_id,
                        id: *id,
                    },
                )])
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
        id: &RecurrenceRuleId,
    ) -> Result<bool, RepositoryError> {
        info!(
            "Checking if recurrence rule exists with ID: {} in project: {}",
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
        info!("Counting recurrence rules in project: {}", project_id);

        if let Some(repository) = self.search_repositories.first() {
            repository.count(project_id).await
        } else {
            Ok(0)
        }
    }
}
