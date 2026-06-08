use std::path::{Path, PathBuf};
use std::sync::Arc;

use reqwest::Client;
use tauri::State;
use yaminabe_launcher_shared::datatypes::{
    DistroSource, InstanceOrigin, ModListEntry, ModLoader, ModpackSearchResults, ModpackVersionFile,
};
use yaminabe_launcher_shared::error::Error;
use crate::AppState;

mod curseforge;
mod modrinth;

pub async fn search_modpacks(
    query: &str,
    index: u32,
    client: &Client,
    api_key: &str,
) -> Result<ModpackSearchResults, Error> {
    curseforge::search_modpacks(query, index, client, api_key).await
}

pub async fn get_modpack_files(
    mod_id: u32,
    client: &Client,
    api_key: &str,
) -> Result<Vec<ModpackVersionFile>, Error> {
    curseforge::get_modpack_files(mod_id, client, api_key).await
}

pub async fn install_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    download_url: String,
    instance_location: String,
    category: String,
    origin: InstanceOrigin,
    api_key: &str,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    curseforge::install_modpack(
        app_handle,
        id,
        instance_name,
        download_url,
        instance_location,
        category,
        origin,
        api_key,
        state,
    ).await
}

pub async fn upgrade_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    instance_path: PathBuf,
    download_url: String,
    project_id: u32,
    new_file_id: u32,
    api_key: &str,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    curseforge::upgrade_modpack(
        app_handle,
        id,
        instance_name,
        instance_path,
        download_url,
        project_id,
        new_file_id,
        api_key,
        state,
    ).await
}

pub async fn search_mods(
    query: &str,
    index: u32,
    mc_version: &str,
    mod_loader: &ModLoader,
    client: &Client,
    api_key: &str,
) -> Result<ModpackSearchResults, Error> {
    curseforge::search_mods(query, index, mc_version, mod_loader, client, api_key).await
}

async fn download_mod_with_fallback(
    mod_file: (String, String),
    mods_dir: &Path,
    source: DistroSource,
    api_key: &str,
    client: &Client,
) -> Result<ModListEntry, Error> {
    let (project_id, file_id) = mod_file;
    // TODO: if got Err (= failed to download finally), return ModListEntry { distro_platform: DistroPlatform::Manual }
    match source {
        DistroSource::Modrinth => modrinth::download_mod(file_id, mods_dir, client).await,
        DistroSource::CurseForge => curseforge::download_mod(&project_id, &file_id, mods_dir, api_key, client).await
    }
}

pub async fn download_mods(
    mod_files: Vec<(String, String)>,
    mods_dir: &Path,
    source: DistroSource,
    api_key: &str,
    client: &Client,
) -> Result<Vec<ModListEntry>, Error> {
    if mod_files.is_empty() {
        return Ok(vec![]);
    }

    let semaphore = Arc::new(tokio::sync::Semaphore::new(3));
    let mut handles: Vec<tokio::task::JoinHandle<Result<ModListEntry, Error>>> = Vec::new();

    for mod_file in mod_files {
        let client = client.clone();
        let api_key = api_key.to_string();
        let mods_dir = mods_dir.to_path_buf();
        let semaphore = Arc::clone(&semaphore);

        handles.push(tokio::spawn(async move {
            let permit = semaphore.acquire_owned().await
                .map_err(|e| Error::ChildProcess(format!("semaphore acquire: {e}")))?;
            let result = download_mod_with_fallback(
                mod_file,
                &mods_dir,
                source,
                &api_key,
                &client,
            ).await;
            drop(permit);
            result
        }));
    }

    let mut modlist = Vec::new();
    for handle in handles {
        modlist.push(handle.await
            .map_err(|e| Error::ChildProcess(format!("download task panicked: {e}")))??);
    }
    Ok(modlist)
}
