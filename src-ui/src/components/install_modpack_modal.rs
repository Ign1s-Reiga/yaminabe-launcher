use bamboo_css_macro::css;
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::{component, IntoView, view};
use crate::components::ui::*;
use crate::curseforge::{ModpackInfo, ModpackVersion};

#[derive(Clone)]
pub struct InstallState {
    pub pack: ModpackInfo,
    pub version: String,
    pub versions: Vec<ModpackVersion>,
    pub versions_loading: bool,
    pub versions_error: Option<String>,
}

#[component]
pub fn InstallModpackModal(
    install: RwSignal<Option<InstallState>>,
    install_name: RwSignal<String>,
    on_submit: Callback<SubmitEvent>,
    on_close: Callback<()>,
) -> impl IntoView {
    let pack_strip = css! {
        display: flex;
        align-items: center;
        gap: 14px;
        padding: 14px;
        border-radius: 10px;
        background-color: var(--secondary-color);
        margin-bottom: 24px;
    };
    let pack_strip_logo = css! {
        width: 52px;
        height: 52px;
        border-radius: 6px;
        object-fit: cover;
        flex-shrink: 0;
    };
    let pack_strip_logo_ph = css! {
        width: 52px;
        height: 52px;
        border-radius: 6px;
        flex-shrink: 0;
        background-color: var(--background-color);
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 1.5rem;
    };
    let pack_strip_meta = css! {
        flex: 1;
        min-width: 0;
    };
    let pack_strip_name = css! {
        font-weight: 600;
        font-size: 0.95rem;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    };
    let pack_strip_summary = css! {
        font-size: 0.8rem;
        opacity: 0.55;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    };

    view! {
        <ModalOverlay>
            <ModalBox>
                <form on:submit=move |ev| on_submit.run(ev)>
                    <ModalBody>
                        <h2 style="margin: 0 0 20px 0;">"Configure Instance"</h2>

                        {move || install.get().map(|s| {
                            let pack = s.pack;
                            let logo_view = if let Some(url) = pack.logo_url.clone() {
                                view! { <img class=pack_strip_logo src=url alt=""/> }.into_any()
                            } else {
                                view! { <div class=pack_strip_logo_ph>"📦"</div> }.into_any()
                            };
                            view! {
                                <div class=pack_strip>
                                    {logo_view}
                                    <div class=pack_strip_meta>
                                        <div class=pack_strip_name>{pack.name}</div>
                                        <div class=pack_strip_summary>{pack.summary}</div>
                                    </div>
                                </div>
                            }
                        })}

                        <FormFields>
                            <FormField label="Instance Name">
                                <TextInput
                                    default_value=install_name.get_untracked()
                                    name="instance_name"
                                    placeholder="My Modpack"
                                />
                            </FormField>
                            <FormField label="Modpack Version">
                                {move || install.get().map(|s| {
                                    if let Some(err) = s.versions_error.clone() {
                                        view! {
                                            <p style="margin: 0; font-size: 0.82rem; color: #c0392b;">{err}</p>
                                        }.into_any()
                                    } else if s.versions_loading {
                                        view! {
                                            <SelectInput name="version" disabled=true>
                                                <option value="">"Loading…"</option>
                                            </SelectInput>
                                        }.into_any()
                                    } else {
                                        let selected_ver = s.version.clone();
                                        view! {
                                            <SelectInput
                                                name="version"
                                                on_change=Callback::new(move |v: String| {
                                                    install.update(|opt| {
                                                        if let Some(st) = opt { st.version = v; }
                                                    });
                                                })
                                            >
                                                {s.versions.iter().map(|v| {
                                                    let val = v.id.to_string();
                                                    let label = format!("{}  [{}]", v.display_name, v.release_type);
                                                    let is_selected = val == selected_ver;
                                                    view! { <option value=val selected=is_selected>{label}</option> }
                                                }).collect_view()}
                                            </SelectInput>
                                        }.into_any()
                                    }
                                })}
                            </FormField>
                            <FormField label="Category">
                                <TextInput
                                    name="category"
                                    placeholder="e.g. Modded, Survival (optional)"
                                />
                            </FormField>
                        </FormFields>
                    </ModalBody>
                    <ModalFooter>
                        <Button
                            variant=ButtonVariant::Secondary
                            on_click=Callback::new(move |_| on_close.run(()))
                        >
                            "Cancel"
                        </Button>
                        <Button
                            variant=ButtonVariant::Primary
                            button_type="submit"
                            disabled=Signal::derive(move || {
                                install.get().map(|s| s.versions_loading).unwrap_or(true)
                            })
                        >
                            "Install →"
                        </Button>
                    </ModalFooter>
                </form>
            </ModalBox>
        </ModalOverlay>
    }
}
