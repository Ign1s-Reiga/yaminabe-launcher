use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::commands::instance::{
    instance_meta_file, modlist_file, replace_modlist_entries_for_file_ids, upsert_modlist_entries,
};
use crate::http_utils::{fetch_json, rejected_status};
use crate::install_task::ensure_game_and_loader;
use crate::json::{read_json, read_json_or_default, write_json};
use crate::mod_repo::{ProjectFileDownload, ProjectFileTarget};
use crate::{AppState, caches_dir, emit_progress};
use log::info;
use serde::Deserialize;
use tauri::State;
use yaminabe_launcher_shared::datatypes::{
    DownloadSource, InstanceMeta, ModListEntry, ModLoader, ModProjectFile, ModProjectInfo,
    ModProjectSearchResults, ModState, ProjectClass, SearchOption,
};
use yaminabe_launcher_shared::error::Error;

#[derive(Debug, Deserialize)]
struct CurseForgeResponse<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct CurseForgeArrayResponse<T> {
    data: Vec<T>,
    #[serde(default)]
    pagination: Pagination,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pagination {
    total_count: u32,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ModFile {
    id: u32,
    mod_id: u32,
    release_type: u32,
    file_name: String,
    download_url: Option<String>,
    display_name: String,
    file_size_on_disk: u64,
    #[serde(default)]
    hashes: Vec<FileHash>,
}

impl ModFile {
    fn to_modlist_entry(self: &ModFile, state: ModState) -> ModListEntry {
        ModListEntry {
            file_name: self.file_name.clone(),
            sha1: self
                .hashes
                .iter()
                .find(|h| h.algo == 1)
                .map(|h| h.value.clone())
                .unwrap_or_default(),
            source: DownloadSource::CurseForge {
                project_id: self.mod_id,
                file_id: self.id,
            },
            size: self.file_size_on_disk,
            state,
        }
    }

    fn to_project_file_download(
        &self,
        target: ProjectFileTarget,
        fallback_to_modrinth: bool,
        require_sha1: bool,
    ) -> ProjectFileDownload {
        ProjectFileDownload {
            source: DownloadSource::CurseForge {
                project_id: self.mod_id,
                file_id: self.id,
            },
            file_name: self.file_name.clone(),
            sha1: self
                .hashes
                .iter()
                .find(|h| h.algo == 1)
                .map(|h| h.value.clone())
                .unwrap_or_default(),
            size: self.file_size_on_disk,
            download_url: self.download_url.clone(),
            target,
            fallback_to_modrinth,
            require_sha1,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FileHash {
    value: String,
    algo: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchModsEntry {
    id: u32,
    name: String,
    summary: String,
    primary_category_id: u32,
    categories: Vec<CategoryItem>,
    logo: Option<Logo>,
    download_count: u32,
    #[serde(default)]
    latest_files_indexes: Vec<FilesIndex>,
}

#[derive(Debug, Deserialize)]
struct CategoryItem {
    id: u32,
    name: String,
}

#[derive(Debug, Deserialize)]
struct Logo {
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FilesIndex {
    #[serde(default)]
    file_id: u32,
    game_version: String,
    #[serde(default)]
    mod_loader: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ModpackManifest {
    minecraft: ManifestVersionSpecifier,
    #[serde(default = "default_overrides_dir")]
    overrides: String,
    #[serde(default)]
    files: Vec<ManifestFilesItem>,
}

#[derive(Debug, Deserialize)]
struct ManifestFilesItem {
    #[serde(rename = "projectID")]
    project_id: u32,
    #[serde(rename = "fileID")]
    file_id: u32,
    required: bool,
}

fn default_overrides_dir() -> String {
    "overrides".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestVersionSpecifier {
    version: String,
    #[serde(default)]
    mod_loaders: Vec<ModLoaderVersion>,
}

#[derive(Debug, Deserialize)]
struct ModLoaderVersion {
    id: String,
    primary: bool,
}

/// The newest `latestFilesIndexes` entry compatible with `mc_version` and
/// `mod_loader` — the file the Add Mod button actually downloads. Loader-
/// agnostic entries (no `modLoader`) are accepted for any loader. `None` when
/// nothing listed fits, so the project is left non-installable rather than
/// offering a file for the wrong version or loader.
fn compatible_file_id(
    indexes: &[FilesIndex],
    mc_version: &str,
    mod_loader: &ModLoader,
) -> Option<u32> {
    indexes
        .iter()
        .find(|f| {
            f.game_version == mc_version
                && (f.mod_loader == mod_loader.mod_loader_id() || f.mod_loader.is_none())
        })
        .map(|f| f.file_id)
        .filter(|id| *id != 0)
}

fn to_search_results(
    body: CurseForgeArrayResponse<SearchModsEntry>,
    compat: Option<(&str, &ModLoader)>,
) -> ModProjectSearchResults {
    let total = body.pagination.total_count;
    let items: Vec<ModProjectInfo> = body
        .data
        .into_iter()
        .map(|m| {
            let mut versions: Vec<String> = m
                .latest_files_indexes
                .iter()
                .map(|f| f.game_version.clone())
                .collect();
            versions.sort();
            versions.dedup();

            let primary_category_id = m.primary_category_id;
            let mut categories = m.categories;
            // Stable sort so the entry whose id matches `primary_category_id`
            // comes first; remaining categories keep their original order.
            categories.sort_by_key(|c| if c.id == primary_category_id { 0 } else { 1 });
            let category: Vec<String> = categories.into_iter().map(|c| c.name).collect();

            // Mods: pick the file matching the searched version/loader. Modpack
            // search has no such filter, so fall back to the latest file.
            let file_id = match compat {
                Some((mc_version, mod_loader)) => {
                    compatible_file_id(&m.latest_files_indexes, mc_version, mod_loader)
                }
                None => m
                    .latest_files_indexes
                    .first()
                    .map(|f| f.file_id)
                    .filter(|id| *id != 0),
            };

            ModProjectInfo {
                id: m.id,
                file_id,
                name: m.name,
                summary: m.summary,
                logo_url: m.logo.map(|l| l.url),
                download_count: m.download_count,
                game_versions: versions,
                category,
                primary_category_id,
            }
        })
        .collect();

    ModProjectSearchResults { items, total }
}

/// CurseForge `classId` for a project class.
fn class_id(class: ProjectClass) -> &'static str {
    match class {
        ProjectClass::Mod => "6",
        ProjectClass::Modpack => "4471",
        ProjectClass::ResourcePack => "12",
    }
}

/// Unified CurseForge search for any project class. Mods can be narrowed to a
/// Minecraft version and/or mod loader (so only compatible files are offered);
/// modpacks and resource packs search broadly. When both a version and loader
/// are given, the per-result `file_id` is the matching file rather than the
/// latest.
pub async fn search_projects(
    option: &SearchOption,
    http_client: &reqwest::Client,
    api_key: &str,
) -> Result<ModProjectSearchResults, Error> {
    if option.query.trim().is_empty() {
        return Ok(ModProjectSearchResults {
            items: vec![],
            total: 0,
        });
    }

    let index = option.index.to_string();
    let mut query: Vec<(&str, &str)> = vec![
        ("gameId", "432"),
        ("classId", class_id(option.class)),
        ("searchFilter", option.query.as_str()),
        ("sortField", "2"),
        ("pageSize", "50"),
        ("sortOrder", "desc"),
        ("index", &index),
    ];
    if let Some(game_version) = option.game_version.as_deref() {
        query.push(("gameVersion", game_version));
    }
    // `mod_loader_id` is `None` for Vanilla, which has no loader filter.
    let loader_id;
    if let Some(id) = option
        .mod_loader
        .as_ref()
        .and_then(ModLoader::mod_loader_id)
    {
        loader_id = id.to_string();
        query.push(("modLoaderType", &loader_id));
    }

    let body = fetch_json(http_client, "https://api.curseforge.com/v1/mods/search")
        .header("x-api-key", api_key)
        .query(&query)
        .send::<CurseForgeArrayResponse<SearchModsEntry>>()
        .await?;

    // Pick the version/loader-matching file only when both are known (mod
    // search); otherwise keep the latest file.
    let compat = match (option.game_version.as_deref(), option.mod_loader.as_ref()) {
        (Some(version), Some(loader)) => Some((version, loader)),
        _ => None,
    };
    Ok(to_search_results(body, compat))
}

pub async fn list_project_files(
    mod_id: u32,
    http_client: &reqwest::Client,
    api_key: &str,
) -> Result<Vec<ModProjectFile>, Error> {
    let mut entries = fetch_json(
        http_client,
        &format!("https://api.curseforge.com/v1/mods/{mod_id}/files"),
    )
    .header("x-api-key", api_key)
    .query(&[("pageSize", "50")])
    .send::<CurseForgeArrayResponse<ModFile>>()
    .await?
    .data;

    entries.sort_by(|a, b| b.id.cmp(&a.id));

    let versions = entries
        .into_iter()
        .filter_map(|f| {
            let download_url = f.download_url?;
            let release_type = match f.release_type {
                1 => "Stable",
                2 => "Beta",
                3 => "Alpha",
                _ => "Unknown",
            }
            .to_string();
            Some(ModProjectFile {
                id: f.id,
                mod_id: f.mod_id,
                release_type,
                file_name: f.file_name,
                download_url,
                display_name: f.display_name,
            })
        })
        .collect();

    Ok(versions)
}

fn read_manifest<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<ModpackManifest, Error> {
    let mut file = archive
        .by_name("manifest.json")
        .map_err(|_| Error::Invalid("modpack zip is missing manifest.json".to_string()))?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(serde_json::from_str(&content)?)
}

/// Resolve the modpack's primary mod loader and its version. CurseForge
/// manifest loader ids are `{name}-{version}` (e.g. `neoforge-21.1.228`).
fn resolve_loader(manifest: &ModpackManifest) -> Result<(ModLoader, Option<String>), Error> {
    let mod_loader_id = manifest
        .minecraft
        .mod_loaders
        .iter()
        .find(|l| l.primary)
        .or_else(|| manifest.minecraft.mod_loaders.first())
        .map(|l| l.id.to_ascii_lowercase())
        .unwrap_or("vanilla".to_string());
    let (loader_name, loader_version) = mod_loader_id
        .split_once('-')
        .map(|(n, v)| (n, Some(v.to_string())))
        .unwrap_or((mod_loader_id.as_str(), None));
    Ok((ModLoader::from_str(loader_name)?, loader_version))
}

fn manifest_file_ids(manifest: &ModpackManifest) -> Vec<u32> {
    manifest
        .files
        .iter()
        .filter(|f| f.required)
        .map(|f| f.file_id)
        .collect()
}

/// Download a modpack's required files into the instance. A CurseForge manifest
/// mixes mods and resource packs: jars go to `mods/` and are returned as modlist
/// entries; `.zip` resource packs go to `resourcepacks/` and are downloaded but
/// not tracked (the modlist only lists mods). Unlike the manual-add path this
/// never falls back to Modrinth — a file that can't be fetched becomes a
/// `DownloadFailed` modlist entry (mods only) for the link flow.
async fn download_modpack_files(
    file_ids: &[u32],
    instance_path: &Path,
    api_key: &str,
    client: &reqwest::Client,
) -> Result<Vec<ModListEntry>, Error> {
    let files = fetch_file_entries(file_ids, api_key, client).await?;
    let project_files = files
        .iter()
        .map(|file| {
            let target = if file.file_name.to_ascii_lowercase().ends_with(".zip") {
                ProjectFileTarget::ResourcePacks
            } else {
                ProjectFileTarget::Mods
            };
            file.to_project_file_download(target, false, false)
        })
        .collect();
    super::download_project_files(project_files, instance_path, client).await
}

pub(crate) async fn resolve_project_file(
    project_id: u32,
    file_id: u32,
    api_key: &str,
    client: &reqwest::Client,
) -> Result<ProjectFileDownload, Error> {
    let file = fetch_json(
        client,
        &format!("https://api.curseforge.com/v1/mods/{project_id}/files/{file_id}"),
    )
    .header("x-api-key", api_key)
    .send::<CurseForgeResponse<ModFile>>()
    .await?
    .data;
    Ok(file.to_project_file_download(ProjectFileTarget::Mods, true, true))
}

/// CurseForge file ids currently recorded in the instance's modlist — the set
/// the upgrade diffs the new manifest against. The modlist is the source of
/// truth for what is installed and is only rewritten when an upgrade finalizes,
/// so a failed (and retried) upgrade keeps diffing against the previous set.
/// Empty when the modlist is missing, degrading to "download everything,
/// remove nothing".
fn installed_curseforge_file_ids(instance_path: &Path) -> Vec<u32> {
    let modlist: Vec<ModListEntry> =
        read_json_or_default(modlist_file(instance_path)).unwrap_or_default();
    modlist
        .iter()
        .filter_map(|entry| entry.source.curseforge_ids().map(|(_, file_id)| file_id))
        .collect()
}

/// Extract the modpack's `overrides/` tree into `instance_path`, overwriting
/// any file the pack ships and leaving everything else (user saves, configs,
/// manually-added mods) untouched.
fn extract_overrides<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    overrides_prefix: &str,
    instance_path: &Path,
) -> Result<(), Error> {
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| Error::Invalid(format!("reading zip entry at index {i}: {e}")))?;

        let entry_name = file.name().to_string();
        let Some(rel) = entry_name.strip_prefix(overrides_prefix) else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }

        let dest = rel
            .split('/')
            .filter(|c| !c.is_empty() && *c != "..")
            .fold(instance_path.to_path_buf(), |p, c| p.join(c));

        if entry_name.ends_with('/') {
            std::fs::create_dir_all(&dest)?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&dest)?;
            std::io::copy(&mut file, &mut out)?;
        }
    }
    Ok(())
}

/// Resolve CurseForge file metadata (download url + filename) for a set of file
/// ids, batching at the API's 50-per-request limit.
async fn fetch_file_entries(
    file_ids: &[u32],
    api_key: &str,
    client: &reqwest::Client,
) -> Result<Vec<ModFile>, Error> {
    let mut entries = Vec::new();
    for chunk in file_ids.chunks(50) {
        let body = serde_json::json!({ "fileIds": chunk });
        let resp = client
            .post("https://api.curseforge.com/v1/mods/files")
            .header("x-api-key", api_key)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(rejected_status(
                resp.status(),
                "https://api.curseforge.com/v1/mods/files",
            ));
        }
        let data = resp
            .json::<CurseForgeArrayResponse<ModFile>>()
            .await
            .map_err(Error::InvalidResponse)?;
        entries.extend(data.data);
    }
    Ok(entries)
}

/// Resolve a CurseForge file's download URL from its project + file ids.
async fn fetch_file_download_url(
    project_id: u32,
    file_id: u32,
    api_key: &str,
    client: &reqwest::Client,
) -> Result<String, Error> {
    let file = fetch_json(
        client,
        &format!("https://api.curseforge.com/v1/mods/{project_id}/files/{file_id}"),
    )
    .header("x-api-key", api_key)
    .send::<CurseForgeResponse<ModFile>>()
    .await?
    .data;
    file.download_url.ok_or_else(|| {
        Error::Invalid(format!(
            "CurseForge modpack file {} has no download URL",
            file.file_name
        ))
    })
}

/// The game/loader/manifest state produced by [`prepare_modpack`], handed back so
/// the install and upgrade paths can finish with their own mod handling.
struct PreparedModpack {
    manifest: ModpackManifest,
    mc_version: String,
    mod_loader: ModLoader,
    loader_version: Option<String>,
    version_id: String,
}

/// Shared install/upgrade prefix: resolve the modpack zip URL from
/// `(project_id, file_id)`, download it to the caches dir, read the manifest,
/// ensure the game + loader are installed, and extract `overrides/` into
/// `instance_path`. The cached zip is processed from disk (not held in memory)
/// and deleted once its overrides are extracted. Mod-list handling and instance
/// metadata differ between install and upgrade, so they stay with the caller.
async fn prepare_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    instance_path: &Path,
    project_id: u32,
    file_id: u32,
    api_key: &str,
    state: &State<'_, AppState>,
) -> Result<PreparedModpack, Error> {
    let http_client = &state.http_client;
    emit_progress(
        app_handle,
        id,
        instance_name,
        "Downloading modpack",
        false,
        None,
    );

    let download_url = fetch_file_download_url(project_id, file_id, api_key, http_client).await?;
    let resp = http_client.get(&download_url).send().await?;
    if !resp.status().is_success() {
        return Err(rejected_status(resp.status(), &download_url));
    }

    // Stage the zip in the caches dir (keyed by the install id, so concurrent
    // installs don't collide) and process it from disk rather than memory.
    let cache_path = caches_dir().join(format!("{id}.zip"));
    {
        let zip_bytes = resp.bytes().await.map_err(Error::InvalidResponse)?;
        std::fs::write(&cache_path, &zip_bytes)?;
    }

    let prepared = prepare_from_cached_zip(
        app_handle,
        id,
        instance_name,
        instance_path,
        &cache_path,
        state,
    )
    .await;

    // Always drop the staged zip — its overrides are now in the instance and
    // its mods are fetched separately from the manifest.
    std::fs::remove_file(&cache_path).ok();
    prepared
}

/// Read the cached modpack zip from disk: parse the manifest, ensure the game +
/// loader, and extract overrides into the instance. Split out so `prepare_modpack`
/// can delete the cache file on every exit path.
async fn prepare_from_cached_zip(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    instance_path: &Path,
    cache_path: &Path,
    state: &State<'_, AppState>,
) -> Result<PreparedModpack, Error> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(cache_path)?)
        .map_err(|e| Error::Invalid(format!("modpack zip is invalid: {e}")))?;

    let manifest = read_manifest(&mut archive)?;
    let mc_version = manifest.minecraft.version.clone();
    let (mod_loader, loader_version) = resolve_loader(&manifest)?;

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

    emit_progress(
        app_handle,
        id,
        instance_name,
        "Extracting files",
        false,
        None,
    );
    let overrides_prefix = format!("{}/", manifest.overrides.trim_end_matches('/'));
    extract_overrides(&mut archive, &overrides_prefix, instance_path)?;

    Ok(PreparedModpack {
        manifest,
        mc_version,
        mod_loader,
        loader_version,
        version_id,
    })
}

pub async fn install_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    source: DownloadSource,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let (project_id, file_id) = source
        .curseforge_ids()
        .ok_or_else(|| Error::Unsupported("not a CurseForge modpack source".to_string()))?;
    let (api_key, install_dir) = {
        let settings = state.settings.read().unwrap();
        (
            settings.curseforge_api_key.clone(),
            settings.instance_install_dir.clone(),
        )
    };

    let instance_path = PathBuf::from(&install_dir).join(instance_name.to_lowercase());
    std::fs::create_dir_all(&instance_path)?;

    let prepared = prepare_modpack(
        app_handle,
        id,
        instance_name,
        &instance_path,
        project_id,
        file_id,
        &api_key,
        state,
    )
    .await?;

    emit_progress(
        app_handle,
        id,
        instance_name,
        "Downloading mods",
        false,
        None,
    );
    let file_ids = manifest_file_ids(&prepared.manifest);
    let modlist_entries =
        download_modpack_files(&file_ids, &instance_path, &api_key, &state.http_client).await?;

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    upsert_modlist_entries(&instance_path, modlist_entries)?;

    let meta = InstanceMeta {
        name: instance_name.to_string(),
        game_version: prepared.mc_version.clone(),
        mod_loader: prepared.mod_loader.clone(),
        mod_loader_version: prepared.loader_version,
        version_id: prepared.version_id,
        category,
        origin: source,
        ..InstanceMeta::default()
    };
    write_json(instance_meta_file(&instance_path), &meta)?;

    info!(
        "Installed '{}' (MC {}, {}) → {}",
        instance_name,
        prepared.mc_version,
        prepared.mod_loader,
        instance_path.display()
    );
    Ok(())
}

/// Upgrade an existing CurseForge instance to a newer modpack file: re-ensure
/// the (possibly changed) game + loader, overlay the new `overrides/`, diff the
/// mod list against the instance's recorded manifest — downloading only new
/// mods and removing only mods the pack dropped — then update instance metadata.
/// User-added files (saves, screenshots, manual mods) are never touched.
pub async fn upgrade_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    instance_path: PathBuf,
    source: DownloadSource,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let (project_id, file_id) = source
        .curseforge_ids()
        .ok_or_else(|| Error::Unsupported("not a CurseForge modpack source".to_string()))?;
    let api_key = state.settings.read().unwrap().curseforge_api_key.clone();
    let http_client = &state.http_client;

    // The modlist (the installed-mod source of truth) is only rewritten at
    // finalize, so this set survives a failed/retried upgrade for a correct re-diff.
    let old_ids = installed_curseforge_file_ids(&instance_path);

    let prepared = prepare_modpack(
        app_handle,
        id,
        instance_name,
        &instance_path,
        project_id,
        file_id,
        &api_key,
        state,
    )
    .await?;

    // Diff the mod list by file id: download additions, remove drops, skip
    // unchanged files already on disk.
    emit_progress(app_handle, id, instance_name, "Updating mods", false, None);
    let new_ids = manifest_file_ids(&prepared.manifest);
    let old_set: HashSet<u32> = old_ids.iter().copied().collect();
    let new_set: HashSet<u32> = new_ids.iter().copied().collect();
    let to_remove: Vec<u32> = old_ids
        .iter()
        .copied()
        .filter(|fid| !new_set.contains(fid))
        .collect();
    let to_add: Vec<u32> = new_ids
        .iter()
        .copied()
        .filter(|fid| !old_set.contains(fid))
        .collect();

    let mut old_file_names = Vec::new();
    if !to_remove.is_empty() {
        let mods_dir = instance_path.join("mods");
        for entry in fetch_file_entries(&to_remove, &api_key, http_client).await? {
            old_file_names.push(entry.file_name.clone());
            let path = mods_dir.join(&entry.file_name);
            if path.exists() {
                if let Err(e) = std::fs::remove_file(&path) {
                    log::warn!("failed to remove old mod {}: {e}", entry.file_name);
                }
            }
        }
    }
    // `to_add` mixes new mods and (all) resource packs — resource packs are never
    // in `old_ids`, since the modlist tracks only mods. `download_modpack_files`
    // routes each to the right dir and returns entries for mods alone.
    let downloaded_modlist_entries =
        download_modpack_files(&to_add, &instance_path, &api_key, http_client).await?;
    // Baseline the new modlist from the full file set, but keep only mods — the
    // downloaded entries (with real state) overwrite their baseline below.
    let mut new_modlist_entries: Vec<ModListEntry> =
        fetch_file_entries(&new_ids, &api_key, http_client)
            .await?
            .into_iter()
            .filter(|entry| !entry.file_name.to_ascii_lowercase().ends_with(".zip"))
            .map(|entry| entry.to_modlist_entry(ModState::Enabled))
            .collect();
    for entry in downloaded_modlist_entries {
        new_modlist_entries.retain(|existing| existing.file_name != entry.file_name);
        new_modlist_entries.push(entry);
    }

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    replace_modlist_entries_for_file_ids(
        &instance_path,
        &old_ids,
        &old_file_names,
        new_modlist_entries,
    )?;

    let mut meta: InstanceMeta = read_json(instance_meta_file(&instance_path))?;
    meta.game_version = prepared.mc_version.clone();
    meta.mod_loader = prepared.mod_loader.clone();
    meta.mod_loader_version = prepared.loader_version;
    meta.version_id = prepared.version_id;
    meta.origin = source;
    write_json(instance_meta_file(&instance_path), &meta)?;

    info!(
        "Upgraded '{}' to file {} (MC {}, {})",
        instance_name, file_id, prepared.mc_version, prepared.mod_loader
    );
    Ok(())
}
