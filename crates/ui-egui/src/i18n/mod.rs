//! UI localisation. Strings in code stay English and are the default lookup keys; a per-language
//! catalog (`*.tsv`, see `zh-hant.tsv` for the format) maps them to display text at render time.
//! Command ids, menu paths used for logic, the control channel, the CLI and MCP never see
//! translated text, so agents and scripts are unaffected. A string without a translation is shown
//! in English.
//!
//! # Adding a language
//! 1. Add `xx.tsv` next to `zh-hant.tsv` (copy its header; translate from the *meaning* of the
//!    English text, clean-room, see `zh-hant.tsv`).
//! 2. Add one row to [`LANGUAGES`] (code, native name, catalog, plural rule).
//!
//! That is all: the Preferences dropdown, the system-locale match and the catalog tests (parse,
//! placeholders, plural forms, menu coverage) pick it up from the registry.
//!
//! # Looking strings up
//! - [`t`] / the `tl!` macro: a plain string in the language the UI is drawn in. [`tr`] takes the
//!   language explicitly; [`tr_ctx`] when one English word needs different translations.
//! - [`tr_id`]: a command-id keyed string with the English label as fallback (menu items), so a
//!   translation survives rewording of the English text and can differ per command.
//! - [`trn`]: plural-aware (`{n}` is filled in). [`fmt`]: fill `{name}` placeholders after [`tr`];
//!   translators may reorder placeholders freely.
//! - [`message`] / [`msg`]: a status or error message, translated only where it is shown. The
//!   message itself stays English (the control channel, MCP and tests read it); `@msg` catalog
//!   rows hold whole messages or templates such as `Couldn't open {name}: {e}`, and the values in
//!   the placeholders are translated in turn (the reason after `: {e}` is often a message too).

mod catalog;

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use catalog::Catalog;

/// One supported UI language.
pub struct LangInfo {
    /// BCP 47 code, lowercase (`ja`, `zh-hant`, `pt-br`). Also the `interfaceLanguage` value.
    pub code: &'static str,
    /// The language's name in itself, shown in the Preferences dropdown.
    pub name: &'static str,
    /// Catalog file contents (empty for the built-in English).
    pub source: &'static str,
    /// Plural form index for a count (English: 0 = one, 1 = other; Chinese: always 0).
    pub plural: fn(u64) -> usize,
    /// Must the catalog cover every menu string? (checked by the tests)
    pub complete_menus: bool,
    catalog: OnceLock<Catalog>,
}

fn plural_one_other(n: u64) -> usize {
    usize::from(n != 1)
}

fn plural_none(_: u64) -> usize {
    0
}

/// Czech: 1 is `one`, 2–4 `few`, everything else (0, 5…) `other`.
fn plural_czech(n: u64) -> usize {
    match n {
        1 => 0,
        2..=4 => 1,
        _ => 2,
    }
}

/// French: 0 and 1 are `one` (« 0 calque »), everything else `other`.
fn plural_french(n: u64) -> usize {
    usize::from(n > 1)
}

/// Russian and Ukrainian integer counts: 1 (but not 11) is `one`, 2–4 (but not 12–14)
/// `few`, everything else `many`.
fn plural_east_slavic(n: u64) -> usize {
    match n % 100 {
        11..=14 => 2,
        _ => match n % 10 {
            1 => 0,
            2..=4 => 1,
            _ => 2,
        },
    }
}

/// The registry. English first: it is the fallback and the source language.
pub static LANGUAGES: [LangInfo; 12] = [
    LangInfo { code: "en", name: "English", source: "", plural: plural_one_other, complete_menus: false, catalog: OnceLock::new() },
    // Japanese: the whole interface (every menu string and `tl!` literal) and the status and error
    // messages, keeping the product, workspace and perspective preset names in English (`MENU_KEEP_AS_IS`).
    LangInfo { code: "ja", name: "日本語", source: include_str!("ja.tsv"), plural: plural_none, complete_menus: true, catalog: OnceLock::new() },
    // Czech: every menu label (`menu_catalogs_translate_every_menu_label`); panels and dialogs not yet.
    LangInfo { code: "cs", name: "Čeština", source: include_str!("cs.tsv"), plural: plural_czech, complete_menus: false, catalog: OnceLock::new() },
    // German: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish; every `de-*` locale (`de-DE`, `de-AT`, `de-CH`, `de-LU` …) resolves here.
    LangInfo {
        code: "de",
        name: "Deutsch",
        source: include_str!("de.tsv"),
        plural: plural_one_other,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Spanish: the whole interface in neutral, international Spanish, keeping the same names in
    // English as Japanese; every `es-*` locale (`es-ES`, `es-MX`, `es-AR`, `es-419` …) resolves here.
    LangInfo {
        code: "es",
        name: "Español",
        source: include_str!("es.tsv"),
        plural: plural_one_other,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // French: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish; every `fr-*` locale (`fr-FR`, `fr-BE`, `fr-CA`, `fr-CH` …) resolves here.
    LangInfo { code: "fr", name: "Français", source: include_str!("fr.tsv"), plural: plural_french, complete_menus: true, catalog: OnceLock::new() },
    // Italian: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish; every `it-*` locale (`it-IT`, `it-CH`, `it-SM` …) resolves here.
    LangInfo {
        code: "it",
        name: "Italiano",
        source: include_str!("it.tsv"),
        plural: plural_one_other,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Russian: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish; every `ru-*` locale (`ru-RU`, `ru-BY`, `ru-KZ` …) resolves here.
    LangInfo {
        code: "ru",
        name: "Русский",
        source: include_str!("ru.tsv"),
        plural: plural_east_slavic,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Ukrainian: the whole interface and the status and error messages, with one/few/many
    // plural forms; every `uk-*` locale (including `uk-UA`) resolves here.
    LangInfo {
        code: "uk",
        name: "Українська",
        source: include_str!("uk.tsv"),
        plural: plural_east_slavic,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Brazilian Portuguese: every menu label (`menu_catalogs_translate_every_menu_label`), every
    // `tl!` literal and the plural messages; left English on purpose are the `MENU_KEEP_AS_IS`
    // strings (product name, language names, workspace names, perspective grid presets).
    LangInfo {
        code: "pt-br",
        name: "Português (Brasil)",
        source: include_str!("pt-br.tsv"),
        plural: plural_one_other,
        complete_menus: false,
        catalog: OnceLock::new(),
    },
    // Traditional Chinese in the vocabulary used in Taiwan; `zh-TW`, `zh-HK`, `zh-MO` and `zh-Hant-*`
    // locales all resolve here (see `candidates`).
    LangInfo {
        code: "zh-hant",
        name: "繁體中文",
        source: include_str!("zh-hant.tsv"),
        plural: plural_none,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Simplified Chinese in the vocabulary used in mainland China; `zh-CN`, `zh-SG`, `zh-Hans-*` and a
    // bare `zh` resolve here (see `candidates`).
    LangInfo {
        code: "zh-hans",
        name: "简体中文",
        source: include_str!("zh-hans.tsv"),
        plural: plural_none,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
];

impl LangInfo {
    fn catalog(&self) -> &Catalog {
        self.catalog.get_or_init(|| Catalog::parse(self.source))
    }
}

/// A language the UI can be shown in (a handle into [`LANGUAGES`]).
#[derive(Clone, Copy)]
pub struct Lang(&'static LangInfo);

impl std::fmt::Debug for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lang({})", self.0.code)
    }
}

impl PartialEq for Lang {
    fn eq(&self, other: &Self) -> bool {
        self.0.code == other.0.code
    }
}

impl Eq for Lang {}

impl Lang {
    pub const EN: Lang = Lang(&LANGUAGES[0]);

    pub fn code(self) -> &'static str {
        self.0.code
    }

    /// A language by its exact code.
    pub fn from_code(code: &str) -> Option<Lang> {
        LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code)).map(Lang)
    }

    /// Resolve the `interfaceLanguage` preference: a language code, or `auto` (and anything
    /// unknown, e.g. a code from a newer version) to follow the system locale.
    pub fn from_pref(pref: &str) -> Lang {
        Lang::from_code(pref).unwrap_or_else(system_lang)
    }

    /// Every registered language.
    pub fn all() -> impl Iterator<Item = Lang> {
        LANGUAGES.iter().map(Lang)
    }

    pub fn name(self) -> &'static str {
        self.0.name
    }

    /// Does this language's catalog claim to cover every menu string and `tl!` literal?
    pub fn complete_menus(self) -> bool {
        self.0.complete_menus
    }

    fn catalog(self) -> &'static Catalog {
        self.0.catalog()
    }
}

/// Candidate language codes for a locale tag, most specific first: `zh_TW.UTF-8` →
/// `zh-tw`, `zh-hant`, `zh`.
fn candidates(tag: &str) -> Vec<String> {
    let base = tag.split(['.', '@']).next().unwrap_or("").replace('_', "-").to_ascii_lowercase();
    let parts: Vec<&str> = base.split('-').filter(|p| !p.is_empty()).collect();
    let Some(&primary) = parts.first() else { return Vec::new() };
    let mut out = Vec::new();
    for n in (1..=parts.len()).rev() {
        out.push(parts[..n].join("-"));
    }
    if primary == "zh" && !parts.iter().any(|p| matches!(*p, "hans" | "hant")) {
        // Chinese by region when no script is given.
        let script = if parts.iter().any(|p| matches!(*p, "tw" | "hk" | "mo")) { "zh-hant" } else { "zh-hans" };
        out.insert(out.len() - 1, script.to_string());
    }
    out
}

/// The registered language for a locale tag such as `ja_JP.UTF-8`, `ja-JP`, `zh-TW`; `None` if
/// the language isn't supported. `C`/`POSIX` mean English.
pub fn lang_from_tag(tag: &str) -> Option<Lang> {
    let cands = candidates(tag);
    if matches!(cands.first().map(String::as_str), Some("c" | "posix")) {
        return Some(Lang::EN);
    }
    cands.iter().find_map(|c| Lang::from_code(c))
}

/// True when `tag` names the generic `C` / `POSIX` locale (or a variant like `C.UTF-8`), which on
/// macOS means "no particular language" rather than English: the system language list decides.
#[cfg(not(target_arch = "wasm32"))]
fn is_c_locale(tag: &str) -> bool {
    let base = tag.split(['.', '@']).next().unwrap_or("").to_ascii_lowercase();
    matches!(base.as_str(), "c" | "posix")
}

/// Work out the system language on a background thread, so the first frame doesn't wait for the
/// locale probe (which runs `reg.exe` on Windows and `defaults` on macOS). Call it at startup; a
/// frame that needs the language before the probe finishes waits for it. A no-op on wasm.
pub fn detect_system_lang_in_background() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // A failed spawn leaves the probe to the first frame that needs it.
        let _ = std::thread::Builder::new().name("locale-probe".into()).spawn(|| {
            system_lang();
        });
    }
}

/// The system language, worked out once.
static SYSTEM: OnceLock<Lang> = OnceLock::new();

/// The system language (cached). English when it can't be determined.
pub fn system_lang() -> Lang {
    // Tests drive the UI by its English labels whatever the developer's locale is.
    if cfg!(test) {
        return Lang::EN;
    }
    *SYSTEM.get_or_init(detect_system_lang)
}

/// Tell the UI the system's preferred languages, most preferred first, where it can't find them
/// itself: the web shell passes the browser's (`navigator.languages`). The first supported one is
/// the system language (English if none is); call it before the first frame. Has no effect once
/// the system language is known.
pub fn set_system_locales<S: AsRef<str>>(tags: &[S]) {
    let lang = tags.iter().find_map(|t| lang_from_tag(t.as_ref())).unwrap_or(Lang::EN);
    // Already set means already detected (or reported): the first answer stands.
    let _ = SYSTEM.set(lang);
}

#[cfg(not(target_arch = "wasm32"))]
fn detect_system_lang() -> Lang {
    // `VECTORCRAFT_LOCALE` overrides everything (also handy for screenshots); then the POSIX
    // variables. `LANGUAGE` is a colon-separated priority list.
    for var in ["VECTORCRAFT_LOCALE", "LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        let Some(v) = std::env::var(var).ok().filter(|v| !v.is_empty()) else { continue };
        // `C` / `POSIX` (and variants like `C.UTF-8`, `POSIX.UTF-8`) mean "no particular locale":
        // don't force English, fall through to the OS preferred-languages list below.
        if v.split(':').all(is_c_locale) {
            continue;
        }
        if let Some(l) = v.split(':').find_map(lang_from_tag) {
            return l;
        }
    }
    // Apps started from the Finder don't inherit LANG: use the macOS preferred-languages list.
    // The absolute path keeps a `defaults` earlier on PATH from running; any failure means English.
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("/usr/bin/defaults").args(["read", "-g", "AppleLanguages"]).output()
        && out.status.success()
        && let Some(l) = first_supported(&String::from_utf8_lossy(&out.stdout))
    {
        return l;
    }
    // Windows sets no LANG: read the user locale (`HKCU\Control Panel\International` ›
    // `LocaleName`, e.g. `zh-TW`).
    #[cfg(target_os = "windows")]
    if let Some(l) = windows_locale().as_deref().and_then(lang_from_tag) {
        return l;
    }
    Lang::EN
}

/// The first supported language in a `defaults read` list like `(\n    "ja-JP",\n    "en-US"\n)`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn first_supported(list: &str) -> Option<Lang> {
    list.split(['(', ')', ',', '"', '\n']).map(str::trim).filter(|s| !s.is_empty()).find_map(lang_from_tag)
}

/// The Windows user locale name (`reg query` so no extra dependency; a hidden console window).
#[cfg(target_os = "windows")]
fn windows_locale() -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // reg.exe by its full path, so nothing named `reg` in the app's folder or on PATH runs instead.
    let root = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
    let out = std::process::Command::new(root.join("System32").join("reg.exe"))
        .args(["query", "HKCU\\Control Panel\\International", "/v", "LocaleName"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    registry_locale(&String::from_utf8_lossy(&out.stdout))
}

/// The value in `reg query` output like `    LocaleName    REG_SZ    zh-TW`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn registry_locale(text: &str) -> Option<String> {
    text.lines().find(|l| l.contains("LocaleName")).and_then(|l| l.split_whitespace().last()).map(str::to_string)
}

/// The browser's languages arrive through [`set_system_locales`]; without them, English.
#[cfg(target_arch = "wasm32")]
fn detect_system_lang() -> Lang {
    Lang::EN
}

/// Index into [`LANGUAGES`] of the language the UI is drawn in this frame.
static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// Set the UI language for drawing (the shell calls this once per frame from the preference), so
/// widgets can translate without every call site carrying a language around.
pub fn set_current(lang: Lang) {
    let i = LANGUAGES.iter().position(|l| l.code == lang.code()).unwrap_or(0);
    CURRENT.store(i, Ordering::Relaxed);
}

/// The language the UI is drawn in.
pub fn current() -> Lang {
    Lang(LANGUAGES.get(CURRENT.load(Ordering::Relaxed)).unwrap_or(&LANGUAGES[0]))
}

/// Does `lang` have a catalog entry for this plain string? (English never does: it is the source.)
pub fn has(lang: Lang, s: &str) -> bool {
    lang.catalog().plain(s).is_some()
}

/// Translate an English UI string into the current language ([`tr`] with [`current`]).
pub fn t(s: &str) -> &str {
    tr(current(), s)
}

/// Translate an English UI string; unknown strings come back unchanged.
pub fn tr(lang: Lang, s: &str) -> &str {
    lang.catalog().plain(s).unwrap_or(s)
}

/// Like [`tr`], for an English string that needs a disambiguating `context`.
pub fn tr_ctx<'a>(lang: Lang, context: &str, s: &'a str) -> &'a str {
    lang.catalog().contextual(context, s).unwrap_or_else(|| tr(lang, s))
}

/// A string keyed by its command id, falling back to the translation of the English `label`.
pub fn tr_id<'a>(lang: Lang, id: &str, label: &'a str) -> &'a str {
    lang.catalog().id(id).unwrap_or_else(|| tr(lang, label))
}

/// [`tr_id`] in the current language.
pub fn t_id<'a>(id: &str, label: &'a str) -> &'a str {
    tr_id(current(), id, label)
}

/// Fill `{name}` placeholders. Unknown placeholders are left as written.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// A plural-aware message: `one`/`other` are the English forms (with `{n}` where the count goes).
pub fn trn(lang: Lang, n: u64, one: &str, other: &str) -> String {
    let idx = (lang.0.plural)(n);
    let text = lang.catalog().plural(one, other, idx).unwrap_or(if n == 1 { one } else { other });
    fmt(text, &[("n", &n.to_string())])
}

/// An entry of a list that mixes interface labels with names (user, file or system data) as
/// shown: a built-in entry in `lang`, a name exactly as it is (a library the user calls "Layers"
/// stays "Layers"). Show the result with the non-translating widgets (`dropdown_names`,
/// `menu_item_name`, `dim_name`).
pub fn label_or_name(lang: Lang, s: &str, built_in: bool) -> &str {
    if built_in { tr(lang, s) } else { s }
}

/// How deep [`message`] translates placeholder values that are messages themselves.
const MESSAGE_DEPTH: usize = 3;

/// Translate a status or error message for display (see the module docs). Unknown messages, and
/// parts of them (file names, numbers, a reason with no entry), stay as they are.
pub fn message(lang: Lang, s: &str) -> String {
    message_at(lang, s, MESSAGE_DEPTH)
}

fn message_at(lang: Lang, s: &str, depth: usize) -> String {
    let Some((template, caps)) = lang.catalog().message(s) else { return s.to_string() };
    let values: Vec<(&str, String)> =
        caps.into_iter().map(|(k, v)| (k, if depth > 1 { message_at(lang, v, depth - 1) } else { v.to_string() })).collect();
    let args: Vec<(&str, &str)> = values.iter().map(|(k, v)| (*k, v.as_str())).collect();
    fmt(template, &args)
}

/// [`message`] in the current language, remembering the last answer: the status bar asks for the
/// same message every frame.
pub fn msg(s: &str) -> String {
    static LAST: std::sync::Mutex<Option<(&'static str, String, String)>> = std::sync::Mutex::new(None);
    let lang = current();
    let mut last = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((code, src, out)) = last.as_ref()
        && *code == lang.code()
        && src == s
    {
        return out.clone();
    }
    let out = message(lang, s);
    *last = Some((lang.code(), s.to_string(), out.clone()));
    out
}

/// [`trn`] in the current language.
pub fn tn(n: u64, one: &str, other: &str) -> String {
    trn(current(), n, one, other)
}

#[cfg(test)]
mod tests;
