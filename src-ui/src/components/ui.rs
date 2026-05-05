pub mod button;
pub mod input;
pub mod modal;
pub mod skeletons;
pub mod tabbar;

pub use button::*;
pub use input::*;
pub use modal::*;
pub use skeletons::*;
pub use tabbar::*;

use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{component, view, IntoView};

// ── Form control CSS helpers ───────────────────────────────────────────────────
//
// Returns a `&'static str` class name so callers keep full control over the
// element's other attributes (type, placeholder, on:input, on:change, etc.):
//
//   <input class=input_class() type="text" prop:value=... on:input=... />
//   <select class=select_class() on:change=...> ... </select>

pub fn input_class() -> &'static str {
    css! {
        background-color: var(--secondary-color);
        color: var(--text-color);
        border: 1px solid var(--secondary-color);
        border-radius: 8px;
        padding: 10px 14px;
        font-size: 0.9rem;
        font-family: inherit;
        width: 100%;
        box-sizing: border-box;
        &:focus {
            outline: none;
            border-color: #3a9e5f;
        }
    }
}

pub fn select_class() -> &'static str {
    css! {
        background-color: var(--secondary-color);
        color: var(--text-color);
        border: 1px solid var(--secondary-color);
        border-radius: 8px;
        padding: 10px 14px;
        font-size: 0.9rem;
        font-family: inherit;
        width: 100%;
        box-sizing: border-box;
        cursor: pointer;
        &:focus {
            outline: none;
            border-color: #3a9e5f;
        }
    }
}

/// Vertical flex stack for a group of form fields (gap 16 px).
#[component]
pub fn FormFields(
    #[prop(optional)] style: &'static str,
    children: Children,
) -> impl IntoView {
    let class = css! {
        display: flex;
        flex-direction: column;
        gap: 16px;
    };
    view! { <div class=class style=style>{children()}</div> }
}

/// A labelled form field: label on top, input control below (gap 6 px).
/// Pass `uppercase=true` to render the label in small-caps style.
#[component]
pub fn FormField(
    label: &'static str,
    #[prop(optional)] uppercase: bool,
    children: Children,
) -> impl IntoView {
    let field_class = css! {
        display: flex;
        flex-direction: column;
        gap: 6px;
    };
    let label_class = css! {
        font-size: 0.82rem;
        font-weight: 600;
        opacity: 0.7;
    };
    let label_upper_class = css! {
        font-size: 0.82rem;
        font-weight: 600;
        opacity: 0.7;
        text-transform: uppercase;
    };
    view! {
        <div class=field_class>
            <label class=if uppercase { label_upper_class } else { label_class }>{label}</label>
            {children()}
        </div>
    }
}
