use crate::components::create_modal::{step_subtitle_class, WizardState};
use crate::components::ui::*;
use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use yaminabe_launcher_shared::datatypes::{GameVersion, ReleaseType};

#[component]
pub fn StepBasics(
    state: WizardState,
    filtered_versions: Memo<Vec<GameVersion>>,
    on_back: Callback<()>,
    on_next: Callback<()>,
) -> impl IntoView {
    let WizardState {
        instance_name,
        category,
        selected_mcver,
        include_snapshot,
        include_beta,
        include_alpha,
        ..
    } = state;

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
        <ModalBody>
            <h2 style="margin: 0 0 4px 0;">"Create Manually"</h2>
            <h3 class=step_subtitle_class()>"Set Up Instance"</h3>
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
                on_click=Callback::new(move |_| on_back.run(()))
            >
                "← Back"
            </Button>
            <Button
                variant=ButtonVariant::Primary
                disabled=Signal::derive(move || {
                    instance_name.get().trim().is_empty() || selected_mcver.get().is_empty()
                })
                on_click=Callback::new(move |_| on_next.run(()))
            >
                "Next →"
            </Button>
        </ModalFooter>
    }
}