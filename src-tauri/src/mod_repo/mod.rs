use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::AppState;
use crate::http_utils::download_resource;
use log::warn;
use reqwest::Client;
use tauri::State;
use yaminabe_launcher_shared::datatypes::ModState;
use yaminabe_launcher_shared::datatypes::{
    DownloadSource, ModListEntry, ModProjectFile, ModProjectSearchResults, SearchOption,
};
use yaminabe_launcher_shared::error::Error;

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

#[derive(Clone, Copy, Debug)]
pub(crate) enum ProjectFileTarget {
    Mods,
    ResourcePacks,
}

impl ProjectFileTarget {
    fn directory(self) -> &'static str {
        match self {
            ProjectFileTarget::Mods => "mods",
            ProjectFileTarget::ResourcePacks => "resourcepacks",
        }
    }

    fn tracks_modlist(self) -> bool {
        matches!(self, ProjectFileTarget::Mods)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ProjectFileDownload {
    pub source: DownloadSource,
    pub file_name: String,
    pub sha1: String,
    pub size: u64,
    pub download_url: Option<String>,
    pub target: ProjectFileTarget,
    pub fallback_to_modrinth: bool,
    pub require_sha1: bool,
}

impl ProjectFileDownload {
    fn to_modlist_entry(&self, state: ModState) -> ModListEntry {
        ModListEntry {
            file_name: self.file_name.clone(),
            sha1: self.sha1.clone(),
            source: self.source.clone(),
            size: self.size,
            state,
        }
    }
}

pub async fn resolve_download_sources(
    sources: Vec<DownloadSource>,
    api_key: &str,
    client: &Client,
) -> Result<Vec<ProjectFileDownload>, Error> {
    let mut files = Vec::new();
    for source in sources {
        let resolved = match source {
            DownloadSource::CurseForge { project_id, file_id } => {
                curseforge::resolve_project_file(project_id, file_id, api_key, client).await
            }
            DownloadSource::Modrinth { version_id, .. } => {
                modrinth::resolve_project_file(version_id, client).await
            }
            DownloadSource::Manual => Err(Error::Invalid(
                "cannot download a Manual source; this mod must be installed by hand".to_string(),
            )),
        }?;
        files.push(resolved);
    }
    Ok(files)
}

pub async fn download_project_files(
    project_files: Vec<ProjectFileDownload>,
    instance_path: &Path,
    client: &Client,
) -> Result<Vec<ModListEntry>, Error> {
    if project_files.is_empty() {
        return Ok(vec![]);
    }

    let semaphore = Arc::new(tokio::sync::Semaphore::new(3));
    let mut handles: Vec<tokio::task::JoinHandle<Result<Option<ModListEntry>, Error>>> = Vec::new();

    for project_file in project_files {
        let instance_path = instance_path.to_path_buf();
        let client = client.clone();
        let semaphore = Arc::clone(&semaphore);

        handles.push(tokio::spawn(async move {
            let permit = semaphore
                .acquire_owned()
                .await
                .map_err(|e| Error::ChildProcess(format!("semaphore acquire: {e}")))?;
            let result = download_project_file(project_file, &instance_path, &client).await;
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

async fn download_project_file(
    project_file: ProjectFileDownload,
    instance_path: &Path,
    client: &Client,
) -> Result<Option<ModListEntry>, Error> {
    if project_file.require_sha1 && project_file.sha1.is_empty() {
        return Err(Error::Invalid(format!(
            "project file {} has no SHA-1 hash",
            project_file.file_name
        )));
    }

    let dir = instance_path.join(project_file.target.directory());
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(&project_file.file_name);

    let download_result = match &project_file.download_url {
        Some(url) => download_resource(client, url, &project_file.sha1, dest).await,
        None if project_file.fallback_to_modrinth => {
            warn!(
                "CurseForge restricts downloads for file {}, falling back to Modrinth lookup by SHA-1",
                project_file.file_name
            );
            if project_file.sha1.is_empty() {
                Err(Error::Invalid(format!(
                    "project file {} has no SHA-1 hash",
                    project_file.file_name
                )))
            } else {
                modrinth::download_file_by_sha1(&project_file.sha1, dest, client).await
            }
        }
        None => {
            warn!(
                "project file {} has no download URL; marking for manual install",
                project_file.file_name
            );
            Err(Error::Invalid(format!(
                "project file {} has no download URL",
                project_file.file_name
            )))
        }
    };

    let state = match download_result {
        Ok(()) => ModState::Enabled,
        Err(e) => {
            warn!(
                "download failed for {}: {e}; marking for manual install",
                project_file.file_name
            );
            ModState::DownloadFailed
        }
    };

    Ok(project_file
        .target
        .tracks_modlist()
        .then(|| project_file.to_modlist_entry(state)))
}

pub async fn download_mods(
    mod_files: Vec<DownloadSource>,
    instance_path: &Path,
    api_key: &str,
    client: &Client,
) -> Result<Vec<ModListEntry>, Error> {
    let project_files = resolve_download_sources(mod_files, api_key, client).await?;
    download_project_files(project_files, instance_path, client).await
}
