use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{IntoView, component, view};
use yaminabe_launcher_shared::datamodels::LocalModpackInfo;

use super::WizardState;
use crate::components::ui::*;
use crate::curseforge::{call_pick_modpack_file, call_read_modpack_file};
use crate::ipc;

/// Whether a dropped path looks like a modpack this step can read. The file
/// itself is still checked by the backend; this only decides which of several
/// dropped paths is worth offering it.
fn is_modpack_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".zip") || lower.ends_with(".mrpack")
}

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
    // The name this step last filled in, to tell it apart from one the user typed.
    let filled_name: StoredValue<Option<String>> = StoredValue::new(None);
    // Whether a file is currently being dragged over the window.
    let dragging: RwSignal<bool> = RwSignal::new(false);
    // Why the last dropped file was not taken, shown beside a pack that a stray
    // drop was not allowed to replace. Separate from `Picked` because it says
    // nothing about which pack is selected.
    let drop_error: RwSignal<Option<String>> = RwSignal::new(None);
    // Counts reads started, so a read that finishes after a newer one began can
    // tell that it has been superseded.
    let reads: StoredValue<u32> = StoredValue::new(0);

    // The drag highlight is a nested rule rather than a second class: the
    // generated bundle is ordered by class hash, so two classes setting the
    // same property would depend on which hash happened to sort later.
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
        &[data-dragging="true"] {
            border-color: #3a9e5f;
            background-color: var(--secondary-color);
        }
    };
    let dragging_attr = move || dragging.get().to_string();
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

    let read_path = move |path: String, from_drop: bool| {
        // Taken before `picked` is overwritten: reading it back afterwards would
        // find `Reading`, with the pack it is meant to restore already gone.
        // Only a drop restores — a file chosen through the dialog is meant to
        // replace what is there.
        let restore = from_drop
            .then(|| picked.get_untracked())
            .filter(|current| matches!(current, Picked::Pack(..)));
        // Reads run concurrently — nothing stops a second file being dropped
        // while the first is still being read. Only the newest may report, or
        // a slower earlier read would land on top of it and select the file the
        // user has already replaced.
        let generation = reads.get_value() + 1;
        reads.set_value(generation);
        picked.set(Picked::Reading);
        leptos::task::spawn_local(async move {
            let outcome = call_read_modpack_file(path.clone()).await;
            if reads.get_value() != generation {
                return;
            }
            match outcome {
                Ok(info) => {
                    // Default the instance name to the pack's own, so the common
                    // case needs no typing. A name this step filled in itself is
                    // replaced when another pack is picked; one the user typed is
                    // left alone.
                    let current = state.instance_name.get_untracked();
                    let ours = filled_name.get_value();
                    if (current.trim().is_empty() || Some(&current) == ours.as_ref())
                        && !info.name.is_empty()
                    {
                        state.instance_name.set(info.name.clone());
                        filled_name.set_value(Some(info.name.clone()));
                    }
                    drop_error.set(None);
                    picked.set(Picked::Pack(path, info));
                }
                // A file the user picked replaces whatever was there, error and
                // all. One that merely landed on the window must not: it can be
                // an unrelated archive, and discarding a pack already chosen and
                // named over it would lose work the user did on purpose.
                Err(e) => match restore {
                    Some(pack) => {
                        drop_error.set(Some(e));
                        picked.set(pack);
                    }
                    None => picked.set(Picked::Rejected(e)),
                },
            }
        });
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
            read_path(path, false);
        });
    };

    // Tauri handles OS file drops itself, so the webview never gets an HTML
    // `drop` event — the paths arrive as an event instead, and only while this
    // step is mounted. The subscriptions detach when it unmounts.
    let on_drop = move |payload: ipc::DragDropPayload| {
        dragging.set(false);
        // A drop can carry several files, or a folder, or something unrelated.
        // Take the first modpack among them rather than refusing the whole drop
        // over the company it arrived in.
        match payload.paths.into_iter().find(|path| is_modpack_file(path)) {
            Some(path) => read_path(path, true),
            // Tauri reports a drop anywhere in the window, so this fires for one
            // aimed at nothing in particular. Say what is wanted only when there
            // is nothing to lose: a pack already chosen and named must survive a
            // stray file landing on the modal.
            None => {
                let complaint = "Drop a .zip or .mrpack modpack file.".to_string();
                if matches!(picked.get_untracked(), Picked::Pack(..)) {
                    drop_error.set(Some(complaint));
                } else {
                    picked.set(Picked::Rejected(complaint));
                }
            }
        }
    };
    let subscriptions = StoredValue::new_local(Some((
        ipc::subscribe::<ipc::DragDropPayload, _>("tauri://drag-drop", on_drop),
        ipc::subscribe::<ipc::DragDropPayload, _>("tauri://drag-enter", move |_| {
            dragging.set(true)
        }),
        // Emitted with no payload at all, so it decodes as `None` rather than
        // failing and leaving the zone lit after the drag goes away.
        ipc::subscribe::<Option<ipc::DragDropPayload>, _>("tauri://drag-leave", move |_| {
            dragging.set(false)
        }),
    )));
    on_cleanup(move || subscriptions.update_value(|s| { s.take(); }));

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
                    <div class=drop_zone data-dragging=dragging_attr>
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
                    {move || drop_error.get().map(|message| view! {
                        <p class=error_text style="margin-top: 10px;">{message}</p>
                    })}
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
                    <div class=drop_zone data-dragging=dragging_attr on:click=choose>
                        <p class=drop_hint>{move || if dragging.get() { "Drop the modpack here" } else { "Click to choose another file, or drop one here" }}</p>
                    </div>
                    <p class=error_text style="margin-top: 10px;">{message}</p>
                }.into_any(),
                Picked::Nothing => view! {
                    <div class=drop_zone data-dragging=dragging_attr on:click=choose>
                        <p class=drop_hint>{move || if dragging.get() { "Drop the modpack here" } else { "Click to choose a .zip or .mrpack, or drop one here" }}</p>
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
