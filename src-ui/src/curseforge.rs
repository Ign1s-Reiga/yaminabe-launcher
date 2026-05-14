use crate::ipc;
use leptos::web_sys;
use serde::Serialize;
use yaminabe_launcher_shared::datatypes::{GameVersion, LoaderVersion, ModpackSearchResults, ModpackVersionFile};

// ── IPC ───────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct SearchArgs {
    query: String,
    index: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GetFilesArgs {
    mod_id: u32,
}

pub async fn call_search(query: String, index: u32) -> Result<ModpackSearchResults, String> {
    ipc::call("search_curseforge_modpacks", SearchArgs { query, index }).await
}

pub async fn call_get_files(mod_id: u32) -> Result<Vec<ModpackVersionFile>, String> {
    ipc::call("get_modpack_files", GetFilesArgs { mod_id }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallArgs {
    download_url: String,
    instance_name: String,
    install_dir: String,
    category: String,
}

impl InstallArgs {
    pub fn from_form_data(install_dir: String, download_url: String, data: &web_sys::FormData) -> Option<Self> {
        let get = |k: &str| data.get(k).as_string().unwrap_or_default();
        let instance_name = get("instance_name");
        if instance_name.trim().is_empty() { return None; }
        Some(Self {
            download_url,
            instance_name,
            install_dir,
            category: get("category"),
        })
    }
}

pub async fn call_install(
    args: InstallArgs
) -> Result<(), String> {
    ipc::call("install_curseforge_modpack", args).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadModsArgs {
    file_ids: Vec<String>,
    instance_location: String,
    source: Option<String>,
}

pub async fn call_download_mods(
    file_ids: Vec<String>,
    instance_location: String,
    source: Option<String>,
) -> Result<(), String> {
    ipc::call("download_mods", DownloadModsArgs { file_ids, instance_location, source }).await
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

pub async fn call_get_modloader_versions(kind: &str, mc_version: &str) -> Result<Vec<LoaderVersion>, String> {
    ipc::call("get_modloader_versions", LoaderVersionsArgs { kind, mc_version }).await
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
