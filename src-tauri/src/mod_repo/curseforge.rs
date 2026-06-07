use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use log::info;
use serde::Deserialize;
use tauri::State;
use yaminabe_launcher_shared::datatypes::{
    DownloadSource, InstanceMeta, InstanceOrigin, ModListEntry, ModLoader, ModpackInfo,
    ModpackSearchResults, ModpackVersionFile,
};
use yaminabe_launcher_shared::error::Error;
use crate::{emit_progress, AppState};
use crate::commands::instance::{
    instance_meta_file, replace_modlist_entries_for_file_ids, upsert_modlist_entries,
};
use crate::http_utils::fetch_json;
use crate::install_task::{ensure_fabric, ensure_forge, ensure_neoforge, ensure_quilt, ensure_vanilla};
use crate::json::{read_json, write_json};

// ── Wire types ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CurseForgeArrayResponse<T>
{
    data: Vec<T>
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
struct ModFilesEntry {
    id: u32,
    mod_id: u32,
    release_type: u32,
    file_name: String,
    download_url: Option<String>,
    display_name: String,
    #[serde(default)]
    hashes: Vec<CfFileHash>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct CfFileHash {
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
    logo: Option<CfLogo>,
    download_count: u32,
    #[serde(default)]
    latest_files_indexes: Vec<CfFileIndex>,
}

#[derive(Debug, Deserialize)]
struct CategoryItem {
    id: u32,
    name: String,
}

#[derive(Debug, Deserialize)]
struct CfLogo {
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfFileIndex {
    game_version: String,
}

// ── Modpack manifest ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CurseForgeModpackManifest {
    minecraft: CfManifestMinecraft,
    #[serde(default = "default_overrides_dir")]
    overrides: String,
    #[serde(default)]
    files: Vec<CfManifestFile>,
}

#[derive(Debug, Deserialize)]
struct CfManifestFile {
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

fn to_search_results(body: CurseForgePaginatedResponse<SearchModsEntry>) -> ModpackSearchResults {
    let total = body.pagination.total_count;
    let items: Vec<ModpackInfo> = body.data.into_iter().map(|m| {
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

        ModpackInfo {
            id: m.id,
            name: m.name,
            summary: m.summary,
            logo_url: m.logo.map(|l| l.url),
            download_count: m.download_count,
            game_versions: versions,
            category,
            primary_category_id,
        }
    }).collect();

    ModpackSearchResults { items, total }
}

pub async fn search_modpacks(
    query: &str,
    index: u32,
    http_client: &reqwest::Client,
    api_key: &str,
) -> Result<ModpackSearchResults, Error> {
    if query.trim().is_empty() {
        return Ok(ModpackSearchResults { items: vec![], total: 0 });
    }

    let index_str = index.to_string();

    let body = fetch_json(http_client, "https://api.curseforge.com/v1/mods/search")
        .header("x-api-key", api_key)
        .query(&[
            ("gameId", "432"),
            ("classId", "4471"),
            ("searchFilter", query),
            ("sortField", "2"),
            ("pageSize", "50"),
            ("sortOrder", "desc"),
            ("index", index_str.as_str()),
        ])
        .send::<CurseForgePaginatedResponse<SearchModsEntry>>()
        .await?;

    Ok(to_search_results(body))
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
) -> Result<ModpackSearchResults, Error> {
    if query.trim().is_empty() {
        return Ok(ModpackSearchResults { items: vec![], total: 0 });
    }

    let index_str = index.to_string();

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
            ("index", index_str.as_str()),
        ])
        .send::<CurseForgePaginatedResponse<SearchModsEntry>>()
        .await?;

    Ok(to_search_results(body))
}

/// Download the newest file of `project_id` that is compatible with the given
/// Minecraft version + mod loader into the instance's `mods/` directory.
pub async fn install_mod(
    instance_path: &Path,
    project_id: u32,
    mc_version: &str,
    mod_loader: &ModLoader,
    api_key: &str,
    client: &reqwest::Client,
) -> Result<Vec<ModListEntry>, Error> {
    let entries = fetch_json(client, &format!("https://api.curseforge.com/v1/mods/{project_id}/files"))
        .header("x-api-key", api_key)
        .query(&[
            ("gameVersion", mc_version),
            ("modLoaderType", mod_loader_type(mod_loader)),
            ("pageSize", "50"),
        ])
        .send::<CurseForgeArrayResponse<ModFilesEntry>>()
        .await?
        .data;

    // Newest downloadable file wins (CurseForge ids increase over time).
    let file = entries.into_iter()
        .filter(|f| f.download_url.is_some())
        .max_by_key(|f| f.id)
        .ok_or_else(|| Error::NotExists(format!("compatible file for mod {project_id}")))?;

    let mods_dir = instance_path.join("mods");
    let url = file.download_url.clone().unwrap_or_default();
    let downloaded = super::download_files(client, vec![(url, file.file_name.clone())], &mods_dir).await?;
    let downloaded_sha1 = downloaded.first().map(|file| file.sha1.as_str());
    Ok(vec![registry_entry(&file, downloaded_sha1)])
}

pub async fn get_modpack_files(
    mod_id: u32,
    http_client: &reqwest::Client,
    api_key: &str,
) -> Result<Vec<ModpackVersionFile>, Error> {
    let mut entries = fetch_json(
        http_client,
        &format!("https://api.curseforge.com/v1/mods/{mod_id}/files")
    )
        .header("x-api-key", api_key)
        .query(&[("pageSize", "50")])
        .send::<CurseForgeArrayResponse<ModFilesEntry>>().await?
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
        Some(ModpackVersionFile {
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
) -> Result<(CurseForgeModpackManifest, String), Error> {
    let mut file = archive.by_name("manifest.json")
        .map_err(|_| Error::Invalid("modpack zip is missing manifest.json".to_string()))?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    let manifest = serde_json::from_str(&content)?;
    Ok((manifest, content))
}

/// Resolve the modpack's primary mod loader and its version. CurseForge
/// manifest loader ids are `{name}-{version}` (e.g. `neoforge-21.1.228`).
fn resolve_loader(manifest: &CurseForgeModpackManifest) -> Result<(ModLoader, Option<String>), Error> {
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

fn manifest_file_ids(manifest: &CurseForgeModpackManifest) -> Vec<u32> {
    manifest.files.iter().filter(|f| f.required).map(|f| f.file_id).collect()
}

fn registry_entry(
    entry: &ModFilesEntry,
    downloaded_sha1: Option<&str>,
) -> ModListEntry {
    let sha1 = entry_sha1(entry)
        .map(str::to_string)
        .or_else(|| downloaded_sha1.map(|sha1| sha1.to_string()))
        .unwrap_or_default();

    ModListEntry {
        file_name: entry.file_name.clone(),
        sha1,
        project_id: entry.mod_id,
        file_id: entry.id,
        download_source: DownloadSource::CurseForge,
    }
}

fn entry_sha1(entry: &ModFilesEntry) -> Option<&str> {
    entry.hashes.iter()
        .find(|hash| hash.algo == 1)
        .map(|hash| hash.value.as_str())
        .filter(|hash| !hash.is_empty())
}

/// Required mod file ids recorded in an instance's persisted `manifest.json`.
/// Returns an empty list when the file is missing or unreadable, so the upgrade
/// diff safely degrades to "download everything, remove nothing".
fn installed_manifest_file_ids(instance_path: &Path) -> Vec<u32> {
    std::fs::read_to_string(instance_path.join("manifest.json")).ok()
        .and_then(|c| serde_json::from_str::<CurseForgeModpackManifest>(&c).ok())
        .map(|m| manifest_file_ids(&m))
        .unwrap_or_default()
}

/// Ensure the vanilla client and (if any) the mod loader for `mc_version` are
/// installed, returning the `versions/<id>` the launch path should use. Shared
/// by install and upgrade.
async fn ensure_game_and_loader(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    mc_version: &str,
    mod_loader: &ModLoader,
    loader_version: &Option<String>,
    state: &State<'_, AppState>,
) -> Result<String, Error> {
    let http_client = &state.http_client;
    emit_progress(app_handle, id, instance_name, &format!("Installing Minecraft {mc_version}"), false, None);
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
async fn fetch_file_entries(
    file_ids: &[u32],
    api_key: &str,
    client: &reqwest::Client,
) -> Result<Vec<ModFilesEntry>, Error> {
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
        let data = resp.json::<CurseForgeArrayResponse<ModFilesEntry>>().await
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
    origin: InstanceOrigin,
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
    let file_ids = manifest_file_ids(&manifest);

    let version_id = ensure_game_and_loader(
        app_handle, id, instance_name, &mc_version, &mod_loader, &loader_version, state,
    ).await?;

    let instance_path = PathBuf::from(&instance_location).join(instance_name.to_lowercase());
    std::fs::create_dir_all(&instance_path)?;

    emit_progress(app_handle, id, instance_name, "Extracting files", false, None);
    let overrides_prefix = format!("{}/", manifest.overrides.trim_end_matches('/'));
    extract_overrides(&mut archive, &overrides_prefix, &instance_path)?;

    emit_progress(app_handle, id, instance_name, "Downloading mods", false, None);
    let modlist_entries = download_mods(file_ids, instance_path.to_str().unwrap_or_default(), api_key, http_client).await?;

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

    if !to_remove.is_empty() {
        let mods_dir = instance_path.join("mods");
        for entry in fetch_file_entries(&to_remove, api_key, http_client).await? {
            let path = mods_dir.join(&entry.file_name);
            if path.exists() {
                if let Err(e) = std::fs::remove_file(&path) {
                    log::warn!("failed to remove old mod {}: {e}", entry.file_name);
                }
            }
        }
    }
    let downloaded_modlist_entries = download_mods(
        to_add,
        instance_path.to_str().unwrap_or_default(),
        api_key,
        http_client,
    ).await?;
    let mut new_modlist_entries: Vec<ModListEntry> = fetch_file_entries(&new_ids, api_key, http_client).await?
        .into_iter()
        .map(|entry| registry_entry(&entry, None))
        .collect();
    for entry in downloaded_modlist_entries {
        new_modlist_entries.retain(|existing| existing.file_name != entry.file_name);
        new_modlist_entries.push(entry);
    }

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    std::fs::write(instance_path.join("manifest.json"), &manifest_raw)?;
    replace_modlist_entries_for_file_ids(
        &instance_path,
        DownloadSource::CurseForge,
        &old_ids,
        new_modlist_entries,
    )?;

    let mut meta: InstanceMeta = read_json(instance_meta_file(&instance_path))?;
    meta.game_version = mc_version.clone();
    meta.mod_loader = mod_loader.clone();
    meta.mod_loader_version = loader_version;
    meta.version_id = version_id;
    meta.origin = InstanceOrigin::CurseForge { project_id, file_id: new_file_id };
    write_json(instance_meta_file(&instance_path), &meta)?;

    info!("Upgraded '{}' to file {} (MC {}, {})", instance_name, new_file_id, mc_version, mod_loader);
    Ok(())
}

pub async fn download_mods(
    file_ids: Vec<u32>,
    instance_location: &str,
    api_key: &str,
    client: &reqwest::Client,
) -> Result<Vec<ModListEntry>, Error> {
    if file_ids.is_empty() {
        return Ok(vec![]);
    }

    let mods_dir = PathBuf::from(instance_location).join("mods");
    let mut modlist = Vec::new();
    for entry in fetch_file_entries(&file_ids, api_key, client).await? {
        let primary = match entry.download_url.as_ref() {
            Some(url) => {
                super::download_files(client, vec![(url.clone(), entry.file_name.clone())], &mods_dir).await
            }
            None => Err(Error::Invalid(format!("CurseForge file {} has no download URL", entry.file_name))),
        };

        match primary {
            Ok(downloaded) => {
                let downloaded_sha1 = downloaded.first().map(|file| file.sha1.as_str());
                modlist.push(registry_entry(&entry, downloaded_sha1));
            }
            Err(primary_error) => {
                log::warn!("CurseForge download failed for {}: {primary_error}", entry.file_name);
                let Some(sha1) = entry_sha1(&entry) else {
                    return Err(Error::Invalid(format!(
                        "CurseForge download failed for {} and no SHA-1 was available for Modrinth fallback",
                        entry.file_name
                    )));
                };
                let fallback = super::modrinth::download_mod_by_sha1(
                    sha1,
                    &entry.file_name,
                    instance_location,
                    client,
                ).await;
                match fallback {
                    Ok(entry) => modlist.push(entry),
                    Err(fallback_error) => {
                        return Err(Error::Invalid(format!(
                            "CurseForge download failed for {}; Modrinth fallback also failed: {fallback_error}",
                            entry.file_name
                        )));
                    }
                }
            }
        }
    }
    Ok(modlist)
}
