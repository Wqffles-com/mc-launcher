//! Mojang version manifest: fetch, cache and query.
//!
//! Source: `https://piston-meta.mojang.com/mc/game/version_manifest_v2.json`

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Mojang's version manifest endpoint.
pub const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

/// How long a cached manifest is considered fresh before re-fetching.
pub const MANIFEST_CACHE_TTL: Duration = Duration::from_hours(6);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionManifest {
    pub latest: Latest,
    #[serde(default)]
    pub versions: Vec<VersionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Latest {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionInfo {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    pub time: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
    #[serde(default)]
    pub sha1: Option<String>,
}

impl VersionManifest {
    /// Look up a version by id.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&VersionInfo> {
        self.versions.iter().find(|v| v.id == id)
    }

    /// All versions whose `type` matches `kind` (`release`, `snapshot`,
    /// `old_alpha`, ...).
    pub fn of_kind<'a>(&'a self, kind: &str) -> impl Iterator<Item = &'a VersionInfo> {
        self.versions.iter().filter(move |v| v.kind == kind)
    }
}

/// Fetch and parse the manifest from the network.
///
/// # Errors
///
/// Fails on network errors, non-2xx responses, or invalid JSON in the body.
pub async fn fetch(client: &reqwest::Client, url: &str) -> Result<VersionManifest> {
    let bytes = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Load the manifest, using the cache at `cache_file` when it exists and is
/// fresh (younger than `ttl`). Pass `force = true` to always re-fetch.
///
/// A cache file that exists but cannot be read or parsed is treated as
/// missing and re-fetched from the network.
///
/// # Errors
///
/// Fails if the network fetch fails or the fetched body is invalid JSON.
pub async fn load(
    client: &reqwest::Client,
    url: &str,
    cache_file: &Path,
    ttl: Duration,
    force: bool,
) -> Result<VersionManifest> {
    if !force
        && is_fresh(cache_file, ttl).await
        && let Ok(bytes) = tokio::fs::read(cache_file).await
        && let Ok(manifest) = serde_json::from_slice(&bytes)
    {
        return Ok(manifest);
    }

    let manifest = fetch(client, url).await?;
    write_cache(cache_file, &manifest).await?;
    Ok(manifest)
}

async fn is_fresh(path: &Path, ttl: Duration) -> bool {
    let Ok(meta) = tokio::fs::metadata(path).await else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    modified.elapsed().is_ok_and(|age| age < ttl)
}

async fn write_cache(path: &Path, manifest: &VersionManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(manifest)?;
    tokio::fs::write(&tmp, &bytes).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration, SystemTime};

    use super::*;

    const MANIFEST_JSON: &str = r#"{
        "latest": { "release": "1.21.4", "snapshot": "25w31a" },
        "versions": [
            {
                "id": "1.21.4",
                "type": "release",
                "url": "https://piston-meta.mojang.com/v1/packages/abc/1.21.4.json",
                "time": "2024-12-03T12:35:58+00:00",
                "releaseTime": "2024-12-03T09:23:39+00:00",
                "sha1": "abc123"
            },
            {
                "id": "25w31a",
                "type": "snapshot",
                "url": "https://piston-meta.mojang.com/v1/packages/def/25w31a.json",
                "time": "2025-07-30T15:12:00+00:00",
                "releaseTime": "2025-07-30T15:12:00+00:00",
                "sha1": "def456"
            }
        ]
    }"#;

    /// Serve one HTTP response with the given status line and body, then exit.
    fn serve_once(status: &'static str, body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
        thread::spawn(move || {
            let mut stream = listener.accept().expect("accept").0;
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn fetch_parses_manifest() {
        let url = serve_once("200 OK", MANIFEST_JSON.to_owned());
        let client = reqwest::Client::new();
        let manifest = fetch(&client, &url).await.expect("fetch");

        assert_eq!(manifest.latest.release, "1.21.4");
        assert_eq!(manifest.latest.snapshot, "25w31a");
        assert_eq!(manifest.versions.len(), 2);
        assert_eq!(manifest.find("1.21.4").expect("find").kind, "release");
        assert_eq!(manifest.find("25w31a").expect("find").kind, "snapshot");
        assert_eq!(manifest.of_kind("release").count(), 1);
        assert!(manifest.find("1.8.9").is_none());
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_json() {
        let url = serve_once("200 OK", "not json".to_owned());
        let client = reqwest::Client::new();
        assert!(fetch(&client, &url).await.is_err());
    }

    #[tokio::test]
    async fn fetch_rejects_http_errors() {
        let url = serve_once("404 Not Found", String::new());
        let client = reqwest::Client::new();
        assert!(fetch(&client, &url).await.is_err());
    }

    #[tokio::test]
    async fn load_uses_fresh_cache_without_network() {
        // Bogus URL: if the code hits the network the test fails.
        let bad_url = "http://127.0.0.1:1/";
        let dir = tempdir();
        let cache = dir.join("manifest.json");
        tokio::fs::write(&cache, MANIFEST_JSON)
            .await
            .expect("write cache");

        let client = reqwest::Client::new();
        let manifest = load(&client, bad_url, &cache, Duration::from_hours(1), false)
            .await
            .expect("load from cache");

        assert_eq!(manifest.latest.release, "1.21.4");
        assert!(cache.exists());
    }

    #[tokio::test]
    async fn load_refetches_when_cache_is_corrupt() {
        let url = serve_once("200 OK", MANIFEST_JSON.to_owned());
        let dir = tempdir();
        let cache = dir.join("manifest.json");
        // Fresh mtime, but garbage content — must fall back to the network.
        tokio::fs::write(&cache, "this is not json{{")
            .await
            .expect("write corrupt cache");

        let client = reqwest::Client::new();
        let manifest = load(&client, &url, &cache, Duration::from_hours(1), false)
            .await
            .expect("refetch after corrupt cache");

        assert_eq!(manifest.latest.release, "1.21.4");
        assert_eq!(manifest.versions.len(), 2);
        // Cache was rewritten with valid content.
        let cached: VersionManifest =
            serde_json::from_slice(&tokio::fs::read(&cache).await.expect("read cache"))
                .expect("parse rewritten cache");
        assert_eq!(cached.latest.release, "1.21.4");
    }

    #[tokio::test]
    async fn load_refetches_when_stale() {
        let url = serve_once("200 OK", MANIFEST_JSON.to_owned());
        let dir = tempdir();
        let cache = dir.join("manifest.json");
        tokio::fs::write(
            &cache,
            "{\"latest\":{\"release\":\"old\",\"snapshot\":\"old\"},\"versions\":[]}",
        )
        .await
        .expect("write stale cache");
        // Make the cache look ancient.
        let past =
            filetime::FileTime::from_system_time(SystemTime::now() - Duration::from_hours(24));
        let _ = filetime::set_file_mtime(&cache, past);

        let client = reqwest::Client::new();
        let manifest = load(&client, &url, &cache, Duration::from_hours(1), false)
            .await
            .expect("refetch");

        assert_eq!(manifest.latest.release, "1.21.4");
        assert_eq!(manifest.versions.len(), 2);
        // Cache was rewritten with fresh content.
        let cached: VersionManifest =
            serde_json::from_slice(&tokio::fs::read(&cache).await.expect("read cache"))
                .expect("parse cache");
        assert_eq!(cached.latest.release, "1.21.4");
    }

    #[tokio::test]
    async fn load_force_refetches_fresh_cache() {
        let url = serve_once("200 OK", MANIFEST_JSON.to_owned());
        let dir = tempdir();
        let cache = dir.join("manifest.json");
        tokio::fs::write(
            &cache,
            "{\"latest\":{\"release\":\"old\",\"snapshot\":\"old\"},\"versions\":[]}",
        )
        .await
        .expect("write cache");

        let client = reqwest::Client::new();
        let manifest = load(&client, &url, &cache, Duration::from_hours(1), true)
            .await
            .expect("force refetch");

        assert_eq!(manifest.latest.release, "1.21.4");
    }

    fn tempdir() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mc-launcher-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }
}
