//! 日時関連ファサード
//!
//! このモジュールは日付条件、曜日条件のService層とのインターフェースを提供します。

use crate::InfrastructureRepositoriesTrait;
use crate::services::datetime_service;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::date_condition::DateCondition;
use flequit_model::models::task_projects::weekday_condition::WeekdayCondition;
use flequit_settings::models::datetime_format::DateTimeFormat;
use flequit_settings::models::settings::Settings;
use flequit_types::errors::service_error::ServiceError;

// =============================================================================
// 日付条件関連ファサード
// =============================================================================

pub async fn create_date_condition<R>(
    repositories: &R,
    condition: DateCondition,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match datetime_service::create_date_condition(repositories, condition).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_date_condition<R>(
    repositories: &R,
    condition_id: String,
) -> Result<Option<DateCondition>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match datetime_service::get_date_condition(repositories, &condition_id).await {
        Ok(condition) => Ok(condition),
        Err(error) => Err(error),
    }
}

pub async fn get_all_date_conditions<R>(
    repositories: &R,
) -> Result<Vec<DateCondition>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match datetime_service::get_all_date_conditions(repositories).await {
        Ok(conditions) => Ok(conditions),
        Err(error) => Err(error),
    }
}

pub async fn update_date_condition<R>(
    repositories: &R,
    condition: DateCondition,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match datetime_service::update_date_condition(repositories, condition).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn delete_date_condition<R>(
    repositories: &R,
    condition_id: String,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match datetime_service::delete_date_condition(repositories, &condition_id).await {
        Ok(_) => Ok(true),
        Err(error) => Err(error),
    }
}

pub async fn evaluate_date_condition<R>(
    repositories: &R,
    condition_id: String,
    target_date: DateTime<Utc>,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match datetime_service::evaluate_date_condition(repositories, &condition_id, target_date).await
    {
        Ok(result) => Ok(result),
        Err(error) => Err(error),
    }
}

// =============================================================================
// 曜日条件関連ファサード
// =============================================================================

pub async fn create_weekday_condition<R>(
    repositories: &R,
    condition: WeekdayCondition,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match datetime_service::create_weekday_condition(repositories, condition).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn get_weekday_condition<R>(
    repositories: &R,
    condition_id: String,
) -> Result<Option<WeekdayCondition>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match datetime_service::get_weekday_condition(repositories, &condition_id).await {
        Ok(condition) => Ok(condition),
        Err(error) => Err(error),
    }
}

pub async fn get_all_weekday_conditions<R>(
    repositories: &R,
) -> Result<Vec<WeekdayCondition>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match datetime_service::get_all_weekday_conditions(repositories).await {
        Ok(conditions) => Ok(conditions),
        Err(error) => Err(error),
    }
}

pub async fn update_weekday_condition<R>(
    repositories: &R,
    condition: WeekdayCondition,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match datetime_service::update_weekday_condition(repositories, condition).await {
        Ok(_) => Ok(true),
        Err(ServiceError::ValidationError(message)) => Err(ServiceError::ValidationError(message)),
        Err(error) => Err(error),
    }
}

pub async fn delete_weekday_condition<R>(
    repositories: &R,
    condition_id: String,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match datetime_service::delete_weekday_condition(repositories, &condition_id).await {
        Ok(_) => Ok(true),
        Err(error) => Err(error),
    }
}

pub async fn evaluate_weekday_condition<R>(
    repositories: &R,
    condition_id: String,
    target_date: DateTime<Utc>,
) -> Result<bool, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    match datetime_service::evaluate_weekday_condition(repositories, &condition_id, target_date)
        .await
    {
        Ok(result) => Ok(result),
        Err(error) => Err(error),
    }
}

// =============================================================================
// 日時フォーマット関連ファサード
// =============================================================================

pub async fn update_datetime_format(
    settings: &mut Settings,
    format: DateTimeFormat,
) -> Result<DateTimeFormat, ServiceError> {
    if let Some(existing) = settings
        .datetime_formats
        .iter_mut()
        .find(|existing| existing.id == format.id)
    {
        *existing = format.clone();
        return Ok(format);
    }

    Err(ServiceError::NotFound(format!(
        "datetime_format id not found: {}",
        format.id
    )))
}

pub async fn delete_datetime_format(settings: &mut Settings, id: &str) -> Result<(), ServiceError> {
    let before_len = settings.datetime_formats.len();
    settings.datetime_formats.retain(|f| f.id != id);

    if settings.datetime_formats.len() == before_len {
        return Err(ServiceError::NotFound(format!(
            "datetime_format id not found: {}",
            id
        )));
    }

    Ok(())
}
