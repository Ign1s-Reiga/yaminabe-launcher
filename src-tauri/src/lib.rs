mod commands;
mod install_task;
mod mod_repo;
mod http_utils;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

use tauri::{Emitter, Manager};
use yaminabe_launcher_shared::datatypes::{AppSettings, JavaInstall};
use yaminabe_launcher_shared::error::InitializationError;
use yaminabe_launcher_shared::ipc::InstallProgress;
use crate::commands::auth::{
    cancel_microsoft_login, get_accounts, get_selected_account, remove_account, set_selected_account,
    start_microsoft_login, AccountStore,
};
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

pub fn emit_progress(app: &tauri::AppHandle, id: &str, name: &str, step: &str, done: bool, error: Option<String>) {
    if let Err(e) = app.emit("instance-install-progress", InstallProgress {
        id: id.to_string(),
        name: name.to_string(),
        step: step.to_string(),
        done,
        error,
    }) {
        log::warn!("failed to emit instance-install-progress for {id}: {e}");
    }
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
    /// Persisted Microsoft accounts plus the currently selected UUID. Backed
    /// by `accounts.json`.
    pub accounts: Mutex<AccountStore>,
    /// Cooperative cancel flag for an in-flight `start_microsoft_login` poll
    /// loop. `cancel_microsoft_login` flips it to `true` to abort cleanly.
    pub ms_login_cancel: Arc<AtomicBool>,
}

static TEMP_DIR: OnceLock<PathBuf> = OnceLock::new();
static SETTINGS_PATH: OnceLock<PathBuf> = OnceLock::new();
static ACCOUNTS_PATH: OnceLock<PathBuf> = OnceLock::new();
static BIN_DIR: OnceLock<PathBuf> = OnceLock::new();
static VERSIONS_DIR: OnceLock<PathBuf> = OnceLock::new();
static ASSETS_DIR: OnceLock<PathBuf> = OnceLock::new();
static LIBRARIES_DIR: OnceLock<PathBuf> = OnceLock::new();
static RUNTIMES_DIR: OnceLock<PathBuf> = OnceLock::new();

fn settings_path() -> &'static PathBuf { SETTINGS_PATH.get().unwrap() }
pub fn accounts_path() -> &'static PathBuf { ACCOUNTS_PATH.get().unwrap() }
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
    ACCOUNTS_PATH.set(app_dir.join("accounts.json"))?;
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

            // Accounts file is optional; a malformed file falls back to empty
            // so a corrupted store doesn't block app launch.
            let accounts: AccountStore = std::fs::read_to_string(accounts_path())
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            app.manage(AppState {
                settings: Mutex::new(settings),
                http_client: reqwest::Client::new(),
                mc_versions: OnceLock::new(),
                java_installs: Mutex::new(java_installs),
                running_children: Mutex::new(HashMap::new()),
                accounts: Mutex::new(accounts),
                ms_login_cancel: Arc::new(AtomicBool::new(false)),
            });

            let handle = app.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                match fetch_minecraft_versions(&state.http_client).await {
                    Ok(manifest) => {
                        // `OnceLock::set` only fails if the cell is already
                        // populated, which can't happen at first-launch init.
                        // Log so it surfaces if that assumption ever breaks.
                        if state.mc_versions.set(manifest).is_err() {
                            log::warn!("Minecraft version manifest cache was unexpectedly already populated");
                        }
                    }
                    Err(e) => log::warn!("failed to fetch Minecraft version manifest: {e}"),
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
            start_microsoft_login,
            cancel_microsoft_login,
            get_accounts,
            get_selected_account,
            set_selected_account,
            remove_account,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
