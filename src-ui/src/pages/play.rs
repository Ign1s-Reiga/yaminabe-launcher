use crate::components::log_viewer::LogViewer;
use crate::components::open_in_file_manager::OpenInFileManager;
use crate::components::ui::{Button, ButtonVariant};
use crate::ipc;
use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{IntoView, component, view};
use leptos_router::hooks::{use_navigate, use_params, use_query_map};
use leptos_router::params::Params;
use serde::Serialize;
use yaminabe_launcher_shared::datatypes::{InstanceMeta, LaunchMode, ModLoader};
use yaminabe_launcher_shared::ipc::LogLine;

#[derive(PartialEq, Clone, Params)]
struct PlayParams {
    id: Option<String>,
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

    // Read `?mode=offline|online` once on mount; the query is set by the
    // split Play button on the instance detail page.
    let query = use_query_map();
    let launch_mode = LaunchMode::from_query(
        query.with_untracked(|q| q.get("mode")).as_deref(),
    );

    let instances_ctx = use_context::<RwSignal<Vec<InstanceMeta>>>().expect("instances context");
    let instance: RwSignal<Option<InstanceMeta>> = RwSignal::new(None);

    Effect::new(move |_| {
        let id = id.get();
        instance.set(instances_ctx.get().into_iter().find(|i| i.id == id));
    });

    let log_lines: RwSignal<Vec<String>> = RwSignal::new(vec![]);
    let running: RwSignal<bool> = RwSignal::new(false);
    // True only after the backend has spawned the Java process and registered
    // its PID — Stop is gated on this so a click during version/asset
    // preparation can't race kill_instance against an empty PID map.
    let process_started: RwSignal<bool> = RwSignal::new(false);
    let error: RwSignal<Option<String>> = RwSignal::new(None);

    ipc::on_event::<LogLine, _>("instance-log", move |msg| {
        if msg.instance_id != id.get_untracked() {
            return;
        }
        log_lines.update(|v| v.push(msg.line.clone()));
        if msg.done {
            running.set(false);
            process_started.set(false);
            if msg.error.is_some() {
                error.set(msg.error);
            }
        }
    });

    ipc::on_event::<String, _>("instance-process-started", move |started_id| {
        if started_id == id.get_untracked() {
            process_started.set(true);
        }
    });

    let launched_instance_id: RwSignal<Option<String>> = RwSignal::new(None);
    Effect::new(move |_| {
        let Some(inst) = instance.get() else {
            return;
        };
        if launched_instance_id.get_untracked().as_deref() == Some(inst.id.as_str()) {
            return;
        }
        launched_instance_id.set(Some(inst.id.clone()));

        running.set(true);
        process_started.set(false);
        log_lines.set(vec![]);
        error.set(None);

        leptos::task::spawn_local(async move {
            // Per-line failures arrive via the `instance-log` event stream;
            // IPC-layer rejections don't, so push them into the same viewer.
            if let Err(e) = ipc::call::<_, ()>(
                "launch_instance",
                LaunchArgs {
                    instance_id: inst.id.clone(),
                    mc_version: inst.game_version.clone(),
                    mod_loader: inst.mod_loader.clone(),
                    launch_mode,
                },
            )
            .await
            {
                log_lines.update(|v| v.push(format!("[launch_instance failed] {e}")));
                running.set(false);
                process_started.set(false);
                error.set(Some(e));
            }
        });
    });

    view! {
        <Show when=move || instance.get().is_some()>
            {move || instance.get().map(|inst| view! {
                <PlayContent instance=inst log_lines running process_started error launch_mode />
            })}
        </Show>
    }
}

#[component]
fn PlayContent(
    instance: InstanceMeta,
    log_lines: RwSignal<Vec<String>>,
    running: RwSignal<bool>,
    process_started: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    launch_mode: LaunchMode,
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
                disabled=Signal::derive(move || running.get())
                on_click=Callback::new(move |_| navigate(&back_path, Default::default()))
            >
                "← Back to Instance"
            </Button>

            <h2 style="margin: 0 0 4px 0;">
                {inst_name}
                {match launch_mode {
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

            <LogViewer log_lines>
                <OpenInFileManager instance_id=open_instance_id />
                <Button
                    variant=ButtonVariant::Danger
                    disabled=Signal::derive(move || !process_started.get())
                    on_click=Callback::new(move |_| {
                        let id = kill_instance_id.clone();
                        leptos::task::spawn_local(async move {
                            if let Err(e) = ipc::call::<_, ()>(
                                "kill_instance",
                                KillArgs { instance_id: id },
                            ).await {
                                log_lines.update(|v| v.push(format!("[kill_instance failed] {e}")));
                            }
                        });
                    })
                >
                    "Stop"
                </Button>
            </LogViewer>
        </div>
    }
}