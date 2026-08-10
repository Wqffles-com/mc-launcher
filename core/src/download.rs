//! File downloads with byte-level progress, HTTP range resume and SHA-1
//! verification.
//!
//! Downloads land in a `.part` file next to the destination and are renamed
//! into place only once fully verified, so interrupted downloads are never
//! confused with complete ones. A destination whose SHA-1 already matches is
//! left untouched (no request is made).

use std::path::{Path, PathBuf};

use reqwest::header::RANGE;
use sha1::{Digest, Sha1};
use tokio::io::AsyncWriteExt;

use crate::error::{Error, Result};

/// Events reported to a progress callback while downloading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// Byte-level progress for a single file.
    File { name: String, done: u64, total: u64 },
    /// A single file finished.
    FileDone { name: String, fresh: bool },
    /// One item of a batch (e.g. one library of N) finished.
    BatchDone {
        name: String,
        done: usize,
        total: usize,
    },
}

/// Whether a file was freshly downloaded or was already present and verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadResult {
    Fresh,
    Verified,
}

/// Download `url` into `dest`, optionally verifying the SHA-1 and expected
/// size on completion.
///
/// - Existing verified files are reused ([`DownloadResult::Verified`]).
/// - A partial file is resumed via an HTTP `Range` request when the server
///   honors it; servers that ignore the range (200 instead of 206) cause a
///   clean restart.
/// - On a SHA-1 mismatch the partial file is removed and an error is
///   returned; nothing is written to `dest`.
///
/// `expected` is `(sha1_hex, size)`. Size is advisory: when the server
/// reports a Content-Length different from it, the mismatch is an error.
///
/// # Errors
///
/// Fails on network errors, non-2xx responses, I/O failures, or a SHA-1
/// mismatch.
pub async fn fetch(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected: Option<(&str, u64)>,
    progress: Option<&(dyn Fn(Progress) + Send + Sync)>,
) -> Result<DownloadResult> {
    let name = dest
        .file_name()
        .map_or_else(|| url.to_owned(), |n| n.to_string_lossy().into_owned());
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if let Some((sha1, _)) = expected
        && sha1_file(dest).await == Some(sha1.to_owned())
    {
        if let Some(cb) = progress {
            cb(Progress::FileDone { name, fresh: true });
        }
        return Ok(DownloadResult::Verified);
    }

    let partial = partial_path(dest);
    let mut offset = file_len(&partial).await;
    loop {
        let mut request = client.get(url);
        if offset > 0 {
            request = request.header(RANGE, format!("bytes={offset}-"));
        }
        let response = request.send().await?;
        // 416 (range beyond end) means the partial is stale, e.g. the server
        // file shrank since it was written: drop it and restart from zero.
        if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            tokio::fs::remove_file(&partial).await.ok();
            offset = 0;
            continue;
        }
        let response = response.error_for_status()?;
        let append = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        if !append {
            // The server ignored our range: start over.
            offset = 0;
        }

        let total = response
            .content_length()
            .map_or(0, |len| len.saturating_add(offset));
        if expected.is_some_and(|(_, size)| total != 0 && total != size) {
            tokio::fs::remove_file(&partial).await.ok();
            return Err(Error::DownloadSizeMismatch {
                url: url.to_owned(),
            });
        }

        let mut hasher = Sha1::new();
        let mut file = if append {
            tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&partial)
                .await?
        } else {
            tokio::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&partial)
                .await?
        };
        if append && offset > 0 {
            hasher.update(tokio::fs::read(&partial).await?);
        }
        let mut written: u64 = 0;
        let mut last_percent = 0u32;
        let mut response = response;
        while let Some(chunk) = response.chunk().await? {
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            written += u64::try_from(chunk.len()).map_err(|_| Error::Io(size_overflow()))?;
            let done = offset.saturating_add(written);
            let percent = percent(done, total);
            if percent != last_percent {
                last_percent = percent;
                if let Some(cb) = progress {
                    cb(Progress::File {
                        name: name.clone(),
                        done,
                        total,
                    });
                }
            }
        }
        file.flush().await?;

        let actual = hex(&hasher.finalize());
        if let Some((expected_sha1, _)) = expected
            && actual != expected_sha1
        {
            tokio::fs::remove_file(&partial).await.ok();
            return Err(Error::ChecksumMismatch {
                url: url.to_owned(),
                expected: expected_sha1.to_owned(),
                actual,
            });
        }
        tokio::fs::rename(&partial, dest).await?;
        if let Some(cb) = progress {
            cb(Progress::FileDone { name, fresh: false });
        }
        return Ok(DownloadResult::Fresh);
    }
}

fn size_overflow() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, "file size overflow")
}

/// The `.part` path used while a download is in flight.
fn partial_path(dest: &Path) -> PathBuf {
    dest.with_extension("part")
}

/// SHA-1 hex digest of a file, or `None` when it cannot be read.
#[must_use]
pub async fn sha1_file(path: &Path) -> Option<String> {
    let bytes = tokio::fs::read(path).await.ok()?;
    Some(hex(&Sha1::digest(&bytes)))
}

fn hex(digest: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn percent(done: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    u32::try_from(done.saturating_mul(100) / total).unwrap_or(100)
}

async fn file_len(path: &Path) -> u64 {
    tokio::fs::metadata(path).await.map_or(0, |meta| meta.len())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    fn hex_of(bytes: &[u8]) -> String {
        hex(&Sha1::digest(bytes))
    }

    fn tempdir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mc-launcher-download-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// Serve `files` (GET path -> body) over HTTP. Honors `Range: bytes=N-`
    /// when `ranges` is true (206 + Content-Range), otherwise always 200.
    fn serve(files: BTreeMap<String, Vec<u8>>, ranges: bool) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
        thread::spawn(move || {
            for stream in listener.incoming() {
                let files = files.clone();
                let ranges = ranges;
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
                    let range = head.lines().find_map(|l| {
                        let lower = l.to_ascii_lowercase();
                        lower
                            .starts_with("range:")
                            .then(|| lower.split_whitespace().nth(1).unwrap_or("").to_owned())
                    });
                    let body = files.get(&path).cloned().unwrap_or_default();
                    let mut status = "200 OK";
                    let mut payload = body;
                    let mut extra = String::new();
                    if ranges
                        && let Some(r) = range
                        && let Some(start) = r
                            .trim_start_matches("bytes=")
                            .trim_end_matches('-')
                            .parse::<usize>()
                            .ok()
                    {
                        if start >= payload.len() {
                            let response = "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                            let _ = stream.write_all(response.as_bytes());
                            return;
                        }
                        extra = format!(
                            "Content-Range: bytes {}-{}/{}\r\n",
                            start,
                            payload.len() - 1,
                            payload.len()
                        );
                        payload = payload[start..].to_vec();
                        status = "206 Partial Content";
                    }
                    let response = format!(
                        "HTTP/1.1 {status}\r\n{extra}Content-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        payload.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.write_all(&payload);
                    let _ = stream.flush();
                });
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn downloads_fresh_file_and_verifies_sha1() {
        let body = b"hello world".to_vec();
        let url = serve(BTreeMap::from([("/f.bin".to_owned(), body.clone())]), false);
        let dir = tempdir();
        let dest = dir.join("f.bin");
        let result = fetch(
            &reqwest::Client::new(),
            &format!("{url}/f.bin"),
            &dest,
            Some((&hex_of(&body), body.len() as u64)),
            None,
        )
        .await
        .expect("download");
        assert_eq!(result, DownloadResult::Fresh);
        assert_eq!(std::fs::read(&dest).expect("read"), body);
        assert!(!dest.with_extension("part").exists());
    }

    #[tokio::test]
    async fn reuses_existing_verified_file_without_network() {
        // Bogus URL: a request would fail, so success proves no request.
        let dir = tempdir();
        let dest = dir.join("f.bin");
        let body = b"cached".to_vec();
        std::fs::write(&dest, &body).expect("write");
        let result = fetch(
            &reqwest::Client::new(),
            "http://127.0.0.1:1/f.bin",
            &dest,
            Some((&hex_of(&body), body.len() as u64)),
            None,
        )
        .await
        .expect("verified");
        assert_eq!(result, DownloadResult::Verified);
    }

    #[tokio::test]
    async fn redownloads_when_existing_file_is_stale() {
        let body = b"fresh data".to_vec();
        let url = serve(BTreeMap::from([("/f.bin".to_owned(), body.clone())]), false);
        let dir = tempdir();
        let dest = dir.join("f.bin");
        std::fs::write(&dest, b"old").expect("write stale");
        let result = fetch(
            &reqwest::Client::new(),
            &format!("{url}/f.bin"),
            &dest,
            Some((&hex_of(&body), body.len() as u64)),
            None,
        )
        .await
        .expect("redownload");
        assert_eq!(result, DownloadResult::Fresh);
        assert_eq!(std::fs::read(&dest).expect("read"), body);
    }

    #[tokio::test]
    async fn resumes_from_a_partial_file() {
        let body: Vec<u8> = (0..100_000)
            .map(|i| u8::try_from(i % 251).expect("in range"))
            .collect();
        let url = serve(
            BTreeMap::from([("/big.bin".to_owned(), body.clone())]),
            true,
        );
        let dir = tempdir();
        let dest = dir.join("big.bin");
        // Simulate an interrupted download: partial file with the first half.
        let first_half = &body[..body.len() / 2];
        std::fs::write(dest.with_extension("part"), first_half).expect("write partial");
        let result = fetch(
            &reqwest::Client::new(),
            &format!("{url}/big.bin"),
            &dest,
            Some((&hex_of(&body), body.len() as u64)),
            None,
        )
        .await
        .expect("resumed");
        assert_eq!(result, DownloadResult::Fresh);
        assert_eq!(std::fs::read(&dest).expect("read"), body);
    }

    #[tokio::test]
    async fn restarts_when_server_ignores_range() {
        let body: Vec<u8> = (0..10_000)
            .map(|i| u8::try_from(i % 251).expect("in range"))
            .collect();
        let url = serve(
            BTreeMap::from([("/big.bin".to_owned(), body.clone())]),
            false,
        );
        let dir = tempdir();
        let dest = dir.join("big.bin");
        std::fs::write(dest.with_extension("part"), &body[..100]).expect("write partial");
        let result = fetch(
            &reqwest::Client::new(),
            &format!("{url}/big.bin"),
            &dest,
            Some((&hex_of(&body), body.len() as u64)),
            None,
        )
        .await
        .expect("restart");
        assert_eq!(result, DownloadResult::Fresh);
        assert_eq!(std::fs::read(&dest).expect("read"), body);
    }

    #[tokio::test]
    async fn restarts_when_partial_is_larger_than_server_content() {
        let body: Vec<u8> = (0..10_000)
            .map(|i| u8::try_from(i % 251).expect("in range"))
            .collect();
        let url = serve(
            BTreeMap::from([("/big.bin".to_owned(), body.clone())]),
            true,
        );
        let dir = tempdir();
        let dest = dir.join("big.bin");
        // A stale partial larger than the current server file triggers 416 on
        // resume; the client must drop it and start over instead of wedging.
        let stale: Vec<u8> = (0..(body.len() + 100))
            .map(|i| u8::try_from(i % 251).expect("in range"))
            .collect();
        std::fs::write(dest.with_extension("part"), &stale).expect("write stale partial");
        let result = fetch(
            &reqwest::Client::new(),
            &format!("{url}/big.bin"),
            &dest,
            Some((&hex_of(&body), body.len() as u64)),
            None,
        )
        .await
        .expect("restart after 416");
        assert_eq!(result, DownloadResult::Fresh);
        assert_eq!(std::fs::read(&dest).expect("read"), body);
        assert!(!dest.with_extension("part").exists());
    }

    #[tokio::test]
    async fn rejects_checksum_mismatch_and_cleans_up() {
        let url = serve(
            BTreeMap::from([("/bad.bin".to_owned(), b"not the right bytes".to_vec())]),
            false,
        );
        let dir = tempdir();
        let dest = dir.join("bad.bin");
        let err = fetch(
            &reqwest::Client::new(),
            &format!("{url}/bad.bin"),
            &dest,
            Some((&hex_of(b"expected bytes"), 19)),
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::ChecksumMismatch { .. }));
        assert!(!dest.exists());
        assert!(!dest.with_extension("part").exists());
    }

    #[tokio::test]
    async fn rejects_size_mismatch() {
        let url = serve(
            BTreeMap::from([("/s.bin".to_owned(), b"123456789".to_vec())]),
            false,
        );
        let dir = tempdir();
        let dest = dir.join("s.bin");
        let err = fetch(
            &reqwest::Client::new(),
            &format!("{url}/s.bin"),
            &dest,
            Some((&hex_of(b"123456789"), 1_000)),
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::DownloadSizeMismatch { .. }));
        // The partial is removed so a later attempt can start fresh instead
        // of failing the size check again forever.
        assert!(!dest.exists());
        assert!(!dest.with_extension("part").exists());
    }

    #[tokio::test]
    async fn reports_byte_progress() {
        let body: Vec<u8> = vec![b'x'; 500];
        let url = serve(BTreeMap::from([("/p.bin".to_owned(), body.clone())]), false);
        let dir = tempdir();
        let dest = dir.join("p.bin");
        let events: std::sync::Mutex<Vec<Progress>> = std::sync::Mutex::new(Vec::new());
        let result = fetch(
            &reqwest::Client::new(),
            &format!("{url}/p.bin"),
            &dest,
            Some((&hex_of(&body), body.len() as u64)),
            Some(&|p| events.lock().expect("lock").push(p)),
        )
        .await
        .expect("download");
        assert_eq!(result, DownloadResult::Fresh);
        let events = events.into_inner().expect("unlock");
        let Progress::FileDone { fresh: false, .. } = events.last().expect("last event") else {
            panic!("expected FileDone last, got {:?}", events.last());
        };
        let in_flight: Vec<_> = events
            .iter()
            .filter_map(|p| match p {
                Progress::File { done, total, .. } => Some((*done, *total)),
                _ => None,
            })
            .collect();
        assert!(!in_flight.is_empty());
        assert_eq!(in_flight[0].1, 500);
        assert_eq!(in_flight.last().expect("last").0, 500);
    }
}
