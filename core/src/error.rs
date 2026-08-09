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
    #[error("instance not found: {0}")]
    InstanceNotFound(String),
    #[error("invalid instance name: {0}")]
    InvalidInstanceName(String),
    #[error("an instance named '{0}' already exists")]
    InstanceNameTaken(String),
    #[error("could not generate a unique instance id")]
    InstanceIdExhausted,
    #[error("invalid instance archive entry: {0}")]
    InvalidArchiveEntry(String),
    #[error("instance archive is missing {0}")]
    ArchiveMissingConfig(String),
    #[error("instance archive exceeds the import size limit")]
    ArchiveTooLarge,
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("unknown mod loader: {0}")]
    UnknownLoader(String),
}

pub type Result<T> = std::result::Result<T, Error>;
