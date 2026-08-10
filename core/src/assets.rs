//! Asset indexes: fetch, verify, download the object store and materialize
//! the layout the game expects.
//!
//! Modern (virtual) indexes are materialized under `assets/virtual/<id>/`,
//! which the game resolves against the shared assets root. Legacy
//! (non-virtual) indexes must sit inside the game directory's `assets/`
//! folder; the objects are copied there by the launch engine.
//!
//! Object downloads come from `https://resources.download.minecraft.net/<h0h1>/<hash>`
//! and are stored in a content-addressed store at `assets/objects/<h0h1>/<hash>`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::download;
use crate::download::Progress;
use crate::error::{Error, Result};

/// Host serving the content-addressed asset store.
pub const ASSETS_BASE_URL: &str = "https://resources.download.minecraft.net";

/// Maximum concurrent object downloads.
const OBJECT_CONCURRENCY: usize = 32;

/// Download attempts per object before giving up.
const OBJECT_ATTEMPTS: usize = 3;

/// A parsed asset index (`assets/indexes/<id>.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetIndex {
    /// Virtual indexes are served from `assets/virtual/<id>/`; legacy ones
    /// from the game directory's `assets/` folder.
    #[serde(rename = "virtual", default)]
    pub virtual_: bool,
    #[serde(default)]
    pub objects: BTreeMap<String, AssetObject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

/// Shared progress callback used by batch downloads.
pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

/// Parse an asset index document.
///
/// # Errors
///
/// Fails if the bytes are not valid JSON or do not match the schema.
pub fn parse(bytes: &[u8]) -> Result<AssetIndex> {
    Ok(serde_json::from_slice(bytes)?)
}

/// Fetch and verify an asset index, writing it to `dest` (`assets/indexes/<id>.json`).
///
/// # Errors
///
/// Fails on network errors, size/SHA-1 mismatches, or invalid JSON.
pub async fn fetch_index(
    client: &reqwest::Client,
    download: &crate::version_json::AssetIndex,
    dest: &Path,
) -> Result<AssetIndex> {
    let result = download::fetch(
        client,
        &download.url,
        dest,
        Some((&download.sha1, download.size)),
        None,
    )
    .await?;
    if result == download::DownloadResult::Verified {
        let bytes = tokio::fs::read(dest).await?;
        return parse(&bytes);
    }
    let bytes = tokio::fs::read(dest).await?;
    let index = parse(&bytes)?;
    Ok(index)
}

/// Download every object in `index` into `<objects_dir>/<h0h1>/<hash>`,
/// verifying each against its SHA-1. Already-present objects are skipped.
///
/// `base_url` overrides [`ASSETS_BASE_URL`] (used by tests); the object path
/// is appended to it.
///
/// # Errors
///
/// Fails on network errors, checksum mismatches, or I/O failures.
pub async fn download_objects(
    client: &reqwest::Client,
    index: &AssetIndex,
    objects_dir: &Path,
    base_url: &str,
    progress: Option<ProgressFn>,
) -> Result<()> {
    let semaphore = Arc::new(Semaphore::new(OBJECT_CONCURRENCY));
    let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // Duplicate hashes (same object under several keys) are fetched once.
    let mut seen = BTreeSet::new();
    let mut objects: Vec<(&String, &AssetObject)> = Vec::new();
    for (key, object) in &index.objects {
        if seen.insert(object.hash.clone()) {
            objects.push((key, object));
        }
    }
    let total = objects.len();
    let mut tasks = tokio::task::JoinSet::new();
    for (key, object) in objects {
        let dest = objects_dir.join(object_dir(&object.hash)?);
        tasks.spawn(download_object(
            client.clone(),
            base_url.to_owned(),
            object.clone(),
            dest,
            Arc::clone(&semaphore),
            Arc::clone(&done),
            total,
            key.clone(),
            progress.clone(),
        ));
    }
    while let Some(joined) = tasks.join_next().await {
        joined.map_err(|e| Error::Task(e.to_string()))??;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn download_object(
    client: reqwest::Client,
    base_url: String,
    object: AssetObject,
    dest: PathBuf,
    semaphore: Arc<Semaphore>,
    done: Arc<std::sync::atomic::AtomicUsize>,
    total: usize,
    key: String,
    progress: Option<ProgressFn>,
) -> Result<()> {
    let _permit = semaphore
        .acquire_owned()
        .await
        .map_err(|e| Error::Task(e.to_string()))?;
    let url = format!("{base_url}/{}/{}", &object.hash[..2], object.hash);
    let mut last_error = None;
    for attempt in 0..OBJECT_ATTEMPTS {
        let outcome = download::fetch(
            &client,
            &url,
            &dest,
            Some((&object.hash, object.size)),
            None,
        )
        .await;
        match outcome {
            Ok(_) => {
                let count = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if let Some(cb) = progress {
                    cb(Progress::BatchDone {
                        name: key,
                        done: count,
                        total,
                    });
                }
                return Ok(());
            }
            Err(err) => {
                last_error = Some(err);
                if attempt + 1 == OBJECT_ATTEMPTS {
                    return Err(last_error.expect("attempted"));
                }
                let backoff_ms = 250u64.saturating_mul(u64::try_from(attempt + 1).unwrap_or(1));
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }
        }
    }
    Err(last_error.unwrap_or_else(|| Error::Task("no download attempt made".to_owned())))
}

/// Materialize the game-facing layout of an index from the object store:
/// every object is linked (hard link, copying when unavailable) into
/// `target_dir` under its index key. Existing files are left untouched.
///
/// # Errors
///
/// Fails if an object is missing from the store or cannot be linked/copied.
pub async fn materialize(index: &AssetIndex, target_dir: &Path, objects_dir: &Path) -> Result<u64> {
    let mut count = 0u64;
    for (key, object) in &index.objects {
        let source = objects_dir.join(object_dir(&object.hash)?);
        let target = target_dir.join(key);
        if !target.exists() {
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            if tokio::fs::hard_link(&source, &target).await.is_err() {
                tokio::fs::copy(&source, &target).await?;
            }
        }
        count += 1;
    }
    Ok(count)
}

fn object_dir(hash: &str) -> Result<PathBuf> {
    if hash.len() < 2 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::InvalidAssetHash(hash.to_owned()));
    }
    Ok(PathBuf::from(&hash[..2]).join(hash))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;
    use sha1::Digest as _;

    const INDEX_JSON: &str = r#"{
        "virtual": true,
        "objects": {
            "icons/icon_16x16.png": {"hash": "0123456789abcdef0123456789abcdef01234567", "size": 11},
            "minecraft/lang/en_us.lang": {"hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size": 13}
        }
    }"#;

    fn tempdir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mc-launcher-assets-test-{}-{}",
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

    /// Serve an asset object store keyed by `<h0h1>/<hash>` path.
    fn serve_objects(objects: &[(&str, Vec<u8>)]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let files: BTreeMap<String, Vec<u8>> = objects
            .iter()
            .map(|(hash, body)| (format!("/{}/{hash}", &hash[..2]), body.clone()))
            .collect();
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

    #[test]
    fn parses_index() {
        let index = parse(INDEX_JSON.as_bytes()).expect("parse");
        assert!(index.virtual_);
        assert_eq!(index.objects.len(), 2);
        assert_eq!(index.objects["icons/icon_16x16.png"].size, 11);
    }

    #[tokio::test]
    async fn downloads_objects_into_store() {
        let body_a = b"hello world".to_vec();
        let body_b = b"gamemode=survival".to_vec();
        let hash_a = sha1_hex(&body_a);
        let hash_b = sha1_hex(&body_b);
        let url = serve_objects(&[(&hash_a, body_a.clone()), (&hash_b, body_b.clone())]);
        let index = AssetIndex {
            virtual_: true,
            objects: BTreeMap::from([
                (
                    "a.txt".to_owned(),
                    AssetObject {
                        hash: hash_a.clone(),
                        size: body_a.len() as u64,
                    },
                ),
                (
                    "b.txt".to_owned(),
                    AssetObject {
                        hash: hash_b.clone(),
                        size: body_b.len() as u64,
                    },
                ),
            ]),
        };
        let dir = tempdir();
        let objects = dir.join("objects");
        download_objects(&reqwest::Client::new(), &index, &objects, &url, None)
            .await
            .expect("download");
        assert_eq!(
            std::fs::read(objects.join(&hash_a[..2]).join(&hash_a)).expect("read a"),
            body_a
        );
        assert_eq!(
            std::fs::read(objects.join(&hash_b[..2]).join(&hash_b)).expect("read b"),
            body_b
        );
    }

    #[tokio::test]
    async fn materializes_virtual_layout() {
        let body_a = b"hello world".to_vec();
        let hash_a = sha1_hex(&body_a);
        let url = serve_objects(&[(&hash_a, body_a.clone())]);
        let index = AssetIndex {
            virtual_: true,
            objects: BTreeMap::from([(
                "icons/icon.png".to_owned(),
                AssetObject {
                    hash: hash_a.clone(),
                    size: body_a.len() as u64,
                },
            )]),
        };
        let dir = tempdir();
        let objects = dir.join("objects");
        download_objects(&reqwest::Client::new(), &index, &objects, &url, None)
            .await
            .expect("download");
        let target = dir.join("virtual");
        let count = materialize(&index, &target, &objects)
            .await
            .expect("materialize");
        assert_eq!(count, 1);
        assert_eq!(
            std::fs::read(target.join("icons/icon.png")).expect("read"),
            body_a
        );
        // Idempotent: second pass keeps the file.
        let count = materialize(&index, &target, &objects).await.expect("again");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn materialize_missing_object_fails() {
        let index = AssetIndex {
            virtual_: true,
            objects: BTreeMap::from([(
                "icons/icon.png".to_owned(),
                AssetObject {
                    hash: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_owned(),
                    size: 1,
                },
            )]),
        };
        let dir = tempdir();
        let err = materialize(&index, &dir.join("virtual"), &dir.join("objects"))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Io(_)));
    }

    #[test]
    fn rejects_malformed_hashes() {
        assert!(matches!(object_dir("a"), Err(Error::InvalidAssetHash(_))));
        assert!(matches!(
            object_dir("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
            Err(Error::InvalidAssetHash(_))
        ));
        assert!(object_dir("0123456789abcdef0123456789abcdef01234567").is_ok());
    }
}
