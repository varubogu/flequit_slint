//! Application lifecycle events.
//!
//! Desktop and mobile differ sharply here: a mobile OS suspends and then kills
//! the process without notice. Persist on every operation; never implement
//! save-on-exit.
//!
//! The OS reports these transitions on its own thread, so [`LifecycleHub`]
//! exists to fan them out to observers registered from anywhere in the app.
//! Backends own a hub; callers only see [`Platform::subscribe_lifecycle`].
//!
//! [`Platform::subscribe_lifecycle`]: crate::Platform::subscribe_lifecycle

use std::sync::{Arc, Mutex, Weak};

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

impl LifecycleEvent {
    /// Decodes the discriminant used at the FFI boundary with Java / Swift glue.
    ///
    /// Returns `None` for values the glue should never send, so an out-of-date
    /// bridge is ignored rather than misinterpreted.
    ///
    /// # Examples
    ///
    /// ```
    /// use flequit_platform::LifecycleEvent;
    ///
    /// assert_eq!(LifecycleEvent::from_code(0), Some(LifecycleEvent::Suspend));
    /// assert_eq!(LifecycleEvent::from_code(9), None);
    /// ```
    pub fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::Suspend),
            1 => Some(Self::Resume),
            2 => Some(Self::LowMemory),
            _ => None,
        }
    }
}

/// Receives lifecycle transitions.
///
/// Called from an OS thread, never from the UI thread. Implementations must not
/// touch Slint state directly — hop through `upgrade_in_event_loop()` instead.
pub trait LifecycleObserver: Send + Sync {
    fn on_event(&self, event: LifecycleEvent);
}

/// Fans lifecycle events out to every registered observer.
///
/// Observers are held weakly: dropping the `Arc` a caller registered
/// unsubscribes it, so a ViewModel that goes away does not keep receiving
/// events or leak.
#[derive(Default)]
pub struct LifecycleHub {
    observers: Mutex<Vec<Weak<dyn LifecycleObserver>>>,
}

impl LifecycleHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an observer for as long as the caller keeps its `Arc` alive.
    pub fn subscribe(&self, observer: &Arc<dyn LifecycleObserver>) {
        let mut observers = self.lock();
        observers.retain(|existing| existing.strong_count() > 0);
        observers.push(Arc::downgrade(observer));
    }

    /// Delivers `event` to every live observer.
    ///
    /// Observers are cloned out before being called so that one which
    /// subscribes or unsubscribes in response cannot deadlock the hub.
    pub fn dispatch(&self, event: LifecycleEvent) {
        let live: Vec<Arc<dyn LifecycleObserver>> = {
            let mut observers = self.lock();
            observers.retain(|existing| existing.strong_count() > 0);
            observers.iter().filter_map(Weak::upgrade).collect()
        };

        tracing::debug!(
            ?event,
            observers = live.len(),
            "dispatching lifecycle event"
        );
        for observer in live {
            observer.on_event(event);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Weak<dyn LifecycleObserver>>> {
        // A panicking observer must not silently stop lifecycle delivery for
        // the rest of the session; the list itself is always consistent.
        self.observers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    struct Counter {
        seen: Mutex<Vec<LifecycleEvent>>,
    }

    impl LifecycleObserver for Counter {
        fn on_event(&self, event: LifecycleEvent) {
            self.seen.lock().expect("counter poisoned").push(event);
        }
    }

    #[test]
    fn delivers_events_to_every_subscriber() {
        let hub = LifecycleHub::new();
        let first = Arc::new(Counter::default());
        let second = Arc::new(Counter::default());
        hub.subscribe(&(first.clone() as Arc<dyn LifecycleObserver>));
        hub.subscribe(&(second.clone() as Arc<dyn LifecycleObserver>));

        hub.dispatch(LifecycleEvent::Suspend);
        hub.dispatch(LifecycleEvent::Resume);

        let expected = vec![LifecycleEvent::Suspend, LifecycleEvent::Resume];
        assert_eq!(*first.seen.lock().unwrap(), expected);
        assert_eq!(*second.seen.lock().unwrap(), expected);
    }

    #[test]
    fn dropping_the_subscriber_unsubscribes_it() {
        struct Tally(Arc<AtomicUsize>);
        impl LifecycleObserver for Tally {
            fn on_event(&self, _event: LifecycleEvent) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let hub = LifecycleHub::new();
        let observer = Arc::new(Tally(calls.clone()));
        hub.subscribe(&(observer.clone() as Arc<dyn LifecycleObserver>));

        hub.dispatch(LifecycleEvent::Resume);
        drop(observer);
        hub.dispatch(LifecycleEvent::Resume);

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
