use leptos::prelude::*;
use phosphor_leptos::WARNING;
use serde::Serialize;
use yaminabe_launcher_shared::datamodels::{ModProjectInfo, ProjectId};

use crate::components::ui::Toasts;
use crate::ipc;

#[derive(Serialize)]
struct AddBookmarkArgs {
    project: ModProjectInfo,
}

#[derive(Serialize)]
struct RemoveBookmarkArgs {
    id: ProjectId,
}

/// The bookmarked modpacks, newest first, as the backend last saved them.
#[derive(Clone, Copy)]
pub struct Bookmarks {
    list: RwSignal<Vec<ModProjectInfo>>,
    toasts: Option<Toasts>,
}

impl Bookmarks {
    /// Start empty and fill in from the backend.
    pub fn load() -> Self {
        let bookmarks = Bookmarks { list: RwSignal::new(Vec::new()), toasts: use_context::<Toasts>() };
        leptos::task::spawn_local(async move {
            match ipc::call_noargs::<Vec<ModProjectInfo>>("get_bookmarks").await {
                Ok(list) => bookmarks.list.set(list),
                Err(e) => log::error!("get_bookmarks failed: {e}"),
            }
        });
        bookmarks
    }

    pub fn list(self) -> Vec<ModProjectInfo> {
        self.list.get()
    }

    pub fn len(self) -> usize {
        self.list.with(Vec::len)
    }

    pub fn is_empty(self) -> bool {
        self.list.with(Vec::is_empty)
    }

    pub fn contains(self, id: &ProjectId) -> bool {
        self.list.with(|list| list.iter().any(|project| project.id == *id))
    }

    /// Bookmark `project`, or drop it if it already is. The list follows what
    /// the backend saved, so a save that fails changes nothing and says so.
    pub fn toggle(self, project: ModProjectInfo) {
        let bookmarked = self.list.with_untracked(|list| list.iter().any(|p| p.id == project.id));
        leptos::task::spawn_local(async move {
            let saved: Result<Vec<ModProjectInfo>, String> = if bookmarked {
                ipc::call("remove_bookmark", RemoveBookmarkArgs { id: project.id }).await
            } else {
                ipc::call("add_bookmark", AddBookmarkArgs { project }).await
            };
            match saved {
                Ok(list) => self.list.set(list),
                Err(e) => {
                    log::error!("saving a bookmark failed: {e}");
                    if let Some(toasts) = self.toasts {
                        toasts.push(WARNING, "Could not save the bookmark".to_string(), vec![e]);
                    }
                }
            }
        });
    }
}
