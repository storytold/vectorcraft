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
//! That is all: the Preferences dropdown, the system-locale match, the catalog tests (parse,
//! placeholders, plural forms) and the untranslated report pick it up from the registry.
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
    /// Does the catalog aim to cover every menu string? The untranslated report lists what it lacks.
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

/// Polish: only 1 is `one`; 2–4 (but not 12–14) `few`; everything else (0, 5–21, 25…) `many`
/// (unlike Russian and Ukrainian, 21, 31 … take `many`: "21 warstw").
fn plural_polish(n: u64) -> usize {
    if n == 1 {
        return 0;
    }
    match (n % 10, n % 100) {
        (2..=4, r) if !(12..=14).contains(&r) => 1,
        _ => 2,
    }
}

/// The registry. English first: it is the fallback and the source language.
pub static LANGUAGES: [LangInfo; 26] = [
    LangInfo { code: "en", name: "English", source: "", plural: plural_one_other, complete_menus: false, catalog: OnceLock::new() },
    // Greek: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish; every `el-*` locale (including `el-GR` and `el-CY`) resolves here.
    LangInfo {
        code: "el",
        name: "Ελληνικά",
        source: include_str!("i18n/el.tsv"),
        plural: plural_one_other,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Japanese: the whole interface (every menu string and `tl!` literal) and the status and error
    // messages, keeping the product, workspace and perspective preset names in English (`MENU_KEEP_AS_IS`).
    LangInfo {
        code: "ja", name: "日本語", source: include_str!("i18n/ja.tsv"), plural: plural_none, complete_menus: true, catalog: OnceLock::new()
    },
    // Korean: the whole interface and the status and error messages, keeping the same names in
    // English as Japanese; a count takes no plural, so plurals have one form. Hangul is drawn with an
    // installed system font (craft-fonts has no Korean UI face yet); every `ko-*` locale resolves here.
    LangInfo {
        code: "ko", name: "한국어", source: include_str!("i18n/ko.tsv"), plural: plural_none, complete_menus: true, catalog: OnceLock::new()
    },
    // Vietnamese: complete menus and messages, with one plural form.
    // Every `vi-*` locale resolves here.
    LangInfo {
        code: "vi",
        name: "Tiếng Việt",
        source: include_str!("i18n/vi.tsv"),
        plural: plural_none,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Czech: the whole interface (every menu string and `tl!` literal) and the status and error
    // messages, with one/few/other plurals, keeping the same names in English as Spanish.
    LangInfo {
        code: "cs",
        name: "Čeština",
        source: include_str!("i18n/cs.tsv"),
        plural: plural_czech,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // German: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish; every `de-*` locale (`de-DE`, `de-AT`, `de-CH`, `de-LU` …) resolves here.
    LangInfo {
        code: "de",
        name: "Deutsch",
        source: include_str!("i18n/de.tsv"),
        plural: plural_one_other,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Spanish: the whole interface in neutral, international Spanish, keeping the same names in
    // English as Japanese; every `es-*` locale (`es-ES`, `es-MX`, `es-AR`, `es-419` …) resolves here.
    LangInfo {
        code: "es",
        name: "Español",
        source: include_str!("i18n/es.tsv"),
        plural: plural_one_other,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // French: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish; every `fr-*` locale (`fr-FR`, `fr-BE`, `fr-CA`, `fr-CH` …) resolves here.
    LangInfo {
        code: "fr",
        name: "Français",
        source: include_str!("i18n/fr.tsv"),
        plural: plural_french,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Italian: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish; every `it-*` locale (`it-IT`, `it-CH`, `it-SM` …) resolves here.
    LangInfo {
        code: "it",
        name: "Italiano",
        source: include_str!("i18n/it.tsv"),
        plural: plural_one_other,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Georgian: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish. Georgian nouns stay singular after a number, so one plural form;
    // `ka-GE` and `ka_GE.UTF-8` resolve here.
    LangInfo {
        code: "ka",
        name: "ქართული",
        source: include_str!("i18n/ka.tsv"),
        plural: plural_none,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Russian: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish; every `ru-*` locale (`ru-RU`, `ru-BY`, `ru-KZ` …) resolves here.
    LangInfo {
        code: "ru",
        name: "Русский",
        source: include_str!("i18n/ru.tsv"),
        plural: plural_east_slavic,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Ukrainian: the whole interface and the status and error messages, with one/few/many
    // plural forms; every `uk-*` locale (including `uk-UA`) resolves here.
    LangInfo {
        code: "uk",
        name: "Українська",
        source: include_str!("i18n/uk.tsv"),
        plural: plural_east_slavic,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Bulgarian: the whole interface (every menu string and `tl!` literal) and the status and error
    // messages, with one/other plural forms; every `bg-*` locale (including `bg-BG`) resolves here.
    LangInfo {
        code: "bg",
        name: "Български",
        source: include_str!("i18n/bg.tsv"),
        plural: plural_one_other,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Hungarian: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish; a count takes the singular ("3 réteg"), so plurals have one form. Every
    // `hu-*` locale (including `hu-HU`) resolves here.
    LangInfo { code: "hu", name: "Magyar", source: include_str!("i18n/hu.tsv"), plural: plural_none, complete_menus: true, catalog: OnceLock::new() },
    // Polish: the whole interface and the status and error messages, with one/few/many plurals,
    // keeping the same names in English as Spanish; every `pl-*` locale (including `pl-PL`)
    // resolves here.
    LangInfo {
        code: "pl",
        name: "Polski",
        source: include_str!("i18n/pl.tsv"),
        plural: plural_polish,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Swedish: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish; every `sv-*` locale (`sv-SE`, `sv-FI` …) resolves here.
    LangInfo {
        code: "sv",
        name: "Svenska",
        source: include_str!("i18n/sv.tsv"),
        plural: plural_one_other,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Brazilian Portuguese: the whole interface and the status and error messages, keeping the
    // same names in English as Spanish; every `pt-BR` / `pt_BR` locale resolves here (see
    // `candidates`: `pt-br` then `pt`).
    LangInfo {
        code: "pt-br",
        name: "Português (Brasil)",
        source: include_str!("i18n/pt-br.tsv"),
        plural: plural_one_other,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Traditional Chinese in the vocabulary used in Taiwan: the whole interface and the status and
    // error messages; `zh-TW`, `zh-HK`, `zh-MO` and `zh-Hant-*` locales all resolve here (see `candidates`).
    LangInfo {
        code: "zh-hant",
        name: "繁體中文",
        source: include_str!("i18n/zh-hant.tsv"),
        plural: plural_none,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Simplified Chinese in the vocabulary used in mainland China: the whole interface and the status
    // and error messages; `zh-CN`, `zh-SG`, `zh-Hans-*` and a bare `zh` resolve here (see `candidates`).
    LangInfo {
        code: "zh-hans",
        name: "简体中文",
        source: include_str!("i18n/zh-hans.tsv"),
        plural: plural_none,
        complete_menus: true,
        catalog: OnceLock::new(),
    }, // Dutch; every `nl-*` locale (`nl-NL`, `nl-BE`, `nl_NL.UTF-8`…) resolves here.
    LangInfo {
        code: "nl",
        name: "Nederlands",
        source: include_str!("i18n/nl.tsv"),
        plural: plural_one_other,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Danish; every `da-*` locale (`da-DK`, `da_DK.UTF-8`…) resolves here.
    LangInfo {
        code: "da",
        name: "Dansk",
        source: include_str!("i18n/da.tsv"),
        plural: plural_one_other,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Indonesian; every `id-*` locale (`id-ID`, `id_ID.UTF-8`…) resolves here. CLDR `id` counts
    // have one form, so `@plural` entries list a single form.
    LangInfo {
        code: "id",
        name: "Bahasa Indonesia",
        source: include_str!("i18n/id.tsv"),
        plural: plural_none,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Finnish: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish; one/other plurals with the partitive after any number but 1 ("2 tasoa").
    // Every `fi-*` locale resolves here.
    LangInfo {
        code: "fi",
        name: "Suomi",
        source: include_str!("i18n/fi.tsv"),
        plural: plural_one_other,
        complete_menus: true,
        catalog: OnceLock::new(),
    },
    // Turkish: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish; a count takes the singular ("3 katman"), so plurals have one form. Every
    // `tr-*` locale (including `tr-TR` and `tr-CY`) resolves here.
    LangInfo {
        code: "tr", name: "Türkçe", source: include_str!("i18n/tr.tsv"), plural: plural_none, complete_menus: true, catalog: OnceLock::new()
    },
    // Catalan: the whole interface and the status and error messages, keeping the same names in
    // English as Spanish; one/other plurals. Every `ca-*` locale (including `ca-ES`, `ca-AD`,
    // `ca-FR`, `ca-IT` and `ca-ES-VALENCIA`) resolves here.
    LangInfo {
        code: "ca",
        name: "Català",
        source: include_str!("i18n/ca.tsv"),
        plural: plural_one_other,
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

    /// Every registered language in the order a picker lists them: English first (the source
    /// language and the fallback), then the rest by their own name with accents folded, so
    /// Čeština sits under C rather than after Z. Names in other scripts follow the Latin ones in
    /// code point order: Greek, Cyrillic, Arabic, then the CJK languages.
    pub fn for_picker() -> Vec<Lang> {
        let mut rest: Vec<Lang> = Lang::all().filter(|l| l.code() != Lang::EN.code()).collect();
        rest.sort_by_cached_key(|l| name_sort_key(l.name()));
        std::iter::once(Lang::EN).chain(rest).collect()
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

/// Index into [`LANGUAGES`] of the language the UI is drawn in this frame. The app keeps one
/// index for the whole process. Unit tests keep one per thread, as `ui_mirror!` keeps the
/// shortcut and workspace mirrors, so a language one test sets does not change the language of
/// the tests running beside it on other threads.
fn current_index() -> &'static AtomicUsize {
    #[cfg(not(test))]
    {
        static CURRENT: AtomicUsize = AtomicUsize::new(0);
        &CURRENT
    }
    #[cfg(test)]
    {
        thread_local!(static CURRENT: &'static AtomicUsize = Box::leak(Box::default()));
        CURRENT.with(|c| *c)
    }
}

/// Set the UI language for drawing (the shell calls this once per frame from the preference), so
/// widgets can translate without every call site carrying a language around.
pub fn set_current(lang: Lang) {
    let i = LANGUAGES.iter().position(|l| l.code == lang.code()).unwrap_or(0);
    current_index().store(i, Ordering::Relaxed);
}

/// The language the UI is drawn in.
pub fn current() -> Lang {
    Lang(LANGUAGES.get(current_index().load(Ordering::Relaxed)).unwrap_or(&LANGUAGES[0]))
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

/// A name the app gives something (a default swatch's), translated where the current language has
/// a `context` row for it; any other name (one the user gave) comes back as it is.
pub fn t_name<'a>(context: &str, name: &'a str) -> &'a str {
    current().catalog().contextual(context, name).unwrap_or(name)
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

/// How deep [`message`] translates placeholder values that are messages themselves. The status
/// bar's line for an EPS file that can't be opened has five levels: `Couldn't open {name}: {e}`,
/// the loader's `can't open …: {e}` (matched by `{path}: {e}`), the reader's `this PostScript file
/// can't be read ({why}): …`, `a PostScript error (…; {meaning})` and the meaning.
const MESSAGE_DEPTH: usize = 5;

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

/// `name` lowercased with the Latin accents that occur in language names folded away, so a
/// byte-wise sort reads alphabetically. Other scripts are left as they are.
pub fn name_sort_key(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'ď' | 'đ' => 'd',
            'è'..='ë' | 'ē' | 'ě' | 'ę' | 'ế' | 'ệ' => 'e',
            'ğ' | 'ģ' => 'g',
            'ì'..='ï' | 'ī' | 'ı' => 'i',
            'ł' | 'ļ' => 'l',
            'ñ' | 'ń' | 'ň' | 'ņ' => 'n',
            'ò'..='ö' | 'ø' | 'ő' => 'o',
            'ř' => 'r',
            'ś' | 'š' | 'ş' | 'ș' => 's',
            'ť' | 'ț' | 'ţ' => 't',
            'ù'..='ü' | 'ū' | 'ů' | 'ű' => 'u',
            'ý' | 'ÿ' => 'y',
            'ź' | 'ż' | 'ž' => 'z',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod picker_order_tests {
    use super::*;

    #[test]
    fn picker_lists_english_first_then_names_alphabetically() {
        let order = Lang::for_picker();
        assert_eq!(order.first().map(|l| l.code()), Some(Lang::EN.code()));
        assert_eq!(order.len(), Lang::all().count());
        let keys: Vec<String> = order[1..].iter().map(|l| name_sort_key(l.name())).collect();
        assert!(keys.windows(2).all(|w| w[0] <= w[1]), "{keys:?}");
        assert_eq!(name_sort_key("Čeština"), "cestina");
        assert_eq!(name_sort_key("Türkçe"), "turkce");
        assert_eq!(name_sort_key("Łącza"), "lacza");
    }
}
