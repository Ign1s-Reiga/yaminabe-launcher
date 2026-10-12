use crate::bookmarks::Bookmarks;
use crate::components::card::pack_tile::PackTile;
use crate::components::card::result_card::ResultCard;
use crate::components::modal::create_modal::CreateInstanceModal;
use crate::components::modal::install_modpack_modal::{ModpackInstallDialog, ModpackInstaller};
use crate::components::ui::*;
use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use phosphor_leptos::{Icon, IconData, IconWeight, BOOKMARK_SIMPLE, BOOKS, ROWS, SQUARES_FOUR};
use yaminabe_launcher_shared::datamodels::{InstanceMeta, ModProjectInfo};
use crate::components::card::instance_card::InstanceCard;

fn matches_active_category(inst: &InstanceMeta, active: &str) -> bool {
    if active == "Default" { inst.category.is_empty() } else { inst.category == active }
}

/// One choice an `IconSwitch` offers.
trait SwitchOption: Copy + PartialEq + Send + Sync + 'static {
    const ALL: &'static [Self];

    fn label(self) -> &'static str;

    fn icon(self) -> IconData;
}

/// What the Library page shows, picked from the switch at its side.
#[derive(Clone, Copy, PartialEq)]
enum LibraryView {
    Instances,
    Bookmarks,
}

impl SwitchOption for LibraryView {
    const ALL: &'static [Self] = &[LibraryView::Instances, LibraryView::Bookmarks];

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

/// How the bookmarks are laid out: full-width rows, or a grid of tiles.
#[derive(Clone, Copy, PartialEq)]
enum BookmarkLayout {
    List,
    Cards,
}

impl SwitchOption for BookmarkLayout {
    const ALL: &'static [Self] = &[BookmarkLayout::List, BookmarkLayout::Cards];

    fn label(self) -> &'static str {
        match self {
            BookmarkLayout::List => "List",
            BookmarkLayout::Cards => "Cards",
        }
    }

    fn icon(self) -> IconData {
        match self {
            BookmarkLayout::List => ROWS,
            BookmarkLayout::Cards => SQUARES_FOUR,
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
    // Kept here rather than in the list, so a trip to the instances and back
    // keeps the layout.
    let bookmark_layout = RwSignal::new(BookmarkLayout::List);

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
    // Held in the middle of the visible page, however far it scrolls.
    let switch_rail = css! {
        position: sticky;
        top: 50%;
        transform: translateY(-50%);
    };
    let title = move || format!("# {}", view_mode.get().label());

    view! {
        <div class=page_grid>
            <div>
                <h1>{title}</h1>
                <Show
                    when=move || view_mode.get() == LibraryView::Instances
                    fallback=move || view! { <BookmarkList bookmarks=bookmarks layout=bookmark_layout /> }
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
            <div class=switch_rail>
                <IconSwitch selected=view_mode />
            </div>
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

/// A pill of icon buttons, one per option, with an indicator that slides to
/// the selected one. The options stack vertically unless `horizontal` is set.
#[component]
fn IconSwitch<T: SwitchOption>(
    selected: RwSignal<T>,
    #[prop(optional)] horizontal: bool,
) -> impl IntoView {
    let switch = css! {
        position: relative;
        display: flex;
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

    let direction = if horizontal { "row" } else { "column" };
    // Each option is 40px across with a 4px gap, so the indicator steps by 44px.
    let offset = move || {
        let index = T::ALL.iter().position(|&choice| choice == selected.get()).unwrap_or(0);
        let axis = if horizontal { "X" } else { "Y" };
        format!("translate{axis}({}px)", index * 44)
    };
    let options = T::ALL.iter().copied().map(|choice| {
        let weight = Signal::derive(move || {
            if selected.get() == choice { IconWeight::Fill } else { IconWeight::Regular }
        });
        view! {
            <button
                type="button"
                class=option
                title=choice.label()
                aria-label=choice.label()
                aria-pressed=move || (selected.get() == choice).to_string()
                on:click=move |_| selected.set(choice)
            >
                <Icon icon=choice.icon() size="22px" weight=weight />
            </button>
        }
    }).collect_view();

    view! {
        <div class=switch style:flex-direction=direction role="group">
            <div class=indicator style:transform=offset></div>
            {options}
        </div>
    }
}

/// The bookmarked modpacks, newest first, as rows or as tiles, each opening the
/// install dialog here.
#[component]
fn BookmarkList(bookmarks: Bookmarks, layout: RwSignal<BookmarkLayout>) -> impl IntoView {
    let installer = ModpackInstaller::new();
    let install = Callback::new(move |pack: ModProjectInfo| installer.open(pack));

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
    let toolbar = css! {
        display: flex;
        align-items: center;
        justify-content: space-between;
        margin-bottom: 16px;
    };
    let count = css! {
        font-size: 0.9rem;
        opacity: 0.55;
    };
    let rows = css! {
        display: flex;
        flex-direction: column;
        gap: 10px;
    };
    let tiles = css! {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
        gap: 12px;
    };

    let count_text = move || match bookmarks.len() {
        1 => "1 bookmark".to_string(),
        n => format!("{n} bookmarks"),
    };
    let row = move |pack: ModProjectInfo| {
        view! { <ResultCard pack=pack bookmarks=Some(bookmarks) show_platform=true on_select=install /> }
    };
    let tile = move |pack: ModProjectInfo| {
        view! { <PackTile pack=pack bookmarks=Some(bookmarks) show_platform=true on_select=install /> }
    };
    let packs = move || match layout.get() {
        BookmarkLayout::List => view! {
            <div class=rows>
                <For each=move || bookmarks.list() key=|pack| pack.id.clone() children=row />
            </div>
        }
        .into_any(),
        BookmarkLayout::Cards => view! {
            <div class=tiles>
                <For each=move || bookmarks.list() key=|pack| pack.id.clone() children=tile />
            </div>
        }
        .into_any(),
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
            <div class=toolbar>
                <span class=count>{count_text}</span>
                <IconSwitch selected=layout horizontal=true />
            </div>
            {packs}
        </Show>
        <ModpackInstallDialog installer=installer />
    }
}
