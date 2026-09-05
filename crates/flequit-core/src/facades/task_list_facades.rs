use crate::InfrastructureRepositoriesTrait;
use crate::ports::infrastructure_repositories::*;
use crate::services::task_list_service;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::task_list::{PartialTaskList, TaskList};
use flequit_model::types::id_types::{ProjectId, TaskListId, UserId};
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
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
    let automerge = match repositories.automerge_repositories() {
        Some(a) => a,
        None => {
            return Err(ServiceError::InternalError(
                "Automerge repositories not initialized".to_string(),
            ));
        }
    };
    let automerge_guard = automerge.read().await;

    // 1. Automergeから削除済みタスクリストを取得
    let deleted_task_list = match automerge_guard
        .projects_repo()
        .get_deleted_task_list_by_id(project_id, id)
        .await
    {
        Ok(Some(tl)) => tl,
        Ok(None) => {
            return Err(ServiceError::NotFound(format!(
                "Task list not found or not deleted: {}",
                id
            )));
        }
        Err(error) => return Err(error.into()),
    };

    // 2. SQLiteにタスクリストを再作成
    if let Err(e) = repositories
        .task_lists()
        .save(project_id, &deleted_task_list, user_id, timestamp)
        .await
    {
        return Err(e.into());
    }

    // 3. Automergeでタスクリストを復元（deleted=false）
    if let Err(e) = automerge_guard
        .projects_repo()
        .restore_task_list(project_id, id, user_id, timestamp)
        .await
    {
        // Automerge復元失敗 → SQLiteから再削除してロールバック
        if let Err(del_err) = repositories.task_lists().delete(project_id, id).await {
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
