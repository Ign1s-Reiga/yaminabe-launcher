use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::AppState;
use crate::http_utils::download_resource;
use log::warn;
use reqwest::Client;
use tauri::State;
use yaminabe_launcher_shared::datamodels::ModState;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, ModListEntry, ModLoader, ModProjectSearchResults, Platform, ProjectFileInfo,
    ProjectFileTarget, SearchOptions,
};
use yaminabe_launcher_shared::error::Error;

mod curseforge;
mod modrinth;

pub async fn search_projects(
    option: &SearchOptions,
    client: &Client,
    api_key: &str,
) -> Result<ModProjectSearchResults, Error> {
    match option.platform {
        Platform::CurseForge => curseforge::search_projects(option, client, api_key).await,
        Platform::Modrinth => modrinth::search_projects(option, client).await,
    }
}

pub async fn list_project_files(
    mod_id: u32,
    target: ProjectFileTarget,
    game_version: Option<String>,
    mod_loader: Option<ModLoader>,
    index: u32,
    client: &Client,
    api_key: &str,
) -> Result<Vec<ProjectFileInfo>, Error> {
    curseforge::list_project_files(
        mod_id,
        target,
        game_version.as_deref(),
        mod_loader.as_ref(),
        index,
        client,
        api_key,
    )
    .await
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
            curseforge::install_modpack(app_handle, id, instance_name, category, source, state)
                .await
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
            curseforge::upgrade_modpack(app_handle, id, instance_name, instance_path, source, state)
                .await
        }
        DownloadSource::Modrinth { .. } => {
            modrinth::upgrade_modpack(app_handle, id, instance_name, instance_path, source, state)
                .await
        }
        DownloadSource::Manual => Err(Error::Unsupported(
            "manual modpack upgrade is not yet supported".to_string(),
        )),
    }
}

/// Download already-resolved project files into the instance, three at a time.
/// Per-file failures never abort the batch: a file that can't be fetched yields
/// a `DownloadFailed` modlist entry (mods only) so the user can link a jar by
/// hand. Resource packs are downloaded but not returned (the modlist is mods
/// only). Only a task panic propagates as `Err`.
pub async fn download_project_files(
    files: Vec<ProjectFileInfo>,
    instance_path: &Path,
    client: &Client,
) -> Result<Vec<ModListEntry>, Error> {
    if files.is_empty() {
        return Ok(vec![]);
    }

    let semaphore = Arc::new(tokio::sync::Semaphore::new(3));
    let mut handles: Vec<tokio::task::JoinHandle<Result<Option<ModListEntry>, Error>>> = Vec::new();

    for file in files {
        let instance_path = instance_path.to_path_buf();
        let client = client.clone();
        let semaphore = Arc::clone(&semaphore);

        handles.push(tokio::spawn(async move {
            let permit = semaphore
                .acquire_owned()
                .await
                .map_err(|e| Error::ChildProcess(format!("semaphore acquire: {e}")))?;
            let result = download_project_file(file, &instance_path, &client).await;
            drop(permit);
            result
        }));
    }

    let mut modlist = Vec::new();
    for handle in handles {
        if let Some(entry) = handle
            .await
            .map_err(|e| Error::ChildProcess(format!("download task panicked: {e}")))??
        {
            modlist.push(entry);
        }
    }
    Ok(modlist)
}

/// Download one resolved file to its type-appropriate subdir. A missing SHA-1,
/// a CurseForge file with downloads disabled (mirrored from Modrinth by SHA-1
/// when possible), or a failed fetch all yield a `DownloadFailed` entry rather
/// than an error. Returns `None` for resource packs (downloaded, untracked).
async fn download_project_file(
    file: ProjectFileInfo,
    instance_path: &Path,
    client: &Client,
) -> Result<Option<ModListEntry>, Error> {
    let dir = instance_path.join(file.target.directory());
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(&file.file_name);

    let download_result = if file.sha1.is_empty() {
        Err(Error::Invalid(format!(
            "project file {} has no SHA-1 hash",
            file.file_name
        )))
    } else if let Some(url) = &file.download_url {
        download_resource(client, url, &file.sha1, dest).await
    } else if matches!(file.source, DownloadSource::CurseForge { .. }) {
        warn!(
            "CurseForge restricts downloads for file {}, falling back to Modrinth lookup by SHA-1",
            file.file_name
        );
        modrinth::download_file_by_sha1(&file.sha1, dest, client).await
    } else {
        Err(Error::Invalid(format!(
            "project file {} has no download URL",
            file.file_name
        )))
    };

    let state = match download_result {
        Ok(()) => ModState::Enabled,
        Err(e) => {
            warn!(
                "download failed for {}: {e}; marking for manual install",
                file.file_name
            );
            ModState::DownloadFailed
        }
    };

    let track = file.target.tracks_modlist() || state == ModState::DownloadFailed;
    Ok(track.then(|| file.to_modlist_entry(state)))
}
