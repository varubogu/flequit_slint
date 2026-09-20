//! 書き込みトランザクションとキューへの登録
//!
//! エンティティを書き込むトランザクションの **最初の文** でキューの行を INSERT する。
//! SQLite は読み取りから書き込みへの昇格時に、別の接続が書き込み中だと待たずに
//! `SQLITE_BUSY` を返すことがある。最初に書き込むことで、書き込みロックの取得を
//! ビジータイムアウトで待つ形にし、ワーカーの状態更新と衝突しても失敗しないようにする。

use std::sync::Arc;

use chrono::Utc;
use sea_orm::DatabaseTransaction;
use tokio::sync::{Notify, RwLock};

use flequit_infrastructure_sqlite::infrastructure::automerge_sync_queue::{
    AutomergeSyncQueueLocalSqliteRepository, NewSyncQueueEntry,
};
use flequit_infrastructure_sqlite::infrastructure::database_manager::DatabaseManager;
use flequit_model::traits::TransactionManager;
use flequit_types::errors::repository_error::RepositoryError;

use super::change::AutomergeChange;

/// Automerge 同期キューへの書き込み口
///
/// SQLite ストレージが有効なときに 1 つ作り、統合リポジトリ間で共有する。
/// Automerge ストレージが無効なら `enabled == false` で、行を入れずに
/// SQLite のトランザクションだけを提供する。
#[derive(Debug, Clone)]
pub struct AutomergeSyncQueue {
    repository: AutomergeSyncQueueLocalSqliteRepository,
    database_manager: Arc<RwLock<DatabaseManager>>,
    enabled: bool,
    wake: Arc<Notify>,
}

impl AutomergeSyncQueue {
    pub fn new(database_manager: Arc<RwLock<DatabaseManager>>, enabled: bool) -> Self {
        Self {
            repository: AutomergeSyncQueueLocalSqliteRepository::new(Arc::clone(&database_manager)),
            database_manager,
            enabled,
            wake: Arc::new(Notify::new()),
        }
    }

    /// 変更をキューへ入れるか（Automerge ストレージが有効か）
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn repository(&self) -> &AutomergeSyncQueueLocalSqliteRepository {
        &self.repository
    }

    /// コミットのたびに通知される。ワーカーはこれで起きる
    pub(crate) fn wake_signal(&self) -> Arc<Notify> {
        Arc::clone(&self.wake)
    }

    /// 書き込みトランザクションを始め、最初の文として `changes` をキューへ入れる。
    ///
    /// 返した [`QueuedTransaction`] の上で SQLite への書き込みを行い、
    /// [`QueuedTransaction::finish`] で確定または取り消す。
    pub async fn begin(
        &self,
        changes: Vec<AutomergeChange>,
    ) -> Result<QueuedTransaction<'_>, RepositoryError> {
        let entries = if self.enabled {
            changes
                .iter()
                .map(to_entry)
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };

        let txn = {
            let database_manager = self.database_manager.read().await;
            database_manager.begin().await?
        };

        if let Err(error) = self
            .repository
            .enqueue_with_txn(&txn, &entries, Utc::now())
            .await
        {
            return Err(rollback_with(txn, error).await);
        }

        Ok(QueuedTransaction {
            queue: self,
            txn,
            queued: !entries.is_empty(),
        })
    }
}

/// キューの行を入れ終えた書き込みトランザクション
#[must_use = "finish() で確定しないとトランザクションはロールバックされる"]
pub struct QueuedTransaction<'q> {
    queue: &'q AutomergeSyncQueue,
    txn: DatabaseTransaction,
    queued: bool,
}

impl QueuedTransaction<'_> {
    /// SQLite リポジトリの `*_with_txn` に渡すトランザクション
    pub fn txn(&self) -> &DatabaseTransaction {
        &self.txn
    }

    /// `result` が成功ならコミットしてワーカーを起こし、失敗ならロールバックする。
    ///
    /// ロールバックするとキューの行も消えるため、失敗した操作が Automerge へ
    /// 反映されることはない。
    pub async fn finish<T>(self, result: Result<T, RepositoryError>) -> Result<T, RepositoryError> {
        match result {
            Ok(value) => {
                self.txn.commit().await.map_err(|error| {
                    RepositoryError::TransactionError(format!(
                        "failed to commit transaction: {error}"
                    ))
                })?;
                if self.queued {
                    self.queue.wake.notify_one();
                }
                Ok(value)
            }
            Err(error) => Err(rollback_with(self.txn, error).await),
        }
    }
}

async fn rollback_with(txn: DatabaseTransaction, error: RepositoryError) -> RepositoryError {
    match txn.rollback().await {
        Ok(()) => error,
        Err(rollback_error) => {
            tracing::error!(%rollback_error, "failed to roll back transaction");
            RepositoryError::TransactionError(format!("{error}; rollback failed: {rollback_error}"))
        }
    }
}

fn to_entry(change: &AutomergeChange) -> Result<NewSyncQueueEntry, RepositoryError> {
    let payload = serde_json::to_string(change)
        .map_err(|error| RepositoryError::SerializationError(error.to_string()))?;
    Ok(NewSyncQueueEntry {
        document_key: change.document_key(),
        change_kind: change.kind(),
        payload,
    })
}

/// SQLite への書き込み先と、同じトランザクションで使うキュー
///
/// 統合リポジトリは SQLite ストレージが有効なときこれを持ち、書き込みを
/// 「キューへの登録 + SQLite への書き込み」の 1 トランザクションにする。
#[derive(Debug)]
pub struct QueuedSqlite<S> {
    pub sqlite: S,
    pub queue: AutomergeSyncQueue,
}

impl<S> QueuedSqlite<S> {
    pub fn new(sqlite: S, queue: AutomergeSyncQueue) -> Self {
        Self { sqlite, queue }
    }
}
