use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::commands::instance::{
    create_instance_dir, discard_unfinished_instance_dir, instance_meta_file, is_bare_file_name,
    upsert_modlist_entries,
};
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

    Ok(versions
        .iter()
        .skip(index as usize)
        .take(PAGE_SIZE)
        .filter_map(|version| match version.to_project_file_info(target) {
            Ok(file) => Some(file),
            Err(e) => {
                warn!("skipping Modrinth version {}: {e}", version.id);
                None
            }
        })
        .collect())
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

    emit_progress(app_handle, id, instance_name, "Downloading mods", false, None);
    let mut modlist_entries: Vec<ModListEntry> = Vec::new();
    for file in index.files.iter().filter(|file| file.wanted_by_client()) {
        let target = target_for(&file.path);
        // A file that cannot even be attempted is recorded the same way a failed
        // download is, so the Mods tab offers to link it rather than the install
        // reporting success with the file quietly absent.
        let mut record_failure = |reason: String| {
            warn!("{}: {reason}", file.path);
            if let Some(target) = target {
                modlist_entries.push(unfetched_entry(file, target));
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
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mod_state = match download_from_mirrors(file, &dest, &state.http_client).await {
            Ok(()) => ModState::Enabled,
            Err(e) => {
                warn!("download failed for {}: {e}; marking for manual install", file.path);
                ModState::DownloadFailed
            }
        };
        let Some(target) = target else { continue };
        // Only mods are tracked once installed; anything else is tracked solely
        // to drive the link prompt when it could not be fetched.
        if target.tracks_modlist() || mod_state == ModState::DownloadFailed {
            // The index states a size; fall back to the file itself when it
            // does not, so a mod never lists as 0 B once it is on disk.
            let size = match (file.file_size, mod_state) {
                (0, ModState::Enabled) => std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0),
                (size, _) => size,
            };
            modlist_entries.push(ModListEntry {
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

    // After the downloads, not before: the format has overrides win over what a
    // pack lists, and download_resource replaces a file whose hash does not
    // match — so extracting first would let the stock jar overwrite a patched
    // one the pack deliberately ships.
    emit_progress(app_handle, id, instance_name, "Extracting files", false, None);
    extract_overrides(&mut archive, instance_path)?;

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    upsert_modlist_entries(instance_path, modlist_entries)?;

    // The pack's own blurb where it ships one; its name is all a pack without a
    // summary offers.
    let description = if index.summary.is_empty() {
        index.name.clone()
    } else {
        index.summary.clone()
    };
    let meta = InstanceMeta {
        id: id.to_string(),
        name: instance_name.to_string(),
        description,
        game_version: mc_version.clone(),
        mod_loader: mod_loader.clone(),
        mod_loader_version: loader_version,
        version_id,
        category,
        origin,
        ..InstanceMeta::default()
    };
    write_json(instance_meta_file(instance_path), &meta)?;

    info!(
        "Installed '{}' from {} (MC {}, {})",
        instance_name,
        zip_path.display(),
        mc_version,
        mod_loader
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

    let url = format!("https://api.modrinth.com/v2/version/{version_id}");
    let version = fetch_json(&state.http_client, &url)
        .send::<Version>()
        .await?;
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
    download_resource(
        &state.http_client,
        &file.url,
        &file.hashes.sha1,
        cache_path.clone(),
    )
    .await?;

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
}
