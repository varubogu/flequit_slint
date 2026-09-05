//! 繰り返し関連ファサード
//!
//! このモジュールは繰り返しルール、調整、詳細、タスク・サブタスク関連付けの
//! Service層とのインターフェースを提供します。

use crate::InfrastructureRepositoriesTrait;
use crate::services::recurrence_service;
use flequit_model::{
    models::task_projects::{
        recurrence_adjustment::RecurrenceAdjustment,
        recurrence_details::RecurrenceDetails,
        recurrence_rule::{PartialRecurrenceRule, RecurrenceRule},
        subtask_recurrence::SubTaskRecurrence,
        task_recurrence::TaskRecurrence,
    },
    types::id_types::{ProjectId, RecurrenceRuleId, SubTaskId, TaskId, UserId},
};
use flequit_types::errors::service_error::ServiceError;

// 実際のドメインモデルを使用（Commandモデルは削除）
// TODO: 繰り返しファサードで使用すべき適切なドメインモデルを確認して置換

// =============================================================================
// 繰り返しルール関連ファサード
// =============================================================================

/// 繰り返しルールを作成します。
pub async fn create_recurrence_rule<R>(
    repositories: &R,
    project_id: &ProjectId,
    rule: RecurrenceRule,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::create_recurrence_rule(repositories, project_id, rule, user_id).await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

/// 繰り返しルールを取得します。
pub async fn get_recurrence_rule<R>(
    repositories: &R,
    project_id: &ProjectId,
    rule_id: String,
) -> Result<Option<RecurrenceRule>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::get_recurrence_rule(repositories, project_id, &rule_id).await {
        Ok(rule) => Ok(rule),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

/// すべての繰り返しルールを取得します。
pub async fn get_all_recurrence_rules<R>(
    repositories: &R,
    project_id: &ProjectId,
) -> Result<Vec<RecurrenceRule>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::get_all_recurrence_rules(repositories, project_id).await {
        Ok(rules) => Ok(rules),
        Err(error) => Err(error),
    }
}

/// 繰り返しルールを更新します。
pub async fn update_recurrence_rule<R>(
    repositories: &R,
    project_id: &ProjectId,
    rule_id: &RecurrenceRuleId,
    patch: &PartialRecurrenceRule,
    user_id: &UserId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::update_recurrence_rule(
        repositories,
        project_id,
        rule_id,
        patch,
        user_id,
    )
    .await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

/// 繰り返しルールを削除します。
pub async fn delete_recurrence_rule<R>(
    repositories: &R,
    project_id: &ProjectId,
    rule_id: String,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::delete_recurrence_rule(repositories, project_id, &rule_id).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

// =============================================================================
// 繰り返し調整関連ファサード
// =============================================================================

/// 繰り返し調整を作成します。
pub async fn create_recurrence_adjustment<R>(
    repositories: &R,
    project_id: &ProjectId,
    adjustment: RecurrenceAdjustment,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::create_recurrence_adjustment(repositories, project_id, adjustment)
        .await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

/// 繰り返しルールIDによる調整一覧を取得します。
pub async fn get_recurrence_adjustments_by_rule_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    rule_id: String,
) -> Result<Vec<RecurrenceAdjustment>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::get_recurrence_adjustments_by_rule_id(
        repositories,
        project_id,
        &rule_id,
    )
    .await
    {
        Ok(adjustments) => Ok(adjustments),
        Err(error) => Err(error),
    }
}

/// 繰り返し調整を削除します。
pub async fn delete_recurrence_adjustment<R>(
    repositories: &R,
    project_id: &ProjectId,
    adjustment_id: String,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::delete_recurrence_adjustment(repositories, project_id, &adjustment_id)
        .await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

// =============================================================================
// 繰り返し詳細関連ファサード
// =============================================================================

/// 繰り返し詳細を作成します。
pub async fn create_recurrence_details<R>(
    repositories: &R,
    project_id: &ProjectId,
    details: RecurrenceDetails,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::create_recurrence_details(repositories, project_id, details).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

/// 繰り返しルールIDによる詳細を取得します。
pub async fn get_recurrence_details_by_rule_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    rule_id: String,
) -> Result<Option<RecurrenceDetails>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::get_recurrence_details_by_rule_id(repositories, project_id, &rule_id)
        .await
    {
        Ok(details) => Ok(details),
        Err(error) => Err(error),
    }
}

/// 繰り返し詳細を更新します。
pub async fn update_recurrence_details<R>(
    repositories: &R,
    project_id: &ProjectId,
    details: RecurrenceDetails,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::update_recurrence_details(repositories, project_id, details).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

/// 繰り返し詳細を削除します。
pub async fn delete_recurrence_details<R>(
    repositories: &R,
    project_id: &ProjectId,
    details_id: &RecurrenceRuleId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::delete_recurrence_details(repositories, project_id, details_id).await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

// =============================================================================
// タスク繰り返し関連付けファサード
// =============================================================================

/// タスクに繰り返しルールを関連付けます。
pub async fn create_task_recurrence<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
    recurrence_rule_id: &RecurrenceRuleId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::create_task_recurrence(
        repositories,
        project_id,
        task_id,
        recurrence_rule_id,
    )
    .await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

/// タスクIDによる繰り返し関連付けを取得します。
pub async fn get_task_recurrence_by_task_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
) -> Result<Option<TaskRecurrence>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::get_task_recurrence_by_task_id(repositories, project_id, task_id)
        .await
    {
        Ok(task_recurrence) => Ok(task_recurrence),
        Err(error) => Err(error),
    }
}

/// タスクの繰り返し関連付けを削除します。
pub async fn delete_task_recurrence<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::delete_task_recurrence(repositories, project_id, task_id).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

// =============================================================================
// サブタスク繰り返し関連付けファサード
// =============================================================================

/// サブタスクに繰り返しルールを関連付けます。
pub async fn create_subtask_recurrence<R>(
    repositories: &R,
    project_id: &ProjectId,
    subtask_id: &SubTaskId,
    recurrence_rule_id: &RecurrenceRuleId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::create_subtask_recurrence(
        repositories,
        project_id,
        subtask_id,
        recurrence_rule_id,
    )
    .await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

/// サブタスクIDによる繰り返し関連付けを取得します。
pub async fn get_subtask_recurrence_by_subtask_id<R>(
    repositories: &R,
    project_id: &ProjectId,
    subtask_id: &SubTaskId,
) -> Result<Option<SubTaskRecurrence>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::get_subtask_recurrence_by_subtask_id(
        repositories,
        project_id,
        subtask_id,
    )
    .await
    {
        Ok(subtask_recurrence) => Ok(subtask_recurrence),
        Err(error) => Err(error),
    }
}

/// サブタスクの繰り返し関連付けを削除します。
pub async fn delete_subtask_recurrence<R>(
    repositories: &R,
    project_id: &ProjectId,
    subtask_id: &SubTaskId,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match recurrence_service::delete_subtask_recurrence(repositories, project_id, subtask_id).await
    {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}
