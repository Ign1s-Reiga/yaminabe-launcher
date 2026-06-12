use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use serde::Deserialize;
use yaminabe_launcher_shared::datatypes::ModListEntry;
use crate::components::ui::*;
use crate::curseforge::{call_link_mods, call_pick_jar_files};
use crate::ipc;

/// Payload of Tauri's `tauri://drag-drop` event — only the dropped file paths
/// are needed here.
#[derive(Deserialize)]
struct DragDropPayload {
    paths: Vec<String>,
}

/// Mods-tab notice shown when one or more mods failed to download. Explains the
/// situation and opens a modal where the user links jars from disk by SHA-1.
/// Rendered below `ManagedNoticeCard` for managed instances.
#[component]
pub fn LinkNoticeCard(
    instance_id: String,
    /// The `DownloadFailed` entries needing a hand-supplied jar.
    failed: Signal<Vec<ModListEntry>>,
    /// Fired after at least one jar is successfully linked, so the Mods tab
    /// reloads its list.
    on_linked: Callback<()>,
) -> impl IntoView {
    let instance_id = StoredValue::new(instance_id);
    let open: RwSignal<bool> = RwSignal::new(false);

    let card = css! {
        width: 100%;
        box-sizing: border-box;
        padding: 16px 18px;
        margin-top: 16px;
        border: 1px solid #d4a017;
        border-radius: 10px;
        background-color: rgb(212 160 23 / 0.08);
    };
    let title = css! {
        font-weight: 600;
        margin: 0 0 6px 0;
    };
    let body = css! {
        margin: 0;
        font-size: 0.875rem;
        opacity: 0.75;
        line-height: 1.6;
    };

    let count = move || failed.get().len();
    let has_failed = move || !failed.get().is_empty();

    view! {
        <Show when=has_failed>
            <div class=card>
                <p class=title>"Some mods need manual installation"</p>
                <p class=body>
                    {move || format!(
                        "{} mod file(s) could not be downloaded automatically (the author may have \
                         disabled third-party downloads). Link the matching jars from your computer \
                         to finish setting up this instance.",
                        count(),
                    )}
                </p>
                <div style="margin-top: 14px;">
                    <Button variant=ButtonVariant::Primary on_click=Callback::new(move |_| open.set(true))>
                        "Link mods…"
                    </Button>
                </div>
            </div>

            <Show when=move || open.get()>
                <LinkModal
                    instance_id=instance_id.get_value()
                    failed=failed
                    on_linked=on_linked
                    on_close=Callback::new(move |_: ()| open.set(false))
                />
            </Show>
        </Show>
    }
}

#[component]
fn LinkModal(
    instance_id: String,
    failed: Signal<Vec<ModListEntry>>,
    on_linked: Callback<()>,
    on_close: Callback<()>,
) -> impl IntoView {
    let instance_id = StoredValue::new(instance_id);
    let staged: RwSignal<Vec<String>> = RwSignal::new(vec![]);
    let linking: RwSignal<bool> = RwSignal::new(false);
    let message: RwSignal<Option<String>> = RwSignal::new(None);

    // Append dropped `.jar` paths to the staged list while the modal is open.
    // The subscription detaches when the modal unmounts (its `on_cleanup`).
    let add_paths = move |paths: Vec<String>| {
        staged.update(|s| {
            for path in paths {
                if path.to_lowercase().ends_with(".jar") && !s.contains(&path) {
                    s.push(path);
                }
            }
        });
    };
    let subscription = ipc::subscribe::<DragDropPayload, _>(
        "tauri://drag-drop",
        move |payload| add_paths(payload.paths),
    );
    let subscription = StoredValue::new_local(Some(subscription));
    on_cleanup(move || subscription.update_value(|s| { s.take(); }));

    let browse = move |_| {
        leptos::task::spawn_local(async move {
            if let Ok(paths) = call_pick_jar_files().await {
                add_paths(paths);
            }
        });
    };

    let on_link = move |_| {
        let paths = staged.get_untracked();
        if paths.is_empty() || linking.get_untracked() { return; }
        linking.set(true);
        message.set(None);
        let iid = instance_id.get_value();
        leptos::task::spawn_local(async move {
            match call_link_mods(iid, paths).await {
                Ok(outcome) => {
                    linking.set(false);
                    staged.set(vec![]);
                    if !outcome.linked.is_empty() {
                        on_linked.run(());
                    }
                    if outcome.unmatched.is_empty() {
                        on_close.run(());
                    } else {
                        message.set(Some(format!(
                            "Linked {}. {} file(s) matched no pending mod.",
                            outcome.linked.len(),
                            outcome.unmatched.len(),
                        )));
                    }
                }
                Err(e) => {
                    linking.set(false);
                    message.set(Some(e));
                }
            }
        });
    };

    let needed_list = css! {
        margin: 0 0 16px 0;
        padding: 0;
        list-style: none;
        max-height: 140px;
        overflow-y: auto;
        scrollbar-width: thin;
        display: flex;
        flex-direction: column;
        gap: 4px;
    };
    let needed_item = css! {
        font-size: 0.82rem;
        padding: 6px 10px;
        border: 1px solid var(--secondary-color);
        border-radius: 6px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let drop_zone = css! {
        border: 2px dashed var(--tertiary-color);
        border-radius: 10px;
        padding: 24px 16px;
        text-align: center;
        cursor: pointer;
        transition: border-color 0.12s ease, background-color 0.12s ease;
        &:hover { border-color: #3a9e5f; background-color: var(--secondary-color); }
    };
    let drop_hint = css! {
        font-size: 0.85rem;
        opacity: 0.7;
    };
    let staged_list = css! {
        margin: 14px 0 0 0;
        padding: 0;
        list-style: none;
        display: flex;
        flex-direction: column;
        gap: 4px;
    };
    let staged_item = css! {
        display: flex;
        align-items: center;
        gap: 8px;
        font-size: 0.8rem;
    };
    let label = css! {
        font-size: 0.78rem;
        font-weight: 600;
        opacity: 0.6;
        text-transform: uppercase;
        letter-spacing: 0.4px;
        margin: 0 0 8px 0;
    };
    let msg = css! {
        margin: 12px 0 0 0;
        font-size: 0.82rem;
        opacity: 0.75;
    };

    let base_name = |path: &str| path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();

    view! {
        <ModalOverlay>
            <ModalBox>
                <ModalBody>
                    <h2 style="margin: 0 0 16px 0;">"Link mods"</h2>

                    <p class=label>"Needs linking"</p>
                    <ul class=needed_list>
                        {move || failed.get().into_iter().map(|m| {
                            let name = m.file_name;
                            let title = name.clone();
                            view! { <li class=needed_item title=title>{name}</li> }
                        }).collect_view()}
                    </ul>

                    <div class=drop_zone on:click=browse>
                        <p class=drop_hint>"Drop .jar files here, or click to browse"</p>
                    </div>

                    <Show when=move || !staged.get().is_empty()>
                        <p class=label style="margin-top: 16px;">"Selected files"</p>
                        <ul class=staged_list>
                            {move || staged.get().into_iter().map(|path| {
                                let name = base_name(&path);
                                let path_for_remove = path.clone();
                                view! {
                                    <li class=staged_item>
                                        <span style="flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;" title=path>
                                            {name}
                                        </span>
                                        <Button
                                            variant=ButtonVariant::Text
                                            on_click=Callback::new(move |_| {
                                                let p = path_for_remove.clone();
                                                staged.update(|s| s.retain(|x| x != &p));
                                            })
                                        >
                                            "✕"
                                        </Button>
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                    </Show>

                    {move || message.get().map(|m| view! { <p class=msg>{m}</p> })}
                </ModalBody>
                <ModalFooter>
                    <Button variant=ButtonVariant::Secondary on_click=Callback::new(move |_| on_close.run(()))>
                        "Close"
                    </Button>
                    <Button
                        variant=ButtonVariant::Primary
                        disabled=Signal::derive(move || staged.get().is_empty() || linking.get())
                        on_click=Callback::new(on_link)
                    >
                        {move || if linking.get() { "Linking…" } else { "Link" }}
                    </Button>
                </ModalFooter>
            </ModalBox>
        </ModalOverlay>
    }
}