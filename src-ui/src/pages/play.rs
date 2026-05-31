use crate::components::log_viewer::LogViewer;
use crate::components::open_in_file_manager::OpenInFileManager;
use crate::components::running_sidebar::{start_launch, stop_instance, RunningRegistry, RunningSidebarOpen};
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

    // `?mode=offline|online` (split Play button / defaults to Online), kept
    // reactive so navigating straight from one instance's play page to
    // another's picks up the new mode.
    let query = use_query_map();
    let launch_mode = Memo::new(move |_| {
        LaunchMode::from_query(query.with(|q| q.get("mode")).as_deref())
    });

    let instances_ctx = use_context::<RwSignal<Vec<InstanceMeta>>>().expect("instances context");
    let registry = use_context::<RunningRegistry>().expect("running registry");
    let sidebar_open = use_context::<RunningSidebarOpen>();
    let instance: RwSignal<Option<InstanceMeta>> = RwSignal::new(None);

    Effect::new(move |_| {
        let id = id.get();
        instance.set(instances_ctx.get().into_iter().find(|i| i.id == id));
    });

    // Auto-launch when the viewed instance changes, but only if it isn't
    // already running — navigating in from the Running sidebar should just view
    // its live logs, not spawn a second process. Keyed on id so switching
    // between two instances' play pages launches the new one.
    let launched_id: RwSignal<Option<String>> = RwSignal::new(None);
    Effect::new(move |_| {
        let Some(inst) = instance.get() else {
            return;
        };
        if launched_id.get_untracked().as_deref() == Some(inst.id.as_str()) {
            return;
        }
        launched_id.set(Some(inst.id.clone()));
        let already_running = registry
            .with_untracked(|list| list.iter().any(|r| r.id == inst.id && r.running));
        if !already_running {
            start_launch(registry, &inst, launch_mode.get_untracked());
        }
        if let Some(open) = sidebar_open {
            open.0.set(true);
        }
    });

    // Per-instance view derived from the global registry so logs/status persist
    // across navigation and stay live while this page is mounted. Each reads
    // just the field it needs, so status changes don't clone the log buffer.
    let log_lines = Signal::derive(move || {
        let id = id.get();
        registry.with(|l| l.iter().find(|r| r.id == id).map(|r| r.log_lines.clone()).unwrap_or_default())
    });
    let running = Signal::derive(move || {
        let id = id.get();
        registry.with(|l| l.iter().find(|r| r.id == id).map(|r| r.running).unwrap_or(false))
    });
    let process_started = Signal::derive(move || {
        let id = id.get();
        registry.with(|l| l.iter().find(|r| r.id == id).map(|r| r.process_started).unwrap_or(false))
    });
    let error = Signal::derive(move || {
        let id = id.get();
        registry.with(|l| l.iter().find(|r| r.id == id).and_then(|r| r.error.clone()))
    });

    view! {
        <Show when=move || instance.get().is_some()>
            {move || instance.get().map(|inst| view! {
                <PlayContent
                    instance=inst
                    registry=registry
                    log_lines=log_lines
                    running=running
                    process_started=process_started
                    error=error
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
    running: Signal<bool>,
    process_started: Signal<bool>,
    error: Signal<Option<String>>,
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
                <Show
                    when=move || error.get().is_some()
                    fallback=move || view! {
                        <Show
                            when=move || running.get()
                            fallback=move || view! {
                                <div class=dot_stopped></div>
                                <span style="opacity: 0.5;">"Stopped"</span>
                            }
                        >
                            <div class=dot_running></div>
                            <span>"Running"</span>
                        </Show>
                    }
                >
                    <div class=dot_error></div>
                    <span style="color: #e74c3c;">"Error"</span>
                </Show>
            </div>

            <LogViewer log_lines=log_lines>
                <OpenInFileManager instance_id=open_instance_id />
                <Button
                    variant=ButtonVariant::Danger
                    disabled=Signal::derive(move || !process_started.get())
                    on_click=Callback::new(move |_| stop_instance(registry, kill_instance_id.clone()))
                >
                    "Stop"
                </Button>
            </LogViewer>
        </div>
    }
}