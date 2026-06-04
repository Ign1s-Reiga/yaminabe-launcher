use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use log::info;
use serde::Deserialize;
use tauri::State;
use yaminabe_launcher_shared::datatypes::{InstanceMeta, InstanceOrigin, ModLoader, ModpackInfo, ModpackSearchResults, ModpackVersionFile};
use yaminabe_launcher_shared::error::Error;
use crate::{emit_progress, AppState};
use crate::http_utils::fetch_json;
use crate::install_task::{ensure_fabric, ensure_forge, ensure_neoforge, ensure_quilt, ensure_vanilla};

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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModFilesEntry {
    id: u32,
    mod_id: u32,
    release_type: u32,
    file_name: String,
    download_url: Option<String>,
    display_name: String,
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

    Ok(ModpackSearchResults { items, total })
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
    download_mods(file_ids, instance_path.to_str().unwrap_or_default(), api_key, http_client).await?;

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    // Persist the modpack manifest so a later upgrade can diff the mod list.
    std::fs::write(instance_path.join("manifest.json"), &manifest_raw)?;

    let meta = InstanceMeta {
        name: instance_name.to_string(),
        game_version: mc_version.clone(),
        mod_loader: mod_loader.clone(),
        mod_loader_version: loader_version,
        version_id,
        origin,
        ..InstanceMeta::default()
    };
    std::fs::write(
        instance_path.join("instance.json"),
        serde_json::to_string_pretty(&meta).unwrap(),
    )?;

    info!("Installed '{}' (MC {}, {}) → {}", instance_name, mc_version, mod_loader, instance_path.display());
    Ok(())
}

/// Upgrade an existing CurseForge instance to a newer modpack file: re-ensure
/// the (possibly changed) game + loader, overlay the new `overrides/`, diff the
/// mod list against the instance's recorded manifest — downloading only new
/// mods and removing only mods the pack dropped — then update `instance.json`.
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
    download_mods(to_add, instance_path.to_str().unwrap_or_default(), api_key, http_client).await?;

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    std::fs::write(instance_path.join("manifest.json"), &manifest_raw)?;

    let meta_path = instance_path.join("instance.json");
    let mut meta: InstanceMeta = serde_json::from_str(&std::fs::read_to_string(&meta_path)?)?;
    meta.game_version = mc_version.clone();
    meta.mod_loader = mod_loader.clone();
    meta.mod_loader_version = loader_version;
    meta.version_id = version_id;
    meta.origin = InstanceOrigin::CurseForge { project_id, file_id: new_file_id };
    std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;

    info!("Upgraded '{}' to file {} (MC {}, {})", instance_name, new_file_id, mc_version, mod_loader);
    Ok(())
}

pub async fn download_mods(
    file_ids: Vec<u32>,
    instance_location: &str,
    api_key: &str,
    client: &reqwest::Client,
) -> Result<(), Error> {
    if file_ids.is_empty() {
        return Ok(());
    }

    let mods_dir = PathBuf::from(instance_location).join("mods");
    let files: Vec<(String, String)> = fetch_file_entries(&file_ids, api_key, client).await?
        .into_iter()
        .filter_map(|entry| match entry.download_url {
            Some(url) => Some((url, entry.file_name)),
            None => {
                info!("Skipping {} (distribution restricted)", entry.file_name);
                None
            }
        })
        .collect();

    super::download_files(client, files, &mods_dir).await
}