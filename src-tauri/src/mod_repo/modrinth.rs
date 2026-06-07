use std::path::PathBuf;

use serde::Deserialize;
use yaminabe_launcher_shared::datatypes::{DownloadSource, ModListEntry};
use yaminabe_launcher_shared::error::Error;
use crate::http_utils::fetch_json;

#[derive(Deserialize)]
struct ModrinthVersion {
    id: String,
    #[serde(default)]
    files: Vec<ModrinthFile>,
}

#[derive(Deserialize)]
struct ModrinthFile {
    url: String,
    filename: String,
    #[serde(default)]
    hashes: ModrinthHashes,
    #[serde(default)]
    primary: bool,
}

#[derive(Default, Deserialize)]
struct ModrinthHashes {
    #[serde(default)]
    sha1: String,
}

fn registry_entry(file_name: String, sha1: String) -> ModListEntry {
    ModListEntry {
        file_name,
        sha1,
        project_id: 0,
        file_id: 0,
        download_source: DownloadSource::Modrinth,
    }
}

fn selected_file(version: &ModrinthVersion) -> Option<&ModrinthFile> {
    version.files.iter().find(|f| f.primary).or_else(|| version.files.first())
}

async fn download_version_file(
    version: &ModrinthVersion,
    target_file_name: Option<&str>,
    instance_location: &str,
    client: &reqwest::Client,
) -> Result<ModListEntry, Error> {
    let file = selected_file(version)
        .ok_or_else(|| Error::NotExists(format!("file for Modrinth version {}", version.id)))?;
    let file_name = target_file_name.unwrap_or(&file.filename).to_string();
    let mods_dir = PathBuf::from(instance_location).join("mods");
    let downloaded = super::download_files(client, vec![(file.url.clone(), file_name.clone())], &mods_dir).await?;
    let downloaded_sha1 = downloaded.first().map(|file| file.sha1.clone()).unwrap_or_default();
    let sha1 = if file.hashes.sha1.is_empty() {
        downloaded_sha1
    } else {
        file.hashes.sha1.clone()
    };
    Ok(registry_entry(file_name, sha1))
}

pub async fn download_mod_by_sha1(
    sha1: &str,
    target_file_name: &str,
    instance_location: &str,
    client: &reqwest::Client,
) -> Result<ModListEntry, Error> {
    let version = fetch_json(client, &format!("https://api.modrinth.com/v2/version_file/{sha1}"))
        .query(&[("algorithm", "sha1")])
        .send::<ModrinthVersion>()
        .await?;
    download_version_file(&version, Some(target_file_name), instance_location, client).await
}

pub async fn download_mods(
    version_ids: &[String],
    instance_location: &str,
    client: &reqwest::Client,
) -> Result<Vec<ModListEntry>, Error> {
    if version_ids.is_empty() {
        return Ok(vec![]);
    }

    let ids_json = serde_json::to_string(version_ids)?;
    let versions = fetch_json(client, "https://api.modrinth.com/v2/versions")
        .query(&[("ids", ids_json.as_str())])
        .send::<Vec<ModrinthVersion>>()
        .await?;

    // One file per version: prefer the primary, else the first listed.
    let mut entries = Vec::new();
    for version in versions {
        let Some(file) = selected_file(&version) else {
            continue;
        };
        let entry = download_version_file(&version, None, instance_location, client)
            .await
            .map_err(|e| Error::Invalid(format!("Modrinth download failed for {}: {e}", file.filename)))?;
        entries.push(entry);
    }
    Ok(entries)
}
