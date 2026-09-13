use std::future::Future;
use std::pin::Pin;

use crate::bindings::{RecurrenceUnit, ReminderUnit, ThemeMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimeFormatKind {
    Default,
    Preset,
    Custom,
    CustomFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTimeFormatPreference {
    pub id: String,
    pub name: String,
    pub format: String,
    pub kind: DateTimeFormatKind,
    pub order: i32,
}

impl Default for DateTimeFormatPreference {
    fn default() -> Self {
        Self {
            id: "-1".to_string(),
            name: "default".to_string(),
            format: String::new(),
            kind: DateTimeFormatKind::Default,
            order: 0,
        }
    }
}

/// Built-in due filters and their defaults.
pub(super) const BUILTIN_DUE_FILTERS: &[(&str, &str, bool)] = &[
    ("overdue", "@overdue", false),
    ("today", "@today", true),
    ("tomorrow", "@tomorrow", true),
    ("three-days", "@3days", false),
    ("this-week", "@week", true),
    ("this-month", "@month", false),
    ("this-quarter", "@quarter", false),
    ("this-year", "@year", false),
    ("this-fiscal-year", "@fiscalyear", false),
];

/// Longest name a user can give a filter or preset, in characters.
pub const MAX_NAME_CHARS: usize = 40;

/// Trims a user-entered name and caps its length. Empty means "use the
/// translated default label".
pub fn clean_name(name: &str) -> String {
    name.trim().chars().take(MAX_NAME_CHARS).collect()
}

/// A UI-facing due-filter preference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueButtonPreference {
    pub key: String,
    pub query: String,
    pub visible: bool,
    /// What the user calls this filter, e.g. "今期" for this quarter. Empty
    /// shows the translated default label.
    pub name: String,
}

/// The horizon a user-defined due filter covers.
///
/// Mirrors the units the settings dialog offers. Anything longer is already
/// covered by the built-in week/month/quarter/year buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CustomDueUnit {
    Minute,
    Hour,
    Day,
}

impl CustomDueUnit {
    /// Largest value that still makes sense for the unit.
    pub fn max_value(self) -> i32 {
        match self {
            Self::Minute => 43_200,
            Self::Hour => 8_760,
            Self::Day => 3_650,
        }
    }

    /// The search keyword the filter writes into the search box.
    fn keyword(self) -> &'static str {
        match self {
            Self::Minute => "minutes",
            Self::Hour => "hours",
            Self::Day => "days",
        }
    }
}

/// A due filter the user added, e.g. "within 10 minutes" or "next 5 days".
///
/// The unit and the value identify the filter; the name is only what it is
/// called, so two filters differing only by name are the same filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomDueFilter {
    pub unit: CustomDueUnit,
    pub value: i32,
    /// Empty shows the translated default label.
    pub name: String,
}

impl CustomDueFilter {
    pub fn new(value: i32, unit: CustomDueUnit) -> Self {
        Self::named(value, unit, String::new())
    }

    pub fn named(value: i32, unit: CustomDueUnit, name: String) -> Self {
        Self { unit, value, name }
    }

    /// Ordered by unit first so the list reads shortest horizon first.
    pub(super) fn sort_key(&self) -> (CustomDueUnit, i32) {
        (self.unit, self.value)
    }

    pub fn same_filter(&self, other: &Self) -> bool {
        self.sort_key() == other.sort_key()
    }

    /// Stable identity used as the sidebar button key.
    pub fn key(&self) -> String {
        format!("custom-{}-{}", self.value, self.unit.keyword())
    }

    /// Search text the button writes into the search box.
    pub fn query(&self) -> String {
        format!("@{}{}", self.value, self.unit.keyword())
    }

    pub fn is_valid(&self) -> bool {
        (1..=self.unit.max_value()).contains(&self.value)
    }
}

/// A recurrence pattern the user saved for reuse.
///
/// The Slint-generated `RecurrenceUnit` has no `Ord`, so the ordering the list
/// is shown in comes from `sort_key` rather than a derive.
#[derive(Debug, Clone, PartialEq)]
pub struct RecurrencePreset {
    pub unit: RecurrenceUnit,
    pub interval: i32,
    /// Empty shows the translated default summary.
    pub name: String,
}

impl RecurrencePreset {
    pub fn new(interval: i32, unit: RecurrenceUnit) -> Self {
        Self::named(interval, unit, String::new())
    }

    pub fn named(interval: i32, unit: RecurrenceUnit, name: String) -> Self {
        Self {
            unit,
            interval,
            name,
        }
    }

    /// The name does not take part: it only says what the pattern is called.
    pub fn same_pattern(&self, other: &Self) -> bool {
        self.sort_key() == other.sort_key()
    }

    /// An interval of zero or less repeats nothing, and anything past a few
    /// hundred periods is better expressed with a longer unit.
    pub fn is_valid(&self) -> bool {
        (1..=999).contains(&self.interval)
    }

    /// Shortest period first, so the list reads the way the units do.
    pub(super) fn sort_key(&self) -> (u8, i32) {
        let unit = match self.unit {
            RecurrenceUnit::Minute => 0,
            RecurrenceUnit::Hour => 1,
            RecurrenceUnit::Day => 2,
            RecurrenceUnit::Week => 3,
            RecurrenceUnit::Month => 4,
            RecurrenceUnit::Quarter => 5,
            RecurrenceUnit::HalfYear => 6,
            RecurrenceUnit::Year => 7,
        };
        (unit, self.interval)
    }
}

/// A relative reminder choice saved for reuse in the task detail editor.
#[derive(Debug, Clone, PartialEq)]
pub struct ReminderPreset {
    pub unit: ReminderUnit,
    pub value: i32,
    /// Empty shows the translated default label.
    pub name: String,
}

impl ReminderPreset {
    pub fn new(value: i32, unit: ReminderUnit) -> Self {
        Self::named(value, unit, String::new())
    }

    pub fn named(value: i32, unit: ReminderUnit, name: String) -> Self {
        Self { unit, value, name }
    }

    /// The name does not take part: it only says what the offset is called.
    pub fn same_offset(&self, other: &Self) -> bool {
        self.unit == other.unit && self.value == other.value
    }

    pub fn is_valid(&self) -> bool {
        (1..=self.max_value()).contains(&self.value)
    }

    pub fn minutes_before(&self) -> i32 {
        match self.unit {
            ReminderUnit::Minute => self.value,
            ReminderUnit::Hour => self.value.saturating_mul(60),
            ReminderUnit::Day => self.value.saturating_mul(24 * 60),
        }
    }

    fn max_value(&self) -> i32 {
        match self.unit {
            ReminderUnit::Minute => 43_200,
            ReminderUnit::Hour => 8_760,
            ReminderUnit::Day => 3_650,
        }
    }

    pub(super) fn sort_key(&self) -> (i32, u8, i32) {
        let unit = match self.unit {
            ReminderUnit::Minute => 0,
            ReminderUnit::Hour => 1,
            ReminderUnit::Day => 2,
        };
        (self.minutes_before(), unit, self.value)
    }
}

fn default_reminder_presets() -> Vec<ReminderPreset> {
    vec![
        ReminderPreset::new(30, ReminderUnit::Minute),
        ReminderPreset::new(1, ReminderUnit::Hour),
        ReminderPreset::new(1, ReminderUnit::Day),
    ]
}

/// Preferences owned by the settings screen.
///
/// This deliberately contains no `flequit-settings` types. The application
/// crate adapts this value at the composition root, preserving the UI crate's
/// dependency direction.
#[derive(Debug, Clone, PartialEq)]
pub struct UserSettings {
    /// The UI language tag, e.g. "ja". Empty or unsupported means "follow the
    /// system locale", which `resolve_locale` turns into a supported tag.
    pub language: String,
    pub week_start: String,
    pub vim_mode: bool,
    pub timezone: String,
    pub datetime_format: DateTimeFormatPreference,
    pub datetime_formats: Vec<DateTimeFormatPreference>,
    pub due_buttons: Vec<DueButtonPreference>,
    pub custom_due_filters: Vec<CustomDueFilter>,
    pub recurrence_presets: Vec<RecurrencePreset>,
    pub reminder_presets: Vec<ReminderPreset>,
    pub theme_mode: ThemeMode,
    pub font: String,
    pub font_size: i32,
    pub font_color: String,
    pub background_color: String,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            language: String::new(),
            week_start: "sunday".to_string(),
            vim_mode: false,
            timezone: "system".to_string(),
            datetime_format: DateTimeFormatPreference::default(),
            datetime_formats: Vec::new(),
            due_buttons: BUILTIN_DUE_FILTERS
                .iter()
                .map(|(key, query, visible)| DueButtonPreference {
                    key: (*key).to_string(),
                    query: (*query).to_string(),
                    visible: *visible,
                    name: String::new(),
                })
                .collect(),
            custom_due_filters: Vec::new(),
            recurrence_presets: Vec::new(),
            reminder_presets: default_reminder_presets(),
            theme_mode: ThemeMode::System,
            font: "system".to_string(),
            font_size: 14,
            font_color: "default".to_string(),
            background_color: "default".to_string(),
        }
    }
}

impl UserSettings {
    /// Fills missing built-in due filters while preserving stored visibility.
    pub fn normalize(mut self) -> Self {
        if self.datetime_format.kind == DateTimeFormatKind::Default
            && self.datetime_format.id.is_empty()
        {
            self.datetime_format = DateTimeFormatPreference::default();
        }
        self.due_buttons = BUILTIN_DUE_FILTERS
            .iter()
            .map(|(key, query, default_visible)| {
                self.due_buttons
                    .iter()
                    .find(|button| button.key == *key)
                    .cloned()
                    .unwrap_or_else(|| DueButtonPreference {
                        key: (*key).to_string(),
                        query: (*query).to_string(),
                        visible: *default_visible,
                        name: String::new(),
                    })
            })
            .collect();
        for button in &mut self.due_buttons {
            button.name = clean_name(&button.name);
        }
        // Stable sorts, so when two entries differ only by name the first one
        // written is the one kept.
        self.custom_due_filters.retain(CustomDueFilter::is_valid);
        self.custom_due_filters
            .sort_by_key(CustomDueFilter::sort_key);
        self.custom_due_filters.dedup_by(|a, b| a.same_filter(b));
        self.custom_due_filters.truncate(20);
        for filter in &mut self.custom_due_filters {
            filter.name = clean_name(&filter.name);
        }
        self.recurrence_presets.retain(RecurrencePreset::is_valid);
        self.recurrence_presets
            .sort_by_key(RecurrencePreset::sort_key);
        self.recurrence_presets.dedup_by(|a, b| a.same_pattern(b));
        self.recurrence_presets.truncate(20);
        for preset in &mut self.recurrence_presets {
            preset.name = clean_name(&preset.name);
        }
        self.reminder_presets.retain(ReminderPreset::is_valid);
        self.reminder_presets.sort_by_key(ReminderPreset::sort_key);
        self.reminder_presets.dedup_by(|a, b| a.same_offset(b));
        self.reminder_presets.truncate(20);
        for preset in &mut self.reminder_presets {
            preset.name = clean_name(&preset.name);
        }
        self.datetime_formats
            .retain(|format| format.kind == DateTimeFormatKind::CustomFormat);
        self.datetime_formats.sort_by_key(|format| format.order);
        self
    }
}

/// Future returned by the application-provided settings store.
pub type SettingsSaveFuture = Pin<Box<dyn Future<Output = Result<(), SettingsStoreError>> + Send>>;

/// Failure reported by the application-provided settings store.
#[derive(Debug, thiserror::Error)]
pub enum SettingsStoreError {
    #[error("settings persistence failed: {0}")]
    Persistence(String),
}

/// Persistence port implemented at the application composition root.
pub trait SettingsStore: Send + Sync {
    fn save(&self, settings: UserSettings) -> SettingsSaveFuture;
}

#[derive(Debug, Default)]
pub(super) struct NoopSettingsStore;

impl SettingsStore for NoopSettingsStore {
    fn save(&self, _settings: UserSettings) -> SettingsSaveFuture {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug)]
pub(super) struct SettingsModel {
    pub(super) current: UserSettings,
    persisted: UserSettings,
    revision: u64,
}

impl SettingsModel {
    pub(super) fn new(settings: UserSettings) -> Self {
        Self {
            current: settings.clone(),
            persisted: settings,
            revision: 0,
        }
    }

    pub(super) fn update(
        &mut self,
        mutate: impl FnOnce(&mut UserSettings),
    ) -> Option<(UserSettings, u64)> {
        let previous = self.current.clone();
        mutate(&mut self.current);
        if self.current == previous {
            return None;
        }
        self.revision += 1;
        Some((self.current.clone(), self.revision))
    }

    pub(super) fn mark_persisted(&mut self, settings: UserSettings) {
        self.persisted = settings;
    }

    pub(super) fn rollback_if_current(&mut self, revision: u64) -> Option<UserSettings> {
        if self.revision != revision {
            return None;
        }
        self.current = self.persisted.clone();
        Some(self.current.clone())
    }
}
