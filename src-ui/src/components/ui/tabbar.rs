use bamboo_css_macro::{css, cx, styled};
use leptos::control_flow::For;
use leptos::prelude::*;
use leptos::{component, view, IntoView};

styled!(TabBarContainer, div, {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 4px;
    padding: 4px;
    background-color: var(--primary-color);
    border-radius: 8px;
    width: fit-content;
    user-select: none;
});

/// Horizontal pill-style tab bar.
///
/// `tabs` — reactive list of all tab labels.
/// `active` — currently selected label.
/// `allow_add` — when `true`, a `+` button appears at the right end.
/// `on_add` — callback invoked when the `+` button is clicked.
/// `style` — optional inline style on the container (e.g. `"margin-top: 24px;"`).
#[component]
pub fn TabBar(
    #[prop(into)] tabs: Signal<Vec<String>>,
    active: RwSignal<String>,
    #[prop(optional)] allow_add: bool,
    #[prop(optional)] on_add: Option<Callback<()>>,
) -> impl IntoView {
    let tab = css! {
        background-color: transparent;
        color: var(--text-color);
        border: none;
        border-radius: 6px;
        padding: 6px 16px;
        font-size: 0.875rem;
        font-family: inherit;
        cursor: pointer;
        transition: background-color 0.15s ease;
        &:hover { background-color: var(--secondary-color); }
    };
    let tab_selected = css! {
        background-color: var(--tertiary-color);
        font-weight: 600;
    };
    let add_btn = css! {
        background-color: transparent;
        color: var(--text-color);
        border: none;
        border-radius: 6px;
        width: 28px;
        height: 28px;
        font-size: 1.1rem;
        line-height: 1;
        font-family: inherit;
        cursor: pointer;
        opacity: 0.5;
        display: flex;
        align-items: center;
        justify-content: center;
        transition: background-color 0.15s ease, opacity 0.15s ease;
        &:hover { background-color: var(--secondary-color); opacity: 1; }
    };

    view! {
        <TabBarContainer>
            <For
                each=move || tabs.get()
                key=|t| t.clone()
                children=move |label: String| {
                    let label_click = label.clone();
                    let label_text  = label.clone();
                    view! {
                        <button
                            class=move || cx!(tab, if active.get() == label { tab_selected } else { "" })
                            on:click=move |_| active.set(label_click.clone())
                        >
                            {label_text}
                        </button>
                    }
                }
            />
            {allow_add.then(move || view! {
                <button
                    class=add_btn
                    on:click=move |_| { if let Some(cb) = on_add { cb.run(()); } }
                >
                    "+"
                </button>
            })}
        </TabBarContainer>
    }
}
