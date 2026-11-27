use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use yaminabe_launcher_shared::ipc::{LogEvent, LogLevel};

/// How long the game's output can stay quiet before an unfinished line or log
/// event is shown as it came. An event larger than Java's output buffer is
/// written in parts, so this allows for a long JVM pause between them.
const HELD_EVENT_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest line of output held in memory; a longer one, or output that never
/// prints a newline, is passed on in parts of this size.
const MAX_LINE_BYTES: usize = 1024 * 1024;

const EVENT_START: &str = "<log4j:Event";
const EVENT_END: &str = "</log4j:Event>";
/// Lines, or bytes, after which an event that has not ended is shown as it
/// came, so a stray start tag cannot hold back the rest of the output.
const MAX_EVENT_LINES: usize = 10_000;
const MAX_EVENT_BYTES: usize = 8 * 1024 * 1024;

/// Read one of the game's output streams until it ends, passing each entry to
/// `emit`. stdout goes through a [`GameLogReader`]; stderr lines are passed on
/// as they came, marked `[STDERR]`. Lines are read as bytes and decoded
/// lossily: Java's stderr uses the system codepage on Windows (e.g. CP932),
/// and `BufReader::lines()` would stop at the first non-UTF-8 line.
pub(super) async fn drain(reader: impl AsyncRead + Unpin, is_stderr: bool, mut emit: impl FnMut(Entry)) {
    let mut reader = BufReader::new(reader);
    let mut out = OutputLine { is_stderr, ..OutputLine::default() };
    loop {
        // The wait is for the next bytes, not the whole line, so a line that
        // arrives slowly is not cut while the output keeps coming.
        let chunk = match tokio::time::timeout(HELD_EVENT_TIMEOUT, reader.fill_buf()).await {
            // Quiet output: pass on what is held back, as a line or event cut
            // off by a crash would otherwise hide the output after it.
            Err(_quiet) => {
                let ready = ready_len(&out.line);
                if ready > 0 {
                    out.pass_on(ready, false);
                }
                out.game_log.finish(&mut out.entries);
                out.entries.drain(..).for_each(&mut emit);
                continue;
            }
            Ok(Err(_)) => break,
            Ok(Ok([])) => break,
            Ok(Ok(chunk)) => chunk,
        };
        let room = MAX_LINE_BYTES - out.line.len();
        let (taken, ends) = match chunk[..chunk.len().min(room)].iter().position(|&b| b == b'\n') {
            Some(at) => (at + 1, true),
            None => (chunk.len().min(room), false),
        };
        out.line.extend_from_slice(&chunk[..taken]);
        reader.consume(taken);
        if ends {
            out.pass_on(out.line.len(), false);
        } else if out.line.len() >= MAX_LINE_BYTES {
            let ready = ready_len(&out.line).min(out.line.len() - held_start_len(&out.line));
            out.pass_on(ready, true);
        }
        out.entries.drain(..).for_each(&mut emit);
    }
    if !out.line.is_empty() {
        out.pass_on(out.line.len(), false);
    }
    out.game_log.finish(&mut out.entries);
    out.entries.drain(..).for_each(&mut emit);
}

/// The line of output [`drain`] is reading, which it passes on in parts when
/// the line is longer than [`MAX_LINE_BYTES`] or the output goes quiet.
#[derive(Default)]
struct OutputLine {
    is_stderr: bool,
    line: Vec<u8>,
    /// Whether part of `line` was passed on already, so the rest continues it.
    continues: bool,
    /// Whether the size cap cut the line just before `line`, so it starts in
    /// the middle of the text rather than where an entry starts.
    after_cap: bool,
    game_log: GameLogReader,
    entries: Vec<Entry>,
}

impl OutputLine {
    /// Pass on the first `len` bytes of `line`, which end the line, or were
    /// cut there by the size cap (`by_cap`) or by quiet output.
    fn pass_on(&mut self, len: usize, by_cap: bool) {
        let ends = self.line[..len].ends_with(b"\n");
        let text = {
            let raw = String::from_utf8_lossy(&self.line[..len]);
            let text = raw.trim_end_matches(['\r', '\n']);
            if self.after_cap { text.to_string() } else { text.trim_start_matches('\r').to_string() }
        };
        self.line.drain(..len);
        // Nothing left is a lone `\r`, or the end of a line split off by a cut.
        let skip = text.is_empty() && (self.continues || !ends);
        if !skip {
            if self.is_stderr {
                self.entries.push(Entry::plain(format!("[STDERR] {text}")));
            } else if self.continues {
                self.game_log.rest_of_line(text, &mut self.entries);
            } else {
                self.game_log.line(text, &mut self.entries);
            }
        }
        self.continues = !ends && (self.continues || !skip);
        self.after_cap = by_cap;
    }
}

/// How much of a line cut short can be passed on now: all but a UTF-8
/// character not finished yet, which waits for the next part.
fn ready_len(bytes: &[u8]) -> usize {
    let len = bytes.len();
    for back in 1..=len.min(3) {
        let byte = bytes[len - back];
        if byte & 0xC0 != 0x80 {
            let needs = match byte {
                0xC0..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF7 => 4,
                _ => 1,
            };
            return if needs > back { len - back } else { len };
        }
    }
    len
}

/// How many bytes at the end of a line cut at the size cap begin a `\r\n`, or
/// a marker that [`scan`] looks for; they wait for the next part so each is
/// read whole, and a `\r` inside the text is kept.
fn held_start_len(bytes: &[u8]) -> usize {
    ["\r\n", "]]>", "<![CDATA[", EVENT_END]
        .iter()
        .flat_map(|marker| (1..marker.len()).filter(move |&k| bytes.ends_with(&marker.as_bytes()[..k])))
        .max()
        .unwrap_or(0)
}

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
    /// The size of `pending`'s lines, so a huge event is shown before it uses
    /// up memory.
    pending_bytes: usize,
    /// Whether the pending event's last line left a CDATA section open, so the
    /// next line is part of a message rather than the XML around it.
    in_cdata: bool,
    /// Whether the line being read went into `pending`, so a later part of it
    /// joins the event rather than showing on its own.
    line_held: bool,
}

impl GameLogReader {
    /// Read one output line, adding the entries it completes to `out`.
    pub fn line(&mut self, line: String, out: &mut Vec<Entry>) {
        self.read(&line, false, out);
    }

    /// Read more of a line whose start was read already, as a long line comes
    /// in parts. Held in an event, it joins that line without a break.
    pub fn rest_of_line(&mut self, part: String, out: &mut Vec<Entry>) {
        self.read(&part, true, out);
    }

    fn read(&mut self, line: &str, continues: bool, out: &mut Vec<Entry>) {
        let mut part = line;
        let mut continues = continues;
        while let Some(rest) = self.read_part(part, continues, out) {
            part = rest;
            continues = false;
        }
    }

    /// Read `part` of an output line. Returns what follows an event that ended
    /// in it, when there is more to read.
    fn read_part<'a>(&mut self, part: &'a str, continues: bool, out: &mut Vec<Entry>) -> Option<&'a str> {
        if continues && !self.line_held {
            out.push(Entry::plain(part.to_string()));
            return None;
        }
        if !continues {
            self.line_held = false;
            let trimmed = part.trim_start();
            // A message can quote a start tag, but not usually a whole one;
            // taking a real one as text would hold back the output after it.
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
        }
        let (in_cdata, end) = scan(part, self.in_cdata);
        self.in_cdata = in_cdata;
        let held = &part[..end.unwrap_or(part.len())];
        self.pending_bytes += held.len();
        match self.pending.last_mut() {
            Some(last) if continues => last.push_str(held),
            _ => self.pending.push(held.to_string()),
        }
        self.line_held = true;
        let Some(end) = end else {
            if self.pending.len() >= MAX_EVENT_LINES || self.pending_bytes >= MAX_EVENT_BYTES {
                self.flush(out);
            }
            return None;
        };
        match read_event(&self.pending.join("\n")) {
            Some(entry) => {
                self.pending.clear();
                self.pending_bytes = 0;
                self.line_held = false;
                out.push(entry);
            }
            None => self.flush(out),
        }
        let rest = &part[end..];
        (!rest.trim().is_empty()).then_some(rest)
    }

    /// Pass on what is left of an event the output ended, or stopped for a
    /// while, in the middle of.
    pub fn finish(&mut self, out: &mut Vec<Entry>) {
        self.flush(out);
    }

    fn flush(&mut self, out: &mut Vec<Entry>) {
        self.in_cdata = false;
        self.pending_bytes = 0;
        self.line_held = false;
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

    /// Write `chunks` to a pipe, waiting `pause` between them, and return what
    /// `drain` passes on with the time (from the start) it did so.
    async fn drain_chunks(chunks: &[&[u8]], pause: Duration, is_stderr: bool) -> Vec<(Duration, String)> {
        use tokio::io::AsyncWriteExt;
        let (mut writer, reader) = tokio::io::duplex(64);
        let start = tokio::time::Instant::now();
        let out = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = out.clone();
        let task = tokio::spawn(drain(reader, is_stderr, move |e| seen.lock().unwrap().push((start.elapsed(), e.text))));
        for chunk in chunks {
            writer.write_all(chunk).await.unwrap();
            tokio::time::sleep(pause).await;
        }
        drop(writer);
        task.await.unwrap();
        std::mem::take(&mut *out.lock().unwrap())
    }

    #[tokio::test(start_paused = true)]
    async fn lines_are_read_across_writes() {
        let out = drain_chunks(&[b"a\nlo", b"ng ", b"line\r\nb\n", b"c"], Duration::ZERO, false).await;
        let texts: Vec<_> = out.into_iter().map(|(_, t)| t).collect();
        assert_eq!(texts, ["a", "long line", "b", "c"]);
    }

    #[tokio::test(start_paused = true)]
    async fn stderr_lines_are_marked() {
        let out = drain_chunks(&[b"Error: \x82\xa0\n"], Duration::ZERO, true).await;
        assert_eq!(out[0].1, "[STDERR] Error: \u{FFFD}\u{FFFD}");
    }

    #[tokio::test(start_paused = true)]
    async fn quiet_output_passes_on_a_cut_off_line_and_event() {
        let cut_off = concat!(
            r#"<log4j:Event logger="x" timestamp="5" level="INFO" thread="t">"#,
            "\n  <log4j:Message><![CDATA[Exception in thr",
        );
        let pause = HELD_EVENT_TIMEOUT * 2;
        let out = drain_chunks(&[cut_off.as_bytes(), b"later\n"], pause, false).await;
        let texts: Vec<_> = out.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, [
            r#"<log4j:Event logger="x" timestamp="5" level="INFO" thread="t">"#,
            "  <log4j:Message><![CDATA[Exception in thr",
            "later",
        ]);
        assert!(out[1].0 < pause, "the cut-off line waited for the output to end");
    }

    #[tokio::test(start_paused = true)]
    async fn a_line_without_a_newline_is_passed_on_in_parts() {
        let long = vec![b'x'; MAX_LINE_BYTES * 2 + 10];
        let out = drain_chunks(&[&long], Duration::ZERO, false).await;
        let lens: Vec<_> = out.iter().map(|(_, t)| t.len()).collect();
        assert_eq!(lens, [MAX_LINE_BYTES, MAX_LINE_BYTES, 10]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_line_is_not_cut_while_output_keeps_coming() {
        let pause = HELD_EVENT_TIMEOUT * 3 / 5;
        let out = drain_chunks(&[b"ab", b"cd", b"ef\n"], pause, false).await;
        let texts: Vec<_> = out.into_iter().map(|(_, t)| t).collect();
        assert_eq!(texts, ["abcdef"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_line_is_not_cut_inside_a_character_or_line_break() {
        let mut long = vec![b'x'; MAX_LINE_BYTES - 1];
        long.extend_from_slice("あ\n".as_bytes());
        let out = drain_chunks(&[&long], Duration::ZERO, false).await;
        let texts: Vec<_> = out.into_iter().map(|(_, t)| t).collect();
        assert_eq!(texts, ["x".repeat(MAX_LINE_BYTES - 1), "あ".to_string()]);

        let mut long = vec![b'x'; MAX_LINE_BYTES - 1];
        long.extend_from_slice(b"\r\n");
        let out = drain_chunks(&[&long], Duration::ZERO, false).await;
        let texts: Vec<_> = out.into_iter().map(|(_, t)| t).collect();
        assert_eq!(texts, ["x".repeat(MAX_LINE_BYTES - 1)]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_marker_cut_by_the_line_cap_is_read_whole() {
        let start = r#"<log4j:Event logger="x" timestamp="5" level="INFO" thread="t">"#;
        let prefix = "  <log4j:Message><![CDATA[";
        let text = "x".repeat(MAX_LINE_BYTES - prefix.len() - 1);
        let output = format!("{start}\n{prefix}{text}]]></log4j:Message>\n</log4j:Event>\n");
        let out = drain_chunks(&[output.as_bytes()], Duration::ZERO, false).await;
        let texts: Vec<_> = out.into_iter().map(|(_, t)| t).collect();
        assert_eq!(texts, [text]);
    }

    #[tokio::test(start_paused = true)]
    async fn carriage_returns_around_a_pause_are_dropped() {
        let pause = HELD_EVENT_TIMEOUT * 2;
        let out = drain_chunks(&[b"\r", b"\r50%\r", b"\r60%\n"], pause, false).await;
        let texts: Vec<_> = out.into_iter().map(|(_, t)| t).collect();
        assert_eq!(texts, ["50%", "60%"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_carriage_return_cut_by_the_line_cap_is_kept() {
        let start = r#"<log4j:Event logger="x" timestamp="5" level="INFO" thread="t">"#;
        let prefix = "  <log4j:Message><![CDATA[";
        let text = "x".repeat(MAX_LINE_BYTES - prefix.len() - 1);
        let output = format!("{start}\n{prefix}{text}\rdone]]></log4j:Message>\n</log4j:Event>\n");
        let out = drain_chunks(&[output.as_bytes()], Duration::ZERO, false).await;
        let texts: Vec<_> = out.into_iter().map(|(_, t)| t).collect();
        assert_eq!(texts, [format!("{text}\rdone")]);
    }

    #[test]
    fn other_output_in_parts_stays_out_of_an_event() {
        let mut reader = GameLogReader::default();
        let mut out = Vec::new();
        reader.line(r#"<log4j:Event logger="x" timestamp="5" level="INFO" thread="t">"#.to_string(), &mut out);
        reader.line("native".to_string(), &mut out);
        reader.rest_of_line(" warning".to_string(), &mut out);
        reader.line("  <log4j:Message><![CDATA[Loaded]]></log4j:Message>".to_string(), &mut out);
        reader.line("</log4j:Event>".to_string(), &mut out);
        assert_eq!(out, [
            Entry::plain("native".to_string()),
            Entry::plain(" warning".to_string()),
            event(5, "t", LogLevel::Info, "Loaded"),
        ]);
    }

    #[test]
    fn a_message_line_in_parts_stays_whole() {
        let mut reader = GameLogReader::default();
        let mut out = Vec::new();
        reader.line(r#"<log4j:Event logger="x" timestamp="5" level="INFO" thread="t">"#.to_string(), &mut out);
        reader.line("  <log4j:Message><![CDATA[aaa".to_string(), &mut out);
        reader.rest_of_line("bbb]]></log4j:Message>".to_string(), &mut out);
        reader.line("</log4j:Event>".to_string(), &mut out);
        assert_eq!(out, [event(5, "t", LogLevel::Info, "aaabbb")]);
    }

    #[test]
    fn a_huge_event_is_shown_before_it_ends() {
        let mut reader = GameLogReader::default();
        let mut out = Vec::new();
        reader.line(r#"<log4j:Event logger="x" timestamp="5" level="INFO" thread="t">"#.to_string(), &mut out);
        reader.line("  <log4j:Message><![CDATA[".to_string(), &mut out);
        for _ in 0..MAX_EVENT_BYTES / MAX_LINE_BYTES {
            reader.rest_of_line("x".repeat(MAX_LINE_BYTES), &mut out);
        }
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].text.len(), MAX_EVENT_BYTES + "  <log4j:Message><![CDATA[".len());
    }

    /// Taking a whole start tag inside a message as a new event is what keeps
    /// an event cut off mid-message from holding back the output after it. The
    /// cost is that a message quoting one shows as raw lines. None is lost, but
    /// the message's tail comes out before the quoted tag, which is still held.
    #[test]
    fn a_message_quoting_a_whole_start_tag_is_shown_raw() {
        let lines = [
            r#"<log4j:Event logger="x" timestamp="5" level="INFO" thread="t">"#,
            "  <log4j:Message><![CDATA[dump:",
            r#"<log4j:Event logger="a" timestamp="1" level="INFO" thread="q">"#,
            "done]]></log4j:Message>",
            "</log4j:Event>",
        ];
        let out = read(&lines);
        assert!(out.iter().all(|e| e.event.is_none()));
        let texts: Vec<_> = out.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, [lines[0], lines[1], lines[3], lines[2], lines[4]]);
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
