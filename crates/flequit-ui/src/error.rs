//! Errors surfaced to the user by the UI layer.

use flequit_platform::PlatformError;
use flequit_types::errors::repository_error::RepositoryError;
use flequit_types::errors::service_error::ServiceError;

/// A failure that reached the UI and must be shown to the user.
///
/// Carries a stable [`UiError::code`] rather than a message: translation happens
/// in `.slint` via `I18n.error-message(code)`, so no user-facing text is built
/// in Rust. See `docs/ja/develop/design/ui/i18n-system.md`.
#[derive(Debug, thiserror::Error)]
pub enum UiError {
    /// A domain operation failed.
    ///
    /// Holds the raw description for logging only; it is never displayed.
    #[error("domain operation failed: {detail}")]
    Domain { code: &'static str, detail: String },

    /// An OS-level operation failed.
    #[error("platform operation failed: {0}")]
    Platform(#[from] PlatformError),

    /// An identifier coming from the UI could not be parsed.
    #[error("malformed identifier: {0}")]
    MalformedId(String),
}

impl UiError {
    /// Builds a domain error with the i18n code the UI should display.
    pub fn domain(code: &'static str, detail: impl Into<String>) -> Self {
        Self::Domain {
            code,
            detail: detail.into(),
        }
    }

    /// The stable code `I18n.error-message()` maps to translated text.
    ///
    /// # Examples
    ///
    /// ```
    /// use flequit_ui::UiError;
    ///
    /// let err = UiError::domain("task.save-failed", "connection reset");
    /// assert_eq!(err.code(), "task.save-failed");
    /// ```
    pub fn code(&self) -> &'static str {
        match self {
            Self::Domain { code, .. } => code,
            Self::Platform(_) => "platform.failed",
            Self::MalformedId(_) => "input.malformed-id",
        }
    }
}

impl From<ServiceError> for UiError {
    fn from(error: ServiceError) -> Self {
        let code = match &error {
            ServiceError::ValidationError(_) | ServiceError::InvalidArgument(_) => {
                "input.validation-failed"
            }
            ServiceError::NotFound(_) => "entity.not-found",
            ServiceError::Forbidden(_) => "permission.denied",
            ServiceError::InternalError(_) => "internal.failed",
            ServiceError::Repository(repository_error) => match repository_error {
                RepositoryError::NotFound(_) | RepositoryError::UserNotFound(_) => {
                    "entity.not-found"
                }
                RepositoryError::EmailConflict(_)
                | RepositoryError::InvalidOperation(_)
                | RepositoryError::ValidationError(_)
                | RepositoryError::ConstraintViolation(_) => "input.validation-failed",
                _ => "storage.failed",
            },
        };

        Self::domain(code, error.to_string())
    }
}

pub type UiResult<T> = Result<T, UiError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_service_errors_to_stable_ui_codes() {
        let cases = [
            (
                ServiceError::NotFound("task".to_string()),
                "entity.not-found",
            ),
            (
                ServiceError::ValidationError("title".to_string()),
                "input.validation-failed",
            ),
            (
                ServiceError::Repository(RepositoryError::DatabaseError("write".to_string())),
                "storage.failed",
            ),
        ];

        for (error, expected_code) in cases {
            assert_eq!(UiError::from(error).code(), expected_code);
        }
    }

    #[test]
    fn preserves_repository_error_categories() {
        assert_eq!(
            UiError::from(ServiceError::Repository(RepositoryError::NotFound(
                "task".to_string()
            )))
            .code(),
            "entity.not-found"
        );
        assert_eq!(
            UiError::from(ServiceError::Repository(
                RepositoryError::ConstraintViolation("title".to_string())
            ))
            .code(),
            "input.validation-failed"
        );
    }
}
