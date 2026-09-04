use std::path::Path;
use tauri::State;
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_opener::OpenerExt;

use crate::{settings_path, AppSettings, AppState};
use yaminabe_launcher_shared::error::Error;
use crate::commands::instance::find_instance_dir;
use crate::json::write_json;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppSettings {
    state.settings.read().unwrap().clone()
}

#[tauri::command]
pub async fn pick_folder(app: tauri::AppHandle) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<String>>();
    app.dialog()
        .file()
        .pick_folder(move |fp| {
            let result = fp.and_then(|file_path| match file_path {
                FilePath::Path(p) => p.to_str().map(|s| s.to_string()),
                _ => None,
            });
            // Send failure means the receiver was dropped before we got here
            // (window closed during pick); nothing actionable to log.
            tx.send(result).ok();
        });
    rx.await.ok().flatten()
}

/// Open a native multi-select picker for the files the link flow can accept —
/// `.jar` mods and the `.zip` resource packs a modpack may also fail to fetch —
/// returning the chosen paths. The click-to-browse alternative to the drop zone.
#[tauri::command]
pub async fn pick_mod_files(app: tauri::AppHandle) -> Vec<String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<String>>();
    app.dialog()
        .file()
        .add_filter("Mod or resource pack", &["jar", "zip"])
        .pick_files(move |paths| {
            let result = paths.unwrap_or_default().into_iter().filter_map(|fp| match fp {
                FilePath::Path(p) => p.to_str().map(|s| s.to_string()),
                _ => None,
            }).collect();
            tx.send(result).ok();
        });
    rx.await.unwrap_or_default()
}

/// Open a native picker for a modpack zip already on disk, returning the chosen
/// path. Filtered to `.zip`, the shape CurseForge exports.
#[tauri::command]
pub async fn pick_modpack_file(app: tauri::AppHandle) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<String>>();
    app.dialog()
        .file()
        .add_filter("Modpack zip", &["zip"])
        .pick_file(move |path| {
            let picked = path.and_then(|fp| match fp {
                FilePath::Path(p) => p.to_str().map(|s| s.to_string()),
                _ => None,
            });
            tx.send(picked).ok();
        });
    rx.await.ok().flatten()
}

/// Names of the well-known subfolders that actually exist for this instance —
/// the single source of truth for the list (the UI renders the instance root
/// plus whatever this returns). Extend the array here to offer more folders.
#[tauri::command]
pub fn get_instance_subfolders(id: String, state: State<'_, AppState>) -> Vec<String> {
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let Ok(dir) = find_instance_dir(Path::new(&install_dir), &id) else {
        return Vec::new();
    };
    ["config", "mods", "resourcepacks", "saves", "shaderpacks", "datapacks"]
        .into_iter()
        .filter(|s| dir.join(s).exists())
        .map(String::from)
        .collect()
}

#[tauri::command]
pub fn open_instance_subfolder(id: String, subfolder: String, app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), Error> {
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let dir = find_instance_dir(Path::new(&install_dir), &id)?;
    let path = if subfolder.is_empty() { dir } else { dir.join(&subfolder) };
    app.opener().open_path(path.to_string_lossy().as_ref(), Option::<String>::None)
        .map_err(|e| Error::ChildProcess(e.to_string()))
}

#[tauri::command]
pub fn save_settings(
    settings: AppSettings,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    write_json(settings_path(), &settings)?;
    *state.settings.write().unwrap() = settings;
    Ok(())
}
