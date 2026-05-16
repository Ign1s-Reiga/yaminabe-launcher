use crate::components::ui::{Button, ButtonVariant};
use crate::ipc;
use bamboo_css_macro::css;
use leptos::__reexports::send_wrapper::SendWrapper;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{IntoView, component, html, view, web_sys};
use leptos_router::hooks::{use_navigate, use_params};
use leptos_router::params::Params;
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use yaminabe_launcher_shared::datatypes::{InstanceMeta, ModLoader};

const LOG_STICKY_THRESHOLD_PX: i32 = 8;
const LOG_SCROLL_THROTTLE_MS: i32 = 50;

struct ScheduledScroll {
    handle: i32,
    _callback: Closure<dyn FnMut()>,
}

type ScheduledScrollState = SendWrapper<Rc<RefCell<Option<ScheduledScroll>>>>;

fn log_is_near_bottom(log_box_ref: NodeRef<html::Div>) -> bool {
    log_box_ref.get().map_or(true, |el| {
        el.scroll_height() - el.scroll_top() - el.client_height() <= LOG_STICKY_THRESHOLD_PX
    })
}

fn has_text_selection() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Ok(get_selection) =
        js_sys::Reflect::get(window.as_ref(), &JsValue::from_str("getSelection"))
    else {
        return false;
    };
    let Ok(get_selection) = get_selection.dyn_into::<js_sys::Function>() else {
        return false;
    };
    let Ok(selection) = get_selection.call0(window.as_ref()) else {
        return false;
    };
    if selection.is_null() || selection.is_undefined() {
        return false;
    }
    js_sys::Reflect::get(&selection, &JsValue::from_str("isCollapsed"))
        .ok()
        .and_then(|value| value.as_bool())
        .map_or(false, |is_collapsed| !is_collapsed)
}

fn schedule_scroll_to_bottom(
    log_box_ref: NodeRef<html::Div>,
    auto_scroll_enabled: RwSignal<bool>,
    selecting_text: RwSignal<bool>,
    scroll_pending: StoredValue<bool>,
    scheduled_scroll: ScheduledScrollState,
) {
    if !auto_scroll_enabled.get_untracked()
        || selecting_text.get_untracked()
        || has_text_selection()
        || scroll_pending.get_value()
    {
        return;
    }

    let scheduled_scroll_on_timeout = scheduled_scroll.clone();

    scroll_pending.set_value(true);
    let callback = Closure::<dyn FnMut()>::new(move || {
        scroll_pending.set_value(false);
        let _scheduled_scroll = scheduled_scroll_on_timeout.borrow_mut().take();
        if !auto_scroll_enabled.get_untracked()
            || selecting_text.get_untracked()
            || has_text_selection()
        {
            return;
        }
        if let Some(el) = log_box_ref.get() {
            el.set_scroll_top(el.scroll_height());
        }
    });

    if let Some(window) = web_sys::window() {
        if let Ok(handle) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.as_ref().unchecked_ref(),
            LOG_SCROLL_THROTTLE_MS,
        ) {
            *scheduled_scroll.borrow_mut() = Some(ScheduledScroll {
                handle,
                _callback: callback,
            });
            return;
        }
    }

    scroll_pending.set_value(false);
}

fn finish_log_text_selection(
    log_box_ref: NodeRef<html::Div>,
    auto_scroll_enabled: RwSignal<bool>,
    selecting_text: RwSignal<bool>,
    scroll_pending: StoredValue<bool>,
    scheduled_scroll: ScheduledScrollState,
) {
    if !selecting_text.get_untracked() {
        return;
    }
    selecting_text.set(false);
    auto_scroll_enabled.set(log_is_near_bottom(log_box_ref));
    schedule_scroll_to_bottom(
        log_box_ref,
        auto_scroll_enabled,
        selecting_text,
        scroll_pending,
        scheduled_scroll,
    );
}

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
    mod_loader: ModLoader,
}

fn log_box_class() -> &'static str {
    css! {
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
    }
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

    let instances_ctx = use_context::<RwSignal<Vec<InstanceMeta>>>().expect("instances context");
    let instance: RwSignal<Option<InstanceMeta>> = RwSignal::new(None);

    Effect::new(move |_| {
        let id = id.get();
        instance.set(instances_ctx.get().into_iter().find(|i| i.id == id));
    });

    let log_lines: RwSignal<Vec<String>> = RwSignal::new(vec![]);
    let running: RwSignal<bool> = RwSignal::new(false);
    let error: RwSignal<Option<String>> = RwSignal::new(None);

    ipc::on_event::<LogLine, _>("instance-log", move |msg| {
        if msg.instance_id != id.get_untracked() {
            return;
        }
        log_lines.update(|v| v.push(msg.line.clone()));
        if msg.done {
            running.set(false);
            if msg.error.is_some() {
                error.set(msg.error);
            }
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
        log_lines.set(vec![]);
        error.set(None);

        leptos::task::spawn_local(async move {
            let _ = ipc::call::<_, ()>(
                "launch_instance",
                LaunchArgs {
                    instance_id: inst.id.clone(),
                    mc_version: inst.game_version.clone(),
                    mod_loader: inst.mod_loader.clone(),
                },
            )
            .await;
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
    running: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    let navigate = use_navigate();
    let inst_name = instance.name.clone();
    let back_path = format!("/library/{}", instance.id);
    let log_box_ref: NodeRef<html::Div> = NodeRef::new();
    let auto_scroll_enabled = RwSignal::new(true);
    let selecting_text = RwSignal::new(false);
    let scroll_pending = StoredValue::new(false);
    let scheduled_scroll = SendWrapper::new(Rc::new(RefCell::new(None::<ScheduledScroll>)));

    if let Some(window) = web_sys::window() {
        let scheduled_scroll_on_mouseup = scheduled_scroll.clone();
        let callback = Closure::<dyn FnMut()>::new(move || {
            finish_log_text_selection(
                log_box_ref,
                auto_scroll_enabled,
                selecting_text,
                scroll_pending,
                scheduled_scroll_on_mouseup.clone(),
            );
        });
        let listener = callback
            .as_ref()
            .unchecked_ref::<js_sys::Function>()
            .clone();
        let _ = window.add_event_listener_with_callback("mouseup", listener.as_ref());
        let callback = SendWrapper::new(callback);
        on_cleanup(move || {
            if let Some(window) = web_sys::window() {
                let _ = window.remove_event_listener_with_callback("mouseup", listener.as_ref());
            }
            drop(callback);
        });
    }

    let scheduled_scroll_on_cleanup = scheduled_scroll.clone();
    on_cleanup(move || {
        if let Some(scheduled_scroll) = scheduled_scroll_on_cleanup.borrow_mut().take() {
            if let Some(window) = web_sys::window() {
                window.clear_timeout_with_handle(scheduled_scroll.handle);
            }
        }
    });

    let scheduled_scroll_on_effect = scheduled_scroll.clone();
    Effect::new(move |_| {
        let _ = log_lines.get();
        schedule_scroll_to_bottom(
            log_box_ref,
            auto_scroll_enabled,
            selecting_text,
            scroll_pending,
            scheduled_scroll_on_effect.clone(),
        );
    });

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

            <div
                class=log_box_class()
                node_ref=log_box_ref
                on:scroll=move |_| {
                    if !selecting_text.get_untracked() {
                        auto_scroll_enabled.set(log_is_near_bottom(log_box_ref));
                    }
                }
                on:mousedown=move |_| {
                    selecting_text.set(true);
                }
                on:mouseup=move |_| {
                    finish_log_text_selection(
                        log_box_ref,
                        auto_scroll_enabled,
                        selecting_text,
                        scroll_pending,
                        scheduled_scroll.clone(),
                    );
                }
            >
                {move || log_lines.get().join("\n")}
            </div>
        </div>
    }
}
