use crate::InfrastructureRepositoriesTrait;
use crate::services::tag_bookmark_service;
use flequit_model::models::user_preferences::tag_bookmark::TagBookmark;
use flequit_model::types::id_types::{ProjectId, TagBookmarkId, TagId, UserId};
use flequit_types::errors::service_error::ServiceError;

pub async fn create_bookmark<R>(
    repositories: &R,
    bookmark: &TagBookmark,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    tag_bookmark_service::create_bookmark(repositories, bookmark)
        .await
        .map(|_| true)
}

pub async fn list_bookmarks_by_user<R>(
    repositories: &R,
    user_id: &UserId,
) -> Result<Vec<TagBookmark>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    tag_bookmark_service::list_bookmarks_by_user(repositories, user_id).await
}

pub async fn delete_bookmark<R>(
    repositories: &R,
    bookmark_id: &TagBookmarkId,
    user_id: &UserId,
    project_id: &ProjectId,
    tag_id: &TagId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    tag_bookmark_service::delete_bookmark(repositories, bookmark_id, user_id, project_id, tag_id)
        .await
        .map(|_| true)
}
