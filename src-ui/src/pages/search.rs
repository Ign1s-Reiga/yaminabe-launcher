use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, IntoView, view, web_sys};
use wasm_bindgen::JsCast;
use phosphor_leptos::{Icon, IconWeight, CARET_LEFT, CARET_RIGHT};
use yaminabe_launcher_shared::datatypes::AppSettings;
use crate::components::install_modpack_modal::{InstallModpackModal, InstallState};
use crate::components::ui::*;
use crate::curseforge::{call_get_files, call_install, call_search, fmt_downloads, InstallArgs, ModpackInfo};
use crate::ipc;

const PAGE_SIZE: usize = 20;

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
    has_next: bool,
    last_page: Option<usize>,
}

// ── Component ─────────────────────────────────────────────────────────────────

#[component]
pub fn SearchPage() -> impl IntoView {
    let search_input: RwSignal<String> = RwSignal::new(String::new());
    let search_query: RwSignal<SearchQuery> = RwSignal::new(SearchQuery::default());
    let search_state: RwSignal<SearchState> = RwSignal::new(SearchState::default());
    let install: RwSignal<Option<InstallState>> = RwSignal::new(None);
    let install_name: RwSignal<String> = RwSignal::new(String::new());
    let default_location: RwSignal<String> = RwSignal::new(String::new());

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
            s.has_next = false;
            s.error = None;
        });
        let index = (q.page * PAGE_SIZE) as u32;
        leptos::task::spawn_local(async move {
            match call_search(q.query, index).await {
                Ok(data) => {
                    let is_last = data.len() < PAGE_SIZE;
                    search_state.update(|s| {
                        s.has_next = !is_last;
                        if is_last {
                            s.last_page = Some(q.page);
                        }
                        s.results = data;
                        s.is_loading = false;
                    });
                }
                Err(e) => {
                    search_state.update(|s| {
                        s.has_next = false;
                        s.error = Some(e);
                        s.is_loading = false;
                    });
                }
            }
        });
    });

    let do_search = move || {
        let q = search_input.get_untracked();
        search_state.update(|s| s.last_page = None);
        search_query.set(SearchQuery { query: q, page: 0 });
    };

    let prev_page = move || search_query.update(|q| q.page = q.page.saturating_sub(1));
    let next_page = move || search_query.update(|q| q.page += 1);

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

        let Some(args) = InstallArgs::from_form_data(install_dir, download_url, &data) else { return };
        install.set(None);

        leptos::task::spawn_local(async move {
            let _ = call_install(args).await;
        });
    };

    // ── pagination page list ──────────────────────────────────────────────────
    let page_items: Signal<Vec<Option<usize>>> = Signal::derive(move || {
        let state = search_state.get();
        let cur = search_query.get().page;

        let mut set = std::collections::BTreeSet::new();
        set.insert(0usize);
        if cur > 0 { set.insert(cur - 1); }
        set.insert(cur);
        if state.has_next { set.insert(cur + 1); }
        if let Some(l) = state.last_page { set.insert(l); }

        let mut result: Vec<Option<usize>> = vec![];
        let mut prev: Option<usize> = None;
        for p in set {
            if let Some(pp) = prev {
                if p == pp + 2 {
                    result.push(Some(pp + 1));
                } else if p > pp + 1 {
                    result.push(None);
                }
            }
            result.push(Some(p));
            prev = Some(p);
        }
        result
    });

    // ── page root: flex column that fills MainView's content area ────────────
    let page_root = css! {
        display: flex;
        flex-direction: column;
        height: 100%;
        overflow: hidden;
    };

    // ── search bar styles ─────────────────────────────────────────────────────
    let search_bar = css! {
        display: flex;
        gap: 10px;
        margin-bottom: 24px;
        flex-shrink: 0;
    };

    // ── status / empty-state styles ───────────────────────────────────────────
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

    // ── scrollable results section ────────────────────────────────────────────
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

    // ── result card styles ────────────────────────────────────────────────────
    let card = css! {
        display: flex;
        align-items: center;
        gap: 16px;
        padding: 14px 16px;
        border-radius: 10px;
        border: 1.5px solid var(--secondary-color);
        transition: border-color 0.12s ease;
        &:hover { border-color: rgba(58, 158, 95, 0.4); }
    };
    let card_logo = css! {
        width: 64px;
        height: 64px;
        border-radius: 8px;
        object-fit: cover;
        flex-shrink: 0;
        background-color: var(--secondary-color);
    };
    let card_logo_ph = css! {
        width: 64px;
        height: 64px;
        border-radius: 8px;
        flex-shrink: 0;
        background-color: var(--secondary-color);
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 1.8rem;
    };
    let card_body = css! {
        flex: 1;
        min-width: 0;
    };
    let card_name = css! {
        font-weight: 600;
        font-size: 0.95rem;
        margin-bottom: 4px;
    };
    let card_summary = css! {
        font-size: 0.82rem;
        opacity: 0.6;
        display: -webkit-box;
        -webkit-line-clamp: 2;
        -webkit-box-orient: vertical;
        overflow: hidden;
        margin-bottom: 6px;
        line-height: 1.45;
    };
    let card_meta = css! {
        font-size: 0.76rem;
        opacity: 0.45;
    };

    // ── pagination styles ─────────────────────────────────────────────────────
    let pagination = css! {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 4px;
        padding-top: 16px;
        margin-top: 8px;
        border-top: 1px solid var(--secondary-color);
        flex-shrink: 0;
    };
    let page_btn = css! {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 32px;
        height: 32px;
        border-radius: 50%;
        border: 1.5px solid var(--secondary-color);
        background: none;
        color: inherit;
        font-size: 0.8rem;
        cursor: pointer;
        transition: border-color 0.12s ease, background-color 0.12s ease;
        &:hover:not(:disabled) {
            border-color: rgba(58, 158, 95, 0.6);
            background-color: var(--secondary-color);
        }
        &:disabled { opacity: 0.3; cursor: default; }
    };
    let page_btn_active = css! {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 32px;
        height: 32px;
        border-radius: 50%;
        border: 1.5px solid var(--text-color);
        background-color: var(--text-color);
        color: var(--background-color);
        font-size: 0.8rem;
        font-weight: 600;
        cursor: default;
    };
    let ellipsis_style = css! {
        width: 32px;
        text-align: center;
        font-size: 0.8rem;
        opacity: 0.4;
        user-select: none;
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
        <Show when=move || search_state.get().is_loading fallback=|| ()>
            <div class=status_area>"Searching…"</div>
        </Show>
        <Show when=move || !search_state.get().is_loading && search_query.get().query.is_empty() fallback=|| ()>
            <div class=status_area>
                <div style="font-size: 2.5rem; opacity: 0.8;">"🔍"</div>
                "Type a modpack name above and press Search to begin."
            </div>
        </Show>
        <Show
            when=move || {
                let s = search_state.get();
                !s.is_loading && !search_query.get().query.is_empty() && s.error.is_some()
            }
            fallback=|| ()
        >
            <div class=status_area>{move || search_state.get().error.unwrap_or_default()}</div>
        </Show>
        <Show
            when=move || {
                let s = search_state.get();
                !s.is_loading
                    && !search_query.get().query.is_empty()
                    && s.results.is_empty()
                    && s.error.is_none()
            }
            fallback=|| ()
        >
            <div class=status_area>"No modpacks found."</div>
        </Show>

        // ── scrollable result cards ───────────────────────────────────────────
        <Show when=move || !search_state.get().results.is_empty() fallback=|| ()>
            <div class=results_wrapper>
                <div class=results_list>
                    {move || search_state.get().results.into_iter().map(|pack| {
                        let pack_btn = pack.clone();
                        view! {
                            <div class=card>
                                {if let Some(ref url) = pack.logo_url {
                                    let url = url.clone();
                                    view! { <img class=card_logo src=url alt=""/> }.into_any()
                                } else {
                                    view! { <div class=card_logo_ph>"📦"</div> }.into_any()
                                }}
                                <div class=card_body>
                                    <div class=card_name>{pack.name.clone()}</div>
                                    <div class=card_summary>{pack.summary.clone()}</div>
                                    <div class=card_meta>
                                        {format!(
                                            "{} downloads{}",
                                            fmt_downloads(pack.download_count),
                                            pack.game_versions.last()
                                                .map(|v| format!(" · {v}"))
                                                .unwrap_or_default()
                                        )}
                                    </div>
                                </div>
                                <Button
                                    variant=ButtonVariant::Primary
                                    on_click=Callback::new(move |_| open_install(pack_btn.clone()))
                                >
                                    "Install"
                                </Button>
                            </div>
                        }
                    }).collect_view()}
                </div>
            </div>
        </Show>

        // ── pagination (always rendered to hold space, hidden when not needed) ──
        <div
            class=pagination
            style=move || {
                let q = search_query.get();
                let s = search_state.get();
                if q.page == 0 && !s.has_next { "visibility: hidden;" } else { "visibility: visible;" }
            }
        >
            <button
                class=page_btn
                disabled=move || {
                    let q = search_query.get();
                    q.page == 0 || search_state.get().is_loading
                }
                on:click=move |_| prev_page()
            >
                <Icon icon=CARET_LEFT size="18px" weight=IconWeight::Bold />
            </button>

            {move || {
                let cur = search_query.get().page;
                let is_loading = search_state.get().is_loading;
                page_items.get().into_iter().map(|item| {
                    match item {
                        None => view! {
                            <span class=ellipsis_style>"…"</span>
                        }.into_any(),
                        Some(p) => {
                            let is_active = p == cur;
                            view! {
                                <button
                                    class=if is_active { page_btn_active } else { page_btn }
                                    disabled=is_active || is_loading
                                    on:click=move |_| search_query.update(|q| q.page = p)
                                >
                                    {p + 1}
                                </button>
                            }.into_any()
                        }
                    }
                }).collect_view()
            }}

            <button
                class=page_btn
                disabled=move || {
                    let s = search_state.get();
                    !s.has_next || s.is_loading
                }
                on:click=move |_| next_page()
            >
                <Icon icon=CARET_RIGHT size="18px" weight=IconWeight::Bold />
            </button>
        </div>
        
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
