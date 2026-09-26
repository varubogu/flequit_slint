//! キューを読み、Automerge へ反映するバックグラウンド処理
//!
//! - 行は id 順に 1 件ずつ適用する。同じドキュメントの行は順序を守る
//! - 適用に成功したら `processed` にする（少なくとも 1 回の配送。適用は冪等）
//! - 失敗したら指数バックオフで再試行し、その間は同じドキュメントの後続を止める
//! - 再試行の上限に達したか、入力そのものが原因のエラーは `failed` にして後続を進める
//! - `processed` の行は保持期間（30 日）を過ぎたら削除する

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;

use flequit_core::ports::infrastructure_repositories::{
    FailedSyncChange, SyncQueueSummary, SyncRequeueReport,
};
use flequit_infrastructure_sqlite::infrastructure::automerge_sync_queue::{
    FailedSyncQueueEntry, RequeueOutcome, SyncQueueEntry,
};
use flequit_infrastructure_sqlite::models::automerge_sync_queue::SyncQueueStatus;
use flequit_types::errors::repository_error::RepositoryError;

use super::apply::{AutomergeSyncTargets, is_permanent};
use super::change::AutomergeChange;
use super::queue::AutomergeSyncQueue;

/// 処理済みの行を残す日数
pub const PROCESSED_RETENTION_DAYS: i64 = 30;

/// 再試行の上限。超えたら `failed` にする
pub const MAX_ATTEMPTS: i32 = 10;

/// バックオフの上限
const MAX_BACKOFF_SECS: i64 = 5 * 60;

/// 1 回に読む行数
const BATCH_SIZE: u64 = 100;

/// 通知が無くてもキューを見に行く間隔（取りこぼしへの保険）
const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// 処理済みの行を掃除する間隔
const CLEANUP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// 起動時のログに 1 件ずつ出す `failed` の行の上限
const FAILED_LOG_LIMIT: u64 = 20;

/// `attempts` 回目の失敗の後、次に試すまでの時間（1, 2, 4, ... 秒、上限 5 分）
pub fn backoff_after(attempts: i32) -> chrono::Duration {
    let exponent = attempts.saturating_sub(1).clamp(0, 16) as u32;
    let seconds = 2_i64.saturating_pow(exponent).min(MAX_BACKOFF_SECS);
    chrono::Duration::seconds(seconds)
}

/// 1 回のパスの結果
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncPassReport {
    /// Automerge へ反映して `processed` にした行
    pub processed: usize,
    /// 失敗して再試行を待つことになった行
    pub retried: usize,
    /// `failed` にした行
    pub failed: usize,
    /// 未処理の行が残っているドキュメント（バックオフ中・先行する行の失敗待ち）
    pub pending_documents: HashSet<String>,
    /// 最も早い再試行予定時刻
    pub next_retry_at: Option<DateTime<Utc>>,
}

enum ApplyFailure {
    Retryable(String),
    Permanent(String),
}

/// キューの行を Automerge へ適用する
///
/// バックグラウンドのワーカーと、Automerge を読む直前に反映を済ませたい処理
/// （ゴミ箱からの復元）の両方から使う。同時に 2 つのパスが走らないよう排他する。
#[derive(Debug)]
pub struct AutomergeSyncProcessor {
    queue: AutomergeSyncQueue,
    targets: AutomergeSyncTargets,
    pass_lock: Mutex<()>,
}

impl AutomergeSyncProcessor {
    pub fn new(queue: AutomergeSyncQueue, targets: AutomergeSyncTargets) -> Self {
        Self {
            queue,
            targets,
            pass_lock: Mutex::new(()),
        }
    }

    pub fn targets(&self) -> &AutomergeSyncTargets {
        &self.targets
    }

    pub fn queue(&self) -> &AutomergeSyncQueue {
        &self.queue
    }

    /// 期限の来た未処理の行をすべて id 順に適用する
    pub async fn process_pending(&self) -> Result<SyncPassReport, RepositoryError> {
        let _pass = self.pass_lock.lock().await;
        let repository = self.queue.repository();
        let mut report = SyncPassReport::default();
        // 先行する行がまだ終わっていないドキュメント。後続の行を追い越させない
        let mut blocked: HashSet<String> = HashSet::new();
        let mut after_id = 0;

        loop {
            let batch = repository.find_pending_after(after_id, BATCH_SIZE).await?;
            let Some(last) = batch.last() else {
                break;
            };
            after_id = last.id;

            for entry in batch {
                if blocked.contains(&entry.document_key) {
                    continue;
                }
                let now = Utc::now();
                if let Some(next_attempt_at) = entry.next_attempt_at
                    && next_attempt_at > now
                {
                    report.next_retry_at = earliest(report.next_retry_at, next_attempt_at);
                    blocked.insert(entry.document_key);
                    continue;
                }

                match self.apply_entry(&entry).await {
                    Ok(()) => {
                        repository.mark_processed(entry.id, Utc::now()).await?;
                        report.processed += 1;
                    }
                    Err(ApplyFailure::Permanent(message)) => {
                        tracing::error!(
                            id = entry.id,
                            kind = %entry.change_kind,
                            document = %entry.document_key,
                            error = %message,
                            "gave up applying a queued change to Automerge: the error will not go away on retry"
                        );
                        repository.mark_failed(entry.id, &message).await?;
                        report.failed += 1;
                    }
                    Err(ApplyFailure::Retryable(message)) => {
                        let attempts = entry.attempts + 1;
                        if attempts >= MAX_ATTEMPTS {
                            tracing::error!(
                                id = entry.id,
                                kind = %entry.change_kind,
                                document = %entry.document_key,
                                attempts,
                                error = %message,
                                "gave up applying a queued change to Automerge after the maximum number of attempts"
                            );
                            repository.mark_failed(entry.id, &message).await?;
                            report.failed += 1;
                        } else {
                            let next_attempt_at = Utc::now() + backoff_after(attempts);
                            tracing::warn!(
                                id = entry.id,
                                kind = %entry.change_kind,
                                document = %entry.document_key,
                                attempts,
                                error = %message,
                                %next_attempt_at,
                                "failed to apply a queued change to Automerge; will retry"
                            );
                            repository
                                .record_failure(entry.id, &message, next_attempt_at)
                                .await?;
                            report.retried += 1;
                            report.next_retry_at = earliest(report.next_retry_at, next_attempt_at);
                            blocked.insert(entry.document_key);
                        }
                    }
                }
            }
        }

        report.pending_documents = blocked;
        Ok(report)
    }

    /// `document_key` の未処理の行を今すぐ適用する。
    ///
    /// Automerge の内容を読んで判断する処理の前に呼ぶ（読み取りバリア）。
    /// 失敗の再試行待ちで行が残った場合はエラーを返す。
    pub async fn flush_document(&self, document_key: &str) -> Result<(), RepositoryError> {
        let report = self.process_pending().await?;
        if report.pending_documents.contains(document_key) {
            return Err(RepositoryError::AutomergeError(format!(
                "Automerge document {document_key} has changes that could not be applied yet"
            )));
        }
        Ok(())
    }

    /// 保持期間を過ぎた `processed` の行を削除する
    pub async fn cleanup_processed(&self, now: DateTime<Utc>) -> Result<u64, RepositoryError> {
        let cutoff = now - chrono::Duration::days(PROCESSED_RETENTION_DAYS);
        self.queue
            .repository()
            .delete_processed_before(cutoff)
            .await
    }

    /// 未処理・反映を諦めた行の数
    pub async fn summary(&self) -> Result<SyncQueueSummary, RepositoryError> {
        let repository = self.queue.repository();
        Ok(SyncQueueSummary {
            pending: repository.count_by_status(SyncQueueStatus::Pending).await?,
            failed: repository.count_by_status(SyncQueueStatus::Failed).await?,
        })
    }

    /// `failed` の行を古い順に最大 `limit` 件
    pub async fn failed_changes(
        &self,
        limit: u64,
    ) -> Result<Vec<FailedSyncChange>, RepositoryError> {
        let entries = self.queue.repository().find_failed(limit).await?;
        Ok(entries.into_iter().map(to_failed_change).collect())
    }

    /// `ids` の `failed` の行を `pending` に戻し、ワーカーを起こす。
    ///
    /// 同じドキュメントの後の行が反映済みの行は戻さない（Automerge を古い内容へ
    /// 巻き戻すため）。判定はリポジトリが 1 文で行う。
    pub async fn requeue_failed(&self, ids: &[i64]) -> Result<SyncRequeueReport, RepositoryError> {
        let repository = self.queue.repository();
        let mut report = SyncRequeueReport::default();
        for &id in ids {
            match repository.requeue_failed(id).await? {
                RequeueOutcome::Requeued => report.requeued.push(id),
                RequeueOutcome::Superseded => report.superseded.push(id),
                RequeueOutcome::NotFailed => report.not_failed.push(id),
            }
        }
        tracing::info!(
            requeued = ?report.requeued,
            superseded = ?report.superseded,
            not_failed = ?report.not_failed,
            "requeued failed Automerge sync queue entries"
        );
        if !report.requeued.is_empty() {
            self.queue.wake();
        }
        Ok(report)
    }

    /// 反映を諦めた行があれば、調査の手がかりとしてログに出す
    pub async fn log_failed(&self) -> Result<(), RepositoryError> {
        let failed = self
            .queue
            .repository()
            .count_by_status(SyncQueueStatus::Failed)
            .await?;
        if failed == 0 {
            return Ok(());
        }
        tracing::warn!(
            failed,
            "some changes were never applied to Automerge; review them under Settings > Data sync"
        );
        for entry in self
            .queue
            .repository()
            .find_failed(FAILED_LOG_LIMIT)
            .await?
        {
            tracing::warn!(
                id = entry.id,
                kind = %entry.change_kind,
                document = %entry.document_key,
                attempts = entry.attempts,
                created_at = %entry.created_at,
                error = entry.last_error.as_deref().unwrap_or(""),
                "failed Automerge sync queue entry"
            );
        }
        Ok(())
    }

    async fn apply_entry(&self, entry: &SyncQueueEntry) -> Result<(), ApplyFailure> {
        let change: AutomergeChange = serde_json::from_str(&entry.payload).map_err(|error| {
            ApplyFailure::Permanent(format!(
                "undecodable {} payload: {error}",
                entry.change_kind
            ))
        })?;
        self.targets.apply(&change).await.map_err(|error| {
            if is_permanent(&error) {
                ApplyFailure::Permanent(error.to_string())
            } else {
                ApplyFailure::Retryable(error.to_string())
            }
        })
    }
}

fn to_failed_change(entry: FailedSyncQueueEntry) -> FailedSyncChange {
    FailedSyncChange {
        id: entry.id,
        document_key: entry.document_key,
        change_kind: entry.change_kind,
        attempts: entry.attempts,
        last_error: entry.last_error,
        created_at: entry.created_at,
    }
}

fn earliest(current: Option<DateTime<Utc>>, candidate: DateTime<Utc>) -> Option<DateTime<Utc>> {
    Some(current.map_or(candidate, |current| current.min(candidate)))
}

/// 動いているワーカーへの操作口
///
/// アプリの終了時に [`AutomergeSyncHandle::shutdown`] を呼ぶ。呼ばずに捨てても
/// キューは SQLite に残り、次回起動時に続きから反映される。
#[derive(Debug)]
pub struct AutomergeSyncHandle {
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl AutomergeSyncHandle {
    /// 残っている行をできるだけ反映してから止める。
    ///
    /// `timeout` を過ぎたらタスクを中断する（残りは次回起動時に反映される）。
    /// 中断が効くのは次の `.await` からで、実行中の Automerge への適用は同期処理の
    /// ため最後まで走る。時間内に止まったら `true` を返す。
    pub async fn shutdown(mut self, timeout: Duration) -> bool {
        let _ = self.shutdown.send(true);
        if tokio::time::timeout(timeout, &mut self.task).await.is_ok() {
            return true;
        }
        // JoinHandle を捨てるだけではタスクは止まらず、終了処理と並んで走り続ける
        self.task.abort();
        false
    }
}

/// ワーカーを `runtime` 上で起動する。
///
/// 起動直後に前回の残りを反映し、処理済みの古い行を掃除する。
pub fn spawn_worker(
    processor: Arc<AutomergeSyncProcessor>,
    runtime: &tokio::runtime::Handle,
) -> AutomergeSyncHandle {
    let (shutdown, mut shutdown_requested) = watch::channel(false);
    let wake = processor.queue().wake_signal();

    let task = runtime.spawn(async move {
        if let Err(error) = processor.log_failed().await {
            tracing::warn!(%error, "failed to read failed Automerge sync queue entries");
        }
        let mut last_cleanup: Option<tokio::time::Instant> = None;
        loop {
            if last_cleanup.is_none_or(|at| at.elapsed() >= CLEANUP_INTERVAL) {
                match processor.cleanup_processed(Utc::now()).await {
                    Ok(0) => {}
                    Ok(deleted) => {
                        tracing::info!(deleted, "deleted processed Automerge sync queue entries")
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to clean up the Automerge sync queue")
                    }
                }
                last_cleanup = Some(tokio::time::Instant::now());
            }

            let wait = match processor.process_pending().await {
                Ok(report) => report
                    .next_retry_at
                    .and_then(|at| (at - Utc::now()).to_std().ok())
                    .map_or(POLL_INTERVAL, |until_retry| until_retry.min(POLL_INTERVAL)),
                Err(error) => {
                    tracing::warn!(%error, "failed to process the Automerge sync queue");
                    POLL_INTERVAL
                }
            };

            tokio::select! {
                _ = wake.notified() => {}
                _ = tokio::time::sleep(wait) => {}
                _ = shutdown_requested.changed() => {
                    if let Err(error) = processor.process_pending().await {
                        tracing::warn!(%error, "failed to apply the Automerge sync queue before shutdown");
                    }
                    break;
                }
            }
        }
    });

    AutomergeSyncHandle { shutdown, task }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_from_one_second_and_stops_at_five_minutes() {
        assert_eq!(backoff_after(1), chrono::Duration::seconds(1));
        assert_eq!(backoff_after(2), chrono::Duration::seconds(2));
        assert_eq!(backoff_after(4), chrono::Duration::seconds(8));
        assert_eq!(backoff_after(9), chrono::Duration::seconds(256));
        assert_eq!(backoff_after(10), chrono::Duration::seconds(300));
        assert_eq!(backoff_after(40), chrono::Duration::seconds(300));
    }
}
