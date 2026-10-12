use crate::bookmarks::Bookmarks;
use crate::components::bookmark_toggle::BookmarkToggle;
use crate::curseforge::fmt_downloads;
use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use yaminabe_launcher_shared::datamodels::ModProjectInfo;

/// A modpack as a tile under its square logo, for a grid of packs. Picking the
/// tile hands the pack to `on_select`.
#[component]
pub fn PackTile(
    pack: ModProjectInfo,
    /// Given, the tile carries a toggle that bookmarks the pack.
    #[prop(optional_no_strip)] bookmarks: Option<Bookmarks>,
    /// Names the platform under the downloads, for a grid that mixes them.
    #[prop(optional)] show_platform: bool,
    on_select: Callback<ModProjectInfo>,
) -> impl IntoView {
    let card = css! {
        display: flex;
        flex-direction: column;
        gap: 8px;
        padding: 12px;
        border: 1.5px solid var(--secondary-color);
        border-radius: 10px;
        cursor: pointer;
        user-select: none;
        transition: border-color 0.12s ease, background-color 0.12s ease;
        &:hover {
            border-color: rgba(58, 158, 95, 0.45);
            background-color: rgba(58, 158, 95, 0.04);
        }
    };
    let logo = css! {
        width: 100%;
        aspect-ratio: 1 / 1;
        border-radius: 8px;
        object-fit: cover;
        background-color: var(--secondary-color);
    };
    let logo_placeholder = css! {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 100%;
        aspect-ratio: 1 / 1;
        border-radius: 8px;
        background-color: var(--secondary-color);
        font-size: 1.6rem;
    };
    let footer = css! {
        display: flex;
        align-items: center;
        gap: 4px;
    };
    let text = css! {
        flex: 1;
        min-width: 0;
    };
    let name = css! {
        font-weight: 600;
        font-size: 0.85rem;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let detail = css! {
        font-size: 0.72rem;
        opacity: 0.5;
    };

    let logo_view = match pack.logo_url.clone() {
        Some(url) => view! { <img class=logo src=url alt="" /> }.into_any(),
        None => view! { <div class=logo_placeholder>"📦"</div> }.into_any(),
    };
    let count = format!("{} downloads", fmt_downloads(pack.download_count));
    let platform = show_platform.then(|| view! { <div class=detail>{pack.id.platform().to_string()}</div> });
    let bookmark = bookmarks.map(|bookmarks| view! { <BookmarkToggle pack=pack.clone() bookmarks=bookmarks /> });
    let title = pack.name.clone();
    let hover = title.clone();
    let chosen = StoredValue::new(pack);
    let pick = move || on_select.run(chosen.get_value());

    view! {
        <div
            class=card
            role="button"
            tabindex="0"
            title=hover
            on:click=move |_| pick()
            // A div has no activation behaviour of its own.
            on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                if ev.key() == "Enter" || ev.key() == " " {
                    ev.prevent_default();
                    pick();
                }
            }
        >
            {logo_view}
            <div class=footer>
                <div class=text>
                    <div class=name>{title}</div>
                    <div class=detail>{count}</div>
                    {platform}
                </div>
                {bookmark}
            </div>
        </div>
    }
}
