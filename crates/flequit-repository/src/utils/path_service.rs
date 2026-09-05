//! Deprecated path helpers.
//!
//! Directory resolution now belongs to `flequit-platform`
//! (`flequit_platform::paths::AppPaths`), which is the only place allowed to
//! branch on the target operating system. Mobile sandboxes are not predictable
//! at compile time, so resolving paths from `dirs` inside the repository layer
//! produced wrong results on Android and iOS.
//!
//! Nothing in the workspace calls this module. It is retained only so the
//! removal can be reviewed separately; new code must take the base directory as
//! an argument, sourced from `flequit_platform::paths`.
//!
//! See `docs/ja/develop/design/platform/platform-abstraction.md`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::fs;

/// Directory overrides supplied by the caller.
///
/// `use_system_default` no longer resolves anything by itself; the caller must
/// provide `data_dir` from `flequit_platform::paths::AppPaths::data_dir()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathConfig {
    pub data_dir: Option<PathBuf>,
    pub backup_dir: Option<PathBuf>,
    pub export_dir: Option<PathBuf>,
    pub use_system_default: bool,
}

impl Default for PathConfig {
    fn default() -> Self {
        Self {
            data_dir: None,
            backup_dir: None,
            export_dir: None,
            use_system_default: true,
        }
    }
}

#[deprecated(
    since = "0.1.0",
    note = "use flequit_platform::paths::AppPaths instead; this type cannot resolve mobile sandbox paths"
)]
#[allow(dead_code)]
pub struct PathService {
    config: PathConfig,
}

#[allow(deprecated)]
impl PathService {
    /// Builds a service from an explicit configuration.
    pub fn with_config(config: PathConfig) -> Self {
        Self { config }
    }

    /// Returns the configured data directory.
    ///
    /// # Errors
    ///
    /// Returns an error when no directory was configured. There is no fallback:
    /// guessing a location is exactly the behaviour that broke on mobile.
    pub fn get_data_dir(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
        self.config.data_dir.clone().ok_or_else(|| {
            "no data directory configured; supply one from flequit_platform::paths".into()
        })
    }

    /// Returns the backup directory, defaulting to `<data_dir>/backups`.
    pub fn get_backup_dir(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
        match &self.config.backup_dir {
            Some(dir) => Ok(dir.clone()),
            None => Ok(self.get_data_dir()?.join("backups")),
        }
    }

    /// Returns the export directory, defaulting to `<data_dir>/exports`.
    pub fn get_export_dir(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
        match &self.config.export_dir {
            Some(dir) => Ok(dir.clone()),
            None => Ok(self.get_data_dir()?.join("exports")),
        }
    }

    /// Creates every configured directory.
    pub async fn ensure_directories(&self) -> Result<(), Box<dyn std::error::Error>> {
        for dir in [
            self.get_data_dir()?,
            self.get_backup_dir()?,
            self.get_export_dir()?,
        ] {
            fs::create_dir_all(&dir).await?;
        }
        Ok(())
    }
}
