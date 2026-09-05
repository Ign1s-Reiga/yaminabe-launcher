use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::commands::instance::{
    create_instance_dir, discard_unfinished_instance_dir, instance_meta_file, is_bare_file_name,
    modlist_file, replace_modlist_entries, upsert_modlist_entries,
};
use crate::emit_progress;
use crate::http_utils::{download_resource, fetch_json};
use crate::install_task::ensure_game_and_loader;
use crate::json::{read_json, read_json_or_default, write_json};
use crate::AppState;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use tauri::State;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, InstanceMeta, LocalModpackInfo, ModListEntry, ModLoader, ModProjectInfo,
    ModProjectSearchResults, ModState, ModpackFormat, ProjectFileInfo,
    ProjectFileTarget, ProjectId, SearchOptions,
};
use yaminabe_launcher_shared::error::Error;

#[derive(Deserialize)]
struct Version {
    id: String,
    project_id: String,
    version_type: String,
    /// The version's display name, e.g. "Sodium 0.5.8". Optional in practice,
    /// so `version_number` stands in when it is missing.
    #[serde(default)]
    name: String,
    #[serde(default)]
    version_number: String,
    /// ISO-8601 UTC, so string order is chronological order.
    #[serde(default)]
    date_published: String,
    #[serde(default)]
    files: Vec<VersionFile>,
    #[serde(default)]
    dependencies: Vec<Dependency>,
}

impl Version {
    fn to_project_file_info(&self, target: ProjectFileTarget) -> Result<ProjectFileInfo, Error> {
        let version_file = selected_file(self).ok_or_else(|| {
            Error::NotExists(format!("Modrinth version-file {} does not exists.", self.id))
        })?;
        let display_name = [&self.name, &self.version_number, &version_file.filename]
            .into_iter()
            .find(|candidate| !candidate.is_empty())
            .cloned()
            .unwrap_or_default();
        Ok(ProjectFileInfo {
            source: DownloadSource::Modrinth {
                project_id: self.project_id.clone(),
                version_id: self.id.clone(),
            },
            target,
            release_type: self.version_type.clone().into(),
            file_name: version_file.filename.clone(),
            download_url: Some(version_file.url.clone()),
            display_name,
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

/// A version this one pulls in. Modrinth names a dependency by version, by
/// project, or by neither — an embedded jar it only knows a file name for
/// reports both ids as null — so neither id can be required.
#[derive(Deserialize)]
struct Dependency {
    version_id: Option<String>,
    project_id: Option<String>,
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
    /// Modrinth's own opaque project id, e.g. `AANobbMI`.
    project_id: String,
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
/// shape are normalized to the shared `ModProjectInfo`.
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
                id: ProjectId::Modrinth(hit.project_id),
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

/// How many versions one page of a project's file list holds. Modrinth returns
/// every matching version in one response, so the page is cut here to match what
/// the frontend expects from the CurseForge path.
const PAGE_SIZE: usize = 50;

/// List a project's files via `GET /v2/project/{id}/version`, newest first.
///
/// Modrinth filters by loader and game version but does not paginate, so the
/// requested page is sliced out of the full response. A version whose files
/// cannot be read is skipped rather than failing the whole page.
pub async fn list_project_files(
    project_id: &str,
    target: ProjectFileTarget,
    game_version: Option<&str>,
    mod_loader: Option<&ModLoader>,
    index: u32,
    client: &reqwest::Client,
) -> Result<Vec<ProjectFileInfo>, Error> {
    // Both filters are JSON arrays in the query string, so they outlive the
    // borrow the query builder takes.
    let loaders = mod_loader
        .and_then(|loader| loader.modrinth_name())
        .map(|name| format!("[\"{name}\"]"));
    let game_versions = game_version.map(|version| format!("[\"{version}\"]"));

    let mut query: Vec<(&str, &str)> = Vec::new();
    if let Some(loaders) = loaders.as_deref() {
        query.push(("loaders", loaders));
    }
    if let Some(game_versions) = game_versions.as_deref() {
        query.push(("game_versions", game_versions));
    }

    let url = format!("https://api.modrinth.com/v2/project/{project_id}/version");
    let mut versions = fetch_json(client, &url)
        .query(&query)
        .send::<Vec<Version>>()
        .await?;

    // The endpoint documents no order, and pages are sliced out of the whole
    // response — so newest-first is imposed here rather than assumed, or one
    // page could repeat or skip a version another page already showed.
    versions.sort_by(|a, b| b.date_published.cmp(&a.date_published));

    let mut files = page_of(&versions, target, index);

    // Every version on this page belongs to the one project asked for, so a
    // single lookup names them all. A file added from here is recorded with
    // that name and icon, the way the CurseForge path already does it.
    if !files.is_empty() {
        if let Some(project) = fetch_project(project_id, client).await {
            for file in &mut files {
                file.project_name = project.title.clone();
                file.icon_url = project.icon_url.clone();
            }
        }
    }
    Ok(files)
}

/// One page of `versions` as project files, in the order given.
///
/// Unreadable versions are dropped before the page is cut, not after. The
/// caller reads a short page as "no more versions", so filtering afterwards
/// would end the list early at the first withdrawn file and hide every version
/// below it.
fn page_of(versions: &[Version], target: ProjectFileTarget, index: u32) -> Vec<ProjectFileInfo> {
    versions
        .iter()
        .filter_map(|version| match version.to_project_file_info(target) {
            Ok(file) => Some(file),
            Err(e) => {
                warn!("skipping Modrinth version {}: {e}", version.id);
                None
            }
        })
        .skip(index as usize)
        .take(PAGE_SIZE)
        .collect()
}

/// One project's name and icon. `None` on any failure: this only decorates a
/// list that is already correct without it.
async fn fetch_project(project_id: &str, client: &reqwest::Client) -> Option<ProjectSummary> {
    let url = format!("https://api.modrinth.com/v2/project/{project_id}");
    match fetch_json(client, &url).send::<ProjectSummary>().await {
        Ok(project) => Some(project),
        Err(e) => {
            warn!("no name for Modrinth project {project_id}: {e}");
            None
        }
    }
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
    /// What the index says the file weighs. Recorded even for a file that could
    /// not be fetched, so the Mods tab can show what is missing rather than 0 B.
    #[serde(default)]
    file_size: u64,
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
    // Normalised the same way safe_destination does, so `./mods/x.jar` is not
    // read as a different directory from `mods/x.jar` and left untracked.
    let first = relative
        .split(['/', '\\'])
        .find(|part| !part.is_empty() && *part != "." && *part != ".." && !part.contains(':'))?;
    match first {
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

/// The name a path ends in, for the modlist.
fn file_name_of(dest: &Path) -> String {
    dest.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A modlist row for a file that was never fetched, so the Mods tab can offer
/// to link it by hand.
fn unfetched_entry(file: &MrpackFile, target: ProjectFileTarget) -> ModListEntry {
    ModListEntry {
        file_name: file_name_of(Path::new(&file.path)),
        project_name: String::new(),
        icon_url: None,
        sha1: file.hashes.sha1.clone(),
        source: DownloadSource::Manual,
        target,
        size: file.file_size,
        state: ModState::DownloadFailed,
    }
}

/// Try each URL the index lists in turn. They are mirrors of one file, so the
/// first that works wins and only the last error is worth reporting.
async fn download_from_mirrors(
    file: &MrpackFile,
    dest: &Path,
    client: &reqwest::Client,
) -> Result<(), Error> {
    let mut last = Error::Invalid(format!("no download URL for {}", file.path));
    for url in &file.downloads {
        match download_resource(client, url, &file.hashes.sha1, dest.to_path_buf()).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                warn!("mirror {url} failed for {}: {e}", file.path);
                last = e;
            }
        }
    }
    Err(last)
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

/// What Modrinth knows about a file the pack shipped, found by its hash.
struct ResolvedFile {
    project_id: String,
    version_id: String,
    project_name: String,
    icon_url: Option<String>,
}

#[derive(Serialize)]
struct HashQuery<'a> {
    hashes: &'a [String],
    algorithm: &'a str,
}

#[derive(Deserialize)]
struct ProjectSummary {
    id: String,
    title: String,
    #[serde(default)]
    icon_url: Option<String>,
}

/// How many hashes or ids one bulk request carries.
const BULK_CHUNK: usize = 100;

/// Identify a pack's files on Modrinth by SHA-1.
///
/// A `.mrpack` index lists URLs and hashes but names no project, so an installed
/// pack would otherwise show jar file names with no icons. Two bulk lookups close
/// that gap: hashes to versions, then those versions' projects to names and icons.
///
/// This is cosmetic. A failed lookup logs and yields nothing for the files it
/// covered, leaving them named by their file — never failing an install whose
/// files are already on disk.
async fn resolve_by_hash(
    hashes: Vec<String>,
    client: &reqwest::Client,
) -> HashMap<String, ResolvedFile> {
    let mut versions: HashMap<String, Version> = HashMap::new();
    for chunk in hashes.chunks(BULK_CHUNK) {
        let query = HashQuery { hashes: chunk, algorithm: "sha1" };
        match fetch_json(client, "https://api.modrinth.com/v2/version_files")
            .method(reqwest::Method::POST)
            .payload(&query)
            .send::<HashMap<String, Version>>()
            .await
        {
            Ok(found) => versions.extend(found),
            Err(e) => warn!("Modrinth hash lookup failed for {} files: {e}", chunk.len()),
        }
    }
    if versions.is_empty() {
        return HashMap::new();
    }

    let mut ids: Vec<String> = versions
        .values()
        .map(|version| version.project_id.clone())
        .collect();
    ids.sort();
    ids.dedup();

    let mut projects: HashMap<String, ProjectSummary> = HashMap::new();
    for chunk in ids.chunks(BULK_CHUNK) {
        let Ok(encoded) = serde_json::to_string(chunk) else {
            continue;
        };
        match fetch_json(client, "https://api.modrinth.com/v2/projects")
            .query(&[("ids", encoded.as_str())])
            .send::<Vec<ProjectSummary>>()
            .await
        {
            Ok(found) => projects.extend(
                found
                    .into_iter()
                    .map(|project| (project.id.clone(), project)),
            ),
            Err(e) => warn!("Modrinth project lookup failed for {} ids: {e}", chunk.len()),
        }
    }

    versions
        .into_iter()
        .map(|(sha1, version)| {
            let project = projects.get(&version.project_id);
            let resolved = ResolvedFile {
                project_name: project.map(|p| p.title.clone()).unwrap_or_default(),
                icon_url: project.and_then(|p| p.icon_url.clone()),
                project_id: version.project_id,
                version_id: version.id,
            };
            (sha1.to_ascii_lowercase(), resolved)
        })
        .collect()
}

/// Name the pack's files after the projects they come from, so the Mods tab
/// shows "Sodium" with its icon rather than `sodium-fabric-0.5.8.jar`. Also
/// records where each file came from, which the index alone cannot say — a
/// failed entry can then link to its project page.
async fn name_entries_by_project(entries: &mut [ModListEntry], client: &reqwest::Client) {
    let hashes: Vec<String> = entries
        .iter()
        .filter(|entry| !entry.sha1.is_empty())
        .map(|entry| entry.sha1.to_ascii_lowercase())
        .collect();
    if hashes.is_empty() {
        return;
    }

    let resolved = resolve_by_hash(hashes, client).await;
    for entry in entries.iter_mut() {
        let Some(found) = resolved.get(&entry.sha1.to_ascii_lowercase()) else {
            continue;
        };
        entry.project_name = found.project_name.clone();
        entry.icon_url = found.icon_url.clone();
        entry.source = DownloadSource::Modrinth {
            project_id: found.project_id.clone(),
            version_id: found.version_id.clone(),
        };
    }
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
    // Nothing usable exists until the instance record is written, so a failure
    // partway through clears the directory rather than leaving one that the
    // library never shows and that blocks retrying the same name.
    let result = install_into(
        app_handle,
        id,
        instance_name,
        category,
        zip_path,
        DownloadSource::Manual,
        &instance_path,
        state,
    )
    .await;
    if result.is_err() {
        discard_unfinished_instance_dir(&instance_path);
    }
    result
}

/// Install a `.mrpack` already on disk into `instance_path`. `origin` is what
/// the instance records as its provenance: `Manual` for a zip the user picked,
/// or the Modrinth version it was fetched from.
#[allow(clippy::too_many_arguments)]
/// A staged `.mrpack` with its game and loader already installed. Shared by
/// install and upgrade, which differ only in what they do with the files.
struct PreparedPack {
    archive: zip::ZipArchive<std::fs::File>,
    index: MrpackIndex,
    mc_version: String,
    mod_loader: ModLoader,
    loader_version: Option<String>,
    version_id: String,
}

async fn prepare_pack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    zip_path: &Path,
    state: &State<'_, AppState>,
) -> Result<PreparedPack, Error> {
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

    Ok(PreparedPack {
        archive,
        index,
        mc_version,
        mod_loader,
        loader_version,
        version_id,
    })
}

/// What the instance already had, by file name, so an upgrade can tell an
/// unchanged file from a new one.
fn installed_entries(instance_path: &Path) -> HashMap<String, ModListEntry> {
    let modlist: Vec<ModListEntry> =
        read_json_or_default(modlist_file(instance_path)).unwrap_or_default();
    modlist
        .into_iter()
        .map(|entry| (entry.file_name.clone(), entry))
        .collect()
}

/// Whether `previous` is the same file the index now lists. Compared by hash,
/// not by name: a pack can ship a different jar under a name it used before.
fn is_unchanged(previous: &ModListEntry, file: &MrpackFile) -> bool {
    !previous.sha1.is_empty() && previous.sha1.eq_ignore_ascii_case(&file.hashes.sha1)
}

/// Where a disabled mod actually sits on disk.
fn disabled_path(dest: &Path) -> PathBuf {
    dest.with_file_name(format!("{}.disabled", file_name_of(dest)))
}

/// Put the files the index lists into the instance, and report what a mod list
/// should say about them.
///
/// `previous` is what the instance already had, empty for a fresh install. A
/// file whose hash has not changed keeps the state it was in — so a mod the
/// user disabled stays disabled through an upgrade, and is left on disk under
/// its `.disabled` name rather than being downloaded back into place.
///
/// Alongside the mod-list entries it returns the instance-relative path of every
/// file the pack installed, which is what tells a later upgrade that a version
/// has dropped one. The mod list cannot answer that: it holds mods, and it keys
/// on bare file names, which two directories can share.
async fn sync_pack_files(
    index: &MrpackIndex,
    instance_path: &Path,
    previous: &HashMap<String, ModListEntry>,
    client: &reqwest::Client,
) -> Result<SyncedPack, Error> {
    let mut entries: Vec<ModListEntry> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    for file in index.files.iter().filter(|file| file.wanted_by_client()) {
        let target = target_for(&file.path);
        // A file that cannot even be attempted is recorded the same way a failed
        // download is, so the Mods tab offers to link it rather than the install
        // reporting success with the file quietly absent.
        let mut record_failure = |reason: String| {
            warn!("{}: {reason}", file.path);
            if let Some(target) = target {
                entries.push(unfetched_entry(file, target));
            }
        };

        let Some(dest) = safe_destination(instance_path, &file.path) else {
            record_failure("unusable path".to_string());
            continue;
        };
        if file.downloads.is_empty() {
            record_failure("no download URL".to_string());
            continue;
        }

        let disabled = disabled_path(&dest);
        let installed = previous.get(&file_name_of(&dest));
        let was_disabled = installed.is_some_and(|entry| entry.state == ModState::Disabled);

        // A disabled mod lives under another name, so downloading `dest` would
        // reinstate the jar the user turned off and leave both copies on disk.
        // Only when the file has not changed: a new version of it still has to
        // be fetched, and is disabled again below.
        if was_disabled && installed.is_some_and(|entry| is_unchanged(entry, file)) && disabled.exists()
        {
            entries.push(installed.expect("checked above").clone());
            continue;
        }

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let (mod_state, written) = match download_from_mirrors(file, &dest, client).await {
            // The user turned this mod off, so the version replacing it is off
            // too — the choice was about the mod, not about that build of it.
            Ok(()) if was_disabled => {
                std::fs::remove_file(&disabled).ok();
                match std::fs::rename(&dest, &disabled) {
                    Ok(()) => (ModState::Disabled, disabled.clone()),
                    Err(e) => {
                        warn!("cannot disable {}: {e}; leaving it enabled", file.path);
                        (ModState::Enabled, dest.clone())
                    }
                }
            }
            Ok(()) => (ModState::Enabled, dest.clone()),
            Err(e) => {
                warn!("download failed for {}: {e}; marking for manual install", file.path);
                // Whatever sits at this name is the old pack's copy, and it is
                // not what this one ships — `download_resource` leaves a file
                // whose hash did not match in place. Left there it would load
                // against the upgraded pack while the mod list reports the file
                // as missing, so the two would disagree about what is installed.
                for stale in [&dest, &disabled] {
                    if stale.exists() {
                        if let Err(e) = std::fs::remove_file(stale) {
                            warn!("cannot remove stale {}: {e}", stale.display());
                        }
                    }
                }
                (ModState::DownloadFailed, dest.clone())
            }
        };
        if mod_state != ModState::DownloadFailed {
            if let Some(relative) = relative_path(instance_path, &dest) {
                paths.push(relative);
            }
        }

        let Some(target) = target else { continue };
        // Only mods are tracked once installed; anything else is tracked solely
        // to drive the link prompt when it could not be fetched.
        if target.tracks_modlist() || mod_state == ModState::DownloadFailed {
            // The index states a size; fall back to the file itself when it does
            // not, so a mod never lists as 0 B once it is on disk.
            let size = match (file.file_size, mod_state) {
                (0, ModState::DownloadFailed) => 0,
                (0, _) => std::fs::metadata(&written).map(|m| m.len()).unwrap_or(0),
                (size, _) => size,
            };
            entries.push(ModListEntry {
                file_name: file_name_of(&dest),
                project_name: String::new(),
                icon_url: None,
                sha1: file.hashes.sha1.clone(),
                source: DownloadSource::Manual,
                target,
                size,
                state: mod_state,
            });
        }
    }
    Ok(SyncedPack { entries, paths })
}

/// What one pass over a pack's index put on disk.
struct SyncedPack {
    /// Mod-list rows: the mods, plus any file awaiting a hand-supplied copy.
    entries: Vec<ModListEntry>,
    /// Instance-relative path of every file installed, for the next upgrade.
    paths: Vec<String>,
}

/// Where the paths a pack installed are recorded.
fn pack_files_file(instance_path: &Path) -> PathBuf {
    instance_path.join(".launcher").join("pack_files.json")
}

/// A file's location as the pack list records it: instance-relative, with
/// forward slashes, so a record written on one platform reads on another.
fn relative_path(instance_path: &Path, dest: &Path) -> Option<String> {
    let relative = dest.strip_prefix(instance_path).ok()?;
    let joined = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    (!joined.is_empty()).then_some(joined)
}

/// Delete a file a newer version of the pack no longer ships, under whichever
/// name it has — a disabled mod is stored with a `.disabled` suffix.
///
/// The path came from our own record, but it is resolved through the same guard
/// as one from an index: a record that has been edited by hand cannot direct a
/// delete outside the instance.
fn remove_pack_file(instance_path: &Path, relative: &str) {
    let Some(dest) = safe_destination(instance_path, relative) else {
        warn!("refusing to remove '{relative}': not inside the instance");
        return;
    };
    for path in [disabled_path(&dest), dest] {
        if path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                warn!("failed to remove dropped file {}: {e}", path.display());
            }
        }
    }
}

/// The pack's own blurb where it ships one; its name is all a pack without a
/// summary offers.
fn pack_description(index: &MrpackIndex) -> String {
    if index.summary.is_empty() {
        index.name.clone()
    } else {
        index.summary.clone()
    }
}

async fn install_into(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    zip_path: &Path,
    origin: DownloadSource,
    instance_path: &Path,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let mut prepared = prepare_pack(app_handle, id, instance_name, zip_path, state).await?;

    emit_progress(app_handle, id, instance_name, "Downloading mods", false, None);
    let synced = sync_pack_files(
        &prepared.index,
        instance_path,
        &HashMap::new(),
        &state.http_client,
    )
    .await?;
    let mut modlist_entries = synced.entries;

    // After the downloads, not before: the format has overrides win over what a
    // pack lists, and download_resource replaces a file whose hash does not
    // match — so extracting first would let the stock jar overwrite a patched
    // one the pack deliberately ships.
    emit_progress(app_handle, id, instance_name, "Extracting files", false, None);
    extract_overrides(&mut prepared.archive, instance_path)?;

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    name_entries_by_project(&mut modlist_entries, &state.http_client).await;
    upsert_modlist_entries(instance_path, modlist_entries)?;
    write_json(pack_files_file(instance_path), &synced.paths)?;

    let meta = InstanceMeta {
        id: id.to_string(),
        name: instance_name.to_string(),
        description: pack_description(&prepared.index),
        game_version: prepared.mc_version.clone(),
        mod_loader: prepared.mod_loader.clone(),
        mod_loader_version: prepared.loader_version,
        version_id: prepared.version_id,
        category,
        origin,
        ..InstanceMeta::default()
    };
    write_json(instance_meta_file(instance_path), &meta)?;

    info!(
        "Installed '{}' from {} (MC {}, {})",
        instance_name,
        zip_path.display(),
        prepared.mc_version,
        prepared.mod_loader
    );
    Ok(())
}

/// Install a modpack named by a Modrinth version id.
///
/// The version's `.mrpack` is staged in the instance's cache directory and then
/// installed by the same code that handles a pack the user picked off disk — the
/// file is identical either way. What differs is the origin recorded: an install
/// from here knows the project and version it came from.
pub async fn install_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    source: DownloadSource,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let DownloadSource::Modrinth { version_id, .. } = &source else {
        return Err(Error::Unsupported(
            "not a Modrinth modpack source".to_string(),
        ));
    };
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let instance_path = create_instance_dir(&install_dir, instance_name)?;
    // Nothing usable exists until the instance record is written, so clear the
    // directory on failure rather than leaving one the library never shows and
    // that blocks retrying the same name.
    let result = install_by_version(
        app_handle,
        id,
        instance_name,
        category,
        &source,
        version_id,
        &instance_path,
        state,
    )
    .await;
    if result.is_err() {
        discard_unfinished_instance_dir(&instance_path);
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn install_by_version(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    source: &DownloadSource,
    version_id: &str,
    instance_path: &Path,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    emit_progress(app_handle, id, instance_name, "Downloading modpack", false, None);
    let cache_path = stage_version(version_id, instance_path, &state.http_client).await?;

    let installed = install_into(
        app_handle,
        id,
        instance_name,
        category,
        &cache_path,
        source.clone(),
        instance_path,
        state,
    )
    .await;

    // Always drop the staged zip: its overrides are in the instance now and its
    // files were fetched from the URLs the index carries.
    std::fs::remove_file(&cache_path).ok();
    installed
}

/// Upgrade an instance to another version of the Modrinth pack it came from.
///
/// The new `.mrpack` is staged and installed over the instance: files it still
/// ships are downloaded (unchanged ones are skipped by their hash), files it no
/// longer ships are deleted, and the user's saves, configs and screenshots are
/// left untouched.
pub async fn upgrade_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    instance_path: PathBuf,
    source: DownloadSource,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let DownloadSource::Modrinth { version_id, .. } = &source else {
        return Err(Error::Unsupported(
            "not a Modrinth modpack source".to_string(),
        ));
    };

    emit_progress(app_handle, id, instance_name, "Downloading modpack", false, None);
    let cache_path = stage_version(version_id, &instance_path, &state.http_client).await?;
    let upgraded = upgrade_into(
        app_handle,
        id,
        instance_name,
        &instance_path,
        &cache_path,
        &source,
        state,
    )
    .await;
    std::fs::remove_file(&cache_path).ok();
    upgraded
}

#[allow(clippy::too_many_arguments)]
async fn upgrade_into(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    instance_path: &Path,
    zip_path: &Path,
    source: &DownloadSource,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let mut prepared = prepare_pack(app_handle, id, instance_name, zip_path, state).await?;

    // Read before anything is written: this is the record of what the instance
    // had, and it is only replaced once the new files are on disk.
    let previous = installed_entries(instance_path);

    // What the previous version put on disk. Absent for an instance installed
    // before this was recorded, in which case nothing is known to drop and the
    // old files are left alone rather than guessed at.
    let installed_before: Vec<String> =
        read_json_or_default(pack_files_file(instance_path)).unwrap_or_default();

    emit_progress(app_handle, id, instance_name, "Updating mods", false, None);
    let synced = sync_pack_files(
        &prepared.index,
        instance_path,
        &previous,
        &state.http_client,
    )
    .await?;
    let mut modlist_entries = synced.entries;

    // Only now that the new files are down: a failure before this point leaves
    // the old instance intact rather than stripped of mods it still lists.
    let shipped: HashSet<&str> = synced.paths.iter().map(String::as_str).collect();
    for path in &installed_before {
        if shipped.contains(path.as_str()) {
            continue;
        }
        remove_pack_file(instance_path, path);
    }

    emit_progress(app_handle, id, instance_name, "Extracting files", false, None);
    extract_overrides(&mut prepared.archive, instance_path)?;

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    name_entries_by_project(&mut modlist_entries, &state.http_client).await;
    // The new pack defines the mod set outright, so the list is replaced rather
    // than merged — a merge would keep rows for jars just deleted.
    replace_modlist_entries(instance_path, modlist_entries)?;
    write_json(pack_files_file(instance_path), &synced.paths)?;

    let mut meta: InstanceMeta = read_json(instance_meta_file(instance_path))?;
    meta.description = pack_description(&prepared.index);
    meta.game_version = prepared.mc_version.clone();
    meta.mod_loader = prepared.mod_loader.clone();
    meta.mod_loader_version = prepared.loader_version;
    meta.version_id = prepared.version_id;
    meta.origin = source.clone();
    write_json(instance_meta_file(instance_path), &meta)?;

    info!(
        "Upgraded '{}' (MC {}, {})",
        instance_name, prepared.mc_version, prepared.mod_loader
    );
    Ok(())
}

/// Download a version's `.mrpack` into the instance's cache directory.
async fn stage_version(
    version_id: &str,
    instance_path: &Path,
    client: &reqwest::Client,
) -> Result<PathBuf, Error> {
    let url = format!("https://api.modrinth.com/v2/version/{version_id}");
    let version = fetch_json(client, &url).send::<Version>().await?;
    let file = selected_file(&version).ok_or_else(|| {
        Error::NotExists(format!("Modrinth version {version_id} lists no file"))
    })?;

    // The name comes from the API, not from us; refuse one that would stage the
    // zip outside the instance's cache directory.
    if !is_bare_file_name(&file.filename) {
        return Err(Error::Invalid(format!(
            "Modrinth version {version_id} names its file '{}'",
            file.filename
        )));
    }
    let cache_path = instance_path
        .join(ProjectFileTarget::Modpack.directory())
        .join(&file.filename);
    download_resource(client, &file.url, &file.hashes.sha1, cache_path.clone()).await?;
    Ok(cache_path)
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

#[cfg(test)]
mod upgrade_tests {
    use super::{disabled_path, is_unchanged, remove_pack_file, MrpackFile, MrpackHashes};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use yaminabe_launcher_shared::datamodels::{
        DownloadSource, ModListEntry, ModState, ProjectFileTarget,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yaminabe-upgrade-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("mods")).expect("create mods dir");
        dir
    }

    fn entry(file_name: &str, sha1: &str, state: ModState) -> ModListEntry {
        ModListEntry {
            file_name: file_name.to_string(),
            project_name: String::new(),
            icon_url: None,
            sha1: sha1.to_string(),
            source: DownloadSource::Manual,
            target: ProjectFileTarget::Mod,
            size: 0,
            state,
        }
    }

    fn index_with(file: MrpackFile) -> super::MrpackIndex {
        super::MrpackIndex {
            name: String::new(),
            version_id: String::new(),
            summary: String::new(),
            files: vec![file],
            dependencies: HashMap::new(),
        }
    }

    fn indexed(sha1: &str) -> MrpackFile {
        MrpackFile {
            path: "mods/a.jar".to_string(),
            hashes: MrpackHashes { sha1: sha1.to_string() },
            env: None,
            downloads: vec![],
            file_size: 0,
        }
    }

    #[test]
    fn a_file_is_unchanged_only_when_its_hash_matches() {
        let installed = entry("a.jar", "ABC123", ModState::Enabled);
        assert!(is_unchanged(&installed, &indexed("abc123")));
        assert!(!is_unchanged(&installed, &indexed("def456")));

        // Nothing to compare against is not a match: re-fetching is the safe
        // reading, since the alternative is keeping a file that may differ.
        let unhashed = entry("a.jar", "", ModState::Enabled);
        assert!(!is_unchanged(&unhashed, &indexed("")));
    }

    #[test]
    fn a_disabled_mod_is_named_for_where_it_actually_sits() {
        assert_eq!(
            disabled_path(Path::new("/i/mods/a.jar")),
            PathBuf::from("/i/mods/a.jar.disabled")
        );
    }

    /// A dropped mod the user had disabled is on disk under its `.disabled`
    /// name, so deleting only the plain name would leave it loading against the
    /// upgraded pack.
    #[test]
    fn dropping_a_disabled_mod_removes_the_file_it_really_has() {
        let dir = temp_dir("disabled");
        let disabled = dir.join("mods").join("old.jar.disabled");
        std::fs::write(&disabled, b"jar").expect("write disabled jar");

        remove_pack_file(&dir, "mods/old.jar");
        assert!(!disabled.exists(), "the .disabled file should be gone");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A mirror failure leaves the previous jar at that name on disk.
    /// `download_resource` only replaces a file once it has verified bytes, so
    /// the upgrade has to clear it: the pack no longer ships that build, and
    /// the mod list already calls the file missing.
    #[tokio::test]
    async fn a_failed_replacement_does_not_leave_the_old_jar_behind() {
        let dir = temp_dir("failed-replacement");
        let jar = dir.join("mods").join("a.jar");
        std::fs::write(&jar, b"the previous version").expect("write old jar");

        // No mirrors reachable: the index points at a host that cannot serve it.
        let index = index_with(MrpackFile {
            path: "mods/a.jar".to_string(),
            hashes: MrpackHashes { sha1: "0000000000000000000000000000000000000000".to_string() },
            env: None,
            downloads: vec!["http://127.0.0.1:1/nope.jar".to_string()],
            file_size: 0,
        });
        let previous = HashMap::from([(
            "a.jar".to_string(),
            entry("a.jar", "aaaa", ModState::Enabled),
        )]);

        let synced = super::sync_pack_files(&index, &dir, &previous, &reqwest::Client::new())
            .await
            .expect("sync");

        assert!(!jar.exists(), "the superseded jar must not survive a failed fetch");
        assert_eq!(synced.entries.len(), 1);
        assert_eq!(synced.entries[0].state, ModState::DownloadFailed);
        // A file that was not fetched is not recorded as installed, so a later
        // upgrade does not try to remove something that never landed.
        assert!(synced.paths.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Disabling a mod is a choice about the mod, not about one build of it, so
    /// a new version arrives disabled too rather than switching itself back on.
    #[tokio::test]
    async fn a_changed_mod_the_user_disabled_stays_disabled() {
        let dir = temp_dir("changed-disabled");
        let disabled = dir.join("mods").join("a.jar.disabled");
        std::fs::write(&disabled, b"the previous version").expect("write disabled jar");

        let index = index_with(MrpackFile {
            path: "mods/a.jar".to_string(),
            hashes: MrpackHashes { sha1: String::new() },
            env: None,
            downloads: vec![String::new()],
            file_size: 0,
        });
        // Recorded under a different hash, so this is a new build of it.
        let previous = HashMap::from([(
            "a.jar".to_string(),
            entry("a.jar", "0ld0ld", ModState::Disabled),
        )]);

        let synced = super::sync_pack_files(&index, &dir, &previous, &reqwest::Client::new())
            .await
            .expect("sync");

        // The fetch cannot succeed here, so what matters is that the stale
        // disabled copy is gone rather than left beside a re-enabled jar.
        assert!(!disabled.exists(), "the superseded disabled copy must not survive");
        assert!(!dir.join("mods").join("a.jar").exists(), "and it must not come back enabled");
        assert_eq!(synced.entries.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two directories can hold the same file name — a pack shipping a matching
    /// resource pack and data pack is the ordinary case. The record keeps the
    /// whole path, so dropping one does not read as dropping the other.
    #[test]
    fn a_name_two_directories_share_is_recorded_once_per_directory() {
        let dir = temp_dir("same-name");
        assert_eq!(
            super::relative_path(&dir, &dir.join("resourcepacks").join("extras.zip")),
            Some("resourcepacks/extras.zip".to_string())
        );
        assert_eq!(
            super::relative_path(&dir, &dir.join("datapacks").join("extras.zip")),
            Some("datapacks/extras.zip".to_string())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Removal follows the recorded path rather than rebuilding one from the
    /// target directory, so a file the pack nests is still found.
    #[test]
    fn a_dropped_file_is_removed_from_where_the_pack_put_it() {
        let dir = temp_dir("nested");
        let nested = dir.join("mods").join("sub");
        std::fs::create_dir_all(&nested).expect("create nested dir");
        let jar = nested.join("a.jar");
        std::fs::write(&jar, b"jar").expect("write nested jar");

        super::remove_pack_file(&dir, "mods/sub/a.jar");
        assert!(!jar.exists(), "a nested file must be removed where it actually is");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The record is ours, but it is a file on disk that could be edited. A path
    /// climbing out of the instance is refused rather than followed.
    #[test]
    fn removal_refuses_a_path_outside_the_instance() {
        let dir = temp_dir("escape");
        let outside = dir.parent().expect("temp parent").join("yaminabe-not-mine.jar");
        std::fs::write(&outside, b"someone else's file").expect("write outside file");

        super::remove_pack_file(&dir, "../yaminabe-not-mine.jar");
        assert!(outside.exists(), "a path climbing out must not be followed");
        std::fs::remove_file(&outside).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dropping_an_enabled_mod_removes_its_jar() {
        let dir = temp_dir("enabled");
        let jar = dir.join("mods").join("old.jar");
        std::fs::write(&jar, b"jar").expect("write jar");

        remove_pack_file(&dir, "mods/old.jar");
        assert!(!jar.exists(), "the jar should be gone");
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod api_tests {
    use super::Version;
    use yaminabe_launcher_shared::datamodels::{ProjectFileTarget, ProjectId};

    /// Trimmed from a real `/v2/project/{id}/version` response. The last
    /// dependency is the shape that matters: Modrinth reports an embedded jar
    /// it knows only a file name for with both ids null.
    const VERSION_JSON: &str = r#"{
        "id": "BSg2ZS8u",
        "project_id": "t1tOiUHZ",
        "name": "Create+ 6.0.0 Alpha f",
        "version_number": "6.0.0-alpha-f",
        "date_published": "2026-06-15T00:34:52.861465Z",
        "version_type": "alpha",
        "files": [{
            "hashes": { "sha1": "7849fc32ce67b03033ad6aad68665b9b62a8627e" },
            "url": "https://cdn.modrinth.com/data/t1tOiUHZ/versions/BSg2ZS8u/pack.mrpack",
            "filename": "Create+ 6.0.0 Alpha f.mrpack",
            "primary": true,
            "size": 3540285
        }],
        "dependencies": [
            { "version_id": "qYqVf5jP", "project_id": "SNVQ2c0g", "dependency_type": "embedded" },
            { "version_id": null, "project_id": null, "dependency_type": "embedded" }
        ]
    }"#;

    #[test]
    fn decodes_a_version_whose_dependency_names_no_project() {
        let version: Version = serde_json::from_str(VERSION_JSON).expect("decode version");
        assert_eq!(version.dependencies.len(), 2);
        assert!(version.dependencies[1].project_id.is_none());
    }

    #[test]
    fn a_version_carries_its_own_id_and_display_name() {
        let version: Version = serde_json::from_str(VERSION_JSON).expect("decode version");
        let file = version
            .to_project_file_info(ProjectFileTarget::Modpack)
            .expect("primary file");
        assert_eq!(file.display_name, "Create+ 6.0.0 Alpha f");
        assert_eq!(file.file_name, "Create+ 6.0.0 Alpha f.mrpack");
        assert_eq!(
            file.source.version_key().as_deref(),
            Some("BSg2ZS8u")
        );
        assert_eq!(
            ProjectId::Modrinth(version.project_id.clone()),
            ProjectId::Modrinth("t1tOiUHZ".to_string())
        );
    }

    /// A version whose file list is empty — a withdrawn file, the case
    /// `page_of` drops.
    const NO_FILES_JSON: &str = r#"{
        "id": "gone1234",
        "project_id": "t1tOiUHZ",
        "name": "Withdrawn",
        "version_number": "0.0.1",
        "date_published": "2026-06-14T00:00:00.000000Z",
        "version_type": "release",
        "files": [],
        "dependencies": []
    }"#;

    fn version_named(id: &str) -> Version {
        let json = VERSION_JSON.replace("BSg2ZS8u", id);
        serde_json::from_str(&json).expect("decode version")
    }

    /// Paging counts only the versions a page can actually show. Slicing before
    /// the filter would let a withdrawn version at the top of the list shift
    /// every later page, re-showing a version the previous page already listed.
    #[test]
    fn a_withdrawn_version_does_not_shift_the_next_page() {
        let versions = vec![
            serde_json::from_str::<Version>(NO_FILES_JSON).expect("decode"),
            version_named("aaaaaaaa"),
            version_named("bbbbbbbb"),
        ];

        let first = super::page_of(&versions, ProjectFileTarget::Modpack, 0);
        assert_eq!(
            first
                .iter()
                .filter_map(|f| f.source.version_key())
                .collect::<Vec<_>>(),
            vec!["aaaaaaaa", "bbbbbbbb"]
        );

        let second = super::page_of(&versions, ProjectFileTarget::Modpack, 1);
        assert_eq!(
            second
                .iter()
                .filter_map(|f| f.source.version_key())
                .collect::<Vec<_>>(),
            vec!["bbbbbbbb"]
        );
    }
}
