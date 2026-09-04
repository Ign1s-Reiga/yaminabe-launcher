use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::commands::instance::{create_instance_dir, instance_meta_file, upsert_modlist_entries};
use crate::emit_progress;
use crate::http_utils::{download_resource, fetch_json};
use crate::install_task::ensure_game_and_loader;
use crate::json::write_json;
use crate::AppState;
use log::{info, warn};
use serde::Deserialize;
use tauri::State;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, InstanceMeta, LocalModpackInfo, ModListEntry, ModLoader, ModProjectInfo,
    ModProjectSearchResults, ModState, ModpackFormat, Platform, ProjectFileInfo,
    ProjectFileTarget, SearchOptions,
};
use yaminabe_launcher_shared::error::Error;

#[derive(Deserialize)]
struct Version {
    id: String,
    project_id: String,
    version_type: String,
    #[serde(default)]
    files: Vec<VersionFile>,
    #[serde(default)]
    dependencies: Vec<Dependency>,
}

impl Version {
    fn to_project_file_info(&self, project_type: &str) -> Result<ProjectFileInfo, Error> {
        let version_file = selected_file(self).ok_or_else(|| {
            Error::NotExists(format!("Modrinth version-file {} does not exists.", self.id))
        })?;
        Ok(ProjectFileInfo {
            source: DownloadSource::Modrinth {
                project_id: self.project_id.clone(),
                version_id: self.id.clone(),
            },
            target: ProjectFileTarget::from_modrinth_type(project_type),
            release_type: self.version_type.clone().into(),
            file_name: version_file.filename.clone(),
            download_url: Some(version_file.url.clone()),
            display_name: version_file.filename.clone(),
            project_name: String::new(),
            icon_url: None,
            sha1: version_file.hashes.sha1.clone(),
            size: version_file.size,
        })
    }
}

#[derive(Deserialize)]
struct VersionFile {
    url: String,
    filename: String,
    #[serde(default)]
    hashes: Hashes,
    size: u64,
    #[serde(default)]
    primary: bool,
}

#[derive(Deserialize)]
struct Dependency {
    version_id: Option<String>,
    project_id: String,
    dependency_type: DependencyType,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum DependencyType {
    Required,
    Optional,
    Incompatible,
    Embedded,
    #[serde(other)]
    Unknown,
}

#[derive(Default, Deserialize)]
struct Hashes {
    #[serde(default)]
    sha1: String,
}

/// Download the Modrinth file matching `sha1` to `dest_path` (a full path,
/// including the file name), so a caller mirroring another platform's file can
/// keep that platform's on-disk name. The lookup returns the whole version, whose
/// files may include more than the one asked for, so the download picks the entry
/// carrying `sha1` rather than the version's primary file.
pub async fn download_file_by_sha1(
    sha1: &str,
    dest_path: PathBuf,
    client: &reqwest::Client,
) -> Result<(), Error> {
    let version = fetch_json(
        client,
        &format!("https://api.modrinth.com/v2/version_file/{sha1}"),
    )
    .send::<Version>()
    .await?;
    let file = version
        .files
        .iter()
        .find(|file| file.hashes.sha1.eq_ignore_ascii_case(sha1))
        .ok_or_else(|| {
            Error::NotExists(format!(
                "Modrinth file with SHA-1 {sha1} in version {}",
                version.id
            ))
        })?;
    download_resource(client, &file.url, &file.hashes.sha1, dest_path).await?;
    Ok(())
}

#[derive(Deserialize)]
struct SearchResponse {
    hits: Vec<SearchHit>,
    total_hits: u32,
}

#[derive(Deserialize)]
struct SearchHit {
    title: String,
    description: String,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    downloads: u64,
    /// Categories without the loader/environment tags `categories` carries, so
    /// the result-card chips don't show `fabric`/`forge` alongside real ones.
    #[serde(default)]
    display_categories: Vec<String>,
    #[serde(default)]
    versions: Vec<String>,
}

/// Search Modrinth projects via `GET /v2/search`. Mods are narrowed by game
/// version and loader through the `facets` filter; the sort `index` and result
/// shape are normalized to the shared `ModProjectInfo`. Modrinth identifies
/// projects by string id, which `ModProjectInfo` (CurseForge-shaped `u32` id)
/// can't carry, so results are display-only for now — `id` is left 0.
pub async fn search_projects(
    option: &SearchOptions,
    client: &reqwest::Client,
) -> Result<ModProjectSearchResults, Error> {
    let mut facets: Vec<Vec<String>> = vec![vec![format!(
        "project_type:{}",
        option.target.to_modrinth_type()
    )]];
    if let Some(version) = option.game_version.as_deref() {
        facets.push(vec![format!("versions:{version}")]);
    }
    if let Some(loader) = option.mod_loader.as_ref().and_then(|l| l.modrinth_name()) {
        facets.push(vec![format!("categories:{loader}")]);
    }
    let facets = serde_json::to_string(&facets)?;
    let offset = option.index.to_string();

    let mut query: Vec<(&str, &str)> = vec![
        ("facets", facets.as_str()),
        ("index", option.sort.to_modrinth_index()),
        ("offset", &offset),
        ("limit", "50"),
    ];
    if !option.query.trim().is_empty() {
        query.push(("query", option.query.as_str()));
    }

    let body = fetch_json(client, "https://api.modrinth.com/v2/search")
        .query(&query)
        .send::<SearchResponse>()
        .await?;

    let items = body
        .hits
        .into_iter()
        .map(|hit| {
            // Keep only release-shaped versions (drop snapshots/pre-releases)
            // and sort, so the card's newest-version display is stable.
            let mut game_versions: Vec<String> = hit
                .versions
                .into_iter()
                .filter(|v| v.chars().all(|c| c.is_ascii_digit() || c == '.'))
                .collect();
            game_versions.sort();
            game_versions.dedup();
            ModProjectInfo {
                id: 0,
                platform: Platform::Modrinth,
                file_id: None,
                name: hit.title,
                summary: hit.description,
                logo_url: hit.icon_url,
                download_count: hit.downloads,
                game_versions,
                category: hit.display_categories,
                primary_category_id: 0,
            }
        })
        .collect();

    Ok(ModProjectSearchResults {
        items,
        total: body.total_hits,
    })
}

pub fn list_project_files() {
    unimplemented!()
}

fn selected_file(version: &Version) -> Option<&VersionFile> {
    version
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| version.files.first())
}

/// The index at the root of a `.mrpack`, Modrinth's modpack format.
///
/// The format is handled here beside the API client rather than in a module of
/// its own: `mod_repo` is one module per provider, and `curseforge.rs` likewise
/// holds both its API and the manifest format it exports. Installing a pack
/// fetched from the Modrinth API will read this from the same file rather than
/// reaching across modules for it.
///
/// Unlike a CurseForge manifest, this carries each file's download URL and hash
/// outright, so installing from it needs no API and no key.
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
    let instance_path = create_instance_dir(&install_dir, instance_name)?;

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
mod mrpack_tests {
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
