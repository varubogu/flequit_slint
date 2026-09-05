//! Windows / macOS / Linux backend.

use std::path::Path;
use std::sync::Arc;

use crate::{
    AppPaths, Capabilities, Capability, FileFilter, FileHandle, FormFactor, NotificationId,
    NotificationRequest, PermissionState, Platform, PlatformError, PlatformResult,
};

pub struct DesktopPlatform {
    capabilities: Capabilities,
    paths: AppPaths,
}

impl DesktopPlatform {
    pub fn new() -> PlatformResult<Self> {
        let dirs = directories::ProjectDirs::from("com", "flequit", "flequit").ok_or(
            PlatformError::DirectoryUnavailable {
                kind: "application",
            },
        )?;

        let data = dirs.data_local_dir().to_path_buf();
        let log = data.join("logs");
        let paths = AppPaths::new(
            data,
            dirs.config_dir().to_path_buf(),
            dirs.cache_dir().to_path_buf(),
            log,
        );
        paths.ensure_all_exist()?;

        Ok(Self {
            capabilities: Capabilities::from_supported([
                Capability::SystemTray,
                Capability::GlobalHotkey,
                Capability::MultiWindow,
                Capability::OsTrash,
                Capability::ArbitraryFilePath,
                Capability::LocalNotification,
                Capability::FilePicker,
                Capability::BackgroundSync,
                Capability::SelfUpdate,
            ]),
            paths,
        })
    }

    fn build_dialog(filter: Option<FileFilter>) -> rfd::AsyncFileDialog {
        let dialog = rfd::AsyncFileDialog::new();
        match filter {
            Some(f) if !f.extensions.is_empty() => {
                let extensions: Vec<&str> = f.extensions.iter().map(String::as_str).collect();
                dialog.add_filter(&f.name, &extensions)
            }
            _ => dialog,
        }
    }
}

#[async_trait::async_trait]
impl Platform for DesktopPlatform {
    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    fn form_factor(&self) -> FormFactor {
        FormFactor::Desktop
    }

    fn paths(&self) -> &AppPaths {
        &self.paths
    }

    async fn notification_permission(&self) -> PlatformResult<PermissionState> {
        Ok(PermissionState::Granted)
    }

    async fn request_notification_permission(&self) -> PlatformResult<PermissionState> {
        Ok(PermissionState::Granted)
    }

    async fn notify(&self, request: NotificationRequest) -> PlatformResult<NotificationId> {
        let payload = request.payload.clone().unwrap_or_default();
        notify_rust::Notification::new()
            .summary(&request.title)
            .body(&request.body)
            .show()
            .map_err(|e| {
                tracing::warn!(error = %e, "failed to show notification");
                PlatformError::Io {
                    path: None,
                    source: std::io::Error::other(e.to_string()),
                }
            })?;
        Ok(NotificationId(payload))
    }

    async fn pick_file(&self, filter: Option<FileFilter>) -> PlatformResult<Option<FileHandle>> {
        Ok(Self::build_dialog(filter)
            .pick_file()
            .await
            .map(|f| FileHandle::Path(f.path().to_path_buf())))
    }

    async fn save_file(&self, suggested_name: &str) -> PlatformResult<Option<FileHandle>> {
        Ok(rfd::AsyncFileDialog::new()
            .set_file_name(suggested_name)
            .save_file()
            .await
            .map(|f| FileHandle::Path(f.path().to_path_buf())))
    }

    async fn read_file(&self, handle: &FileHandle) -> PlatformResult<Vec<u8>> {
        let path = handle
            .as_path()
            .ok_or(PlatformError::Unsupported("opaque file handles on desktop"))?;
        std::fs::read(path).map_err(|source| PlatformError::Io {
            path: Some(path.to_path_buf()),
            source,
        })
    }

    async fn write_file(&self, handle: &FileHandle, contents: &[u8]) -> PlatformResult<()> {
        let path = handle
            .as_path()
            .ok_or(PlatformError::Unsupported("opaque file handles on desktop"))?;
        std::fs::write(path, contents).map_err(|source| PlatformError::Io {
            path: Some(path.to_path_buf()),
            source,
        })
    }

    fn open_url(&self, url: &str) -> PlatformResult<()> {
        opener::open_browser(url).map_err(|e| PlatformError::Io {
            path: None,
            source: std::io::Error::other(e.to_string()),
        })
    }

    fn open_path(&self, path: &Path) -> PlatformResult<()> {
        opener::open(path).map_err(|e| PlatformError::Io {
            path: Some(path.to_path_buf()),
            source: std::io::Error::other(e.to_string()),
        })
    }

    fn delete_permanently(&self, path: &Path) -> PlatformResult<()> {
        trash::delete(path).map_err(|e| PlatformError::Io {
            path: Some(path.to_path_buf()),
            source: std::io::Error::other(e.to_string()),
        })
    }
}

/// Keeps `Arc<DesktopPlatform>` usable where `Arc<dyn Platform>` is expected.
impl From<DesktopPlatform> for Arc<dyn Platform> {
    fn from(value: DesktopPlatform) -> Self {
        Arc::new(value)
    }
}
