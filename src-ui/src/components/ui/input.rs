use crate::components::ui::button::{Button, ButtonVariant};
use crate::components::ui::{input_class, select_class};
use crate::ipc;
use bamboo_css_macro::css;
use leptos::html::{Input, Select};
use leptos::prelude::*;
use leptos::{component, view, IntoView};

// ── Form input components ──────────────────────────────────────────────────────

/// Single-line text or password input.
/// `on_change` fires with the new value whenever the user types.
#[component]
pub fn TextInput(
    #[prop(optional)] name: &'static str,
    #[prop(optional)] placeholder: &'static str,
    #[prop(optional)] default_value: String,
    #[prop(optional)] password: bool,
    #[prop(optional)] on_change: Option<Callback<String>>,
) -> impl IntoView {
    view! {
        <input
            class=input_class()
            name=name
            type=if password { "password" } else { "text" }
            placeholder=placeholder
            value=default_value
            on:input=move |ev| {
                if let Some(cb) = on_change { cb.run(event_target_value(&ev)); }
            }
        />
    }
}

/// File-path text input with a trailing Browse button.
/// `on_change` fires with the new value on user input or folder selection.
/// Set `name` to include this field in a form submission.
#[component]
pub fn PathInput(
    #[prop(optional)] name: &'static str,
    #[prop(optional)] placeholder: &'static str,
    #[prop(optional)] default_value: String,
    #[prop(optional)] on_change: Option<Callback<String>>,
) -> impl IntoView {
    let input_ref: NodeRef<Input> = NodeRef::new();

    view! {
        <div class=css! { display: flex; gap: 8px; }>
            <input
                node_ref=input_ref
                class=input_class()
                style="flex: 1; width: auto;"
                type="text"
                name=name
                placeholder=placeholder
                value=default_value
                on:input=move |ev| {
                    if let Some(cb) = on_change { cb.run(event_target_value(&ev)); }
                }
            />
            <Button
                variant=ButtonVariant::Secondary
                style="white-space: nowrap; flex-shrink: 0;"
                on_click=Callback::new(move |_| {
                    leptos::task::spawn_local(async move {
                        if let Ok(Some(path)) = ipc::call_noargs::<Option<String>>("pick_folder").await {
                            if let Some(el) = input_ref.get() {
                                el.set_value(&path);
                            }
                            if let Some(cb) = on_change { cb.run(path); }
                        }
                    });
                })
            >
                "Browse…"
            </Button>
        </div>
    }
}

/// Monospace resizable textarea bound to a `RwSignal<String>`.
/// `on_change` fires (with the new value) after the signal is updated.
#[component]
pub fn Textarea(
    #[prop(optional)] default_value: String,
    #[prop(optional)] name: &'static str,
    #[prop(optional)] placeholder: &'static str,
    #[prop(optional)] on_change: Option<Callback<String>>,
) -> impl IntoView {
    let class = css! {
        background-color: var(--secondary-color);
        color: var(--text-color);
        border: 1px solid var(--secondary-color);
        border-radius: 8px;
        padding: 10px 14px;
        font-size: 0.88rem;
        font-family: var(--font-mono, monospace);
        width: 100%;
        box-sizing: border-box;
        resize: vertical;
        min-height: 80px;
        &:focus { outline: none; border-color: #3a9e5f; }
    };
    view! {
        <textarea
            class=class
            name=name
            placeholder=placeholder
            prop:value=default_value
            on:input=move |ev| {
                if let Some(cb) = on_change { cb.run(event_target_value(&ev)); }
            }
        />
    }
}

/// Styled `<select>` bound to a readable `Signal<String>`.
/// `prop:value` controls which option is selected; `on_change` fires with the new string.
#[component]
pub fn SelectInput(
    #[prop(optional)] name: &'static str,
    #[prop(optional, into)] disabled: bool,
    #[prop(optional)] node_ref: NodeRef<Select>,
    #[prop(optional)] on_change: Option<Callback<String>>,
    children: Children,
) -> impl IntoView {
    view! {
        <select
            class=select_class()
            name=name
            node_ref=node_ref
            disabled=disabled
            on:change=move |ev| {
                if let Some(cb) = on_change { cb.run(event_target_value(&ev)); }
            }
        >
            {children()}
        </select>
    }
}

/// Memory range slider (1 024 – 16 384 MB) with tick marks every 1 024 MB.
/// `on_change` fires (with the new MB value) after the signal is updated.
#[component]
pub fn SliderInput(
    min: &'static str,
    max: &'static str,
    #[prop(default = "1")] step: &'static str,
    #[prop(optional)] default_value: u32,
    #[prop(optional)] name: &'static str,
    #[prop(optional)] on_change: Option<Callback<u32>>,
) -> impl IntoView {
    let (readout_value, set_readout_value) = signal(default_value.to_string());

    let readout = css! {
        font-size: 0.88rem;
        font-weight: 600;
        min-width: 80px;
        text-align: right;
        opacity: 0.8;
    };
    view! {
        <div class=css! { display: flex; align-items: center; gap: 12px; }>
            <input
                type="range"
                name=name
                min=min
                max=max
                step=step
                list="memory-marks"
                class=css! { flex: 1; accent-color: #3a9e5f; cursor: pointer; }
                prop:value=default_value.to_string()
                on:input=move |ev| {
                    set_readout_value.set(event_target_value(&ev));
                    if let Some(cb) = on_change { cb.run(event_target_value(&ev).parse().unwrap_or_default()); }
                }
            />
            <span class=readout>{move || format!("{} MB", readout_value.get())}</span>
        </div>
        <datalist id="memory-marks">
            {(1u32..=16).map(|i| view! {
                <option value=(i * 1024).to_string()></option>
            }).collect_view()}
        </datalist>
    }
}
