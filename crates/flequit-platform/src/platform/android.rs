//! Android backend.
//!
//! Everything here goes through JNI. The `JavaVM` and the `Activity` object are
//! published by `android-activity` through `ndk-context`, so this module needs
//! no initialisation call from `flequit-app` and stays independent of whichever
//! `android-activity` version the UI layer links.
//!
//! Two operations cannot be driven from Rust alone, because Android delivers
//! their results to the `Activity` rather than to the caller:
//!
//! - the Storage Access Framework picker, whose result arrives in
//!   `onActivityResult`;
//! - lifecycle transitions, which arrive in `onPause` / `onResume` /
//!   `onTrimMemory`.
//!
//! Both are bridged by `mobile/android/app/src/main/java/com/flequit/app/FlequitActivity.java`,
//! which calls the `Java_com_flequit_app_FlequitActivity_*` functions at the
//! bottom of this file. An APK built against a plain `NativeActivity` has no
//! such methods, so those calls fail and are reported as
//! [`PlatformError::Unsupported`].

use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{DateTime, Utc};
use jni::JNIEnv;
use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JString, JValue};

use crate::{
    AppPaths, Capabilities, Capability, FileFilter, FileHandle, FormFactor, LifecycleEvent,
    LifecycleHub, LifecycleObserver, NotificationId, NotificationRequest, PermissionState,
    Platform, PlatformError, PlatformResult,
};

/// Channel the reminder notifications are posted to.
///
/// Android remembers the user's per-channel preferences, so the id must stay
/// stable across releases.
const NOTIFICATION_CHANNEL_ID: &str = "flequit.reminders";
const NOTIFICATION_CHANNEL_NAME: &str = "Reminders";
/// `NotificationManager.IMPORTANCE_DEFAULT`.
const IMPORTANCE_DEFAULT: i32 = 3;
/// `PackageManager.PERMISSION_GRANTED`.
const PERMISSION_GRANTED: i32 = 0;
/// `Build.VERSION_CODES.TIRAMISU`, the first release that gates notifications.
const SDK_TIRAMISU: i32 = 33;
/// `Configuration.smallestScreenWidthDp` above which Android itself calls the
/// device a tablet.
const TABLET_SMALLEST_WIDTH_DP: i32 = 600;
/// `Activity.RESULT_OK`.
const RESULT_OK: i32 = -1;
/// Size of one `InputStream.read` chunk when draining a content URI.
const READ_CHUNK: i32 = 8192;

// --- logcat ---------------------------------------------------------------

/// `ANDROID_LOG_INFO`. The level is already part of the formatted line, so one
/// priority keeps the bridge trivial and the line readable.
const ANDROID_LOG_INFO: c_int = 4;

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// Writes one formatted line to logcat under the `flequit` tag.
pub(super) fn write_to_logcat(line: &str) {
    let Ok(text) = CString::new(line) else {
        // An interior NUL means the line did not come from our formatter;
        // dropping it beats truncating at an arbitrary byte.
        return;
    };
    let tag = c"flequit";
    // SAFETY: both pointers are NUL-terminated and outlive the call, which is
    // all liblog requires of them.
    unsafe {
        __android_log_write(ANDROID_LOG_INFO, tag.as_ptr(), text.as_ptr());
    }
}

// --- JNI plumbing ---------------------------------------------------------

fn jni_error(context: &'static str, error: jni::errors::Error) -> PlatformError {
    tracing::warn!(%error, context, "jni call failed");
    PlatformError::Io {
        path: None,
        source: std::io::Error::other(format!("{context}: {error}")),
    }
}

struct JavaRuntime {
    vm: jni::JavaVM,
    activity: GlobalRef,
}

/// The process-wide JVM handle and Activity reference.
///
/// Resolved once: `ndk-context` publishes both before any of our code runs and
/// they stay valid for the life of the process.
fn java_runtime() -> PlatformResult<&'static JavaRuntime> {
    static RUNTIME: OnceLock<Option<JavaRuntime>> = OnceLock::new();

    RUNTIME
        .get_or_init(|| {
            let context = ndk_context::android_context();
            // SAFETY: android-activity publishes a valid JavaVM pointer here
            // before the first Rust frame of ours runs.
            let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }.ok()?;
            let mut env = vm.attach_current_thread().ok()?;
            // SAFETY: `context()` is the Activity's jobject. It is borrowed
            // just long enough to promote it to a global reference.
            let activity = unsafe { JObject::from_raw(context.context().cast()) };
            let activity = env.new_global_ref(&activity).ok()?;
            drop(env);
            Some(JavaRuntime { vm, activity })
        })
        .as_ref()
        .ok_or(PlatformError::Unsupported("the android JNI context"))
}

/// Runs `body` with a `JNIEnv` attached to the calling thread.
///
/// Background threads must attach before touching JNI and the guard detaches
/// them again on drop; on the UI thread attaching is a no-op.
///
/// A failing JNI call usually leaves a Java exception pending, which would
/// poison every later call on the same thread, so it is cleared here.
fn with_env<R>(
    context: &'static str,
    body: impl FnOnce(&mut JNIEnv<'_>, &JObject<'static>) -> jni::errors::Result<R>,
) -> PlatformResult<R> {
    let runtime = java_runtime()?;
    let mut guard = runtime
        .vm
        .attach_current_thread()
        .map_err(|error| jni_error(context, error))?;

    let result = body(&mut guard, runtime.activity.as_obj());
    if result.is_err() {
        let _ = guard.exception_describe();
        let _ = guard.exception_clear();
    }
    result.map_err(|error| jni_error(context, error))
}

fn java_string<'local>(
    env: &mut JNIEnv<'local>,
    value: &str,
) -> jni::errors::Result<JString<'local>> {
    env.new_string(value)
}

fn notification_manager<'local>(
    env: &mut JNIEnv<'local>,
    activity: &JObject<'_>,
) -> jni::errors::Result<JObject<'local>> {
    let service_name = java_string(env, "notification")?;
    env.call_method(
        activity,
        "getSystemService",
        "(Ljava/lang/String;)Ljava/lang/Object;",
        &[JValue::Object(&service_name)],
    )?
    .l()
}

/// Opens an `InputStream` / `OutputStream` for a SAF content URI.
fn content_resolver_stream<'local>(
    env: &mut JNIEnv<'local>,
    activity: &JObject<'_>,
    reference: &str,
    method: &'static str,
    signature: &'static str,
    mode: Option<&str>,
) -> jni::errors::Result<JObject<'local>> {
    let resolver = env
        .call_method(
            activity,
            "getContentResolver",
            "()Landroid/content/ContentResolver;",
            &[],
        )?
        .l()?;
    let reference = java_string(env, reference)?;
    let uri = env
        .call_static_method(
            "android/net/Uri",
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&reference)],
        )?
        .l()?;

    match mode {
        Some(mode) => {
            let mode = java_string(env, mode)?;
            env.call_method(
                &resolver,
                method,
                signature,
                &[JValue::Object(&uri), JValue::Object(&mode)],
            )?
            .l()
        }
        None => env
            .call_method(&resolver, method, signature, &[JValue::Object(&uri)])?
            .l(),
    }
}

// --- activity result bridge -----------------------------------------------

type ResultSender = tokio::sync::oneshot::Sender<Option<FileHandle>>;

fn pending_requests() -> &'static Mutex<HashMap<i32, ResultSender>> {
    static PENDING: OnceLock<Mutex<HashMap<i32, ResultSender>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Request codes only have to be unique among the few picks in flight at once,
/// and Android rejects codes that do not fit in 16 bits.
fn next_request_code() -> i32 {
    static NEXT: AtomicI32 = AtomicI32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed) & 0xffff
}

fn lifecycle_hub() -> &'static LifecycleHub {
    static HUB: OnceLock<LifecycleHub> = OnceLock::new();
    HUB.get_or_init(LifecycleHub::new)
}

// --- backend --------------------------------------------------------------

pub struct AndroidPlatform {
    capabilities: Capabilities,
    paths: AppPaths,
    form_factor: FormFactor,
    scheduled_notifications: Mutex<HashMap<NotificationId, tokio::task::JoinHandle<()>>>,
}

impl AndroidPlatform {
    pub fn new() -> PlatformResult<Self> {
        let paths = AppPaths::rooted_at(files_dir()?);
        paths.ensure_all_exist()?;

        Ok(Self {
            capabilities: Capabilities::from_supported([
                Capability::LocalNotification,
                Capability::FilePicker,
                Capability::BackgroundSync,
            ]),
            paths,
            // A device we cannot measure is treated as the smaller class, so the
            // layout errs towards fitting rather than overflowing.
            form_factor: form_factor().unwrap_or(FormFactor::Phone),
            scheduled_notifications: Mutex::new(HashMap::new()),
        })
    }

    /// Opens the SAF picker and waits for `onActivityResult`.
    async fn request_document(
        method: &'static str,
        argument: String,
    ) -> PlatformResult<Option<FileHandle>> {
        let request_code = next_request_code();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        pending_requests()
            .lock()
            .expect("pending requests poisoned")
            .insert(request_code, sender);

        let started = with_env("start document picker", |env, activity| {
            let argument = java_string(env, &argument)?;
            env.call_method(
                activity,
                method,
                "(Ljava/lang/String;I)V",
                &[JValue::Object(&argument), JValue::Int(request_code)],
            )?;
            Ok(())
        });

        if started.is_err() {
            pending_requests()
                .lock()
                .expect("pending requests poisoned")
                .remove(&request_code);
            // The method is missing when the APK uses a plain NativeActivity.
            return Err(PlatformError::Unsupported(
                "the android document picker bridge",
            ));
        }

        // The sender is dropped without a value when the activity is recreated
        // mid-pick, which is indistinguishable from the user cancelling.
        Ok(receiver.await.unwrap_or(None))
    }

    fn post_notification(request: &NotificationRequest, tag: &str) -> PlatformResult<()> {
        with_env("post notification", |env, activity| {
            let manager = notification_manager(env, activity)?;

            // Creating a channel that already exists is a no-op, so this is
            // cheaper than tracking whether it has been done and it survives the
            // user clearing app data.
            let channel_id = java_string(env, NOTIFICATION_CHANNEL_ID)?;
            let channel_name = java_string(env, NOTIFICATION_CHANNEL_NAME)?;
            let channel = env.new_object(
                "android/app/NotificationChannel",
                "(Ljava/lang/String;Ljava/lang/CharSequence;I)V",
                &[
                    JValue::Object(&channel_id),
                    JValue::Object(&channel_name),
                    JValue::Int(IMPORTANCE_DEFAULT),
                ],
            )?;
            env.call_method(
                &manager,
                "createNotificationChannel",
                "(Landroid/app/NotificationChannel;)V",
                &[JValue::Object(&channel)],
            )?;

            let builder = env.new_object(
                "android/app/Notification$Builder",
                "(Landroid/content/Context;Ljava/lang/String;)V",
                &[JValue::Object(activity), JValue::Object(&channel_id)],
            )?;
            let title = java_string(env, &request.title)?;
            env.call_method(
                &builder,
                "setContentTitle",
                "(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;",
                &[JValue::Object(&title)],
            )?;
            let body = java_string(env, &request.body)?;
            env.call_method(
                &builder,
                "setContentText",
                "(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;",
                &[JValue::Object(&body)],
            )?;
            env.call_method(
                &builder,
                "setAutoCancel",
                "(Z)Landroid/app/Notification$Builder;",
                &[JValue::Bool(1)],
            )?;
            // A framework drawable, so a notification renders correctly before
            // the app ships icon resources of its own.
            let icon = env
                .get_static_field("android/R$drawable", "ic_dialog_info", "I")?
                .i()?;
            env.call_method(
                &builder,
                "setSmallIcon",
                "(I)Landroid/app/Notification$Builder;",
                &[JValue::Int(icon)],
            )?;
            let notification = env
                .call_method(&builder, "build", "()Landroid/app/Notification;", &[])?
                .l()?;

            // Tagging by our own id lets a reminder replace its previous post
            // instead of stacking duplicates.
            let tag = java_string(env, tag)?;
            env.call_method(
                &manager,
                "notify",
                "(Ljava/lang/String;ILandroid/app/Notification;)V",
                &[
                    JValue::Object(&tag),
                    JValue::Int(0),
                    JValue::Object(&notification),
                ],
            )?;
            Ok(())
        })
    }
}

fn files_dir() -> PlatformResult<PathBuf> {
    with_env("resolve files dir", |env, activity| {
        let dir = env
            .call_method(activity, "getFilesDir", "()Ljava/io/File;", &[])?
            .l()?;
        let path = env
            .call_method(&dir, "getAbsolutePath", "()Ljava/lang/String;", &[])?
            .l()?;
        let path = JString::from(path);
        Ok(PathBuf::from(String::from(env.get_string(&path)?)))
    })
}

fn form_factor() -> PlatformResult<FormFactor> {
    let smallest_width = with_env("read screen configuration", |env, activity| {
        let resources = env
            .call_method(
                activity,
                "getResources",
                "()Landroid/content/res/Resources;",
                &[],
            )?
            .l()?;
        let configuration = env
            .call_method(
                &resources,
                "getConfiguration",
                "()Landroid/content/res/Configuration;",
                &[],
            )?
            .l()?;
        env.get_field(&configuration, "smallestScreenWidthDp", "I")?
            .i()
    })?;

    Ok(if smallest_width >= TABLET_SMALLEST_WIDTH_DP {
        FormFactor::Tablet
    } else {
        FormFactor::Phone
    })
}

fn sdk_int() -> PlatformResult<i32> {
    with_env("read sdk level", |env, _activity| {
        env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
            .i()
    })
}

#[async_trait::async_trait]
impl Platform for AndroidPlatform {
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
        // `areNotificationsEnabled` covers both the runtime permission and the
        // user switching notifications off in settings, which a permission check
        // alone would miss.
        let enabled = with_env("read notification permission", |env, activity| {
            let manager = notification_manager(env, activity)?;
            env.call_method(&manager, "areNotificationsEnabled", "()Z", &[])?
                .z()
        })?;
        if enabled {
            return Ok(PermissionState::Granted);
        }

        // Before API 33 there is nothing to grant, so "off" means the user
        // turned it off rather than never having been asked.
        if sdk_int().unwrap_or(0) < SDK_TIRAMISU {
            return Ok(PermissionState::Denied);
        }

        let permission = with_env("check notification permission", |env, activity| {
            let permission = java_string(env, "android.permission.POST_NOTIFICATIONS")?;
            env.call_method(
                activity,
                "checkSelfPermission",
                "(Ljava/lang/String;)I",
                &[JValue::Object(&permission)],
            )?
            .i()
        })?;

        // Granted-but-disabled means the user revoked it in settings; only a
        // permission that was never granted is still worth asking for.
        Ok(if permission == PERMISSION_GRANTED {
            PermissionState::Denied
        } else {
            PermissionState::NotDetermined
        })
    }

    async fn request_notification_permission(&self) -> PlatformResult<PermissionState> {
        if sdk_int()? < SDK_TIRAMISU {
            return self.notification_permission().await;
        }

        with_env("request notification permission", |env, activity| {
            let permission = java_string(env, "android.permission.POST_NOTIFICATIONS")?;
            let permissions = env.new_object_array(1, "java/lang/String", JObject::null())?;
            env.set_object_array_element(&permissions, 0, &permission)?;
            env.call_method(
                activity,
                "requestPermissions",
                "([Ljava/lang/String;I)V",
                &[
                    JValue::Object(&permissions),
                    JValue::Int(next_request_code()),
                ],
            )?;
            Ok(())
        })?;

        // Android answers through `onRequestPermissionsResult` on the activity,
        // not to this call. Report the state as it stands and let the caller
        // re-query once the dialog is gone.
        self.notification_permission().await
    }

    async fn notify(&self, request: NotificationRequest) -> PlatformResult<NotificationId> {
        let id = NotificationId(request.payload.clone().unwrap_or_default());
        Self::post_notification(&request, &id.0)?;
        Ok(id)
    }

    async fn schedule_notification(
        &self,
        request: NotificationRequest,
        scheduled_at: DateTime<Utc>,
    ) -> PlatformResult<NotificationId> {
        // In-process only: Android kills the process at will, and a reminder
        // whose time arrives after that is lost. Moving this to AlarmManager is
        // tracked in plans/plan.md.
        let payload = request.payload.clone().unwrap_or_default();
        let id = NotificationId::scheduled(&payload, &scheduled_at);
        let delay = scheduled_at
            .signed_duration_since(Utc::now())
            .to_std()
            .unwrap_or_default();
        let tag = id.0.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            if let Err(error) = Self::post_notification(&request, &tag) {
                tracing::warn!(%error, "scheduled notification failed");
            }
        });

        let mut scheduled = self
            .scheduled_notifications
            .lock()
            .expect("scheduled notifications poisoned");
        scheduled.retain(|_, handle| !handle.is_finished());
        if let Some(previous) = scheduled.insert(id.clone(), handle) {
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

        with_env("cancel notification", |env, activity| {
            let manager = notification_manager(env, activity)?;
            let tag = java_string(env, &id.0)?;
            env.call_method(
                &manager,
                "cancel",
                "(Ljava/lang/String;I)V",
                &[JValue::Object(&tag), JValue::Int(0)],
            )?;
            Ok(())
        })
    }

    async fn pick_file(&self, filter: Option<FileFilter>) -> PlatformResult<Option<FileHandle>> {
        // SAF filters on MIME types, not extensions. Anything we cannot map is
        // widened to */* rather than silently hiding the user's file.
        let mime = filter
            .as_ref()
            .and_then(|f| f.extensions.first())
            .map(|extension| match extension.as_str() {
                "json" => "application/json",
                "txt" | "md" => "text/plain",
                _ => "*/*",
            })
            .unwrap_or("*/*");

        Self::request_document("pickDocument", mime.to_string()).await
    }

    async fn save_file(&self, suggested_name: &str) -> PlatformResult<Option<FileHandle>> {
        Self::request_document("createDocument", suggested_name.to_string()).await
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

        with_env("read content uri", |env, activity| {
            let stream = content_resolver_stream(
                env,
                activity,
                reference,
                "openInputStream",
                "(Landroid/net/Uri;)Ljava/io/InputStream;",
                None,
            )?;

            let mut contents = Vec::new();
            let chunk = env.new_byte_array(READ_CHUNK)?;
            loop {
                let read = env
                    .call_method(&stream, "read", "([B)I", &[JValue::Object(&chunk)])?
                    .i()?;
                if read <= 0 {
                    break;
                }
                let mut buffer = vec![0i8; read as usize];
                env.get_byte_array_region(&chunk, 0, &mut buffer)?;
                contents.extend(buffer.into_iter().map(|byte| byte as u8));
            }
            env.call_method(&stream, "close", "()V", &[])?;
            Ok(contents)
        })
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

        with_env("write content uri", |env, activity| {
            // "wt" truncates; without it a shorter document would keep the tail
            // of whatever was there before.
            let stream = content_resolver_stream(
                env,
                activity,
                reference,
                "openOutputStream",
                "(Landroid/net/Uri;Ljava/lang/String;)Ljava/io/OutputStream;",
                Some("wt"),
            )?;
            let bytes: JByteArray<'_> = env.byte_array_from_slice(contents)?;
            env.call_method(&stream, "write", "([B)V", &[JValue::Object(&bytes)])?;
            env.call_method(&stream, "flush", "()V", &[])?;
            env.call_method(&stream, "close", "()V", &[])?;
            Ok(())
        })
    }

    fn open_url(&self, url: &str) -> PlatformResult<()> {
        with_env("start view intent", |env, activity| {
            let target = java_string(env, url)?;
            let uri = env
                .call_static_method(
                    "android/net/Uri",
                    "parse",
                    "(Ljava/lang/String;)Landroid/net/Uri;",
                    &[JValue::Object(&target)],
                )?
                .l()?;
            let action = java_string(env, "android.intent.action.VIEW")?;
            let intent = env.new_object(
                "android/content/Intent",
                "(Ljava/lang/String;Landroid/net/Uri;)V",
                &[JValue::Object(&action), JValue::Object(&uri)],
            )?;
            // FLAG_ACTIVITY_NEW_TASK: the browser must not join our task, or
            // backing out of it would land the user inside Flequit's history.
            let flag = env
                .get_static_field("android/content/Intent", "FLAG_ACTIVITY_NEW_TASK", "I")?
                .i()?;
            env.call_method(
                &intent,
                "addFlags",
                "(I)Landroid/content/Intent;",
                &[JValue::Int(flag)],
            )?;
            env.call_method(
                activity,
                "startActivity",
                "(Landroid/content/Intent;)V",
                &[JValue::Object(&intent)],
            )?;
            Ok(())
        })
    }

    fn open_path(&self, path: &Path) -> PlatformResult<()> {
        // Handing out a file:// URI for a sandboxed path throws
        // FileUriExposedException on API 24+; doing it properly needs a
        // FileProvider, which the app does not declare.
        let _ = path;
        Err(PlatformError::Unsupported(
            "opening sandboxed paths on android",
        ))
    }

    fn delete_permanently(&self, path: &Path) -> PlatformResult<()> {
        // No OS trash on Android; delete outright.
        std::fs::remove_file(path).map_err(|source| PlatformError::Io {
            path: Some(path.to_path_buf()),
            source,
        })
    }
}

// --- JNI exports called by FlequitActivity --------------------------------

/// Delivers a document picker result.
///
/// `uri` is null when the user cancelled or the activity was recreated.
///
/// # Safety
///
/// Called by the JVM with a valid `JNIEnv` for the calling thread and with
/// arguments matching `FlequitActivity`'s `native` declaration.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_com_flequit_app_FlequitActivity_nativeOnActivityResult(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    request_code: i32,
    result_code: i32,
    uri: JString<'_>,
    display_name: JString<'_>,
) {
    let Some(sender) = pending_requests()
        .lock()
        .expect("pending requests poisoned")
        .remove(&request_code)
    else {
        tracing::debug!(request_code, "activity result for an unknown request");
        return;
    };

    let handle = if result_code == RESULT_OK && !uri.is_null() {
        // The name is cosmetic and the Java side may not have been able to
        // query it, so the URI stands in for it.
        let name = if display_name.is_null() {
            None
        } else {
            env.get_string(&display_name).ok().map(String::from)
        };

        env.get_string(&uri)
            .ok()
            .map(String::from)
            .map(|reference| FileHandle::Opaque {
                display_name: name.unwrap_or_else(|| reference.clone()),
                reference,
            })
    } else {
        None
    };

    // The receiver is gone when the awaiting task was cancelled; nothing to do.
    let _ = sender.send(handle);
}

/// Delivers a lifecycle transition observed by the activity.
///
/// # Safety
///
/// Called by the JVM with arguments matching `FlequitActivity`'s `native`
/// declaration; `event` is one of the codes [`LifecycleEvent::from_code`] knows.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_com_flequit_app_FlequitActivity_nativeOnLifecycleEvent(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    event: i32,
) {
    match LifecycleEvent::from_code(event) {
        Some(event) => lifecycle_hub().dispatch(event),
        None => tracing::warn!(event, "unknown lifecycle code from the android bridge"),
    }
}
