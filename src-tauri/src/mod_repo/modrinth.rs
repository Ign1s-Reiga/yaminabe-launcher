use std::path::PathBuf;

use crate::http_utils::{download_resource, fetch_json};
use serde::Deserialize;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, ModProjectInfo, ModProjectSearchResults, Platform, ProjectFileInfo,
    ProjectFileTarget, SearchOptions,
};
use yaminabe_launcher_shared::error::Error;

#[derive(Deserialize)]
struct Version {
    id: String,
    project_id: String,
    version_type: String,
    #[serde(default)]
    files: Vec<VersionFile>,
    #[serde(default)]
    dependencies: Vec<Dependency>,
}

impl Version {
    fn to_project_file_info(&self, project_type: &str) -> Result<ProjectFileInfo, Error> {
        let version_file = selected_file(self).ok_or_else(|| {
            Error::NotExists(format!("Modrinth version-file {} does not exists.", self.id))
        })?;
        Ok(ProjectFileInfo {
            source: DownloadSource::Modrinth {
                project_id: self.project_id.clone(),
                version_id: self.id.clone(),
            },
            target: ProjectFileTarget::from_modrinth_type(project_type),
            release_type: self.version_type.clone().into(),
            file_name: version_file.filename.clone(),
            download_url: Some(version_file.url.clone()),
            display_name: version_file.filename.clone(),
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

#[derive(Deserialize)]
struct Dependency {
    version_id: Option<String>,
    project_id: String,
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
/// shape are normalized to the shared `ModProjectInfo`. Modrinth identifies
/// projects by string id, which `ModProjectInfo` (CurseForge-shaped `u32` id)
/// can't carry, so results are display-only for now — `id` is left 0.
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
                id: 0,
                platform: Platform::Modrinth,
                file_id: None,
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

pub fn list_project_files() {
    unimplemented!()
}

fn selected_file(version: &Version) -> Option<&VersionFile> {
    version
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| version.files.first())
}
