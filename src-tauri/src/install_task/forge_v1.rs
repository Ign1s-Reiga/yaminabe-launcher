use std::io::Read;
use std::path::Path;
use serde::Deserialize;
use zip::ZipArchive;
use yaminabe_launcher_shared::error::Error;
use yaminabe_launcher_shared::version_manifest::ClientManifest;
use crate::{libraries_dir, versions_dir};
use crate::http_utils::download_from_maven;
use super::maven_coord_to_path;

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
    let mut zip = ZipArchive::new(std::fs::File::open(installer_path)?)
        .map_err(|e| Error::Invalid(e.to_string()))?;

    let profile: InstallProfile = {
        let mut buf = String::new();
        zip.by_name("install_profile.json")
            .map_err(|e| Error::Invalid(e.to_string()))?
            .read_to_string(&mut buf)?;
        serde_json::from_str(&buf)?
    };

    let mut universal_jar_bytes = Vec::new();
    zip.by_name(&profile.install.file_path)
        .map_err(|e| Error::Invalid(format!("Embedded JAR '{}' not found: {e}", profile.install.file_path)))?
        .read_to_end(&mut universal_jar_bytes)?;

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

    let version_dir = versions_dir().join(&version_id);
    std::fs::create_dir_all(&version_dir)?;
    std::fs::write(version_dir.join(format!("{version_id}.json")), version_info_text)?;

    for lib in &version_info.libraries {
        let url = lib.url.as_deref().unwrap_or_default();
        if url.is_empty() || lib.name.split(':').nth(3).is_some() {
            continue;
        }
        let lib_path = libraries_dir().join(maven_coord_to_path(&lib.name));
        if lib_path.exists() { continue; }
        download_from_maven(client, url, lib.name.clone(), None, "jar", libraries_dir().clone()).await?;
    }

    Ok(version_id)
}