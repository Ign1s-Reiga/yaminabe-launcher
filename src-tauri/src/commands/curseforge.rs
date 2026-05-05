use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use log::info;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::AppState;

// ── Public game-data types ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McVersion {
    pub id: u32,
    #[serde(rename = "versionString")]
    pub version_string: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McModloader {
    pub name: String,
    #[serde(rename = "gameVersion")]
    pub game_version: String,
    #[serde(rename = "type")]
    pub loader_type: u32,
    pub latest: bool,
    pub recommended: bool,
}

// ── Wire types ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CfMcVersionResponse {
    data: Vec<McVersion>,
}

#[derive(Debug, Deserialize)]
struct CfMcModloaderResponse {
    data: Vec<McModloader>,
}

#[derive(Debug, Deserialize)]
struct CfSearchResponse {
    data: Vec<CfMod>,
}

#[derive(Debug, Deserialize)]
struct CfFilesResponse {
    data: Vec<CfFileEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfFileEntry {
    id: u32,
    mod_id: u32,
    release_type: u32,
    file_name: String,
    download_url: Option<String>,
    display_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfMod {
    id: u32,
    name: String,
    summary: String,
    logo: Option<CfLogo>,
    download_count: f64,
    #[serde(default)]
    latest_files_indexes: Vec<CfFileIndex>,
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

#[derive(Debug, Serialize)]
pub struct ModpackInfo {
    pub id: u32,
    pub name: String,
    pub summary: String,
    pub logo_url: Option<String>,
    pub download_count: u64,
    pub game_versions: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ModpackVersion {
    id: u32,
    mod_id: u32,
    release_type: String,
    file_name: String,
    download_url: String,
    display_name: String,
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

// ── Pre-fetch helpers (called from lib.rs setup) ──────────────────────────────

pub(crate) async fn fetch_minecraft_versions(
    api_key: &str,
    client: &reqwest::Client,
) -> Result<Vec<McVersion>, String> {
    let resp = client
        .get("https://api.curseforge.com/v1/minecraft/version")
        .header("x-api-key", api_key)
        .send().await
        .map_err(|e| format!("Failed to fetch MC versions: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("CurseForge API returned {}", resp.status()));
    }
    let body: CfMcVersionResponse = resp.json().await
        .map_err(|e| format!("Failed to parse MC versions: {e}"))?;
    Ok(body.data)
}

pub(crate) async fn fetch_minecraft_modloaders(
    api_key: &str,
    client: &reqwest::Client,
) -> Result<Vec<McModloader>, String> {
    let resp = client
        .get("https://api.curseforge.com/v1/minecraft/modloader")
        .header("x-api-key", api_key)
        .send().await
        .map_err(|e| format!("Failed to fetch MC modloaders: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("CurseForge API returned {}", resp.status()));
    }
    let body: CfMcModloaderResponse = resp.json().await
        .map_err(|e| format!("Failed to parse MC modloaders: {e}"))?;
    Ok(body.data)
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_minecraft_versions(state: State<'_, AppState>) -> Result<Vec<McVersion>, String> {
    {
        let cache = state.mc_versions.lock().unwrap();
        if !cache.is_empty() {
            return Ok(cache.clone());
        }
    }
    let api_key = state.settings.lock().unwrap().curseforge_api_key.clone();
    if api_key.is_empty() {
        return Ok(vec![]);
    }
    let versions = fetch_minecraft_versions(&api_key, &state.http_client).await?;
    *state.mc_versions.lock().unwrap() = versions.clone();
    Ok(versions)
}

#[tauri::command]
pub async fn get_minecraft_modloaders(state: State<'_, AppState>) -> Result<Vec<McModloader>, String> {
    {
        let cache = state.mc_modloaders.lock().unwrap();
        if !cache.is_empty() {
            return Ok(cache.clone());
        }
    }
    let api_key = state.settings.lock().unwrap().curseforge_api_key.clone();
    if api_key.is_empty() {
        return Ok(vec![]);
    }
    let loaders = fetch_minecraft_modloaders(&api_key, &state.http_client).await?;
    *state.mc_modloaders.lock().unwrap() = loaders.clone();
    Ok(loaders)
}

#[tauri::command]
pub async fn search_curseforge_modpacks(
    query: String,
    index: u32,
    state: State<'_, AppState>,
) -> Result<Vec<ModpackInfo>, String> {
    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    let api_key = state.settings.lock().unwrap().curseforge_api_key.clone();
    let index_str = index.to_string();

    let resp = state.http_client
        .get("https://api.curseforge.com/v1/mods/search")
        .header("x-api-key", &api_key)
        .query(&[
            ("gameId", "432"),
            ("classId", "4471"),
            ("searchFilter", query.as_str()),
            ("sortField", "2"),
            ("sortOrder", "desc"),
            ("pageSize", "20"),
            ("index", index_str.as_str()),
        ])
        .send().await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("CurseForge API returned {}, API key: {}", resp.status(), api_key));
    }

    let body: CfSearchResponse = resp.json().await
        .map_err(|e| format!("Parse error: {e}"))?;

    let results = body.data.into_iter().map(|m| {
        let mut versions: Vec<String> = m.latest_files_indexes.iter()
            .map(|f| f.game_version.clone())
            .collect();
        versions.sort();
        versions.dedup();
        ModpackInfo {
            id: m.id,
            name: m.name,
            summary: m.summary,
            logo_url: m.logo.map(|l| l.url),
            download_count: m.download_count as u64,
            game_versions: versions,
        }
    }).collect();

    Ok(results)
}

#[tauri::command]
pub async fn get_modpack_files(
    mod_id: u32,
    state: State<'_, AppState>,
) -> Result<Vec<ModpackVersion>, String> {
    let api_key = state.settings.lock().unwrap().curseforge_api_key.clone();

    let resp = state.http_client
        .get(format!("https://api.curseforge.com/v1/mods/{mod_id}/files"))
        .header("x-api-key", &api_key)
        .query(&[("pageSize", "50")])
        .send().await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("CurseForge API returned {}", resp.status()));
    }

    let body: CfFilesResponse = resp.json().await
        .map_err(|e| format!("Parse error: {e}"))?;

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
        Some(ModpackVersion {
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
) -> Result<(), String> {
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
            crate::emit_progress(&app_handle, &id, &instance_name, "Done", true, None);
            Ok(())
        }
        Err(e) => {
            crate::emit_progress(&app_handle, &id, &instance_name, "Failed", false, Some(e.clone()));
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
) -> Result<(), String> {

    crate::emit_progress(app_handle, id, instance_name, "Downloading modpack", false, None);

    let resp = http_client
        .get(&download_url)
        .send().await
        .map_err(|e| format!("Download failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Download returned {}", resp.status()));
    }

    let zip_bytes = resp.bytes().await
        .map_err(|e| format!("Failed to read download: {e}"))?.to_vec();

    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| format!("Invalid zip: {e}"))?;

    let manifest: CurseForgeModpackManifest = {
        let mut file = archive.by_name("manifest.json")
            .map_err(|_| "modpack is missing manifest.json".to_string())?;
        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| format!("Failed to read manifest.json: {e}"))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse manifest.json: {e}"))?
    };

    let mc_version = manifest.minecraft.version.clone();
    let mod_tool = manifest.minecraft.mod_loaders.iter()
        .find(|l| l.primary)
        .or_else(|| manifest.minecraft.mod_loaders.first())
        .map(|l| loader_id_to_mod_tool(&l.id))
        .unwrap_or("Vanilla");
    let file_ids: Vec<u32> = manifest.files.iter()
        .filter(|f| f.required)
        .map(|f| f.file_id)
        .collect();

    let instance_path = PathBuf::from(&instance_location).join(instance_name.to_lowercase());
    std::fs::create_dir_all(&instance_path)
        .map_err(|e| format!("Failed to create instance directory: {e}"))?;

    let overrides_prefix = format!("{}/", manifest.overrides.trim_end_matches('/'));

    crate::emit_progress(app_handle, id, instance_name, "Extracting files", false, None);

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)
            .map_err(|e| format!("Zip read error at index {i}: {e}"))?;

        let entry_name = file.name().to_string();
        let Some(rel) = entry_name.strip_prefix(&overrides_prefix) else { continue };
        if rel.is_empty() { continue }

        let dest = rel.split('/')
            .filter(|c| !c.is_empty() && *c != "..")
            .fold(instance_path.clone(), |p, c| p.join(c));

        if entry_name.ends_with('/') {
            std::fs::create_dir_all(&dest)
                .map_err(|e| format!("Failed to create dir {}: {e}", dest.display()))?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent dir: {e}"))?;
            }
            let mut out = std::fs::File::create(&dest)
                .map_err(|e| format!("Failed to create {}: {e}", dest.display()))?;
            std::io::copy(&mut file, &mut out)
                .map_err(|e| format!("Failed to write {}: {e}", dest.display()))?;
        }
    }

    crate::emit_progress(app_handle, id, instance_name, "Downloading mods", false, None);
    download_mods_core(file_ids, instance_path.to_str().unwrap_or_default(), api_key, http_client).await?;

    crate::emit_progress(app_handle, id, instance_name, "Finalizing", false, None);

    let meta_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string();

    let meta = serde_json::json!({
        "id":        meta_id,
        "name":      instance_name,
        "mc_version": mc_version,
        "mod_tool":  mod_tool,
        "category":  category,
        "ram_mb":    4096,
        "java_args": "",
    });

    std::fs::write(
        instance_path.join("instance.json"),
        serde_json::to_string_pretty(&meta).unwrap(),
    ).map_err(|e| format!("Failed to write instance.json: {e}"))?;

    info!("Installed '{}' (MC {}, {}) → {}", instance_name, mc_version, mod_tool, instance_path.display());
    Ok(())
}

pub(crate) async fn download_mods_core(
    file_ids: Vec<u32>,
    instance_location: &str,
    api_key: &str,
    client: &reqwest::Client,
) -> Result<(), String> {
    if file_ids.is_empty() {
        return Ok(());
    }

    let mods_dir = PathBuf::from(instance_location).join("mods");
    std::fs::create_dir_all(&mods_dir)
        .map_err(|e| format!("Failed to create mods directory: {e}"))?;

    let mut file_entries: Vec<CfFileEntry> = Vec::new();
    for chunk in file_ids.chunks(50) {
        let body = serde_json::json!({ "fileIds": chunk });
        let resp = client
            .post("https://api.curseforge.com/v1/mods/files")
            .header("x-api-key", api_key)
            .json(&body)
            .send().await
            .map_err(|e| format!("Mod info request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "CurseForge API returned {} while fetching mod info",
                resp.status()
            ));
        }
        let data: CfFilesResponse = resp.json().await
            .map_err(|e| format!("Failed to parse mod file info: {e}"))?;
        file_entries.extend(data.data);
    }

    let client = client.clone();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(3));
    let mut handles: Vec<tokio::task::JoinHandle<Result<(), String>>> = Vec::new();

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
                .map_err(|e| format!("Semaphore error: {e}"))?;
            let resp = client.get(&url).send().await
                .map_err(|e| format!("Download failed for {file_name}: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("Download of {file_name} returned {}", resp.status()));
            }
            let bytes = resp.bytes().await
                .map_err(|e| format!("Failed to read {file_name}: {e}"))?;
            std::fs::write(mods_dir.join(&file_name), &bytes)
                .map_err(|e| format!("Failed to write {file_name}: {e}"))?;
            info!("Downloaded {file_name}");
            Ok(())
        }));
    }

    for handle in handles {
        handle.await.map_err(|e| format!("Download task failed: {e}"))??;
    }

    Ok(())
}
