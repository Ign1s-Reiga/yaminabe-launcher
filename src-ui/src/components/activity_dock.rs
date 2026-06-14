use bamboo_css_macro::css;
use leptos::control_flow::{For, Show};
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use leptos_router::components::A;
use phosphor_leptos::{Icon, IconWeight, CARET_DOWN, CARET_UP, STOP, X};
use serde::Serialize;
use yaminabe_launcher_shared::datamodels::{InstanceMeta, LaunchMode, ModLoader};
use yaminabe_launcher_shared::ipc::InstallProgress;
use crate::ipc;
use crate::signal_ext::HasId;

/// Re-export of the shared wire type under the dock's local name. Keeps the rest
/// of the frontend importing `InstallJob` while making the backend's
/// `InstallProgress` the single source of truth for the schema.
pub type InstallJob = InstallProgress;

/// One launched instance tracked globally so launches survive navigation away
/// from their play page and several can run at once. Plain data held in a single
/// `RwSignal<Vec<_>>`; the app-level event listeners mutate the matching entry
/// as `instance-log` / `instance-process-started` events arrive.
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

/// Context wrapper for the dock's expanded/collapsed signal. Newtyped so it
/// doesn't collide with other `RwSignal<bool>` values in the context map.
#[derive(Clone, Copy)]
pub struct ActivityDockOpen(pub RwSignal<bool>);

/// Logs-free projection of a `RunningInstance` for the dock list, so log bursts
/// (which mutate `log_lines`) don't force the `<For>` to clone every instance's
/// whole buffer on each update.
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
    // Never spawn a second copy of an instance that's already running. The launch
    // buttons disable themselves for running instances; this is the backstop in
    // case any caller reaches here anyway.
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

fn plural(n: usize) -> &'static str {
    if n == 1 { "instance" } else { "instances" }
}

/// Floating bottom-right dock that surfaces both install pipelines and launched
/// instances. Collapsed it is a small summary pill; expanded it lists each
/// installing job (with a progress bar) and running instance (linking to its
/// play page, with Stop / dismiss controls).
#[component]
pub fn ActivityDock(
    jobs: RwSignal<Vec<InstallJob>>,
    registry: RunningRegistry,
    expanded: RwSignal<bool>,
) -> impl IntoView {
    let installing_active = Signal::derive(move || {
        jobs.with(|l| l.iter().filter(|j| !j.done && j.error.is_none()).count())
    });
    let running_active = Signal::derive(move || {
        registry.with(|l| l.iter().filter(|r| r.status.is_active()).count())
    });
    let total = Signal::derive(move || jobs.with(|l| l.len()) + registry.with(|l| l.len()));
    let has_settled = Signal::derive(move || {
        jobs.with(|l| l.iter().any(|j| j.done || j.error.is_some()))
            || registry.with(|l| l.iter().any(|r| !r.status.is_active()))
    });

    let dock_root = css! {
        position: fixed;
        right: 24px;
        bottom: 116px;
        z-index: 100;
    };
    let pill = css! {
        display: flex;
        align-items: center;
        gap: 14px;
        min-width: 210px;
        padding: 12px 14px;
        background-color: var(--primary-color);
        border: 1px solid var(--tertiary-color);
        border-radius: 12px;
        box-shadow: 0 6px 24px rgb(0 0 0 / 0.22);
        cursor: pointer;
        transition: background-color 0.15s ease, transform 0.15s ease;
        &:hover { background-color: var(--secondary-color); transform: translateY(-2px); }
    };
    let pill_lines = css! {
        flex: 1;
        display: flex;
        flex-direction: column;
        gap: 6px;
    };
    let pill_line = css! {
        display: flex;
        align-items: center;
        gap: 8px;
        font-size: 0.85rem;
        font-weight: 500;
    };
    let pill_line_muted = css! {
        font-size: 0.85rem;
        opacity: 0.5;
    };
    let caret_hint = css! {
        flex-shrink: 0;
        display: flex;
        opacity: 0.5;
    };

    let card = css! {
        width: 280px;
        max-height: 380px;
        display: flex;
        flex-direction: column;
        overflow: hidden;
        background-color: var(--primary-color);
        border: 1px solid var(--tertiary-color);
        border-radius: 12px;
        box-shadow: 0 8px 28px rgb(0 0 0 / 0.26);
    };
    let card_header = css! {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 12px 14px;
        border-bottom: 1px solid var(--secondary-color);
        flex-shrink: 0;
    };
    let card_title = css! {
        font-size: 0.8rem;
        font-weight: 600;
        letter-spacing: 0.5px;
        text-transform: uppercase;
        opacity: 0.7;
    };
    let icon_btn = css! {
        display: flex;
        background: none;
        border: none;
        cursor: pointer;
        color: var(--text-color);
        opacity: 0.55;
        padding: 2px;
        line-height: 1;
        transition: opacity 0.12s ease;
        &:hover { opacity: 1; }
    };
    let card_body = css! {
        flex: 1;
        overflow-y: auto;
        scrollbar-width: thin;
        scrollbar-color: var(--secondary-color) transparent;
        padding: 6px 0;
    };

    let row_install = css! {
        display: flex;
        align-items: flex-start;
        gap: 10px;
        padding: 9px 14px;
        border-bottom: 1px solid var(--secondary-color);
        &:last-child { border-bottom: none; }
    };
    let row_run = css! {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 9px 14px;
        border-bottom: 1px solid var(--secondary-color);
        &:last-child { border-bottom: none; }
    };
    let row_main = css! {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 6px;
    };
    let row_top = css! {
        display: flex;
        align-items: center;
        gap: 8px;
    };
    let row_sub = css! {
        display: flex;
        flex-direction: column;
        gap: 5px;
    };
    let row_link = css! {
        flex: 1;
        min-width: 0;
        text-decoration: none;
        color: inherit;
    };
    let name_install = css! {
        flex: 1;
        min-width: 0;
        font-size: 0.875rem;
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let name_class = css! {
        font-size: 0.875rem;
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let status_class = css! {
        font-size: 0.75rem;
        opacity: 0.55;
        margin-top: 2px;
    };
    let step_text = css! {
        font-size: 0.75rem;
        opacity: 0.55;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let err_text = css! {
        font-size: 0.75rem;
        color: #c0392b;
    };
    let progress_track = css! {
        position: relative;
        height: 4px;
        border-radius: 999px;
        background-color: rgb(59 130 246 / 0.18);
        overflow: hidden;
    };
    let progress_fill = css! {
        position: absolute;
        inset: 0;
        border-radius: 999px;
        background-color: #3b82f6;
        animation: pulse 1.2s ease-in-out infinite;
    };

    let mini_dot_blue = css! {
        flex-shrink: 0;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background-color: #3b82f6;
        animation: pulse 1.2s ease-in-out infinite;
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

    let stop_btn = css! {
        flex-shrink: 0;
        display: flex;
        align-items: center;
        background: none;
        border: 1px solid #c0392b;
        color: #c0392b;
        border-radius: 6px;
        padding: 3px 6px;
        cursor: pointer;
        transition: background-color 0.12s ease;
        &:hover { background-color: rgb(192 57 43 / 0.12); }
    };
    let dismiss_btn = css! {
        flex-shrink: 0;
        display: flex;
        background: none;
        border: none;
        cursor: pointer;
        color: var(--text-color);
        opacity: 0.4;
        padding: 0;
        line-height: 1;
        transition: opacity 0.12s ease;
        &:hover { opacity: 0.85; }
    };
    let card_footer = css! {
        padding: 8px 12px;
        border-top: 1px solid var(--secondary-color);
        flex-shrink: 0;
    };
    let clear_btn = css! {
        width: 100%;
        background: none;
        border: 1px solid var(--secondary-color);
        border-radius: 6px;
        padding: 6px 0;
        font-size: 0.78rem;
        font-family: inherit;
        color: var(--text-color);
        opacity: 0.6;
        cursor: pointer;
        transition: background-color 0.12s ease, opacity 0.12s ease;
        &:hover { background-color: var(--secondary-color); opacity: 1; }
    };

    view! {
        <Show when=move || { total.get() > 0 }>
            <div class=dock_root>
                <Show
                    when=move || expanded.get()
                    fallback=move || view! {
                        <div class=pill on:click=move |_| expanded.set(true)>
                            <div class=pill_lines>
                                <Show when=move || { installing_active.get() > 0 }>
                                    <div class=pill_line>
                                        <span class=mini_dot_blue></span>
                                        <span>
                                            {move || format!(
                                                "Installing {} {}…",
                                                installing_active.get(),
                                                plural(installing_active.get()),
                                            )}
                                        </span>
                                    </div>
                                </Show>
                                <Show when=move || { running_active.get() > 0 }>
                                    <div class=pill_line>
                                        <span class=dot_running></span>
                                        <span>
                                            {move || format!(
                                                "Running {} {}…",
                                                running_active.get(),
                                                plural(running_active.get()),
                                            )}
                                        </span>
                                    </div>
                                </Show>
                                <Show when=move || installing_active.get() == 0 && running_active.get() == 0>
                                    <span class=pill_line_muted>"Recent activity"</span>
                                </Show>
                            </div>
                            <span class=caret_hint>
                                <Icon icon=CARET_UP size="16px" weight=IconWeight::Regular />
                            </span>
                        </div>
                    }
                >
                    <div class=card>
                        <div class=card_header>
                            <span class=card_title>"Activity"</span>
                            <button class=icon_btn on:click=move |_| expanded.set(false)>
                                <Icon icon=CARET_DOWN size="16px" weight=IconWeight::Regular />
                            </button>
                        </div>

                        <div class=card_body>
                            <For
                                each=move || jobs.get()
                                key=|j| j.id.clone()
                                children=move |job: InstallJob| {
                                    let job_id = StoredValue::new(job.id.clone());
                                    let current = Signal::derive(move || {
                                        jobs.get().into_iter().find(|j| j.id == job_id.get_value())
                                    });
                                    let name_text = move || current.get().map(|j| j.name).unwrap_or_default();
                                    let is_settled = move || current.get()
                                        .map(|j| j.done || j.error.is_some())
                                        .unwrap_or(false);
                                    let sub = move || match current.get() {
                                        Some(j) if j.error.is_some() =>
                                            view! { <span class=err_text>{j.error.unwrap_or_default()}</span> }.into_any(),
                                        Some(j) if j.done =>
                                            view! { <span class=step_text>{j.step}</span> }.into_any(),
                                        Some(j) =>
                                            view! {
                                                <div class=row_sub>
                                                    <div class=progress_track><span class=progress_fill></span></div>
                                                    <span class=step_text>{j.step}</span>
                                                </div>
                                            }.into_any(),
                                        None => view! { <span></span> }.into_any(),
                                    };
                                    view! {
                                        <div class=row_install>
                                            <div class=row_main>
                                                <div class=row_top>
                                                    <span class=name_install>{name_text}</span>
                                                    <Show when=is_settled>
                                                        <button
                                                            class=dismiss_btn
                                                            on:click=move |_| {
                                                                let id = job_id.get_value();
                                                                jobs.update(move |l| l.retain(|j| j.id != id));
                                                            }
                                                        >
                                                            <Icon icon=X size="14px" weight=IconWeight::Regular />
                                                        </button>
                                                    </Show>
                                                </div>
                                                {sub}
                                            </div>
                                        </div>
                                    }
                                }
                            />
                            <For
                                each=move || registry.with(|l| {
                                    l.iter().map(|r| RowView {
                                        id: r.id.clone(),
                                        name: r.name.clone(),
                                        status: r.status.clone(),
                                    }).collect::<Vec<_>>()
                                })
                                key=|r| (r.id.clone(), r.status.clone())
                                children=move |r: RowView| {
                                    let id_stop = r.id.clone();
                                    let id_dismiss = r.id.clone();
                                    let href = format!("/library/{}/play", r.id);
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
                                        <div class=row_run>
                                            <span class=dot></span>
                                            <A href=href attr:class=row_link>
                                                <div class=name_class>{r.name.clone()}</div>
                                                <div class=status_class>{status}</div>
                                            </A>
                                            {can_stop.then(move || view! {
                                                <button
                                                    class=stop_btn
                                                    title="Stop"
                                                    on:click=move |_| stop_instance(registry, id_stop.clone())
                                                >
                                                    <Icon icon=STOP size="14px" weight=IconWeight::Fill />
                                                </button>
                                            })}
                                            {settled.then(move || view! {
                                                <button
                                                    class=dismiss_btn
                                                    on:click=move |_| {
                                                        let id = id_dismiss.clone();
                                                        registry.update(|l| l.retain(|x| x.id != id));
                                                    }
                                                >
                                                    <Icon icon=X size="14px" weight=IconWeight::Regular />
                                                </button>
                                            })}
                                        </div>
                                    }
                                }
                            />
                        </div>

                        <Show when=move || has_settled.get()>
                            <div class=card_footer>
                                <button
                                    class=clear_btn
                                    on:click=move |_| {
                                        jobs.update(|l| l.retain(|j| !j.done && j.error.is_none()));
                                        registry.update(|l| l.retain(|r| r.status.is_active()));
                                    }
                                >
                                    "Clear finished"
                                </button>
                            </div>
                        </Show>
                    </div>
                </Show>
            </div>
        </Show>
    }
}
