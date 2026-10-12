use crate::bookmarks::Bookmarks;
use crate::components::modal::install_modpack_modal::{ModpackInstallDialog, ModpackInstaller};
use crate::components::project_search::ProjectSearch;
use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{IntoView, component, view};
use phosphor_leptos::DOWNLOAD_SIMPLE;
use yaminabe_launcher_shared::datamodels::{ModProjectInfo, ProjectFileTarget};

/// A modpack picked somewhere other than this page — the Home page's popular
/// strip — waiting for the Search page to open its install dialog. Carried
/// through context rather than a route param so the whole `ModProjectInfo`
/// survives, sparing a second lookup.
#[derive(Clone, Copy)]
pub struct PendingInstall(pub RwSignal<Option<ModProjectInfo>>);

#[component]
pub fn SearchPage() -> impl IntoView {
    let installer = ModpackInstaller::new();
    let pending = use_context::<PendingInstall>().expect("pending install context");
    let bookmarks = Bookmarks::load();

    // A pack picked on the Home page: open its dialog once, then clear it so a
    // later visit to Search does not reopen it.
    Effect::new(move |_| {
        if let Some(pack) = pending.0.get() {
            pending.0.set(None);
            installer.open(pack);
        }
    });

    // ── page root: flex column that fills MainView's content area ────────────
    let page_root = css! {
        display: flex;
        flex-direction: column;
        height: 100%;
        overflow: hidden;
    };

    view! {
      <div class=page_root>
        <h1 style="margin: 0 0 8px 0; flex-shrink: 0;">"# Search"</h1>
        <h2 style="margin: 0 0 24px 0; font-size: 0.95rem; font-weight: 400; opacity: 0.55; flex-shrink: 0;">
            "Browse and install modpacks directly from CurseForge or Modrinth."
        </h2>

        <ProjectSearch
            target=ProjectFileTarget::Modpack
            placeholder="Search modpacks…"
            empty_message="No modpacks found."
            action_icon=DOWNLOAD_SIMPLE
            bookmarks=bookmarks
            on_select=Callback::new(move |p: ModProjectInfo| installer.open(p))
        />

        <ModpackInstallDialog installer=installer />
      </div>
    }
}
