//! Windows / macOS / Linux backend.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use crate::{
    AppPaths, Capabilities, Capability, FileFilter, FileHandle, FormFactor, NotificationId,
    NotificationRequest, PermissionState, Platform, PlatformError, PlatformResult, SystemTheme,
    SystemThemeWatcher,
};

struct DesktopThemeWatcher {
    watcher: dark_light::Watcher,
}

impl SystemThemeWatcher for DesktopThemeWatcher {
    fn try_recv(&self) -> PlatformResult<Option<SystemTheme>> {
        match self.watcher.try_recv() {
            Ok(mode) => Ok(Some(system_theme(mode))),
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(PlatformError::Io {
                path: None,
                source: std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "system theme watcher disconnected",
                ),
            }),
        }
    }
}

fn system_theme(mode: dark_light::Mode) -> SystemTheme {
    match mode {
        dark_light::Mode::Light => SystemTheme::Light,
        dark_light::Mode::Dark => SystemTheme::Dark,
        dark_light::Mode::Unspecified => SystemTheme::Unspecified,
    }
}

pub struct DesktopPlatform {
    capabilities: Capabilities,
    paths: AppPaths,
    scheduled_notifications: Mutex<HashMap<NotificationId, tokio::task::JoinHandle<()>>>,
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
                Capability::FontEnumeration,
            ]),
            paths,
            scheduled_notifications: Mutex::new(HashMap::new()),
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

    fn show_notification(request: &NotificationRequest) -> PlatformResult<NotificationId> {
        let payload = request.payload.clone().unwrap_or_default();
        notify_rust::Notification::new()
            .summary(&request.title)
            .body(&request.body)
            .show()
            .map_err(|error| {
                tracing::warn!(%error, "failed to show notification");
                PlatformError::Io {
                    path: None,
                    source: std::io::Error::other(error.to_string()),
                }
            })?;
        Ok(NotificationId(payload))
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

    fn available_fonts(&self) -> PlatformResult<Vec<String>> {
        // The first family of each face is its primary name; the rest are
        // localized aliases for the same family and would only duplicate the
        // list.
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        let mut families = database
            .faces()
            .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
            .collect::<Vec<_>>();
        families.sort_unstable();
        families.dedup();
        Ok(families)
    }

    fn system_theme(&self) -> PlatformResult<SystemTheme> {
        dark_light::detect()
            .map(system_theme)
            .map_err(|error| PlatformError::Io {
                path: None,
                source: std::io::Error::other(error.to_string()),
            })
    }

    fn subscribe_system_theme(&self) -> PlatformResult<Box<dyn SystemThemeWatcher>> {
        dark_light::subscribe()
            .map(|watcher| Box::new(DesktopThemeWatcher { watcher }) as Box<dyn SystemThemeWatcher>)
            .map_err(|error| PlatformError::Io {
                path: None,
                source: std::io::Error::other(error.to_string()),
            })
    }

    async fn notification_permission(&self) -> PlatformResult<PermissionState> {
        Ok(PermissionState::Granted)
    }

    async fn request_notification_permission(&self) -> PlatformResult<PermissionState> {
        Ok(PermissionState::Granted)
    }

    async fn notify(&self, request: NotificationRequest) -> PlatformResult<NotificationId> {
        Self::show_notification(&request)
    }

    async fn schedule_notification(
        &self,
        request: NotificationRequest,
        scheduled_at: DateTime<Utc>,
    ) -> PlatformResult<NotificationId> {
        let payload = request.payload.clone().unwrap_or_default();
        let id = NotificationId::scheduled(&payload, &scheduled_at);
        let delay = scheduled_at
            .signed_duration_since(Utc::now())
            .to_std()
            .unwrap_or_default();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            if let Err(error) = Self::show_notification(&request) {
                tracing::warn!(%error, "scheduled notification failed");
            }
        });

        let mut scheduled_notifications = self
            .scheduled_notifications
            .lock()
            .expect("scheduled notifications poisoned");
        scheduled_notifications.retain(|_, handle| !handle.is_finished());
        if let Some(previous) = scheduled_notifications.insert(id.clone(), handle) {
            previous.abort();
        }
        Ok(id)
    }

    async fn cancel_notification(&self, id: &NotificationId) -> PlatformResult<()> {
        if let Some(handle) = self
            .scheduled_notifications
            .lock()
            .expect("scheduled notifications poisoned")
            .remove(id)
        {
            handle.abort();
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::system_theme;
    use crate::SystemTheme;

    #[test]
    fn maps_every_detected_theme() {
        assert_eq!(system_theme(dark_light::Mode::Light), SystemTheme::Light);
        assert_eq!(system_theme(dark_light::Mode::Dark), SystemTheme::Dark);
        assert_eq!(
            system_theme(dark_light::Mode::Unspecified),
            SystemTheme::Unspecified
        );
    }
}
