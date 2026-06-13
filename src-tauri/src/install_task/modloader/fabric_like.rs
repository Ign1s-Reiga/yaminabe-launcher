use super::version_manifest_path;
use crate::install_task::maven_coord_to_path;
use crate::maven::MavenCoords;
use crate::{bin_dir, libraries_dir};
use log::info;
use std::path::PathBuf;
use yaminabe_launcher_shared::datatypes::ModLoader;
use yaminabe_launcher_shared::error::Error;
use yaminabe_launcher_shared::version_manifest::ClientManifest;

pub async fn run_installer(
    loader: &ModLoader,
    installer_path: PathBuf,
    mc_version: &str,
    loader_version: &str,
) -> Result<(), Error> {
    let status = tokio::process::Command::new("java")
        .args([
            "-jar", &installer_path.to_string_lossy(),
            "client",
            "-dir", &bin_dir().to_string_lossy(),
            "-mcversion", mc_version,
            "-loader", loader_version,
            "-noprofile",
        ])
        .status()
        .await
        .map_err(|e| Error::ChildProcess(format!("[{loader}] running installer: {e}")))?;
    std::fs::remove_file(&installer_path).ok();

    if !status.success() {
        return Err(Error::ChildProcess(format!("[{loader}] installer exited with {status}")));
    }

    Ok(())
}

pub async fn pre_download_libraries(
    version_id: &str,
    client: &reqwest::Client,
) -> Result<(), Error> {
    let version_json_path = version_manifest_path(version_id);
    let text = std::fs::read_to_string(&version_json_path)?;
    let version = serde_json::from_str::<ClientManifest>(&text)?;

    for lib in &version.libraries {
        // Fabric/Quilt `url` is the maven repo base for the bare `name`;
        // skip if missing — launch-time `ensure_libraries` covers those.
        let Some(repo_url) = lib.url.as_deref().filter(|u| !u.is_empty()) else {
            continue;
        };
        let dest = libraries_dir().join(maven_coord_to_path(&lib.name));
        if dest.exists() {
            continue;
        }
        MavenCoords::new(repo_url, &lib.name)
            .download(client, libraries_dir()).await?;
    }

    info!("Pre-downloaded libraries for {version_id}");
    Ok(())
}
