use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use serde::Deserialize;
use yaminabe_launcher_shared::datamodels::ModListEntry;
use crate::components::ui::*;
use phosphor_leptos::{ARROW_SQUARE_OUT, Icon, IconWeight};
use crate::curseforge::{call_link_mods, call_open_project_page, call_pick_mod_files};
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

    let bar = css! {
        display: flex;
        align-items: center;
        gap: 12px;
        width: 100%;
        box-sizing: border-box;
        padding: 8px 8px 8px 14px;
        margin-top: 10px;
        border: 1px solid rgb(212 160 23 / 0.5);
        border-radius: 8px;
        background-color: rgb(212 160 23 / 0.08);
    };
    let dot = css! {
        flex-shrink: 0;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background-color: #d4a017;
    };
    let text = css! {
        flex: 1;
        min-width: 0;
        margin: 0;
        font-size: 0.85rem;
    };
    let count = move || failed.get().len();
    let has_failed = move || !failed.get().is_empty();

    view! {
        <Show when=has_failed>
            <div class=bar>
                <span class=dot></span>
                <p class=text>
                    {move || format!("{} file(s) need manual installation.", count())}
                </p>
                <Button
                    variant=ButtonVariant::Primary
                    size=ButtonSize::Small
                    on_click=Callback::new(move |_| open.set(true))
                >
                    "Link files…"
                </Button>
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

    // Append dropped paths the link flow can use to the staged list while the
    // modal is open — `.jar` mods and the `.zip` resource packs a modpack may
    // also fail to fetch. The subscription detaches when the modal unmounts.
    let add_paths = move |paths: Vec<String>| {
        staged.update(|s| {
            for path in paths {
                let lower = path.to_lowercase();
                let accepted = lower.ends_with(".jar") || lower.ends_with(".zip");
                if accepted && !s.contains(&path) {
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
            if let Ok(paths) = call_pick_mod_files().await {
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
        display: flex;
        align-items: center;
        gap: 8px;
        font-size: 0.82rem;
        padding: 5px 6px 5px 10px;
        border: 1px solid var(--secondary-color);
        border-radius: 6px;
    };
    let needed_name = css! {
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let open_btn = css! {
        flex-shrink: 0;
        display: flex;
        background: none;
        border: none;
        cursor: pointer;
        color: var(--text-color);
        opacity: 0.55;
        padding: 2px;
        line-height: 1;
        transition: opacity 0.12s ease;
        &:hover { opacity: 1; }
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
    let intro = css! {
        margin: 0 0 16px 0;
        font-size: 0.82rem;
        opacity: 0.6;
        line-height: 1.6;
    };

    let base_name = |path: &str| path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();

    view! {
        <ModalOverlay>
            <ModalBox>
                <ModalBody>
                    <h2 style="margin: 0 0 8px 0;">"Link files"</h2>
                    <p class=intro>
                        "These could not be downloaded automatically. The author may have \
                         disabled third-party downloads, or the site publishes no checksum \
                         to verify them by. Supply your own copies to finish setting up \
                         this instance."
                    </p>

                    <p class=label>"Needs linking"</p>
                    <ul class=needed_list>
                        {move || failed.get().into_iter().map(|m| {
                            let name = m.file_name;
                            let title = name.clone();
                            let source = m.source;
                            // A hand-added file has no project page to open.
                            let has_page = source.is_managed();
                            view! {
                                <li class=needed_item title=title>
                                    <span class=needed_name>{name}</span>
                                    {has_page.then(move || view! {
                                        <button
                                            type="button"
                                            class=open_btn
                                            title="Open the download page"
                                            aria-label="Open the download page"
                                            on:click=move |_| {
                                                let source = source.clone();
                                                leptos::task::spawn_local(async move {
                                                    if let Err(e) = call_open_project_page(source).await {
                                                        log::error!("open_project_page failed: {e}");
                                                    }
                                                });
                                            }
                                        >
                                            <Icon icon=ARROW_SQUARE_OUT size="16px" weight=IconWeight::Bold />
                                        </button>
                                    })}
                                </li>
                            }
                        }).collect_view()}
                    </ul>

                    <div class=drop_zone on:click=browse>
                        <p class=drop_hint>"Drop .jar or .zip files here, or click to browse"</p>
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