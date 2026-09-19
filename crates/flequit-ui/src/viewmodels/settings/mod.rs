//! Settings dialog state, callback wiring, and persistence boundary.

mod binding;
mod datetime_format;
mod font;
mod locale;
mod model;
mod mutations;
mod navigation;
mod publisher;
mod reminder;
mod timezone;
mod worker;

#[cfg(test)]
mod tests;

pub use binding::{SearchMemory, SettingsViewModel};
pub use font::filter as filter_fonts;
pub use locale::{SUPPORTED_LOCALES, resolve_locale, supported, system_locale};
pub use model::{
    CustomDueFilter, CustomDueUnit, DateTimeFormatKind, DateTimeFormatPreference,
    DueButtonPreference, RECENT_ADD_TARGETS, RecurrencePreset, ReminderPreset, SettingsSaveFuture,
    SettingsStore, SettingsStoreError, UserSettings,
};
