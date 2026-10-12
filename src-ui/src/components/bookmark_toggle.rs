use crate::bookmarks::Bookmarks;
use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use phosphor_leptos::{Icon, IconWeight, BOOKMARK_SIMPLE};
use yaminabe_launcher_shared::datamodels::ModProjectInfo;

/// The bookmark button on a modpack's card: filled blue while the pack is
/// bookmarked, and pressing it adds or drops the bookmark. It keeps its clicks
/// and key presses to itself, so it can sit inside a card that is clickable.
#[component]
pub fn BookmarkToggle(pack: ModProjectInfo, bookmarks: Bookmarks) -> impl IntoView {
    let toggle = css! {
        flex-shrink: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        width: 36px;
        height: 36px;
        padding: 0;
        border: none;
        border-radius: 8px;
        background: none;
        color: inherit;
        cursor: pointer;
        opacity: 0.5;
        transition: background-color 0.15s ease, opacity 0.15s ease;
        &:hover { background-color: var(--secondary-color); opacity: 1; }
        // The activity dock's blue, kept apart from the green of the Install action.
        &[aria-pressed="true"] { color: #3b82f6; opacity: 1; }
    };

    let id = pack.id.clone();
    let marked = Memo::new(move |_| bookmarks.contains(&id));
    let pack = StoredValue::new(pack);
    let label = move || if marked.get() { "Remove bookmark" } else { "Bookmark" };
    let weight = Signal::derive(move || if marked.get() { IconWeight::Fill } else { IconWeight::Regular });

    view! {
        <button
            type="button"
            class=toggle
            title=label
            aria-label=label
            aria-pressed=move || marked.get().to_string()
            on:click=move |ev| {
                ev.stop_propagation();
                bookmarks.toggle(pack.get_value());
            }
            on:keydown=|ev| ev.stop_propagation()
        >
            <Icon icon=BOOKMARK_SIMPLE size="20px" weight=weight />
        </button>
    }
}
