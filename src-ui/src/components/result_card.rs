//! A single CurseForge search-result card.

use crate::components::ui::{Button, ButtonVariant};
use crate::curseforge::fmt_downloads;
use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use yaminabe_launcher_shared::datatypes::ModpackInfo;

#[component]
pub fn ResultCard(
    pack: ModpackInfo,
    on_install: Callback<ModpackInfo>,
) -> impl IntoView {
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
    let card_categories = css! {
        display: flex;
        flex-wrap: wrap;
        gap: 4px;
        margin-bottom: 6px;
    };
    let card_category_chip = css! {
        padding: 2px 8px;
        border-radius: 999px;
        background-color: var(--secondary-color);
        font-size: 0.7rem;
        line-height: 1.4;
        opacity: 0.8;
    };
    let card_meta = css! {
        font-size: 0.76rem;
        opacity: 0.45;
    };

    let pack_for_click = pack.clone();

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
                <Show when={
                    let cats = pack.category.clone();
                    move || !cats.is_empty()
                } fallback=|| ()>
                    <div class=card_categories>
                        {pack.category.clone().into_iter().map(|c| view! {
                            <span class=card_category_chip>{c}</span>
                        }).collect_view()}
                    </div>
                </Show>
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
                on_click=Callback::new(move |_| on_install.run(pack_for_click.clone()))
            >
                "Install"
            </Button>
        </div>
    }
}