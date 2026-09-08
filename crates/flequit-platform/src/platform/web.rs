//! Browser (WebAssembly) backend.
//!
//! **UI only, by design.** Slint runs in the browser on the WebGL renderer, but
//! nothing below the UI does: `sea-orm`/`sqlx`, the multi-threaded Tokio runtime
//! and every desktop OS crate are unavailable on `wasm32-unknown-unknown`.
//!
//! That is not going to be fixed here. A browser build is never meant to store
//! anything locally — it will talk to a Flequit backend server, which the
//! desktop and mobile builds will also offer alongside local storage. See
//! `plans/plan.md` section 8.
//!
//! What this backend provides is enough to run the Slint UI against in-memory
//! data: a capability set that hides every affordance the browser cannot
//! honour, and a log sink that reaches the developer console.
//!
//! [`AppPaths`] points at a virtual root. Nothing in a browser build may touch
//! the filesystem, so the paths exist only to satisfy the trait; any code that
//! actually opens them will fail, which is the intended signal that it has no
//! business running here.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use wasm_bindgen::prelude::wasm_bindgen;

use crate::{
    AppPaths, Capabilities, FileFilter, FileHandle, FormFactor, NotificationId,
    NotificationRequest, PermissionState, Platform, PlatformError, PlatformResult,
};

#[wasm_bindgen]
unsafe extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = log)]
    fn console_log(message: &str);
}

/// Writes one formatted line to the browser's developer console.
pub(super) fn write_to_console(line: &str) {
    console_log(line);
}

pub struct WebPlatform {
    capabilities: Capabilities,
    paths: AppPaths,
}

impl WebPlatform {
    pub fn new() -> PlatformResult<Self> {
        Ok(Self {
            // Deliberately empty. Advertising a capability the browser cannot
            // honour would put dead buttons in the UI. `BackgroundSync` becomes
            // true once the backend API client exists, and the rest stay false.
            capabilities: Capabilities::default(),
            paths: AppPaths::rooted_at(PathBuf::from("/flequit")),
        })
    }
}

#[async_trait::async_trait]
impl Platform for WebPlatform {
    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    fn form_factor(&self) -> FormFactor {
        // Only a hint for initial window sizing, and the browser canvas is
        // sized by the page. Layout branches on width regardless.
        FormFactor::Desktop
    }

    fn paths(&self) -> &AppPaths {
        &self.paths
    }

    async fn notification_permission(&self) -> PlatformResult<PermissionState> {
        Ok(PermissionState::Denied)
    }

    async fn request_notification_permission(&self) -> PlatformResult<PermissionState> {
        Ok(PermissionState::Denied)
    }

    async fn notify(&self, _request: NotificationRequest) -> PlatformResult<NotificationId> {
        Err(PlatformError::Unsupported("notifications in the browser"))
    }

    async fn schedule_notification(
        &self,
        _request: NotificationRequest,
        _scheduled_at: DateTime<Utc>,
    ) -> PlatformResult<NotificationId> {
        Err(PlatformError::Unsupported(
            "scheduled notifications in the browser",
        ))
    }

    async fn cancel_notification(&self, _id: &NotificationId) -> PlatformResult<()> {
        Err(PlatformError::Unsupported(
            "scheduled notifications in the browser",
        ))
    }

    async fn pick_file(&self, _filter: Option<FileFilter>) -> PlatformResult<Option<FileHandle>> {
        Err(PlatformError::Unsupported("the file picker in the browser"))
    }

    async fn save_file(&self, _suggested_name: &str) -> PlatformResult<Option<FileHandle>> {
        Err(PlatformError::Unsupported("the save dialog in the browser"))
    }

    async fn read_file(&self, _handle: &FileHandle) -> PlatformResult<Vec<u8>> {
        Err(PlatformError::Unsupported("file access in the browser"))
    }

    async fn write_file(&self, _handle: &FileHandle, _contents: &[u8]) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("file access in the browser"))
    }

    fn open_url(&self, url: &str) -> PlatformResult<()> {
        // Deliberately not implemented via `window.open`: browsers block popups
        // that are not a direct result of a user gesture, so it would fail
        // silently more often than it worked.
        let _ = url;
        Err(PlatformError::Unsupported("opening urls in the browser"))
    }

    fn open_path(&self, path: &Path) -> PlatformResult<()> {
        let _ = path;
        Err(PlatformError::Unsupported("opening paths in the browser"))
    }

    fn delete_permanently(&self, path: &Path) -> PlatformResult<()> {
        let _ = path;
        Err(PlatformError::Unsupported("deleting files in the browser"))
    }
}
