use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::super::pack_files::{
    align_states, disabled_path, installed_entries, path_key, safe_destination, ModStates,
};
use super::api::{fetch_project_summaries, resolve_project_files};
use super::manifest::{manifest_file_ids, read_manifest, resolve_loader, ModpackManifest};
use crate::commands::instance::{
    create_instance_dir, discard_unfinished_instance_dir, instance_meta_file, is_bare_file_name,
    is_current_dir, is_launcher_dir, is_parent_dir, replace_modlist_entries_for_file_ids,
    upsert_modlist_entries,
};
use crate::emit_progress;
use crate::http_utils::download_resource;
use crate::install_task::ensure_game_and_loader;
use crate::json::{read_json, write_json};
use crate::AppState;
use log::{info, warn};
use tauri::State;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, InstanceMeta, ModListEntry, ModLoader, ModState, ProjectFileInfo, ProjectId,
};
use yaminabe_launcher_shared::error::Error;

/// What the modlist records for one CurseForge file the instance holds.
struct InstalledFile {
    state: ModState,
    file_name: String,
}

/// CurseForge file id → what `previous`, the instance's modlist, records for it:
/// whether its download failed and is due a retry, and the name on disk to
/// delete when the pack drops the file. The modlist is the source of truth for
/// what is installed and is only rewritten when an upgrade finalizes, so a
/// failed (and retried) upgrade keeps diffing against the previous set. Empty
/// when the modlist is missing, degrading to "download everything, remove nothing".
fn installed_curseforge_files(previous: &HashMap<String, ModListEntry>) -> HashMap<u32, InstalledFile> {
    previous
        .values()
        .filter_map(|entry| {
            let (_, file_id) = entry.source.curseforge_ids()?;
            Some((
                file_id,
                InstalledFile { state: entry.state, file_name: entry.file_name.clone() },
            ))
        })
        .collect()
}

/// Where a mod the manifest lists sits in the instance.
fn relative_of(file: &ProjectFileInfo) -> String {
    format!("{}/{}", file.target.directory(), file.file_name)
}

/// Delete the jars the new version dropped, under either name: a mod the user
/// turned off sits under `<name>.disabled`.
///
/// A name the new version also uses belongs to the file just written, so it is
/// left alone and its old disabled copy is [`settle_states`]'s to replace. But
/// only once that file arrived: where `downloaded` reports it failed, what sits
/// at the name is still the old build, and left there it would load while the
/// mod list calls the new version missing.
fn remove_dropped_mods(
    instance_path: &Path,
    old_file_names: &[String],
    new_file_names: &[String],
    downloaded: &[ModListEntry],
) {
    let failed: HashSet<&str> = downloaded
        .iter()
        .filter(|entry| entry.state == ModState::DownloadFailed)
        .map(|entry| entry.file_name.as_str())
        .collect();
    let kept_names: HashSet<&str> = new_file_names
        .iter()
        .map(String::as_str)
        .filter(|name| !failed.contains(name))
        .collect();
    for file_name in old_file_names {
        if kept_names.contains(file_name.as_str()) {
            continue;
        }
        // The name is the modlist's, which is a file on disk that could have been
        // edited; one that climbs out of the instance is not the pack's to delete.
        let Some(dest) = safe_destination(instance_path, &format!("mods/{file_name}")) else {
            warn!("refusing to remove '{file_name}': not inside the instance");
            continue;
        };
        for path in [disabled_path(&dest), dest] {
            if path.exists() {
                if let Err(e) = std::fs::remove_file(&path) {
                    warn!("failed to remove old mod {}: {e}", path.display());
                }
            }
        }
    }
}

/// The new version's mod-list rows, each in the state the user left its old
/// version in. Run last, once the old versions are gone, so nothing after it
/// removes what it renames.
///
/// A row comes from the download where one ran, and is built from the file
/// otherwise, since an unchanged mod is not fetched again. Its state is what
/// disk says once [`align_states`] has run, which matches each mod to its old
/// version by path, or failing that by its CurseForge project.
fn settle_states(
    instance_path: &Path,
    new_files: &[ProjectFileInfo],
    downloaded: Vec<ModListEntry>,
    states: &ModStates,
) -> Vec<ModListEntry> {
    let mods: Vec<&ProjectFileInfo> = new_files
        .iter()
        .filter(|file| file.target.tracks_modlist())
        .collect();
    let claimed: HashSet<String> = new_files.iter().map(|file| path_key(&relative_of(file))).collect();
    let placed: Vec<(String, Option<ProjectId>)> = mods
        .iter()
        .map(|file| (relative_of(file), file.source.project_id()))
        .collect();
    let turned_off = align_states(
        instance_path,
        placed.iter().map(|(relative, project)| (relative.as_str(), project.clone())),
        states,
        &claimed,
    );

    let mut entries: Vec<ModListEntry> = mods
        .iter()
        .map(|file| file.to_modlist_entry(ModState::Enabled))
        .collect();
    for entry in downloaded {
        entries.retain(|existing| existing.file_name != entry.file_name);
        entries.push(entry);
    }
    for entry in &mut entries {
        if entry.state != ModState::DownloadFailed {
            let relative = format!("{}/{}", entry.target.directory(), entry.file_name);
            let off = turned_off.contains(&path_key(&relative));
            entry.state = if off { ModState::Disabled } else { ModState::Enabled };
        }
    }
    entries
}

/// Extract the modpack's `overrides/` tree into `instance_path`, overwriting
/// any file the pack ships and leaving everything else (user saves, configs,
/// manually-added mods) untouched.
fn extract_overrides<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    overrides_prefix: &str,
    instance_path: &Path,
) -> Result<(), Error> {
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| Error::Invalid(format!("reading zip entry at index {i}: {e}")))?;

        let entry_name = file.name().to_string();
        let Some(rel) = entry_name.strip_prefix(overrides_prefix) else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }

        // Entry names come from an untrusted zip. Split on both separators —
        // Windows treats `\` as one — and refuse any that could climb out:
        // `..`, and anything holding a drive prefix like `C:`, which `join`
        // would take as an absolute path replacing everything before it.
        //
        // Refused rather than filtered: dropping the `..` from
        // `overrides/../.launcher/instance.json` writes the instance's own
        // record instead, which is not what the pack described either.
        let mut components: Vec<&str> = Vec::new();
        let mut escapes = false;
        for part in rel.split(['/', '\\']) {
            if is_parent_dir(part) || part.contains(':') {
                escapes = true;
                break;
            }
            if !is_current_dir(part) {
                components.push(part);
            }
        }
        if escapes {
            warn!("skipping override with an unusable path: {entry_name}");
            continue;
        }
        if components.is_empty() {
            continue;
        }
        // `.launcher/` is the launcher's own record of the instance, not part
        // of the pack. Nothing needs `..` to reach it, and a modlist planted
        // there is merged into the instance's own and renders whatever it
        // likes in the Mods tab.
        if is_launcher_dir(components[0]) {
            warn!("skipping override writing into the launcher's own directory: {entry_name}");
            continue;
        }
        let dest = components
            .iter()
            .fold(instance_path.to_path_buf(), |p, c| p.join(c));

        if entry_name.ends_with('/') {
            std::fs::create_dir_all(&dest)?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&dest)?;
            std::io::copy(&mut file, &mut out)?;
        }
    }
    Ok(())
}


/// The game/loader/manifest state produced by [`prepare_modpack`], handed back so
/// the install and upgrade paths can finish with their own mod handling.
struct PreparedModpack {
    manifest: ModpackManifest,
    mc_version: String,
    mod_loader: ModLoader,
    loader_version: Option<String>,
    version_id: String,
}

/// Shared install/upgrade prefix: resolve the modpack zip URL from
/// `(project_id, file_id)`, stage it in the instance cache, read the manifest,
/// ensure the game + loader are installed, and extract `overrides/` into
/// `instance_path`. The cached zip is processed from disk (not held in memory)
/// and deleted once its overrides are extracted. Mod-list handling and instance
/// metadata differ between install and upgrade, so they stay with the caller.
async fn prepare_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    instance_path: &Path,
    file_id: u32,
    api_key: &str,
    state: &State<'_, AppState>,
) -> Result<PreparedModpack, Error> {
    let http_client = &state.http_client;
    emit_progress(
        app_handle,
        id,
        instance_name,
        "Downloading modpack",
        false,
        None,
    );

    let file = resolve_project_files(&[file_id], api_key, http_client)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| Error::NotExists(format!("CurseForge file {file_id}")))?;
    let download_url = file.download_url.as_deref().ok_or_else(|| {
        Error::Invalid(format!(
            "CurseForge modpack file {} has no download URL",
            file.file_name
        ))
    })?;
    // The name comes from the API, not from us; refuse one that would stage the
    // zip outside the instance's cache directory.
    if !is_bare_file_name(&file.file_name) {
        return Err(Error::Invalid(format!(
            "CurseForge file {file_id} names itself '{}'",
            file.file_name
        )));
    }
    let cache_path = instance_path
        .join(file.target.directory())
        .join(&file.file_name);
    download_resource(http_client, download_url, &file.sha1, cache_path.clone()).await?;

    let prepared = prepare_from_cached_zip(
        app_handle,
        id,
        instance_name,
        instance_path,
        &cache_path,
        state,
    )
    .await;

    // Always drop the staged zip — its overrides are now in the instance and
    // its mods are fetched separately from the manifest.
    std::fs::remove_file(&cache_path).ok();
    prepared
}

/// Read the cached modpack zip from disk: parse the manifest, ensure the game +
/// loader, and extract overrides into the instance. Split out so `prepare_modpack`
/// can delete the cache file on every exit path.
async fn prepare_from_cached_zip(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    instance_path: &Path,
    cache_path: &Path,
    state: &State<'_, AppState>,
) -> Result<PreparedModpack, Error> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(cache_path)?)
        .map_err(|e| Error::Invalid(format!("modpack zip is invalid: {e}")))?;

    let manifest = read_manifest(&mut archive)?;
    let mc_version = manifest.minecraft.version.clone();
    let (mod_loader, loader_version) = resolve_loader(&manifest)?;

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

    emit_progress(
        app_handle,
        id,
        instance_name,
        "Extracting files",
        false,
        None,
    );
    let overrides_prefix = format!("{}/", manifest.overrides.trim_end_matches('/'));
    extract_overrides(&mut archive, &overrides_prefix, instance_path)?;

    Ok(PreparedModpack {
        manifest,
        mc_version,
        mod_loader,
        loader_version,
        version_id,
    })
}

/// What differs between installing a pack fetched by id and one read off disk:
/// where the instance says it came from, and what seeds its description.
struct InstallOrigin {
    source: DownloadSource,
    description: String,
}

/// Finish an install once the pack is prepared: resolve and download the mods
/// its manifest lists, record them, and write the instance.
async fn finish_install(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    instance_path: &Path,
    prepared: PreparedModpack,
    origin: InstallOrigin,
    api_key: &str,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    emit_progress(app_handle, id, instance_name, "Downloading mods", false, None);
    let file_ids = manifest_file_ids(&prepared.manifest);
    let files = resolve_project_files(&file_ids, api_key, &state.http_client).await?;
    let modlist_entries =
        crate::mod_repo::download_project_files(files, instance_path, &state.http_client).await?;

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    upsert_modlist_entries(instance_path, modlist_entries)?;

    let meta = InstanceMeta {
        id: id.to_string(),
        name: instance_name.to_string(),
        description: origin.description,
        game_version: prepared.mc_version.clone(),
        mod_loader: prepared.mod_loader.clone(),
        mod_loader_version: prepared.loader_version,
        version_id: prepared.version_id,
        category,
        origin: origin.source,
        ..InstanceMeta::default()
    };
    write_json(instance_meta_file(instance_path), &meta)?;

    info!(
        "Installed '{}' (MC {}, {}) → {}",
        instance_name,
        prepared.mc_version,
        prepared.mod_loader,
        instance_path.display()
    );
    Ok(())
}

pub async fn install_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    source: DownloadSource,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let (project_id, file_id) = source
        .curseforge_ids()
        .ok_or_else(|| Error::Unsupported("not a CurseForge modpack source".to_string()))?;
    let (api_key, install_dir) = {
        let settings = state.settings.read().unwrap();
        (settings.curseforge_api_key.clone(), settings.instance_install_dir.clone())
    };
    let instance_path = create_instance_dir(&install_dir, instance_name)?;
    // Nothing usable exists until the instance record is written, so clear the
    // directory on failure rather than leaving one the library never shows and
    // that blocks retrying the same name — a missing API key gets this far.
    let result = install_by_id(
        app_handle,
        id,
        instance_name,
        category,
        source,
        project_id,
        file_id,
        &api_key,
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
async fn install_by_id(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    source: DownloadSource,
    project_id: u32,
    file_id: u32,
    api_key: &str,
    instance_path: &Path,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let prepared = prepare_modpack(
        app_handle,
        id,
        instance_name,
        instance_path,
        file_id,
        api_key,
        state,
    ).await?;

    // Seed the instance description with the pack's own blurb. It is cosmetic,
    // so a failed lookup leaves it empty instead of failing the install.
    let description = match fetch_project_summaries(&[project_id], api_key, &state.http_client).await {
        Ok(projects) => projects
            .get(&project_id)
            .map(|project| project.summary.clone())
            .unwrap_or_default(),
        Err(e) => {
            log::warn!("no description for CurseForge project {project_id}: {e}");
            String::new()
        }
    };

    finish_install(
        app_handle,
        id,
        instance_name,
        category,
        instance_path,
        prepared,
        InstallOrigin { source, description },
        api_key,
        state,
    )
    .await
}

/// Upgrade an existing CurseForge instance to a newer modpack file: re-ensure
/// the (possibly changed) game + loader, overlay the new `overrides/`, diff the
/// mod list against the instance's recorded manifest — downloading only new
/// mods and removing only mods the pack dropped — then update instance metadata.
/// User-added files (saves, screenshots, manual mods) are never touched.
pub async fn upgrade_modpack(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    instance_path: PathBuf,
    source: DownloadSource,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let file_id = source
        .curseforge_ids()
        .map(|(_, file_id)| file_id)
        .ok_or_else(|| Error::Unsupported("not a CurseForge modpack source".to_string()))?;
    let api_key = state.settings.read().unwrap().curseforge_api_key.clone();
    let http_client = &state.http_client;

    // The modlist (the installed-mod source of truth) is only rewritten at
    // finalize, so this set survives a failed/retried upgrade for a correct re-diff.
    let previous = installed_entries(&instance_path);
    let installed = installed_curseforge_files(&previous);
    let old_ids: Vec<u32> = installed.keys().copied().collect();
    // Read before the overrides are extracted over the instance: they and the
    // downloads both rewrite the files that say which mods are off.
    let states = ModStates::read(&instance_path, &previous, &[]);

    let prepared = prepare_modpack(
        app_handle,
        id,
        instance_name,
        &instance_path,
        file_id,
        &api_key,
        state,
    )
    .await?;

    // Diff the mod list by file id: download additions, remove drops, skip
    // unchanged files already on disk.
    emit_progress(app_handle, id, instance_name, "Updating mods", false, None);
    let new_ids = manifest_file_ids(&prepared.manifest);
    let new_set: HashSet<u32> = new_ids.iter().copied().collect();
    // A file the pack still ships but whose download previously failed has no jar
    // on disk, so an upgrade is the moment to retry it alongside the genuine additions.
    let to_add: HashSet<u32> = new_ids
        .iter()
        .copied()
        .filter(|fid| matches!(installed.get(fid).map(|f| f.state), None | Some(ModState::DownloadFailed)))
        .collect();

    // TODO: the modlist tracks mods, so a resource pack the previous version
    // installed is invisible here and survives a version that drops it. The
    // Modrinth path records what a pack installed to answer this.
    // Names the pack drops, taken from the modlist rather than resolved against
    // the API — an id CurseForge no longer serves would otherwise be skipped,
    // leaving its jar loading against the upgraded pack. Deleting is deferred
    // until the downloads land: a failure in between would strip jars the
    // modlist still calls installed, and a later upgrade to a different target
    // would never fetch them again.
    let old_file_names: Vec<String> = installed
        .iter()
        .filter(|(file_id, _)| !new_set.contains(*file_id))
        .map(|(_, installed_file)| installed_file.file_name.clone())
        .collect();

    // One resolve for the whole new file set — `to_add` is a subset of it, so
    // splitting the result here saves a second round of /v1/mods/files (plus its
    // per-chunk /v1/mods fan-out) against the CurseForge rate limit.
    let new_files = resolve_project_files(&new_ids, &api_key, http_client).await?;
    let new_files_names: Vec<String> = new_files.iter().map(|f| f.file_name.clone()).collect();

    // `to_add` mixes new mods and (all) resource packs — resource packs are never
    // in `installed`, since the modlist tracks only mods. `download_project_files`
    // routes each to the right dir and returns entries for mods alone.
    let added_files: Vec<ProjectFileInfo> = new_files
        .iter()
        .filter(|file| {
            file.source
                .curseforge_ids()
                .is_some_and(|(_, fid)| to_add.contains(&fid))
        })
        .cloned()
        .collect();
    let downloaded =
        crate::mod_repo::download_project_files(added_files, &instance_path, http_client).await?;

    // Everything the new pack ships is now on disk, so the old versions can go,
    // and only then does each new one take the state its old one was left in.
    remove_dropped_mods(&instance_path, &old_file_names, &new_files_names, &downloaded);
    let new_modlist_entries = settle_states(&instance_path, &new_files, downloaded, &states);

    emit_progress(app_handle, id, instance_name, "Finalizing", false, None);
    replace_modlist_entries_for_file_ids(
        &instance_path,
        &old_ids,
        &old_file_names,
        new_modlist_entries,
    )?;

    let mut meta: InstanceMeta = read_json(instance_meta_file(&instance_path))?;
    meta.game_version = prepared.mc_version.clone();
    meta.mod_loader = prepared.mod_loader.clone();
    meta.mod_loader_version = prepared.loader_version;
    meta.version_id = prepared.version_id;
    meta.origin = source;
    write_json(instance_meta_file(&instance_path), &meta)?;

    info!(
        "Upgraded '{}' to file {} (MC {}, {})",
        instance_name, file_id, prepared.mc_version, prepared.mod_loader
    );
    Ok(())
}


/// Install a modpack from a zip already on disk. The zip is read where it lies
/// and left alone — it is the user's file, not a staged download — so this skips
/// the fetch that `install_modpack` performs and shares everything after it.
///
/// The instance records a `Manual` origin: a manifest names the mods it wants
/// but not the pack it came from, so there is no project to offer an upgrade
/// against.
pub async fn install_modpack_from_file(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    zip_path: &Path,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let (api_key, install_dir) = {
        let settings = state.settings.read().unwrap();
        (settings.curseforge_api_key.clone(), settings.instance_install_dir.clone())
    };
    let instance_path = create_instance_dir(&install_dir, instance_name)?;
    let result = install_from_zip(
        app_handle,
        id,
        instance_name,
        category,
        zip_path,
        &api_key,
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
async fn install_from_zip(
    app_handle: &tauri::AppHandle,
    id: &str,
    instance_name: &str,
    category: String,
    zip_path: &Path,
    api_key: &str,
    instance_path: &Path,
    state: &State<'_, AppState>,
) -> Result<(), Error> {
    let prepared = prepare_from_cached_zip(
        app_handle,
        id,
        instance_name,
        instance_path,
        zip_path,
        state,
    )
    .await?;

    // A manifest carries no project id for the pack itself, so there is nothing
    // to offer an upgrade against; the pack's own name is the best description
    // available without an API lookup that could not identify it anyway.
    let description = prepared.manifest.name.clone();
    finish_install(
        app_handle,
        id,
        instance_name,
        category,
        instance_path,
        prepared,
        InstallOrigin { source: DownloadSource::Manual, description },
        api_key,
        state,
    )
    .await
}

#[cfg(test)]
mod upgrade_tests {
    use super::super::super::pack_files::{disabled_path, ModStates};
    use super::{remove_dropped_mods, settle_states};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use yaminabe_launcher_shared::datamodels::{
        DownloadSource, ModListEntry, ModState, ProjectFileInfo, ProjectFileReleaseType,
        ProjectFileTarget,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yaminabe-cf-upgrade-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("mods")).expect("create mods dir");
        dir
    }

    /// A mod file of CurseForge project `project_id`, as the API resolves it.
    fn mod_file(file_name: &str, project_id: u32, file_id: u32) -> ProjectFileInfo {
        ProjectFileInfo {
            source: DownloadSource::CurseForge { project_id, file_id },
            target: ProjectFileTarget::Mod,
            release_type: ProjectFileReleaseType::Release,
            file_name: file_name.to_string(),
            download_url: None,
            display_name: file_name.to_string(),
            project_name: String::new(),
            icon_url: None,
            sha1: String::new(),
            size: 0,
        }
    }

    /// The mod list as it stood before the upgrade.
    fn listed(rows: &[(&ProjectFileInfo, ModState)]) -> HashMap<String, ModListEntry> {
        rows.iter()
            .map(|(file, state)| (file.file_name.clone(), file.to_modlist_entry(*state)))
            .collect()
    }

    /// A mod update arrives as a new file under a new name. The user's choice
    /// follows the project: the old version is deleted, and the new one is off.
    #[test]
    fn a_mod_updated_to_a_new_file_stays_off() {
        let dir = temp_dir("updated");
        let old = mod_file("a-1.0.jar", 10, 100);
        let new = mod_file("a-2.0.jar", 10, 200);
        let old_twin = dir.join("mods").join("a-1.0.jar.disabled");
        std::fs::write(&old_twin, b"1.0").expect("write old");
        let states = ModStates::read(&dir, &listed(&[(&old, ModState::Disabled)]), &[]);
        // The new version, downloaded under its plain name.
        let jar = dir.join("mods").join("a-2.0.jar");
        std::fs::write(&jar, b"2.0").expect("write new");

        let downloaded = vec![new.to_modlist_entry(ModState::Enabled)];
        remove_dropped_mods(&dir, &["a-1.0.jar".to_string()], &["a-2.0.jar".to_string()], &downloaded);
        let entries = settle_states(&dir, std::slice::from_ref(&new), downloaded, &states);

        assert!(!old_twin.exists(), "the old version is gone");
        assert!(!jar.exists() && disabled_path(&jar).exists(), "and the new one is off");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].state, ModState::Disabled);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A new file may reuse its old name. The clean-up leaves that name to the
    /// file just written, so the old disabled copy beside it is replaced by the
    /// new version, turned off, rather than left behind with the new one on.
    #[test]
    fn a_new_file_under_the_old_name_replaces_the_disabled_copy() {
        let dir = temp_dir("same-name");
        let old = mod_file("a.jar", 10, 100);
        let new = mod_file("a.jar", 10, 200);
        let jar = dir.join("mods").join("a.jar");
        std::fs::write(disabled_path(&jar), b"old").expect("write old");
        let states = ModStates::read(&dir, &listed(&[(&old, ModState::Disabled)]), &[]);
        std::fs::write(&jar, b"new").expect("write new");

        let downloaded = vec![new.to_modlist_entry(ModState::Enabled)];
        remove_dropped_mods(&dir, &["a.jar".to_string()], &["a.jar".to_string()], &downloaded);
        let entries = settle_states(&dir, std::slice::from_ref(&new), downloaded, &states);

        assert!(!jar.exists());
        assert_eq!(std::fs::read(disabled_path(&jar)).expect("read"), b"new");
        assert_eq!(entries[0].state, ModState::Disabled);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A new file under its old name whose download failed: what sits at that
    /// name is still the old build, enabled or not, and left in place it would
    /// load while the mod list says the new version is missing.
    #[test]
    fn a_failed_replacement_under_the_old_name_leaves_no_old_build() {
        for (old_on_disk, state) in [("a.jar", ModState::Enabled), ("a.jar.disabled", ModState::Disabled)] {
            let dir = temp_dir("failed-same-name");
            let old = mod_file("a.jar", 10, 100);
            let new = mod_file("a.jar", 10, 200);
            let jar = dir.join("mods").join("a.jar");
            std::fs::write(dir.join("mods").join(old_on_disk), b"old").expect("write old");
            let states = ModStates::read(&dir, &listed(&[(&old, state)]), &[]);

            let downloaded = vec![new.to_modlist_entry(ModState::DownloadFailed)];
            remove_dropped_mods(&dir, &["a.jar".to_string()], &["a.jar".to_string()], &downloaded);
            let entries = settle_states(&dir, std::slice::from_ref(&new), downloaded, &states);

            assert!(
                !jar.exists() && !disabled_path(&jar).exists(),
                "the old build left as {old_on_disk} must go"
            );
            assert_eq!(entries[0].state, ModState::DownloadFailed);
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    /// An unchanged mod is not fetched again, and its state is read off disk
    /// rather than the mod list: an interrupted toggle leaves the two
    /// disagreeing, and disk is what the game loads.
    #[test]
    fn an_unchanged_mod_keeps_the_state_disk_shows() {
        let dir = temp_dir("unchanged");
        let a = mod_file("a.jar", 10, 100);
        let b = mod_file("b.jar", 11, 110);
        std::fs::write(dir.join("mods").join("a.jar.disabled"), b"a").expect("write a");
        std::fs::write(dir.join("mods").join("b.jar"), b"b").expect("write b");
        // Recorded the other way round from what disk says.
        let previous = listed(&[(&a, ModState::Enabled), (&b, ModState::Disabled)]);
        let states = ModStates::read(&dir, &previous, &[]);

        let entries = settle_states(&dir, &[a.clone(), b.clone()], Vec::new(), &states);

        let state_of = |name: &str| entries.iter().find(|entry| entry.file_name == name).map(|entry| entry.state);
        assert_eq!(state_of("a.jar"), Some(ModState::Disabled));
        assert_eq!(state_of("b.jar"), Some(ModState::Enabled));
        assert!(dir.join("mods").join("a.jar.disabled").exists(), "neither is moved");
        assert!(dir.join("mods").join("b.jar").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A new version that could not be fetched stays recorded as such, so the
    /// Mods tab still offers to link it, whatever state its old version was in.
    #[test]
    fn a_new_version_that_failed_to_download_stays_failed() {
        let dir = temp_dir("failed");
        let old = mod_file("a-1.0.jar", 10, 100);
        let new = mod_file("a-2.0.jar", 10, 200);
        std::fs::write(dir.join("mods").join("a-1.0.jar.disabled"), b"1.0").expect("write old");
        let states = ModStates::read(&dir, &listed(&[(&old, ModState::Disabled)]), &[]);

        let downloaded = vec![new.to_modlist_entry(ModState::DownloadFailed)];
        let entries = settle_states(&dir, std::slice::from_ref(&new), downloaded, &states);

        assert_eq!(entries[0].state, ModState::DownloadFailed);
        std::fs::remove_dir_all(&dir).ok();
    }
}
