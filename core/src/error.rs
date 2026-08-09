use thiserror::Error;

/// Errors produced by the core library.
#[derive(Debug, Error)]
pub enum Error {
    #[error("network request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("failed to parse JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no user data directory found")]
    NoDataDir,
    #[error("version not found in manifest: {0}")]
    VersionNotFound(String),
}

pub type Result<T> = std::result::Result<T, Error>;
