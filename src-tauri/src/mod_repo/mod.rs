use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::AppState;
use crate::commands::instance::is_bare_file_name;
use crate::http_utils::download_resource;
use log::warn;
use reqwest::Client;
use tauri::State;
use yaminabe_launcher_shared::datamodels::ModState;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, LocalModpackInfo, ModListEntry, ModLoader, ModProjectSearchResults,
    ModpackFormat, Platform, ProjectFileInfo, ProjectFileTarget, ProjectId, SearchOptions,
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

/// List one project's downloadable files. The id names its own platform, so a
/// Modrinth project cannot be sent to a CurseForge endpoint that could not
/// answer for it.
pub async fn list_project_files(
    project_id: ProjectId,
    target: ProjectFileTarget,
    game_version: Option<String>,
    mod_loader: Option<ModLoader>,
    index: u32,
    client: &Client,
    api_key: &str,
) -> Result<Vec<ProjectFileInfo>, Error> {
    match &project_id {
        ProjectId::CurseForge(mod_id) => {
            curseforge::list_project_files(
                *mod_id,
                target,
                game_version.as_deref(),
                mod_loader.as_ref(),
                index,
                client,
                api_key,
            )
            .await
        }
        ProjectId::Modrinth(id) => {
            modrinth::list_project_files(
                id,
                target,
                game_version.as_deref(),
                mod_loader.as_ref(),
                index,
                client,
            )
            .await
        }
    }
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
    let written = dest.clone();

    let download_result = if !is_bare_file_name(&file.file_name) {
        // The name is the site's, not ours. A name that would escape its own
        // directory is recorded like any other unfetchable file rather than
        // aborting the batch — link_mods applies the same guard before writing.
        Err(Error::Invalid(format!(
            "project file '{}' is not a plain file name",
            file.file_name
        )))
    } else if file.sha1.is_empty() {
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
    Ok(track.then(|| {
        let mut entry = file.to_modlist_entry(state);
        // CurseForge publishes no size for some files. The jar is on disk now,
        // so measure it rather than listing the mod as 0 B forever.
        if entry.size == 0 && entry.state == ModState::Enabled {
            match std::fs::metadata(&written) {
                Ok(meta) => entry.size = meta.len(),
                Err(e) => warn!("cannot size {}: {e}", entry.file_name),
            }
        }
        entry
    }))
}

/// Where the user can fetch a failed file by hand.
pub async fn project_page_url(
    source: &DownloadSource,
    api_key: &str,
    client: &Client,
) -> Result<String, Error> {
    match source {
        DownloadSource::CurseForge { project_id, file_id } => {
            curseforge::project_file_page_url(*project_id, *file_id, api_key, client).await
        }
        DownloadSource::Modrinth { project_id, version_id } => {
            Ok(format!("https://modrinth.com/project/{project_id}/version/{version_id}"))
        }
        DownloadSource::Manual => Err(Error::Unsupported(
            "a hand-added file has no project page".to_string(),
        )),
    }
}

/// Which format a modpack file is, decided by the index it carries rather than
/// its extension — a `.mrpack` is a zip, and either format may reach us under
/// either name.
fn detect_format(zip_path: &Path) -> Result<ModpackFormat, Error> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| Error::Invalid(format!("cannot open {}: {e}", zip_path.display())))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| Error::Invalid(format!("{} is not a zip file", zip_path.display())))?;
    if archive.by_name("modrinth.index.json").is_ok() {
        return Ok(ModpackFormat::Modrinth);
    }
    if archive.by_name("manifest.json").is_ok() {
        return Ok(ModpackFormat::CurseForge);
    }
    Err(Error::Invalid(format!(
        "{} carries neither manifest.json nor modrinth.index.json, so it is not a modpack",
        zip_path.display()
    )))
}

/// Read a modpack file on disk, in whichever of the two supported formats it
/// turns out to be.
pub fn read_local_modpack(zip_path: &Path) -> Result<LocalModpackInfo, Error> {
    match detect_format(zip_path)? {
        ModpackFormat::CurseForge => curseforge::read_local_modpack(zip_path),
        ModpackFormat::Modrinth => modrinth::read_local_modpack(zip_path),
    }
}

/// Install from a zip the user already has, rather than one fetched by id.
pub async fn install_modpack_from_file(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    zip_path: &Path,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    match detect_format(zip_path)? {
        ModpackFormat::CurseForge => {
            curseforge::install_modpack_from_file(
                app_handle,
                id,
                instance_name,
                category,
                zip_path,
                state,
            )
            .await
        }
        ModpackFormat::Modrinth => {
            modrinth::install_modpack_from_file(
                app_handle,
                id,
                instance_name,
                category,
                zip_path,
                state,
            )
            .await
        }
    }
}
