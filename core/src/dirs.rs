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
//! - `config.json` — launcher config (directory overrides, see [`crate::config`])
//! - `cache/` — version manifest and version JSON files
//! - `downloads/` — client jars, libraries, assets (shared across instances)
//! - `java/` — managed Java runtimes
//! - `accounts/` — Microsoft auth tokens
//! - `instances/` — game instances, one folder per instance
//! - `exports/` — exported instance archives
//!
//! Any directory except `cache/` and `exports/` can be relocated through
//! `config.json`.

use std::path::{Path, PathBuf};

use crate::config::LauncherConfig;
use crate::error::{Error, Result};

/// Resolved launcher data directories.
#[derive(Debug, Clone)]
pub struct Directories {
    root: PathBuf,
    config: LauncherConfig,
}

impl Directories {
    /// Create a layout rooted at `root` (for tests and custom setups).
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            config: LauncherConfig::default(),
        }
    }

    /// Default layout: the platform user data directory (`dirs::data_dir()`),
    /// with any overrides from `config.json` applied.
    ///
    /// Windows `%APPDATA%\mc-launcher`, Linux `~/.local/share/mc-launcher`,
    /// macOS `~/Library/Application Support/mc-launcher`.
    ///
    /// # Errors
    ///
    /// Fails if no platform user data directory can be resolved, or if the
    /// launcher config exists but is invalid.
    pub fn discover() -> Result<Self> {
        let base = dirs::data_dir().ok_or(Error::NoDataDir)?;
        let mut dirs = Self::new(base.join("mc-launcher"));
        dirs.load_config()?;
        Ok(dirs)
    }

    /// (Re)load the launcher config from disk, replacing any in-memory copy.
    ///
    /// # Errors
    ///
    /// Fails if `config.json` exists but cannot be read or parsed.
    pub fn load_config(&mut self) -> Result<()> {
        self.config = LauncherConfig::load(&self.root)?;
        Ok(())
    }

    /// Write the current launcher config to `config.json`.
    ///
    /// # Errors
    ///
    /// Fails if the config cannot be written.
    pub fn save_config(&self) -> Result<()> {
        self.config.save(&self.root)
    }

    /// The current launcher config (after [`Self::discover`] or
    /// [`Self::load_config`], otherwise the defaults).
    #[must_use]
    pub fn config(&self) -> &LauncherConfig {
        &self.config
    }

    /// Replace the launcher config in memory (persist with [`Self::save_config`]).
    pub fn set_config(&mut self, config: LauncherConfig) {
        self.config = config;
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    /// Shared downloads: client jars, libraries and assets. Configurable via
    /// `downloads_dir` in `config.json`.
    #[must_use]
    pub fn downloads_dir(&self) -> PathBuf {
        self.resolve_override(self.config.downloads_dir.as_ref(), "downloads")
    }

    /// Managed Java runtimes. Configurable via `java_dir` in `config.json`.
    #[must_use]
    pub fn java_dir(&self) -> PathBuf {
        self.resolve_override(self.config.java_dir.as_ref(), "java")
    }

    /// Microsoft account token storage. Configurable via `accounts_dir` in
    /// `config.json`.
    #[must_use]
    pub fn accounts_dir(&self) -> PathBuf {
        self.resolve_override(self.config.accounts_dir.as_ref(), "accounts")
    }

    /// Game instances, one folder per instance. Configurable via
    /// `instances_dir` in `config.json`.
    #[must_use]
    pub fn instances_dir(&self) -> PathBuf {
        self.resolve_override(self.config.instances_dir.as_ref(), "instances")
    }

    /// Exported instance archives (created by `mc-launcher instance export`).
    #[must_use]
    pub fn exports_dir(&self) -> PathBuf {
        self.root.join("exports")
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
        std::fs::create_dir_all(self.downloads_dir())?;
        std::fs::create_dir_all(self.java_dir())?;
        std::fs::create_dir_all(self.accounts_dir())?;
        std::fs::create_dir_all(self.instances_dir())?;
        std::fs::create_dir_all(self.exports_dir())?;
        Ok(())
    }

    /// Resolve a config override, falling back to `root/<sub>`.
    /// Absolute overrides are used as-is; relative ones resolve against the root.
    fn resolve_override(&self, configured: Option<&PathBuf>, sub: &str) -> PathBuf {
        match configured {
            Some(path) if path.is_absolute() => path.clone(),
            Some(path) => self.root.join(path),
            None => self.root.join(sub),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mc-launcher-dirs-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn defaults_are_root_subdirectories() {
        let root = tempdir();
        let dirs = Directories::new(&root);
        assert_eq!(dirs.instances_dir(), root.join("instances"));
        assert_eq!(dirs.downloads_dir(), root.join("downloads"));
        assert_eq!(dirs.cache_dir(), root.join("cache"));
        assert_eq!(
            dirs.manifest_cache_path(),
            root.join("cache/version_manifest_v2.json")
        );
    }

    #[test]
    fn config_overrides_are_applied() {
        let root = tempdir();
        let absolute = tempdir().join("elsewhere");
        LauncherConfig {
            instances_dir: Some(absolute.clone()),
            downloads_dir: Some(PathBuf::from("dl")),
            ..LauncherConfig::default()
        }
        .save(&root)
        .expect("save config");

        let mut dirs = Directories::new(&root);
        dirs.load_config().expect("load config");
        assert_eq!(dirs.instances_dir(), absolute);
        assert_eq!(dirs.downloads_dir(), root.join("dl"));
        assert_eq!(dirs.accounts_dir(), root.join("accounts"));
    }

    #[test]
    fn ensure_all_creates_every_directory() {
        let root = tempdir();
        Directories::new(&root).ensure_all().expect("ensure all");
        for sub in [
            "cache",
            "downloads",
            "java",
            "accounts",
            "instances",
            "exports",
        ] {
            assert!(root.join(sub).is_dir(), "missing {sub}");
        }
    }
}
