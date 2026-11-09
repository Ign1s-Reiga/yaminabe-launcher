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
    /// Server returned HTTP 429 — too many requests. Kept distinct from
    /// `HttpRequestRejected` so callers and the UI can message a rate limit
    /// specifically (e.g. "slow down / try again later").
    #[error("Rate limit exceeded for {0}")]
    RateLimitExceeded(String),
    /// Server returned invalid response.
    #[error("Malformed server response: {0}")]
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
    /// Unsupported operation.
    #[error("Unsupported: {0}")]
    Unsupported(String),
    /// Operation cannot proceed because the target is already doing exclusive work.
    #[error("Busy: {0}")]
    Busy(String),
    /// Authentication-flow failure (Microsoft / Xbox Live / Minecraft services).
    #[error("Authentication failed: {0}")]
    Auth(String),
    /// The Microsoft account signed in fine but does not own the game, which
    /// signing in again cannot change.
    #[error("This Microsoft account does not own Minecraft: Java Edition.")]
    GameNotOwned,
    /// Failure interacting with the OS credential store (keyring backend).
    /// Kept separate from `Auth` so a storage-layer problem doesn't get
    /// mislabelled as an authentication error.
    #[error("Credential storage failed: {0}")]
    Keyring(String),
    /// Checking for, downloading or installing a launcher update.
    #[error("Update failed: {0}")]
    Update(String),
}

impl Error {
    /// Whether the server could not be reached or could not answer, rather
    /// than refused. Such an error says nothing about the request, and a later
    /// try may work.
    pub fn is_transient(&self) -> bool {
        match self {
            Error::NetworkError(_) | Error::RateLimitExceeded(_) | Error::InvalidResponse(_) => true,
            Error::HttpRequestRejected(status, _) => *status >= 500 || *status == 429,
            _ => false,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn a_refusal_is_not_transient() {
        assert!(!Error::Auth("refresh_token rejected (invalid_grant)".into()).is_transient());
        assert!(!Error::HttpRequestRejected(400, String::new()).is_transient());
        assert!(!Error::HttpRequestRejected(401, String::new()).is_transient());
        assert!(!Error::GameNotOwned.is_transient());
        assert!(!Error::Keyring("read failed".into()).is_transient());
    }

    #[test]
    fn a_server_that_could_not_answer_is_transient() {
        assert!(Error::HttpRequestRejected(503, String::new()).is_transient());
        assert!(Error::HttpRequestRejected(429, String::new()).is_transient());
        assert!(Error::RateLimitExceeded("api.minecraftservices.com".into()).is_transient());
    }
}
