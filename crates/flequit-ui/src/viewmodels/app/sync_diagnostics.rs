//! Settings > Data sync: the state of the Automerge sync queue.
//!
//! Writes reach Automerge through a queue in SQLite. Changes the queue gave up
//! on (`failed`) are listed here with their last error, and can be put back.
//! Whether a change may be put back is decided below the facade: one that a
//! later, already-applied change would be overwritten by is left alone and
//! counted separately.

use std::sync::Arc;

use flequit_core::InfrastructureRepositoriesTrait;
use flequit_core::facades::sync_diagnostics_facades;
use flequit_core::ports::infrastructure_repositories::{
    FailedSyncChange, SyncQueueSummary, SyncRequeueReport,
};
use flequit_types::errors::service_error::ServiceError;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};
use tokio::runtime::Handle;

use super::{display_settings, report_error};
use crate::adapters::datetime::{DisplayTimezone, format_due};
use crate::bindings::{Actions, AppWindow, FailedSyncChangeItem, SettingsState};

/// How many failed changes the section lists. The oldest come first: they are
/// the ones holding up nothing any more but most likely to need attention.
const FAILED_PAGE: u64 = 50;

/// Upper bound for "Retry all". The queue keeps failed rows until requeued, so
/// this only guards against a runaway table.
const REQUEUE_ALL_LIMIT: u64 = 10_000;

/// What the section shows, read off the UI thread.
struct SyncStatus {
    /// `None` when the storage configuration keeps no queue.
    summary: Option<SyncQueueSummary>,
    failed: Vec<FailedSyncChange>,
}

pub(super) fn bind<R>(
    window: &AppWindow,
    repositories: &Arc<R>,
    runtime: &Handle,
    timezone: DisplayTimezone,
) where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    let actions = window.global::<Actions>();

    {
        let weak = window.as_weak();
        let repositories = Arc::clone(repositories);
        let runtime = runtime.clone();
        actions.on_refresh_sync_status(move || {
            start_loading(&weak);
            let weak = weak.clone();
            let repositories = Arc::clone(&repositories);
            runtime.spawn(async move {
                let status = read_status(repositories.as_ref()).await;
                publish_status(&weak, status, None, timezone);
            });
        });
    }

    {
        let weak = window.as_weak();
        let repositories = Arc::clone(repositories);
        let runtime = runtime.clone();
        actions.on_requeue_failed_sync_change(move |id| {
            let Ok(id) = id.parse::<i64>() else {
                tracing::warn!(%id, "requeue asked for a sync queue row id that is not a number");
                report_error(&weak, "input.malformed-id");
                return;
            };
            start_loading(&weak);
            let weak = weak.clone();
            let repositories = Arc::clone(&repositories);
            runtime.spawn(async move {
                requeue(&weak, repositories.as_ref(), Some(id), timezone).await;
            });
        });
    }

    {
        let weak = window.as_weak();
        let repositories = Arc::clone(repositories);
        let runtime = runtime.clone();
        actions.on_requeue_all_failed_sync_changes(move || {
            start_loading(&weak);
            let weak = weak.clone();
            let repositories = Arc::clone(&repositories);
            runtime.spawn(async move {
                requeue(&weak, repositories.as_ref(), None, timezone).await;
            });
        });
    }
}

/// Requeues `id`, or every failed change when `None`, then re-reads the queue.
async fn requeue<R>(
    weak: &Weak<AppWindow>,
    repositories: &R,
    id: Option<i64>,
    timezone: DisplayTimezone,
) where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let ids = match id {
        Some(id) => Ok(vec![id]),
        None => sync_diagnostics_facades::list_failed_sync_changes(repositories, REQUEUE_ALL_LIMIT)
            .await
            .map(|changes| changes.into_iter().map(|change| change.id).collect()),
    };
    let report = match ids {
        Ok(ids) => sync_diagnostics_facades::requeue_failed_sync_changes(repositories, &ids).await,
        Err(error) => Err(error),
    };
    let report = match report {
        Ok(report) => Some(report),
        Err(error) => {
            tracing::warn!(%error, "failed to requeue failed Automerge sync changes");
            report_error(weak, "storage.failed");
            None
        }
    };
    let status = read_status(repositories).await;
    publish_status(weak, status, report, timezone);
}

async fn read_status<R>(repositories: &R) -> Result<SyncStatus, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let summary = sync_diagnostics_facades::sync_queue_summary(repositories).await?;
    let failed = match summary {
        Some(summary) if summary.failed > 0 => {
            sync_diagnostics_facades::list_failed_sync_changes(repositories, FAILED_PAGE).await?
        }
        _ => Vec::new(),
    };
    Ok(SyncStatus { summary, failed })
}

fn start_loading(weak: &Weak<AppWindow>) {
    if let Some(window) = weak.upgrade() {
        window.global::<SettingsState>().set_sync_loading(true);
    }
}

fn publish_status(
    weak: &Weak<AppWindow>,
    status: Result<SyncStatus, ServiceError>,
    report: Option<SyncRequeueReport>,
    timezone: DisplayTimezone,
) {
    let status = status.map_err(|error| {
        tracing::warn!(%error, "failed to read the Automerge sync queue");
    });
    let posted = weak.upgrade_in_event_loop(move |window| {
        let display = display_settings(&window, timezone);
        let settings = window.global::<SettingsState>();
        settings.set_sync_loading(false);
        if let Some(report) = report {
            settings.set_sync_has_requeue_result(true);
            settings.set_sync_requeued_count(count(report.requeued.len()));
            settings.set_sync_superseded_count(count(report.superseded.len()));
        }
        let Ok(status) = status else {
            settings.set_sync_load_failed(true);
            return;
        };
        settings.set_sync_load_failed(false);
        let summary = status.summary.unwrap_or_default();
        settings.set_sync_available(status.summary.is_some());
        settings.set_sync_pending_count(count(summary.pending));
        settings.set_sync_failed_count(count(summary.failed));
        let items: Vec<FailedSyncChangeItem> = status
            .failed
            .into_iter()
            .map(|change| FailedSyncChangeItem {
                id: change.id.to_string().into(),
                kind: change.change_kind.into(),
                document: change.document_key.into(),
                attempts: change.attempts,
                error: change.last_error.unwrap_or_default().into(),
                created_at: SharedString::from(format_due(&change.created_at, &display)),
            })
            .collect();
        settings.set_sync_failed_changes(ModelRc::new(VecModel::from(items)));
    });
    if let Err(error) = posted {
        tracing::error!(%error, "could not deliver the sync status to the UI thread");
    }
}

/// Slint counts are `i32`; a queue that large is already unreadable on screen.
fn count(value: impl TryInto<i32>) -> i32 {
    value.try_into().unwrap_or(i32::MAX)
}
