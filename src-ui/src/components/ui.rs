use leptos::prelude::{ClassAttribute, ElementChild};
use leptos::{component, IntoView};
use styled::{style, view};

#[component]
pub fn SlideButton(state: bool) -> impl IntoView {
    // TODO: It declares style when this function is called. should be fixed.
    // Solution?: Move styles to global style sheet.
    let styles = style! {
        .slide-button {
            appearance: none;
            display: flex;
            align-items: center;
            justify-content: center;
            width: 64px;
            height: 24px;
            border-radius: 50%;
            background-color: var(--button-background);
            color: var(--text-color);
            cursor: pointer;
            transition: background-color 0.2s ease, color 0.2s ease;
        }
        .slide-button:hover {
            background-color: var(--button-hover-background);
        }
    };
}


