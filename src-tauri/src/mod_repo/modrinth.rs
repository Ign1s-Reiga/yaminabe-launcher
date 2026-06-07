use std::path::PathBuf;

use serde::Deserialize;
use yaminabe_launcher_shared::datatypes::{DownloadSource, ModListEntry};
use yaminabe_launcher_shared::error::Error;
use crate::http_utils::fetch_json;

#[derive(Deserialize)]
struct ModrinthVersion {
    id: String,
    project_id: String,
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

pub async fn download_mods(
    version_ids: &[String],
    instance_location: &str,
    client: &reqwest::Client,
) -> Result<Vec<ModListEntry>, Error> {
    if version_ids.is_empty() {
        return Ok(vec![]);
    }

    let mods_dir = PathBuf::from(instance_location).join("mods");

    let ids_json = serde_json::to_string(version_ids)?;
    let versions = fetch_json(client, "https://api.modrinth.com/v2/versions")
        .query(&[("ids", ids_json.as_str())])
        .send::<Vec<ModrinthVersion>>()
        .await?;

    // One file per version: prefer the primary, else the first listed.
    let mut files = Vec::new();
    let mut entries = Vec::new();
    for version in versions {
        let Some(file) = version.files.iter().find(|f| f.primary).or_else(|| version.files.first()) else {
            continue;
        };
        files.push((file.url.clone(), file.filename.clone()));
        entries.push((
            file.filename.clone(),
            file.hashes.sha1.clone(),
            version.project_id.parse::<u32>().unwrap_or(0),
            version.id.parse::<u32>().unwrap_or(0),
        ));
    }

    let downloaded = super::download_files(client, files, &mods_dir).await?;
    let downloaded_sha1_by_name: std::collections::HashMap<String, String> = downloaded.into_iter()
        .map(|file| (file.file_name, file.sha1))
        .collect();
    let modlist = entries.into_iter()
        .map(|(file_name, api_sha1, project_id, file_id)| {
            let sha1 = if api_sha1.is_empty() {
                downloaded_sha1_by_name.get(&file_name).cloned().unwrap_or_default()
            } else {
                api_sha1
            };
            ModListEntry {
                file_name,
                sha1,
                project_id,
                file_id,
                download_source: DownloadSource::Modrinth,
            }
        })
        .collect();
    Ok(modlist)
}
