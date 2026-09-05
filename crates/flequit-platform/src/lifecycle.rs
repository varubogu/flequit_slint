//! Application lifecycle events.
//!
//! Desktop and mobile differ sharply here: a mobile OS suspends and then kills
//! the process without notice. Persist on every operation; never implement
//! save-on-exit.

/// A lifecycle transition reported by the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent {
    /// The app moved to the background.
    ///
    /// Flush unsaved changes, stop sync tasks and timers.
    Suspend,
    /// The app returned to the foreground.
    ///
    /// Reload data and restart sync.
    Resume,
    /// The OS is asking the app to reduce memory usage.
    ///
    /// Release Automerge documents for inactive projects. Never fires on desktop.
    LowMemory,
}

/// Receives lifecycle transitions.
pub trait LifecycleObserver: Send + Sync {
    fn on_event(&self, event: LifecycleEvent);
}
