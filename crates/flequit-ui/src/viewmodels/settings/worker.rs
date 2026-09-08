use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{ComponentHandle, Weak};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use super::model::{SettingsModel, SettingsStore, UserSettings};
use super::publisher;
use crate::bindings::{AppState, AppWindow, I18n};

const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

struct SaveRequest {
    weak: Weak<AppWindow>,
    settings: UserSettings,
    revision: u64,
}

pub(super) struct SettingsQueue {
    model: Arc<Mutex<SettingsModel>>,
    sender: mpsc::UnboundedSender<SaveRequest>,
}

impl SettingsQueue {
    pub(super) fn new(
        settings: UserSettings,
        store: Arc<dyn SettingsStore>,
        runtime: Handle,
    ) -> Self {
        let model = Arc::new(Mutex::new(SettingsModel::new(settings.normalize())));
        let (sender, mut receiver) = mpsc::unbounded_channel::<SaveRequest>();
        let worker_model = Arc::clone(&model);
        runtime.spawn(async move {
            while let Some(mut request) = receiver.recv().await {
                loop {
                    match tokio::time::timeout(SAVE_DEBOUNCE, receiver.recv()).await {
                        Ok(Some(newer)) => request = newer,
                        Ok(None) => {
                            process_save_request(&worker_model, store.as_ref(), request).await;
                            return;
                        }
                        Err(_) => break,
                    }
                }
                process_save_request(&worker_model, store.as_ref(), request).await;
            }
        });
        Self { model, sender }
    }

    pub(super) fn current(&self) -> UserSettings {
        self.model
            .lock()
            .expect("settings state poisoned")
            .current
            .clone()
    }

    pub(super) fn updater(&self, weak: Weak<AppWindow>) -> SettingsUpdater {
        SettingsUpdater {
            weak,
            model: Arc::clone(&self.model),
            sender: self.sender.clone(),
        }
    }

    pub(super) fn clone_reader(
        &self,
    ) -> impl Fn() -> Vec<super::model::DateTimeFormatPreference> + 'static {
        let model = Arc::clone(&self.model);
        move || {
            model
                .lock()
                .expect("settings state poisoned")
                .current
                .datetime_formats
                .clone()
        }
    }

    pub(super) fn settings_reader(&self) -> impl Fn() -> UserSettings + 'static {
        let model = Arc::clone(&self.model);
        move || {
            model
                .lock()
                .expect("settings state poisoned")
                .current
                .clone()
        }
    }
}

#[derive(Clone)]
pub(super) struct SettingsUpdater {
    weak: Weak<AppWindow>,
    model: Arc<Mutex<SettingsModel>>,
    sender: mpsc::UnboundedSender<SaveRequest>,
}

impl SettingsUpdater {
    pub(super) fn update(&self, mutate: impl FnOnce(&mut UserSettings)) {
        let Some((next, revision)) = self
            .model
            .lock()
            .expect("settings state poisoned")
            .update(mutate)
        else {
            return;
        };

        if let Some(window) = self.weak.upgrade() {
            publisher::publish(&window, &next);
        }
        if self
            .sender
            .send(SaveRequest {
                weak: self.weak.clone(),
                settings: next,
                revision,
            })
            .is_err()
        {
            tracing::error!("settings save worker stopped unexpectedly");
        }
    }
}

async fn process_save_request(
    model: &Mutex<SettingsModel>,
    store: &dyn SettingsStore,
    request: SaveRequest,
) {
    match store.save(request.settings.clone()).await {
        Ok(()) => model
            .lock()
            .expect("settings state poisoned")
            .mark_persisted(request.settings),
        Err(error) => {
            tracing::error!(%error, "failed to persist settings");
            let rollback = model
                .lock()
                .expect("settings state poisoned")
                .rollback_if_current(request.revision);
            if let Some(settings) = rollback {
                post_rollback(&request.weak, settings);
            }
        }
    }
}

fn post_rollback(weak: &Weak<AppWindow>, settings: UserSettings) {
    let posted = weak.upgrade_in_event_loop(move |window| {
        publisher::publish(&window, &settings);
        let message = window
            .global::<I18n>()
            .invoke_error_message("settings.save-failed".into());
        window.global::<AppState>().set_error_message(message);
    });
    if let Err(error) = posted {
        tracing::error!(%error, "could not deliver settings rollback to UI");
    }
}
