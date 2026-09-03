pub mod components;
pub mod pages;
pub mod app;
pub mod ipc;
pub mod changelog;
pub mod curseforge;
pub mod signal_ext;
pub mod trivia;

use leptos::prelude::*;
use crate::app::App;

fn main() {
    console_error_panic_hook::set_once();
    suppress_native_context_menu();
    mount_to_body(|| {
        view! {
            <App/>
        }
    })
}

/// Suppress the WebView's native right-click menu app-wide, *except* over
/// editable controls. Our own context menus are driven by explicit handlers, so
/// the default one only appears in areas without one — but text fields have no
/// replacement, so they keep the native menu for copy/paste/select-all/spellcheck.
fn suppress_native_context_menu() {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;
    let Some(window) = leptos::web_sys::window() else { return };
    let cb = Closure::<dyn Fn(leptos::web_sys::Event)>::new(|ev: leptos::web_sys::Event| {
        if !is_editable_target(ev.target()) {
            ev.prevent_default();
        }
    });
    if let Err(e) = window.add_event_listener_with_callback("contextmenu", cb.as_ref().unchecked_ref()) {
        log::error!("failed to attach contextmenu suppressor: {e:?}");
    }
    cb.forget();
}

/// Whether a contextmenu target is a text-editable control (input, textarea, or
/// contenteditable) that should keep its native menu.
fn is_editable_target(target: Option<leptos::web_sys::EventTarget>) -> bool {
    use leptos::web_sys::{Element, HtmlElement};
    use wasm_bindgen::JsCast;
    let Some(target) = target else { return false };
    if let Some(html) = target.dyn_ref::<HtmlElement>() {
        if html.is_content_editable() {
            return true;
        }
    }
    target
        .dyn_ref::<Element>()
        .map(|el| {
            let tag = el.tag_name();
            tag.eq_ignore_ascii_case("input") || tag.eq_ignore_ascii_case("textarea")
        })
        .unwrap_or(false)
}
