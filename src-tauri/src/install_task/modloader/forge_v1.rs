use std::path::Path;
use serde::Deserialize;
use yaminabe_launcher_shared::error::Error;
use yaminabe_launcher_shared::version_manifest::ClientManifest;
use crate::libraries_dir;
use crate::maven::MavenCoords;
use crate::install_task::{installer_archive, maven_coord_to_path};
use super::version_manifest_path;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallProfile {
    install: InstallInfo,
    version_info: ClientManifest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallInfo {
    file_path: String,
    path: String,
}

pub async fn install(installer_path: &Path, client: &reqwest::Client) -> Result<String, Error> {
    let profile: InstallProfile = {
        let text = installer_archive::read_entry_str(installer_path, "install_profile.json")?;
        serde_json::from_str(&text)?
    };
    let universal_jar_bytes = installer_archive::read_entry_bytes(installer_path, &profile.install.file_path)?;

    // V1 nests the version manifest inside install_profile.json, so re-serialize
    // it back out for on-disk storage.
    let version_info_text = serde_json::to_string(&profile.version_info)?;

    install_from_parsed(
        &profile.version_info,
        &version_info_text,
        &profile.install.path,
        &universal_jar_bytes,
        client,
    ).await
}

/// Install a pre-1.13-style Forge client from an already-parsed version
/// manifest plus the universal jar bytes.
///
/// The universal jar is written under `libraries/` at the `universal_coord`
/// maven path — it is the `net.minecraftforge:forge:<ver>` library, not a
/// version jar. Pre-1.13 Forge has no patched client jar: LaunchWrapper's
/// FMLTweaker applies the binpatches packed inside the universal jar at
/// runtime, against the vanilla client jar inherited via `inheritsFrom`.
///
/// The raw `version_info_text` is written to disk verbatim instead of being
/// re-serialized from `version_info`. This matters when the caller (e.g. the
/// V2-with-empty-processors path) hands us a manifest whose libraries use a
/// schema V1's `Library` struct does not understand — re-serializing through
/// V1's struct would silently drop those fields and break downstream
/// consumers that expect the original layout.
///
/// The library download loop tolerates entries with empty `url`: they are
/// skipped here and assumed to be handled by a downstream pass (V2 callers
/// rely on `forge_v2::pre_download_libraries` for the actual fetch).
pub async fn install_from_parsed(
    version_info: &ClientManifest,
    version_info_text: &str,
    universal_coord: &str,
    universal_jar_bytes: &[u8],
    client: &reqwest::Client,
) -> Result<String, Error> {
    let version_id = version_info.id.clone()
        .ok_or_else(|| Error::Invalid("Forge V1 version manifest is missing top-level `id`".to_string()))?;

    // Place the universal jar before the manifest: the caller gates re-install
    // on the manifest's existence, so a present manifest must imply the jar is
    // already on disk.
    let universal_path = libraries_dir().join(maven_coord_to_path(universal_coord));
    if !universal_path.exists() {
        if let Some(parent) = universal_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&universal_path, universal_jar_bytes)?;
    }

    let manifest_path = version_manifest_path(&version_id);
    if let Some(version_dir) = manifest_path.parent() {
        std::fs::create_dir_all(version_dir)?;
    }
    std::fs::write(&manifest_path, version_info_text)?;

    for lib in &version_info.libraries {
        let url = lib.url.as_deref().unwrap_or_default();
        if url.is_empty() || lib.name.split(':').nth(3).is_some() {
            continue;
        }
        let lib_path = libraries_dir().join(maven_coord_to_path(&lib.name));
        if lib_path.exists() { continue; }
        MavenCoords::new(url, &lib.name).download(client, libraries_dir()).await?;
    }

    Ok(version_id)
}