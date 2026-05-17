mod commands;
mod install_task;
mod mod_repo;
mod http_utils;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::{Emitter, Manager};
use yaminabe_launcher_shared::datatypes::{AppSettings, JavaInstall};
use yaminabe_launcher_shared::error::InitializationError;
use crate::commands::modfile::{
    download_mods, get_modpack_files, install_curseforge_modpack,
    search_curseforge_modpacks,
};
use crate::commands::minecraft::{
    fetch_minecraft_versions, get_minecraft_versions, get_modloader_versions, VersionManifest,
};
use crate::commands::instance::{create_instance, get_instances, save_instance_settings};
use crate::commands::launch::{kill_instance, launch_instance};
use crate::commands::java::{detect_java_installs, get_java_installs};
use crate::commands::settings::{get_instance_subfolders, get_settings, open_instance_subfolder, pick_folder, save_settings};

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
    pub http_client: reqwest::Client,
    pub mc_versions: OnceLock<VersionManifest>,
    pub java_installs: Mutex<Vec<JavaInstall>>,
    /// Maps `instance_id` to the OS PID of the currently spawned Java process.
    /// Populated by `launch_instance` after spawn and cleared when the child
    /// exits; read by `kill_instance` to issue a TerminateProcess.
    pub running_children: Mutex<HashMap<String, u32>>,
}

static TEMP_DIR: OnceLock<PathBuf> = OnceLock::new();
static SETTINGS_PATH: OnceLock<PathBuf> = OnceLock::new();
static BIN_DIR: OnceLock<PathBuf> = OnceLock::new();
static VERSIONS_DIR: OnceLock<PathBuf> = OnceLock::new();
static ASSETS_DIR: OnceLock<PathBuf> = OnceLock::new();
static LIBRARIES_DIR: OnceLock<PathBuf> = OnceLock::new();
static RUNTIMES_DIR: OnceLock<PathBuf> = OnceLock::new();

fn settings_path() -> &'static PathBuf { SETTINGS_PATH.get().unwrap() }
pub fn temp_dir() -> &'static PathBuf { TEMP_DIR.get().unwrap() }
pub fn bin_dir() -> &'static PathBuf { BIN_DIR.get().unwrap() }
pub fn versions_dir() -> &'static PathBuf { VERSIONS_DIR.get().unwrap() }
pub fn assets_dir() -> &'static PathBuf { ASSETS_DIR.get().unwrap() }
pub fn libraries_dir() -> &'static PathBuf { LIBRARIES_DIR.get().unwrap() }
pub fn runtimes_dir() -> &'static PathBuf { RUNTIMES_DIR.get().unwrap() }

fn init_dirs(app: &tauri::App) -> Result<(), InitializationError> {
    fn path_err(e: tauri::Error) -> InitializationError {
        InitializationError::PathResolution(e.to_string())
    }
    let app_dir = app.path().local_data_dir().map_err(path_err)?.join(".yaminabe");
    let bin_dir = app_dir.join("bin");
    TEMP_DIR.set(app.path().temp_dir().map_err(path_err)?)?;
    VERSIONS_DIR.set(bin_dir.join("versions"))?;
    LIBRARIES_DIR.set(bin_dir.join("libraries"))?;
    ASSETS_DIR.set(bin_dir.join("assets"))?;
    RUNTIMES_DIR.set(bin_dir.join("runtimes"))?;
    BIN_DIR.set(bin_dir)?;
    SETTINGS_PATH.set(app_dir.join("settings.json"))?;
    for p in [versions_dir(), libraries_dir(), assets_dir(), runtimes_dir()] {
        std::fs::create_dir_all(p)?;
    }
    Ok(())
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
            init_dirs(app)?;

            // Initialize and load AppSettings
            let settings_text = std::fs::read_to_string(settings_path())?;
            let mut settings: AppSettings = serde_json::from_str(&settings_text)?;
            if settings.instance_install_dir.is_empty() {
                settings.instance_install_dir = app.path().local_data_dir()?.join(".yaminabe").join("instances")
                    .to_string_lossy()
                    .into_owned();
            }
            std::fs::create_dir_all(Path::new(&settings.instance_install_dir))?;

            let java_installs = detect_java_installs();

            app.manage(AppState {
                settings: Mutex::new(settings),
                http_client: reqwest::Client::new(),
                mc_versions: OnceLock::new(),
                java_installs: Mutex::new(java_installs),
                running_children: Mutex::new(HashMap::new()),
            });

            let handle = app.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                if let Ok(manifest) = fetch_minecraft_versions(versions_dir(), &state.http_client).await {
                    let _ = state.mc_versions.set(manifest);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            pick_folder,
            get_instance_subfolders,
            open_instance_subfolder,
            launch_instance,
            kill_instance,
            search_curseforge_modpacks,
            get_modpack_files,
            install_curseforge_modpack,
            download_mods,
            create_instance,
            get_instances,
            save_instance_settings,
            get_minecraft_versions,
            get_modloader_versions,
            get_java_installs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
