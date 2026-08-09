//! Game instances: per-instance configuration, game directories and lifecycle.
//!
//! An instance is a folder under `instances/` (configurable via
//! `config.json`) holding:
//!
//! - `instance.json` — the instance config (name, version, loader, paths)
//! - `game/` — the instance's own game directory (`saves/`, `mods/`,
//!   `logs/`, ...)
//!
//! Instances are fully isolated from each other; shared artifacts (client
//! jars, libraries, assets) live in `downloads/` and are never duplicated.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::clock;
use crate::dirs::Directories;
use crate::error::{Error, Result};

/// File name of the instance config inside each instance folder.
pub const INSTANCE_CONFIG_FILE: &str = "instance.json";

/// Default game directory name inside an instance folder.
const GAME_DIR_NAME: &str = "game";

/// Maximum length of an instance name.
const MAX_NAME_LEN: usize = 64;

/// Supported mod loaders. Selecting a loader only records the choice in the
/// instance config; installing the loader itself is a later epic.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LoaderKind {
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl LoaderKind {
    /// Parse a loader kind from its CLI spelling (`fabric`, `quilt`,
    /// `forge`, `neoforge`), case-insensitively.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "fabric" => Some(Self::Fabric),
            "quilt" => Some(Self::Quilt),
            "forge" => Some(Self::Forge),
            "neoforge" => Some(Self::NeoForge),
            _ => None,
        }
    }
}

impl std::fmt::Display for LoaderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Fabric => "Fabric",
            Self::Quilt => "Quilt",
            Self::Forge => "Forge",
            Self::NeoForge => "NeoForge",
        })
    }
}

/// A mod loader selection recorded on an instance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Loader {
    pub kind: LoaderKind,
    /// Loader version string (e.g. `0.16.10`); validated against loader APIs
    /// by the loader-install epic.
    pub version: String,
}

/// Instance configuration, persisted as `instance.json` inside the instance
/// folder. Human-readable and diff-friendly for future import/export.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstanceConfig {
    /// Stable identifier, also the instance folder name.
    pub id: String,
    /// Display name chosen by the user.
    pub name: String,
    /// Minecraft version id (e.g. `1.21.4` or a snapshot id).
    pub version: String,
    /// Optional mod loader selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<Loader>,
    /// Game directory, relative to the instance folder.
    #[serde(default)]
    pub game_dir: PathBuf,
    /// RFC 3339 UTC creation timestamp.
    pub created_at: String,
    /// RFC 3339 UTC timestamp of the last launch, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_played_at: Option<String>,
}

/// A loaded instance: its config plus the absolute path of its folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub config: InstanceConfig,
    dir: PathBuf,
}

impl Instance {
    /// Absolute path of the instance folder.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Absolute path of the instance's game directory (`game/` by default).
    /// Each instance has its own; the launch engine passes this as
    /// `--gameDir`.
    #[must_use]
    pub fn game_dir(&self) -> PathBuf {
        if self.config.game_dir.is_absolute() {
            self.config.game_dir.clone()
        } else {
            self.dir.join(&self.config.game_dir)
        }
    }

    /// Absolute path of `instance.json`.
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.dir.join(INSTANCE_CONFIG_FILE)
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.config.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.config.name
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.config.version
    }

    #[must_use]
    pub fn loader(&self) -> Option<&Loader> {
        self.config.loader.as_ref()
    }
}

/// Manages the instances on disk: create, delete, clone, query and modify.
#[derive(Debug, Clone)]
pub struct InstanceManager {
    dirs: Directories,
}

impl InstanceManager {
    /// Create a manager over a launcher directory layout.
    #[must_use]
    pub fn new(dirs: Directories) -> Self {
        Self { dirs }
    }

    /// The directory layout this manager operates on.
    #[must_use]
    pub fn dirs(&self) -> &Directories {
        &self.dirs
    }

    /// Create a new instance with the given name and Minecraft version.
    ///
    /// The instance folder and its game directory are created immediately and
    /// the config is persisted.
    ///
    /// # Errors
    ///
    /// Fails if the name is invalid or taken, or if the instance folder cannot
    /// be created.
    pub fn create(&self, name: &str, version: &str) -> Result<Instance> {
        validate_name(name)?;
        if self.find_by_name(name)?.is_some() {
            return Err(Error::InstanceNameTaken(name.to_owned()));
        }
        let id = self.new_id()?;
        let dir = self.instances_dir().join(&id);
        let config = InstanceConfig {
            id,
            name: name.to_owned(),
            version: version.to_owned(),
            loader: None,
            game_dir: PathBuf::from(GAME_DIR_NAME),
            created_at: clock::now_rfc3339(),
            last_played_at: None,
        };
        fs::create_dir_all(dir.join(&config.game_dir))?;
        write_config(&dir, &config)?;
        Ok(Instance { config, dir })
    }

    /// Delete an instance (by id or name) including its game directory.
    ///
    /// # Errors
    ///
    /// Fails if the instance does not exist or cannot be removed.
    pub fn delete(&self, name_or_id: &str) -> Result<()> {
        let instance = self.resolve(name_or_id)?;
        fs::remove_dir_all(&instance.dir)?;
        Ok(())
    }

    /// Clone an instance under a new name. The game directory contents are
    /// copied, so source and clone are fully independent.
    ///
    /// # Errors
    ///
    /// Fails if the source does not exist, the new name is invalid or taken,
    /// or the copy fails.
    pub fn clone(&self, name_or_id: &str, new_name: &str) -> Result<Instance> {
        let source = self.resolve(name_or_id)?;
        validate_name(new_name)?;
        if self.find_by_name(new_name)?.is_some() {
            return Err(Error::InstanceNameTaken(new_name.to_owned()));
        }
        let id = self.new_id()?;
        let dir = self.instances_dir().join(&id);
        copy_dir_all(&source.dir, &dir)?;
        let mut config = read_config(&dir)?;
        config.id = id;
        config.name.clear();
        config.name.push_str(new_name);
        write_config(&dir, &config)?;
        Ok(Instance { config, dir })
    }

    /// List all instances, sorted by name.
    ///
    /// # Errors
    ///
    /// Fails if the instances directory cannot be read; a folder with a
    /// corrupt config is skipped.
    pub fn list(&self) -> Result<Vec<Instance>> {
        let mut instances = Vec::new();
        let dir = self.instances_dir();
        if !dir.is_dir() {
            return Ok(instances);
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            if let Ok(config) = read_config(&entry.path()) {
                instances.push(Instance {
                    config,
                    dir: entry.path(),
                });
            }
        }
        instances.sort_by(|a, b| a.config.name.cmp(&b.config.name));
        Ok(instances)
    }

    /// Look up an instance by id or name.
    ///
    /// # Errors
    ///
    /// Fails if no instance matches.
    pub fn get(&self, name_or_id: &str) -> Result<Instance> {
        self.resolve(name_or_id)
    }

    /// Change the Minecraft version of an instance.
    ///
    /// # Errors
    ///
    /// Fails if the instance does not exist or the config cannot be written.
    pub fn set_version(&self, name_or_id: &str, version: &str) -> Result<Instance> {
        let mut instance = self.resolve(name_or_id)?;
        instance.config.version.clear();
        instance.config.version.push_str(version);
        write_config(&instance.dir, &instance.config)?;
        Ok(instance)
    }

    /// Select a mod loader (kind + version) for an instance.
    ///
    /// # Errors
    ///
    /// Fails if the instance does not exist or the config cannot be written.
    pub fn set_loader(
        &self,
        name_or_id: &str,
        kind: LoaderKind,
        version: &str,
    ) -> Result<Instance> {
        let mut instance = self.resolve(name_or_id)?;
        instance.config.loader = Some(Loader {
            kind,
            version: version.to_owned(),
        });
        write_config(&instance.dir, &instance.config)?;
        Ok(instance)
    }

    /// Remove the loader selection from an instance.
    ///
    /// # Errors
    ///
    /// Fails if the instance does not exist or the config cannot be written.
    pub fn clear_loader(&self, name_or_id: &str) -> Result<Instance> {
        let mut instance = self.resolve(name_or_id)?;
        instance.config.loader = None;
        write_config(&instance.dir, &instance.config)?;
        Ok(instance)
    }

    /// Record a launch: sets `last_played_at` to now.
    ///
    /// # Errors
    ///
    /// Fails if the instance does not exist or the config cannot be written.
    pub fn touch(&self, name_or_id: &str) -> Result<Instance> {
        let mut instance = self.resolve(name_or_id)?;
        instance.config.last_played_at = Some(clock::now_rfc3339());
        write_config(&instance.dir, &instance.config)?;
        Ok(instance)
    }

    /// Export an instance to a ZIP archive: `instance.json` plus the game
    /// directory. Returns the path of the archive.
    ///
    /// Without an explicit `output`, the archive is written to
    /// `<root>/exports/<name>-<id>.zip`.
    ///
    /// # Errors
    ///
    /// Fails if the instance does not exist, the archive cannot be created, or
    /// writing fails.
    pub fn export(&self, name_or_id: &str, output: Option<&Path>) -> Result<PathBuf> {
        let instance = self.resolve(name_or_id)?;
        let output = if let Some(path) = output {
            path.to_path_buf()
        } else {
            let dir = self.dirs.exports_dir();
            fs::create_dir_all(&dir)?;
            dir.join(format!(
                "{}-{}.zip",
                instance.config.name, instance.config.id
            ))
        };
        let file = fs::File::create(&output)?;
        let mut writer = ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        add_dir_to_zip(&mut writer, &instance.dir, "", &options)?;
        writer.finish()?;
        Ok(output)
    }

    /// Import an instance from a ZIP archive produced by [`Self::export`].
    /// The archived instance id is kept unless it is already taken or does
    /// not match the expected id shape (then a fresh id is assigned); the
    /// name can be overridden with `name`, otherwise the archived name is
    /// validated and must not collide with an existing instance.
    ///
    /// # Errors
    ///
    /// Fails if the archive is missing `instance.json`, contains entries or an
    /// id that escape the instances directory, exceeds the import size limit,
    /// or the instance cannot be written.
    pub fn import(&self, archive: &Path, name: Option<&str>) -> Result<Instance> {
        if let Some(name) = name {
            validate_name(name)?;
        }
        let file = fs::File::open(archive)?;
        let mut zip = zip::ZipArchive::new(file)?;

        let mut config_bytes: Option<Vec<u8>> = None;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            let safe_name = sanitize_entry_path(entry.name())?;
            if safe_name == INSTANCE_CONFIG_FILE {
                config_bytes = Some(read_limited(&mut entry, MAX_CONFIG_BYTES)?);
            }
        }
        let bytes = config_bytes
            .ok_or_else(|| Error::ArchiveMissingConfig(INSTANCE_CONFIG_FILE.to_owned()))?;
        let mut config: InstanceConfig = serde_json::from_slice(&bytes)?;

        // The archived id is attacker-controlled (archives can come from other
        // machines): only trust ids of the exact shape we generate, and only
        // when the folder is free. Anything else gets a fresh id.
        let id = if is_valid_instance_id(&config.id)
            && !self.instances_dir().join(&config.id).exists()
        {
            config.id.clone()
        } else {
            self.new_id()?
        };
        let target_name = if let Some(name) = name {
            name.to_owned()
        } else {
            validate_name(&config.name)?;
            config.name.clone()
        };
        if self.find_by_name(&target_name)?.is_some() {
            return Err(Error::InstanceNameTaken(target_name));
        }
        let dir = self.instances_dir().join(&id);
        fs::create_dir_all(&dir)?;
        let mut total_bytes: u64 = 0;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            let safe_name = sanitize_entry_path(entry.name())?;
            if safe_name == INSTANCE_CONFIG_FILE {
                continue;
            }
            let path = dir.join(&safe_name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = fs::File::create(&path)?;
            copy_limited(&mut entry, &mut output, MAX_ENTRY_BYTES, &mut total_bytes)?;
        }
        config.id = id;
        config.name = target_name;
        write_config(&dir, &config)?;
        Ok(Instance { config, dir })
    }

    fn instances_dir(&self) -> PathBuf {
        self.dirs.instances_dir()
    }

    /// Resolve an instance by id (folder name) or by display name.
    fn resolve(&self, name_or_id: &str) -> Result<Instance> {
        let by_id = self.instances_dir().join(name_or_id);
        if by_id.join(INSTANCE_CONFIG_FILE).is_file() {
            return read_config(&by_id).map(|config| Instance { config, dir: by_id });
        }
        self.find_by_name(name_or_id)?
            .ok_or_else(|| Error::InstanceNotFound(name_or_id.to_owned()))
    }

    fn find_by_name(&self, name: &str) -> Result<Option<Instance>> {
        Ok(self.list()?.into_iter().find(|i| i.config.name == name))
    }

    /// Generate a fresh, unused instance id.
    fn new_id(&self) -> Result<String> {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        for _ in 0..100 {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| {
                    d.as_secs().saturating_mul(1_000_000_000) + u64::from(d.subsec_nanos())
                });
            let entropy =
                nanos ^ (COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed) << 48);
            let id = format!("in-{entropy:016x}");
            if !self.instances_dir().join(&id).exists() {
                return Ok(id);
            }
        }
        Err(Error::InstanceIdExhausted)
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(Error::InvalidInstanceName(
            "name must not be empty".to_owned(),
        ));
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(Error::InvalidInstanceName(format!(
            "name must be at most {MAX_NAME_LEN} characters"
        )));
    }
    let invalid = name.chars().any(|c| {
        c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    });
    if invalid || matches!(name, "." | "..") {
        return Err(Error::InvalidInstanceName(
            "name must not contain path separators or reserved characters".to_owned(),
        ));
    }
    Ok(())
}

fn read_config(dir: &Path) -> Result<InstanceConfig> {
    let bytes = fs::read(dir.join(INSTANCE_CONFIG_FILE))?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn write_config(dir: &Path, config: &InstanceConfig) -> Result<()> {
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{INSTANCE_CONFIG_FILE}.tmp{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(config)?;
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, dir.join(INSTANCE_CONFIG_FILE))?;
    Ok(())
}

/// Recursively copy a directory tree. Symlinks are copied by content (their
/// targets, directories included), so the copy works without link privileges.
fn copy_dir_all(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        // `Path::is_dir` follows symlinks; `file_type` does not.
        if file_type.is_dir() || (file_type.is_symlink() && from.is_dir()) {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Recursively add an instance folder to a ZIP archive using forward-slash
/// paths, including `instance.json`.
fn add_dir_to_zip(
    writer: &mut ZipWriter<fs::File>,
    dir: &Path,
    prefix: &str,
    options: &SimpleFileOptions,
) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let entry_name = entry.file_name().to_string_lossy().into_owned();
        let archive_path = if prefix.is_empty() {
            entry_name.clone()
        } else {
            format!("{prefix}/{entry_name}")
        };
        if file_type.is_dir() || (file_type.is_symlink() && entry.path().is_dir()) {
            writer.add_directory(archive_path.clone(), *options)?;
            add_dir_to_zip(writer, &entry.path(), &archive_path, options)?;
        } else {
            writer.start_file(archive_path, *options)?;
            let mut file = fs::File::open(entry.path())?;
            std::io::copy(&mut file, writer)?;
        }
    }
    Ok(())
}

/// Cap for `instance.json` inside an archive (a config is a few KB; anything
/// larger is hostile).
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// Cap for a single extracted archive entry (4 GiB).
const MAX_ENTRY_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Cap for the total extracted size of an archive (32 GiB) — insurance
/// against zip bombs while leaving room for heavily modded instances.
const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024 * 1024;

/// Whether an id matches the shape generated by [`InstanceManager::new_id`]
/// (`in-` followed by 16 hex digits). Ids are used as folder names, so
/// anything else (separators, `..`, absolute paths, drive letters) is
/// rejected and replaced with a fresh id.
fn is_valid_instance_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("in-") else {
        return false;
    };
    rest.len() == 16 && rest.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Read at most `limit` bytes from a reader; larger inputs error out instead
/// of exhausting memory.
fn read_limited(reader: &mut impl std::io::Read, limit: u64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    reader.take(limit + 1).read_to_end(&mut out)?;
    if out.len() as u64 > limit {
        return Err(Error::ArchiveTooLarge);
    }
    Ok(out)
}

/// Stream a reader into a writer while tracking per-entry and total byte
/// budgets, aborting past the limits instead of unboundedly expanding a
/// hostile archive.
fn copy_limited(
    reader: &mut impl std::io::Read,
    writer: &mut impl std::io::Write,
    per_entry: u64,
    total: &mut u64,
) -> Result<u64> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut written: u64 = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Ok(written);
        }
        written += n as u64;
        *total += n as u64;
        if written > per_entry || *total > MAX_TOTAL_BYTES {
            return Err(Error::ArchiveTooLarge);
        }
        writer.write_all(&buf[..n])?;
    }
}

/// Normalize an archive entry path: forward slashes, no absolute paths, no
/// `..` or `.` components, no drive letters.
fn sanitize_entry_path(entry_name: &str) -> Result<String> {
    let normalized = entry_name.replace('\\', "/");
    let path = Path::new(&normalized);
    if path.is_absolute() {
        return Err(Error::InvalidArchiveEntry(entry_name.to_owned()));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => return Err(Error::InvalidArchiveEntry(entry_name.to_owned())),
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::dirs::Directories;

    fn tempdir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mc-launcher-instances-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn manager() -> InstanceManager {
        InstanceManager::new(Directories::new(tempdir()))
    }

    #[test]
    fn create_makes_folder_config_and_game_dir() {
        let manager = manager();
        let instance = manager.create("My World", "1.21.4").expect("create");

        assert!(instance.dir().join("game").is_dir());
        assert!(instance.config_path().is_file());
        assert_eq!(instance.id().len(), "in-0000000000000000".len());
        assert_eq!(instance.name(), "My World");
        assert_eq!(instance.version(), "1.21.4");
        assert!(instance.loader().is_none());
        assert!(instance.config.last_played_at.is_none());
        assert!(!instance.config.created_at.is_empty());
        // game dir resolves inside the instance folder.
        assert_eq!(instance.game_dir(), instance.dir().join("game"));
    }

    #[test]
    fn create_rejects_invalid_names() {
        let manager = manager();
        for name in [
            "", "   ", ".", "..", "a/b", "a\\b", "a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b",
            "a\x00b",
        ] {
            let err = manager.create(name, "1.21.4").unwrap_err();
            assert!(
                matches!(err, Error::InvalidInstanceName(_)),
                "{name}: {err}"
            );
        }
    }

    #[test]
    fn create_rejects_duplicate_names() {
        let manager = manager();
        manager.create("Dup", "1.21.4").expect("create");
        let err = manager.create("Dup", "1.20.4").unwrap_err();
        assert!(matches!(err, Error::InstanceNameTaken(_)));
    }

    #[test]
    fn list_returns_sorted_instances() {
        let manager = manager();
        manager.create("beta", "1.21.4").expect("create");
        manager.create("alpha", "1.20.4").expect("create");
        let instances = manager.list().expect("list");
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].name(), "alpha");
        assert_eq!(instances[1].name(), "beta");
    }

    #[test]
    fn get_by_id_and_by_name() {
        let manager = manager();
        let created = manager.create("Get Me", "1.21.4").expect("create");
        assert_eq!(manager.get(created.id()).expect("by id").name(), "Get Me");
        assert_eq!(manager.get("Get Me").expect("by name").id(), created.id());
    }

    #[test]
    fn get_unknown_errors() {
        let err = manager().get("nope").unwrap_err();
        assert!(matches!(err, Error::InstanceNotFound(_)));
    }

    #[test]
    fn delete_removes_instance() {
        let manager = manager();
        let created = manager.create("Doomed", "1.21.4").expect("create");
        manager.delete(created.id()).expect("delete");
        assert!(!created.dir().exists());
        assert!(manager.list().expect("list").is_empty());
    }

    #[test]
    fn delete_unknown_errors() {
        let err = manager().delete("nope").unwrap_err();
        assert!(matches!(err, Error::InstanceNotFound(_)));
    }

    #[test]
    fn set_version_persists() {
        let manager = manager();
        let created = manager.create("V", "1.21.4").expect("create");
        let updated = manager.set_version(created.id(), "1.20.4").expect("set");
        assert_eq!(updated.version(), "1.20.4");
        assert_eq!(manager.get(created.id()).expect("get").version(), "1.20.4");
    }

    #[test]
    fn set_and_clear_loader_persist() {
        let manager = manager();
        let created = manager.create("L", "1.21.4").expect("create");
        let updated = manager
            .set_loader(created.id(), LoaderKind::Fabric, "0.16.10")
            .expect("set loader");
        let loader = updated.loader().expect("loader");
        assert_eq!(loader.kind, LoaderKind::Fabric);
        assert_eq!(loader.version, "0.16.10");
        assert_eq!(
            manager
                .get(created.id())
                .expect("get")
                .loader()
                .expect("loader")
                .kind,
            LoaderKind::Fabric
        );
        assert!(
            manager
                .clear_loader(created.id())
                .expect("clear")
                .loader()
                .is_none()
        );
    }

    #[test]
    fn touch_sets_last_played() {
        let manager = manager();
        let created = manager.create("T", "1.21.4").expect("create");
        let touched = manager.touch(created.id()).expect("touch");
        assert!(touched.config.last_played_at.is_some());
    }

    #[test]
    fn clone_is_independent() {
        let manager = manager();
        let source = manager.create("Source", "1.21.4").expect("create");
        fs::create_dir_all(source.game_dir().join("saves")).expect("mkdir");
        fs::write(source.game_dir().join("saves/seed.txt"), "abc").expect("write save");

        let clone = manager.clone(source.id(), "Clone").expect("clone");
        assert_eq!(clone.name(), "Clone");
        assert_ne!(clone.id(), source.id());
        assert_ne!(clone.dir(), source.dir());
        assert_ne!(clone.game_dir(), source.game_dir());
        // Content was copied.
        assert_eq!(
            fs::read_to_string(clone.game_dir().join("saves/seed.txt")).expect("read clone save"),
            "abc"
        );
        // Mutating the clone leaves the source untouched.
        fs::write(clone.game_dir().join("saves/seed.txt"), "changed").expect("write");
        assert_eq!(
            fs::read_to_string(source.game_dir().join("saves/seed.txt")).expect("read source"),
            "abc"
        );
    }

    #[test]
    fn export_import_round_trip() {
        let manager = manager();
        let source = manager.create("Roundtrip", "1.20.4").expect("create");
        manager
            .set_loader(source.id(), LoaderKind::Forge, "1.21.4-52.0.1")
            .expect("set loader");
        fs::create_dir_all(source.game_dir().join("config")).expect("mkdir");
        fs::write(
            source.game_dir().join("config/options.txt"),
            "renderDistance: 12",
        )
        .expect("write file");

        let archive = manager.export(source.id(), None).expect("export");
        assert!(archive.is_file());

        // Import with a new name into a fresh manager.
        let imported = InstanceManager::new(Directories::new(tempdir()))
            .import(&archive, Some("Roundtrip 2"))
            .expect("import");
        assert_eq!(imported.name(), "Roundtrip 2");
        assert_eq!(imported.version(), "1.20.4");
        assert_eq!(imported.loader().expect("loader").kind, LoaderKind::Forge);
        assert_eq!(
            fs::read_to_string(imported.game_dir().join("config/options.txt"))
                .expect("read imported"),
            "renderDistance: 12"
        );
    }

    #[test]
    fn export_round_trip_keeps_id_when_free() {
        let manager = manager();
        let source = manager.create("Same", "1.21.4").expect("create");
        let archive = manager.export(source.id(), None).expect("export");
        manager.delete(source.id()).expect("delete");
        // The id is free again, so a round trip preserves it.
        let imported = manager.import(&archive, None).expect("import");
        assert_eq!(imported.id(), source.id());
    }

    #[test]
    fn import_reassigns_taken_id() {
        let manager = manager();
        let source = manager.create("Same", "1.21.4").expect("create");
        let archive = manager.export(source.id(), None).expect("export");
        // Re-import without deleting the original: id is taken, so it gets a new one.
        let imported = manager.import(&archive, Some("Other")).expect("import");
        assert_ne!(imported.id(), source.id());
        assert_eq!(manager.list().expect("list").len(), 2);
    }

    #[test]
    fn import_rejects_path_traversal() {
        let dir = tempdir();
        let file = fs::File::create(dir.join("evil.zip")).expect("create archive");
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        writer
            .start_file("../evil.txt", options)
            .expect("start file");
        writer.write_all(b"boom").expect("write");
        writer.finish().expect("finish");

        let err = manager().import(&dir.join("evil.zip"), None).unwrap_err();
        assert!(matches!(err, Error::InvalidArchiveEntry(_)));
        assert!(!dir.parent().expect("parent").join("evil.txt").exists());
    }

    #[test]
    fn import_requires_config() {
        let dir = tempdir();
        let file = fs::File::create(dir.join("noconfig.zip")).expect("create archive");
        let mut writer = ZipWriter::new(file);
        writer
            .start_file("game/saves/x.txt", SimpleFileOptions::default())
            .expect("start");
        writer.write_all(b"x").expect("write");
        writer.finish().expect("finish");

        let err = manager()
            .import(&dir.join("noconfig.zip"), None)
            .unwrap_err();
        assert!(matches!(err, Error::ArchiveMissingConfig(_)));
    }

    #[test]
    fn import_reassigns_unsafe_archived_id() {
        let dir = tempdir();
        let manager = manager();
        let archive = dir.join("evil-id.zip");
        write_test_archive(
            &archive,
            &archive_config("../evil", "Safe Name"),
            &[("game/saves/x.txt", "x")],
        );
        let imported = manager.import(&archive, None).expect("import reassigns id");
        assert!(imported.dir().starts_with(manager.dirs().instances_dir()));
        assert!(
            !manager
                .dirs()
                .instances_dir()
                .parent()
                .expect("parent")
                .join("evil")
                .exists()
        );

        for bad_id in ["/tmp/pwned", "C:\\pwned", "in-zzzzzzzzzzzzzzzz"] {
            let archive = dir.join(format!("evil-{}.zip", bad_id.len()));
            write_test_archive(&archive, &archive_config(bad_id, "Other"), &[]);
            let manager = InstanceManager::new(Directories::new(tempdir()));
            let imported = manager.import(&archive, None).expect("import reassigns id");
            assert!(imported.dir().starts_with(manager.dirs().instances_dir()));
        }
    }

    #[test]
    fn import_rejects_invalid_archived_name() {
        let manager = manager();
        let dir = tempdir();
        let archive = dir.join("evil-name.zip");
        write_test_archive(
            &archive,
            &archive_config("in-1111111111111111", "../evil"),
            &[],
        );
        let err = manager.import(&archive, None).unwrap_err();
        assert!(matches!(err, Error::InvalidInstanceName(_)));
        assert!(manager.list().expect("list").is_empty());
    }

    #[test]
    fn import_rejects_duplicate_name() {
        let manager = manager();
        manager.create("Dup", "1.21.4").expect("create");
        let dir = tempdir();
        let archive = dir.join("dup.zip");
        write_test_archive(&archive, &archive_config("in-2222222222222222", "Dup"), &[]);
        let err = manager.import(&archive, None).unwrap_err();
        assert!(matches!(err, Error::InstanceNameTaken(_)));
        assert_eq!(manager.list().expect("list").len(), 1);
    }

    #[test]
    fn read_limited_caps_config_size() {
        let mut cursor = std::io::Cursor::new(vec![b'x'; 10]);
        let err = read_limited(&mut cursor, 5).unwrap_err();
        assert!(matches!(err, Error::ArchiveTooLarge));
        let mut cursor = std::io::Cursor::new(vec![b'x'; 10]);
        let bytes = read_limited(&mut cursor, 10).expect("within limit");
        assert_eq!(bytes.len(), 10);
    }

    fn write_test_archive(path: &Path, config_json: &str, entries: &[(&str, &str)]) {
        let file = fs::File::create(path).expect("create archive");
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        writer
            .start_file("instance.json", options)
            .expect("start config");
        writer
            .write_all(config_json.as_bytes())
            .expect("write config");
        for (name, content) in entries {
            writer.start_file(*name, options).expect("start entry");
            writer.write_all(content.as_bytes()).expect("write entry");
        }
        writer.finish().expect("finish");
    }

    fn archive_config(id: &str, name: &str) -> String {
        serde_json::to_string(&InstanceConfig {
            id: id.to_owned(),
            name: name.to_owned(),
            version: "1.21.4".to_owned(),
            loader: None,
            game_dir: PathBuf::from("game"),
            created_at: "2026-08-10T00:00:00Z".to_owned(),
            last_played_at: None,
        })
        .expect("serialize config")
    }

    #[test]
    fn loader_kind_parses_case_insensitively() {
        assert_eq!(LoaderKind::parse("fabric"), Some(LoaderKind::Fabric));
        assert_eq!(LoaderKind::parse("NEOFORGE"), Some(LoaderKind::NeoForge));
        assert_eq!(LoaderKind::parse("optifine"), None);
        assert_eq!(LoaderKind::parse(""), None);
    }
}
