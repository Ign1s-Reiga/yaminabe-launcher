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

/// Human label for a subfolder name: `""` (the instance root) → "Instance
/// folder", `"config"` → "Config folder", and so on. The folder set itself is
/// owned by the backend's `get_instance_subfolders`; this only renders names.
pub fn folder_label(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => format!("{}{} folder", first.to_uppercase(), chars.as_str()),
        None => "Instance folder".to_string(),
    }
}

#[component]
pub fn OpenInFileManager(instance_id: String) -> impl IntoView {
    let (open_dropdown, set_open_dropdown) = signal(false);

    // The id never changes; StoredValue keeps it Copy for the closures below
    // without the overhead (and reactivity) of an RwSignal.
    let instance_id = StoredValue::new(instance_id);
    let existing = LocalResource::new(move || {
        let id = instance_id.get_value();
        async move {
            ipc::call::<_, Vec<String>>("get_instance_subfolders", GetSubfoldersArgs { id })
                .await
                .unwrap_or_default()
        }
    });

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
                    // Instance root (always) followed by whatever subfolders exist.
                    {move || {
                        std::iter::once(String::new())
                            .chain(existing.get().unwrap_or_default())
                            .map(|subfolder| {
                                let label = folder_label(&subfolder);
                                view! {
                                    <button
                                        class=dropdown_item
                                        on:click=move |_| {
                                            set_open_dropdown.set(false);
                                            let id = instance_id.get_value();
                                            let subfolder = subfolder.clone();
                                            leptos::task::spawn_local(async move {
                                                if let Err(e) = ipc::call::<_, ()>("open_instance_subfolder",
                                                    OpenSubfolderArgs { id, subfolder }).await {
                                                    log::error!("open_instance_subfolder failed: {e}");
                                                }
                                            });
                                        }
                                    >
                                        {label}
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