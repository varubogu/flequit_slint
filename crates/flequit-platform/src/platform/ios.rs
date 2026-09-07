//! iOS backend.
//!
//! UIKit is reached through a small Swift bridge (`mobile/ios/Sources/FlequitBridge.swift`)
//! that exposes plain C entry points, rather than through Objective-C bindings
//! in Rust. Two reasons:
//!
//! - `UNUserNotificationCenter` and `UIDocumentPickerViewController` are
//!   delegate- and completion-handler-based. Expressing that in Rust means
//!   defining Objective-C classes at runtime; in Swift it is a dozen lines the
//!   Xcode toolchain type-checks.
//! - The bridge is part of the app target, so it can reach the key window and
//!   the root view controller, which a library cannot.
//!
//! Everything the sandbox alone can answer — the container paths — is resolved
//! here without the bridge, so the crate is useful even before the app target
//! exists.
//!
//! Logging deliberately has no bridge: on iOS the process's stderr is what
//! Xcode's console shows, and the file sink under the container is what a user
//! can export from the Files app.

use std::ffi::{CString, c_char, c_double, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use chrono::{DateTime, Utc};

use crate::{
    AppPaths, Capabilities, Capability, FileFilter, FileHandle, FormFactor, LifecycleEvent,
    LifecycleHub, LifecycleObserver, NotificationId, NotificationRequest, PermissionState,
    Platform, PlatformError, PlatformResult,
};

/// Bridge status codes. Anything other than [`BRIDGE_OK`] is a failure the
/// Swift side has already logged with its own context.
const BRIDGE_OK: c_int = 0;
/// The bridge is not linked into this binary (unit tests, `cargo check`).
const BRIDGE_MISSING: c_int = -1;

/// Permission codes shared with the Swift bridge.
const PERMISSION_GRANTED: c_int = 0;
const PERMISSION_DENIED: c_int = 1;

/// Form factor codes shared with the Swift bridge, matching `UIUserInterfaceIdiom`.
const IDIOM_PAD: c_int = 1;

/// Result of a document picker presentation.
///
/// `reference` and `display_name` are NUL-terminated UTF-8 owned by Swift and
/// valid only for the duration of the callback.
type DocumentCallback =
    extern "C" fn(context: *mut c_void, reference: *const c_char, display_name: *const c_char);

unsafe extern "C" {
    fn flequit_ios_form_factor() -> c_int;
    fn flequit_ios_notification_permission() -> c_int;
    fn flequit_ios_request_notification_permission() -> c_int;
    fn flequit_ios_notify(
        title: *const c_char,
        body: *const c_char,
        identifier: *const c_char,
        delay_seconds: c_double,
    ) -> c_int;
    fn flequit_ios_cancel_notification(identifier: *const c_char) -> c_int;
    fn flequit_ios_open_url(url: *const c_char) -> c_int;
    fn flequit_ios_pick_document(
        content_type: *const c_char,
        callback: DocumentCallback,
        context: *mut c_void,
    ) -> c_int;
    fn flequit_ios_create_document(
        suggested_name: *const c_char,
        callback: DocumentCallback,
        context: *mut c_void,
    ) -> c_int;
    fn flequit_ios_read_document(
        reference: *const c_char,
        out_bytes: *mut *mut u8,
        out_len: *mut usize,
    ) -> c_int;
    fn flequit_ios_write_document(reference: *const c_char, bytes: *const u8, len: usize) -> c_int;
    fn flequit_ios_free(bytes: *mut u8, len: usize);
}

fn bridge_error(operation: &'static str, code: c_int) -> PlatformError {
    if code == BRIDGE_MISSING {
        return PlatformError::Unsupported("the ios bridge");
    }
    tracing::warn!(operation, code, "ios bridge call failed");
    PlatformError::Io {
        path: None,
        source: std::io::Error::other(format!("{operation} failed with code {code}")),
    }
}

/// Converts to a C string, rejecting interior NULs rather than truncating.
fn c_string(value: &str) -> PlatformResult<CString> {
    CString::new(value).map_err(|_| PlatformError::Io {
        path: None,
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "text passed to the ios bridge contains a NUL byte",
        ),
    })
}

fn lifecycle_hub() -> &'static LifecycleHub {
    static HUB: OnceLock<LifecycleHub> = OnceLock::new();
    HUB.get_or_init(LifecycleHub::new)
}

/// Resolves the app container's Application Support directory.
///
/// iOS points `HOME` at the container, and the container path changes between
/// launches, so it must be read at startup rather than remembered.
fn container_root() -> PlatformResult<PathBuf> {
    let home = std::env::var_os("HOME").ok_or(PlatformError::DirectoryUnavailable {
        kind: "application",
    })?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Flequit"))
}

// --- document picker ------------------------------------------------------

type DocumentSender = tokio::sync::oneshot::Sender<Option<FileHandle>>;

/// Receives the picker result and forwards it to the awaiting task.
///
/// # Safety
///
/// `context` must be the pointer produced by `Box::into_raw` in
/// [`present_picker`], and the two strings must be NUL-terminated UTF-8 or null.
extern "C" fn on_document_selected(
    context: *mut c_void,
    reference: *const c_char,
    display_name: *const c_char,
) {
    if context.is_null() {
        return;
    }
    // SAFETY: the pointer came from `Box::into_raw` and the bridge contract is
    // that the callback fires exactly once.
    let sender = unsafe { Box::from_raw(context.cast::<DocumentSender>()) };

    let read = |pointer: *const c_char| -> Option<String> {
        if pointer.is_null() {
            return None;
        }
        // SAFETY: the bridge guarantees NUL-terminated UTF-8 for the duration
        // of this call, and the value is copied before returning.
        unsafe { std::ffi::CStr::from_ptr(pointer) }
            .to_str()
            .ok()
            .map(str::to_owned)
    };

    let handle = read(reference).map(|reference| FileHandle::Opaque {
        display_name: read(display_name).unwrap_or_else(|| reference.clone()),
        reference,
    });

    // The receiver is gone when the awaiting task was cancelled; nothing to do.
    let _ = sender.send(handle);
}

async fn present_picker(
    present: unsafe extern "C" fn(*const c_char, DocumentCallback, *mut c_void) -> c_int,
    argument: &str,
    operation: &'static str,
) -> PlatformResult<Option<FileHandle>> {
    let argument = c_string(argument)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let context = Box::into_raw(Box::new(sender)).cast::<c_void>();

    // SAFETY: `argument` outlives the call, and ownership of `context` passes to
    // the bridge, which hands it back to the callback exactly once.
    let code = unsafe { present(argument.as_ptr(), on_document_selected, context) };
    if code != BRIDGE_OK {
        // The callback will never fire, so the sender has to be reclaimed here
        // or it leaks.
        // SAFETY: the bridge did not take ownership, so the pointer is still ours.
        drop(unsafe { Box::from_raw(context.cast::<DocumentSender>()) });
        return Err(bridge_error(operation, code));
    }

    // A dropped sender means the presentation was dismissed by the system,
    // which is indistinguishable from the user cancelling.
    Ok(receiver.await.unwrap_or(None))
}

// --- backend --------------------------------------------------------------

pub struct IosPlatform {
    capabilities: Capabilities,
    paths: AppPaths,
    form_factor: FormFactor,
}

impl IosPlatform {
    pub fn new() -> PlatformResult<Self> {
        let paths = AppPaths::rooted_at(container_root()?);
        paths.ensure_all_exist()?;

        // SAFETY: the bridge takes no arguments and returns a plain integer.
        let idiom = unsafe { flequit_ios_form_factor() };

        Ok(Self {
            capabilities: Capabilities::from_supported([
                Capability::LocalNotification,
                Capability::FilePicker,
                Capability::BackgroundSync,
            ]),
            paths,
            form_factor: if idiom == IDIOM_PAD {
                FormFactor::Tablet
            } else {
                FormFactor::Phone
            },
        })
    }

    fn schedule(
        request: &NotificationRequest,
        identifier: &str,
        delay_seconds: f64,
    ) -> PlatformResult<()> {
        let title = c_string(&request.title)?;
        let body = c_string(&request.body)?;
        let identifier = c_string(identifier)?;

        // SAFETY: every pointer is NUL-terminated and outlives the call.
        let code = unsafe {
            flequit_ios_notify(
                title.as_ptr(),
                body.as_ptr(),
                identifier.as_ptr(),
                delay_seconds,
            )
        };
        if code == BRIDGE_OK {
            Ok(())
        } else {
            Err(bridge_error("notify", code))
        }
    }
}

#[async_trait::async_trait]
impl Platform for IosPlatform {
    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    fn form_factor(&self) -> FormFactor {
        self.form_factor
    }

    fn paths(&self) -> &AppPaths {
        &self.paths
    }

    fn subscribe_lifecycle(&self, observer: Arc<dyn LifecycleObserver>) -> PlatformResult<()> {
        lifecycle_hub().subscribe(&observer);
        Ok(())
    }

    async fn notification_permission(&self) -> PlatformResult<PermissionState> {
        // SAFETY: no arguments, plain integer result.
        Ok(match unsafe { flequit_ios_notification_permission() } {
            PERMISSION_GRANTED => PermissionState::Granted,
            PERMISSION_DENIED => PermissionState::Denied,
            _ => PermissionState::NotDetermined,
        })
    }

    async fn request_notification_permission(&self) -> PlatformResult<PermissionState> {
        // SAFETY: no arguments, plain integer result.
        Ok(
            match unsafe { flequit_ios_request_notification_permission() } {
                PERMISSION_GRANTED => PermissionState::Granted,
                PERMISSION_DENIED => PermissionState::Denied,
                _ => PermissionState::NotDetermined,
            },
        )
    }

    async fn notify(&self, request: NotificationRequest) -> PlatformResult<NotificationId> {
        let id = NotificationId(request.payload.clone().unwrap_or_default());
        // A zero delay makes UNUserNotificationCenter deliver immediately,
        // which keeps one code path for both cases.
        Self::schedule(&request, &id.0, 0.0)?;
        Ok(id)
    }

    async fn schedule_notification(
        &self,
        request: NotificationRequest,
        scheduled_at: DateTime<Utc>,
    ) -> PlatformResult<NotificationId> {
        let payload = request.payload.clone().unwrap_or_default();
        let id = NotificationId::scheduled(&payload, &scheduled_at);
        // The OS owns the timer, so the reminder still fires after the app is
        // suspended or killed — unlike the desktop backend's in-process sleep.
        let delay = scheduled_at
            .signed_duration_since(Utc::now())
            .num_milliseconds()
            .max(0) as f64
            / 1000.0;
        Self::schedule(&request, &id.0, delay)?;
        Ok(id)
    }

    async fn cancel_notification(&self, id: &NotificationId) -> PlatformResult<()> {
        let identifier = c_string(&id.0)?;
        // SAFETY: NUL-terminated and outlives the call.
        let code = unsafe { flequit_ios_cancel_notification(identifier.as_ptr()) };
        if code == BRIDGE_OK {
            Ok(())
        } else {
            Err(bridge_error("cancel notification", code))
        }
    }

    async fn pick_file(&self, filter: Option<FileFilter>) -> PlatformResult<Option<FileHandle>> {
        // The picker takes uniform type identifiers. Anything we cannot map is
        // widened to public.data rather than silently hiding the user's file.
        let content_type = filter
            .as_ref()
            .and_then(|f| f.extensions.first())
            .map(|extension| match extension.as_str() {
                "json" => "public.json",
                "txt" | "md" => "public.plain-text",
                _ => "public.data",
            })
            .unwrap_or("public.data");

        present_picker(flequit_ios_pick_document, content_type, "pick document").await
    }

    async fn save_file(&self, suggested_name: &str) -> PlatformResult<Option<FileHandle>> {
        present_picker(
            flequit_ios_create_document,
            suggested_name,
            "create document",
        )
        .await
    }

    async fn read_file(&self, handle: &FileHandle) -> PlatformResult<Vec<u8>> {
        let FileHandle::Opaque { reference, .. } = handle else {
            let path = handle
                .as_path()
                .expect("a non-opaque handle carries a path");
            return std::fs::read(path).map_err(|source| PlatformError::Io {
                path: Some(path.to_path_buf()),
                source,
            });
        };

        let reference = c_string(reference)?;
        let mut bytes: *mut u8 = std::ptr::null_mut();
        let mut len: usize = 0;
        // SAFETY: both out-parameters are valid for writes for the call, and the
        // bridge only writes them when it returns BRIDGE_OK.
        let code = unsafe { flequit_ios_read_document(reference.as_ptr(), &mut bytes, &mut len) };
        if code != BRIDGE_OK {
            return Err(bridge_error("read document", code));
        }
        if bytes.is_null() {
            return Ok(Vec::new());
        }

        // SAFETY: the bridge reports a buffer of `len` initialised bytes that
        // stays alive until `flequit_ios_free`, which is called right after the
        // copy.
        let contents = unsafe { std::slice::from_raw_parts(bytes, len) }.to_vec();
        // SAFETY: the pointer and length are exactly what the bridge handed out.
        unsafe { flequit_ios_free(bytes, len) };
        Ok(contents)
    }

    async fn write_file(&self, handle: &FileHandle, contents: &[u8]) -> PlatformResult<()> {
        let FileHandle::Opaque { reference, .. } = handle else {
            let path = handle
                .as_path()
                .expect("a non-opaque handle carries a path");
            return std::fs::write(path, contents).map_err(|source| PlatformError::Io {
                path: Some(path.to_path_buf()),
                source,
            });
        };

        let reference = c_string(reference)?;
        // SAFETY: the slice outlives the call and the bridge only reads it.
        let code = unsafe {
            flequit_ios_write_document(reference.as_ptr(), contents.as_ptr(), contents.len())
        };
        if code == BRIDGE_OK {
            Ok(())
        } else {
            Err(bridge_error("write document", code))
        }
    }

    fn open_url(&self, url: &str) -> PlatformResult<()> {
        let url = c_string(url)?;
        // SAFETY: NUL-terminated and outlives the call.
        let code = unsafe { flequit_ios_open_url(url.as_ptr()) };
        if code == BRIDGE_OK {
            Ok(())
        } else {
            Err(bridge_error("open url", code))
        }
    }

    fn open_path(&self, path: &Path) -> PlatformResult<()> {
        // Nothing outside the app can read the container, so handing a path to
        // another app would open a file it cannot see.
        let _ = path;
        Err(PlatformError::Unsupported("opening sandboxed paths on ios"))
    }

    fn delete_permanently(&self, path: &Path) -> PlatformResult<()> {
        // No OS trash on iOS; delete outright.
        std::fs::remove_file(path).map_err(|source| PlatformError::Io {
            path: Some(path.to_path_buf()),
            source,
        })
    }
}

// --- exports called by the Swift bridge -----------------------------------

/// Delivers a lifecycle transition observed by the app delegate.
///
/// # Safety
///
/// Called from the Swift bridge with one of the codes
/// [`LifecycleEvent::from_code`] knows. Safe to call from any thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flequit_ios_lifecycle_event(event: c_int) {
    match LifecycleEvent::from_code(event) {
        Some(event) => lifecycle_hub().dispatch(event),
        None => tracing::warn!(event, "unknown lifecycle code from the ios bridge"),
    }
}
