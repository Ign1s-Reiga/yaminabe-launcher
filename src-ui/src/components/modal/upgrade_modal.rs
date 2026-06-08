use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, IntoView, view};
use yaminabe_launcher_shared::datatypes::ModProjectFile;
use crate::components::ui::*;
use crate::curseforge::{call_get_files, call_upgrade_modpack};

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
    let files: RwSignal<Vec<ModProjectFile>> = RwSignal::new(vec![]);
    let loading: RwSignal<bool> = RwSignal::new(true);
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    let selected: RwSignal<u32> = RwSignal::new(0);
    let instance_id = StoredValue::new(instance_id);

    leptos::task::spawn_local(async move {
        match call_get_files(project_id).await {
            Ok(list) => {
                // Default to the newest file strictly newer than the installed one.
                let default = list.iter().map(|f| f.id).filter(|id| *id > current_file_id).max().unwrap_or(0);
                selected.set(default);
                files.set(list);
                loading.set(false);
            }
            Err(e) => {
                error.set(Some(e));
                loading.set(false);
            }
        }
    });

    // Loaded, with nothing newer than the installed file to offer.
    let up_to_date = Signal::derive(move || {
        !loading.get() && error.get().is_none() && files.get().iter().all(|f| f.id <= current_file_id)
    });
    let can_upgrade = Signal::derive(move || selected.get() > current_file_id);

    let on_confirm = move |_ev: leptos::web_sys::MouseEvent| {
        let target_id = selected.get_untracked();
        if target_id <= current_file_id { return; }
        let Some(file) = files.get_untracked().into_iter().find(|f| f.id == target_id) else { return; };
        let download_url = file.download_url.clone();
        let iid = instance_id.get_value();
        on_close.run(());
        leptos::task::spawn_local(async move {
            // Upgrade progress flows through the install sidebar's event stream;
            // a synchronous IPC rejection does not, so log it here.
            if let Err(e) = call_upgrade_modpack(iid, project_id, target_id, download_url).await {
                log::error!("upgrade_curseforge_modpack failed: {e}");
            }
        });
    };

    let intro = css! {
        font-size: 0.85rem;
        opacity: 0.65;
        margin: 0 0 20px 0;
        line-height: 1.6;
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
                            {move || {
                                if loading.get() {
                                    view! {
                                        <SelectInput disabled=true>
                                            <option value="">"Loading…"</option>
                                        </SelectInput>
                                    }.into_any()
                                } else if let Some(err) = error.get() {
                                    view! { <p style="margin: 0; font-size: 0.82rem; color: #c0392b;">{err}</p> }.into_any()
                                } else {
                                    let cur = current_file_id;
                                    let sel = selected.get_untracked();
                                    view! {
                                        <SelectInput on_change=Callback::new(move |v: String| selected.set(v.parse().unwrap_or(0)))>
                                            {files.get().into_iter().map(|f| {
                                                let suffix = if f.id == cur { "  (current)" }
                                                    else if f.id < cur { "  (older)" }
                                                    else { "" };
                                                let label = format!("{}  [{}]{}", f.display_name, f.release_type, suffix);
                                                let is_selected = f.id == sel;
                                                let disabled = f.id <= cur;
                                                view! {
                                                    <option value=f.id.to_string() selected=is_selected disabled=disabled>{label}</option>
                                                }
                                            }).collect_view()}
                                        </SelectInput>
                                    }.into_any()
                                }
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