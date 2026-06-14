use std::collections::HashMap;
use std::path::Path;
use serde::Deserialize;
use yaminabe_launcher_shared::error::Error;
use yaminabe_launcher_shared::datamodels::AssetIndex;
use crate::http_utils::{fetch_and_verify, sha1_hex};

#[derive(Deserialize)]
struct AssetIndexJson {
    objects: HashMap<String, AssetObject>,
}

#[derive(Deserialize)]
struct AssetObject {
    hash: String,
}

pub(super) async fn download_assets(
    assets_dir: &Path,
    asset_index: &AssetIndex,
    client: &reqwest::Client,
    mut log_progress: impl FnMut(String),
) -> Result<(), Error> {
    let indexes_dir = assets_dir.join("indexes");
    std::fs::create_dir_all(&indexes_dir)?;
    let asset_index_file_name = format!("{}.json", asset_index.id);
    let index_path = indexes_dir.join(&asset_index_file_name);

    let index_bytes = if index_path.exists() {
        log_progress(format!(
            "Assets: using cached asset index {} (file: {}).",
            asset_index.id, asset_index_file_name
        ));
        std::fs::read(&index_path)?
    } else {
        log_progress(format!(
            "Assets: downloading asset index {} (file: {}).",
            asset_index.id, asset_index_file_name
        ));
        let url = asset_index.url.as_ref()
            .ok_or_else(|| Error::Invalid(format!("asset index {} missing url", asset_index.id)))?;
        let sha1 = asset_index.sha1.as_ref()
            .ok_or_else(|| Error::Invalid(format!("asset index {} missing sha1", asset_index.id)))?;
        let bytes = fetch_and_verify(client, url, sha1, &format!("asset index {}", asset_index.id)).await?;
        std::fs::write(&index_path, &bytes)?;
        bytes
    };

    let parsed: AssetIndexJson = serde_json::from_slice(&index_bytes)?;
    let objects_dir = assets_dir.join("objects");
    let total_assets = parsed.objects.len();
    let mut checked_assets = 0usize;
    let mut cached_assets = 0usize;
    let mut downloaded_assets = 0usize;
    const ASSET_PROGRESS_INTERVAL: usize = 100;

    log_progress(format!("Verifying {total_assets} indexed asset files..."));

    for (path, object) in &parsed.objects {
        if object.hash.len() < 2 {
            return Err(Error::Invalid(format!("asset {path} has invalid hash {}", object.hash)));
        }
        let prefix = &object.hash[..2];
        let dest_dir = objects_dir.join(prefix);
        let dest = dest_dir.join(&object.hash);
        let needs_download = if dest.exists() {
            match std::fs::read(&dest) {
                Ok(bytes) => {
                    let hex = sha1_hex(&bytes);
                    if hex == object.hash {
                        cached_assets += 1;
                        false
                    } else {
                        log_progress(format!(
                            "Failed to verify '{path}' (Hash mismatch: expected {}, got {}).",
                            object.hash, hex
                        ));
                        true
                    }
                }
                Err(e) => {
                    log_progress(format!(
                        "Failed to read cached asset '{path}' ({e})."
                    ));
                    true
                }
            }
        } else {
            true
        };
        if needs_download {
            let url = format!("https://resources.download.minecraft.net/{prefix}/{}", object.hash);
            let bytes = match fetch_and_verify(client, &url, &object.hash, &format!("asset {path}")).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    log_progress(format!("Failed to download '{path}' ({e})."));
                    return Err(e);
                }
            };
            std::fs::create_dir_all(&dest_dir)?;
            std::fs::write(&dest, &bytes)?;
            downloaded_assets += 1;
        }

        checked_assets += 1;
        if checked_assets == total_assets || checked_assets % ASSET_PROGRESS_INTERVAL == 0 {
            let percent = if total_assets == 0 { 100 } else { checked_assets * 100 / total_assets };
            log_progress(format!(
                "Downloading Assets: {percent}% ({checked_assets}/{total_assets}); {downloaded_assets} downloaded, {cached_assets} verified."
            ));
        }
    }
    if total_assets == 0 {
        log_progress("Asset verification complete. Asset index contained no files.".to_string());
    } else if downloaded_assets == 0 {
        log_progress("Asset verification complete. All files matched.".to_string());
    } else {
        log_progress(format!(
            "Asset synchronization complete. {downloaded_assets} downloaded, {cached_assets} verified."
        ));
    }
    Ok(())
}