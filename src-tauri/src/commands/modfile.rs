use std::path::{Path, PathBuf};
use tauri::State;
use yaminabe_launcher_shared::datatypes::{
    DistroSource, InstanceMeta, InstanceOrigin, ModListEntry, ModLoader, ModpackSearchResults, ModpackVersionFile,
};
use yaminabe_launcher_shared::error::Error;
use crate::{emit_progress, ActivityGuard, AppState, InstanceActivity};
use crate::commands::instance::{find_instance_dir, instance_meta_file, modlist_file, remove_modlist_file, upsert_modlist_entries};
use crate::json::read_json;
use crate::mod_repo;

#[tauri::command]
pub async fn search_curseforge_modpacks(
    query: String,
    index: u32,
    state: State<'_, AppState>,
) -> Result<ModpackSearchResults, Error> {
    let api_key = state.settings.read().unwrap().curseforge_api_key.clone();
    mod_repo::search_modpacks(&query, index, &state.http_client, &api_key).await
}

#[tauri::command]
pub async fn get_modpack_files(
    mod_id: u32,
    state: State<'_, AppState>,
) -> Result<Vec<ModpackVersionFile>, Error> {
    let api_key = state.settings.read().unwrap().curseforge_api_key.clone();
    mod_repo::get_modpack_files(mod_id, &state.http_client, &api_key).await
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

    let result = mod_repo::install_modpack(
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
    let Some(_activity) = ActivityGuard::claim(&state.instance_activity, &instance_id, InstanceActivity::Upgrading) else {
        return Err(Error::Busy(format!("instance '{instance_id}' is already busy")));
    };

    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let instance_dir = find_instance_dir(Path::new(&install_dir), &instance_id)?;

    let meta: InstanceMeta = read_json(instance_meta_file(&instance_dir))?;
    if meta.origin.curseforge_ids().is_none() {
        return Err(Error::Invalid("instance is not a CurseForge modpack instance".to_string()));
    }
    let instance_name = meta.name.clone();
    let api_key = state.settings.read().unwrap().curseforge_api_key.clone();

    emit_progress(&app_handle, &instance_id, &instance_name, "Preparing upgrade", false, None);
    let result = mod_repo::upgrade_modpack(
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
pub async fn search_mods(
    query: String,
    mc_version: String,
    mod_loader: ModLoader,
    index: u32,
    state: State<'_, AppState>,
) -> Result<ModpackSearchResults, Error> {
    let api_key = state.settings.read().unwrap().curseforge_api_key.clone();
    mod_repo::search_mods(&query, index, &mc_version, &mod_loader, &state.http_client, &api_key).await
}

#[tauri::command]
pub fn list_instance_mods(
    instance_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ModListEntry>, Error> {
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let instance_dir = find_instance_dir(Path::new(&install_dir), &instance_id)?;
    let mut modlist: Vec<ModListEntry> = read_json(modlist_file(&instance_dir))
        .unwrap_or_default();
    modlist.sort_by(|a, b| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()));
    Ok(modlist)
}

#[tauri::command]
pub fn delete_instance_mod(
    instance_id: String,
    file_name: String,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    // Only a bare file name inside `mods/` may be removed — reject traversal.
    if file_name.is_empty() || file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
        return Err(Error::Invalid(format!("invalid mod file name '{file_name}'")));
    }
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let instance_dir = find_instance_dir(Path::new(&install_dir), &instance_id)?;
    let path = instance_dir.join("mods").join(&file_name);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    remove_modlist_file(&instance_dir, &file_name)
}

#[tauri::command]
pub async fn download_mods(
    mod_files: Vec<(String, String)>,
    instance_id: String,
    source: DistroSource,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    let (install_dir, api_key) = {
        let settings = state.settings.read().unwrap();
        (
            settings.instance_install_dir.clone(),
            settings.curseforge_api_key.clone(),
        )
    };
    let instance_path = find_instance_dir(Path::new(&install_dir), &instance_id)?;
    let entries = mod_repo::download_mods(
        mod_files,
        &instance_path,
        source,
        &api_key,
        &state.http_client,
    ).await?;
    upsert_modlist_entries(&instance_path, entries)
}
