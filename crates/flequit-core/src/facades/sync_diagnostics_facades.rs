//! Automerge 同期キューの診断（設定画面の「データ同期」）

use crate::InfrastructureRepositoriesTrait;
use crate::ports::infrastructure_repositories::{
    FailedSyncChange, SyncQueueSummary, SyncRequeueReport,
};
use flequit_types::errors::service_error::ServiceError;

/// キューに残っている行の数。キューが無い構成なら `None`
pub async fn sync_queue_summary<R>(
    repositories: &R,
) -> Result<Option<SyncQueueSummary>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    Ok(repositories.sync_queue_summary().await?)
}

/// 反映を諦めた変更を古い順に最大 `limit` 件
pub async fn list_failed_sync_changes<R>(
    repositories: &R,
    limit: u64,
) -> Result<Vec<FailedSyncChange>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    Ok(repositories.failed_sync_changes(limit).await?)
}

/// 反映を諦めた変更を再投入する。順序を崩すものは戻さず、結果に分けて返す
pub async fn requeue_failed_sync_changes<R>(
    repositories: &R,
    ids: &[i64],
) -> Result<SyncRequeueReport, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    Ok(repositories.requeue_failed_sync_changes(ids).await?)
}
