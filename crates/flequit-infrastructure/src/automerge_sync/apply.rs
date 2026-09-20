//! 変更を Automerge ドキュメントへ適用する
//!
//! ワーカーはキューの行を少なくとも 1 回適用する（適用後・`processed` 更新前に
//! 終了すると、次回起動時にもう一度適用する）。そのため適用は冪等でなければならない。
//! 保存は全体の上書き、削除・関連の解除は「すでに無い」を成功として扱う。

use std::sync::Arc;

use tokio::sync::Mutex;

use flequit_infrastructure_automerge::infrastructure::accounts::account::AccountLocalAutomergeRepository;
use flequit_infrastructure_automerge::infrastructure::document_manager::DocumentManager;
use flequit_infrastructure_automerge::infrastructure::task_projects::{
    project::ProjectLocalAutomergeRepository,
    recurrence_rule::RecurrenceRuleLocalAutomergeRepository,
    subtask::SubTaskLocalAutomergeRepository,
    subtask_assignments::SubtaskAssignmentLocalAutomergeRepository,
    subtask_recurrence::SubtaskRecurrenceLocalAutomergeRepository,
    subtask_tag::SubtaskTagLocalAutomergeRepository, tag::TagLocalAutomergeRepository,
    task::TaskLocalAutomergeRepository, task_assignments::TaskAssignmentLocalAutomergeRepository,
    task_list::TaskListLocalAutomergeRepository,
    task_recurrence::TaskRecurrenceLocalAutomergeRepository,
    task_tag::TaskTagLocalAutomergeRepository,
};
use flequit_infrastructure_automerge::infrastructure::user_preferences::tag_bookmark::TagBookmarkLocalAutomergeRepository;
use flequit_infrastructure_automerge::infrastructure::users::user::UserLocalAutomergeRepository;
use flequit_repository::repositories::base_repository_trait::Repository;
use flequit_repository::repositories::project_relation_repository_trait::ProjectRelationRepository;
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_types::errors::repository_error::RepositoryError;

use super::change::{
    AutomergeChange, ProjectChange, RelationChange, RootChange, TagBookmarkChange, TrashChange,
};

/// 変更の適用先になる Automerge リポジトリ一式
///
/// 統合リポジトリと同じ `DocumentManager` を共有する。読み取り（ゴミ箱からの復元時の
/// 削除済みデータの取得）と書き込みが同じドキュメントを見るため。
#[derive(Debug)]
pub struct AutomergeSyncTargets {
    accounts: AccountLocalAutomergeRepository,
    users: UserLocalAutomergeRepository,
    projects: ProjectLocalAutomergeRepository,
    task_lists: TaskListLocalAutomergeRepository,
    tasks: TaskLocalAutomergeRepository,
    sub_tasks: SubTaskLocalAutomergeRepository,
    tags: TagLocalAutomergeRepository,
    recurrence_rules: RecurrenceRuleLocalAutomergeRepository,
    task_tags: TaskTagLocalAutomergeRepository,
    subtask_tags: SubtaskTagLocalAutomergeRepository,
    task_assignments: TaskAssignmentLocalAutomergeRepository,
    subtask_assignments: SubtaskAssignmentLocalAutomergeRepository,
    task_recurrences: TaskRecurrenceLocalAutomergeRepository,
    subtask_recurrences: SubtaskRecurrenceLocalAutomergeRepository,
    tag_bookmarks: TagBookmarkLocalAutomergeRepository,
}

impl AutomergeSyncTargets {
    pub async fn new(
        document_manager: Arc<Mutex<DocumentManager>>,
    ) -> Result<Self, RepositoryError> {
        let manager = || Arc::clone(&document_manager);
        Ok(Self {
            accounts: AccountLocalAutomergeRepository::new_with_manager(manager()).await?,
            users: UserLocalAutomergeRepository::new_with_manager(manager()).await?,
            projects: ProjectLocalAutomergeRepository::new_with_manager(manager()).await?,
            task_lists: TaskListLocalAutomergeRepository::new_with_manager(manager()).await?,
            tasks: TaskLocalAutomergeRepository::new_with_manager(manager()).await?,
            sub_tasks: SubTaskLocalAutomergeRepository::new_with_manager(manager()).await?,
            tags: TagLocalAutomergeRepository::new_with_manager(manager()).await?,
            recurrence_rules: RecurrenceRuleLocalAutomergeRepository::new_with_manager(manager())
                .await?,
            task_tags: TaskTagLocalAutomergeRepository::new_with_manager(manager()).await?,
            subtask_tags: SubtaskTagLocalAutomergeRepository::new_with_manager(manager()).await?,
            task_assignments: TaskAssignmentLocalAutomergeRepository::new_with_manager(manager())
                .await?,
            subtask_assignments: SubtaskAssignmentLocalAutomergeRepository::new_with_manager(
                manager(),
            )
            .await?,
            task_recurrences: TaskRecurrenceLocalAutomergeRepository::new_with_manager(manager())
                .await?,
            subtask_recurrences: SubtaskRecurrenceLocalAutomergeRepository::new_with_manager(
                manager(),
            )
            .await?,
            tag_bookmarks: TagBookmarkLocalAutomergeRepository::new_with_manager(manager()).await?,
        })
    }

    /// ゴミ箱からの復元で削除済みデータを読むためのプロジェクトリポジトリ
    pub fn projects(&self) -> &ProjectLocalAutomergeRepository {
        &self.projects
    }

    /// 1 件の変更を適用する
    pub async fn apply(&self, change: &AutomergeChange) -> Result<(), RepositoryError> {
        match change {
            AutomergeChange::Account(change) => apply_root(&self.accounts, change).await,
            AutomergeChange::User(change) => apply_root(&self.users, change).await,
            AutomergeChange::Project(change) => apply_root(&self.projects, change).await,
            AutomergeChange::TaskList(change) => apply_project(&self.task_lists, change).await,
            AutomergeChange::Task(change) => apply_project(&self.tasks, change).await,
            AutomergeChange::SubTask(change) => apply_project(&self.sub_tasks, change).await,
            AutomergeChange::Tag(change) => apply_project(&self.tags, change).await,
            AutomergeChange::RecurrenceRule(change) => {
                apply_project(&self.recurrence_rules, change).await
            }
            AutomergeChange::TaskTag(RelationChange::RemoveAllByChild {
                project_id,
                child_id,
            }) => {
                self.task_tags
                    .remove_all_relations_by_tag_id(project_id, child_id)
                    .await
            }
            AutomergeChange::TaskTag(change) => apply_relation(&self.task_tags, change).await,
            AutomergeChange::SubTaskTag(RelationChange::RemoveAllByChild {
                project_id,
                child_id,
            }) => {
                self.subtask_tags
                    .remove_all_relations_by_tag_id(project_id, child_id)
                    .await
            }
            AutomergeChange::SubTaskTag(change) => apply_relation(&self.subtask_tags, change).await,
            AutomergeChange::TaskAssignment(change) => {
                apply_relation(&self.task_assignments, change).await
            }
            AutomergeChange::SubTaskAssignment(change) => {
                apply_relation(&self.subtask_assignments, change).await
            }
            AutomergeChange::TaskRecurrence(change) => {
                apply_relation(&self.task_recurrences, change).await
            }
            AutomergeChange::SubTaskRecurrence(change) => {
                apply_relation(&self.subtask_recurrences, change).await
            }
            AutomergeChange::TagBookmark(change) => self.apply_tag_bookmark(change).await,
            AutomergeChange::Trash(change) => self.apply_trash(change).await,
        }
    }

    async fn apply_tag_bookmark(&self, change: &TagBookmarkChange) -> Result<(), RepositoryError> {
        match change {
            TagBookmarkChange::Create { bookmark } => self.tag_bookmarks.create(bookmark).await,
            TagBookmarkChange::Update { bookmark } => self.tag_bookmarks.update(bookmark).await,
            TagBookmarkChange::Delete {
                user_id,
                project_id,
                tag_id,
            } => ignore_missing(self.tag_bookmarks.delete(user_id, project_id, tag_id).await),
        }
    }

    async fn apply_trash(&self, change: &TrashChange) -> Result<(), RepositoryError> {
        let projects = &self.projects;
        match change {
            TrashChange::DeleteProject {
                project_id,
                user_id,
                timestamp,
            } => {
                projects
                    .mark_all_tasks_deleted(project_id, user_id, timestamp)
                    .await?;
                projects
                    .mark_all_tags_deleted(project_id, user_id, timestamp)
                    .await?;
                projects
                    .mark_all_task_lists_deleted(project_id, user_id, timestamp)
                    .await?;
                projects
                    .mark_project_deleted(project_id, user_id, timestamp)
                    .await
            }
            TrashChange::DeleteTaskList {
                project_id,
                task_list_id,
                user_id,
                timestamp,
            } => {
                projects
                    .mark_task_list_deleted(project_id, task_list_id, user_id, timestamp)
                    .await
            }
            TrashChange::DeleteTask {
                project_id,
                task_id,
                user_id,
                timestamp,
            } => {
                projects
                    .mark_task_deleted(project_id, task_id, user_id, timestamp)
                    .await
            }
            TrashChange::DeleteTag {
                project_id,
                tag_id,
                user_id,
                timestamp,
            } => {
                projects
                    .mark_tag_deleted(project_id, tag_id, user_id, timestamp)
                    .await
            }
            // 復元は「削除済みでない」を InvalidOperation で返す。
            // 2 回目の適用ではすでに戻っているので成功として扱う。
            TrashChange::RestoreProject {
                project_id,
                user_id,
                timestamp,
            } => {
                ignore_already_restored(
                    projects
                        .restore_project(project_id, user_id, timestamp)
                        .await,
                )?;
                projects
                    .restore_all_task_lists(project_id, user_id, timestamp)
                    .await?;
                projects
                    .restore_all_tags(project_id, user_id, timestamp)
                    .await?;
                projects
                    .restore_all_tasks(project_id, user_id, timestamp)
                    .await
            }
            TrashChange::RestoreTaskList {
                project_id,
                task_list_id,
                user_id,
                timestamp,
            } => ignore_already_restored(
                projects
                    .restore_task_list(project_id, task_list_id, user_id, timestamp)
                    .await,
            ),
            TrashChange::RestoreTask {
                project_id,
                task_id,
                user_id,
                timestamp,
            } => ignore_already_restored(
                projects
                    .restore_task(project_id, task_id, user_id, timestamp)
                    .await,
            ),
            TrashChange::RestoreTag {
                project_id,
                tag_id,
                user_id,
                timestamp,
            } => ignore_already_restored(
                projects
                    .restore_tag(project_id, tag_id, user_id, timestamp)
                    .await,
            ),
        }
    }
}

async fn apply_root<T, Id, R>(
    repository: &R,
    change: &RootChange<T, Id>,
) -> Result<(), RepositoryError>
where
    T: Send + Sync,
    Id: Send + Sync,
    R: Repository<T, Id>,
{
    match change {
        RootChange::Save {
            entity,
            user_id,
            timestamp,
        } => repository.save(entity, user_id, timestamp).await,
        RootChange::Delete { id } => ignore_missing(repository.delete(id).await),
    }
}

async fn apply_project<T, Id, R>(
    repository: &R,
    change: &ProjectChange<T, Id>,
) -> Result<(), RepositoryError>
where
    T: Send + Sync,
    Id: Send + Sync,
    R: ProjectRepository<T, Id>,
{
    match change {
        ProjectChange::Save {
            project_id,
            entity,
            user_id,
            timestamp,
        } => {
            repository
                .save(project_id, entity, user_id, timestamp)
                .await
        }
        ProjectChange::Delete { project_id, id } => {
            ignore_missing(repository.delete(project_id, id).await)
        }
    }
}

async fn apply_relation<T, P, C, R>(
    repository: &R,
    change: &RelationChange<P, C>,
) -> Result<(), RepositoryError>
where
    T: Send + Sync,
    P: Send + Sync,
    C: Send + Sync,
    R: ProjectRelationRepository<T, P, C>,
{
    match change {
        RelationChange::Add {
            project_id,
            parent_id,
            child_id,
            user_id,
            timestamp,
        } => {
            repository
                .add(project_id, parent_id, child_id, user_id, timestamp)
                .await
        }
        RelationChange::Remove {
            project_id,
            parent_id,
            child_id,
        } => ignore_missing(repository.remove(project_id, parent_id, child_id).await),
        RelationChange::RemoveAll {
            project_id,
            parent_id,
        } => ignore_missing(repository.remove_all(project_id, parent_id).await),
        RelationChange::RemoveAllByChild { .. } => Err(RepositoryError::InvalidOperation(
            "removing relations by child is only supported for tags".to_string(),
        )),
    }
}

/// 消す対象がすでに無いのは、前回の適用が済んでいるか、もともと無かったかのどちらか。
/// どちらも Automerge の状態としては目的どおりなので成功とする
fn ignore_missing(result: Result<(), RepositoryError>) -> Result<(), RepositoryError> {
    match result {
        Err(RepositoryError::NotFound(_)) => Ok(()),
        other => other,
    }
}

fn ignore_already_restored(result: Result<(), RepositoryError>) -> Result<(), RepositoryError> {
    match result {
        Err(RepositoryError::InvalidOperation(_)) => Ok(()),
        other => other,
    }
}

/// 再試行しても結果が変わらないエラーか
///
/// 入力（ペイロード）そのものに原因があるものは待っても直らないので、
/// すぐに `failed` にして同じドキュメントの後続の変更を進める。
pub fn is_permanent(error: &RepositoryError) -> bool {
    matches!(
        error,
        RepositoryError::InvalidOperation(_)
            | RepositoryError::ValidationError(_)
            | RepositoryError::SerializationError(_)
            | RepositoryError::ConversionError(_)
            | RepositoryError::Conversion(_)
    )
}
