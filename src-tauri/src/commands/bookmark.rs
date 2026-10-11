use log::warn;
use tauri::State;
use yaminabe_launcher_shared::datamodels::{ModProjectInfo, ProjectId};
use yaminabe_launcher_shared::error::Error;

use crate::json::{read_json_or_default, write_json};
use crate::{bookmarks_path, AppState};

/// The bookmarked modpacks, read once at startup. A file that will not load
/// costs the bookmarks, not the launcher: the list starts empty, and the next
/// bookmark saved replaces the file.
pub fn load_bookmarks() -> Vec<ModProjectInfo> {
    read_json_or_default(bookmarks_path()).unwrap_or_else(|e| {
        warn!("cannot read the bookmarks in {}: {e}", bookmarks_path().display());
        Vec::new()
    })
}

#[tauri::command]
pub fn get_bookmarks(state: State<'_, AppState>) -> Vec<ModProjectInfo> {
    state.bookmarks.lock().unwrap().clone()
}

#[tauri::command]
pub fn add_bookmark(
    project: ModProjectInfo,
    state: State<'_, AppState>,
) -> Result<Vec<ModProjectInfo>, Error> {
    save_change(&state, |list| add(list, project))
}

#[tauri::command]
pub fn remove_bookmark(
    id: ProjectId,
    state: State<'_, AppState>,
) -> Result<Vec<ModProjectInfo>, Error> {
    save_change(&state, |list| remove(list, &id))
}

/// Apply `change` to the bookmarks and save them, returning the saved list. A
/// save that fails keeps the old list, so what the UI is handed never runs
/// ahead of the file.
fn save_change(
    state: &AppState,
    change: impl FnOnce(&mut Vec<ModProjectInfo>),
) -> Result<Vec<ModProjectInfo>, Error> {
    let mut bookmarks = state.bookmarks.lock().unwrap();
    let mut changed = bookmarks.clone();
    change(&mut changed);
    write_json(bookmarks_path(), &changed)?;
    *bookmarks = changed.clone();
    Ok(changed)
}

/// Put `project` first, newest bookmark on top. One already bookmarked moves up
/// and takes the fresher details, rather than appearing twice.
fn add(list: &mut Vec<ModProjectInfo>, project: ModProjectInfo) {
    remove(list, &project.id);
    list.insert(0, project);
}

fn remove(list: &mut Vec<ModProjectInfo>, id: &ProjectId) {
    list.retain(|project| project.id != *id);
}

#[cfg(test)]
mod tests {
    use super::{add, remove};
    use yaminabe_launcher_shared::datamodels::{ModProjectInfo, ProjectId};

    fn project(id: ProjectId, name: &str) -> ModProjectInfo {
        ModProjectInfo {
            id,
            name: name.to_string(),
            summary: String::new(),
            logo_url: None,
            download_count: 0,
            game_versions: Vec::new(),
            category: Vec::new(),
            primary_category_id: 0,
        }
    }

    fn names(list: &[ModProjectInfo]) -> Vec<&str> {
        list.iter().map(|p| p.name.as_str()).collect()
    }

    /// The newest bookmark goes on top, and bookmarking one again moves it up
    /// with its fresher details instead of listing it twice.
    #[test]
    fn adds_newest_first_without_duplicates() {
        let mut list = Vec::new();
        add(&mut list, project(ProjectId::CurseForge(1), "Old"));
        add(&mut list, project(ProjectId::Modrinth("abc".to_string()), "Other"));
        add(&mut list, project(ProjectId::CurseForge(1), "Renamed"));

        assert_eq!(names(&list), vec!["Renamed", "Other"]);
    }

    /// The same number on two platforms names two projects, so removing one
    /// leaves the other.
    #[test]
    fn removes_by_project_and_platform() {
        let mut list = vec![
            project(ProjectId::CurseForge(7), "CurseForge"),
            project(ProjectId::Modrinth("7".to_string()), "Modrinth"),
        ];
        remove(&mut list, &ProjectId::CurseForge(7));

        assert_eq!(names(&list), vec!["Modrinth"]);
    }
}
