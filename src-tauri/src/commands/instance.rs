use std::collections::{HashMap, HashSet};
use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use base64::Engine;
use log::{info, warn};
use tauri::State;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, InstanceList, InstanceMeta, ModListEntry, ModState, ProjectFileTarget,
    SkippedInstance,
};
use yaminabe_launcher_shared::error::Error;
use crate::{emit_progress, libraries_dir, versions_dir, ActivityGuard, AppState, InstanceActivity};
use crate::commands::java::download_java_runtime;
use crate::http_utils::download_resource;
use crate::json::{read_json, read_json_or_default, write_json};
use crate::install_task::{ensure_game_and_loader, version_manifest_path};

/// The launcher's own directory inside an instance. Its contents are the
/// launcher's record of the instance, and no modpack may write there.
pub const LAUNCHER_DIR: &str = ".launcher";

/// Whether a path component names [`LAUNCHER_DIR`], read the way the
/// filesystem will read it.
///
/// An exact comparison refuses `.launcher` and admits `.Launcher`, which lands
/// in the very same directory on Windows and macOS. Windows also drops trailing
/// dots and spaces, so `.launcher.` and `.launcher ` arrive there too.
pub fn is_launcher_dir(component: &str) -> bool {
    component
        .trim_end_matches(['.', ' '])
        .eq_ignore_ascii_case(LAUNCHER_DIR)
}

/// Whether a path component climbs to a parent directory.
///
/// `..` is the plain spelling. Windows drops trailing dots and spaces before
/// resolving a component, so `.. ` arrives at the same place — the very
/// normalisation [`is_launcher_dir`] is written against, and the reason an
/// exact comparison is not enough here either. A component of nothing but dots
/// and spaces is never a name a pack meant to ship, so any holding two or more
/// dots is treated as a climb rather than reasoned about further.
pub fn is_parent_dir(component: &str) -> bool {
    component.chars().filter(|c| *c == '.').count() >= 2
        && component.chars().all(|c| c == '.' || c == ' ')
}

/// Whether a path component names the directory it sits in, and so descends
/// nowhere. Empty comes from a doubled separator; `.` and `. ` both resolve to
/// the current directory once Windows has trimmed the component.
pub fn is_current_dir(component: &str) -> bool {
    component.trim_end_matches(['.', ' ']).is_empty()
}

pub fn instance_meta_file(instance_dir: &Path) -> PathBuf {
    instance_dir.join(LAUNCHER_DIR).join("instance.json")
}

/// Instance metadata used to live at `<instance>/instance.json`. Move a file
/// left at the old path into `.launcher/`, so an instance created before the
/// move keeps appearing in the library instead of silently vanishing with its
/// worlds still on disk.
fn migrate_legacy_meta(instance_dir: &Path) {
    let current = instance_meta_file(instance_dir);
    let legacy = instance_dir.join("instance.json");
    if current.exists() || !legacy.exists() {
        return;
    }
    if let Some(parent) = current.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            warn!("cannot create {} for legacy metadata: {e}", parent.display());
            return;
        }
    }
    match std::fs::rename(&legacy, &current) {
        Ok(()) => info!("moved legacy metadata into {}", current.display()),
        Err(e) => warn!("cannot move {}: {e}", legacy.display()),
    }
}

pub fn modlist_file(instance_dir: &Path) -> PathBuf {
    instance_dir.join(LAUNCHER_DIR).join("modlist.json")
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

/// Whether `name` addresses a file inside one directory and nowhere else.
/// Separators and `..` are the obvious escapes; a drive prefix is the quiet one,
/// since joining `C:evil.jar` discards the directory it was joined onto.
pub fn is_bare_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && !name.contains(':')
}

/// Remove a directory that was created for an instance that never finished
/// installing. Only ever touches one with no metadata file, so a real instance
/// cannot be deleted this way; leaving it would block retrying the same name,
/// while the library would never show it to be deleted by hand.
fn discard_unfinished_instance_dir(instance_path: &Path) {
    if instance_meta_file(instance_path).exists() {
        return;
    }
    if let Err(e) = std::fs::remove_dir_all(instance_path) {
        warn!("cannot clear the unfinished install at {}: {e}", instance_path.display());
    }
}

/// The file in [`LAUNCHER_DIR`] that marks a new instance's folder as still
/// being installed, from the folder's creation until its record is written.
fn installing_marker(instance_dir: &Path) -> PathBuf {
    instance_dir.join(LAUNCHER_DIR).join("installing")
}

/// Where a new folder's marker is made and locked before it moves into place,
/// so a scan never finds the marker unlocked while the install is running.
fn staged_marker(instance_dir: &Path) -> PathBuf {
    instance_dir.join(".installing")
}

/// A new instance's folder while its install runs.
///
/// The install holds the folder's marker locked throughout. The lock goes with
/// the process, so a later scan, in this launcher or another, can tell a running
/// install from one cut short by the launcher closing or the machine going down.
/// Dropped before [`finish`](Self::finish), it clears the folder away, as an
/// install that fails has nothing usable to leave.
pub struct NewInstanceDir {
    path: PathBuf,
    marker: Option<File>,
    finished: bool,
}

impl NewInstanceDir {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Hand the folder over to the instance whose record has just been written
    /// into it. A marker left behind is cleared by the next scan instead.
    pub fn finish(mut self) {
        self.finished = true;
        drop(self.marker.take());
        let marker = installing_marker(&self.path);
        if let Err(e) = std::fs::remove_file(&marker) {
            warn!("cannot remove {}: {e}", marker.display());
        }
    }
}

impl Drop for NewInstanceDir {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // Closed first, so no handle of ours is open in the folder it removes.
        drop(self.marker.take());
        discard_unfinished_instance_dir(&self.path);
    }
}

fn sort_modlist(entries: &mut [ModListEntry]) {
    entries.sort_by(|a, b| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()));
}

/// Write `entries` as the whole modlist, replacing whatever was there.
///
/// For an upgrade, which recomputes the mod set outright: merging would leave
/// rows describing jars the upgrade has just deleted.
pub fn replace_modlist_entries(
    instance_dir: &Path,
    mut entries: Vec<ModListEntry>,
) -> Result<(), Error> {
    sort_modlist(&mut entries);
    write_json(modlist_file(instance_dir), &entries)
}

/// Create the directory a new instance will live in, refusing to reuse one that
/// already exists — installing over an existing instance would overlay its files
/// and overwrite its record, orphaning it from the library.
pub fn create_instance_dir(install_dir: &str, instance_name: &str) -> Result<NewInstanceDir, Error> {
    let folder = instance_name.trim().to_lowercase();
    // The name is untrusted: an import fills it from the pack's own manifest.
    // A separator or `..` here would put the instance outside the install
    // directory, which the overrides extracted into it would then follow.
    if !is_bare_file_name(&folder) {
        return Err(Error::Invalid(format!(
            "'{instance_name}' cannot be used as an instance name"
        )));
    }
    let instance_path = Path::new(install_dir).join(&folder);
    if instance_path.exists() {
        return Err(Error::Invalid(format!(
            "folder '{folder}' already exists at this location"
        )));
    }
    std::fs::create_dir_all(&instance_path)?;
    // Owned from here, so a marker that cannot be made still clears the folder.
    let mut new_dir = NewInstanceDir { path: instance_path, marker: None, finished: false };
    let staged = staged_marker(&new_dir.path);
    let marker = File::create(&staged)?;
    marker.lock()?;
    new_dir.marker = Some(marker);
    // The folder counts as an instance only once `.launcher/` exists, and by
    // then the staged marker already hides it from the scan.
    std::fs::create_dir(new_dir.path.join(LAUNCHER_DIR))?;
    std::fs::rename(&staged, installing_marker(&new_dir.path))?;
    Ok(new_dir)
}

pub fn find_instance_dir(install_dir: &Path, id: &str) -> Result<PathBuf, Error> {
    install_dir.read_dir()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .find_map(|e| {
            let path = e.path();
            migrate_legacy_meta(&path);
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
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    // Dropped by any `fail!` below, it clears the folder away.
    let instance_dir = match create_instance_dir(&install_dir, &name) {
        Ok(dir) => dir,
        Err(e) => fail!(e),
    };

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
    if let Err(e) = write_json(instance_meta_file(instance_dir.path()), &instance_meta) { fail!(e); }
    info!("Created '{}' (MC {}, {}) → {}", name, mc_version, mod_loader, instance_dir.path().display());
    instance_dir.finish();

    emit_progress(&app_handle, &id, &name, "Done", true, None);
    Ok(())
}

#[tauri::command]
pub async fn get_instances(state: State<'_, AppState>) -> Result<InstanceList, Error> {
    let location = state.settings.read().unwrap().instance_install_dir.clone();
    if location.is_empty() {
        return Ok(InstanceList::default());
    }
    let root = PathBuf::from(&location);
    if !root.exists() {
        return Ok(InstanceList::default());
    }
    let list = scan_instances(&root)?;
    for skipped in &list.skipped {
        warn!("skipped instance folder '{}': {}", skipped.folder, skipped.reason);
    }
    info!("Found {} instances", list.instances.len());
    Ok(list)
}

/// Read every instance under `root`. A folder with launcher data whose
/// `instance.json` cannot be loaded is reported in `skipped`, leaving the rest of
/// the library to load; a folder without any, or one an install is still
/// filling, is not an instance and is ignored. One an interrupted install left
/// is cleared away.
fn scan_instances(root: &Path) -> Result<InstanceList, Error> {
    let mut list = InstanceList::default();
    for entry in std::fs::read_dir(root)?.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        migrate_legacy_meta(&path);
        clear_interrupted_install(&path);
        if !path.join(LAUNCHER_DIR).is_dir() || is_unfinished_install(&path) { continue; }
        match read_json::<InstanceMeta>(instance_meta_file(&path)) {
            Ok(meta) => list.instances.push(meta),
            Err(e) => list.skipped.push(SkippedInstance {
                folder: entry.file_name().to_string_lossy().into_owned(),
                reason: skip_reason(&e),
            }),
        }
    }
    list.instances.sort_by(|a, b| a.name.cmp(&b.name));
    list.skipped.sort_by(|a, b| a.folder.cmp(&b.folder));
    Ok(list)
}

/// Clear away what an install cut short left in `dir`. The lock on its marker
/// went with the process, so finding it unlocked means no install is filling
/// the folder any more. Without a record, the folder holds nothing usable and
/// only blocks its name. With one, the install had finished, and only the
/// marker is stray.
fn clear_interrupted_install(dir: &Path) {
    let marker_path = installing_marker(dir);
    let Ok(marker) = File::open(&marker_path) else { return };
    match marker.try_lock() {
        Ok(()) => drop(marker),
        // An install in this launcher or another is still running.
        Err(TryLockError::WouldBlock) => return,
        Err(TryLockError::Error(e)) => {
            warn!("cannot tell whether an install is still filling {}: {e}", dir.display());
            return;
        }
    }
    if instance_meta_file(dir).exists() {
        if let Err(e) = std::fs::remove_file(&marker_path) {
            warn!("cannot remove {}: {e}", marker_path.display());
        }
    } else {
        info!("clearing {}, left by an interrupted install", dir.display());
        discard_unfinished_instance_dir(dir);
    }
}

/// Whether `dir` is not an instance yet, nor a damaged one: an install is still
/// filling it or still setting up its marker, or it is what an install from
/// before the marker left when closed part-way, a `.launcher/` holding nothing
/// but the cache its modpack download is staged in.
fn is_unfinished_install(dir: &Path) -> bool {
    // A record outweighs either marker. The folder is also the game's, so a
    // stray `.installing` in it may not be the launcher's.
    if staged_marker(dir).exists() || installing_marker(dir).exists() {
        return !instance_meta_file(dir).exists();
    }
    let cache = dir.join(ProjectFileTarget::Modpack.directory());
    let Ok(entries) = std::fs::read_dir(dir.join(LAUNCHER_DIR)) else { return false };
    let paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths == [cache]
}

/// Why an `instance.json` would not load, put the way a person reading the log
/// would want it.
fn skip_reason(error: &Error) -> String {
    match error {
        Error::IO(e) if e.kind() == std::io::ErrorKind::NotFound => {
            "instance.json is missing".to_string()
        }
        Error::IO(e) => format!("instance.json could not be read: {e}"),
        Error::ParseJson(e) => format!("instance.json is malformed: {e}"),
        other => other.to_string(),
    }
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

/// The image types an instance icon is kept as: its file extension in the
/// launcher directory, and the MIME type it is shown with.
const ICON_TYPES: [(&str, &str); 5] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("webp", "image/webp"),
    ("gif", "image/gif"),
];

/// The extension an icon at `url` is kept under, read off the URL's path, or
/// `None` for a type not in [`ICON_TYPES`].
fn icon_extension(url: &str) -> Option<String> {
    let url = reqwest::Url::parse(url).ok()?;
    let ext = Path::new(url.path()).extension()?.to_str()?.to_ascii_lowercase();
    ICON_TYPES.iter().any(|(known, _)| *known == ext).then_some(ext)
}

/// Keep a copy of the modpack's icon at `url` in the instance's launcher
/// directory, so its library card can show it without the network.
pub async fn save_instance_icon(id: &str, url: &str, state: &State<'_, AppState>) -> Result<(), Error> {
    let ext = icon_extension(url)
        .ok_or_else(|| Error::Unsupported(format!("icon type of {url}")))?;
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let dir = find_instance_dir(Path::new(&install_dir), id)?;
    let dest = dir.join(LAUNCHER_DIR).join(format!("icon.{ext}"));
    download_resource(&state.http_client, url, "", dest).await
}

/// The instance's icon as a `data:` URL, or `None` for one without: created by
/// hand, installed from a file, or installed before icons were kept.
#[tauri::command]
pub async fn get_instance_icon(id: String, state: State<'_, AppState>) -> Result<Option<String>, Error> {
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let launcher_dir = find_instance_dir(Path::new(&install_dir), &id)?.join(LAUNCHER_DIR);
    let Some((path, mime)) = ICON_TYPES
        .iter()
        .map(|(ext, mime)| (launcher_dir.join(format!("icon.{ext}")), mime))
        .find(|(path, _)| path.exists())
    else {
        return Ok(None);
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(std::fs::read(path)?);
    Ok(Some(format!("data:{mime};base64,{encoded}")))
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
            project_name: String::new(),
            icon_url: None,
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
    if !is_bare_file_name(&file_name) {
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

#[cfg(test)]
mod scan_tests {
    use super::{
        create_instance_dir, installing_marker, instance_meta_file, modlist_file, scan_instances,
        staged_marker, LAUNCHER_DIR,
    };
    use std::path::{Path, PathBuf};
    use yaminabe_launcher_shared::datamodels::{InstanceMeta, ProjectFileTarget};

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yaminabe-scan-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("create root");
        dir
    }

    fn named(name: &str) -> InstanceMeta {
        InstanceMeta { name: name.to_string(), ..InstanceMeta::default() }
    }

    /// Write a modpack zip into `dir`'s install cache, as an install stages it.
    fn stage_pack(dir: &Path) {
        let cache = dir.join(ProjectFileTarget::Modpack.directory());
        std::fs::create_dir_all(&cache).expect("mkdir");
        std::fs::write(cache.join("pack.zip"), b"PK").expect("write");
    }

    /// Two instances that load, three whose metadata will not, and the folders
    /// and files an install directory may also hold. Only the three broken
    /// instances are reported, and the rest of the library still loads.
    #[test]
    fn a_broken_instance_is_reported_and_the_rest_still_load() {
        let root = temp_root("broken");
        crate::json::write_json(instance_meta_file(&root.join("good")), &named("Good")).expect("write");
        // Metadata from before `.launcher/` existed is moved in before the check.
        crate::json::write_json(root.join("legacy").join("instance.json"), &named("Legacy")).expect("write");
        std::fs::create_dir_all(root.join("malformed").join(LAUNCHER_DIR)).expect("mkdir");
        std::fs::write(instance_meta_file(&root.join("malformed")), b"{ not json").expect("write");
        // A vanilla instance's `.launcher/` holds its metadata and nothing else.
        std::fs::create_dir_all(root.join("missing").join(LAUNCHER_DIR)).expect("mkdir");
        stage_pack(&root.join("cached"));
        crate::json::write_json(modlist_file(&root.join("cached")), &Vec::<String>::new()).expect("write");
        // Not instances: a folder with no launcher data, an install closed
        // before it got past the download, and a stray file.
        std::fs::create_dir_all(root.join("screenshots")).expect("mkdir");
        stage_pack(&root.join("unfinished"));
        std::fs::write(root.join("notes.txt"), b"mine").expect("write");

        let list = scan_instances(&root).expect("scan");

        let names: Vec<&str> = list.instances.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["Good", "Legacy"]);
        let folders: Vec<&str> = list.skipped.iter().map(|s| s.folder.as_str()).collect();
        assert_eq!(folders, vec!["cached", "malformed", "missing"]);
        assert_eq!(list.skipped[0].reason, "instance.json is missing");
        assert!(list.skipped[1].reason.starts_with("instance.json is malformed"), "{}", list.skipped[1].reason);
        assert_eq!(list.skipped[2].reason, "instance.json is missing");
        std::fs::remove_dir_all(&root).ok();
    }

    /// The scan clears a folder whose install was cut short and drops the stray
    /// marker of one that had finished, but leaves an install still running or
    /// still setting up its marker alone, unlisted until it finishes. A finished
    /// instance stays listed with a stray `.installing` beside its record.
    #[test]
    fn an_interrupted_install_is_cleared_and_a_running_one_left_alone() {
        let root = temp_root("interrupted");
        let root_str = root.to_str().expect("utf-8 path");
        // Nothing holds these markers locked, as with a process that is gone.
        let interrupted = root.join("interrupted");
        stage_pack(&interrupted);
        std::fs::write(installing_marker(&interrupted), b"").expect("write");
        let finished = root.join("finished");
        crate::json::write_json(instance_meta_file(&finished), &named("Finished")).expect("write");
        std::fs::write(installing_marker(&finished), b"").expect("write");
        let running = create_instance_dir(root_str, "Running").expect("create");
        // Caught between making `.launcher/` and moving the marker into it.
        let starting = root.join("starting");
        std::fs::create_dir_all(starting.join(LAUNCHER_DIR)).expect("mkdir");
        std::fs::write(staged_marker(&starting), b"").expect("write");
        let stray = root.join("stray");
        crate::json::write_json(instance_meta_file(&stray), &named("Stray")).expect("write");
        std::fs::write(staged_marker(&stray), b"").expect("write");

        let list = scan_instances(&root).expect("scan");

        assert!(!interrupted.exists());
        assert!(!installing_marker(&finished).exists());
        assert!(installing_marker(running.path()).exists());
        assert!(!staged_marker(running.path()).exists());
        assert!(staged_marker(&starting).exists());
        let names: Vec<&str> = list.instances.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["Finished", "Stray"]);
        assert!(list.skipped.is_empty());
        drop(running);
        std::fs::remove_dir_all(&root).ok();
    }

    /// A new instance's folder is kept once finished, without its marker, and
    /// cleared away when dropped unfinished, freeing the name to try again.
    #[test]
    fn a_new_instance_dir_is_kept_only_once_finished() {
        let root = temp_root("new-dir");
        let root_str = root.to_str().expect("utf-8 path");

        let kept = create_instance_dir(root_str, "Kept").expect("create");
        let kept_path = kept.path().to_path_buf();
        crate::json::write_json(instance_meta_file(&kept_path), &named("Kept")).expect("write");
        kept.finish();
        assert!(instance_meta_file(&kept_path).exists());
        assert!(!installing_marker(&kept_path).exists());

        let failed = create_instance_dir(root_str, "Failed").expect("create");
        let failed_path = failed.path().to_path_buf();
        drop(failed);
        assert!(!failed_path.exists());
        assert!(create_instance_dir(root_str, "Failed").is_ok());
        std::fs::remove_dir_all(&root).ok();
    }
}

#[cfg(test)]
mod icon_tests {
    use super::icon_extension;

    #[test]
    fn icon_extension_reads_the_url_path() {
        assert_eq!(icon_extension("https://media.forgecdn.net/avatars/1/2/logo.png").as_deref(), Some("png"));
        assert_eq!(icon_extension("https://cdn.modrinth.com/data/abc/icon.WEBP?v=2").as_deref(), Some("webp"));
    }

    #[test]
    fn icon_extension_refuses_other_types() {
        assert_eq!(icon_extension("https://cdn.modrinth.com/data/abc/icon.svg"), None);
        assert_eq!(icon_extension("https://cdn.modrinth.com/data/abc/icon"), None);
        assert_eq!(icon_extension("not a url"), None);
    }
}
