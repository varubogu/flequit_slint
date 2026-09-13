//! ViewModel layer.
//!
//! ViewModels are the only place that registers Slint callbacks and the only
//! place that calls `flequit-core` facades. They own the `VecModel`s backing the
//! UI lists, apply optimistic updates, and roll them back when persistence fails.
//!
//! See `docs/ja/develop/design/ui/viewmodel-architecture.md`.

pub mod app;
pub mod ordering;
pub mod project_editor;
pub mod recurrence;
pub mod reload_gate;
mod runtime_store;
pub mod search;
pub mod settings;
pub mod tag_editor;
pub mod task_list_ui;

pub use app::AppViewModel;
pub use search::SearchQuery;
pub use settings::{
    CustomDueFilter, CustomDueUnit, DateTimeFormatKind, DateTimeFormatPreference,
    DueButtonPreference, RecurrencePreset, ReminderPreset, SettingsSaveFuture, SettingsStore,
    SettingsStoreError, UserSettings, resolve_locale, system_locale,
};
pub use task_list_ui::TaskListUiViewModel;
