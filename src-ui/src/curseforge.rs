use crate::ipc;
use leptos::web_sys;
use serde::Serialize;
use yaminabe_launcher_shared::datatypes::{
    DownloadSource, GameVersion, LoaderVersion, ModListEntry, ModLoader, ModProjectSearchResults,
    ModProjectFile,
};

// ── IPC ───────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct SearchModpackArgs {
    query: String,
    index: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GetFilesArgs {
    mod_id: u32,
}

pub async fn call_search_modpacks(query: String, index: u32) -> Result<ModProjectSearchResults, String> {
    ipc::call("search_curseforge_modpacks", SearchModpackArgs { query, index }).await
}

pub async fn call_get_files(mod_id: u32) -> Result<Vec<ModProjectFile>, String> {
    ipc::call("get_modpack_files", GetFilesArgs { mod_id }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallModpackArgs {
    download_url: String,
    instance_name: String,
    install_dir: String,
    category: String,
    project_id: u32,
    file_id: u32,
}

impl InstallModpackArgs {
    pub fn from_form_data(
        install_dir: String,
        download_url: String,
        project_id: u32,
        file_id: u32,
        data: &web_sys::FormData,
    ) -> Option<Self> {
        let get = |k: &str| data.get(k).as_string().unwrap_or_default();
        let instance_name = get("instance_name");
        if instance_name.trim().is_empty() { return None; }
        Some(Self {
            download_url,
            instance_name,
            install_dir,
            category: get("category"),
            project_id,
            file_id,
        })
    }
}

pub async fn call_install_modpack(
    args: InstallModpackArgs
) -> Result<(), String> {
    ipc::call("install_curseforge_modpack", args).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpgradeModpackArgs {
    instance_id: String,
    download_url: String,
    project_id: u32,
    file_id: u32,
}

pub async fn call_upgrade_modpack(
    instance_id: String,
    project_id: u32,
    file_id: u32,
    download_url: String,
) -> Result<(), String> {
    ipc::call("upgrade_curseforge_modpack", UpgradeModpackArgs { instance_id, download_url, project_id, file_id }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadModsArgs {
    mod_files: Vec<DownloadSource>,
    instance_id: String,
}

pub async fn call_download_mods(
    mod_files: Vec<DownloadSource>,
    instance_id: String,
) -> Result<(), String> {
    ipc::call("download_mods", DownloadModsArgs { mod_files, instance_id }).await
}

// ── Manual mod management (issue #29) ───────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchModsArgs {
    query: String,
    mc_version: String,
    mod_loader: ModLoader,
    index: u32,
}

pub async fn call_search_mods(
    query: String,
    mc_version: String,
    mod_loader: ModLoader,
    index: u32,
) -> Result<ModProjectSearchResults, String> {
    ipc::call("search_mods", SearchModsArgs { query, mc_version, mod_loader, index }).await
}

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
struct DeleteModsArgs {
    instance_id: String,
    file_name: String,
}

pub async fn call_delete_mod(instance_id: String, file_name: String) -> Result<(), String> {
    ipc::call("delete_instance_mod", DeleteModsArgs { instance_id, file_name }).await
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
