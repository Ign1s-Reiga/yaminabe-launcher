use std::collections::HashMap;

use crate::http_utils::{fetch_json, rejected_status};
use serde::Deserialize;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, ModLoader, ModProjectInfo, ModProjectSearchResults, ProjectFileInfo,
    ProjectFileTarget, ProjectId, SearchOptions,
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
pub struct ProjectSummary {
    pub target: ProjectFileTarget,
    pub name: String,
    pub summary: String,
    pub icon_url: Option<String>,
    pub website_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FilesIndex {
    game_version: String,
}


fn to_search_results(body: CurseForgeArrayResponse<SearchModsEntry>) -> ModProjectSearchResults {
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

            ModProjectInfo {
                id: ProjectId::CurseForge(m.id),
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

    Ok(to_search_results(body))
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

pub async fn fetch_project_summaries(
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
