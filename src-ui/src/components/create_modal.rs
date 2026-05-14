use crate::components::ui::*;
use crate::curseforge::{call_get_minecraft_versions, call_get_modloader_versions};
use crate::ipc;
use bamboo_css_macro::{css, cx};
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use serde::Serialize;
use std::str::FromStr;
use log::info;
use yaminabe_launcher_shared::datatypes::{InstanceMeta, ModLoader, ReleaseType};

// ── IPC arg type ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateInstanceArgs {
    instance_meta: InstanceMeta,
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

    // ── form state ─────────────────────────────────────────────────────────────
    let instance_name: RwSignal<String> = RwSignal::new(String::new());
    let category: RwSignal<String> = RwSignal::new(String::new());
    let selected_mcver: RwSignal<String> = RwSignal::new(String::new());
    let selected_modloader: RwSignal<String> = RwSignal::new(String::from("vanilla"));
    let selected_modloader_version: RwSignal<String> = RwSignal::new(String::new());

    // ── version-type filters ───────────────────────────────────────────────────
    let include_snapshot: RwSignal<bool> = RwSignal::new(false);
    let include_beta: RwSignal<bool> = RwSignal::new(false);
    let include_alpha: RwSignal<bool> = RwSignal::new(false);

    let mc_versions = LocalResource::new(|| async move {
        call_get_minecraft_versions().await.unwrap_or_default()
    });

    // ── lazy modloader fetch (kicked off on entering step 3) ──────────────────
    // The backend filters by (kind, mc_version), so we re-fetch whenever those change
    // while the user is on step 3 and has selected a non-vanilla loader.
    let loader_versions = LocalResource::new(move || {
        let step = modal_step.get();
        let kind = selected_modloader.get();
        let mcver = selected_mcver.get();
        async move {
            if step != 3 || kind == "vanilla" || mcver.is_empty() {
                return Vec::new();
            }
            call_get_modloader_versions(&kind, &mcver).await.unwrap_or_default()
        }
    });

    let filtered_versions = Memo::new(move |_| {
        let snapshot = include_snapshot.get();
        let beta = include_beta.get();
        let alpha = include_alpha.get();
        mc_versions.get().unwrap_or_default()
            .into_iter()
            .filter(|v| match v.release_type {
                ReleaseType::Release => true,
                ReleaseType::Snapshot => snapshot,
                ReleaseType::Beta => beta,
                ReleaseType::Alpha => alpha,
            })
            .collect::<Vec<_>>()
    });

    // Keep `selected_mcver` valid as filters change.
    Effect::new(move |_| {
        let versions = filtered_versions.get();
        let current = selected_mcver.get_untracked();
        if !versions.iter().any(|v| v.version_string == current) {
            if let Some(first) = versions.first() {
                selected_mcver.set(first.version_string.clone());
            } else {
                selected_mcver.set(String::new());
            }
        }
    });

    // Keep `selected_modloader_version` valid as the loader list changes.
    Effect::new(move |_| {
        let kind = selected_modloader.get();
        if kind == "vanilla" {
            selected_modloader_version.set(String::new());
            return;
        }
        let candidates = loader_versions.get().unwrap_or_default();
        let current = selected_modloader_version.get_untracked();
        if !candidates.iter().any(|m| m.version == current) {
            if let Some(first) = candidates.first() {
                selected_modloader_version.set(first.version.clone());
            } else {
                selected_modloader_version.set(String::new());
            }
        }
    });

    let reset = move || {
        modal_step.set(1);
        selected_method.set(None);
        instance_name.set(String::new());
        category.set(String::new());
        selected_modloader.set(String::from("vanilla"));
        selected_modloader_version.set(String::new());
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

    let filter_row = css! {
        display: flex;
        gap: 14px;
        font-size: 0.82rem;
        opacity: 0.85;
        margin-bottom: 4px;
    };
    let filter_label = css! {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        cursor: pointer;
        user-select: none;
    };

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

                    // ── Step 2: name + game version + category ───────────────
                    <Show when=move || modal_step.get() == 2 && selected_method.get() == Some(1) fallback=|| ()>
                        <ModalBody>
                            <h2 style="margin: 0 0 16px 0;">"Create Manually"</h2>
                            <FormFields style="margin-top: 8px;">
                                <FormField label="Instance Name" uppercase=true>
                                    <TextInput
                                        placeholder="My Modpack"
                                        default_value=instance_name.get_untracked()
                                        on_change=Callback::new(move |v: String| instance_name.set(v))
                                    />
                                </FormField>
                                <FormField label="Minecraft Version" uppercase=true>
                                    <div class=filter_row>
                                        <label class=filter_label>
                                            <input
                                                type="checkbox"
                                                prop:checked=move || include_snapshot.get()
                                                on:change=move |ev| include_snapshot.set(event_target_checked(&ev))
                                            />
                                            "Snapshot"
                                        </label>
                                        <label class=filter_label>
                                            <input
                                                type="checkbox"
                                                prop:checked=move || include_beta.get()
                                                on:change=move |ev| include_beta.set(event_target_checked(&ev))
                                            />
                                            "Beta"
                                        </label>
                                        <label class=filter_label>
                                            <input
                                                type="checkbox"
                                                prop:checked=move || include_alpha.get()
                                                on:change=move |ev| include_alpha.set(event_target_checked(&ev))
                                            />
                                            "Alpha"
                                        </label>
                                    </div>
                                    {move || {
                                        let versions = filtered_versions.get();
                                        let current = selected_mcver.get();
                                        view! {
                                            <SelectInput
                                                on_change=Callback::new(move |val: String| selected_mcver.set(val))
                                            >
                                                {versions.into_iter().map(|v| {
                                                    let is_selected = v.version_string == current;
                                                    let label = if v.release_type == ReleaseType::Release {
                                                        v.version_string.clone()
                                                    } else {
                                                        format!("{} [{}]", v.version_string, v.release_type)
                                                    };
                                                    view! {
                                                        <option value=v.version_string.clone() selected=is_selected>{label}</option>
                                                    }
                                                }).collect_view()}
                                            </SelectInput>
                                        }
                                    }}
                                </FormField>
                                <FormField label="Category" uppercase=true>
                                    <TextInput
                                        placeholder="e.g. Modded, Survival (optional)"
                                        default_value=category.get_untracked()
                                        on_change=Callback::new(move |v: String| category.set(v))
                                    />
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
                            <Button
                                variant=ButtonVariant::Primary
                                disabled=Signal::derive(move || {
                                    instance_name.get().trim().is_empty() || selected_mcver.get().is_empty()
                                })
                                on_click=Callback::new(move |_| {
                                    modal_step.set(3);
                                })
                            >
                                "Next →"
                            </Button>
                        </ModalFooter>
                    </Show>

                    // ── Step 3: mod loader ────────────────────────────────────
                    <Show when=move || modal_step.get() == 3 && selected_method.get() == Some(1) fallback=|| ()>
                        <ModalBody>
                            <h2 style="margin: 0 0 16px 0;">"Mod Loader"</h2>
                            <FormFields style="margin-top: 8px;">
                                <FormField label="Mod Loader" uppercase=true>
                                    {move || {
                                        let current = selected_modloader.get();
                                        view! {
                                            <SelectInput
                                                on_change=Callback::new(move |val: String| selected_modloader.set(val))
                                            >
                                                <option value="vanilla" selected={current == "vanilla"}>"Vanilla (no mod loader)"</option>
                                                <option value="forge" selected={current == "forge"}>"Forge"</option>
                                                <option value="fabric" selected={current == "fabric"}>"Fabric"</option>
                                                <option value="neoforge" selected={current == "neoforge"}>"NeoForge"</option>
                                                <option value="quilt" selected={current == "quilt"}>"Quilt"</option>
                                            </SelectInput>
                                        }
                                    }}
                                </FormField>
                                <FormField label="Mod Loader Version" uppercase=true>
                                    {move || {
                                        let is_vanilla = selected_modloader.get() == "vanilla";
                                        let candidates = if is_vanilla {
                                            Vec::new()
                                        } else {
                                            loader_versions.get().unwrap_or_default()
                                        };
                                        let current = selected_modloader_version.get();
                                        view! {
                                            <SelectInput
                                                disabled=is_vanilla
                                                on_change=Callback::new(move |val: String| selected_modloader_version.set(val))
                                            >
                                                {candidates.into_iter().map(|m| {
                                                    let is_selected = m.version == current;
                                                    view! {
                                                        <option value=m.version.clone() selected=is_selected>{m.version.clone()}</option>
                                                    }
                                                }).collect_view()}
                                            </SelectInput>
                                        }
                                    }}
                                </FormField>
                            </FormFields>
                        </ModalBody>
                        <ModalFooter>
                            <Button
                                variant=ButtonVariant::Secondary
                                on_click=Callback::new(move |_| modal_step.set(2))
                            >
                                "← Back"
                            </Button>
                            <Button
                                variant=ButtonVariant::Primary
                                disabled=Signal::derive(move || {
                                    selected_modloader.get() != "vanilla"
                                        && selected_modloader_version.get().trim().is_empty()
                                })
                                on_click=Callback::new(move |_| {
                                    let mod_loader = ModLoader::from_str(&selected_modloader.get_untracked())
                                        .unwrap_or(ModLoader::Vanilla);
                                    let mod_loader_version = if matches!(mod_loader, ModLoader::Vanilla) {
                                        None
                                    } else {
                                        let v = selected_modloader_version.get_untracked();
                                        if v.trim().is_empty() { None } else { Some(v) }
                                    };

                                    let meta = InstanceMeta {
                                        name: instance_name.get_untracked(),
                                        game_version: selected_mcver.get_untracked(),
                                        mod_loader,
                                        mod_loader_version,
                                        category: category.get_untracked(),
                                        ..InstanceMeta::default()
                                    };
                                    info!("{:?}", meta);

                                    // Defer DOM-mutating signal updates and IPC out of the
                                    // event handler so the button's RefCell event listener
                                    // is no longer borrowed when the modal unmounts.
                                    leptos::task::spawn_local(async move {
                                        show.set(false);
                                        reset();
                                        if let Some(cb) = on_creating { cb.run(meta.clone()); }

                                        let name = meta.name.clone();
                                        let args = CreateInstanceArgs { instance_meta: meta };
                                        ipc::call::<_, ()>("create_instance", args).await.ok();
                                        if let Some(cb) = on_created { cb.run(name); }
                                    });
                                })
                            >
                                "Create"
                            </Button>
                        </ModalFooter>
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
