//! `app.preferences`, as the help page <https://ai-scripting.docsforadobe.dev/jsobjref/Preferences/>
//! describes it: values a script reads and stores by key with a getter and a setter for each type.
//!
//! `ShowExternalJSXWarning` is the preference `showExternalScriptWarning`, the warning before a
//! script picked with File › Scripts › Other Script… runs. A script turns it on at once; turning it
//! off waits for the person to answer Yes to a confirm, and throws when the run cannot ask anyone
//! (VectorCraft's own rule). Every other key is kept in
//! [`crate::Prefs::script_preferences`], with the preferences, so it lasts across runs and restarts
//! and a failed or stopped run puts it back as it was.
//!
//! VectorCraft's own rules, which the help page does not state:
//! - A key that was never stored reads as `false`, `0`, `0` or `""`.
//! - A getter of another type converts the stored value as `Boolean`, `Number` or `String` do;
//!   `getIntegerPreference` drops the fraction, and gives `0` for a value that is not a number.
//! - A key is a string of 1 to [`MAX_KEY_UNITS`] UTF-16 units without control characters, a string
//!   value has at most [`MAX_STRING_BYTES`] bytes, and at most [`MAX_KEYS`] keys and
//!   [`MAX_TOTAL_BYTES`] bytes are stored. A set that would pass a limit throws and stores nothing.
//! - A setter's value must have its type: `setIntegerPreference` takes a whole number within
//!   ±2^53, and `setRealPreference` a finite number.
//! - Removing a key that is not stored does nothing; removing `ShowExternalJSXWarning` turns the
//!   warning back on.

use vectorcraft_js::{Data, HostClass, HostError, HostMember, host_ref, host_ref_parts};

use super::{Answer, Described, Host, Link, arg, method, named, num, returns, text, typed, typename_of};
use crate::ScriptPreference;
use crate::script::dialogs::{DialogKind, OnYes, ScriptDialog};

/// The key that stands for the preference `showExternalScriptWarning`.
pub const EXTERNAL_SCRIPT_WARNING: &str = "ShowExternalJSXWarning";
/// The longest key, in UTF-16 units.
pub const MAX_KEY_UNITS: usize = 256;
/// The longest string value, in bytes.
pub const MAX_STRING_BYTES: usize = 64 * 1024;
/// The most keys stored.
pub const MAX_KEYS: usize = 4096;
/// The most bytes stored, keys and values together; a value that is not a string counts 8.
pub const MAX_TOTAL_BYTES: usize = 1024 * 1024;

/// The confirm a run shows before a script turns off [`EXTERNAL_SCRIPT_WARNING`]; No is its default.
pub const WARNING_OFF_QUESTION: &str = "The script asks to turn off the warning shown before a script picked with File › Scripts › Other Script… runs. Scripts picked later would then run without it. Turn the warning off?";
/// The error when a script turns off [`EXTERNAL_SCRIPT_WARNING`] in a run that cannot ask the person.
pub const WARNING_OFF_NEEDS_A_PERSON: &str =
    "ShowExternalJSXWarning can be turned off only after the person at the computer agrees, in a run that can show a dialog";

/// The largest whole number `setIntegerPreference` takes.
const MAX_INTEGER: f64 = 9_007_199_254_740_992.0;

/// The `app.preferences` member of `Application`.
pub(super) fn app_members() -> Vec<Described> {
    named(vec![("preferences", HostMember::Get(std::rc::Rc::new(|_| Ok(host_ref("Preferences", &[])))), typed("Preferences"))])
}

/// The `Preferences` class.
pub(super) fn class(link: &Link) -> HostClass {
    let l = link;
    let key = [("key", "string")];
    super::class(
        "Preferences",
        None,
        named(vec![
            ("typename", typename_of("Preferences"), typed("string")),
            ("getBooleanPreference", method(l, |h, a| get(h, a, Kind::Boolean)), returns("boolean", &key)),
            ("getIntegerPreference", method(l, |h, a| get(h, a, Kind::Integer)), returns("number", &key)),
            ("getRealPreference", method(l, |h, a| get(h, a, Kind::Real)), returns("number", &key)),
            ("getStringPreference", method(l, |h, a| get(h, a, Kind::String)), returns("string", &key)),
            ("setBooleanPreference", method(l, |h, a| set(h, a, Kind::Boolean)), returns("undefined", &[("key", "string"), ("value", "boolean")])),
            ("setIntegerPreference", method(l, |h, a| set(h, a, Kind::Integer)), returns("undefined", &[("key", "string"), ("value", "number")])),
            ("setRealPreference", method(l, |h, a| set(h, a, Kind::Real)), returns("undefined", &[("key", "string"), ("value", "number")])),
            ("setStringPreference", method(l, |h, a| set(h, a, Kind::String)), returns("undefined", &[("key", "string"), ("value", "string")])),
            ("removePreference", method(l, remove), returns("undefined", &key)),
        ]),
        None,
    )
}

#[derive(Clone, Copy)]
enum Kind {
    Boolean,
    Integer,
    Real,
    String,
}

/// The key of a call on `app.preferences`: `this` must be that object, and the key a string within
/// the limits.
fn key_of(a: &[Data]) -> std::result::Result<&str, HostError> {
    if !matches!(host_ref_parts(arg(a, 0)), Some(("Preferences", _))) {
        return Err(HostError::Type("this is not a Preferences object".into()));
    }
    let Data::String(key) = arg(a, 1) else { return Err(HostError::Type("a preference's key must be a string".into())) };
    if key.is_empty() || key.encode_utf16().count() > MAX_KEY_UNITS {
        return Err(HostError::Range(format!(
            "a preference's key must have a length of 1 to {MAX_KEY_UNITS} (UTF-16 code units, as `length` counts)"
        )));
    }
    if key.chars().any(char::is_control) {
        return Err(HostError::Error("a preference's key cannot hold control characters".into()));
    }
    Ok(key)
}

/// The value stored under `key`, if any.
fn stored(h: &Host, key: &str) -> Option<ScriptPreference> {
    let s = h.session.borrow();
    if key == EXTERNAL_SCRIPT_WARNING {
        return Some(ScriptPreference::Boolean(s.prefs.show_external_script_warning));
    }
    s.prefs.script_preferences.get(key).cloned()
}

/// `Number(text)`: blank text is 0, `0x`, `0o` and `0b` start a whole number in base 16, 8 or 2,
/// and text that is not a number is NaN.
fn text_number(t: &str) -> f64 {
    let t = t.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if t.is_empty() {
        return 0.0;
    }
    let radix = match t.get(..2) {
        Some("0x" | "0X") => 16,
        Some("0o" | "0O") => 8,
        Some("0b" | "0B") => 2,
        _ => 0,
    };
    if radix != 0 {
        let digits = t.get(2..).unwrap_or("");
        let whole = digits.chars().try_fold(0.0_f64, |n, c| c.to_digit(radix).map(|d| n * f64::from(radix) + f64::from(d)));
        return whole.filter(|_| !digits.is_empty()).unwrap_or(f64::NAN);
    }
    match t.strip_prefix(['+', '-']).unwrap_or(t) {
        "Infinity" => {
            if t.starts_with('-') {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }
        }
        rest if rest.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-')) => t.parse().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

#[allow(clippy::cast_precision_loss)]
fn as_number(v: &ScriptPreference) -> f64 {
    match v {
        ScriptPreference::Boolean(b) => f64::from(u8::from(*b)),
        ScriptPreference::Integer(n) => *n as f64,
        ScriptPreference::Real(n) => *n,
        ScriptPreference::String(t) => text_number(t),
    }
}

fn get(h: &Host, a: &[Data], kind: Kind) -> Answer {
    let key = key_of(a)?;
    let v = stored(h, key);
    Ok(match (kind, v) {
        (Kind::Boolean, None) => Data::Bool(false),
        (Kind::Integer | Kind::Real, None) => num(0.0),
        (Kind::String, None) => text(""),
        (Kind::Boolean, Some(v)) => Data::Bool(match v {
            ScriptPreference::Boolean(b) => b,
            ScriptPreference::String(t) => !t.is_empty(),
            v => {
                let n = as_number(&v);
                n != 0.0 && !n.is_nan()
            }
        }),
        (Kind::Real, Some(v)) => num(as_number(&v)),
        (Kind::Integer, Some(v)) => {
            let n = as_number(&v).trunc();
            num(if n.is_finite() { n } else { 0.0 })
        }
        (Kind::String, Some(v)) => text(match v {
            ScriptPreference::Boolean(b) => b.to_string(),
            ScriptPreference::String(t) => t,
            v => vectorcraft_js::number_to_string(as_number(&v)),
        }),
    })
}

/// The bytes `key` and `v` count toward [`MAX_TOTAL_BYTES`].
fn size(key: &str, v: &ScriptPreference) -> usize {
    key.len() + if let ScriptPreference::String(t) = v { t.len() } else { 8 }
}

/// Whether `store` holds at most [`MAX_KEYS`] keys and [`MAX_TOTAL_BYTES`] bytes.
pub fn within_limits(store: &std::collections::BTreeMap<String, ScriptPreference>) -> bool {
    store.len() <= MAX_KEYS && store.iter().map(|(k, v)| size(k, v)).sum::<usize>() <= MAX_TOTAL_BYTES
}

#[allow(clippy::cast_possible_truncation)]
fn set(h: &Host, a: &[Data], kind: Kind) -> Answer {
    let key = key_of(a)?.to_string();
    let v = arg(a, 2);
    let value = match (kind, v) {
        (Kind::Boolean, Data::Bool(b)) => ScriptPreference::Boolean(*b),
        (Kind::Integer, Data::Number(n)) if n.fract() == 0.0 && n.abs() <= MAX_INTEGER => ScriptPreference::Integer(*n as i64),
        (Kind::Real, Data::Number(n)) if n.is_finite() => ScriptPreference::Real(*n),
        (Kind::String, Data::String(t)) if t.len() <= MAX_STRING_BYTES => ScriptPreference::String(t.clone()),
        (Kind::String, Data::String(_)) => return Err(HostError::Range(format!("a preference's text has at most {MAX_STRING_BYTES} bytes"))),
        (Kind::Boolean, _) => return Err(HostError::Type("setBooleanPreference needs true or false".into())),
        (Kind::Integer, _) => return Err(HostError::Type("setIntegerPreference needs a whole number".into())),
        (Kind::Real, _) => return Err(HostError::Type("setRealPreference needs a finite number".into())),
        (Kind::String, _) => return Err(HostError::Type("setStringPreference needs a string".into())),
    };
    if key == EXTERNAL_SCRIPT_WARNING {
        let ScriptPreference::Boolean(on) = value else {
            return Err(HostError::Type(format!("{EXTERNAL_SCRIPT_WARNING} is set with setBooleanPreference")));
        };
        return set_warning(h, on);
    }
    let mut s = h.session.borrow_mut();
    let store = &mut s.prefs.script_preferences;
    if store.get(&key) == Some(&value) {
        return Ok(Data::Undefined);
    }
    let total: usize = store.iter().filter(|(k, _)| **k != key).map(|(k, v)| size(k, v)).sum();
    let count = store.len() + usize::from(!store.contains_key(&key));
    if count > MAX_KEYS {
        return Err(HostError::Range(format!("app.preferences holds at most {MAX_KEYS} keys")));
    }
    if total + size(&key, &value) > MAX_TOTAL_BYTES {
        return Err(HostError::Range(format!("app.preferences holds at most {MAX_TOTAL_BYTES} bytes")));
    }
    store.insert(key, value);
    Ok(Data::Undefined)
}

/// Sets `showExternalScriptWarning` in the run's preferences: on at once, and off only after the
/// person answers Yes to [`WARNING_OFF_QUESTION`] ([`crate::Session::script_answer`] sets it). With
/// nobody to ask, turning it off throws and changes nothing. Scripts cannot run `prefs.set`; the
/// run's preferences are put back when the run fails and merged into the app's when it ends.
fn set_warning(h: &Host, on: bool) -> Answer {
    if on || !h.session.borrow().prefs.show_external_script_warning {
        h.session.borrow_mut().prefs.show_external_script_warning = on;
        return Ok(Data::Undefined);
    }
    let question = ScriptDialog {
        kind: DialogKind::Confirm,
        message: WARNING_OFF_QUESTION.into(),
        title: None,
        error_icon: false,
        no_as_default: true,
        preset: String::new(),
    };
    if !h.dialogs.ask(question, OnYes::ExternalScriptWarningOff) {
        return Err(HostError::Error(WARNING_OFF_NEEDS_A_PERSON.into()));
    }
    Err(HostError::Wait)
}

fn remove(h: &Host, a: &[Data]) -> Answer {
    let key = key_of(a)?;
    if key == EXTERNAL_SCRIPT_WARNING {
        h.session.borrow_mut().prefs.show_external_script_warning = true;
    } else if h.session.borrow().prefs.script_preferences.contains_key(key) {
        h.session.borrow_mut().prefs.script_preferences.remove(key);
    }
    Ok(Data::Undefined)
}
