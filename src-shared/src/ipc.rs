use serde::{Deserialize, Serialize};

/// One log line emitted by an instance launch.
///
/// Emitted via `app.emit("instance-log", LogLine { … })` from the backend's
/// `launch_instance` and consumed by `PlayPage` on the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    pub instance_id: String,
    pub line: String,
    pub done: bool,
    pub error: Option<String>,
}

/// Install pipeline progress for a single instance creation.
///
/// Emitted via `app.emit("instance-install-progress", InstallProgress { … })`
/// from `emit_progress` and consumed by the frontend's `ActivityDock`.
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
/// Emitted as `ms-login-prompt`.
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
/// Emitted as `ms-login-result`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MsLoginResult {
    pub kind: String,
    pub message: String,
    pub account: Option<crate::datatypes::AccountSummary>,
}