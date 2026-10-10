use std::time::Duration;

use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use phosphor_leptos::{Icon, IconData, IconWeight, X};

/// How long a toast stays before it dismisses itself.
const TOAST_LIFETIME: Duration = Duration::from_secs(12);

#[derive(Clone)]
pub struct Toast {
    id: u64,
    icon: IconData,
    title: String,
    lines: Vec<String>,
}

/// The toasts on screen, shared so any part of the app can raise one.
#[derive(Clone, Copy)]
pub struct Toasts {
    list: RwSignal<Vec<Toast>>,
    next_id: StoredValue<u64>,
}

impl Toasts {
    pub fn provide() -> Self {
        let toasts = Toasts { list: RwSignal::new(Vec::new()), next_id: StoredValue::new(0) };
        provide_context(toasts);
        toasts
    }

    /// Show a toast, which dismisses itself after [`TOAST_LIFETIME`].
    pub fn push(self, icon: IconData, title: String, lines: Vec<String>) {
        let id = self.next_id.get_value();
        self.next_id.set_value(id + 1);
        self.list.update(|list| list.push(Toast { id, icon, title, lines }));
        set_timeout(move || self.dismiss(id), TOAST_LIFETIME);
    }

    fn dismiss(self, id: u64) {
        self.list.update(|list| list.retain(|toast| toast.id != id));
    }
}

/// Where toasts appear: stacked in the top-right corner, clear of the activity
/// dock and the navbar below.
#[component]
pub fn ToastHost() -> impl IntoView {
    let toasts = use_context::<Toasts>().expect("toasts");

    let host = css! {
        position: fixed;
        top: 24px;
        right: 24px;
        z-index: 200;
        width: 340px;
        display: flex;
        flex-direction: column;
        gap: 10px;
    };
    let card = css! {
        display: flex;
        align-items: flex-start;
        gap: 12px;
        padding: 14px 16px;
        background-color: var(--primary-color);
        border: 1px solid var(--tertiary-color);
        border-radius: 12px;
        box-shadow: 0 6px 24px rgb(0 0 0 / 0.22);
    };
    let icon_class = css! {
        flex-shrink: 0;
        color: #d9a03a;
        line-height: 0;
    };
    let body = css! {
        flex: 1;
        min-width: 0;
    };
    let title_class = css! {
        margin: 0 0 4px 0;
        font-size: 0.9rem;
        font-weight: 600;
    };
    let line_class = css! {
        margin: 0;
        font-size: 0.8rem;
        opacity: 0.75;
        overflow-wrap: anywhere;
    };
    let close = css! {
        flex-shrink: 0;
        padding: 0;
        border: none;
        background: none;
        color: inherit;
        line-height: 0;
        cursor: pointer;
        opacity: 0.6;
        &:hover { opacity: 1; }
    };

    let render = move |toast: Toast| {
        let id = toast.id;
        let lines = toast
            .lines
            .into_iter()
            .map(|line| view! { <p class=line_class>{line}</p> })
            .collect_view();
        view! {
            <div class=card role="status">
                <span class=icon_class><Icon icon=toast.icon size="20px" weight=IconWeight::Fill /></span>
                <div class=body>
                    <p class=title_class>{toast.title}</p>
                    {lines}
                </div>
                <button class=close title="Dismiss" on:click=move |_| toasts.dismiss(id)>
                    <Icon icon=X size="16px" weight=IconWeight::Bold />
                </button>
            </div>
        }
    };

    view! {
        <div class=host>
            <For each=move || toasts.list.get() key=|toast| toast.id children=render />
        </div>
    }
}
