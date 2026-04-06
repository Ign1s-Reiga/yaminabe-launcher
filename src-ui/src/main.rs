use leptos::prelude::*;
use yaminabe_launcher_ui::app::App;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| {
        view! {
            <App/>
        }
    })
}
