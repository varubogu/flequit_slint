//! File selection.
//!
//! Mobile pickers return a URI or a security-scoped handle rather than a path,
//! so the result is a [`FileHandle`] and all access goes through it. Callers
//! must not assume a [`std::path::PathBuf`] is available.

use std::path::PathBuf;

use crate::PlatformResult;

/// A file the user selected, opaque with respect to how it is addressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileHandle {
    /// A real filesystem path (desktop).
    Path(PathBuf),
    /// An opaque, platform-managed reference (Android SAF URI, iOS bookmark).
    Opaque {
        /// Platform-specific reference string.
        reference: String,
        /// Name to show in the UI.
        display_name: String,
    },
}

impl FileHandle {
    /// The name to show in the UI.
    pub fn display_name(&self) -> String {
        match self {
            Self::Path(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Self::Opaque { display_name, .. } => display_name.clone(),
        }
    }

    /// The filesystem path, when the platform exposes one.
    ///
    /// Returns `None` on platforms that hand out opaque references. Prefer
    /// reading and writing through the platform API instead of branching on this.
    pub fn as_path(&self) -> Option<&std::path::Path> {
        match self {
            Self::Path(path) => Some(path.as_path()),
            Self::Opaque { .. } => None,
        }
    }
}

/// Restricts which files the picker offers.
#[derive(Debug, Clone, Default)]
pub struct FileFilter {
    /// Human-readable name shown in the dialog, e.g. `"Flequit backup"`.
    pub name: String,
    /// Extensions without the leading dot, e.g. `["json", "automerge"]`.
    pub extensions: Vec<String>,
}

/// Reads the whole contents of a selected file.
pub trait FileReader {
    fn read(&self, handle: &FileHandle) -> PlatformResult<Vec<u8>>;
}

/// Writes the whole contents of a selected file.
pub trait FileWriter {
    fn write(&self, handle: &FileHandle, contents: &[u8]) -> PlatformResult<()>;
}
