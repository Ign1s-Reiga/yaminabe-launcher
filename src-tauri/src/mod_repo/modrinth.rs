use std::path::PathBuf;

use serde::Deserialize;
use yaminabe_launcher_shared::error::Error;
use crate::http_utils::fetch_json;

#[derive(Deserialize)]
struct ModrinthVersion {
    #[serde(default)]
    files: Vec<ModrinthFile>,
}

#[derive(Deserialize)]
struct ModrinthFile {
    url: String,
    filename: String,
    #[serde(default)]
    primary: bool,
}

pub async fn download_mods(
    version_ids: &[String],
    instance_location: &str,
    client: &reqwest::Client,
) -> Result<(), Error> {
    if version_ids.is_empty() {
        return Ok(());
    }

    let mods_dir = PathBuf::from(instance_location).join("mods");

    let ids_json = serde_json::to_string(version_ids)?;
    let versions = fetch_json(client, "https://api.modrinth.com/v2/versions")
        .query(&[("ids", ids_json.as_str())])
        .send::<Vec<ModrinthVersion>>()
        .await?;

    // One file per version: prefer the primary, else the first listed.
    let files: Vec<(String, String)> = versions.into_iter().filter_map(|v| {
        let file = v.files.iter().find(|f| f.primary).or_else(|| v.files.first())?;
        Some((file.url.clone(), file.filename.clone()))
    }).collect();

    super::download_files(client, files, &mods_dir).await
}