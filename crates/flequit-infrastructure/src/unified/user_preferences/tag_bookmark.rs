//! タグブックマーク用統合リポジトリ
//!
//! 読み取りは SQLite。書き込みは SQLite と Automerge 同期キューへの登録を
//! 1 トランザクションで行い、Automerge（ユーザードキュメント）へはワーカーが反映する。

use async_trait::async_trait;

use flequit_core::ports::infrastructure_repositories::TagBookmarkRepositoryPort;
use flequit_infrastructure_sqlite::infrastructure::user_preferences::tag_bookmark::TagBookmarkLocalSqliteRepository;
use flequit_model::models::user_preferences::tag_bookmark::TagBookmark;
use flequit_model::types::id_types::{ProjectId, TagBookmarkId, TagId, UserId};
use flequit_types::errors::repository_error::RepositoryError;

use crate::automerge_sync::{AutomergeChange, QueuedSqlite, TagBookmarkChange};

#[derive(Debug)]
pub struct TagBookmarkUnifiedRepository {
    queued_sqlite: QueuedSqlite<TagBookmarkLocalSqliteRepository>,
}

impl TagBookmarkUnifiedRepository {
    pub fn new(queued_sqlite: QueuedSqlite<TagBookmarkLocalSqliteRepository>) -> Self {
        Self { queued_sqlite }
    }

    fn sqlite(&self) -> &TagBookmarkLocalSqliteRepository {
        &self.queued_sqlite.sqlite
    }
}

#[async_trait]
impl TagBookmarkRepositoryPort for TagBookmarkUnifiedRepository {
    async fn create(&self, bookmark: &TagBookmark) -> Result<(), RepositoryError> {
        let txn = self
            .queued_sqlite
            .queue
            .begin(vec![AutomergeChange::TagBookmark(
                TagBookmarkChange::Create {
                    bookmark: bookmark.clone(),
                },
            )])
            .await?;
        let result = self.sqlite().create_with_txn(txn.txn(), bookmark).await;
        txn.finish(result).await
    }

    async fn find_by_id(&self, id: &TagBookmarkId) -> Result<Option<TagBookmark>, RepositoryError> {
        self.sqlite().find_by_id(id).await
    }

    async fn find_by_user_project_tag(
        &self,
        user_id: &UserId,
        project_id: &ProjectId,
        tag_id: &TagId,
    ) -> Result<Option<TagBookmark>, RepositoryError> {
        self.sqlite()
            .find_by_user_project_tag(user_id, project_id, tag_id)
            .await
    }

    async fn find_by_user_and_project(
        &self,
        user_id: &UserId,
        project_id: &ProjectId,
    ) -> Result<Vec<TagBookmark>, RepositoryError> {
        self.sqlite()
            .find_by_user_and_project(user_id, project_id)
            .await
    }

    async fn find_by_user(&self, user_id: &UserId) -> Result<Vec<TagBookmark>, RepositoryError> {
        self.sqlite().find_by_user(user_id).await
    }

    async fn find_by_project_and_tag(
        &self,
        project_id: &ProjectId,
        tag_id: &TagId,
    ) -> Result<Vec<TagBookmark>, RepositoryError> {
        self.sqlite()
            .find_by_project_and_tag(project_id, tag_id)
            .await
    }

    async fn update(&self, bookmark: &TagBookmark) -> Result<(), RepositoryError> {
        let txn = self
            .queued_sqlite
            .queue
            .begin(vec![AutomergeChange::TagBookmark(
                TagBookmarkChange::Update {
                    bookmark: bookmark.clone(),
                },
            )])
            .await?;
        let result = self.sqlite().update_with_txn(txn.txn(), bookmark).await;
        txn.finish(result).await
    }

    async fn update_bulk(&self, bookmarks: &[TagBookmark]) -> Result<(), RepositoryError> {
        let changes = bookmarks
            .iter()
            .map(|bookmark| {
                AutomergeChange::TagBookmark(TagBookmarkChange::Update {
                    bookmark: bookmark.clone(),
                })
            })
            .collect();
        let txn = self.queued_sqlite.queue.begin(changes).await?;
        let result = self
            .sqlite()
            .update_bulk_with_txn(txn.txn(), bookmarks)
            .await;
        txn.finish(result).await
    }

    async fn delete(&self, id: &TagBookmarkId) -> Result<(), RepositoryError> {
        // Automerge ではユーザー・プロジェクト・タグの組で保存しているので、行から引く
        let Some(bookmark) = self.sqlite().find_by_id(id).await? else {
            return Ok(());
        };
        let txn = self
            .queued_sqlite
            .queue
            .begin(vec![AutomergeChange::TagBookmark(
                TagBookmarkChange::Delete {
                    user_id: bookmark.user_id,
                    project_id: bookmark.project_id,
                    tag_id: bookmark.tag_id,
                },
            )])
            .await?;
        let result = self.sqlite().delete_with_txn(txn.txn(), id).await;
        txn.finish(result).await
    }

    async fn get_max_order_index(
        &self,
        user_id: &UserId,
        project_id: &ProjectId,
    ) -> Result<i32, RepositoryError> {
        self.sqlite().get_max_order_index(user_id, project_id).await
    }
}
