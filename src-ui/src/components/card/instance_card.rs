use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, view, web_sys, IntoView};
use leptos_router::components::A;
use leptos_router::hooks::use_navigate;
use phosphor_leptos::{Icon, IconWeight, CLOCK, FOLDER_OPEN, GEAR_SIX, PLAY, TRASH};
use serde::Serialize;
use yaminabe_launcher_shared::datamodels::InstanceMeta;
use crate::components::open_in_file_manager::folder_label;
use crate::components::activity_dock::RunningRegistry;
use crate::components::ui::{Button, ButtonVariant, DialogBox, DialogFooter, DialogOverlay};
use crate::ipc;
use crate::signal_ext::VecSignalExt;

#[derive(Serialize)]
struct IdArg { id: String }

#[derive(Serialize)]
struct OpenSubfolderArgs { id: String, subfolder: String }

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
    let registry = use_context::<RunningRegistry>().expect("running registry");
    // Whether this instance is currently launching/running, so the menu's Play
    // item reads "Running" and is disabled rather than offering a second launch.
    let is_running = Signal::derive(move || {
        registry.map_by_id(&instance_id.get_value(), |r| r.status.is_active()).unwrap_or(false)
    });

    // ── context-menu / dialog state ───────────────────────────────────────
    // `menu` is Some(cursor position) while the context menu is open.
    let menu: RwSignal<Option<(i32, i32)>> = RwSignal::new(None);
    let submenu_open = RwSignal::new(false);
    let dialog = RwSignal::new(DeleteDialog::Closed);
    // Names of the subfolders that exist, fetched when the menu opens (the
    // instance root is always offered in addition to these).
    let subfolders: RwSignal<Vec<String>> = RwSignal::new(vec![]);

    // The art fills the card, square as the modpack icons are, with the name
    // on a dark fade over its bottom edge. Hovering, or tabbing to the play
    // button, brings up the play overlay.
    let card_wrapper = css! {
        position: relative;
        display: block;
        aspect-ratio: 1;
        border-radius: 12px;
        overflow: hidden;
        color: white;
        text-decoration: none;
        cursor: pointer;
        transition: transform 0.15s ease, box-shadow 0.15s ease;
        &:hover {
            transform: translateY(-3px);
            box-shadow: 0 8px 24px rgb(0 0 0 / 0.25);
        }
        &:hover [data-play-overlay], &:focus-within [data-play-overlay] { opacity: 1; }
    };
    let card_wrapper_pending = css! {
        position: relative;
        display: block;
        aspect-ratio: 1;
        border-radius: 12px;
        overflow: hidden;
        color: white;
        text-decoration: none;
        opacity: 0.6;
    };
    // Holds the activity chip too, rather than the art's top corners, where a
    // pack's art tends to put its title. It lies over the play overlay, so it
    // lets clicks through to it rather than to the card's link.
    let caption = css! {
        position: absolute;
        left: 0;
        right: 0;
        bottom: 0;
        display: flex;
        flex-direction: column;
        gap: 2px;
        padding: 56px 14px 12px;
        background: linear-gradient(transparent, rgb(0 0 0 / 0.8));
        pointer-events: none;
    };
    let name_style = css! {
        font-weight: 700;
        font-size: 1rem;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let meta_style = css! {
        font-size: 0.8rem;
        opacity: 0.8;
    };
    let activity_chip = css! {
        align-self: flex-start;
        display: flex;
        align-items: center;
        gap: 6px;
        margin-bottom: 6px;
        padding: 4px 10px;
        border-radius: 999px;
        font-size: 0.72rem;
        font-weight: 600;
        background-color: rgb(0 0 0 / 0.55);
        backdrop-filter: blur(6px);
    };
    let running_dot = css! {
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background-color: #3a9e5f;
        animation: pulse 1.2s ease-in-out infinite;
    };
    let play_overlay = css! {
        position: absolute;
        inset: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        background-color: rgb(0 0 0 / 0.35);
        opacity: 0;
        transition: opacity 0.15s ease;
    };
    // Nudged right, as a play triangle's visual middle sits left of its box's.
    let play_button = css! {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 60px;
        height: 60px;
        padding: 0 0 0 4px;
        border: none;
        border-radius: 50%;
        background-color: #3a9e5f;
        color: white;
        cursor: pointer;
        box-shadow: 0 6px 20px rgb(0 0 0 / 0.35);
        transition: background-color 0.15s ease;
        &:hover { background-color: #2e7d4f; }
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
        &:disabled { opacity: 0.45; cursor: not-allowed; }
        &:disabled:hover { background-color: transparent; }
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

    let name = instance.name.clone();
    let details = format!("MC {} · {}", instance.game_version, instance.mod_loader);
    // A pending instance has no folder yet to keep an icon in.
    let icon = LocalResource::new(move || {
        let id = instance_id.get_value();
        async move {
            if pending {
                return None;
            }
            ipc::call::<_, Option<String>>("get_instance_icon", IdArg { id })
                .await
                .ok()
                .flatten()
        }
    });
    let icon = Signal::derive(move || icon.get().flatten());

    let last_played = instance.last_played;
    let activity = Memo::new(move |_| {
        if pending {
            Activity::Installing
        } else if is_running.get() {
            Activity::Running
        } else {
            Activity::Idle { last_played }
        }
    });
    let activity_label = move || match activity.get() {
        Activity::Installing => view! { "Installing…" }.into_any(),
        Activity::Running => view! { <span class=running_dot></span>"Running" }.into_any(),
        Activity::Idle { last_played } => view! {
            <Icon icon=CLOCK size="12px" weight=IconWeight::Bold />
            {last_played_text(last_played)}
        }
        .into_any(),
    };
    let play = move || {
        navigate.with_value(|nav| nav(
            &format!("/library/{}/play?mode=online", instance_id.get_value()),
            Default::default(),
        ));
    };

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
        <A
            href=format!("/library/{}", instance_id.get_value())
            attr:class=if pending { card_wrapper_pending } else { card_wrapper }
            on:contextmenu=move |ev: web_sys::MouseEvent| {
                ev.prevent_default();
                if pending { return; }
                menu.set(Some((ev.client_x(), ev.client_y())));
                submenu_open.set(false);
                let id = instance_id.get_value();
                leptos::task::spawn_local(async move {
                    let res = ipc::call::<_, Vec<String>>("get_instance_subfolders", IdArg { id })
                        .await
                        .unwrap_or_default();
                    subfolders.set(res);
                });
            }
        >
            <Art instance=instance icon=icon />
            // Only an idle instance offers to launch: a pending one is not on
            // disk yet, and a running one must not start a second copy.
            <Show when=move || matches!(activity.get(), Activity::Idle { .. })>
                <div class=play_overlay data-play-overlay="">
                    <button
                        type="button"
                        class=play_button
                        aria-label="Play"
                        on:click=move |ev: web_sys::MouseEvent| {
                            // It sits inside the card's link: launch rather than
                            // open the instance.
                            ev.prevent_default();
                            ev.stop_propagation();
                            play();
                        }
                    >
                        <Icon icon=PLAY size="28px" weight=IconWeight::Fill />
                    </button>
                </div>
            </Show>
            <div class=caption>
                <span class=activity_chip>{activity_label}</span>
                <span class=name_style>{name}</span>
                <span class=meta_style>{details}</span>
            </div>
        </A>

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
                    prop:disabled=move || is_running.get()
                    on:click=move |_| {
                        menu.set(None);
                        play();
                    }
                >
                    <Icon icon=PLAY size="16px" weight=IconWeight::Fill />
                    <span>{move || if is_running.get() { "Running" } else { "Play" }}</span>
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
                        // Instance root (always) followed by whatever subfolders exist.
                        {move || {
                            std::iter::once(String::new())
                                .chain(subfolders.get())
                                .map(|subfolder| {
                                    let label = folder_label(&subfolder);
                                    view! {
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
                                            <span>{label}</span>
                                        </button>
                                    }
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

/// What the card's chip says the instance is doing.
#[derive(Clone, PartialEq)]
enum Activity {
    /// Still being created, so not yet on disk.
    Installing,
    Running,
    Idle { last_played: Option<i64> },
}

/// How long ago the instance was last played, for its activity chip.
fn last_played_text(last_played: Option<i64>) -> String {
    let Some(at) = last_played else {
        return "Never played".to_string();
    };
    let minutes = ((js_sys::Date::now() as i64 - at) / 60_000).max(0);
    match minutes {
        0 => "Just now".to_string(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} h ago", minutes / 60),
        1440..=2879 => "Yesterday".to_string(),
        _ => format!("{} days ago", minutes / 1440),
    }
}

/// Up to two initials from the name, for art without an icon.
fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

/// The instance's icon filling the card, or without one, its initials on the
/// loader's color.
#[component]
fn Art(instance: InstanceMeta, icon: Signal<Option<String>>) -> impl IntoView {
    let art = css! {
        width: 100%;
        height: 100%;
        container-type: size;
        display: flex;
        align-items: center;
        justify-content: center;
    };
    let image = css! {
        display: block;
        width: 100%;
        height: 100%;
        object-fit: cover;
    };
    // Sized to the art, so the letters keep their proportion at any card size.
    let monogram = css! {
        font-size: 40cqmin;
        font-weight: 700;
        letter-spacing: -1px;
        color: rgb(255 255 255 / 0.85);
        user-select: none;
    };
    let color = instance.mod_loader.mod_loader_color();
    let background = format!("linear-gradient(135deg, {color}, color-mix(in srgb, {color} 45%, black))");
    let letters = initials(&instance.name);

    view! {
        <div class=art style:background=background>
            {move || match icon.get() {
                Some(src) => view! { <img class=image src=src alt="" /> }.into_any(),
                None => view! { <span class=monogram>{letters.clone()}</span> }.into_any(),
            }}
        </div>
    }
}