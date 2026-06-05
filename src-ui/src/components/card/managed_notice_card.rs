use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{component, view, web_sys, IntoView};
use crate::components::ui::{Button, ButtonVariant};

/// Read-only notice for the Mods tab of a modpack-managed instance, explaining
/// that the mod list is managed automatically. CurseForge instances also get an
/// "Upgrade modpack…" action wired to `on_upgrade`.
#[component]
pub fn ManagedNoticeCard(
    /// Whether to surface the modpack upgrade action (CurseForge instances).
    can_upgrade: bool,
    /// Fired when the upgrade button is clicked.
    on_upgrade: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    let card = css! {
        width: 100%;
        box-sizing: border-box;
        padding: 16px 18px;
        border: 1px solid var(--secondary-color);
        border-radius: 10px;
        background-color: var(--secondary-color);
    };
    let title = css! {
        font-weight: 600;
        margin: 0 0 6px 0;
    };
    let body = css! {
        margin: 0;
        font-size: 0.875rem;
        opacity: 0.7;
        line-height: 1.6;
    };

    view! {
        <div class=card>
            <p class=title>"Managed by a modpack"</p>
            <p class=body>
                "This instance was installed from a modpack, so its mod list is managed automatically and can't be edited here."
            </p>
            {can_upgrade.then(move || view! {
                <div style="margin-top: 14px;">
                    <Button variant=ButtonVariant::Primary on_click=on_upgrade>
                        "Upgrade modpack…"
                    </Button>
                </div>
            })}
        </div>
    }
}