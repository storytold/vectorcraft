//! `app.toast`: a short message shown over the window that does not stop work and goes away by
//! itself (a toast). Scripts (`vc.run`), actions (a recorded step), MCP clients and plug-ins raise
//! one with the command, and the app's code can raise one with [`Session::toast`]. The engine
//! queues the messages and the desktop app takes them each frame ([`Session::take_toasts`]) and
//! shows them. A failed script run puts the documents back but keeps the messages it raised.

use std::collections::VecDeque;

use serde_json::{Value, json};

use super::{CommandSpec, Result, always, bad, cmd};
use crate::Session;

const ID: &str = "app.toast";

/// The most characters a message has.
pub const MAX_CHARS: usize = 500;

/// How long a message shows when `seconds` is omitted, and the shortest and longest time it may
/// show.
pub const DEFAULT_SECONDS: f64 = 4.0;
pub const MIN_SECONDS: f64 = 0.5;
pub const MAX_SECONDS: f64 = 60.0;

/// The most messages waiting for the app to take them: past it the oldest one goes. A session
/// without a window (the headless MCP server, `vectorcraft-cli run`) never takes them.
pub const MAX_QUEUED: usize = 32;

/// What kind of news a message is, which sets its color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Level {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

impl Level {
    pub const ALL: [Level; 4] = [Level::Info, Level::Success, Level::Warning, Level::Error];

    /// Its `level` value.
    pub fn param(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Success => "success",
            Level::Warning => "warning",
            Level::Error => "error",
        }
    }

    /// The level whose `level` value is `param`.
    pub fn from_param(param: &str) -> Option<Level> {
        Level::ALL.into_iter().find(|l| l.param() == param)
    }
}

/// A message to show.
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub message: String,
    pub level: Level,
    /// How long it shows, in seconds.
    pub seconds: f64,
}

impl Toast {
    /// A message of `level` shown for [`DEFAULT_SECONDS`].
    pub fn new(message: impl Into<String>, level: Level) -> Toast {
        Toast { message: message.into(), level, seconds: DEFAULT_SECONDS }
    }
}

/// The messages waiting for the app to show them.
#[derive(Clone, Debug, Default)]
pub struct Queue(VecDeque<Toast>);

impl Session {
    /// Queue `toast` for the app to show. With [`MAX_QUEUED`] waiting, the oldest one goes. Its
    /// time is held to [`MIN_SECONDS`] to [`MAX_SECONDS`], and a time that is not a number is
    /// [`DEFAULT_SECONDS`].
    pub fn toast(&mut self, mut toast: Toast) {
        toast.seconds = if toast.seconds.is_nan() { DEFAULT_SECONDS } else { toast.seconds.clamp(MIN_SECONDS, MAX_SECONDS) };
        while self.toasts.0.len() >= MAX_QUEUED {
            self.toasts.0.pop_front();
        }
        self.toasts.0.push_back(toast);
    }

    /// Take the messages queued since the last call, oldest first.
    pub fn take_toasts(&mut self) -> Vec<Toast> {
        self.toasts.0.drain(..).collect()
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "app.toast",
        "Show Message",
        [],
        None,
        "{message: string (1 to 500 characters, not only whitespace), level?: \"info\" | \"success\" | \"warning\" | \"error\" (default \"info\"), seconds?: number 0.5 to 60 (default 4)} show `message` over the window without stopping work: it goes away after `seconds` or when clicked; messages stack at the window's bottom right, the newest lowest, at most 5 at once; edits no document; an action records it as a step → {message, level, seconds}",
        always,
        run
    )]
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let toast = parse(p)?;
    let r = json!({"message": &toast.message, "level": toast.level.param(), "seconds": toast.seconds});
    s.toast(toast);
    Ok(r)
}

/// The message `p` asks for; an invalid value of any key is an error.
fn parse(p: &Value) -> Result<Toast> {
    let message = match p.get("message") {
        Some(Value::String(m)) => m,
        Some(_) => return Err(bad(ID, "`message` must be a string")),
        None => return Err(bad(ID, "`message` is required")),
    };
    if message.trim().is_empty() {
        return Err(bad(ID, "`message` must have a character other than whitespace"));
    }
    let chars = message.chars().count();
    if chars > MAX_CHARS {
        return Err(bad(ID, format!("`message` has {chars} characters; at most {MAX_CHARS} are allowed")));
    }
    let level = match p.get("level") {
        None => Level::default(),
        Some(v) => v.as_str().and_then(Level::from_param).ok_or_else(|| {
            let all: Vec<&str> = Level::ALL.iter().map(|l| l.param()).collect();
            bad(ID, format!("`level` must be one of {}", all.join(", ")))
        })?,
    };
    let seconds = match p.get("seconds") {
        None => DEFAULT_SECONDS,
        Some(v) => v
            .as_f64()
            .filter(|s| (MIN_SECONDS..=MAX_SECONDS).contains(s))
            .ok_or_else(|| bad(ID, format!("`seconds` must be a number from {MIN_SECONDS} to {MAX_SECONDS}")))?,
    };
    Ok(Toast { message: message.clone(), level, seconds })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_queued_with_its_defaults() {
        let mut s = Session::new();
        let r = s.execute(ID, &json!({"message": "Saved 3 files"})).unwrap();
        assert_eq!(r, json!({"message": "Saved 3 files", "level": "info", "seconds": 4.0}));
        assert_eq!(s.take_toasts(), vec![Toast::new("Saved 3 files", Level::Info)]);
        // Taken once.
        assert_eq!(s.take_toasts(), vec![]);
    }

    #[test]
    fn level_and_seconds_are_kept() {
        let mut s = Session::new();
        for level in Level::ALL {
            s.execute(ID, &json!({"message": "m", "level": level.param(), "seconds": 0.5})).unwrap();
        }
        s.execute(ID, &json!({"message": "long", "seconds": 60})).unwrap();
        let got: Vec<(Level, f64)> = s.take_toasts().iter().map(|t| (t.level, t.seconds)).collect();
        assert_eq!(got, vec![(Level::Info, 0.5), (Level::Success, 0.5), (Level::Warning, 0.5), (Level::Error, 0.5), (Level::Info, 60.0)]);
    }

    #[test]
    fn invalid_values_are_errors_and_queue_nothing() {
        let mut s = Session::new();
        let long = "x".repeat(MAX_CHARS + 1);
        for p in [
            json!({}),
            json!({"message": 3}),
            json!({"message": ""}),
            json!({"message": " \n\t"}),
            json!({"message": long}),
            json!({"message": "m", "level": "loud"}),
            json!({"message": "m", "level": 1}),
            json!({"message": "m", "seconds": 0.4}),
            json!({"message": "m", "seconds": 61}),
            json!({"message": "m", "seconds": "4"}),
            json!({"message": "m", "seconds": null}),
        ] {
            let e = s.execute(ID, &p).unwrap_err().to_string();
            assert!(e.contains(ID), "{p}: {e}");
        }
        assert_eq!(s.take_toasts(), vec![]);
        // The longest message is allowed, counted in characters, not bytes.
        let most = "é".repeat(MAX_CHARS);
        s.execute(ID, &json!({"message": most})).unwrap();
        assert_eq!(s.take_toasts().len(), 1);
    }

    #[test]
    fn a_message_queued_by_the_app_shows_for_0_5_to_60_seconds() {
        let mut s = Session::new();
        for seconds in [f64::INFINITY, 1e9, f64::NEG_INFINITY, 0.0, f64::NAN, 10.0] {
            s.toast(Toast { seconds, ..Toast::new("m", Level::Info) });
        }
        let got: Vec<f64> = s.take_toasts().iter().map(|t| t.seconds).collect();
        assert_eq!(got, vec![MAX_SECONDS, MAX_SECONDS, MIN_SECONDS, MIN_SECONDS, DEFAULT_SECONDS, 10.0]);
    }

    #[test]
    fn the_oldest_waiting_message_goes_past_the_limit() {
        let mut s = Session::new();
        for i in 0..MAX_QUEUED + 2 {
            s.toast(Toast::new(format!("{i}"), Level::Info));
        }
        let left: Vec<String> = s.take_toasts().into_iter().map(|t| t.message).collect();
        assert_eq!(left.len(), MAX_QUEUED);
        assert_eq!(left.first().map(String::as_str), Some("2"));
        assert_eq!(left.last().map(String::as_str), Some("33"));
    }

    #[test]
    fn a_message_is_journaled_without_an_undo_step_or_a_document() {
        let mut s = Session::new();
        // No document open.
        s.execute(ID, &json!({"message": "no document"})).unwrap();
        assert_eq!(s.journal.last().unwrap(), &(ID.to_string(), json!({"message": "no document"})));
        s.execute("file.new", &json!({})).unwrap();
        let undo = s.active().unwrap().history.undo.len();
        let revision = s.active().unwrap().revision;
        s.execute(ID, &json!({"message": "m", "level": "success"})).unwrap();
        assert_eq!(s.journal.last().unwrap(), &(ID.to_string(), json!({"message": "m", "level": "success"})));
        assert_eq!(s.active().unwrap().history.undo.len(), undo);
        assert_eq!(s.active().unwrap().revision, revision);
    }

    #[test]
    fn a_script_raises_a_message_with_vc_run() {
        let mut s = Session::new();
        let r = s
            .execute(
                "script.run",
                &json!({"source": "vc.run('app.toast', {message: 'Done: ' + (1 + 1), level: 'success'});", "name": "t.js", "answers": []}),
            )
            .unwrap();
        assert!(r.get("error").is_none_or(Value::is_null), "{r}");
        assert_eq!(s.take_toasts(), vec![Toast::new("Done: 2", Level::Success)]);
    }

    #[test]
    fn a_failed_script_run_puts_the_documents_back_and_keeps_its_messages() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let art = format!("{:?}", s.active().unwrap().doc.layers);
        let e = s
            .execute(
                "script.run",
                &json!({
                    "source": "vc.run('shape.rectangle', {x: 0, y: 0, width: 10, height: 10}); vc.run('app.toast', {message: 'Failed: no artboard'}); throw new Error('stop');",
                    "name": "t.js",
                    "answers": [],
                }),
            )
            .unwrap_err()
            .to_string();
        assert!(e.contains("stop"), "{e}");
        assert_eq!(format!("{:?}", s.active().unwrap().doc.layers), art, "the rectangle is put back");
        assert_eq!(s.take_toasts(), vec![Toast::new("Failed: no artboard", Level::Info)]);
    }

    #[test]
    fn a_command_batch_raises_its_messages_in_order() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s.execute(
            "command.batch",
            &json!({"commands": [
                {"command": "app.toast", "params": {"message": "one"}},
                {"command": "app.toast", "params": {"message": "two", "level": "warning"}},
            ]}),
        )
        .unwrap();
        assert_eq!(s.take_toasts(), vec![Toast::new("one", Level::Info), Toast::new("two", Level::Warning)]);
    }
}
