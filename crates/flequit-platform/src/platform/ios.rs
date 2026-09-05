//! iOS backend.
//!
//! **Phase 2 — not yet implemented.** The structure exists so that the crate
//! compiles for `aarch64-apple-ios` in CI and so that the capability set is
//! recorded in one place.
//!
//! Remaining work:
//!
//! - Obtain the sandbox root from UIKit (`Application Support` under the app
//!   container) instead of the placeholder below.
//! - Notifications via `UNUserNotificationCenter`, including the runtime
//!   permission request.
//! - File selection via `UIDocumentPickerViewController`, returning
//!   [`FileHandle::Opaque`] with a security-scoped bookmark.
//! - `open_url` / `open_path` via `UIApplication.open`.

use std::path::{Path, PathBuf};

use crate::{
    AppPaths, Capabilities, Capability, FileFilter, FileHandle, FormFactor, NotificationId,
    NotificationRequest, PermissionState, Platform, PlatformError, PlatformResult,
};

pub struct IosPlatform {
    capabilities: Capabilities,
    paths: AppPaths,
}

impl IosPlatform {
    pub fn new() -> PlatformResult<Self> {
        // TODO(phase2): replace with the sandbox root reported by UIKit.
        let root = std::env::var_os("FLEQUIT_IOS_DATA_ROOT")
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
impl Platform for IosPlatform {
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
        Err(PlatformError::Unsupported("ios notification permission"))
    }

    async fn notify(&self, _request: NotificationRequest) -> PlatformResult<NotificationId> {
        Err(PlatformError::Unsupported("ios notifications"))
    }

    async fn pick_file(&self, _filter: Option<FileFilter>) -> PlatformResult<Option<FileHandle>> {
        Err(PlatformError::Unsupported("ios file picker"))
    }

    async fn save_file(&self, _suggested_name: &str) -> PlatformResult<Option<FileHandle>> {
        Err(PlatformError::Unsupported("ios save dialog"))
    }

    async fn read_file(&self, _handle: &FileHandle) -> PlatformResult<Vec<u8>> {
        Err(PlatformError::Unsupported("ios file read"))
    }

    async fn write_file(&self, _handle: &FileHandle, _contents: &[u8]) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("ios file write"))
    }

    fn open_url(&self, _url: &str) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("ios open url"))
    }

    fn open_path(&self, _path: &Path) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("ios open path"))
    }

    fn delete_permanently(&self, path: &Path) -> PlatformResult<()> {
        // No OS trash on iOS; delete outright.
        std::fs::remove_file(path).map_err(|source| PlatformError::Io {
            path: Some(path.to_path_buf()),
            source,
        })
    }
}
