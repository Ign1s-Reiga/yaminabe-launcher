use std::collections::HashSet;
use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, web_sys, IntoView, view};
use yaminabe_launcher_shared::datatypes::{ModLoader, ModpackInfo};
use crate::components::ui::*;
use crate::curseforge::{call_delete_mod, call_install_mod, call_list_mods, call_search_mods};

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

/// Mods tab body for a manually-created, non-Vanilla instance: lists the files
/// in `mods/` (so hand-dropped jars also show) with a Remove action, plus an
/// Add-mod search flow filtered to the instance's MC version + loader.
#[component]
pub fn ModManager(
    instance_id: String,
    game_version: String,
    mod_loader: ModLoader,
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

    let header = css! {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        margin-bottom: 16px;
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
        margin-bottom: 8px;
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

    let on_remove = move |file_name: String| {
        let id = instance_id.get_value();
        leptos::task::spawn_local(async move {
            match call_delete_mod(id, file_name).await {
                Ok(()) => refresh.update(|n| *n += 1),
                Err(e) => log::error!("delete_instance_mod failed: {e}"),
            }
        });
    };

    view! {
        <div class=header>
            <span class=hint>"Add or remove mods for this instance."</span>
            <Button variant=ButtonVariant::Primary on_click=Callback::new(move |_| show_add.set(true))>
                "Add mod"
            </Button>
        </div>

        {move || match mods.get() {
            None => view! { <p class=empty>"Loading…"</p> }.into_any(),
            Some(list) if list.is_empty() => view! { <p class=empty>"No mods installed."</p> }.into_any(),
            Some(list) => list.into_iter().map(|m| {
                let name = m.file_name.clone();
                let name_for_remove = m.file_name.clone();
                view! {
                    <div class=row>
                        <span class=mod_name>{name}</span>
                        <span class=mod_size>{format_size(m.size)}</span>
                        <Button
                            variant=ButtonVariant::Danger
                            on_click=Callback::new(move |_| on_remove(name_for_remove.clone()))
                        >
                            "Remove"
                        </Button>
                    </div>
                }
            }).collect_view().into_any(),
        }}

        <Show when=move || show_add.get()>
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

    let query: RwSignal<String> = RwSignal::new(String::new());
    let results: RwSignal<Vec<ModpackInfo>> = RwSignal::new(vec![]);
    let loading: RwSignal<bool> = RwSignal::new(false);
    let searched: RwSignal<bool> = RwSignal::new(false);
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    // Per-project install lifecycle, keyed by CurseForge project id.
    let installing: RwSignal<HashSet<u32>> = RwSignal::new(HashSet::new());
    let installed: RwSignal<HashSet<u32>> = RwSignal::new(HashSet::new());

    let do_search = move || {
        let q = query.get_untracked();
        if q.trim().is_empty() { return; }
        loading.set(true);
        error.set(None);
        let mcv = game_version.get_value();
        let loader = mod_loader.get_value();
        leptos::task::spawn_local(async move {
            match call_search_mods(q, mcv, loader, 0).await {
                Ok(res) => {
                    results.set(res.items);
                    searched.set(true);
                    loading.set(false);
                }
                Err(e) => {
                    error.set(Some(e));
                    loading.set(false);
                }
            }
        });
    };

    let on_add = move |project_id: u32| {
        if installing.get_untracked().contains(&project_id) || installed.get_untracked().contains(&project_id) {
            return;
        }
        installing.update(|s| { s.insert(project_id); });
        let id = instance_id.get_value();
        leptos::task::spawn_local(async move {
            match call_install_mod(id, project_id).await {
                Ok(()) => {
                    installing.update(|s| { s.remove(&project_id); });
                    installed.update(|s| { s.insert(project_id); });
                    on_installed.run(());
                }
                Err(e) => {
                    installing.update(|s| { s.remove(&project_id); });
                    log::error!("install_curseforge_mod failed: {e}");
                }
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
            <ModalBox>
                <ModalBody>
                    <h2 style="margin: 0 0 16px 0;">"Add Mod"</h2>
                    <div class=css! { display: flex; gap: 10px; }>
                        <input
                            class=input_class()
                            style="flex: 1; width: auto;"
                            type="text"
                            placeholder="Search mods on CurseForge…"
                            prop:value=move || query.get()
                            on:input=move |ev| query.set(event_target_value(&ev))
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                if ev.key() == "Enter" { do_search(); }
                            }
                        />
                        <Button variant=ButtonVariant::Primary on_click=Callback::new(move |_| do_search())>
                            "Search"
                        </Button>
                    </div>

                    {move || {
                        if loading.get() {
                            view! { <div class=status_area>"Searching…"</div> }.into_any()
                        } else if let Some(e) = error.get() {
                            view! { <div class=status_area>{e}</div> }.into_any()
                        } else if searched.get() && results.get().is_empty() {
                            view! { <div class=status_area>"No compatible mods found."</div> }.into_any()
                        } else {
                            view! {
                                <div class=results_list>
                                    {move || results.get().into_iter().map(|pack| {
                                        let pid = pack.id;
                                        let logo_view = if let Some(url) = pack.logo_url.clone() {
                                            view! { <img class=logo src=url alt="" /> }.into_any()
                                        } else {
                                            view! { <div class=logo_ph>"📦"</div> }.into_any()
                                        };
                                        view! {
                                            <div class=card>
                                                {logo_view}
                                                <div class=body>
                                                    <div class=name>{pack.name.clone()}</div>
                                                    <div class=summary>{pack.summary.clone()}</div>
                                                </div>
                                                <Button
                                                    variant=ButtonVariant::Primary
                                                    disabled=Signal::derive(move || {
                                                        installing.get().contains(&pid) || installed.get().contains(&pid)
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
                                        }
                                    }).collect_view()}
                                </div>
                            }.into_any()
                        }
                    }}
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