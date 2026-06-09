mod modloader;
mod vanilla;
pub mod installer_archive;

pub use modloader::{ensure_fabric, ensure_forge, ensure_neoforge, ensure_quilt, version_manifest_path};

use std::collections::HashSet;
use std::path::PathBuf;
use log::info;
use tauri::State;
use yaminabe_launcher_shared::datatypes::ModLoader;
use yaminabe_launcher_shared::error::Error;
use yaminabe_launcher_shared::version_manifest::ClientManifest;
use crate::{emit_progress, libraries_dir, versions_dir, AppState};
use crate::http_utils::download_resource;
use crate::maven::MavenCoords;

// ── Vanilla ───────────────────────────────────────────────────────────────────

pub async fn ensure_vanilla(
    mc_version: &str,
    state: &State<'_, AppState>,
) -> Result<String, Error> {
    let version_dir = versions_dir().join(mc_version);
    std::fs::create_dir_all(&version_dir)?;

    let client_path = version_dir.join(format!("{mc_version}.jar"));
    let client_manifest_path = version_dir.join(format!("{mc_version}.json"));

    if !client_manifest_path.exists() {
        let version_metadata = state.mc_versions.get()
            .ok_or_else(|| Error::Invalid("version manifest cache not yet populated".to_string()))?
            .versions
            .iter().find(|v| v.id == mc_version)
            .ok_or_else(|| Error::Invalid(format!(
                "Minecraft version '{mc_version}' not present in the Mojang version manifest"
            )))?;

        download_resource(&state.http_client, &version_metadata.manifest_url, &version_metadata.sha1, client_manifest_path.clone()).await?
    }

    let text = std::fs::read_to_string(&client_manifest_path)?;
    let manifest = serde_json::from_str::<ClientManifest>(&text)?;

    if !client_path.exists() {
        let client_metadata = manifest.downloads.get("client")
            .ok_or_else(|| Error::Invalid(format!(
                "version manifest for '{mc_version}' has no client download entry"
            )))?;
        download_resource(&state.http_client, &client_metadata.url, &client_metadata.sha1, client_path).await?
    }

    info!("Downloaded vanilla Minecraft {mc_version}");

    vanilla::pre_download_libraries(mc_version, &state.http_client).await?;
    if let Err(e) = vanilla::pre_download_log_config(mc_version, &state.http_client).await {
        log::warn!("log config pre-download failed for {mc_version}: {e}");
    }
    Ok(mc_version.to_string())
}

/// Ensure the vanilla client and (if any) the mod loader for `mc_version` are
/// installed, returning the `versions/<id>` the launch path should use. Shared
/// by instance creation, modpack install, and modpack upgrade.
pub async fn ensure_game_and_loader(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    mc_version: &str,
    mod_loader: &ModLoader,
    loader_version: &Option<String>,
    state: &State<'_, AppState>,
) -> Result<String, Error> {
    let http_client = &state.http_client;
    emit_progress(app_handle, id, instance_name, &format!("Downloading Minecraft {mc_version}"), false, None);
    let vanilla_version_id = ensure_vanilla(mc_version, state).await?;

    let require_loader_version = || loader_version.as_deref()
        .ok_or_else(|| Error::Invalid(format!("Mod loader version required for {mod_loader}")));
    let loader_version_id = match mod_loader {
        ModLoader::Fabric => {
            emit_progress(app_handle, id, instance_name, "Installing Fabric", false, None);
            Some(ensure_fabric(mc_version, require_loader_version()?, http_client).await?)
        }
        ModLoader::Quilt => {
            emit_progress(app_handle, id, instance_name, "Installing Quilt", false, None);
            Some(ensure_quilt(mc_version, require_loader_version()?, http_client).await?)
        }
        ModLoader::Forge => {
            emit_progress(app_handle, id, instance_name, "Installing Forge", false, None);
            Some(ensure_forge(mc_version, require_loader_version()?, http_client).await?)
        }
        ModLoader::NeoForge => {
            emit_progress(app_handle, id, instance_name, "Installing NeoForge", false, None);
            Some(ensure_neoforge(mc_version, require_loader_version()?, http_client).await?)
        }
        ModLoader::Vanilla => None,
    };
    // The loader's own version manifest takes precedence; Vanilla instances
    // fall back to the bare MC id.
    Ok(loader_version_id.unwrap_or(vanilla_version_id))
}

// ── Shared library pre-download (install + launch) ────────────────────────────

fn load_lib_manifest(version_id: &str) -> Result<ClientManifest, Error> {
    let text = std::fs::read_to_string(version_manifest_path(version_id))?;
    Ok(serde_json::from_str(&text)?)
}

/// Download every classpath library referenced by `versions/<id>/<id>.json`
/// (and, one level up, its `inheritsFrom` parent) that is not already on disk.
///
/// Handles both manifest schemas: modern entries carry `downloads.artifact`
/// with an explicit path + URL; legacy (pre-1.13 Forge) entries are bare maven
/// coordinates whose jar lives at `<url base>/<maven path>`, defaulting to the
/// Mojang library repo when no `url` is given.
///
/// Native (`natives`-bearing) entries are skipped — their classifier jars are
/// unpacked separately and never go on the classpath. Entries already present
/// on disk are skipped, so this is safe to call repeatedly (it runs again
/// before launch to backfill anything an install left behind).
pub async fn ensure_libraries(version_id: &str, client: &reqwest::Client) -> Result<(), Error> {
    const DEFAULT_MAVEN: &str = "https://libraries.minecraft.net/";

    let mut manifests = vec![load_lib_manifest(version_id)?];
    if let Some(parent) = manifests[0].inherits_from.clone() {
        manifests.push(load_lib_manifest(&parent)?);
    }

    let mut seen: HashSet<String> = HashSet::new();
    for manifest in &manifests {
        for lib in &manifest.libraries {
            if !seen.insert(lib.name.clone()) { continue; }

            // Pre-1.13 natives: the DLLs/SOs live in a classifier jar that
            // `extract_natives` later unpacks; missing here = empty
            // java.library.path = LWJGL/JNI failure at runtime.
            if let Some(key) = lib.natives.get("windows").map(|s| s.replace("${arch}", "64")) {
                let Some(classifiers) = lib.downloads.as_ref().and_then(|d| d.classifiers.as_ref()) else {
                    continue;
                };
                let Some(artifact) = classifiers.get(&key) else { continue; };
                if artifact.url.is_empty() { continue; }
                let Some(path) = artifact.path.as_deref() else { continue; };
                let dest = libraries_dir().join(path);
                if dest.exists() { continue; }
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                download_resource(client, &artifact.url, &artifact.sha1, dest).await?;
                continue;
            }

            if let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
                if artifact.url.is_empty() { continue; }
                let Some(path) = artifact.path.as_deref() else { continue; };
                let dest = libraries_dir().join(path);
                if dest.exists() { continue; }
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                download_resource(client, &artifact.url, &artifact.sha1, dest).await?;
            } else {
                let dest = libraries_dir().join(maven_coord_to_path(&lib.name));
                if dest.exists() { continue; }
                let base = lib.url.as_deref().filter(|u| !u.is_empty()).unwrap_or(DEFAULT_MAVEN);
                MavenCoords::new(base, &lib.name).download(client, libraries_dir()).await?;
            }
        }
    }

    info!("Ensured libraries for {version_id}");
    Ok(())
}

/// Convert a Maven coordinate (`group:artifact:version[:classifier][@ext]`)
/// into the relative path used under `libraries/`. Lenient: missing parts
/// become empty path segments rather than an error, since most callers want
/// "build a best-effort path, then check if it exists on disk".
pub fn maven_coord_to_path(coord: &str) -> PathBuf {
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
