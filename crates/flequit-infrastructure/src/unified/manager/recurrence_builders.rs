//! 繰り返しルール・タスク繰り返し用UnifiedRepositoryビルダー
//!
//! RecurrenceRule、TaskRecurrence エンティティのUnifiedRepositoryを構築するメソッドを提供する

use super::{UnifiedManager, get_default_automerge_path};
use crate::automerge_sync::QueuedSqlite;
use crate::unified::{RecurrenceRuleUnifiedRepository, TaskRecurrenceUnifiedRepository};
use flequit_infrastructure_automerge::infrastructure::task_projects::{
    recurrence_rule::RecurrenceRuleLocalAutomergeRepository,
    task_recurrence::TaskRecurrenceLocalAutomergeRepository,
};
use flequit_infrastructure_sqlite::infrastructure::task_projects::{
    recurrence_rule::RecurrenceRuleLocalSqliteRepository,
    task_recurrence::TaskRecurrenceLocalSqliteRepository,
};

impl UnifiedManager {
    /// RecurrenceRule用UnifiedRepositoryを構築
    pub async fn create_recurrence_rule_unified_repository(
        &self,
    ) -> Result<RecurrenceRuleUnifiedRepository, Box<dyn std::error::Error>> {
        let mut repo = RecurrenceRuleUnifiedRepository::default();

        // SQLiteリポジトリの設定
        if self.config.sqlite_search_enabled || self.config.sqlite_storage_enabled {
            let db_manager = self.database_manager()?;

            if self.config.sqlite_search_enabled {
                let sqlite_repo = RecurrenceRuleLocalSqliteRepository::new(db_manager.clone());
                repo.add_sqlite_for_search(sqlite_repo);
                tracing::info!("SQLiteリポジトリを検索用に追加しました（RecurrenceRule）");
            }

            if self.config.sqlite_storage_enabled {
                let sqlite_repo = RecurrenceRuleLocalSqliteRepository::new(db_manager.clone());
                repo.set_queued_sqlite(QueuedSqlite::new(sqlite_repo, self.sync_queue()?));
                tracing::info!("SQLiteリポジトリを保存用に追加しました（RecurrenceRule）");
            }
        }

        // Automergeリポジトリの設定
        // SQLite があるときは同期キュー経由で反映するため、Automerge へ直接は書かない
        if self.config.automerge_storage_enabled && !self.config.sqlite_storage_enabled {
            let automerge_repo = if let Some(doc_manager) = &self.shared_document_manager {
                RecurrenceRuleLocalAutomergeRepository::new_with_manager(doc_manager.clone())
                    .await?
            } else {
                let base_path =
                    get_default_automerge_path().ok_or("Failed to get default Automerge path")?;
                RecurrenceRuleLocalAutomergeRepository::new(base_path).await?
            };

            repo.add_automerge_for_save(automerge_repo);
            tracing::info!("Automergeリポジトリを保存用に追加しました（RecurrenceRule）");
        }

        tracing::info!(
            "RecurrenceRuleUnifiedRepository構築完了 - 保存用: {} 検索用: {} リポジトリ",
            repo.save_repositories_count(),
            repo.search_repositories_count()
        );

        Ok(repo)
    }

    /// TaskRecurrence用UnifiedRepositoryを構築
    pub async fn create_task_recurrence_unified_repository(
        &self,
    ) -> Result<TaskRecurrenceUnifiedRepository, Box<dyn std::error::Error>> {
        let mut repo = TaskRecurrenceUnifiedRepository::default();

        // SQLiteリポジトリの設定
        if self.config.sqlite_search_enabled || self.config.sqlite_storage_enabled {
            let db_manager = self.database_manager()?;

            if self.config.sqlite_search_enabled {
                let sqlite_repo = TaskRecurrenceLocalSqliteRepository::new(db_manager.clone());
                repo.add_sqlite_for_search(sqlite_repo);
                tracing::info!("SQLiteリポジトリを検索用に追加しました（TaskRecurrence）");
            }

            if self.config.sqlite_storage_enabled {
                let sqlite_repo = TaskRecurrenceLocalSqliteRepository::new(db_manager.clone());
                repo.set_queued_sqlite(QueuedSqlite::new(sqlite_repo, self.sync_queue()?));
                tracing::info!("SQLiteリポジトリを保存用に追加しました（TaskRecurrence）");
            }
        }

        // Automergeリポジトリの設定
        // SQLite があるときは同期キュー経由で反映するため、Automerge へ直接は書かない
        if self.config.automerge_storage_enabled && !self.config.sqlite_storage_enabled {
            let automerge_repo = if let Some(doc_manager) = &self.shared_document_manager {
                TaskRecurrenceLocalAutomergeRepository::new_with_manager(doc_manager.clone())
                    .await?
            } else {
                let base_path =
                    get_default_automerge_path().ok_or("Failed to get default Automerge path")?;
                TaskRecurrenceLocalAutomergeRepository::new(base_path).await?
            };

            repo.add_automerge_for_save(automerge_repo);
            tracing::info!("Automergeリポジトリを保存用に追加しました（TaskRecurrence）");
        }

        tracing::info!(
            "TaskRecurrenceUnifiedRepository構築完了 - 保存用: {} 検索用: {} リポジトリ",
            repo.save_repositories_count(),
            repo.search_repositories_count()
        );

        Ok(repo)
    }
}
