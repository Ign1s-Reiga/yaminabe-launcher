use std::cell::{Cell, RefCell};
use std::rc::Rc;
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

/// Payload of Tauri's drag events (`tauri://drag-enter`, `drag-over`,
/// `drag-drop`, `drag-leave`). Only `drag-enter` and `drag-drop` carry paths,
/// so the field defaults rather than failing to decode the other two.
///
/// Tauri intercepts OS file drops itself, so the webview never sees an HTML
/// `drop` event — these carry real filesystem paths instead, which is what the
/// backend commands take anyway.
#[derive(Debug, Default, serde::Deserialize)]
pub struct DragDropPayload {
    #[serde(default)]
    pub paths: Vec<String>,
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
        if let Err(e) = listen(event, cb.as_ref().unchecked_ref()).await {
            log::error!("Tauri listen({event}) failed: {e:?}");
        }
        cb.forget();
    });
}

/// A scoped Tauri event subscription. Unlike [`on_event`], dropping this handle
/// detaches the listener — use it for component-local subscriptions (e.g. a
/// modal that should only react while it is open).
pub struct EventSubscription {
    _closure: Closure<dyn Fn(JsValue)>,
    unlisten: Rc<RefCell<Option<js_sys::Function>>>,
    /// Set on drop. `listen` may still be in flight, in which case the handle is
    /// gone before there is anything to detach; the task reads this and detaches
    /// the moment it resolves.
    dropped: Rc<Cell<bool>>,
}

impl Drop for EventSubscription {
    fn drop(&mut self) {
        self.dropped.set(true);
        if let Some(unlisten) = self.unlisten.borrow_mut().take() {
            unlisten.call0(&JsValue::NULL).ok();
        }
    }
}

/// Subscribe to a Tauri event until the returned [`EventSubscription`] is
/// dropped. The closure is kept alive by the handle; if it is dropped before
/// the async `listen` resolves, the listener is detached as soon as it does —
/// otherwise Tauri would keep calling a closure that has already been freed.
pub fn subscribe<T, F>(event: &'static str, handler: F) -> EventSubscription
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
    let unlisten: Rc<RefCell<Option<js_sys::Function>>> = Rc::new(RefCell::new(None));
    let dropped = Rc::new(Cell::new(false));
    let func: js_sys::Function = cb.as_ref().unchecked_ref::<js_sys::Function>().clone();
    let unlisten_for_task = Rc::clone(&unlisten);
    let dropped_for_task = Rc::clone(&dropped);
    leptos::task::spawn_local(async move {
        match listen(event, &func).await {
            Ok(u) => {
                let unlisten = u.unchecked_into::<js_sys::Function>();
                if dropped_for_task.get() {
                    unlisten.call0(&JsValue::NULL).ok();
                } else {
                    *unlisten_for_task.borrow_mut() = Some(unlisten);
                }
            }
            Err(e) => log::error!("Tauri listen({event}) failed: {e:?}"),
        }
    });
    EventSubscription { _closure: cb, unlisten, dropped }
}
