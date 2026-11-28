//! UI localisation. Strings in code stay English and are the default lookup keys; a per-language
//! catalog (`*.tsv`, see `zh-hant.tsv` for the format) maps them to display text at render time.
//! Command ids, menu paths used for logic, the control channel, the CLI and MCP never see
//! translated text, so agents and scripts are unaffected. A string without a translation is shown
//! in English. The catalogs are read by `craft-i18n`; this module is the app's face on it.
//!
//! # Adding a language
//! 1. Add `xx.tsv` next to `zh-hant.tsv` (copy its header; translate from the *meaning* of the
//!    English text, clean-room, see `zh-hant.tsv`), with an `@id` row `language.name` holding the
//!    language's name in itself.
//! 2. Add one entry to [`EMBEDDED`].
//!
//! That is all: the Preferences dropdown, the system-locale match, the catalog tests (parse,
//! placeholders, plural forms) and the untranslated report pick it up from the embedded list.
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
//!   A `@msgplural` row is a template with a count: its source holds the English `one|other`
//!   forms, its translation the language's plural forms, and the `{n}` value picks the form;
//!   [`msg_n`] writes such a message in English. How rows match a message is `craft-i18n`'s.

// PLAN: crates/ui-egui/Cargo.toml needs `craft-i18n = { workspace = true }`; the manifest is not in
// this set's Files table.
use std::borrow::Cow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use craft::i18n::{Embedded, I18n, fmt_plain};

pub use craft::i18n::Lang;

/// The catalogs of every language but English (the source language), by code. English is reserved
/// by `craft-i18n` and has no catalog.
static EMBEDDED: &Embedded = &[
    ("hi", "", include_str!("i18n/hi.tsv")),
    ("gu", "", include_str!("i18n/gu.tsv")),
    ("el", "", include_str!("i18n/el.tsv")),
    ("ja", "", include_str!("i18n/ja.tsv")),
    ("ko", "", include_str!("i18n/ko.tsv")),
    ("vi", "", include_str!("i18n/vi.tsv")),
    ("cs", "", include_str!("i18n/cs.tsv")),
    ("bs", "", include_str!("i18n/bs.tsv")),
    ("de", "", include_str!("i18n/de.tsv")),
    ("es", "", include_str!("i18n/es.tsv")),
    ("fr", "", include_str!("i18n/fr.tsv")),
    ("it", "", include_str!("i18n/it.tsv")),
    ("ka", "", include_str!("i18n/ka.tsv")),
    ("ru", "", include_str!("i18n/ru.tsv")),
    ("uk", "", include_str!("i18n/uk.tsv")),
    ("bg", "", include_str!("i18n/bg.tsv")),
    ("hu", "", include_str!("i18n/hu.tsv")),
    ("pl", "", include_str!("i18n/pl.tsv")),
    ("sv", "", include_str!("i18n/sv.tsv")),
    ("pt-br", "", include_str!("i18n/pt-br.tsv")),
    ("zh-hant", "", include_str!("i18n/zh-hant.tsv")),
    ("zh-hans", "", include_str!("i18n/zh-hans.tsv")),
    ("nl", "", include_str!("i18n/nl.tsv")),
    ("da", "", include_str!("i18n/da.tsv")),
    ("id", "", include_str!("i18n/id.tsv")),
    ("fi", "", include_str!("i18n/fi.tsv")),
    ("tr", "", include_str!("i18n/tr.tsv")),
    ("ca", "", include_str!("i18n/ca.tsv")),
    ("he", "", include_str!("i18n/he.tsv")),
];

/// The codes of [`EMBEDDED`] the interface offers. Browsers cannot use the native installed-font
/// fallback, so on the web a language whose text would be missing-glyph boxes (Gujarati without a
/// bundled Gujarati face) is not offered, including when restoring a preference.
fn offered_codes(web: bool, gujarati_font: bool) -> Vec<&'static str> {
    EMBEDDED.iter().map(|(code, ..)| *code).filter(|code| *code != "gu" || !web || gujarati_font).collect()
}

/// The one catalog set of the process, built on first use.
fn shared_cell() -> &'static Arc<I18n> {
    static SHARED: OnceLock<Arc<I18n>> = OnceLock::new();
    SHARED.get_or_init(|| {
        let offered = offered_codes(cfg!(target_arch = "wasm32"), vectorcraft_text::CRAFT_FONTS.iter().any(|f| f.has_script("Gujr")));
        let (i18n, problems) = I18n::new(EMBEDDED, &offered);
        for problem in problems {
            log::warn!("vectorcraft: interface language catalog: {problem}");
        }
        Arc::new(i18n)
    })
}

/// The app's catalogs, the same for the whole process.
pub fn shared() -> Arc<I18n> {
    Arc::clone(shared_cell())
}

/// [`shared`] without the clone, for the lookups that hand out text borrowed from it.
fn i18n() -> &'static I18n {
    shared_cell()
}

/// The languages offered, English first, then by code.
fn offered() -> &'static [Lang] {
    static OFFERED: OnceLock<Vec<Lang>> = OnceLock::new();
    OFFERED.get_or_init(|| i18n().languages())
}

/// What the app asks of a [`Lang`] besides what `craft-i18n` gives it.
pub trait LangExt: Sized {
    /// The language's code, lowercase BCP 47 (`ja`, `zh-hant`, `pt-br`). Also the
    /// `interfaceLanguage` value.
    fn code(self) -> &'static str;
    /// The language's name in itself, shown in the Preferences dropdown.
    fn name(self) -> &'static str;
    /// Does this language's catalog claim to cover every menu string and `tl!` literal? Every
    /// catalog but English does.
    fn complete_menus(self) -> bool;
    /// A language by its exact code, if this build offers it.
    fn from_code(code: &str) -> Option<Lang>;
    /// Resolve the `interfaceLanguage` preference: a language code, or `auto` (and anything
    /// unknown, e.g. a code from a newer version) to follow the system locale.
    fn from_pref(pref: &str) -> Lang;
    /// Every language available in this build.
    fn all() -> impl Iterator<Item = Lang>;
    /// Every available language in the order a picker lists them: English first (the source
    /// language and the fallback), then the rest by their own name with accents folded, so
    /// Čeština sits under C rather than after Z. Names in other scripts follow the Latin ones in
    /// code point order: Greek, Cyrillic, Arabic, then the CJK languages.
    fn for_picker() -> Vec<Lang>;
}

impl LangExt for Lang {
    fn code(self) -> &'static str {
        i18n().code(self)
    }

    fn name(self) -> &'static str {
        i18n().name(self)
    }

    fn complete_menus(self) -> bool {
        self != Lang::EN
    }

    fn from_code(code: &str) -> Option<Lang> {
        // The pseudo locale is a framework debugging aid; a saved `pseudo` must not select it.
        i18n().from_code(code).filter(|l| *l != Lang::PSEUDO)
    }

    fn from_pref(pref: &str) -> Lang {
        <Lang as LangExt>::from_code(pref).unwrap_or_else(system_lang)
    }

    fn all() -> impl Iterator<Item = Lang> {
        offered().iter().copied()
    }

    fn for_picker() -> Vec<Lang> {
        let mut rest: Vec<Lang> = offered().iter().copied().filter(|l| *l != Lang::EN).collect();
        rest.sort_by_cached_key(|l| name_sort_key(l.name()));
        std::iter::once(Lang::EN).chain(rest).collect()
    }
}

/// True when `tag` names the generic `C` / `POSIX` locale (or a variant like `C.UTF-8`), which
/// means "no particular language" rather than English: the system language list decides.
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
    *SYSTEM.get_or_init(|| {
        // `VECTORCRAFT_LOCALE` (a `:` list) overrides everything, also handy for screenshots.
        let forced = std::env::var("VECTORCRAFT_LOCALE")
            .ok()
            .and_then(|list| list.split(':').filter(|tag| !is_c_locale(tag)).find_map(|tag| i18n().match_tag(tag)));
        forced.unwrap_or_else(|| i18n().system_lang())
    })
}

/// Tell the UI the system's preferred languages, most preferred first, where it can't find them
/// itself: the web shell passes the browser's (`navigator.languages`). The first supported one is
/// the system language (English if none is); call it before the first frame. Has no effect once
/// the system language is known.
pub fn set_system_locales<S: AsRef<str>>(tags: &[S]) {
    let lang = i18n().match_tags(tags.iter().map(AsRef::as_ref));
    // Already set means already detected (or reported): the first answer stands.
    let _ = SYSTEM.set(lang);
}

/// Position in [`offered`] of the language the UI is drawn in this frame. The app keeps one
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
    let i = offered().iter().position(|l| *l == lang).unwrap_or(0);
    current_index().store(i, Ordering::Relaxed);
}

/// The language the UI is drawn in.
pub fn current() -> Lang {
    offered().get(current_index().load(Ordering::Relaxed)).copied().unwrap_or(Lang::EN)
}

/// Does `lang` have a catalog entry for this plain string? (English never does: it is the source.)
pub fn has(lang: Lang, s: &str) -> bool {
    i18n().get(lang, "", s).is_some()
}

/// Translate an English UI string into the current language ([`tr`] with [`current`]).
pub fn t(s: &str) -> &str {
    tr(current(), s)
}

/// The words of the native Replace question that a Save panel asks over an existing file, in the
/// current language.
#[cfg(not(target_arch = "wasm32"))]
pub fn replace_words() -> craft::dialogs::service::ReplaceWords {
    craft::dialogs::service::ReplaceWords {
        title: t("Replace").to_string(),
        question: t("“{name}” already exists. Replace it?").to_string(),
        replace: t("Replace").to_string(),
        cancel: t("Cancel").to_string(),
    }
}

/// `label`, already in the current language, followed by a field label's colon as the current
/// language writes it (its catalog row for ":"): French puts a no-break space before the colon, and
/// Chinese and Japanese write a full-width colon.
pub fn colon(label: &str) -> String {
    format!("{label}{}", t(":"))
}

/// Translate an English UI string; unknown strings come back unchanged.
pub fn tr(lang: Lang, s: &str) -> &str {
    i18n().t(lang, s)
}

/// Like [`tr`], for an English string that needs a disambiguating `context`.
pub fn tr_ctx<'a>(lang: Lang, context: &str, s: &'a str) -> &'a str {
    i18n().tc(lang, context, s)
}

/// The translation of `s` in `lang`'s `context` rows alone, or `None` when it has no such row.
pub fn tr_ctx_row<'a>(lang: Lang, context: &str, s: &'a str) -> Option<&'a str> {
    i18n().get(lang, context, s)
}

/// A name the app gives something (a default swatch's), translated where the current language has
/// a `context` row for it; any other name (one the user gave) comes back as it is.
pub fn t_name<'a>(context: &str, name: &'a str) -> &'a str {
    i18n().get(current(), context, name).unwrap_or(name)
}

/// A string keyed by its command id, falling back to the translation of the English `label`.
pub fn tr_id<'a>(lang: Lang, id: &str, label: &'a str) -> &'a str {
    i18n().tid(lang, id, label)
}

/// [`tr_id`] in the current language.
pub fn t_id<'a>(id: &str, label: &'a str) -> &'a str {
    tr_id(current(), id, label)
}

/// The Effect menu's label of effect `id` in `lang`: an `@id` row for the effect's id, else the
/// row of its English menu label. A plug-in effect's label is the plug-in's own text. `None` for
/// an id that names no effect. The Effect menu, History, Edit › Undo and Redo, the Appearance
/// panel and the effect dialogs all name an effect through this lookup.
pub fn effect_menu_label(lang: Lang, id: &str) -> Option<String> {
    match built_in_effect_label(id) {
        Some(label) => Some(tr_id(lang, id, label).to_string()),
        None => vectorcraft_plugins::effect::plugin_id(id).and_then(|_| vectorcraft_effects::effect_info(id)).map(|info| info.label.to_string()),
    }
}

/// [`effect_menu_label`] without its trailing ellipsis: an effect's name in `lang`.
pub fn effect_name(lang: Lang, id: &str) -> Option<String> {
    effect_menu_label(lang, id).map(|s| s.trim_end_matches('…').to_string())
}

/// Does `lang`'s catalog name built-in effect `id`: an `@id` row for the id, or a row for its
/// English menu label?
pub fn has_effect_name(lang: Lang, id: &str) -> bool {
    built_in_effect_label(id).is_some_and(|label| i18n().get(lang, "@id", id).is_some() || has(lang, label))
}

/// The English Effect menu label of built-in effect `id`.
fn built_in_effect_label(id: &str) -> Option<&'static str> {
    vectorcraft_effects::built_in_effect(id).map(|(_, label)| label)
}

/// Fill `{name}` placeholders in one pass: a value that holds `{name}` text stays as it is.
/// Unknown placeholders are left as written.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    fmt_plain(template, args)
}

/// A plural-aware message: `one`/`other` are the English forms (with `{n}` where the count goes).
/// The count is written in plain digits.
pub fn trn(lang: Lang, n: u64, one: &str, other: &str) -> String {
    i18n().trn_plain(lang, n, one, other)
}

/// An entry of a list that mixes interface labels with names (user, file or system data) as
/// shown: a built-in entry in `lang`, a name exactly as it is (a library the user calls "Layers"
/// stays "Layers"). Show the result with the non-translating widgets (`dropdown_names`,
/// `menu_item_name`, `dim_name`).
pub fn label_or_name(lang: Lang, s: &str, built_in: bool) -> &str {
    if built_in { tr(lang, s) } else { s }
}

/// A built-in name that may end in a number ("Warm Gray 3") in `lang`: its own row, or the row of
/// the name before the number followed by the number as it is. Any other name, a color value
/// such as `K=35` among them, comes back as it is.
pub fn numbered_label(lang: Lang, s: &str) -> Cow<'_, str> {
    let catalog = i18n();
    if let Some(shown) = catalog.get(lang, "", s) {
        return Cow::Borrowed(shown);
    }
    if let Some((head, n)) = s.rsplit_once(' ')
        && !n.is_empty()
        && n.bytes().all(|b| b.is_ascii_digit())
        && let Some(shown) = catalog.get(lang, "", head)
    {
        return Cow::Owned(format!("{shown} {n}"));
    }
    Cow::Borrowed(s)
}

/// Translate a status or error message for display (see the module docs). Unknown messages, and
/// parts of them (file names, numbers, a reason with no entry, a JSON object or array such as
/// `{"objects": [...]}`, a value the message quotes in `"…"`, `“…”`, `'…'` or backticks, such as a
/// symbol's or a script property's name, and an `{ident}` placeholder's value, a script identifier),
/// stay as they are. A `{to}` placeholder's value that is not quoted, a printer's name such as
/// `Office Laser (Floor 2)`, is translated only when it has an exact entry, such as
/// `the default printer`. A part that starts with `{` is translated only when it is a whole message
/// with an exact entry. A part that starts with `[` is translated when it has an exact entry, such as
/// `[x, y] (two numbers)`, or when it fits a template that starts with `[` and an index of digits,
/// such as an effect's refusal ``[0] `id`: {why}``.
pub fn message(lang: Lang, s: &str) -> String {
    // The hook sees only values that are not between quotes; a quoted value stays as written.
    i18n().message_with(lang, s, &|m, key, value| match key {
        "kinds" => left_out_kinds(lang, s, m.captures),
        "_1" | "_2" => liquify_skip_values(lang, s, m.captures).map(|(tool, kinds)| if key == "_1" { tool } else { kinds }),
        "ident" => Some(value.to_string()),
        "to" => Some(i18n().message_exact(lang, value).unwrap_or_else(|| value.to_string())),
        _ => None,
    })
}

/// The kinds the warning about an SVG's variables that aren't text were left out (the engine's
/// `left_out_kind` in `crates/engine/src/cmd/variablecmds/library.rs`), each translated by its
/// `variable-kind` row.
const LEFT_OUT_KINDS: [&str; 4] = ["Visibility", "Linked File", "Graph Data", "Unknown"];

/// The list of kinds in `lang` when `s` is the warning about an SVG's variables that aren't text
/// and `caps` are its captures. Each entry is a kind of [`LEFT_OUT_KINDS`] or a `trait` the file
/// names, in backticks with each backtick in it doubled, followed by its count in parentheses; the
/// entries are separated by `, `. A kind of [`LEFT_OUT_KINDS`] is shown in `lang`, a `trait` as the
/// warning writes it, and the entries keep their order and counts, joined with the
/// `variable-list-two` row for two, and the `variable-list-middle` and `variable-list-end` rows for
/// more; a missing row leaves its part in English. `None` for any other message, or a list that
/// does not have that form.
fn left_out_kinds(lang: Lang, s: &str, caps: &[(&str, &str)]) -> Option<String> {
    let [("kinds", kinds)] = caps else { return None };
    if s.strip_prefix("variables that aren't text were left out: ") != Some(*kinds) {
        return None;
    }
    let catalog = i18n();
    let mut entries = Vec::new();
    let mut rest = *kinds;
    loop {
        let (name, after) = if let Some(quoted) = rest.strip_prefix('`') {
            // The `trait` ends at the first backtick that is not doubled.
            let mut at = 0;
            let close = loop {
                let found = at + quoted[at..].find('`')?;
                if quoted[found + 1..].starts_with('`') {
                    at = found + 2;
                } else {
                    break found;
                }
            };
            rest.split_at(close + 2)
        } else {
            let kind = *LEFT_OUT_KINDS.iter().find(|k| rest.starts_with(**k))?;
            (catalog.get(lang, "variable-kind", kind).unwrap_or(kind), &rest[kind.len()..])
        };
        let (count, after) = after.strip_prefix(" (")?.split_once(')')?;
        if count.is_empty() || !count.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        entries.push(format!("{name} ({count})"));
        if after.is_empty() {
            break;
        }
        rest = after.strip_prefix(", ")?;
    }
    let join = |context: &str, a: &str, b: &str| {
        let pattern = "{0}, {1}";
        fmt_plain(catalog.get(lang, context, pattern).unwrap_or(pattern), &[("0", a), ("1", b)])
    };
    match entries.as_slice() {
        [] => None,
        [one] => Some(one.clone()),
        [a, b] => Some(join("variable-list-two", a, b)),
        [first, middle @ .., last] => {
            let head = middle.iter().fold(first.clone(), |acc, w| join("variable-list-middle", &acc, w));
            Some(join("variable-list-end", &head, last))
        }
    }
}

/// The tool labels in Liquify's warning about the objects under the brush that it left as they
/// were, each translated by its `liquify-tool` row.
const LIQUIFY_TOOLS: [&str; 7] = ["Warp", "Twirl", "Pucker", "Bloat", "Scallop", "Crystallize", "Wrinkle"];

/// The kinds of object that warning lists (the engine's `not_liquified` in
/// `crates/engine/src/cmd/distortcmds.rs`), each translated by its `liquify-kind` row.
const LIQUIFY_KINDS: [&str; 9] = ["graphs", "type", "symbols", "images", "meshes", "envelopes", "repeats", "placed documents", "blends"];

/// The tool and the list of kinds in `lang` when `s` is Liquify's warning about the objects under
/// the brush that it left as they were and `caps` are its captures. The kinds keep their order and
/// are joined with the `liquify-list-two` row for two, and the `liquify-list-middle` and
/// `liquify-list-end` rows for more; a missing row leaves its part in English. `None` for any
/// other message, or a tool or kind that is not one of Liquify's.
fn liquify_skip_values(lang: Lang, s: &str, caps: &[(&str, &str)]) -> Option<(String, String)> {
    let cap = |name: &str| caps.iter().find(|(k, _)| *k == name).map(|(_, v)| *v);
    let (tool, n, kinds) = (cap("_1")?, cap("n")?, cap("_2")?);
    let english = if n == "1" {
        format!("{tool} left {n} object under the brush as it was ({kinds}): it reshapes paths only")
    } else {
        format!("{tool} left {n} objects under the brush as they were ({kinds}): it reshapes paths only")
    };
    if caps.len() != 3 || english != s || !LIQUIFY_TOOLS.contains(&tool) {
        return None;
    }
    let catalog = i18n();
    let words = kinds
        .split(", ")
        .map(|k| LIQUIFY_KINDS.contains(&k).then(|| catalog.get(lang, "liquify-kind", k).unwrap_or(k)))
        .collect::<Option<Vec<&str>>>()?;
    let join = |context: &str, a: &str, b: &str| {
        let pattern = "{0}, {1}";
        fmt_plain(catalog.get(lang, context, pattern).unwrap_or(pattern), &[("0", a), ("1", b)])
    };
    let list = match words.as_slice() {
        [] => return None,
        [one] => (*one).to_string(),
        [a, b] => join("liquify-list-two", a, b),
        [first, middle @ .., last] => {
            let head = middle.iter().fold((*first).to_string(), |acc, w| join("liquify-list-middle", &acc, w));
            join("liquify-list-end", &head, last)
        }
    };
    Some((catalog.get(lang, "liquify-tool", tool).unwrap_or(tool).to_string(), list))
}

/// The last answer of [`msg`] or [`msg_keeping`] for `key` in the current language: the status bar
/// asks for the same message every frame. The memo holds every input the answer reads.
fn remembered(key: &str, make: impl FnOnce(Lang) -> String) -> String {
    static LAST: Mutex<Option<(Lang, String, String)>> = Mutex::new(None);
    let lang = current();
    let mut last = LAST.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((l, k, out)) = last.as_ref()
        && *l == lang
        && k == key
    {
        return out.clone();
    }
    let out = make(lang);
    *last = Some((lang, key.to_string(), out.clone()));
    out
}

/// [`message`] in the current language, remembering the last answer.
pub fn msg(s: &str) -> String {
    remembered(s, |lang| message(lang, s))
}

/// The message `before`, `kept` and `after` make, with `before` and `after` translated as
/// [`message`] translates a message and `kept` (such as text a script threw) shown as it is, even
/// when it reads like a message that has a translation. When the translation does not hold the
/// place of `kept` exactly once, the message is shown in English.
pub fn message_keeping(lang: Lang, before: &str, kept: &str, after: &str) -> String {
    i18n().message_keeping(lang, before, kept, after)
}

/// [`message_keeping`] in the current language, remembering the last translation as [`msg`] does.
pub fn msg_keeping(before: &str, kept: &str, after: &str) -> String {
    // U+FFFC cannot start a message, so the key never equals one [`msg`] remembers.
    remembered(&format!("\u{FFFC}{before}\u{FFFC}{kept}\u{FFFC}{after}"), |lang| message_keeping(lang, before, kept, after))
}

/// [`trn`] in the current language.
pub fn tn(n: u64, one: &str, other: &str) -> String {
    trn(current(), n, one, other)
}

/// A status or error message with a count, in English: `one` for a count of 1, else `other`, with
/// `{n}` and the `args` placeholders filled in one pass, so a value that holds `{name}` text stays
/// as it is. Unknown placeholders are left as written. The message stays English, and [`msg`]
/// shows it in the interface language through the catalogs' `@msgplural` rows.
pub fn msg_n(n: u64, one: &str, other: &str, args: &[(&str, &str)]) -> String {
    let count = n.to_string();
    let all: Vec<(&str, &str)> = std::iter::once(("n", count.as_str())).chain(args.iter().copied()).collect();
    fmt(if n == 1 { one } else { other }, &all)
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
