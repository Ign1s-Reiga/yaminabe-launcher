use std::io::Read;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use log::info;
use serde::{Deserialize, Serialize};
use tauri::State;
use yaminabe_launcher_shared::datatypes::{InstanceMeta, ModLoader, ModpackInfo, ModpackSearchResults, ModpackVersionFile};
use crate::{emit_progress, AppState};
use yaminabe_launcher_shared::error::Error;
use crate::http_utils::fetch_json;

// ── Wire types ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CurseForgeArrayResponse<T>
{
    data: Vec<T>
}

#[derive(Debug, Deserialize)]
struct CurseForgePaginatedResponse<T> {
    data: Vec<T>,
    pagination: CfPagination,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfPagination {
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

fn loader_id_to_mod_tool(id: &str) -> &'static str {
    let s = id.to_ascii_lowercase();
    if s.starts_with("neoforge")     { "NeoForge" }
    else if s.starts_with("forge")   { "Forge" }
    else if s.starts_with("fabric")  { "Fabric" }
    else if s.starts_with("quilt")   { "Quilt" }
    else                             { "Vanilla" }
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn search_curseforge_modpacks(
    query: String,
    index: u32,
    state: State<'_, AppState>,
) -> Result<ModpackSearchResults, Error> {
    if query.trim().is_empty() {
        return Ok(ModpackSearchResults { items: vec![], total: 0 });
    }

    let api_key = state.settings.lock().unwrap().curseforge_api_key.clone();
    let index_str = index.to_string();

    let body = fetch_json::<CurseForgePaginatedResponse<SearchModsEntry>>(
        &state.http_client,
        "https://api.curseforge.com/v1/mods/search",
        &[
            ("gameId", "432"),
            ("classId", "4471"),
            ("searchFilter", query.as_str()),
            ("sortField", "2"),
            ("pageSize", "50"),
            ("sortOrder", "desc"),
            ("index", index_str.as_str()),
        ],
        Some(api_key),
    ).await?;

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

#[tauri::command]
pub async fn get_modpack_files(
    mod_id: u32,
    state: State<'_, AppState>,
) -> Result<Vec<ModpackVersionFile>, Error> {
    let api_key = state.settings.lock().unwrap().curseforge_api_key.clone();

    let body = fetch_json::<CurseForgeArrayResponse<ModFilesEntry>>(
        &state.http_client,
        &format!("https://api.curseforge.com/v1/mods/{mod_id}/files"),
        &[("pageSize", "50")],
        Some(api_key),
    ).await?;

    let mut entries = body.data;
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

#[tauri::command]
pub async fn install_curseforge_modpack(
    app_handle: tauri::AppHandle,
    download_url: String,
    instance_name: String,
    install_dir: String,
    category: String,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string();

    let api_key = state.settings.lock().unwrap().curseforge_api_key.clone();

    let result = install_modpack_inner(
        &app_handle, &id, &instance_name,
        download_url, install_dir, category,
        &api_key, &state.http_client,
    ).await;

    match result {
        Ok(()) => {
            emit_progress(&app_handle, &id, &instance_name, "Done", true, None);
            Ok(())
        }
        Err(e) => {
            emit_progress(&app_handle, &id, &instance_name, "Failed", false, Some(e.to_string()));
            Err(e)
        }
    }
}

async fn install_modpack_inner(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    download_url: String,
    instance_location: String,
    category: String,
    api_key: &str,
    http_client: &reqwest::Client,
) -> Result<(), Error> {
    emit_progress(app_handle, id, instance_name, "Downloading modpack", false, None);

    let resp = http_client
        .get(&download_url)
        .send().await?;

    if !resp.status().is_success() {
        return Err(Error::HttpRequestRejected(resp.status().as_u16(), download_url));
    }

    let zip_bytes = resp.bytes().await
        .map_err(Error::InvalidResponse)?.to_vec();

    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| Error::Invalid(format!("modpack zip is invalid: {e}")))?;

    let manifest: CurseForgeModpackManifest = {
        let mut file = archive.by_name("manifest.json")
            .map_err(|_| Error::Invalid("modpack zip is missing manifest.json".to_string()))?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        serde_json::from_str(&content)?
    };

    let mc_version = manifest.minecraft.version.clone();
    let mod_loader = manifest.minecraft.mod_loaders.iter()
        .find(|l| l.primary)
        .or_else(|| manifest.minecraft.mod_loaders.first())
        .map(|l| l.id.to_ascii_lowercase())
        .unwrap_or("vanilla".to_string());
    let file_ids: Vec<u32> = manifest.files.iter()
        .filter(|f| f.required)
        .map(|f| f.file_id)
        .collect();

    let instance_path = PathBuf::from(&instance_location).join(instance_name.to_lowercase());
    std::fs::create_dir_all(&instance_path)?;

    let overrides_prefix = format!("{}/", manifest.overrides.trim_end_matches('/'));

    emit_progress(app_handle, id, instance_name, "Extracting files", false, None);

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)
            .map_err(|e| Error::Invalid(format!("reading zip entry at index {i}: {e}")))?;

        let entry_name = file.name().to_string();
        let Some(rel) = entry_name.strip_prefix(&overrides_prefix) else { continue };
        if rel.is_empty() { continue }

        let dest = rel.split('/')
            .filter(|c| !c.is_empty() && *c != "..")
            .fold(instance_path.clone(), |p, c| p.join(c));

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

    emit_progress(app_handle, id, instance_name, "Downloading mods", false, None);
    download_mods_core(file_ids, instance_path.to_str().unwrap_or_default(), api_key, http_client).await?;

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);

    let mod_loader = ModLoader::from_str(mod_loader.as_str()).unwrap();
    let meta = InstanceMeta {
        name: instance_name.to_string(),
        game_version: mc_version.clone(),
        mod_loader: mod_loader.clone(),
        ..InstanceMeta::default()
    };

    std::fs::write(
        instance_path.join("instance.json"),
        serde_json::to_string_pretty(&meta).unwrap(),
    )?;

    info!("Installed '{}' (MC {}, {}) → {}", instance_name, mc_version, mod_loader, instance_path.display());
    Ok(())
}

pub(crate) async fn download_mods_core(
    file_ids: Vec<u32>,
    instance_location: &str,
    api_key: &str,
    client: &reqwest::Client,
) -> Result<(), Error> {
    if file_ids.is_empty() {
        return Ok(());
    }

    let mods_dir = PathBuf::from(instance_location).join("mods");
    std::fs::create_dir_all(&mods_dir)?;

    let mut file_entries: Vec<ModFilesEntry> = Vec::new();
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
        file_entries.extend(data.data);
    }

    let client = client.clone();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(3));
    let mut handles: Vec<tokio::task::JoinHandle<Result<(), Error>>> = Vec::new();

    for entry in file_entries {
        let Some(url) = entry.download_url else {
            info!("Skipping {} (distribution restricted)", entry.file_name);
            continue;
        };
        let client = client.clone();
        let mods_dir = mods_dir.clone();
        let sem = Arc::clone(&semaphore);
        let file_name = entry.file_name.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await
                .map_err(|e| Error::ChildProcess(format!("semaphore acquire: {e}")))?;
            let resp = client.get(&url).send().await?;
            if !resp.status().is_success() {
                return Err(Error::HttpRequestRejected(resp.status().as_u16(), url));
            }
            let bytes = resp.bytes().await.map_err(Error::InvalidResponse)?;
            std::fs::write(mods_dir.join(&file_name), &bytes)?;
            info!("Downloaded {file_name}");
            Ok(())
        }));
    }

    for handle in handles {
        handle.await.map_err(|e| Error::ChildProcess(format!("download task panicked: {e}")))?? ;
    }

    Ok(())
}
