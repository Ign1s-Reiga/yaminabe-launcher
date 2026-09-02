use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use phosphor_leptos::{Icon, IconWeight, CARET_LEFT, CARET_RIGHT};

/// Build the list of page slots to render: `Some(n)` is a page button,
/// `None` is an ellipsis.
fn build_page_items(current: usize, last_page: usize) -> Vec<Option<usize>> {
    let mut set = std::collections::BTreeSet::new();
    set.insert(0usize);
    if current > 0 { set.insert(current - 1); }
    set.insert(current);
    if current < last_page { set.insert(current + 1); }
    set.insert(last_page);

    let mut result: Vec<Option<usize>> = vec![];
    let mut prev: Option<usize> = None;
    for p in set {
        if let Some(pp) = prev {
            if p == pp + 2 {
                result.push(Some(pp + 1));
            } else if p > pp + 1 {
                result.push(None);
            }
        }
        result.push(Some(p));
        prev = Some(p);
    }
    result
}

#[component]
pub fn Pagination(
    current: Signal<usize>,
    last_page: Signal<usize>,
    is_loading: Signal<bool>,
    on_change: Callback<usize>,
) -> impl IntoView {
    let pagination = css! {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 4px;
        padding-top: 16px;
        margin-top: 8px;
        border-top: 1px solid var(--secondary-color);
        flex-shrink: 0;
    };
    let page_btn = css! {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 32px;
        height: 32px;
        border-radius: 50%;
        border: 1.5px solid var(--secondary-color);
        background: none;
        color: inherit;
        font-size: 0.8rem;
        cursor: pointer;
        transition: border-color 0.12s ease, background-color 0.12s ease;
        &:hover:not(:disabled) {
            border-color: rgba(58, 158, 95, 0.6);
            background-color: var(--secondary-color);
        }
        &:disabled { opacity: 0.3; cursor: default; }
    };
    let page_btn_active = css! {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 32px;
        height: 32px;
        border-radius: 50%;
        border: 1.5px solid var(--text-color);
        background-color: var(--text-color);
        color: var(--background-color);
        font-size: 0.8rem;
        font-weight: 600;
        cursor: default;
    };
    let ellipsis_style = css! {
        width: 32px;
        text-align: center;
        font-size: 0.8rem;
        opacity: 0.4;
        user-select: none;
    };

    let page_items: Signal<Vec<Option<usize>>> = Signal::derive(move || {
        build_page_items(current.get(), last_page.get())
    });

    view! {
        <div
            class=pagination
            style=move || {
                if last_page.get() == 0 { "visibility: hidden;" } else { "visibility: visible;" }
            }
        >
            <button
                class=page_btn
                disabled=move || current.get() == 0 || is_loading.get()
                on:click=move |_| on_change.run(current.get_untracked().saturating_sub(1))
            >
                <Icon icon=CARET_LEFT size="18px" weight=IconWeight::Bold />
            </button>

            {move || {
                let cur = current.get();
                let loading = is_loading.get();
                page_items.get().into_iter().map(|item| {
                    match item {
                        None => view! {
                            <span class=ellipsis_style>"…"</span>
                        }.into_any(),
                        Some(p) => {
                            let is_active = p == cur;
                            view! {
                                <button
                                    class=if is_active { page_btn_active } else { page_btn }
                                    disabled=is_active || loading
                                    on:click=move |_| on_change.run(p)
                                >
                                    {p + 1}
                                </button>
                            }.into_any()
                        }
                    }
                }).collect_view()
            }}

            <button
                class=page_btn
                // Braces are load-bearing: a bare `>` in an attribute expression
                // closes the opening tag, and the rest of the line renders as text.
                disabled=move || { current.get() >= last_page.get() || is_loading.get() }
                on:click=move |_| on_change.run(current.get_untracked() + 1)
            >
                <Icon icon=CARET_RIGHT size="18px" weight=IconWeight::Bold />
            </button>
        </div>
    }
}