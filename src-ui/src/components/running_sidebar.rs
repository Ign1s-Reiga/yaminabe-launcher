use bamboo_css_macro::{css, styled};
use leptos::control_flow::{For, Show};
use leptos::prelude::*;
use leptos::{component, view, web_sys, IntoView};
use leptos_router::hooks::use_navigate;
use serde::Serialize;
use yaminabe_launcher_shared::datatypes::{InstanceMeta, LaunchMode, ModLoader};
use crate::ipc;
use crate::signal_ext::HasId;

// ── Data model ────────────────────────────────────────────────────────────────

/// One launched instance tracked globally so launches survive navigation away
/// from their play page and several can run at once. Plain data held in a
/// single `RwSignal<Vec<_>>`; the app-level event listeners mutate the matching
/// entry as `instance-log` / `instance-process-started` events arrive.
#[derive(Clone)]
pub struct RunningInstance {
    pub id: String,
    pub name: String,
    pub mode: LaunchMode,
    pub log_lines: Vec<String>,
    pub status: RunStatus,
}

/// Lifecycle of a launched instance. A sum type so impossible combinations
/// (e.g. "started but not running") can't be represented, and the label / dot
/// mapping lives in one place instead of scattered boolean ladders.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum RunStatus {
    /// Launch requested; libraries/assets resolving, process not yet spawned.
    Preparing,
    /// The Java process has spawned and is running.
    Running,
    /// The process exited (or never started) without an error.
    Stopped,
    /// The launch failed or the run ended with an error.
    Errored(String),
}

impl RunStatus {
    /// A launch is in flight (preparing or running) — don't start another.
    pub fn is_active(&self) -> bool {
        matches!(self, RunStatus::Preparing | RunStatus::Running)
    }
    /// The process has actually spawned, so it can be stopped.
    pub fn is_stoppable(&self) -> bool {
        matches!(self, RunStatus::Running)
    }
    /// Short status text for the UI.
    pub fn label(&self) -> &'static str {
        match self {
            RunStatus::Preparing => "Preparing…",
            RunStatus::Running => "Running",
            RunStatus::Stopped => "Stopped",
            RunStatus::Errored(_) => "Error",
        }
    }
}

impl HasId for RunningInstance {
    fn id(&self) -> &str {
        &self.id
    }
}

/// Global registry of launched instances, provided via context by `App`.
pub type RunningRegistry = RwSignal<Vec<RunningInstance>>;

/// Context wrapper for the Running sidebar's open/closed signal. Newtyped so it
/// doesn't collide with other `RwSignal<bool>` values in the context map.
#[derive(Clone, Copy)]
pub struct RunningSidebarOpen(pub RwSignal<bool>);

/// Logs-free projection of a `RunningInstance` for the sidebar list, so log
/// bursts (which mutate `log_lines`) don't force the `<For>` to clone every
/// instance's whole buffer on each update.
#[derive(Clone, PartialEq)]
struct RowView {
    id: String,
    name: String,
    status: RunStatus,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LaunchArgs {
    instance_id: String,
    mc_version: String,
    mod_loader: ModLoader,
    launch_mode: LaunchMode,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KillArgs {
    instance_id: String,
}

/// Register a fresh run for `inst` (resetting any prior entry for the same id)
/// and spawn the backend launch. Per-line output and the done/started lifecycle
/// flow back through the app-level `instance-log` / `instance-process-started`
/// listeners; only the synchronous IPC rejection is handled here.
pub fn start_launch(registry: RunningRegistry, inst: &InstanceMeta, mode: LaunchMode) {
    // Never spawn a second copy of an instance that's already running. The
    // launch buttons disable themselves for running instances; this is the
    // backstop in case any caller reaches here anyway.
    if registry.with_untracked(|list| list.iter().any(|r| r.id == inst.id && r.status.is_active())) {
        return;
    }
    let entry = RunningInstance {
        id: inst.id.clone(),
        name: inst.name.clone(),
        mode,
        log_lines: vec![],
        status: RunStatus::Preparing,
    };
    registry.update(|list| {
        if let Some(existing) = list.iter_mut().find(|r| r.id == entry.id) {
            *existing = entry;
        } else {
            list.push(entry);
        }
    });

    let args = LaunchArgs {
        instance_id: inst.id.clone(),
        mc_version: inst.game_version.clone(),
        mod_loader: inst.mod_loader.clone(),
        launch_mode: mode,
    };
    let id = inst.id.clone();
    leptos::task::spawn_local(async move {
        if let Err(e) = ipc::call::<_, ()>("launch_instance", args).await {
            registry.update(|list| {
                if let Some(r) = list.iter_mut().find(|r| r.id == id) {
                    r.log_lines.push(format!("[launch_instance failed] {e}"));
                    r.status = RunStatus::Errored(e);
                }
            });
        }
    });
}

/// Terminate a running instance via the backend. IPC failures are appended to
/// that instance's log buffer so they surface in the viewer.
pub fn stop_instance(registry: RunningRegistry, id: String) {
    leptos::task::spawn_local(async move {
        if let Err(e) = ipc::call::<_, ()>("kill_instance", KillArgs { instance_id: id.clone() }).await {
            registry.update(|list| {
                if let Some(r) = list.iter_mut().find(|r| r.id == id) {
                    r.log_lines.push(format!("[kill_instance failed] {e}"));
                }
            });
        }
    });
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

styled!(RowList, div, {
    flex: 1;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--secondary-color) transparent;
    padding: 8px 0;
});

styled!(PanelFooter, div, {
    padding: 10px 16px;
    border-top: 1px solid var(--secondary-color);
    flex-shrink: 0;
});

// ── Component ─────────────────────────────────────────────────────────────────

/// Left-anchored slide-in panel listing launched instances. Clicking a row
/// opens that instance's play page; a Stop control kills running games and a
/// dismiss button clears settled (stopped/errored) entries.
#[component]
pub fn RunningSidebar(registry: RunningRegistry, open: RwSignal<bool>) -> impl IntoView {
    let navigate = use_navigate();

    let panel_class = css! {
        position: fixed;
        top: 0;
        left: 0;
        height: 100vh;
        width: 300px;
        background-color: var(--primary-color);
        border-right: 1px solid var(--secondary-color);
        box-shadow: 4px 0 24px rgba(0, 0, 0, 0.18);
        z-index: 100;
        display: flex;
        flex-direction: column;
        transition: transform 0.25s cubic-bezier(0.4, 0, 0.2, 1);
    };
    let toggle_base = css! {
        position: fixed;
        top: 50%;
        z-index: 101;
        width: 24px;
        height: 72px;
        background-color: var(--primary-color);
        border: 1px solid var(--secondary-color);
        border-left: none;
        border-radius: 0 6px 6px 0;
        cursor: pointer;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 0.8rem;
        color: var(--text-color);
        transition: left 0.25s cubic-bezier(0.4, 0, 0.2, 1), background-color 0.15s ease;
        transform: translateY(-50%);
        &:hover { background-color: var(--secondary-color); }
    };
    let toggle_style = move || if open.get() { "left: 300px;" } else { "left: 0;" };
    let toggle_label = move || if open.get() { "‹" } else { "›" };
    let panel_style = move || {
        if open.get() { "transform: translateX(0);" } else { "transform: translateX(-100%);" }
    };

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
    let row = css! {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 10px 16px;
        border-bottom: 1px solid var(--secondary-color);
        cursor: pointer;
        transition: background-color 0.12s ease;
        &:hover { background-color: var(--secondary-color); }
        &:last-child { border-bottom: none; }
    };
    let dot_running = css! {
        flex-shrink: 0;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background-color: #3a9e5f;
        animation: pulse 1.2s ease-in-out infinite;
    };
    let dot_preparing = css! {
        flex-shrink: 0;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background-color: #d4a017;
    };
    let dot_stopped = css! {
        flex-shrink: 0;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background-color: var(--text-color);
        opacity: 0.4;
    };
    let dot_error = css! {
        flex-shrink: 0;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background-color: #c0392b;
    };
    let name_class = css! {
        font-size: 0.875rem;
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let status_class = css! {
        font-size: 0.78rem;
        opacity: 0.55;
        margin-top: 2px;
    };
    let stop_btn = css! {
        flex-shrink: 0;
        background: none;
        border: 1px solid #c0392b;
        color: #c0392b;
        border-radius: 6px;
        padding: 3px 10px;
        font-size: 0.75rem;
        font-family: inherit;
        cursor: pointer;
        transition: background-color 0.12s ease;
        &:hover { background-color: rgba(192, 57, 43, 0.12); }
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

    let has_settled = Signal::derive(move || registry.with(|list| list.iter().any(|r| !r.status.is_active())));

    view! {
        <button
            class=toggle_base
            style=toggle_style
            on:click=move |_| open.update(|v| *v = !*v)
        >
            {toggle_label}
        </button>

        <div class=panel_class style=panel_style>
            <PanelHeader>
                <span class=title_style>"Running"</span>
                <button class=close_btn on:click=move |_| open.set(false)>"×"</button>
            </PanelHeader>

            <RowList>
                <Show
                    when=move || registry.with(|list| !list.is_empty())
                    fallback=move || view! { <p class=empty_hint>"No instances running."</p> }
                >
                    <For
                        each=move || registry.with(|list| {
                            list.iter().map(|r| RowView {
                                id: r.id.clone(),
                                name: r.name.clone(),
                                status: r.status.clone(),
                            }).collect::<Vec<_>>()
                        })
                        key=|r| (r.id.clone(), r.status.clone())
                        children={
                            let navigate = navigate.clone();
                            move |r: RowView| {
                                let id_nav = r.id.clone();
                                let id_stop = r.id.clone();
                                let id_dismiss = r.id.clone();
                                let navigate = navigate.clone();
                                let settled = !r.status.is_active();
                                let can_stop = r.status.is_stoppable();
                                let status = r.status.label();
                                let dot = match &r.status {
                                    RunStatus::Errored(_) => dot_error,
                                    RunStatus::Running => dot_running,
                                    RunStatus::Preparing => dot_preparing,
                                    RunStatus::Stopped => dot_stopped,
                                };
                                view! {
                                    <div
                                        class=row
                                        on:click=move |_| navigate(
                                            &format!("/library/{id_nav}/play"),
                                            Default::default(),
                                        )
                                    >
                                        <span class=dot></span>
                                        <div style="flex: 1; min-width: 0;">
                                            <div class=name_class>{r.name.clone()}</div>
                                            <div class=status_class>{status}</div>
                                        </div>
                                        {can_stop.then(move || view! {
                                            <button
                                                class=stop_btn
                                                on:click=move |ev: web_sys::MouseEvent| {
                                                    ev.stop_propagation();
                                                    stop_instance(registry, id_stop.clone());
                                                }
                                            >
                                                "Stop"
                                            </button>
                                        })}
                                        {settled.then(move || view! {
                                            <button
                                                class=dismiss_btn
                                                on:click=move |ev: web_sys::MouseEvent| {
                                                    ev.stop_propagation();
                                                    let id = id_dismiss.clone();
                                                    registry.update(|list| list.retain(|r| r.id != id));
                                                }
                                            >
                                                "×"
                                            </button>
                                        })}
                                    </div>
                                }
                            }
                        }
                    />
                </Show>
            </RowList>

            <Show when=move || has_settled.get()>
                <PanelFooter>
                    <button
                        class=clear_btn
                        on:click=move |_| registry.update(|list| list.retain(|r| r.status.is_active()))
                    >
                        "Clear stopped"
                    </button>
                </PanelFooter>
            </Show>
        </div>
    }
}