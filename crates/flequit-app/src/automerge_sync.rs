//! Keeps the Automerge sync queue moving across the app's lifecycle.
//!
//! Writes commit to SQLite together with a queue row; a background worker
//! applies the rows to Automerge. On desktop the worker runs until the window
//! closes and `run()` drains it. A mobile OS instead suspends the process when
//! it moves to the background and may kill it later without notice, so the
//! queue is applied as the app goes to the background too.

use std::sync::Arc;
use std::time::Duration;

use flequit_infrastructure::automerge_sync::AutomergeSyncProcessor;
use flequit_platform::{LifecycleEvent, LifecycleObserver, Platform, PlatformError};

/// How long moving to the background may wait for the queue to be applied.
///
/// The OS delivers the transition on the main thread and expects it back
/// quickly (Android reports an ANR after 5 s, iOS kills an app that has not
/// returned from `applicationDidEnterBackground` within about 5 s). Waiting
/// here is also what keeps the process awake long enough to finish. Whatever is
/// left stays queued in SQLite and is applied on resume or next launch.
const SUSPEND_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// Applies the queue when the app moves to the background.
struct FlushOnSuspend {
    processor: Arc<AutomergeSyncProcessor>,
    runtime: tokio::runtime::Handle,
}

impl LifecycleObserver for FlushOnSuspend {
    fn on_event(&self, event: LifecycleEvent) {
        if event != LifecycleEvent::Suspend {
            return;
        }
        let processor = Arc::clone(&self.processor);
        let (done, finished) = std::sync::mpsc::channel();
        self.runtime.spawn(async move {
            match processor.process_pending().await {
                Ok(report) => tracing::info!(
                    processed = report.processed,
                    retried = report.retried,
                    failed = report.failed,
                    "applied the Automerge sync queue before moving to the background"
                ),
                Err(error) => tracing::warn!(
                    %error,
                    "failed to apply the Automerge sync queue before moving to the background"
                ),
            }
            let _ = done.send(());
        });
        // Called on an OS thread, never a runtime worker, so blocking is allowed.
        if finished.recv_timeout(SUSPEND_FLUSH_TIMEOUT).is_err() {
            tracing::warn!(
                "the Automerge sync queue was still being applied when the app moved to the background"
            );
        }
    }
}

/// Subscribes the queue flush to lifecycle events.
///
/// Returns the observer, which the caller must hold for the whole session: the
/// platform keeps only a weak reference. `None` on platforms without lifecycle
/// events (desktop) or when there is no queue to apply.
#[must_use = "dropping the observer unsubscribes it"]
pub(crate) fn flush_on_suspend(
    platform: &dyn Platform,
    processor: Option<&Arc<AutomergeSyncProcessor>>,
    runtime: &tokio::runtime::Handle,
) -> Option<Arc<dyn LifecycleObserver>> {
    let observer: Arc<dyn LifecycleObserver> = Arc::new(FlushOnSuspend {
        processor: Arc::clone(processor?),
        runtime: runtime.clone(),
    });
    match platform.subscribe_lifecycle(Arc::clone(&observer)) {
        Ok(()) => Some(observer),
        Err(PlatformError::Unsupported(_)) => None,
        Err(error) => {
            tracing::warn!(%error, "could not subscribe the Automerge sync queue to lifecycle events");
            None
        }
    }
}
