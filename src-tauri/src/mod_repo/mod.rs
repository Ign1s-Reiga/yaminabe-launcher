use std::path::Path;
use std::sync::Arc;

use log::info;
use reqwest::Client;
use yaminabe_launcher_shared::error::Error;
use crate::http_utils::sha1_hex;

pub mod curseforge;
pub mod modrinth;

/// Download each `(url, filename)` pair into `dest_dir`, at most three at a time.
/// Shared by the CurseForge and Modrinth mod downloaders. Creates `dest_dir` if
/// it doesn't exist; entries with an empty url or filename are skipped, and the
/// first failed (or panicked) download aborts the batch with that error.
#[derive(Debug, Clone)]
pub struct DownloadedFile {
    pub file_name: String,
    pub sha1: String,
}

pub async fn download_files(
    client: &Client,
    files: Vec<(String, String)>,
    dest_dir: &Path,
) -> Result<Vec<DownloadedFile>, Error> {
    if files.is_empty() {
        return Ok(vec![]);
    }
    std::fs::create_dir_all(dest_dir)?;

    let semaphore = Arc::new(tokio::sync::Semaphore::new(3));
    let mut handles: Vec<tokio::task::JoinHandle<Result<Option<DownloadedFile>, Error>>> = Vec::new();

    for (url, filename) in files {
        if url.is_empty() || filename.is_empty() {
            continue;
        }
        let client = client.clone();
        let dest_dir = dest_dir.to_path_buf();
        let sem = Arc::clone(&semaphore);

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await
                .map_err(|e| Error::ChildProcess(format!("semaphore acquire: {e}")))?;
            let resp = client.get(&url).send().await?;
            if !resp.status().is_success() {
                return Err(Error::HttpRequestRejected(resp.status().as_u16(), url));
            }
            let bytes = resp.bytes().await.map_err(Error::InvalidResponse)?;
            let sha1 = sha1_hex(&bytes);
            std::fs::write(dest_dir.join(&filename), &bytes)?;
            info!("Downloaded {filename}");
            Ok(Some(DownloadedFile { file_name: filename, sha1 }))
        }));
    }

    let mut downloaded = Vec::new();
    for handle in handles {
        if let Some(file) = handle.await
            .map_err(|e| Error::ChildProcess(format!("download task panicked: {e}")))??
        {
            downloaded.push(file);
        }
    }
    Ok(downloaded)
}
