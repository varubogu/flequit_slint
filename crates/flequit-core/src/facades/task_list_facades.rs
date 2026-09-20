use crate::InfrastructureRepositoriesTrait;
use crate::services::task_list_service;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::task_list::{PartialTaskList, TaskList};
use flequit_model::types::id_types::{ProjectId, TaskListId, UserId};
use flequit_types::errors::service_error::ServiceError;

pub async fn create_task_list<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_list: &TaskList,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_list_service::create_task_list(repositories, project_id, task_list, user_id).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_task_list<R>(
    repositories: &R,
    project_id: &ProjectId,
    id: &TaskListId,
) -> Result<Option<TaskList>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_list_service::get_task_list(repositories, project_id, id).await {
        Ok(task_list) => Ok(task_list),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn search_task_lists<R>(
    repositories: &R,
    project_id: &ProjectId,
    name: Option<&str>,
    is_archived: Option<bool>,
    order_index: Option<i32>,
    limit: Option<i32>,
    offset: Option<i32>,
) -> Result<Vec<TaskList>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_list_service::search_task_lists(
        repositories,
        project_id,
        name,
        is_archived,
        order_index,
        limit,
        offset,
    )
    .await
    {
        Ok(task_lists) => Ok(task_lists),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn update_task_list<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_list_id: &TaskListId,
    patch: &PartialTaskList,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_list_service::update_task_list(
        repositories,
        project_id,
        task_list_id,
        patch,
        user_id,
    )
    .await
    {
        Ok(changed) => Ok(changed),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn delete_task_list<R>(
    repositories: &R,
    project_id: &ProjectId,
    id: &TaskListId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    repositories
        .delete_task_list_transactionally(project_id, id, user_id, timestamp)
        .await?;
    Ok(true)
}

pub async fn restore_task_list<R>(
    repositories: &R,
    project_id: &ProjectId,
    id: &TaskListId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    repositories
        .restore_task_list_transactionally(project_id, id, user_id, timestamp)
        .await?;
    Ok(true)
}
