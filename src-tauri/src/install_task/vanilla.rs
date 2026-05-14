use log::info;
use serde::Deserialize;
use yaminabe_launcher_shared::error::Error;
use crate::{assets_dir, libraries_dir, versions_dir};
use crate::http_utils::download_resource;

#[derive(Deserialize)]
struct VersionJson {
    libraries: Vec<VersionLibrary>,
}

#[derive(Deserialize)]
struct VersionLibrary {
    #[serde(default)]
    downloads: Option<VersionLibraryDownloads>,
    #[serde(default)]
    rules: Vec<LibRule>,
}

#[derive(Deserialize)]
struct VersionLibraryDownloads {
    artifact: Option<VersionLibraryArtifact>,
}

#[derive(Deserialize)]
struct VersionLibraryArtifact {
    path: String,
    url: String,
}

#[derive(Deserialize)]
struct LibRule {
    action: String,
    #[serde(default)]
    os: LibRuleOs,
}

#[derive(Deserialize, Default)]
struct LibRuleOs {
    name: Option<String>,
}

fn os_allowed(rules: &[LibRule]) -> bool {
    if rules.is_empty() { return true; }
    let mut result = false;
    for rule in rules {
        let os_ok = rule.os.name.as_deref().map_or(true, |n| n == "windows");
        if os_ok { result = rule.action == "allow"; }
    }
    result
}

#[derive(Deserialize)]
struct LogConfigJson {
    #[serde(default)]
    logging: Option<LoggingSection>,
}

#[derive(Deserialize)]
struct LoggingSection {
    client: Option<LoggingClient>,
}

#[derive(Deserialize)]
struct LoggingClient {
    file: LoggingFile,
}

#[derive(Deserialize)]
struct LoggingFile {
    id: String,
    url: String,
}

pub async fn pre_download_libraries(version_id: &str, client: &reqwest::Client) -> Result<(), Error> {
    let version_json_path = versions_dir().join(version_id).join(format!("{version_id}.json"));
    let text = std::fs::read_to_string(&version_json_path)?;
    let version = serde_json::from_str::<VersionJson>(&text)?;

    for lib in &version.libraries {
        if !os_allowed(&lib.rules) { continue; }
        let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) else {
            continue;
        };
        let dest = libraries_dir().join(&artifact.path);
        if dest.exists() { continue; }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        download_resource(client, &artifact.url, dest).await?;
    }

    info!("Pre-downloaded libraries for {version_id}");
    Ok(())
}

pub async fn pre_download_log_config(version_id: &str, client: &reqwest::Client) -> Result<(), Error> {
    let version_json_path = versions_dir().join(version_id).join(format!("{version_id}.json"));
    let text = std::fs::read_to_string(&version_json_path)?;
    let parsed = serde_json::from_str::<LogConfigJson>(&text)?;
    let Some(file) = parsed.logging.and_then(|l| l.client).map(|c| c.file) else {
        return Ok(());
    };

    let dest_dir = assets_dir().join("log_configs");
    let dest = dest_dir.join(&file.id);
    if dest.exists() { return Ok(()); }
    std::fs::create_dir_all(&dest_dir)?;
    download_resource(client, &file.url, dest).await?;

    info!("Pre-downloaded log config {} for {version_id}", file.id);
    Ok(())
}