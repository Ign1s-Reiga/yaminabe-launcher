use crate::bookmarks::Bookmarks;
use crate::components::card::result_card::ResultCard;
use crate::components::modal::create_modal::CreateInstanceModal;
use crate::components::ui::*;
use crate::pages::search::PendingInstall;
use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use leptos_router::hooks::use_navigate;
use phosphor_leptos::{Icon, IconData, IconWeight, BOOKMARK_SIMPLE, BOOKS};
use yaminabe_launcher_shared::datamodels::{InstanceMeta, ModProjectInfo};
use crate::components::card::instance_card::InstanceCard;

fn matches_active_category(inst: &InstanceMeta, active: &str) -> bool {
    if active == "Default" { inst.category.is_empty() } else { inst.category == active }
}

/// What the Library page shows, picked from the switch at its side.
#[derive(Clone, Copy, PartialEq)]
enum LibraryView {
    Instances,
    Bookmarks,
}

impl LibraryView {
    const ALL: [LibraryView; 2] = [LibraryView::Instances, LibraryView::Bookmarks];

    fn label(self) -> &'static str {
        match self {
            LibraryView::Instances => "Library",
            LibraryView::Bookmarks => "Bookmarks",
        }
    }

    fn icon(self) -> IconData {
        match self {
            LibraryView::Instances => BOOKS,
            LibraryView::Bookmarks => BOOKMARK_SIMPLE,
        }
    }
}

#[component]
pub fn LibraryPage() -> impl IntoView {
    // ── instance state ─────────────────────────────────────────────────────
    let instances = use_context::<RwSignal<Vec<InstanceMeta>>>().expect("instances context");
    let refresh   = use_context::<RwSignal<u32>>().expect("refresh context");

    // ── modal state ────────────────────────────────────────────────────────
    let show_create_modal = RwSignal::new(false);
    let pending_instances: RwSignal<Vec<InstanceMeta>> = RwSignal::new(vec![]);

    // ── category state ─────────────────────────────────────────────────────
    let extra_categories: RwSignal<Vec<String>> = RwSignal::new(vec![]);
    let active_category: RwSignal<String> = RwSignal::new("Default".to_string());
    let show_add_category_modal = RwSignal::new(false);
    let new_category_name: RwSignal<String> = RwSignal::new(String::new());

    let view_mode = RwSignal::new(LibraryView::Instances);
    let bookmarks = Bookmarks::load();

    // ── derived tab list ───────────────────────────────────────────────────
    let all_tabs = Signal::derive(move || {
        let mut v = vec!["Default".to_string()];
        for inst in instances.get() {
            if !inst.category.is_empty() && !v.contains(&inst.category) {
                v.push(inst.category.clone());
            }
        }
        for cat in extra_categories.get() {
            if !v.contains(&cat) {
                v.push(cat);
            }
        }
        v
    });

    // ── styles ─────────────────────────────────────────────────────────────
    let grid = css! {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
        gap: 16px;
        margin-top: 16px;
    };
    let dialog_title = css! {
        margin: 0 0 8px 0;
        font-size: 1.1rem;
        font-weight: 600;
    };
    // The page's content, with the view switch in a column at its right. At
    // least the page's full height, so a short view leaves the switch room to
    // sit in the middle.
    let page_grid = css! {
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto;
        gap: 0 24px;
        align-items: start;
        min-height: 100%;
    };
    let title = move || format!("# {}", view_mode.get().label());

    view! {
        <div class=page_grid>
            <div>
                <h1>{title}</h1>
                <Show
                    when=move || view_mode.get() == LibraryView::Instances
                    fallback=move || view! { <BookmarkList bookmarks=bookmarks /> }
                >
                    <Button
                        variant=ButtonVariant::Primary
                        size=ButtonSize::Big
                        style="margin-bottom: 24px;"
                        on_click=Callback::new(move |_| show_create_modal.set(true))
                    >
                        "+ Create New Instance"
                    </Button>

                    // Category tab bar
                    <TabBar
                        tabs=all_tabs
                        active=active_category
                        allow_add=true
                        on_add=Callback::new(move |_| show_add_category_modal.set(true))
                    />

                    <div class=grid>
                        <For
                            each=move || {
                                let active = active_category.get();
                                instances.get().into_iter()
                                    .filter(|inst| matches_active_category(inst, &active))
                                    .collect::<Vec<_>>()
                            }
                            key=|v| v.id.clone()
                            let(inst)
                        >
                            <InstanceCard instance=inst />
                        </For>
                        <For
                            each=move || {
                                let active = active_category.get();
                                pending_instances.get().into_iter()
                                    .filter(|inst| matches_active_category(inst, &active))
                                    .collect::<Vec<_>>()
                            }
                            key=|v| v.id.clone()
                            let(inst)
                        >
                            <InstanceCard instance=inst pending=true />
                        </For>
                    </div>
                </Show>
            </div>
            <ViewSwitch selected=view_mode />
        </div>

        // ── create instance modal ─────────────────────────────────────────
        <CreateInstanceModal
            show=show_create_modal
            on_creating=Callback::new(move |inst: InstanceMeta| pending_instances.update(|v| v.push(inst)))
            on_created=Callback::new(move |name: String| {
                pending_instances.update(|v| v.retain(|i| i.name != name));
                refresh.update(|n| *n += 1);
            })
        />

        // ── add category modal ────────────────────────────────────────────
        <Show when=move || show_add_category_modal.get()>
            <ModalOverlay>
                <DialogBox>
                    <div>
                        <p class=dialog_title>"Add Category"</p>
                    </div>
                    <input
                        class=input_class()
                        type="text"
                        placeholder="Category name"
                        prop:value=move || new_category_name.get()
                        on:input=move |ev| new_category_name.set(event_target_value(&ev))
                    />
                    <DialogFooter>
                        <Button
                            variant=ButtonVariant::Secondary
                            on_click=Callback::new(move |_| {
                                new_category_name.set(String::new());
                                show_add_category_modal.set(false);
                            })
                        >
                            "Cancel"
                        </Button>
                        <Button
                            variant=ButtonVariant::Primary
                            on_click=Callback::new(move |_| {
                                let name = new_category_name.get_untracked().trim().to_string();
                                if !name.is_empty() && name != "Default" {
                                    extra_categories.update(|cats| {
                                        if !cats.iter().any(|c| c == &name) {
                                            let new_cat = name.clone();
                                            cats.push(name);
                                            active_category.set(new_cat);
                                        }
                                    });
                                }
                                new_category_name.set(String::new());
                                show_add_category_modal.set(false);
                            })
                        >
                            "Add"
                        </Button>
                    </DialogFooter>
                </DialogBox>
            </ModalOverlay>
        </Show>
    }
}

/// A vertical switch between the page's views: their icons stacked in a pill,
/// with an indicator that slides to the selected one.
#[component]
fn ViewSwitch(selected: RwSignal<LibraryView>) -> impl IntoView {
    // Held in the middle of the visible page, however far it scrolls.
    let switch = css! {
        position: sticky;
        top: 50%;
        transform: translateY(-50%);
        display: flex;
        flex-direction: column;
        gap: 4px;
        padding: 4px;
        border-radius: 10px;
        background-color: var(--secondary-color);
    };
    let indicator = css! {
        position: absolute;
        top: 4px;
        left: 4px;
        width: 40px;
        height: 40px;
        border-radius: 8px;
        background-color: #3a9e5f;
        transition: transform 0.22s cubic-bezier(0.4, 0, 0.2, 1);
        pointer-events: none;
    };
    let option = css! {
        position: relative;
        display: flex;
        align-items: center;
        justify-content: center;
        width: 40px;
        height: 40px;
        padding: 0;
        border: none;
        border-radius: 8px;
        background: none;
        color: inherit;
        cursor: pointer;
        opacity: 0.6;
        transition: color 0.15s ease, opacity 0.15s ease;
        &:hover { opacity: 1; }
        &[aria-pressed="true"] { color: white; opacity: 1; }
    };

    // Each option is 40px tall with a 4px gap, so the indicator steps by 44px.
    let offset = move || format!("translateY({}px)", selected.get() as usize * 44);
    let options = LibraryView::ALL.into_iter().map(|mode| {
        let weight = Signal::derive(move || {
            if selected.get() == mode { IconWeight::Fill } else { IconWeight::Regular }
        });
        view! {
            <button
                type="button"
                class=option
                title=mode.label()
                aria-label=mode.label()
                aria-pressed=move || (selected.get() == mode).to_string()
                on:click=move |_| selected.set(mode)
            >
                <Icon icon=mode.icon() size="22px" weight=weight />
            </button>
        }
    }).collect_view();

    view! {
        <div class=switch role="group" aria-label="Library view">
            <div class=indicator style:transform=offset></div>
            {options}
        </div>
    }
}

/// The bookmarked modpacks, newest first. Installing one hands it to the Search
/// page's install dialog, as the Home page's popular strip does.
#[component]
fn BookmarkList(bookmarks: Bookmarks) -> impl IntoView {
    let pending = use_context::<PendingInstall>().expect("pending install context");
    let navigate = StoredValue::new(use_navigate());
    let on_select = Callback::new(move |pack: ModProjectInfo| {
        pending.0.set(Some(pack));
        navigate.with_value(|nav| nav("/search", Default::default()));
    });

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
    let list = css! {
        display: flex;
        flex-direction: column;
        gap: 10px;
    };

    let render = move |pack: ModProjectInfo| {
        view! { <ResultCard pack=pack bookmarks=Some(bookmarks) show_platform=true on_select=on_select /> }
    };

    view! {
        <Show
            when=move || !bookmarks.is_empty()
            fallback=move || view! {
                <div class=status_area>
                    <Icon icon=BOOKMARK_SIMPLE size="40px" />
                    "No bookmarks yet. Bookmark a modpack in the search results to keep it here."
                </div>
            }
        >
            <div class=list>
                <For each=move || bookmarks.list() key=|pack| pack.id.clone() children=render />
            </div>
        </Show>
    }
}
