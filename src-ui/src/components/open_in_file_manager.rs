use crate::components::ui::{Button, ButtonSize, ButtonVariant};
use crate::ipc;
use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use serde::Serialize;

#[derive(Serialize)]
struct OpenSubfolderArgs { id: String, subfolder: String }

#[derive(Serialize)]
struct GetSubfoldersArgs { id: String }

#[component]
pub fn OpenInFileManager(instance_id: String) -> impl IntoView {
    let (open_dropdown, set_open_dropdown) = signal(false);

    let id_for_resource = instance_id.clone();
    let existing = LocalResource::new(move || {
        let id = id_for_resource.clone();
        async move {
            ipc::call::<_, Vec<bool>>("get_instance_subfolders", GetSubfoldersArgs { id })
                .await
                .unwrap_or_default()
        }
    });

    let id_root: RwSignal<String> = RwSignal::new(instance_id.clone());
    let id_subs: RwSignal<String> = RwSignal::new(instance_id);

    let dropdown_wrap = css! {
        position: relative;
        display: inline-block;
        z-index: 50;
    };
    let dropdown_list = css! {
        position: absolute;
        top: calc(100% + 4px);
        left: 0;
        background-color: var(--background-color);
        border: 1px solid var(--secondary-color);
        border-radius: 8px;
        padding: 4px;
        min-width: 190px;
        box-shadow: 0 8px 24px rgb(0 0 0 / 0.2);
    };
    let dropdown_item = css! {
        display: block;
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

    view! {
        <div class=dropdown_wrap>
            <Button
                variant=ButtonVariant::Secondary
                size=ButtonSize::Big
                on_click=Callback::new(move |_| set_open_dropdown.update(|v| *v = !*v))
            >
                "Open...  ▾"
            </Button>
            <Show when=move || open_dropdown.get()>
                <div class=dropdown_list>
                    <button
                        class=dropdown_item
                        on:click=move |_| {
                            set_open_dropdown.set(false);
                            let id = id_root.get_untracked();
                            leptos::task::spawn_local(async move {
                                let _ = ipc::call::<_, ()>("open_instance_subfolder",
                                    OpenSubfolderArgs { id, subfolder: String::new() }).await;
                            });
                        }
                    >
                        "Instance folder"
                    </button>
                    {move || {
                        let existing = existing.get().unwrap_or_default();
                        let id_str = id_subs.get_untracked();
                        [("config", "Config folder"), ("mods", "Mods folder"),
                         ("resourcepacks", "Resourcepacks folder"), ("saves", "Saves folder")]
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| existing.get(*i).copied().unwrap_or(false))
                            .map(|(_, (sub, label))| {
                                let id = id_str.clone();
                                let subfolder = sub.to_string();
                                view! {
                                    <button
                                        class=dropdown_item
                                        on:click=move |_| {
                                            set_open_dropdown.set(false);
                                            let id = id.clone();
                                            let sf = subfolder.clone();
                                            leptos::task::spawn_local(async move {
                                                let _ = ipc::call::<_, ()>("open_instance_subfolder",
                                                    OpenSubfolderArgs { id, subfolder: sf }).await;
                                            });
                                        }
                                    >
                                        {*label}
                                    </button>
                                }
                            })
                            .collect_view()
                    }}
                </div>
            </Show>
        </div>
    }
}