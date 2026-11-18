//! The members of a script's `$` object besides `$.write`, `$.writeln`, `$.evalFile`, `$.fileName`
//! and `$.includePath`, from the Dollar object
//! page (<https://extendscript.docsforadobe.dev/extendscript-tools-features/dollar-object/>), and
//! the locale the global `localize` reads
//! (<https://extendscript.docsforadobe.dev/extendscript-tools-features/localizing-extendscript-strings/>).
//!
//! Every value here belongs to one run, in a named engine too ([`RunState`]): a run starts with the
//! locale of the interface, the environment values of [`ENV_NAMES`] read from the process,
//! localization off and a timer at zero, and what a script changes ends with its run. `$.setenv`
//! never changes the process's environment.
//!
//! These are VectorCraft's own values and rules: `$.appEncoding` is always UTF-8; `$.build` and
//! `$.version` give VectorCraft's version, `$.engineName` is the name `#targetengine` gave, or
//! `VectorCraft` without one, and `$.buildDate` is `null`; `$.flags` takes only 512, which turns
//! localization on; `$.level` and `$.memCache` are 0 and `$.screens` is empty; `$.os` names the system without its version; `$.strict` is whether the
//! realm reads scripts as strict mode code; `$.bp` and `$.colorPicker` throw, since scripts have no
//! debugger and cannot show a dialog; `$.gc` does nothing, since the interpreter collects its own
//! garbage; `$.getenv` and `$.setenv` know only [`ENV_NAMES`], with values of at most
//! [`MAX_ENV_BYTES`] bytes of letters, digits and `_ . - @`; and `$.sleep` waits at most
//! [`MAX_SLEEP_MS`] milliseconds, between two frames of the app, and does not wait on the web,
//! which has no clock.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use vectorcraft_js::{Data, HostError, HostItem, Localization, ScriptItem};

use super::{Link, current, function};

/// The longest `$.sleep`, in milliseconds.
pub const MAX_SLEEP_MS: f64 = 1_000.0;
/// The environment variables `$.getenv` reads and `$.setenv` sets.
pub const ENV_NAMES: &[&str] = &["LANG", "LC_MESSAGES", "LC_ALL", "OS"];
/// The most bytes of an environment value.
pub const MAX_ENV_BYTES: usize = 64;
/// The value of `$.flags` that turns localization on, the only one a script may set.
pub const LOCALIZE_FLAG: f64 = 512.0;

/// What `$` knows of its realm.
pub(crate) struct RunInfo {
    /// The realm reads scripts as strict mode code.
    pub strict: bool,
    /// The name `#targetengine` gave the realm's engine.
    pub engine: Option<String>,
}

/// The values of `$` that belong to one run, which the run's host holds.
pub(crate) struct RunState {
    /// The locale the run starts with, such as `de_DE`.
    locale: String,
    env: RefCell<Vec<(&'static str, Option<String>)>>,
    /// The time since the run started at which `$.hiresTimer` was last read.
    timer: Cell<Duration>,
}

impl RunState {
    pub(crate) fn new(locale: &str) -> RunState {
        RunState { locale: locale.to_string(), env: RefCell::new(process_env()), timer: Cell::new(Duration::ZERO) }
    }
}

/// Sets `localization` as a run starts: `locale`, this system's platform and localization off.
pub(super) fn start_run(localization: &Localization, locale: &str) {
    localization.set_enabled(false);
    localization.set_locale(locale);
    localization.set_platform(platform());
}

/// The locale for an interface language code such as `de`, `pt-br` or `zh-hant`: the language,
/// with its region after `_` where the code gives one (`pt_BR`; `zh_CN` for `zh-hans` and `zh_TW`
/// for `zh-hant`); `en` for a code that is not a language.
pub(crate) fn ui_locale(code: &str) -> String {
    let mut parts = code.split(['-', '_']);
    let language = parts.next().unwrap_or_default().to_ascii_lowercase();
    if !(2..=3).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_lowercase()) {
        return "en".into();
    }
    let region = match parts.next().map(str::to_ascii_lowercase).as_deref() {
        Some("hans") => Some("CN".to_string()),
        Some("hant") => Some("TW".to_string()),
        Some(r) if r.len() == 2 && r.bytes().all(|b| b.is_ascii_lowercase()) => Some(r.to_ascii_uppercase()),
        _ => None,
    };
    region.map_or(language.clone(), |r| format!("{language}_{r}"))
}

/// A locale a script gives `$.locale`, as `ll` or `ll_RR`: a language of two or three letters and
/// an optional region of two letters or three digits, joined by `_` or `-`, in any case.
fn script_locale(text: &str) -> Option<String> {
    let mut parts = text.split(['-', '_']);
    let language = parts.next()?;
    let region = parts.next();
    if parts.next().is_some() || !(2..=3).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let language = language.to_ascii_lowercase();
    match region {
        None => Some(language),
        Some(r) if r.len() == 2 && r.bytes().all(|b| b.is_ascii_alphabetic()) => Some(format!("{language}_{}", r.to_ascii_uppercase())),
        Some(r) if r.len() == 3 && r.bytes().all(|b| b.is_ascii_digit()) => Some(format!("{language}_{r}")),
        Some(_) => None,
    }
}

/// The languages whose numbers are written with a decimal comma (VectorCraft's own list).
const DECIMAL_COMMA: &[&str] = &[
    "bg", "ca", "cs", "da", "de", "el", "es", "et", "fi", "fr", "hr", "hu", "id", "it", "lt", "lv", "nb", "nl", "no", "pl", "pt", "ro", "ru", "sk",
    "sl", "sr", "sv", "tr", "uk", "vi",
];

/// The character `locale` writes before a number's fraction.
pub(crate) fn decimal_point(locale: &str) -> &'static str {
    let language = locale.split('_').next().unwrap_or_default();
    if DECIMAL_COMMA.contains(&language) { "," } else { "." }
}

/// The platform suffix of a localization object's names on this system.
pub(crate) fn platform() -> Option<&'static str> {
    if cfg!(target_arch = "wasm32") {
        None
    } else if cfg!(windows) {
        Some("Win")
    } else if cfg!(target_os = "macos") {
        Some("Mac")
    } else {
        Some("Unix")
    }
}

/// `$.os`: the system, without its version.
fn os() -> &'static str {
    if cfg!(target_arch = "wasm32") {
        "Web"
    } else if cfg!(windows) {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else if cfg!(target_os = "freebsd") {
        "FreeBSD"
    } else {
        "Unix"
    }
}

/// Is `value` one an environment value may be?
fn env_value_ok(value: &str) -> bool {
    value.len() <= MAX_ENV_BYTES && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'@'))
}

/// The values of [`ENV_NAMES`] in the process's environment, each left out when it is not one an
/// environment value may be.
fn process_env() -> Vec<(&'static str, Option<String>)> {
    ENV_NAMES.iter().map(|&n| (n, std::env::var(n).ok().filter(|v| env_value_ok(v)))).collect()
}

fn one_string(args: &[Data], name: &str) -> Result<String, HostError> {
    match args.first() {
        Some(Data::String(s)) => Ok(s.clone()),
        _ => Err(HostError::Type(format!("$.{name}: give a name, as a string"))),
    }
}

fn value(v: Data) -> HostItem {
    HostItem::Value(v)
}

fn text(s: impl Into<String>) -> Data {
    Data::String(s.into())
}

/// The members `$` has besides `write`, `writeln`, `evalFile`, `fileName` and `includePath`. They
/// act for the run that `link` holds when they are called.
pub(super) fn members(link: &Link, info: RunInfo, localization: &Rc<Localization>) -> Vec<(String, HostItem)> {
    let version = env!("CARGO_PKG_VERSION");
    let getter = |f: Rc<dyn Fn() -> Data>| HostItem::Getter(function(move |_| Ok(f())));
    let property = |get: Rc<dyn Fn() -> Data>, set: Rc<dyn Fn(Data) -> Result<(), HostError>>| HostItem::Property {
        get: function(move |_| Ok(get())),
        set: function(move |mut a| {
            set(if a.is_empty() { Data::Undefined } else { a.swap_remove(0) })?;
            Ok(Data::Undefined)
        }),
    };
    let fail = |message: &'static str| HostItem::Function(function(move |_| Err(HostError::Error(message.to_string()))));

    let locale = {
        let (l, s, run) = (localization.clone(), localization.clone(), link.clone());
        property(
            Rc::new(move || text(l.locale())),
            Rc::new(move |v| {
                let locale = match v {
                    Data::Null | Data::Undefined => current(&run)?.dollar.locale.clone(),
                    Data::String(t) if t.is_empty() => current(&run)?.dollar.locale.clone(),
                    Data::String(t) => script_locale(&t).ok_or_else(|| HostError::Range(BAD_LOCALE.to_string()))?,
                    _ => return Err(HostError::Type(BAD_LOCALE.to_string())),
                };
                s.set_locale(&locale);
                Ok(())
            }),
        )
    };
    let localize = {
        let (l, s) = (localization.clone(), localization.clone());
        property(
            Rc::new(move || Data::Bool(l.enabled())),
            Rc::new(move |v| match v {
                Data::Bool(on) => {
                    s.set_enabled(on);
                    Ok(())
                }
                _ => Err(HostError::Type("$.localize: give true or false".into())),
            }),
        )
    };
    let flags = {
        let (l, s) = (localization.clone(), localization.clone());
        property(
            Rc::new(move || Data::Number(if l.enabled() { LOCALIZE_FLAG } else { 0.0 })),
            Rc::new(move |v| match v {
                Data::Number(n) if n == 0.0 || n == LOCALIZE_FLAG => {
                    s.set_enabled(n == LOCALIZE_FLAG);
                    Ok(())
                }
                Data::Number(_) => Err(HostError::Range("$.flags: only 512, which turns localization on, or 0 can be set".into())),
                _ => Err(HostError::Type("$.flags: give a number".into())),
            }),
        )
    };
    let decimal = {
        let l = localization.clone();
        getter(Rc::new(move || text(decimal_point(&l.locale()))))
    };
    let app_encoding = property(
        Rc::new(|| text("UTF-8")),
        Rc::new(|v| match v {
            Data::String(t) if t.eq_ignore_ascii_case("utf-8") || t.eq_ignore_ascii_case("utf8") => Ok(()),
            Data::String(_) => Err(HostError::Range("$.appEncoding: scripts here read and write UTF-8 only".into())),
            _ => Err(HostError::Type("$.appEncoding: give an encoding's name, as a string".into())),
        }),
    );
    let mem_cache =
        property(Rc::new(|| Data::Number(0.0)), Rc::new(|_| Err(HostError::Error("$.memCache: the memory cache size cannot be set".into()))));
    let hires_timer = {
        let l = link.clone();
        getter(Rc::new(move || {
            let Ok(h) = current(&l) else { return Data::Number(0.0) };
            let Some(now) = h.elapsed() else { return Data::Number(0.0) };
            let since = now.saturating_sub(h.dollar.timer.replace(now));
            #[allow(clippy::cast_precision_loss)]
            Data::Number(since.as_micros() as f64)
        }))
    };
    let getenv = {
        let l = link.clone();
        function(move |a| {
            let name = one_string(&a, "getenv")?;
            let h = current(&l)?;
            h.charge_call(name.len() as u64 + 2)?;
            let env = h.dollar.env.borrow();
            let found = env.iter().find(|(n, _)| *n == name).and_then(|(_, v)| v.clone());
            Ok(found.map_or(Data::Null, Data::String))
        })
    };
    let setenv = {
        let l = link.clone();
        function(move |a| {
            let name = one_string(&a, "setenv")?;
            let Some(Data::String(v)) = a.get(1) else { return Err(HostError::Type("$.setenv: give a value, as a string".into())) };
            let h = current(&l)?;
            h.charge_call((name.len() + v.len()) as u64 + 4)?;
            let mut env = h.dollar.env.borrow_mut();
            let Some(slot) = env.iter_mut().find(|(n, _)| *n == name) else {
                return Err(HostError::Error(format!("$.setenv: only {} can be set", ENV_NAMES.join(", "))));
            };
            if !env_value_ok(v) {
                return Err(HostError::Range(format!("$.setenv: a value has at most {MAX_ENV_BYTES} letters, digits and _ . - @")));
            }
            slot.1 = Some(v.clone());
            Ok(Data::Undefined)
        })
    };
    let sleep = {
        let l = link.clone();
        function(move |a| {
            let h = current(&l)?;
            let ms = match a.first() {
                Some(Data::Number(n)) if (0.0..=MAX_SLEEP_MS).contains(n) => *n,
                Some(Data::Number(_)) => return Err(HostError::Range(format!("$.sleep: give a number of milliseconds from 0 to {MAX_SLEEP_MS}"))),
                _ => return Err(HostError::Type("$.sleep: give a number of milliseconds".into())),
            };
            h.charge_call(8)?;
            h.sleep(Duration::from_secs_f64(ms / 1000.0))?;
            Ok(Data::Undefined)
        })
    };
    let about = text(format!("VectorCraft script engine {version}"));

    vec![
        ("about".into(), HostItem::Function(function(move |_| Ok(about.clone())))),
        ("appEncoding".into(), app_encoding),
        ("bp".into(), fail("$.bp: scripts here have no debugger")),
        ("build".into(), value(text(version))),
        ("buildDate".into(), value(Data::Null)),
        ("colorPicker".into(), fail("$.colorPicker: scripts cannot show a dialog yet")),
        ("decimalPoint".into(), decimal),
        ("engineName".into(), value(text(info.engine.unwrap_or_else(|| "VectorCraft".into())))),
        ("error".into(), HostItem::Script(ScriptItem::LastError)),
        ("flags".into(), flags),
        ("gc".into(), HostItem::Function(function(|_| Ok(Data::Undefined)))),
        ("getenv".into(), HostItem::Function(getenv)),
        ("global".into(), HostItem::Script(ScriptItem::Global)),
        ("hiresTimer".into(), hires_timer),
        ("level".into(), value(Data::Number(0.0))),
        ("line".into(), HostItem::Script(ScriptItem::Line)),
        ("locale".into(), locale),
        ("localize".into(), localize),
        ("memCache".into(), mem_cache),
        ("os".into(), value(text(os()))),
        ("screens".into(), value(Data::Array(Vec::new()))),
        ("setenv".into(), HostItem::Function(setenv)),
        ("sleep".into(), HostItem::Function(sleep)),
        ("stack".into(), HostItem::Script(ScriptItem::Stack)),
        ("strict".into(), value(Data::Bool(info.strict))),
        ("version".into(), value(text(version))),
    ]
}

const BAD_LOCALE: &str = "$.locale: give a locale such as \"en\" or \"de_DE\", or null for the interface's";
