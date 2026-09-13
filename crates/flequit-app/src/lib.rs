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

// Platform entry points. Selected by Cargo feature, not by `cfg(target_os)`:
// that attribute belongs to `flequit-platform` alone.
#[cfg(feature = "android")]
mod entry_android;
#[cfg(feature = "ios")]
mod entry_ios;

use flequit_infrastructure::{InfrastructureRepositories, UnifiedConfig};
use flequit_platform::Platform;
use flequit_settings::types::datetime_format_types::DateTimeFormatGroup;
use flequit_settings::{
    CustomDueFilter as StoredDueFilter, CustomDueUnit as StoredDueUnit, DateTimeFormat,
    DueDateButtons, RecurrencePreset as StoredRecurrencePreset,
    ReminderPreset as StoredReminderPreset, Settings, SettingsManager,
    SettingsRecurrenceUnit as StoredRecurrenceUnit, SettingsReminderUnit as StoredReminderUnit,
};
use flequit_ui::bindings::{RecurrenceUnit, ReminderUnit, ThemeMode};
use flequit_ui::{
    AppViewModel, AppWindow, CustomDueFilter, CustomDueUnit, DateTimeFormatKind,
    DateTimeFormatPreference, DueButtonPreference, RecurrencePreset, ReminderPreset,
    SettingsSaveFuture, SettingsStore, SettingsStoreError, UserSettings, resolve_locale,
    system_locale,
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
    // Held for the whole session: dropping it stops the file log being flushed.
    let _log_guard = init_logging(platform.as_ref());

    tracing::info!(
        data_dir = ?platform.paths().data_dir(),
        form_factor = ?platform.form_factor(),
        "starting flequit"
    );

    let runtime = build_runtime(platform.as_ref())?;
    let (user_settings, settings_store) = load_settings(&runtime, platform.as_ref())?;
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
    // Held for the whole session: the platform keeps only a weak reference, so
    // dropping this would silently stop lifecycle delivery. `None` on desktop.
    let _lifecycle = view_model.observe_lifecycle(&window);
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
    platform: &dyn Platform,
) -> Result<(UserSettings, Arc<dyn SettingsStore>), BootstrapError> {
    let manager = Arc::new(SettingsManager::new(platform.paths().config_dir().clone())?);
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
                // The file stores the id as the name until the user renames it.
                name: if button.name == button.id {
                    String::new()
                } else {
                    button.name.clone()
                },
            })
        })
        .collect();

    UserSettings {
        language: settings.language.clone(),
        week_start: settings.week_start.clone(),
        vim_mode: settings.vim_mode,
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
            .map(|filter| {
                CustomDueFilter::named(
                    filter.value,
                    to_due_unit(filter.unit),
                    filter.name.clone(),
                )
            })
            .collect(),
        recurrence_presets: settings
            .custom_recurrence_presets
            .iter()
            .map(|preset| {
                RecurrencePreset::named(
                    preset.interval,
                    to_recurrence_unit(preset.unit),
                    preset.name.clone(),
                )
            })
            .collect(),
        reminder_presets: settings
            .reminder_presets
            .iter()
            .map(|preset| {
                ReminderPreset::named(
                    preset.value,
                    match preset.unit {
                        StoredReminderUnit::Minute => ReminderUnit::Minute,
                        StoredReminderUnit::Hour => ReminderUnit::Hour,
                        StoredReminderUnit::Day => ReminderUnit::Day,
                    },
                    preset.name.clone(),
                )
            })
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
    stored.vim_mode = settings.vim_mode;
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
        .map(|filter| {
            StoredDueFilter::named(filter.value, from_due_unit(filter.unit), filter.name)
        })
        .collect();
    stored.custom_recurrence_presets = settings
        .recurrence_presets
        .into_iter()
        .map(|preset| {
            StoredRecurrencePreset::named(
                preset.interval,
                from_recurrence_unit(preset.unit),
                preset.name,
            )
        })
        .collect();
    stored.reminder_presets = settings
        .reminder_presets
        .into_iter()
        .map(|preset| {
            StoredReminderPreset::named(
                preset.value,
                match preset.unit {
                    ReminderUnit::Minute => StoredReminderUnit::Minute,
                    ReminderUnit::Hour => StoredReminderUnit::Hour,
                    ReminderUnit::Day => StoredReminderUnit::Day,
                },
                preset.name,
            )
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
            let name = if button.name.is_empty() {
                button.key.clone()
            } else {
                button.name
            };
            DueDateButtons::new(button.key, name, button.visible, index as i32)
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
    let config = UnifiedConfig::new(true, true, true).with_storage_paths(
        platform.paths().database_file(),
        platform.paths().automerge_dir(),
    );

    InfrastructureRepositories::setup_with_sqlite_and_automerge(config)
        .await
        .map_err(|source| BootstrapError::Infrastructure(source.to_string()))
}

/// How many days of log files are kept before the oldest is deleted.
///
/// Long enough to cover a weekend of an issue going unreported, short enough
/// that an app that is left running does not fill a user's disk.
const LOG_FILES_KEPT: usize = 7;

/// Installs the tracing subscriber.
///
/// Writes go to a console sink, which is what a developer reads, and to a daily
/// file under `platform.paths().log_dir()`, which is what a user can attach to
/// a bug report once the terminal is gone. A log directory that cannot be
/// opened costs the file sink only — starting without logs on screen would be
/// worse than starting without them on disk.
///
/// Which console that is depends on the platform: stderr on desktop and iOS,
/// logcat on Android, the developer console in a browser. `flequit-platform`
/// makes that choice, because deciding it here would need a `cfg(target_os)`
/// outside the one crate allowed to have them.
///
/// The returned guard flushes the file writer when it is dropped, so the caller
/// has to hold it for as long as the application runs. `None` means no file
/// sink was installed.
#[must_use = "dropping the guard stops the file log from being flushed"]
fn init_logging(platform: &dyn Platform) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{EnvFilter, Layer, Registry, fmt};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // The platform sink replaces stderr rather than joining it: on the
    // platforms that have one, stderr goes nowhere a developer can read.
    let console: Box<dyn Layer<Registry> + Send + Sync> =
        match flequit_platform::SystemLogWriter::current() {
            // A fresh writer per event, so a partial line from one event cannot
            // be interleaved into the next.
            Some(writer) => fmt::layer()
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .boxed(),
            None => fmt::layer().boxed(),
        };

    let mut layers: Vec<Box<dyn Layer<Registry> + Send + Sync>> = vec![console];
    let guard = match open_log_file(platform) {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            // No ANSI escapes: they are control characters in a file, not colour.
            layers.push(fmt::layer().with_ansi(false).with_writer(writer).boxed());
            Some(guard)
        }
        Err(error) => {
            // Reported through the sink that is about to be installed, so it
            // reaches stderr rather than disappearing.
            eprintln!("flequit: file logging is disabled: {error}");
            None
        }
    };

    if tracing_subscriber::registry()
        .with(layers)
        .with(filter)
        .try_init()
        .is_err()
    {
        // A subscriber is already installed, e.g. by a test harness that called
        // `run` twice. The guard is useless then and would flush a sink nobody
        // writes to.
        return None;
    }

    guard
}

/// Opens the rolling log file for today under the platform's log directory.
fn open_log_file(
    platform: &dyn Platform,
) -> Result<tracing_appender::rolling::RollingFileAppender, String> {
    let dir = platform.paths().log_dir();
    std::fs::create_dir_all(dir).map_err(|source| format!("{}: {source}", dir.display()))?;

    tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("flequit")
        .filename_suffix("log")
        .max_log_files(LOG_FILES_KEPT)
        .build(dir)
        .map_err(|source| format!("{}: {source}", dir.display()))
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
    fn user_given_names_round_trip_through_the_settings_file() {
        let mut stored = Settings {
            custom_due_filters: vec![StoredDueFilter::named(
                90,
                StoredDueUnit::Day,
                "3ヶ月以内".to_string(),
            )],
            custom_recurrence_presets: vec![StoredRecurrencePreset::named(
                2,
                StoredRecurrenceUnit::Week,
                "隔週".to_string(),
            )],
            reminder_presets: vec![StoredReminderPreset::named(
                1,
                StoredReminderUnit::Day,
                "前日".to_string(),
            )],
            ..Settings::default()
        };
        stored.due_date_buttons[6].name = "今期".to_string();

        let ui = to_user_settings(&stored);
        assert_eq!(ui.due_buttons[6].name, "今期");
        assert_eq!(ui.due_buttons[1].name, "", "an unrenamed button has no name");
        assert_eq!(ui.custom_due_filters[0].name, "3ヶ月以内");
        assert_eq!(ui.recurrence_presets[0].name, "隔週");
        assert_eq!(ui.reminder_presets[0].name, "前日");

        let mut restored = Settings::default();
        apply_user_settings(&mut restored, ui);
        assert_eq!(restored.due_date_buttons[6].name, "今期");
        assert_eq!(restored.due_date_buttons[1].name, "today");
        assert_eq!(restored.custom_due_filters, stored.custom_due_filters);
        assert_eq!(
            restored.custom_recurrence_presets,
            stored.custom_recurrence_presets
        );
        assert_eq!(restored.reminder_presets, stored.reminder_presets);
    }

    #[test]
    fn reminder_settings_round_trip_including_an_empty_list() {
        let stored = Settings {
            reminder_presets: vec![StoredReminderPreset::new(2, StoredReminderUnit::Day)],
            ..Settings::default()
        };
        let ui = to_user_settings(&stored);
        assert_eq!(ui.reminder_presets[0].minutes_before(), 2880);
        let mut restored = Settings::default();
        apply_user_settings(&mut restored, ui);
        assert_eq!(restored.reminder_presets, stored.reminder_presets);
        let mut ui = to_user_settings(&stored);
        ui.reminder_presets.clear();
        apply_user_settings(&mut restored, ui);
        assert!(to_user_settings(&restored).reminder_presets.is_empty());
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
