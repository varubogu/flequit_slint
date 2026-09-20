use crate::InfrastructureRepositoriesTrait;
use crate::services::project_service;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::project::{PartialProject, Project};
use flequit_model::types::id_types::{ProjectId, UserId};
use flequit_model::types::project_types::ProjectStatus;
use flequit_types::errors::service_error::ServiceError;

pub async fn create_project<R>(
    repositories: &R,
    project: &Project,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match project_service::create_project(repositories, project, user_id).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_project<R>(
    repositories: &R,
    id: &ProjectId,
) -> Result<Option<Project>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match project_service::get_project(repositories, id).await {
        Ok(Some(project)) => Ok(Some(project)),
        Ok(None) => Ok(None),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn search_projects<R>(
    repositories: &R,
    name: Option<&str>,
    status: Option<&ProjectStatus>,
    owner_id: Option<&str>,
    is_archived: Option<bool>,
    limit: Option<i32>,
    offset: Option<i32>,
) -> Result<Vec<Project>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match project_service::search_projects(
        repositories,
        name,
        status,
        owner_id,
        is_archived,
        limit,
        offset,
    )
    .await
    {
        Ok(projects) => Ok(projects),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn update_project<R>(
    repositories: &R,
    project_id: &ProjectId,
    patch: &PartialProject,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match project_service::update_project(repositories, project_id, patch, user_id).await {
        Ok(changed) => Ok(changed),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn delete_project<R>(
    repositories: &R,
    id: &ProjectId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    repositories
        .delete_project_transactionally(id, user_id, timestamp)
        .await?;
    Ok(true)
}

pub async fn restore_project<R>(
    repositories: &R,
    id: &ProjectId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    repositories
        .restore_project_transactionally(id, user_id, timestamp)
        .await?;
    Ok(true)
}
