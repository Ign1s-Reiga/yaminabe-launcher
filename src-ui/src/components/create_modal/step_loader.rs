//! Step 3 of the create-instance wizard: mod loader + loader version.
//!
//! The loader-version dropdown reads from a shared per-loader cache the
//! shell prefills on step-3 entry. When the user picks Forge against a
//! pre-1.6 MC version, `is_noprofile_forge` lights up and disables Create —
//! that era of Forge needs an FML bootstrap-lib table the launcher doesn't
//! yet ship (see the `NoProfile FML bootstrap blocker` memory).

use crate::components::create_modal::{step_subtitle_class, WizardState};
use crate::components::ui::*;
use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use std::collections::{HashMap, HashSet};
use yaminabe_launcher_shared::datatypes::LoaderVersion;

#[component]
pub fn StepLoader(
    state: WizardState,
    loader_versions: RwSignal<HashMap<String, Vec<LoaderVersion>>>,
    loader_pending: RwSignal<HashSet<String>>,
    is_noprofile_forge: Memo<bool>,
    on_back: Callback<()>,
    on_create: Callback<()>,
) -> impl IntoView {
    let WizardState {
        selected_modloader,
        selected_modloader_version,
        ..
    } = state;

    let noprofile_warning = css! {
        margin: 8px 0 0 0;
        padding: 8px 12px;
        background-color: rgba(192, 57, 43, 0.08);
        color: #c0392b;
        border-radius: 6px;
        font-size: 0.82rem;
        line-height: 1.45;
    };

    view! {
        <ModalBody>
            <h2 style="margin: 0 0 4px 0;">"Create Manually"</h2>
            <h3 class=step_subtitle_class()>"Select a Mod Loader"</h3>
            <FormFields style="margin-top: 8px;">
                <FormField label="Mod Loader" uppercase=true>
                    <SegmentedControl
                        items=vec![
                            ("vanilla", "Vanilla"),
                            ("forge", "Forge"),
                            ("fabric", "Fabric"),
                            ("neoforge", "NeoForge"),
                            ("quilt", "Quilt"),
                        ]
                        selected=selected_modloader
                        on_change=Callback::new(move |val: String| selected_modloader.set(val))
                    />
                </FormField>
                <FormField label="Mod Loader Version" uppercase=true>
                    {move || {
                        let kind = selected_modloader.get();
                        let is_vanilla = kind == "vanilla";
                        let is_loading = !is_vanilla && loader_pending.get().contains(&kind);
                        let candidates = if is_vanilla {
                            Vec::new()
                        } else {
                            loader_versions.get().get(&kind).cloned().unwrap_or_default()
                        };
                        let current = selected_modloader_version.get();
                        view! {
                            <SelectInput
                                disabled=is_vanilla || is_loading
                                on_change=Callback::new(move |val: String| selected_modloader_version.set(val))
                            >
                                {if is_loading {
                                    view! { <option disabled selected>"Loading…"</option> }.into_any()
                                } else {
                                    candidates.into_iter().map(|m| {
                                        let is_selected = m.version == current;
                                        view! {
                                            <option value=m.version.clone() selected=is_selected>{m.version.clone()}</option>
                                        }
                                    }).collect_view().into_any()
                                }}
                            </SelectInput>
                        }
                    }}
                    <Show when=move || is_noprofile_forge.get()>
                        <p class=noprofile_warning>
                            "NoProfile (pre-1.6 jar-mod) Forge is not currently supported. Please pick MC 1.6 or later."
                        </p>
                    </Show>
                </FormField>
            </FormFields>
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
                disabled=Signal::derive(move || {
                    is_noprofile_forge.get()
                        || (selected_modloader.get() != "vanilla"
                            && selected_modloader_version.get().trim().is_empty())
                })
                on_click=Callback::new(move |_| on_create.run(()))
            >
                "Create"
            </Button>
        </ModalFooter>
    }
}