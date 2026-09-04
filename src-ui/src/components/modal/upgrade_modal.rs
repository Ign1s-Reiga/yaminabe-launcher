use crate::components::ui::*;
use crate::curseforge::{call_list_project_files, call_upgrade_modpack};
use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{IntoView, component, view, web_sys};
use leptos_router::hooks::use_navigate;
use wasm_bindgen::JsCast;
use yaminabe_launcher_shared::datamodels::{
    DownloadSource, ProjectFileInfo, ProjectFileTarget, ProjectId,
};

const PAGE_SIZE: usize = 50;

/// The CurseForge file id behind a resolved file. Upgrade is CurseForge-only,
/// so a (never-expected) non-CurseForge source sorts as 0.
fn file_id(file: &ProjectFileInfo) -> u32 {
    file.source.curseforge_ids().map(|(_, id)| id).unwrap_or(0)
}

/// Version picker for upgrading a CurseForge-origin instance to a newer modpack
/// file. Fetches the project's file list, lets the user pick a newer file (older
/// and the current file are disabled), and kicks off the upgrade — whose
/// progress flows through the install sidebar.
#[component]
pub fn UpgradeModpackModal(
    instance_id: String,
    project_id: u32,
    current_file_id: u32,
    on_close: Callback<()>,
) -> impl IntoView {
    let files: RwSignal<Vec<ProjectFileInfo>> = RwSignal::new(vec![]);
    let loading: RwSignal<bool> = RwSignal::new(true);
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    let done: RwSignal<bool> = RwSignal::new(false);
    let selected: RwSignal<u32> = RwSignal::new(0);
    let instance_id = StoredValue::new(instance_id);
    let navigate = StoredValue::new(use_navigate());

    leptos::task::spawn_local(async move {
        match call_list_project_files(
            ProjectId::CurseForge(project_id),
            ProjectFileTarget::Modpack,
            None,
            None,
            0,
        )
        .await {
            Ok(list) => {
                // Default to the newest file strictly newer than the installed one.
                let default = list
                    .iter()
                    .map(file_id)
                    .filter(|id| *id > current_file_id)
                    .max()
                    .unwrap_or(0);
                selected.set(default);
                done.set(list.len() < PAGE_SIZE);
                files.set(list);
                loading.set(false);
            }
            Err(e) => {
                error.set(Some(e));
                loading.set(false);
            }
        }
    });

    let load_more = move || {
        if loading.get_untracked() || done.get_untracked() {
            return;
        }
        let index = files.get_untracked().len() as u32;
        loading.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match call_list_project_files(
                ProjectId::CurseForge(project_id),
                ProjectFileTarget::Modpack,
                None,
                None,
                index,
            )
            .await
            {
                Ok(mut list) => {
                    done.set(list.len() < PAGE_SIZE);
                    files.update(|existing| existing.append(&mut list));
                    loading.set(false);
                }
                Err(e) => {
                    error.set(Some(e));
                    loading.set(false);
                }
            }
        });
    };

    // Loaded, with nothing newer than the installed file to offer.
    let up_to_date = Signal::derive(move || {
        !loading.get()
            && error.get().is_none()
            && files.get().iter().all(|f| file_id(f) <= current_file_id)
    });
    let can_upgrade = Signal::derive(move || selected.get() > current_file_id);

    let on_confirm = move |_ev: leptos::web_sys::MouseEvent| {
        let target_id = selected.get_untracked();
        if target_id <= current_file_id {
            return;
        }
        let iid = instance_id.get_value();
        let source = DownloadSource::CurseForge {
            project_id,
            file_id: target_id,
        };
        on_close.run(());
        // An upgrade is a whole-instance rewrite: send the user back to the
        // library and let the activity dock track progress, rather than leaving
        // them on a detail page whose contents are mid-change.
        navigate.with_value(|nav| nav("/library", Default::default()));
        leptos::task::spawn_local(async move {
            // Upgrade progress flows through the install sidebar's event stream;
            // a synchronous IPC rejection does not, so log it here.
            if let Err(e) = call_upgrade_modpack(iid, source).await {
                log::error!("upgrade_modpack failed: {e}");
            }
        });
    };

    let intro = css! {
        font-size: 0.85rem;
        opacity: 0.65;
        margin: 0 0 20px 0;
        line-height: 1.6;
    };
    let version_list = version_list_class();
    let version_note = version_note_class();
    let version_error = version_error_class();

    let rows = move || {
        let list = files.get();
        if list.is_empty() {
            let note = if loading.get() { "Loading versions…" } else { "No versions available." };
            return view! { <p class=version_note>{note}</p> }.into_any();
        }
        list.into_iter()
            .map(|file| {
                let fid = file_id(&file);
                // The installed file and anything older cannot be upgraded to.
                let older = fid <= current_file_id;
                let note = if fid == current_file_id {
                    "current"
                } else if older {
                    "older"
                } else {
                    ""
                };
                view! {
                    <VersionRow
                        label=file.display_name
                        size=file.size
                        release_type=file.release_type
                        note=note
                        disabled=older
                        selected=Signal::derive(move || selected.get() == fid)
                        on_pick=Callback::new(move |_: ()| selected.set(fid))
                    />
                }
            })
            .collect_view()
            .into_any()
    };

    let on_scroll = move |ev: leptos::ev::Event| {
        let Some(list) = ev
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let remaining = list.scroll_height() - list.scroll_top() - list.client_height();
        if remaining <= 8 {
            load_more();
        }
    };

    view! {
        <ModalOverlay>
            <ModalBox>
                <ModalBody>
                    <h2 style="margin: 0 0 12px 0;">"Upgrade Modpack"</h2>
                    <p class=intro>
                        "Choose a newer version to upgrade to. Your saves, screenshots and manual changes are kept; only changed mods are downloaded."
                    </p>
                    <FormFields>
                        <FormField label="Target Version">
                            {move || match error.get() {
                                Some(err) => view! { <p class=version_error>{err}</p> }.into_any(),
                                None => view! {
                                    <div class=version_list on:scroll=on_scroll>
                                        {rows}
                                        <Show when=move || loading.get() && !files.get().is_empty()>
                                            <p class=version_note>"Loading more…"</p>
                                        </Show>
                                    </div>
                                }.into_any(),
                            }}
                        </FormField>
                    </FormFields>
                    <Show when=move || up_to_date.get() fallback=|| ()>
                        <p style="margin: 12px 0 0 0; font-size: 0.82rem; opacity: 0.55;">
                            "This instance is already on the latest available version."
                        </p>
                    </Show>
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
                        disabled=Signal::derive(move || !can_upgrade.get())
                        on_click=Callback::new(on_confirm)
                    >
                        "Upgrade →"
                    </Button>
                </ModalFooter>
            </ModalBox>
        </ModalOverlay>
    }
}
