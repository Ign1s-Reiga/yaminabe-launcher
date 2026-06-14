use crate::ipc;
use leptos::web_sys;
use serde::Serialize;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, GameVersion, LoaderVersion, ModListEntry, ModLoader, ModProjectSearchResults,
    ProjectFileInfo, ProjectFileTarget, SearchOptions,
};

// ── IPC ───────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct SearchProjectsArgs {
    option: SearchOptions,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GetFilesArgs {
    mod_id: u32,
    target: ProjectFileTarget,
    game_version: Option<String>,
    mod_loader: Option<ModLoader>,
    index: u32,
}

pub async fn call_search_projects(option: SearchOptions) -> Result<ModProjectSearchResults, String> {
    ipc::call("search_projects", SearchProjectsArgs { option }).await
}

pub async fn call_list_project_files(
    mod_id: u32,
    target: ProjectFileTarget,
    game_version: Option<String>,
    mod_loader: Option<ModLoader>,
    index: u32,
) -> Result<Vec<ProjectFileInfo>, String> {
    ipc::call(
        "list_project_files",
        GetFilesArgs {
            mod_id,
            target,
            game_version,
            mod_loader,
            index,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallModpackArgs {
    instance_name: String,
    category: String,
    source: DownloadSource,
}

impl InstallModpackArgs {
    pub fn from_form_data(source: DownloadSource, data: &web_sys::FormData) -> Option<Self> {
        let get = |k: &str| data.get(k).as_string().unwrap_or_default();
        let instance_name = get("instance_name");
        if instance_name.trim().is_empty() {
            return None;
        }
        Some(Self {
            instance_name,
            category: get("category"),
            source,
        })
    }
}

pub async fn call_install_modpack(args: InstallModpackArgs) -> Result<(), String> {
    ipc::call("install_modpack", args).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpgradeModpackArgs {
    instance_id: String,
    source: DownloadSource,
}

pub async fn call_upgrade_modpack(
    instance_id: String,
    source: DownloadSource,
) -> Result<(), String> {
    ipc::call(
        "upgrade_modpack",
        UpgradeModpackArgs {
            instance_id,
            source,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadModsArgs {
    files: Vec<ProjectFileInfo>,
    instance_id: String,
}

pub async fn call_download_mods(
    files: Vec<ProjectFileInfo>,
    instance_id: String,
) -> Result<(), String> {
    ipc::call("download_mods", DownloadModsArgs { files, instance_id }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkModsArgs {
    instance_id: String,
    file_paths: Vec<String>,
}

/// Outcome of [`call_link_mods`]: jars linked, and supplied paths that matched
/// no failed mod.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LinkOutcome {
    pub linked: Vec<String>,
    pub unmatched: Vec<String>,
}

pub async fn call_link_mods(
    instance_id: String,
    file_paths: Vec<String>,
) -> Result<LinkOutcome, String> {
    ipc::call(
        "link_mods",
        LinkModsArgs {
            instance_id,
            file_paths,
        },
    )
    .await
}

pub async fn call_pick_jar_files() -> Result<Vec<String>, String> {
    ipc::call_noargs("pick_jar_files").await
}

// ── Manual mod management (issue #29) ───────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListInstanceModsArgs {
    instance_id: String,
}

pub async fn call_list_mods(instance_id: String) -> Result<Vec<ModListEntry>, String> {
    ipc::call("list_instance_mods", ListInstanceModsArgs { instance_id }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ToggleModStateArgs {
    instance_id: String,
    file_name: String,
}

pub async fn call_toggle_mod_state(instance_id: String, file_name: String) -> Result<(), String> {
    ipc::call(
        "toggle_state_instance_mod",
        ToggleModStateArgs {
            instance_id,
            file_name,
        },
    )
    .await
}

pub async fn call_get_minecraft_versions() -> Result<Vec<GameVersion>, String> {
    ipc::call_noargs("get_minecraft_versions").await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoaderVersionsArgs<'a> {
    kind: &'a str,
    mc_version: &'a str,
}

pub async fn call_get_modloader_versions(
    kind: &str,
    mc_version: &str,
) -> Result<Vec<LoaderVersion>, String> {
    ipc::call(
        "get_modloader_versions",
        LoaderVersionsArgs { kind, mc_version },
    )
    .await
}

pub fn fmt_downloads(n: u32) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f32 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f32 / 1_000.0)
    } else {
        n.to_string()
    }
}
