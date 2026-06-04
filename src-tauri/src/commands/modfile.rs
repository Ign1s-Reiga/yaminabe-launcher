use std::path::Path;
use tauri::State;
use yaminabe_launcher_shared::datatypes::{InstanceMeta, InstanceOrigin, ModpackSearchResults, ModpackVersionFile};
use yaminabe_launcher_shared::error::Error;
use crate::{emit_progress, AppState};
use crate::commands::instance::find_instance_dir;
use crate::mod_repo::{curseforge, modrinth};

#[tauri::command]
pub async fn search_curseforge_modpacks(
    query: String,
    index: u32,
    state: State<'_, AppState>,
) -> Result<ModpackSearchResults, Error> {
    let api_key = state.settings.read().unwrap().curseforge_api_key.clone();
    curseforge::search_modpacks(&query, index, &state.http_client, &api_key).await
}

#[tauri::command]
pub async fn get_modpack_files(
    mod_id: u32,
    state: State<'_, AppState>,
) -> Result<Vec<ModpackVersionFile>, Error> {
    let api_key = state.settings.read().unwrap().curseforge_api_key.clone();
    curseforge::get_modpack_files(mod_id, &state.http_client, &api_key).await
}

#[tauri::command]
pub async fn install_curseforge_modpack(
    app_handle: tauri::AppHandle,
    download_url: String,
    instance_name: String,
    install_dir: String,
    category: String,
    project_id: u32,
    file_id: u32,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string();

    let api_key = state.settings.read().unwrap().curseforge_api_key.clone();
    let origin = InstanceOrigin::CurseForge { project_id, file_id };

    let result = curseforge::install_modpack(
        &app_handle, &id, &instance_name,
        download_url, install_dir, category, origin,
        &api_key, &state,
    ).await;

    match result {
        Ok(()) => {
            emit_progress(&app_handle, &id, &instance_name, "Done", true, None);
            Ok(())
        }
        Err(e) => {
            emit_progress(&app_handle, &id, &instance_name, "Failed", false, Some(e.to_string()));
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn upgrade_curseforge_modpack(
    app_handle: tauri::AppHandle,
    instance_id: String,
    download_url: String,
    project_id: u32,
    file_id: u32,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let instance_dir = find_instance_dir(Path::new(&install_dir), &instance_id)?;

    let meta: InstanceMeta = serde_json::from_str(&std::fs::read_to_string(instance_dir.join("instance.json"))?)?;
    if meta.origin.curseforge_ids().is_none() {
        return Err(Error::Invalid("instance is not a CurseForge modpack instance".to_string()));
    }
    let instance_name = meta.name.clone();
    let api_key = state.settings.read().unwrap().curseforge_api_key.clone();

    emit_progress(&app_handle, &instance_id, &instance_name, "Preparing upgrade", false, None);
    let result = curseforge::upgrade_modpack(
        &app_handle, &instance_id, &instance_name, instance_dir,
        download_url, project_id, file_id, &api_key, &state,
    ).await;

    match result {
        Ok(()) => {
            emit_progress(&app_handle, &instance_id, &instance_name, "Done", true, None);
            Ok(())
        }
        Err(e) => {
            emit_progress(&app_handle, &instance_id, &instance_name, "Failed", false, Some(e.to_string()));
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn download_mods(
    file_ids: Vec<String>,
    instance_location: String,
    source: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    match source.as_deref().unwrap_or("modrinth") {
        "curseforge" => {
            let ids: Vec<u32> = file_ids.iter()
                .filter_map(|s| s.parse().ok())
                .collect();
            let api_key = state.settings.read().unwrap().curseforge_api_key.clone();
            curseforge::download_mods(ids, &instance_location, &api_key, &state.http_client).await
        }
        _ => modrinth::download_mods(&file_ids, &instance_location, &state.http_client).await,
    }
}