use log::info;
use serde::Deserialize;
use yaminabe_launcher_shared::datatypes::ModLoader;
use yaminabe_launcher_shared::error::Error;
use crate::{bin_dir, libraries_dir, temp_dir, versions_dir};
use crate::http_utils::{download_from_maven, get_resource_name};

#[derive(Deserialize)]
struct VersionJson {
    libraries: Vec<VersionLibrary>,
}

#[derive(Deserialize)]
struct VersionLibrary {
    name: String,
    #[serde(rename = "url")]
    repo_url: String,
}

pub async fn run_installer(
    loader: &ModLoader,
    installer_url: &str,
    mc_version: &str,
    loader_version: &str,
    client: &reqwest::Client,
) -> Result<(), Error> {
    let expected_sha1 = client.get(format!("{installer_url}.sha1"))
        .send().await?
        .text().await.map_err(Error::InvalidResponse)?;

    let file_name = get_resource_name(installer_url)
        .ok_or_else(|| Error::Invalid(format!("Cannot determine filename from URL: {installer_url}")))?;
    let temp_path = temp_dir().join(file_name);

    let bytes = std::fs::read(&temp_path)?;
    let hex = super::sha1_hex(&bytes);
    if hex != expected_sha1.trim() {
        std::fs::remove_file(&temp_path).ok();
        return Err(Error::ChecksumMismatch {
            resource: installer_url.to_string(),
            sha1: expected_sha1,
            hex,
        });
    }

    let status = tokio::process::Command::new("java")
        .args([
            "-jar", &temp_path.to_string_lossy(),
            "client",
            "-dir", &bin_dir().to_string_lossy(),
            "-mcversion", mc_version,
            "-loader", loader_version,
            "-noprofile"
        ])
        .status().await
        .map_err(|e| Error::ChildProcess(format!("[{loader}] running installer: {e}")))?;

    std::fs::remove_file(&temp_path).ok();

    if !status.success() {
        return Err(Error::ChildProcess(format!("[{loader}] installer exited with {status}")));
    }

    Ok(())
}

pub async fn pre_download_libraries(version_id: &str, client: &reqwest::Client) -> Result<(), Error> {
    let version_json_path = versions_dir().join(version_id).join(format!("{version_id}.json"));
    let text = std::fs::read_to_string(&version_json_path)?;
    let version = serde_json::from_str::<VersionJson>(&text)?;

    for lib in &version.libraries {
        let dest = libraries_dir().join(super::maven_coord_to_path(&lib.name));
        if dest.exists() { continue; }
        download_from_maven(client, &lib.repo_url, lib.name.clone(), None, "jar", libraries_dir().clone()).await?;
    }

    info!("Pre-downloaded libraries for {version_id}");
    Ok(())
}
