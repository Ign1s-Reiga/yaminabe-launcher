use crate::components::modal::install_modpack_modal::{InstallModpackModal, InstallState};
use crate::components::project_search::ProjectSearch;
use crate::curseforge::{InstallModpackArgs, call_install_modpack, call_list_project_files};
use crate::ipc;
use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{IntoView, component, view, web_sys};
use wasm_bindgen::JsCast;
use yaminabe_launcher_shared::datamodels::{AppSettings, ModProjectInfo, ProjectFileTarget};

const PAGE_SIZE: usize = 50;

/// A modpack picked somewhere other than this page — the Home page's popular
/// strip — waiting for the Search page to open its install dialog. Carried
/// through context rather than a route param so the whole `ModProjectInfo`
/// survives, sparing a second lookup.
#[derive(Clone, Copy)]
pub struct PendingInstall(pub RwSignal<Option<ModProjectInfo>>);

#[component]
pub fn SearchPage() -> impl IntoView {
    let install: RwSignal<Option<InstallState>> = RwSignal::new(None);
    let install_name: RwSignal<String> = RwSignal::new(String::new());
    let default_location: RwSignal<String> = RwSignal::new(String::new());
    let pending = use_context::<PendingInstall>().expect("pending install context");

    leptos::task::spawn_local(async move {
        if let Ok(s) = ipc::call_noargs::<AppSettings>("get_settings").await {
            default_location.set(s.instance_install_dir);
        }
    });

    let open_install = move |pack: ModProjectInfo| {
        install_name.set(pack.name.clone());
        let project_id = pack.id.clone();
        install.set(Some(InstallState {
            pack,
            version: String::new(),
            versions: vec![],
            versions_loading: true,
            versions_error: None,
            versions_done: false,
        }));
        leptos::task::spawn_local(async move {
            match call_list_project_files(project_id, ProjectFileTarget::Modpack, None, None, 0).await
            {
                Ok(files) => {
                    let first_version = files
                        .first()
                        .and_then(|f| f.source.version_key())
                        .unwrap_or_default();
                    install.update(|opt| {
                        if let Some(s) = opt {
                            s.version = first_version;
                            s.versions_done = files.len() < PAGE_SIZE;
                            s.versions = files;
                            s.versions_loading = false;
                        }
                    });
                }
                Err(e) => {
                    install.update(|opt| {
                        if let Some(s) = opt {
                            s.versions_loading = false;
                            s.versions_error = Some(e);
                        }
                    });
                }
            }
        });
    };

    // A pack picked on the Home page: open its dialog once, then clear it so a
    // later visit to Search does not reopen it.
    Effect::new(move |_| {
        if let Some(pack) = pending.0.get() {
            pending.0.set(None);
            open_install(pack);
        }
    });

    let close_install = move || install.set(None);

    let load_more_versions = move || {
        let Some(state) = install.get_untracked() else {
            return;
        };
        if state.versions_loading || state.versions_done {
            return;
        }
        let project_id = state.pack.id.clone();
        let index = state.versions.len() as u32;
        install.update(|opt| {
            if let Some(s) = opt {
                s.versions_loading = true;
                s.versions_error = None;
            }
        });
        leptos::task::spawn_local(async move {
            match call_list_project_files(project_id, ProjectFileTarget::Modpack, None, None, index)
                .await
            {
                Ok(mut files) => {
                    install.update(|opt| {
                        if let Some(s) = opt {
                            s.versions_done = files.len() < PAGE_SIZE;
                            s.versions.append(&mut files);
                            s.versions_loading = false;
                        }
                    });
                }
                Err(e) => {
                    install.update(|opt| {
                        if let Some(s) = opt {
                            s.versions_error = Some(e);
                            s.versions_loading = false;
                        }
                    });
                }
            }
        });
    };

    // ── install form submit ───────────────────────────────────────────────────
    let on_install = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let Some(state) = install.get_untracked() else {
            return;
        };
        if state.versions_loading {
            return;
        }
        let Some(form) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlFormElement>().ok())
        else {
            return;
        };
        let Ok(data) = web_sys::FormData::new_with_form(&form) else {
            return;
        };

        // The backend reads the install dir from settings now; still gate on a
        // configured location so we don't kick off an install with none set.
        if default_location.get_untracked().trim().is_empty() {
            return;
        }

        let version_key = data.get("version").as_string().unwrap_or_default();
        let Some(ver) = state
            .versions
            .into_iter()
            .find(|v| v.source.version_key().as_deref() == Some(version_key.as_str()))
        else {
            return;
        };
        let source = ver.source;

        let Some(args) = InstallModpackArgs::from_form_data(source, &data) else {
            return;
        };
        install.set(None);

        leptos::task::spawn_local(async move {
            // Install progress flows through the install sidebar's event
            // stream; synchronous IPC failures don't, so log them here.
            if let Err(e) = call_install_modpack(args).await {
                log::error!("install_modpack failed: {e}");
            }
        });
    };

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
            on_select=Callback::new(move |p: ModProjectInfo| open_install(p))
        />

        <Show when=move || install.get().is_some()>
            <InstallModpackModal
                install=install
                install_name=install_name
                on_submit=Callback::new(on_install)
                on_load_more=Callback::new(move |_: ()| load_more_versions())
                on_close=Callback::new(move |_: ()| close_install())
            />
        </Show>
      </div>
    }
}