use bamboo_css_macro::{css, cx};
use leptos::prelude::*;
use leptos::{IntoView, component, view};
use yaminabe_launcher_shared::datamodels::ProjectFileReleaseType;

use super::format_size;

/// Scroll container holding [`VersionRow`]s. Hang the caller's `on:scroll` on it
/// to page in more versions.
pub fn version_list_class() -> &'static str {
    css! {
        display: flex;
        flex-direction: column;
        gap: 4px;
        max-height: 280px;
        overflow-y: auto;
        padding: 6px;
        border: 1px solid var(--secondary-color);
        border-radius: 10px;
        scrollbar-width: thin;
        scrollbar-color: var(--tertiary-color) transparent;
    }
}

/// Muted line inside a version list: "Loading…", "Loading more…", "none found".
pub fn version_note_class() -> &'static str {
    css! {
        padding: 18px 4px;
        text-align: center;
        font-size: 0.85rem;
        opacity: 0.5;
    }
}

/// Failure text shown in place of a version list.
pub fn version_error_class() -> &'static str {
    css! {
        margin: 0;
        font-size: 0.82rem;
        color: #c0392b;
    }
}

/// One selectable version: its name, an optional note ("current", "installed"),
/// its size, and a release-type pill coloured so a beta or alpha stands out.
///
/// `disabled` greys the row and drops the click, for a version that exists but
/// cannot be chosen — the file an instance is already on, say.
#[component]
pub fn VersionRow(
    label: String,
    size: u64,
    release_type: ProjectFileReleaseType,
    #[prop(optional)] note: &'static str,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(into)] selected: Signal<bool>,
    on_pick: Callback<()>,
) -> impl IntoView {
    let row = css! {
        display: grid;
        grid-template-columns: 1fr auto auto auto;
        align-items: center;
        gap: 10px;
        padding: 9px 12px;
        border: 1.5px solid transparent;
        border-radius: 8px;
        cursor: pointer;
        user-select: none;
        transition: border-color 0.12s ease, background-color 0.12s ease;
        &:hover {
            border-color: rgba(58, 158, 95, 0.45);
            background-color: rgba(58, 158, 95, 0.04);
        }
    };
    let row_selected = css! {
        border-color: #3a9e5f;
        background-color: rgba(58, 158, 95, 0.1);
    };
    let row_disabled = css! {
        opacity: 0.4;
        cursor: default;
        &:hover { border-color: transparent; background-color: transparent; }
    };
    let name = css! {
        min-width: 0;
        font-size: 0.875rem;
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let note_class = css! {
        font-size: 0.72rem;
        opacity: 0.55;
        white-space: nowrap;
    };
    let size_class = css! {
        font-size: 0.75rem;
        opacity: 0.5;
        white-space: nowrap;
    };
    // Shape lives on `badge`; each tone carries only its colours, so a row
    // composes the two with `cx!`.
    let badge = css! {
        display: inline-flex;
        align-items: center;
        padding: 3px 8px;
        border-radius: 999px;
        font-size: 0.68rem;
        font-weight: 700;
        letter-spacing: 0.3px;
        text-transform: uppercase;
    };
    let badge_release = css! {
        color: #3a9e5f;
        background-color: rgba(58, 158, 95, 0.14);
    };
    let badge_beta = css! {
        color: #d4a017;
        background-color: rgba(212, 160, 23, 0.14);
    };
    let badge_alpha = css! {
        color: #c0392b;
        background-color: rgba(192, 57, 43, 0.14);
    };
    let badge_unknown = css! {
        color: var(--text-color);
        background-color: var(--secondary-color);
        opacity: 0.7;
    };
    let tone = match release_type {
        ProjectFileReleaseType::Release => badge_release,
        ProjectFileReleaseType::Beta => badge_beta,
        ProjectFileReleaseType::Alpha => badge_alpha,
        ProjectFileReleaseType::Unknown => badge_unknown,
    };
    let release = release_type.to_string();
    let size = format_size(size);

    view! {
        <div
            class=move || cx!(
                row,
                if disabled.get() { row_disabled } else if selected.get() { row_selected } else { "" }
            )
            on:click=move |_| { if !disabled.get_untracked() { on_pick.run(()); } }
        >
            <span class=name>{label}</span>
            <span class=note_class>{note}</span>
            <span class=size_class>{size}</span>
            <span class=cx!(badge, tone)>{release}</span>
        </div>
    }
}
