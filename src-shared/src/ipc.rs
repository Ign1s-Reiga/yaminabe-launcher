use serde::{Deserialize, Serialize};

/// Carries a [`LogLine`] from an instance launch.
pub const EVT_INSTANCE_LOG: &str = "instance-log";
/// Carries the id of an instance whose game process has started.
pub const EVT_PROCESS_STARTED: &str = "instance-process-started";
/// Carries an [`InstallProgress`].
pub const EVT_INSTALL_PROGRESS: &str = "instance-install-progress";
/// Carries an [`MsLoginPrompt`].
pub const EVT_MS_LOGIN_PROMPT: &str = "ms-login-prompt";
/// Carries an [`MsLoginResult`].
pub const EVT_MS_LOGIN_RESULT: &str = "ms-login-result";
/// Says a saved account changed, e.g. its player name, so the frontend
/// refetches the accounts. Carries the account's dashed UUID.
pub const EVT_ACCOUNTS_CHANGED: &str = "accounts-changed";
/// Carries an [`UpdateProgress`].
pub const EVT_UPDATE_PROGRESS: &str = "update-download-progress";

/// One log line emitted by an instance launch, as [`EVT_INSTANCE_LOG`], from
/// the backend's `launch_instance` and consumed by `PlayPage` on the frontend.
/// `line` can span several lines, e.g. a message and its stack trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    pub instance_id: String,
    pub line: String,
    /// Set when the game logged the line through log4j, rather than printing it
    /// or the launcher reporting progress.
    #[serde(default)]
    pub event: Option<LogEvent>,
    pub done: bool,
    pub error: Option<String>,
}

/// When, where and how severely the game logged a line, as log4j reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEvent {
    /// Milliseconds since the Unix epoch.
    pub timestamp: i64,
    pub thread: String,
    pub level: LogLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl LogLevel {
    pub const ALL: [LogLevel; 6] = [
        LogLevel::Trace, LogLevel::Debug, LogLevel::Info, LogLevel::Warn, LogLevel::Error, LogLevel::Fatal,
    ];

    /// The level's name as log4j writes it, e.g. `WARN`.
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Fatal => "FATAL",
        }
    }

    /// Read a level as log4j writes it, ignoring case.
    pub fn parse(name: &str) -> Option<LogLevel> {
        LogLevel::ALL.into_iter().find(|level| level.as_str().eq_ignore_ascii_case(name))
    }
}

/// Install pipeline progress for a single instance creation.
///
/// Emitted as [`EVT_INSTALL_PROGRESS`] from `emit_progress` and consumed by
/// the frontend's `ActivityDock`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallProgress {
    pub id: String,
    pub name: String,
    pub step: String,
    pub done: bool,
    pub error: Option<String>,
}

/// Microsoft device-code grant prompt, emitted once the backend has registered
/// the device with Microsoft and is ready for the user to authenticate on
/// another device. `qr_svg` is a pre-rendered SVG of `verification_uri` so the
/// frontend can drop it straight into the DOM without a WASM QR dependency.
/// The MS v2.0 consumer endpoint does not return `verification_uri_complete`,
/// so the user opens `verification_uri` and types `user_code` to authenticate.
///
/// Emitted as [`EVT_MS_LOGIN_PROMPT`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MsLoginPrompt {
    pub verification_uri: String,
    pub user_code: String,
    pub qr_svg: String,
    pub expires_in: u32,
    pub interval: u32,
}

/// Terminal status of a Microsoft login attempt. `kind` is one of
/// `"success"`, `"error"`, `"cancelled"`, or `"expired"`; `account` is set only
/// on `success` and carries the newly persisted account's public summary.
///
/// Emitted as [`EVT_MS_LOGIN_RESULT`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MsLoginResult {
    pub kind: String,
    pub message: String,
    pub account: Option<crate::datamodels::AccountSummary>,
}
/// A launcher release offered as an update.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub version: String,
    /// The day it was published, as `YYYY-MM-DD`.
    pub date: Option<String>,
    /// Its section of `CHANGELOG.md`, as Markdown bullets.
    pub notes: String,
}

/// What a check for a newer launcher release found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum UpdateCheck {
    /// Nothing newer is published, or nothing yet with an updater manifest.
    UpToDate,
    Available(ReleaseInfo),
}

/// How far an update's download has got. `total` is absent when the server
/// sends no length.
///
/// Emitted as [`EVT_UPDATE_PROGRESS`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}
