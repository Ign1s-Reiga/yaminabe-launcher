use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{IntoView, component, view};
use yaminabe_launcher_shared::datamodels::LocalModpackInfo;

use super::WizardState;
use crate::components::ui::*;
use crate::curseforge::{call_pick_modpack_file, call_read_modpack_file};

/// What the picker knows about the file so far. One value rather than a pile of
/// options, so "read a pack but also hold an error" cannot be represented.
#[derive(Clone, PartialEq)]
enum Picked {
    Nothing,
    Reading,
    /// A readable CurseForge modpack, with the path to install from.
    Pack(String, LocalModpackInfo),
    /// The file was chosen but is not a modpack this can install.
    Rejected(String),
}

/// Step 2 for the import method: choose a modpack zip already on disk, confirm
/// it is the intended pack, and name the instance it becomes.
///
/// The manifest is read as soon as a file is picked, so a zip that is not a
/// CurseForge modpack is refused here rather than part-way through an install
/// that has already created a directory.
#[component]
pub fn StepImport(
    state: WizardState,
    on_back: Callback<()>,
    /// Fired with the chosen zip's path once a name is set.
    on_install: Callback<String>,
) -> impl IntoView {
    let picked: RwSignal<Picked> = RwSignal::new(Picked::Nothing);

    let drop_zone = css! {
        border: 2px dashed var(--tertiary-color);
        border-radius: 10px;
        padding: 24px 16px;
        text-align: center;
        cursor: pointer;
        transition: border-color 0.12s ease, background-color 0.12s ease;
        &:hover {
            border-color: #3a9e5f;
            background-color: var(--secondary-color);
        }
    };
    let drop_hint = css! {
        margin: 0;
        font-size: 0.85rem;
        opacity: 0.6;
    };
    let pack_card = css! {
        display: flex;
        flex-direction: column;
        gap: 4px;
        padding: 14px 16px;
        border: 1px solid var(--secondary-color);
        border-radius: 10px;
        background-color: var(--secondary-color);
    };
    let pack_name = css! {
        font-weight: 600;
        font-size: 0.95rem;
    };
    let pack_meta = css! {
        font-size: 0.8rem;
        opacity: 0.6;
    };
    let pack_path = css! {
        font-size: 0.72rem;
        opacity: 0.45;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let error_text = css! {
        margin: 0;
        font-size: 0.82rem;
        color: #c0392b;
        line-height: 1.5;
    };

    let choose = move |_| {
        picked.set(Picked::Reading);
        leptos::task::spawn_local(async move {
            let Ok(Some(path)) = call_pick_modpack_file().await else {
                // Cancelled, or the dialog failed: fall back to the idle prompt
                // rather than reporting a rejection the user did not cause.
                picked.set(Picked::Nothing);
                return;
            };
            match call_read_modpack_file(path.clone()).await {
                Ok(info) => {
                    // Default the instance name to the pack's own, so the common
                    // case needs no typing.
                    if state.instance_name.get_untracked().trim().is_empty() && !info.name.is_empty()
                    {
                        state.instance_name.set(info.name.clone());
                    }
                    picked.set(Picked::Pack(path, info));
                }
                Err(e) => picked.set(Picked::Rejected(e)),
            }
        });
    };

    let chosen_path = move || match picked.get() {
        Picked::Pack(path, _) => Some(path),
        _ => None,
    };
    let ready = Signal::derive(move || {
        chosen_path().is_some() && !state.instance_name.get().trim().is_empty()
    });

    view! {
        <ModalBody>
            <h2 style="margin: 0 0 16px 0;">"Import Modpack"</h2>

            {move || match picked.get() {
                Picked::Reading => view! {
                    <div class=drop_zone>
                        <p class=drop_hint>"Reading modpack…"</p>
                    </div>
                }.into_any(),
                Picked::Pack(path, info) => {
                    let hover = path.clone();
                    view! {
                    <div class=pack_card>
                        <span class=pack_name>
                            {if info.name.is_empty() { "Modpack".to_string() } else { info.name }}
                        </span>
                        <span class=pack_meta>
                            {format!(
                                "{} · MC {} · {}{} · {} files",
                                info.format,
                                info.game_version,
                                info.mod_loader,
                                if info.version.is_empty() {
                                    String::new()
                                } else {
                                    format!(" · {}", info.version)
                                },
                                info.file_count,
                            )}
                        </span>
                        <span class=pack_path title=hover>{path}</span>
                    </div>
                    <div style="margin-top: 10px;">
                        <Button
                            variant=ButtonVariant::Secondary
                            size=ButtonSize::Small
                            on_click=Callback::new(choose)
                        >
                            "Choose a different file"
                        </Button>
                    </div>
                }.into_any()
                },
                Picked::Rejected(message) => view! {
                    <div class=drop_zone on:click=choose>
                        <p class=drop_hint>"Click to choose another file"</p>
                    </div>
                    <p class=error_text style="margin-top: 10px;">{message}</p>
                }.into_any(),
                Picked::Nothing => view! {
                    <div class=drop_zone on:click=choose>
                        <p class=drop_hint>"Click to choose a .zip or .mrpack"</p>
                    </div>
                }.into_any(),
            }}

            <Show when=move || chosen_path().is_some()>
                <div style="margin-top: 20px;">
                    <FormFields>
                        <FormField label="Instance Name">
                            <input
                                class=input_class()
                                type="text"
                                placeholder="My Modpack"
                                prop:value=move || state.instance_name.get()
                                on:input=move |ev| state.instance_name.set(event_target_value(&ev))
                            />
                        </FormField>
                        <FormField label="Category">
                            <input
                                class=input_class()
                                type="text"
                                placeholder="e.g. Modded, Survival (optional)"
                                prop:value=move || state.category.get()
                                on:input=move |ev| state.category.set(event_target_value(&ev))
                            />
                        </FormField>
                    </FormFields>
                </div>
            </Show>
        </ModalBody>
        <ModalFooter>
            <Button
                variant=ButtonVariant::Secondary
                on_click=Callback::new(move |_| on_back.run(()))
            >
                "← Back"
            </Button>
            <Button
                variant=ButtonVariant::Primary
                disabled=Signal::derive(move || !ready.get())
                on_click=Callback::new(move |_| {
                    if let Some(path) = chosen_path() {
                        on_install.run(path);
                    }
                })
            >
                "Install →"
            </Button>
        </ModalFooter>
    }
}
