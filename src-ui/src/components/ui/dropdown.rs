use crate::components::ui::{Button, ButtonSize, ButtonVariant};
use bamboo_css_macro::css;
use leptos::children::{Children, ChildrenFn};
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{IntoView, component, view};

/// A non-native dropdown: a `Secondary` button trigger plus a custom menu of
/// [`DropdownItem`]s styled to match. The menu opens on the trigger and closes
/// on any click inside it (so selecting an item closes it). Use for both value
/// selectors (set `label` to a reactive current value) and action menus.
#[component]
pub fn Dropdown(
    #[prop(into)] label: Signal<String>,
    #[prop(default = ButtonSize::Normal)] size: ButtonSize,
    children: ChildrenFn,
) -> impl IntoView {
    let (open, set_open) = signal(false);

    let wrap = css! {
        position: relative;
        display: inline-block;
        z-index: 50;
    };
    let list = css! {
        position: absolute;
        top: calc(100% + 4px);
        left: 0;
        background-color: var(--background-color);
        border: 1px solid var(--secondary-color);
        border-radius: 8px;
        padding: 4px;
        min-width: 190px;
        box-shadow: 0 8px 24px rgb(0 0 0 / 0.2);
    };

    view! {
        <div class=wrap>
            <Button
                variant=ButtonVariant::Secondary
                size=size
                on_click=Callback::new(move |_| set_open.update(|v| *v = !*v))
            >
                {move || label.get()}
                " ▾"
            </Button>
            <Show when=move || open.get() fallback=|| ()>
                <div class=list on:click=move |_| set_open.set(false)>
                    {children()}
                </div>
            </Show>
        </div>
    }
}

/// One row in a [`Dropdown`] menu, styled as a borderless full-width button.
#[component]
pub fn DropdownItem(on_select: Callback<()>, children: Children) -> impl IntoView {
    let item = css! {
        display: block;
        width: 100%;
        background-color: transparent;
        color: var(--text-color);
        border: none;
        border-radius: 6px;
        padding: 8px 12px;
        text-align: left;
        font-size: 0.875rem;
        font-family: inherit;
        cursor: pointer;
        box-sizing: border-box;
        transition: background-color 0.12s ease;
        &:hover { background-color: var(--secondary-color); }
    };

    view! {
        <button class=item on:click=move |_| on_select.run(())>
            {children()}
        </button>
    }
}