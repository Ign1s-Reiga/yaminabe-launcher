use bamboo_css_macro::{css, styled};
use leptos::prelude::*;
use leptos::{component, IntoView, view};
use phosphor_leptos::{Icon, IconWeight, CHECK_CIRCLE, PLUS, TRASH};
use serde::Serialize;
use yaminabe_launcher_shared::datamodels::{AccountSummary, AppSettings};

use crate::components::activity_dock::{InstallJob, RunningRegistry};
use crate::components::modal::login_modal::LoginModal;
use crate::components::settings::{SaveState, SettingsSection, SettingsProp};
use crate::components::ui::*;
use crate::ipc;
use crate::update::{UpdateState, Updates};

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
        let Some(data) = ipc::form_data_from_submit(&ev) else { return };
        let get = |k: &str| data.get(k).as_string().unwrap_or_default();
        let get_u32 = |k: &str| data.get(k).as_string().unwrap_or_default().parse::<u32>().unwrap_or(0);

        let settings = AppSettings {
            instance_install_dir: get("instance_install_dir"),
            memory_mb: get("memory_mb").parse().unwrap_or(4096),
            jvm_args: get("jvm_args"),
            curseforge_api_key: get("curseforge_api_key"),
            window_width: get_u32("window_width"),
            window_height: get_u32("window_height"),
            // Not editable here — preserve whatever launch last recorded.
            last_played_instance_id: app_settings.get()
                .map(|s| s.last_played_instance_id)
                .unwrap_or_default(),
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
                // Accounts sits outside the <form>: login is event-driven via
                // a modal, not a form submission.
                <SettingsSection id="accounts" heading="Accounts">
                    <AccountsSection />
                </SettingsSection>
                <SettingsSection id="updates" heading="Software Update">
                    <SoftwareUpdateSection />
                </SettingsSection>
            </div>

            <Sidebar>
                <SidebarLabel>"On this page"</SidebarLabel>
                <SidebarLink attr:href="#general">"General"</SidebarLink>
                <SidebarLink attr:href="#instance">"Instance Defaults"</SidebarLink>
                <SidebarLink attr:href="#api-keys">"API Keys"</SidebarLink>
                <SidebarLink attr:href="#accounts">"Accounts"</SidebarLink>
                <SidebarLink attr:href="#updates">"Software Update"</SidebarLink>
            </Sidebar>
        </div>
    }
}

/// Lists configured Microsoft accounts, lets the user select one as active,
/// remove any of them, and add a new one via QR-code login. Backs onto the
/// `get_accounts`, `get_selected_account`, `set_selected_account`,
/// `remove_account`, and `start_microsoft_login` IPC commands.
#[component]
fn AccountsSection() -> impl IntoView {
    // Bump this counter after any backend mutation to refetch both resources.
    let refresh: RwSignal<u32> = RwSignal::new(0);
    let show_modal: RwSignal<bool> = RwSignal::new(false);

    let accounts = LocalResource::new(move || {
        refresh.track();
        async move {
            ipc::call_noargs::<Vec<AccountSummary>>("get_accounts")
                .await
                .unwrap_or_default()
        }
    });
    let selected = LocalResource::new(move || {
        refresh.track();
        async move {
            ipc::call_noargs::<Option<String>>("get_selected_account")
                .await
                .ok()
                .flatten()
        }
    });

    let row = css! {
        display: flex;
        align-items: center;
        gap: 14px;
        padding: 12px 14px;
        border: 1px solid var(--secondary-color);
        border-radius: 10px;
        margin-bottom: 10px;
    };
    let avatar = css! {
        width: 40px;
        height: 40px;
        border-radius: 6px;
        background-color: var(--secondary-color);
        flex-shrink: 0;
        image-rendering: pixelated;
        object-fit: cover;
    };
    let meta = css! {
        flex: 1;
        min-width: 0;
    };
    let name = css! {
        font-weight: 600;
        font-size: 0.95rem;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    };
    let uuid_mono = css! {
        font-family: var(--font-mono, monospace);
        font-size: 0.72rem;
        opacity: 0.45;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    };
    let pill = css! {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        font-size: 0.72rem;
        font-weight: 700;
        text-transform: uppercase;
        color: #3a9e5f;
        padding: 4px 8px;
        background-color: rgb(58 158 95 / 0.12);
        border-radius: 999px;
    };
    let actions = css! {
        display: flex;
        gap: 6px;
        align-items: center;
        flex-shrink: 0;
    };
    let empty = css! {
        font-size: 0.85rem;
        opacity: 0.55;
        padding: 18px 4px;
    };
    let add_row = css! {
        display: flex;
        justify-content: flex-start;
        margin-top: 6px;
    };

    let open_modal = Callback::new(move |_| show_modal.set(true));

    view! {
        <p style="font-size: 0.8rem; opacity: 0.55; margin: 0 0 16px 0;">
            "Sign in with a Microsoft account to launch with your real profile. The selected account is used for every instance launch."
        </p>

        {move || match accounts.get() {
            None => view! { <p class=empty>"Loading accounts…"</p> }.into_any(),
            Some(list) if list.is_empty() => view! {
                <p class=empty>"No accounts yet. Sign in below to add one."</p>
            }.into_any(),
            Some(list) => {
                let selected_uuid = selected.get().flatten();
                list.into_iter().map(|acc| {
                    let acc_uuid = acc.uuid.clone();
                    let undashed = acc_uuid.replace('-', "");
                    let avatar_url = format!("https://mc-heads.net/avatar/{undashed}/64");
                    let is_selected = selected_uuid.as_deref() == Some(acc_uuid.as_str());
                    let acc_uuid_for_use = acc_uuid.clone();
                    let acc_uuid_for_remove = acc_uuid.clone();
                    view! {
                        <div class=row>
                            <img class=avatar src=avatar_url alt="" />
                            <div class=meta>
                                <div class=name>{acc.username}</div>
                                <div class=uuid_mono>{acc_uuid}</div>
                            </div>
                            <div class=actions>
                                {if is_selected {
                                    view! {
                                        <span class=pill>
                                            <Icon icon=CHECK_CIRCLE size="14px" weight=IconWeight::Fill />
                                            "Selected"
                                        </span>
                                    }.into_any()
                                } else {
                                    view! {
                                        <Button
                                            variant=ButtonVariant::Secondary
                                            on_click=Callback::new(move |_| {
                                                let uuid = acc_uuid_for_use.clone();
                                                leptos::task::spawn_local(async move {
                                                    let args = SelectArgs { uuid: Some(uuid) };
                                                    match ipc::call::<_, ()>("set_selected_account", args).await {
                                                        Ok(_) => refresh.update(|n| *n += 1),
                                                        Err(e) => log::error!("set_selected_account failed: {e}"),
                                                    }
                                                });
                                            })
                                        >
                                            "Use this"
                                        </Button>
                                    }.into_any()
                                }}
                                <Button
                                    variant=ButtonVariant::Danger
                                    on_click=Callback::new(move |_| {
                                        let uuid = acc_uuid_for_remove.clone();
                                        leptos::task::spawn_local(async move {
                                            let args = RemoveArgs { uuid };
                                            match ipc::call::<_, ()>("remove_account", args).await {
                                                Ok(_) => refresh.update(|n| *n += 1),
                                                Err(e) => log::error!("remove_account failed: {e}"),
                                            }
                                        });
                                    })
                                >
                                    <Icon icon=TRASH size="14px" weight=IconWeight::Regular />
                                </Button>
                            </div>
                        </div>
                    }
                }).collect_view().into_any()
            }
        }}

        <div class=add_row>
            <Button variant=ButtonVariant::Primary on_click=open_modal>
                <span style="display: inline-flex; align-items: center; gap: 6px;">
                    <Icon icon=PLUS size="14px" weight=IconWeight::Bold />
                    "Add Microsoft Account"
                </span>
            </Button>
        </div>

        <Show when=move || show_modal.get() fallback=|| ()>
            <LoginModal
                on_close=Callback::new(move |_| show_modal.set(false))
                on_success=Callback::new(move |_summary: AccountSummary| {
                    refresh.update(|n| *n += 1);
                })
            />
        </Show>
    }
}

/// The running version and the way to a newer one: a check, then the release's
/// notes and an install that restarts the launcher into it.
///
/// The install is held back while a game runs or a pack installs: the installer
/// closes the launcher, which would cut either off part-way.
#[component]
fn SoftwareUpdateSection() -> impl IntoView {
    let updates = use_context::<Updates>().expect("update state");
    let registry = use_context::<RunningRegistry>().expect("running registry");
    let jobs = use_context::<RwSignal<Vec<InstallJob>>>().expect("install jobs");
    let state = updates.0;

    let blocked = Signal::derive(move || {
        if registry.with(|list| list.iter().any(|r| r.status.is_active())) {
            Some("Close the running instances first: installing restarts the launcher.")
        } else if jobs.with(|list| list.iter().any(|job| !job.done)) {
            Some("Wait for the installs in progress to finish: installing restarts the launcher.")
        } else {
            None
        }
    });

    let status = move || match state.get() {
        UpdateState::Unchecked => "Not checked for updates yet.".to_string(),
        UpdateState::Checking => "Checking for updates…".to_string(),
        UpdateState::UpToDate => "You are on the latest version.".to_string(),
        UpdateState::CheckFailed(e) => format!("Could not check for updates: {e}"),
        UpdateState::Available(release) => match release.date {
            Some(date) => format!("Version {} is available, released {date}.", release.version),
            None => format!("Version {} is available.", release.version),
        },
        UpdateState::Installing(release, progress) => match progress {
            Some(p) if p.total.is_some_and(|total| p.downloaded >= total) => {
                "Starting the installer…".to_string()
            }
            Some(p) => match p.total {
                Some(total) => format!(
                    "Downloading version {}… {}%",
                    release.version,
                    p.downloaded * 100 / total.max(1)
                ),
                None => format!(
                    "Downloading version {}… {:.1} MB",
                    release.version,
                    p.downloaded as f64 / 1_048_576.0
                ),
            },
            None => format!("Downloading version {}…", release.version),
        },
        UpdateState::InstallFailed(release, e) => {
            format!("Could not install version {}: {e}", release.version)
        }
    };
    let notes = move || {
        state.with(|s| s.offered().map(|release| crate::changelog::changes(&release.notes)))
            .unwrap_or_default()
            .into_iter()
            .map(|change| view! { <li>{change}</li> })
            .collect_view()
    };
    let has_notes = move || state.with(|s| s.offered().is_some_and(|r| !r.notes.trim().is_empty()));
    let offered_version = move || state.with(|s| s.offered().map(|r| r.version.clone()));
    let install = Callback::new(move |_| {
        let Some(release) = state.with_untracked(|s| s.offered().cloned()) else { return };
        leptos::task::spawn_local(updates.install(release));
    });
    let check = Callback::new(move |_| leptos::task::spawn_local(updates.check()));

    let intro = css! {
        font-size: 0.8rem;
        opacity: 0.55;
        margin: 0 0 16px 0;
    };
    let status_line = css! {
        font-size: 0.9rem;
        margin: 0 0 12px 0;
    };
    let notes_list = css! {
        font-size: 0.85rem;
        margin: 0 0 16px 0;
        padding-left: 20px;
        line-height: 1.5;
    };
    let hint = css! {
        font-size: 0.8rem;
        opacity: 0.55;
        margin: 8px 0 0 0;
    };

    view! {
        <p class=intro>{format!("Yaminabe Launcher {}", env!("CARGO_PKG_VERSION"))}</p>
        <p class=status_line>{status}</p>
        <Show when=has_notes fallback=|| ()>
            <ul class=notes_list>{notes}</ul>
        </Show>
        {move || match offered_version() {
            Some(version) => view! {
                <Button
                    variant=ButtonVariant::Primary
                    disabled=Signal::derive(move || blocked.get().is_some() || state.with(UpdateState::is_busy))
                    on_click=install
                >
                    {format!("Install {version} and restart")}
                </Button>
                {move || blocked.get().map(|reason| view! { <p class=hint>{reason}</p> })}
            }.into_any(),
            None => view! {
                <Button
                    variant=ButtonVariant::Secondary
                    disabled=Signal::derive(move || state.with(UpdateState::is_busy))
                    on_click=check
                >
                    "Check for updates"
                </Button>
            }.into_any(),
        }}
    }
}

#[derive(Serialize)]
struct SelectArgs {
    uuid: Option<String>,
}

#[derive(Serialize)]
struct RemoveArgs {
    uuid: String,
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