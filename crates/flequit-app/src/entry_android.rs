//! Android entry point.
//!
//! Enabled by the `android` feature rather than by `cfg(target_os)`, because
//! `#[cfg(target_os = ...)]` is reserved for `flequit-platform` (see
//! `scripts/check-crate-deps.sh`). The mobile build passes
//! `--features android`; a desktop build never sees this module.
//!
//! The feature is only usable when the target *is* Android: `slint::android`
//! itself is gated on `target_os = "android"`, so
//! `cargo check --features android` on a desktop host fails by design. Build it
//! as `cargo check --target aarch64-linux-android --features android`.
//!
//! Android starts the process at `android_main`; `#[unsafe(no_mangle)]` is what
//! makes the loader find it. Everything after `slint::android::init` is the
//! ordinary [`crate::run`] bootstrap: paths, notifications and lifecycle all
//! resolve themselves through `ndk-context`, which `android-activity` has
//! populated by this point, so nothing has to be threaded through by hand.

/// Process entry point invoked by `NativeActivity`.
#[unsafe(no_mangle)]
pub fn android_main(app: slint::android::AndroidApp) {
    // Installs Slint's Android backend. Nothing may create a window before this
    // returns, so it happens ahead of the bootstrap rather than inside it.
    if let Err(error) = slint::android::init(app) {
        eprintln!("flequit: could not initialise the android backend: {error}");
        return;
    }

    // Returning from `android_main` ends the process, so a failure is logged and
    // then falls through — there is no shell to report an exit code to.
    if let Err(error) = crate::run() {
        tracing::error!(%error, "flequit failed to start");
    }
}
