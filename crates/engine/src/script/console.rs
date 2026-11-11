//! The Script Console: what scripts write with `$.write` and `$.writeln`, what they log with
//! `vc.log`, their uncaught errors, and a start, an end and a report line for each run.
//!
//! A session keeps its own console, and each run keeps one more for its report. Each holds at most
//! [`MAX_CONSOLE_LINES`] lines and [`MAX_CONSOLE_BYTES`] of text, dropping its oldest lines first,
//! and a line holds at most [`MAX_LINE_BYTES`], cut at a character boundary and ended with
//! [`CUT_MARK`]. A script's name in its lines is cut the same way to [`MAX_NAME_BYTES`]. These are
//! VectorCraft's own limits. A line break is a line feed, a carriage
//! return or the two together. Other control characters show as their Unicode Control Pictures
//! (U+2400 to U+2421), and the line and paragraph separators as U+2424.

use std::collections::VecDeque;
use std::sync::Arc;

use serde_json::{Value, json};

/// The most lines a console keeps.
pub const MAX_CONSOLE_LINES: usize = 2_048;
/// The most bytes of text (UTF-8) a console keeps.
pub const MAX_CONSOLE_BYTES: usize = 1 << 20;
/// The most bytes (UTF-8) of one line, its cut mark included.
pub const MAX_LINE_BYTES: usize = 4_096;
/// The most bytes (UTF-8) of a script's name as console lines give it, its cut mark included.
pub const MAX_NAME_BYTES: usize = 256;
/// The end of a line that was cut.
pub const CUT_MARK: char = '…';

/// How a run ended, as its end line gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// The script ran to its end and its changes were kept.
    Finished,
    /// An uncaught error or a syntax error.
    Error,
    /// The instructions ran out (`maxFuel`).
    Fuel,
    /// A host limit: host calls, data passed to commands, growth, or a command's fault.
    HostLimit,
    /// Another limit of the interpreter: heap, call depth or a nested run that could not pause.
    Limit,
    /// `script.stop`, Stop or a cancel from the caller.
    Stopped,
    /// A fault inside VectorCraft.
    Internal,
}

impl Ending {
    /// The id `script.console` gives.
    pub fn id(self) -> &'static str {
        match self {
            Ending::Finished => "finished",
            Ending::Error => "error",
            Ending::Fuel => "fuel",
            Ending::HostLimit => "hostLimit",
            Ending::Limit => "limit",
            Ending::Stopped => "stopped",
            Ending::Internal => "internal",
        }
    }
}

/// A command among the slowest of a run: its id, how often it ran and how long it took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slow {
    pub id: String,
    pub calls: u64,
    pub total_ms: u64,
    pub max_ms: u64,
}

/// What a console line holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// The run started.
    Start,
    /// A line of `$.write` and `$.writeln` output.
    Output(String),
    /// A `vc.log` entry: a string as it is, any other value as JSON.
    Log(String),
    /// The uncaught error that ended the run: `name:line:column: message`.
    Error(String),
    /// The run ended.
    End(Ending),
    /// The run's report: its time (when the platform has a clock), host calls and slowest commands.
    Report { ms: Option<u64>, host_calls: u64, slowest: Vec<Slow> },
}

impl Entry {
    /// The id of the line's kind that `script.console` gives.
    pub fn kind(&self) -> &'static str {
        match self {
            Entry::Start => "start",
            Entry::Output(_) => "output",
            Entry::Log(_) => "log",
            Entry::Error(_) => "error",
            Entry::End(_) => "end",
            Entry::Report { .. } => "report",
        }
    }

    /// The line's text in English, as `script.console` and the run report give it.
    pub fn text(&self) -> String {
        match self {
            Entry::Start => "started".to_string(),
            Entry::Output(t) | Entry::Log(t) | Entry::Error(t) => t.clone(),
            Entry::End(e) => format!("ended: {}", e.id()),
            Entry::Report { ms, host_calls, slowest } => {
                let time = ms.map_or_else(|| "time unavailable".to_string(), |ms| format!("{ms} ms"));
                let calls = if *host_calls == 1 { "host call" } else { "host calls" };
                let mut s = format!("{time}, {host_calls} {calls}");
                if !slowest.is_empty() {
                    let list: Vec<String> = slowest.iter().map(|c| format!("{} ×{} {} ms", c.id, c.calls, c.total_ms)).collect();
                    s.push_str("; slowest: ");
                    s.push_str(&list.join(", "));
                }
                s
            }
        }
    }

    /// The bytes the line counts toward [`MAX_CONSOLE_BYTES`].
    fn bytes(&self) -> usize {
        match self {
            Entry::Output(t) | Entry::Log(t) | Entry::Error(t) => t.len(),
            Entry::Start | Entry::End(_) => 16,
            Entry::Report { slowest, .. } => 48 + slowest.iter().map(|c| c.id.len() + 24).sum::<usize>(),
        }
    }
}

/// A console line: the run it belongs to, by its number in the session, and the script's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleLine {
    pub run: u64,
    pub name: Arc<str>,
    pub entry: Entry,
}

/// A bounded store of console lines.
#[derive(Clone, Debug, Default)]
pub struct Console {
    lines: VecDeque<ConsoleLine>,
    bytes: usize,
    dropped: u64,
    truncated: u64,
    /// The number of the first line kept: every line the console was given has its own number,
    /// counted from 0, which dropping and clearing do not change.
    first: u64,
}

impl Console {
    /// The lines kept, oldest first.
    pub fn lines(&self) -> &VecDeque<ConsoleLine> {
        &self.lines
    }

    /// The number of [`Console::lines`]' first line; each line after it has the next number.
    pub fn first_number(&self) -> u64 {
        self.first
    }

    /// The lines dropped to make room since the console was last cleared.
    pub fn dropped_lines(&self) -> u64 {
        self.dropped
    }

    /// The lines cut to [`MAX_LINE_BYTES`] since the console was last cleared.
    pub fn truncated_lines(&self) -> u64 {
        self.truncated
    }

    /// Removes every line and sets the counts to zero.
    pub fn clear(&mut self) {
        *self = Self { first: self.first + self.lines.len() as u64, ..Self::default() };
    }

    /// Adds a line, dropping the oldest ones past the limits. `cut`: the line's text was cut.
    pub(crate) fn push(&mut self, line: ConsoleLine, cut: bool) {
        if cut {
            self.truncated = self.truncated.saturating_add(1);
        }
        let bytes = line.entry.bytes();
        while !self.lines.is_empty() && (self.lines.len() >= MAX_CONSOLE_LINES || self.bytes + bytes > MAX_CONSOLE_BYTES) {
            if let Some(old) = self.lines.pop_front() {
                self.bytes -= old.entry.bytes();
                self.dropped = self.dropped.saturating_add(1);
                self.first += 1;
            }
        }
        self.bytes += bytes;
        self.lines.push_back(line);
    }

    /// `{lines: [text…], droppedLines, truncatedLines}`, as a run report gives its console.
    pub(crate) fn report(&self) -> Value {
        let lines: Vec<String> = self.lines.iter().map(|l| l.entry.text()).collect();
        json!({"lines": lines, "droppedLines": self.dropped, "truncatedLines": self.truncated})
    }

    /// `{lines: [{run, name, kind, text}…], droppedLines, truncatedLines}`, as `script.console`
    /// gives it.
    pub fn to_json(&self) -> Value {
        let lines: Vec<Value> =
            self.lines.iter().map(|l| json!({"run": l.run, "name": &*l.name, "kind": l.entry.kind(), "text": l.entry.text()})).collect();
        json!({"lines": lines, "droppedLines": self.dropped, "truncatedLines": self.truncated})
    }
}

/// `c` as the console shows it: a control character as its Control Picture, a line or paragraph
/// separator as U+2424, anything else as it is.
fn shown(c: char) -> char {
    match c {
        '\u{0}'..='\u{1f}' => char::from_u32(0x2400 + c as u32).unwrap_or('\u{fffd}'),
        '\u{7f}' => '\u{2421}',
        '\u{80}'..='\u{9f}' => '\u{fffd}',
        '\u{2028}' | '\u{2029}' => '\u{2424}',
        c => c,
    }
}

/// Adds `text` to `line` as the console shows it, up to [`MAX_LINE_BYTES`]. → `true` when part of
/// it did not fit.
fn append(line: &mut String, text: &str) -> bool {
    for c in text.chars().map(shown) {
        if line.len() + c.len_utf8() > MAX_LINE_BYTES {
            return true;
        }
        line.push(c);
    }
    false
}

/// A script's name as console lines give it: shown as a line shows its text, and cut to
/// [`MAX_NAME_BYTES`] with [`CUT_MARK`].
pub(crate) fn line_name(name: &str) -> Arc<str> {
    let mut out = String::new();
    for c in name.chars().map(shown) {
        if out.len() + c.len_utf8() > MAX_NAME_BYTES {
            mark_cut(&mut out, MAX_NAME_BYTES);
            break;
        }
        out.push(c);
    }
    Arc::from(out)
}

/// Ends a line that was cut with [`CUT_MARK`], within `limit` bytes.
fn mark_cut(line: &mut String, limit: usize) {
    let mut at = line.len().min(limit - CUT_MARK.len_utf8());
    while !line.is_char_boundary(at) {
        at -= 1;
    }
    line.truncate(at);
    line.push(CUT_MARK);
}

/// What `$.write` has written of the line it has not ended yet.
#[derive(Debug, Default)]
pub(crate) struct Pending {
    text: String,
    /// Part of the line did not fit; the rest is left out until the line ends.
    cut: bool,
    /// The last text written ended with a carriage return, so a line feed next ends no line.
    after_cr: bool,
}

impl Pending {
    /// Writes `text`, calling `done` with each line it ends and whether that line was cut.
    pub(crate) fn write(&mut self, text: &str, mut done: impl FnMut(String, bool)) {
        if text.is_empty() {
            return;
        }
        let mut rest = text;
        if self.after_cr && rest.starts_with('\n') {
            rest = &rest[1..];
        }
        self.after_cr = false;
        while let Some(at) = rest.find(['\r', '\n']) {
            self.add(&rest[..at]);
            done(self.take_line(), self.cut);
            self.cut = false;
            let skip = if rest[at..].starts_with("\r\n") { 2 } else { 1 };
            if skip == 1 && rest[at..].starts_with('\r') && at + 1 == rest.len() {
                self.after_cr = true;
            }
            rest = &rest[at + skip..];
        }
        self.add(rest);
    }

    /// Ends the current line, empty or not.
    pub(crate) fn end_line(&mut self, mut done: impl FnMut(String, bool)) {
        let cut = self.cut;
        done(self.take_line(), cut);
        self.cut = false;
        self.after_cr = false;
    }

    /// Is part of a line waiting for its end?
    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty() && !self.cut
    }

    fn add(&mut self, text: &str) {
        if !self.cut && !text.is_empty() {
            self.cut = append(&mut self.text, text);
        }
    }

    fn take_line(&mut self) -> String {
        if self.cut {
            mark_cut(&mut self.text, MAX_LINE_BYTES);
        }
        std::mem::take(&mut self.text)
    }
}
