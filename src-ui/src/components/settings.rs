use bamboo_css_macro::css;
use leptos::{component, view};
use leptos::control_flow::Show;
use leptos::prelude::*;
use crate::components::ui::{Button, ButtonVariant};

#[derive(Clone, PartialEq)]
pub enum SaveState {
    Idle,
    Saving,
    Ok,
    Err(String),
}

#[component]
pub fn SettingsSection(
    id: &'static str,
    heading: &'static str,
    #[prop(optional)] save_state: Option<RwSignal<SaveState>>,
    children: Children,
) -> impl IntoView {
    let section = css! {
        margin-bottom: 56px;
    };
    let section_heading = css! {
        font-size: 1.05rem;
        font-weight: 700;
        margin: 0 0 20px 0;
        padding-bottom: 12px;
        border-bottom: 1px solid var(--secondary-color);
    };
    let section_footer = css! {
        display: flex;
        align-items: center;
        justify-content: flex-end;
        gap: 12px;
        margin-top: 20px;
        padding-top: 14px;
        border-top: 1px solid var(--secondary-color);
    };
    let save_feedback = css! {
        font-size: 0.82rem;
        opacity: 0.7;
    };

    view! {
        <section class=section id=id>
            <h2 class=section_heading>{heading}</h2>
            {children()}
            {save_state.map(|ss| view! {
                <div class=section_footer>
                    <span class=save_feedback>
                        {move || match ss.get() {
                            SaveState::Ok     => "Saved.",
                            SaveState::Saving => "Saving…",
                            SaveState::Err(_) => "Save failed.",
                            SaveState::Idle   => "",
                        }}
                    </span>
                    <Show when=move || matches!(ss.get(), SaveState::Err(_)) fallback=|| ()>
                        <span style="font-size: 0.78rem; opacity: 0.55; max-width: 260px; text-align: right;">
                            {move || if let SaveState::Err(e) = ss.get() { e } else { String::new() }}
                        </span>
                    </Show>
                    <Button
                        variant=ButtonVariant::Primary
                        button_type="submit"
                        disabled=Signal::derive(move || ss.get() == SaveState::Saving)
                    >
                        "Save"
                    </Button>
                </div>
            })}
        </section>
    }
}

#[component]
pub fn SettingsProp(
    label: &'static str,
    hint: &'static str,
    children: Children,
) -> impl IntoView {
    let prop_row = css! {
        display: grid;
        grid-template-columns: 200px 1fr;
        align-items: start;
        gap: 10px 24px;
        margin-bottom: 18px;
    };
    let prop_label = css! {
        font-size: 0.84rem;
        font-weight: 600;
        opacity: 0.7;
        padding-top: 11px;
    };
    let prop_hint = css! {
        font-size: 0.76rem;
        opacity: 0.45;
        margin-top: 5px;
        user-select: none;
    };

    view! {
        <div class=prop_row>
            <span class=prop_label>{label}</span>
            <div>
                {children()}
                <p class=prop_hint>{hint}</p>
            </div>
        </div>
    }
}
