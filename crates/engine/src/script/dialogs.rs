//! `alert(message[, title, errorIcon])`, `confirm(message[, noAsDflt, title])` and
//! `prompt(message, preset[, title])`: the dialogs a script opens while it runs.
//!
//! A message and a preset arrive as the string `String(value)` gives, a title the same way unless
//! it is left out or `undefined` ([`DEFAULT_TITLE`]), and `errorIcon` and `noAsDflt` as the
//! boolean `Boolean(value)` gives. `alert` returns `undefined`, `confirm` `true` for Yes and
//! `false` for No, and `prompt` the text in its field for OK and `null` for Cancel.
//!
//! In the app the run waits at the call for the person's answer, and spends no fuel while it
//! waits; every dialog also has Stop, which ends the run and puts everything back. A run with
//! nobody to ask takes its answers, in order, from `script.run`'s `answers`; a call with no
//! answer left, or with the next answer for another kind of dialog, ends the run. The report's
//! `dialogs` lists each dialog and its answer.
//!
//! The values here are VectorCraft's own limits: [`MAX_MESSAGE_BYTES`], [`MAX_TITLE_BYTES`],
//! [`MAX_PROMPT_BYTES`], [`MAX_ANSWERS`], [`MAX_ANSWERS_BYTES`], [`MAX_RECORDS`] and
//! [`MAX_RECORD_BYTES`].

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use serde_json::{Map, Value, json};
use vectorcraft_js::{Arg, Data, HostError, HostFunction, Realm, RunError};

use super::{Host, console};

/// The most bytes (UTF-8) of a dialog's message.
pub const MAX_MESSAGE_BYTES: usize = 16 << 10;
/// The most bytes (UTF-8) of a dialog's title.
pub const MAX_TITLE_BYTES: usize = 256;
/// The most bytes (UTF-8) of a prompt's preset and of the text it returns.
pub const MAX_PROMPT_BYTES: usize = 16 << 10;
/// The most answers `script.run` takes.
pub const MAX_ANSWERS: usize = 10_000;
/// The most bytes of `answers`, as JSON.
pub const MAX_ANSWERS_BYTES: usize = 1 << 20;
/// The most dialogs a report lists.
pub const MAX_RECORDS: usize = 10_000;
/// The most bytes of the dialogs a report lists, as JSON.
pub const MAX_RECORD_BYTES: usize = 1 << 20;
/// A dialog's title when the script gives none, in English; the app translates it when it shows it.
pub const DEFAULT_TITLE: &str = "Script Alert";

/// Which dialog a script opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
}

impl DialogKind {
    /// The function's name, which is also the `kind` of an answer and of a report's dialog.
    pub fn id(self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::Confirm => "confirm",
            Self::Prompt => "prompt",
        }
    }
}

/// A dialog a script opened, which waits for its answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptDialog {
    pub kind: DialogKind,
    pub message: String,
    /// `None`: the script gave no title ([`DEFAULT_TITLE`]).
    pub title: Option<String>,
    /// `alert`: show the error icon.
    pub error_icon: bool,
    /// `confirm`: No is the default button.
    pub no_as_default: bool,
    /// `prompt`: the text its field starts with.
    pub preset: String,
}

/// The answer to a [`ScriptDialog`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogAnswer {
    /// OK, for `alert`.
    Ok,
    /// Yes (`true`) or No, for `confirm`.
    Confirm(bool),
    /// OK with the field's text, or Cancel (`None`), for `prompt`.
    Prompt(Option<String>),
}

impl DialogAnswer {
    pub fn kind(&self) -> DialogKind {
        match self {
            Self::Ok => DialogKind::Alert,
            Self::Confirm(_) => DialogKind::Confirm,
            Self::Prompt(_) => DialogKind::Prompt,
        }
    }

    /// What the function returns.
    fn data(&self) -> Data {
        match self {
            Self::Ok => Data::Undefined,
            Self::Confirm(b) => Data::Bool(*b),
            Self::Prompt(Some(t)) => Data::String(t.clone()),
            Self::Prompt(None) => Data::Null,
        }
    }

    /// The answer as a report gives it: `"ok"`, a boolean, a string or `null`.
    fn json(&self) -> Value {
        match self {
            Self::Ok => json!("ok"),
            Self::Confirm(b) => json!(b),
            Self::Prompt(t) => json!(t),
        }
    }
}

/// `text` as a dialog shows it. With `lines`, a line feed, a carriage return or the two together
/// is a line break; every other control character, and with `lines` false every line break too,
/// shows as its Unicode Control Picture, as the Script Console shows it.
pub fn shown_text(text: &str, lines: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' if lines => {
                chars.next_if_eq(&'\n');
                out.push('\n');
            }
            '\n' if lines => out.push('\n'),
            c => out.push(console::shown(c)),
        }
    }
    out
}

/// The longest start of `text` that is at most `max` bytes, cut at a character boundary.
pub fn cut_to(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

/// `script.run`'s `answers`: a list of `{kind: "alert", value: "ok"}`, `{kind: "confirm", value:
/// true | false}` and `{kind: "prompt", value: text | null}`. An empty object, which the parameter
/// took before dialogs existed, is an empty list.
pub(crate) fn parse_answers(v: Option<&Value>) -> Result<Vec<DialogAnswer>, String> {
    let list = match v {
        None => return Ok(Vec::new()),
        Some(Value::Object(o)) if o.is_empty() => return Ok(Vec::new()),
        Some(Value::Array(a)) => a,
        Some(_) => return Err("`answers` must be a list of answers".into()),
    };
    if list.len() > MAX_ANSWERS {
        return Err(format!("`answers` has more than {MAX_ANSWERS} answers"));
    }
    if serde_json::to_vec(list).map_or(true, |b| b.len() > MAX_ANSWERS_BYTES) {
        return Err("`answers` is larger than 1 MiB".into());
    }
    list.iter().enumerate().map(|(i, a)| answer(a).map_err(|e| format!("`answers[{i}]` {e}"))).collect()
}

fn answer(a: &Value) -> Result<DialogAnswer, &'static str> {
    let Some(o) = a.as_object() else { return Err("must be an object with `kind` and `value`") };
    if o.len() != 2 || !o.contains_key("value") {
        return Err("must have exactly `kind` and `value`");
    }
    match (o.get("kind").and_then(Value::as_str), &o["value"]) {
        (Some("alert"), Value::String(s)) if s == "ok" => Ok(DialogAnswer::Ok),
        (Some("alert"), _) => Err("is an alert's answer, whose `value` must be \"ok\""),
        (Some("confirm"), Value::Bool(b)) => Ok(DialogAnswer::Confirm(*b)),
        (Some("confirm"), _) => Err("is a confirm's answer, whose `value` must be true or false"),
        (Some("prompt"), Value::Null) => Ok(DialogAnswer::Prompt(None)),
        (Some("prompt"), Value::String(s)) if s.len() <= MAX_PROMPT_BYTES => Ok(DialogAnswer::Prompt(Some(s.clone()))),
        (Some("prompt"), Value::String(_)) => Err("is longer than 16 KiB"),
        (Some("prompt"), _) => Err("is a prompt's answer, whose `value` must be a string or null"),
        _ => Err("must have `kind` alert, confirm or prompt"),
    }
}

/// The dialogs of one run: the answers left for a run with nobody to ask, the dialog that waits
/// for the person's answer, and the list the report gives.
pub(crate) struct Dialogs {
    /// Answers come from the person, not from `answers`.
    framed: bool,
    /// The realm's [`Realm::wait_flag`].
    can_wait: Rc<Cell<bool>>,
    answers: RefCell<VecDeque<DialogAnswer>>,
    waiting: RefCell<Option<ScriptDialog>>,
    /// The error that ended the run when an answer could not reach the script.
    failed: RefCell<Option<RunError>>,
    records: RefCell<Vec<Value>>,
    record_bytes: Cell<usize>,
    count: Cell<u64>,
    truncated: Cell<bool>,
}

impl Dialogs {
    pub(crate) fn new(framed: bool, realm: &Realm, answers: Vec<DialogAnswer>) -> Self {
        Self {
            framed,
            can_wait: realm.wait_flag(),
            answers: RefCell::new(answers.into()),
            waiting: RefCell::default(),
            failed: RefCell::default(),
            records: RefCell::default(),
            record_bytes: Cell::new(0),
            count: Cell::new(0),
            truncated: Cell::new(false),
        }
    }

    /// The dialog that waits for its answer.
    pub(crate) fn waiting(&self) -> Option<ScriptDialog> {
        self.waiting.borrow().clone()
    }

    pub(crate) fn is_waiting(&self) -> bool {
        self.waiting.borrow().is_some()
    }

    /// The error that ended the run when its answer could not reach the script.
    pub(crate) fn take_failure(&self) -> Option<RunError> {
        self.failed.borrow_mut().take()
    }

    /// How many of `answers` the run did not use.
    pub(crate) fn unused(&self) -> usize {
        self.answers.borrow().len()
    }

    /// Gives the waiting dialog `answer`, which must be for its kind and, for a prompt, at most
    /// [`MAX_PROMPT_BYTES`]. → false when it is not, or nothing waits.
    pub(crate) fn answer(&self, realm: &mut Realm, answer: DialogAnswer) -> bool {
        let fits = match &answer {
            DialogAnswer::Prompt(Some(t)) => t.len() <= MAX_PROMPT_BYTES,
            _ => true,
        };
        let mut waiting = self.waiting.borrow_mut();
        let Some(d) = waiting.as_ref().filter(|d| d.kind == answer.kind() && fits) else { return false };
        self.record(d, answer.json());
        *waiting = None;
        if let Err(e) = realm.answer(answer.data()) {
            *self.failed.borrow_mut() = Some(e);
        }
        true
    }

    /// The run ends while a dialog waits: the report lists it with the answer `"stop"`.
    pub(crate) fn stopped(&self) {
        if let Some(d) = self.waiting.borrow_mut().take() {
            self.record(&d, json!("stop"));
        }
    }

    fn record(&self, d: &ScriptDialog, answer: Value) {
        self.count.set(self.count.get().saturating_add(1));
        let mut r = Map::new();
        r.insert("kind".into(), json!(d.kind.id()));
        r.insert("message".into(), json!(d.message));
        r.insert("title".into(), json!(d.title.as_deref().unwrap_or(DEFAULT_TITLE)));
        match d.kind {
            DialogKind::Alert => r.insert("errorIcon".into(), json!(d.error_icon)),
            DialogKind::Confirm => r.insert("noAsDefault".into(), json!(d.no_as_default)),
            DialogKind::Prompt => r.insert("preset".into(), json!(d.preset)),
        };
        r.insert("answer".into(), answer);
        let r = Value::Object(r);
        let bytes = serde_json::to_vec(&r).map_or(usize::MAX, |b| b.len());
        let mut records = self.records.borrow_mut();
        if self.truncated.get() || records.len() >= MAX_RECORDS || self.record_bytes.get().saturating_add(bytes) > MAX_RECORD_BYTES {
            self.truncated.set(true);
            return;
        }
        self.record_bytes.set(self.record_bytes.get() + bytes);
        records.push(r);
    }

    /// Adds `dialogs`, `dialogCount` and `dialogsTruncated` to a report.
    pub(crate) fn report(&self, report: &mut Map<String, Value>) {
        report.insert("dialogs".into(), Value::Array(self.records.borrow().clone()));
        report.insert("dialogCount".into(), json!(self.count.get()));
        report.insert("dialogsTruncated".into(), json!(self.truncated.get()));
    }
}

/// Adds `alert`, `confirm` and `prompt` to the realm.
pub(super) fn install(realm: &mut Realm, host: &Rc<Host>) -> Result<(), RunError> {
    use Arg::{Bool, Text, TextOrUndefined};
    let call = |kind: DialogKind| -> HostFunction {
        let h = host.clone();
        Rc::new(move |args| open(&h, kind, &args))
    };
    realm.define_host_function("alert", vec![Text, TextOrUndefined, Bool], call(DialogKind::Alert))?;
    realm.define_host_function("confirm", vec![Text, Bool, TextOrUndefined], call(DialogKind::Confirm))?;
    realm.define_host_function("prompt", vec![Text, Text, TextOrUndefined], call(DialogKind::Prompt))
}

fn text(args: &[Data], i: usize) -> Option<String> {
    match args.get(i) {
        Some(Data::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn flag(args: &[Data], i: usize) -> bool {
    matches!(args.get(i), Some(Data::Bool(true)))
}

/// A call of `alert`, `confirm` or `prompt`.
fn open(host: &Host, kind: DialogKind, args: &[Data]) -> Result<Data, HostError> {
    let name = kind.id();
    host.charge_call(args.iter().map(super::data_size).sum())?;
    let Some(message) = text(args, 0) else { return Err(HostError::Type(format!("{name}: give a message"))) };
    let (title, error_icon, no_as_default, preset) = match kind {
        DialogKind::Alert => (text(args, 1), flag(args, 2), false, String::new()),
        DialogKind::Confirm => (text(args, 2), false, flag(args, 1), String::new()),
        DialogKind::Prompt => {
            let Some(preset) = text(args, 1) else { return Err(HostError::Type(format!("{name}: give the text the field starts with"))) };
            (text(args, 2), false, false, preset)
        }
    };
    if message.len() > MAX_MESSAGE_BYTES {
        return Err(HostError::Range(format!("{name}: the message is longer than 16 KiB")));
    }
    if title.as_ref().is_some_and(|t| t.len() > MAX_TITLE_BYTES) {
        return Err(HostError::Range(format!("{name}: the title is longer than 256 bytes")));
    }
    if preset.len() > MAX_PROMPT_BYTES {
        return Err(HostError::Range(format!("{name}: the text the field starts with is longer than 16 KiB")));
    }
    let dialog = ScriptDialog { kind, message, title, error_icon, no_as_default, preset };
    let d = &host.dialogs;
    if d.framed {
        if !d.can_wait.get() {
            return Err(HostError::Error(format!(
                "{name}: a dialog cannot open inside a callback of a built-in function, a getter, a conversion or a file $.evalFile runs; call it from the script's own code"
            )));
        }
        *d.waiting.borrow_mut() = Some(dialog);
        return Err(HostError::Wait);
    }
    let next = d.answers.borrow_mut().pop_front();
    match next {
        Some(a) if a.kind() == kind => {
            d.record(&dialog, a.json());
            Ok(a.data())
        }
        Some(a) => {
            d.record(&dialog, json!("missing"));
            Err(HostError::Abort(format!("{name}: the next answer in `answers` is for {}, not {name}", a.kind().id())))
        }
        None => {
            d.record(&dialog, json!("missing"));
            Err(HostError::Abort(format!("{name}: `answers` has no answer left for this dialog")))
        }
    }
}
