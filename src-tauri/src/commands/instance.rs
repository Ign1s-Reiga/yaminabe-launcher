use std::path::{Path, PathBuf};
use log::info;
use tauri::State;
use yaminabe_launcher_shared::datatypes::{DownloadSource, InstanceMeta, ModListEntry, ModLoader};
use yaminabe_launcher_shared::error::Error;
use crate::{emit_progress, libraries_dir, versions_dir, ActivityGuard, AppState, InstanceActivity};
use crate::commands::java::download_java_runtime;
use crate::json::{read_json, read_json_or_default, write_json};
use crate::install_task::{
    ensure_fabric, ensure_forge, ensure_neoforge, ensure_quilt, ensure_vanilla,
    version_manifest_path,
};

pub fn instance_meta_file(instance_dir: &Path) -> PathBuf {
    instance_dir.join(".launcher").join("instance.json")
}

pub fn modlist_file(instance_dir: &Path) -> PathBuf {
    instance_dir.join(".launcher").join("modlist.json")
}

pub fn upsert_modlist_entries(instance_dir: &Path, entries: Vec<ModListEntry>) -> Result<(), Error> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut modlist: Vec<ModListEntry> = read_json_or_default(modlist_file(instance_dir))?;
    for entry in entries {
        modlist.retain(|existing| existing.file_name != entry.file_name);
        modlist.push(entry);
    }
    sort_modlist(&mut modlist);
    write_json(modlist_file(instance_dir), &modlist)
}

pub fn remove_modlist_file(instance_dir: &Path, file_name: &str) -> Result<(), Error> {
    let mut modlist: Vec<ModListEntry> = read_json_or_default(modlist_file(instance_dir))?;
    let before = modlist.len();
    modlist.retain(|entry| entry.file_name != file_name);
    if modlist.len() == before {
        return Ok(());
    }
    write_json(modlist_file(instance_dir), &modlist)
}

pub fn replace_modlist_entries_for_file_ids(
    instance_dir: &Path,
    download_source: DownloadSource,
    old_file_ids: &[u32],
    new_entries: Vec<ModListEntry>,
) -> Result<(), Error> {
    let old_file_ids: std::collections::HashSet<u32> = old_file_ids.iter().copied().collect();
    let mut modlist: Vec<ModListEntry> = read_json_or_default(modlist_file(instance_dir))?;
    modlist.retain(|entry| {
        entry.download_source != download_source || !old_file_ids.contains(&entry.file_id)
    });
    for entry in new_entries {
        modlist.retain(|existing| existing.file_name != entry.file_name);
        modlist.push(entry);
    }
    sort_modlist(&mut modlist);
    write_json(modlist_file(instance_dir), &modlist)
}

fn sort_modlist(entries: &mut [ModListEntry]) {
    entries.sort_by(|a, b| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()));
}

pub fn find_instance_dir(install_dir: &Path, id: &str) -> Result<PathBuf, Error> {
    install_dir.read_dir()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .find_map(|e| {
            let path = e.path();
            let meta: InstanceMeta = read_json(instance_meta_file(&path)).ok()?;

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
    let instance_path = PathBuf::from(&state.settings.read().unwrap().instance_install_dir).join(name.to_lowercase());
    if instance_path.exists() {
        fail!(Error::Invalid(format!("folder '{}' already exists at this location", name.to_lowercase())));
    }
    if let Err(e) = std::fs::create_dir_all(&instance_path) { fail!(Error::IO(e)); }

    for dir in [versions_dir(), libraries_dir()] {
        if let Err(e) = std::fs::create_dir_all(dir) { fail!(Error::IO(e)); }
    }

    step!(&format!("Downloading Minecraft {mc_version}"));
    let vanilla_version_id = match ensure_vanilla(&mc_version, &state).await {
        Ok(id) => id,
        Err(e) => fail!(e),
    };

    let java_component = {
        #[derive(serde::Deserialize, Default)]
        struct JavaVersion { component: String }
        #[derive(serde::Deserialize, Default)]
        struct VersionJson {
            #[serde(rename = "javaVersion", default)]
            java_version: JavaVersion,
        }
        let json_path = version_manifest_path(&mc_version);
        std::fs::read_to_string(&json_path).ok()
            .and_then(|s| serde_json::from_str::<VersionJson>(&s).ok())
            .map(|v| v.java_version.component)
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "java-runtime-epsilon".to_string())
    };

    step!(&format!("Downloading Java ({java_component})"));
    if let Err(e) = download_java_runtime(&java_component, &state.http_client).await {
        fail!(Error::Invalid(format!("failed to download recommended JRE '{java_component}': {e}")));
    }

    let require_hint = || mod_loader_version.as_deref()
        .ok_or_else(|| Error::Invalid(format!("Mod loader version required for {mod_loader}")));

    // TODO: Show version what is installing
    let loader_version_id = match &mod_loader {
        ModLoader::Fabric => {
            step!("Installing Fabric");
            let hint = match require_hint() { Ok(h) => h, Err(e) => fail!(e) };
            match ensure_fabric(&mc_version, hint, &state.http_client).await {
                Ok(id) => Some(id),
                Err(e) => fail!(e),
            }
        }
        ModLoader::Quilt => {
            step!("Installing Quilt");
            let hint = match require_hint() { Ok(h) => h, Err(e) => fail!(e) };
            match ensure_quilt(&mc_version, hint, &state.http_client).await {
                Ok(id) => Some(id),
                Err(e) => fail!(e),
            }
        }
        ModLoader::Forge => {
            step!("Installing Forge");
            let hint = match require_hint() { Ok(h) => h, Err(e) => fail!(e) };
            match ensure_forge(&mc_version, hint, &state.http_client).await {
                Ok(id) => Some(id),
                Err(e) => fail!(e),
            }
        }
        ModLoader::NeoForge => {
            step!("Installing NeoForge");
            let hint = match require_hint() { Ok(h) => h, Err(e) => fail!(e) };
            match ensure_neoforge(&mc_version, hint, &state.http_client).await {
                Ok(id) => Some(id),
                Err(e) => fail!(e),
            }
        }
        ModLoader::Vanilla => None,
    };

    // The loader's own version manifest takes precedence; Vanilla instances
    // fall back to the bare MC id.
    instance_meta.version_id = loader_version_id.unwrap_or(vanilla_version_id);

    step!("Finalizing");
    if let Err(e) = write_json(instance_meta_file(&instance_path), &instance_meta) { fail!(e); }

    emit_progress(&app_handle, &id, &name, "Done", true, None);
    info!("Created '{}' (MC {}, {}) → {}", name, mc_version, mod_loader, instance_path.display());
    Ok(())
}

#[tauri::command]
pub async fn get_instances(state: State<'_, AppState>) -> Result<Vec<InstanceMeta>, Error> {
    let location = state.settings.read().unwrap().instance_install_dir.clone();
    if location.is_empty() {
        return Ok(vec![]);
    }
    let root = PathBuf::from(&location);
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut instances: Vec<InstanceMeta> = Vec::new();
    for entry in std::fs::read_dir(&root)?.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        let Ok(meta) = read_json(instance_meta_file(&path)) else { continue };
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
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let dir = find_instance_dir(Path::new(&install_dir), &id)?;
    let mut meta: InstanceMeta = read_json(instance_meta_file(&dir))?;

    meta.ram_mb = ram_mb;
    meta.jvm_args = jvm_args;
    meta.jre_path = jre_path;
    meta.description = description;
    meta.window_width = window_width;
    meta.window_height = window_height;

    write_json(instance_meta_file(&dir), &meta)
}

#[tauri::command]
pub fn delete_instance(id: String, state: State<'_, AppState>) -> Result<(), Error> {
    // Claim a Deleting slot atomically and hold it across removal, so a launch
    // or upgrade can't touch the directory while it is being removed. The guard
    // frees the slot, not the lock, so file I/O doesn't block other instances.
    let Some(_activity) = ActivityGuard::claim(&state.instance_activity, &id, InstanceActivity::Deleting) else {
        return Err(Error::Busy(format!("instance '{id}' is already busy")));
    };
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let dir = find_instance_dir(Path::new(&install_dir), &id)?;
    std::fs::remove_dir_all(&dir)?;
    info!("Deleted instance '{id}' at {}", dir.display());
    Ok(())
}
