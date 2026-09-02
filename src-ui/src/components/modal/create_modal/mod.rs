mod step_basics;
mod step_loader;
mod step_method;

use crate::components::ui::*;
use crate::curseforge::{call_get_minecraft_versions, call_get_modloader_versions};
use crate::ipc;
use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use log::info;
use yaminabe_launcher_shared::datamodels::{InstanceMeta, LoaderVersion, ModLoader, ReleaseType};

// ── IPC arg type ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateInstanceArgs {
    instance_meta: InstanceMeta,
}

// ── Shared cross-step form state ──────────────────────────────────────────────

/// Form state shared between wizard steps. `RwSignal` is `Copy`, so the whole
/// struct is `Copy` and trivially threaded through component props without
/// any clone bookkeeping.
#[derive(Copy, Clone)]
pub(super) struct WizardState {
    pub instance_name: RwSignal<String>,
    pub category: RwSignal<String>,
    pub selected_mcver: RwSignal<String>,
    pub selected_modloader: RwSignal<String>,
    pub selected_modloader_version: RwSignal<String>,
    pub include_snapshot: RwSignal<bool>,
    pub include_beta: RwSignal<bool>,
    pub include_alpha: RwSignal<bool>,
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

    let state = WizardState {
        instance_name: RwSignal::new(String::new()),
        category: RwSignal::new(String::new()),
        selected_mcver: RwSignal::new(String::new()),
        selected_modloader: RwSignal::new(String::from("vanilla")),
        selected_modloader_version: RwSignal::new(String::new()),
        include_snapshot: RwSignal::new(false),
        include_beta: RwSignal::new(false),
        include_alpha: RwSignal::new(false),
    };

    let mc_versions = LocalResource::new(|| async move {
        call_get_minecraft_versions().await.unwrap_or_default()
    });

    // Every step-3 entry kicks off one fetch per loader; `loader_pending`
    // tracks in-flight kinds so the selector can show "Loading…". Re-fetching
    // is intentional — it keeps the cache aligned with `selected_mcver`.
    let loader_versions: RwSignal<HashMap<String, Vec<LoaderVersion>>> = RwSignal::new(HashMap::new());
    let loader_pending: RwSignal<HashSet<String>> = RwSignal::new(HashSet::new());

    Effect::new(move |_| {
        let step = modal_step.get();
        let mcver = state.selected_mcver.get();
        if step != 3 || mcver.is_empty() { return; }

        loader_versions.set(HashMap::new());
        let kinds = ["forge", "fabric", "neoforge", "quilt"];
        loader_pending.set(kinds.iter().map(|s| s.to_string()).collect());

        for kind in kinds {
            let mcver = mcver.clone();
            leptos::task::spawn_local(async move {
                let versions = call_get_modloader_versions(kind, &mcver).await.unwrap_or_default();
                loader_versions.update(|map| { map.insert(kind.to_string(), versions); });
                loader_pending.update(|set| { set.remove(kind); });
            });
        }
    });

    // Forge + MC 1.0–1.5.x: the jar-mod era we don't yet support (see the
    // `NoProfile FML bootstrap blocker` memory). Gates the warning + Create.
    let is_noprofile_forge: Memo<bool> = Memo::new(move |_| {
        state.selected_modloader.get() == "forge" && {
            let parts: Vec<u32> = state.selected_mcver.get()
                .split('.').filter_map(|p| p.parse().ok()).collect();
            matches!(parts.as_slice(), [1, m, ..] if *m <= 5)
        }
    });

    let filtered_versions = Memo::new(move |_| {
        let snapshot = state.include_snapshot.get();
        let beta = state.include_beta.get();
        let alpha = state.include_alpha.get();
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
        let current = state.selected_mcver.get_untracked();
        if !versions.iter().any(|v| v.version_string == current) {
            if let Some(first) = versions.first() {
                state.selected_mcver.set(first.version_string.clone());
            } else {
                state.selected_mcver.set(String::new());
            }
        }
    });

    // Keep `selected_modloader_version` valid as the loader list changes;
    // skips while the fetch is pending and re-runs once the cache populates.
    Effect::new(move |_| {
        let kind = state.selected_modloader.get();
        if kind == "vanilla" {
            state.selected_modloader_version.set(String::new());
            return;
        }
        let map = loader_versions.get();
        let Some(candidates) = map.get(&kind) else { return; };
        let current = state.selected_modloader_version.get_untracked();
        if !candidates.iter().any(|m| m.version == current) {
            if let Some(first) = candidates.first() {
                state.selected_modloader_version.set(first.version.clone());
            } else {
                state.selected_modloader_version.set(String::new());
            }
        }
    });

    let reset = move || {
        modal_step.set(1);
        selected_method.set(None);
        state.instance_name.set(String::new());
        state.category.set(String::new());
        state.selected_modloader.set(String::from("vanilla"));
        state.selected_modloader_version.set(String::new());
    };

    // Step 3 Create handler: close modal, notify parent of pending tile, run
    // the IPC, notify parent again on success.
    let on_create = move || {
        let mod_loader = ModLoader::from_str(&state.selected_modloader.get_untracked())
            .unwrap_or(ModLoader::Vanilla);
        let mod_loader_version = if matches!(mod_loader, ModLoader::Vanilla) {
            None
        } else {
            let v = state.selected_modloader_version.get_untracked();
            if v.trim().is_empty() { None } else { Some(v) }
        };

        let meta = InstanceMeta {
            name: state.instance_name.get_untracked(),
            game_version: state.selected_mcver.get_untracked(),
            mod_loader,
            mod_loader_version,
            category: state.category.get_untracked(),
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
            if let Err(e) = ipc::call::<_, ()>("create_instance", args).await {
                log::error!("create_instance failed: {e}");
            }
            if let Some(cb) = on_created { cb.run(name); }
        });
    };

    view! {
        // ── main modal ────────────────────────────────────────────────────────
        <Show when=move || show.get()>
            <ModalOverlay>
                <ModalBox>
                    <Show when=move || modal_step.get() == 1 fallback=|| ()>
                        <step_method::StepMethod
                            selected_method=selected_method
                            on_next=Callback::new(move |_: ()| {
                                if selected_method.get_untracked() == Some(1) {
                                    modal_step.set(2);
                                }
                            })
                            on_cancel=Callback::new(move |_: ()| show_cancel_dialog.set(true))
                        />
                    </Show>

                    <Show when=move || modal_step.get() == 2 && selected_method.get() == Some(1) fallback=|| ()>
                        <step_basics::StepBasics
                            state=state
                            filtered_versions=filtered_versions
                            on_back=Callback::new(move |_: ()| modal_step.set(1))
                            on_next=Callback::new(move |_: ()| modal_step.set(3))
                        />
                    </Show>

                    <Show when=move || modal_step.get() == 3 && selected_method.get() == Some(1) fallback=|| ()>
                        <step_loader::StepLoader
                            state=state
                            loader_versions=loader_versions
                            loader_pending=loader_pending
                            is_noprofile_forge=is_noprofile_forge
                            on_back=Callback::new(move |_: ()| modal_step.set(2))
                            on_create=Callback::new(move |_: ()| on_create())
                        />
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

// ── Shared step styles ────────────────────────────────────────────────────────

pub(super) fn step_subtitle_class() -> &'static str {
    css! {
        margin: 0 0 16px 0;
        font-weight: 500;
        font-size: 1rem;
        opacity: 0.65;
    }
}