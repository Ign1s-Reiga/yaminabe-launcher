use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use log::info;
use tauri::State;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, InstanceMeta, ModListEntry, ModState, ProjectFileTarget,
};
use yaminabe_launcher_shared::error::Error;
use crate::{emit_progress, libraries_dir, versions_dir, ActivityGuard, AppState, InstanceActivity};
use crate::commands::java::download_java_runtime;
use crate::json::{read_json, read_json_or_default, write_json};
use crate::install_task::{ensure_game_and_loader, version_manifest_path};

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

pub fn replace_modlist_entries_for_file_ids(
    instance_dir: &Path,
    old_file_ids: &[u32],
    old_file_names: &[String],
    new_entries: Vec<ModListEntry>,
) -> Result<(), Error> {
    let old_file_ids: HashSet<u32> = old_file_ids.iter().copied().collect();
    let old_file_names: HashSet<&str> = old_file_names.iter().map(String::as_str).collect();
    let mut modlist: Vec<ModListEntry> = read_json_or_default(modlist_file(instance_dir))?;
    modlist.retain(|entry| {
        if old_file_names.contains(entry.file_name.as_str()) { return false; }
        !matches!(&entry.source, DownloadSource::CurseForge { file_id, .. } if old_file_ids.contains(file_id))
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

    let version_id = match ensure_game_and_loader(
        &app_handle, &id, &name, &mc_version, &mod_loader, &mod_loader_version, &state,
    ).await {
        Ok(id) => id,
        Err(e) => fail!(e),
    };

    // Resolve and download the JRE the game launches with. This is independent
    // of the loader install (those installers shell out to the system `java`),
    // but depends on the vanilla manifest `ensure_game_and_loader` just wrote.
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

    instance_meta.version_id = version_id;

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

/// Every mod the instance has: the tracked entries of `modlist.json` plus the
/// jars sitting in `mods/` that no entry claims. `DownloadFailed` entries have
/// no file yet and are reported from the modlist alone.
#[tauri::command]
pub fn list_instance_mods(
    instance_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ModListEntry>, Error> {
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let instance_dir = find_instance_dir(Path::new(&install_dir), &instance_id)?;
    let mut modlist: Vec<ModListEntry> = read_json(modlist_file(&instance_dir))
        .unwrap_or_default();
    let untracked = untracked_mods(&instance_dir, &modlist);
    modlist.extend(untracked);
    sort_modlist(&mut modlist);
    Ok(modlist)
}

/// Jars in `mods/` that the launcher did not download — dropped in by hand, or
/// installed before the modlist existed. They carry no provenance, so they are
/// reported as `Manual` with their state read off the `.disabled` suffix. An
/// enabled jar wins over a `<name>.disabled` twin of the same mod.
fn untracked_mods(instance_dir: &Path, modlist: &[ModListEntry]) -> Vec<ModListEntry> {
    let tracked: HashSet<&str> =
        modlist.iter().map(|entry| entry.file_name.as_str()).collect();
    let Ok(dir) = std::fs::read_dir(instance_dir.join("mods")) else {
        return Vec::new();
    };
    let mut found: HashMap<String, ModListEntry> = HashMap::new();
    for dir_entry in dir.flatten() {
        if !dir_entry.path().is_file() { continue; }
        let name = dir_entry.file_name().to_string_lossy().into_owned();
        let (file_name, state) = match name.strip_suffix(".disabled") {
            Some(base) => (base.to_string(), ModState::Disabled),
            None => (name, ModState::Enabled),
        };
        if !file_name.ends_with(".jar") || tracked.contains(file_name.as_str()) { continue; }
        if state == ModState::Disabled && found.contains_key(&file_name) { continue; }
        found.insert(file_name.clone(), ModListEntry {
            file_name,
            sha1: String::new(),
            source: DownloadSource::Manual,
            target: ProjectFileTarget::Mod,
            size: dir_entry.metadata().map(|m| m.len()).unwrap_or_default(),
            state,
        });
    }
    found.into_values().collect()
}

/// Toggle a mod between `Enabled` and `Disabled`. Disabling renames its jar to
/// `<name>.disabled` (which mod loaders ignore) and flips the modlist state;
/// enabling renames it back. The modlist only ever holds mods (resource packs
/// aren't tracked), so the file always lives under `mods/`. An untracked jar
/// carries its state in the file name alone, so for those the rename is the
/// whole toggle. A `DownloadFailed` entry has no jar yet and can't be toggled.
#[tauri::command]
pub fn toggle_state_instance_mod(
    instance_id: String,
    file_name: String,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    // Only a bare file name inside `mods/` may be touched — reject traversal.
    if file_name.is_empty() || file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
        return Err(Error::Invalid(format!("invalid mod file name '{file_name}'")));
    }
    let Some(_activity) = ActivityGuard::claim(&state.instance_activity, &instance_id, InstanceActivity::Modifying) else {
        return Err(Error::Busy(format!("instance '{instance_id}' is already busy")));
    };
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let instance_dir = find_instance_dir(Path::new(&install_dir), &instance_id)?;
    let mut modlist: Vec<ModListEntry> = read_json_or_default(modlist_file(&instance_dir))?;
    let mods_dir = instance_dir.join("mods");
    let enabled_path = mods_dir.join(&file_name);
    let disabled_path = mods_dir.join(format!("{file_name}.disabled"));

    let Some(entry) = modlist.iter_mut().find(|e| e.file_name == file_name) else {
        let new_state = if enabled_path.exists() {
            std::fs::rename(&enabled_path, &disabled_path)?;
            ModState::Disabled
        } else if disabled_path.exists() {
            std::fs::rename(&disabled_path, &enabled_path)?;
            ModState::Enabled
        } else {
            return Err(Error::NotExists(format!("mod '{file_name}'")));
        };
        info!("Toggled untracked mod '{file_name}' to {new_state:?} in instance '{instance_id}'");
        return Ok(());
    };

    match entry.state {
        ModState::Enabled => {
            if enabled_path.exists() {
                std::fs::rename(&enabled_path, &disabled_path)?;
            }
            entry.state = ModState::Disabled;
        }
        ModState::Disabled => {
            if disabled_path.exists() {
                std::fs::rename(&disabled_path, &enabled_path)?;
            }
            entry.state = ModState::Enabled;
        }
        ModState::DownloadFailed => {
            return Err(Error::Invalid(format!("mod '{file_name}' has not been downloaded yet")));
        }
    }
    let new_state = entry.state;
    write_json(modlist_file(&instance_dir), &modlist)?;
    info!("Toggled mod '{file_name}' to {new_state:?} in instance '{instance_id}'");
    Ok(())
}
