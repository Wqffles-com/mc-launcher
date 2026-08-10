//! The install & launch engine: artifact downloads, natives unpacking, asset
//! layout, argument resolution and process lifecycle.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use tokio::io::AsyncWriteExt;

use crate::args;
use crate::assets;
use crate::clock;
use crate::dirs::Directories;
use crate::download;
use crate::error::{Error, Result};
use crate::rules::{Features, Platform, resolve_libraries};
use crate::version_json::VersionJson;
use crate::version_manifest::VersionInfo;

/// Maximum concurrent library downloads.
const LIBRARY_CONCURRENCY: usize = 16;

/// A player profile to launch with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    pub name: String,
    /// UUID without dashes.
    pub uuid: String,
    pub access_token: String,
    pub user_type: String,
}

impl Player {
    /// An offline profile: access token `0`, `legacy` user type, and a
    /// version-3 UUID derived from `OfflinePlayer:<name>` — the historical
    /// offline-mode scheme. Used until Microsoft auth lands.
    #[must_use]
    pub fn offline(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            uuid: offline_uuid(name),
            access_token: "0".to_owned(),
            user_type: "legacy".to_owned(),
        }
    }

    /// A Microsoft account profile: real UUID, `msa` user type and the live
    /// Minecraft access token. The UUID is normalized to the dash-less form
    /// the game expects.
    #[must_use]
    pub fn microsoft(name: &str, uuid_dashed: &str, access_token: &str) -> Self {
        Self {
            name: name.to_owned(),
            uuid: uuid_dashed.replace('-', ""),
            access_token: access_token.to_owned(),
            user_type: "msa".to_owned(),
        }
    }
}

/// The outcome of a finished game process.
#[derive(Debug, Clone)]
pub struct LaunchOutcome {
    pub exit: std::process::ExitStatus,
    /// File with the captured game stdout/stderr.
    pub log_file: PathBuf,
    /// RFC 3339 UTC timestamp of the launch.
    pub started_at: String,
}

/// Shared game output callback.
pub type OutputCallback = Box<dyn Fn(&str) + Send + Sync>;

/// Options controlling a game launch.
pub struct LaunchOptions {
    /// The game directory (an instance's game dir, or a scratch dir for bare
    /// version launches).
    pub game_dir: PathBuf,
    /// Java executable (or JAVA_HOME-style directory). Defaults to
    /// `JAVA_HOME/bin/java` or `java` on PATH.
    pub java: Option<PathBuf>,
    /// JVM heap (`-Xmx`) size, e.g. `2G`.
    pub memory: Option<String>,
    /// Custom window size; enables the `has_custom_resolution` feature.
    pub resolution: Option<(u32, u32)>,
    /// Called with each line of game stdout/stderr (after it is logged).
    pub on_output: Option<OutputCallback>,
}

/// Everything downloaded and resolved for a version, ready to launch.
#[derive(Debug)]
pub struct InstalledVersion {
    pub version: VersionJson,
    pub client_jar: PathBuf,
    /// Library jars in classpath order.
    pub libraries: Vec<PathBuf>,
    pub natives_dir: PathBuf,
    pub assets_root: PathBuf,
    pub asset_index_id: Option<String>,
    pub virtual_assets: bool,
    /// Expanded logging argument (e.g. `-Dlog4j.configurationFile=<path>`).
    pub logging_argument: Option<String>,
}

/// Fetch and cache a per-version JSON under `cache/versions/<id>.json`.
/// `force` refetches even when the cache is present.
///
/// # Errors
///
/// Fails on network errors or invalid JSON in the response.
pub async fn load_version_json(
    dirs: &Directories,
    client: &reqwest::Client,
    info: &VersionInfo,
    force: bool,
) -> Result<VersionJson> {
    let path = dirs
        .cache_dir()
        .join("versions")
        .join(format!("{}.json", info.id));
    if !force
        && let Ok(bytes) = tokio::fs::read(&path).await
        && let Ok(version) = serde_json::from_slice(&bytes)
    {
        return Ok(version);
    }
    let version = crate::version_json::fetch(client, &info.url).await?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, serde_json::to_vec_pretty(&version)?).await?;
    Ok(version)
}

/// Download everything a version needs: the client jar, all allowed
/// libraries (in parallel), the platform natives (unpacked), the asset
/// index with its objects, and the logging config.
///
/// Every artifact is verified by SHA-1 and reused when already present, so
/// repeated installs are cheap.
///
/// `game_dir` receives the materialized assets of legacy (non-virtual)
/// indexes.
///
/// # Errors
///
/// Fails on network errors, checksum mismatches, malformed archives, or
/// versions without a client jar.
pub async fn install(
    dirs: &Directories,
    client: &reqwest::Client,
    version: &VersionJson,
    game_dir: &Path,
    progress: Option<assets::ProgressFn>,
) -> Result<InstalledVersion> {
    install_with_asset_base(
        dirs,
        client,
        version,
        game_dir,
        progress,
        assets::ASSETS_BASE_URL,
    )
    .await
}

/// Like [`install`], but with a custom asset object base URL (used by
/// tests and mirrors).
///
/// # Errors
///
/// Same as [`install`].
#[doc(hidden)]
pub async fn install_with_asset_base(
    dirs: &Directories,
    client: &reqwest::Client,
    version: &VersionJson,
    game_dir: &Path,
    progress: Option<assets::ProgressFn>,
    asset_base_url: &str,
) -> Result<InstalledVersion> {
    let platform = Platform::current();
    let downloads_root = dirs.downloads_dir();

    let Some(client_download) = version.downloads.client.as_ref() else {
        return Err(Error::NoClientJar(version.id.clone()));
    };
    let client_jar = downloads_root
        .join("versions")
        .join(&version.id)
        .join("client.jar");
    download::fetch(
        client,
        &client_download.url,
        &client_jar,
        Some((&client_download.sha1, client_download.size)),
        progress.as_deref(),
    )
    .await?;

    let library_files = resolve_libraries(version, &platform, &Features::default())?;
    let libraries_root = downloads_root.join("libraries");
    let libraries =
        download_libraries(client, &library_files, &libraries_root, progress.clone()).await?;

    let natives_dir = downloads_root.join("natives").join(&version.id);
    let native_files: Vec<_> = library_files.iter().filter(|f| f.extract).collect();
    if !native_files.is_empty() {
        std::fs::remove_dir_all(&natives_dir).ok();
        std::fs::create_dir_all(&natives_dir)?;
        for file in &native_files {
            extract_native_jar(
                &libraries_root.join(&file.path),
                &natives_dir,
                &file.exclude,
            )?;
        }
    }

    let (asset_index_id, virtual_assets) = install_assets(
        client,
        version,
        game_dir,
        &downloads_root.join("assets"),
        asset_base_url,
        progress.clone(),
    )
    .await?;

    let logging_argument = if let Some(logging) = &version.logging {
        let log_file = downloads_root.join("logging").join(&logging.client.file.id);
        download::fetch(
            client,
            &logging.client.file.url,
            &log_file,
            Some((&logging.client.file.sha1, logging.client.file.size)),
            progress.as_deref(),
        )
        .await?;
        Some(
            logging
                .client
                .argument
                .replace("${path}", &log_file.to_string_lossy()),
        )
    } else {
        None
    };

    Ok(InstalledVersion {
        version: version.clone(),
        client_jar,
        libraries,
        natives_dir,
        assets_root: downloads_root.join("assets"),
        asset_index_id,
        virtual_assets,
        logging_argument,
    })
}

async fn install_assets(
    client: &reqwest::Client,
    version: &VersionJson,
    game_dir: &Path,
    assets_root: &Path,
    asset_base_url: &str,
    progress: Option<assets::ProgressFn>,
) -> Result<(Option<String>, bool)> {
    let Some(index_download) = &version.asset_index else {
        return Ok((None, false));
    };
    let index_dest = assets_root
        .join("indexes")
        .join(format!("{}.json", index_download.id));
    let index = assets::fetch_index(client, index_download, &index_dest).await?;
    let objects_dir = assets_root.join("objects");
    assets::download_objects(client, &index, &objects_dir, asset_base_url, progress).await?;
    if index.virtual_ {
        assets::materialize(
            &index,
            &assets_root.join("virtual").join(&index_download.id),
            &objects_dir,
        )
        .await?;
    } else {
        let game_assets = game_dir.join("assets");
        assets::materialize(&index, &game_assets, &objects_dir).await?;
        // Legacy indexes are resolved by the game relative to its own assets
        // dir; put the index JSON there too.
        let index_name = index_dest
            .file_name()
            .ok_or_else(|| Error::InvalidAssetHash(index_download.id.clone()))?;
        let index_target = game_assets.join("indexes").join(index_name);
        if let Some(parent) = index_target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::copy(&index_dest, &index_target).await?;
    }
    Ok((Some(index_download.id.clone()), index.virtual_))
}

async fn download_libraries(
    client: &reqwest::Client,
    files: &[crate::rules::LibraryFile],
    libraries_root: &Path,
    progress: Option<assets::ProgressFn>,
) -> Result<Vec<PathBuf>> {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(LIBRARY_CONCURRENCY));
    let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let total = files.len();
    let mut tasks = tokio::task::JoinSet::new();
    let mut paths = Vec::with_capacity(files.len());
    for file in files {
        let dest = libraries_root.join(&file.path);
        paths.push(dest.clone());
        tasks.spawn(download_library(
            client.clone(),
            file.clone(),
            dest,
            Arc::clone(&semaphore),
            Arc::clone(&done),
            total,
            progress.clone(),
        ));
    }
    while let Some(joined) = tasks.join_next().await {
        joined.map_err(|e| Error::Task(e.to_string()))??;
    }
    Ok(paths)
}

async fn download_library(
    client: reqwest::Client,
    file: crate::rules::LibraryFile,
    dest: PathBuf,
    semaphore: Arc<tokio::sync::Semaphore>,
    done: Arc<std::sync::atomic::AtomicUsize>,
    total: usize,
    progress: Option<assets::ProgressFn>,
) -> Result<()> {
    let _permit = semaphore
        .acquire_owned()
        .await
        .map_err(|e| Error::Task(e.to_string()))?;
    let expected = (!file.download.sha1.is_empty())
        .then_some((file.download.sha1.as_str(), file.download.size));
    download::fetch(&client, &file.download.url, &dest, expected, None).await?;
    let count = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if let Some(cb) = progress {
        cb(download::Progress::BatchDone {
            name: file.path.display().to_string(),
            done: count,
            total,
        });
    }
    Ok(())
}

/// Unpack a native library archive into the natives directory, skipping
/// `exclude`d entry prefixes (e.g. `META-INF/`) and any unsafe paths.
///
/// # Errors
///
/// Fails on I/O errors, corrupt archives, or hostile entry paths.
pub fn extract_native_jar(archive_path: &Path, target: &Path, excludes: &[String]) -> Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_owned();
        if excludes
            .iter()
            .any(|prefix| name.starts_with(prefix.as_str()))
        {
            continue;
        }
        let dest = target.join(sanitize_entry_path(&name)?);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output = std::fs::File::create(&dest)?;
        std::io::copy(&mut entry, &mut output)?;
    }
    Ok(())
}

fn sanitize_entry_path(name: &str) -> Result<PathBuf> {
    let normalized = name.replace('\\', "/");
    let path = Path::new(&normalized);
    if path.is_absolute() {
        return Err(Error::UnsafeZipPath(name.to_owned()));
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(Error::UnsafeZipPath(name.to_owned()));
        }
    }
    Ok(path.to_path_buf())
}

/// Locate a Java executable: the configured path (a file, or a directory
/// containing `bin/java`), else `JAVA_HOME`, else `java` on PATH.
///
/// # Errors
///
/// Fails when no Java runtime can be found.
pub fn resolve_java(configured: Option<&Path>) -> Result<PathBuf> {
    let exe = java_executable_name();
    if let Some(path) = configured {
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
        let candidate = path.join("bin").join(exe);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        let candidate = PathBuf::from(home).join("bin").join(exe);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(exe);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(Error::JavaNotFound)
}

fn java_executable_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "java.exe"
    } else {
        "java"
    }
}

/// Build the full Java command line for an installed version.
///
/// # Errors
///
/// Fails if the version has no main class or no Java runtime is available.
pub fn build_command(
    installed: &InstalledVersion,
    player: &Player,
    options: &LaunchOptions,
    platform: &Platform,
) -> Result<Vec<String>> {
    let mut template = args::Template::default();
    let game_dir = &options.game_dir;
    let assets_root = &installed.assets_root;
    let game_assets = if installed.virtual_assets {
        assets_root.clone()
    } else {
        game_dir.join("assets")
    };
    template.insert(args::TOKEN_AUTH_PLAYER_NAME, &player.name);
    template.insert(args::TOKEN_VERSION_NAME, &installed.version.id);
    template.insert(args::TOKEN_VERSION_TYPE, &installed.version.kind);
    template.insert(
        args::TOKEN_GAME_DIRECTORY,
        game_dir.to_string_lossy().into_owned(),
    );
    template.insert(
        args::TOKEN_GAME_ASSETS,
        game_assets.to_string_lossy().into_owned(),
    );
    template.insert(
        args::TOKEN_ASSETS_ROOT,
        assets_root.to_string_lossy().into_owned(),
    );
    template.insert(
        args::TOKEN_ASSETS_INDEX_NAME,
        installed.asset_index_id.clone().unwrap_or_default(),
    );
    template.insert(args::TOKEN_AUTH_UUID, &player.uuid);
    template.insert(args::TOKEN_AUTH_ACCESS_TOKEN, &player.access_token);
    template.insert(args::TOKEN_USER_TYPE, &player.user_type);
    template.insert(args::TOKEN_USER_PROPERTIES, "{}");
    template.insert(
        args::TOKEN_NATIVES_DIRECTORY,
        installed.natives_dir.to_string_lossy().into_owned(),
    );
    template.insert(args::TOKEN_LAUNCHER_NAME, "mc-launcher");
    template.insert(args::TOKEN_LAUNCHER_VERSION, env!("CARGO_PKG_VERSION"));
    let mut classpath_paths: Vec<&Path> = vec![installed.client_jar.as_path()];
    classpath_paths.extend(installed.libraries.iter().map(PathBuf::as_path));
    let classpath = args::classpath(&classpath_paths);
    template.insert(args::TOKEN_CLASSPATH, &classpath);
    let separator = if cfg!(target_os = "windows") {
        ";"
    } else {
        ":"
    };
    template.insert(args::TOKEN_CLASSPATH_SEPARATOR, separator);

    let mut features = Features::default();
    if let Some((width, height)) = options.resolution {
        template.insert(args::TOKEN_RESOLUTION_WIDTH, width.to_string());
        template.insert(args::TOKEN_RESOLUTION_HEIGHT, height.to_string());
        features.has_custom_resolution = true;
    }

    let mut jvm_args = args::jvm_arguments(&installed.version, &template, platform, &features)?;
    if let Some(argument) = &installed.logging_argument {
        jvm_args.push(argument.clone());
    }
    let game_args = args::game_arguments(&installed.version, &template, platform, &features)?;

    let main_class = installed
        .version
        .main_class
        .clone()
        .ok_or_else(|| Error::NoMainClass(installed.version.id.clone()))?;

    let java = resolve_java(options.java.as_deref())?;
    let mut command = vec![java.to_string_lossy().into_owned()];
    if let Some(memory) = &options.memory {
        command.push(format!("-Xmx{memory}"));
    }
    command.extend(jvm_args);
    command.push(main_class);
    command.extend(game_args);
    Ok(command)
}

/// Install (if needed) and launch a version, streaming game stdout/stderr to
/// `<game_dir>/logs/launcher/<timestamp>.log` and the `on_output` callback.
/// Returns when the game exits.
///
/// # Errors
///
/// Fails if the install fails, no Java runtime is found, or the process
/// cannot be spawned.
pub async fn launch(
    dirs: &Directories,
    client: &reqwest::Client,
    version: &VersionJson,
    player: &Player,
    options: LaunchOptions,
    progress: Option<assets::ProgressFn>,
) -> Result<LaunchOutcome> {
    let installed = install(dirs, client, version, &options.game_dir, progress).await?;
    let platform = Platform::current();
    let command = build_command(&installed, player, &options, &platform)?;
    let java = &command[0];

    let log_dir = options.game_dir.join("logs").join("launcher");
    std::fs::create_dir_all(&log_dir)?;
    let started_at = clock::now_rfc3339();
    let log_file = log_dir.join(format!("{}.log", started_at.replace(':', "-")));

    let mut child = tokio::process::Command::new(java)
        .args(&command[1..])
        .current_dir(&options.game_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::JavaNotFound
            } else {
                Error::Io(e)
            }
        })?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Io(io_error("failed to capture stdout")))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Io(io_error("failed to capture stderr")))?;
    let stdout_log = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .await?;
    let stderr_log = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .await?;
    let on_output: Option<Arc<OutputCallback>> = options.on_output.map(Arc::from);
    let stdout_task = tokio::spawn(forward_lines(stdout, stdout_log, on_output.clone()));
    let stderr_task = tokio::spawn(forward_lines(stderr, stderr_log, on_output));

    let exit = child.wait().await?;
    let _ = tokio::join!(stdout_task, stderr_task);
    Ok(LaunchOutcome {
        exit,
        log_file,
        started_at,
    })
}

fn io_error(message: &str) -> std::io::Error {
    std::io::Error::other(message)
}

async fn forward_lines(
    stream: impl tokio::io::AsyncRead + Unpin,
    mut log: tokio::fs::File,
    on_output: Option<Arc<OutputCallback>>,
) {
    use tokio::io::AsyncBufReadExt;
    let mut lines = tokio::io::BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let _ = log.write_all(format!("{line}\n").as_bytes()).await;
        if let Some(callback) = &on_output {
            callback(&line);
        }
    }
}

/// Version-3 UUID (no dashes) derived from `OfflinePlayer:<name>`.
fn offline_uuid(name: &str) -> String {
    use md5::{Digest, Md5};
    use std::fmt::Write as _;
    let digest = Md5::digest(format!("OfflinePlayer:{name}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(digest.as_slice());
    bytes[6] = (bytes[6] & 0x0F) | 0x30;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let mut out = String::with_capacity(32);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::*;
    use crate::dirs::Directories;
    use crate::version_json::{
        Argument, Arguments, ArtifactDownload, AssetIndex, Downloads, JavaVersion, Library,
        LibraryDownloads, Logging, LoggingClient, LoggingFile,
    };
    use sha1::Digest as _;

    fn tempdir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mc-launcher-launch-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn sha1_hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        let digest = sha1::Sha1::digest(bytes);
        let mut out = String::with_capacity(40);
        for byte in &digest {
            let _ = write!(out, "{byte:02x}");
        }
        out
    }

    /// The platform-canonical rendering of a path in command-line args.
    fn shown(path: &str) -> String {
        PathBuf::from(path).to_string_lossy().into_owned()
    }

    /// Serve a map of GET path -> body until the test process exits.
    fn serve_files(files: BTreeMap<String, Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
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
        format!("http://{addr}")
    }

    fn dl(url: &str, sha1: &str, size: u64) -> ArtifactDownload {
        ArtifactDownload {
            sha1: sha1.to_owned(),
            size,
            url: url.to_owned(),
        }
    }

    const CLIENT_BODY: &[u8] = b"client jar";
    const LIB_BODY: &[u8] = b"library jar";
    const LOGGING_BODY: &[u8] = b"<log4j2/>";

    /// A modern-format version with client jar, one plain library, an
    /// optional native library (for the current OS) and a logging config.
    fn modern_version(
        base: &str,
        asset_index: Option<AssetIndex>,
        native: Option<(&str, u64)>,
    ) -> VersionJson {
        let native_key = format!("natives-{}", Platform::current().os);
        let native_library = native.map(|(sha1, size)| Library {
            name: "org.example:natives:1.0".to_owned(),
            downloads: Some(LibraryDownloads {
                artifact: None,
                classifiers: Some(BTreeMap::from([(
                    native_key.clone(),
                    dl(&format!("{base}/native.jar"), sha1, size),
                )])),
            }),
            rules: None,
            natives: Some(BTreeMap::from([(
                Platform::current().os.to_owned(),
                native_key,
            )])),
            extract: Some(crate::version_json::Extract {
                exclude: vec!["META-INF/".to_owned()],
            }),
        });
        VersionJson {
            id: "1.21.4".to_owned(),
            kind: "release".to_owned(),
            main_class: Some("net.minecraft.client.main.Main".to_owned()),
            arguments: Some(Arguments {
                game: vec![
                    Argument::Plain("--username".to_owned()),
                    Argument::Plain("${auth_player_name}".to_owned()),
                    Argument::Plain("--uuid".to_owned()),
                    Argument::Plain("${auth_uuid}".to_owned()),
                    Argument::Plain("--gameDir".to_owned()),
                    Argument::Plain("${game_directory}".to_owned()),
                    Argument::Plain("--assetsDir".to_owned()),
                    Argument::Plain("${assets_root}".to_owned()),
                    Argument::Plain("--assetIndex".to_owned()),
                    Argument::Plain("${assets_index_name}".to_owned()),
                ],
                jvm: vec![
                    Argument::Plain("-Djava.library.path=${natives_directory}".to_owned()),
                    Argument::Plain("-cp ${classpath}".to_owned()),
                ],
            }),
            minecraft_arguments: None,
            asset_index,
            assets: Some("25".to_owned()),
            java_version: Some(JavaVersion {
                component: "java-runtime-delta".to_owned(),
                major_version: 21,
            }),
            downloads: Downloads {
                client: Some(dl(
                    &format!("{base}/client.jar"),
                    &sha1_hex(CLIENT_BODY),
                    CLIENT_BODY.len() as u64,
                )),
                client_mappings: None,
                server: None,
                server_mappings: None,
            },
            libraries: std::iter::once(Library {
                name: "org.example:lib:1.0".to_owned(),
                downloads: Some(LibraryDownloads {
                    artifact: Some(dl(
                        &format!("{base}/lib.jar"),
                        &sha1_hex(LIB_BODY),
                        LIB_BODY.len() as u64,
                    )),
                    classifiers: None,
                }),
                rules: None,
                natives: None,
                extract: None,
            })
            .chain(native_library)
            .collect(),
            logging: Some(Logging {
                client: LoggingClient {
                    argument: "-Dlog4j.configurationFile=${path}".to_owned(),
                    file: LoggingFile {
                        id: "client-1.21.xml".to_owned(),
                        sha1: sha1_hex(LOGGING_BODY),
                        size: LOGGING_BODY.len() as u64,
                        url: format!("{base}/log.xml"),
                    },
                    kind: "log4j2-xml".to_owned(),
                },
            }),
            minimum_launcher_version: Some(21),
            time: "2024-12-03T12:35:58+00:00".to_owned(),
            release_time: "2024-12-03T09:23:39+00:00".to_owned(),
        }
    }

    fn native_zip() -> Vec<u8> {
        let mut out = Vec::new();
        let mut writer = ZipWriter::new(std::io::Cursor::new(&mut out));
        let options = SimpleFileOptions::default();
        writer.start_file("libexample.dll", options).expect("start");
        writer.write_all(b"dll").expect("write");
        writer
            .start_file("META-INF/skip.txt", options)
            .expect("start");
        writer.write_all(b"skip").expect("write");
        writer.finish().expect("finish");
        out
    }

    #[tokio::test]
    async fn installs_client_libraries_natives_assets_and_logging() {
        let client_body = CLIENT_BODY.to_vec();
        let lib_body = LIB_BODY.to_vec();
        let logging_body = LOGGING_BODY.to_vec();
        let native_body = native_zip();
        let asset_body = b"asset bytes".to_vec();
        let asset_hash = sha1_hex(&asset_body);
        let index_body = format!(
            r#"{{"virtual": true, "objects": {{"icons/icon.png": {{"hash": "{asset_hash}", "size": {}}}}}}}"#,
            asset_body.len()
        )
        .into_bytes();
        let url = serve_files(BTreeMap::from([
            ("/client.jar".to_owned(), client_body.clone()),
            ("/lib.jar".to_owned(), lib_body.clone()),
            ("/native.jar".to_owned(), native_body.clone()),
            ("/log.xml".to_owned(), logging_body.clone()),
            ("/indexes/25.json".to_owned(), index_body.clone()),
            (
                format!("/{}/{}", &asset_hash[..2], asset_hash),
                asset_body.clone(),
            ),
        ]));
        let version = modern_version(
            &url,
            Some(AssetIndex {
                id: "25".to_owned(),
                sha1: sha1_hex(&index_body),
                size: index_body.len() as u64,
                total_size: 1,
                url: format!("{url}/indexes/25.json"),
            }),
            Some((&sha1_hex(&native_body), native_body.len() as u64)),
        );
        let dirs = Directories::new(tempdir());
        let game_dir = tempdir();
        let installed = install_with_asset_base(
            &dirs,
            &reqwest::Client::new(),
            &version,
            &game_dir,
            None,
            &url,
        )
        .await
        .expect("install");

        assert!(installed.client_jar.is_file());
        assert_eq!(
            std::fs::read(&installed.client_jar).expect("read"),
            client_body
        );
        assert_eq!(installed.libraries.len(), 2);
        assert_eq!(
            std::fs::read(&installed.libraries[0]).expect("read"),
            lib_body
        );
        // Natives were extracted, skipping META-INF.
        assert_eq!(
            std::fs::read(installed.natives_dir.join("libexample.dll")).expect("read dll"),
            b"dll"
        );
        assert!(!installed.natives_dir.join("META-INF/skip.txt").exists());
        assert!(installed.logging_argument.is_some());
        let argument = installed.logging_argument.as_ref().expect("logging");
        assert!(argument.starts_with("-Dlog4j.configurationFile="));
        assert!(argument.ends_with("client-1.21.xml"));
        assert_eq!(
            std::fs::read(dirs.downloads_dir().join("logging/client-1.21.xml"))
                .expect("read log config"),
            logging_body
        );
        // Virtual assets: index JSON + materialized object tree.
        assert_eq!(installed.asset_index_id.as_deref(), Some("25"));
        assert!(installed.virtual_assets);
        assert!(
            dirs.downloads_dir()
                .join("assets/indexes/25.json")
                .is_file()
        );
        assert_eq!(
            std::fs::read(
                dirs.downloads_dir()
                    .join("assets/virtual/25/icons/icon.png")
            )
            .expect("read asset"),
            asset_body
        );
        // Legacy game dir was not touched for virtual indexes.
        assert!(!game_dir.join("assets").exists());
    }

    #[tokio::test]
    async fn install_materializes_legacy_assets_into_game_dir() {
        let asset_body = b"legacy sound".to_vec();
        let asset_hash = sha1_hex(&asset_body);
        let index_body = format!(
            r#"{{"virtual": false, "objects": {{"sound/step/grass.ogg": {{"hash": "{asset_hash}", "size": {}}}}}}}"#,
            asset_body.len()
        )
        .into_bytes();
        let url = serve_files(BTreeMap::from([
            ("/client.jar".to_owned(), CLIENT_BODY.to_vec()),
            ("/lib.jar".to_owned(), LIB_BODY.to_vec()),
            ("/log.xml".to_owned(), LOGGING_BODY.to_vec()),
            ("/indexes/legacy.json".to_owned(), index_body.clone()),
            (
                format!("/{}/{}", &asset_hash[..2], asset_hash),
                asset_body.clone(),
            ),
        ]));
        let version = modern_version(
            &url,
            Some(AssetIndex {
                id: "legacy".to_owned(),
                sha1: sha1_hex(&index_body),
                size: index_body.len() as u64,
                total_size: 1,
                url: format!("{url}/indexes/legacy.json"),
            }),
            None,
        );
        let dirs = Directories::new(tempdir());
        let game_dir = tempdir();
        let installed = install_with_asset_base(
            &dirs,
            &reqwest::Client::new(),
            &version,
            &game_dir,
            None,
            &url,
        )
        .await
        .expect("install");

        assert!(!installed.virtual_assets);
        assert_eq!(
            std::fs::read(game_dir.join("assets/sound/step/grass.ogg")).expect("read asset"),
            asset_body
        );
        assert!(
            game_dir.join("assets/indexes/legacy.json").is_file(),
            "legacy index JSON must be visible to the game"
        );
    }

    #[tokio::test]
    async fn install_rejects_missing_client_jar() {
        let version = VersionJson {
            id: "server-only".to_owned(),
            kind: "release".to_owned(),
            main_class: Some("x".to_owned()),
            arguments: None,
            minecraft_arguments: None,
            asset_index: None,
            assets: None,
            java_version: None,
            downloads: Downloads::default(),
            libraries: Vec::new(),
            logging: None,
            minimum_launcher_version: None,
            time: String::new(),
            release_time: String::new(),
        };
        let err = install(
            &Directories::new(tempdir()),
            &reqwest::Client::new(),
            &version,
            &tempdir(),
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::NoClientJar(_)));
    }

    #[test]
    fn offline_uuid_is_v3_shaped_and_deterministic() {
        let a = offline_uuid("Notch");
        let b = offline_uuid("Notch");
        let c = offline_uuid("Steve");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(&a[12..13], "3");
        assert!(matches!(&a[16..17], "8" | "9" | "a" | "b"));
    }

    fn installed_fixture() -> InstalledVersion {
        InstalledVersion {
            version: VersionJson {
                id: "1.21.4".to_owned(),
                kind: "release".to_owned(),
                main_class: Some("net.minecraft.client.main.Main".to_owned()),
                arguments: Some(Arguments {
                    game: vec![
                        Argument::Plain("--username".to_owned()),
                        Argument::Plain("${auth_player_name}".to_owned()),
                        Argument::Plain("--uuid".to_owned()),
                        Argument::Plain("${auth_uuid}".to_owned()),
                        Argument::Plain("--assetsDir".to_owned()),
                        Argument::Plain("${assets_root}".to_owned()),
                        Argument::Plain("--assetIndex".to_owned()),
                        Argument::Plain("${assets_index_name}".to_owned()),
                    ],
                    jvm: vec![
                        Argument::Plain("-Djava.library.path=${natives_directory}".to_owned()),
                        Argument::Plain("-cp ${classpath}".to_owned()),
                    ],
                }),
                minecraft_arguments: None,
                asset_index: Some(AssetIndex {
                    id: "25".to_owned(),
                    sha1: "a".to_owned(),
                    size: 1,
                    total_size: 1,
                    url: "u".to_owned(),
                }),
                assets: Some("25".to_owned()),
                java_version: None,
                downloads: Downloads::default(),
                libraries: Vec::new(),
                logging: None,
                minimum_launcher_version: None,
                time: String::new(),
                release_time: String::new(),
            },
            client_jar: PathBuf::from("C:/dl/versions/1.21.4/client.jar"),
            libraries: vec![PathBuf::from("C:/dl/libraries/org/example/lib-1.0.jar")],
            natives_dir: PathBuf::from("C:/dl/natives/1.21.4"),
            assets_root: PathBuf::from("C:/dl/assets"),
            asset_index_id: Some("25".to_owned()),
            virtual_assets: true,
            logging_argument: None,
        }
    }

    #[test]
    fn build_command_assembles_full_java_invocation() {
        let installed = installed_fixture();
        let player = Player::offline("Steve");
        let java = tempdir().join(if cfg!(target_os = "windows") {
            "java.exe"
        } else {
            "java"
        });
        std::fs::write(&java, b"").expect("write java");
        let options = LaunchOptions {
            game_dir: PathBuf::from("C:/game"),
            java: Some(java.clone()),
            memory: Some("2G".to_owned()),
            resolution: None,
            on_output: None,
        };
        let command =
            build_command(&installed, &player, &options, &Platform::current()).expect("command");
        assert_eq!(PathBuf::from(&command[0]), java);
        assert!(command.contains(&"-Xmx2G".to_owned()));
        assert!(command.contains(&format!(
            "-Djava.library.path={}",
            shown("C:/dl/natives/1.21.4")
        )));
        let cp = command
            .iter()
            .find_map(|a| a.strip_prefix("-cp ").map(str::to_owned))
            .expect("classpath");
        let separator = if cfg!(target_os = "windows") {
            ";"
        } else {
            ":"
        };
        assert_eq!(
            cp,
            format!(
                "{}{separator}{}",
                shown("C:/dl/versions/1.21.4/client.jar"),
                shown("C:/dl/libraries/org/example/lib-1.0.jar")
            )
        );
        assert!(command.contains(&"net.minecraft.client.main.Main".to_owned()));
        assert!(command.contains(&"--username".to_owned()));
        assert!(command.contains(&"Steve".to_owned()));
        assert!(command.contains(&"--uuid".to_owned()));
        assert!(command.contains(&player.uuid));
        assert!(command.contains(&"--assetsDir".to_owned()));
        assert!(command.contains(&shown("C:/dl/assets")));
        assert!(command.contains(&"--assetIndex".to_owned()));
        assert!(command.contains(&"25".to_owned()));
    }

    #[test]
    fn build_command_sets_custom_resolution_feature() {
        let mut installed = installed_fixture();
        installed.version.arguments = Some(Arguments {
            game: vec![
                Argument::Plain("--username".to_owned()),
                Argument::Plain("${auth_player_name}".to_owned()),
                Argument::Ruled(crate::version_json::RuledArgument {
                    rules: vec![crate::version_json::Rule {
                        action: "allow".to_owned(),
                        os: None,
                        features: Some(BTreeMap::from([(
                            "has_custom_resolution".to_owned(),
                            true,
                        )])),
                    }],
                    value: crate::version_json::ArgumentValue::Multi(vec![
                        "--width".to_owned(),
                        "${resolution_width}".to_owned(),
                        "--height".to_owned(),
                        "${resolution_height}".to_owned(),
                    ]),
                }),
            ],
            jvm: Vec::new(),
        });
        let options = LaunchOptions {
            game_dir: PathBuf::from("g"),
            java: Some(PathBuf::from("java")),
            memory: None,
            resolution: Some((1920, 1080)),
            on_output: None,
        };
        let command = build_command(
            &installed,
            &Player::offline("A"),
            &options,
            &Platform::current(),
        )
        .expect("command");
        assert!(command.contains(&"--width".to_owned()));
        assert!(command.contains(&"1920".to_owned()));
        assert!(command.contains(&"--height".to_owned()));
        assert!(command.contains(&"1080".to_owned()));
    }

    #[test]
    fn build_command_uses_legacy_arguments_when_modern_block_is_absent() {
        let mut installed = installed_fixture();
        installed.version.arguments = None;
        installed.version.minecraft_arguments = Some(
            "--username ${auth_player_name} --gameDir ${game_directory} --assetsDir ${game_assets}"
                .to_owned(),
        );
        installed.version.asset_index = Some(AssetIndex {
            id: "legacy".to_owned(),
            sha1: "a".to_owned(),
            size: 1,
            total_size: 1,
            url: "u".to_owned(),
        });
        installed.asset_index_id = Some("legacy".to_owned());
        installed.virtual_assets = false;
        let options = LaunchOptions {
            game_dir: PathBuf::from("C:/game"),
            java: Some(PathBuf::from("java")),
            memory: None,
            resolution: None,
            on_output: None,
        };
        let command = build_command(
            &installed,
            &Player::offline("S"),
            &options,
            &Platform::current(),
        )
        .expect("command");
        // Legacy versions get the default JVM args.
        assert!(command.contains(&format!(
            "-Djava.library.path={}",
            shown("C:/dl/natives/1.21.4")
        )));
        assert!(command.iter().any(|a| a.starts_with("-cp ")));
        // Non-virtual assets resolve inside the game directory.
        let index = command
            .iter()
            .position(|a| a == "--assetsDir")
            .expect("assetsDir");
        assert_eq!(
            command[index + 1],
            PathBuf::from("C:/game")
                .join("assets")
                .to_string_lossy()
                .into_owned()
        );
    }

    #[test]
    fn resolve_java_prefers_configured_path() {
        let dir = tempdir();
        let exe = dir.join(if cfg!(target_os = "windows") {
            "java.exe"
        } else {
            "java"
        });
        std::fs::write(&exe, b"").expect("write");
        assert_eq!(resolve_java(Some(&exe)).expect("resolve"), exe);
        // A directory is accepted when it contains bin/java.
        let home = tempdir();
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).expect("mkdir");
        std::fs::write(
            bin.join(if cfg!(target_os = "windows") {
                "java.exe"
            } else {
                "java"
            }),
            b"",
        )
        .expect("write");
        assert_eq!(
            resolve_java(Some(&home)).expect("resolve"),
            bin.join(if cfg!(target_os = "windows") {
                "java.exe"
            } else {
                "java"
            })
        );
    }

    #[test]
    fn extract_native_jar_skips_excludes_and_unsafe_paths() {
        let dir = tempdir();
        let archive = dir.join("natives.jar");
        let mut writer = ZipWriter::new(std::fs::File::create(&archive).expect("create"));
        let options = SimpleFileOptions::default();
        for name in ["liba.dll", "META-INF/skip.txt", "../evil.txt"] {
            writer.start_file(name, options).expect("start");
            writer.write_all(b"x").expect("write");
        }
        writer.finish().expect("finish");

        let target = dir.join("out");
        let err = extract_native_jar(&archive, &target, &["META-INF/".to_owned()]);
        // The unsafe entry aborts extraction, but safe ones are in place.
        assert!(matches!(err, Err(Error::UnsafeZipPath(_))));
        assert_eq!(std::fs::read(target.join("liba.dll")).expect("read"), b"x");
        assert!(!target.join("META-INF/skip.txt").exists());
    }

    #[test]
    fn extract_native_jar_skips_excludes_only() {
        let dir = tempdir();
        let archive = dir.join("natives.jar");
        let mut writer = ZipWriter::new(std::fs::File::create(&archive).expect("create"));
        let options = SimpleFileOptions::default();
        writer.start_file("liba.dll", options).expect("start");
        writer.write_all(b"x").expect("write");
        writer
            .start_file("META-INF/skip.txt", options)
            .expect("start");
        writer.write_all(b"y").expect("write");
        writer.finish().expect("finish");

        let target = dir.join("out");
        extract_native_jar(&archive, &target, &["META-INF/".to_owned()]).expect("extract");
        assert!(target.join("liba.dll").is_file());
        assert!(!target.join("META-INF/skip.txt").exists());
    }
}
