use std::path::Path;
use tauri::State;
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_opener::OpenerExt;

use crate::{settings_path, AppSettings, AppState};
use yaminabe_launcher_shared::error::Error;
use crate::commands::instance::find_instance_dir;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppSettings {
    state.settings.lock().unwrap().clone()
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
            let _ = tx.send(result);
        });
    rx.await.ok().flatten()
}

#[tauri::command]
pub fn get_instance_subfolders(id: String, state: State<'_, AppState>) -> Vec<bool> {
    let install_dir = state.settings.lock().unwrap().instance_install_dir.clone();
    let Ok(dir) = find_instance_dir(Path::new(&install_dir), &id) else {
        return vec![false; 4];
    };
    ["config", "mods", "resourcepacks", "saves"]
        .iter()
        .map(|s| dir.join(s).exists())
        .collect()
}

#[tauri::command]
pub fn open_instance_subfolder(id: String, subfolder: String, app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), Error> {
    let install_dir = state.settings.lock().unwrap().instance_install_dir.clone();
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
    let json = serde_json::to_string_pretty(&settings)?;
    std::fs::write(settings_path(), &json)?;
    *state.settings.lock().unwrap() = settings;
    Ok(())
}
