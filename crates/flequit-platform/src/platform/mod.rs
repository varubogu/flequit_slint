//! Platform backends.
//!
//! **This is the only module in the workspace allowed to use
//! `#[cfg(target_os = ...)]`.** Everything above it sees the
//! [`crate::Platform`] trait.

use std::sync::Arc;

use crate::{LogLineEmitter, Platform, PlatformResult};

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
mod desktop;

#[cfg(target_os = "android")]
pub mod android;

#[cfg(target_os = "ios")]
pub mod ios;

#[cfg(target_arch = "wasm32")]
mod web;

/// Builds the backend for the current target.
pub fn build() -> PlatformResult<Arc<dyn Platform>> {
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    {
        desktop::DesktopPlatform::new().map(|p| Arc::new(p) as Arc<dyn Platform>)
    }

    #[cfg(target_os = "android")]
    {
        android::AndroidPlatform::new().map(|p| Arc::new(p) as Arc<dyn Platform>)
    }

    #[cfg(target_os = "ios")]
    {
        ios::IosPlatform::new().map(|p| Arc::new(p) as Arc<dyn Platform>)
    }

    #[cfg(target_arch = "wasm32")]
    {
        web::WebPlatform::new().map(|p| Arc::new(p) as Arc<dyn Platform>)
    }

    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "linux",
        target_os = "android",
        target_os = "ios",
        target_arch = "wasm32",
    )))]
    {
        Err(crate::PlatformError::Unsupported("this target platform"))
    }
}

/// The platform's own log sink, or `None` when stderr already reaches the
/// platform's tooling.
///
/// See [`crate::logging`] for why this decision lives in the platform layer.
pub(crate) fn log_line_emitter() -> Option<LogLineEmitter> {
    #[cfg(target_os = "android")]
    {
        Some(android::write_to_logcat as LogLineEmitter)
    }

    #[cfg(target_arch = "wasm32")]
    {
        Some(web::write_to_console as LogLineEmitter)
    }

    // Desktop terminals and the Xcode console both read stderr, which the
    // default `fmt` layer already writes to. iOS is included deliberately: a
    // second sink would duplicate every line in the Xcode console.
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    {
        None
    }
}
