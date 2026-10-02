use chrono::{DateTime, Utc};
use flequit_model::types::id_types::{ProjectId, UserId};
use flequit_types::errors::repository_error::RepositoryError;

use super::super::InfrastructureRepositories;
use crate::automerge_sync::{AutomergeChange, TrashChange};

pub(super) async fn delete(
    repositories: &InfrastructureRepositories,
    project_id: &ProjectId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<(), RepositoryError> {
    // SQLite の削除と Automerge への論理削除の登録を 1 トランザクションで確定する。
    // Automerge への反映はワーカーが後から行う
    let sqlite_repositories = repositories.sqlite()?;
    let sqlite_guard = sqlite_repositories.read().await;
    let txn = repositories
        .sync_queue()?
        .begin(vec![AutomergeChange::Trash(TrashChange::DeleteProject {
            project_id: *project_id,
            user_id: *user_id,
            timestamp: *timestamp,
        })])
        .await?;

    let sqlite_result: Result<(), RepositoryError> = async {
        let task_ids = sqlite_guard
            .tasks()
            .find_ids_by_project_id(project_id)
            .await?;
        // サブタスクも含むすべての階層のタスク。親を先に消すと子は外部キーで消えるので、
        // 子に対する以降の削除は何もしない
        for task_id in task_ids {
            sqlite_guard
                .task_tags
                .remove_all_by_task_id_with_txn(txn.txn(), project_id, &task_id)
                .await?;
            sqlite_guard
                .task_assignments()
                .remove_all_by_task_id_with_txn(txn.txn(), &task_id)
                .await?;
            sqlite_guard
                .task_recurrences
                .remove_all_with_txn(txn.txn(), project_id, &task_id)
                .await?;
            sqlite_guard
                .tasks()
                .delete_with_txn(txn.txn(), project_id, &task_id)
                .await?;
        }

        let tag_ids = sqlite_guard
            .tags()
            .find_ids_by_project_id(project_id)
            .await?;
        for tag_id in tag_ids {
            sqlite_guard
                .tag_bookmarks()
                .remove_all_by_tag_id_with_txn(txn.txn(), project_id, &tag_id)
                .await?;
            sqlite_guard
                .task_tags
                .remove_all_by_tag_id_with_txn(txn.txn(), project_id, &tag_id)
                .await?;
            sqlite_guard
                .tags()
                .delete_with_txn(txn.txn(), project_id, &tag_id)
                .await?;
        }

        sqlite_guard
            .task_lists()
            .remove_all_by_project_id_with_txn(txn.txn(), project_id)
            .await?;
        sqlite_guard
            .projects()
            .delete_with_txn(txn.txn(), project_id)
            .await?;
        Ok(())
    }
    .await;
    drop(sqlite_guard);

    txn.finish(sqlite_result).await
}
