use crate::app::Navigation;
use bamboo_css_macro::{css, styled};
use leptos::control_flow::Show;
use leptos::prelude::{ClassAttribute, ElementChild, Get, Signal};
use leptos::{component, view, IntoView};
use phosphor_leptos::{Icon, IconData, IconWeight};

styled!(NavigationButtonContainer, div, {
    padding: 8px 6px 12px;
    border-radius: 6px;
    width: 96px;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    transition: background-color 0.3s ease;
    &:hover {
        background-color: var(--secondary-color);
    }
});

#[component]
pub fn NavigationButton(
    nav: Navigation,
    icon: IconData,
    #[prop(into)]
    current_nav: Signal<Navigation>,
) -> impl IntoView {
    let navigation_name = nav.to_string();

    view! {
        <NavigationButtonContainer>
            <Show
                when=move || current_nav.get() == nav
                fallback=move || view! { <Icon icon=icon size="32px" weight=IconWeight::Regular /> }
            >
                <Icon icon=icon size="32px" weight=IconWeight::Fill />
            </Show>
            <p class=css! { margin: 0; font-weight: 300; }>
                {navigation_name}
            </p>
        </NavigationButtonContainer>
    }
}
