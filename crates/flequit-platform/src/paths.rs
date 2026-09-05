//! Directory resolution.
//!
//! Every filesystem access in the application starts from one of these
//! directories. Hardcoded paths are forbidden — mobile platforms place app data
//! inside a sandbox whose location is not predictable at compile time.

use std::path::PathBuf;

/// The application directories for the current platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    data: PathBuf,
    config: PathBuf,
    cache: PathBuf,
    log: PathBuf,
}

impl AppPaths {
    /// Builds a path set explicitly. Used by platform backends and by tests.
    pub fn new(data: PathBuf, config: PathBuf, cache: PathBuf, log: PathBuf) -> Self {
        Self {
            data,
            config,
            cache,
            log,
        }
    }

    /// Derives all four directories from a single root.
    ///
    /// Used on mobile, where the OS hands the app one sandbox root, and in tests.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::PathBuf;
    /// use flequit_platform::AppPaths;
    ///
    /// let paths = AppPaths::rooted_at(PathBuf::from("/tmp/flequit"));
    /// assert_eq!(paths.cache_dir(), &PathBuf::from("/tmp/flequit/cache"));
    /// ```
    pub fn rooted_at(root: PathBuf) -> Self {
        Self {
            data: root.join("data"),
            config: root.join("config"),
            cache: root.join("cache"),
            log: root.join("logs"),
        }
    }

    /// SQLite database and Automerge documents.
    pub fn data_dir(&self) -> &PathBuf {
        &self.data
    }

    /// User settings.
    pub fn config_dir(&self) -> &PathBuf {
        &self.config
    }

    /// Regenerable temporary files.
    pub fn cache_dir(&self) -> &PathBuf {
        &self.cache
    }

    /// Rotating log files.
    pub fn log_dir(&self) -> &PathBuf {
        &self.log
    }

    /// The Automerge document store, `<data_dir>/automerge`.
    pub fn automerge_dir(&self) -> PathBuf {
        self.data.join("automerge")
    }

    /// The SQLite database file, `<data_dir>/flequit.db`.
    pub fn database_file(&self) -> PathBuf {
        self.data.join("flequit.db")
    }

    /// Creates every directory in the set if it does not already exist.
    pub fn ensure_all_exist(&self) -> crate::PlatformResult<()> {
        for dir in [&self.data, &self.config, &self.cache, &self.log] {
            std::fs::create_dir_all(dir).map_err(|source| crate::PlatformError::Io {
                path: Some(dir.clone()),
                source,
            })?;
        }
        Ok(())
    }
}
