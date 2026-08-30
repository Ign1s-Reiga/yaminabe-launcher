use crate::commands::instance::{
    find_instance_dir, instance_meta_file, modlist_file, upsert_modlist_entries,
};
use crate::http_utils::sha1_hex;
use crate::json::{read_json, read_json_or_default, write_json};
use crate::mod_repo;
use crate::{ActivityGuard, AppState, InstanceActivity, emit_progress};
use serde::Serialize;
use std::path::Path;
use tauri::State;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, InstanceMeta, ModListEntry, ModLoader, ModProjectSearchResults, ModState,
    ProjectFileInfo, ProjectFileTarget, SearchOptions,
};
use yaminabe_launcher_shared::error::Error;

#[tauri::command]
pub async fn search_projects(
    option: SearchOptions,
    state: State<'_, AppState>,
) -> Result<ModProjectSearchResults, Error> {
    let api_key = state.settings.read().unwrap().curseforge_api_key.clone();
    mod_repo::search_projects(&option, &state.http_client, &api_key).await
}

#[tauri::command]
pub async fn list_project_files(
    mod_id: u32,
    target: ProjectFileTarget,
    game_version: Option<String>,
    mod_loader: Option<ModLoader>,
    index: u32,
    state: State<'_, AppState>,
) -> Result<Vec<ProjectFileInfo>, Error> {
    let api_key = state.settings.read().unwrap().curseforge_api_key.clone();
    mod_repo::list_project_files(
        mod_id,
        target,
        game_version,
        mod_loader,
        index,
        &state.http_client,
        &api_key,
    )
    .await
}

#[tauri::command]
pub async fn install_modpack(
    app_handle: tauri::AppHandle,
    instance_name: String,
    category: String,
    source: DownloadSource,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string();

    let result =
        mod_repo::install_modpack(&app_handle, &id, &instance_name, category, source, &state).await;

    match result {
        Ok(()) => {
            emit_progress(&app_handle, &id, &instance_name, "Done", true, None);
            Ok(())
        }
        Err(e) => {
            emit_progress(
                &app_handle,
                &id,
                &instance_name,
                "Failed",
                false,
                Some(e.to_string()),
            );
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn upgrade_modpack(
    app_handle: tauri::AppHandle,
    instance_id: String,
    source: DownloadSource,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    // The upgrade modal closes before this runs, so an Err alone would be
    // invisible — every failure emits a job the dock can show. Until the
    // instance's name is read, its id stands in for the label.
    let fail = |name: &str, e: Error| {
        emit_progress(
            &app_handle,
            &instance_id,
            name,
            "Failed",
            false,
            Some(e.to_string()),
        );
        e
    };

    let Some(_activity) = ActivityGuard::claim(
        &state.instance_activity,
        &instance_id,
        InstanceActivity::Upgrading,
    ) else {
        return Err(fail(
            &instance_id,
            Error::Busy(format!("instance '{instance_id}' is already busy")),
        ));
    };

    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let instance_dir = find_instance_dir(Path::new(&install_dir), &instance_id)
        .map_err(|e| fail(&instance_id, e))?;

    let meta: InstanceMeta =
        read_json(instance_meta_file(&instance_dir)).map_err(|e| fail(&instance_id, e))?;
    if meta.origin.curseforge_ids().is_none() {
        return Err(fail(
            &meta.name,
            Error::Invalid("instance is not a CurseForge modpack instance".to_string()),
        ));
    }
    let instance_name = meta.name.clone();

    emit_progress(
        &app_handle,
        &instance_id,
        &instance_name,
        "Preparing upgrade",
        false,
        None,
    );
    let result = mod_repo::upgrade_modpack(
        &app_handle,
        &instance_id,
        &instance_name,
        instance_dir,
        source,
        &state,
    )
    .await;

    match result {
        Ok(()) => {
            emit_progress(
                &app_handle,
                &instance_id,
                &instance_name,
                "Done",
                true,
                None,
            );
            Ok(())
        }
        Err(e) => Err(fail(&instance_name, e)),
    }
}

#[tauri::command]
pub async fn download_mods(
    files: Vec<ProjectFileInfo>,
    instance_id: String,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    let Some(_activity) = ActivityGuard::claim(
        &state.instance_activity,
        &instance_id,
        InstanceActivity::Modifying,
    ) else {
        return Err(Error::Busy(format!(
            "instance '{instance_id}' is already busy"
        )));
    };
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let instance_path = find_instance_dir(Path::new(&install_dir), &instance_id)?;
    let entries =
        mod_repo::download_project_files(files, &instance_path, &state.http_client).await?;
    upsert_modlist_entries(&instance_path, entries)
}

/// Result of a link attempt: the file names successfully linked and the input
/// paths that matched no `DownloadFailed` entry (by SHA-1).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkOutcome {
    linked: Vec<String>,
    unmatched: Vec<String>,
}

/// Resolve `DownloadFailed` files by linking copies the user supplies from disk.
/// Each path is hashed and matched against a failed entry's recorded SHA-1, so
/// any local copy of the right file works whatever it is called. A file the site
/// published no hash for cannot be matched that way, so it falls back to the name
/// the manifest recorded — the only identity such an entry has.
///
/// A match is copied into the entry's target directory under the entry's name.
/// Mods flip to `Enabled`; anything else leaves the modlist, which tracks mods
/// and only held the entry to drive this prompt.
#[tauri::command]
pub fn link_mods(
    instance_id: String,
    file_paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<LinkOutcome, Error> {
    let Some(_activity) = ActivityGuard::claim(
        &state.instance_activity,
        &instance_id,
        InstanceActivity::Modifying,
    ) else {
        return Err(Error::Busy(format!(
            "instance '{instance_id}' is already busy"
        )));
    };
    let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
    let instance_dir = find_instance_dir(Path::new(&install_dir), &instance_id)?;
    let mut modlist: Vec<ModListEntry> = read_json_or_default(modlist_file(&instance_dir))?;
    let mut linked = Vec::new();
    let mut unmatched = Vec::new();
    for path in &file_paths {
        let Ok(bytes) = std::fs::read(path) else {
            log::warn!("link_mods: cannot read {path}");
            unmatched.push(path.clone());
            continue;
        };
        let sha1 = sha1_hex(&bytes);
        let supplied_name = Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let matched = modlist
            .iter()
            .position(|e| e.state == ModState::DownloadFailed && !e.sha1.is_empty() && e.sha1 == sha1)
            .or_else(|| {
                modlist.iter().position(|e| {
                    e.state == ModState::DownloadFailed
                        && e.sha1.is_empty()
                        && e.file_name == supplied_name
                })
            });
        let Some(index) = matched else {
            unmatched.push(path.clone());
            continue;
        };

        let (file_name, target) = {
            let entry = &modlist[index];
            (entry.file_name.clone(), entry.target)
        };
        let target_dir = instance_dir.join(target.directory());
        std::fs::create_dir_all(&target_dir)?;
        std::fs::write(target_dir.join(&file_name), &bytes)?;
        linked.push(file_name);
        if target.tracks_modlist() {
            modlist[index].state = ModState::Enabled;
        } else {
            modlist.remove(index);
        }
    }
    if !linked.is_empty() {
        write_json(modlist_file(&instance_dir), &modlist)?;
    }
    Ok(LinkOutcome { linked, unmatched })
}
