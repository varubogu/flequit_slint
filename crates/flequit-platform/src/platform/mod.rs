//! Platform backends.
//!
//! **This is the only module in the workspace allowed to use
//! `#[cfg(target_os = ...)]`.** Everything above it sees the
//! [`crate::Platform`] trait.

use std::sync::Arc;

use crate::{Platform, PlatformResult};

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
mod desktop;

#[cfg(target_os = "android")]
mod android;

#[cfg(target_os = "ios")]
mod ios;

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

    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "linux",
        target_os = "android",
        target_os = "ios",
    )))]
    {
        Err(crate::PlatformError::Unsupported("this target platform"))
    }
}
