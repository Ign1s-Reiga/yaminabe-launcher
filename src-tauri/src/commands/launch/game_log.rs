use yaminabe_launcher_shared::ipc::{LogEvent, LogLevel};

const EVENT_START: &str = "<log4j:Event";
const EVENT_END: &str = "</log4j:Event>";
/// Lines after which an event that has not ended is shown as it came, so a
/// stray start tag cannot hold back the rest of the output.
const MAX_EVENT_LINES: usize = 10_000;

/// One entry of the game's log, as the viewer shows it.
#[derive(Debug, PartialEq)]
pub(super) struct Entry {
    pub text: String,
    pub event: Option<LogEvent>,
}

impl Entry {
    fn plain(text: String) -> Entry {
        Entry { text, event: None }
    }
}

/// Reads the game's standard output line by line. Launched with Mojang's log
/// config, the game writes each log4j event as XML over several lines; those
/// become one entry with the event's time, thread and level. Anything else,
/// or an event that cannot be read, is passed on line by line as it came.
#[derive(Default)]
pub(super) struct GameLogReader {
    pending: Vec<String>,
    /// Whether the pending event's last line left a CDATA section open, so the
    /// next line is part of a message rather than the XML around it.
    in_cdata: bool,
}

impl GameLogReader {
    /// Read one output line, adding the entries it completes to `out`.
    pub fn line(&mut self, line: String, out: &mut Vec<Entry>) {
        let mut part = line.as_str();
        while let Some(rest) = self.read_part(part, out) {
            part = rest;
        }
    }

    /// Read `part` of an output line. Returns what follows an event that ended
    /// in it, when there is more to read.
    fn read_part<'a>(&mut self, part: &'a str, out: &mut Vec<Entry>) -> Option<&'a str> {
        let trimmed = part.trim_start();
        // A message can quote a start tag, but not usually a whole one; taking
        // a real one as text would hold back the output after it.
        let starts = trimmed.starts_with(EVENT_START) && (!self.in_cdata || is_event_tag(trimmed));
        if self.pending.is_empty() && !starts {
            out.push(Entry::plain(part.to_string()));
            return None;
        }
        if starts {
            self.flush(out);
        } else if !self.in_cdata && !trimmed.starts_with("<log4j:") && !trimmed.starts_with("</log4j:") {
            // Printed by something else while an event was being written.
            out.push(Entry::plain(part.to_string()));
            return None;
        }
        let (in_cdata, end) = scan(part, self.in_cdata);
        self.in_cdata = in_cdata;
        let Some(end) = end else {
            self.pending.push(part.to_string());
            if self.pending.len() >= MAX_EVENT_LINES {
                self.flush(out);
            }
            return None;
        };
        self.pending.push(part[..end].to_string());
        match read_event(&self.pending.join("\n")) {
            Some(entry) => {
                self.pending.clear();
                out.push(entry);
            }
            None => self.flush(out),
        }
        let rest = &part[end..];
        (!rest.trim().is_empty()).then_some(rest)
    }

    /// Pass on what is left of an event the output ended in the middle of.
    pub fn finish(&mut self, out: &mut Vec<Entry>) {
        self.flush(out);
    }

    fn flush(&mut self, out: &mut Vec<Entry>) {
        self.in_cdata = false;
        out.extend(self.pending.drain(..).map(Entry::plain));
    }
}

/// Follow `line` through CDATA sections, given whether one was open at its
/// start. Returns whether one is open at its end, or else where the first
/// `</log4j:Event>` outside them ends.
fn scan(line: &str, mut open: bool) -> (bool, Option<usize>) {
    let mut at = 0;
    loop {
        let rest = &line[at..];
        if open {
            let Some(close) = rest.find("]]>") else {
                return (true, None);
            };
            at += close + "]]>".len();
            open = false;
            continue;
        }
        // Look for CDATA only before the next end tag, so a line of many events
        // is read in one pass.
        let end = rest.find(EVENT_END);
        match rest[..end.unwrap_or(rest.len())].find("<![CDATA[") {
            Some(c) => {
                at += c + "<![CDATA[".len();
                open = true;
            }
            None => return (false, end.map(|end| at + end + EVENT_END.len())),
        }
    }
}

/// Whether `line` is a whole event start tag, with a timestamp.
fn is_event_tag(line: &str) -> bool {
    let line = line.trim_end();
    line.ends_with('>') && attribute(line, "timestamp").is_some_and(|t| t.parse::<i64>().is_ok())
}

/// Read one `log4j:Event` element, as Mojang's `LegacyXMLLayout` writes it.
fn read_event(xml: &str) -> Option<Entry> {
    let start = xml.find(EVENT_START)?;
    let tag_end = start + xml[start..].find('>')?;
    let tag = &xml[start..tag_end];
    let body = &xml[tag_end + 1..];
    let event = LogEvent {
        timestamp: attribute(tag, "timestamp")?.parse().ok()?,
        thread: attribute(tag, "thread").unwrap_or_default(),
        level: attribute(tag, "level").and_then(|l| LogLevel::parse(&l)).unwrap_or(LogLevel::Info),
    };
    let mut text = element_text(body, "log4j:Message")?;
    if let Some(throwable) = element_text(body, "log4j:Throwable") {
        let throwable = throwable.trim_end();
        if !throwable.is_empty() {
            text.push('\n');
            text.push_str(throwable);
        }
    }
    let text = text.trim_end_matches(['\r', '\n']).to_string();
    Some(Entry { text, event: Some(event) })
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let len = tag[start..].find('"')?;
    Some(unescape(&tag[start..start + len]))
}

/// The text of the first `name` element in `xml`, joining its CDATA sections
/// (log4j splits a `]]>` in the text across two).
fn element_text(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml.find(&open)? + open.len();
    let inner = &xml[start..start + xml[start..].find(&close)?];
    if !inner.contains("<![CDATA[") {
        return Some(unescape(inner));
    }
    let mut text = String::new();
    let mut rest = inner;
    while let Some(at) = rest.find("<![CDATA[") {
        rest = &rest[at + "<![CDATA[".len()..];
        let end = rest.find("]]>").unwrap_or(rest.len());
        text.push_str(&rest[..end]);
        rest = rest.get(end + "]]>".len()..).unwrap_or("");
    }
    Some(text)
}

fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';') else {
            break;
        };
        let decoded = match &rest[1..end] {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            entity => entity
                .strip_prefix("#x")
                .map(|hex| u32::from_str_radix(hex, 16))
                .or_else(|| entity.strip_prefix('#').map(str::parse))
                .and_then(Result::ok)
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(lines: &[&str]) -> Vec<Entry> {
        let mut reader = GameLogReader::default();
        let mut out = Vec::new();
        for line in lines {
            reader.line(line.to_string(), &mut out);
        }
        reader.finish(&mut out);
        out
    }

    fn event(timestamp: i64, thread: &str, level: LogLevel, text: &str) -> Entry {
        Entry { text: text.to_string(), event: Some(LogEvent { timestamp, thread: thread.to_string(), level }) }
    }

    #[test]
    fn an_event_becomes_one_entry() {
        let out = read(&[
            r#"<log4j:Event logger="net.minecraft.client.Minecraft" timestamp="1700000000123" level="INFO" thread="Render thread">"#,
            "  <log4j:Message><![CDATA[Setting user: Dev]]></log4j:Message>",
            "</log4j:Event>",
            "",
        ]);
        assert_eq!(out, [event(1_700_000_000_123, "Render thread", LogLevel::Info, "Setting user: Dev"), Entry::plain(String::new())]);
    }

    #[test]
    fn a_stack_trace_stays_with_its_event() {
        let out = read(&[
            r#"<log4j:Event logger="x" timestamp="5" level="ERROR" thread="main &amp; co">"#,
            "  <log4j:Message><![CDATA[Failed]]></log4j:Message>",
            "  <log4j:Throwable><![CDATA[java.lang.Exception: boom",
            "\tat a.b(C.java:1)",
            "]]></log4j:Throwable>",
            "</log4j:Event>",
        ]);
        assert_eq!(out, [event(5, "main & co", LogLevel::Error, "Failed\njava.lang.Exception: boom\n\tat a.b(C.java:1)")]);
    }

    #[test]
    fn split_cdata_and_escapes_are_read() {
        let out = read(&[
            r#"<log4j:Event logger="x" timestamp="5" level="warn" thread="t"><log4j:Message><![CDATA[a]]]]><![CDATA[>b]]></log4j:Message></log4j:Event>"#,
            r#"<log4j:Event logger="x" timestamp="6" level="INFO" thread="t"><log4j:Message>1 &lt; 2 &#x263A;</log4j:Message></log4j:Event>"#,
        ]);
        assert_eq!(out, [event(5, "t", LogLevel::Warn, "a]]>b"), event(6, "t", LogLevel::Info, "1 < 2 \u{263A}")]);
    }

    #[test]
    fn output_printed_inside_an_event_is_kept() {
        let out = read(&[
            r#"<log4j:Event logger="x" timestamp="5" level="INFO" thread="t">"#,
            "native warning",
            "  <log4j:Message><![CDATA[Loaded]]></log4j:Message>",
            "</log4j:Event>",
        ]);
        assert_eq!(out, [Entry::plain("native warning".to_string()), event(5, "t", LogLevel::Info, "Loaded")]);
    }

    #[test]
    fn an_end_tag_inside_a_message_does_not_end_the_event() {
        let out = read(&[
            r#"<log4j:Event logger="x" timestamp="5" level="INFO" thread="t">"#,
            "  <log4j:Message><![CDATA[dump:",
            "</log4j:Event>",
            "<log4j:Event logger=\"y\">",
            "done]]></log4j:Message>",
            "</log4j:Event>",
        ]);
        assert_eq!(out, [event(5, "t", LogLevel::Info, "dump:\n</log4j:Event>\n<log4j:Event logger=\"y\">\ndone")]);
    }

    #[test]
    fn output_after_an_end_tag_on_the_same_line_is_kept() {
        let out = read(&[
            r#"<log4j:Event logger="x" timestamp="5" level="INFO" thread="t">"#,
            "  <log4j:Message><![CDATA[Loaded]]></log4j:Message>",
            "</log4j:Event>native msg",
        ]);
        assert_eq!(out, [event(5, "t", LogLevel::Info, "Loaded"), Entry::plain("native msg".to_string())]);
    }

    #[test]
    fn many_events_on_one_line_are_all_read() {
        let one = r#"<log4j:Event logger="x" timestamp="5" level="INFO" thread="t"><log4j:Message>m</log4j:Message></log4j:Event>"#;
        let out = read(&[&one.repeat(100_000)]);
        assert_eq!(out.len(), 100_000);
        assert!(out.iter().all(|e| *e == event(5, "t", LogLevel::Info, "m")));
    }

    #[test]
    fn a_new_event_ends_one_cut_off_in_its_message() {
        let out = read(&[
            r#"<log4j:Event logger="x" timestamp="5" level="INFO" thread="t">"#,
            "  <log4j:Message><![CDATA[cut off",
            r#"<log4j:Event logger="x" timestamp="6" level="INFO" thread="t">"#,
            "  <log4j:Message><![CDATA[Next]]></log4j:Message>",
            "</log4j:Event>",
        ]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[2], event(6, "t", LogLevel::Info, "Next"));
    }

    #[test]
    fn other_output_starting_with_a_tag_is_kept() {
        let out = read(&[
            r#"<log4j:Event logger="x" timestamp="5" level="INFO" thread="t">"#,
            "<init> failed",
            "  <log4j:Message><![CDATA[Loaded]]></log4j:Message>",
            "</log4j:Event>",
        ]);
        assert_eq!(out, [Entry::plain("<init> failed".to_string()), event(5, "t", LogLevel::Info, "Loaded")]);
    }

    #[test]
    fn other_output_passes_through() {
        let out = read(&[
            "Picked up JAVA_TOOL_OPTIONS",
            r#"<log4j:Event logger="x" timestamp="oops" level="INFO" thread="t">"#,
            "</log4j:Event>",
            r#"<log4j:Event logger="x" timestamp="7" level="INFO" thread="t">"#,
        ]);
        let texts: Vec<_> = out.iter().map(|e| (e.text.as_str(), e.event.is_some())).collect();
        assert_eq!(texts, [
            ("Picked up JAVA_TOOL_OPTIONS", false),
            (r#"<log4j:Event logger="x" timestamp="oops" level="INFO" thread="t">"#, false),
            ("</log4j:Event>", false),
            (r#"<log4j:Event logger="x" timestamp="7" level="INFO" thread="t">"#, false),
        ]);
    }
}
