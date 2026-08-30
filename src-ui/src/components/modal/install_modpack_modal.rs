use crate::components::ui::*;
use bamboo_css_macro::{css, cx};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::{IntoView, component, view, web_sys};
use wasm_bindgen::JsCast;
use yaminabe_launcher_shared::datamodels::{
    ModProjectInfo, ProjectFileInfo, ProjectFileReleaseType,
};

#[derive(Clone)]
pub struct InstallState {
    pub pack: ModProjectInfo,
    pub version: String,
    pub versions: Vec<ProjectFileInfo>,
    pub versions_loading: bool,
    pub versions_error: Option<String>,
    pub versions_done: bool,
}

#[component]
pub fn InstallModpackModal(
    install: RwSignal<Option<InstallState>>,
    install_name: RwSignal<String>,
    on_submit: Callback<SubmitEvent>,
    on_load_more: Callback<()>,
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
    let version_list = css! {
        display: flex;
        flex-direction: column;
        gap: 4px;
        max-height: 280px;
        overflow-y: auto;
        padding: 6px;
        border: 1px solid var(--secondary-color);
        border-radius: 10px;
        scrollbar-width: thin;
        scrollbar-color: var(--tertiary-color) transparent;
    };
    let version_row = css! {
        display: grid;
        grid-template-columns: 1fr auto auto;
        align-items: center;
        gap: 12px;
        padding: 9px 12px;
        border: 1.5px solid transparent;
        border-radius: 8px;
        cursor: pointer;
        user-select: none;
        transition: border-color 0.12s ease, background-color 0.12s ease;
        &:hover {
            border-color: rgba(58, 158, 95, 0.45);
            background-color: rgba(58, 158, 95, 0.04);
        }
    };
    let version_row_selected = css! {
        border-color: #3a9e5f;
        background-color: rgba(58, 158, 95, 0.1);
    };
    let version_name = css! {
        min-width: 0;
        font-size: 0.875rem;
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let version_size = css! {
        font-size: 0.75rem;
        opacity: 0.5;
        white-space: nowrap;
    };
    let version_note = css! {
        padding: 18px 4px;
        text-align: center;
        font-size: 0.85rem;
        opacity: 0.5;
    };
    let version_error = css! {
        margin: 0;
        font-size: 0.82rem;
        color: #c0392b;
    };
    // Shape lives on `badge`; each tone only carries its colours, so a row
    // composes the two with `cx!`.
    let badge = css! {
        display: inline-flex;
        align-items: center;
        padding: 3px 8px;
        border-radius: 999px;
        font-size: 0.68rem;
        font-weight: 700;
        letter-spacing: 0.3px;
        text-transform: uppercase;
    };
    let badge_release = css! {
        color: #3a9e5f;
        background-color: rgba(58, 158, 95, 0.14);
    };
    let badge_beta = css! {
        color: #d4a017;
        background-color: rgba(212, 160, 23, 0.14);
    };
    let badge_alpha = css! {
        color: #c0392b;
        background-color: rgba(192, 57, 43, 0.14);
    };
    let badge_unknown = css! {
        color: var(--text-color);
        background-color: var(--secondary-color);
        opacity: 0.7;
    };
    let badge_tone = move |release_type: ProjectFileReleaseType| match release_type {
        ProjectFileReleaseType::Release => badge_release,
        ProjectFileReleaseType::Beta => badge_beta,
        ProjectFileReleaseType::Alpha => badge_alpha,
        ProjectFileReleaseType::Unknown => badge_unknown,
    };

    // Split the install state into its own memos so picking a version only
    // re-runs the row classes. Re-rendering the whole list would reset its
    // scroll position on every click.
    let versions = Memo::new(move |_| {
        install.get().map(|s| s.versions).unwrap_or_default()
    });
    let selected = Memo::new(move |_| {
        install.get().map(|s| s.version).unwrap_or_default()
    });
    let loading = Memo::new(move |_| {
        install.get().map(|s| s.versions_loading).unwrap_or(true)
    });
    let versions_error = Memo::new(move |_| install.get().and_then(|s| s.versions_error));

    let on_scroll = move |ev: leptos::ev::Event| {
        let Some(list) = ev
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let remaining = list.scroll_height() - list.scroll_top() - list.client_height();
        if remaining <= 8 {
            on_load_more.run(());
        }
    };

    let rows = move || {
        let list = versions.get();
        if list.is_empty() {
            let note = if loading.get() { "Loading versions…" } else { "No versions available." };
            return view! { <p class=version_note>{note}</p> }.into_any();
        }
        list.into_iter()
            .map(|file| {
                let value = file
                    .source
                    .curseforge_ids()
                    .map(|(_, id)| id)
                    .unwrap_or(0)
                    .to_string();
                let picked = value.clone();
                let release = file.release_type.to_string();
                let tone = badge_tone(file.release_type);
                let size = format_size(file.size);
                view! {
                    <div
                        class=move || cx!(
                            version_row,
                            if selected.get() == value { version_row_selected } else { "" }
                        )
                        on:click=move |_| {
                            let value = picked.clone();
                            install.update(|state| {
                                if let Some(state) = state { state.version = value; }
                            });
                        }
                    >
                        <span class=version_name>{file.display_name}</span>
                        <span class=version_size>{size}</span>
                        <span class=cx!(badge, tone)>{release}</span>
                    </div>
                }
            })
            .collect_view()
            .into_any()
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
                                {move || match versions_error.get() {
                                    Some(err) => view! { <p class=version_error>{err}</p> }.into_any(),
                                    None => view! {
                                        // The form reads the pick by name, so the
                                        // selection rides along in a hidden field.
                                        <input type="hidden" name="version" prop:value=move || selected.get() />
                                        <div class=version_list on:scroll=on_scroll>
                                            {rows}
                                            <Show when=move || loading.get() && !versions.get().is_empty()>
                                                <p class=version_note>"Loading more…"</p>
                                            </Show>
                                        </div>
                                    }.into_any(),
                                }}
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
