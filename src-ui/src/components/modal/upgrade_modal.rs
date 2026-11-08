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
    let loading: RwSignal<bool> = RwSignal::new(false);
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
        files.with(|list| {
            list.iter()
                .position(|file| file.source.version_key().as_deref() == Some(installed.as_str()))
        })
    });

    let is_older = move |index: usize| current_index.get().is_some_and(|current| index >= current);

    // With nothing loaded yet, on opening or after that failed, the next page is
    // the first one.
    let load_versions = move || {
        if loading.get_untracked() || done.get_untracked() {
            return;
        }
        let index = files.with_untracked(Vec::len) as u32;
        loading.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            let result = call_list_project_files(
                project_id.get_value(),
                ProjectFileTarget::Modpack,
                None,
                None,
                index,
            )
            .await;
            // Closing the dialog meanwhile disposed its values, and reading one
            // would panic.
            let Some(installed) = current_version.try_get_value() else { return };
            match result {
                Ok(mut list) => {
                    // The list is newest-first, so the newest upgrade target is
                    // the top of the first page — unless that is what is already
                    // installed.
                    if index == 0 {
                        let newest = list
                            .first()
                            .filter(|file| {
                                file.source.version_key().as_deref() != Some(installed.as_str())
                            })
                            .and_then(|file| file.source.version_key())
                            .unwrap_or_default();
                        selected.set(newest);
                    }
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
    load_versions();

    // Nothing above the installed version to offer. The list is newest-first,
    // so a later page still loading or failing below it can't change that.
    let up_to_date = Signal::derive(move || current_index.get() == Some(0));
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
        let Some(source) = files.with_untracked(|list| {
            list.iter()
                .find(|file| file.source.version_key().as_deref() == Some(picked.as_str()))
                .map(|file| file.source.clone())
        }) else {
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

    let empty_note = move || {
        files.with(Vec::is_empty).then(|| {
            let note = if loading.get() { "Loading versions…" } else { "No versions available." };
            view! { <p class=version_note>{note}</p> }
        })
    };

    let row = move |(index, file): (usize, ProjectFileInfo)| {
        let key = file.source.version_key().unwrap_or_default();
        let is_current = key == current_version.get_value();
        // The installed version and anything below it cannot be upgraded to.
        let older = Signal::derive(move || is_older(index));
        let note = Signal::derive(move || {
            if is_current {
                "current"
            } else if older.get() {
                "older"
            } else {
                ""
            }
        });
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
    };

    let on_scroll = move |ev: leptos::ev::Event| {
        let Some(list) = ev
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let remaining = list.scroll_height() - list.scroll_top() - list.client_height();
        // After a failed page, wait for "Try again" instead of retrying on every scroll.
        if remaining <= 8 && error.with_untracked(Option::is_none) {
            load_versions();
        }
    };

    // Only a failed first page takes the list's place. A later one shows below
    // the versions already loaded, which can still be picked.
    let first_page_error = Memo::new(move |_| error.get().filter(|_| files.with(Vec::is_empty)));

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
                            {move || match first_page_error.get() {
                                Some(err) => view! {
                                    <VersionPageError
                                        error=err
                                        on_retry=Callback::new(move |_: ()| load_versions())
                                    />
                                }.into_any(),
                                None => view! {
                                    <div class=version_list on:scroll=on_scroll>
                                        {empty_note}
                                        <For
                                            each=move || files.get().into_iter().enumerate()
                                            key=version_row_key
                                            children=row
                                        />
                                        <Show when=move || loading.get() && !files.with(Vec::is_empty)>
                                            <p class=version_note>"Loading more…"</p>
                                        </Show>
                                        {move || error.get().map(|err| view! {
                                            <VersionPageError
                                                error=err
                                                on_retry=Callback::new(move |_: ()| load_versions())
                                            />
                                        })}
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
