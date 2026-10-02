use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flequit_model::models::accounts::account::Account;
use flequit_model::models::task_projects::project::Project;
use flequit_model::models::task_projects::recurrence_rule::RecurrenceRule;
use flequit_model::models::task_projects::tag::Tag;
use flequit_model::models::task_projects::task::Task;
use flequit_model::models::task_projects::task_assignment::TaskAssignment;
use flequit_model::models::task_projects::task_list::TaskList;
use flequit_model::models::task_projects::task_recurrence::TaskRecurrence;
use flequit_model::models::task_projects::task_tag::TaskTag;
use flequit_model::models::user_preferences::tag_bookmark::TagBookmark;
use flequit_model::models::users::user::User;
use flequit_model::types::id_types::{
    AccountId, ProjectId, RecurrenceRuleId, TagBookmarkId, TagId, TaskId, TaskListId, UserId,
};
use flequit_repository::repositories::base_repository_trait::Repository;
use flequit_repository::repositories::patchable_trait::Patchable;
use flequit_repository::repositories::project_patchable_trait::ProjectPatchable;
use flequit_repository::repositories::project_relation_repository_trait::ProjectRelationRepository;
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_types::errors::repository_error::RepositoryError;
use sea_orm::DatabaseTransaction;
use std::sync::Arc;
use tokio::sync::RwLock;

#[async_trait]
pub trait TagRepositoryExt: Send + Sync {
    async fn delete_with_relations(
        &self,
        project_id: &ProjectId,
        tag_id: &TagId,
    ) -> Result<(), RepositoryError>;
}

/// タグブックマークの読み書き
///
/// 実装は SQLite へ書き込み、Automerge へは同期キュー経由で反映する
/// （呼び出し側は Automerge を意識しない）。
#[async_trait]
pub trait TagBookmarkRepositoryPort: Send + Sync {
    async fn create(&self, bookmark: &TagBookmark) -> Result<(), RepositoryError>;
    async fn find_by_id(&self, id: &TagBookmarkId) -> Result<Option<TagBookmark>, RepositoryError>;
    async fn find_by_user_project_tag(
        &self,
        user_id: &UserId,
        project_id: &ProjectId,
        tag_id: &TagId,
    ) -> Result<Option<TagBookmark>, RepositoryError>;
    async fn find_by_user_and_project(
        &self,
        user_id: &UserId,
        project_id: &ProjectId,
    ) -> Result<Vec<TagBookmark>, RepositoryError>;
    async fn find_by_user(&self, user_id: &UserId) -> Result<Vec<TagBookmark>, RepositoryError>;
    async fn find_by_project_and_tag(
        &self,
        project_id: &ProjectId,
        tag_id: &TagId,
    ) -> Result<Vec<TagBookmark>, RepositoryError>;
    async fn update(&self, bookmark: &TagBookmark) -> Result<(), RepositoryError>;
    async fn update_bulk(&self, bookmarks: &[TagBookmark]) -> Result<(), RepositoryError>;
    async fn delete(&self, id: &TagBookmarkId) -> Result<(), RepositoryError>;
    async fn get_max_order_index(
        &self,
        user_id: &UserId,
        project_id: &ProjectId,
    ) -> Result<i32, RepositoryError>;
}

#[async_trait]
pub trait SqliteProjectRepositoryPort: Send + Sync {
    async fn delete_with_txn(
        &self,
        txn: &DatabaseTransaction,
        id: &ProjectId,
    ) -> Result<(), RepositoryError>;
}

#[async_trait]
pub trait SqliteTaskListRepositoryPort: Send + Sync {
    async fn delete_with_txn(
        &self,
        txn: &DatabaseTransaction,
        project_id: &ProjectId,
        id: &TaskListId,
    ) -> Result<(), RepositoryError>;
    async fn remove_all_by_project_id_with_txn(
        &self,
        txn: &DatabaseTransaction,
        project_id: &ProjectId,
    ) -> Result<(), RepositoryError>;
}

#[async_trait]
pub trait SqliteTaskRepositoryPort: Send + Sync {
    async fn find_ids_by_project_id(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<TaskId>, RepositoryError>;
    async fn delete_with_txn(
        &self,
        txn: &DatabaseTransaction,
        project_id: &ProjectId,
        id: &TaskId,
    ) -> Result<(), RepositoryError>;
}

#[async_trait]
pub trait SqliteTagRepositoryPort: Send + Sync {
    async fn find_ids_by_project_id(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<TagId>, RepositoryError>;
    async fn delete_with_txn(
        &self,
        txn: &DatabaseTransaction,
        project_id: &ProjectId,
        tag_id: &TagId,
    ) -> Result<(), RepositoryError>;
}

#[async_trait]
pub trait SqliteTaskTagRepositoryPort: Send + Sync {
    async fn remove_all_by_task_id_with_txn(
        &self,
        txn: &DatabaseTransaction,
        project_id: &ProjectId,
        task_id: &TaskId,
    ) -> Result<(), RepositoryError>;
    async fn remove_all_by_tag_id_with_txn(
        &self,
        txn: &DatabaseTransaction,
        project_id: &ProjectId,
        tag_id: &TagId,
    ) -> Result<(), RepositoryError>;
}

#[async_trait]
pub trait SqliteTaskAssignmentRepositoryPort: Send + Sync {
    async fn remove_all_by_task_id_with_txn(
        &self,
        txn: &DatabaseTransaction,
        task_id: &TaskId,
    ) -> Result<(), RepositoryError>;
}

#[async_trait]
pub trait SqliteTaskRecurrenceRepositoryPort: Send + Sync {
    async fn remove_all_with_txn(
        &self,
        txn: &DatabaseTransaction,
        project_id: &ProjectId,
        task_id: &TaskId,
    ) -> Result<(), RepositoryError>;
}

#[async_trait]
pub trait SqliteTagBookmarkRepositoryPort: Send + Sync {
    async fn remove_all_by_tag_id_with_txn(
        &self,
        txn: &DatabaseTransaction,
        project_id: &ProjectId,
        tag_id: &TagId,
    ) -> Result<(), RepositoryError>;
}

pub trait SqliteRepositoriesPort: Send + Sync {
    type ProjectsRepository: SqliteProjectRepositoryPort;
    type TaskListsRepository: SqliteTaskListRepositoryPort;
    type TasksRepository: SqliteTaskRepositoryPort;
    type TagsRepository: SqliteTagRepositoryPort;
    type TaskTagsRepository: SqliteTaskTagRepositoryPort;
    type TaskAssignmentsRepository: SqliteTaskAssignmentRepositoryPort;
    type TaskRecurrencesRepository: SqliteTaskRecurrenceRepositoryPort;
    type TagBookmarksRepository: SqliteTagBookmarkRepositoryPort;

    fn projects_repo(&self) -> &Self::ProjectsRepository;
    fn task_lists_repo(&self) -> &Self::TaskListsRepository;
    fn tasks_repo(&self) -> &Self::TasksRepository;
    fn tags_repo(&self) -> &Self::TagsRepository;
    fn task_tags_repo(&self) -> &Self::TaskTagsRepository;
    fn task_assignments_repo(&self) -> &Self::TaskAssignmentsRepository;
    fn task_recurrences_repo(&self) -> &Self::TaskRecurrencesRepository;
    fn tag_bookmarks_repo(&self) -> &Self::TagBookmarksRepository;
}

/// Storage-agnostic boundary for deletions that must span multiple backends.
///
/// Implementations own their concrete transaction and rollback mechanics so
/// callers never need to name a database-specific transaction type.
#[async_trait]
pub trait TransactionalDeletionPort: Send + Sync {
    async fn delete_project_transactionally(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError>;

    /// タスクを子孫ごと削除する
    async fn delete_task_transactionally(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError>;

    async fn delete_task_list_transactionally(
        &self,
        project_id: &ProjectId,
        task_list_id: &TaskListId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError>;

    async fn delete_tag_transactionally(
        &self,
        project_id: &ProjectId,
        tag_id: &TagId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError>;
}

/// ゴミ箱（論理削除済み）からの復元
///
/// 削除済みのデータは Automerge にだけ残っている。実装は Automerge への未反映の
/// 変更を先に反映してから削除済みデータを読み、SQLite へ戻す。
#[async_trait]
pub trait TransactionalRestorePort: Send + Sync {
    async fn restore_project_transactionally(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError>;

    /// タスクと、同じ削除で消えた子孫を戻す
    async fn restore_task_transactionally(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError>;

    async fn restore_task_list_transactionally(
        &self,
        project_id: &ProjectId,
        task_list_id: &TaskListId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError>;

    async fn restore_tag_transactionally(
        &self,
        project_id: &ProjectId,
        tag_id: &TagId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError>;
}

/// Automerge へ反映を諦めた変更（同期キューの `failed` の行）
#[derive(Debug, Clone, PartialEq)]
pub struct FailedSyncChange {
    pub id: i64,
    /// 反映先ドキュメント（`project:{id}` / `account` / `user`）
    pub document_key: String,
    /// 変更の種類（`task.save` など）
    pub change_kind: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// 同期キューに残っている行の数
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncQueueSummary {
    /// Automerge へ未反映（再試行待ちを含む）
    pub pending: u64,
    /// 反映を諦めた
    pub failed: u64,
}

/// `failed` の行を再投入した結果
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncRequeueReport {
    /// 未反映に戻した行
    pub requeued: Vec<i64>,
    /// 同じドキュメントの後の変更が反映済みのため戻さなかった行
    /// （戻すと Automerge を古い内容へ巻き戻す）
    pub superseded: Vec<i64>,
    /// 見つからないか、すでに `failed` ではない行
    pub not_failed: Vec<i64>,
}

/// Automerge 同期キューの診断と、反映を諦めた変更の再投入
///
/// Automerge ストレージを使わない構成ではキューが無い。そのとき
/// [`Self::sync_queue_summary`] は `None`、一覧は空、再投入は何もしない。
#[async_trait]
pub trait SyncDiagnosticsPort: Send + Sync {
    async fn sync_queue_summary(&self) -> Result<Option<SyncQueueSummary>, RepositoryError>;

    /// `failed` の行を古い順に最大 `limit` 件
    async fn failed_sync_changes(
        &self,
        limit: u64,
    ) -> Result<Vec<FailedSyncChange>, RepositoryError>;

    /// `ids` の `failed` の行を、順序を崩さないものだけ未反映に戻す
    async fn requeue_failed_sync_changes(
        &self,
        ids: &[i64],
    ) -> Result<SyncRequeueReport, RepositoryError>;
}

#[async_trait]
pub trait InfrastructureRepositoriesTrait:
    TransactionalDeletionPort
    + TransactionalRestorePort
    + SyncDiagnosticsPort
    + Send
    + Sync
    + std::fmt::Debug
{
    type AccountsRepository: Repository<Account, AccountId> + Send + Sync;
    type ProjectsRepository: Repository<Project, ProjectId>
        + Patchable<Project, ProjectId>
        + Send
        + Sync;
    type TagsRepository: ProjectRepository<Tag, TagId> + TagRepositoryExt + Send + Sync;
    type TasksRepository: ProjectPatchable<Task, TaskId> + Send + Sync;
    type TaskListsRepository: ProjectPatchable<TaskList, TaskListId> + Send + Sync;
    type UsersRepository: Repository<User, UserId> + Send + Sync;
    type RecurrenceRulesRepository: ProjectPatchable<RecurrenceRule, RecurrenceRuleId> + Send + Sync;
    type TaskAssignmentsRepository: ProjectRelationRepository<TaskAssignment, TaskId, UserId>
        + Send
        + Sync;
    type TaskTagsRepository: ProjectRelationRepository<TaskTag, TaskId, TagId> + Send + Sync;
    type TaskRecurrencesRepository: ProjectRelationRepository<TaskRecurrence, TaskId, RecurrenceRuleId>
        + Send
        + Sync;

    type TagBookmarksRepository: TagBookmarkRepositoryPort;

    type SqliteRepositories: SqliteRepositoriesPort;

    fn accounts(&self) -> &Self::AccountsRepository;
    fn projects(&self) -> &Self::ProjectsRepository;
    fn tags(&self) -> &Self::TagsRepository;
    fn tasks(&self) -> &Self::TasksRepository;
    fn task_lists(&self) -> &Self::TaskListsRepository;
    fn users(&self) -> &Self::UsersRepository;
    fn recurrence_rules(&self) -> &Self::RecurrenceRulesRepository;
    fn task_assignments(&self) -> &Self::TaskAssignmentsRepository;
    fn task_tags(&self) -> &Self::TaskTagsRepository;
    fn task_recurrences(&self) -> &Self::TaskRecurrencesRepository;

    fn tag_bookmarks(&self) -> &Self::TagBookmarksRepository;

    fn sqlite_repositories(&self) -> Option<&Arc<RwLock<Self::SqliteRepositories>>>;

    async fn initialize(&mut self) -> Result<(), Box<dyn std::error::Error>>;
    async fn cleanup(&mut self) -> Result<(), Box<dyn std::error::Error>>;
}
