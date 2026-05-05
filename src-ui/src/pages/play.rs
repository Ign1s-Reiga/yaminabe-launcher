use bamboo_css_macro::{css, styled};
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, IntoView, view};
use leptos_router::hooks::{use_navigate, use_params};
use leptos_router::params::Params;
use serde::{Deserialize, Serialize};
use yaminabe_launcher_shared::datatypes::InstanceMeta;
use crate::components::ui::{Button, ButtonVariant};
use crate::ipc;

#[derive(PartialEq, Clone, Params)]
struct PlayParams {
    id: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct LogLine {
    pub instance_id: String,
    pub line: String,
    pub done: bool,
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LaunchArgs {
    instance_id: String,
    mc_version: String,
    mod_tool: String,
}

styled!(LogBox, div, {
    background-color: #0d0d0d;
    border-radius: 8px;
    padding: 16px;
    font-family: "Roboto Mono", monospace;
    font-weight: 400;
    font-size: 0.8rem;
    line-height: 1.6;
    overflow-y: auto;
    max-height: calc(100vh - 300px);
    min-height: 240px;
    white-space: pre-wrap;
    word-break: break-all;
    color: #d4d4d4;
    flex: 1;
});

#[component]
pub fn PlayPage() -> impl IntoView {
    let params = use_params::<PlayParams>();

    let id = Memo::new(move |_| {
        params.with(|p| p.as_ref().ok().and_then(|p| p.id.clone()).unwrap_or_default())
    });

    let instances_ctx = use_context::<RwSignal<Vec<InstanceMeta>>>().expect("instances context");
    let instance: RwSignal<Option<InstanceMeta>> = RwSignal::new(None);

    Effect::new(move |_| {
        let id = id.get();
        instance.set(instances_ctx.get().into_iter().find(|i| i.id == id));
    });

    let log_lines: RwSignal<Vec<String>>    = RwSignal::new(vec![]);
    let running:   RwSignal<bool>           = RwSignal::new(false);
    let error:     RwSignal<Option<String>> = RwSignal::new(None);

    ipc::on_event::<LogLine, _>("instance-log", move |msg| {
        if msg.instance_id != id.get_untracked() { return; }
        log_lines.update(|v| v.push(msg.line.clone()));
        if msg.done {
            running.set(false);
            if msg.error.is_some() { error.set(msg.error); }
        }
    });

    let launched = StoredValue::new(false);
    Effect::new(move |_| {
        let Some(inst) = instance.get() else { return; };
        if launched.get_value() { return; }
        launched.set_value(true);

        running.set(true);
        log_lines.set(vec![]);
        error.set(None);

        leptos::task::spawn_local(async move {
            let _ = ipc::call::<_, ()>("launch_instance", LaunchArgs {
                instance_id: inst.id.clone(),
                mc_version:  inst.mc_version.clone(),
                mod_tool:    inst.mod_tool.clone(),
            }).await;
        });
    });

    view! {
        <Show when=move || instance.get().is_some()>
            {move || instance.get().map(|inst| view! {
                <PlayContent instance=inst log_lines running error />
            })}
        </Show>
    }
}

#[component]
fn PlayContent(
    instance: InstanceMeta,
    log_lines: RwSignal<Vec<String>>,
    running:   RwSignal<bool>,
    error:     RwSignal<Option<String>>,
) -> impl IntoView {
    let navigate  = use_navigate();
    let inst_name = instance.name.clone();
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

            <h2 style="margin: 0 0 4px 0;">{inst_name}" — Offline Play"</h2>

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

            <LogBox>
                {move || log_lines.get().join("\n")}
            </LogBox>
        </div>
    }
}
