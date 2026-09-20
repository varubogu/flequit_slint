use crate::InfrastructureRepositoriesTrait;
use crate::services::tag_service;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::tag::{PartialTag, Tag};
use flequit_model::types::id_types::{ProjectId, TagId, UserId};
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
    repositories
        .restore_tag_transactionally(project_id, id, user_id, timestamp)
        .await?;
    Ok(true)
}
