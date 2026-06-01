use bamboo_css_macro::{css, styled};
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, view, web_sys, IntoView};
use leptos_router::hooks::use_navigate;
use phosphor_leptos::{Icon, IconWeight, FOLDER_OPEN, GEAR_SIX, PLAY, TRASH};
use serde::Serialize;
use yaminabe_launcher_shared::datatypes::InstanceMeta;
use crate::components::ui::{Button, ButtonVariant, DialogBox, DialogFooter, DialogOverlay};
use crate::ipc;

#[derive(Serialize)]
struct IdArg { id: String }

#[derive(Serialize)]
struct OpenSubfolderArgs { id: String, subfolder: String }

/// Folders offered by the context-menu "Open Folder" submenu. The empty
/// subfolder is the instance root and is always shown; the rest map to the
/// existence flags returned by `get_instance_subfolders` (config, mods,
/// resourcepacks, saves — in that order). Add an entry here to extend it.
const FOLDERS: &[(&str, &str)] = &[
    ("", "Instance folder"),
    ("config", "Config folder"),
    ("mods", "Mods folder"),
    ("resourcepacks", "Resourcepacks folder"),
    ("saves", "Saves folder"),
];

/// State of the delete-confirmation dialog. One value rather than three
/// booleans/options, so impossible combinations (e.g. deleting while showing an
/// error) can't be represented.
#[derive(Clone, PartialEq)]
enum DeleteDialog {
    Closed,
    Confirming,
    Deleting,
    Failed(String),
}

#[component]
pub fn InstanceCard(
    instance: InstanceMeta,
    #[prop(optional)] pending: bool
) -> impl IntoView {
    // StoredValue keeps these Copy so they can be captured by the `Fn` closures
    // the context-menu's `<Show>` children require.
    let navigate = StoredValue::new(use_navigate());
    let instance_id = StoredValue::new(instance.id.clone());
    let instance_name = StoredValue::new(instance.name.clone());
    let refresh = use_context::<RwSignal<u32>>().expect("refresh context");

    // ── context-menu / dialog state ───────────────────────────────────────
    // `menu` is Some(cursor position) while the context menu is open.
    let menu: RwSignal<Option<(i32, i32)>> = RwSignal::new(None);
    let submenu_open = RwSignal::new(false);
    let dialog = RwSignal::new(DeleteDialog::Closed);
    // Existence of [config, mods, resourcepacks, saves], fetched when the menu
    // opens so we only list folders that actually exist on disk.
    let subfolders: RwSignal<Vec<bool>> = RwSignal::new(vec![]);

    let card_wrapper = css! {
        background-color: var(--secondary-color);
        border-radius: 12px;
        overflow: hidden;
        cursor: pointer;
        transition: transform 0.15s ease, box-shadow 0.15s ease;
        &:hover {
            transform: translateY(-3px);
            box-shadow: 0 8px 24px rgb(0 0 0 / 0.2);
        }
    };
    let card_wrapper_pending = css! {
        background-color: var(--secondary-color);
        border-radius: 12px;
        overflow: hidden;
        opacity: 0.6;
    };
    let name_style = css! {
        font-weight: 600;
        font-size: 0.95rem;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let meta_style = css! {
        font-size: 0.8rem;
        opacity: 0.6;
    };

    let backdrop = css! {
        position: fixed;
        inset: 0;
        background-color: transparent;
        z-index: 150;
    };
    let menu_class = css! {
        display: flex;
        flex-direction: column;
        min-width: 190px;
        background-color: var(--background-color);
        border: 1px solid var(--secondary-color);
        border-radius: 8px;
        padding: 4px;
        box-shadow: 0 8px 24px rgb(0 0 0 / 0.25);
        z-index: 151;
    };
    let item = css! {
        display: flex;
        align-items: center;
        gap: 10px;
        width: 100%;
        background-color: transparent;
        color: var(--text-color);
        border: none;
        border-radius: 6px;
        padding: 8px 12px;
        text-align: left;
        font-size: 0.875rem;
        font-family: inherit;
        cursor: pointer;
        box-sizing: border-box;
        transition: background-color 0.12s ease;
        &:hover { background-color: var(--secondary-color); }
    };
    let item_danger = css! {
        display: flex;
        align-items: center;
        gap: 10px;
        width: 100%;
        background-color: transparent;
        color: #c0392b;
        border: none;
        border-radius: 6px;
        padding: 8px 12px;
        text-align: left;
        font-size: 0.875rem;
        font-family: inherit;
        cursor: pointer;
        box-sizing: border-box;
        transition: background-color 0.12s ease;
        &:hover { background-color: rgba(192, 57, 43, 0.12); }
    };
    let submenu = css! {
        display: flex;
        flex-direction: column;
        margin-left: 12px;
        padding-left: 4px;
        border-left: 1px solid var(--secondary-color);
    };
    let divider = css! {
        height: 1px;
        background-color: var(--secondary-color);
        margin: 4px 0;
    };
    let dialog_title = css! {
        margin: 0 0 8px 0;
        font-size: 1.1rem;
        font-weight: 600;
    };
    let dialog_text = css! {
        margin: 0;
        font-size: 0.9rem;
        opacity: 0.7;
        line-height: 1.5;
    };

    let bg = format!("background-color: {}", &instance.mod_loader.get_modloader_color());
    let name = instance.name.clone();
    let mc_version = format!("MC {}", instance.game_version);
    let mod_loader = instance.mod_loader.clone();

    let on_confirm_delete = Callback::new(move |_: web_sys::MouseEvent| {
        let id = instance_id.get_value();
        dialog.set(DeleteDialog::Deleting);
        leptos::task::spawn_local(async move {
            // Close on success; on failure (e.g. the instance is running) move to
            // Failed so the dialog stays open and shows the reason.
            match ipc::call::<_, ()>("delete_instance", IdArg { id }).await {
                Ok(()) => {
                    refresh.update(|n| *n += 1);
                    dialog.set(DeleteDialog::Closed);
                }
                Err(e) => dialog.set(DeleteDialog::Failed(e)),
            }
        });
    });

    view! {
        <div
            class=if pending { card_wrapper_pending } else { card_wrapper }
            on:click=move |_| navigate.with_value(|nav| {
                nav(&format!("/library/{}", instance_id.get_value()), Default::default())
            })
            on:contextmenu=move |ev: web_sys::MouseEvent| {
                ev.prevent_default();
                if pending { return; }
                menu.set(Some((ev.client_x(), ev.client_y())));
                submenu_open.set(false);
                let id = instance_id.get_value();
                leptos::task::spawn_local(async move {
                    let res = ipc::call::<_, Vec<bool>>("get_instance_subfolders", IdArg { id })
                        .await
                        .unwrap_or_default();
                    subfolders.set(res);
                });
            }
        >
            <div class=css! { width: 100%; aspect-ratio: 16 / 9; } style=bg />
            <CardBody>
                <span class=name_style>{name}</span>
                <span class=meta_style>{mc_version}</span>
                <span class=meta_style>{mod_loader.to_string()}</span>
            </CardBody>
        </div>

        <Show when=move || menu.get().is_some()>
            <div
                class=backdrop
                on:mousedown=move |_| menu.set(None)
                on:contextmenu=move |ev: web_sys::MouseEvent| { ev.prevent_default(); menu.set(None); }
            ></div>
            <div
                class=menu_class
                style=move || {
                    let (x, y) = menu.get().unwrap_or_default();
                    format!("position: fixed; top: {y}px; left: {x}px;")
                }
            >
                <button
                    class=item
                    on:click=move |_| {
                        menu.set(None);
                        navigate.with_value(|nav| nav(
                            &format!("/library/{}/play?mode=online", instance_id.get_value()),
                            Default::default(),
                        ));
                    }
                >
                    <Icon icon=PLAY size="16px" weight=IconWeight::Fill />
                    <span>"Play"</span>
                </button>
                <button
                    class=item
                    on:click=move |_| {
                        menu.set(None);
                        navigate.with_value(|nav| nav(
                            &format!("/library/{}?tab=Settings", instance_id.get_value()),
                            Default::default(),
                        ));
                    }
                >
                    <Icon icon=GEAR_SIX size="16px" weight=IconWeight::Regular />
                    <span>"Settings"</span>
                </button>
                <button class=item on:click=move |_| submenu_open.update(|v| *v = !*v)>
                    <Icon icon=FOLDER_OPEN size="16px" weight=IconWeight::Regular />
                    <span style="flex: 1;">"Open Folder"</span>
                    <span style="opacity: 0.6;">{move || if submenu_open.get() { "▾" } else { "▸" }}</span>
                </button>
                <Show when=move || submenu_open.get()>
                    <div class=submenu>
                        {move || {
                            let exist = subfolders.get();
                            FOLDERS.iter().enumerate().filter_map(move |(i, (sub, label))| {
                                let shown = sub.is_empty()
                                    || exist.get(i - 1).copied().unwrap_or(false);
                                if !shown { return None; }
                                let subfolder = sub.to_string();
                                Some(view! {
                                    <button
                                        class=item
                                        on:click=move |_| {
                                            menu.set(None);
                                            let id = instance_id.get_value();
                                            let subfolder = subfolder.clone();
                                            leptos::task::spawn_local(async move {
                                                if let Err(e) = ipc::call::<_, ()>(
                                                    "open_instance_subfolder",
                                                    OpenSubfolderArgs { id, subfolder },
                                                ).await {
                                                    log::error!("open_instance_subfolder failed: {e}");
                                                }
                                            });
                                        }
                                    >
                                        <span>{*label}</span>
                                    </button>
                                })
                            }).collect_view()
                        }}
                    </div>
                </Show>
                <div class=divider></div>
                <button
                    class=item_danger
                    on:click=move |_| { menu.set(None); dialog.set(DeleteDialog::Confirming); }
                >
                    <Icon icon=TRASH size="16px" weight=IconWeight::Regular />
                    <span>"Delete"</span>
                </button>
            </div>
        </Show>

        <Show when=move || dialog.get() != DeleteDialog::Closed>
            <DialogOverlay>
                <DialogBox>
                    <div>
                        <p class=dialog_title>"Delete instance"</p>
                        <p class=dialog_text>
                            {move || format!(
                                "Delete \"{}\"? This permanently removes the instance folder \
                                 and cannot be undone.",
                                instance_name.get_value()
                            )}
                        </p>
                        {move || match dialog.get() {
                            DeleteDialog::Failed(e) => Some(view! {
                                <p style="margin: 12px 0 0 0; color: #c0392b; font-size: 0.85rem;">{e}</p>
                            }),
                            _ => None,
                        }}
                    </div>
                    <DialogFooter>
                        <Button
                            variant=ButtonVariant::Secondary
                            disabled=Signal::derive(move || dialog.get() == DeleteDialog::Deleting)
                            on_click=Callback::new(move |_| dialog.set(DeleteDialog::Closed))
                        >
                            "Cancel"
                        </Button>
                        <Button
                            variant=ButtonVariant::Danger
                            disabled=Signal::derive(move || dialog.get() == DeleteDialog::Deleting)
                            on_click=on_confirm_delete
                        >
                            "Delete"
                        </Button>
                    </DialogFooter>
                </DialogBox>
            </DialogOverlay>
        </Show>
    }
}

styled!(CardBody, div, {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
});