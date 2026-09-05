use crate::InfrastructureRepositoriesTrait;
use crate::services::subtask_assignment_service as service;
use flequit_model::models::task_projects::subtask_assignment::SubTaskAssignment;
use flequit_model::types::id_types::{ProjectId, SubTaskId, UserId};
use flequit_types::errors::service_error::ServiceError;

pub async fn add<R>(
    repositories: &R,
    project_id: &ProjectId,
    subtask_id: &SubTaskId,
    assigned_user_id: &UserId,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::add_subtask_assignment(
        repositories,
        project_id,
        subtask_id,
        assigned_user_id,
        user_id,
    )
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
    subtask_id: &SubTaskId,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::remove_subtask_assignment(repositories, project_id, subtask_id, user_id).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_user_ids_by_subtask_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    subtask_id: &SubTaskId,
) -> Result<Vec<UserId>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::get_user_ids_by_subtask_id(repositories, project_id, subtask_id).await {
        Ok(user_ids) => Ok(user_ids),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_subtask_ids_by_user_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    user_id: &UserId,
) -> Result<Vec<SubTaskId>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::get_subtask_ids_by_user_id(repositories, project_id, user_id).await {
        Ok(subtask_ids) => Ok(subtask_ids),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn update<R>(
    repositories: &R,
    project_id: &ProjectId,
    subtask_id: &SubTaskId,
    user_ids: &[UserId],
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::update_subtask_assignments(
        repositories,
        project_id,
        subtask_id,
        user_ids,
        user_id,
    )
    .await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn remove_all_by_subtask_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    subtask_id: &SubTaskId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::remove_all_subtask_assignments_by_subtask_id(
        repositories,
        project_id,
        subtask_id,
    )
    .await
    {
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
    match service::remove_all_subtask_assignments_by_user_id(repositories, project_id, user_id)
        .await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_all<R>(
    repositories: &R,
    project_id: &ProjectId,
) -> Result<Vec<SubTaskAssignment>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match service::get_all_subtask_assignments(repositories, project_id).await {
        Ok(assignments) => Ok(assignments),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}
