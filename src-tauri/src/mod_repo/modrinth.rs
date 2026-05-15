use std::path::PathBuf;
use std::sync::Arc;

use log::info;
use yaminabe_launcher_shared::error::Error;

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
    let versions: serde_json::Value = client
        .get("https://api.modrinth.com/v2/versions")
        .query(&[("ids", ids_json.as_str())])
        .send().await?
        .json().await
        .map_err(|e| Error::InvalidResponse(e))?;

    let semaphore = Arc::new(tokio::sync::Semaphore::new(3));
    let mut handles: Vec<tokio::task::JoinHandle<Result<(), Error>>> = Vec::new();

    for version in versions.as_array().unwrap_or(&vec![]) {
        let files = version["files"].as_array();
        let file = files.and_then(|f| {
            f.iter().find(|e| e["primary"].as_bool() == Some(true)).or_else(|| f.first())
        });
        let Some(file) = file else { continue };

        let url      = file["url"].as_str().unwrap_or_default().to_string();
        let filename = file["filename"].as_str().unwrap_or_default().to_string();
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