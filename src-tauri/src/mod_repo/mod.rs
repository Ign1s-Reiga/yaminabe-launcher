use std::path::{Path, PathBuf};
use std::sync::Arc;

use log::info;
use reqwest::Client;
use tauri::State;
use yaminabe_launcher_shared::datatypes::{
    InstanceOrigin, ModListEntry, ModLoader, ModpackSearchResults, ModpackVersionFile,
};
use yaminabe_launcher_shared::error::Error;
use crate::http_utils::sha1_hex;
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

pub async fn install_mod(
    instance_path: &Path,
    project_id: u32,
    mc_version: &str,
    mod_loader: &ModLoader,
    api_key: &str,
    client: &Client,
) -> Result<Vec<ModListEntry>, Error> {
    curseforge::install_mod(instance_path, project_id, mc_version, mod_loader, api_key, client).await
}

pub async fn download_mods(
    file_ids: Vec<String>,
    instance_location: &str,
    source: Option<&str>,
    api_key: &str,
    client: &Client,
) -> Result<Vec<ModListEntry>, Error> {
    match source.unwrap_or("curseforge") {
        "modrinth" => modrinth::download_mods(&file_ids, instance_location, client).await,
        _ => {
            let ids: Vec<u32> = file_ids.iter()
                .filter_map(|s| s.parse().ok())
                .collect();
            curseforge::download_mods(ids, instance_location, api_key, client).await
        }
    }
}

/// Download each `(url, filename)` pair into `dest_dir`, at most three at a time.
/// Shared by the CurseForge and Modrinth mod downloaders. Creates `dest_dir` if
/// it doesn't exist; entries with an empty url or filename are skipped, and the
/// first failed (or panicked) download aborts the batch with that error.
#[derive(Debug, Clone)]
pub struct DownloadedFile {
    pub sha1: String,
}

pub async fn download_files(
    client: &Client,
    files: Vec<(String, String)>,
    dest_dir: &Path,
) -> Result<Vec<DownloadedFile>, Error> {
    if files.is_empty() {
        return Ok(vec![]);
    }
    std::fs::create_dir_all(dest_dir)?;

    let semaphore = Arc::new(tokio::sync::Semaphore::new(3));
    let mut handles: Vec<tokio::task::JoinHandle<Result<Option<DownloadedFile>, Error>>> = Vec::new();

    for (url, filename) in files {
        if url.is_empty() || filename.is_empty() {
            continue;
        }
        let client = client.clone();
        let dest_dir = dest_dir.to_path_buf();
        let sem = Arc::clone(&semaphore);

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await
                .map_err(|e| Error::ChildProcess(format!("semaphore acquire: {e}")))?;
            let resp = client.get(&url).send().await?;
            if !resp.status().is_success() {
                return Err(Error::HttpRequestRejected(resp.status().as_u16(), url));
            }
            let bytes = resp.bytes().await.map_err(Error::InvalidResponse)?;
            let sha1 = sha1_hex(&bytes);
            std::fs::write(dest_dir.join(&filename), &bytes)?;
            info!("Downloaded {filename}");
            Ok(Some(DownloadedFile { sha1 }))
        }));
    }

    let mut downloaded = Vec::new();
    for handle in handles {
        if let Some(file) = handle.await
            .map_err(|e| Error::ChildProcess(format!("download task panicked: {e}")))??
        {
            downloaded.push(file);
        }
    }
    Ok(downloaded)
}
