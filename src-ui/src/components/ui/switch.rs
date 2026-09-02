use bamboo_css_macro::{css, cx};
use leptos::prelude::*;
use leptos::{component, view, IntoView};

/// Sliding on/off switch for a single boolean. The knob translates between the
/// two ends of the track; `on_change` reports the state the switch was flipped
/// *to*, so a caller never has to re-read the signal it just toggled.
///
/// Prefer this over a pair of Enable/Disable buttons when the control sits in a
/// row and its label is carried by the row itself. The switch shows no text, so
/// `label` is its whole accessible name — pass what identifies *this* row, not
/// what the switch does, or every row in a list announces identically.
#[component]
pub fn Switch(
    #[prop(into)] checked: Signal<bool>,
    on_change: Callback<bool>,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] label: String,
) -> impl IntoView {
    let track = css! {
        position: relative;
        display: inline-flex;
        flex-shrink: 0;
        width: 38px;
        height: 22px;
        padding: 0;
        border: none;
        border-radius: 999px;
        background-color: var(--tertiary-color);
        cursor: pointer;
        transition: background-color 0.18s ease, opacity 0.15s ease;
        &:disabled { opacity: 0.35; cursor: not-allowed; }
    };
    let track_on = css! {
        background-color: #3a9e5f;
    };
    let knob = css! {
        position: absolute;
        top: 3px;
        left: 3px;
        width: 16px;
        height: 16px;
        border-radius: 50%;
        background-color: white;
        pointer-events: none;
        transition: transform 0.18s cubic-bezier(0.4, 0, 0.2, 1);
    };
    let knob_on = css! {
        transform: translateX(16px);
    };

    view! {
        <button
            type="button"
            role="switch"
            aria-label=label
            aria-checked=move || if checked.get() { "true" } else { "false" }
            class=move || cx!(track, if checked.get() { track_on } else { "" })
            prop:disabled=move || disabled.get()
            on:click=move |_| on_change.run(!checked.get_untracked())
        >
            <span class=move || cx!(knob, if checked.get() { knob_on } else { "" })></span>
        </button>
    }
}
