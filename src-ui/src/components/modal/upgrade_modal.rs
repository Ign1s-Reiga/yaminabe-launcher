use crate::components::ui::*;
use crate::curseforge::{call_list_project_files, call_upgrade_modpack};
use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{IntoView, component, view, web_sys};
use leptos_router::hooks::use_navigate;
use wasm_bindgen::JsCast;
use yaminabe_launcher_shared::datamodels::{ProjectFileInfo, ProjectFileTarget, ProjectId};

const PAGE_SIZE: usize = 50;

/// Version picker for upgrading a modpack instance to a newer pack file.
///
/// Age comes from position, not from the ids: the backend returns a project's
/// files newest-first on both platforms, so everything above the installed
/// version is newer than it. CurseForge file ids happen to sort that way and
/// Modrinth version ids do not, so comparing ids would only work on one.
#[component]
pub fn UpgradeModpackModal(
    instance_id: String,
    project_id: ProjectId,
    /// The installed file, as `DownloadSource::version_key` spells it.
    current_version: String,
    on_close: Callback<()>,
) -> impl IntoView {
    let files: RwSignal<Vec<ProjectFileInfo>> = RwSignal::new(vec![]);
    let loading: RwSignal<bool> = RwSignal::new(true);
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    let done: RwSignal<bool> = RwSignal::new(false);
    let selected: RwSignal<String> = RwSignal::new(String::new());
    let instance_id = StoredValue::new(instance_id);
    let project_id = StoredValue::new(project_id);
    let current_version = StoredValue::new(current_version);
    let navigate = StoredValue::new(use_navigate());

    // How far down the list the installed version sits. `None` while the list is
    // still short of it, which is also what a version the site has withdrawn
    // looks like — either way nothing loaded so far is older than it.
    let current_index = Memo::new(move |_| {
        let installed = current_version.get_value();
        files
            .get()
            .iter()
            .position(|file| file.source.version_key().as_deref() == Some(installed.as_str()))
    });

    let is_older = move |index: usize| current_index.get().is_some_and(|current| index >= current);

    leptos::task::spawn_local(async move {
        match call_list_project_files(
            project_id.get_value(),
            ProjectFileTarget::Modpack,
            None,
            None,
            0,
        )
        .await
        {
            Ok(list) => {
                // The list is newest-first, so the newest upgrade target is the
                // top of it — unless the top is what is already installed.
                let installed = current_version.get_value();
                let newest = list
                    .first()
                    .filter(|file| {
                        file.source.version_key().as_deref() != Some(installed.as_str())
                    })
                    .and_then(|file| file.source.version_key())
                    .unwrap_or_default();
                selected.set(newest);
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
                project_id.get_value(),
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

    // Loaded, with nothing above the installed version to offer.
    let up_to_date = Signal::derive(move || {
        !loading.get() && error.get().is_none() && current_index.get() == Some(0)
    });
    let can_upgrade = Signal::derive(move || {
        let picked = selected.get();
        !picked.is_empty() && picked != current_version.get_value()
    });

    let on_confirm = move |_ev: leptos::web_sys::MouseEvent| {
        let picked = selected.get_untracked();
        if picked.is_empty() || picked == current_version.get_value() {
            return;
        }
        // The chosen file already carries the right source for its platform, so
        // it is taken from the list rather than rebuilt from the ids.
        let Some(source) = files
            .get_untracked()
            .into_iter()
            .find(|file| file.source.version_key().as_deref() == Some(picked.as_str()))
            .map(|file| file.source)
        else {
            return;
        };
        let iid = instance_id.get_value();
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
        let installed = current_version.get_value();
        list.into_iter()
            .enumerate()
            .map(|(index, file)| {
                let key = file.source.version_key().unwrap_or_default();
                let is_current = key == installed;
                // The installed version and anything below it cannot be
                // upgraded to.
                let older = is_older(index);
                let note = if is_current {
                    "current"
                } else if older {
                    "older"
                } else {
                    ""
                };
                let picked = key.clone();
                view! {
                    <VersionRow
                        label=file.display_name
                        size=file.size
                        release_type=file.release_type
                        note=note
                        disabled=older
                        selected=Signal::derive(move || selected.get() == key)
                        on_pick=Callback::new(move |_: ()| selected.set(picked.clone()))
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
