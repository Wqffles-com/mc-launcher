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
    #[error("Minecraft version {0} is not supported by Fabric")]
    FabricGameNotFound(String),
    #[error("Fabric loader version {loader} is not available for Minecraft {game}")]
    FabricLoaderNotFound { game: String, loader: String },
    #[error("download of {url} failed SHA-1 verification (expected {expected}, got {actual})")]
    ChecksumMismatch {
        url: String,
        expected: String,
        actual: String,
    },
    #[error("download of {url} has an unexpected size")]
    DownloadSizeMismatch { url: String },
    #[error("no client jar download is available for version {0}")]
    NoClientJar(String),
    #[error(
        "no usable Java runtime found (install one with `mc-launcher java install <major>`, set JAVA_HOME, or pass --java)"
    )]
    JavaNotFound,
    #[error("java runtime error: {0}")]
    JavaRuntime(String),
    #[error("invalid maven coordinate: {0}")]
    InvalidMavenName(String),
    #[error("internal task failed: {0}")]
    Task(String),
    #[error("asset object hash is malformed: {0}")]
    InvalidAssetHash(String),
    #[error("unsafe zip entry path: {0}")]
    UnsafeZipPath(String),
    #[error("version {0} has no main class")]
    NoMainClass(String),
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("the authorization request was declined")]
    AuthDeclined,
    #[error("the device code expired before the user approved it")]
    AuthExpired,
    #[error("account not found: {0}")]
    AccountNotFound(String),
    #[error("the account's refresh token could not be recovered from secure storage")]
    RefreshTokenUnavailable,
    #[error("keyring error: {0}")]
    Keyring(String),
}

pub type Result<T> = std::result::Result<T, Error>;
