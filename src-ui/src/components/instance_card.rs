use bamboo_css_macro::{css, styled};
use leptos::prelude::*;
use leptos::{component, IntoView, view};
use leptos_router::hooks::use_navigate;
use yaminabe_launcher_shared::datatypes::InstanceMeta;
use crate::pages::library::mod_tool_color;

#[component]
pub fn InstanceCard(
    instance: InstanceMeta,
    #[prop(optional)] pending: bool
) -> impl IntoView {
    let navigate = use_navigate();

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

    let bg = format!("background-color: {}", mod_tool_color(&instance.mod_tool));
    let name = instance.name.clone();
    let mc_version = format!("MC {}", instance.mc_version);
    let mod_tool = instance.mod_tool.clone();

    view! {
        <div
            class=if pending { card_wrapper_pending } else { card_wrapper }
            on:click=move |_| navigate(&format!("/library/{}", instance.id), Default::default())
        >
            <div class=css! { width: 100%; aspect-ratio: 16 / 9; } style=bg />
            <CardBody>
                <span class=name_style>{name}</span>
                <span class=meta_style>{mc_version}</span>
                <span class=meta_style>{mod_tool}</span>
            </CardBody>
        </div>
    }
}

styled!(CardBody, div, {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
});
