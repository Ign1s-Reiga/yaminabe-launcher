use std::str::FromStr;
use bamboo_css_macro::{css, cx};
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, IntoView, view, web_sys};
use serde::Serialize;
use wasm_bindgen::JsCast;
use yaminabe_launcher_shared::datatypes::{AppSettings, InstanceMeta, ModTool};
use crate::components::ui::*;
use crate::curseforge::{call_get_minecraft_modloaders, call_get_minecraft_versions};
use crate::ipc;

// ── IPC arg type ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateInstanceArgs {
    instance_name:     String,
    instance_location: String,
    mc_version:        String,
    mod_tool:          ModTool,
    mod_tool_version:  Option<String>,
}

impl CreateInstanceArgs {
    fn from_form_data(data: &web_sys::FormData, instance_location: String) -> Option<Self> {
        if instance_location.trim().is_empty() { return None; }
        let get = |key: &str| data.get(key).as_string().unwrap_or_default();
        let instance_name = get("instance_name");
        let mc_version    = get("mc_version");
        if instance_name.trim().is_empty() || mc_version.trim().is_empty() { return None; }
        let mod_tool = ModTool::from_str(get("mod_tool").as_str()).unwrap_or(ModTool::Vanilla);
        let mod_tool_version_str = get("mod_tool_version");
        let mod_tool_version = if matches!(mod_tool, ModTool::Vanilla) || mod_tool_version_str.trim().is_empty() {
            None
        } else {
            Some(mod_tool_version_str)
        };
        Some(Self { instance_name, instance_location, mc_version, mod_tool, mod_tool_version })
    }
}

// ── Component ─────────────────────────────────────────────────────────────────

/// Modal for creating a new instance.
///
/// `show` is controlled by the parent — set it to `true` to open.
#[component]
pub fn CreateInstanceModal(
    show: RwSignal<bool>,
    #[prop(optional)] on_creating: Option<Callback<InstanceMeta>>,
    #[prop(optional)] on_created: Option<Callback<String>>,
) -> impl IntoView {
    let modal_step: RwSignal<u8> = RwSignal::new(1);
    let selected_method: RwSignal<Option<u8>> = RwSignal::new(None);
    let show_cancel_dialog = RwSignal::new(false);

    let app_settings = LocalResource::new(|| async move {
        ipc::call_noargs::<AppSettings>("get_settings").await.unwrap_or_default()
    });
    let mc_versions = LocalResource::new(|| async move {
        call_get_minecraft_versions().await.unwrap_or_default()
    });
    let mc_modloaders = LocalResource::new(|| async move {
        call_get_minecraft_modloaders().await.unwrap_or_default()
    });

    let selected_modtool: RwSignal<String> = RwSignal::new(String::from("vanilla"));
    let selected_mcver: RwSignal<String> = RwSignal::new(String::new());

    Effect::new(move |_| {
        if let Some(versions) = mc_versions.get() {
            if selected_mcver.get_untracked().is_empty() {
                if let Some(first) = versions.first() {
                    selected_mcver.set(first.version_string.clone());
                }
            }
        }
    });

    let reset = move || {
        modal_step.set(1);
        selected_method.set(None);
    };

    // ── submit handler: convert FormData to CreateInstanceArgs and call IPC ────
    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();

        let Some(form) = ev.target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlFormElement>().ok())
        else { return };

        let Ok(data) = web_sys::FormData::new_with_form(&form) else { return };

        let instance_location = app_settings.get()
            .map(|s| s.instance_install_dir)
            .unwrap_or_default();
        let Some(args) = CreateInstanceArgs::from_form_data(&data, instance_location) else { return };

        let stub = InstanceMeta {
            id: String::new(),
            name: args.instance_name.clone(),
            mc_version: args.mc_version.clone(),
            mod_tool: args.mod_tool.to_string(),
            mod_tool_version: args.mod_tool_version.clone(),
            category: String::new(),
            ram_mb: 4096,
            jvm_args: String::new(),
            jre_path: String::new(),
            description: String::new(),
            window_width: 854,
            window_height: 480,
        };

        let instance_name = args.instance_name.clone();
        show.set(false);
        reset();
        if let Some(cb) = on_creating { cb.run(stub); }

        leptos::task::spawn_local(async move {
            let _ = ipc::call::<_, ()>("create_instance", args).await;
            if let Some(cb) = on_created { cb.run(instance_name); }
        });
    };

    // ── option card styles ─────────────────────────────────────────────────────
    let option_list = css! {
        display: flex;
        flex-direction: column;
        gap: 8px;
        margin-top: 8px;
    };
    let option_card = css! {
        display: flex;
        align-items: center;
        gap: 16px;
        padding: 14px 16px;
        border-radius: 8px;
        border: 1.5px solid var(--secondary-color);
        cursor: pointer;
        user-select: none;
        transition: border-color 0.12s ease, background-color 0.12s ease;
        &:hover {
            border-color: rgba(58, 158, 95, 0.45);
            background-color: rgba(58, 158, 95, 0.04);
        }
    };
    let option_selected = css! {
        border-color: #3a9e5f;
        background-color: rgba(58, 158, 95, 0.1);
    };
    let option_icon  = css! {
        font-size: 1.2rem;
        width: 24px;
        text-align: center;
        flex-shrink: 0;
        opacity: 0.8;
    };
    let option_info  = css! { display: flex; flex-direction: column; gap: 3px; };
    let option_title = css! { font-weight: 600; font-size: 0.9rem; };
    let option_desc  = css! { font-size: 0.8rem; opacity: 0.55; };

    view! {
        // ── main modal ────────────────────────────────────────────────────────
        <Show when=move || show.get()>
            <ModalOverlay>
                <ModalBox>

                    // ── Step 1: method selection ──────────────────────────────
                    <Show when=move || modal_step.get() == 1 fallback=|| ()>
                        <ModalBody>
                            <h2 style="margin: 0 0 16px 0;">"New Instance"</h2>
                            <div class=option_list>
                                <div
                                    class=move || cx!(option_card, if selected_method.get() == Some(0) { option_selected } else { "" })
                                    on:click=move |_| selected_method.set(Some(0))
                                >
                                    <span class=option_icon>"📁"</span>
                                    <div class=option_info>
                                        <span class=option_title>"Import from Local"</span>
                                        <span class=option_desc>"Import a modpack from a local file."</span>
                                    </div>
                                </div>
                                <div
                                    class=move || cx!(option_card, if selected_method.get() == Some(1) { option_selected } else { "" })
                                    on:click=move |_| selected_method.set(Some(1))
                                >
                                    <span class=option_icon>"✏️"</span>
                                    <div class=option_info>
                                        <span class=option_title>"Create Manually"</span>
                                        <span class=option_desc>"Set up an instance from scratch."</span>
                                    </div>
                                </div>
                            </div>
                        </ModalBody>
                        <ModalFooter>
                            <Button
                                variant=ButtonVariant::Secondary
                                on_click=Callback::new(move |_| show_cancel_dialog.set(true))
                            >
                                "Cancel"
                            </Button>
                            <Button
                                variant=ButtonVariant::Primary
                                disabled=Signal::derive(move || selected_method.get().is_none())
                                on_click=Callback::new(move |_| {
                                    if selected_method.get() == Some(1) {
                                        modal_step.set(2);
                                    }
                                })
                            >
                                "Next →"
                            </Button>
                        </ModalFooter>
                    </Show>

                    // ── Step 2: Create Manually ───────────────────────────────
                    <Show when=move || modal_step.get() == 2 && selected_method.get() == Some(1) fallback=|| ()>
                        <form on:submit=on_submit>
                            <ModalBody>
                                <h2 style="margin: 0 0 16px 0;">"Create Manually"</h2>
                                <FormFields style="margin-top: 8px;">
                                    <FormField label="Instance Name" uppercase=true>
                                        <TextInput
                                            name="instance_name"
                                            placeholder="My Modpack"
                                        />
                                    </FormField>
                                    <FormField label="Minecraft Version" uppercase=true>
                                        <SelectInput
                                            name="mc_version"
                                            on_change=Callback::new(move |val: String| selected_mcver.set(val))
                                        >
                                            <ForEnumerate
                                                each=move || mc_versions.get().unwrap_or_default()
                                                key=|v| v.id
                                                children=|i, v| view! {
                                                    <option value={v.version_string} selected={move || i.get() == 0}>{v.version_string.clone()}</option>
                                                }
                                            />
                                        </SelectInput>
                                    </FormField>
                                    <FormField label="Mod Tool" uppercase=true>
                                        <SelectInput
                                            name="mod_tool"
                                            on_change=Callback::new(move |val: String| selected_modtool.set(val))
                                        >
                                            <option value="vanilla" selected>"Vanilla"</option>
                                            <option value="forge">"Forge"</option>
                                            <option value="fabric">"Fabric"</option>
                                            <option value="neoforge">"NeoForge"</option>
                                            <option value="quilt">"Quilt"</option>
                                        </SelectInput>
                                    </FormField>
                                    <FormField label="Mod Tool Version" uppercase=true>
                                        {move || view! {
                                            <SelectInput name="mod_tool_version" disabled={modtool_to_id(&selected_modtool.get()) == 0}>
                                                <ForEnumerate
                                                    each=move || {
                                                        mc_modloaders.get().unwrap_or_default()
                                                            .into_iter()
                                                            .filter(|m| m.loader_type == modtool_to_id(&selected_modtool.get()) && m.game_version == selected_mcver.get())
                                                            .collect::<Vec<_>>()
                                                    }
                                                    key=|m| m.name.clone()
                                                    children=|i, m| view! {
                                                        <option value={m.name} selected={move || i.get() == 0}>{m.name.clone()}</option>
                                                    }
                                                />
                                            </SelectInput>
                                        }}
                                    </FormField>
                                </FormFields>
                            </ModalBody>
                            <ModalFooter>
                                <Button
                                    variant=ButtonVariant::Secondary
                                    on_click=Callback::new(move |_| modal_step.set(1))
                                >
                                    "← Back"
                                </Button>
                                <Button variant=ButtonVariant::Primary button_type="submit">
                                    "Create"
                                </Button>
                            </ModalFooter>
                        </form>
                    </Show>
                </ModalBox>
            </ModalOverlay>
        </Show>

        // ── cancel confirmation dialog ─────────────────────────────────────────
        <Show when=move || show_cancel_dialog.get()>
            <DialogOverlay>
                <DialogBox>
                    <div>
                        <p style="margin: 0 0 8px 0; font-size: 1.1rem; font-weight: 600;">"Cancel instance creation?"</p>
                        <p style="opacity: 0.7; font-size: 0.9rem;">"Your progress will be discarded."</p>
                    </div>
                    <DialogFooter>
                        <Button
                            variant=ButtonVariant::Secondary
                            on_click=Callback::new(move |_| show_cancel_dialog.set(false))
                        >
                            "No"
                        </Button>
                        <Button
                            variant=ButtonVariant::Danger
                            on_click=Callback::new(move |_| {
                                show_cancel_dialog.set(false);
                                show.set(false);
                                reset();
                            })
                        >
                            "Yes"
                        </Button>
                    </DialogFooter>
                </DialogBox>
            </DialogOverlay>
        </Show>
    }
}

fn modtool_to_id(s: &str) -> u32 {
    match s {
        "forge" => 1,
        "fabric" => 4,
        "quilt" => 5,
        "neoforge" => 6,
        _ => 0,
    }
}
