//! Android backend.
//!
//! **Phase 2 — not yet implemented.** The structure exists so that the crate
//! compiles for `aarch64-linux-android` in CI and so that the capability set is
//! recorded in one place.
//!
//! Remaining work:
//!
//! - Obtain the sandbox root from `android-activity` (`internal_data_path`)
//!   instead of the placeholder below.
//! - Notifications via `NotificationManager` and a notification channel,
//!   including the runtime permission request.
//! - File selection via the Storage Access Framework (`ACTION_OPEN_DOCUMENT`),
//!   returning [`FileHandle::Opaque`] with the content URI.
//! - `open_url` / `open_path` via `Intent.ACTION_VIEW`.

use std::path::{Path, PathBuf};

use crate::{
    AppPaths, Capabilities, Capability, FileFilter, FileHandle, FormFactor, NotificationId,
    NotificationRequest, PermissionState, Platform, PlatformError, PlatformResult,
};

pub struct AndroidPlatform {
    capabilities: Capabilities,
    paths: AppPaths,
}

impl AndroidPlatform {
    pub fn new() -> PlatformResult<Self> {
        // TODO(phase2): replace with the sandbox root reported by android-activity.
        let root = std::env::var_os("FLEQUIT_ANDROID_DATA_ROOT")
            .map(PathBuf::from)
            .ok_or(PlatformError::DirectoryUnavailable {
                kind: "application",
            })?;

        let paths = AppPaths::rooted_at(root);
        paths.ensure_all_exist()?;

        Ok(Self {
            capabilities: Capabilities::from_supported([
                Capability::LocalNotification,
                Capability::FilePicker,
                Capability::BackgroundSync,
            ]),
            paths,
        })
    }
}

#[async_trait::async_trait]
impl Platform for AndroidPlatform {
    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    fn form_factor(&self) -> FormFactor {
        FormFactor::Phone
    }

    fn paths(&self) -> &AppPaths {
        &self.paths
    }

    async fn notification_permission(&self) -> PlatformResult<PermissionState> {
        Ok(PermissionState::NotDetermined)
    }

    async fn request_notification_permission(&self) -> PlatformResult<PermissionState> {
        Err(PlatformError::Unsupported(
            "android notification permission",
        ))
    }

    async fn notify(&self, _request: NotificationRequest) -> PlatformResult<NotificationId> {
        Err(PlatformError::Unsupported("android notifications"))
    }

    async fn pick_file(&self, _filter: Option<FileFilter>) -> PlatformResult<Option<FileHandle>> {
        Err(PlatformError::Unsupported("android file picker"))
    }

    async fn save_file(&self, _suggested_name: &str) -> PlatformResult<Option<FileHandle>> {
        Err(PlatformError::Unsupported("android save dialog"))
    }

    async fn read_file(&self, _handle: &FileHandle) -> PlatformResult<Vec<u8>> {
        Err(PlatformError::Unsupported("android file read"))
    }

    async fn write_file(&self, _handle: &FileHandle, _contents: &[u8]) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("android file write"))
    }

    fn open_url(&self, _url: &str) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("android open url"))
    }

    fn open_path(&self, _path: &Path) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("android open path"))
    }

    fn delete_permanently(&self, path: &Path) -> PlatformResult<()> {
        // No OS trash on Android; delete outright.
        std::fs::remove_file(path).map_err(|source| PlatformError::Io {
            path: Some(path.to_path_buf()),
            source,
        })
    }
}
