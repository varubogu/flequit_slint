use tracing::info;

use crate::InfrastructureRepositoriesTrait;
use crate::ports::infrastructure_repositories::*;
use crate::services::{tag_service, task_service, task_tag_service};
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::tag::Tag;
use flequit_model::models::task_projects::task::{PartialTask, Task};
use flequit_model::models::task_projects::task_tag::TaskTag;
use flequit_model::types::id_types::{ProjectId, TagId, TaskId, UserId};
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_types::errors::service_error::ServiceError;
use uuid::Uuid;

pub async fn create_task<R>(
    repositories: &R,
    project_id: &ProjectId,
    task: &Task,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_service::create_task(repositories, project_id, task, user_id).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_task<R>(
    repositories: &R,
    project_id: &ProjectId,
    id: &TaskId,
) -> Result<Option<Task>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    info!("get_task called with id: {}", id);
    match task_service::get_task(repositories, project_id, id).await {
        Ok(t) => Ok(t),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn search_tasks<R>(
    repositories: &R,
    project_id: &ProjectId,
    condition: &task_service::TaskSearchCondition,
) -> Result<Vec<Task>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_service::search_tasks(repositories, project_id, condition).await {
        Ok(tasks) => Ok(tasks),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn update_task<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
    patch: &PartialTask,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_service::update_task(repositories, project_id, task_id, patch, user_id).await {
        Ok(changed) => Ok(changed),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn delete_task<R>(
    repositories: &R,
    project_id: &ProjectId,
    id: &TaskId,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    repositories
        .delete_task_transactionally(project_id, id, user_id, timestamp)
        .await?;
    Ok(true)
}

pub async fn restore_task<R>(
    repositories: &R,
    project_id: &ProjectId,
    id: &TaskId,
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

    // 1. Automergeから削除済みタスクを取得
    let deleted_task = match automerge_guard
        .projects_repo()
        .get_deleted_task_by_id(project_id, id)
        .await
    {
        Ok(Some(t)) => t,
        Ok(None) => {
            return Err(ServiceError::NotFound(format!(
                "Task not found or not deleted: {}",
                id
            )));
        }
        Err(error) => return Err(error.into()),
    };

    // 2. SQLiteにタスクを再作成
    if let Err(e) = repositories
        .tasks()
        .save(project_id, &deleted_task, user_id, timestamp)
        .await
    {
        return Err(e.into());
    }

    // 3. Automergeでタスクを復元（deleted=false）
    if let Err(e) = automerge_guard
        .projects_repo()
        .restore_task(project_id, id, user_id, timestamp)
        .await
    {
        // Automerge復元失敗 → SQLiteから再削除してロールバック
        if let Err(del_err) = repositories.tasks().delete(project_id, id).await {
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

/// TaskTag facades (moved from tagging_facades.rs)
pub async fn add_task_tag_relation<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
    tag_id: &TagId,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_tag_service::add_task_tag_relation(
        repositories,
        project_id,
        task_id,
        tag_id,
        user_id,
    )
    .await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

// 名前から作成/取得し、紐づけを原子的に行う
pub async fn add_task_tag<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
    tag_name: &str,
    user_id: &UserId,
) -> Result<Tag, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    // 1) 既存タグ検索（完全一致）
    let existing = tag_service::list_tags(repositories, project_id)
        .await?
        .into_iter()
        .find(|tag| tag.name == tag_name);

    // 2) 無ければ作成
    let tag: Tag = if let Some(existing_tag) = existing {
        existing_tag
    } else {
        let now = Utc::now();
        let new_tag = Tag {
            id: TagId::from(Uuid::new_v4()),
            name: tag_name.to_string(),
            color: None,
            order_index: None,
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: *user_id,
        };
        tag_service::create_tag(repositories, project_id, &new_tag, user_id).await?;
        new_tag
    };

    // 3) 関連付け
    task_tag_service::add_task_tag_relation(repositories, project_id, task_id, &tag.id, user_id)
        .await?;
    Ok(tag)
}

pub async fn remove_task_tag_relation<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
    tag_id: &TagId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_tag_service::remove_task_tag_relation(repositories, project_id, task_id, tag_id)
        .await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_tag_ids_by_task_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
) -> Result<Vec<TagId>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_tag_service::get_tag_ids_by_task_id(repositories, project_id, task_id).await {
        Ok(tag_ids) => Ok(tag_ids),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_task_ids_by_tag_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    tag_id: &TagId,
) -> Result<Vec<TaskId>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_tag_service::get_task_ids_by_tag_id(repositories, project_id, tag_id).await {
        Ok(task_ids) => Ok(task_ids),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn update_task_tag_relations<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
    tag_ids: &[TagId],
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_tag_service::update_task_tag_relations(
        repositories,
        project_id,
        task_id,
        tag_ids,
        user_id,
    )
    .await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn remove_all_task_tags_by_task_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_tag_service::remove_all_task_tags_by_task_id(repositories, project_id, task_id).await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn remove_all_task_tags_by_tag_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    tag_id: &TagId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_tag_service::remove_all_task_tags_by_tag_id(repositories, project_id, tag_id).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_all_task_tags<R>(
    repositories: &R,
    project_id: &ProjectId,
) -> Result<Vec<TaskTag>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match task_tag_service::get_all_task_tags(repositories, project_id).await {
        Ok(task_tags) => Ok(task_tags),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}
