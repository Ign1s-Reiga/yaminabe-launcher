//! Tauri event payloads shared by backend emitter and frontend listener.
//!
//! These types cross the wasm boundary as JSON. Both crates must agree on
//! field names and types — keeping the single definition here makes any
//! schema drift a compile-time problem.

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
/// from `emit_progress` and consumed by the frontend's `InstallSidebar`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallProgress {
    pub id: String,
    pub name: String,
    pub step: String,
    pub done: bool,
    pub error: Option<String>,
}