use std::path::Path;
use serde::{Deserialize, Serialize};
use tauri::State;
use yaminabe_launcher_shared::datatypes::{GameVersion, LoaderVersion, ReleaseType};
use yaminabe_launcher_shared::error::Error;
use crate::AppState;
use crate::http_utils::fetch_json;

const VERSION_MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Debug, Deserialize)]
pub struct LatestVersion {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Deserialize)]
pub struct VersionManifest {
    pub latest: LatestVersion,
    pub versions: Vec<VersionMetadata>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionMetadata {
    pub id: String,
    #[serde(rename = "type")]
    pub rel_type: ReleaseType,
    #[serde(rename = "url")]
    pub manifest_url: String,
    pub release_time: String,
    pub sha1: String,
}

pub async fn fetch_minecraft_versions(
    versions_dir: &Path,
    client: &reqwest::Client,
) -> Result<VersionManifest, Error> {
    let cache_path = versions_dir.join("version_manifest.json");
    let lm_path = versions_dir.join("version_manifest.json.lastmodified");

    let mut req = client.get(VERSION_MANIFEST_URL);

    if cache_path.exists() {
        if let Ok(lm) = std::fs::read_to_string(&lm_path) {
            let lm = lm.trim().to_string();
            if !lm.is_empty() {
                req = req.header("If-Modified-Since", lm);
            }
        }
    }

    let resp = req.send().await?;

    let text = if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        std::fs::read_to_string(&cache_path)?
    } else {
        if !resp.status().is_success() {
            return Err(Error::HttpRequestRejected(resp.status().as_u16(), VERSION_MANIFEST_URL.to_string()));
        }

        let last_modified = resp.headers()
            .get("last-modified")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let text = resp.text().await?;

        std::fs::write(&cache_path, &text)?;
        if let Some(lm) = last_modified {
            std::fs::write(&lm_path, lm)?;
        }
        text
    };
    Ok(serde_json::from_str::<VersionManifest>(&text)?)
}

pub async fn fetch_loader_versions(
    kind: &str,
    mc_version: &str,
    client: &reqwest::Client,
) -> Result<Vec<LoaderVersion>, Error> {
    #[derive(Deserialize)]
    struct Response {
        loaders: Vec<LoaderVersion>,
    }
    let url = format!("https://api.feed-the-beast.com/v1/modpacks/loaders/{mc_version}/{kind}");
    Ok(fetch_json::<Response>(client, &url, &[], None).await?.loaders)
}

#[tauri::command]
pub async fn get_minecraft_versions(state: State<'_, AppState>) -> Result<Vec<GameVersion>, Error> {
    match state.mc_versions.get() {
        Some(manifest) => Ok(manifest.versions.iter().map(|v| GameVersion {
            id: 0,
            version_string: v.id.clone(),
            release_type: v.rel_type.clone(),
        }).collect()),
        None => Ok(vec![]),
    }
}

#[tauri::command]
pub async fn get_modloader_versions(
    kind: String,
    mc_version: String,
    state: State<'_, AppState>,
) -> Result<Vec<LoaderVersion>, Error> {
    fetch_loader_versions(&kind, &mc_version, &state.http_client).await
}
