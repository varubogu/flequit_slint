//! Errors returned by the platform abstraction layer.

use std::path::PathBuf;

/// Failures that can occur when calling into OS-specific functionality.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// The feature does not exist on this platform.
    ///
    /// Encountering this in a normal flow means a [`crate::Capability`] check was
    /// skipped. Query capabilities first and hide the affordance instead.
    #[error("feature is not supported on this platform: {0}")]
    Unsupported(&'static str),

    /// The user declined a permission request (notifications, storage, ...).
    #[error("permission denied: {0}")]
    PermissionDenied(&'static str),

    /// The user dismissed a dialog. Not an error condition; do nothing.
    #[error("cancelled by user")]
    Cancelled,

    /// The requested directory could not be resolved on this platform.
    #[error("could not resolve {kind} directory")]
    DirectoryUnavailable { kind: &'static str },

    /// Underlying I/O failure, with the path that caused it when known.
    #[error("io error at {path:?}: {source}")]
    Io {
        path: Option<PathBuf>,
        #[source]
        source: std::io::Error,
    },
}

impl From<std::io::Error> for PlatformError {
    fn from(source: std::io::Error) -> Self {
        Self::Io { path: None, source }
    }
}

pub type PlatformResult<T> = Result<T, PlatformError>;
