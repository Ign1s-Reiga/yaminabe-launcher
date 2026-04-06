use std::fmt::{Display, Formatter};
use bamboo_css_macro::{css, styled};
use leptos::{component, IntoView};
use leptos::control_flow::Show;
use leptos::prelude::*;
use phosphor_leptos::{BOOKS, GEAR_SIX, HOUSE, MAGNIFYING_GLASS};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use crate::components::navigation::NavigationButton;
use crate::components::page::{HomePage, SettingsPage};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
}

#[derive(Serialize, Deserialize)]
struct GreetArgs<'a> {
    name: &'a str,
}


styled!(MainViewWrapper, div, {
    height: 100vh;
    width: 100vw;
    display: grid;
    grid: 1fr 100px / auto-flow;
});
styled!(MainView, main, {
    padding: 64px 128px;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-gutter: stable both-edges;
    scrollbar-color: darkgrey var(--background-color);
});
styled!(MainViewNavbar, nav, {
    padding: 10px 128px;
    background-color: var(--primary-color);
    gap: 16px;
    display: flex;
    flex-direction: row;
    justify-content: center;
});

#[component]
pub fn App() -> impl IntoView {
    let (current_nav, set_current_nav) = signal(Navigation::Home);

    view! {
        <MainViewWrapper>
            <MainView>
                <Show when=move || current_nav.get() == Navigation::Home>
                    <h1>"# Home"</h1>
                    <HomePage />
                </Show>
                <Show when=move || current_nav.get() == Navigation::Library>
                    <h1>"# Library"</h1>
                    <h2>Library Page</h2>
                </Show>
                <Show when=move || current_nav.get() == Navigation::Search>
                    <h1>"# Search"</h1>
                    <h2>Search Page</h2>
                </Show>
                <Show when=move || current_nav.get() == Navigation::Settings>
                    <h1>"# Settings"</h1>
                    <SettingsPage />
                </Show>
            </MainView>
            // TODO: Add PLAY button in center of the navbar. Instantly launch recent played profile.
            <MainViewNavbar>
                <NavigationButton
                    nav=Navigation::Library
                    icon=BOOKS
                    current_nav=current_nav
                    on:click=move |_| set_current_nav.set(Navigation::Library)
                />
                <NavigationButton
                    nav=Navigation::Home
                    icon=HOUSE
                    current_nav=current_nav
                    on:click=move |_| set_current_nav.set(Navigation::Home)
                />
                <NavigationButton
                    nav=Navigation::Search
                    icon=MAGNIFYING_GLASS
                    current_nav=current_nav
                    on:click=move |_| set_current_nav.set(Navigation::Search)
                />
                <NavigationButton
                    nav=Navigation::Settings
                    icon=GEAR_SIX
                    current_nav=current_nav
                    on:click=move |_| set_current_nav.set(Navigation::Settings)
                />
            </MainViewNavbar>
        </MainViewWrapper>
    }
}

#[derive(Clone, PartialEq)]
pub enum Navigation {
    Home,
    Library,
    Search,
    Settings,
}

impl Display for Navigation {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Navigation::Home => write!(f, "Home"),
            Navigation::Library => write!(f, "Library"),
            Navigation::Search => write!(f, "Search"),
            Navigation::Settings => write!(f, "Settings"),
        }
    }
}
