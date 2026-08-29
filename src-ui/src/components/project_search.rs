use crate::components::card::result_card::ResultCard;
use crate::components::pagination::Pagination;
use crate::components::ui::*;
use crate::curseforge::call_search_projects;
use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{IntoView, component, view, web_sys};
use yaminabe_launcher_shared::datamodels::{
    ModLoader, ModProjectInfo, Platform, ProjectFileTarget, ProjectSortField, SearchOptions,
};

const PAGE_SIZE: usize = 50;

const CURSEFORGE_LOGO: &str = include_str!("../../assets/curse-logo.svg");
const MODRINTH_LOGO: &str = include_str!("../../assets/Modrinth_icon_light.svg");

#[derive(Clone, Default)]
struct SearchQuery {
    query: String,
    page: usize,
}

#[derive(Clone, Default)]
struct SearchState {
    is_loading: bool,
    error: Option<String>,
    results: Vec<ModProjectInfo>,
    total: u32,
}

/// Reusable CurseForge project search: the search bar, the paginated result
/// cards, and the search state machine. The caller picks the project class via
/// `target` (and, for mods, narrows by `game_version`/`mod_loader`); a chosen
/// result is handed back through `on_select`. Its root fills its parent as a
/// flex column, so place it inside a flex-column container with a bounded
/// height (a page area or a fixed-height modal section).
#[component]
pub fn ProjectSearch(
    target: ProjectFileTarget,
    #[prop(optional)] game_version: Option<String>,
    #[prop(optional)] mod_loader: Option<ModLoader>,
    #[prop(into)] placeholder: String,
    #[prop(into, default = "Install".to_string())] action_label: String,
    #[prop(into)] empty_message: String,
    /// Run a search (with whatever query is present, including empty → browse)
    /// as soon as the component mounts, instead of showing the idle prompt.
    #[prop(optional)] search_on_open: bool,
    on_select: Callback<ModProjectInfo>,
) -> impl IntoView {
    let game_version = StoredValue::new(game_version);
    let mod_loader = StoredValue::new(mod_loader);
    let action_label = StoredValue::new(action_label);
    let empty_message = StoredValue::new(empty_message);

    let search_input: RwSignal<String> = RwSignal::new(String::new());
    let search_query: RwSignal<SearchQuery> = RwSignal::new(SearchQuery::default());
    let search_state: RwSignal<SearchState> = RwSignal::new(SearchState::default());
    let sort: RwSignal<ProjectSortField> = RwSignal::new(ProjectSortField::default());
    let platform: RwSignal<Platform> = RwSignal::new(Platform::default());
    let request_id = StoredValue::new(0u64);
    let results_wrapper_ref: NodeRef<leptos::html::Div> = NodeRef::new();

    // Reset the scroll position whenever the active query/page changes so the
    // user lands at the top of the new result set.
    Effect::new(move |_| {
        search_query.track();
        if let Some(el) = results_wrapper_ref.get() {
            el.set_scroll_top(0);
        }
    });

    Effect::new(move |_| {
        let q = search_query.get();
        // Bump on every run so an in-flight request whose platform/sort no
        // longer matches can be dropped once it resolves.
        request_id.update_value(|n| *n += 1);
        let this_request = request_id.get_value();
        if q.query.is_empty() && !search_on_open {
            search_state.set(SearchState::default());
            return;
        }
        search_state.update(|s| {
            s.is_loading = true;
            s.error = None;
        });
        let option = SearchOptions {
            query: q.query,
            index: (q.page * PAGE_SIZE) as u32,
            target,
            game_version: game_version.get_value(),
            mod_loader: mod_loader.get_value(),
            // Read untracked: sort/platform changes drive the re-search by
            // resetting the query page below, so this effect keys only on
            // `search_query`.
            sort: sort.get_untracked(),
            platform: platform.get_untracked(),
        };
        leptos::task::spawn_local(async move {
            let result = call_search_projects(option).await;
            // A newer search superseded this one; drop the stale response so it
            // can't overwrite the current platform's results.
            if request_id.get_value() != this_request {
                return;
            }
            match result {
                Ok(data) => search_state.update(|s| {
                    s.total = data.total;
                    s.results = data.items;
                    s.is_loading = false;
                }),
                Err(e) => search_state.update(|s| {
                    s.error = Some(e);
                    s.is_loading = false;
                }),
            }
        });
    });

    let do_search = move || {
        let q = search_input.get_untracked();
        search_query.set(SearchQuery { query: q, page: 0 });
    };

    let last_page: Signal<usize> = Signal::derive(move || {
        let total = search_state.get().total as usize;
        if total == 0 { 0 } else { (total - 1) / PAGE_SIZE }
    });
    let current_page: Signal<usize> = Signal::derive(move || search_query.get().page);
    let is_loading: Signal<bool> = Signal::derive(move || search_state.get().is_loading);

    let root = css! {
        display: flex;
        flex-direction: column;
        flex: 1 1 auto;
        min-height: 0;
    };
    let search_bar = css! {
        display: flex;
        gap: 10px;
        margin-bottom: 24px;
        flex-shrink: 0;
    };
    // Platform split button: two joined segments, each carrying a logo that
    // inherits the segment's text color (the SVGs use `currentColor`).
    // Grid columns rather than flex: the two logos have different aspect ratios,
    // so equal-width columns keep the segments from sizing to their own artwork.
    let platform_switch = css! {
        display: grid;
        grid-auto-flow: column;
        grid-auto-columns: 1fr;
        flex-shrink: 0;
        border: 1px solid var(--secondary-color);
        border-radius: 8px;
        overflow: hidden;
    };
    let platform_seg = css! {
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 8px 12px;
        background: none;
        border: none;
        cursor: pointer;
        color: var(--text-color);
        transition: background-color 0.15s ease;
        &:hover { background-color: var(--secondary-color); }
    };
    let platform_seg_active = css! {
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 8px 12px;
        background-color: #3a9e5f;
        border: none;
        cursor: pointer;
        color: white;
    };
    let platform_logo = css! {
        display: flex;
        height: 16px;
        & svg { height: 100%; width: auto; display: block; }
    };
    let seg_class = move |p: Platform| if platform.get() == p { platform_seg_active } else { platform_seg };
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
      <div class=root>
        <div class=search_bar>
            <div class=platform_switch>
                <button
                    type="button"
                    title="CurseForge"
                    class=move || seg_class(Platform::CurseForge)
                    on:click=move |_| {
                        platform.set(Platform::CurseForge);
                        search_query.update(|q| q.page = 0);
                    }
                >
                    <span class=platform_logo inner_html=CURSEFORGE_LOGO></span>
                </button>
                <button
                    type="button"
                    title="Modrinth"
                    class=move || seg_class(Platform::Modrinth)
                    on:click=move |_| {
                        platform.set(Platform::Modrinth);
                        search_query.update(|q| q.page = 0);
                    }
                >
                    <span class=platform_logo inner_html=MODRINTH_LOGO></span>
                </button>
            </div>
            <input
                class=input_class()
                style="flex: 1; width: auto;"
                type="text"
                placeholder=placeholder
                prop:value=move || search_input.get()
                on:input=move |ev| search_input.set(event_target_value(&ev))
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    if ev.key() == "Enter" { do_search(); }
                }
            />
            <Dropdown label=Signal::derive(move || sort.get().to_string())>
                {move || ProjectSortField::all().into_iter().map(|field| {
                    view! {
                        <DropdownItem on_select=Callback::new(move |_| {
                            sort.set(field);
                            search_query.update(|q| q.page = 0);
                        })>
                            {field.to_string()}
                        </DropdownItem>
                    }
                }).collect_view()}
            </Dropdown>
            <Button variant=ButtonVariant::Primary on_click=Callback::new(move |_| do_search())>
                "Search"
            </Button>
        </div>

        {move || {
            let s = search_state.get();
            let q = search_query.get();
            if s.is_loading {
                view! { <div class=status_area>"Searching…"</div> }.into_any()
            } else if q.query.is_empty() && !search_on_open {
                view! {
                    <div class=status_area>
                        <div style="font-size: 2.5rem; opacity: 0.8;">"🔍"</div>
                        "Type a name above and press Search to begin."
                    </div>
                }.into_any()
            } else if let Some(e) = s.error {
                view! { <div class=status_area>{e}</div> }.into_any()
            } else if s.results.is_empty() {
                view! { <div class=status_area>{empty_message.get_value()}</div> }.into_any()
            } else {
                ().into_any()
            }
        }}

        <Show when=move || !search_state.get().results.is_empty() fallback=|| ()>
            <div class=results_wrapper node_ref=results_wrapper_ref>
                <div class=results_list>
                    {move || search_state.get().results.into_iter().map(|pack| {
                        view! {
                            <ResultCard
                                pack=pack
                                action_label=action_label.get_value()
                                on_select=on_select
                            />
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
      </div>
    }
}