mod fabric_like;
mod forge_v1;
mod forge_v2;

use super::installer_archive;
use crate::http_utils::fetch_json;
use crate::maven::MavenCoords;
use crate::{temp_dir, versions_dir};
use log::info;
use serde::Deserialize;
use std::path::PathBuf;
use yaminabe_launcher_shared::datamodels::ModLoader;
use yaminabe_launcher_shared::error::Error;

// ── Path helpers (visible to submodules and other commands) ──────────────────

/// The on-disk path of `versions/<version_id>/<version_id>.json` — the
/// canonical Mojang version manifest for a given installed version. Used by
/// every consumer that reads or writes a per-version JSON, which is many.
pub fn version_manifest_path(version_id: &str) -> PathBuf {
    versions_dir()
        .join(version_id)
        .join(format!("{version_id}.json"))
}

#[derive(Debug, Deserialize)]
struct FabricLikeMetadata {
    maven: String,
}

async fn ensure_fabric_like(
    loader: ModLoader,
    meta_url: &str,
    maven_url: &str,
    mc_version: &str,
    loader_version: &str,
    client: &reqwest::Client,
) -> Result<String, Error> {
    let installer_coords = fetch_json(client, &format!("{meta_url}installer/"))
        .send::<Vec<FabricLikeMetadata>>().await?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Invalid(format!("No {loader} installer metadata available")))?
        .maven;

    let installer_path = MavenCoords::new(maven_url, &installer_coords)
        .download(client, temp_dir()).await?;

    fabric_like::run_installer(&loader, installer_path, mc_version, loader_version).await?;
    let version_id = fetch_json(
        client,
        &format!("{meta_url}loader/{mc_version}/{loader_version}/profile/json/"),
    ).send::<VersionInfo>().await?.id;
    fabric_like::pre_download_libraries(&version_id, client).await?;

    info!("Installed {loader} - {loader_version} for MC {mc_version}");
    Ok(version_id)
}

pub async fn ensure_fabric(
    mc_version: &str,
    loader_version: &str,
    client: &reqwest::Client,
) -> Result<String, Error> {
    ensure_fabric_like(
        ModLoader::Fabric,
        "https://meta.fabricmc.net/v2/versions/",
        "https://maven.fabricmc.net/",
        mc_version,
        loader_version,
        client,
    ).await
}

pub async fn ensure_quilt(
    mc_version: &str,
    loader_version: &str,
    client: &reqwest::Client,
) -> Result<String, Error> {
    ensure_fabric_like(
        ModLoader::Quilt,
        "https://meta.quiltmc.org/v3/versions/",
        "https://maven.quiltmc.org/repository/release/",
        mc_version,
        loader_version,
        client,
    ).await
}

/// Forge maven artifact version (the `<ver>` in `net.minecraftforge:forge:<ver>`
/// and in the file path `forge/<ver>/forge-<ver>-installer.jar`).
///
/// Always `{mc}-{loader_version}`. The post-`{mc}-` portion of the maven
/// version (e.g. `-mc172`, `-1.7.10`, `-1.10.0`, or none) varies per era and
/// is carried by `loader_version` itself; do not try to re-append it here.
fn forge_maven_version(mc_version: &str, loader_version: &str) -> String {
    format!("{mc_version}-{loader_version}")
}

pub async fn ensure_forge(
    mc_version: &str,
    loader_version: &str,
    client: &reqwest::Client,
) -> Result<String, Error> {
    let forge_build = loader_version
        .strip_prefix("forge-")
        .unwrap_or(loader_version);
    let forge_version = forge_maven_version(mc_version, forge_build);

    // Pre-1.6 (jar-mod era) is unsupported; the installer download 404s for
    // those MC versions, surfacing as a clean network error.
    let installer_path = MavenCoords::new(
        "https://maven.minecraftforge.net/",
        &format!("net.minecraftforge:forge:{forge_version}"),
    ).resource_suffix("installer")
        .download(client, temp_dir()).await?;
    let install_type = detect_install_type(&installer_path)?;

    let version_id = match install_type {
        ForgeInstallType::V1 => {
            let version_id = read_v1_version(&installer_path)?;
            // `install_from_parsed` writes the universal jar before the
            // manifest, so a present manifest implies the jar is on disk too.
            if !version_manifest_path(&version_id).exists() {
                forge_v1::install(&installer_path, client).await?;
            } else {
                info!("Forge {forge_build} already installed, skipping");
            }
            version_id
        }
        ForgeInstallType::V2 => {
            let version_id = read_v2_version(&installer_path)?;
            if versions_dir().join(&version_id).exists() {
                info!("Forge {forge_build} already installed, skipping installer");
            } else {
                forge_v2::install(&ModLoader::Forge, &installer_path, client).await?;
            }
            forge_v2::pre_download_libraries(&version_id, client).await?;
            version_id
        }
    };

    info!("Installed Forge {forge_build} for MC {mc_version}");
    Ok(version_id)
}

pub async fn ensure_neoforge(
    mc_version: &str,
    loader_version: &str,
    client: &reqwest::Client,
) -> Result<String, Error> {
    let nf_version = loader_version
        .strip_prefix("neoforge-")
        .unwrap_or(loader_version);

    // Download the installer once up front so we can read the authoritative
    // version_id out of its `version.json` before deciding to skip.
    let installer_path = MavenCoords::new(
        "https://maven.neoforged.net/releases/",
        &format!("net.neoforged:neoforge:{nf_version}"),
    ).resource_suffix("installer")
        .download(client, temp_dir()).await?;

    let version_id = read_v2_version(&installer_path)?;

    // Version-dir presence only proves install started; always run
    // pre_download_libraries to backfill anything the installer left out.
    if versions_dir().join(&version_id).exists() {
        info!("NeoForge {nf_version} already installed, skipping installer");
    } else {
        forge_v2::install(&ModLoader::NeoForge, &installer_path, client).await?;
    }
    forge_v2::pre_download_libraries(&version_id, client).await?;

    info!("Installed NeoForge {nf_version} for MC {mc_version}");
    Ok(version_id)
}

// ── Shared helpers (visible to submodules) ───────────────────────────────────

enum ForgeInstallType {
    V1,
    V2,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallProfileV1 {
    version_info: VersionInfo,
}

#[derive(Deserialize)]
struct VersionInfo {
    id: String,
}

fn detect_install_type(installer_path: &std::path::Path) -> Result<ForgeInstallType, Error> {
    let mut zip = installer_archive::open(installer_path)?;
    if zip.by_name("install_profile.json").is_err() {
        return Err(Error::Unsupported(format!(
            "Forge installer at {} has no install_profile.json — pre-1.6 jar-mod-era Forge is not currently supported",
            installer_path.display()
        )));
    }
    Ok(if zip.by_name("version.json").is_ok() {
        ForgeInstallType::V2
    } else {
        ForgeInstallType::V1
    })
}

fn read_v1_version(installer_path: &std::path::Path) -> Result<String, Error> {
    let text = installer_archive::read_entry_str(installer_path, "install_profile.json")?;
    Ok(serde_json::from_str::<InstallProfileV1>(&text)?.version_info.id)
}

fn read_v2_version(installer_path: &std::path::Path) -> Result<String, Error> {
    let text = installer_archive::read_entry_str(installer_path, "version.json")?;
    Ok(serde_json::from_str::<VersionInfo>(&text)?.id)
}
