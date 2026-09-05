use crate::InfrastructureRepositoriesTrait;
use crate::ports::infrastructure_repositories::*;
use crate::services::tag_service;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::tag::{PartialTag, Tag};
use flequit_model::types::id_types::{ProjectId, TagId, UserId};
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_types::errors::service_error::ServiceError;

pub async fn create_tag<R>(
    repositories: &R,
    project_id: &ProjectId,
    tag: &Tag,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match tag_service::create_tag(repositories, project_id, tag, user_id).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_tag<R>(
    repositories: &R,
    project_id: &ProjectId,
    id: &TagId,
) -> Result<Option<Tag>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match tag_service::get_tag(repositories, project_id, id).await {
        Ok(Some(tag)) => Ok(Some(tag)),
        Ok(None) => Ok(None),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn search_tags<R>(
    repositories: &R,
    project_id: &ProjectId,
    name: Option<&str>,
    limit: Option<i32>,
    offset: Option<i32>,
) -> Result<Vec<Tag>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match tag_service::search_tags(repositories, project_id, name, limit, offset).await {
        Ok(tags) => Ok(tags),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn update_tag<R>(
    repositories: &R,
    project_id: &ProjectId,
    tag_id: &TagId,
    patch: &PartialTag,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match tag_service::update_tag(repositories, project_id, tag_id, patch, user_id).await {
        Ok(changed) => Ok(changed),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn delete_tag<R>(
    repositories: &R,
    project_id: &ProjectId,
    id: &TagId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    repositories
        .delete_tag_transactionally(project_id, id, user_id, timestamp)
        .await?;
    Ok(true)
}

pub async fn restore_tag<R>(
    repositories: &R,
    project_id: &ProjectId,
    id: &TagId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let automerge = match repositories.automerge_repositories() {
        Some(a) => a,
        None => {
            return Err(ServiceError::InternalError(
                "Automerge repositories not initialized".to_string(),
            ));
        }
    };
    let automerge_guard = automerge.read().await;

    // 1. Automergeから削除済みタグを取得
    let deleted_tag = match automerge_guard
        .projects_repo()
        .get_deleted_tag_by_id(project_id, id)
        .await
    {
        Ok(Some(t)) => t,
        Ok(None) => {
            return Err(ServiceError::NotFound(format!(
                "Tag not found or not deleted: {}",
                id
            )));
        }
        Err(error) => return Err(error.into()),
    };

    // 2. SQLiteにタグを再作成
    if let Err(e) = repositories
        .tags()
        .save(project_id, &deleted_tag, user_id, timestamp)
        .await
    {
        return Err(e.into());
    }

    // 3. Automergeでタグを復元（deleted=false）
    if let Err(e) = automerge_guard
        .projects_repo()
        .restore_tag(project_id, id, user_id, timestamp)
        .await
    {
        // Automerge復元失敗 → SQLiteから再削除してロールバック
        if let Err(del_err) = repositories.tags().delete(project_id, id).await {
            tracing::error!(
                "Failed to restore Automerge and cleanup SQLite also failed: automerge={:?}, sqlite={:?}",
                e,
                del_err
            );
        }
        return Err(e.into());
    }

    Ok(true)
}
