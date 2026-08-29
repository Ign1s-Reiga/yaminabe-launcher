use crate::components::card::link_notice_card::LinkNoticeCard;
use crate::components::project_search::ProjectSearch;
use crate::components::ui::*;
use crate::curseforge::{
    call_download_mods, call_list_mods, call_list_project_files, call_toggle_mod_state,
};
use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{IntoView, component, view, web_sys};
use std::collections::HashSet;
use wasm_bindgen::JsCast;
use yaminabe_launcher_shared::datamodels::{
    ModListEntry, ModLoader, ModProjectInfo, ModState, Platform, ProjectFileInfo, ProjectFileTarget,
};

/// Human-readable file size.
fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let b = bytes as f64;
    if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Mods tab body for a non-Vanilla instance: lists the files in `mods/` (so
/// hand-dropped jars also show). Manual instances can add/remove mods; managed
/// modpack instances render the same list read-only.
#[component]
pub fn ModManager(
    instance_id: String,
    game_version: String,
    mod_loader: ModLoader,
    #[prop(optional)] read_only: bool,
) -> impl IntoView {
    let instance_id = StoredValue::new(instance_id);
    let game_version = StoredValue::new(game_version);
    let mod_loader = StoredValue::new(mod_loader);
    let refresh: RwSignal<u32> = RwSignal::new(0);
    let show_add: RwSignal<bool> = RwSignal::new(false);

    let mods = LocalResource::new(move || {
        refresh.track();
        let id = instance_id.get_value();
        async move { call_list_mods(id).await.unwrap_or_default() }
    });

    // Mods whose automatic download failed — surfaced by the link card so the
    // user can supply the jars from disk. Independent of `read_only`: managed
    // instances can still have failures to repair.
    let failed_mods: Signal<Vec<ModListEntry>> = Signal::derive(move || {
        mods.get()
            .map(|list| {
                list.into_iter()
                    .filter(|m| m.state == ModState::DownloadFailed)
                    .collect()
            })
            .unwrap_or_default()
    });

    let header = css! {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        margin-bottom: 16px;
    };
    let list = css! {
        display: flex;
        flex-direction: column;
        gap: 8px;
    };
    let list_read_only = css! {
        display: flex;
        flex-direction: column;
        gap: 8px;
        opacity: 0.45;
        pointer-events: none;
        user-select: none;
    };
    let hint = css! {
        font-size: 0.85rem;
        opacity: 0.6;
    };
    let row = css! {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 10px 14px;
        border: 1px solid var(--secondary-color);
        border-radius: 8px;
    };
    let mod_name = css! {
        flex: 1;
        min-width: 0;
        font-size: 0.9rem;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let mod_size = css! {
        font-size: 0.78rem;
        opacity: 0.5;
        flex-shrink: 0;
    };
    let empty = css! {
        opacity: 0.45;
        font-size: 0.9rem;
    };

    let on_toggle = move |file_name: String| {
        let id = instance_id.get_value();
        leptos::task::spawn_local(async move {
            match call_toggle_mod_state(id, file_name).await {
                Ok(()) => refresh.update(|n| *n += 1),
                Err(e) => log::error!("toggle_state_instance_mod failed: {e}"),
            }
        });
    };

    view! {
        <LinkNoticeCard
            instance_id=instance_id.get_value()
            failed=failed_mods
            on_linked=Callback::new(move |_: ()| refresh.update(|n| *n += 1))
        />

        <div class=header>
            <span class=hint>
                {if read_only {
                    "Mods installed by this modpack."
                } else {
                    "Add mods, or enable and disable them."
                }}
            </span>
            {(!read_only).then(move || view! {
                <Button variant=ButtonVariant::Primary on_click=Callback::new(move |_| show_add.set(true))>
                    "Add mod"
                </Button>
            })}
        </div>

        <div class=if read_only { list_read_only } else { list }>
            {move || match mods.get() {
                None => view! { <p class=empty>"Loading…"</p> }.into_any(),
                Some(list) if list.is_empty() => view! { <p class=empty>"No mods installed."</p> }.into_any(),
                Some(list) => list.into_iter().map(|m| {
                    let name = m.file_name.clone();
                    let file_for_toggle = m.file_name.clone();
                    let state = m.state;
                    let name_style = if state == ModState::Enabled { "" } else { "opacity: 0.5;" };
                    view! {
                        <div class=row>
                            <span class=mod_name style=name_style>{name}</span>
                            <span class=mod_size>{format_size(m.size)}</span>
                            {(!read_only).then(move || match state {
                                ModState::DownloadFailed => view! {
                                    <span class=mod_size style="color: #d4a017;">"Download failed"</span>
                                }.into_any(),
                                _ => view! {
                                    <Switch
                                        checked=Signal::derive(move || state == ModState::Enabled)
                                        on_change=Callback::new(move |_| on_toggle(file_for_toggle.clone()))
                                        label="Enable mod"
                                    />
                                }.into_any(),
                            })}
                        </div>
                    }
                }).collect_view().into_any(),
            }}
        </div>

        <Show when=move || !read_only && show_add.get()>
            <AddModModal
                instance_id=instance_id.get_value()
                game_version=game_version.get_value()
                mod_loader=mod_loader.get_value()
                on_installed=Callback::new(move |_: ()| refresh.update(|n| *n += 1))
                on_close=Callback::new(move |_: ()| show_add.set(false))
            />
        </Show>
    }
}

#[component]
fn AddModModal(
    instance_id: String,
    game_version: String,
    mod_loader: ModLoader,
    on_installed: Callback<()>,
    on_close: Callback<()>,
) -> impl IntoView {
    let instance_id = StoredValue::new(instance_id);
    let game_version = StoredValue::new(game_version);
    let mod_loader = StoredValue::new(mod_loader);

    let selected_project: RwSignal<Option<ModProjectInfo>> = RwSignal::new(None);
    let versions: RwSignal<Vec<ProjectFileInfo>> = RwSignal::new(vec![]);
    let versions_loading: RwSignal<bool> = RwSignal::new(false);
    let versions_error: RwSignal<Option<String>> = RwSignal::new(None);
    let versions_done: RwSignal<bool> = RwSignal::new(false);
    let selected_file_id: RwSignal<String> = RwSignal::new(String::new());
    let installing: RwSignal<HashSet<u32>> = RwSignal::new(HashSet::new());
    let installed: RwSignal<HashSet<u32>> = RwSignal::new(HashSet::new());

    let load_versions = move |project_id: u32, append: bool| {
        if versions_loading.get_untracked() || (append && versions_done.get_untracked()) {
            return;
        }
        let index = if append {
            versions.get_untracked().len() as u32
        } else {
            0
        };
        versions_loading.set(true);
        versions_error.set(None);
        if !append {
            versions.set(vec![]);
            selected_file_id.set(String::new());
            versions_done.set(false);
        }
        let game_version = Some(game_version.get_value());
        let mod_loader = Some(mod_loader.get_value());
        leptos::task::spawn_local(async move {
            match call_list_project_files(
                project_id,
                ProjectFileTarget::Mod,
                game_version,
                mod_loader,
                index,
            )
            .await
            {
                Ok(mut files) => {
                    if files.len() < 50 {
                        versions_done.set(true);
                    }
                    if append {
                        versions.update(|existing| existing.append(&mut files));
                    } else {
                        let first_id = files
                            .first()
                            .and_then(|file| file.source.curseforge_ids())
                            .map(|(_, file_id)| file_id.to_string())
                            .unwrap_or_default();
                        selected_file_id.set(first_id);
                        versions.set(files);
                    }
                    versions_loading.set(false);
                }
                Err(e) => {
                    versions_error.set(Some(e));
                    versions_loading.set(false);
                }
            }
        });
    };

    let on_details = move |project: ModProjectInfo| {
        // The version/download flow is CurseForge-only for now; Modrinth search
        // results are display-only.
        if project.platform != Platform::CurseForge {
            return;
        }
        let project_id = project.id;
        selected_project.set(Some(project));
        load_versions(project_id, false);
    };

    let on_add = move |project_id: u32| {
        if installing.get_untracked().contains(&project_id)
            || installed.get_untracked().contains(&project_id)
        {
            return;
        }
        let file_id = selected_file_id.get_untracked();
        let Some(file) = versions.get_untracked().into_iter().find(|file| {
            file.source.curseforge_ids().map(|(_, id)| id.to_string()) == Some(file_id.clone())
        }) else {
            return;
        };
        installing.update(|s| {
            s.insert(project_id);
        });
        let id = instance_id.get_value();
        leptos::task::spawn_local(async move {
            let result = call_download_mods(vec![file], id).await;
            installing.update(|s| {
                s.remove(&project_id);
            });
            match result {
                Ok(()) => {
                    installed.update(|s| {
                        s.insert(project_id);
                    });
                    on_installed.run(());
                }
                Err(e) => log::error!("add mod failed: {e}"),
            }
        });
    };

    let results_list = css! {
        margin-top: 16px;
        max-height: 360px;
        overflow-y: auto;
        scrollbar-width: thin;
        display: flex;
        flex-direction: column;
        gap: 8px;
    };
    let card = css! {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 10px 12px;
        border: 1px solid var(--secondary-color);
        border-radius: 8px;
    };
    let logo = css! {
        width: 44px;
        height: 44px;
        border-radius: 6px;
        object-fit: cover;
        flex-shrink: 0;
        background-color: var(--secondary-color);
    };
    let logo_ph = css! {
        width: 44px;
        height: 44px;
        border-radius: 6px;
        flex-shrink: 0;
        background-color: var(--secondary-color);
        display: flex;
        align-items: center;
        justify-content: center;
    };
    let body = css! {
        flex: 1;
        min-width: 0;
    };
    let name = css! {
        font-weight: 600;
        font-size: 0.9rem;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let summary = css! {
        font-size: 0.78rem;
        opacity: 0.55;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let status_area = css! {
        text-align: center;
        opacity: 0.5;
        font-size: 0.85rem;
        padding: 28px 0;
    };

    view! {
        <ModalOverlay>
            <ModalBox size=ModalSize::Large>
                <ModalBody>
                    <h2 style="margin: 0 0 16px 0;">"Add Mod"</h2>

                    // Hidden (not unmounted) while a project's versions are
                    // shown, so the search results survive a Back.
                    <div style=move || if selected_project.get().is_some() {
                        "display: none;"
                    } else {
                        "display: flex; flex-direction: column; height: 440px;"
                    }>
                        <ProjectSearch
                            target=ProjectFileTarget::Mod
                            game_version=game_version.get_value()
                            mod_loader=mod_loader.get_value()
                            placeholder="Search mods…"
                            action_label="Details"
                            empty_message="No compatible mods found."
                            search_on_open=true
                            on_select=Callback::new(move |p: ModProjectInfo| on_details(p))
                        />
                    </div>

                    {move || selected_project.get().map(|project| {
                        let pid = project.id;
                        let logo_view = if let Some(url) = project.logo_url.clone() {
                            view! { <img class=logo src=url alt="" /> }.into_any()
                        } else {
                            view! { <div class=logo_ph>"📦"</div> }.into_any()
                        };
                        view! {
                            <div class=results_list>
                                <div class=card>
                                    {logo_view}
                                    <div class=body>
                                        <div class=name>{project.name}</div>
                                        <div class=summary>{project.summary}</div>
                                    </div>
                                </div>
                                {move || {
                                    if let Some(e) = versions_error.get() {
                                        view! { <div class=status_area>{e}</div> }.into_any()
                                    } else if versions_loading.get() && versions.get().is_empty() {
                                        view! { <div class=status_area>"Loading versions…"</div> }.into_any()
                                    } else if versions.get().is_empty() {
                                        view! { <div class=status_area>"No compatible versions found."</div> }.into_any()
                                    } else {
                                        view! {
                                            <select
                                                class=select_class()
                                                size="8"
                                                prop:value=move || selected_file_id.get()
                                                on:change=move |ev| selected_file_id.set(event_target_value(&ev))
                                                on:scroll=move |ev| {
                                                    let Some(select) = ev.target()
                                                        .and_then(|target| target.dyn_into::<web_sys::HtmlSelectElement>().ok())
                                                    else { return; };
                                                    let remaining = select.scroll_height() - select.scroll_top() - select.client_height();
                                                    if remaining <= 8 {
                                                        load_versions(pid, true);
                                                    }
                                                }
                                            >
                                                {move || versions.get().into_iter().map(|file| {
                                                    let val = file.source.curseforge_ids()
                                                        .map(|(_, id)| id.to_string())
                                                        .unwrap_or_default();
                                                    let label = format!("{}  [{}]", file.display_name, file.release_type);
                                                    let is_selected = val == selected_file_id.get();
                                                    view! { <option value=val selected=is_selected>{label}</option> }
                                                }).collect_view()}
                                            </select>
                                            <div style="display: flex; justify-content: space-between; align-items: center; gap: 10px;">
                                                <Button
                                                    variant=ButtonVariant::Secondary
                                                    on_click=Callback::new(move |_| selected_project.set(None))
                                                >
                                                    "Back"
                                                </Button>
                                                <Button
                                                    variant=ButtonVariant::Primary
                                                    disabled=Signal::derive(move || {
                                                        selected_file_id.get().is_empty()
                                                            || versions_loading.get()
                                                            || installing.get().contains(&pid)
                                                            || installed.get().contains(&pid)
                                                    })
                                                    on_click=Callback::new(move |_| on_add(pid))
                                                >
                                                    {move || if installed.get().contains(&pid) {
                                                        "Added"
                                                    } else if installing.get().contains(&pid) {
                                                        "Adding…"
                                                    } else {
                                                        "Add"
                                                    }}
                                                </Button>
                                            </div>
                                        }.into_any()
                                    }
                                }}
                            </div>
                        }
                    })}
                </ModalBody>
                <ModalFooter>
                    <Button variant=ButtonVariant::Secondary on_click=Callback::new(move |_| on_close.run(()))>
                        "Close"
                    </Button>
                </ModalFooter>
            </ModalBox>
        </ModalOverlay>
    }
}
