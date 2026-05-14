use serde::Serialize;
use std::io;
use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Provided JSON data is invalid format.
    #[error("Failed to parse JSON data: {0}")]
    ParseJson(#[from] serde_json::Error),
    /// Directory or file is not exists.
    #[error("{0} is not exists.")]
    NotExists(String),
    /// Error occurred while I/O operation.
    #[error("I/O error: {0}")]
    IO(#[from] io::Error),
    #[error("Unexpected or malformed data: {0}")]
    Invalid(String),
    /// Failure to connect, resolve DNS, or request timeout.
    #[error("Failed to fetch resource: {0}")]
    NetworkError(#[from] reqwest::Error),
    /// HTTP Request rejected by server.
    #[error("Server rejected request: {0} {1}")]
    HttpRequestRejected(u16, String),
    /// Server returned invalid response.
    #[error("Unexpected or malformed data: {0}")]
    InvalidResponse(reqwest::Error),
    /// Checksum validation is failed.
    #[error("SHA1 mismatch for {resource}: expected {sha1}, got {hex}")]
    ChecksumMismatch {
        resource: String,
        sha1: String,
        hex: String,
    },
    /// Child process has exit with non-zero exit code.
    #[error("Child process failed: {0}")]
    ChildProcess(String),
}

impl Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum InitializationError {
    #[error("Failed to resolve path: {0}")]
    PathResolution(String),
    #[error("Application directory constant already initialized: {0}")]
    AlreadyInitialized(String),
    #[error("Failed to create application directory: {0}")]
    CreateDir(#[from] io::Error),
}

impl From<PathBuf> for InitializationError {
    fn from(p: PathBuf) -> Self {
        InitializationError::AlreadyInitialized(p.display().to_string())
    }
}
