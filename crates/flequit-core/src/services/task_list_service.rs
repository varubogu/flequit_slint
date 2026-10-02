use crate::InfrastructureRepositoriesTrait;
use chrono::Utc;
use flequit_model::models::task_projects::task_list::{PartialTaskList, TaskList};
use flequit_model::types::id_types::{ProjectId, TaskListId, UserId};
use flequit_repository::repositories::project_patchable_trait::ProjectPatchable;
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_types::errors::service_error::ServiceError;

pub async fn create_task_list<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_list: &TaskList,
    user_id: &UserId,
) -> Result<(), ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let now = Utc::now();
    repositories
        .task_lists()
        .save(project_id, task_list, user_id, &now)
        .await?;
    Ok(())
}

pub async fn get_task_list<R>(
    repositories: &R,
    project_id: &ProjectId,
    list_id: &TaskListId,
) -> Result<Option<TaskList>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    Ok(repositories
        .task_lists()
        .find_by_id(project_id, list_id)
        .await?)
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
    let now = Utc::now();
    Ok(repositories
        .task_lists()
        .patch(project_id, task_list_id, patch, user_id, &now)
        .await?)
}

pub async fn delete_task_list<R>(
    repositories: &R,
    project_id: &ProjectId,
    id: &TaskListId,
) -> Result<(), ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    repositories.task_lists().delete(project_id, id).await?;
    Ok(())
}

pub async fn list_task_lists<R>(
    repositories: &R,
    project_id: &ProjectId,
) -> Result<Vec<TaskList>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let all_task_lists = repositories.task_lists().find_all(project_id).await?;

    // project_idでフィルタリングは不要（find_allで既にフィルタされている）
    let mut filtered_lists = all_task_lists;

    // order_indexでソート
    filtered_lists.sort_by_key(|a| a.order_index);

    Ok(filtered_lists)
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
    let mut task_lists = repositories.task_lists().find_all(project_id).await?;

    if let Some(name) = name {
        let name = name.trim().to_lowercase();
        if !name.is_empty() {
            task_lists.retain(|task_list| task_list.name.to_lowercase().contains(&name));
        }
    }

    if let Some(is_archived) = is_archived {
        task_lists.retain(|task_list| task_list.is_archived == is_archived);
    }

    if let Some(order_index) = order_index {
        task_lists.retain(|task_list| task_list.order_index == order_index);
    }

    task_lists.sort_by_key(|a| a.order_index);

    let offset = offset.unwrap_or(0).max(0) as usize;
    let limit = limit.unwrap_or(i32::MAX).max(0) as usize;
    let task_lists = task_lists.into_iter().skip(offset).take(limit).collect();

    Ok(task_lists)
}
