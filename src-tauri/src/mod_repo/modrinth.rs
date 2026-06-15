use std::path::PathBuf;

use crate::AppState;
use crate::http_utils::{download_resource, fetch_json};
use serde::Deserialize;
use tauri::State;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, ModProjectSearchResults, ProjectFileInfo, ProjectFileTarget, SearchOptions,
};
use yaminabe_launcher_shared::error::Error;

#[derive(Deserialize)]
struct Version {
    id: String,
    project_id: String,
    version_type: String,
    #[serde(default)]
    files: Vec<VersionFile>,
    #[serde(default)]
    dependencies: Vec<Dependency>,
}

impl Version {
    fn to_project_file_info(&self, project_type: &str) -> Result<ProjectFileInfo, Error> {
        let version_file = selected_file(self).ok_or_else(|| {
            Error::NotExists(format!("Modrinth version-file {} does not exists.", self.id))
        })?;
        Ok(ProjectFileInfo {
            source: DownloadSource::Modrinth {
                project_id: self.project_id.clone(),
                version_id: self.id.clone(),
            },
            target: ProjectFileTarget::from_modrinth_type(project_type),
            release_type: self.version_type.clone().into(),
            file_name: version_file.filename.clone(),
            download_url: Some(version_file.url.clone()),
            display_name: version_file.filename.clone(),
            sha1: version_file.hashes.sha1.clone(),
            size: version_file.size,
        })
    }
}

#[derive(Deserialize)]
struct VersionFile {
    url: String,
    filename: String,
    #[serde(default)]
    hashes: Hashes,
    size: u64,
    #[serde(default)]
    primary: bool,
}

#[derive(Deserialize)]
struct Dependency {
    version_id: Option<String>,
    project_id: String,
    dependency_type: DependencyType,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum DependencyType {
    Required,
    Optional,
    Incompatible,
    Embedded,
    #[serde(other)]
    Unknown,
}

#[derive(Default, Deserialize)]
struct Hashes {
    #[serde(default)]
    sha1: String,
}

/// Download the Modrinth file matching `sha1` to `dest_path` (a full path,
/// including the file name), so a caller mirroring another platform's file can
/// keep that platform's on-disk name. Verifies against the Modrinth hash.
pub async fn download_file_by_sha1(
    sha1: &str,
    dest_path: PathBuf,
    client: &reqwest::Client,
) -> Result<(), Error> {
    let version = fetch_json(
        client,
        &format!("https://api.modrinth.com/v2/version_file/{sha1}"),
    )
    .send::<Version>()
    .await?;
    let file = selected_file(&version).ok_or_else(|| {
        Error::NotExists(format!(
            "Modrinth version-file {} does not exists.",
            version.id
        ))
    })?;
    download_resource(client, &file.url, &file.hashes.sha1, dest_path).await?;
    Ok(())
}

#[allow(unused_variables)]
pub async fn search_projects(
    option: &SearchOptions,
    client: &reqwest::Client,
    api_key: &str,
) -> Result<ModProjectSearchResults, Error> {
    unimplemented!()
}

pub fn list_project_files() {
    unimplemented!()
}

#[allow(unused_variables)]
pub async fn install_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    source: DownloadSource,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    unimplemented!()
}

#[allow(unused_variables)]
pub async fn upgrade_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    instance_path: PathBuf,
    source: DownloadSource,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    unimplemented!()
}

fn selected_file(version: &Version) -> Option<&VersionFile> {
    version
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| version.files.first())
}
