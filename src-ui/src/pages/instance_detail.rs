use crate::components::settings::{SaveState, SettingsProp, SettingsSection};
use crate::components::ui::{Button, ButtonSize, ButtonVariant, SelectInput, SliderInput, TabBar, Textarea, input_class};
use crate::ipc;
use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use leptos_router::hooks::{use_navigate, use_params};
use leptos_router::params::Params;
use serde::Serialize;
use yaminabe_launcher_shared::datatypes::{InstanceMeta, JavaInstall};

#[derive(Params, PartialEq, Clone)]
struct InstanceParams {
    id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveInstanceSettingsArgs {
    id: String,
    ram_mb: u32,
    jvm_args: String,
    jre_path: String,
    description: String,
    window_width: u32,
    window_height: u32,
}

#[derive(Serialize)]
struct OpenSubfolderArgs { id: String, subfolder: String }

#[derive(Serialize)]
struct GetSubfoldersArgs { id: String }

#[component]
pub fn InstanceDetailPage() -> impl IntoView {
    let params = use_params::<InstanceParams>();

    let id = Memo::new(move |_| {
        params.with(|p| p.as_ref().ok().and_then(|p| p.id.clone()).unwrap_or_default())
    });

    let instances_ctx = use_context::<RwSignal<Vec<InstanceMeta>>>().expect("instances context");
    let instance: RwSignal<Option<InstanceMeta>> = RwSignal::new(None);

    Effect::new(move |_| {
        let id = id.get();
        instance.set(instances_ctx.get().into_iter().find(|i| i.id == id));
    });

    view! {
        <Show when=move || instance.get().is_some()>
            {move || instance.get().map(|inst| view! { <InstanceDetailView instance=inst /> })}
        </Show>
    }
}

#[component]
fn InstanceDetailView(instance: InstanceMeta) -> impl IntoView {
    let navigate = use_navigate();
    let navigate_play = navigate.clone();
    let active_tab = RwSignal::new(String::from("Description"));
    let save_state: RwSignal<SaveState> = RwSignal::new(SaveState::Idle);

    let header_bg = format!("background-color: {}", &instance.mod_loader.get_modloader_color());
    let instance_name = instance.name.clone();
    let category_label = if instance.category.is_empty() { "Default".to_string() } else { instance.category.clone() };
    let meta_text = format!("MC {}  ·  {}  ·  {}", instance.game_version, instance.mod_loader, category_label);

    let instance_id = instance.id.clone();
    let instance_id_play = instance_id.clone();
    let instance_id_save: RwSignal<String> = RwSignal::new(instance_id.clone());
    let instance_id_open = instance_id.clone();
    let jre_path: RwSignal<String> = RwSignal::new(instance.jre_path.clone());

    let java_installs = LocalResource::new(|| async move {
        ipc::call_noargs::<Vec<JavaInstall>>("get_java_installs").await.unwrap_or_default()
    });

    let description_sig = RwSignal::new(instance.description.clone());
    let jvm_args_init: RwSignal<String> = RwSignal::new(instance.jvm_args.clone());

    let on_settings_submit = move |ev: SubmitEvent| {
        let Some(data) = ipc::form_data_from_submit(&ev) else { return };
        let get = |k: &str| data.get(k).as_string().unwrap_or_default();
        let get_u32 = |k: &str| data.get(k).as_string().unwrap_or_default().parse::<u32>().unwrap_or(0);
        let args = SaveInstanceSettingsArgs {
            id: instance_id_save.get_untracked(),
            ram_mb: get("ram_mb").parse().unwrap_or(4096),
            jvm_args: get("jvm_args"),
            jre_path: get("jre_path"),
            description: get("description"),
            window_width: get_u32("window_width"),
            window_height: get_u32("window_height"),
        };
        let new_description = args.description.clone();
        save_state.set(SaveState::Saving);
        leptos::task::spawn_local(async move {
            match ipc::call::<_, ()>("save_instance_settings", args).await {
                Ok(()) => {
                    save_state.set(SaveState::Ok);
                    description_sig.set(new_description);
                }
                Err(e) => save_state.set(SaveState::Err(e)),
            }
        });
    };

    let header_strip = css! {
        width: 100%;
        height: 6px;
        border-radius: 3px;
        margin-bottom: 16px;
    };
    let desc_text = css! {
        line-height: 1.75;
        white-space: pre-wrap;
        max-width: 640px;
    };
    let desc_empty = css! {
        opacity: 0.45;
        font-size: 0.9rem;
    };

    view! {
        <Button
            variant=ButtonVariant::Text
            style="margin-bottom: 24px;"
            on_click=Callback::new(move |_| navigate("/library", Default::default()))
        >
            "← Back to Library"
        </Button>

        <div class=header_strip style=header_bg></div>
        <InstanceDetailHeader instance_name=instance_name meta_text=meta_text>
            <OpenInFileManager instance_id=instance_id_open />
            <Button
                variant=ButtonVariant::Primary
                size=ButtonSize::Big
                on:click=move |_| {
                    navigate_play(&format!("/library/{}/play", instance_id_play), Default::default());
                }
            >
                "▶  Play Instance"
            </Button>
        </InstanceDetailHeader>

        <TabBar
            tabs=Signal::derive(|| vec!["Description".to_string(), "Mods".to_string(), "Settings".to_string()])
            active=active_tab
            attr:class=css! { margin-bottom: 28px; }
        />

        // ── Description tab ───────────────────────────────────────────────────
        <Show when=move || active_tab.get() == "Description">
            {move || {
                let desc = description_sig.get();
                if desc.is_empty() {
                    view! { <p class=desc_empty>"No description provided."</p> }.into_any()
                } else {
                    view! { <p class=desc_text>{desc}</p> }.into_any()
                }
            }}
        </Show>

        // ── Mods tab ──────────────────────────────────────────────────────────
        <Show when=move || active_tab.get() == "Mods">
            <p style="opacity: 0.45; font-size: 0.9rem;">"No mods installed."</p>
        </Show>

        // ── Settings tab ──────────────────────────────────────────────────────
        <Show when=move || active_tab.get() == "Settings">
            <form on:submit=on_settings_submit>
                <SettingsSection id="instance-defaults" heading="Instance Defaults" save_state=save_state>
                    <SettingsProp
                        label="Java"
                        hint="Overrides the global Java setting for this instance."
                    >
                        {move || {
                            let installs = java_installs.get().unwrap_or_default();
                            let current = jre_path.get_untracked();
                            view! {
                                <SelectInput name="jre_path">
                                    <option value="" selected={current.is_empty()}>"Recommended"</option>
                                    {installs.iter().map(|j| {
                                        let label = format!("{}-{}-{}", j.vendor, j.version, j.path);
                                        let val = j.path.clone();
                                        let sel = val == current;
                                        view! { <option value=val selected=sel>{label}</option> }
                                    }).collect_view()}
                                </SelectInput>
                            }
                        }}
                    </SettingsProp>
                    <SettingsProp
                        label="Memory"
                        hint="Overrides the global memory allocation for this instance."
                    >
                        <SliderInput
                            name="ram_mb"
                            default_value=instance.ram_mb
                            min="1024"
                            max="16384"
                            step="1024"
                        />
                    </SettingsProp>
                    <SettingsProp
                        label="JVM Arguments"
                        hint="Overrides global JVM flags for this instance."
                    >
                        <Textarea
                            name="jvm_args"
                            default_value=jvm_args_init.get_untracked()
                            placeholder="-XX:+UseG1GC -XX:MaxGCPauseMillis=50"
                        />
                    </SettingsProp>
                    <SettingsProp
                        label="Window Size"
                        hint="Game window dimensions (0 = use global/Minecraft default)."
                    >
                        <div class=css! { display: flex; gap: 8px; align-items: center; }>
                            <input
                                type="number"
                                name="window_width"
                                class=input_class()
                                style="width: 90px;"
                                min="0"
                                placeholder="Width"
                                value=instance.window_width.to_string()
                            />
                            <span style="opacity: 0.5; flex-shrink: 0;">"×"</span>
                            <input
                                type="number"
                                name="window_height"
                                class=input_class()
                                style="width: 90px;"
                                min="0"
                                placeholder="Height"
                                value=instance.window_height.to_string()
                            />
                        </div>
                    </SettingsProp>
                </SettingsSection>

                <SettingsSection id="about" heading="About" save_state=save_state>
                    <SettingsProp
                        label="Description"
                        hint="Optional notes or description for this instance."
                    >
                        <Textarea
                            name="description"
                            default_value=description_sig.get_untracked()
                            placeholder="Describe this instance…"
                        />
                    </SettingsProp>
                </SettingsSection>
            </form>
        </Show>
    }
}

#[component]
fn InstanceDetailHeader(
    instance_name: String,
    meta_text: String,
    children: Children,
) -> impl IntoView {
    let header = css! {
        display: flex;
        justify-content: space-between;
        align-items: flex-start;
        margin-bottom: 28px;
    };
    let actions = css! {
        display: flex;
        align-items: center;
        gap: 8px;
    };

    view! {
        <div class=header>
            <div>
                <h2 class=css! { margin: 0 0 6px 0; }>{instance_name}</h2>
                <p class=css! { font-size: 0.875rem; opacity: 0.6; margin: 0; }>
                    {meta_text}
                </p>
            </div>
            <div class=actions>
                {children()}
            </div>
        </div>
    }
}

#[component]
fn OpenInFileManager(instance_id: String) -> impl IntoView {
    let (open_dropdown, set_open_dropdown) = signal(false);

    let id_for_resource = instance_id.clone();
    let existing = LocalResource::new(move || {
        let id = id_for_resource.clone();
        async move {
            ipc::call::<_, Vec<bool>>("get_instance_subfolders", GetSubfoldersArgs { id })
                .await
                .unwrap_or_default()
        }
    });

    let id_root: RwSignal<String> = RwSignal::new(instance_id.clone());
    let id_subs: RwSignal<String> = RwSignal::new(instance_id);

    let dropdown_wrap = css! {
        position: relative;
        display: inline-block;
        z-index: 50;
    };
    let dropdown_list = css! {
        position: absolute;
        top: calc(100% + 4px);
        left: 0;
        background-color: var(--background-color);
        border: 1px solid var(--secondary-color);
        border-radius: 8px;
        padding: 4px;
        min-width: 190px;
        box-shadow: 0 8px 24px rgb(0 0 0 / 0.2);
    };
    let dropdown_item = css! {
        display: block;
        width: 100%;
        background-color: transparent;
        color: var(--text-color);
        border: none;
        border-radius: 6px;
        padding: 8px 12px;
        text-align: left;
        font-size: 0.875rem;
        font-family: inherit;
        cursor: pointer;
        box-sizing: border-box;
        transition: background-color 0.12s ease;
        &:hover { background-color: var(--secondary-color); }
    };

    view! {
        <div class=dropdown_wrap>
            <Button
                variant=ButtonVariant::Secondary
                size=ButtonSize::Big
                on_click=Callback::new(move |_| set_open_dropdown.update(|v| *v = !*v))
            >
                "Open...  ▾"
            </Button>
            <Show when=move || open_dropdown.get()>
                <div class=dropdown_list>
                    <button
                        class=dropdown_item
                        on:click=move |_| {
                            set_open_dropdown.set(false);
                            let id = id_root.get_untracked();
                            leptos::task::spawn_local(async move {
                                let _ = ipc::call::<_, ()>("open_instance_subfolder",
                                    OpenSubfolderArgs { id, subfolder: String::new() }).await;
                            });
                        }
                    >
                        "Instance folder"
                    </button>
                    {move || {
                        let existing = existing.get().unwrap_or_default();
                        let id_str = id_subs.get_untracked();
                        [("config", "Config folder"), ("mods", "Mods folder"),
                         ("resourcepacks", "Resourcepacks folder"), ("saves", "Saves folder")]
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| existing.get(*i).copied().unwrap_or(false))
                            .map(|(_, (sub, label))| {
                                let id = id_str.clone();
                                let subfolder = sub.to_string();
                                view! {
                                    <button
                                        class=dropdown_item
                                        on:click=move |_| {
                                            set_open_dropdown.set(false);
                                            let id = id.clone();
                                            let sf = subfolder.clone();
                                            leptos::task::spawn_local(async move {
                                                let _ = ipc::call::<_, ()>("open_instance_subfolder",
                                                    OpenSubfolderArgs { id, subfolder: sf }).await;
                                            });
                                        }
                                    >
                                        {*label}
                                    </button>
                                }
                            })
                            .collect_view()
                    }}
                </div>
            </Show>
        </div>
    }
}
