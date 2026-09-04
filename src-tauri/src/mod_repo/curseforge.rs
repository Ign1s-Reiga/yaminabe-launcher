use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::commands::instance::{
    create_instance_dir, instance_meta_file, modlist_file, replace_modlist_entries_for_file_ids,
    upsert_modlist_entries,
};
use crate::http_utils::{download_resource, fetch_json, rejected_status};
use crate::install_task::ensure_game_and_loader;
use crate::json::{read_json, read_json_or_default, write_json};
use crate::{AppState, emit_progress};
use log::info;
use serde::Deserialize;
use tauri::State;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, InstanceMeta, LocalModpackInfo, ModListEntry, ModLoader, ModProjectInfo,
    ModProjectSearchResults, ModState, ModpackFormat, Platform, ProjectFileInfo,
    ProjectFileTarget, SearchOptions,
};
use yaminabe_launcher_shared::error::Error;

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
    /// CurseForge omits this for some files (10 of FTB StoneBlock 4's 420, for
    /// one), and a required field there fails the whole 50-file chunk. It only
    /// feeds a size label, so an absent or null value is worth 0, not an abort.
    #[serde(default)]
    file_size_on_disk: Option<u64>,
    #[serde(default)]
    hashes: Vec<FileHash>,
}

impl ModFile {
    fn to_project_file_info(
        &self,
        target: ProjectFileTarget,
        project: Option<&ProjectSummary>,
    ) -> ProjectFileInfo {
        ProjectFileInfo {
            source: DownloadSource::CurseForge {
                project_id: self.mod_id,
                file_id: self.id,
            },
            target,
            release_type: self.release_type.into(),
            file_name: self.file_name.clone(),
            download_url: self.download_url.clone(),
            display_name: self.display_name.clone(),
            project_name: project.map(|p| p.name.clone()).unwrap_or_default(),
            icon_url: project.and_then(|p| p.icon_url.clone()),
            sha1: self
                .hashes
                .iter()
                .find(|h| h.algo == 1)
                .map(|h| h.value.clone())
                .unwrap_or_default(),
            size: self.file_size_on_disk.unwrap_or_default(),
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
    download_count: u64,
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
struct ProjectMetadata {
    id: u32,
    /// Absent for a project CurseForge has not classified; such a project is
    /// treated as a mod rather than failing the whole batch.
    #[serde(default)]
    class_id: Option<u32>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    logo: Option<Logo>,
    #[serde(default)]
    links: Option<ProjectLinks>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectLinks {
    #[serde(default)]
    website_url: String,
}

/// What `/v1/mods` tells us about a project beyond its files: where its files
/// belong on disk, and the name, blurb and icon the site shows for it.
#[derive(Debug, Clone)]
struct ProjectSummary {
    target: ProjectFileTarget,
    name: String,
    summary: String,
    icon_url: Option<String>,
    website_url: String,
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
    /// Shown before installing a pack picked off disk. Optional, since nothing
    /// in the install itself depends on what the pack calls itself.
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    author: String,
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
            f.game_version == mc_version && (f.mod_loader == mod_loader.mod_loader_id() || f.mod_loader.is_none())
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
                platform: Platform::CurseForge,
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

/// Unified CurseForge search for any project class. Mods can be narrowed to a
/// Minecraft version and/or mod loader (so only compatible files are offered);
/// modpacks and resource packs search broadly. When both a version and loader
/// are given, the per-result `file_id` is the matching file rather than the
/// latest.
pub async fn search_projects(
    option: &SearchOptions,
    http_client: &reqwest::Client,
    api_key: &str,
) -> Result<ModProjectSearchResults, Error> {
    let index = option.index.to_string();
    let mut query: Vec<(&str, &str)> = vec![
        ("gameId", "432"),
        ("classId", option.target.to_curseforge_type()),
        ("pageSize", "50"),
        ("index", &index),
    ];
    // An empty query is a browse (popular/sorted listing), so the filter is
    // omitted rather than sent empty.
    if !option.query.trim().is_empty() {
        query.push(("searchFilter", option.query.as_str()));
    }
    // Relevancy maps to an omitted `sortField` (the API's default ranking);
    // `sortOrder` only applies alongside an explicit field.
    let sort_field = option.sort.to_curseforge_field();
    if !sort_field.is_empty() {
        query.push(("sortField", sort_field));
        query.push(("sortOrder", "desc"));
    }
    if let Some(game_version) = option.game_version.as_deref() {
        query.push(("gameVersion", game_version));
    }
    // `mod_loader_id` is `None` for Vanilla, which has no loader filter.
    let loader_id;
    if let Some(id) = option.mod_loader.as_ref().and_then(ModLoader::mod_loader_id) {
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
    target: ProjectFileTarget,
    game_version: Option<&str>,
    mod_loader: Option<&ModLoader>,
    index: u32,
    http_client: &reqwest::Client,
    api_key: &str,
) -> Result<Vec<ProjectFileInfo>, Error> {
    let index = index.to_string();
    let mut query: Vec<(&str, &str)> = vec![("pageSize", "50"), ("index", &index)];
    if let Some(game_version) = game_version {
        query.push(("gameVersion", game_version));
    }
    let loader_id;
    if let Some(id) = mod_loader.and_then(ModLoader::mod_loader_id) {
        loader_id = id.to_string();
        query.push(("modLoaderType", &loader_id));
    }

    let mut entries = fetch_json(
        http_client,
        &format!("https://api.curseforge.com/v1/mods/{mod_id}/files"),
    )
    .header("x-api-key", api_key)
    .query(&query)
    .send::<CurseForgeArrayResponse<ModFile>>()
    .await?
    .data;

    entries.sort_by(|a, b| b.id.cmp(&a.id));

    // These files reach the modlist through `download_mods`, so resolve the
    // project once here — the entries themselves carry no name or icon. It is
    // display-only, so a rate-limited or failing lookup costs the names, never
    // the version list itself.
    let projects = fetch_project_summaries(&[mod_id], api_key, http_client)
        .await
        .unwrap_or_else(|e| {
            log::warn!("no project details for CurseForge project {mod_id}: {e}");
            HashMap::new()
        });
    let project = projects.get(&mod_id);
    let versions = entries
        .iter()
        .map(|file| file.to_project_file_info(target, project))
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

/// What the modlist records for one CurseForge file the instance holds.
struct InstalledFile {
    state: ModState,
    file_name: String,
}

/// CurseForge file id → what the instance's modlist records for it: the state an
/// upgrade carries forward, and the name on disk to delete when the pack drops
/// the file. The modlist is the source of truth for what is installed and is only
/// rewritten when an upgrade finalizes, so a failed (and retried) upgrade keeps
/// diffing against the previous set. Empty when the modlist is missing, degrading
/// to "download everything, remove nothing".
fn installed_curseforge_files(instance_path: &Path) -> HashMap<u32, InstalledFile> {
    let modlist: Vec<ModListEntry> = read_json_or_default(modlist_file(instance_path)).unwrap_or_default();
    modlist
        .into_iter()
        .filter_map(|entry| {
            let (_, file_id) = entry.source.curseforge_ids()?;
            Some((
                file_id,
                InstalledFile { state: entry.state, file_name: entry.file_name },
            ))
        })
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

        // Entry names come from an untrusted zip. Split on both separators —
        // Windows treats `\` as one — and drop every component that could climb
        // out: `..`, and anything holding a drive prefix like `C:`, which `join`
        // would take as an absolute path replacing everything before it.
        let components: Vec<&str> = rel
            .split(['/', '\\'])
            .filter(|c| !c.is_empty() && *c != "." && *c != ".." && !c.contains(':'))
            .collect();
        if components.is_empty() {
            continue;
        }
        let dest = components
            .iter()
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

/// Resolve CurseForge file metadata for a set of file ids via POST
/// /v1/mods/files, batching at the API's 50-per-request limit. Maps each file
/// to the cross-platform `ProjectFileInfo` the download pipeline consumes; used
/// by the modpack install/upgrade flows.
pub(crate) async fn resolve_project_files(
    file_ids: &[u32],
    api_key: &str,
    client: &reqwest::Client,
) -> Result<Vec<ProjectFileInfo>, Error> {
    let mut files = Vec::new();
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
        let projects = fetch_project_summaries(
            &data.data.iter().map(|file| file.mod_id).collect::<Vec<_>>(),
            api_key,
            client,
        )
        .await?;
        files.extend(data.data.iter().map(|file| {
            let project = projects.get(&file.mod_id);
            let target = project.map(|p| p.target).unwrap_or(ProjectFileTarget::Mod);
            file.to_project_file_info(target, project)
        }));
    }
    Ok(files)
}

async fn fetch_project_summaries(
    project_ids: &[u32],
    api_key: &str,
    client: &reqwest::Client,
) -> Result<HashMap<u32, ProjectSummary>, Error> {
    let mut summaries = HashMap::new();
    for chunk in project_ids.chunks(50) {
        let body = serde_json::json!({ "modIds": chunk });
        let resp = client
            .post("https://api.curseforge.com/v1/mods")
            .header("x-api-key", api_key)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(rejected_status(
                resp.status(),
                "https://api.curseforge.com/v1/mods",
            ));
        }
        let data = resp
            .json::<CurseForgeArrayResponse<ProjectMetadata>>()
            .await
            .map_err(Error::InvalidResponse)?;
        summaries.extend(data.data.into_iter().map(|project| {
            (
                project.id,
                ProjectSummary {
                    target: project
                        .class_id
                        .map(ProjectFileTarget::from_curseforge_type)
                        .unwrap_or_default(),
                    name: project.name,
                    summary: project.summary,
                    icon_url: project.logo.map(|logo| logo.url),
                    website_url: project
                        .links
                        .map(|links| links.website_url)
                        .unwrap_or_default(),
                },
            )
        }));
    }
    Ok(summaries)
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
/// `(project_id, file_id)`, stage it in the instance cache, read the manifest,
/// ensure the game + loader are installed, and extract `overrides/` into
/// `instance_path`. The cached zip is processed from disk (not held in memory)
/// and deleted once its overrides are extracted. Mod-list handling and instance
/// metadata differ between install and upgrade, so they stay with the caller.
async fn prepare_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    instance_path: &Path,
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

    let file = resolve_project_files(&[file_id], api_key, http_client)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| Error::NotExists(format!("CurseForge file {file_id}")))?;
    let download_url = file.download_url.as_deref().ok_or_else(|| {
        Error::Invalid(format!(
            "CurseForge modpack file {} has no download URL",
            file.file_name
        ))
    })?;
    let cache_path = instance_path
        .join(file.target.directory())
        .join(&file.file_name);
    download_resource(http_client, download_url, &file.sha1, cache_path.clone()).await?;

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

/// What differs between installing a pack fetched by id and one read off disk:
/// where the instance says it came from, and what seeds its description.
struct InstallOrigin {
    source: DownloadSource,
    description: String,
}

/// Finish an install once the pack is prepared: resolve and download the mods
/// its manifest lists, record them, and write the instance.
async fn finish_install(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    instance_path: &Path,
    prepared: PreparedModpack,
    origin: InstallOrigin,
    api_key: &str,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    emit_progress(app_handle, id, instance_name, "Downloading mods", false, None);
    let file_ids = manifest_file_ids(&prepared.manifest);
    let files = resolve_project_files(&file_ids, api_key, &state.http_client).await?;
    let modlist_entries =
        super::download_project_files(files, instance_path, &state.http_client).await?;

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    upsert_modlist_entries(instance_path, modlist_entries)?;

    let meta = InstanceMeta {
        id: id.to_string(),
        name: instance_name.to_string(),
        description: origin.description,
        game_version: prepared.mc_version.clone(),
        mod_loader: prepared.mod_loader.clone(),
        mod_loader_version: prepared.loader_version,
        version_id: prepared.version_id,
        category,
        origin: origin.source,
        ..InstanceMeta::default()
    };
    write_json(instance_meta_file(instance_path), &meta)?;

    info!(
        "Installed '{}' (MC {}, {}) → {}",
        instance_name,
        prepared.mc_version,
        prepared.mod_loader,
        instance_path.display()
    );
    Ok(())
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
        (settings.curseforge_api_key.clone(), settings.instance_install_dir.clone())
    };
    let instance_path = create_instance_dir(&install_dir, instance_name)?;

    let prepared = prepare_modpack(
        app_handle,
        id,
        instance_name,
        &instance_path,
        file_id,
        &api_key,
        state,
    ).await?;

    // Seed the instance description with the pack's own blurb. It is cosmetic,
    // so a failed lookup leaves it empty instead of failing the install.
    let description = match fetch_project_summaries(&[project_id], &api_key, &state.http_client).await {
        Ok(projects) => projects
            .get(&project_id)
            .map(|project| project.summary.clone())
            .unwrap_or_default(),
        Err(e) => {
            log::warn!("no description for CurseForge project {project_id}: {e}");
            String::new()
        }
    };

    finish_install(
        app_handle,
        id,
        instance_name,
        category,
        &instance_path,
        prepared,
        InstallOrigin { source, description },
        &api_key,
        state,
    )
    .await
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
    let file_id = source
        .curseforge_ids()
        .map(|(_, file_id)| file_id)
        .ok_or_else(|| Error::Unsupported("not a CurseForge modpack source".to_string()))?;
    let api_key = state.settings.read().unwrap().curseforge_api_key.clone();
    let http_client = &state.http_client;

    // The modlist (the installed-mod source of truth) is only rewritten at
    // finalize, so this set survives a failed/retried upgrade for a correct re-diff.
    let installed = installed_curseforge_files(&instance_path);
    let old_ids: Vec<u32> = installed.keys().copied().collect();

    let prepared = prepare_modpack(
        app_handle,
        id,
        instance_name,
        &instance_path,
        file_id,
        &api_key,
        state,
    )
    .await?;

    // Diff the mod list by file id: download additions, remove drops, skip
    // unchanged files already on disk.
    emit_progress(app_handle, id, instance_name, "Updating mods", false, None);
    let new_ids = manifest_file_ids(&prepared.manifest);
    let new_set: HashSet<u32> = new_ids.iter().copied().collect();
    // A file the pack still ships but whose download previously failed has no jar
    // on disk, so an upgrade is the moment to retry it alongside the genuine additions.
    let to_add: HashSet<u32> = new_ids
        .iter()
        .copied()
        .filter(|fid| matches!(installed.get(fid).map(|f| f.state), None | Some(ModState::DownloadFailed)))
        .collect();

    // Names the pack drops, taken from the modlist rather than resolved against
    // the API — an id CurseForge no longer serves would otherwise be skipped,
    // leaving its jar loading against the upgraded pack. Deleting is deferred
    // until the downloads land: a failure in between would strip jars the
    // modlist still calls installed, and a later upgrade to a different target
    // would never fetch them again.
    let old_file_names: Vec<String> = installed
        .iter()
        .filter(|(file_id, _)| !new_set.contains(*file_id))
        .map(|(_, installed_file)| installed_file.file_name.clone())
        .collect();

    // One resolve for the whole new file set — `to_add` is a subset of it, so
    // splitting the result here saves a second round of /v1/mods/files (plus its
    // per-chunk /v1/mods fan-out) against the CurseForge rate limit.
    let new_files = resolve_project_files(&new_ids, &api_key, http_client).await?;
    let new_files_names: Vec<String> = new_files.iter().map(|f| f.file_name.clone()).collect();

    // Baseline the new modlist from the full file set, but keep only mods. A file
    // the instance already had keeps its recorded state (a mod the user disabled
    // stays disabled); the downloaded entries overwrite their baseline below.
    let mut new_modlist_entries: Vec<ModListEntry> = new_files
        .iter()
        .filter(|file| file.target.tracks_modlist())
        .map(|file| {
            let previous = file
                .source
                .curseforge_ids()
                .and_then(|(_, fid)| installed.get(&fid).map(|f| f.state));
            file.to_modlist_entry(previous.unwrap_or(ModState::Enabled))
        })
        .collect();

    // `to_add` mixes new mods and (all) resource packs — resource packs are never
    // in `installed`, since the modlist tracks only mods. `download_project_files`
    // routes each to the right dir and returns entries for mods alone.
    let added_files: Vec<ProjectFileInfo> = new_files
        .into_iter()
        .filter(|file| {
            file.source
                .curseforge_ids()
                .is_some_and(|(_, fid)| to_add.contains(&fid))
        })
        .collect();
    let downloaded_modlist_entries =
        super::download_project_files(added_files, &instance_path, http_client).await?;

    // Everything the new pack ships is now on disk, so the dropped jars can go.
    // A name the new pack also uses belongs to the file just written, not to the
    // one being dropped, so it is left alone.
    let kept_names: HashSet<&str> = new_files_names.iter().map(String::as_str).collect();
    let mods_dir = instance_path.join("mods");
    for file_name in &old_file_names {
        if kept_names.contains(file_name.as_str()) {
            continue;
        }
        // A disabled mod lives under `<name>.disabled`, so drop both spellings.
        let disabled = mods_dir.join(format!("{file_name}.disabled"));
        for path in [mods_dir.join(file_name), disabled] {
            if path.exists() {
                if let Err(e) = std::fs::remove_file(&path) {
                    log::warn!("failed to remove old mod {}: {e}", path.display());
                }
            }
        }
    }

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

/// The page a user can download `file_id` from by hand, resolved from the
/// project's own `websiteUrl` rather than assembled from an id — CurseForge
/// addresses projects by slug, which nothing in the modlist records.
pub async fn project_file_page_url(
    project_id: u32,
    file_id: u32,
    api_key: &str,
    client: &reqwest::Client,
) -> Result<String, Error> {
    let website = fetch_project_summaries(&[project_id], api_key, client)
        .await?
        .remove(&project_id)
        .map(|project| project.website_url)
        .unwrap_or_default();
    if website.is_empty() {
        return Err(Error::NotExists(format!(
            "a page for CurseForge project {project_id}"
        )));
    }
    Ok(format!("{website}/files/{file_id}"))
}

/// Read a modpack zip's manifest without installing anything, so the user can
/// confirm the file they picked and a zip that is not a CurseForge modpack is
/// rejected before an instance directory exists.
pub fn read_local_modpack(zip_path: &Path) -> Result<LocalModpackInfo, Error> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| Error::Invalid(format!("cannot open {}: {e}", zip_path.display())))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| Error::Invalid(format!("{} is not a zip file", zip_path.display())))?;
    let manifest = read_manifest(&mut archive)?;
    let (mod_loader, mod_loader_version) = resolve_loader(&manifest)?;
    Ok(LocalModpackInfo {
        format: ModpackFormat::CurseForge,
        name: manifest.name,
        version: manifest.version,
        author: manifest.author,
        game_version: manifest.minecraft.version,
        mod_loader,
        mod_loader_version,
        file_count: manifest.files.len(),
    })
}

/// Install a modpack from a zip already on disk. The zip is read where it lies
/// and left alone — it is the user's file, not a staged download — so this skips
/// the fetch that `install_modpack` performs and shares everything after it.
///
/// The instance records a `Manual` origin: a manifest names the mods it wants
/// but not the pack it came from, so there is no project to offer an upgrade
/// against.
pub async fn install_modpack_from_file(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    zip_path: &Path,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let (api_key, install_dir) = {
        let settings = state.settings.read().unwrap();
        (settings.curseforge_api_key.clone(), settings.instance_install_dir.clone())
    };
    let instance_path = create_instance_dir(&install_dir, instance_name)?;

    let prepared = prepare_from_cached_zip(
        app_handle,
        id,
        instance_name,
        &instance_path,
        zip_path,
        state,
    )
    .await?;

    // A manifest carries no project id for the pack itself, so there is nothing
    // to offer an upgrade against; the pack's own name is the best description
    // available without an API lookup that could not identify it anyway.
    let description = prepared.manifest.name.clone();
    finish_install(
        app_handle,
        id,
        instance_name,
        category,
        &instance_path,
        prepared,
        InstallOrigin { source: DownloadSource::Manual, description },
        &api_key,
        state,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::read_local_modpack;
    use std::io::Write;
    use yaminabe_launcher_shared::datamodels::ModLoader;

    /// Write a zip holding exactly `entries`, and return where it landed.
    fn zip_with(name: &str, entries: &[(&str, &str)]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("yaminabe-test-{name}.zip"));
        let file = std::fs::File::create(&path).expect("create test zip");
        let mut writer = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (entry, body) in entries {
            writer.start_file(*entry, options).expect("start entry");
            writer.write_all(body.as_bytes()).expect("write entry");
        }
        writer.finish().expect("finish zip");
        path
    }

    /// Shaped after a real CurseForge export, field for field.
    const MANIFEST: &str = r#"{
        "minecraft": {
            "version": "1.21.1",
            "modLoaders": [{ "id": "neoforge-21.1.248", "primary": true }]
        },
        "manifestType": "minecraftModpack",
        "manifestVersion": 1,
        "name": "FTB StoneBlock 4",
        "version": "1.20.0",
        "author": "FTB Team",
        "files": [
            { "projectID": 1, "fileID": 2, "required": true },
            { "projectID": 3, "fileID": 4, "required": false }
        ],
        "overrides": "overrides"
    }"#;

    #[test]
    fn reads_what_a_curseforge_manifest_declares() {
        let path = zip_with("manifest", &[("manifest.json", MANIFEST)]);

        let info = read_local_modpack(&path).expect("manifest should parse");

        assert_eq!(info.name, "FTB StoneBlock 4");
        assert_eq!(info.version, "1.20.0");
        assert_eq!(info.author, "FTB Team");
        assert_eq!(info.game_version, "1.21.1");
        assert_eq!(info.mod_loader, ModLoader::NeoForge);
        assert_eq!(info.mod_loader_version.as_deref(), Some("21.1.248"));
        // Optional files count too: the picker reports the manifest, not the
        // subset that will be downloaded.
        assert_eq!(info.file_count, 2);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rejects_a_zip_that_is_not_a_modpack() {
        let path = zip_with("no-manifest", &[("readme.txt", "just a zip")]);

        let error = read_local_modpack(&path).expect_err("a zip with no manifest is not a modpack");

        assert!(
            error.to_string().contains("manifest.json"),
            "the error should name what is missing, got: {error}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rejects_a_file_that_is_not_a_zip() {
        let path = std::env::temp_dir().join("yaminabe-test-not-a-zip.zip");
        std::fs::write(&path, b"not a zip at all").expect("write test file");

        let error = read_local_modpack(&path).expect_err("plain bytes are not a zip");

        assert!(error.to_string().contains("not a zip"), "got: {error}");
        std::fs::remove_file(&path).ok();
    }
}
