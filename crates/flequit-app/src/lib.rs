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
use flequit_ui::{AppViewModel, AppWindow};
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
    let repositories = runtime.block_on(setup_infrastructure(platform.as_ref()))?;

    let window = AppWindow::new()?;
    let view_model = AppViewModel::new(
        Arc::new(repositories),
        Arc::clone(&platform),
        runtime.handle().clone(),
    );

    view_model.apply_capabilities(&window);
    view_model.apply_due_filters(&window);
    view_model.bind(&window);
    view_model.load_initial(&window);

    window.run()?;

    tracing::info!("shutting down");
    Ok(())
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
