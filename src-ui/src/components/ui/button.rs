use bamboo_css_macro::{css, cx};
use leptos::prelude::*;
use leptos::{component, view, web_sys, IntoView};

#[derive(Clone)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Danger,
    Text,
}

impl ButtonVariant {
    fn class(&self) -> &'static str {
        match self {
            ButtonVariant::Primary => css! {
                background-color: #3a9e5f;
                color: white;
                border: none;
                border-radius: 8px;
                font-size: 0.9rem;
                font-weight: 600;
                font-family: inherit;
                cursor: pointer;
                transition: background-color 0.15s ease, opacity 0.15s ease;
                &:hover { background-color: #2e7d4f; }
                &:disabled { opacity: 0.35; cursor: not-allowed; }
            },
            ButtonVariant::Secondary => css! {
                background-color: transparent;
                color: var(--text-color);
                border: 1px solid var(--secondary-color);
                border-radius: 8px;
                font-size: 0.9rem;
                font-family: inherit;
                cursor: pointer;
                transition: background-color 0.15s ease;
                &:hover { background-color: var(--secondary-color); }
            },
            ButtonVariant::Danger => css! {
                background-color: #c0392b;
                color: white;
                border: none;
                border-radius: 8px;
                font-size: 0.9rem;
                font-weight: 600;
                font-family: inherit;
                cursor: pointer;
                transition: background-color 0.15s ease;
                &:hover { background-color: #a93226; }
            },
            ButtonVariant::Text => css! {
                background-color: transparent;
                color: var(--text-color);
                border: none;
                font-size: 0.9rem;
                font-weight: 600;
                font-family: inherit;
                cursor: pointer;
                opacity: 0.6;
                transition: opacity 0.15s ease;
                &:hover { opacity: 1; }
            },
        }
    }
}

#[derive(Clone)]
pub enum ButtonSize {
    Normal,
    Big
}

impl ButtonSize {
    fn class(&self) -> &'static str {
        match self {
            ButtonSize::Normal => css! {
                padding: 8px 20px;
            },
            ButtonSize::Big => css! {
                padding: 10px 20px;
            },
        }
    }
}

/// Generic button. Use `variant` to select Primary / Secondary / Danger styling.
///
/// ```rust
/// <Button variant=ButtonVariant::Primary on_click=Callback::new(move |_| do_thing())>
///     "Confirm"
/// </Button>
/// <Button variant=ButtonVariant::Secondary disabled=Signal::derive(move || !ready.get())>
///     "Next →"
/// </Button>
/// ```
#[component]
pub fn Button(
    variant: ButtonVariant,
    #[prop(default = ButtonSize::Normal)] size: ButtonSize,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional)] on_click: Option<Callback<web_sys::MouseEvent>>,
    #[prop(optional)] style: &'static str,
    #[prop(default = "button")] button_type: &'static str,
    children: Children,
) -> impl IntoView {
    view! {
        <button
            type=button_type
            class=cx!(variant.class(), size.class())
            style=style
            prop:disabled=move || disabled.get()
            on:click=move |ev| { if let Some(cb) = on_click { cb.run(ev); } }
        >
            {children()}
        </button>
    }
}
