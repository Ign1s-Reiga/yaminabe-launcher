mod commands;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{Emitter, Manager};
use yaminabe_launcher_shared::datatypes::{AppSettings, InstanceMeta};
use yaminabe_launcher_shared::datatypes::JavaInstall;
use crate::commands::curseforge::{
    fetch_minecraft_modloaders, fetch_minecraft_versions,
    get_minecraft_modloaders, get_minecraft_versions,
    get_modpack_files, install_curseforge_modpack, McModloader, McVersion,
    search_curseforge_modpacks,
};
use crate::commands::instance::{create_instance, download_mods, get_instances, save_instance_settings};
use crate::commands::launch::launch_instance;
use crate::commands::java::{detect_java_installs, get_java_installs};
use crate::commands::settings::{check_paths_exist, get_instance_subfolders, get_settings, open_folder, open_instance_subfolder, pick_folder, save_settings};

// ── Shared types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct InstallProgress {
    pub id: String,
    pub name: String,
    pub step: String,
    pub done: bool,
    pub error: Option<String>,
}

pub fn emit_progress(app: &tauri::AppHandle, id: &str, name: &str, step: &str, done: bool, error: Option<String>) {
    let _ = app.emit("instance-install-progress", InstallProgress {
        id: id.to_string(),
        name: name.to_string(),
        step: step.to_string(),
        done,
        error,
    });
}


pub struct AppState {
    pub settings: Mutex<AppSettings>,
    pub settings_path: PathBuf,
    pub versions_dir: PathBuf,
    pub assets_dir: PathBuf,
    pub libraries_dir: PathBuf,
    pub runtimes_dir: PathBuf,
    pub http_client: reqwest::Client,
    pub mc_versions: Mutex<Vec<McVersion>>,
    pub mc_modloaders: Mutex<Vec<McModloader>>,
    pub java_installs: Mutex<Vec<JavaInstall>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_log::Builder::new()
            .target(tauri_plugin_log::Target::new(
                tauri_plugin_log::TargetKind::Stdout,
            ))
            .build()
        )
        .setup(|app| {
            // Create directory for app data
            let app_dir = app.path().local_data_dir()?.join(".yaminabe");
            let versions_dir  = Path::new(&app_dir).join("bin").join("versions");
            let libraries_dir = Path::new(&app_dir).join("bin").join("libraries");
            let assets_dir    = Path::new(&app_dir).join("bin").join("assets");
            let runtimes_dir  = Path::new(&app_dir).join("bin").join("runtimes");
            for p in [&versions_dir, &libraries_dir, &assets_dir, &runtimes_dir] {
                std::fs::create_dir_all(p)?;
            }

            // Initialize and load AppSettings
            let settings_path = app_dir.join("settings.json");
            let mut settings: AppSettings = std::fs::read_to_string(&settings_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            if settings.instance_install_dir.is_empty() {
                settings.instance_install_dir = app_dir.join("instances")
                    .to_string_lossy()
                    .into_owned();
            }

            std::fs::create_dir_all(Path::new(&settings.instance_install_dir))?;

            let java_installs = detect_java_installs();

            app.manage(AppState {
                settings: Mutex::new(settings),
                settings_path,
                versions_dir,
                assets_dir,
                libraries_dir,
                runtimes_dir,
                http_client: reqwest::Client::new(),
                mc_versions: Mutex::new(Vec::new()),
                mc_modloaders: Mutex::new(Vec::new()),
                java_installs: Mutex::new(java_installs),
            });

            let handle = app.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                let api_key = state.settings.lock().unwrap().curseforge_api_key.clone();
                if api_key.is_empty() { return; }
                if let Ok(versions) = fetch_minecraft_versions(&api_key, &state.http_client).await {
                    *state.mc_versions.lock().unwrap() = versions;
                }
                if let Ok(loaders) = fetch_minecraft_modloaders(&api_key, &state.http_client).await {
                    *state.mc_modloaders.lock().unwrap() = loaders;
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            pick_folder,
            open_folder,
            check_paths_exist,
            get_instance_subfolders,
            open_instance_subfolder,
            launch_instance,
            search_curseforge_modpacks,
            get_modpack_files,
            install_curseforge_modpack,
            download_mods,
            create_instance,
            get_instances,
            save_instance_settings,
            get_minecraft_versions,
            get_minecraft_modloaders,
            get_java_installs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
