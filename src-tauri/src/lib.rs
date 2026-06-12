mod auth;
mod commands;
mod install_task;
mod json;
mod mod_repo;
mod http_utils;
mod maven;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use log::warn;
use tauri::{Emitter, Manager};
use yaminabe_launcher_shared::datatypes::{AppSettings, JavaInstall};
use yaminabe_launcher_shared::error::InitializationError;
use yaminabe_launcher_shared::ipc::InstallProgress;
use crate::auth::{load_account_store, AccountStore};
use crate::commands::auth::{
    cancel_microsoft_login, get_accounts, get_selected_account,
    remove_account, set_selected_account, start_microsoft_login,
};
use crate::commands::modfile::{
    delete_instance_mod, download_mods, get_modpack_files, install_modpack, link_mods,
    list_instance_mods, search_curseforge_modpacks,
    search_mods, upgrade_modpack,
};
use crate::commands::minecraft::{
    fetch_minecraft_versions, get_minecraft_versions, get_modloader_versions, VersionManifest,
};
use crate::commands::instance::{create_instance, delete_instance, get_instances, save_instance_settings};
use crate::commands::launch::{kill_instance, launch_instance};
use crate::commands::java::{detect_java_installs, get_java_installs};
use crate::commands::settings::{get_instance_subfolders, get_settings, open_instance_subfolder, pick_folder, pick_jar_files, save_settings};

pub fn emit_progress(app: &tauri::AppHandle, id: &str, name: &str, step: &str, done: bool, error: Option<String>) {
    if let Err(e) = app.emit("instance-install-progress", InstallProgress {
        id: id.to_string(),
        name: name.to_string(),
        step: step.to_string(),
        done,
        error,
    }) {
        warn!("failed to emit instance-install-progress for {id}: {e}");
    }
}


/// What an instance id is currently busy with. Its presence in
/// [`AppState::instance_activity`] makes launching, a second launch,
/// deletion, and content modification mutually exclusive.
pub enum InstanceActivity {
    /// A launch is in flight but its process hasn't spawned yet.
    Preparing,
    /// A launch's process is running with this OS PID.
    Running(u32),
    /// A delete is removing the instance directory.
    Deleting,
    /// A partial edit — a manual mod add or remove (the Mods tab stays usable).
    Modifying,
    /// A whole-instance rewrite — a modpack upgrade or mod-loader version
    /// change. The UI sends the user back to the library and tracks progress in
    /// the activity dock while this is in flight.
    Upgrading,
}

/// Map of instance id → its in-flight exclusive activity.
pub type InstanceActivityMap = Arc<Mutex<HashMap<String, InstanceActivity>>>;

/// RAII claim on an instance id's activity slot. [`ActivityGuard::claim`] inserts
/// it under the lock (failing if already busy) and the guard frees it on drop,
/// covering every exit path out of guarded commands (success, `?`, early return).
pub struct ActivityGuard {
    map: InstanceActivityMap,
    id: String,
}

impl ActivityGuard {
    /// Atomically claim `id` for `activity` if it isn't already occupied.
    /// Returns `None` — leaving the map untouched — when the id is already busy.
    pub fn claim(map: &InstanceActivityMap, id: &str, activity: InstanceActivity) -> Option<Self> {
        let mut guarded = map.lock().unwrap();
        if guarded.contains_key(id) {
            return None;
        }
        guarded.insert(id.to_string(), activity);
        Some(Self { map: map.clone(), id: id.to_string() })
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        self.map.lock().unwrap().remove(&self.id);
    }
}

pub struct AppState {
    pub settings: RwLock<AppSettings>,
    pub http_client: reqwest::Client,
    pub mc_versions: OnceLock<VersionManifest>,
    pub java_installs: Mutex<Vec<JavaInstall>>,
    /// In-flight exclusive activity per instance id. Launch, delete, upgrade,
    /// and manual mod-edit commands claim a slot via `ActivityGuard` under this
    /// lock, so they are mutually exclusive; `kill_instance` reads the
    /// `Running` PID.
    pub instance_activity: InstanceActivityMap,
    /// Persisted Microsoft accounts plus the currently selected UUID. Backed
    /// by `accounts.json`.
    pub account_store: Mutex<AccountStore>,
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
            // Filter out Trace; the builder's default is LevelFilter::Trace,
            // which surfaces every hyper/h2/reqwest trace call on stdout.
            .level(log::LevelFilter::Debug)
            .target(tauri_plugin_log::Target::new(
                tauri_plugin_log::TargetKind::Stdout,
            ))
            .build()
        )
        .setup(|app| {
            init_dirs(app)?;

            // Register the OS-native credential store as the keyring default
            // (DPAPI on Windows, Keychain on macOS, keyutils on Linux). A
            // failure is non-fatal: account secrets simply won't persist and
            // the login flow will surface a clear error next time it's used.
            #[cfg(target_os = "windows")]
            let native_store = windows_native_keyring_store::Store::new();

            #[cfg(target_os = "linux")]
            let native_store = dbus_secret_service_keyring_store::Store::new();

            #[cfg(target_os = "macos")]
            let native_store = apple_native_keyring_store::Store::new();

            match native_store {
                Ok(store) => keyring_core::set_default_store(store),
                Err(e) => warn!("failed to register native keyring store: {e}")
            }

            // Initialize and load AppSettings
            let mut settings: AppSettings = json::read_json(settings_path())?;
            if settings.instance_install_dir.is_empty() {
                settings.instance_install_dir = app.path().local_data_dir()?.join(".yaminabe").join("instances")
                    .to_string_lossy()
                    .into_owned();
            }
            std::fs::create_dir_all(Path::new(&settings.instance_install_dir))?;

            let java_installs = detect_java_installs();

            // Loads the accounts list, migrating any pre-keyring token fields
            // out of accounts.json into the OS keyring on the way through.
            let account_store: AccountStore = load_account_store();

            app.manage(AppState {
                settings: RwLock::new(settings),
                http_client: reqwest::Client::new(),
                mc_versions: OnceLock::new(),
                java_installs: Mutex::new(java_installs),
                instance_activity: Arc::new(Mutex::new(HashMap::new())),
                account_store: Mutex::new(account_store),
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
                            warn!("Minecraft version manifest cache was unexpectedly already populated");
                        }
                    }
                    Err(e) => warn!("failed to fetch Minecraft version manifest: {e}"),
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
            install_modpack,
            upgrade_modpack,
            search_mods,
            list_instance_mods,
            delete_instance_mod,
            download_mods,
            link_mods,
            pick_jar_files,
            create_instance,
            get_instances,
            save_instance_settings,
            delete_instance,
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
