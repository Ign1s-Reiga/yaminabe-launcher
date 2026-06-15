use crate::components::ui::{ButtonSize, Dropdown, DropdownItem};
use crate::ipc;
use leptos::prelude::*;
use leptos::{IntoView, component, view};
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

    view! {
        <Dropdown label=Signal::derive(|| "Open...".to_string()) size=ButtonSize::Big>
            // Instance root (always) followed by whatever subfolders exist.
            {move || {
                std::iter::once(String::new())
                    .chain(existing.get().unwrap_or_default())
                    .map(|subfolder| {
                        let label = folder_label(&subfolder);
                        view! {
                            <DropdownItem on_select=Callback::new(move |_| {
                                let id = instance_id.get_value();
                                let subfolder = subfolder.clone();
                                leptos::task::spawn_local(async move {
                                    if let Err(e) = ipc::call::<_, ()>("open_instance_subfolder",
                                        OpenSubfolderArgs { id, subfolder }).await {
                                        log::error!("open_instance_subfolder failed: {e}");
                                    }
                                });
                            })>
                                {label}
                            </DropdownItem>
                        }
                    })
                    .collect_view()
            }}
        </Dropdown>
    }
}