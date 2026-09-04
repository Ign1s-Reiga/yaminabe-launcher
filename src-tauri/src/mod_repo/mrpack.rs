use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::commands::instance::{instance_meta_file, upsert_modlist_entries};
use crate::http_utils::download_resource;
use crate::install_task::ensure_game_and_loader;
use crate::json::write_json;
use crate::{AppState, emit_progress};
use log::{info, warn};
use serde::Deserialize;
use tauri::State;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, InstanceMeta, LocalModpackInfo, ModListEntry, ModLoader, ModState,
    ModpackFormat, ProjectFileTarget,
};
use yaminabe_launcher_shared::error::Error;

/// The index at the root of a `.mrpack`. Unlike a CurseForge manifest, this
/// carries each file's download URL and hash outright, so installing needs no
/// API and no key.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MrpackIndex {
    #[serde(default)]
    name: String,
    #[serde(default)]
    version_id: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    files: Vec<MrpackFile>,
    /// Maps `minecraft` and one loader to their versions.
    #[serde(default)]
    dependencies: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MrpackFile {
    /// Where the file belongs, relative to the instance root.
    path: String,
    #[serde(default)]
    hashes: MrpackHashes,
    #[serde(default)]
    env: Option<MrpackEnv>,
    /// Mirrors to try in order; the first that works wins.
    #[serde(default)]
    downloads: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct MrpackHashes {
    #[serde(default)]
    sha1: String,
}

#[derive(Debug, Deserialize)]
struct MrpackEnv {
    /// `required`, `optional` or `unsupported` — a launcher installs a client.
    #[serde(default)]
    client: String,
}

impl MrpackFile {
    /// Whether a client install wants this file. Only an explicit `unsupported`
    /// excludes it; an optional file is still installed, since a pack ships the
    /// set it expects to run with.
    fn wanted_by_client(&self) -> bool {
        !matches!(self.env.as_ref().map(|env| env.client.as_str()), Some("unsupported"))
    }
}

const INDEX_NAME: &str = "modrinth.index.json";

fn read_index<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<MrpackIndex, Error> {
    let entry = archive
        .by_name(INDEX_NAME)
        .map_err(|_| Error::Invalid(format!("modpack zip is missing {INDEX_NAME}")))?;
    serde_json::from_reader(entry).map_err(Error::from)
}

/// The loader a pack asks for. Modrinth keys the loader by its own name, and
/// spells the two that end in `-loader` differently from how the launcher does.
fn resolve_loader(dependencies: &HashMap<String, String>) -> (ModLoader, Option<String>) {
    for (key, loader) in [
        ("neoforge", ModLoader::NeoForge),
        ("forge", ModLoader::Forge),
        ("fabric-loader", ModLoader::Fabric),
        ("quilt-loader", ModLoader::Quilt),
    ] {
        if let Some(version) = dependencies.get(key) {
            return (loader, Some(version.clone()));
        }
    }
    (ModLoader::Vanilla, None)
}

/// Resolve a path from the index against `instance_path`, refusing one that
/// would land outside it. The index is attacker-controlled in the same way a
/// zip's entry names are: `..` climbs out, and a drive prefix discards the base
/// entirely on Windows.
fn safe_destination(instance_path: &Path, relative: &str) -> Option<PathBuf> {
    let components: Vec<&str> = relative
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != "." && *part != ".." && !part.contains(':'))
        .collect();
    if components.is_empty() {
        return None;
    }
    Some(
        components
            .iter()
            .fold(instance_path.to_path_buf(), |path, part| path.join(part)),
    )
}

/// Which tracked kind a path belongs to, so a failed download can be linked by
/// hand into the right place. `None` for a path the modlist does not model.
fn target_for(relative: &str) -> Option<ProjectFileTarget> {
    match relative.split(['/', '\\']).next()? {
        "mods" => Some(ProjectFileTarget::Mod),
        "resourcepacks" => Some(ProjectFileTarget::ResourcePack),
        "shaderpacks" => Some(ProjectFileTarget::ShaderPack),
        "datapacks" => Some(ProjectFileTarget::DataPack),
        _ => None,
    }
}

/// Describe a `.mrpack` without installing it.
pub fn read_local_modpack(zip_path: &Path) -> Result<LocalModpackInfo, Error> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| Error::Invalid(format!("cannot open {}: {e}", zip_path.display())))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| Error::Invalid(format!("{} is not a zip file", zip_path.display())))?;
    let index = read_index(&mut archive)?;
    let (mod_loader, mod_loader_version) = resolve_loader(&index.dependencies);
    Ok(LocalModpackInfo {
        format: ModpackFormat::Modrinth,
        name: index.name,
        version: index.version_id,
        // The format carries no author field; a summary is the closest thing to
        // show, and an empty string simply renders nothing.
        author: index.summary,
        game_version: index.dependencies.get("minecraft").cloned().unwrap_or_default(),
        mod_loader,
        mod_loader_version,
        file_count: index.files.len(),
    })
}

/// Extract every `overrides/` tree the pack ships. `client-overrides/` is
/// applied after `overrides/`, since it exists precisely to win on a client.
fn extract_overrides<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    instance_path: &Path,
) -> Result<(), Error> {
    for prefix in ["overrides/", "client-overrides/"] {
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| Error::Invalid(format!("reading zip entry at index {i}: {e}")))?;
            let name = entry.name().to_string();
            let Some(relative) = name.strip_prefix(prefix) else {
                continue;
            };
            if relative.is_empty() {
                continue;
            }
            let Some(dest) = safe_destination(instance_path, relative) else {
                warn!("skipping override with an unusable path: {name}");
                continue;
            };
            if name.ends_with('/') {
                std::fs::create_dir_all(&dest)?;
                continue;
            }
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&dest)?;
            std::io::copy(&mut entry, &mut out)?;
        }
    }
    Ok(())
}

/// Install a `.mrpack` from disk. Each file is fetched straight from the URLs
/// the index lists and verified against its hash, so nothing here needs an API
/// key. A file that cannot be fetched is recorded rather than aborting the
/// install, matching how a CurseForge pack handles a file it cannot get.
pub async fn install_modpack_from_file(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    zip_path: &Path,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let instance_path = PathBuf::from(&install_dir).join(instance_name.to_lowercase());
    if instance_path.exists() {
        return Err(Error::Invalid(format!(
            "folder '{}' already exists at this location",
            instance_name.to_lowercase()
        )));
    }
    std::fs::create_dir_all(&instance_path)?;

    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?)
        .map_err(|e| Error::Invalid(format!("modpack zip is invalid: {e}")))?;
    let index = read_index(&mut archive)?;
    let mc_version = index
        .dependencies
        .get("minecraft")
        .cloned()
        .ok_or_else(|| Error::Invalid(format!("{INDEX_NAME} names no Minecraft version")))?;
    let (mod_loader, loader_version) = resolve_loader(&index.dependencies);

    let version_id = ensure_game_and_loader(
        app_handle,
        id,
        instance_name,
        &mc_version,
        &mod_loader,
        &loader_version,
        state,
    )
    .await?;

    emit_progress(app_handle, id, instance_name, "Extracting files", false, None);
    extract_overrides(&mut archive, &instance_path)?;

    emit_progress(app_handle, id, instance_name, "Downloading mods", false, None);
    let mut modlist_entries: Vec<ModListEntry> = Vec::new();
    for file in index.files.iter().filter(|file| file.wanted_by_client()) {
        let Some(dest) = safe_destination(&instance_path, &file.path) else {
            warn!("skipping pack file with an unusable path: {}", file.path);
            continue;
        };
        let Some(url) = file.downloads.first() else {
            warn!("no download URL for {}", file.path);
            continue;
        };
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file_name = dest
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let result = download_resource(&state.http_client, url, &file.hashes.sha1, dest).await;
        let Some(target) = target_for(&file.path) else {
            // Downloaded, but the modlist models mods and their neighbours only.
            if let Err(e) = result {
                warn!("download failed for {}: {e}", file.path);
            }
            continue;
        };
        let mod_state = match result {
            Ok(()) => ModState::Enabled,
            Err(e) => {
                warn!("download failed for {}: {e}; marking for manual install", file.path);
                ModState::DownloadFailed
            }
        };
        // Only mods are tracked once installed; anything else is tracked solely
        // to drive the link prompt when it could not be fetched.
        if target.tracks_modlist() || mod_state == ModState::DownloadFailed {
            modlist_entries.push(ModListEntry {
                file_name,
                project_name: String::new(),
                icon_url: None,
                sha1: file.hashes.sha1.clone(),
                source: DownloadSource::Manual,
                target,
                size: 0,
                state: mod_state,
            });
        }
    }

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    upsert_modlist_entries(&instance_path, modlist_entries)?;

    let meta = InstanceMeta {
        id: id.to_string(),
        name: instance_name.to_string(),
        description: index.name.clone(),
        game_version: mc_version.clone(),
        mod_loader: mod_loader.clone(),
        mod_loader_version: loader_version,
        version_id,
        category,
        ..InstanceMeta::default()
    };
    write_json(instance_meta_file(&instance_path), &meta)?;

    info!(
        "Installed '{}' from {} (MC {}, {})",
        instance_name,
        zip_path.display(),
        mc_version,
        mod_loader
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{resolve_loader, safe_destination, target_for};
    use std::collections::HashMap;
    use std::path::Path;
    use yaminabe_launcher_shared::datamodels::{ModLoader, ProjectFileTarget};

    fn deps(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn reads_the_loader_modrinth_names() {
        let (loader, version) = resolve_loader(&deps(&[
            ("minecraft", "1.21.1"),
            ("fabric-loader", "0.16.9"),
        ]));

        assert_eq!(loader, ModLoader::Fabric);
        assert_eq!(version.as_deref(), Some("0.16.9"));
    }

    #[test]
    fn a_pack_with_no_loader_is_vanilla() {
        let (loader, version) = resolve_loader(&deps(&[("minecraft", "1.21.1")]));

        assert_eq!(loader, ModLoader::Vanilla);
        assert_eq!(version, None);
    }

    #[test]
    fn refuses_a_path_that_climbs_out_of_the_instance() {
        let root = Path::new("/instances/pack");

        assert_eq!(safe_destination(root, "mods/../../evil.jar"), Some(root.join("mods/evil.jar")));
        assert_eq!(safe_destination(root, "../evil.jar"), Some(root.join("evil.jar")));
        // A drive prefix would otherwise replace the base outright on Windows.
        assert_eq!(safe_destination(root, "C:/evil.jar"), Some(root.join("evil.jar")));
        assert_eq!(safe_destination(root, ".."), None);
    }

    #[test]
    fn maps_a_path_to_what_the_modlist_models() {
        assert_eq!(target_for("mods/a.jar"), Some(ProjectFileTarget::Mod));
        assert_eq!(target_for("resourcepacks/b.zip"), Some(ProjectFileTarget::ResourcePack));
        assert_eq!(target_for("shaderpacks/c.zip"), Some(ProjectFileTarget::ShaderPack));
        assert_eq!(target_for("config/d.json"), None);
    }
}
