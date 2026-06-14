use log::info;
use yaminabe_launcher_shared::error::Error;
use yaminabe_launcher_shared::datamodels::{ArgRule, ClientManifest};
use crate::{assets_dir, libraries_dir};
use crate::http_utils::download_resource;
use super::version_manifest_path;

/// Evaluate Mojang library `rules` for the current OS. Only the `os.name`
/// field is consulted here (vanilla never ships archi-/version-conditional
/// library rules); features and `versionRange` are ignored.
fn os_allowed(rules: &[ArgRule]) -> bool {
    if rules.is_empty() { return true; }
    let mut result = false;
    for rule in rules {
        let os_ok = rule.os.name.as_deref().map_or(true, |n| n == "windows");
        if os_ok { result = rule.action == "allow"; }
    }
    result
}

pub async fn pre_download_libraries(version_id: &str, client: &reqwest::Client) -> Result<(), Error> {
    let version_json_path = version_manifest_path(version_id);
    let text = std::fs::read_to_string(&version_json_path)?;
    let version = serde_json::from_str::<ClientManifest>(&text)?;

    for lib in &version.libraries {
        if !os_allowed(&lib.rules) { continue; }
        let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) else {
            continue;
        };
        let Some(path) = artifact.path.as_deref() else { continue; };
        download_resource(client, &artifact.url, &artifact.sha1, libraries_dir().join(path)).await?;
    }

    info!("Pre-downloaded libraries for {version_id}");
    Ok(())
}

pub async fn pre_download_log_config(version_id: &str, client: &reqwest::Client) -> Result<(), Error> {
    let version_json_path = version_manifest_path(version_id);
    let text = std::fs::read_to_string(&version_json_path)?;
    let parsed = serde_json::from_str::<ClientManifest>(&text)?;
    let Some(file) = parsed.logging.and_then(|l| l.client).map(|c| c.file) else {
        return Ok(());
    };

    let dest_dir = assets_dir().join("log_configs");
    download_resource(client, &file.url, &file.sha1, dest_dir.join(&file.id)).await?;

    info!("Pre-downloaded log config {} for {version_id}", file.id);
    Ok(())
}
