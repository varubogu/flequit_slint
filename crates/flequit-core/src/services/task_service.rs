use std::collections::HashMap;

use crate::InfrastructureRepositoriesTrait;
use chrono::Utc;
use flequit_model::models::task_projects::task::{PartialTask, Task};
use flequit_model::types::id_types::{ProjectId, TaskId, UserId};
use flequit_model::types::task_types::TaskStatus;
use flequit_repository::repositories::project_patchable_trait::ProjectPatchable;
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_types::errors::service_error::ServiceError;

#[derive(Debug, Clone, Default)]
pub struct TaskSearchCondition {
    pub list_id: Option<String>,
    pub status: Option<TaskStatus>,
    pub assigned_user_id: Option<String>,
    pub tag_id: Option<String>,
    pub title: Option<String>,
    pub is_archived: Option<bool>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

/// タスクを作る
///
/// 親を持つタスク（サブタスク）は、親が同じプロジェクトに存在することを確かめ、
/// `list_id` を `None` に揃える（リストへの所属は最上位のタスクだけが持つ）。
pub async fn create_task<R>(
    repositories: &R,
    project_id: &ProjectId,
    task: &Task,
    user_id: &UserId,
) -> Result<(), ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let mut task = task.clone();
    if let Some(parent_id) = task.parent_task_id {
        let tasks = repositories.tasks().find_all(project_id).await?;
        validate_parent(&tasks, &task.id, &parent_id)?;
        task.list_id = None;
    }

    let now = Utc::now();
    repositories
        .tasks()
        .save(project_id, &task, user_id, &now)
        .await?;
    Ok(())
}

/// `task_id` の親を `parent_id` にできるか確かめる
///
/// 親は同じプロジェクトに存在して削除されておらず、`task_id` 自身でもその子孫でもないこと
/// （子孫を親にすると親子が輪になる）。
pub fn validate_parent(
    tasks: &[Task],
    task_id: &TaskId,
    parent_id: &TaskId,
) -> Result<(), ServiceError> {
    if parent_id == task_id {
        return Err(ServiceError::ValidationError(
            "A task cannot be its own parent".to_string(),
        ));
    }
    let parent_of: HashMap<TaskId, Option<TaskId>> = tasks
        .iter()
        .map(|task| (task.id, task.parent_task_id))
        .collect();
    let parent_is_live = tasks
        .iter()
        .any(|task| task.id == *parent_id && !task.deleted);
    if !parent_is_live {
        return Err(ServiceError::NotFound(format!(
            "Parent task not found: {parent_id}"
        )));
    }

    // 親から上へたどって自分に着くなら、自分の子孫を親にしようとしている。
    // 保存済みのデータが輪になっていても止まるよう、たどった数で打ち切る
    let mut ancestor = Some(*parent_id);
    let mut steps = 0;
    while let Some(id) = ancestor {
        if id == *task_id {
            return Err(ServiceError::ValidationError(
                "A task cannot be moved under its own subtask".to_string(),
            ));
        }
        steps += 1;
        if steps > parent_of.len() {
            break;
        }
        ancestor = parent_of.get(&id).copied().flatten();
    }
    Ok(())
}

pub async fn get_task<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
) -> Result<Option<Task>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    Ok(repositories.tasks().find_by_id(project_id, task_id).await?)
}

pub async fn list_tasks<R>(
    repositories: &R,
    project_id: &ProjectId,
) -> Result<Vec<Task>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    Ok(repositories.tasks().find_all(project_id).await?)
}

pub async fn search_tasks<R>(
    repositories: &R,
    project_id: &ProjectId,
    condition: &TaskSearchCondition,
) -> Result<Vec<Task>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let mut tasks = repositories.tasks().find_all(project_id).await?;

    if let Some(list_id) = condition.list_id.as_deref() {
        let list_id = list_id.trim();
        if !list_id.is_empty() {
            tasks.retain(|task| task.list_id.is_some_and(|id| id.to_string() == list_id));
        }
    }

    if let Some(status) = condition.status.as_ref() {
        tasks.retain(|task| task.status == *status);
    }

    if let Some(assigned_user_id) = condition.assigned_user_id.as_deref() {
        let assigned_user_id = assigned_user_id.trim();
        if !assigned_user_id.is_empty() {
            tasks.retain(|task| {
                task.assigned_user_ids
                    .iter()
                    .any(|id| id.to_string() == assigned_user_id)
            });
        }
    }

    if let Some(tag_id) = condition.tag_id.as_deref() {
        let tag_id = tag_id.trim();
        if !tag_id.is_empty() {
            tasks.retain(|task| task.tag_ids.iter().any(|id| id.to_string() == tag_id));
        }
    }

    if let Some(title) = condition.title.as_deref() {
        let title = title.trim().to_lowercase();
        if !title.is_empty() {
            tasks.retain(|task| task.title.to_lowercase().contains(&title));
        }
    }

    if let Some(is_archived) = condition.is_archived {
        tasks.retain(|task| task.is_archived == is_archived);
    }

    let offset = condition.offset.unwrap_or(0).max(0) as usize;
    let limit = condition.limit.unwrap_or(i32::MAX).max(0) as usize;
    let tasks = tasks.into_iter().skip(offset).take(limit).collect();

    Ok(tasks)
}

/// タスクを部分更新する
///
/// 親（`parent_task_id`）や所属リスト（`list_id`）を変えるときは階層の決まりを守らせる。
/// 親を付けるとリストからは外れ、リストへ入れられるのは最上位のタスクだけ。
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
    let mut patch = patch.clone();
    match (patch.parent_task_id, patch.list_id) {
        (Some(Some(parent_id)), _) => {
            let tasks = repositories.tasks().find_all(project_id).await?;
            validate_parent(&tasks, task_id, &parent_id)?;
            patch.list_id = Some(None);
        }
        (None, Some(Some(_))) => {
            let task = repositories
                .tasks()
                .find_by_id(project_id, task_id)
                .await?
                .ok_or_else(|| ServiceError::NotFound(format!("Task not found: {task_id}")))?;
            if !task.is_root() {
                return Err(ServiceError::ValidationError(
                    "Only a top-level task can belong to a task list".to_string(),
                ));
            }
        }
        _ => {}
    }

    let now = Utc::now();
    Ok(repositories
        .tasks()
        .patch(project_id, task_id, &patch, user_id, &now)
        .await?)
}

pub async fn delete_task<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
) -> Result<(), ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    repositories.tasks().delete(project_id, task_id).await?;
    Ok(())
}

pub async fn list_tasks_by_assignee<R>(
    repositories: &R,
    project_id: &str,
    user_id: &str,
) -> Result<Vec<Task>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let project_id_typed = ProjectId::from(project_id.to_string());
    let all_tasks = repositories.tasks().find_all(&project_id_typed).await?;

    // user_idでフィルタリング
    let filtered_tasks = all_tasks
        .into_iter()
        .filter(|task| {
            task.assigned_user_ids
                .iter()
                .any(|id| id.to_string() == user_id)
        })
        .collect();

    Ok(filtered_tasks)
}

pub async fn list_tasks_by_status<R>(
    repositories: &R,
    project_id: &str,
    status: &TaskStatus,
) -> Result<Vec<Task>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let project_id_typed = ProjectId::from(project_id.to_string());
    let all_tasks = repositories.tasks().find_all(&project_id_typed).await?;

    // statusでフィルタリング
    let filtered_tasks = all_tasks
        .into_iter()
        .filter(|task| task.status == *status)
        .collect();

    Ok(filtered_tasks)
}

pub async fn assign_task<R>(
    repositories: &R,
    project_id: &str,
    task_id: &str,
    assignee_id: Option<String>,
    user_id: &UserId,
) -> Result<(), ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    // タスクIDから TaskId 型に変換
    let task_id_typed = TaskId::from(task_id.to_string());
    let project_id_typed = ProjectId::from(project_id.to_string());

    if let Some(mut task) = repositories
        .tasks()
        .find_by_id(&project_id_typed, &task_id_typed)
        .await?
    {
        // プロジェクトIDが一致するかチェック
        // project_idチェックをコメントアウト
        /*if task.project_id.to_string() != project_id {
            return Err(ServiceError::InternalError(
                "Task does not belong to the specified project".to_string(),
            ));
        }*/

        // assignee_idがある場合は追加、ない場合は全てクリア
        if let Some(assignee_id) = assignee_id {
            let user_id = UserId::from(assignee_id);
            // 既に含まれていない場合のみ追加
            if !task.assigned_user_ids.contains(&user_id) {
                task.assigned_user_ids.push(user_id);
            }
        } else {
            // assignee_idがNoneの場合は全てクリア
            task.assigned_user_ids.clear();
        }

        // 更新日時を設定
        task.updated_at = Utc::now();

        // 保存
        let now = Utc::now();
        repositories
            .tasks()
            .save(&project_id_typed, &task, user_id, &now)
            .await?;
    }

    Ok(())
}

pub async fn update_task_status<R>(
    repositories: &R,
    project_id: &str,
    task_id: &str,
    status: &TaskStatus,
    user_id: &UserId,
) -> Result<(), ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    // タスクIDから TaskId 型に変換
    use TaskId;
    let task_id_typed = TaskId::from(task_id.to_string());
    let project_id_typed = ProjectId::from(project_id.to_string());

    if let Some(mut task) = repositories
        .tasks()
        .find_by_id(&project_id_typed, &task_id_typed)
        .await?
    {
        // プロジェクトIDが一致するかチェック
        // project_idチェックをコメントアウト
        /*if task.project_id.to_string() != project_id {
            return Err(ServiceError::InternalError(
                "Task does not belong to the specified project".to_string(),
            ));
        }*/

        // ステータス更新
        task.status = status.clone();

        // 更新日時を設定
        task.updated_at = Utc::now();

        // 保存
        let now = Utc::now();
        repositories
            .tasks()
            .save(&project_id_typed, &task, user_id, &now)
            .await?;
    }

    Ok(())
}

pub async fn update_task_priority<R>(
    repositories: &R,
    project_id: &str,
    task_id: &str,
    priority: i32,
    user_id: &UserId,
) -> Result<(), ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    // タスクIDから TaskId 型に変換
    use TaskId;
    let task_id_typed = TaskId::from(task_id.to_string());
    let project_id_typed = ProjectId::from(project_id.to_string());

    if let Some(mut task) = repositories
        .tasks()
        .find_by_id(&project_id_typed, &task_id_typed)
        .await?
    {
        // プロジェクトIDが一致するかチェック
        // project_idチェックをコメントアウト
        /*if task.project_id.to_string() != project_id {
            return Err(ServiceError::InternalError(
                "Task does not belong to the specified project".to_string(),
            ));
        }*/

        // 優先度更新
        task.priority = priority;

        // 更新日時を設定
        task.updated_at = Utc::now();

        // 保存
        let now = Utc::now();
        repositories
            .tasks()
            .save(&project_id_typed, &task, user_id, &now)
            .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flequit_model::types::id_types::TaskListId;

    fn task(parent_task_id: Option<TaskId>) -> Task {
        let now = Utc::now();
        Task {
            id: TaskId::new(),
            project_id: ProjectId::new(),
            list_id: parent_task_id.is_none().then(TaskListId::new),
            parent_task_id,
            title: "task".to_string(),
            description: None,
            status: TaskStatus::NotStarted,
            priority: 0,
            plan_start_date: None,
            plan_end_date: None,
            do_start_date: None,
            do_end_date: None,
            is_range_date: None,
            recurrence_rule: None,
            reminders: Vec::new(),
            order_index: 0,
            is_archived: false,
            assigned_user_ids: Vec::new(),
            tag_ids: Vec::new(),
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
        }
    }

    #[test]
    fn any_live_task_outside_the_subtree_can_be_a_parent() {
        let root = task(None);
        let child = task(Some(root.id));
        let other = task(None);
        let tasks = vec![root.clone(), child.clone(), other.clone()];

        assert!(validate_parent(&tasks, &child.id, &other.id).is_ok());
        assert!(validate_parent(&tasks, &other.id, &child.id).is_ok());
    }

    #[test]
    fn a_task_cannot_go_under_itself_or_its_descendants() {
        let root = task(None);
        let child = task(Some(root.id));
        let grandchild = task(Some(child.id));
        let tasks = vec![root.clone(), child.clone(), grandchild.clone()];

        assert!(matches!(
            validate_parent(&tasks, &root.id, &root.id),
            Err(ServiceError::ValidationError(_))
        ));
        assert!(matches!(
            validate_parent(&tasks, &root.id, &grandchild.id),
            Err(ServiceError::ValidationError(_))
        ));
    }

    #[test]
    fn a_missing_or_deleted_parent_is_refused() {
        let child = task(None);
        let mut gone = task(None);
        gone.deleted = true;
        let tasks = vec![child.clone(), gone.clone()];

        assert!(matches!(
            validate_parent(&tasks, &child.id, &TaskId::new()),
            Err(ServiceError::NotFound(_))
        ));
        assert!(matches!(
            validate_parent(&tasks, &child.id, &gone.id),
            Err(ServiceError::NotFound(_))
        ));
    }

    #[test]
    fn a_stored_cycle_does_not_hang_the_check() {
        let mut a = task(None);
        let mut b = task(None);
        a.parent_task_id = Some(b.id);
        b.parent_task_id = Some(a.id);
        let outsider = task(None);
        let tasks = vec![a.clone(), b.clone(), outsider.clone()];

        assert!(validate_parent(&tasks, &outsider.id, &a.id).is_ok());
    }
}
