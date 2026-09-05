//! Application bootstrap, shared by every platform entry point.
//!
//! Responsibilities, in order:
//!
//! 1. Resolve the platform backend (paths, capabilities).
//! 2. Initialise logging with a platform-appropriate sink.
//! 3. Build the Tokio runtime.
//! 4. Set up infrastructure (SQLite + Automerge).
//! 5. Create the window, wire the ViewModel, run the event loop.
//!
//! Nothing below this crate constructs a runtime or reads a hardcoded path.

use std::sync::Arc;

use flequit_infrastructure::{InfrastructureRepositories, UnifiedConfig};
use flequit_platform::Platform;
use flequit_settings::types::datetime_format_types::DateTimeFormatGroup;
use flequit_settings::{
    CustomDueFilter as StoredDueFilter, CustomDueUnit as StoredDueUnit, DateTimeFormat,
    DueDateButtons, RecurrencePreset as StoredRecurrencePreset, Settings, SettingsManager,
    SettingsRecurrenceUnit as StoredRecurrenceUnit,
};
use flequit_ui::bindings::{RecurrenceUnit, ThemeMode};
use flequit_ui::{
    AppViewModel, AppWindow, CustomDueFilter, CustomDueUnit, DateTimeFormatKind,
    DateTimeFormatPreference, DueButtonPreference, RecurrencePreset, SettingsSaveFuture,
    SettingsStore, SettingsStoreError, UserSettings, resolve_locale, system_locale,
};
use slint::ComponentHandle;

/// Failures that prevent the application from starting.
#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    #[error("platform initialisation failed: {0}")]
    Platform(#[from] flequit_platform::PlatformError),

    #[error("could not build the async runtime: {0}")]
    Runtime(#[source] std::io::Error),

    #[error("infrastructure setup failed: {0}")]
    Infrastructure(String),

    #[error("settings setup failed: {0}")]
    Settings(#[from] flequit_settings::SettingsError),

    #[error("user interface failed: {0}")]
    Ui(#[from] slint::PlatformError),
}

/// Starts the application and blocks until the window closes.
///
/// The Tokio runtime is created here and kept alive for the whole session; the
/// UI event loop runs on the calling thread.
pub fn run() -> Result<(), BootstrapError> {
    let platform = flequit_platform::current()?;
    init_logging(platform.as_ref());

    tracing::info!(
        data_dir = ?platform.paths().data_dir(),
        form_factor = ?platform.form_factor(),
        "starting flequit"
    );

    // Must happen before any thread starts; see publish_storage_paths.
    publish_storage_paths(platform.as_ref())?;

    let runtime = build_runtime(platform.as_ref())?;
    let (user_settings, settings_store) = load_settings(&runtime)?;
    let repositories = runtime.block_on(setup_infrastructure(platform.as_ref()))?;

    let window = AppWindow::new()?;
    let view_model = AppViewModel::new_with_settings(
        Arc::new(repositories),
        Arc::clone(&platform),
        runtime.handle().clone(),
        user_settings,
        settings_store,
    );

    view_model.apply_capabilities(&window);
    view_model.apply_settings(&window);
    view_model.apply_project_colors(&window);
    view_model.bind(&window);
    view_model.load_initial(&window);

    window.run()?;

    tracing::info!("shutting down");
    Ok(())
}

struct AppSettingsStore {
    manager: Arc<SettingsManager>,
    current: Arc<tokio::sync::Mutex<Settings>>,
}

impl SettingsStore for AppSettingsStore {
    fn save(&self, settings: UserSettings) -> SettingsSaveFuture {
        let manager = Arc::clone(&self.manager);
        let current = Arc::clone(&self.current);
        Box::pin(async move {
            let mut stored = current.lock().await;
            let mut next = stored.clone();
            apply_user_settings(&mut next, settings);
            manager
                .save_settings(&next)
                .await
                .map_err(|error| SettingsStoreError::Persistence(error.to_string()))?;
            *stored = next;
            Ok(())
        })
    }
}

fn load_settings(
    runtime: &tokio::runtime::Runtime,
) -> Result<(UserSettings, Arc<dyn SettingsStore>), BootstrapError> {
    let manager = Arc::new(SettingsManager::new()?);
    let settings = runtime.block_on(manager.load_settings())?;
    let user_settings = to_user_settings(&settings);
    let store = AppSettingsStore {
        manager,
        current: Arc::new(tokio::sync::Mutex::new(settings)),
    };
    Ok((user_settings, Arc::new(store)))
}

fn to_user_settings(settings: &Settings) -> UserSettings {
    let due_buttons = settings
        .due_date_buttons
        .iter()
        .filter_map(|button| {
            due_query(&button.id).map(|query| DueButtonPreference {
                key: button.id.clone(),
                query: query.to_string(),
                visible: button.is_visible,
            })
        })
        .collect();

    UserSettings {
        language: settings.language.clone(),
        week_start: settings.week_start.clone(),
        timezone: settings.timezone.clone(),
        datetime_format: to_datetime_format_preference(&settings.datetime_format),
        datetime_formats: settings
            .datetime_formats
            .iter()
            .map(to_datetime_format_preference)
            .collect(),
        due_buttons,
        custom_due_filters: settings
            .custom_due_filters
            .iter()
            .map(|filter| CustomDueFilter::new(filter.value, to_due_unit(filter.unit)))
            .collect(),
        recurrence_presets: settings
            .custom_recurrence_presets
            .iter()
            .map(|preset| RecurrencePreset::new(preset.interval, to_recurrence_unit(preset.unit)))
            .collect(),
        theme_mode: match settings.theme.as_str() {
            "light" => ThemeMode::Light,
            "dark" => ThemeMode::Dark,
            _ => ThemeMode::System,
        },
        font: settings.font.clone(),
        font_size: settings.font_size,
        font_color: settings.font_color.clone(),
        background_color: settings.background_color.clone(),
    }
    .normalize()
}

fn apply_user_settings(stored: &mut Settings, settings: UserSettings) {
    // An unsupported or empty language means "follow the system locale", which
    // the settings file cannot express: it validates a two-letter code. Store
    // the tag the UI actually resolved to instead.
    stored.language = resolve_locale(&settings.language, system_locale().as_deref()).to_string();
    stored.week_start = settings.week_start;
    stored.timezone = settings.timezone;
    stored.datetime_format = from_datetime_format_preference(settings.datetime_format);
    stored.datetime_formats = settings
        .datetime_formats
        .into_iter()
        .map(from_datetime_format_preference)
        .collect();
    stored.custom_due_filters = settings
        .custom_due_filters
        .into_iter()
        .map(|filter| StoredDueFilter::new(filter.value, from_due_unit(filter.unit)))
        .collect();
    stored.custom_recurrence_presets = settings
        .recurrence_presets
        .into_iter()
        .map(|preset| {
            StoredRecurrencePreset::new(preset.interval, from_recurrence_unit(preset.unit))
        })
        .collect();
    stored.theme = match settings.theme_mode {
        ThemeMode::Light => "light",
        ThemeMode::Dark => "dark",
        ThemeMode::System => "system",
    }
    .to_string();
    stored.font = settings.font;
    stored.font_size = settings.font_size;
    stored.font_color = settings.font_color;
    stored.background_color = settings.background_color;
    stored.due_date_buttons = settings
        .due_buttons
        .into_iter()
        .enumerate()
        .map(|(index, button)| {
            DueDateButtons::new(button.key.clone(), button.key, button.visible, index as i32)
        })
        .collect();
}

fn to_datetime_format_preference(format: &DateTimeFormat) -> DateTimeFormatPreference {
    DateTimeFormatPreference {
        id: format.id.clone(),
        name: format.name.clone(),
        format: format.format.clone(),
        kind: match format.group {
            DateTimeFormatGroup::Default => DateTimeFormatKind::Default,
            DateTimeFormatGroup::Preset => DateTimeFormatKind::Preset,
            DateTimeFormatGroup::Custom => DateTimeFormatKind::Custom,
            DateTimeFormatGroup::CustomFormat => DateTimeFormatKind::CustomFormat,
        },
        order: format.order,
    }
}

fn from_datetime_format_preference(format: DateTimeFormatPreference) -> DateTimeFormat {
    DateTimeFormat {
        id: format.id,
        name: format.name,
        format: format.format,
        group: match format.kind {
            DateTimeFormatKind::Default => DateTimeFormatGroup::Default,
            DateTimeFormatKind::Preset => DateTimeFormatGroup::Preset,
            DateTimeFormatKind::Custom => DateTimeFormatGroup::Custom,
            DateTimeFormatKind::CustomFormat => DateTimeFormatGroup::CustomFormat,
        },
        order: format.order,
    }
}

fn to_due_unit(unit: StoredDueUnit) -> CustomDueUnit {
    match unit {
        StoredDueUnit::Minute => CustomDueUnit::Minute,
        StoredDueUnit::Hour => CustomDueUnit::Hour,
        StoredDueUnit::Day => CustomDueUnit::Day,
    }
}

fn from_due_unit(unit: CustomDueUnit) -> StoredDueUnit {
    match unit {
        CustomDueUnit::Minute => StoredDueUnit::Minute,
        CustomDueUnit::Hour => StoredDueUnit::Hour,
        CustomDueUnit::Day => StoredDueUnit::Day,
    }
}

fn to_recurrence_unit(unit: StoredRecurrenceUnit) -> RecurrenceUnit {
    match unit {
        StoredRecurrenceUnit::Minute => RecurrenceUnit::Minute,
        StoredRecurrenceUnit::Hour => RecurrenceUnit::Hour,
        StoredRecurrenceUnit::Day => RecurrenceUnit::Day,
        StoredRecurrenceUnit::Week => RecurrenceUnit::Week,
        StoredRecurrenceUnit::Month => RecurrenceUnit::Month,
        StoredRecurrenceUnit::Quarter => RecurrenceUnit::Quarter,
        StoredRecurrenceUnit::HalfYear => RecurrenceUnit::HalfYear,
        StoredRecurrenceUnit::Year => RecurrenceUnit::Year,
    }
}

fn from_recurrence_unit(unit: RecurrenceUnit) -> StoredRecurrenceUnit {
    match unit {
        RecurrenceUnit::Minute => StoredRecurrenceUnit::Minute,
        RecurrenceUnit::Hour => StoredRecurrenceUnit::Hour,
        RecurrenceUnit::Day => StoredRecurrenceUnit::Day,
        RecurrenceUnit::Week => StoredRecurrenceUnit::Week,
        RecurrenceUnit::Month => StoredRecurrenceUnit::Month,
        RecurrenceUnit::Quarter => StoredRecurrenceUnit::Quarter,
        RecurrenceUnit::HalfYear => StoredRecurrenceUnit::HalfYear,
        RecurrenceUnit::Year => StoredRecurrenceUnit::Year,
    }
}

fn due_query(key: &str) -> Option<&'static str> {
    match key {
        "overdue" => Some("@overdue"),
        "today" => Some("@today"),
        "tomorrow" => Some("@tomorrow"),
        "three-days" => Some("@3days"),
        "this-week" => Some("@week"),
        "this-month" => Some("@month"),
        "this-quarter" => Some("@quarter"),
        "this-year" => Some("@year"),
        "this-fiscal-year" => Some("@fiscalyear"),
        _ => None,
    }
}

/// Points the storage layers at the platform-resolved directories.
///
/// `flequit-infrastructure-sqlite` and `flequit-infrastructure-automerge` pick
/// their own locations from `dirs`, which is wrong on Android and iOS where the
/// app is confined to a sandbox the OS assigns at runtime. Both honour an
/// environment variable override, so this is where the platform's answer wins.
///
/// # Errors
///
/// Returns an error if a resolved path is not valid UTF-8, since the storage
/// layers take the override as a string.
///
/// TODO: replace with explicit configuration once `UnifiedConfig` accepts
/// paths. The environment is a process-wide global and this indirection only
/// exists because the storage crates resolve paths themselves.
fn publish_storage_paths(platform: &dyn Platform) -> Result<(), BootstrapError> {
    let paths = platform.paths();

    let database = paths.database_file();
    let automerge = paths.automerge_dir();

    let database = database.to_str().ok_or_else(|| {
        BootstrapError::Infrastructure(format!("database path is not valid UTF-8: {database:?}"))
    })?;
    let automerge = automerge.to_str().ok_or_else(|| {
        BootstrapError::Infrastructure(format!("automerge path is not valid UTF-8: {automerge:?}"))
    })?;

    // SAFETY: called before the Tokio runtime and the UI event loop start, so
    // this process is still single-threaded and no other thread can be reading
    // the environment concurrently.
    unsafe {
        std::env::set_var("FLEQUIT_DB_PATH", database);
        std::env::set_var("FLEQUIT_AUTOMERGE_PATH", automerge);
    }

    Ok(())
}

/// Builds the async runtime, sized for the device class.
///
/// Mobile devices get fewer workers: threads cost memory and battery, and the
/// workload is a handful of concurrent SQLite and file operations.
fn build_runtime(platform: &dyn Platform) -> Result<tokio::runtime::Runtime, BootstrapError> {
    use flequit_platform::FormFactor;

    let worker_threads = match platform.form_factor() {
        FormFactor::Desktop => 4,
        FormFactor::Tablet | FormFactor::Phone => 2,
    };

    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()
        .map_err(BootstrapError::Runtime)
}

/// Creates the storage directories and opens the repositories.
///
/// Uses `setup_with_sqlite_and_automerge` rather than `InfrastructureRepositories::new()`:
/// the latter builds placeholder repositories intended for tests and would
/// silently write to a temporary directory.
async fn setup_infrastructure(
    platform: &dyn Platform,
) -> Result<InfrastructureRepositories, BootstrapError> {
    platform.paths().ensure_all_exist()?;
    std::fs::create_dir_all(platform.paths().automerge_dir()).map_err(|source| {
        BootstrapError::Infrastructure(format!(
            "could not create the automerge directory: {source}"
        ))
    })?;

    // Local-first: SQLite answers queries, Automerge records history for sync.
    let config = UnifiedConfig::new(true, true, true);

    InfrastructureRepositories::setup_with_sqlite_and_automerge(config)
        .await
        .map_err(|source| BootstrapError::Infrastructure(source.to_string()))
}

/// Installs the tracing subscriber.
///
/// The sink differs per platform (files on desktop, logcat on Android, OSLog on
/// iOS); the decision belongs here so no other crate needs a `cfg`.
fn init_logging(_platform: &dyn Platform) {
    use tracing_subscriber::{EnvFilter, fmt};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // TODO(phase2): add a rolling file appender under `platform.paths().log_dir()`
    // and route Android/iOS to their native sinks.
    let _ = fmt().with_env_filter(filter).try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_adapter_uses_persisted_due_visibility() {
        let mut stored = Settings::default();
        stored.due_date_buttons[1].is_visible = false;

        let settings = to_user_settings(&stored);

        assert_eq!(settings.due_buttons.len(), 9);
        assert!(!settings.due_buttons[1].visible);
    }

    #[test]
    fn applying_ui_settings_preserves_unowned_fields() {
        let mut stored = Settings {
            language: "en".to_string(),
            timezone: "UTC".to_string(),
            ..Settings::default()
        };
        let settings = UserSettings {
            week_start: "monday".to_string(),
            font_size: 18,
            ..to_user_settings(&stored)
        };

        apply_user_settings(&mut stored, settings);

        assert_eq!(stored.week_start, "monday");
        assert_eq!(stored.font_size, 18);
        assert_eq!(stored.language, "en");
        assert_eq!(stored.timezone, "UTC");
    }

    #[test]
    fn datetime_settings_round_trip_through_the_ui_boundary() {
        let mut stored = Settings::default();
        stored.timezone = "America/New_York".to_string();
        stored.datetime_format = DateTimeFormat {
            id: "custom-id".to_string(),
            name: "Work".to_string(),
            format: "%Y/%m/%d %H:%M".to_string(),
            group: DateTimeFormatGroup::CustomFormat,
            order: 100,
        };
        stored.datetime_formats = vec![stored.datetime_format.clone()];

        let ui = to_user_settings(&stored);
        let mut restored = Settings::default();
        apply_user_settings(&mut restored, ui);

        assert_eq!(restored.timezone, "America/New_York");
        assert_eq!(restored.datetime_format.id, stored.datetime_format.id);
        assert_eq!(restored.datetime_format.name, stored.datetime_format.name);
        assert_eq!(
            restored.datetime_format.format,
            stored.datetime_format.format
        );
        assert_eq!(restored.datetime_formats.len(), 1);
        assert_eq!(restored.datetime_formats[0].id, "custom-id");
    }
}
