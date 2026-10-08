use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::super::pack_files::{
    align_states, installed_entries, path_key, recorded_entry, safe_destination, target_for,
    toggle_twin, ModStates,
};
use super::api::{name_entries_by_project, selected_file, ResolvedFiles, Version};
use super::mrpack::{read_index, resolve_loader, MrpackFile, MrpackIndex, INDEX_NAME};
use crate::commands::instance::{
    create_instance_dir, discard_unfinished_instance_dir, instance_meta_file, is_bare_file_name,
    replace_modlist_entries, upsert_modlist_entries,
};
use crate::emit_progress;
use crate::http_utils::{download_resource, fetch_json};
use crate::install_task::ensure_game_and_loader;
use crate::json::{read_json, write_json};
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
    /// Where it goes, spelled as [`claimed_paths`] spells it.
    relative: String,
    dest: PathBuf,
    /// `None` when the mod list does not model where this file goes.
    target: Option<ProjectFileTarget>,
    sha1: String,
    downloads: Vec<String>,
    file_size: u64,
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

/// Whether `previous` is the same file the index now lists. Compared by hash,
/// not by name: a pack can ship a different jar under a name it used before.
fn is_unchanged(previous: &ModListEntry, file: &MrpackFile) -> bool {
    !previous.sha1.is_empty() && previous.sha1.eq_ignore_ascii_case(&file.hashes.sha1)
}

/// What this version installs through its index or its overrides, keyed by
/// [`path_key`]. The index is resolved as [`sync_pack_files`] resolves each
/// path, so the spellings compare.
fn claimed_paths(index: &MrpackIndex, instance_path: &Path, overrides: &[String]) -> HashSet<String> {
    index
        .files
        .iter()
        .filter(|file| file.wanted_by_client())
        .filter_map(|file| safe_destination(instance_path, &file.path))
        .filter_map(|dest| relative_path(instance_path, &dest))
        .chain(overrides.iter().cloned())
        .map(|relative| path_key(&relative))
        .collect()
}

/// The names a pack owns at `dest`: the file itself, and the twin it is
/// toggled with where it has one.
fn owned_names(
    dest: &Path,
    relative: &str,
    target: Option<ProjectFileTarget>,
    claimed: &HashSet<String>,
) -> Vec<PathBuf> {
    std::iter::once(dest.to_path_buf())
        .chain(toggle_twin(dest, relative, target, claimed))
        .collect()
}

/// Remove what the previous version left at a path this run claims but could
/// not write. Left there it loads against the upgraded pack while the mod list
/// reports the file missing, so disk and record would disagree.
///
/// A failure here stops the upgrade. Reporting the file missing while it is
/// still on disk and still loading is the mismatch this exists to prevent, so
/// there is nothing useful to do but leave the instance as it was and let the
/// user retry once whatever holds the file has let go.
fn clear_stale(
    dest: &Path,
    relative: &str,
    target: Option<ProjectFileTarget>,
    claimed: &HashSet<String>,
) -> Result<(), Error> {
    for path in owned_names(dest, relative, target, claimed) {
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

/// Put the files the index lists into the instance under their plain names,
/// and report what a mod list should say about them. Turning mods off is left
/// to [`align_states`], once the old version is gone — except that a mod
/// `states` has off whose hash has not changed is kept where it sits, under its
/// `.disabled` name, rather than fetched again beside it.
///
/// It also returns the path of every file the pack installed, which is what
/// tells a later upgrade that a version has dropped one: the mod list holds
/// only mods, keyed on bare names that two directories can share.
async fn sync_pack_files(
    index: &MrpackIndex,
    instance_path: &Path,
    previous: &HashMap<String, ModListEntry>,
    states: &ModStates,
    claimed: &HashSet<String>,
    client: &reqwest::Client,
    report: impl Fn(usize, usize),
) -> Result<SyncedPack, Error> {
    let mut entries: Vec<(Option<String>, ModListEntry)> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    let mut kept: Vec<String> = Vec::new();
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
                entries.push((None, unfetched_entry(file, target)));
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
            clear_stale(&dest, &relative, target, claimed)?;
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
            clear_stale(&dest, &relative, target, claimed)?;
        }

        let installed = recorded_entry(previous, &relative);
        // A mod the user turned off sits under its twin, so fetching `dest` would
        // put a second copy beside it. Unchanged, it stays where it is, and is
        // recorded off because disk says so, whatever the row last claimed.
        let unchanged_off = installed.filter(|entry| {
            is_unchanged(entry, file)
                && states.was_disabled(&relative, None)
                && toggle_twin(&dest, &relative, target, claimed).is_some()
        });
        if let Some(entry) = unchanged_off {
            kept.push(format!("{relative}.disabled"));
            let entry = ModListEntry { state: ModState::Disabled, ..entry.clone() };
            entries.push((Some(relative), entry));
            continue;
        }

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        planned.push(PlannedDownload {
            path: file.path.clone(),
            relative,
            dest,
            target,
            sha1: file.hashes.sha1.clone(),
            downloads: file.downloads.clone(),
            file_size: file.file_size,
        });
    }

    for (file, outcome) in run_downloads(planned, client, report).await? {
        let mod_state = match outcome {
            Ok(()) => ModState::Enabled,
            Err(e) => {
                warn!("download failed for {}: {e}; marking for manual install", file.path);
                clear_stale(&file.dest, &file.relative, file.target, claimed)?;
                ModState::DownloadFailed
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
                (0, _) => std::fs::metadata(&file.dest).map(|m| m.len()).unwrap_or(0),
                (size, _) => size,
            };
            let entry = ModListEntry {
                file_name: file_name_of(&file.dest),
                project_name: String::new(),
                icon_url: None,
                sha1: file.sha1,
                source: DownloadSource::Manual,
                target,
                size,
                state: mod_state,
            };
            entries.push((Some(file.relative), entry));
        }
    }
    Ok(SyncedPack { entries, paths, kept })
}

/// What one pass over a pack's index put on disk.
struct SyncedPack {
    /// Mod-list rows — the mods, plus any file awaiting a hand-supplied copy —
    /// each with the path its file was put at, `None` for one never placed.
    entries: Vec<(Option<String>, ModListEntry)>,
    /// Instance-relative path of every file installed, for the next upgrade.
    paths: Vec<String>,
    /// The `.disabled` paths of unchanged mods kept where they sat, which no
    /// clean-up may take for a file the old version dropped.
    kept: Vec<String>,
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

/// Delete a file a newer version of the pack no longer ships, under whichever
/// of its [`owned_names`] it has — a disabled mod sits under its twin, unless
/// `claimed` says the pack put a file of its own at that name.
///
/// The path came from our own record, but it is resolved through the same guard
/// as one from an index: a record that has been edited by hand cannot direct a
/// delete outside the instance.
///
/// Returns whether the path stays in the record: true when the file may still
/// be there for a later upgrade to retry, false for one outside the instance.
fn remove_pack_file(instance_path: &Path, relative: &str, claimed: &HashSet<String>) -> bool {
    let Some(dest) = safe_destination(instance_path, relative) else {
        warn!("refusing to remove '{relative}': not inside the instance");
        return false;
    };

    let mut retry = false;
    for path in owned_names(&dest, relative, target_for(relative), claimed) {
        if path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                warn!("failed to remove dropped file {}: {e}", path.display());
                retry = true;
            }
        }
    }
    retry
}

/// Delete what the version being replaced installed and this one does not, and
/// return the record to write: `installed_paths`, plus any dropped path whose
/// file could not be removed — locked by another process, say — so that a later
/// upgrade tries again rather than leaving it to load against the new pack.
///
/// A path in `kept` is left alone even where the old version shipped a file of
/// its own: an unchanged mod the user turned off is being kept under it, and
/// that mod's own path answers for it in the record from here on.
fn remove_dropped(
    instance_path: &Path,
    installed_before: &[String],
    installed_paths: Vec<String>,
    claimed: &HashSet<String>,
    kept: &[String],
) -> Vec<String> {
    let shipped: HashSet<String> = installed_paths.iter().map(|path| path_key(path)).collect();
    let held: HashSet<String> = kept.iter().map(|path| path_key(path)).collect();
    let mut kept_paths = installed_paths;
    for path in installed_before {
        let key = path_key(path);
        if shipped.contains(&key) || held.contains(&key) {
            continue;
        }
        if remove_pack_file(instance_path, path, claimed) {
            kept_paths.push(path.clone());
        }
    }
    kept_paths
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

    // Read before the downloads, though extracted well after them: what the
    // overrides ship is part of what this version claims, which is what tells a
    // `.disabled` file of the pack's own from a mod's twin.
    let shipped_overrides = override_paths(&mut prepared.archive, instance_path)?;
    let claimed = claimed_paths(&prepared.index, instance_path, &shipped_overrides);

    emit_progress(app_handle, id, instance_name, "Downloading mods", false, None);
    let synced = sync_pack_files(
        &prepared.index,
        instance_path,
        &HashMap::new(),
        &ModStates::default(),
        &claimed,
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
    let mut modlist_entries: Vec<ModListEntry> =
        synced.entries.into_iter().map(|(_, entry)| entry).collect();

    // After the downloads, not before: the format has overrides win over what a
    // pack lists, and download_resource replaces a file whose hash does not
    // match — so extracting first would let the stock jar overwrite a patched
    // one the pack deliberately ships.
    emit_progress(app_handle, id, instance_name, "Extracting files", false, None);
    let mut installed_paths = synced.paths;
    for path in shipped_overrides {
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

    // Read before the downloads: what the overrides ship is part of what this
    // version claims, which the downloads, the clean-up and the states consult.
    let shipped_overrides = override_paths(&mut prepared.archive, instance_path)?;
    let claimed = claimed_paths(&prepared.index, instance_path, &shipped_overrides);
    let states = ModStates::read(instance_path, &previous, &installed_before);

    emit_progress(app_handle, id, instance_name, "Updating mods", false, None);
    let synced = sync_pack_files(
        &prepared.index,
        instance_path,
        &previous,
        &states,
        &claimed,
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

    // Only now that the new files are down: a failure before this point leaves
    // the old instance intact rather than stripped of mods it still lists.
    let mut installed_paths = synced.paths;
    for path in &shipped_overrides {
        if !installed_paths.contains(path) {
            installed_paths.push(path.clone());
        }
    }
    let kept_paths = remove_dropped(
        instance_path,
        &installed_before,
        installed_paths,
        &claimed,
        &synced.kept,
    );

    emit_progress(app_handle, id, instance_name, "Extracting files", false, None);
    extract_overrides(&mut prepared.archive, instance_path)?;

    // The lookup that names the new files also says which project each comes
    // from, which is how a mod's new version, under a new name, is matched to
    // the old one the user turned off.
    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    let resolved = ResolvedFiles::of(synced.entries.iter().map(|(_, entry)| entry), &state.http_client).await;
    let placed = synced.entries.iter().filter_map(|(relative, entry)| {
        Some((relative.as_deref()?, resolved.project(&entry.sha1)))
    });
    let overridden = shipped_overrides.iter().map(|path| (path.as_str(), None));
    let turned_off = align_states(instance_path, placed.chain(overridden), &states, &claimed);
    let mut modlist_entries: Vec<ModListEntry> = synced
        .entries
        .into_iter()
        .map(|(relative, mut entry)| {
            if entry.state != ModState::DownloadFailed {
                let off = relative.is_some_and(|relative| turned_off.contains(&path_key(&relative)));
                entry.state = if off { ModState::Disabled } else { ModState::Enabled };
            }
            entry
        })
        .collect();
    resolved.name(&mut modlist_entries);
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
    use super::super::super::pack_files::{align_states, disabled_path, path_key, ModStates};
    use super::super::mrpack::{MrpackFile, MrpackHashes};
    use super::{is_unchanged, remove_pack_file};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use yaminabe_launcher_shared::datamodels::{
        DownloadSource, ModListEntry, ModState, ProjectFileTarget, ProjectId,
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

    /// `entry`, as the mod list records it once Modrinth has named its project.
    fn of_project(entry: ModListEntry, project_id: &str) -> ModListEntry {
        ModListEntry {
            source: DownloadSource::Modrinth {
                project_id: project_id.to_string(),
                version_id: String::new(),
            },
            ..entry
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

        let synced = super::sync_pack_files(&index, &dir, &previous, &ModStates::default(), &HashSet::new(), &reqwest::Client::new(), |_, _| {})
            .await
            .expect("sync");

        assert!(!jar.exists(), "the superseded jar must not survive a failed fetch");
        assert_eq!(synced.entries.len(), 1);
        assert_eq!(synced.entries[0].1.state, ModState::DownloadFailed);
        // Recorded even though the fetch failed: the file is the pack's, and the
        // user may supply it by hand before the next upgrade.
        assert_eq!(synced.paths, vec!["mods/a.jar".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A new version of a mod the user turned off is fetched, not kept. When it
    /// cannot be, the old disabled copy goes too: it is the build the pack no
    /// longer ships, and left there it would read as the new one.
    #[tokio::test]
    async fn a_failed_new_version_leaves_no_old_disabled_copy() {
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
        let states = ModStates::read(&dir, &previous, &[]);

        let synced = super::sync_pack_files(&index, &dir, &previous, &states, &HashSet::new(), &reqwest::Client::new(), |_, _| {})
            .await
            .expect("sync");

        assert!(!disabled.exists(), "the superseded disabled copy must not survive");
        assert!(!dir.join("mods").join("a.jar").exists(), "and it must not come back enabled");
        assert_eq!(synced.entries.len(), 1);
        assert_eq!(synced.entries[0].1.state, ModState::DownloadFailed);
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
        let states = ModStates::read(&dir, &previous, &[]);

        let synced = super::sync_pack_files(&index, &dir, &previous, &states, &HashSet::new(), &reqwest::Client::new(), |_, _| {})
            .await
            .expect("sync");

        assert!(disabled.exists(), "the kept file must be left where it is");
        assert_eq!(synced.entries.len(), 1);
        assert_eq!(synced.entries[0].1.state, ModState::Disabled);
        assert_eq!(synced.paths, vec!["mods/a.jar".to_string()]);
        assert_eq!(synced.kept, vec!["mods/a.jar.disabled".to_string()]);
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

        let synced = super::sync_pack_files(&index, &dir, &HashMap::new(), &ModStates::default(), &HashSet::new(), &reqwest::Client::new(), |_, _| {})
            .await
            .expect("sync");

        // Never adopted: whatever the outcome, the previous version's jar is
        // not reported as the new one.
        assert_eq!(synced.entries.len(), 1);
        assert_eq!(synced.entries[0].1.state, ModState::DownloadFailed);
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

        let synced = super::sync_pack_files(&index, &dir, &HashMap::new(), &ModStates::default(), &HashSet::new(), &reqwest::Client::new(), |_, _| {})
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

    /// A nested mod shares a name with a flat one the user turned off. The mod
    /// list holds bare names, so it can only speak for the flat file; inheriting
    /// its state would keep a mod the pack means to be on.
    #[tokio::test]
    async fn a_nested_mod_does_not_inherit_a_flat_one_s_state() {
        let dir = temp_dir("nested-state");
        std::fs::write(dir.join("mods").join("a.jar.disabled"), b"the flat one").expect("write twin");
        let index = index_with(MrpackFile {
            path: "mods/sub/a.jar".to_string(),
            hashes: MrpackHashes { sha1: "a1b2c3".to_string() },
            env: None,
            downloads: vec!["http://127.0.0.1:1/nope.jar".to_string()],
            file_size: 0,
        });
        let previous = HashMap::from([(
            "a.jar".to_string(),
            entry("a.jar", "a1b2c3", ModState::Disabled),
        )]);
        let states = ModStates::read(&dir, &previous, &[]);

        let synced = super::sync_pack_files(&index, &dir, &previous, &states, &HashSet::new(), &reqwest::Client::new(), |_, _| {})
            .await
            .expect("sync");

        assert_eq!(synced.paths, vec!["mods/sub/a.jar".to_string()]);
        assert_ne!(synced.entries[0].1.state, ModState::Disabled);
        assert!(synced.kept.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Clearing a mod's path takes the `.disabled` copy with it, since both
    /// names are the pack's file under the launcher's own rule. That is why the
    /// disable has to be read before any clear: read afterwards, the disk says
    /// nothing and a mod the user turned off is planned as enabled.
    #[test]
    fn clearing_a_mod_takes_its_disabled_copy_too() {
        let dir = temp_dir("clear-takes-disabled");
        let dest = dir.join("mods").join("a.jar");
        let disabled = disabled_path(&dest);
        std::fs::write(&dest, b"the version on disk").expect("write dest");
        std::fs::write(&disabled, b"the version the user turned off").expect("write disabled");

        super::clear_stale(&dest, "mods/a.jar", Some(ProjectFileTarget::Mod), &HashSet::new())
            .expect("clear");

        assert!(!dest.exists());
        assert!(
            !disabled.exists(),
            "the disabled copy survives, so reading the disable after a clear would still work"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A pack may ship `a.jar` and `a.jar.disabled` side by side. When only
    /// `a.jar` fails to arrive, clearing what stood at its name must not take
    /// the other with it: that file is the pack's own, just verified, and the
    /// mod list records it as installed.
    #[tokio::test]
    async fn a_failed_mod_does_not_clear_a_twin_the_pack_ships() {
        let dir = temp_dir("shipped-twin");
        let twin = dir.join("mods").join("a.jar.disabled");
        std::fs::write(&twin, b"the pack's alternate build").expect("write twin");
        let unreachable = vec!["http://127.0.0.1:1/nope.jar".to_string()];
        let mut index = index_with(MrpackFile {
            path: "mods/a.jar".to_string(),
            hashes: MrpackHashes { sha1: "a1b2c3".to_string() },
            env: None,
            downloads: unreachable.clone(),
            file_size: 0,
        });
        // Already on disk under the hash the index names, so it arrives without
        // a fetch while `a.jar` cannot.
        index.files.push(MrpackFile {
            path: "mods/a.jar.disabled".to_string(),
            hashes: MrpackHashes { sha1: crate::http_utils::sha1_hex(b"the pack's alternate build") },
            env: None,
            downloads: unreachable,
            file_size: 0,
        });
        let claimed = super::claimed_paths(&index, &dir, &[]);

        let synced = super::sync_pack_files(&index, &dir, &HashMap::new(), &ModStates::default(), &claimed, &reqwest::Client::new(), |_, _| {})
            .await
            .expect("sync");

        assert!(twin.exists(), "the pack's own file is not a.jar's toggle to clear");
        let twin_state = synced
            .entries
            .iter()
            .find(|(_, entry)| entry.file_name == "a.jar.disabled")
            .map(|(_, entry)| entry.state);
        assert_eq!(twin_state, Some(ModState::Enabled));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A new version of a mod usually arrives under a new file name. The user's
    /// choice is about the mod, so it follows the project across the rename:
    /// read first, the old version deleted, then the new one turned off.
    #[test]
    fn a_mod_renamed_by_its_new_version_stays_off() {
        let dir = temp_dir("renamed");
        let old_twin = dir.join("mods").join("a-1.0.jar.disabled");
        let new = dir.join("mods").join("a-2.0.jar");
        std::fs::write(&old_twin, b"1.0").expect("write old");
        let previous = HashMap::from([(
            "a-1.0.jar".to_string(),
            of_project(entry("a-1.0.jar", "1010", ModState::Disabled), "P"),
        )]);
        let installed_before = vec!["mods/a-1.0.jar".to_string()];
        let states = ModStates::read(&dir, &previous, &installed_before);
        // The new version, fetched under its plain name.
        std::fs::write(&new, b"2.0").expect("write new");
        let installed_paths = vec!["mods/a-2.0.jar".to_string()];
        let claimed = HashSet::from([path_key("mods/a-2.0.jar")]);

        let record = super::remove_dropped(&dir, &installed_before, installed_paths.clone(), &claimed, &[]);
        let project = Some(ProjectId::Modrinth("P".to_string()));
        let off = align_states(&dir, [("mods/a-2.0.jar", project)], &states, &claimed);

        assert!(!old_twin.exists(), "the old version is gone");
        assert!(!new.exists() && disabled_path(&new).exists(), "and the new one is off");
        assert!(off.contains(&path_key("mods/a-2.0.jar")));
        assert_eq!(record, installed_paths);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Turning `a.jar` off renamed it over the alternate build the old version
    /// shipped as `a.jar.disabled`, and the new version drops that alternate.
    /// The bytes there are `a.jar`'s, so the file is the user's choice: it is
    /// kept rather than fetched again, survives the clean-up of the dropped
    /// path, and the mod is still off at the end.
    #[tokio::test]
    async fn a_mod_the_user_turned_off_over_an_old_twin_stays_off() {
        let dir = temp_dir("toggled-over-twin");
        let twin = dir.join("mods").join("a.jar.disabled");
        std::fs::write(&twin, b"the pack's mod").expect("write twin");
        let sha1 = crate::http_utils::sha1_hex(b"the pack's mod");
        let previous = HashMap::from([
            ("a.jar".to_string(), entry("a.jar", &sha1, ModState::Disabled)),
            ("a.jar.disabled".to_string(), entry("a.jar.disabled", "0ld0ld", ModState::Enabled)),
        ]);
        let installed_before = vec!["mods/a.jar".to_string(), "mods/a.jar.disabled".to_string()];
        // Unchanged, so the kept copy is all there is: a fetch here would fail.
        let index = index_with(MrpackFile {
            path: "mods/a.jar".to_string(),
            hashes: MrpackHashes { sha1 },
            env: None,
            downloads: vec!["http://127.0.0.1:1/nope.jar".to_string()],
            file_size: 0,
        });
        let states = ModStates::read(&dir, &previous, &installed_before);
        let claimed = super::claimed_paths(&index, &dir, &[]);

        let synced = super::sync_pack_files(&index, &dir, &previous, &states, &claimed, &reqwest::Client::new(), |_, _| {})
            .await
            .expect("sync");
        let record = super::remove_dropped(&dir, &installed_before, synced.paths.clone(), &claimed, &synced.kept);
        let off = align_states(&dir, [("mods/a.jar", None)], &states, &claimed);

        assert_eq!(synced.entries[0].1.state, ModState::Disabled);
        assert!(twin.exists(), "the user's disabled copy is kept");
        assert!(!dir.join("mods").join("a.jar").exists(), "and not switched back on");
        assert!(off.contains(&path_key("mods/a.jar")));
        assert_eq!(record, vec!["mods/a.jar".to_string()], "the mod's own path answers for it");
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
