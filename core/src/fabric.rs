//! Fabric loader install via the fabric-meta API.
//!
//! The loader metadata lives on `meta.fabricmc.net`: game versions supported
//! by Fabric, the loader versions available for each, and per-combination
//! launcher profiles. A profile is a version JSON that `inheritsFrom` the
//! game version and adds the loader's main class (`KnotClient`), its
//! libraries (from the `maven.fabricmc.net` repository) and its JVM
//! arguments; the launcher merges it with the game's own version JSON and
//! launches the merged document like any other version.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::dirs::Directories;
use crate::error::{Error, Result};
use crate::launch::InstalledVersion;
use crate::version_json::{Arguments, Library, VersionJson};

/// Base URL of the fabric-meta API (v2).
pub const FABRIC_META_URL: &str = "https://meta.fabricmc.net/v2";

/// A Minecraft version Fabric supports.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FabricGame {
    pub version: String,
    pub stable: bool,
}

/// A loader version available for a game version, plus the matching
/// intermediary (the mapping between Minecraft's obfuscated names and
/// Fabric's intermediary names).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FabricLoaderInfo {
    pub loader: FabricLoaderVersion,
    pub intermediary: FabricIntermediary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FabricLoaderVersion {
    pub separator: String,
    pub build: u32,
    pub maven: String,
    pub version: String,
    pub stable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FabricIntermediary {
    pub maven: String,
    pub version: String,
    pub stable: bool,
}

/// The `profile/json` document for a game + loader combination: a launcher
/// version JSON that inherits everything else from the game version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricProfile {
    pub id: String,
    #[serde(rename = "inheritsFrom")]
    pub inherits_from: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(rename = "mainClass")]
    pub main_class: String,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
    pub time: String,
}

/// GET a JSON document from the fabric-meta API, mapping a 404 response to
/// `not_found`.
async fn get_json<T>(client: &reqwest::Client, url: &str, not_found: Error) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let response = client.get(url).send().await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(not_found);
    }
    let bytes = response.error_for_status()?.bytes().await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Percent-encode a path segment for a URL (spaces in snapshot ids like
/// `1.14 Pre-Release 5` are not valid raw URL characters).
fn url_path_segment(segment: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// All game versions Fabric supports, newest first.
///
/// # Errors
///
/// Fails on network errors or invalid JSON in the response.
pub async fn list_games(client: &reqwest::Client) -> Result<Vec<FabricGame>> {
    list_games_with_base(client, FABRIC_META_URL).await
}

/// [`list_games`] against a custom meta base URL (tests, mirrors).
///
/// # Errors
///
/// Fails on network errors or invalid JSON in the response.
pub async fn list_games_with_base(
    client: &reqwest::Client,
    base: &str,
) -> Result<Vec<FabricGame>> {
    get_json(
        client,
        &format!("{base}/versions/game"),
        Error::FabricGameNotFound(String::new()),
    )
    .await
}

/// All loader versions available for `game_version`, newest first.
///
/// # Errors
///
/// Fails on network errors, invalid JSON, or when Fabric does not support
/// the game version.
pub async fn list_loaders(
    client: &reqwest::Client,
    game_version: &str,
) -> Result<Vec<FabricLoaderInfo>> {
    list_loaders_with_base(client, FABRIC_META_URL, game_version).await
}

/// [`list_loaders`] against a custom meta base URL (tests, mirrors).
///
/// # Errors
///
/// Fails on network errors, invalid JSON, or when Fabric does not support
/// the game version.
pub async fn list_loaders_with_base(
    client: &reqwest::Client,
    base: &str,
    game_version: &str,
) -> Result<Vec<FabricLoaderInfo>> {
    get_json(
        client,
        &format!(
            "{base}/versions/loader/{}",
            url_path_segment(game_version)
        ),
        Error::FabricGameNotFound(game_version.to_owned()),
    )
    .await
}

/// The newest stable loader version for `game_version`, falling back to the
/// newest version overall when none is marked stable.
///
/// # Errors
///
/// Fails on network errors, invalid JSON, an unsupported game version, or an
/// empty loader list.
pub async fn latest_loader(
    client: &reqwest::Client,
    game_version: &str,
) -> Result<FabricLoaderVersion> {
    latest_loader_with_base(client, FABRIC_META_URL, game_version).await
}

/// [`latest_loader`] against a custom meta base URL (tests, mirrors).
///
/// # Errors
///
/// Same as [`latest_loader`].
pub async fn latest_loader_with_base(
    client: &reqwest::Client,
    base: &str,
    game_version: &str,
) -> Result<FabricLoaderVersion> {
    let loaders = list_loaders_with_base(client, base, game_version).await?;
    loaders
        .iter()
        .find(|info| info.loader.stable)
        .or_else(|| loaders.first())
        .map(|info| info.loader.clone())
        .ok_or_else(|| Error::FabricLoaderNotFound {
            game: game_version.to_owned(),
            loader: "latest stable".to_owned(),
        })
}

/// Resolve a specific loader version for `game_version`.
///
/// # Errors
///
/// Fails on network errors, invalid JSON, an unsupported game version, or an
/// unknown loader version.
pub async fn resolve_loader(
    client: &reqwest::Client,
    game_version: &str,
    loader_version: &str,
) -> Result<FabricLoaderVersion> {
    resolve_loader_with_base(client, FABRIC_META_URL, game_version, loader_version).await
}

/// [`resolve_loader`] against a custom meta base URL (tests, mirrors).
///
/// # Errors
///
/// Same as [`resolve_loader`].
pub async fn resolve_loader_with_base(
    client: &reqwest::Client,
    base: &str,
    game_version: &str,
    loader_version: &str,
) -> Result<FabricLoaderVersion> {
    let loaders = list_loaders_with_base(client, base, game_version).await?;
    loaders
        .into_iter()
        .find(|info| info.loader.version == loader_version)
        .map(|info| info.loader)
        .ok_or_else(|| Error::FabricLoaderNotFound {
            game: game_version.to_owned(),
            loader: loader_version.to_owned(),
        })
}

/// Fetch the launcher profile for a game + loader combination.
///
/// # Errors
///
/// Fails on network errors, invalid JSON, or an unknown combination.
pub async fn fetch_profile(
    client: &reqwest::Client,
    game_version: &str,
    loader_version: &str,
) -> Result<FabricProfile> {
    fetch_profile_with_base(client, FABRIC_META_URL, game_version, loader_version).await
}

/// [`fetch_profile`] against a custom meta base URL (tests, mirrors).
///
/// # Errors
///
/// Same as [`fetch_profile`].
pub async fn fetch_profile_with_base(
    client: &reqwest::Client,
    base: &str,
    game_version: &str,
    loader_version: &str,
) -> Result<FabricProfile> {
    get_json(
        client,
        &format!(
            "{base}/versions/loader/{}/{}/profile/json",
            url_path_segment(game_version),
            url_path_segment(loader_version)
        ),
        Error::FabricLoaderNotFound {
            game: game_version.to_owned(),
            loader: loader_version.to_owned(),
        },
    )
    .await
}

/// Fetch the launcher profile, caching it at
/// `cache/loaders/fabric/<game>-<loader>.json`. A cached profile is reused
/// without a network request; pass `force` to always re-fetch. Cache writes
/// are best-effort — a read-only or full cache directory must not fail an
/// otherwise successful fetch.
///
/// # Errors
///
/// Fails on network errors, invalid JSON, or an unknown combination.
pub async fn load_profile(
    dirs: &Directories,
    client: &reqwest::Client,
    game_version: &str,
    loader_version: &str,
    force: bool,
) -> Result<FabricProfile> {
    load_profile_with_base(dirs, client, FABRIC_META_URL, game_version, loader_version, force).await
}

/// [`load_profile`] against a custom meta base URL (tests, mirrors).
///
/// # Errors
///
/// Same as [`load_profile`].
pub async fn load_profile_with_base(
    dirs: &Directories,
    client: &reqwest::Client,
    base: &str,
    game_version: &str,
    loader_version: &str,
    force: bool,
) -> Result<FabricProfile> {
    let cache = dirs
        .cache_dir()
        .join("loaders")
        .join("fabric")
        .join(format!("{game_version}-{loader_version}.json"));
    if !force
        && let Ok(bytes) = tokio::fs::read(&cache).await
        && let Ok(profile) = serde_json::from_slice(&bytes)
    {
        return Ok(profile);
    }
    let profile = fetch_profile_with_base(client, base, game_version, loader_version).await?;
    if let Some(parent) = cache.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    tokio::fs::write(&cache, serde_json::to_vec_pretty(&profile)?).await.ok();
    Ok(profile)
}

/// Merge a game version JSON with a loader profile into a single version
/// JSON ready to install and launch: the profile's id and main class win,
/// its libraries and arguments are appended to the game's, and everything
/// else (client jar, assets, Java version, logging) is inherited from the
/// game version.
///
/// A profile library that shares its `group:artifact` with a game library
/// replaces it (the loader pins its own versions — e.g. ASM — which the
/// game JSON also ships; keeping both would put duplicate classes on the
/// classpath and abort Fabric's classpath verification).
#[must_use]
pub fn merge(game: &VersionJson, profile: &FabricProfile) -> VersionJson {
    let mut arguments = Arguments {
        game: Vec::new(),
        jvm: Vec::new(),
    };
    let mut has_arguments = false;
    if let Some(game_args) = &game.arguments {
        arguments.game.extend(game_args.game.iter().cloned());
        arguments.jvm.extend(game_args.jvm.iter().cloned());
        has_arguments = true;
    }
    if let Some(profile_args) = &profile.arguments {
        arguments.game.extend(profile_args.game.iter().cloned());
        arguments.jvm.extend(profile_args.jvm.iter().cloned());
        has_arguments = true;
    }
    let mut libraries: Vec<Library> = game
        .libraries
        .iter()
        .filter(|game_lib| {
            let game_key = maven_artifact_key(&game_lib.name);
            !profile
                .libraries
                .iter()
                .any(|profile_lib| maven_artifact_key(&profile_lib.name) == game_key)
        })
        .cloned()
        .collect();
    libraries.extend(profile.libraries.iter().cloned());
    VersionJson {
        id: profile.id.clone(),
        kind: if profile.kind.is_empty() {
            game.kind.clone()
        } else {
            profile.kind.clone()
        },
        main_class: Some(profile.main_class.clone()),
        // A legacy game version keeps its `minecraftArguments` template when
        // neither document carries a modern arguments block; an empty
        // `Some` block would shadow it in argument resolution.
        arguments: has_arguments.then_some(arguments),
        minecraft_arguments: game.minecraft_arguments.clone(),
        asset_index: game.asset_index.clone(),
        assets: game.assets.clone(),
        java_version: game.java_version.clone(),
        downloads: game.downloads.clone(),
        libraries,
        logging: game.logging.clone(),
        minimum_launcher_version: game.minimum_launcher_version,
        time: profile.time.clone(),
        release_time: profile.release_time.clone(),
    }
}

/// The `group:artifact` portion of a maven coordinate (`org.ow2.asm:asm:9.6`
/// → `org.ow2.asm:asm`), used to detect overlapping libraries when merging.
fn maven_artifact_key(name: &str) -> String {
    name.split(':').take(2).collect::<Vec<_>>().join(":")
}

/// Install the merged loader version: client jar, game libraries plus the
/// loader's own libraries, natives, assets and logging config — everything
/// `crate::launch::install` normally handles, with the loader profile baked
/// in.
///
/// # Errors
///
/// Fails on network errors, checksum mismatches, or malformed archives.
pub async fn install(
    dirs: &Directories,
    client: &reqwest::Client,
    game: &VersionJson,
    profile: &FabricProfile,
    game_dir: &Path,
    progress: Option<crate::assets::ProgressFn>,
) -> Result<InstalledVersion> {
    let merged = merge(game, profile);
    crate::launch::install(dirs, client, &merged, game_dir, progress).await
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::thread;

    use sha1::Digest as _;

    use super::*;
    use crate::version_json::{Argument, ArtifactDownload, Downloads, JavaVersion};

    fn tempdir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mc-launcher-fabric-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// Serve `files` (GET path -> (status, body)) until the process exits.
    fn serve(files: BTreeMap<String, (u16, Vec<u8>)>) -> String {
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
                    let (status, body) = files.get(&path).cloned().unwrap_or((404, Vec::new()));
                    let reason = if status == 200 { "OK" } else { "Not Found" };
                    let response = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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

    fn json(body: &str) -> (u16, Vec<u8>) {
        (200, body.as_bytes().to_vec())
    }

    const GAME_LIST: &str = r#"[
        {"version": "1.21.4", "stable": true},
        {"version": "26.3-snapshot-1", "stable": false}
    ]"#;

    const LOADER_LIST: &str = r#"[
        {"loader": {"separator": ".", "build": 3, "maven": "net.fabricmc:fabric-loader:0.19.3",
                    "version": "0.19.3", "stable": true},
         "intermediary": {"maven": "net.fabricmc:intermediary:1.21.4", "version": "1.21.4", "stable": true}},
        {"loader": {"separator": ".", "build": 2, "maven": "net.fabricmc:fabric-loader:0.19.2",
                    "version": "0.19.2", "stable": false},
         "intermediary": {"maven": "net.fabricmc:intermediary:1.21.4", "version": "1.21.4", "stable": true}}
    ]"#;

    const PROFILE_JSON: &str = r#"{
        "id": "fabric-loader-0.16.10-1.21.4",
        "inheritsFrom": "1.21.4",
        "releaseTime": "2026-08-11T01:16:22+0000",
        "time": "2026-08-11T01:16:22+0000",
        "type": "release",
        "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
        "arguments": {
            "game": [],
            "jvm": ["-DFabricMcEmu= net.minecraft.client.main.Main "]
        },
        "libraries": [
            {"name": "org.ow2.asm:asm:9.7.1", "url": "https://maven.fabricmc.net/",
             "sha1": "f0ed132a49244b042cd0e15702ab9f2ce3cc8436", "size": 126093},
            {"name": "net.fabricmc:fabric-loader:0.16.10", "url": "https://maven.fabricmc.net/"}
        ]
    }"#;

    #[tokio::test]
    async fn lists_games() {
        let url = serve(BTreeMap::from([(
            "/versions/game".to_owned(),
            json(GAME_LIST),
        )]));
        let client = reqwest::Client::new();
        let games = list_games_with_base(&client, &url).await.expect("games");
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].version, "1.21.4");
        assert!(games[0].stable);
        assert!(!games[1].stable);
    }

    #[tokio::test]
    async fn lists_loaders_and_picks_latest_stable() {
        let url = serve(BTreeMap::from([(
            "/versions/loader/1.21.4".to_owned(),
            json(LOADER_LIST),
        )]));
        let client = reqwest::Client::new();
        let loaders = list_loaders_with_base(&client, &url, "1.21.4")
            .await
            .expect("loaders");
        assert_eq!(loaders.len(), 2);
        assert_eq!(loaders[0].loader.version, "0.19.3");
        assert_eq!(loaders[0].intermediary.version, "1.21.4");
        let latest = latest_loader_with_base(&client, &url, "1.21.4")
            .await
            .expect("latest");
        assert_eq!(latest.version, "0.19.3");
        assert!(latest.stable);
    }

    #[tokio::test]
    async fn resolves_exact_loader_version() {
        let url = serve(BTreeMap::from([(
            "/versions/loader/1.21.4".to_owned(),
            json(LOADER_LIST),
        )]));
        let client = reqwest::Client::new();
        let loader = resolve_loader_with_base(&client, &url, "1.21.4", "0.19.2")
            .await
            .expect("resolve");
        assert_eq!(loader.version, "0.19.2");
        let err = resolve_loader_with_base(&client, &url, "1.21.4", "9.9.9")
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::FabricLoaderNotFound { game, loader } if game == "1.21.4" && loader == "9.9.9"
        ));
    }

    #[tokio::test]
    async fn maps_404_to_game_not_found() {
        let url = serve(BTreeMap::from([(
            "/versions/loader/99.99".to_owned(),
            (404, Vec::new()),
        )]));
        let err = list_loaders_with_base(&reqwest::Client::new(), &url, "99.99")
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::FabricGameNotFound(game) if game == "99.99"
        ));
    }

    #[tokio::test]
    async fn fetches_and_parses_profile() {
        let url = serve(BTreeMap::from([(
            "/versions/loader/1.21.4/0.16.10/profile/json".to_owned(),
            json(PROFILE_JSON),
        )]));
        let profile = fetch_profile_with_base(&reqwest::Client::new(), &url, "1.21.4", "0.16.10")
            .await
            .expect("profile");
        assert_eq!(profile.id, "fabric-loader-0.16.10-1.21.4");
        assert_eq!(profile.inherits_from, "1.21.4");
        assert_eq!(profile.main_class, "net.fabricmc.loader.impl.launch.knot.KnotClient");
        assert_eq!(profile.libraries.len(), 2);
        // Legacy libraries carry a maven `url` (+ optional sha1/size).
        assert_eq!(profile.libraries[0].url.as_deref(), Some("https://maven.fabricmc.net/"));
        assert_eq!(profile.libraries[0].sha1.as_deref(), Some("f0ed132a49244b042cd0e15702ab9f2ce3cc8436"));
        assert_eq!(profile.libraries[1].size, None);
        let jvm: Vec<String> = profile
            .arguments
            .as_ref()
            .expect("arguments")
            .jvm
            .iter()
            .filter_map(|a| match a {
                Argument::Plain(s) => Some(s.clone()),
                Argument::Ruled(_) => None,
            })
            .collect();
        assert_eq!(jvm, vec!["-DFabricMcEmu= net.minecraft.client.main.Main "]);
    }

    #[tokio::test]
    async fn load_profile_caches() {
        let url = serve(BTreeMap::from([(
            "/versions/loader/1.21.4/0.16.10/profile/json".to_owned(),
            json(PROFILE_JSON),
        )]));
        let dirs = Directories::new(tempdir());
        let client = reqwest::Client::new();
        let profile = load_profile_with_base(&dirs, &client, &url, "1.21.4", "0.16.10", false)
            .await
            .expect("profile");
        assert_eq!(profile.id, "fabric-loader-0.16.10-1.21.4");
        let cache = dirs
            .cache_dir()
            .join("loaders/fabric/1.21.4-0.16.10.json");
        assert!(cache.is_file());
        // The cache round-trips.
        let cached: FabricProfile =
            serde_json::from_slice(&std::fs::read(&cache).expect("read cache")).expect("parse");
        assert_eq!(cached.id, profile.id);
    }

    /// A minimal modern game version with a client jar and one library.
    fn game_version(base: &str) -> VersionJson {
        VersionJson {
            id: "1.21.4".to_owned(),
            kind: "release".to_owned(),
            main_class: Some("net.minecraft.client.main.Main".to_owned()),
            arguments: Some(Arguments {
                game: vec![
                    crate::version_json::Argument::Plain("--username".to_owned()),
                    crate::version_json::Argument::Plain("${auth_player_name}".to_owned()),
                ],
                jvm: vec![crate::version_json::Argument::Plain("-cp ${classpath}".to_owned())],
            }),
            minecraft_arguments: None,
            asset_index: None,
            assets: Some("25".to_owned()),
            java_version: Some(JavaVersion {
                component: "java-runtime-delta".to_owned(),
                major_version: 21,
            }),
            downloads: Downloads {
                client: Some(ArtifactDownload {
                    sha1: "client-sha1".to_owned(),
                    size: 9,
                    url: format!("{base}/client.jar"),
                }),
                client_mappings: None,
                server: None,
                server_mappings: None,
            },
            libraries: vec![Library {
                name: "org.example:game-lib:1.0".to_owned(),
                url: None,
                sha1: None,
                size: None,
                downloads: Some(crate::version_json::LibraryDownloads {
                    artifact: Some(ArtifactDownload {
                        sha1: "lib-sha1".to_owned(),
                        size: 8,
                        url: format!("{base}/game-lib.jar"),
                    }),
                    classifiers: None,
                }),
                rules: None,
                natives: None,
                extract: None,
            }],
            logging: None,
            minimum_launcher_version: Some(21),
            time: "2024-12-03T12:35:58+00:00".to_owned(),
            release_time: "2024-12-03T09:23:39+00:00".to_owned(),
        }
    }

    #[test]
    fn merge_combines_profile_and_game() {
        let game = game_version("http://unused");
        let profile: FabricProfile =
            serde_json::from_str(PROFILE_JSON).expect("parse profile");
        let merged = merge(&game, &profile);

        assert_eq!(merged.id, "fabric-loader-0.16.10-1.21.4");
        assert_eq!(
            merged.main_class.as_deref(),
            Some("net.fabricmc.loader.impl.launch.knot.KnotClient")
        );
        // Game libraries first, then the loader's.
        assert_eq!(merged.libraries.len(), 3);
        assert_eq!(merged.libraries[0].name, "org.example:game-lib:1.0");
        assert_eq!(merged.libraries[2].name, "net.fabricmc:fabric-loader:0.16.10");
        // Game args first, then the profile's JVM emulation arg.
        let jvm: Vec<_> = merged
            .arguments
            .as_ref()
            .expect("arguments")
            .jvm
            .iter()
            .filter_map(|a| match a {
                crate::version_json::Argument::Plain(s) => Some(s.clone()),
                crate::version_json::Argument::Ruled(_) => None,
            })
            .collect();
        assert_eq!(
            jvm,
            vec![
                "-cp ${classpath}".to_owned(),
                "-DFabricMcEmu= net.minecraft.client.main.Main ".to_owned()
            ]
        );
        // Game-owned fields pass through.
        assert_eq!(merged.java_version.as_ref().expect("java").major_version, 21);
        assert_eq!(merged.assets.as_deref(), Some("25"));
        assert_eq!(merged.kind, "release");
    }

    #[test]
    fn merge_drops_game_libraries_the_profile_pins() {
        // The game JSON ships asm 9.6; the loader pins its own asm version.
        // Keeping both on the classpath trips Fabric's duplicate-ASM
        // classpath check, so the game's copy must be dropped.
        let mut game = game_version("http://unused");
        game.libraries.push(Library {
            name: "org.ow2.asm:asm:9.6".to_owned(),
            url: None,
            sha1: None,
            size: None,
            downloads: Some(crate::version_json::LibraryDownloads {
                artifact: Some(ArtifactDownload {
                    sha1: "asm-sha1".to_owned(),
                    size: 10,
                    url: "https://libraries.minecraft.net/org/ow2/asm/asm/9.6/asm-9.6.jar".to_owned(),
                }),
                classifiers: None,
            }),
            rules: None,
            natives: None,
            extract: None,
        });
        let mut profile: FabricProfile =
            serde_json::from_str(PROFILE_JSON).expect("parse profile");
        profile.libraries[0].name = "org.ow2.asm:asm:9.10.1".to_owned();
        let merged = merge(&game, &profile);

        let names: Vec<_> = merged.libraries.iter().map(|l| l.name.as_str()).collect();
        assert!(!names.contains(&"org.ow2.asm:asm:9.6"));
        assert!(names.contains(&"org.ow2.asm:asm:9.10.1"));
        // The rest of the game libraries survive.
        assert!(names.contains(&"org.example:game-lib:1.0"));
        assert!(names.contains(&"net.fabricmc:fabric-loader:0.16.10"));
    }

    #[test]
    fn merge_keeps_legacy_arguments_template_when_no_modern_block_exists() {
        // A legacy game version carries `minecraftArguments` instead of a
        // modern `arguments` block. The merge must not produce an empty
        // `Some(arguments)` that shadows the template in argument
        // resolution.
        let mut game = game_version("http://unused");
        game.arguments = None;
        game.minecraft_arguments = Some(
            "--username ${auth_player_name} --gameDir ${game_directory}".to_owned(),
        );
        let mut profile: FabricProfile =
            serde_json::from_str(PROFILE_JSON).expect("parse profile");
        profile.arguments = None;

        let merged = merge(&game, &profile);
        assert!(merged.arguments.is_none());
        assert_eq!(
            merged.minecraft_arguments.as_deref(),
            Some("--username ${auth_player_name} --gameDir ${game_directory}")
        );
    }

    #[tokio::test]
    async fn installs_merged_artifacts() {
        let client_jar = b"client jar".to_vec();
        let game_lib = b"game library".to_vec();
        let fabric_lib = b"fabric loader library".to_vec();
        let base = serve(BTreeMap::from([
            ("/client.jar".to_owned(), (200, client_jar.clone())),
            ("/game-lib.jar".to_owned(), (200, game_lib.clone())),
            (
                "/net/fabricmc/fabric-loader/0.16.10/fabric-loader-0.16.10.jar".to_owned(),
                (200, fabric_lib.clone()),
            ),
        ]));
        let mut game = game_version(&base);
        game.downloads.client = Some(ArtifactDownload {
            sha1: sha1_hex(&client_jar),
            size: client_jar.len() as u64,
            url: format!("{base}/client.jar"),
        });
        game.libraries[0].downloads = Some(crate::version_json::LibraryDownloads {
            artifact: Some(ArtifactDownload {
                sha1: sha1_hex(&game_lib),
                size: game_lib.len() as u64,
                url: format!("{base}/game-lib.jar"),
            }),
            classifiers: None,
        });
        // The loader library lives on the same server (its `url` is the base).
        // The asm entry carries a real-world sha1 that our fake body cannot
        // match, so it is dropped: only the unverified fabric-loader lib stays.
        let mut profile: FabricProfile = serde_json::from_str(PROFILE_JSON).expect("profile");
        profile.libraries.remove(0);
        profile.libraries[0].url = Some(base.clone());

        let dirs = Directories::new(tempdir());
        let game_dir = tempdir();
        let installed = install(&dirs, &reqwest::Client::new(), &game, &profile, &game_dir, None)
            .await
            .expect("install");

        assert_eq!(
            std::fs::read(&installed.client_jar).expect("read client"),
            client_jar
        );
        assert_eq!(installed.libraries.len(), 2);
        assert_eq!(
            std::fs::read(&installed.libraries[0]).expect("read game lib"),
            game_lib
        );
        assert_eq!(
            std::fs::read(&installed.libraries[1]).expect("read fabric lib"),
            fabric_lib
        );
        // The merged version installs under its own version dir.
        assert!(installed
            .client_jar
            .to_string_lossy()
            .contains("fabric-loader-0.16.10-1.21.4"));
    }

    #[test]
    fn url_path_segment_encodes_special_characters() {
        assert_eq!(url_path_segment("1.21.4"), "1.21.4");
        assert_eq!(url_path_segment("1.14 Pre-Release 5"), "1.14%20Pre-Release%205");
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
}
