mod delete;
mod recovery;
mod update;

pub(super) use delete::delete;
pub(super) use recovery::recover;
pub(super) use update::update;

use flequit_infrastructure_sqlite::infrastructure::operation_journal::OperationStatus;
use flequit_types::errors::repository_error::RepositoryError;

use super::InfrastructureRepositories;

async fn mark_status(
    repositories: &InfrastructureRepositories,
    operation_id: &str,
    status: OperationStatus,
) -> Result<(), RepositoryError> {
    let sqlite = repositories
        .unified_manager
        .sqlite_repositories()
        .ok_or_else(|| {
            RepositoryError::ConfigurationError("SQLite repositories not initialized".to_string())
        })?;
    sqlite
        .read()
        .await
        .operation_journal()
        .mark_status(operation_id, status)
        .await
}

fn prepared_recovery_error(
    operation_id: &str,
    operation_error: &RepositoryError,
    recovery_error: &RepositoryError,
) -> RepositoryError {
    RepositoryError::TransactionError(format!(
        "{operation_error}; {recovery_error}; operation {operation_id} remains prepared for startup recovery"
    ))
}

fn result_summary<T>(result: &Result<T, RepositoryError>) -> String {
    match result {
        Ok(_) => "succeeded".to_string(),
        Err(error) => format!("failed ({error})"),
    }
}
