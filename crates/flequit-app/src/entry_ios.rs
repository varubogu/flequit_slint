//! iOS entry point.
//!
//! Enabled by the `ios` feature rather than by `cfg(target_os)`, because
//! `#[cfg(target_os = ...)]` is reserved for `flequit-platform` (see
//! `scripts/check-crate-deps.sh`).
//!
//! Unlike Android, iOS does not start the process in Rust: `UIApplicationMain`
//! owns the run loop and the app delegate lives in Swift. The Swift side calls
//! [`flequit_ios_main`] once the delegate has a window scene, and this crate is
//! linked into the app as a static library.
//!
//! See `mobile/ios/README.md` for the Xcode project layout.

use std::ffi::c_int;

/// Exit code reported to the Swift caller when the bootstrap fails.
const BOOTSTRAP_FAILED: c_int = 1;

/// Runs the application. Does not return until the UI is torn down.
///
/// # Safety
///
/// Must be called exactly once, from the main thread, after
/// `UIApplicationMain` has created the application object. Calling it from a
/// background thread would run the Slint event loop off the main thread, which
/// UIKit forbids.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flequit_ios_main() -> c_int {
    match crate::run() {
        Ok(()) => 0,
        Err(error) => {
            // Logging may not be installed yet if the failure happened early,
            // so this also goes to stderr, which Xcode's console shows.
            eprintln!("flequit failed to start: {error}");
            tracing::error!(%error, "flequit failed to start");
            BOOTSTRAP_FAILED
        }
    }
}
