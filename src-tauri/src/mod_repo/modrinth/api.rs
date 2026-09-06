use std::collections::HashMap;
use std::path::PathBuf;

use crate::http_utils::{download_resource, fetch_json};
use log::warn;
use serde::{Deserialize, Serialize};
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, ModListEntry, ModLoader, ModProjectInfo, ModProjectSearchResults,
    ProjectFileInfo, ProjectFileTarget, ProjectId, SearchOptions,
};
use yaminabe_launcher_shared::error::Error;

#[derive(Deserialize)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub version_type: String,
    /// The version's display name, e.g. "Sodium 0.5.8". Optional in practice,
    /// so `version_number` stands in when it is missing.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version_number: String,
    /// ISO-8601 UTC, so string order is chronological order.
    #[serde(default)]
    pub date_published: String,
    #[serde(default)]
    pub files: Vec<VersionFile>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
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
pub struct VersionFile {
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub hashes: Hashes,
    pub size: u64,
    #[serde(default)]
    pub primary: bool,
}

/// A version this one pulls in. Modrinth names a dependency by version, by
/// project, or by neither — an embedded jar it only knows a file name for
/// reports both ids as null — so neither id can be required.
#[derive(Deserialize)]
pub struct Dependency {
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
pub struct Hashes {
    #[serde(default)]
    pub sha1: String,
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

pub fn selected_file(version: &Version) -> Option<&VersionFile> {
    version
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| version.files.first())
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
pub async fn name_entries_by_project(entries: &mut [ModListEntry], client: &reqwest::Client) {
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
