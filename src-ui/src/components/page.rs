use leptos::prelude::AddAnyAttr;
use bamboo_css_macro::{css, styled};
use leptos::prelude::{ClassAttribute, ElementChild};
use leptos::{component, IntoView, view};

styled!(Heading, h2, {
    color: green;
});
styled!(FancyInput, input, {
    background-color: purple;
});

#[component]
pub fn HomePage() -> impl IntoView {
    let styles = css! {
        color: red;
    };

    view! {
        <div>
            <h1>Hello World</h1>
            <p class=styles>This is styled with bamboo-css.</p>
            <Heading>This is styled with styled.</Heading>
            <FancyInput attr:value="This is FancyInput" attr:placeholder="aaa" />
            <input />
        </div>
    }
}

#[component]
pub fn LibraryPage() -> impl IntoView {
    let container = css! {
        display: flex;
        flex-direction: column;
    };

    view! {
        <div class=container></div>
    }
}

#[component]
pub fn SettingsPage() -> impl IntoView {
    let settings_container = css! {
        display: grid;
        grid: auto-flow / 1fr 200px;
    };
    let settings_category = css! {
        height: 800px;
        & > p {
            margin: 0.8rem 0;
        }
    };
    let settings_nav = css! {
        position: fixed;
        right: 100px;
        width: 150px;
        display: flex;
        flex-direction: column;
        gap: 4px;
        justify-content: center;
        list-style: none;
        margin: 0;
    };
    let settings_nav_anchor = css! {
        display: block;
        text-decoration: none;
        color: var(--text-color);
        border-radius: 4px;
        transition: background-color 0.2s ease;
        width: 100%;
        line-height: 2rem;
        padding-left: 8px;
        &:hover {
            background-color: var(--primary-color-hover);
        }
    };

    view! {
        <main class=settings_container>
            <div class="settings-content">
                <section class=settings_category>
                    <h2 id="general">General</h2>
                    <p>Configure your application settings here.</p>
                    <input list="test-input-range" type="range" id="dark-mode" min="0" max="100" style="width: 600px; accent-color: red;" />
                    <datalist id="test-input-range">
                        <option value="0" label="0%" />
                        <option value="20" label="20%" />
                        <option value="40" label="40%" />
                        <option value="60" label="60%" />
                        <option value="80" label="80%" />
                        <option value="100" label="100%" />
                    </datalist>
                </section>
                <div class=settings_category>
                    <h2 id="instance">Instance Defaults</h2>
                    <p>Configure your application settings here.</p>
                </div>
                <div class=settings_category>
                    <h2 id="network">Settings</h2>
                    <p>Configure your application settings here.</p>
                </div>
                <div class=settings_category>
                    <h2 id="about">About</h2>
                    <p>Configure your application settings here.</p>
                    <p>Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.</p>
                </div>
            </div>
            <nav>
                <ul class=settings_nav>
                    <li><a href="#general" class=settings_nav_anchor>General</a></li>
                    <li><a href="#instance" class=settings_nav_anchor>Instance Defaults</a></li>
                    <li><a href="#network" class=settings_nav_anchor>Network</a></li>
                    <li><a href="#about" class=settings_nav_anchor>About</a></li>
                </ul>
            </nav>
        </main>
    }
}
