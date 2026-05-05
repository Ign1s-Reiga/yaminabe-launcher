use bamboo_css_macro::{css, styled};
use leptos::control_flow::{For, Show};
use leptos::prelude::*;
use leptos::{component, view, IntoView};

// ── Data model ────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, serde::Deserialize)]
pub struct InstallJob {
    pub id: String,
    pub name: String,
    pub step: String,
    pub done: bool,
    pub error: Option<String>,
}

// ── Styled primitives ─────────────────────────────────────────────────────────

styled!(PanelHeader, div, {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 16px 12px 16px;
    border-bottom: 1px solid var(--secondary-color);
    flex-shrink: 0;
});

styled!(JobList, div, {
    flex: 1;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--secondary-color) transparent;
    padding: 8px 0;
});

styled!(JobRow, div, {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--secondary-color);
    &:last-child { border-bottom: none; }
});

styled!(PanelFooter, div, {
    padding: 10px 16px;
    border-top: 1px solid var(--secondary-color);
    flex-shrink: 0;
});

// ── Component ─────────────────────────────────────────────────────────────────

#[component]
pub fn InstallSidebar(
    jobs: RwSignal<Vec<InstallJob>>,
    open: RwSignal<bool>,
) -> impl IntoView {
    // ── panel base class (transform applied via inline style) ─────────────
    let panel_class = css! {
        position: fixed;
        top: 0;
        right: 0;
        height: 100vh;
        width: 300px;
        background-color: var(--primary-color);
        border-left: 1px solid var(--secondary-color);
        box-shadow: -4px 0 24px rgba(0, 0, 0, 0.18);
        z-index: 100;
        display: flex;
        flex-direction: column;
        transition: transform 0.25s cubic-bezier(0.4, 0, 0.2, 1);
    };

    // ── toggle tab ─────────────────────────────────────────────────────────
    let toggle_base = css! {
        position: fixed;
        top: 50%;
        z-index: 101;
        width: 24px;
        height: 72px;
        background-color: var(--primary-color);
        border: 1px solid var(--secondary-color);
        border-right: none;
        border-radius: 6px 0 0 6px;
        cursor: pointer;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 0.8rem;
        color: var(--text-color);
        transition: right 0.25s cubic-bezier(0.4, 0, 0.2, 1), background-color 0.15s ease;
        transform: translateY(-50%);
        &:hover { background-color: var(--secondary-color); }
    };
    let toggle_style = move || {
        if open.get() { "right: 300px;" } else { "right: 0;" }
    };
    let toggle_label = move || if open.get() { "›" } else { "‹" };

    let panel_style = move || {
        if open.get() { "transform: translateX(0);" } else { "transform: translateX(100%);" }
    };

    // ── inner styles ───────────────────────────────────────────────────────
    let title_style = css! {
        font-size: 0.85rem;
        font-weight: 600;
        letter-spacing: 0.5px;
        text-transform: uppercase;
        opacity: 0.7;
    };
    let close_btn = css! {
        background: none;
        border: none;
        cursor: pointer;
        color: var(--text-color);
        font-size: 1.1rem;
        opacity: 0.5;
        padding: 0 2px;
        line-height: 1;
        transition: opacity 0.12s ease;
        &:hover { opacity: 1; }
    };
    let dot_active = css! {
        flex-shrink: 0;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background-color: #d4a017;
        margin-top: 5px;
    };
    let dot_done = css! {
        flex-shrink: 0;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background-color: #3a9e5f;
        margin-top: 5px;
    };
    let dot_error = css! {
        flex-shrink: 0;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background-color: #c0392b;
        margin-top: 5px;
    };
    let job_name_class = css! {
        font-size: 0.875rem;
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let job_step_class = css! {
        font-size: 0.78rem;
        opacity: 0.55;
        margin-top: 2px;
    };
    let job_error_class = css! {
        font-size: 0.78rem;
        color: #c0392b;
        margin-top: 2px;
    };
    let dismiss_btn = css! {
        flex-shrink: 0;
        background: none;
        border: none;
        cursor: pointer;
        color: var(--text-color);
        font-size: 0.9rem;
        opacity: 0.35;
        padding: 0;
        line-height: 1;
        margin-top: 2px;
        transition: opacity 0.12s ease;
        &:hover { opacity: 0.8; }
    };
    let clear_btn = css! {
        width: 100%;
        background: none;
        border: 1px solid var(--secondary-color);
        border-radius: 6px;
        padding: 6px 0;
        font-size: 0.8rem;
        font-family: inherit;
        color: var(--text-color);
        opacity: 0.6;
        cursor: pointer;
        transition: background-color 0.12s ease, opacity 0.12s ease;
        &:hover { background-color: var(--secondary-color); opacity: 1; }
    };
    let empty_hint = css! {
        font-size: 0.8rem;
        opacity: 0.35;
        text-align: center;
        padding: 32px 16px;
    };

    let has_done = Signal::derive(move || jobs.get().iter().any(|j| j.done || j.error.is_some()));

    view! {
        // ── toggle tab ────────────────────────────────────────────────────
        <button
            class=toggle_base
            style=toggle_style
            on:click=move |_| open.update(|v| *v = !*v)
        >
            {toggle_label}
        </button>

        // ── sliding panel ─────────────────────────────────────────────────
        <div class=panel_class style=panel_style>
            <PanelHeader>
                <span class=title_style>"Installations"</span>
                <button class=close_btn on:click=move |_| open.set(false)>"×"</button>
            </PanelHeader>

            <JobList>
                <Show
                    when=move || !jobs.get().is_empty()
                    fallback=move || view! { <p class=empty_hint>"No installations yet."</p> }
                >
                    <For
                        each=move || jobs.get()
                        key=|j| j.id.clone()
                        children=move |job: InstallJob| {
                            // Use StoredValue so the id is Copy-able into closures.
                            let job_id = StoredValue::new(job.id.clone());

                            // Derive current job state reactively from the signal so
                            // step/done/error updates render without recreating the row.
                            let current = Signal::derive(move || {
                                jobs.get().into_iter().find(|j| j.id == job_id.get_value())
                            });

                            let dot_class = move || match current.get() {
                                Some(j) if j.error.is_some() => dot_error,
                                Some(j) if j.done          => dot_done,
                                _                           => dot_active,
                            };
                            let name_text  = move || current.get().map(|j| j.name).unwrap_or_default();
                            let is_settled = move || current.get()
                                .map(|j| j.done || j.error.is_some())
                                .unwrap_or(false);

                            view! {
                                <JobRow>
                                    <span class=dot_class></span>
                                    <div style="flex: 1; min-width: 0;">
                                        <div class=job_name_class>{name_text}</div>
                                        {move || match current.get() {
                                            Some(j) if j.error.is_some() =>
                                                view! { <div class=job_error_class>{j.error.unwrap_or_default()}</div> }.into_any(),
                                            Some(j) =>
                                                view! { <div class=job_step_class>{j.step}</div> }.into_any(),
                                            None =>
                                                view! { <div></div> }.into_any(),
                                        }}
                                    </div>
                                    <Show when=is_settled>
                                        <button
                                            class=dismiss_btn
                                            on:click=move |_| {
                                                // job_id is Copy (StoredValue), so this is Fn.
                                                let id = job_id.get_value();
                                                jobs.update(move |list| list.retain(|j| j.id != id));
                                            }
                                        >
                                            "×"
                                        </button>
                                    </Show>
                                </JobRow>
                            }
                        }
                    />
                </Show>
            </JobList>

            <Show when=move || has_done.get()>
                <PanelFooter>
                    <button
                        class=clear_btn
                        on:click=move |_| jobs.update(|list| list.retain(|j| !j.done && j.error.is_none()))
                    >
                        "Clear completed"
                    </button>
                </PanelFooter>
            </Show>
        </div>
    }
}