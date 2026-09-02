use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{component, view, web_sys, IntoView};
use crate::components::ui::{Button, ButtonSize, ButtonVariant};

/// One-line notice for the Mods tab of a modpack-managed instance: the list is
/// maintained by the pack, not by hand. CurseForge instances also get an
/// "Upgrade modpack…" action wired to `on_upgrade`. Kept to a single bar so the
/// mod list itself stays the first thing on screen.
#[component]
pub fn ManagedNoticeCard(
    /// Whether to surface the modpack upgrade action (CurseForge instances).
    can_upgrade: bool,
    /// Fired when the upgrade button is clicked.
    on_upgrade: Callback<web_sys::MouseEvent>,
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
            <p class=text>
                "Managed by a modpack — its mod list updates with the pack."
            </p>
            {can_upgrade.then(move || view! {
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
