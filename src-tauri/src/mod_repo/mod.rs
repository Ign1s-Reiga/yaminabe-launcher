use std::path::{Path, PathBuf};
use std::sync::Arc;

use reqwest::Client;
use tauri::State;
use yaminabe_launcher_shared::datatypes::{
    DownloadSource, ModListEntry, ModProjectSearchResults, ModProjectFile, SearchOption,
};
use yaminabe_launcher_shared::error::Error;
use crate::AppState;

mod curseforge;
mod modrinth;

pub async fn search_projects(
    option: &SearchOption,
    client: &Client,
    api_key: &str,
) -> Result<ModProjectSearchResults, Error> {
    curseforge::search_projects(option, client, api_key).await
}

pub async fn list_project_files(
    mod_id: u32,
    client: &Client,
    api_key: &str,
) -> Result<Vec<ModProjectFile>, Error> {
    curseforge::list_project_files(mod_id, client, api_key).await
}

pub async fn install_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    source: DownloadSource,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    match source {
        DownloadSource::CurseForge { .. } => {
            curseforge::install_modpack(app_handle, id, instance_name, category, source, state).await
        }
        DownloadSource::Modrinth { .. } => {
            modrinth::install_modpack(app_handle, id, instance_name, category, source, state).await
        }
        DownloadSource::Manual => Err(Error::Unsupported(
            "manual modpack installation is not yet supported".to_string(),
        )),
    }
}

pub async fn upgrade_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    instance_path: PathBuf,
    source: DownloadSource,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    match source {
        DownloadSource::CurseForge { .. } => {
            curseforge::upgrade_modpack(app_handle, id, instance_name, instance_path, source, state).await
        }
        DownloadSource::Modrinth { .. } => {
            modrinth::upgrade_modpack(app_handle, id, instance_name, instance_path, source, state).await
        }
        DownloadSource::Manual => Err(Error::Unsupported(
            "manual modpack upgrade is not yet supported".to_string(),
        )),
    }
}

/// Dispatch a single download on its source variant. The CurseForge → Modrinth
/// (by SHA-1) fallback for opted-out files lives inside
/// `curseforge::download_project_file`.
///
/// A failure to fetch the jar does not error here: the platform fn returns a
/// `ModListEntry` with `state = DownloadFailed` (keeping its real source) so the
/// batch continues and the UI can prompt the user to link a jar by hand. Only a
/// metadata/API failure propagates as `Err`.
async fn download_from_source(
    source: DownloadSource,
    mods_dir: &Path,
    api_key: &str,
    client: &Client,
) -> Result<ModListEntry, Error> {
    match source {
        DownloadSource::CurseForge { project_id, file_id } => {
            curseforge::download_project_file(project_id, file_id, mods_dir, api_key, client).await
        }
        DownloadSource::Modrinth { version_id, .. } => {
            modrinth::download_project_file(version_id, mods_dir, client).await
        }
        DownloadSource::Manual => Err(Error::Invalid(
            "cannot download a Manual source; this mod must be installed by hand".to_string(),
        )),
    }
}

pub async fn download_mods(
    mod_files: Vec<DownloadSource>,
    instance_path: &Path,
    api_key: &str,
    client: &Client,
) -> Result<Vec<ModListEntry>, Error> {
    if mod_files.is_empty() {
        return Ok(vec![]);
    }
    let mods_dir = instance_path.join("mods");
    std::fs::create_dir_all(&mods_dir)?;

    let semaphore = Arc::new(tokio::sync::Semaphore::new(3));
    let mut handles: Vec<tokio::task::JoinHandle<Result<ModListEntry, Error>>> = Vec::new();

    for mod_file in mod_files {
        let client = client.clone();
        let api_key = api_key.to_string();
        let mods_dir = mods_dir.clone();
        let semaphore = Arc::clone(&semaphore);

        handles.push(tokio::spawn(async move {
            let permit = semaphore.acquire_owned().await
                .map_err(|e| Error::ChildProcess(format!("semaphore acquire: {e}")))?;
            let result = download_from_source(mod_file, &mods_dir, &api_key, &client).await;
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
