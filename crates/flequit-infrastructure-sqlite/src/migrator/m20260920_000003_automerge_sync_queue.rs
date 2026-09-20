//! Automerge 同期キューのテーブル
//!
//! SQLite への書き込みと同じトランザクションで Automerge へ反映する変更を記録し、
//! バックグラウンド処理がそれを Automerge へ適用する。
//! 設計: `docs/ja/develop/design/data/automerge-sync-queue.md`

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let connection = manager.get_connection();

        // AUTOINCREMENT: 処理済みの行を消しても id を再利用させない。
        // id の昇順がそのまま適用順になるため。
        connection
            .execute_unprepared(
                r#"
                CREATE TABLE IF NOT EXISTS automerge_sync_queue (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    document_key VARCHAR NOT NULL,
                    change_kind VARCHAR NOT NULL,
                    payload TEXT NOT NULL,
                    status VARCHAR NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending', 'processed', 'failed')),
                    attempts INTEGER NOT NULL DEFAULT 0,
                    last_error TEXT,
                    next_attempt_at TIMESTAMP,
                    created_at TIMESTAMP NOT NULL,
                    processed_at TIMESTAMP
                );
                "#,
            )
            .await?;

        // 未処理の行を id 順に読む（ワーカー）
        connection
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_automerge_sync_queue_status_id \
                 ON automerge_sync_queue(status, id);",
            )
            .await?;

        // 処理済みの古い行を消す（クリーンアップ）
        connection
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_automerge_sync_queue_status_processed_at \
                 ON automerge_sync_queue(status, processed_at);",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS automerge_sync_queue;")
            .await?;
        Ok(())
    }
}
