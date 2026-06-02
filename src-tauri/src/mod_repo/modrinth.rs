use std::path::PathBuf;
use std::sync::Arc;

use log::info;
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
    std::fs::create_dir_all(&mods_dir)?;

    let ids_json = serde_json::to_string(version_ids)?;
    let versions = fetch_json(client, "https://api.modrinth.com/v2/versions")
        .query(&[("ids", ids_json.as_str())])
        .send::<Vec<ModrinthVersion>>()
        .await?;

    let semaphore = Arc::new(tokio::sync::Semaphore::new(3));
    let mut handles: Vec<tokio::task::JoinHandle<Result<(), Error>>> = Vec::new();

    for version in versions {
        // Prefer the primary file, falling back to the first listed.
        let file = version.files.iter().find(|f| f.primary).or_else(|| version.files.first());
        let Some(file) = file else { continue };
        let url = file.url.clone();
        let filename = file.filename.clone();
        if url.is_empty() || filename.is_empty() { continue }

        let client   = client.clone();
        let mods_dir = mods_dir.clone();
        let sem      = Arc::clone(&semaphore);

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await
                .map_err(|e| Error::ChildProcess(format!("semaphore acquire: {e}")))?;
            let resp = client.get(&url).send().await?;
            if !resp.status().is_success() {
                return Err(Error::HttpRequestRejected(resp.status().as_u16(), url));
            }
            let bytes = resp.bytes().await.map_err(Error::InvalidResponse)?;
            std::fs::write(mods_dir.join(&filename), &bytes)?;
            info!("Downloaded {filename}");
            Ok(())
        }));
    }

    for handle in handles {
        handle.await.map_err(|e| Error::ChildProcess(format!("download task panicked: {e}")))??;
    }

    Ok(())
}