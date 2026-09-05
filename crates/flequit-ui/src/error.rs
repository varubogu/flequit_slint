//! Errors surfaced to the user by the UI layer.

use flequit_platform::PlatformError;

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

pub type UiResult<T> = Result<T, UiError>;
