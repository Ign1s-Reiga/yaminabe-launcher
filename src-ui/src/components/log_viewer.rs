use crate::components::ui::{ButtonSize, Dropdown, DropdownItem, TextInput};
use bamboo_css_macro::{css, cx};
use leptos::__reexports::send_wrapper::SendWrapper;
use leptos::prelude::*;
use leptos::{IntoView, component, html, view, web_sys};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use yaminabe_launcher_shared::ipc::{LogEvent, LogLevel};

const LOG_STICKY_THRESHOLD_PX: i32 = 8;
const LOG_SCROLL_THROTTLE_MS: i32 = 50;
/// Most lines kept of one run's log, so a long session cannot grow the buffer,
/// or the page, without bound.
const MAX_LOG_LINES: usize = 100_000;
/// How many of the latest lines the log box can show, as the choices offered.
const LINES_SHOWN_CHOICES: [(usize, &str); 3] = [(1_000, "1,000"), (10_000, "10,000"), (MAX_LOG_LINES, "100,000")];
/// The lines-shown choice last made, which a newly opened log viewer starts
/// from, so it lasts for the session.
static LINES_SHOWN: AtomicUsize = AtomicUsize::new(10_000);

/// One entry of a run's log: a line of the game's output, or of the launcher's
/// progress. A log4j event's text can span several lines.
pub struct LogEntry {
    pub text: String,
    pub event: Option<LogEvent>,
}

impl LogEntry {
    /// An entry that is not a log4j event.
    pub fn plain(text: String) -> LogEntry {
        LogEntry { text, event: None }
    }
}

/// One run's log entries, numbered from the run's start so a reader can tell
/// which entries it has already seen, even after the oldest are dropped.
#[derive(Default)]
pub struct LogBuffer {
    lines: VecDeque<LogEntry>,
    pushed: usize,
}

impl LogBuffer {
    /// Append a line. Once `MAX_LOG_LINES` are held, the oldest tenth goes at
    /// once, so the shown text jumps rarely rather than shifting every line.
    pub fn push(&mut self, line: LogEntry) {
        if self.lines.len() >= MAX_LOG_LINES {
            self.lines.drain(..MAX_LOG_LINES / 10);
        }
        self.lines.push_back(line);
        self.pushed += 1;
    }

    /// Number of the oldest line still held.
    fn first(&self) -> usize {
        self.pushed - self.lines.len()
    }

    /// The held lines numbered `from` and later.
    fn since(&self, from: usize) -> impl Iterator<Item = &LogEntry> {
        self.lines.iter().skip(from.saturating_sub(self.first()))
    }
}

/// The lines the log box shows, as numbers into the run's [`LogBuffer`]: one
/// `<div>` child per line, for lines `first` up to `end`, at most about
/// `window` of them. Lines not containing `search` (in lower case) are hidden.
struct Shown {
    log: Option<ArcRwSignal<LogBuffer>>,
    window: usize,
    search: String,
    first: usize,
    end: usize,
}

/// The level groups the viewer can hide, each as `(data-level value, label)`.
/// TRACE counts as debug and FATAL as error, as their lines are rare.
const LEVEL_GROUPS: [(&str, &str); 4] = [("debug", "Debug"), ("info", "Info"), ("warn", "Warn"), ("error", "Error")];

fn level_group(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Trace | LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error | LogLevel::Fatal => "error",
    }
}

/// Hide `line` unless its text contains `search`, which is in lower case.
fn show_if_found(line: &web_sys::Element, search: &str) {
    let found = search.is_empty() || line.text_content().is_some_and(|t| t.to_lowercase().contains(search));
    if let Err(e) = line.toggle_attribute_with_force("hidden", !found) {
        log::warn!("failed to filter a log line: {e:?}");
    }
}

/// Apply a new search to every line the log box shows.
fn search_lines(el: &web_sys::HtmlDivElement, search: &str) {
    let lines = el.children();
    for i in 0..lines.length() {
        if let Some(line) = lines.item(i) {
            show_if_found(&line, search);
        }
    }
}

/// Where the log box's lines should start: no older than the buffer holds, and
/// within the latest `window` lines. It trims only once the lines overflow the
/// window by a tenth, so they leave the top in batches.
fn first_shown(buffer: &LogBuffer, shown: &Shown) -> usize {
    let first = buffer.first().max(shown.first);
    if buffer.pushed - first > shown.window + shown.window / 10 {
        buffer.pushed - shown.window
    } else {
        first
    }
}

/// Bring the log box's children in line with `buffer`. Only appends new lines
/// and removes old ones from the top, so a selection, and the lines being
/// read, stay put.
fn sync_lines(el: &web_sys::HtmlDivElement, buffer: &LogBuffer, shown: &mut Shown, keep_place: bool) {
    let first = first_shown(buffer, shown);
    let dropped = first.clamp(shown.first, shown.end) - shown.first;
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
    shown.first = first;
    shown.end = shown.end.max(first);
    if shown.end == buffer.pushed {
        return;
    }
    let Some(document) = el.owner_document() else {
        return;
    };
    let lines = document.create_document_fragment();
    for entry in buffer.since(shown.end) {
        match document.create_element("div") {
            Ok(line) => {
                match &entry.event {
                    Some(event) => {
                        line.set_text_content(Some(&event_text(event, &entry.text)));
                        line.set_class_name(level_class(event.level));
                        if let Err(e) = line.set_attribute("data-level", level_group(event.level)) {
                            log::warn!("failed to mark a log line's level: {e:?}");
                        }
                    }
                    None => line.set_text_content(Some(&entry.text)),
                }
                if !shown.search.is_empty() {
                    show_if_found(&line, &shown.search);
                }
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
}

/// An event's line as the game writes it to `latest.log`:
/// `[12:34:56] [Render thread/INFO]: message`, in local time.
fn event_text(event: &LogEvent, text: &str) -> String {
    let at = js_sys::Date::new(&JsValue::from_f64(event.timestamp as f64));
    format!(
        "[{:02}:{:02}:{:02}] [{}/{}]: {text}",
        at.get_hours(), at.get_minutes(), at.get_seconds(), event.thread, event.level.as_str()
    )
}

fn level_class(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Trace | LogLevel::Debug => css! { color: #8b8b8b; },
        LogLevel::Info => "",
        LogLevel::Warn => css! { color: #e5c07b; },
        LogLevel::Error | LogLevel::Fatal => css! { color: #f07178; },
    }
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
        flex-wrap: wrap;
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
        &[data-hide-debug] > [data-level="debug"],
        &[data-hide-info] > [data-level="info"],
        &[data-hide-warn] > [data-level="warn"],
        &[data-hide-error] > [data-level="error"] {
            display: none;
        }
    }
}

fn controls_class() -> &'static str {
    css! {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 8px;
        margin-right: auto;
    }
}

fn level_toggle_class() -> &'static str {
    css! {
        padding: 4px 10px;
        border: 1px solid #2a2a2a;
        border-radius: 6px;
        background-color: #222;
        color: #d4d4d4;
        font-family: inherit;
        font-size: 0.8rem;
        cursor: pointer;
        transition: opacity 0.12s ease;
    }
}

fn level_toggle_off_class() -> &'static str {
    css! {
        opacity: 0.4;
        text-decoration: line-through;
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

    let lines_shown = RwSignal::new(LINES_SHOWN.load(Ordering::Relaxed));
    let lines_shown_label = Signal::derive(move || {
        let n = lines_shown.get();
        let label = LINES_SHOWN_CHOICES.iter().find(|(c, _)| *c == n).map_or("", |(_, label)| label);
        format!("Last {label} lines")
    });
    let search = RwSignal::new(String::new());
    let hidden_levels = RwSignal::new(Vec::<&'static str>::new());
    let level_hidden = move |group: &str| hidden_levels.with(|hidden| hidden.contains(&group));
    let shown = StoredValue::new_local(Shown { log: None, window: 0, search: String::new(), first: 0, end: 0 });
    let scroll_for_effect = scroll.clone();
    Effect::new(move |_| {
        let current = log.get();
        let window = lines_shown.get();
        let search = search.with(|s| s.trim().to_lowercase());
        hidden_levels.track();
        let Some(el) = log_box_ref.get() else {
            return;
        };
        let keep_place = scroll_for_effect.should_skip_scroll();
        shown.update_value(|shown| {
            if shown.log != current || shown.window != window {
                el.set_text_content(None);
                *shown = Shown { log: current.clone(), window, search: search.clone(), first: 0, end: 0 };
            }
            if shown.search != search {
                search_lines(&el, &search);
                shown.search = search;
            }
            if let Some(log) = &current {
                log.with(|buffer| sync_lines(&el, buffer, shown, keep_place));
            }
        });
        // Keeping the reader's place moves the view up; that must not read as
        // the user scrolling up, which would turn auto-scroll off.
        if keep_place {
            scroll_for_effect.last_scroll_top.set_value(el.scroll_top());
        }
        scroll_for_effect.schedule_scroll_to_bottom();
    });

    view! {
        <div class=log_viewer_class()>
            <div class=log_viewer_header_class()>
                <div class=controls_class()>
                    <Dropdown label=lines_shown_label size=ButtonSize::Small>
                        {move || LINES_SHOWN_CHOICES.map(|(n, label)| view! {
                            <DropdownItem on_select=Callback::new(move |_| {
                                LINES_SHOWN.store(n, Ordering::Relaxed);
                                lines_shown.set(n);
                            })>
                                {format!("Last {label} lines")}
                            </DropdownItem>
                        })}
                    </Dropdown>
                    {LEVEL_GROUPS.map(|(group, label)| view! {
                        <button
                            type="button"
                            class=move || cx!(level_toggle_class(), if level_hidden(group) { level_toggle_off_class() } else { "" })
                            aria-pressed=move || if level_hidden(group) { "false" } else { "true" }
                            on:click=move |_| hidden_levels.update(|hidden| {
                                match hidden.iter().position(|g| *g == group) {
                                    Some(at) => {
                                        hidden.remove(at);
                                    }
                                    None => hidden.push(group),
                                }
                            })
                        >
                            {label}
                        </button>
                    })}
                    <div style="width: 220px;">
                        <TextInput
                            placeholder="Search the log"
                            on_change=Callback::new(move |text: String| search.set(text))
                        />
                    </div>
                </div>
                {children()}
            </div>
            <div
                class=log_box_class()
                node_ref=log_box_ref
                data-hide-debug=move || level_hidden("debug").then_some("")
                data-hide-info=move || level_hidden("info").then_some("")
                data-hide-warn=move || level_hidden("warn").then_some("")
                data-hide-error=move || level_hidden("error").then_some("")
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
            buffer.push(LogEntry::plain(i.to_string()));
        }
        assert_eq!(buffer.lines.len(), MAX_LOG_LINES - MAX_LOG_LINES / 10 + 5);
        assert_eq!(buffer.first(), 10_000);
        let texts: Vec<_> = buffer.since(MAX_LOG_LINES + 3).map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["100003", "100004"]);
        assert_eq!(buffer.since(0).next().map(|e| e.text.as_str()), Some("10000"));
        assert!(buffer.since(MAX_LOG_LINES + 5).next().is_none());
    }

    #[test]
    fn the_window_trims_in_batches() {
        let mut buffer = LogBuffer::default();
        let mut shown = Shown { log: None, window: 1_000, search: String::new(), first: 0, end: 0 };
        for i in 0..1_100 {
            buffer.push(LogEntry::plain(i.to_string()));
        }
        assert_eq!(first_shown(&buffer, &shown), 0);
        buffer.push(LogEntry::plain(String::new()));
        assert_eq!(first_shown(&buffer, &shown), 101);
        shown.first = 101;
        shown.end = 1_101;
        buffer.push(LogEntry::plain(String::new()));
        assert_eq!(first_shown(&buffer, &shown), 101);
    }
}
