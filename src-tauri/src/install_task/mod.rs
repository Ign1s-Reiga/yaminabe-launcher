mod fabric_like;
mod forge_noprofile;
mod forge_v1;
mod forge_v2;
mod vanilla;

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use log::info;
use serde::Deserialize;
use tauri::State;
use zip::ZipArchive;
use yaminabe_launcher_shared::datatypes::ModLoader;
use yaminabe_launcher_shared::error::Error;
use crate::{temp_dir, versions_dir, AppState};
use crate::http_utils::{download_from_maven, download_resource, fetch_json, get_resource_name, sha1_hex};

// ── Vanilla ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadJarMetadata {
    sha1: String,
    size: u32,
    url: String,
}

#[derive(Debug, Deserialize)]
struct VersionManifestJson {
    downloads: HashMap<String, DownloadJarMetadata>,
}

pub async fn ensure_vanilla(
    mc_version: &str,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let version_dir = versions_dir().join(mc_version);
    std::fs::create_dir_all(&version_dir)?;

    let client_path = version_dir.join(format!("{mc_version}.jar"));
    let client_manifest_path = version_dir.join(format!("{mc_version}.json"));

    if !client_manifest_path.exists() {
        let version_metadata = state.mc_versions.get()
            .ok_or_else(|| Error::Invalid("version manifest".to_string()))?
            .versions
            .iter().find(|v| v.id == mc_version)
            .unwrap();

        download_resource(&state.http_client, &version_metadata.manifest_url, client_manifest_path.clone()).await?
    }

    let text = std::fs::read_to_string(&client_manifest_path)?;
    let manifest = serde_json::from_str::<VersionManifestJson>(&text)?;

    if !client_path.exists() {
        let client_metadata = manifest.downloads.get("client").unwrap();
        download_resource(&state.http_client, &client_metadata.url, client_path).await?
    }

    info!("Downloaded vanilla Minecraft {mc_version}");

    vanilla::pre_download_libraries(mc_version, &state.http_client).await?;
    if let Err(e) = vanilla::pre_download_log_config(mc_version, &state.http_client).await {
        log::warn!("log config pre-download failed for {mc_version}: {e}");
    }
    Ok(())
}

// ── Fabric / Quilt ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct FabricLikeMetadata {
    url: String,
}

async fn ensure_fabric_like(
    loader: ModLoader,
    label: &str,
    meta_url: &str,
    version_id_prefix: &str,
    mc_version: &str,
    loader_version: &str,
    client: &reqwest::Client,
) -> Result<(), Error> {
    let installer_url = fetch_json::<Vec<FabricLikeMetadata>>(client, meta_url, &[], None)
        .await?
        .into_iter().next()
        .ok_or_else(|| Error::Invalid(format!("No {label} installer metadata available")))?
        .url;

    let temp_installer_path = temp_dir().join(get_resource_name(&installer_url).unwrap_or_default());
    download_resource(client, &installer_url, temp_installer_path).await?;

    fabric_like::run_installer(&loader, &installer_url, mc_version, loader_version, client).await?;

    let version_id = format!("{version_id_prefix}-loader-{loader_version}-{mc_version}");
    fabric_like::pre_download_libraries(&version_id, client).await?;

    info!("Installed {label} {loader_version} for MC {mc_version}");
    Ok(())
}

pub async fn ensure_fabric(
    mc_version: &str,
    loader_version: &str,
    client: &reqwest::Client,
) -> Result<(), Error> {
    ensure_fabric_like(
        ModLoader::Fabric,
        "Fabric",
        "https://meta.fabricmc.net/v2/versions/installer",
        "fabric",
        mc_version,
        loader_version,
        client,
    ).await
}

pub async fn ensure_quilt(
    mc_version: &str,
    loader_version: &str,
    client: &reqwest::Client,
) -> Result<(), Error> {
    ensure_fabric_like(
        ModLoader::Quilt,
        "Quilt",
        "https://meta.quiltmc.org/v3/versions/installer",
        "quilt",
        mc_version,
        loader_version,
        client,
    ).await
}

// ── Forge ─────────────────────────────────────────────────────────────────────

fn is_old_format(mc_version: &str) -> bool {
    let parts: Vec<u32> = mc_version.split('.').filter_map(|p| p.parse().ok()).collect();
    matches!(parts.as_slice(), [1, minor, ..] if *minor <= 5)
}

/// Naming convention tier for Forge based on MC version:
/// - `Modern` (1.12+): version_id `{mc}-forge-{loader}`, maven `{mc}-{loader}`
/// - `Old` (1.8 – 1.11.2): version_id `{mc}-forge{mc}-{loader}`, maven `{mc}-{loader}-{mc}`
/// - `VeryOld` (1.7.10 and earlier): version_id `{mc}-Forge{loader}-{mc}`, maven `{mc}-{loader}-{mc}`
pub enum ForgeNaming { Modern, Old, VeryOld }

pub fn forge_naming(mc_version: &str) -> ForgeNaming {
    let parts: Vec<u32> = mc_version.split('.').filter_map(|p| p.parse().ok()).collect();
    let minor = parts.get(1).copied().unwrap_or(0);
    if minor >= 12 { ForgeNaming::Modern }
    else if minor >= 8 { ForgeNaming::Old }
    else { ForgeNaming::VeryOld }
}

/// Forge maven artifact version (the `<ver>` in `net.minecraftforge:forge:<ver>`
/// and in the file path `forge/<ver>/forge-<ver>-installer.jar`).
pub fn forge_maven_version(mc_version: &str, loader_version: &str) -> String {
    match forge_naming(mc_version) {
        ForgeNaming::Modern => format!("{mc_version}-{loader_version}"),
        ForgeNaming::Old | ForgeNaming::VeryOld => format!("{mc_version}-{loader_version}-{mc_version}"),
    }
}

/// Launcher-side version id (the folder name under `versions/`) matching the
/// `id` that each era's Forge installer writes into its own `install_profile.json`.
pub fn forge_version_id(mc_version: &str, loader_version: &str) -> String {
    match forge_naming(mc_version) {
        ForgeNaming::Modern => format!("{mc_version}-forge-{loader_version}"),
        ForgeNaming::Old => format!("{mc_version}-forge{mc_version}-{loader_version}"),
        ForgeNaming::VeryOld => format!("{mc_version}-Forge{loader_version}-{mc_version}"),
    }
}

pub async fn ensure_forge(
    mc_version: &str,
    loader_version: &str,
    client: &reqwest::Client,
) -> Result<(), Error> {
    let forge_build = loader_version.strip_prefix("forge-").unwrap_or(loader_version);
    let forge_version = forge_maven_version(mc_version, forge_build);

    let (install_type, installer_path) = if is_old_format(mc_version) {
        download_from_maven(
            client,
            "https://maven.minecraftforge.net/",
            format!("net.minecraftforge:forge:{forge_version}"),
            Some("universal"),
            "zip",
            temp_dir().clone(),
        ).await?;
        let path = temp_dir()
            .join("net").join("minecraftforge").join("forge")
            .join(&forge_version)
            .join(format!("forge-{forge_version}-universal.zip"));
        (ForgeInstallType::NoProfile, path)
    } else {
        download_from_maven(
            client,
            "https://maven.minecraftforge.net/",
            format!("net.minecraftforge:forge:{forge_version}"),
            Some("installer"),
            "jar",
            temp_dir().clone(),
        ).await?;
        let path = temp_dir()
            .join("net").join("minecraftforge").join("forge")
            .join(&forge_version)
            .join(format!("forge-{forge_version}-installer.jar"));
        let install_type = detect_install_type(&path)?;
        (install_type, path)
    };

    match install_type {
        ForgeInstallType::NoProfile => {
            let version_id = forge_version_id(mc_version, forge_build);
            forge_noprofile::install(&installer_path, &forge_version, &version_id)?;
        }
        ForgeInstallType::V1 => {
            let version_id = read_v1_version(&installer_path)?;
            if versions_dir().join(&version_id).exists() {
                info!("Forge {forge_build} already installed, skipping");
                return Ok(());
            }
            forge_v1::install(&installer_path, client).await?;
        }
        ForgeInstallType::V2 => {
            let version_id = forge_version_id(mc_version, forge_build);
            if versions_dir().join(&version_id).exists() {
                info!("Forge {forge_build} already installed, skipping installer");
            } else {
                forge_v2::install(&ModLoader::Forge, &installer_path, client).await?;
            }
            forge_v2::pre_download_libraries(&version_id, client).await?;
        }
    }

    info!("Installed Forge {forge_build} for MC {mc_version}");
    Ok(())
}

// ── NeoForge ──────────────────────────────────────────────────────────────────

pub async fn ensure_neoforge(
    mc_version: &str,
    loader_version: &str,
    client: &reqwest::Client,
) -> Result<(), Error> {
    let nf_version = loader_version.strip_prefix("neoforge-").unwrap_or(loader_version);
    let version_id = format!("neoforge-{nf_version}");

    // The version dir's existence only proves install was started, not that
    // client.json libraries were all downloaded. Skip the installer phase
    // when already present but always run pre_download_libraries.
    if versions_dir().join(&version_id).exists() {
        info!("NeoForge {nf_version} already installed, skipping installer");
    } else {
        download_from_maven(
            client,
            "https://maven.neoforged.net/releases/",
            format!("net.neoforged:neoforge:{nf_version}"),
            Some("installer"),
            "jar",
            temp_dir().clone(),
        ).await?;

        let installer_path = temp_dir()
            .join("net").join("neoforged").join("neoforge")
            .join(&nf_version)
            .join(format!("neoforge-{nf_version}-installer.jar"));

        forge_v2::install(&ModLoader::NeoForge, &installer_path, client).await?;
    }
    forge_v2::pre_download_libraries(&version_id, client).await?;

    info!("Installed NeoForge {nf_version} for MC {mc_version}");
    Ok(())
}

// ── Shared helpers (visible to submodules) ───────────────────────────────────

enum ForgeInstallType {
    NoProfile,
    V1,
    V2,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallProfileV1 {
    version_info: VersionInfoV1,
}

#[derive(Deserialize)]
struct VersionInfoV1 {
    id: String,
}

fn detect_install_type(installer_path: &Path) -> Result<ForgeInstallType, Error> {
    let mut zip = ZipArchive::new(std::fs::File::open(installer_path)?)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    if zip.by_name("install_profile.json").is_err() {
        return Ok(ForgeInstallType::NoProfile);
    }
    Ok(if zip.by_name("version.json").is_ok() {
        ForgeInstallType::V2
    } else {
        ForgeInstallType::V1
    })
}

fn read_v1_version(installer_path: &Path) -> Result<String, Error> {
    let mut zip = ZipArchive::new(std::fs::File::open(installer_path)?)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let mut entry = zip.by_name("install_profile.json")
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let mut buf = String::new();
    entry.read_to_string(&mut buf)?;
    Ok(serde_json::from_str::<InstallProfileV1>(&buf)?.version_info.id)
}

/// Convert a Maven coordinate (`group:artifact:version[:classifier][@ext]`) into a relative path.
fn maven_coord_to_path(coord: &str) -> PathBuf {
    let (coord, ext) = coord.rsplit_once('@').unwrap_or((coord, "jar"));
    let mut parts = coord.splitn(4, ':');
    let group = parts.next().unwrap_or_default();
    let artifact = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    let filename = match parts.next() {
        Some(cls) => format!("{artifact}-{version}-{cls}.{ext}"),
        None => format!("{artifact}-{version}.{ext}"),
    };
    group.split('.').collect::<PathBuf>().join(artifact).join(version).join(filename)
}

