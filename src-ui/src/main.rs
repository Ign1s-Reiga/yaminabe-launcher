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
    mount_to_body(|| {
        view! {
            <App/>
        }
    })
}
