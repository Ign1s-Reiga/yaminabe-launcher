use std::path::{Path, PathBuf};
use log::info;
use tauri::State;
use yaminabe_launcher_shared::datatypes::{ModLoader, InstanceMeta};
use yaminabe_launcher_shared::error::Error;
use crate::{emit_progress, libraries_dir, versions_dir, AppState};
use crate::commands::java::download_java_runtime;
use crate::install_task::{ensure_fabric, ensure_forge, ensure_neoforge, ensure_quilt, ensure_vanilla};

pub fn find_instance_dir(install_dir: &Path, id: &str) -> Result<PathBuf, Error> {
    install_dir.read_dir()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .find_map(|e| {
            let path = e.path();
            let json_path = path.join("instance.json");

            let content = std::fs::read_to_string(&json_path).ok()?;
            let meta: InstanceMeta = serde_json::from_str(&content).ok()?;

            if meta.id == id { Some(path) } else { None }
        })
        .ok_or_else(|| Error::NotExists(format!("Instance '{id}'")))
}

#[tauri::command]
pub async fn create_instance(
    app_handle: tauri::AppHandle,
    mut instance_meta: InstanceMeta,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    if instance_meta.id.is_empty() {
        instance_meta.id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .to_string();
    }
    let InstanceMeta {
        id,
        name,
        game_version: mc_version,
        mod_loader,
        mod_loader_version,
        ..
    } = instance_meta.clone();

    macro_rules! step {
        ($s:expr) => { crate::emit_progress(&app_handle, &id, &name, $s, false, None); };
    }
    macro_rules! fail {
        ($e:expr) => {{
            let err = $e;
            crate::emit_progress(&app_handle, &id, &name, "Failed", false, Some(err.to_string()));
            return Err(err);
        }};
    }

    step!("Preparing directories");
    let instance_path = PathBuf::from(&state.settings.lock().unwrap().instance_install_dir).join(name.to_lowercase());
    if instance_path.exists() {
        fail!(Error::Invalid(format!("folder '{}' already exists at this location", name.to_lowercase())));
    }
    if let Err(e) = std::fs::create_dir_all(&instance_path) { fail!(Error::IO(e)); }

    for dir in [versions_dir(), libraries_dir()] {
        if let Err(e) = std::fs::create_dir_all(dir) { fail!(Error::IO(e)); }
    }

    step!(&format!("Downloading Minecraft {mc_version}"));
    if let Err(e) = ensure_vanilla(&mc_version, &state).await { fail!(e); }

    let java_component = {
        #[derive(serde::Deserialize, Default)]
        struct JavaVersion { component: String }
        #[derive(serde::Deserialize, Default)]
        struct VersionJson {
            #[serde(rename = "javaVersion", default)]
            java_version: JavaVersion,
        }
        let json_path = versions_dir().join(&mc_version).join(format!("{mc_version}.json"));
        std::fs::read_to_string(&json_path).ok()
            .and_then(|s| serde_json::from_str::<VersionJson>(&s).ok())
            .map(|v| v.java_version.component)
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "java-runtime-epsilon".to_string())
    };

    step!(&format!("Downloading Java ({java_component})"));
    let _ = match download_java_runtime(&java_component, &state.http_client).await {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(e) => {
            log::warn!("JRE download failed ({e}), falling back to system javaw");
            "javaw".to_string()
        }
    };

    let require_hint = || mod_loader_version.as_deref()
        .ok_or_else(|| Error::Invalid(format!("Mod loader version required for {mod_loader}")));

    // TODO: Show version what is installing
    match &mod_loader {
        ModLoader::Fabric => {
            step!("Installing Fabric");
            let hint = match require_hint() { Ok(h) => h, Err(e) => fail!(e) };
            if let Err(e) = ensure_fabric(&mc_version, hint, &state.http_client).await { fail!(e); }
        }
        ModLoader::Quilt => {
            step!("Installing Quilt");
            let hint = match require_hint() { Ok(h) => h, Err(e) => fail!(e) };
            if let Err(e) = ensure_quilt(&mc_version, hint, &state.http_client).await { fail!(e); }
        }
        ModLoader::Forge => {
            step!("Installing Forge");
            let hint = match require_hint() { Ok(h) => h, Err(e) => fail!(e) };
            if let Err(e) = ensure_forge(&mc_version, hint, &state.http_client).await { fail!(e); }
        }
        ModLoader::NeoForge => {
            step!("Installing NeoForge");
            let hint = match require_hint() { Ok(h) => h, Err(e) => fail!(e) };
            if let Err(e) = ensure_neoforge(&mc_version, hint, &state.http_client).await { fail!(e); }
        }
        ModLoader::Vanilla => {}
    }

    step!("Finalizing");
    let json_str = match serde_json::to_string_pretty(&instance_meta) {
        Ok(s) => s,
        Err(e) => fail!(Error::from(e)),
    };
    if let Err(e) = std::fs::write(instance_path.join("instance.json"), json_str) { fail!(Error::IO(e)); }

    emit_progress(&app_handle, &id, &name, "Done", true, None);
    info!("Created '{}' (MC {}, {}) → {}", name, mc_version, mod_loader, instance_path.display());
    Ok(())
}

#[tauri::command]
pub async fn get_instances(state: State<'_, AppState>) -> Result<Vec<InstanceMeta>, Error> {
    let location = state.settings.lock().unwrap().instance_install_dir.clone();
    if location.is_empty() {
        return Ok(vec![]);
    }
    let root = PathBuf::from(&location);
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut instances = Vec::new();
    for entry in std::fs::read_dir(&root)?.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        let json_path = path.join("instance.json");
        if !json_path.exists() { continue; }
        let Ok(content) = std::fs::read_to_string(&json_path) else { continue };
        let Ok(meta) = serde_json::from_str::<InstanceMeta>(&content) else { continue };
        instances.push(meta);
    }
    instances.sort_by(|a, b| a.name.cmp(&b.name));
    info!("Found {} instances", instances.len());
    Ok(instances)
}

#[tauri::command]
pub fn save_instance_settings(
    id: String,
    ram_mb: u32,
    jvm_args: String,
    jre_path: String,
    description: String,
    window_width: u32,
    window_height: u32,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    let install_dir = state.settings.lock().unwrap().instance_install_dir.clone();
    let dir = find_instance_dir(Path::new(&install_dir), &id)?;
    let json_path = dir.join("instance.json");
    let content = std::fs::read_to_string(&json_path)?;
    let mut meta: InstanceMeta = serde_json::from_str(&content)?;

    meta.ram_mb = ram_mb;
    meta.jvm_args = jvm_args;
    meta.jre_path = jre_path;
    meta.description = description;
    meta.window_width = window_width;
    meta.window_height = window_height;

    let json = serde_json::to_string_pretty(&meta)?;
    std::fs::write(&json_path, json)?;
    Ok(())
}
