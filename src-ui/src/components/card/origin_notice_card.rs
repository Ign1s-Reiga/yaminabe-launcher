use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{component, view, web_sys, IntoView};
use crate::components::ui::{Button, ButtonSize, ButtonVariant};

/// One-line notice for the Mods tab saying where this instance's mods came
/// from, and what that means for keeping them current.
///
/// A pack fetched from a site passes `on_upgrade` and gets the action; a pack
/// read off disk passes none and gets only the explanation, so the absence of
/// an upgrade button reads as a stated reason rather than an omission. Kept to
/// a single bar so the mod list itself stays the first thing on screen.
#[component]
pub fn OriginNoticeCard(
    /// What to say about where the mods come from.
    #[prop(into)] message: String,
    /// Fired when the upgrade button is clicked. Without it, no button.
    #[prop(optional)] on_upgrade: Option<Callback<web_sys::MouseEvent>>,
) -> impl IntoView {
    let bar = css! {
        display: flex;
        align-items: center;
        gap: 12px;
        width: 100%;
        box-sizing: border-box;
        padding: 8px 8px 8px 14px;
        border-radius: 8px;
        background-color: var(--secondary-color);
    };
    let text = css! {
        flex: 1;
        min-width: 0;
        margin: 0;
        font-size: 0.85rem;
        opacity: 0.7;
    };

    view! {
        <div class=bar>
            <p class=text>{message}</p>
            {on_upgrade.map(move |on_upgrade| view! {
                <Button
                    variant=ButtonVariant::Primary
                    size=ButtonSize::Small
                    on_click=on_upgrade
                >
                    "Upgrade modpack…"
                </Button>
            })}
        </div>
    }
}
