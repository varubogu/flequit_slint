//! Runtime feature detection.
//!
//! Unsupported functionality is reported through [`Capability`] rather than by
//! failing at call time, so the UI can hide affordances it cannot honour.

use std::collections::BTreeSet;

/// A platform feature that may or may not be available at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// Persistent system tray / menu bar icon.
    SystemTray,
    /// Application-wide keyboard shortcuts registered with the OS.
    GlobalHotkey,
    /// More than one top-level window.
    MultiWindow,
    /// Moving files to the OS trash instead of deleting them.
    OsTrash,
    /// Reading and writing arbitrary filesystem paths (as opposed to sandboxed handles).
    ArbitraryFilePath,
    /// Local (non-push) notifications, used for reminders.
    LocalNotification,
    /// A file picker of any kind.
    FilePicker,
    /// Synchronisation while the app is not in the foreground.
    ///
    /// Available on mobile only under OS-imposed limits; treat as interruptible.
    BackgroundSync,
    /// In-app self-update. Disabled for store-distributed builds.
    SelfUpdate,
    /// Listing the fonts installed on the system.
    ///
    /// Mobile platforms ship a fixed set, so the picker offers named choices
    /// there instead of an enumeration.
    FontEnumeration,
}

/// The set of features available on the current platform and build.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    supported: BTreeSet<Capability>,
}

impl Capabilities {
    /// Builds a capability set from an iterator of supported features.
    pub fn from_supported(items: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            supported: items.into_iter().collect(),
        }
    }

    /// Returns whether `capability` can be used on this platform.
    ///
    /// # Examples
    ///
    /// ```
    /// use flequit_platform::{Capabilities, Capability};
    ///
    /// let caps = Capabilities::from_supported([Capability::FilePicker]);
    /// assert!(caps.has(Capability::FilePicker));
    /// assert!(!caps.has(Capability::SystemTray));
    /// ```
    pub fn has(&self, capability: Capability) -> bool {
        self.supported.contains(&capability)
    }

    /// Iterates over the supported capabilities in a stable order.
    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.supported.iter().copied()
    }
}

/// Coarse device class, derived from the platform rather than the window size.
///
/// This is a *hint* for defaults (initial window size, whether to start the
/// sidebar collapsed). Layout itself must branch on window width instead —
/// see `docs/ja/develop/design/ui/responsive-layout.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormFactor {
    Desktop,
    Tablet,
    Phone,
}
