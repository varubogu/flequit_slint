//! OS abstraction layer for Flequit.
//!
//! Flequit targets Windows, macOS, Linux, Android and iOS from a single
//! codebase. Every OS-dependent behaviour is expressed here as a
//! platform-independent interface, so that no other crate needs
//! `#[cfg(target_os = ...)]`.
//!
//! # Rules
//!
//! - `#[cfg(target_os = ...)]` appears only under [`platform`].
//!   CI enforces this via `./scripts/check-crate-deps.sh`.
//! - Unsupported features are reported through [`Capability`], not through
//!   errors at call time. Query first, hide the affordance if unavailable.
//! - All filesystem access starts from [`AppPaths`]. Never hardcode a path.
//!
//! # Examples
//!
//! ```no_run
//! use flequit_platform::{Capability, Platform};
//!
//! let platform = flequit_platform::current()?;
//! if platform.capabilities().has(Capability::SystemTray) {
//!     // show the "minimise to tray" setting
//! }
//! let db = platform.paths().database_file();
//! # Ok::<(), flequit_platform::PlatformError>(())
//! ```

pub mod capability;
pub mod error;
pub mod file_dialog;
pub mod lifecycle;
pub mod notification;
pub mod paths;
pub mod platform;
pub mod theme;

pub use capability::{Capabilities, Capability, FormFactor};
pub use error::{PlatformError, PlatformResult};
pub use file_dialog::{FileFilter, FileHandle};
pub use lifecycle::{LifecycleEvent, LifecycleObserver};
pub use notification::{NotificationId, NotificationRequest, PermissionState};
pub use paths::AppPaths;
pub use theme::{SystemTheme, SystemThemeWatcher};

use chrono::{DateTime, Utc};

/// The platform-independent interface every backend implements.
///
/// Injected into ViewModels and infrastructure at startup rather than accessed
/// through a global, so tests can substitute a mock.
#[async_trait::async_trait]
pub trait Platform: Send + Sync {
    /// Features available on this platform and build.
    fn capabilities(&self) -> &Capabilities;

    /// Device class hint. Layout must still branch on window width.
    fn form_factor(&self) -> FormFactor;

    /// Application directories.
    fn paths(&self) -> &AppPaths;

    /// Returns the operating system's current colour scheme.
    fn system_theme(&self) -> PlatformResult<SystemTheme> {
        Err(PlatformError::Unsupported("system theme"))
    }

    /// Subscribes to operating-system colour scheme changes.
    fn subscribe_system_theme(&self) -> PlatformResult<Box<dyn SystemThemeWatcher>> {
        Err(PlatformError::Unsupported("system theme notifications"))
    }

    /// Font families installed on the system, sorted and deduplicated.
    ///
    /// Enumerating them touches the filesystem, so callers must treat this as
    /// slow and run it off the UI thread.
    fn available_fonts(&self) -> PlatformResult<Vec<String>> {
        Err(PlatformError::Unsupported("font enumeration"))
    }

    /// Current notification permission state.
    async fn notification_permission(&self) -> PlatformResult<PermissionState>;

    /// Asks the user for notification permission.
    ///
    /// Returns immediately with [`PermissionState::Granted`] on platforms that
    /// do not gate notifications.
    async fn request_notification_permission(&self) -> PlatformResult<PermissionState>;

    /// Shows a local notification.
    async fn notify(&self, request: NotificationRequest) -> PlatformResult<NotificationId>;

    /// Registers a notification for a future UTC timestamp.
    async fn schedule_notification(
        &self,
        request: NotificationRequest,
        scheduled_at: DateTime<Utc>,
    ) -> PlatformResult<NotificationId>;

    /// Cancels a previously scheduled notification.
    async fn cancel_notification(&self, id: &NotificationId) -> PlatformResult<()>;

    /// Opens a file picker.
    ///
    /// Returns `Ok(None)` when the user cancels.
    async fn pick_file(&self, filter: Option<FileFilter>) -> PlatformResult<Option<FileHandle>>;

    /// Opens a save dialog.
    ///
    /// Returns `Ok(None)` when the user cancels.
    async fn save_file(&self, suggested_name: &str) -> PlatformResult<Option<FileHandle>>;

    /// Reads a file the user selected.
    async fn read_file(&self, handle: &FileHandle) -> PlatformResult<Vec<u8>>;

    /// Writes a file the user selected.
    async fn write_file(&self, handle: &FileHandle, contents: &[u8]) -> PlatformResult<()>;

    /// Opens a URL in the default browser.
    fn open_url(&self, url: &str) -> PlatformResult<()>;

    /// Opens a path with the OS default application.
    fn open_path(&self, path: &std::path::Path) -> PlatformResult<()>;

    /// Removes a file permanently.
    ///
    /// Moves it to the OS trash when [`Capability::OsTrash`] is available,
    /// otherwise deletes it outright.
    fn delete_permanently(&self, path: &std::path::Path) -> PlatformResult<()>;
}

/// Builds the platform backend for the current target.
pub fn current() -> PlatformResult<std::sync::Arc<dyn Platform>> {
    platform::build()
}
