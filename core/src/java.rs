//! Java runtime management: system JVM detection, runtime downloads (Mojang
//! per-file manifests, Adoptium archives) and per-version selection.
//!
//! Managed runtimes are cached under `java/<major>/` (see
//! [`crate::dirs::Directories::java_dir`]) and verified by checksum before
//! use, so `launch` never needs the user to install Java by hand.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::assets::ProgressFn;
use crate::dirs::Directories;
use crate::download;
use crate::error::{Error, Result};
use crate::version_json::{ArtifactDownload, JavaVersion};

/// Mojang's Java runtime product manifest. The hash is pinned per launcher
/// release; it only changes when Mojang ships new runtimes. Override for
/// mirrors/testing via `MC_LAUNCHER_JAVA_MANIFEST_URL`.
pub const MOJANG_JAVA_MANIFEST_URL: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

/// Adoptium assets API base. Override for mirrors/testing via
/// `MC_LAUNCHER_ADOPTIUM_URL`.
pub const ADOPTIUM_ASSETS_URL: &str = "https://api.adoptium.net/v3/assets/latest";
const DOWNLOAD_CONCURRENCY: usize = 16;

/// Where a runtime was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSource {
    /// Found on the system (`JAVA_HOME`, PATH, well-known locations).
    System,
    /// Downloaded by mc-launcher and cached under the java dir.
    Managed,
    /// Explicitly configured by the user (`--java`).
    Configured,
}

/// A usable Java runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaRuntime {
    /// Runtime home directory (contains `bin/java`).
    pub home: PathBuf,
    /// Detected major version (e.g. 8, 17, 21).
    pub major: u32,
    pub source: RuntimeSource,
}

impl JavaRuntime {
    /// The `java` executable inside this runtime.
    #[must_use]
    pub fn java_executable(&self) -> PathBuf {
        self.home.join("bin").join(java_executable_name())
    }
}

// --- Detection (TASK-28y9j) -------------------------------------------------

/// Scan for system JVMs: `JAVA_HOME`, `java` on PATH, and per-OS well-known
/// installation roots. Returns runtimes sorted by major (descending),
/// deduplicated by canonical path.
#[must_use]
pub fn detect_system() -> Vec<JavaRuntime> {
    collect_runtimes(system_java_homes())
}

/// Probe, deduplicate and sort a list of candidate homes.
fn collect_runtimes(homes: Vec<PathBuf>) -> Vec<JavaRuntime> {
    let mut seen = std::collections::HashSet::new();
    let mut runtimes = Vec::new();
    for home in homes {
        let Some(major) = runtime_major(&home) else {
            continue;
        };
        let key = std::fs::canonicalize(&home).unwrap_or_else(|_| home.clone());
        if !seen.insert(key) {
            continue;
        }
        runtimes.push(JavaRuntime {
            home,
            major,
            source: RuntimeSource::System,
        });
    }
    sort_runtimes(&mut runtimes);
    runtimes
}

/// Managed runtimes currently cached under `java/<major>/`.
#[must_use]
pub fn managed_runtimes(dirs: &Directories) -> Vec<JavaRuntime> {
    let Ok(entries) = std::fs::read_dir(dirs.java_dir()) else {
        return Vec::new();
    };
    let mut runtimes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let java = path.join("bin").join(java_executable_name());
        if java.is_file()
            && let Some(major) = runtime_major(&path)
        {
            runtimes.push(JavaRuntime {
                home: path,
                major,
                source: RuntimeSource::Managed,
            });
        }
    }
    sort_runtimes(&mut runtimes);
    runtimes
}

fn sort_runtimes(runtimes: &mut [JavaRuntime]) {
    runtimes.sort_by(|a, b| b.major.cmp(&a.major).then_with(|| a.home.cmp(&b.home)));
}

/// Candidate runtime homes: `JAVA_HOME`, PATH entries with a java executable,
/// and per-OS well-known roots.
fn system_java_homes() -> Vec<PathBuf> {
    let mut homes = Vec::new();
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        homes.push(PathBuf::from(home));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.join(java_executable_name()).is_file() {
                homes.push(dir);
            }
        }
    }
    for root in well_known_roots() {
        homes.extend(scan_root(&root));
    }
    homes
}

/// Per-OS directories known to contain JVM installs.
fn well_known_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "windows")]
    {
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            for sub in [
                "Java",
                "Eclipse Adoptium",
                "Zulu",
                "Amazon Corretto",
                "Microsoft",
            ] {
                roots.push(PathBuf::from(&pf).join(sub));
            }
        }
        if let Some(pf86) = std::env::var_os("ProgramFiles(x86)") {
            roots.push(PathBuf::from(&pf86).join("Java"));
        }
        if let Some(user) = std::env::var_os("USERPROFILE") {
            roots.push(PathBuf::from(&user).join(".jdks"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        roots.push(PathBuf::from("/Library/Java/JavaVirtualMachines"));
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(&home).join("Library/Java/JavaVirtualMachines"));
        }
        roots.push(PathBuf::from("/opt/homebrew/opt"));
        roots.push(PathBuf::from("/usr/local/opt"));
    }
    #[cfg(target_os = "linux")]
    {
        roots.push(PathBuf::from("/usr/lib/jvm"));
        roots.push(PathBuf::from("/usr/lib64/jvm"));
        roots.push(PathBuf::from("/usr/java"));
        roots.push(PathBuf::from("/opt/java"));
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(&home).join(".sdkman/candidates/java"));
        }
    }
    roots
}

/// Collect immediate children of `root` that look like runtime homes: either
/// containing `bin/java` directly, or a macOS-style bundle with the home under
/// `Contents/Home`.
fn scan_root(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut homes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join("bin").join(java_executable_name()).is_file() {
            homes.push(path);
            continue;
        }
        let bundled = path
            .join("Contents")
            .join("Home")
            .join("bin")
            .join(java_executable_name());
        if bundled.is_file() {
            homes.push(path.join("Contents").join("Home"));
        }
    }
    homes
}

/// Determine the major version of a runtime home: prefer the `release` file,
/// fall back to probing `java -version`.
#[must_use]
pub fn runtime_major(home: &Path) -> Option<u32> {
    read_release_major(home)
        .or_else(|| probe_executable_major(&home.join("bin").join(java_executable_name())))
}

/// Parse `JAVA_VERSION=` from a JVM `release` file.
#[must_use]
pub fn read_release_major(home: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(home.join("release")).ok()?;
    let line = text.lines().find(|l| l.starts_with("JAVA_VERSION="))?;
    let value = line.split_once('=')?.1.trim().trim_matches('"');
    major_from_version_string(value)
}

/// Extract the familiar major version from a Java version string:
/// `21.0.7` → 21, `1.8.0_442` → 8, `8u202` → 8, `16.0.1.9.1` → 16.
#[must_use]
pub fn major_from_version_string(version: &str) -> Option<u32> {
    let version = version.trim();
    if let Some(rest) = version.strip_prefix("1.") {
        rest.split(['.', '_', '-', '+']).next()?.parse().ok()
    } else {
        version
            .split(['.', '_', '-', '+', 'u'])
            .next()?
            .parse()
            .ok()
    }
}

/// Run `<java> -version` and parse the major from its stderr.
fn probe_executable_major(java: &Path) -> Option<u32> {
    if !java.is_file() {
        return None;
    }
    let output = std::process::Command::new(java)
        .arg("-version")
        .output()
        .ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stderr.lines().find(|l| l.contains("version"))?;
    let value = line
        .split('"')
        .nth(1)
        .or_else(|| line.split("version ").nth(1)?.split_whitespace().next())?;
    major_from_version_string(value)
}

fn java_executable_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "java.exe"
    } else {
        "java"
    }
}

// --- Download (TASK-4pd8f) --------------------------------------------------

/// The Mojang component name for a major version, when one exists today.
#[must_use]
pub fn component_for_major(major: u32) -> Option<&'static str> {
    match major {
        8 => Some("jre-legacy"),
        16 => Some("java-runtime-alpha"),
        17 => Some("java-runtime-gamma"),
        21 => Some("java-runtime-delta"),
        25 => Some("java-runtime-epsilon"),
        _ => None,
    }
}

/// Directory where a managed runtime for `major` is cached.
#[must_use]
pub fn managed_dir(dirs: &Directories, major: u32) -> PathBuf {
    dirs.java_dir().join(major.to_string())
}

/// The cached runtime for `major`, when `java/<major>/` holds a complete,
/// verifiable JVM.
#[must_use]
pub fn managed_runtime(dirs: &Directories, major: u32) -> Option<JavaRuntime> {
    let home = managed_dir(dirs, major);
    if !home.join("bin").join(java_executable_name()).is_file() {
        return None;
    }
    let major = runtime_major(&home)?;
    Some(JavaRuntime {
        home,
        major,
        source: RuntimeSource::Managed,
    })
}

/// Download (if needed) a managed Java runtime for `major` and return the
/// cached runtime. Tries the Mojang runtime manifest for `component` first
/// (when it actually matches `major`), then Adoptium archives.
///
/// # Errors
///
/// Fails when the runtime cannot be downloaded or verified by any source.
pub async fn ensure_runtime(
    dirs: &Directories,
    client: &reqwest::Client,
    major: u32,
    component: Option<&str>,
    progress: Option<ProgressFn>,
) -> Result<JavaRuntime> {
    let mojang_url = std::env::var("MC_LAUNCHER_JAVA_MANIFEST_URL")
        .unwrap_or_else(|_| MOJANG_JAVA_MANIFEST_URL.to_owned());
    let adoptium_url = std::env::var("MC_LAUNCHER_ADOPTIUM_URL")
        .unwrap_or_else(|_| ADOPTIUM_ASSETS_URL.to_owned());
    ensure_runtime_from(
        dirs,
        client,
        major,
        component,
        progress,
        &mojang_url,
        &adoptium_url,
    )
    .await
}

/// Like [`ensure_runtime`], but with explicit source URLs (used by tests and
/// mirrors).
#[doc(hidden)]
pub async fn ensure_runtime_from(
    dirs: &Directories,
    client: &reqwest::Client,
    major: u32,
    component: Option<&str>,
    progress: Option<ProgressFn>,
    mojang_manifest_url: &str,
    adoptium_assets_url: &str,
) -> Result<JavaRuntime> {
    if let Some(runtime) = managed_runtime(dirs, major) {
        return Ok(runtime);
    }
    let mut mojang_error: Option<String> = None;
    if let Some(component) = component {
        match download_mojang_runtime(
            dirs,
            client,
            major,
            component,
            progress.clone(),
            mojang_manifest_url,
        )
        .await
        {
            Ok(()) => return Ok(managed_runtime(dirs, major).expect("runtime was installed")),
            Err(e) => mojang_error = Some(e.to_string()),
        }
    }
    match download_adoptium_runtime(dirs, client, major, progress, adoptium_assets_url).await {
        Ok(()) => Ok(managed_runtime(dirs, major).expect("runtime was installed")),
        Err(e) => Err(Error::JavaRuntime(format!(
            "failed to download a Java {major} runtime{}",
            mojang_error.map_or_else(String::new, |mojang| {
                format!(" (Mojang: {mojang}; Adoptium: {e})")
            })
        ))),
    }
}

/// Download a runtime from Mojang's per-file manifests into `java/<major>/`.
/// The component is only used when its manifest entry's version matches
/// `major` (e.g. `java-runtime-alpha` is Java 16 today, so a Java 8 request
/// falls through to the Adoptium fallback).
async fn download_mojang_runtime(
    dirs: &Directories,
    client: &reqwest::Client,
    major: u32,
    component: &str,
    progress: Option<ProgressFn>,
    manifest_url: &str,
) -> Result<()> {
    let manifest: MojangJavaManifest = fetch_json(client, manifest_url).await?;
    let os_key = mojang_os_key();
    let entry = manifest.by_os_component(os_key, component).ok_or_else(|| {
        Error::JavaRuntime(format!("Mojang has no '{component}' runtime for {os_key}"))
    })?;
    let entry_major = major_from_version_string(&entry.version.name).ok_or_else(|| {
        Error::JavaRuntime(format!(
            "could not parse Mojang runtime version '{}'",
            entry.version.name
        ))
    })?;
    if entry_major != major {
        return Err(Error::JavaRuntime(format!(
            "Mojang '{component}' is Java {entry_major}, not {major}"
        )));
    }
    let component_manifest: MojangComponentManifest =
        fetch_json(client, &entry.manifest.url).await?;

    let staging = staging_dir(dirs, major);
    std::fs::create_dir_all(&staging)?;
    match download_mojang_files(client, &component_manifest, &staging, progress).await {
        Ok(()) => {
            let dest = managed_dir(dirs, major);
            std::fs::remove_dir_all(&dest).ok();
            std::fs::rename(&staging, &dest)?;
            Ok(())
        }
        Err(e) => {
            std::fs::remove_dir_all(&staging).ok();
            Err(e)
        }
    }
}

/// A unique staging directory under `java/` for an in-progress install.
fn staging_dir(dirs: &Directories, major: u32) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    dirs.java_dir().join(format!(
        ".tmp-{major}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}

/// Download every `raw` file of a component manifest, 16-way concurrent, with
/// SHA-1 + size verification and executable bits applied.
async fn download_mojang_files(
    client: &reqwest::Client,
    manifest: &MojangComponentManifest,
    staging: &Path,
    progress: Option<ProgressFn>,
) -> Result<()> {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(DOWNLOAD_CONCURRENCY));
    let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut tasks = tokio::task::JoinSet::new();
    let total = manifest
        .files
        .values()
        .filter(|entry| entry.r#type != "directory")
        .count();
    for (name, entry) in &manifest.files {
        if entry.r#type == "directory" {
            continue;
        }
        let raw = entry
            .downloads
            .as_ref()
            .and_then(|d| d.raw.as_ref())
            .ok_or_else(|| Error::JavaRuntime(format!("no raw download for '{name}'")))?
            .clone();
        let dest = staging.join(name);
        let executable = entry.executable;
        tasks.spawn(download_java_file(
            client.clone(),
            raw,
            dest,
            executable,
            Arc::clone(&semaphore),
            Arc::clone(&done),
            total,
            progress.clone(),
        ));
    }
    while let Some(joined) = tasks.join_next().await {
        joined.map_err(|e| Error::Task(e.to_string()))??;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn download_java_file(
    client: reqwest::Client,
    file: ArtifactDownload,
    dest: PathBuf,
    executable: bool,
    semaphore: Arc<tokio::sync::Semaphore>,
    done: Arc<std::sync::atomic::AtomicUsize>,
    total: usize,
    progress: Option<ProgressFn>,
) -> Result<()> {
    let _permit = semaphore
        .acquire_owned()
        .await
        .map_err(|e| Error::Task(e.to_string()))?;
    let name = dest
        .file_name()
        .map_or_else(|| file.url.clone(), |n| n.to_string_lossy().into_owned());
    download::fetch(
        &client,
        &file.url,
        &dest,
        Some((&file.sha1, file.size)),
        None,
    )
    .await
    .map_err(|e| {
        Error::JavaRuntime(format!(
            "failed to download '{name}' to '{}': {e}",
            dest.display()
        ))
    })?;
    mark_executable(&dest, executable);
    let count = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if let Some(cb) = progress {
        cb(download::Progress::BatchDone {
            name,
            done: count,
            total,
        });
    }
    Ok(())
}

/// Download a runtime from Adoptium: one archive (zip or tar.gz) verified by
/// SHA-256, extracted, and renamed into `java/<major>/`.
async fn download_adoptium_runtime(
    dirs: &Directories,
    client: &reqwest::Client,
    major: u32,
    progress: Option<ProgressFn>,
    assets_url: &str,
) -> Result<()> {
    let (os, arch) = adoptium_os_arch();
    let assets_url = assets_url.trim_end_matches('/');
    let url = format!(
        "{assets_url}/{major}/hotspot?architecture={arch}&image_type=jre&os={os}&vendor=eclipse&page_size=1"
    );
    let assets: Vec<AdoptiumAsset> = fetch_json(client, &url).await?;
    let package = &assets
        .first()
        .ok_or_else(|| {
            Error::JavaRuntime(format!(
                "Adoptium has no Java {major} {os}/{arch} jre build"
            ))
        })?
        .binary
        .package;
    let staging = staging_dir(dirs, major);
    std::fs::create_dir_all(&staging)?;
    let is_zip = package
        .link
        .rsplit('.')
        .next()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    let archive = staging.join(if is_zip {
        "runtime.zip"
    } else {
        "runtime.tar.gz"
    });
    let result = async {
        download::fetch(client, &package.link, &archive, None, progress.as_deref()).await?;
        let actual = download::sha256_file(&archive).await.ok_or_else(|| {
            Error::JavaRuntime("could not read the downloaded archive".to_owned())
        })?;
        if !actual.eq_ignore_ascii_case(&package.checksum) {
            return Err(Error::ChecksumMismatch {
                url: package.link.clone(),
                expected: package.checksum.clone(),
                actual,
            });
        }
        extract_archive(&archive, &staging)?;
        std::fs::remove_file(&archive)?;
        let root = find_runtime_root(&staging).ok_or_else(|| {
            Error::JavaRuntime("archive contained no JVM (no bin/java found)".to_owned())
        })?;
        let root_major = runtime_major(&root).ok_or_else(|| {
            Error::JavaRuntime("could not determine the extracted runtime's version".to_owned())
        })?;
        if root_major != major {
            return Err(Error::JavaRuntime(format!(
                "Adoptium returned Java {root_major}, not {major}"
            )));
        }
        let dest = managed_dir(dirs, major);
        std::fs::remove_dir_all(&dest).ok();
        std::fs::rename(&root, &dest)?;
        Ok(())
    }
    .await;
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn find_runtime_root(staging: &Path) -> Option<PathBuf> {
    let java = staging.join("bin").join(java_executable_name());
    if java.is_file() {
        return Some(staging.to_path_buf());
    }
    for entry in std::fs::read_dir(staging).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() && path.join("bin").join(java_executable_name()).is_file() {
            return Some(path);
        }
    }
    None
}

fn extract_archive(archive: &Path, target: &Path) -> Result<()> {
    if archive
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e == "zip")
    {
        extract_zip(archive, target)
    } else {
        extract_tar_gz(archive, target)
    }
}

fn extract_zip(archive: &Path, target: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let dest = target.join(crate::launch::sanitize_entry_path(entry.name())?);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output = std::fs::File::create(&dest)?;
        std::io::copy(&mut entry, &mut output)?;
        let executable = entry.unix_mode().is_some_and(|mode| mode & 0o111 != 0);
        mark_executable(&dest, executable);
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, target: &Path) -> Result<()> {
    use flate2::read::GzDecoder;
    let file = std::fs::File::open(archive)?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let dest = target.join(crate::launch::sanitize_entry_path(&path.to_string_lossy())?);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let executable = entry.header().mode().is_ok_and(|mode| mode & 0o111 != 0);
        entry.unpack(&dest)?;
        mark_executable(&dest, executable);
    }
    Ok(())
}

#[cfg(unix)]
fn mark_executable(path: &Path, executable: bool) {
    use std::os::unix::fs::PermissionsExt;
    if executable {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
}

#[cfg(not(unix))]
fn mark_executable(_path: &Path, _executable: bool) {}

async fn fetch_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T> {
    let bytes = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Mojang product manifest: `<os> -> <component> -> [entries]`.
#[derive(Debug, Deserialize)]
struct MojangJavaManifest(BTreeMap<String, BTreeMap<String, Vec<MojangComponentEntry>>>);

impl MojangJavaManifest {
    fn by_os_component(&self, os: &str, component: &str) -> Option<&MojangComponentEntry> {
        self.0.get(os)?.get(component)?.first()
    }
}

#[derive(Debug, Deserialize)]
struct MojangComponentEntry {
    manifest: MojangManifestRef,
    version: MojangRuntimeVersion,
}

#[derive(Debug, Deserialize)]
struct MojangManifestRef {
    #[allow(dead_code)]
    sha1: String,
    #[allow(dead_code)]
    size: u64,
    url: String,
}

#[derive(Debug, Deserialize)]
struct MojangRuntimeVersion {
    name: String,
    #[allow(dead_code)]
    released: String,
}

/// Mojang component manifest: relative file path -> entry.
#[derive(Debug, Deserialize)]
struct MojangComponentManifest {
    files: BTreeMap<String, MojangFileEntry>,
}

#[derive(Debug, Deserialize)]
struct MojangFileEntry {
    r#type: String,
    #[serde(default)]
    downloads: Option<MojangFileDownloads>,
    #[serde(default)]
    executable: bool,
}

#[derive(Debug, Deserialize)]
struct MojangFileDownloads {
    #[serde(default)]
    raw: Option<ArtifactDownload>,
    #[allow(dead_code)]
    #[serde(default)]
    lzma: Option<ArtifactDownload>,
}

#[derive(Debug, Deserialize)]
struct AdoptiumAsset {
    binary: AdoptiumBinary,
}

#[derive(Debug, Deserialize)]
struct AdoptiumBinary {
    #[serde(rename = "package")]
    package: AdoptiumPackage,
}

#[derive(Debug, Deserialize)]
struct AdoptiumPackage {
    link: String,
    #[allow(dead_code)]
    name: String,
    checksum: String,
}

fn mojang_os_key() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x64",
        ("windows", "aarch64") => "windows-arm64",
        ("windows", _) => "windows-x86",
        ("macos", "aarch64") => "mac-os-arm64",
        ("macos", _) => "mac-os",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "x86") => "linux-i386",
        _ => "linux",
    }
}

fn adoptium_os_arch() -> (&'static str, &'static str) {
    let os = match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "mac",
        _ => "linux",
    };
    let arch = match std::env::consts::ARCH {
        "x86" => "x86",
        "aarch64" => "aarch64",
        "arm" => "arm",
        "powerpc64" => "ppc64le",
        "s390x" => "s390x",
        _ => "x64",
    };
    (os, arch)
}

// --- Selection (TASK-9ik83) -------------------------------------------------

/// Pick the best runtime for a launch: an explicit `configured` path wins;
/// then a system JVM of the exact required major; then a managed
/// (auto-downloaded) runtime of that major; then the nearest system JVM.
/// Versions without a declared `javaVersion` need Java 8.
///
/// # Errors
///
/// Fails when no runtime can be found or downloaded.
pub async fn resolve_runtime(
    dirs: &Directories,
    client: &reqwest::Client,
    required: Option<&JavaVersion>,
    configured: Option<&Path>,
    progress: Option<ProgressFn>,
) -> Result<JavaRuntime> {
    if let Some(path) = configured {
        let exe = crate::launch::resolve_java(Some(path))?;
        let home = java_home_of_exe(&exe);
        let major = runtime_major(&home).unwrap_or(0);
        return Ok(JavaRuntime {
            home,
            major,
            source: RuntimeSource::Configured,
        });
    }
    let required_major = required.map_or(8, |jv| u32::try_from(jv.major_version).unwrap_or(8));
    let system = detect_system();
    if let Some(best) = pick_system(&system, required_major) {
        return Ok(best);
    }
    let component = required
        .map(|jv| jv.component.clone())
        .filter(|c| !c.is_empty())
        .or_else(|| component_for_major(required_major).map(str::to_owned));
    if let Ok(runtime) =
        ensure_runtime(dirs, client, required_major, component.as_deref(), progress).await
    {
        return Ok(runtime);
    }
    if let Some(nearest) = nearest_system(&system, required_major) {
        return Ok(nearest);
    }
    Err(Error::JavaNotFound)
}

/// The exact-major system runtime, if any.
#[must_use]
fn pick_system(system: &[JavaRuntime], required: u32) -> Option<JavaRuntime> {
    system.iter().find(|r| r.major == required).cloned()
}

/// The closest system runtime: the smallest major >= required, else the
/// newest overall (list is sorted by major descending).
#[must_use]
fn nearest_system(system: &[JavaRuntime], required: u32) -> Option<JavaRuntime> {
    system
        .iter()
        .filter(|r| r.major >= required)
        .min_by_key(|r| r.major)
        .or_else(|| system.first())
        .cloned()
}

/// The home directory owning a resolved java executable (`bin/java` or the
/// executable's parent itself).
fn java_home_of_exe(exe: &Path) -> PathBuf {
    let parent = exe.parent().unwrap_or_else(|| Path::new(""));
    if parent.file_name().is_some_and(|n| n == "bin") {
        parent.parent().unwrap_or(exe).to_path_buf()
    } else {
        parent.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mc-launcher-java-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// A fake runtime home with a `release` file for `major`.
    fn fake_home(root: &Path, major: u32) -> PathBuf {
        let home = root.join(major.to_string()).join("bin");
        std::fs::create_dir_all(&home).expect("mkdir");
        std::fs::write(
            root.join(major.to_string()).join("release"),
            format!("JAVA_VERSION=\"{major}.0.1\"\n"),
        )
        .expect("write release");
        std::fs::write(
            home.join(if cfg!(target_os = "windows") {
                "java.exe"
            } else {
                "java"
            }),
            b"",
        )
        .expect("write java");
        root.join(major.to_string())
    }

    #[test]
    fn major_from_version_string_handles_all_formats() {
        assert_eq!(major_from_version_string("21.0.7"), Some(21));
        assert_eq!(major_from_version_string("1.8.0_442"), Some(8));
        assert_eq!(major_from_version_string("8u202"), Some(8));
        assert_eq!(major_from_version_string("16.0.1.9.1"), Some(16));
        assert_eq!(major_from_version_string("17.0.15"), Some(17));
        assert_eq!(major_from_version_string(" 25.0.1 "), Some(25));
        assert_eq!(major_from_version_string("junk"), None);
    }

    #[test]
    fn scan_root_finds_direct_and_bundled_homes() {
        let root = tempdir();
        let direct = fake_home(&root, 17);
        let bundle = root.join("bundle/Contents/Home");
        std::fs::create_dir_all(bundle.join("bin")).expect("mkdir");
        std::fs::write(bundle.join(java_exe_rel()), b"").expect("write");
        std::fs::write(bundle.join("release"), "JAVA_VERSION=\"21.0.7\"\n").expect("write");
        std::fs::write(root.join("not-a-jvm.txt"), b"").expect("write");

        let homes = scan_root(&root);
        assert!(homes.contains(&direct));
        assert!(homes.contains(&bundle));
        assert_eq!(homes.len(), 2);
    }

    #[test]
    fn runtime_major_reads_release_file() {
        let root = tempdir();
        let home = fake_home(&root, 21);
        assert_eq!(runtime_major(&home), Some(21));
        assert_eq!(read_release_major(&home), Some(21));
    }

    #[test]
    fn collect_runtimes_probes_dedups_and_sorts() {
        let root = tempdir();
        let java21 = fake_home(&root, 21);
        let java8 = fake_home(&root, 8);
        // The same home twice must appear once; a bogus dir is dropped.
        let runtimes = collect_runtimes(vec![
            java21.clone(),
            java21.clone(),
            java8.clone(),
            root.join("missing"),
        ]);
        assert_eq!(runtimes.len(), 2);
        assert_eq!(runtimes[0].major, 21);
        assert_eq!(runtimes[1].major, 8);
        assert!(runtimes.iter().all(|r| r.home.is_dir()));
    }

    #[test]
    fn pick_and_nearest_follow_the_policy() {
        let runtimes = vec![
            JavaRuntime {
                home: PathBuf::from("/jvm25"),
                major: 25,
                source: RuntimeSource::System,
            },
            JavaRuntime {
                home: PathBuf::from("/jvm17"),
                major: 17,
                source: RuntimeSource::System,
            },
            JavaRuntime {
                home: PathBuf::from("/jvm8"),
                major: 8,
                source: RuntimeSource::System,
            },
        ];
        assert_eq!(
            pick_system(&runtimes, 17).expect("exact").home,
            PathBuf::from("/jvm17")
        );
        assert!(pick_system(&runtimes, 21).is_none());
        // Smallest runtime >= required wins for a mismatch.
        assert_eq!(
            nearest_system(&runtimes, 21).expect("nearest").home,
            PathBuf::from("/jvm25")
        );
        // Newest overall when nothing is big enough.
        assert_eq!(
            nearest_system(&runtimes, 32).expect("newest").home,
            PathBuf::from("/jvm25")
        );
    }

    #[test]
    fn managed_runtime_and_runtimes_scan_the_cache() {
        let root = tempdir();
        let dirs = Directories::new(&root);
        fake_home(&dirs.java_dir(), 17);
        fake_home(&dirs.java_dir(), 21);
        std::fs::write(dirs.java_dir().join("stray.txt"), b"").expect("write");
        let managed = managed_runtimes(&dirs);
        assert_eq!(managed.len(), 2);
        assert_eq!(managed[0].major, 21);
        assert_eq!(managed[1].major, 17);
        assert_eq!(managed_runtime(&dirs, 21).expect("cached").major, 21);
        assert!(managed_runtime(&dirs, 8).is_none());
    }

    // --- Download integration tests (fixture server) ---

    fn sha1_hex(bytes: &[u8]) -> String {
        use sha1::Digest as _;
        let digest = sha1::Sha1::digest(bytes);
        let mut out = String::with_capacity(40);
        for byte in &digest {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
        }
        out
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::Digest as _;
        let digest = sha2::Sha256::digest(bytes);
        let mut out = String::with_capacity(64);
        for byte in &digest {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
        }
        out
    }

    fn java_exe_rel() -> &'static str {
        if cfg!(target_os = "windows") {
            "bin/java.exe"
        } else {
            "bin/java"
        }
    }

    /// The `release` file content for a fake runtime.
    fn release_body(major: u32) -> Vec<u8> {
        format!("JAVA_VERSION=\"{major}.0.7\"\n").into_bytes()
    }

    fn tar_gz_with_runtime(major: u32) -> Vec<u8> {
        use std::io::Write as _;
        let prefix = format!("runtime-{major}");
        let mut builder = tar::Builder::new(Vec::new());
        for (path, contents) in [
            (
                format!("{prefix}/{}", java_exe_rel()),
                b"#!/bin/sh\nexit 0\n".to_vec(),
            ),
            (format!("{prefix}/release"), release_body(major)),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(u64::try_from(contents.len()).expect("size"));
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, path, contents.as_slice())
                .expect("append");
        }
        let tar_bytes = builder.into_inner().expect("finish tar");
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_bytes).expect("write gz");
        encoder.finish().expect("finish gz").clone()
    }

    /// Bind a local HTTP listener and return it with its URL.
    fn bind_server() -> (std::net::TcpListener, String) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
        (listener, format!("http://{addr}"))
    }

    fn spawn_serve(listener: std::net::TcpListener, files: BTreeMap<String, Vec<u8>>) {
        use std::io::{Read, Write};
        use std::thread;
        thread::spawn(move || {
            for stream in listener.incoming() {
                let files = files.clone();
                thread::spawn(move || {
                    let mut stream = stream.expect("accept");
                    let mut buf = [0u8; 8192];
                    let _ = stream.read(&mut buf);
                    let head = String::from_utf8_lossy(&buf);
                    let path = head
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .unwrap_or("/")
                        .split('?')
                        .next()
                        .unwrap_or("/")
                        .to_owned();
                    let body = files.get(&path).cloned().unwrap_or_default();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.write_all(&body);
                    let _ = stream.flush();
                });
            }
        });
    }

    fn mojang_product_manifest(base: &str, component: &str, version_name: &str) -> Vec<u8> {
        format!(
            r#"{{"{os}": {{"{component}": [{{"manifest": {{"sha1": "0", "size": 0, "url": "{base}/component.json"}}, "version": {{"name": "{version_name}", "released": "2026-01-01T00:00:00+00:00"}}}}]}}}}"#,
            os = mojang_os_key(),
            component = component,
        )
        .into_bytes()
    }

    fn mojang_component_manifest(base: &str) -> Vec<u8> {
        let exe = java_exe_rel();
        format!(
            r#"{{"files": {{
                "bin": {{"type": "directory"}},
                "{exe}": {{"type": "file", "executable": true, "downloads": {{"raw": {{"sha1": "{exe_sha}", "size": {exe_size}, "url": "{base}/bin/java"}}}}}},
                "bin/foo.dll": {{"type": "file", "downloads": {{"raw": {{"sha1": "{foo_sha}", "size": 4, "url": "{base}/bin/foo.dll"}}}}}},
                "bin/foo.exe": {{"type": "file", "downloads": {{"raw": {{"sha1": "{foo_sha}", "size": 4, "url": "{base}/bin/foo.exe"}}}}}},
                "release": {{"type": "file", "downloads": {{"raw": {{"sha1": "{rel_sha}", "size": {rel_size}, "url": "{base}/release"}}}}}}
            }}}}"#,
            exe = exe,
            exe_sha = sha1_hex(b"#!/bin/sh\nexit 0\n"),
            exe_size = 17,
            foo_sha = sha1_hex(b"foo!"),
            rel_sha = sha1_hex(&release_body(21)),
            rel_size = release_body(21).len(),
        )
        .into_bytes()
    }

    #[tokio::test]
    async fn installs_runtime_from_mojang_manifest() {
        let (listener, base) = bind_server();
        let java_body = b"#!/bin/sh\nexit 0\n".to_vec();
        let rel_body = release_body(21);
        spawn_serve(
            listener,
            BTreeMap::from([
                (
                    "/all.json".to_owned(),
                    mojang_product_manifest(&base, "java-runtime-delta", "21.0.7"),
                ),
                (
                    "/component.json".to_owned(),
                    mojang_component_manifest(&base),
                ),
                ("/bin/java".to_owned(), java_body),
                ("/bin/foo.dll".to_owned(), b"foo!".to_vec()),
                ("/bin/foo.exe".to_owned(), b"foo!".to_vec()),
                ("/release".to_owned(), rel_body),
            ]),
        );

        let dirs = Directories::new(tempdir());
        let runtime = ensure_runtime_from(
            &dirs,
            &reqwest::Client::new(),
            21,
            Some("java-runtime-delta"),
            None,
            &format!("{base}/all.json"),
            "http://127.0.0.1:1/latest",
        )
        .await
        .expect("install");
        assert_eq!(runtime.major, 21);
        assert_eq!(runtime.source, RuntimeSource::Managed);
        assert!(runtime.java_executable().is_file());
        assert!(runtime.home.join("bin/foo.dll").is_file());
        assert!(runtime.home.join("bin/foo.exe").is_file());
        assert!(runtime.home.join("release").is_file());

        // A second call is a cache hit: no network is used (both URLs below
        // would fail on any request).
        let again = ensure_runtime_from(
            &dirs,
            &reqwest::Client::new(),
            21,
            Some("java-runtime-delta"),
            None,
            "http://127.0.0.1:1/all.json",
            "http://127.0.0.1:1/latest",
        )
        .await
        .expect("cache hit");
        assert_eq!(again, runtime);
        // No staging leftovers.
        let leftovers: Vec<_> = std::fs::read_dir(dirs.java_dir())
            .expect("read java dir")
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(
            leftovers.is_empty(),
            "staging dirs left behind: {leftovers:?}"
        );
    }

    #[tokio::test]
    async fn falls_back_to_adoptium_when_component_major_mismatches() {
        // The manifest entry claims Java 16 for a request of Java 8 (the
        // 1.16.5 situation): the Mojang path must be skipped and Adoptium
        // used instead.
        let (listener, base) = bind_server();
        let archive = tar_gz_with_runtime(8);
        let assets_json = format!(
            r#"[{{"binary": {{"package": {{"link": "{base}/jre.tar.gz", "name": "jre.tar.gz", "checksum": "{sha}"}}}}}}]"#,
            sha = sha256_hex(&archive),
        );
        spawn_serve(
            listener,
            BTreeMap::from([
                (
                    "/all.json".to_owned(),
                    mojang_product_manifest(&base, "jre-legacy", "16.0.1.9.1"),
                ),
                ("/8/hotspot".to_owned(), assets_json.into_bytes()),
                ("/jre.tar.gz".to_owned(), archive),
            ]),
        );

        let dirs = Directories::new(tempdir());
        let runtime = ensure_runtime_from(
            &dirs,
            &reqwest::Client::new(),
            8,
            Some("jre-legacy"),
            None,
            &format!("{base}/all.json"),
            &format!("{base}/"),
        )
        .await
        .expect("adoptium fallback");
        assert_eq!(runtime.major, 8);
        assert!(runtime.java_executable().is_file());
        assert!(runtime.home.join("release").is_file());
        assert!(dirs.java_dir().join("8").is_dir());
    }

    #[tokio::test]
    async fn download_failure_reports_both_sources() {
        let (listener, base) = bind_server();
        spawn_serve(
            listener,
            BTreeMap::from([
                (
                    "/all.json".to_owned(),
                    mojang_product_manifest(&base, "java-runtime-delta", "21.0.7"),
                ),
                ("/8/hotspot".to_owned(), b"[]".to_vec()),
            ]),
        );
        let dirs = Directories::new(tempdir());
        dirs.ensure_all().expect("ensure");
        let err = ensure_runtime_from(
            &dirs,
            &reqwest::Client::new(),
            8,
            Some("java-runtime-delta"),
            None,
            &format!("{base}/all.json"),
            &format!("{base}/"),
        )
        .await
        .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("Mojang"), "message: {message}");
        assert!(message.contains("Adoptium"), "message: {message}");
        // Nothing cached, no staging leftovers.
        assert!(
            std::fs::read_dir(dirs.java_dir())
                .expect("read dir")
                .next()
                .is_none()
        );
    }
}
