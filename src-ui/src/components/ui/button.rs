use bamboo_css_macro::{css, cx};
use leptos::prelude::*;
use leptos::{component, view, web_sys, IntoView};
use phosphor_leptos::{Icon, IconData, IconWeight};

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
    Small,
    Normal,
    Big
}

impl ButtonSize {
    fn class(&self) -> &'static str {
        match self {
            ButtonSize::Small => css! {
                padding: 6px 14px;
            },
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

/// A square button that shows only `icon`, in the same variants as `Button`.
/// `label` names it, as the tooltip and for screen readers, since there is no
/// text to read.
///
/// ```rust
/// <IconButton
///     variant=ButtonVariant::Primary
///     icon=MAGNIFYING_GLASS
///     label="Search"
///     on_click=Callback::new(move |_| search())
/// />
/// ```
#[component]
pub fn IconButton(
    variant: ButtonVariant,
    icon: IconData,
    #[prop(into)] label: String,
    #[prop(optional)] on_click: Option<Callback<web_sys::MouseEvent>>,
) -> impl IntoView {
    // The height of a text input beside it, so a row of the two lines up.
    let square = css! {
        flex-shrink: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        width: 40px;
        height: 40px;
        padding: 0;
    };
    let title = label.clone();

    view! {
        <button
            type="button"
            class=cx!(variant.class(), square)
            title=title
            aria-label=label
            on:click=move |ev| { if let Some(cb) = on_click { cb.run(ev); } }
        >
            <Icon icon=icon size="20px" weight=IconWeight::Bold />
        </button>
    }
}
