pub mod components;
pub mod pages;
pub mod app;
pub mod ipc;
pub mod curseforge;
pub mod signal_ext;

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

/// Suppress the WebView's native right-click menu app-wide. Our own context
/// menus are driven by explicit handlers; the default one only appears in areas
/// without one, where it's unwanted in both debug and release builds.
fn suppress_native_context_menu() {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;
    let Some(window) = leptos::web_sys::window() else { return };
    let cb = Closure::<dyn Fn(leptos::web_sys::Event)>::new(|ev: leptos::web_sys::Event| {
        ev.prevent_default();
    });
    if let Err(e) = window.add_event_listener_with_callback("contextmenu", cb.as_ref().unchecked_ref()) {
        log::error!("failed to attach contextmenu suppressor: {e:?}");
    }
    cb.forget();
}
