use bamboo_css_macro::css;
use leptos::__reexports::send_wrapper::SendWrapper;
use leptos::prelude::*;
use leptos::{IntoView, component, html, view, web_sys};
use std::collections::VecDeque;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

const LOG_STICKY_THRESHOLD_PX: i32 = 8;
const LOG_SCROLL_THROTTLE_MS: i32 = 50;
/// Most lines kept of one run's log. Older lines are dropped as new ones arrive,
/// so a long session cannot grow the buffer, or the page, without bound.
const MAX_LOG_LINES: usize = 100_000;

/// One run's log lines, numbered from the run's start so a reader can tell
/// which lines it has already seen, even after the oldest are dropped.
#[derive(Default)]
pub struct LogBuffer {
    lines: VecDeque<String>,
    pushed: usize,
}

impl LogBuffer {
    /// Append a line, dropping the oldest once `MAX_LOG_LINES` are held.
    pub fn push(&mut self, line: String) {
        if self.lines.len() >= MAX_LOG_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        self.pushed += 1;
    }

    /// Number of the oldest line still held.
    fn first(&self) -> usize {
        self.pushed - self.lines.len()
    }

    /// The held lines numbered `from` and later.
    fn since(&self, from: usize) -> impl Iterator<Item = &String> {
        self.lines.iter().skip(from.saturating_sub(self.first()))
    }
}

/// The lines the log box shows, as numbers into the run's [`LogBuffer`]: one
/// `<div>` child per line, for lines `first` up to `end`.
struct Shown {
    log: Option<ArcRwSignal<LogBuffer>>,
    first: usize,
    end: usize,
}

/// Bring the log box's children in line with `buffer`. Only appends new lines
/// and removes dropped ones from the top, so a selection, and the lines being
/// read, stay put. Returns whether anything changed.
fn sync_lines(el: &web_sys::HtmlDivElement, buffer: &LogBuffer, shown: &mut Shown, keep_place: bool) -> bool {
    let dropped = buffer.first().clamp(shown.first, shown.end) - shown.first;
    if dropped > 0 {
        let height = el.scroll_height();
        for _ in 0..dropped {
            if let Some(line) = el.first_child() {
                if let Err(e) = el.remove_child(&line) {
                    log::warn!("failed to remove a log line: {e:?}");
                }
            }
        }
        // Lines removed above the view would pull the text up under the reader.
        if keep_place {
            el.set_scroll_top(el.scroll_top() - (height - el.scroll_height()));
        }
    }
    shown.first = buffer.first();
    shown.end = shown.end.max(shown.first);
    if shown.end == buffer.pushed {
        return dropped > 0;
    }
    let Some(document) = el.owner_document() else {
        return dropped > 0;
    };
    let lines = document.create_document_fragment();
    for text in buffer.since(shown.end) {
        match document.create_element("div") {
            Ok(line) => {
                line.set_text_content(Some(text));
                if let Err(e) = lines.append_child(&line) {
                    log::warn!("failed to add a log line: {e:?}");
                }
            }
            Err(e) => log::warn!("failed to create a log line: {e:?}"),
        }
    }
    if let Err(e) = el.append_child(&lines) {
        log::warn!("failed to add log lines: {e:?}");
    }
    shown.end = buffer.pushed;
    true
}

struct ScheduledScroll {
    handle: i32,
    _callback: Closure<dyn FnMut()>,
}

type ScheduledScrollState = SendWrapper<Rc<RefCell<Option<ScheduledScroll>>>>;

fn log_is_near_bottom(log_box_ref: NodeRef<html::Div>) -> bool {
    log_box_ref.get().map_or(true, |el| {
        el.scroll_height() - el.scroll_top() - el.client_height() <= LOG_STICKY_THRESHOLD_PX
    })
}

fn has_text_selection() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Ok(get_selection) =
        js_sys::Reflect::get(window.as_ref(), &JsValue::from_str("getSelection"))
    else {
        return false;
    };
    let Ok(get_selection) = get_selection.dyn_into::<js_sys::Function>() else {
        return false;
    };
    let Ok(selection) = get_selection.call0(window.as_ref()) else {
        return false;
    };
    if selection.is_null() || selection.is_undefined() {
        return false;
    }
    js_sys::Reflect::get(&selection, &JsValue::from_str("isCollapsed"))
        .ok()
        .and_then(|value| value.as_bool())
        .map_or(false, |is_collapsed| !is_collapsed)
}

#[derive(Clone)]
struct LogScrollState {
    log_box_ref: NodeRef<html::Div>,
    auto_scroll_enabled: RwSignal<bool>,
    selecting_text: RwSignal<bool>,
    scroll_pending: StoredValue<bool>,
    scheduled_scroll: ScheduledScrollState,
    /// Last observed `scrollTop`, used by `handle_scroll_event` to discriminate
    /// user-initiated upward scrolls from programmatic catch-up scrolls.
    last_scroll_top: StoredValue<i32>,
}

impl LogScrollState {
    fn should_skip_scroll(&self) -> bool {
        !self.auto_scroll_enabled.get_untracked()
            || self.selecting_text.get_untracked()
            || has_text_selection()
    }

    fn schedule_scroll_to_bottom(&self) {
        if self.should_skip_scroll() || self.scroll_pending.get_value() {
            return;
        }

        let on_timeout = self.clone();
        self.scroll_pending.set_value(true);
        let callback = Closure::<dyn FnMut()>::new(move || {
            on_timeout.scroll_pending.set_value(false);
            let _taken = on_timeout.scheduled_scroll.borrow_mut().take();
            if on_timeout.should_skip_scroll() {
                return;
            }
            if let Some(el) = on_timeout.log_box_ref.get() {
                el.set_scroll_top(el.scroll_height());
            }
        });

        if let Some(window) = web_sys::window() {
            if let Ok(handle) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                LOG_SCROLL_THROTTLE_MS,
            ) {
                *self.scheduled_scroll.borrow_mut() = Some(ScheduledScroll {
                    handle,
                    _callback: callback,
                });
                return;
            }
        }

        self.scroll_pending.set_value(false);
    }

    fn finish_text_selection(&self) {
        if !self.selecting_text.get_untracked() {
            return;
        }
        self.selecting_text.set(false);
        self.auto_scroll_enabled
            .set(log_is_near_bottom(self.log_box_ref));
        self.schedule_scroll_to_bottom();
    }

    /// Update `auto_scroll_enabled` from a `scroll` DOM event.
    ///
    /// `scroll` events fire for both user input and programmatic
    /// `scrollTop = scrollHeight` writes. When logs arrive faster than the
    /// 50ms scroll throttle, content added between the write and the event
    /// makes `log_is_near_bottom` return false even though we just scrolled
    /// to the bottom — that race used to disable auto-scroll permanently
    /// during heavy log bursts. Only upward movement (`new_top < prev_top`)
    /// can disable auto-scroll now; downward / stationary events at most
    /// re-enable it when the view actually reaches the bottom.
    fn handle_scroll_event(&self) {
        if self.selecting_text.get_untracked() {
            return;
        }
        let Some(el) = self.log_box_ref.get() else { return; };
        let new_top = el.scroll_top();
        let prev_top = self.last_scroll_top.get_value();
        self.last_scroll_top.set_value(new_top);

        if new_top < prev_top {
            self.auto_scroll_enabled.set(log_is_near_bottom(self.log_box_ref));
        } else if log_is_near_bottom(self.log_box_ref) {
            self.auto_scroll_enabled.set(true);
        }
    }
}

fn log_viewer_class() -> &'static str {
    css! {
        display: flex;
        flex-direction: column;
        background-color: #0d0d0d;
        border-radius: 8px;
        overflow: hidden;
        flex: 1;
        min-height: 0;
    }
}

fn log_viewer_header_class() -> &'static str {
    css! {
        display: flex;
        align-items: center;
        justify-content: flex-end;
        gap: 8px;
        padding: 8px 12px;
        background-color: #161616;
        border-bottom: 1px solid #2a2a2a;
    }
}

fn log_box_class() -> &'static str {
    css! {
        padding: 16px;
        font-family: "Roboto Mono", monospace;
        font-weight: 400;
        font-size: 0.8rem;
        line-height: 1.6;
        overflow: auto;
        max-height: calc(100vh - 340px);
        min-height: 240px;
        white-space: pre;
        color: #d4d4d4;
        flex: 1;
        overflow-anchor: none;

        & > div:empty::before {
            content: " ";
        }
    }
}

/// Dark log box with a sticky-tail scroll behaviour and a header slot for
/// per-page action buttons. Owns its scroll state; the caller hands in the
/// run's log, if there is one yet.
///
/// Children passed to the component are rendered into the header action bar
/// above the log box — typically a `<Button>` or two (Stop, Open folder).
#[component]
pub fn LogViewer(
    #[prop(into)] log: Signal<Option<ArcRwSignal<LogBuffer>>>,
    children: Children,
) -> impl IntoView {
    let log_box_ref: NodeRef<html::Div> = NodeRef::new();
    let scroll = LogScrollState {
        log_box_ref,
        auto_scroll_enabled: RwSignal::new(true),
        selecting_text: RwSignal::new(false),
        scroll_pending: StoredValue::new(false),
        scheduled_scroll: SendWrapper::new(Rc::new(RefCell::new(None::<ScheduledScroll>))),
        last_scroll_top: StoredValue::new(0),
    };
    let selecting_text = scroll.selecting_text;

    if let Some(window) = web_sys::window() {
        let scroll_for_mouseup = scroll.clone();
        let callback = Closure::<dyn FnMut()>::new(move || {
            scroll_for_mouseup.finish_text_selection();
        });
        let listener = callback
            .as_ref()
            .unchecked_ref::<js_sys::Function>()
            .clone();
        if let Err(e) = window.add_event_listener_with_callback("mouseup", listener.as_ref()) {
            log::warn!("failed to attach window mouseup listener: {e:?}");
        }
        let callback = SendWrapper::new(callback);
        on_cleanup(move || {
            if let Some(window) = web_sys::window() {
                if let Err(e) = window.remove_event_listener_with_callback("mouseup", listener.as_ref()) {
                    log::warn!("failed to remove window mouseup listener: {e:?}");
                }
            }
            drop(callback);
        });
    }

    let scroll_for_cleanup = scroll.clone();
    on_cleanup(move || {
        if let Some(scheduled) = scroll_for_cleanup.scheduled_scroll.borrow_mut().take() {
            if let Some(window) = web_sys::window() {
                window.clear_timeout_with_handle(scheduled.handle);
            }
        }
    });

    let shown = StoredValue::new_local(Shown { log: None, first: 0, end: 0 });
    let scroll_for_effect = scroll.clone();
    Effect::new(move |_| {
        let current = log.get();
        let Some(el) = log_box_ref.get() else {
            return;
        };
        let keep_place = scroll_for_effect.should_skip_scroll();
        let changed = shown.try_update_value(|shown| {
            if shown.log != current {
                el.set_text_content(None);
                *shown = Shown { log: current.clone(), first: 0, end: 0 };
            }
            current.as_ref().is_some_and(|log| log.with(|buffer| sync_lines(&el, buffer, shown, keep_place)))
        });
        if changed == Some(true) {
            scroll_for_effect.schedule_scroll_to_bottom();
        }
    });

    view! {
        <div class=log_viewer_class()>
            <div class=log_viewer_header_class()>
                {children()}
            </div>
            <div
                class=log_box_class()
                node_ref=log_box_ref
                on:scroll={
                    let scroll = scroll.clone();
                    move |_| scroll.handle_scroll_event()
                }
                on:mousedown=move |_| {
                    selecting_text.set(true);
                }
                on:mouseup=move |_| {
                    scroll.finish_text_selection();
                }
            ></div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_keep_their_numbers_after_the_oldest_are_dropped() {
        let mut buffer = LogBuffer::default();
        for i in 0..MAX_LOG_LINES + 5 {
            buffer.push(i.to_string());
        }
        assert_eq!(buffer.lines.len(), MAX_LOG_LINES);
        assert_eq!(buffer.first(), 5);
        assert_eq!(buffer.since(MAX_LOG_LINES + 3).collect::<Vec<_>>(), ["100003", "100004"]);
        assert_eq!(buffer.since(0).next().map(String::as_str), Some("5"));
        assert_eq!(buffer.since(MAX_LOG_LINES + 5).next(), None);
    }
}
