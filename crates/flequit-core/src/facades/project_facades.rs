use crate::InfrastructureRepositoriesTrait;
use crate::ports::infrastructure_repositories::*;
use crate::services::project_service;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::project::{PartialProject, Project};
use flequit_model::types::id_types::{ProjectId, UserId};
use flequit_model::types::project_types::ProjectStatus;
use flequit_repository::repositories::base_repository_trait::Repository;
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
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
    let automerge = match repositories.automerge_repositories() {
        Some(a) => a,
        None => {
            return Err(ServiceError::InternalError(
                "Automerge repositories not initialized".to_string(),
            ));
        }
    };
    let automerge_guard = automerge.read().await;

    // 1. Automergeから削除済みプロジェクトと子データを取得
    let deleted_project = match automerge_guard
        .projects_repo()
        .get_deleted_project(id)
        .await
    {
        Ok(Some(p)) => p,
        Ok(None) => {
            return Err(ServiceError::NotFound(format!(
                "Project not found or not deleted: {}",
                id
            )));
        }
        Err(error) => return Err(error.into()),
    };

    let deleted_task_lists = match automerge_guard
        .projects_repo()
        .get_deleted_task_lists(id)
        .await
    {
        Ok(lists) => lists,
        Err(error) => return Err(error.into()),
    };

    let deleted_tags = match automerge_guard.projects_repo().get_deleted_tags(id).await {
        Ok(tags) => tags,
        Err(error) => return Err(error.into()),
    };

    let deleted_tasks = match automerge_guard.projects_repo().get_deleted_tasks(id).await {
        Ok(tasks) => tasks,
        Err(error) => return Err(error.into()),
    };

    // 2. SQLiteにプロジェクトを再作成
    if let Err(e) = repositories
        .projects()
        .save(&deleted_project, user_id, timestamp)
        .await
    {
        return Err(e.into());
    }

    // 3. SQLiteにタスクリストを再作成（タスクより先に復元）
    for task_list in &deleted_task_lists {
        if let Err(e) = repositories
            .task_lists()
            .save(id, task_list, user_id, timestamp)
            .await
        {
            if let Err(del_err) = repositories.projects().delete(id).await {
                tracing::error!(
                    "Restore task_list failed and project cleanup also failed: {:?} / {:?}",
                    e,
                    del_err
                );
            }
            return Err(e.into());
        }
    }

    // 4. SQLiteにタグを再作成
    for tag in &deleted_tags {
        if let Err(e) = repositories.tags().save(id, tag, user_id, timestamp).await {
            if let Err(del_err) = repositories.projects().delete(id).await {
                tracing::error!(
                    "Restore tag failed and project cleanup also failed: {:?} / {:?}",
                    e,
                    del_err
                );
            }
            return Err(e.into());
        }
    }

    // 5. SQLiteにタスクを再作成（タスクリストの後）
    for task in &deleted_tasks {
        if let Err(e) = repositories
            .tasks()
            .save(id, task, user_id, timestamp)
            .await
        {
            if let Err(del_err) = repositories.projects().delete(id).await {
                tracing::error!(
                    "Restore task failed and project cleanup also failed: {:?} / {:?}",
                    e,
                    del_err
                );
            }
            return Err(e.into());
        }
    }

    // 6. Automergeでプロジェクトを復元（deleted=false）
    if let Err(e) = automerge_guard
        .projects_repo()
        .restore_project(id, user_id, timestamp)
        .await
    {
        // Automerge復元失敗 → SQLiteから再削除してロールバック
        if let Err(del_err) = repositories.projects().delete(id).await {
            tracing::error!(
                "Failed to restore Automerge and cleanup SQLite also failed: automerge={:?}, sqlite={:?}",
                e,
                del_err
            );
        }
        return Err(e.into());
    }

    // 7. Automergeで子データを復元（エラーはウォーニングのみ）
    if let Err(e) = automerge_guard
        .projects_repo()
        .restore_all_task_lists(id, user_id, timestamp)
        .await
    {
        tracing::warn!(
            "Failed to restore task lists in Automerge (non-fatal): {:?}",
            e
        );
    }
    if let Err(e) = automerge_guard
        .projects_repo()
        .restore_all_tags(id, user_id, timestamp)
        .await
    {
        tracing::warn!("Failed to restore tags in Automerge (non-fatal): {:?}", e);
    }
    if let Err(e) = automerge_guard
        .projects_repo()
        .restore_all_tasks(id, user_id, timestamp)
        .await
    {
        tracing::warn!("Failed to restore tasks in Automerge (non-fatal): {:?}", e);
    }

    Ok(true)
}
