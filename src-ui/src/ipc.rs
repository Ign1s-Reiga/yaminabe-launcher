use leptos::web_sys;
use serde::Serialize;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], catch)]
    async fn listen(event: &str, handler: &js_sys::Function) -> Result<JsValue, JsValue>;
}

/// Call a Tauri command with named args (any Serialize struct).
pub async fn call<A, R>(
    cmd: &str,
    args: A,
) -> Result<R, String>
where
    A: Serialize,
    R: for<'de> serde::Deserialize<'de>,
{
    let js_args = serde_wasm_bindgen::to_value(&args).map_err(|e| e.to_string())?;
    let result = invoke(cmd, js_args).await.map_err(|e| format!("{e:?}"))?;
    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Call a Tauri command that takes no arguments.
pub async fn call_noargs<R: for<'de> serde::Deserialize<'de>>(cmd: &str) -> Result<R, String> {
    #[derive(Serialize)]
    struct NoArgs {}
    call(cmd, NoArgs {}).await
}

/// Calls `prevent_default()` on `ev` and returns the form's `FormData`, or
/// `None` if the target is not an `HtmlFormElement` or `FormData` construction
/// fails. Centralises the boilerplate every form submit handler needs.
pub fn form_data_from_submit(ev: &leptos::ev::SubmitEvent) -> Option<web_sys::FormData> {
    ev.prevent_default();
    let form = ev.target()?.dyn_into::<web_sys::HtmlFormElement>().ok()?;
    web_sys::FormData::new_with_form(&form).ok()
}

/// Subscribe to a Tauri backend event for the lifetime of the app.
/// The handler receives the deserialized payload of each event.
pub fn on_event<T, F>(event: &'static str, handler: F)
where
    T: for<'de> serde::Deserialize<'de> + 'static,
    F: Fn(T) + 'static,
{
    let cb = Closure::<dyn Fn(JsValue)>::new(move |raw: JsValue| {
        let payload = js_sys::Reflect::get(&raw, &JsValue::from_str("payload"))
            .unwrap_or(JsValue::UNDEFINED);
        if let Ok(val) = serde_wasm_bindgen::from_value::<T>(payload) {
            handler(val);
        }
    });
    leptos::task::spawn_local(async move {
        let _ = listen(event, cb.as_ref().unchecked_ref()).await;
        cb.forget();
    });
}
