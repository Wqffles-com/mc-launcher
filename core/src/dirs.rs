//! Launcher directory layout.
//!
//! Global state lives under the platform's user data directory:
//!
//! - Windows: `%APPDATA%\mc-launcher` (e.g. `C:\Users\<user>\AppData\Roaming\mc-launcher`)
//! - Linux: `$XDG_DATA_HOME/mc-launcher` (default `~/.local/share/mc-launcher`)
//! - macOS: `~/Library/Application Support/mc-launcher`
//!
//! Contents:
//!
//! - `cache/` — version manifest and version JSON files
//! - `downloads/` — client jars, libraries, assets (added later)
//! - `java/` — managed Java runtimes (added later)
//! - `accounts/` — Microsoft auth tokens (added later)
//! - `instances/` — game instances (added later)

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Resolved launcher data directories.
#[derive(Debug, Clone)]
pub struct Directories {
    root: PathBuf,
}

impl Directories {
    /// Create a layout rooted at `root` (for tests and custom setups).
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Default layout: the platform user data directory (`dirs::data_dir()`).
    ///
    /// Windows `%APPDATA%\mc-launcher`, Linux `~/.local/share/mc-launcher`,
    /// macOS `~/Library/Application Support/mc-launcher`.
    ///
    /// # Errors
    ///
    /// Fails if no platform user data directory can be resolved.
    pub fn discover() -> Result<Self> {
        let base = dirs::data_dir().ok_or(Error::NoDataDir)?;
        Ok(Self::new(base.join("mc-launcher")))
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    /// Path of the cached Mojang version manifest.
    #[must_use]
    pub fn manifest_cache_path(&self) -> PathBuf {
        self.cache_dir().join("version_manifest_v2.json")
    }

    /// Create all directories in the layout.
    ///
    /// # Errors
    ///
    /// Fails if a directory cannot be created.
    pub fn ensure_all(&self) -> Result<()> {
        std::fs::create_dir_all(self.cache_dir())?;
        Ok(())
    }
}
