use bamboo_css_macro::{css, styled};
use leptos::prelude::*;
use leptos::{component, IntoView, view, web_sys};
use serde::Serialize;
use wasm_bindgen::JsCast;
use crate::components::settings::{SaveState, SettingsSection, SettingsProp};
use crate::components::ui::*;
use crate::ipc;
use yaminabe_launcher_shared::datatypes::AppSettings;

#[derive(Serialize)]
struct SaveArgs {
    settings: AppSettings,
}

async fn do_save(settings: AppSettings) -> Result<(), String> {
    ipc::call::<_, ()>("save_settings", SaveArgs { settings }).await
}

#[component]
pub fn SettingsPage() -> impl IntoView {
    let app_settings = LocalResource::new(|| async move {
        ipc::call_noargs::<AppSettings>("get_settings").await.unwrap_or_default()
    });

    let save_state: RwSignal<SaveState> = RwSignal::new(SaveState::Idle);

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let Some(form) = ev.target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlFormElement>().ok())
        else { return };
        let Ok(data) = web_sys::FormData::new_with_form(&form) else { return };
        let get = |k: &str| data.get(k).as_string().unwrap_or_default();
        let get_u32 = |k: &str| data.get(k).as_string().unwrap_or_default().parse::<u32>().unwrap_or(0);

        let settings = AppSettings {
            instance_install_dir: get("instance_install_dir"),
            memory_mb: get("memory_mb").parse().unwrap_or(4096),
            jvm_args: get("jvm_args"),
            curseforge_api_key: get("curseforge_api_key"),
            window_width: get_u32("window_width"),
            window_height: get_u32("window_height"),
        };
        save_state.set(SaveState::Saving);
        leptos::task::spawn_local(async move {
            match do_save(settings).await {
                Ok(_)  => save_state.set(SaveState::Ok),
                Err(e) => save_state.set(SaveState::Err(e)),
            }
        });
    };

    let page_grid = css! {
        display: grid;
        grid-template-columns: 1fr 160px;
        gap: 0 64px;
        align-items: start;
    };
    view! {
        <h1>"# Settings"</h1>
        <div class=page_grid>
            <div>
                {move || match app_settings.get() {
                    None => view! { <SkeletonSettingsPage /> }.into_any(),
                    Some(s) => view! {
                            <form on:submit=on_submit>
                                <SettingsSection id="general" heading="General">
                                    <SettingsProp
                                        label="Language"
                                        hint="Language support is coming in a future update."
                                    >
                                        <SelectInput disabled=true>
                                            <option value="English" selected>"English"</option>
                                            <option value="Japanese">"日本語"</option>
                                        </SelectInput>
                                    </SettingsProp>
                                    <SettingsProp
                                        label="Theme"
                                        hint="Theme follows your system preference. Manual override coming soon."
                                    >
                                        <SelectInput disabled=true>
                                            <option value="System" selected>"System default"</option>
                                            <option value="Light">"Light"</option>
                                            <option value="Dark">"Dark"</option>
                                        </SelectInput>
                                    </SettingsProp>
                                    <SettingsProp
                                        label="Instance Root"
                                        hint="Parent directory for new instances. Each instance is created in a subfolder named after the instance."
                                    >
                                        <PathInput
                                            default_value=s.instance_install_dir
                                            name="instance_install_dir"
                                            placeholder="e.g. C:\\Users\\You\\instances"
                                        />
                                    </SettingsProp>
                                </SettingsSection>
                                <SettingsSection id="instance" heading="Instance Defaults" save_state=save_state>
                                    <SettingsProp
                                        label="Memory"
                                        hint="Maximum heap size allocated to new instances."
                                    >
                                        <SliderInput
                                            default_value=s.memory_mb
                                            name="memory_mb"
                                            min="1024"
                                            max="16384"
                                            step="1024"
                                        />
                                    </SettingsProp>
                                    <SettingsProp
                                        label="JVM Arguments"
                                        hint="Extra JVM flags prepended to the launch command."
                                    >
                                        <Textarea
                                            default_value=s.jvm_args
                                            name="jvm_args"
                                            placeholder="-XX:+UseG1GC -XX:MaxGCPauseMillis=50"
                                        />
                                    </SettingsProp>
                                    <SettingsProp
                                        label="Window Size"
                                        hint="Default game window dimensions (0 = use Minecraft default)."
                                    >
                                        <div class=css! { display: flex; gap: 8px; align-items: center; }>
                                            <input
                                                type="number"
                                                name="window_width"
                                                class=input_class()
                                                style="width: 90px;"
                                                min="0"
                                                placeholder="Width"
                                                value=s.window_width.to_string()
                                            />
                                            <span style="opacity: 0.5; flex-shrink: 0;">"×"</span>
                                            <input
                                                type="number"
                                                name="window_height"
                                                class=input_class()
                                                style="width: 90px;"
                                                min="0"
                                                placeholder="Height"
                                                value=s.window_height.to_string()
                                            />
                                        </div>
                                    </SettingsProp>
                                </SettingsSection>
                                <SettingsSection id="api-keys" heading="API Keys" save_state=save_state>
                                    <SettingsProp
                                        label="CurseForge"
                                        hint="Required for modpack search. Get a key at console.curseforge.com."
                                    >
                                        <TextInput
                                            default_value=s.curseforge_api_key
                                            name="curseforge_api_key"
                                            password=true
                                            placeholder="Enter API Token..."
                                        />
                                    </SettingsProp>
                                </SettingsSection>
                            </form>
                        }.into_any(),
                    }
                }
            </div>

            <Sidebar>
                <SidebarLabel>"On this page"</SidebarLabel>
                <SidebarLink attr:href="#general">"General"</SidebarLink>
                <SidebarLink attr:href="#instance">"Instance Defaults"</SidebarLink>
                <SidebarLink attr:href="#api-keys">"API Keys"</SidebarLink>
            </Sidebar>
        </div>
    }
}

styled!(Sidebar, nav, {
    position: sticky;
    top: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-top: 4px;
});
styled!(SidebarLink, a, {
    display: block;
    text-decoration: none;
    color: var(--text-color);
    font-size: 0.85rem;
    border-radius: 4px;
    padding: 6px 8px;
    transition: background-color 0.15s ease;
    &:hover {
        background-color: var(--secondary-color);
    }
});
styled!(SidebarLabel, span, {
    font-size: 0.7rem;
    font-weight: 700;
    text-transform: uppercase;
    opacity: 0.4;
    padding: 12px 8px 4px 8px;
});
