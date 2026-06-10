use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use log::{info, warn};
use serde::Deserialize;
use tauri::State;
use yaminabe_launcher_shared::datatypes::{
    DownloadSource, InstanceMeta, ModListEntry, ModLoader, ModProjectInfo,
    ModProjectSearchResults, ModProjectFile,
};
use yaminabe_launcher_shared::error::Error;
use crate::{emit_progress, AppState};
use crate::commands::instance::{
    instance_meta_file, replace_modlist_entries_for_file_ids, upsert_modlist_entries,
};
use crate::http_utils::{download_resource, fetch_json};
use crate::install_task::ensure_game_and_loader;
use crate::json::{read_json, write_json};
use crate::mod_repo::modrinth;

#[derive(Debug, Deserialize)]
struct CurseForgeArrayResponse<T>
{
    data: Vec<T>
}

#[derive(Debug, Deserialize)]
struct CurseForgeResponse<T>
{
    data: T
}

#[derive(Debug, Deserialize)]
struct CurseForgePaginatedResponse<T> {
    data: Vec<T>,
    pagination: Pagination,
}

#[derive(Debug, Deserialize)]
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
    fn to_modlist_entry(self: &ModFile) -> ModListEntry {
        ModListEntry {
            file_name: self.file_name.clone(),
            sha1: self.hashes.iter().find(|h| h.algo == 1).map(|h| h.value.clone()).unwrap_or_default(),
            source: DownloadSource::CurseForge { project_id: self.mod_id, file_id: self.id },
            size: self.file_size_on_disk,
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
    minecraft: CfManifestMinecraft,
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

fn default_overrides_dir() -> String { "overrides".to_string() }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfManifestMinecraft {
    version: String,
    #[serde(default)]
    mod_loaders: Vec<CfManifestLoader>,
}

#[derive(Debug, Deserialize)]
struct CfManifestLoader {
    id: String,
    primary: bool,
}

// ── Platform implementations ──────────────────────────────────────────────────

/// CurseForge `modLoaderType` query value for a mod loader. `0` (Any) for
/// Vanilla, which has no loader-specific mods.
fn mod_loader_type(mod_loader: &ModLoader) -> &'static str {
    match mod_loader {
        ModLoader::Forge => "1",
        ModLoader::Fabric => "4",
        ModLoader::Quilt => "5",
        ModLoader::NeoForge => "6",
        ModLoader::Vanilla => "0",
    }
}

/// Numeric `modLoader` id as it appears in `latestFilesIndexes` (the same loader
/// mapping as [`mod_loader_type`], minus the `0`/Any case). `None` for Vanilla,
/// since loader-agnostic files carry no id.
fn mod_loader_id(mod_loader: &ModLoader) -> Option<u32> {
    match mod_loader {
        ModLoader::Forge => Some(1),
        ModLoader::Fabric => Some(4),
        ModLoader::Quilt => Some(5),
        ModLoader::NeoForge => Some(6),
        ModLoader::Vanilla => None,
    }
}

/// The newest `latestFilesIndexes` entry compatible with `mc_version` and
/// `mod_loader` — the file the Add Mod button actually downloads. Loader-
/// agnostic entries (no `modLoader`) are accepted for any loader. `None` when
/// nothing listed fits, so the project is left non-installable rather than
/// offering a file for the wrong version or loader.
fn compatible_file_id(indexes: &[FilesIndex], mc_version: &str, mod_loader: &ModLoader) -> Option<u32> {
    let loader_id = mod_loader_id(mod_loader);
    indexes.iter()
        .find(|f| f.game_version == mc_version && (f.mod_loader == loader_id || f.mod_loader.is_none()))
        .map(|f| f.file_id)
        .filter(|id| *id != 0)
}

fn to_search_results(
    body: CurseForgePaginatedResponse<SearchModsEntry>,
    compat: Option<(&str, &ModLoader)>,
) -> ModProjectSearchResults {
    let total = body.pagination.total_count;
    let items: Vec<ModProjectInfo> = body.data.into_iter().map(|m| {
        let mut versions: Vec<String> = m.latest_files_indexes.iter()
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
            Some((mc_version, mod_loader)) => compatible_file_id(&m.latest_files_indexes, mc_version, mod_loader),
            None => m.latest_files_indexes.first().map(|f| f.file_id).filter(|id| *id != 0),
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
    }).collect();

    ModProjectSearchResults { items, total }
}

pub async fn search_modpacks(
    query: &str,
    index: u32,
    http_client: &reqwest::Client,
    api_key: &str,
) -> Result<ModProjectSearchResults, Error> {
    if query.trim().is_empty() {
        return Ok(ModProjectSearchResults { items: vec![], total: 0 });
    }

    let body = fetch_json(http_client, "https://api.curseforge.com/v1/mods/search")
        .header("x-api-key", api_key)
        .query(&[
            ("gameId", "432"),
            ("classId", "4471"),
            ("searchFilter", query),
            ("sortField", "2"),
            ("pageSize", "50"),
            ("sortOrder", "desc"),
            ("index", &index.to_string()),
        ])
        .send::<CurseForgePaginatedResponse<SearchModsEntry>>()
        .await?;

    Ok(to_search_results(body, None))
}

/// Search CurseForge mods (classId 6) pre-filtered to a Minecraft version and
/// mod loader, so only files compatible with the instance are offered.
pub async fn search_mods(
    query: &str,
    index: u32,
    mc_version: &str,
    mod_loader: &ModLoader,
    http_client: &reqwest::Client,
    api_key: &str,
) -> Result<ModProjectSearchResults, Error> {
    if query.trim().is_empty() {
        return Ok(ModProjectSearchResults { items: vec![], total: 0 });
    }

    let body = fetch_json(http_client, "https://api.curseforge.com/v1/mods/search")
        .header("x-api-key", api_key)
        .query(&[
            ("gameId", "432"),
            ("classId", "6"),
            ("searchFilter", query),
            ("gameVersion", mc_version),
            ("modLoaderType", mod_loader_type(mod_loader)),
            ("sortField", "2"),
            ("pageSize", "50"),
            ("sortOrder", "desc"),
            ("index", &index.to_string()),
        ])
        .send::<CurseForgePaginatedResponse<SearchModsEntry>>()
        .await?;

    Ok(to_search_results(body, Some((mc_version, mod_loader))))
}

pub async fn get_modpack_files(
    mod_id: u32,
    http_client: &reqwest::Client,
    api_key: &str,
) -> Result<Vec<ModProjectFile>, Error> {
    let mut entries = fetch_json(
        http_client,
        &format!("https://api.curseforge.com/v1/mods/{mod_id}/files")
    )
        .header("x-api-key", api_key)
        .query(&[("pageSize", "50")])
        .send::<CurseForgeArrayResponse<ModFile>>().await?
        .data;
    
    entries.sort_by(|a, b| b.id.cmp(&a.id));

    let versions = entries.into_iter().filter_map(|f| {
        let download_url = f.download_url?;
        let release_type = match f.release_type {
            1 => "Stable",
            2 => "Beta",
            3 => "Alpha",
            _ => "Unknown",
        }.to_string();
        Some(ModProjectFile {
            id: f.id,
            mod_id: f.mod_id,
            release_type,
            file_name: f.file_name,
            download_url,
            display_name: f.display_name,
        })
    }).collect();

    Ok(versions)
}

fn read_manifest<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<(ModpackManifest, String), Error> {
    let mut file = archive.by_name("manifest.json")
        .map_err(|_| Error::Invalid("modpack zip is missing manifest.json".to_string()))?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    let manifest = serde_json::from_str(&content)?;
    Ok((manifest, content))
}

/// Resolve the modpack's primary mod loader and its version. CurseForge
/// manifest loader ids are `{name}-{version}` (e.g. `neoforge-21.1.228`).
fn resolve_loader(manifest: &ModpackManifest) -> Result<(ModLoader, Option<String>), Error> {
    let mod_loader_id = manifest.minecraft.mod_loaders.iter()
        .find(|l| l.primary)
        .or_else(|| manifest.minecraft.mod_loaders.first())
        .map(|l| l.id.to_ascii_lowercase())
        .unwrap_or("vanilla".to_string());
    let (loader_name, loader_version) = mod_loader_id.split_once('-')
        .map(|(n, v)| (n, Some(v.to_string())))
        .unwrap_or((mod_loader_id.as_str(), None));
    Ok((ModLoader::from_str(loader_name)?, loader_version))
}

fn manifest_file_ids(manifest: &ModpackManifest) -> Vec<u32> {
    manifest.files.iter().filter(|f| f.required).map(|f| f.file_id).collect()
}

fn manifest_mod_sources(manifest: &ModpackManifest) -> Vec<DownloadSource> {
    manifest.files.iter()
        .filter(|f| f.required)
        .map(|f| DownloadSource::CurseForge { project_id: f.project_id, file_id: f.file_id })
        .collect()
}

pub async fn download_mod(
    project_id: u32,
    file_id: u32,
    mods_dir: &Path,
    api_key: &str,
    client: &reqwest::Client,
) -> Result<ModListEntry, Error> {
    let file = fetch_json(client, &format!("https://api.curseforge.com/v1/mods/{project_id}/files/{file_id}"))
        .header("x-api-key", api_key)
        .send::<CurseForgeResponse<ModFile>>()
        .await?
        .data;

    let sha1 = file.hashes.iter()
        .find(|v| v.algo == 1)
        .map(|v| v.value.as_str())
        .ok_or_else(|| Error::Invalid(format!("CurseForge file {} has no SHA-1 hash", file.file_name)))?;
    
    if let Some(url) = &file.download_url {
        download_resource(client, url, sha1, mods_dir.join(&file.file_name)).await?;
    } else {
        warn!("CurseForge restricts downloads for file {}, falling back to Modrinth lookup by SHA-1", file.file_name);
        // Save the mirrored Modrinth jar under the CurseForge file name so the
        // returned entry, the Mods tab, and upgrade cleanup all agree on disk.
        modrinth::download_file_by_sha1(sha1, mods_dir.join(&file.file_name), client).await?;
    }
    Ok(file.to_modlist_entry())
}

/// Required mod file ids recorded in an instance's persisted `manifest.json`.
/// Returns an empty list when the file is missing or unreadable, so the upgrade
/// diff safely degrades to "download everything, remove nothing".
fn installed_manifest_file_ids(instance_path: &Path) -> Vec<u32> {
    std::fs::read_to_string(instance_path.join("manifest.json")).ok()
        .and_then(|c| serde_json::from_str::<ModpackManifest>(&c).ok())
        .map(|m| manifest_file_ids(&m))
        .unwrap_or_default()
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
        let mut file = archive.by_index(i)
            .map_err(|e| Error::Invalid(format!("reading zip entry at index {i}: {e}")))?;

        let entry_name = file.name().to_string();
        let Some(rel) = entry_name.strip_prefix(overrides_prefix) else { continue };
        if rel.is_empty() { continue }

        let dest = rel.split('/')
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
pub(crate) async fn fetch_file_entries(
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
            .send().await?;
        if !resp.status().is_success() {
            return Err(Error::HttpRequestRejected(resp.status().as_u16(), "https://api.curseforge.com/v1/mods/files".to_string()));
        }
        let data = resp.json::<CurseForgeArrayResponse<ModFile>>().await
            .map_err(Error::InvalidResponse)?;
        entries.extend(data.data);
    }
    Ok(entries)
}

pub async fn install_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    download_url: String,
    instance_location: String,
    _category: String,
    origin: DownloadSource,
    api_key: &str,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let http_client = &state.http_client;
    emit_progress(app_handle, id, instance_name, "Downloading modpack", false, None);

    let resp = http_client.get(&download_url).send().await?;
    if !resp.status().is_success() {
        return Err(Error::HttpRequestRejected(resp.status().as_u16(), download_url));
    }
    let zip_bytes = resp.bytes().await.map_err(Error::InvalidResponse)?.to_vec();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))
        .map_err(|e| Error::Invalid(format!("modpack zip is invalid: {e}")))?;

    let (manifest, manifest_raw) = read_manifest(&mut archive)?;
    let mc_version = manifest.minecraft.version.clone();
    let (mod_loader, loader_version) = resolve_loader(&manifest)?;
    let mod_files = manifest_mod_sources(&manifest);

    let version_id = ensure_game_and_loader(
        app_handle, id, instance_name, &mc_version, &mod_loader, &loader_version, state,
    ).await?;

    let instance_path = PathBuf::from(&instance_location).join(instance_name.to_lowercase());
    std::fs::create_dir_all(&instance_path)?;

    emit_progress(app_handle, id, instance_name, "Extracting files", false, None);
    let overrides_prefix = format!("{}/", manifest.overrides.trim_end_matches('/'));
    extract_overrides(&mut archive, &overrides_prefix, &instance_path)?;

    emit_progress(app_handle, id, instance_name, "Downloading mods", false, None);
    let mods_dir = instance_path.join("mods");
    std::fs::create_dir_all(&mods_dir)?;
    let modlist_entries = super::download_mods(
        mod_files,
        &mods_dir,
        api_key,
        http_client,
    ).await?;

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    // Persist the modpack manifest so a later upgrade can diff the mod list.
    std::fs::write(instance_path.join("manifest.json"), &manifest_raw)?;
    upsert_modlist_entries(&instance_path, modlist_entries)?;

    let meta = InstanceMeta {
        name: instance_name.to_string(),
        game_version: mc_version.clone(),
        mod_loader: mod_loader.clone(),
        mod_loader_version: loader_version,
        version_id,
        origin,
        ..InstanceMeta::default()
    };
    write_json(instance_meta_file(&instance_path), &meta)?;

    info!("Installed '{}' (MC {}, {}) → {}", instance_name, mc_version, mod_loader, instance_path.display());
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
    download_url: String,
    project_id: u32,
    new_file_id: u32,
    api_key: &str,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let http_client = &state.http_client;
    emit_progress(app_handle, id, instance_name, "Downloading modpack", false, None);

    let resp = http_client.get(&download_url).send().await?;
    if !resp.status().is_success() {
        return Err(Error::HttpRequestRejected(resp.status().as_u16(), download_url));
    }
    let zip_bytes = resp.bytes().await.map_err(Error::InvalidResponse)?.to_vec();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))
        .map_err(|e| Error::Invalid(format!("modpack zip is invalid: {e}")))?;

    let (manifest, manifest_raw) = read_manifest(&mut archive)?;
    let mc_version = manifest.minecraft.version.clone();
    let (mod_loader, loader_version) = resolve_loader(&manifest)?;

    let version_id = ensure_game_and_loader(
        app_handle, id, instance_name, &mc_version, &mod_loader, &loader_version, state,
    ).await?;

    emit_progress(app_handle, id, instance_name, "Extracting files", false, None);
    let overrides_prefix = format!("{}/", manifest.overrides.trim_end_matches('/'));
    extract_overrides(&mut archive, &overrides_prefix, &instance_path)?;

    // Diff the mod list by file id: download additions, remove drops, skip
    // unchanged files already on disk.
    emit_progress(app_handle, id, instance_name, "Updating mods", false, None);
    let old_ids = installed_manifest_file_ids(&instance_path);
    let new_ids = manifest_file_ids(&manifest);
    let old_set: HashSet<u32> = old_ids.iter().copied().collect();
    let new_set: HashSet<u32> = new_ids.iter().copied().collect();
    let to_remove: Vec<u32> = old_ids.iter().copied().filter(|fid| !new_set.contains(fid)).collect();
    let to_add: Vec<u32> = new_ids.iter().copied().filter(|fid| !old_set.contains(fid)).collect();

    let mut old_file_names = Vec::new();
    if !to_remove.is_empty() {
        let mods_dir = instance_path.join("mods");
        for entry in fetch_file_entries(&to_remove, api_key, http_client).await? {
            old_file_names.push(entry.file_name.clone());
            let path = mods_dir.join(&entry.file_name);
            if path.exists() {
                if let Err(e) = std::fs::remove_file(&path) {
                    log::warn!("failed to remove old mod {}: {e}", entry.file_name);
                }
            }
        }
    }
    let to_add_set: HashSet<u32> = to_add.iter().copied().collect();
    let to_add: Vec<DownloadSource> = manifest.files.iter()
        .filter(|file| file.required && to_add_set.contains(&file.file_id))
        .map(|file| DownloadSource::CurseForge { project_id: file.project_id, file_id: file.file_id })
        .collect();
    let downloaded_modlist_entries = super::download_mods(
        to_add,
        &instance_path,
        api_key,
        http_client,
    ).await?;
    let mut new_modlist_entries: Vec<ModListEntry> = fetch_file_entries(&new_ids, api_key, http_client).await?
        .into_iter()
        .map(|entry| entry.to_modlist_entry())
        .collect();
    for entry in downloaded_modlist_entries {
        new_modlist_entries.retain(|existing| existing.file_name != entry.file_name);
        new_modlist_entries.push(entry);
    }

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    std::fs::write(instance_path.join("manifest.json"), &manifest_raw)?;
    replace_modlist_entries_for_file_ids(
        &instance_path,
        &old_ids,
        &old_file_names,
        new_modlist_entries,
    )?;

    let mut meta: InstanceMeta = read_json(instance_meta_file(&instance_path))?;
    meta.game_version = mc_version.clone();
    meta.mod_loader = mod_loader.clone();
    meta.mod_loader_version = loader_version;
    meta.version_id = version_id;
    meta.origin = DownloadSource::CurseForge { project_id, file_id: new_file_id };
    write_json(instance_meta_file(&instance_path), &meta)?;

    info!("Upgraded '{}' to file {} (MC {}, {})", instance_name, new_file_id, mc_version, mod_loader);
    Ok(())
}
