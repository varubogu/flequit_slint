//! Slint user interface and ViewModel layer for Flequit.
//!
//! # Layers
//!
//! | Layer | Module | Responsibility |
//! |---|---|---|
//! | View | `ui/**/*.slint` | Layout and input only; no domain types |
//! | Adapter | [`adapters`] | Pure domain → UI type conversion |
//! | ViewModel | [`viewmodels`] | UI state, callbacks, facade calls |
//!
//! Only ViewModels call `flequit-core`. The UI never reaches for a repository
//! or an OS API directly — the latter goes through `flequit-platform`.
//!
//! See `docs/ja/develop/design/ui/layers.md`.

pub mod adapters;
pub mod bindings;
pub mod error;
pub mod viewmodels;

pub use bindings::AppWindow;
pub use error::{UiError, UiResult};
pub use viewmodels::{
    AppViewModel, CustomDueFilter, CustomDueUnit, DateTimeFormatKind, DateTimeFormatPreference,
    DueButtonPreference, RecurrencePreset, ReminderPreset, SettingsSaveFuture, SettingsStore,
    SettingsStoreError, UserSettings, resolve_locale, system_locale,
};
