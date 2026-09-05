//! System appearance detection and change notifications.

use crate::PlatformResult;

/// The colour scheme requested by the operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemTheme {
    Light,
    Dark,
    Unspecified,
}

/// Receives system appearance changes without exposing an OS-specific watcher.
pub trait SystemThemeWatcher: Send {
    /// Returns the next change, or `None` when no change is pending.
    fn try_recv(&self) -> PlatformResult<Option<SystemTheme>>;
}
