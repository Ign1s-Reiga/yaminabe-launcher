use std::path::{Path, PathBuf};

use crate::http_utils::{download_resource, fetch_json};
use serde::Deserialize;
use yaminabe_launcher_shared::datatypes::{DownloadSource, ModListEntry};
use yaminabe_launcher_shared::error::Error;

#[derive(Deserialize)]
struct Version {
    id: String,
    project_id: String,
    #[serde(default)]
    files: Vec<VersionFile>,
    #[serde(default)]
    dependencies: Vec<Dependency>,
}

impl Version {
    fn to_modlist_entry(self: &Version) -> ModListEntry {
        let version_file = selected_file(self).unwrap();

        ModListEntry {
            file_name: version_file.filename.clone(),
            sha1: version_file.hashes.sha1.clone(),
            source: DownloadSource::Modrinth {
                project_id: self.project_id.clone(),
                version_id: self.id.clone(),
            },
            size: version_file.size,
        }
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

fn selected_file(version: &Version) -> Option<&VersionFile> {
    version.files.iter().find(|f| f.primary).or_else(|| version.files.first())
}

async fn download_version_file(
    version: Version,
    mods_dir: &Path,
    client: &reqwest::Client,
) -> Result<ModListEntry, Error> {
    let file = selected_file(&version)
        .ok_or_else(|| Error::NotExists(format!("Modrinth version-file {} does not exists.", version.id)))?;
    download_resource(client, &file.url, &file.hashes.sha1, mods_dir.join(&file.filename)).await?;
    Ok(version.to_modlist_entry())
}

/// Download the Modrinth file matching `sha1` to `dest_path` (a full path,
/// including the file name), so a caller mirroring another platform's file can
/// keep that platform's on-disk name. Verifies against the Modrinth hash.
pub async fn download_file_by_sha1(
    sha1: &str,
    dest_path: PathBuf,
    client: &reqwest::Client,
) -> Result<(), Error> {
    let version = fetch_json(client, &format!("https://api.modrinth.com/v2/version_file/{sha1}"))
        .send::<Version>()
        .await?;
    let file = selected_file(&version)
        .ok_or_else(|| Error::NotExists(format!("Modrinth version-file {} does not exists.", version.id)))?;
    download_resource(client, &file.url, &file.hashes.sha1, dest_path).await?;
    Ok(())
}

pub async fn download_mod(
    version_id: String,
    mods_dir: &Path,
    client: &reqwest::Client,
) -> Result<ModListEntry, Error> {
    let version = fetch_json(client, &format!("https://api.modrinth.com/v2/version/{version_id}"))
        .send::<Version>()
        .await?;
    download_version_file(version, mods_dir, client).await
}

pub fn install_modpack() {
    unimplemented!()
}

pub fn search_modpacks() {
    unimplemented!()
}
