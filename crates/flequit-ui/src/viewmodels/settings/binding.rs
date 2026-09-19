use std::sync::Arc;

use slint::ComponentHandle;
use tokio::runtime::Handle;

use super::font::FontCatalogue;
use super::model::{NoopSettingsStore, SettingsStore, UserSettings};
use super::worker::{SettingsQueue, SettingsUpdater};
use super::{datetime_format, font, mutations, navigation, publisher, reminder, timezone};
use crate::bindings::AppWindow;

/// Owns settings values and connects dialog actions to persistence.
pub struct SettingsViewModel {
    queue: SettingsQueue,
    fonts: FontCatalogue,
}

impl SettingsViewModel {
    pub fn new(settings: UserSettings, store: Arc<dyn SettingsStore>, runtime: Handle) -> Self {
        Self {
            queue: SettingsQueue::new(settings, store, runtime),
            fonts: FontCatalogue::default(),
        }
    }

    pub fn new_without_persistence(runtime: Handle) -> Self {
        Self::new(
            UserSettings::default(),
            Arc::new(NoopSettingsStore),
            runtime,
        )
    }

    pub(crate) fn new_for_test(runtime: Handle) -> Self {
        let settings = UserSettings {
            timezone: "UTC".to_string(),
            ..UserSettings::default()
        };
        Self::new(settings, Arc::new(NoopSettingsStore), runtime)
    }

    /// The saved search query and recent quick-add destinations.
    pub fn saved_search(&self) -> (String, Vec<String>) {
        let current = self.queue.current();
        (current.search_query, current.recent_add_targets)
    }

    /// A handle that saves the search state without republishing settings.
    pub fn search_memory(&self, window: &AppWindow) -> SearchMemory {
        SearchMemory {
            updater: self.queue.updater(window.as_weak()),
        }
    }

    pub fn apply(&self, window: &AppWindow) {
        publisher::publish(window, &self.queue.current());
        // Seed the dropdowns so they list everything before the first keystroke.
        timezone::publish_matches(window, "");
        font::publish_matches(window, &self.fonts, "");
    }

    /// Records the families the platform reported and refreshes the dropdown.
    ///
    /// Called from the font-scan thread's UI-thread callback, so the list is
    /// empty until the scan finishes.
    pub fn set_available_fonts(&self, window: &AppWindow, families: Vec<String>) {
        self.fonts.replace(families);
        font::publish_matches(window, &self.fonts, "");
    }

    pub fn bind(&self, window: &AppWindow) {
        navigation::bind(window);
        timezone::bind(window);
        datetime_format::bind(window, &self.queue);
        reminder::bind(window, &self.queue);
        font::bind(window, &self.fonts);
        mutations::bind(window, &self.queue);
    }
}

/// Saves what the search box and the quick-add field should reopen with.
#[derive(Clone)]
pub struct SearchMemory {
    updater: SettingsUpdater,
}

impl std::fmt::Debug for SearchMemory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SearchMemory")
            .finish_non_exhaustive()
    }
}

impl SearchMemory {
    pub fn remember(&self, query: String, recent_add_targets: Vec<String>) {
        self.updater.update_silently(move |settings| {
            settings.search_query = query;
            settings.recent_add_targets = recent_add_targets;
        });
    }
}
