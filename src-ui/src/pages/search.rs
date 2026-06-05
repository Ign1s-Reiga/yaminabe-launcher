use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, IntoView, view, web_sys};
use wasm_bindgen::JsCast;
use yaminabe_launcher_shared::datatypes::{AppSettings, ModpackInfo};
use crate::components::modal::install_modpack_modal::{InstallModpackModal, InstallState};
use crate::components::pagination::Pagination;
use crate::components::result_card::ResultCard;
use crate::components::ui::*;
use crate::curseforge::{call_get_files, call_install, call_search, InstallArgs};
use crate::ipc;

const PAGE_SIZE: usize = 50;

#[derive(Clone, Default)]
struct SearchQuery {
    query: String,
    page: usize,
}

#[derive(Clone, Default)]
struct SearchState {
    is_loading: bool,
    error: Option<String>,
    results: Vec<ModpackInfo>,
    total: u32,
}

#[component]
pub fn SearchPage() -> impl IntoView {
    let search_input: RwSignal<String> = RwSignal::new(String::new());
    let search_query: RwSignal<SearchQuery> = RwSignal::new(SearchQuery::default());
    let search_state: RwSignal<SearchState> = RwSignal::new(SearchState::default());
    let install: RwSignal<Option<InstallState>> = RwSignal::new(None);
    let install_name: RwSignal<String> = RwSignal::new(String::new());
    let default_location: RwSignal<String> = RwSignal::new(String::new());
    let results_wrapper_ref: NodeRef<leptos::html::Div> = NodeRef::new();

    // Reset the scroll position whenever the active query/page changes so
    // the user lands at the top of the new result set instead of inheriting
    // the previous page's scroll offset.
    Effect::new(move |_| {
        search_query.track();
        if let Some(el) = results_wrapper_ref.get() {
            el.set_scroll_top(0);
        }
    });

    leptos::task::spawn_local(async move {
        if let Ok(s) = ipc::call_noargs::<AppSettings>("get_settings").await {
            default_location.set(s.instance_install_dir);
        }
    });

    Effect::new(move |_| {
        let q = search_query.get();
        if q.query.is_empty() {
            search_state.set(SearchState::default());
            return;
        }
        search_state.update(|s| {
            s.is_loading = true;
            s.error = None;
        });
        let index = (q.page * PAGE_SIZE) as u32;
        leptos::task::spawn_local(async move {
            match call_search(q.query, index).await {
                Ok(data) => {
                    search_state.update(|s| {
                        s.total = data.total;
                        s.results = data.items;
                        s.is_loading = false;
                    });
                }
                Err(e) => {
                    search_state.update(|s| {
                        s.error = Some(e);
                        s.is_loading = false;
                    });
                }
            }
        });
    });

    let do_search = move || {
        let q = search_input.get_untracked();
        search_query.set(SearchQuery { query: q, page: 0 });
    };

    let open_install = move |pack: ModpackInfo| {
        install_name.set(pack.name.clone());
        let mod_id = pack.id;
        install.set(Some(InstallState {
            pack,
            version: String::new(),
            versions: vec![],
            versions_loading: true,
            versions_error: None,
        }));
        leptos::task::spawn_local(async move {
            match call_get_files(mod_id).await {
                Ok(files) => {
                    let first_version = files.first().map(|f| f.id.to_string()).unwrap_or_default();
                    install.update(|opt| {
                        if let Some(s) = opt {
                            s.version = first_version;
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

    let close_install = move || install.set(None);

    // ── install form submit ───────────────────────────────────────────────────
    let on_install = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let Some(state) = install.get_untracked() else { return };
        if state.versions_loading { return; }
        let Some(form) = ev.target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlFormElement>().ok())
        else { return };
        let Ok(data) = web_sys::FormData::new_with_form(&form) else { return };

        let install_dir = default_location.get_untracked();
        if install_dir.trim().is_empty() { return; }

        let version_id: u32 = data.get("version").as_string().unwrap_or_default().parse().unwrap_or(0);
        let Some(ver) = state.versions.into_iter().find(|v| v.id == version_id) else { return };
        let download_url = ver.download_url.clone();
        let project_id = ver.mod_id;
        let file_id = ver.id;

        let Some(args) = InstallArgs::from_form_data(install_dir, download_url, project_id, file_id, &data) else { return };
        install.set(None);

        leptos::task::spawn_local(async move {
            // Install progress flows through the install sidebar's event
            // stream; synchronous IPC failures don't, so log them here.
            if let Err(e) = call_install(args).await {
                log::error!("install_curseforge_modpack failed: {e}");
            }
        });
    };

    // ── pagination derived values ─────────────────────────────────────────────
    // `last_page` is 0 when the result set is empty, otherwise the index of
    // the last page (so a 50-item set with PAGE_SIZE=20 has last_page=2).
    let last_page: Signal<usize> = Signal::derive(move || {
        let total = search_state.get().total as usize;
        if total == 0 { 0 } else { (total - 1) / PAGE_SIZE }
    });
    let current_page: Signal<usize> = Signal::derive(move || search_query.get().page);
    let is_loading: Signal<bool> = Signal::derive(move || search_state.get().is_loading);

    // ── page root: flex column that fills MainView's content area ────────────
    let page_root = css! {
        display: flex;
        flex-direction: column;
        height: 100%;
        overflow: hidden;
    };

    let search_bar = css! {
        display: flex;
        gap: 10px;
        margin-bottom: 24px;
        flex-shrink: 0;
    };

    let status_area = css! {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        padding: 80px 0;
        gap: 10px;
        opacity: 0.5;
        font-size: 0.9rem;
        text-align: center;
    };

    let results_wrapper = css! {
        flex: 1;
        min-height: 0;
        overflow-y: auto;
        scrollbar-width: thin;
        scrollbar-color: darkgrey var(--background-color);
    };
    let results_list = css! {
        display: flex;
        flex-direction: column;
        gap: 10px;
    };

    view! {
      <div class=page_root>
        <h1 style="margin: 0 0 8px 0; flex-shrink: 0;">"# Search"</h1>
        <h2 style="margin: 0 0 24px 0; font-size: 0.95rem; font-weight: 400; opacity: 0.55; flex-shrink: 0;">
            "Browse and install modpacks directly from CurseForge."
        </h2>

        // ── search bar ────────────────────────────────────────────────────────
        <div class=search_bar>
            <input
                class=input_class()
                style="flex: 1; width: auto;"
                type="text"
                placeholder="Search modpacks on CurseForge…"
                prop:value=move || search_input.get()
                on:input=move |ev| search_input.set(event_target_value(&ev))
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    if ev.key() == "Enter" { do_search(); }
                }
            />
            <Button variant=ButtonVariant::Primary on_click=Callback::new(move |_| do_search())>
                "Search"
            </Button>
        </div>

        // ── status messages (outside scroll area) ─────────────────────────────
        {move || {
            let s = search_state.get();
            let q = search_query.get();
            if s.is_loading {
                view! { <div class=status_area>"Searching…"</div> }.into_any()
            } else if q.query.is_empty() {
                view! {
                    <div class=status_area>
                        <div style="font-size: 2.5rem; opacity: 0.8;">"🔍"</div>
                        "Type a modpack name above and press Search to begin."
                    </div>
                }.into_any()
            } else if let Some(e) = s.error {
                view! { <div class=status_area>{e}</div> }.into_any()
            } else if s.results.is_empty() {
                view! { <div class=status_area>"No modpacks found."</div> }.into_any()
            } else {
                ().into_any()
            }
        }}

        // ── scrollable result cards ───────────────────────────────────────────
        <Show when=move || !search_state.get().results.is_empty() fallback=|| ()>
            <div class=results_wrapper node_ref=results_wrapper_ref>
                <div class=results_list>
                    {move || search_state.get().results.into_iter().map(|pack| {
                        view! {
                            <ResultCard pack=pack on_install=Callback::new(move |p| open_install(p)) />
                        }
                    }).collect_view()}
                </div>
            </div>
        </Show>

        <Pagination
            current=current_page
            last_page=last_page
            is_loading=is_loading
            on_change=Callback::new(move |p: usize| search_query.update(|q| q.page = p))
        />

        <Show when=move || install.get().is_some()>
            <InstallModpackModal
                install=install
                install_name=install_name
                on_submit=Callback::new(on_install)
                on_close=Callback::new(move |_: ()| close_install())
            />
        </Show>
      </div>
    }
}