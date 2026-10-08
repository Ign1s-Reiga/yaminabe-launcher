use std::sync::Mutex;

use crate::AppState;
use log::warn;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use yaminabe_launcher_shared::error::Error;
use yaminabe_launcher_shared::ipc::{ReleaseInfo, UpdateCheck, UpdateProgress};

/// The release the last check found, kept so the install that follows fetches
/// exactly what the user was shown.
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<Update>>);

/// Whether the launcher looks for a newer release by itself at launch.
///
/// Only a release build does. A development build's version says nothing about
/// what is installed, and it would be offered every release there is.
#[tauri::command]
pub fn checks_updates_on_launch() -> bool {
    !cfg!(debug_assertions)
}

/// Ask GitHub whether a newer release has been published.
///
/// The updater reads the latest published release's `latest.json`. A release
/// without one, as every release before the updater was, reads as nothing to
/// update to rather than as a failure.
#[tauri::command]
pub async fn check_for_update(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
) -> Result<UpdateCheck, Error> {
    let updater = app.updater().map_err(|e| Error::Update(e.to_string()))?;
    let found = match updater.check().await {
        Ok(found) => found,
        Err(tauri_plugin_updater::Error::ReleaseNotFound) => None,
        Err(e) => return Err(Error::Update(e.to_string())),
    };
    let check = match &found {
        Some(update) => UpdateCheck::Available(ReleaseInfo {
            version: update.version.clone(),
            date: update.date.map(|date| date.date().to_string()),
            notes: update.body.clone().unwrap_or_default(),
        }),
        None => UpdateCheck::UpToDate,
    };
    *pending.0.lock().unwrap() = found;
    Ok(check)
}

/// Download and install the release the last check found, reporting progress
/// as `update-download-progress`.
///
/// The installer closes the launcher to replace it and starts it again once it
/// is done, so on success this does not return. It refuses while an instance is
/// running or being changed, since closing the launcher would cut that off.
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, AppState>,
    pending: State<'_, PendingUpdate>,
) -> Result<(), Error> {
    if !state.instance_activity.lock().unwrap().is_empty() {
        return Err(Error::Busy(
            "an instance is running or being changed".to_string(),
        ));
    }
    // Cloned out, so the lock is not held across the download, and a failed
    // download can be retried without checking again.
    let update = pending
        .0
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| Error::Update("no update has been found to install".to_string()))?;

    let mut downloaded: u64 = 0;
    update
        .download_and_install(
            |chunk, total| {
                downloaded += chunk as u64;
                let progress = UpdateProgress { downloaded, total };
                if let Err(e) = app.emit("update-download-progress", progress) {
                    warn!("failed to emit update-download-progress: {e}");
                }
            },
            || {},
        )
        .await
        .map_err(|e| Error::Update(e.to_string()))
}
