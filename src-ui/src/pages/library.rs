use crate::components::create_modal::CreateInstanceModal;
use crate::components::instance_card::InstanceCard;
use crate::components::ui::*;
use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use yaminabe_launcher_shared::datatypes::InstanceMeta;

fn matches_active_category(inst: &InstanceMeta, active: &str) -> bool {
    if active == "Default" { inst.category.is_empty() } else { inst.category == active }
}

#[component]
pub fn LibraryPage() -> impl IntoView {
    // ── instance state ─────────────────────────────────────────────────────
    let instances = use_context::<RwSignal<Vec<InstanceMeta>>>().expect("instances context");
    let refresh   = use_context::<RwSignal<u32>>().expect("refresh context");

    // ── modal state ────────────────────────────────────────────────────────
    let show_create_modal = RwSignal::new(false);
    let pending_instances: RwSignal<Vec<InstanceMeta>> = RwSignal::new(vec![]);

    // ── category state ─────────────────────────────────────────────────────
    let extra_categories: RwSignal<Vec<String>> = RwSignal::new(vec![]);
    let active_category: RwSignal<String> = RwSignal::new("Default".to_string());
    let show_add_category_modal = RwSignal::new(false);
    let new_category_name: RwSignal<String> = RwSignal::new(String::new());

    // ── derived tab list ───────────────────────────────────────────────────
    let all_tabs = Signal::derive(move || {
        let mut v = vec!["Default".to_string()];
        for inst in instances.get() {
            if !inst.category.is_empty() && !v.contains(&inst.category) {
                v.push(inst.category.clone());
            }
        }
        for cat in extra_categories.get() {
            if !v.contains(&cat) {
                v.push(cat);
            }
        }
        v
    });

    // ── styles ─────────────────────────────────────────────────────────────
    let grid = css! {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
        gap: 16px;
        margin-top: 16px;
    };
    let dialog_title = css! {
        margin: 0 0 8px 0;
        font-size: 1.1rem;
        font-weight: 600;
    };

    view! {
        <h1>"# Library"</h1>
        <Button
            variant=ButtonVariant::Primary
            size=ButtonSize::Big
            style="margin-bottom: 24px;"
            on_click=Callback::new(move |_| show_create_modal.set(true))
        >
            "+ Create New Instance"
        </Button>

        // Category tab bar
        <TabBar
            tabs=all_tabs
            active=active_category
            allow_add=true
            on_add=Callback::new(move |_| show_add_category_modal.set(true))
        />

        <div class=grid>
            <For
                each=move || {
                    let active = active_category.get();
                    instances.get().into_iter()
                        .filter(|inst| matches_active_category(inst, &active))
                        .collect::<Vec<_>>()
                }
                key=|v| v.id.clone()
                let(inst)
            >
                <InstanceCard instance=inst />
            </For>
            <For
                each=move || {
                    let active = active_category.get();
                    pending_instances.get().into_iter()
                        .filter(|inst| matches_active_category(inst, &active))
                        .collect::<Vec<_>>()
                }
                key=|v| v.id.clone()
                let(inst)
            >
                <InstanceCard instance=inst pending=true />
            </For>
        </div>

        // ── create instance modal ─────────────────────────────────────────
        <CreateInstanceModal
            show=show_create_modal
            on_creating=Callback::new(move |inst: InstanceMeta| pending_instances.update(|v| v.push(inst)))
            on_created=Callback::new(move |name: String| {
                pending_instances.update(|v| v.retain(|i| i.name != name));
                refresh.update(|n| *n += 1);
            })
        />

        // ── add category modal ────────────────────────────────────────────
        <Show when=move || show_add_category_modal.get()>
            <ModalOverlay>
                <DialogBox>
                    <div>
                        <p class=dialog_title>"Add Category"</p>
                    </div>
                    <input
                        class=input_class()
                        type="text"
                        placeholder="Category name"
                        prop:value=move || new_category_name.get()
                        on:input=move |ev| new_category_name.set(event_target_value(&ev))
                    />
                    <DialogFooter>
                        <Button
                            variant=ButtonVariant::Secondary
                            on_click=Callback::new(move |_| {
                                new_category_name.set(String::new());
                                show_add_category_modal.set(false);
                            })
                        >
                            "Cancel"
                        </Button>
                        <Button
                            variant=ButtonVariant::Primary
                            on_click=Callback::new(move |_| {
                                let name = new_category_name.get_untracked().trim().to_string();
                                if !name.is_empty() && name != "Default" {
                                    extra_categories.update(|cats| {
                                        if !cats.iter().any(|c| c == &name) {
                                            let new_cat = name.clone();
                                            cats.push(name);
                                            active_category.set(new_cat);
                                        }
                                    });
                                }
                                new_category_name.set(String::new());
                                show_add_category_modal.set(false);
                            })
                        >
                            "Add"
                        </Button>
                    </DialogFooter>
                </DialogBox>
            </ModalOverlay>
        </Show>
    }
}
