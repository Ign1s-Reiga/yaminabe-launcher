use crate::components::log_viewer::LogViewer;
use crate::components::open_in_file_manager::OpenInFileManager;
use crate::components::running_sidebar::{start_launch, stop_instance, RegistryExt, RunStatus, RunningRegistry, RunningSidebarOpen};
use crate::components::ui::{Button, ButtonVariant};
use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{IntoView, component, view};
use leptos_router::hooks::{use_navigate, use_params, use_query_map};
use leptos_router::params::Params;
use yaminabe_launcher_shared::datatypes::{InstanceMeta, LaunchMode};

#[derive(PartialEq, Clone, Params)]
struct PlayParams {
    id: Option<String>,
}

#[component]
pub fn PlayPage() -> impl IntoView {
    let params = use_params::<PlayParams>();

    let id = Memo::new(move |_| {
        params.with(|p| {
            p.as_ref()
                .ok()
                .and_then(|p| p.id.clone())
                .unwrap_or_default()
        })
    });

    // `?mode=offline|online` is set by the split Play button (detail page) and
    // the navbar Instant-Play button. Kept reactive so a relaunch picks up the
    // mode of the latest navigation rather than the one we first mounted with.
    let query = use_query_map();
    let launch_mode = Memo::new(move |_| {
        LaunchMode::from_query(query.with(|q| q.get("mode")).as_deref())
    });

    let instances_ctx = use_context::<RwSignal<Vec<InstanceMeta>>>().expect("instances context");
    let last_played_ctx = use_context::<RwSignal<Option<String>>>();
    let registry = use_context::<RunningRegistry>().expect("running registry");
    let sidebar_open = use_context::<RunningSidebarOpen>();
    // Derived directly from the instances context + route id — no separate
    // signal or syncing effect needed.
    let instance = Memo::new(move |_| {
        let id = id.get();
        instances_ctx.get().into_iter().find(|i| i.id == id)
    });

    // Decide per viewed instance whether to launch or just view. A `?mode=`
    // query marks a deliberate Play action (the detail page's Play button sets
    // it); the Running sidebar navigates here without it, so clicking a stopped
    // row inspects its retained logs instead of relaunching. Keyed on id so
    // switching between two instances' play pages re-evaluates for the new one.
    let launched_id: RwSignal<Option<String>> = RwSignal::new(None);
    Effect::new(move |_| {
        let Some(inst) = instance.get() else {
            return;
        };
        if launched_id.get_untracked().as_deref() == Some(inst.id.as_str()) {
            return;
        }
        launched_id.set(Some(inst.id.clone()));

        let (has_entry, is_running) = registry.with_untracked(|list| {
            match list.iter().find(|r| r.id == inst.id) {
                Some(r) => (true, r.status.is_active()),
                None => (false, false),
            }
        });
        let launch_intent = query.with_untracked(|q| q.get("mode").is_some());

        // Launch on a first-time open, or an explicit Play of a non-running
        // instance; otherwise just view the existing (live or stopped) entry.
        if !is_running && (!has_entry || launch_intent) {
            start_launch(registry, &inst, launch_mode.get_untracked());
            // Point the navbar Instant-Play button at what we just launched —
            // the backend persists this too, but the in-session signal has no
            // other refresh path.
            if let Some(lp) = last_played_ctx {
                lp.set(Some(inst.id.clone()));
            }
            if let Some(open) = sidebar_open {
                open.0.set(true);
            }
        }
    });

    // Per-instance view derived from the global registry so logs/status persist
    // across navigation and stay live while this page is mounted. `status` is a
    // memo, so it only re-notifies on a real status change, not on every log
    // line; the lookup itself lives in RegistryExt::map_instance.
    let status = Memo::new(move |_| {
        registry.map_instance(&id.get(), |r| r.status.clone()).unwrap_or(RunStatus::Stopped)
    });
    let log_lines = Signal::derive(move || {
        registry.map_instance(&id.get(), |r| r.log_lines.clone()).unwrap_or_default()
    });

    view! {
        <Show when=move || instance.get().is_some()>
            {move || instance.get().map(|inst| view! {
                <PlayContent
                    instance=inst
                    registry=registry
                    log_lines=log_lines
                    status=status
                    launch_mode=launch_mode
                />
            })}
        </Show>
    }
}

#[component]
fn PlayContent(
    instance: InstanceMeta,
    registry: RunningRegistry,
    log_lines: Signal<Vec<String>>,
    status: Memo<RunStatus>,
    launch_mode: Memo<LaunchMode>,
) -> impl IntoView {
    let navigate = use_navigate();
    let inst_name = instance.name.clone();
    let kill_instance_id = instance.id.clone();
    let open_instance_id = instance.id.clone();
    let back_path = format!("/library/{}", instance.id);

    let play_root = css! {
        display: flex;
        flex-direction: column;
        height: 100%;
    };
    let status_row = css! {
        display: flex;
        align-items: center;
        gap: 10px;
        margin-bottom: 16px;
        font-size: 0.875rem;
    };
    let dot_running = css! {
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background-color: #3a9e5f;
        animation: pulse 1.2s ease-in-out infinite;
    };
    let dot_stopped = css! {
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background-color: var(--text-color);
        opacity: 0.4;
    };
    let dot_error = css! {
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background-color: #c0392b;
    };
    let dot_preparing = css! {
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background-color: #d4a017;
    };

    view! {
        <div class=play_root>
            <Button
                variant=ButtonVariant::Text
                style="margin-bottom: 24px;"
                on_click=Callback::new(move |_| navigate(&back_path, Default::default()))
            >
                "← Back to Instance"
            </Button>

            <h2 style="margin: 0 0 4px 0;">
                {inst_name}
                {move || match launch_mode.get() {
                    LaunchMode::Online => " — Online Play",
                    LaunchMode::Offline => " — Offline Play",
                }}
            </h2>

            <div class=status_row>
                {move || {
                    let s = status.get();
                    let (dot, text_style) = match &s {
                        RunStatus::Errored(_) => (dot_error, "color: #e74c3c;"),
                        RunStatus::Running => (dot_running, ""),
                        RunStatus::Preparing => (dot_preparing, ""),
                        RunStatus::Stopped => (dot_stopped, "opacity: 0.5;"),
                    };
                    view! {
                        <div class=dot></div>
                        <span style=text_style>{s.label()}</span>
                    }
                }}
            </div>

            <LogViewer log_lines=log_lines>
                <OpenInFileManager instance_id=open_instance_id />
                <Button
                    variant=ButtonVariant::Danger
                    disabled=Signal::derive(move || !status.get().is_stoppable())
                    on_click=Callback::new(move |_| stop_instance(registry, kill_instance_id.clone()))
                >
                    "Stop"
                </Button>
            </LogViewer>
        </div>
    }
}