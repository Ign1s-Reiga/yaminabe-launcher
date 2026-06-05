use crate::components::ui::*;
use bamboo_css_macro::{css, cx};
use leptos::prelude::*;
use leptos::{component, view, IntoView};

#[component]
pub fn StepMethod(
    selected_method: RwSignal<Option<u8>>,
    on_next: Callback<()>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    let option_list = css! {
        display: flex;
        flex-direction: column;
        gap: 8px;
        margin-top: 8px;
    };
    let option_card = css! {
        display: flex;
        align-items: center;
        gap: 16px;
        padding: 14px 16px;
        border-radius: 8px;
        border: 1.5px solid var(--secondary-color);
        cursor: pointer;
        user-select: none;
        transition: border-color 0.12s ease, background-color 0.12s ease;
        &:hover {
            border-color: rgba(58, 158, 95, 0.45);
            background-color: rgba(58, 158, 95, 0.04);
        }
    };
    let option_selected = css! {
        border-color: #3a9e5f;
        background-color: rgba(58, 158, 95, 0.1);
    };
    let option_icon  = css! {
        font-size: 1.2rem;
        width: 24px;
        text-align: center;
        flex-shrink: 0;
        opacity: 0.8;
    };
    let option_info  = css! { display: flex; flex-direction: column; gap: 3px; };
    let option_title = css! { font-weight: 600; font-size: 0.9rem; };
    let option_desc  = css! { font-size: 0.8rem; opacity: 0.55; };

    view! {
        <ModalBody>
            <h2 style="margin: 0 0 16px 0;">"New Instance"</h2>
            <div class=option_list>
                <div
                    class=move || cx!(option_card, if selected_method.get() == Some(0) { option_selected } else { "" })
                    on:click=move |_| selected_method.set(Some(0))
                >
                    <span class=option_icon>"📁"</span>
                    <div class=option_info>
                        <span class=option_title>"Import from Local"</span>
                        <span class=option_desc>"Import a modpack from a local file."</span>
                    </div>
                </div>
                <div
                    class=move || cx!(option_card, if selected_method.get() == Some(1) { option_selected } else { "" })
                    on:click=move |_| selected_method.set(Some(1))
                >
                    <span class=option_icon>"✏️"</span>
                    <div class=option_info>
                        <span class=option_title>"Create Manually"</span>
                        <span class=option_desc>"Set up an instance from scratch."</span>
                    </div>
                </div>
            </div>
        </ModalBody>
        <ModalFooter>
            <Button
                variant=ButtonVariant::Secondary
                on_click=Callback::new(move |_| on_cancel.run(()))
            >
                "Cancel"
            </Button>
            <Button
                variant=ButtonVariant::Primary
                disabled=Signal::derive(move || selected_method.get().is_none())
                on_click=Callback::new(move |_| on_next.run(()))
            >
                "Next →"
            </Button>
        </ModalFooter>
    }
}