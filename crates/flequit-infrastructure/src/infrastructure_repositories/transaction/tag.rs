use chrono::{DateTime, Utc};
use flequit_model::types::id_types::{ProjectId, TagId, UserId};
use flequit_types::errors::repository_error::RepositoryError;

use super::super::InfrastructureRepositories;
use crate::automerge_sync::{AutomergeChange, TrashChange};

pub(super) async fn delete(
    repositories: &InfrastructureRepositories,
    project_id: &ProjectId,
    tag_id: &TagId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<(), RepositoryError> {
    // SQLite の削除と Automerge への論理削除の登録を 1 トランザクションで確定する。
    // Automerge への反映はワーカーが後から行う
    let sqlite_repositories = repositories.sqlite()?;
    let sqlite_guard = sqlite_repositories.read().await;
    let txn = repositories
        .sync_queue()?
        .begin(vec![AutomergeChange::Trash(TrashChange::DeleteTag {
            project_id: *project_id,
            tag_id: *tag_id,
            user_id: *user_id,
            timestamp: *timestamp,
        })])
        .await?;

    let sqlite_result: Result<(), RepositoryError> = async {
        sqlite_guard
            .tag_bookmarks()
            .remove_all_by_tag_id_with_txn(txn.txn(), project_id, tag_id)
            .await?;
        sqlite_guard
            .task_tags
            .remove_all_by_tag_id_with_txn(txn.txn(), project_id, tag_id)
            .await?;
        sqlite_guard
            .subtask_tags
            .remove_all_by_tag_id_with_txn(txn.txn(), tag_id)
            .await?;
        sqlite_guard
            .tags()
            .delete_with_txn(txn.txn(), project_id, tag_id)
            .await?;
        Ok(())
    }
    .await;
    drop(sqlite_guard);

    txn.finish(sqlite_result).await
}
