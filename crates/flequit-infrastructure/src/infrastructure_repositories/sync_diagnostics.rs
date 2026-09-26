//! Automerge 同期キューの診断と再投入

use async_trait::async_trait;
use flequit_core::ports::infrastructure_repositories::{
    FailedSyncChange, SyncDiagnosticsPort, SyncQueueSummary, SyncRequeueReport,
};
use flequit_types::errors::repository_error::RepositoryError;

use super::InfrastructureRepositories;

#[async_trait]
impl SyncDiagnosticsPort for InfrastructureRepositories {
    async fn sync_queue_summary(&self) -> Result<Option<SyncQueueSummary>, RepositoryError> {
        match self.unified_manager.automerge_sync() {
            Some(processor) => processor.summary().await.map(Some),
            None => Ok(None),
        }
    }

    async fn failed_sync_changes(
        &self,
        limit: u64,
    ) -> Result<Vec<FailedSyncChange>, RepositoryError> {
        match self.unified_manager.automerge_sync() {
            Some(processor) => processor.failed_changes(limit).await,
            None => Ok(Vec::new()),
        }
    }

    async fn requeue_failed_sync_changes(
        &self,
        ids: &[i64],
    ) -> Result<SyncRequeueReport, RepositoryError> {
        match self.unified_manager.automerge_sync() {
            Some(processor) => processor.requeue_failed(ids).await,
            None => Ok(SyncRequeueReport {
                not_failed: ids.to_vec(),
                ..SyncRequeueReport::default()
            }),
        }
    }
}
