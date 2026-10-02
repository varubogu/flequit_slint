//! タスクアサインメント用UnifiedRepositoryビルダー
//!
//! TaskAssignment（サブタスクを含む）エンティティのUnifiedRepositoryを構築するメソッドを提供する

use super::{UnifiedManager, get_default_automerge_path};
use crate::automerge_sync::QueuedSqlite;
use crate::unified::TaskAssignmentUnifiedRepository;
use flequit_infrastructure_automerge::infrastructure::task_projects::task_assignments::TaskAssignmentLocalAutomergeRepository;
use flequit_infrastructure_sqlite::infrastructure::task_projects::task_assignments::TaskAssignmentLocalSqliteRepository;

impl UnifiedManager {
    /// TaskAssignment用UnifiedRepositoryを構築
    pub async fn create_task_assignment_unified_repository(
        &self,
    ) -> Result<TaskAssignmentUnifiedRepository, Box<dyn std::error::Error>> {
        let mut repo = TaskAssignmentUnifiedRepository::default();

        // SQLiteリポジトリの設定
        if self.config.sqlite_search_enabled || self.config.sqlite_storage_enabled {
            let db_manager = self.database_manager()?;

            if self.config.sqlite_search_enabled {
                let sqlite_repo = TaskAssignmentLocalSqliteRepository::new(db_manager.clone());
                repo.add_sqlite_for_search(sqlite_repo);
                tracing::info!("SQLiteリポジトリを検索用に追加しました（TaskAssignment）");
            }

            if self.config.sqlite_storage_enabled {
                let sqlite_repo = TaskAssignmentLocalSqliteRepository::new(db_manager.clone());
                repo.set_queued_sqlite(QueuedSqlite::new(sqlite_repo, self.sync_queue()?));
                tracing::info!("SQLiteリポジトリを保存用に追加しました（TaskAssignment）");
            }
        }

        // Automergeリポジトリの設定
        // SQLite があるときは同期キュー経由で反映するため、Automerge へ直接は書かない
        if self.config.automerge_storage_enabled && !self.config.sqlite_storage_enabled {
            let automerge_repo = if let Some(doc_manager) = &self.shared_document_manager {
                TaskAssignmentLocalAutomergeRepository::new_with_manager(doc_manager.clone())
                    .await?
            } else {
                let base_path =
                    get_default_automerge_path().ok_or("Failed to get default Automerge path")?;
                TaskAssignmentLocalAutomergeRepository::new(base_path).await?
            };

            repo.add_automerge_for_save(automerge_repo);
            tracing::info!("Automergeリポジトリを保存用に追加しました（TaskAssignment）");
        }

        tracing::info!(
            "TaskAssignmentUnifiedRepository構築完了 - 保存用: {} 検索用: {} リポジトリ",
            repo.save_repositories_count(),
            repo.search_repositories_count()
        );

        Ok(repo)
    }
}
