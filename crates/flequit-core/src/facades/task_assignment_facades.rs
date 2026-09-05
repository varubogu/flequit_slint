use crate::InfrastructureRepositoriesTrait;
use crate::services::task_assignment_service as service;
use flequit_model::models::task_projects::task_assignment::TaskAssignment;
use flequit_model::types::id_types::{ProjectId, TaskId, UserId};
use flequit_types::errors::service_error::ServiceError;

pub async fn add<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
    assigned_user_id: &UserId,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::add_task_assignment(repositories, project_id, task_id, assigned_user_id, user_id)
        .await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn remove<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::remove_task_assignment(repositories, project_id, task_id, user_id).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_user_ids_by_task_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
) -> Result<Vec<UserId>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::get_user_ids_by_task_id(repositories, project_id, task_id).await {
        Ok(user_ids) => Ok(user_ids),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_task_ids_by_user_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    user_id: &UserId,
) -> Result<Vec<TaskId>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::get_task_ids_by_user_id(repositories, project_id, user_id).await {
        Ok(task_ids) => Ok(task_ids),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn update<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
    user_ids: &[UserId],
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::update_task_assignments(repositories, project_id, task_id, user_ids, user_id)
        .await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn remove_all_by_task_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::remove_all_task_assignments_by_task_id(repositories, project_id, task_id).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn remove_all_by_user_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::remove_all_task_assignments_by_user_id(repositories, project_id, user_id).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_all<R>(
    repositories: &R,
    project_id: &ProjectId,
) -> Result<Vec<TaskAssignment>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::get_all_task_assignments(repositories, project_id).await {
        Ok(task_assignments) => Ok(task_assignments),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}
