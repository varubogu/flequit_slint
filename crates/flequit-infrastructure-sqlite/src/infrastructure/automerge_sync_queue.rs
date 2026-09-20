//! Automerge 同期キューの SQLite リポジトリ
//!
//! 書き込み側は [`AutomergeSyncQueueLocalSqliteRepository::enqueue_with_txn`] で
//! エンティティの変更と同じトランザクションに行を入れる。
//! 読み出し・状態更新はバックグラウンドのワーカーが 1 文ずつ（自動コミットで）行う。
//! ワーカーの書き込みを短く保ち、UI からの書き込みトランザクションを待たせないため。

use std::sync::Arc;

use chrono::{DateTime, Utc};
use flequit_types::errors::repository_error::RepositoryError;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::{NotSet, Set},
    ColumnTrait, DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use tokio::sync::RwLock;

use super::database_manager::DatabaseManager;
use crate::errors::sqlite_error::SQLiteError;
use crate::models::automerge_sync_queue::{
    ActiveModel, Column, Entity as SyncQueueEntity, Model, SyncQueueStatus,
};

/// キューへ入れる 1 件
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSyncQueueEntry {
    pub document_key: String,
    pub change_kind: String,
    pub payload: String,
}

/// キューから読み出した未処理の 1 件
#[derive(Debug, Clone, PartialEq)]
pub struct SyncQueueEntry {
    pub id: i64,
    pub document_key: String,
    pub change_kind: String,
    pub payload: String,
    pub attempts: i32,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl From<Model> for SyncQueueEntry {
    fn from(model: Model) -> Self {
        Self {
            id: model.id,
            document_key: model.document_key,
            change_kind: model.change_kind,
            payload: model.payload,
            attempts: model.attempts,
            next_attempt_at: model.next_attempt_at,
            created_at: model.created_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AutomergeSyncQueueLocalSqliteRepository {
    db_manager: Arc<RwLock<DatabaseManager>>,
}

fn db_error(error: sea_orm::DbErr) -> RepositoryError {
    RepositoryError::from(SQLiteError::from(error))
}

impl AutomergeSyncQueueLocalSqliteRepository {
    pub fn new(db_manager: Arc<RwLock<DatabaseManager>>) -> Self {
        Self { db_manager }
    }

    /// 呼び出し側のトランザクションで行を追加する。
    ///
    /// コミット・ロールバックは呼び出し側が行う。エンティティの変更と
    /// キューの行は必ず同じトランザクションで確定させること。
    pub async fn enqueue_with_txn(
        &self,
        txn: &DatabaseTransaction,
        entries: &[NewSyncQueueEntry],
        created_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        if entries.is_empty() {
            return Ok(());
        }
        let models = entries.iter().map(|entry| ActiveModel {
            id: NotSet,
            document_key: Set(entry.document_key.clone()),
            change_kind: Set(entry.change_kind.clone()),
            payload: Set(entry.payload.clone()),
            status: Set(SyncQueueStatus::Pending.as_str().to_string()),
            attempts: Set(0),
            last_error: Set(None),
            next_attempt_at: Set(None),
            created_at: Set(created_at),
            processed_at: Set(None),
        });
        SyncQueueEntity::insert_many(models)
            .exec(txn)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    /// `after_id` より後の未処理の行を id 順に最大 `limit` 件返す。
    ///
    /// バックオフ中の行も返す。同じドキュメントの後続の行を追い越させないため、
    /// 待つかどうかは呼び出し側が判断する。
    pub async fn find_pending_after(
        &self,
        after_id: i64,
        limit: u64,
    ) -> Result<Vec<SyncQueueEntry>, RepositoryError> {
        let db_manager = self.db_manager.read().await;
        let db = db_manager.get_connection().await?;
        let models = SyncQueueEntity::find()
            .filter(Column::Status.eq(SyncQueueStatus::Pending.as_str()))
            .filter(Column::Id.gt(after_id))
            .order_by_asc(Column::Id)
            .limit(limit)
            .all(db)
            .await
            .map_err(db_error)?;
        Ok(models.into_iter().map(SyncQueueEntry::from).collect())
    }

    /// Automerge への反映が済んだ行を `processed` にする
    pub async fn mark_processed(
        &self,
        id: i64,
        processed_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        let db_manager = self.db_manager.read().await;
        let db = db_manager.get_connection().await?;
        SyncQueueEntity::update_many()
            .col_expr(
                Column::Status,
                Expr::value(SyncQueueStatus::Processed.as_str()),
            )
            .col_expr(Column::Attempts, Expr::col(Column::Attempts).add(1))
            .col_expr(Column::ProcessedAt, Expr::value(processed_at))
            .col_expr(
                Column::NextAttemptAt,
                Expr::value(Option::<DateTime<Utc>>::None),
            )
            .filter(Column::Id.eq(id))
            .exec(db)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    /// 反映に失敗した行を `pending` のまま残し、次に試す時刻を記録する
    pub async fn record_failure(
        &self,
        id: i64,
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        let db_manager = self.db_manager.read().await;
        let db = db_manager.get_connection().await?;
        SyncQueueEntity::update_many()
            .col_expr(Column::Attempts, Expr::col(Column::Attempts).add(1))
            .col_expr(Column::LastError, Expr::value(error))
            .col_expr(Column::NextAttemptAt, Expr::value(next_attempt_at))
            .filter(Column::Id.eq(id))
            .exec(db)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    /// 反映を諦めた行を `failed` にする（削除せず調査用に残す）
    pub async fn mark_failed(&self, id: i64, error: &str) -> Result<(), RepositoryError> {
        let db_manager = self.db_manager.read().await;
        let db = db_manager.get_connection().await?;
        SyncQueueEntity::update_many()
            .col_expr(
                Column::Status,
                Expr::value(SyncQueueStatus::Failed.as_str()),
            )
            .col_expr(Column::Attempts, Expr::col(Column::Attempts).add(1))
            .col_expr(Column::LastError, Expr::value(error))
            .col_expr(
                Column::NextAttemptAt,
                Expr::value(Option::<DateTime<Utc>>::None),
            )
            .filter(Column::Id.eq(id))
            .exec(db)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    /// `cutoff` より前に処理済みになった行を削除し、削除件数を返す。
    ///
    /// `pending` と `failed` の行は消さない。
    pub async fn delete_processed_before(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<u64, RepositoryError> {
        let db_manager = self.db_manager.read().await;
        let db = db_manager.get_connection().await?;
        let result = SyncQueueEntity::delete_many()
            .filter(Column::Status.eq(SyncQueueStatus::Processed.as_str()))
            .filter(Column::ProcessedAt.lt(cutoff))
            .exec(db)
            .await
            .map_err(db_error)?;
        Ok(result.rows_affected)
    }

    /// 状態ごとの行数（診断・テスト用）
    pub async fn count_by_status(&self, status: SyncQueueStatus) -> Result<u64, RepositoryError> {
        let db_manager = self.db_manager.read().await;
        let db = db_manager.get_connection().await?;
        SyncQueueEntity::find()
            .filter(Column::Status.eq(status.as_str()))
            .count(db)
            .await
            .map_err(db_error)
    }

    /// 1 行を取得する（診断・テスト用）
    pub async fn find_by_id(&self, id: i64) -> Result<Option<Model>, RepositoryError> {
        let db_manager = self.db_manager.read().await;
        let db = db_manager.get_connection().await?;
        SyncQueueEntity::find_by_id(id)
            .one(db)
            .await
            .map_err(db_error)
    }
}
