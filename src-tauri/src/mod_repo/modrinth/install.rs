use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::api::{name_entries_by_project, selected_file, Version};
use super::mrpack::{
    read_index, resolve_loader, safe_destination, target_for, MrpackFile, MrpackIndex, INDEX_NAME,
};
use crate::commands::instance::{
    create_instance_dir, discard_unfinished_instance_dir, instance_meta_file, is_bare_file_name,
    modlist_file, replace_modlist_entries, upsert_modlist_entries,
};
use crate::emit_progress;
use crate::http_utils::{download_resource, fetch_json};
use crate::install_task::ensure_game_and_loader;
use crate::json::{read_json, read_json_or_default, write_json};
use crate::AppState;
use log::{info, warn};
use tauri::State;
use tokio::sync::Semaphore;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, InstanceMeta, ModListEntry, ModLoader, ModState, ProjectFileTarget,
};
use yaminabe_launcher_shared::error::Error;

/// The name a path ends in, for the modlist.
fn file_name_of(dest: &Path) -> String {
    dest.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A modlist row for a file that was never fetched, so the Mods tab can offer
/// to link it by hand.
fn unfetched_entry(file: &MrpackFile, target: ProjectFileTarget) -> ModListEntry {
    ModListEntry {
        file_name: file_name_of(Path::new(&file.path)),
        project_name: String::new(),
        icon_url: None,
        sha1: file.hashes.sha1.clone(),
        source: DownloadSource::Manual,
        target,
        size: file.file_size,
        state: ModState::DownloadFailed,
    }
}

/// Try each URL the index lists in turn. They are mirrors of one file, so the
/// first that works wins and only the last error is worth reporting.
async fn download_from_mirrors(
    planned: &PlannedDownload,
    client: &reqwest::Client,
) -> Result<(), Error> {
    let mut last = Error::Invalid(format!("no download URL for {}", planned.path));
    for url in &planned.downloads {
        match download_resource(client, url, &planned.sha1, planned.dest.clone()).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                warn!("mirror {url} failed for {}: {e}", planned.path);
                last = e;
            }
        }
    }
    Err(last)
}

/// One file's worth of work, owned so it can be handed to a task of its own.
struct PlannedDownload {
    /// The index's own path, for logging.
    path: String,
    dest: PathBuf,
    /// `None` when the mod list does not model where this file goes.
    target: Option<ProjectFileTarget>,
    sha1: String,
    downloads: Vec<String>,
    file_size: u64,
    /// Whether the instance had this mod turned off before the upgrade.
    was_disabled: bool,
}

/// How many files to fetch at once. Matches what the CurseForge path allows, so
/// neither install is markedly harder on a connection than the other.
const DOWNLOAD_CONCURRENCY: usize = 3;

/// Fetch the planned files a few at a time, in the order planned.
///
/// Only the network work is concurrent. What each result then does to the
/// instance — renaming a mod back to disabled, clearing what a failure
/// superseded — stays with the caller, in order, so two files never race over
/// the same directory and a failure still stops the run.
async fn run_downloads(
    planned: Vec<PlannedDownload>,
    client: &reqwest::Client,
    report: impl Fn(usize, usize),
) -> Result<Vec<(PlannedDownload, Result<(), Error>)>, Error> {
    let total = planned.len();
    let semaphore = Arc::new(Semaphore::new(DOWNLOAD_CONCURRENCY));
    let mut handles = Vec::with_capacity(total);

    for file in planned {
        let client = client.clone();
        let semaphore = Arc::clone(&semaphore);
        handles.push(tokio::spawn(async move {
            let permit = semaphore
                .acquire_owned()
                .await
                .map_err(|e| Error::ChildProcess(format!("semaphore acquire: {e}")))?;
            let outcome = download_from_mirrors(&file, &client).await;
            drop(permit);
            Ok::<_, Error>((file, outcome))
        }));
    }

    let mut finished = Vec::with_capacity(total);
    for handle in handles {
        finished.push(
            handle
                .await
                .map_err(|e| Error::ChildProcess(format!("download task panicked: {e}")))??,
        );
        report(finished.len(), total);
    }
    Ok(finished)
}

/// The instance-relative paths an override tree would write **into a directory
/// the pack owns**, read without writing any of them.
///
/// An upgrade needs these before it deletes anything: a file the new version
/// ships as an override is not a file it dropped, and one the old version
/// shipped that way can only be removed if it was recorded.
///
/// Only `mods/`, `resourcepacks/`, `shaderpacks/` and `datapacks/` are recorded.
/// A pack may ship a world or a config as an override, but the moment the user
/// plays or edits it, it is theirs — recording it would let a later version
/// that stops shipping it delete a month of someone's saves.
fn override_paths<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    instance_path: &Path,
) -> Result<Vec<String>, Error> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut paths: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| Error::Invalid(format!("reading zip entry at index {i}: {e}")))?;
        let name = entry.name().to_string();
        if name.ends_with('/') {
            continue;
        }
        // One walk for both trees: a file shipped in each is still one file.
        let relative = ["overrides/", "client-overrides/"]
            .iter()
            .find_map(|prefix| name.strip_prefix(prefix))
            .filter(|relative| !relative.is_empty());
        let Some(relative) = relative else { continue };
        if target_for(relative).is_none() {
            continue;
        }
        let Some(dest) = safe_destination(instance_path, relative) else {
            continue;
        };
        if let Some(path) = relative_path(instance_path, &dest) {
            if seen.insert(path.clone()) {
                paths.push(path);
            }
        }
    }
    Ok(paths)
}

/// Extract every `overrides/` tree the pack ships. `client-overrides/` is
/// applied after `overrides/`, since it exists precisely to win on a client.
fn extract_overrides<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    instance_path: &Path,
) -> Result<(), Error> {
    for prefix in ["overrides/", "client-overrides/"] {
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| Error::Invalid(format!("reading zip entry at index {i}: {e}")))?;
            let name = entry.name().to_string();
            let Some(relative) = name.strip_prefix(prefix) else {
                continue;
            };
            if relative.is_empty() {
                continue;
            }
            let Some(dest) = safe_destination(instance_path, relative) else {
                warn!("skipping override with an unusable path: {name}");
                continue;
            };
            if name.ends_with('/') {
                std::fs::create_dir_all(&dest)?;
                continue;
            }
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&dest)?;
            std::io::copy(&mut entry, &mut out)?;
        }
    }
    Ok(())
}

/// Install a `.mrpack` from disk. Each file is fetched straight from the URLs
/// the index lists and verified against its hash, so nothing here needs an API
/// key. A file that cannot be fetched is recorded rather than aborting the
/// install, matching how a CurseForge pack handles a file it cannot get.
pub async fn install_modpack_from_file(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    zip_path: &Path,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let instance_path = create_instance_dir(&install_dir, instance_name)?;
    // Nothing usable exists until the instance record is written, so a failure
    // partway through clears the directory rather than leaving one that the
    // library never shows and that blocks retrying the same name.
    let result = install_into(
        app_handle,
        id,
        instance_name,
        category,
        zip_path,
        DownloadSource::Manual,
        &instance_path,
        state,
    )
    .await;
    if result.is_err() {
        discard_unfinished_instance_dir(&instance_path);
    }
    result
}

/// A staged `.mrpack` with its game and loader already installed. Shared by
/// install and upgrade, which differ only in what they do with the files.
struct PreparedPack {
    archive: zip::ZipArchive<std::fs::File>,
    index: MrpackIndex,
    mc_version: String,
    mod_loader: ModLoader,
    loader_version: Option<String>,
    version_id: String,
}

async fn prepare_pack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    zip_path: &Path,
    state: &State<'_, AppState>,
) -> Result<PreparedPack, Error> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?)
        .map_err(|e| Error::Invalid(format!("modpack zip is invalid: {e}")))?;
    let index = read_index(&mut archive)?;
    let mc_version = index
        .dependencies
        .get("minecraft")
        .cloned()
        .ok_or_else(|| Error::Invalid(format!("{INDEX_NAME} names no Minecraft version")))?;
    let (mod_loader, loader_version) = resolve_loader(&index.dependencies);

    let version_id = ensure_game_and_loader(
        app_handle,
        id,
        instance_name,
        &mc_version,
        &mod_loader,
        &loader_version,
        state,
    )
    .await?;

    Ok(PreparedPack {
        archive,
        index,
        mc_version,
        mod_loader,
        loader_version,
        version_id,
    })
}

/// What the instance already had, by file name, so an upgrade can tell an
/// unchanged file from a new one.
fn installed_entries(instance_path: &Path) -> HashMap<String, ModListEntry> {
    let modlist: Vec<ModListEntry> =
        read_json_or_default(modlist_file(instance_path)).unwrap_or_default();
    modlist
        .into_iter()
        .map(|entry| (entry.file_name.clone(), entry))
        .collect()
}

/// Whether `previous` is the same file the index now lists. Compared by hash,
/// not by name: a pack can ship a different jar under a name it used before.
fn is_unchanged(previous: &ModListEntry, file: &MrpackFile) -> bool {
    !previous.sha1.is_empty() && previous.sha1.eq_ignore_ascii_case(&file.hashes.sha1)
}

/// Where a disabled mod actually sits on disk.
fn disabled_path(dest: &Path) -> PathBuf {
    dest.with_file_name(format!("{}.disabled", file_name_of(dest)))
}

/// The names a pack owns at `dest`: the file itself, and — only for a mod —
/// the `.disabled` spelling the launcher toggles it with. Elsewhere that suffix
/// is the user's own name for their own file and none of the pack's business.
fn owned_names(dest: &Path, target: Option<ProjectFileTarget>) -> Vec<PathBuf> {
    let mut names = vec![dest.to_path_buf()];
    if target == Some(ProjectFileTarget::Mod) {
        names.push(disabled_path(dest));
    }
    names
}

/// Remove what the previous version left at a path this run claims but could
/// not write. Left there it loads against the upgraded pack while the mod list
/// reports the file missing, so disk and record would disagree.
///
/// A failure here stops the upgrade. Reporting the file missing while it is
/// still on disk and still loading is the mismatch this exists to prevent, so
/// there is nothing useful to do but leave the instance as it was and let the
/// user retry once whatever holds the file has let go.
fn clear_stale(dest: &Path, target: Option<ProjectFileTarget>) -> Result<(), Error> {
    for path in owned_names(dest, target) {
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| {
                Error::Invalid(format!(
                    "cannot remove the superseded {}: {e}",
                    path.display()
                ))
            })?;
        }
    }
    Ok(())
}

/// Put the files the index lists into the instance, and report what a mod list
/// should say about them.
///
/// `previous` is what the instance already had, empty for a fresh install. A
/// file whose hash has not changed keeps the state it was in — so a mod the
/// user disabled stays disabled through an upgrade, and is left on disk under
/// its `.disabled` name rather than being downloaded back into place.
///
/// Alongside the mod-list entries it returns the instance-relative path of every
/// file the pack installed, which is what tells a later upgrade that a version
/// has dropped one. The mod list cannot answer that: it holds mods, and it keys
/// on bare file names, which two directories can share.
async fn sync_pack_files(
    index: &MrpackIndex,
    instance_path: &Path,
    previous: &HashMap<String, ModListEntry>,
    client: &reqwest::Client,
    report: impl Fn(usize, usize),
) -> Result<SyncedPack, Error> {
    let mut entries: Vec<ModListEntry> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    let mut planned: Vec<PlannedDownload> = Vec::new();

    // Planned first, so every file that will not be fetched is settled before
    // any is, and each task owns what it needs.
    for file in index.files.iter().filter(|file| file.wanted_by_client()) {
        let target = target_for(&file.path);
        // A file that cannot even be attempted is recorded the same way a failed
        // download is, so the Mods tab offers to link it rather than the install
        // reporting success with the file quietly absent.
        let mut record_failure = |reason: String| {
            warn!("{}: {reason}", file.path);
            if let Some(target) = target {
                entries.push(unfetched_entry(file, target));
            }
        };

        let Some(dest) = safe_destination(instance_path, &file.path) else {
            record_failure("unusable path".to_string());
            continue;
        };

        // Recorded here rather than after a successful fetch, so that no branch
        // below can drop it. A path missing from this list reads as "the new
        // version dropped that file" and gets deleted — which for an unchanged
        // disabled mod would mean deleting the very file being kept.
        //
        // A file this run could not fetch is recorded too: the user may supply
        // it by hand later, and it is the pack's file either way. Removing one
        // that never landed is a no-op.
        let Some(relative) = relative_path(instance_path, &dest) else {
            record_failure("unusable path".to_string());
            continue;
        };
        paths.push(relative.clone());

        if file.downloads.is_empty() {
            clear_stale(&dest, target)?;
            record_failure("no download URL".to_string());
            continue;
        }

        // `download_resource` treats an absent hash as "whatever is there is
        // fine" and skips the fetch, so a previous version's jar sitting at
        // this name would be kept and reported as the new one.
        //
        // Clearing it first is what makes the fetch happen. Refusing the file
        // instead leaves it on disk and missing from the mod list at once —
        // still loading, and impossible to remove or toggle, since the tab has
        // it recorded as absent. With no hash to compare, fetching what the
        // index names is the only answer that keeps disk and record agreeing.
        if file.hashes.sha1.is_empty() {
            clear_stale(&dest, target)?;
        }

        // The mod list keys on bare names, so an entry can only stand for a file
        // sitting directly in its own target's directory. Anything else sharing
        // that name — `config/common.jar` beside `mods/common.jar`, or a nested
        // `mods/sub/a.jar` — is a different file, and must not inherit its state
        // and be renamed out of the way.
        let installed = previous.get(&file_name_of(&dest)).filter(|entry| {
            relative == format!("{}/{}", entry.target.directory(), file_name_of(&dest))
        });
        // The toggle writes the `.disabled` name whether or not the mod has a
        // row in the list, so the file is the record for both. Asking only the
        // list loses the choice for a mod that arrived as an override and is
        // now named in the index: it would come back switched on, with the
        // disabled copy orphaned beside it.
        let was_disabled = disabled_path(&dest).exists();

        // A disabled mod lives under another name, so downloading `dest` would
        // reinstate the jar the user turned off and leave both copies on disk.
        // Only when the file has not changed: a new version of it still has to
        // be fetched, and is disabled again below.
        if was_disabled
            && installed.is_some_and(|entry| is_unchanged(entry, file))
            && disabled_path(&dest).exists()
        {
            entries.push(installed.expect("checked above").clone());
            continue;
        }

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        planned.push(PlannedDownload {
            path: file.path.clone(),
            dest,
            target,
            sha1: file.hashes.sha1.clone(),
            downloads: file.downloads.clone(),
            file_size: file.file_size,
            was_disabled,
        });
    }

    for (file, outcome) in run_downloads(planned, client, report).await? {
        let disabled = disabled_path(&file.dest);
        let (mod_state, written) = match outcome {
            // The user turned this mod off, so the version replacing it is off
            // too — the choice was about the mod, not about that build of it.
            Ok(()) if file.was_disabled => {
                std::fs::remove_file(&disabled).ok();
                match std::fs::rename(&file.dest, &disabled) {
                    Ok(()) => (ModState::Disabled, disabled.clone()),
                    Err(e) => {
                        warn!("cannot disable {}: {e}; leaving it enabled", file.path);
                        (ModState::Enabled, file.dest.clone())
                    }
                }
            }
            Ok(()) => (ModState::Enabled, file.dest.clone()),
            Err(e) => {
                warn!("download failed for {}: {e}; marking for manual install", file.path);
                clear_stale(&file.dest, file.target)?;
                (ModState::DownloadFailed, file.dest.clone())
            }
        };
        let Some(target) = file.target else { continue };
        // Only mods are tracked once installed; anything else is tracked solely
        // to drive the link prompt when it could not be fetched.
        if target.tracks_modlist() || mod_state == ModState::DownloadFailed {
            // The index states a size; fall back to the file itself when it does
            // not, so a mod never lists as 0 B once it is on disk.
            let size = match (file.file_size, mod_state) {
                (0, ModState::DownloadFailed) => 0,
                (0, _) => std::fs::metadata(&written).map(|m| m.len()).unwrap_or(0),
                (size, _) => size,
            };
            entries.push(ModListEntry {
                file_name: file_name_of(&file.dest),
                project_name: String::new(),
                icon_url: None,
                sha1: file.sha1,
                source: DownloadSource::Manual,
                target,
                size,
                state: mod_state,
            });
        }
    }
    Ok(SyncedPack { entries, paths })
}

/// What one pass over a pack's index put on disk.
struct SyncedPack {
    /// Mod-list rows: the mods, plus any file awaiting a hand-supplied copy.
    entries: Vec<ModListEntry>,
    /// Instance-relative path of every file installed, for the next upgrade.
    paths: Vec<String>,
}

/// The mod list read as pack paths, for an instance predating `pack_files.json`.
///
/// A mod list holds bare file names, so the directory is the one its target
/// names. That is exact for the mods it tracks, which live flat under `mods/`.
fn paths_from_modlist(previous: &HashMap<String, ModListEntry>) -> Vec<String> {
    previous
        .values()
        .map(|entry| format!("{}/{}", entry.target.directory(), entry.file_name))
        .collect()
}

/// Where the paths a pack installed are recorded.
fn pack_files_file(instance_path: &Path) -> PathBuf {
    instance_path.join(".launcher").join("pack_files.json")
}

/// A file's location as the pack list records it: instance-relative, with
/// forward slashes, so a record written on one platform reads on another.
fn relative_path(instance_path: &Path, dest: &Path) -> Option<String> {
    let relative = dest.strip_prefix(instance_path).ok()?;
    let joined = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    (!joined.is_empty()).then_some(joined)
}

/// How a path compares for "is this still shipped".
///
/// Windows and macOS both reach one file by either spelling by default, so two
/// cases are one file there and comparing them exactly would delete the file
/// just written. A case-sensitive filesystem is the opposite: treating them as
/// one leaves both spellings on disk, which the loader reads as a duplicate
/// mod. So the comparison follows the platform rather than picking a side.
///
/// The platform is a stand-in for the filesystem, which is what actually
/// decides; a case-sensitive volume on macOS, or a case-insensitive one on
/// Linux, is judged by its host's usual behaviour rather than its own.
fn path_key(relative: &str) -> String {
    if cfg!(any(windows, target_os = "macos")) {
        relative.to_lowercase()
    } else {
        relative.to_string()
    }
}

/// Delete a file a newer version of the pack no longer ships, under whichever
/// name it has — a disabled mod is stored with a `.disabled` suffix.
///
/// `shipped` is what this version installs, keyed by [`path_key`]. A pack may
/// legitimately ship a file whose name ends in `.disabled`; that file is its
/// own, not the disabled twin of the one being removed, so an alias the new
/// version ships is left alone.
///
/// The path came from our own record, but it is resolved through the same guard
/// as one from an index: a record that has been edited by hand cannot direct a
/// delete outside the instance.
/// The record-relative spelling of `path`, given that `dest` is `relative`.
/// Only the file name can differ between them, which is what the `.disabled`
/// alias changes.
fn relative_path_of(dest: &Path, path: &Path, relative: &str) -> Option<String> {
    let dir = relative.rsplit_once('/').map(|(dir, _)| dir);
    let name = path.file_name()?.to_string_lossy();
    if path == dest {
        return Some(relative.to_string());
    }
    Some(match dir {
        Some(dir) => format!("{dir}/{name}"),
        None => name.into_owned(),
    })
}

/// Returns whether the path should stay in the record — true when the file may
/// still be there and a later upgrade ought to try again. A path pointing
/// outside the instance is not the pack's to begin with, so it is dropped
/// rather than retried forever.
fn remove_pack_file(instance_path: &Path, relative: &str, shipped: &HashSet<String>) -> bool {
    let Some(dest) = safe_destination(instance_path, relative) else {
        warn!("refusing to remove '{relative}': not inside the instance");
        return false;
    };

    // A path the new version ships is its own file, not the disabled twin of
    // the one being dropped, so it is never taken as an alias.
    let targets = owned_names(&dest, target_for(relative))
        .into_iter()
        .filter(|path| match relative_path_of(&dest, path, relative) {
            Some(candidate) => !shipped.contains(&path_key(&candidate)),
            None => true,
        });
    let mut retry = false;
    for path in targets {
        if path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                warn!("failed to remove dropped file {}: {e}", path.display());
                retry = true;
            }
        }
    }
    retry
}

/// The pack's own blurb where it ships one; its name is all a pack without a
/// summary offers.
fn pack_description(index: &MrpackIndex) -> String {
    if index.summary.is_empty() {
        index.name.clone()
    } else {
        index.summary.clone()
    }
}

/// Install a `.mrpack` already on disk into `instance_path`. `origin` is what
/// the instance records as its provenance: `Manual` for a zip the user picked,
/// or the Modrinth version it was fetched from.
#[allow(clippy::too_many_arguments)]
async fn install_into(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    zip_path: &Path,
    origin: DownloadSource,
    instance_path: &Path,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let mut prepared = prepare_pack(app_handle, id, instance_name, zip_path, state).await?;

    emit_progress(app_handle, id, instance_name, "Downloading mods", false, None);
    let synced = sync_pack_files(
        &prepared.index,
        instance_path,
        &HashMap::new(),
        &state.http_client,
        |done, total| {
            emit_progress(
                app_handle,
                id,
                instance_name,
                &format!("Downloading mods ({done}/{total})"),
                false,
                None,
            );
        },
    )
    .await?;
    let mut modlist_entries = synced.entries;

    // After the downloads, not before: the format has overrides win over what a
    // pack lists, and download_resource replaces a file whose hash does not
    // match — so extracting first would let the stock jar overwrite a patched
    // one the pack deliberately ships.
    emit_progress(app_handle, id, instance_name, "Extracting files", false, None);
    let mut installed_paths = synced.paths;
    for path in override_paths(&mut prepared.archive, instance_path)? {
        if !installed_paths.contains(&path) {
            installed_paths.push(path);
        }
    }
    extract_overrides(&mut prepared.archive, instance_path)?;

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    name_entries_by_project(&mut modlist_entries, &state.http_client).await;
    upsert_modlist_entries(instance_path, modlist_entries)?;
    write_json(pack_files_file(instance_path), &installed_paths)?;

    let meta = InstanceMeta {
        id: id.to_string(),
        name: instance_name.to_string(),
        description: pack_description(&prepared.index),
        game_version: prepared.mc_version.clone(),
        mod_loader: prepared.mod_loader.clone(),
        mod_loader_version: prepared.loader_version,
        version_id: prepared.version_id,
        category,
        origin,
        ..InstanceMeta::default()
    };
    write_json(instance_meta_file(instance_path), &meta)?;

    info!(
        "Installed '{}' from {} (MC {}, {})",
        instance_name,
        zip_path.display(),
        prepared.mc_version,
        prepared.mod_loader
    );
    Ok(())
}

/// Install a modpack named by a Modrinth version id.
///
/// The version's `.mrpack` is staged in the instance's cache directory and then
/// installed by the same code that handles a pack the user picked off disk — the
/// file is identical either way. What differs is the origin recorded: an install
/// from here knows the project and version it came from.
pub async fn install_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    source: DownloadSource,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let DownloadSource::Modrinth { version_id, .. } = &source else {
        return Err(Error::Unsupported(
            "not a Modrinth modpack source".to_string(),
        ));
    };
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let instance_path = create_instance_dir(&install_dir, instance_name)?;
    // Nothing usable exists until the instance record is written, so clear the
    // directory on failure rather than leaving one the library never shows and
    // that blocks retrying the same name.
    let result = install_by_version(
        app_handle,
        id,
        instance_name,
        category,
        &source,
        version_id,
        &instance_path,
        state,
    )
    .await;
    if result.is_err() {
        discard_unfinished_instance_dir(&instance_path);
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn install_by_version(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    source: &DownloadSource,
    version_id: &str,
    instance_path: &Path,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    emit_progress(app_handle, id, instance_name, "Downloading modpack", false, None);
    let cache_path = stage_version(version_id, instance_path, &state.http_client).await?;

    let installed = install_into(
        app_handle,
        id,
        instance_name,
        category,
        &cache_path,
        source.clone(),
        instance_path,
        state,
    )
    .await;

    // Always drop the staged zip: its overrides are in the instance now and its
    // files were fetched from the URLs the index carries.
    std::fs::remove_file(&cache_path).ok();
    installed
}

/// Upgrade an instance to another version of the Modrinth pack it came from.
///
/// The new `.mrpack` is staged and installed over the instance: files it still
/// ships are downloaded (unchanged ones are skipped by their hash), files it no
/// longer ships are deleted, and the user's saves, configs and screenshots are
/// left untouched.
pub async fn upgrade_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    instance_path: PathBuf,
    source: DownloadSource,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let DownloadSource::Modrinth { version_id, .. } = &source else {
        return Err(Error::Unsupported(
            "not a Modrinth modpack source".to_string(),
        ));
    };

    emit_progress(app_handle, id, instance_name, "Downloading modpack", false, None);
    let cache_path = stage_version(version_id, &instance_path, &state.http_client).await?;
    let upgraded = upgrade_into(
        app_handle,
        id,
        instance_name,
        &instance_path,
        &cache_path,
        &source,
        state,
    )
    .await;
    std::fs::remove_file(&cache_path).ok();
    upgraded
}

#[allow(clippy::too_many_arguments)]
async fn upgrade_into(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    instance_path: &Path,
    zip_path: &Path,
    source: &DownloadSource,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let mut prepared = prepare_pack(app_handle, id, instance_name, zip_path, state).await?;

    // Read before anything is written: this is the record of what the instance
    // had, and it is only replaced once the new files are on disk.
    let previous = installed_entries(instance_path);

    // What the previous version put on disk. Absence of the record is the test,
    // not emptiness: a pack that ships nothing a client wants writes `[]`, and
    // reading that as "no record" would fall back to the mod list and delete
    // mods the user added themselves.
    let record = pack_files_file(instance_path);
    let installed_before: Vec<String> = if record.exists() {
        // A record that cannot be parsed is not an empty one. Reading it as
        // empty would skip every deletion and then overwrite the file, orphaning
        // everything it named; refusing leaves the instance as it was.
        read_json(record)?
    } else {
        // An instance installed before this record existed has none. Its mod
        // list is the only account of what the old pack put there, and this is
        // the last moment it exists — the list is replaced below. Falling back
        // to it removes the mods a new version drops; anything the list never
        // tracked stays, which is what the old behaviour did with everything.
        paths_from_modlist(&previous)
    };

    emit_progress(app_handle, id, instance_name, "Updating mods", false, None);
    let synced = sync_pack_files(
        &prepared.index,
        instance_path,
        &previous,
        &state.http_client,
        |done, total| {
            emit_progress(
                app_handle,
                id,
                instance_name,
                &format!("Updating mods ({done}/{total})"),
                false,
                None,
            );
        },
    )
    .await?;
    let mut modlist_entries = synced.entries;

    // Only now that the new files are down: a failure before this point leaves
    // the old instance intact rather than stripped of mods it still lists.
    // Read before the deletion pass: a file this version ships as an override
    // is one it still ships, so it must not read as dropped and be removed
    // only to be written again moments later. Kept for the pass after the
    // extraction too, rather than walking the archive again.
    let shipped_overrides = override_paths(&mut prepared.archive, instance_path)?;
    let mut installed_paths = synced.paths.clone();
    for path in &shipped_overrides {
        if !installed_paths.contains(path) {
            installed_paths.push(path.clone());
        }
    }

    let shipped: HashSet<String> = installed_paths.iter().map(|path| path_key(path)).collect();
    // A file that could not be deleted — locked by another process, say — is
    // still there, so its path stays in the record. Dropping it would leave the
    // file loading against the upgraded pack with nothing left that knows to
    // try again.
    let mut kept_paths = installed_paths.clone();
    for path in &installed_before {
        if shipped.contains(&path_key(path)) {
            continue;
        }
        if remove_pack_file(instance_path, path, &shipped) {
            kept_paths.push(path.clone());
        }
    }

    emit_progress(app_handle, id, instance_name, "Extracting files", false, None);
    extract_overrides(&mut prepared.archive, instance_path)?;

    // An override is written under its plain name, so a mod that arrives this
    // way arrives switched on. Only mods: `.disabled` is how the launcher
    // toggles those and nothing else, and a `theme.zip.disabled` beside a
    // resource pack is the user's own backup.
    //
    // The `.disabled` name is the whole record of the choice, not the mod list:
    // a mod that only ever arrived as an override has no row there, and
    // `toggle_state_instance_mod` does not add one. So the file's existence is
    // what says the user turned this mod off — read it, rather than a list that
    // was never asked.
    for path in shipped_overrides.iter().filter(|path| target_for(path) == Some(ProjectFileTarget::Mod)) {
        // A pack may ship `foo.jar` and `foo.jar.disabled` side by side, as one
        // does to offer an alternate build. That second file is the pack's own,
        // not a record of the user turning the first off, and renaming over it
        // would destroy it and switch off a mod meant to be on.
        if shipped.contains(&path_key(&format!("{path}.disabled"))) {
            continue;
        }
        let Some(dest) = safe_destination(instance_path, path) else { continue };
        let disabled = disabled_path(&dest);
        if !disabled.exists() {
            continue;
        }
        // The choice was about the mod, not about how the pack decided to ship
        // it this time. The copy just extracted takes the disabled name.
        std::fs::remove_file(&disabled).ok();
        if let Err(e) = std::fs::rename(&dest, &disabled) {
            warn!("cannot disable {}: {e}; leaving it enabled", dest.display());
        }
    }

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    name_entries_by_project(&mut modlist_entries, &state.http_client).await;
    // The new pack defines the mod set outright, so the list is replaced rather
    // than merged — a merge would keep rows for jars just deleted.
    replace_modlist_entries(instance_path, modlist_entries)?;
    write_json(pack_files_file(instance_path), &kept_paths)?;

    let mut meta: InstanceMeta = read_json(instance_meta_file(instance_path))?;
    meta.description = pack_description(&prepared.index);
    meta.game_version = prepared.mc_version.clone();
    meta.mod_loader = prepared.mod_loader.clone();
    meta.mod_loader_version = prepared.loader_version;
    meta.version_id = prepared.version_id;
    meta.origin = source.clone();
    write_json(instance_meta_file(instance_path), &meta)?;

    info!(
        "Upgraded '{}' (MC {}, {})",
        instance_name, prepared.mc_version, prepared.mod_loader
    );
    Ok(())
}

/// Download a version's `.mrpack` into the instance's cache directory.
async fn stage_version(
    version_id: &str,
    instance_path: &Path,
    client: &reqwest::Client,
) -> Result<PathBuf, Error> {
    let url = format!("https://api.modrinth.com/v2/version/{version_id}");
    let version = fetch_json(client, &url).send::<Version>().await?;
    let file = selected_file(&version).ok_or_else(|| {
        Error::NotExists(format!("Modrinth version {version_id} lists no file"))
    })?;

    // The name comes from the API, not from us; refuse one that would stage the
    // zip outside the instance's cache directory.
    if !is_bare_file_name(&file.filename) {
        return Err(Error::Invalid(format!(
            "Modrinth version {version_id} names its file '{}'",
            file.filename
        )));
    }
    let cache_path = instance_path
        .join(ProjectFileTarget::Modpack.directory())
        .join(&file.filename);
    download_resource(client, &file.url, &file.hashes.sha1, cache_path.clone()).await?;
    Ok(cache_path)
}


#[cfg(test)]
mod upgrade_tests {
    use super::super::mrpack::{MrpackFile, MrpackHashes};
    use super::{disabled_path, is_unchanged, remove_pack_file};
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use yaminabe_launcher_shared::datamodels::{
        DownloadSource, ModListEntry, ModState, ProjectFileTarget,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yaminabe-upgrade-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("mods")).expect("create mods dir");
        dir
    }

    fn entry(file_name: &str, sha1: &str, state: ModState) -> ModListEntry {
        ModListEntry {
            file_name: file_name.to_string(),
            project_name: String::new(),
            icon_url: None,
            sha1: sha1.to_string(),
            source: DownloadSource::Manual,
            target: ProjectFileTarget::Mod,
            size: 0,
            state,
        }
    }

    fn index_with(file: MrpackFile) -> super::MrpackIndex {
        super::MrpackIndex {
            name: String::new(),
            version_id: String::new(),
            summary: String::new(),
            files: vec![file],
            dependencies: HashMap::new(),
        }
    }

    fn indexed(sha1: &str) -> MrpackFile {
        MrpackFile {
            path: "mods/a.jar".to_string(),
            hashes: MrpackHashes { sha1: sha1.to_string() },
            env: None,
            downloads: vec![],
            file_size: 0,
        }
    }

    #[test]
    fn a_file_is_unchanged_only_when_its_hash_matches() {
        let installed = entry("a.jar", "ABC123", ModState::Enabled);
        assert!(is_unchanged(&installed, &indexed("abc123")));
        assert!(!is_unchanged(&installed, &indexed("def456")));

        // Nothing to compare against is not a match: re-fetching is the safe
        // reading, since the alternative is keeping a file that may differ.
        let unhashed = entry("a.jar", "", ModState::Enabled);
        assert!(!is_unchanged(&unhashed, &indexed("")));
    }

    #[test]
    fn a_disabled_mod_is_named_for_where_it_actually_sits() {
        assert_eq!(
            disabled_path(Path::new("/i/mods/a.jar")),
            PathBuf::from("/i/mods/a.jar.disabled")
        );
    }

    /// A dropped mod the user had disabled is on disk under its `.disabled`
    /// name, so deleting only the plain name would leave it loading against the
    /// upgraded pack.
    #[test]
    fn dropping_a_disabled_mod_removes_the_file_it_really_has() {
        let dir = temp_dir("disabled");
        let disabled = dir.join("mods").join("old.jar.disabled");
        std::fs::write(&disabled, b"jar").expect("write disabled jar");

        assert!(!remove_pack_file(&dir, "mods/old.jar", &HashSet::new()));
        assert!(!disabled.exists(), "the .disabled file should be gone");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A mirror failure leaves the previous jar at that name on disk.
    /// `download_resource` only replaces a file once it has verified bytes, so
    /// the upgrade has to clear it: the pack no longer ships that build, and
    /// the mod list already calls the file missing.
    #[tokio::test]
    async fn a_failed_replacement_does_not_leave_the_old_jar_behind() {
        let dir = temp_dir("failed-replacement");
        let jar = dir.join("mods").join("a.jar");
        std::fs::write(&jar, b"the previous version").expect("write old jar");

        // No mirrors reachable: the index points at a host that cannot serve it.
        let index = index_with(MrpackFile {
            path: "mods/a.jar".to_string(),
            hashes: MrpackHashes { sha1: "0000000000000000000000000000000000000000".to_string() },
            env: None,
            downloads: vec!["http://127.0.0.1:1/nope.jar".to_string()],
            file_size: 0,
        });
        let previous = HashMap::from([(
            "a.jar".to_string(),
            entry("a.jar", "aaaa", ModState::Enabled),
        )]);

        let synced = super::sync_pack_files(&index, &dir, &previous, &reqwest::Client::new(), |_, _| {})
            .await
            .expect("sync");

        assert!(!jar.exists(), "the superseded jar must not survive a failed fetch");
        assert_eq!(synced.entries.len(), 1);
        assert_eq!(synced.entries[0].state, ModState::DownloadFailed);
        // Recorded even though the fetch failed: the file is the pack's, and the
        // user may supply it by hand before the next upgrade.
        assert_eq!(synced.paths, vec!["mods/a.jar".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Disabling a mod is a choice about the mod, not about one build of it, so
    /// a new version arrives disabled too rather than switching itself back on.
    #[tokio::test]
    async fn a_changed_mod_the_user_disabled_stays_disabled() {
        let dir = temp_dir("changed-disabled");
        let disabled = dir.join("mods").join("a.jar.disabled");
        std::fs::write(&disabled, b"the previous version").expect("write disabled jar");

        let index = index_with(MrpackFile {
            path: "mods/a.jar".to_string(),
            hashes: MrpackHashes { sha1: "a1b2c3".to_string() },
            env: None,
            downloads: vec![String::new()],
            file_size: 0,
        });
        // Recorded under a different hash, so this is a new build of it.
        let previous = HashMap::from([(
            "a.jar".to_string(),
            entry("a.jar", "0ld0ld", ModState::Disabled),
        )]);

        let synced = super::sync_pack_files(&index, &dir, &previous, &reqwest::Client::new(), |_, _| {})
            .await
            .expect("sync");

        // The fetch cannot succeed here, so what matters is that the stale
        // disabled copy is gone rather than left beside a re-enabled jar.
        assert!(!disabled.exists(), "the superseded disabled copy must not survive");
        assert!(!dir.join("mods").join("a.jar").exists(), "and it must not come back enabled");
        assert_eq!(synced.entries.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// An unchanged disabled mod is kept without being fetched, and its path
    /// still has to reach the record: a path missing from it reads as dropped,
    /// so the upgrade would delete the very file it just decided to keep.
    #[tokio::test]
    async fn a_kept_disabled_mod_is_still_recorded_as_installed() {
        let dir = temp_dir("kept-disabled");
        let disabled = dir.join("mods").join("a.jar.disabled");
        std::fs::write(&disabled, b"jar").expect("write disabled jar");

        let index = index_with(MrpackFile {
            path: "mods/a.jar".to_string(),
            hashes: MrpackHashes { sha1: "abc123".to_string() },
            env: None,
            downloads: vec!["http://127.0.0.1:1/nope.jar".to_string()],
            file_size: 0,
        });
        let previous = HashMap::from([(
            "a.jar".to_string(),
            entry("a.jar", "abc123", ModState::Disabled),
        )]);

        let synced = super::sync_pack_files(&index, &dir, &previous, &reqwest::Client::new(), |_, _| {})
            .await
            .expect("sync");

        assert!(disabled.exists(), "the kept file must be left where it is");
        assert_eq!(synced.entries.len(), 1);
        assert_eq!(synced.entries[0].state, ModState::Disabled);
        assert_eq!(synced.paths, vec!["mods/a.jar".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two directories can hold the same file name — a pack shipping a matching
    /// resource pack and data pack is the ordinary case. The record keeps the
    /// whole path, so dropping one does not read as dropping the other.
    #[test]
    fn a_name_two_directories_share_is_recorded_once_per_directory() {
        let dir = temp_dir("same-name");
        assert_eq!(
            super::relative_path(&dir, &dir.join("resourcepacks").join("extras.zip")),
            Some("resourcepacks/extras.zip".to_string())
        );
        assert_eq!(
            super::relative_path(&dir, &dir.join("datapacks").join("extras.zip")),
            Some("datapacks/extras.zip".to_string())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Removal follows the recorded path rather than rebuilding one from the
    /// target directory, so a file the pack nests is still found.
    #[test]
    fn a_dropped_file_is_removed_from_where_the_pack_put_it() {
        let dir = temp_dir("nested");
        let nested = dir.join("mods").join("sub");
        std::fs::create_dir_all(&nested).expect("create nested dir");
        let jar = nested.join("a.jar");
        std::fs::write(&jar, b"jar").expect("write nested jar");

        assert!(!super::remove_pack_file(&dir, "mods/sub/a.jar", &HashSet::new()));
        assert!(!jar.exists(), "a nested file must be removed where it actually is");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `<name>.disabled` is a legal path for a pack to ship. When the new
    /// version ships it and the old one shipped `<name>`, removing the old file
    /// must not take the new one with it as though it were a disabled twin.
    #[test]
    fn removal_keeps_a_disabled_suffix_path_the_new_version_ships() {
        let dir = temp_dir("alias");
        let shipped_file = dir.join("mods").join("a.jar.disabled");
        std::fs::write(&shipped_file, b"the new pack's own file").expect("write shipped file");

        let shipped = HashSet::from([super::path_key("mods/a.jar.disabled")]);
        assert!(!super::remove_pack_file(&dir, "mods/a.jar", &shipped));

        assert!(shipped_file.exists(), "a path the new version ships is not an alias to delete");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Without a hash there is nothing to check bytes against, and
    /// `download_resource` reads an absent hash as "whatever is there will do"
    /// — so the previous version's jar would pass and be reported as the new
    /// one. It has to be refused before it gets that far.
    #[tokio::test]
    async fn a_file_with_no_hash_is_not_taken_on_trust() {
        let dir = temp_dir("no-hash");
        let jar = dir.join("mods").join("a.jar");
        std::fs::write(&jar, b"the previous version").expect("write old jar");

        let index = index_with(MrpackFile {
            path: "mods/a.jar".to_string(),
            hashes: MrpackHashes { sha1: String::new() },
            env: None,
            downloads: vec!["http://127.0.0.1:1/nope.jar".to_string()],
            file_size: 0,
        });

        let synced = super::sync_pack_files(&index, &dir, &HashMap::new(), &reqwest::Client::new(), |_, _| {})
            .await
            .expect("sync");

        // Never adopted: whatever the outcome, the previous version's jar is
        // not reported as the new one.
        assert_eq!(synced.entries.len(), 1);
        assert_eq!(synced.entries[0].state, ModState::DownloadFailed);
        // Cleared before the fetch, because `download_resource` skips a file
        // that is already there when it has no hash to judge it by. The fetch
        // then fails here, so nothing replaces it — and disk agrees with the
        // mod list, which says the file is missing.
        assert!(!jar.exists(), "the superseded jar must not be left behind");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A pack listing a file it supplies no URL for is the same situation as one
    /// whose every mirror failed: the name is claimed, nothing was written, and
    /// the previous version's copy must not be left to load in its place.
    #[tokio::test]
    async fn a_file_with_no_url_does_not_leave_the_old_one_behind() {
        let dir = temp_dir("no-url");
        let jar = dir.join("mods").join("a.jar");
        std::fs::write(&jar, b"the previous version").expect("write old jar");

        let index = index_with(MrpackFile {
            path: "mods/a.jar".to_string(),
            hashes: MrpackHashes { sha1: "a1b2c3".to_string() },
            env: None,
            downloads: vec![],
            file_size: 0,
        });

        let synced = super::sync_pack_files(&index, &dir, &HashMap::new(), &reqwest::Client::new(), |_, _| {})
            .await
            .expect("sync");

        assert!(!jar.exists(), "the superseded jar must not survive");
        assert_eq!(synced.paths, vec!["mods/a.jar".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The launcher only toggles mods, so only there does `.disabled` name a
    /// file the pack owns. A backup the user made beside a config file is
    /// theirs, whatever it is called.
    #[test]
    fn removal_leaves_a_disabled_suffix_outside_mods_alone() {
        let dir = temp_dir("alias-nonmod");
        std::fs::create_dir_all(dir.join("resourcepacks")).expect("create dir");
        let backup = dir.join("resourcepacks").join("theme.zip.disabled");
        std::fs::write(&backup, b"the user's own copy").expect("write backup");

        assert!(!super::remove_pack_file(&dir, "resourcepacks/theme.zip", &HashSet::new()));
        assert!(backup.exists(), "only mods use the .disabled suffix");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A nested mod shares a name with a flat one the mod list knows about. The
    /// list holds bare names, so it can only speak for the flat file; inheriting
    /// its disabled state would rename a mod the pack means to be on.
    #[tokio::test]
    async fn a_nested_mod_does_not_inherit_a_flat_one_s_state() {
        let dir = temp_dir("nested-state");
        let index = index_with(MrpackFile {
            path: "mods/sub/a.jar".to_string(),
            hashes: MrpackHashes { sha1: "a1b2c3".to_string() },
            env: None,
            downloads: vec!["http://127.0.0.1:1/nope.jar".to_string()],
            file_size: 0,
        });
        let previous = HashMap::from([(
            "a.jar".to_string(),
            entry("a.jar", "aaa", ModState::Disabled),
        )]);

        let synced = super::sync_pack_files(&index, &dir, &previous, &reqwest::Client::new(), |_, _| {})
            .await
            .expect("sync");

        assert_eq!(synced.paths, vec!["mods/sub/a.jar".to_string()]);
        assert_ne!(synced.entries[0].state, ModState::Disabled);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// An instance from before the record exists still has a mod list, and
    /// dropping back to it beats never removing anything again — the list is
    /// overwritten moments later, so this is the last chance to read it.
    #[test]
    fn a_legacy_instance_falls_back_to_its_mod_list() {
        let previous = HashMap::from([
            ("a.jar".to_string(), entry("a.jar", "aaa", ModState::Enabled)),
        ]);
        assert_eq!(
            super::paths_from_modlist(&previous),
            vec!["mods/a.jar".to_string()]
        );
    }

    /// Windows reaches one file by either spelling and Linux does not, so the
    /// comparison has to follow the platform: one way deletes the file just
    /// written, the other leaves both spellings for the loader to trip over.
    #[test]
    fn path_comparison_follows_the_filesystem() {
        if cfg!(any(windows, target_os = "macos")) {
            assert_eq!(super::path_key("mods/Foo.jar"), super::path_key("mods/foo.jar"));
        } else {
            assert_ne!(super::path_key("mods/Foo.jar"), super::path_key("mods/foo.jar"));
        }
    }

    /// The record is ours, but it is a file on disk that could be edited. A path
    /// climbing out of the instance is refused rather than followed.
    #[test]
    fn removal_refuses_a_path_outside_the_instance() {
        let dir = temp_dir("escape");
        let outside = dir.parent().expect("temp parent").join("yaminabe-not-mine.jar");
        std::fs::write(&outside, b"someone else's file").expect("write outside file");

        assert!(!super::remove_pack_file(&dir, "../yaminabe-not-mine.jar", &HashSet::new()),
            "a path that is not the pack's is dropped, not retried forever");
        assert!(outside.exists(), "a path climbing out must not be followed");
        std::fs::remove_file(&outside).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dropping_an_enabled_mod_removes_its_jar() {
        let dir = temp_dir("enabled");
        let jar = dir.join("mods").join("old.jar");
        std::fs::write(&jar, b"jar").expect("write jar");

        assert!(!remove_pack_file(&dir, "mods/old.jar", &HashSet::new()));
        assert!(!jar.exists(), "the jar should be gone");
        std::fs::remove_dir_all(&dir).ok();
    }
}
