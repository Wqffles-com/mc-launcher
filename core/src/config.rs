//! Launcher-level configuration, persisted as `config.json` in the data root.
//!
//! The config lets users relocate the big state directories (instances,
//! downloads, accounts, managed Java) off the default data root — useful for
//! putting instances on a different drive, or sharing them with a portable
//! setup. All paths are optional; any missing entry falls back to a
//! subdirectory of the data root (see [`crate::dirs::Directories`]).
//!
//! Example:
//!
//! ```json
//! {
//!   "instances_dir": "E:/Games/mc-instances",
//!   "downloads_dir": "downloads"
//! }
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// File name of the launcher config inside the data root.
pub const CONFIG_FILE: &str = "config.json";

/// Launcher configuration schema (`config.json`).
///
/// Relative paths are resolved against the data root; absolute paths are used
/// as-is.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LauncherConfig {
    /// Override for the instances directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instances_dir: Option<PathBuf>,
    /// Override for the shared downloads directory (client jars, libraries, assets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads_dir: Option<PathBuf>,
    /// Override for the accounts directory (auth token storage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accounts_dir: Option<PathBuf>,
    /// Override for the managed Java runtimes directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_dir: Option<PathBuf>,
}

impl LauncherConfig {
    /// Load the config from `root/config.json`, defaulting when the file is
    /// absent.
    ///
    /// # Errors
    ///
    /// Fails if the file exists but cannot be read or parsed.
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(CONFIG_FILE);
        if !path.is_file() {
            return Ok(Self::default());
        }
        let bytes = std::fs::read(path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Atomically write the config to `root/config.json`.
    ///
    /// # Errors
    ///
    /// Fails if the directory cannot be created or the file cannot be written.
    pub fn save(&self, root: &Path) -> Result<()> {
        std::fs::create_dir_all(root)?;
        let tmp = root.join(format!("{CONFIG_FILE}.tmp{}", std::process::id()));
        let bytes = serde_json::to_vec_pretty(self)?;
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, root.join(CONFIG_FILE))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mc-launcher-config-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn load_defaults_when_absent() {
        let cfg = LauncherConfig::load(&tempdir()).expect("load absent config");
        assert_eq!(cfg, LauncherConfig::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let root = tempdir();
        let cfg = LauncherConfig {
            instances_dir: Some(PathBuf::from("E:/Games/instances")),
            downloads_dir: Some(PathBuf::from("dl")),
            ..LauncherConfig::default()
        };
        cfg.save(&root).expect("save");

        let loaded = LauncherConfig::load(&root).expect("load");
        assert_eq!(loaded, cfg);
        let raw = std::fs::read_to_string(root.join(CONFIG_FILE)).expect("read file");
        assert!(raw.contains("\"instances_dir\": \"E:/Games/instances\""));
        // Defaults are not serialized.
        assert!(!raw.contains("accounts_dir"));
    }

    #[test]
    fn load_rejects_corrupt_config() {
        let root = tempdir();
        std::fs::write(root.join(CONFIG_FILE), "{not json").expect("write");
        assert!(LauncherConfig::load(&root).is_err());
    }
}
