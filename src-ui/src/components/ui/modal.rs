use bamboo_css_macro::css;
use leptos::children::Children;
use leptos::{component, view, IntoView};
use leptos::prelude::{ClassAttribute, ElementChild};

// ── Modal structure ────────────────────────────────────────────────────────────

/// Full-screen backdrop for main modals (z-index 100).
#[component]
pub fn ModalOverlay(children: Children) -> impl IntoView {
    let class = css! {
        position: fixed;
        inset: 0;
        background-color: rgb(0 0 0 / 0.5);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 100;
    };
    view! { <div class=class>{children()}</div> }
}

/// Large modal container (560px wide, flex column for body/footer).
#[component]
pub fn ModalBox(children: Children) -> impl IntoView {
    let class = css! {
        background-color: var(--background-color);
        border-radius: 12px;
        width: 560px;
        min-height: 400px;
        display: flex;
        flex-direction: column;
        box-shadow: 0 20px 60px rgb(0 0 0 / 0.4);
    };
    view! { <div class=class>{children()}</div> }
}

/// Padded flex-1 body inside ModalBox.
#[component]
pub fn ModalBody(children: Children) -> impl IntoView {
    let class = css! {
        flex: 1;
        padding: 32px;
    };
    view! { <div class=class>{children()}</div> }
}

/// Footer with top border; space-between layout.
#[component]
pub fn ModalFooter(children: Children) -> impl IntoView {
    let class = css! {
        display: flex;
        justify-content: space-between;
        align-items: center;
        padding: 16px 32px;
        border-top: 1px solid var(--secondary-color);
    };
    view! { <div class=class>{children()}</div> }
}

// ── Dialog structure (sits above modals) ──────────────────────────────────────

/// Full-screen backdrop for confirmation dialogs (z-index 200).
#[component]
pub fn DialogOverlay(children: Children) -> impl IntoView {
    let class = css! {
        position: fixed;
        inset: 0;
        background-color: rgb(0 0 0 / 0.35);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 200;
    };
    view! { <div class=class>{children()}</div> }
}

/// Small dialog box (360px, self-contained padding and column gap).
#[component]
pub fn DialogBox(children: Children) -> impl IntoView {
    let class = css! {
        background-color: var(--background-color);
        border-radius: 12px;
        padding: 32px;
        width: 360px;
        display: flex;
        flex-direction: column;
        gap: 24px;
        box-shadow: 0 20px 60px rgb(0 0 0 / 0.4);
    };
    view! { <div class=class>{children()}</div> }
}

/// Right-aligned button row inside DialogBox.
#[component]
pub fn DialogFooter(children: Children) -> impl IntoView {
    let class = css! {
        display: flex;
        justify-content: flex-end;
        gap: 8px;
    };
    view! { <div class=class>{children()}</div> }
}
