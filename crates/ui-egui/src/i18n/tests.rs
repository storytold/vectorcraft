use super::*;

const ZH: fn() -> Lang = || lang("zh-hant");

/// The language with `code`.
fn lang(code: &str) -> Lang {
    Lang::from_code(code).unwrap_or_else(|| panic!("{code} registered"))
}

/// The languages that are not English, the source language.
fn translated() -> Vec<Lang> {
    Lang::all().filter(|l| *l != Lang::EN).collect()
}

/// (language, context, English source, the term the language's glossary gives it). One row per
/// rule: a term for a concept, a punctuation mark's typography.
const TERMS: &[(&str, &str, &str, &str)] = &[
    ("gu", "", "Save", "સાચવો"),
    ("hi", "", "Save", "सेव"),
    ("hi", "", "Save As…", "सेव ऐज़…"),
    ("hi", "", "Path", "पाथ"),
    ("hi", "", "Selection", "सिलेक्शन"),
    ("zh-hant", "", "Layers", "圖層"),
    ("el", "", "Knockout Group", "Ομάδα διάτρησης"),
    ("el", "", "Page Knockout Group", "Ομάδα διάτρησης σελίδας"),
    ("el", "", "Clip Group", "Ομάδα αποκοπής"),
    ("pl", "", "Artwork", "Kompozycja"),
    ("pl", "", "Off-Artboard Art", "Kompozycja poza obszarami kompozycji"),
    ("pl", "", "Art Brush", "Pędzel artystyczny"),
    ("pl", "", "Line Art and Text Resolution:", "Rozdzielczość grafiki liniowej i tekstu:"),
    ("uk", "", "Color Library", "Бібліотека кольорів"),
    ("uk", "", "Color Library…", "Бібліотека кольорів…"),
    // A field label's colon: French puts a no-break space before it, Chinese and Japanese write a
    // full-width colon.
    ("fr", "", ":", "\u{a0}:"),
    ("zh-hans", "", ":", "："),
    ("zh-hant", "", ":", "："),
    ("ja", "", ":", "："),
];

/// A language's rows name a concept with the term its glossary gives it, whatever the English
/// around it says.
#[test]
fn rows_use_the_terms_of_their_language() {
    let wrong: Vec<String> = TERMS
        .iter()
        .filter_map(|(code, context, source, term)| {
            let shown = tr_ctx(lang(code), context, source);
            (shown != *term).then(|| format!("{code}: {source:?} shows {shown:?}, not {term:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{} rows use the wrong term:\n{}", wrong.len(), wrong.join("\n"));
}

/// Strings of the features' dialogs and panels by family; every language but English has a row for
/// each, so no feature ships with a gap in one catalog. [`untranslated_report`] lists the rest.
const UI_STRINGS: &[(&str, &[&str])] = &[
    (
        "font height reference",
        &["Show Font Height Options", "Hide Font Height Options", "Set Font Height Reference", "Em Box", "Cap Height", "x-Height", "ICF Box"],
    ),
    ("attributes panel options", &["Attributes Panel Options", "Number of Recent URLs:"]),
    (
        "proof setup",
        &[
            "Proof Setup",
            "Device to Simulate:",
            "Preserve CMYK Numbers",
            "Preserve RGB Numbers",
            "Rendering intent:",
            "Simulate Paper Color",
            "Simulate Black Ink",
            "Save…",
            "Load…",
        ],
    ),
    ("nature icons library", &["Nature Icons", "Leaf", "Flower", "Tree", "Mountain", "Wave"]),
    ("objects on path", &["Detach", "Detach Object from Path"]),
    (
        "document info image details",
        &[
            "{format}; {size}; {detail}",
            "{x} × {y} ppi",
            "resolution unavailable",
            "estimated memory {memory}",
            "memory unavailable",
            "Unknown format",
        ],
    ),
    (
        "type preferences",
        &[
            "Start Lists From Typed Markers",
            "Show Character Alternates",
            "Click to replace",
            "Show all alternates in the Glyphs panel",
            "Low-Latency Drawing",
        ],
    ),
];

#[test]
fn strings_the_ui_shows_are_in_every_complete_catalog() {
    let mut missing = vec![];
    for (family, keys) in UI_STRINGS {
        for lang in translated() {
            missing.extend(keys.iter().filter(|key| !has(lang, key)).map(|key| format!("{}: {family}: {key}", lang.code())));
        }
    }
    assert!(missing.is_empty(), "{} rows missing:\n{}", missing.len(), missing.join("\n"));
}

/// (English message, the languages that translate it (empty: every language but English), a text
/// the translation keeps, a text of the English it leaves out).
const MESSAGES: &[(&str, &[&str], &str, &str)] = &[
    ("Packaged 1 file in /out/card Folder", &[], "/out/card", "Packaged"),
    ("Packaged 3 files", &[], "", "Packaged"),
    ("Packaged 1 file (fonts not copied: Mono) in /out/card (draft) v2", &[], "Mono", "Packaged"),
    ("command `object.group` is not available right now: nothing selected", &["es"], "`object.group`", "nothing selected"),
];

/// A message is shown in each language with its names kept and its English parts translated, and
/// a count takes the language's plural form.
#[test]
fn messages_translate_in_every_complete_language() {
    let mut wrong = vec![];
    for (english, codes, kept, gone) in MESSAGES {
        assert_eq!(message(Lang::EN, english), *english);
        let languages: Vec<Lang> = if codes.is_empty() { translated() } else { codes.iter().map(|c| lang(c)).collect() };
        for lang in languages {
            let shown = message(lang, english);
            if shown == *english || !shown.contains(kept) || shown.contains(gone) {
                wrong.push(format!("{}: {english:?} shows {shown:?}", lang.code()));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    // A count picks its form; the path keeps its whole text.
    let saved = |n: usize| crate::io::saved_status("/out/art with notes.pdf", &serde_json::json!({ "warnings": vec!["a note"; n] }));
    assert_eq!(message(lang("de"), &saved(1)), "/out/art with notes.pdf mit 1 Hinweis gespeichert: a note");
    assert_eq!(message(lang("de"), &saved(2)), "/out/art with notes.pdf mit 2 Hinweisen gespeichert: a note");
}

/// Gujarati has no face to show it on the web unless one is bundled, so the web does not offer
/// it then; every other build offers it.
#[test]
fn gujarati_web_selection_requires_a_bundled_interface_font() {
    let codes = |web, font| {
        let (i18n, problems) = I18n::new(EMBEDDED, &offered_codes(web, font));
        assert!(problems.is_empty(), "{problems:?}");
        i18n.languages().into_iter().map(|l| i18n.code(l).to_string()).collect::<Vec<_>>()
    };
    let gu = "gu".to_string();
    assert!(!codes(true, false).contains(&gu));
    assert!(codes(true, true).contains(&gu));
    assert!(codes(false, false).contains(&gu));
    assert!(codes(true, false).contains(&"hi".to_string()));
}

/// A numbered built-in name shows its name before the number translated and the number as it
/// is; a name with a row of its own uses that row, and any other name comes back unchanged.
#[test]
fn numbered_labels_translate_the_name_before_the_number() {
    let fr = lang("fr");
    assert_eq!(numbered_label(fr, "Grays"), tr(fr, "Grays"));
    assert_ne!(tr(fr, "Grays"), "Grays");
    assert_eq!(numbered_label(fr, "Grays 12"), format!("{} 12", tr(fr, "Grays")));
    for kept in ["K=35", "#33cc99", "Grays x", "Grays ", "Not A Catalog Name 3"] {
        assert_eq!(numbered_label(fr, kept), kept);
    }
    assert_eq!(numbered_label(Lang::EN, "Grays 12"), "Grays 12");
}

#[test]
fn current_language_is_a_frame_setting() {
    set_current(ZH());
    assert_eq!(current(), ZH());
    assert_eq!(t("Layers"), "圖層");
    set_current(Lang::EN);
    assert_eq!(current(), Lang::EN);
    assert_eq!(t("Layers"), "Layers");
}

/// Unit tests run on parallel threads, and each one draws in its own language: a language set on
/// another thread (by a test running beside this one) leaves this thread's text in English.
#[test]
fn a_language_set_by_another_test_leaves_this_one_in_english() {
    std::thread::spawn(|| set_current(ZH())).join().unwrap();
    assert_eq!(current(), Lang::EN);
    assert_eq!(t("Layers"), "Layers");
}

/// Every bundled catalog is well-formed, and every command id a row names is a command.
#[test]
fn bundled_catalogs_are_consistent() {
    let problems = shared().check_all();
    assert!(problems.is_empty(), "{problems:#?}");
    for (code, _, app) in EMBEDDED {
        for line in app.lines() {
            let mut columns = line.split('\t');
            let (Some("@id"), Some(id)) = (columns.next(), columns.next()) else { continue };
            // A command id, or an effect id: Effect menu items share `effect.apply` /
            // `effect.dialog`, so the effect's own id is what tells them apart.
            let known = id == "language.name"
                || crate::menus::UI_COMMANDS.iter().any(|c| c.0 == id)
                || vectorcraft_engine::cmd::command_specs().iter().any(|c| c.id == id)
                || vectorcraft_effects::effect_catalog().iter().any(|e| e.id == id);
            assert!(known, "{code}: unknown command id {id:?}");
        }
    }
}

/// The engine learns the language the UI is drawn in each frame: new type takes the Japanese
/// defaults while it is Japanese (#432). A Japanese frame draws in Japanese on this test's thread
/// only: the tests running beside it on other threads still draw in English.
#[test]
fn the_engine_follows_the_interface_language() {
    let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let ctx = crate::testing::ui_context();
    let frame = |app: &mut crate::VectorcraftApp| {
        ctx.run_ui(egui::RawInput::default(), |ui| app.tick(ui.ctx())).textures_delta.clear();
    };
    frame(&mut app);
    assert_eq!(app.session.ui_language.as_deref(), Some("en"), "tests resolve `auto` to English");
    assert!(!app.session.japanese_interface());
    app.session.prefs.interface_language = "ja".into();
    frame(&mut app);
    assert_eq!(app.session.ui_language.as_deref(), Some("ja"));
    assert!(app.session.japanese_interface());
    assert_eq!(current().code(), "ja");
    // A test running beside this one on another thread still draws in English.
    assert_eq!(std::thread::spawn(|| (current(), t("Layers"))).join().unwrap(), (Lang::EN, "Layers"));
    // With the preference back on `auto`, the next frame hands the engine `en` again.
    app.session.prefs.interface_language = "auto".into();
    frame(&mut app);
    assert_eq!(app.session.ui_language.as_deref(), Some("en"));
    assert!(!app.session.japanese_interface());
    assert_eq!(current(), Lang::EN);
}

/// `app.language` sets the `interfaceLanguage` preference, which is what persists; the checked
/// item follows the preference, and a bad code is an error, not a change.
#[test]
fn language_command_sets_the_preference() {
    use crate::menus::{checked, run_ui_command};
    let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    assert_eq!(checked(&app, "app.language", &serde_json::json!({"lang": "auto"})), Some(true));
    run_ui_command(&mut app, "app.language", &serde_json::json!({"lang": "ja"})).unwrap().unwrap();
    assert_eq!(app.session.prefs.interface_language, "ja");
    assert_eq!(app.ui_language(), Lang::from_code("ja").unwrap());
    assert_eq!(checked(&app, "app.language", &serde_json::json!({"lang": "ja"})), Some(true));
    assert_eq!(checked(&app, "app.language", &serde_json::json!({"lang": "auto"})), Some(false));
    assert_eq!(checked(&app, "app.language", &serde_json::json!({"lang": "zh-hant"})), Some(false));
    let err = run_ui_command(&mut app, "app.language", &serde_json::json!({"lang": "xx"})).unwrap().unwrap_err();
    assert!(err.contains("auto") && err.contains("zh-hant"), "{err}");
    assert_eq!(app.session.prefs.interface_language, "ja");
    run_ui_command(&mut app, "app.language", &serde_json::json!({"lang": "auto"})).unwrap().unwrap();
    assert_eq!(app.session.prefs.interface_language, "auto");
    assert_eq!(checked(&app, "app.language", &serde_json::json!({"lang": "auto"})), Some(true));
    // Every registered language is in the Mac App menu's Language, by its own name (the in-window
    // bar has no Language menu).
    crate::menus::model::refresh(&mut app, &crate::testing::ui_context());
    let layout = craft::menu::mac_layout(&app.menus.bar, &crate::menus::native::options());
    let app_menu = layout.bar.menus.into_iter().find(|m| m.title == "VectorCraft").expect("App menu");
    fn items(nodes: &[crate::menus::Node], out: &mut Vec<String>) {
        for n in nodes {
            match n {
                crate::menus::Node::Item(it) if it.command.id == "app.language" => out.push(it.label.clone()),
                crate::menus::Node::Submenu { children, .. } => items(children, out),
                _ => {}
            }
        }
    }
    let mut labels = vec![];
    items(&app_menu.children, &mut labels);
    for l in Lang::all() {
        assert!(labels.iter().any(|x| x == l.name()), "{} missing from {labels:?}", l.code());
    }
}

/// Every `tl!("…")` literal in this crate's sources (test modules aside).
pub fn tl_literals() -> std::collections::BTreeSet<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut literals = std::collections::BTreeSet::new();
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if name != "tests" {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") && name != "lib.rs" && !name.starts_with("tests") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                // Code up to the first test item; test-only imports (`#[cfg(test)] use …;`) don't end it.
                let code = text
                    .match_indices("#[cfg(test)]")
                    .find(|&(at, m)| {
                        let item = text[at + m.len()..].trim_start();
                        let item = ["pub(crate) ", "pub(super) ", "pub "].iter().find_map(|p| item.strip_prefix(p)).unwrap_or(item);
                        !item.starts_with("use ")
                    })
                    .map_or(text.as_str(), |(at, _)| &text[..at]);
                let mut rest = code;
                while let Some(at) = rest.find("tl!(\"") {
                    rest = &rest[at + 5..];
                    let mut end = 0;
                    let bytes = rest.as_bytes();
                    while end < bytes.len() && !(bytes[end] == b'"' && (end == 0 || bytes[end - 1] != b'\\')) {
                        end += 1;
                    }
                    let lit = rest.get(..end).unwrap_or("").replace("\\\"", "\"");
                    if rest.get(end + 1..end + 2) == Some(")") {
                        literals.insert(lit);
                    }
                }
            }
        }
    }
    literals
}

/// Crates whose undo steps the History panel lists.
const UNDO_CRATES: &[&str] = &["engine", "tools", "ui-egui"];

/// Calls whose first argument names an undo step.
const UNDO_MARKERS: &[&str] = &[".edit(", "begin_interaction(", "Action::Begin(", "begin_named_undo_group(Some("];

/// The call whose first argument is the template of an undo step's name (`Session::edit_named`).
const NAMED_MARKER: &str = ".edit_named(";

/// A function in a source file, as [`undo_labels`] reads it.
struct SourceFn<'a> {
    file: usize,
    name: &'a str,
    /// Declared `pub` (`pub(crate)`, `pub(super)`), so other files call it too.
    public: bool,
    /// Position of its `label` parameter among a call's arguments (`self` not counted).
    label: Option<usize>,
    /// Takes `self`: called as `.name(`, so a free function of the same name (`background::run`) is not it.
    method: bool,
    body: &'a str,
}

/// Every undo step name in [`UNDO_CRATES`] (test modules aside) that the sources write as a
/// literal: the first argument at each of [`UNDO_MARKERS`], the argument a function whose `label`
/// parameter reaches one of them takes at each of its calls, and the value of each `let label = …`
/// in a function that passes `label` on. A literal counts where it is the value (`"Move"`,
/// `if copy { "Copy" } else { "Move" }`, a match arm's result). A call of a function named
/// `…label` (`pf_label(op)`, `kind.label()`) gives the literals that function returns in the same
/// file. A name built with `format!` or taken from data (an effect's, a plug-in's) is not read.
pub fn undo_labels() -> std::collections::BTreeSet<String> {
    undo_scan().labels
}

/// What [`undo_scan`] finds in [`UNDO_CRATES`].
struct UndoScan {
    /// The step names written as literals ([`undo_labels`]).
    labels: std::collections::BTreeSet<String>,
    /// The templates step names are made from with `Session::edit_named` (its first argument).
    templates: std::collections::BTreeSet<String>,
    /// The format strings of the step names built with `format!` where [`undo_labels`] reads a
    /// name.
    formatted: std::collections::BTreeSet<String>,
}

/// The undo step names and templates in [`UNDO_CRATES`] (see [`undo_labels`]).
fn undo_scan() -> UndoScan {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = std::collections::BTreeSet::new();
    let mut templates = std::collections::BTreeSet::new();
    let mut formatted = std::collections::BTreeSet::new();
    for krate in UNDO_CRATES {
        let files = undo_sources(&crates.join(krate).join("src"));
        let fns: Vec<SourceFn> = files.iter().enumerate().flat_map(|(i, t)| source_fns(i, t)).collect();
        // Functions whose `label` parameter reaches a marker, directly or through one another.
        let mut forwards = vec![false; fns.len()];
        loop {
            let before = forwards.clone();
            for (i, f) in fns.iter().enumerate().filter(|(_, f)| f.label.is_some()) {
                let passed = |g: &SourceFn| call_args(f.body, g).iter().any(|a| bare(a) == "label");
                forwards[i] |= marker_args(f.body).iter().any(|a| bare(a) == "label") || fns.iter().zip(&before).any(|(g, &on)| on && passed(g));
            }
            if forwards == before {
                break;
            }
        }
        let forwarders: Vec<&SourceFn> = fns.iter().zip(&forwards).filter(|(_, on)| **on).map(|(f, _)| f).collect();
        // Each expression that gives a step's name, with the file it is in.
        let mut exprs: Vec<(usize, &str)> = vec![];
        for (i, t) in files.iter().enumerate() {
            exprs.extend(marker_args(t).into_iter().map(|a| (i, a)));
            for g in forwarders.iter().filter(|g| g.public || g.file == i) {
                exprs.extend(call_args(t, g).into_iter().map(|a| (i, a)));
            }
        }
        for f in &fns {
            let mut passed = marker_args(f.body);
            passed.extend(forwarders.iter().flat_map(|g| call_args(f.body, g)));
            if !passed.iter().any(|a| bare(a).starts_with("label")) {
                continue;
            }
            for (at, m) in f.body.match_indices("let label =") {
                let rest = &f.body[at + m.len()..];
                exprs.push((f.file, rest[..top_level(rest).1].trim()));
            }
        }
        for (file, e) in exprs {
            label_literals(e, file, &fns, &mut out, &mut formatted);
        }
        for t in &files {
            let named = t.match_indices(NAMED_MARKER).filter_map(|(at, _)| arguments(&t[at + NAMED_MARKER.len()..]).first().copied());
            templates.extend(named.filter_map(|a| a.strip_prefix('"').and_then(string_literal)));
        }
    }
    UndoScan { labels: out, templates, formatted }
}

/// The sources of a crate's `src` directory, test modules and files and comments aside.
fn undo_sources(dir: &std::path::Path) -> Vec<String> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if name != "tests" {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") && !name.starts_with("tests") && !name.ends_with("_tests.rs") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                out.push(without_comments(without_test_module(&text)));
            }
        }
    }
    out
}

/// `text` with each comment removed: a `//` comment, and a `/* … */` comment (nested ones
/// included), which becomes a space. A `//` or `/*` inside a string or character literal stays.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(c) = text[i..].chars().next() {
        if let Some(end) = (c == '"' || c == '\'').then(|| quoted_end(text, i)).flatten() {
            out.push_str(&text[i..end]);
            i = end;
        } else if text[i..].starts_with("//") {
            i = text[i..].find('\n').map_or(text.len(), |n| i + n);
        } else if text[i..].starts_with("/*") {
            i = block_comment_end(text, i);
            out.push(' ');
        } else {
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// Where the `/* … */` comment starting at `text[at]` ends, past the comments nested in it; the
/// end of `text` when the comment is not closed.
fn block_comment_end(text: &str, at: usize) -> usize {
    let (mut depth, mut i) = (0usize, at);
    while let Some(c) = text[i..].chars().next() {
        if text[i..].starts_with("/*") {
            depth += 1;
            i += 2;
        } else if text[i..].starts_with("*/") {
            depth = depth.saturating_sub(1);
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += c.len_utf8();
        }
    }
    text.len()
}

/// Where the string or character literal starting at `text[at]` (a `"` or `'`) ends, raw strings
/// (`r#"…"#`) included; `None` for a lifetime's `'`.
fn quoted_end(text: &str, at: usize) -> Option<usize> {
    let rest = &text[at + 1..];
    if text[at..].starts_with('\'') {
        let mut it = rest.char_indices();
        let (_, c) = it.next()?;
        if c == '\\' {
            it.next()?;
            return it.find(|(_, c)| *c == '\'').map(|(n, _)| at + 1 + n + 1);
        }
        return it.next().filter(|(_, c)| *c == '\'').map(|(n, _)| at + 1 + n + 1);
    }
    let hashes = text[..at].chars().rev().take_while(|c| *c == '#').count();
    if text[..at - hashes].ends_with('r') {
        let close = format!("\"{}", "#".repeat(hashes));
        return rest.find(&close).map(|n| at + 1 + n + close.len());
    }
    let mut it = rest.char_indices();
    while let Some((n, c)) = it.next() {
        match c {
            '\\' => {
                it.next();
            }
            '"' => return Some(at + 1 + n + 1),
            _ => {}
        }
    }
    None
}

/// In `s` (the text after an opening bracket), the offset of each top-level `,`, and where the
/// list ends: at the bracket that closes it, a top-level `;`, or the end of `s`.
fn top_level(s: &str) -> (Vec<usize>, usize) {
    let (mut commas, mut depth, mut i) = (vec![], 0usize, 0);
    while let Some(c) = s[i..].chars().next() {
        if let Some(end) = (c == '"' || c == '\'').then(|| quoted_end(s, i)).flatten() {
            i = end;
            continue;
        }
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => return (commas, i),
            ')' | ']' | '}' => depth -= 1,
            ';' if depth == 0 => return (commas, i),
            ',' if depth == 0 => commas.push(i),
            _ => {}
        }
        i += c.len_utf8();
    }
    (commas, s.len())
}

/// The length of the block at the start of `s` (which starts with `{`), braces included.
fn block_len(s: &str) -> usize {
    let mut at = 1;
    while at < s.len() {
        let end = at + top_level(&s[at..]).1;
        if !s[end..].starts_with(';') {
            return (end + 1).min(s.len());
        }
        at = end + 1;
    }
    s.len()
}

/// The arguments of the call whose `(` comes just before `s`, trimmed.
fn arguments(s: &str) -> Vec<&str> {
    let (commas, end) = top_level(s);
    let mut out = vec![];
    let mut start = 0;
    for at in commas.into_iter().chain([end]) {
        out.push(s[start..at].trim());
        start = at + 1;
    }
    out
}

/// The functions defined in a source file, with their bodies.
fn source_fns(file: usize, text: &str) -> Vec<SourceFn<'_>> {
    let mut out = vec![];
    for (at, _) in text.match_indices("fn ") {
        if text[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let after = &text[at + 3..];
        let name_len = after.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(after.len());
        let (name, mut rest) = after.split_at(name_len);
        if rest.starts_with('<') {
            let mut depth = 0;
            let close = rest
                .char_indices()
                .find(|&(n, c)| {
                    depth += match c {
                        '<' => 1,
                        '>' if !rest[..n].ends_with('-') => -1,
                        _ => 0,
                    };
                    depth == 0
                })
                .map(|(n, _)| n);
            rest = close.map_or("", |n| &rest[n + 1..]);
        }
        let Some(params) = rest.strip_prefix('(') else { continue };
        let names: Vec<&str> =
            arguments(params).into_iter().map(|p| p.split(':').next().unwrap_or("").trim().trim_start_matches("mut ").trim()).collect();
        let after_params = &params[top_level(params).1..];
        let (Some(open), semi) = (after_params.find('{'), after_params.find(';')) else { continue };
        if semi.is_some_and(|s| s < open) {
            continue;
        }
        let body = &after_params[open..];
        let body = &body[..block_len(body)];
        let skip = usize::from(names.first().is_some_and(|n| n.ends_with("self")));
        let line = &text[text[..at].rfind('\n').map_or(0, |n| n + 1)..at];
        let label = names.iter().position(|n| *n == "label").map(|k| k - skip);
        out.push(SourceFn { file, name, public: line.contains("pub"), label, method: skip == 1, body });
    }
    out
}

/// The first argument at each of [`UNDO_MARKERS`] in `text`.
fn marker_args(text: &str) -> Vec<&str> {
    UNDO_MARKERS.iter().flat_map(|m| text.match_indices(m).filter_map(move |(at, _)| arguments(&text[at + m.len()..]).first().copied())).collect()
}

/// The argument at `g`'s `label` position at each call of `g` in `text`.
fn call_args<'a>(text: &'a str, g: &SourceFn) -> Vec<&'a str> {
    let Some(k) = g.label else { return vec![] };
    let call = format!("{}(", g.name);
    text.match_indices(&call)
        .filter(|(at, _)| !text[..*at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') && !text[..*at].ends_with("fn "))
        .filter(|(at, _)| text[..*at].ends_with('.') == g.method)
        .filter_map(|(at, _)| arguments(&text[at + call.len()..]).get(k).copied())
        .collect()
}

/// An expression without a leading `&` and trailing conversions (`.into()`, `.to_string()`).
fn bare(e: &str) -> &str {
    let mut e = e.trim().trim_start_matches('&');
    while let Some(s) = [".into()", ".to_string()", ".clone()", ".as_str()", ".to_owned()"].iter().find_map(|t| e.strip_suffix(t)) {
        e = s.trim_end();
    }
    e
}

/// Add the step names expression `e` in source file `file` gives (see [`undo_labels`]) to `out`,
/// and the format string of a name it builds with `format!` to `formatted`.
fn label_literals(
    e: &str,
    file: usize,
    fns: &[SourceFn],
    out: &mut std::collections::BTreeSet<String>,
    formatted: &mut std::collections::BTreeSet<String>,
) {
    let mut e = bare(e);
    if let Some(args) = e.strip_prefix("format!(") {
        formatted.extend(args.trim_start().strip_prefix('"').and_then(string_literal));
        return;
    }
    if let Some(body) = e.strip_prefix('|').and_then(|r| r.find('|').map(|n| &r[n + 1..])) {
        e = body.trim();
    }
    let values = string_literals(e, |before, after| {
        let before = before.trim_end();
        let value = before.is_empty() || ["{", "=>", "else", "Some(", "unwrap_or("].iter().any(|t| before.ends_with(t));
        value && !after.trim_start().starts_with("=>") && !after.trim_start().starts_with('|')
    });
    if !values.is_empty() {
        out.extend(values);
        return;
    }
    let Some(inner) = e.strip_suffix(')') else { return };
    let Some(open) = inner.rfind('(').filter(|n| !inner[n + 1..].contains(')')) else { return };
    let name_at = inner[..open].rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |n| n + 1);
    let name = &inner[name_at..open];
    let method = inner[..name_at].ends_with('.');
    if name.ends_with("label") && (name != "label" || method) {
        for f in fns.iter().filter(|f| f.file == file && f.name == name) {
            out.extend(string_literals(f.body, |_, after| !after.trim_start().starts_with("=>") && !after.trim_start().starts_with('|')));
        }
    }
}

/// The string literals in `e` that `keep` accepts, given the text before and after each.
fn string_literals(e: &str, keep: impl Fn(&str, &str) -> bool) -> Vec<String> {
    let mut out = vec![];
    let mut i = 0;
    while let Some(c) = e[i..].chars().next() {
        if let Some(end) = (c == '"' || c == '\'').then(|| quoted_end(e, i)).flatten() {
            if c == '"'
                && let Some(lit) = string_literal(&e[i + 1..])
                && keep(&e[..i], &e[end..])
            {
                out.push(lit);
            }
            i = end;
        } else {
            i += c.len_utf8();
        }
    }
    out
}

/// For translators, `cargo test -p vectorcraft-ui-egui dump_source_strings` writes every English
/// interface string to `VECTORCRAFT_I18N_DUMP=<file>`, and every status and error message as a
/// catalog template that [`untranslated_report`] checks to `VECTORCRAFT_I18N_DUMP_MESSAGES=<file>`,
/// one per line.
#[test]
fn dump_source_strings() {
    if let Ok(path) = std::env::var("VECTORCRAFT_I18N_DUMP") {
        let mut all = crate::menus::menu_strings();
        all.extend(tl_literals());
        all.extend(undo_labels());
        all.extend(undo_scan().templates);
        let text: String = all.iter().map(|s| format!("{}\n", s.replace('\\', "\\\\").replace('\n', "\\n").replace('\t', "\\t"))).collect();
        std::fs::write(path, text).expect("write dump");
    }
    if let Ok(path) = std::env::var("VECTORCRAFT_I18N_DUMP_MESSAGES") {
        std::fs::write(path, message_dump()).expect("write dump");
    }
}

/// The text [`dump_source_strings`] writes to `VECTORCRAFT_I18N_DUMP_MESSAGES`.
fn message_dump() -> String {
    reported_messages().iter().map(|m| format!("{m}\n")).collect()
}

/// The translators' message dump lists every message the untranslated report checks, the
/// phrases a parameter schema's error builds included.
#[test]
fn message_dump_lists_the_reported_messages() {
    let dump = message_dump();
    let lines: std::collections::BTreeSet<&str> = dump.lines().collect();
    for phrase in SCHEMA_PHRASES {
        assert!(lines.contains(phrase), "the dump skips {phrase:?}");
    }
    assert!(lines.contains("the color book is cut short"));
}

/// Languages whose catalogs leave [`MENU_KEEP_AS_IS`] in English.
const KEEPS_MENU_NAMES: [&str; 20] =
    ["cs", "de", "el", "es", "fr", "hi", "hu", "it", "ja", "ka", "ko", "pl", "pt-br", "ru", "sv", "uk", "vi", "fi", "tr", "ca"];

/// Menu labels the menu-complete catalogs (Czech, German, Spanish, Italian, Japanese, Brazilian Portuguese) show as they are: the product name, a format name, the built-in workspace
/// names and the perspective grid presets (names, shown untranslated wherever else they appear).
/// Each language's own name in the Language menu is left alone too.
const MENU_KEEP_AS_IS: &[&str] = &[
    "VectorCraft",
    "OpenType",
    "Essentials",
    "Essentials Classic",
    "Automation",
    "Layout",
    "Painting",
    "Printing and Proofing",
    "Tracing",
    "Touch",
    "Typography",
    "Web",
    "[1P-Normal View]",
    "[1P-Low View]",
    "[1P-High View]",
    "[2P-Normal View]",
    "[2P-Low View]",
    "[2P-High View]",
    "[3P-Normal View]",
    "[3P-Low View]",
];

/// Every menu title, submenu, item and header label with its command and params (`""`/null for
/// the ones that run nothing).
fn menu_labels() -> Vec<(String, String, serde_json::Value)> {
    use crate::menus::Node;
    fn walk(items: &[Node], out: &mut Vec<(String, String, serde_json::Value)>) {
        for i in items {
            match i {
                Node::Item(it) if it.command.id == crate::menus::TODO => out.push((it.label.clone(), String::new(), serde_json::Value::Null)),
                Node::Item(it) => out.push((it.label.clone(), it.command.id.clone(), it.command.params.clone())),
                Node::Heading(l) => out.push((l.clone(), String::new(), serde_json::Value::Null)),
                Node::Submenu { label, children, .. } => {
                    out.push((label.clone(), String::new(), serde_json::Value::Null));
                    walk(children, out);
                }
                _ => {}
            }
        }
    }
    let mut labels = vec![];
    for (title, items) in crate::menus::menu_tree() {
        labels.push((title.to_string(), String::new(), serde_json::Value::Null));
        walk(&items, &mut labels);
    }
    labels
}

/// The effect an Effect menu item (`effect.apply` or `effect.dialog`) runs.
fn menu_item_effect<'a>(id: &str, p: &'a serde_json::Value) -> Option<&'a str> {
    p.get("effect").and_then(serde_json::Value::as_str).filter(|_| matches!(id, "effect.apply" | "effect.dialog"))
}

/// The label a menu item shows in `lang`, as the menus resolve it: an Effect menu item by its
/// effect's `@id` row when the catalog has one, any other command by its id's row or its English
/// label, and a submenu or an unbuilt item by its English label.
fn shown_menu_label(lang: Lang, item: &crate::menus::Node) -> Option<&str> {
    use crate::menus::Node;
    match item {
        Node::Item(it) if it.command.id == crate::menus::TODO => Some(tr(lang, &it.label)),
        Node::Item(it) => {
            let (id, p) = (it.command.id.as_str(), &it.command.params);
            let by_effect = menu_item_effect(id, p).map(|e| tr_id(lang, e, "")).filter(|s| !s.is_empty());
            Some(by_effect.unwrap_or_else(|| tr_id(lang, id, &it.label)))
        }
        Node::Submenu { label, .. } => Some(tr(lang, label)),
        _ => None,
    }
}

/// Items side by side in a menu show different labels in every language, so each can be told
/// from the others. Items with the same English label (the slots of a recent-files list) are
/// alike in every language.
#[test]
fn menu_items_side_by_side_have_different_labels() {
    use crate::menus::Node;
    fn walk(lang: Lang, path: &str, items: &[Node], out: &mut Vec<String>) {
        let mut seen: std::collections::BTreeMap<&str, (&str, &str)> = std::collections::BTreeMap::new();
        for item in items {
            let Some(shown) = shown_menu_label(lang, item) else { continue };
            let (english, what) = match item {
                Node::Item(it) if it.command.id == crate::menus::TODO => (it.label.as_str(), "submenu or unbuilt item"),
                Node::Item(it) => (it.label.as_str(), menu_item_effect(&it.command.id, &it.command.params).unwrap_or(it.command.id.as_str())),
                Node::Submenu { label, .. } => (label.as_str(), "submenu or unbuilt item"),
                _ => continue,
            };
            match seen.get(shown) {
                Some(&(other, other_what)) if other != english => {
                    out.push(format!("{}: {path}: {other:?} ({other_what}) and {english:?} ({what}) both show {shown:?}", lang.code()));
                }
                Some(_) => {}
                None => {
                    seen.insert(shown, (english, what));
                }
            }
            if let Node::Submenu { label, children, .. } = item {
                walk(lang, &format!("{path} › {label}"), children, out);
            }
        }
    }
    let tree = crate::menus::menu_tree();
    let mut same = vec![];
    for lang in Lang::all() {
        for (title, items) in &tree {
            walk(lang, title, items, &mut same);
        }
    }
    assert!(same.is_empty(), "{} pairs of menu items look alike:\n{}", same.len(), same.join("\n"));
}

/// Whether a menu label shows an id: a dotted command id, inside backticks or outside them, or
/// `key`, the command or effect id of its item, on its own.
fn label_shows_an_id(label: &str, key: &str) -> bool {
    let plain = label.replace('`', " ");
    !command_ids(&plain).is_empty() || plain.trim() == key
}

#[test]
fn label_shows_an_id_finds_ids_in_backticks_and_bare_keys() {
    let key = "type.convertListToText";
    assert!(label_shows_an_id("type.convertListToText", key));
    assert!(label_shows_an_id("`type.convertListToText`", key));
    assert!(label_shows_an_id("`type.convertListToText", key));
    assert!(label_shows_an_id("Convert (`slice.list`)", key));
    assert!(label_shows_an_id("roughen", "roughen"));
    assert!(label_shows_an_id("`roughen`", "roughen"));
    assert!(!label_shows_an_id("Convert to Text", key));
    assert!(!label_shows_an_id("Roughen…", "roughen"));
}

/// Menu items show words, never the command or effect id their catalog row is keyed by.
#[test]
fn menu_items_never_show_an_id() {
    fn walk(lang: Lang, items: &[crate::menus::Node], out: &mut Vec<String>) {
        for item in items {
            match item {
                crate::menus::Node::Item(it) if it.command.id != crate::menus::TODO => {
                    let (l, id) = (&it.label, it.command.id.as_str());
                    let key = menu_item_effect(id, &it.command.params).unwrap_or(id);
                    let shown = shown_menu_label(lang, item).unwrap_or_default();
                    if label_shows_an_id(shown, key) && !label_shows_an_id(l, key) {
                        out.push(format!("{}: {l:?} ({key}) shows {shown:?}", lang.code()));
                    }
                }
                crate::menus::Node::Submenu { children, .. } => walk(lang, children, out),
                _ => {}
            }
        }
    }
    let tree = crate::menus::menu_tree();
    let mut ids = vec![];
    for lang in Lang::all() {
        for (_, items) in &tree {
            walk(lang, items, &mut ids);
        }
    }
    assert!(ids.is_empty(), "{} menu items show an id:\n{}", ids.len(), ids.join("\n"));
}

/// The labels toggling items switch to ("Show Guides" once guides are hidden…), from a document
/// with every toggle flipped.
fn toggled_labels() -> Vec<String> {
    use serde_json::json;
    let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("object.envelope.makeWithWarp", json!({})).unwrap();
    let toggles = [
        "view.outline",
        "view.edges",
        "view.cornerWidget",
        "view.shapeWidgets",
        "view.textThreads",
        "type.hiddenCharacters",
        "view.gradientAnnotator",
        "view.livePaintGaps",
        "view.artboards",
        "view.template",
        "view.rulers",
        "view.videoRulers",
        "view.boundingBox",
        "view.transparencyGrid",
        "view.guides",
        "view.grid",
        "view.guides.lock",
        "view.slices.hide",
        "view.printTiling",
        "perspective.grid.show",
        "perspective.grid.rulers",
        "perspective.grid.lock",
        "object.envelope.editContents",
    ];
    let mut labels = vec![];
    for id in toggles {
        let before = crate::menus::dynamic_label(&app, id, "");
        app.run(id, json!({})).unwrap();
        let after = crate::menus::dynamic_label(&app, id, "");
        assert_ne!(before, after, "{id} didn't toggle its label");
        labels.extend([before, after]);
    }
    labels
}

/// Whether `catalog` has a `@msg` or `@msgplural` row for the message template `m` in `lang`.
fn has_message_row(catalog: &I18n, lang: Lang, m: &str) -> bool {
    ["@msg", "@msgplural"].iter().any(|context| catalog.get(lang, context, m).is_some())
}

/// Prints, per language, the strings its catalog has no row for: menu strings (titles, submenus,
/// items, headers, command labels and menu paths), `tl!` literals, undo step names and their
/// templates, the menu labels with the Show/Hide pairs and the context menu, render-site labels,
/// status and error messages, PostScript error meanings and default swatch names. The interface
/// shows a missing string in English, so a gap never fails this test; its assertions catch a scan
/// that stopped finding strings. `cargo xtask ci` and `cargo xtask i18n-report` run it with
/// `--nocapture`.
#[test]
fn untranslated_report() {
    use std::collections::{BTreeMap, BTreeSet};
    // For each kind of string, the languages checked and what each one lacks.
    let mut gaps: Vec<(&str, BTreeMap<&str, BTreeSet<String>>)> = Vec::new();
    let mut add = |code: &'static str, what: &'static str, missing: BTreeSet<String>| {
        let at = gaps.iter().position(|(w, _)| *w == what).unwrap_or_else(|| {
            gaps.push((what, BTreeMap::new()));
            gaps.len() - 1
        });
        gaps[at].1.insert(code, missing);
    };

    let catalog = shared();
    let strings = crate::menus::menu_strings();
    assert!(strings.len() > 500, "menu scan found only {} strings", strings.len());
    let literals = tl_literals();
    assert!(literals.len() > 300, "scan found only {} tl! literals", literals.len());
    // Undo step names: the recorded names, every effect's name without its ellipsis, and the
    // Liquify tools' step names (`{} Tool`). A name's row is its plain row or the row of the same
    // name with an ellipsis (`panels::history::step_name`); a template made with
    // `Session::edit_named` needs a plain row, which `panels::history::entry_name` fills.
    let scan = undo_scan();
    let mut undo = scan.labels;
    assert!(undo.len() > 300, "scan found only {} undo labels", undo.len());
    undo.extend(vectorcraft_effects::effect_catalog().iter().map(|e| e.label.trim_end_matches('…').to_string()));
    let liquify = ["warp", "twirl", "pucker", "bloat", "scallop", "crystallize", "wrinkle"];
    undo.extend(liquify.iter().filter_map(|k| vectorcraft_tools::distort::liquify::LiquifyKind::parse(k)).map(|k| format!("{} Tool", k.label())));
    assert_eq!(scan.formatted.iter().collect::<Vec<_>>(), ["{} Tool"], "step names built with format! (make them with Session::edit_named)");
    assert!(scan.templates.len() >= 3, "scan found only the templates {:?}", scan.templates);
    let messages = reported_messages();
    assert!(messages.len() > 300, "scan found only {} messages", messages.len());
    let (swatches, groups) = vectorcraft_color::default_swatches(vectorcraft_color::cms::Model::Rgb);
    // The greys' `K=` values read the same in any language.
    let swatch_names: BTreeSet<String> = swatches
        .iter()
        .map(|s| s.name.clone())
        .chain(groups.iter().flat_map(|g| std::iter::once(g.name.clone()).chain(g.swatches.iter().map(|s| s.name.clone()))))
        .chain([vectorcraft_color::swatch::REGISTRATION.to_string()])
        .filter(|n| !n.starts_with("K="))
        .collect();

    for lang in Lang::all().filter(|l| l.complete_menus()) {
        let code = lang.code();
        // Languages that keep the product, workspace and perspective preset names in English.
        let kept = |s: &str| KEEPS_MENU_NAMES.contains(&code) && MENU_KEEP_AS_IS.contains(&s);
        add(code, "menu strings", strings.iter().filter(|s| !kept(s) && !has(lang, s)).cloned().collect::<BTreeSet<_>>());
        add(code, "tl! strings", literals.iter().filter(|s| !has(lang, s)).cloned().collect::<BTreeSet<_>>());
        add(code, "undo labels", undo.iter().filter(|s| crate::panels::history::step_name(lang, s).is_none()).cloned().collect::<BTreeSet<_>>());
        add(code, "undo label templates", scan.templates.iter().filter(|t| !has(lang, t)).cloned().collect::<BTreeSet<_>>());
        let render_site =
            RENDER_SITE_ROWS.iter().filter(|(ctx, src)| catalog.get(lang, ctx, src).is_none()).map(|(ctx, src)| format!("{ctx}: {src}"));
        add(code, "render-site labels", render_site.collect());
        add(code, "swatch names", swatch_names.iter().filter(|n| catalog.get(lang, "swatch", n).is_none()).cloned().collect::<BTreeSet<_>>());
    }

    // The menu labels as the menus show them, with what menu strings don't hold: the labels
    // toggling items switch to, the canvas context menu, the macOS menu bar's own labels and its
    // Settings submenu's Preferences pages.
    let labels = menu_labels();
    let language_name = |l: &str| Lang::all().any(|lang| lang.name() == l);
    // Font names and sizes, and installed plug-ins' own names, are not interface text.
    let interface = |(label, id, p): &(String, String, serde_json::Value)| {
        id != "text.setStyle"
            && !crate::menus::lists_plugin(id, p)
            && strings.contains(label.as_str())
            && !MENU_KEEP_AS_IS.contains(&label.as_str())
            && !language_name(label)
    };
    let mut shown: BTreeSet<String> = labels.iter().filter(|l| interface(l)).map(|(l, ..)| l.clone()).collect();
    shown.extend(toggled_labels());
    shown.extend(crate::menus::CONTEXT_LABELS.iter().map(|l| l.to_string()));
    shown.extend(crate::menus::MAC_LABELS.iter().map(|l| l.to_string()));
    shown.extend(vectorcraft_engine::cmd::prefscmds::PREF_CATEGORIES.iter().map(|l| l.to_string()));
    for code in KEEPS_MENU_NAMES {
        let lang = lang(code);
        // Labels the menu strings above already listed aren't listed twice.
        let missing = shown.iter().filter(|l| !has(lang, l) && (!lang.complete_menus() || !strings.contains(*l)));
        add(code, "menu labels", missing.cloned().collect::<BTreeSet<_>>());
    }

    for code in COMPLETE_MESSAGES {
        let lang = lang(code);
        add(code, "messages", messages.iter().filter(|m| !has_message_row(&catalog, lang, m)).cloned().collect::<BTreeSet<_>>());
        let meanings = vectorcraft_eps::POSTSCRIPT_ERRORS.iter().filter(|(_, m)| message(lang, m) == *m).map(|(_, m)| m.to_string());
        add(code, "PostScript error meanings", meanings.collect());
    }

    // A string that every language checked lacks is listed once, under "every language".
    let mut by_language: BTreeMap<&str, Vec<(&str, Vec<&String>)>> = BTreeMap::new();
    let mut everywhere = vec![];
    for (what, langs) in &gaps {
        let first = langs.values().next();
        let common: BTreeSet<&String> = first.into_iter().flatten().filter(|m| langs.values().all(|set| set.contains(*m))).collect();
        if !common.is_empty() {
            everywhere.push((*what, langs.len(), common.iter().copied().collect::<Vec<_>>()));
        }
        for (code, missing) in langs {
            let rest: Vec<&String> = missing.iter().filter(|m| !common.contains(m)).collect();
            if !rest.is_empty() {
                by_language.entry(code).or_default().push((*what, rest));
            }
        }
    }
    let total: usize = gaps.iter().flat_map(|(_, langs)| langs.values()).map(BTreeSet::len).sum();
    let mut out = format!("Untranslated strings, shown in English until a catalog row is added: {total} missing rows\n");
    let list = |out: &mut String, what: &str, missing: &[&String]| {
        *out += &format!("  {what} ({}):\n", missing.len());
        for m in missing {
            *out += &format!("    {}\n", m.replace('\n', "\\n"));
        }
    };
    if !everywhere.is_empty() {
        out += "\nevery language:\n";
        for (what, langs, missing) in &everywhere {
            list(&mut out, &format!("{what}, all {langs} languages checked"), missing);
        }
    }
    for (code, kinds) in &by_language {
        out += &format!("\n{code}:\n");
        for (what, missing) in kinds {
            list(&mut out, what, missing);
        }
    }
    println!("{out}");
}

/// The status and error messages [`untranslated_report`] checks: the message literals of
/// [`MESSAGE_CRATES`] and the phrases a parameter schema's error builds at run time
/// ([`SCHEMA_PHRASES`]), which no literal holds.
fn reported_messages() -> std::collections::BTreeSet<String> {
    let mut messages = message_literals();
    messages.extend(SCHEMA_PHRASES.iter().map(|p| p.to_string()));
    messages
}

/// The untranslated report checks the messages of the effects and of the crates whose errors the
/// engine and the effects pass on, and the phrases a parameter schema's error builds.
#[test]
fn untranslated_report_checks_every_message_crate_and_schema_phrase() {
    let reported = reported_messages();
    let passed_on = [
        ("effects", "the SVG filter has no primitives"),
        ("color", "not a swatch library (.vcswatches or .gpl)"),
        ("render", "invalid scale {scale}"),
        ("three-d", "3D effects require one to ten lights"),
        ("pathops", "the shapes are too degenerate to combine"),
    ];
    for (krate, m) in passed_on {
        assert!(message_literals_in(&[krate]).contains(m), "{krate}: {m}");
        assert!(reported.contains(m), "the report skips {krate}: {m}");
    }
    for phrase in SCHEMA_PHRASES {
        assert!(reported.contains(*phrase), "the report skips the schema phrase {phrase:?}");
    }
}

/// The names [`MENU_KEEP_AS_IS`] lists are menu labels, and no catalog that keeps them in English
/// translates them.
#[test]
fn kept_menu_names_are_menu_labels_left_in_english() {
    let labels = menu_labels();
    for keep in MENU_KEEP_AS_IS {
        assert!(labels.iter().any(|(l, ..)| l == *keep), "`{keep}` isn't a menu label");
        for code in KEEPS_MENU_NAMES {
            assert!(!has(lang(code), keep), "{code}: `{keep}` is both kept and translated");
        }
    }
}

/// The scan reads the message of an error made with one of the engine's [`ERROR_HELPERS`] or with
/// an error enum's variant, `EngineError::Disabled`'s reason included: a literal or a `format!`
/// string, on the call's line or wrapped over several lines, in the same template form as the
/// catalogs' rows. It does not read a call whose name ends in a helper's name (`is_bad(`), a method
/// call (`.err(`), `std::io::Error::new`, one of [`NAME_VARIANTS`], `EngineError::Disabled`'s
/// command id or a call in a comment. A `//` or `/*` inside a string or a raw string, or after a
/// character literal such as `'"'`, does not start a comment. Without the helpers, the scan reads
/// only the four variants in this source.
#[test]
fn message_scan_reads_error_helpers_and_variants() {
    let code = r##"
fn width(p: &Value, cmd: &str, key: &str) -> Result<f64> {
    let w = p.get(key).and_then(Value::as_f64).ok_or_else(|| bad(cmd, format!("`{key}` must be a number")))?;
    if w < 0.0 {
        return Err(bad(cmd, "the width can't be negative"));
    }
    let e = bad(
        "object.example",
        format!(
            "`{}` must be between {lo:.1} and {hi}, not {w}",
            key
        ),
    );
    let _ = bad(&format!("{cmd}, {key}"), "a comma in the first argument");
    let _ = is_bad(cmd, "not an error helper");
    let _ = r.err("not an error helper either");
    let _ = err(format!("can't open `{name}`: damaged"));
    let _ = PsError::Limit(format!("the {what} is nested too deep"));
    let _ = Error::Manifest("the manifest has no name".into());
    let _ = std::io::Error::new(kind, "not an error variant");
    let _ = PsError::Ps("not an error message", op);
    let _ = EngineError::UnknownCommand("not.an.error.message".into());
    let _ = EngineError::Disabled("not.a.reason".into(), "there is nothing to clear".into());
    let _ = EngineError::Disabled(C.into(), format!("the {kind} panel is locked"));
    // return Err(bad(cmd, "not an error: a commented-out call"));
    /* let _ = err("not an error: a call in a block comment");
       /* nested */ let _ = err("not an error: after a nested block comment"); */
    let _ = ('"', '\'', b'"', bad(cmd, "the width // the height"));
    let _ = (r#"a "/* raw string"#, |s: &'a str| -> Result<&'a str> { Err(bad(cmd, "the height is too large")) });
    Err(error("Select only dimensions"))
}
"##;
    let engine: Vec<_> = ERROR_HELPERS.iter().filter(|(c, _, _)| *c == "engine").map(|&(_, call, before)| (call, before)).collect();
    let found = source_messages(code, &engine, &Default::default());
    for want in [
        "`{key}` must be a number",
        "the width can't be negative",
        "`{_1}` must be between {lo} and {hi}, not {w}",
        "a comma in the first argument",
        "can't open `{name}`: damaged",
        "the {what} is nested too deep",
        "the manifest has no name",
        "there is nothing to clear",
        "the {kind} panel is locked",
        "the width // the height",
        "the height is too large",
        "Select only dimensions",
    ] {
        assert!(found.iter().any(|m| m == want), "{want} not in {found:#?}");
    }
    let named = |m: &String| m.contains("not an error") || m.contains("not.an.error") || m.contains("not.a.reason");
    assert!(!found.iter().any(named), "{found:#?}");
    let elsewhere = source_messages(code, &[], &Default::default());
    let in_variants = ["nested", "manifest", "nothing to clear", "panel is locked"];
    assert!(elsewhere.len() == 4 && elsewhere.iter().all(|m| in_variants.iter().any(|w| m.contains(w))), "{elsewhere:#?}");
}

/// The scan reads a message that an error or a status takes from a `&str` constant, by its name or
/// by its path, with `.into()`, `.to_string()` or nothing after it, and the constant declared in
/// another file. It does not read a constant that no error takes, a call of a name that a constant
/// also has, or a constant in a comment.
#[test]
fn message_scan_reads_messages_held_in_constants() {
    let declared = r#"
pub(crate) const TOO_LARGE: &str = "the selection is too large to hand over";
pub const SLOW: &'static str = "time budget";
const UNUSED: &str = "not an error: a constant no error takes";
const fn NOT_A_STR() -> u32 { 0 }
// const COMMENTED: &str = "not an error: a constant in a comment";
"#;
    let code = r#"
fn run() -> Result<()> {
    let _ = Error::Limit(crate::runtime::TOO_LARGE.into());
    let _ = Err(Error::Limit(SLOW.to_string()));
    let _ = Err(COMMENTED);
    let _ = Err(SLOW(3));
    Ok(())
}
"#;
    let mut consts = std::collections::BTreeMap::new();
    for (name, value) in str_consts(declared) {
        consts.entry(name).or_insert_with(Vec::new).push(value);
    }
    assert_eq!(consts.keys().collect::<Vec<_>>(), ["SLOW", "TOO_LARGE", "UNUSED"]);
    let found: std::collections::BTreeSet<String> = source_messages(code, &[], &consts).into_iter().collect();
    assert_eq!(found.iter().collect::<Vec<_>>(), ["the selection is too large to hand over", "time budget"]);
    assert!(source_messages(code, &[], &Default::default()).is_empty());
}

/// A message split over lines with a `\` at the end of a line is read as Rust reads it: the line
/// break and the next line's leading whitespace are dropped, with LF and CRLF line endings, in a
/// constant and in a literal an error takes.
#[test]
fn message_scan_joins_a_literal_continued_on_the_next_line() {
    for eol in ["\n", "\r\n"] {
        let declared = format!("const LONG: &str = \"too many elements: \\{eol}     drawn as a plain group\";{eol}");
        let consts = std::collections::BTreeMap::from_iter(str_consts(&declared).into_iter().map(|(name, value)| (name, vec![value])));
        assert_eq!(consts.get("LONG"), Some(&vec!["too many elements: drawn as a plain group".to_string()]), "{eol:?}");
        let code = format!("fn run() {{ let _ = Err(\"the file \\{eol}\t\t is too large\"); let _ = Err(LONG); }}{eol}");
        let found: std::collections::BTreeSet<String> = source_messages(&code, &[], &consts).into_iter().collect();
        assert_eq!(found.iter().collect::<Vec<_>>(), ["the file is too large", "too many elements: drawn as a plain group"], "{eol:?}");
    }
}

/// The scan reads each of Rust's character escapes in a message as the character Rust gives it
/// (https://doc.rust-lang.org/reference/tokens.html#character-escapes), and stops at the literal's
/// own closing quote, not at an escaped one.
#[test]
fn message_scan_decodes_rust_character_escapes() {
    let cases = [
        (r#"a\rb" tail"#, "a\rb"),
        (r#"a\0b""#, "a\0b"),
        (r#"\x41\x7f\x7F""#, "A\x7f\x7f"),
        (r#"caf\u{e9}""#, "café"),
        (r#"caf\u{E_9}""#, "café"),
        (r#"\u{1F600}\u{10_FFFF}\u{0}""#, "\u{1F600}\u{10FFFF}\0"),
        (r#"a\\b""#, "a\\b"),
        (r#"say \"hi\"" tail"#, "say \"hi\""),
        (r#"it\'s""#, "it's"),
        (r#"\n\t""#, "\n\t"),
    ];
    let wrong: Vec<_> = cases.iter().filter(|(body, text)| string_literal(body).as_deref() != Some(*text)).collect();
    assert!(wrong.is_empty(), "{wrong:?}");
    let found = source_messages(r#"fn run() { let _ = Err("bad \x41 value \u{e9}"); }"#, &[], &Default::default());
    assert_eq!(found, ["bad A value é"]);
}

/// A literal with an escape Rust does not accept is not read as a message: an unknown letter, `\x`
/// with fewer than two hex digits or above `7F`, and `\u` without braces, without digits, with a
/// leading underscore, with more than six digits, or naming a surrogate or a value above `10FFFF`.
#[test]
fn message_scan_rejects_invalid_rust_escapes() {
    let bodies = [
        r#"bad \q""#,
        r#"bad \x4""#,
        r#"bad \x4g""#,
        r#"bad \x80""#,
        r#"bad \x+1""#,
        concat!(r"bad \u", "00e9\""),
        r#"bad \u{""#,
        r#"bad \u{}""#,
        r#"bad \u{_e9}""#,
        r#"bad \u{e9""#,
        r#"bad \u{D800}""#,
        r#"bad \u{110000}""#,
        r#"bad \u{1234567}""#,
    ];
    let read: Vec<_> = bodies.iter().filter(|body| string_literal(body).is_some()).collect();
    assert!(read.is_empty(), "{read:?}");
    let code = r#"fn run() { let _ = Err("bad \q value"); let _ = Err("bad \u{D800} value"); let _ = Err("good value"); }"#;
    assert_eq!(source_messages(code, &[], &Default::default()), ["good value"]);
}

/// The scan reads the message of each of the plug-in host's [`ERROR_HELPERS`]: a literal, a
/// `format!` string with or without a `&` before it, after the location of `out_err` (a
/// `format!` string too), the number and range of `in_range`, the number and location of `unit`,
/// and the block of `.read_packed`, a method. It does not read a value that isn't a literal, a
/// key's name alone, a call with too few arguments, a helper's declaration or a call whose name
/// ends in a helper's name.
#[test]
fn message_scan_reads_plugin_helpers() {
    let code = r#"
fn check(k: &str) -> Result<()> {
    let bad = |what: &str| Err(Error::Manifest(format!("parameter `{k}`: {what}")));
    let wrong = |what: &str| Error::Params(format!("`{k}` must be {what}"));
    let _ = bad("bad range");
    let _ = bad(&format!("`maxLength` must be 1-{MAX_TEXT}, and the default no longer"));
    let _ = wrong("a number");
    let _ = wrong(&format!("a list of {min_items} to {max_items} numbers"));
    let _ = out_err(at, "`opacity` must be a number");
    let _ = out_err(&format!("{at}.{k}"), format!("unknown key `{k}`; the keys here are {}", keys.join(", ")));
    let _ = out_err(at, e);
    let _ = live.read_packed(packed, "the output", limits.max_output_bytes);
    let _ = in_range(l, 0.0, 100.0, at, "a color component");
    let _ = unit(*x, &format!("{at}.plates"), "a plate");
    let _ = unit(s.tint, &format!("{at}.tint"), "`tint`");
    let _ = unit(r);
    let _ = not_bad("not an error helper");
    Err(wrong())
}
fn wrong(&self) -> String { String::new() }
"#;
    let plugins: Vec<_> = ERROR_HELPERS.iter().filter(|(c, _, _)| *c == "plugins").map(|&(_, call, before)| (call, before)).collect();
    let found: std::collections::BTreeSet<String> = source_messages(code, &plugins, &Default::default()).into_iter().collect();
    let want = [
        "`maxLength` must be 1-{MAX_TEXT}, and the default no longer",
        "`opacity` must be a number",
        "`{k}` must be {what}",
        "a color component",
        "a list of {min_items} to {max_items} numbers",
        "a number",
        "a plate",
        "bad range",
        "parameter `{k}`: {what}",
        "the output",
        "unknown key `{k}`; the keys here are {_1}",
    ];
    assert_eq!(found.iter().collect::<Vec<_>>(), want);
}

/// A constant's name reads the text the file declares under it, else every text the files of its
/// own crate declare under it, else every text any scanned crate declares under it: an engine
/// reader's error does not read the Scripts menu's headers, which are constants of the same names
/// in another crate.
#[test]
fn message_scan_reads_constants_of_the_file_and_its_crate() {
    use std::collections::{BTreeMap, BTreeSet};
    let jxl = str_consts(r#"const TRUNCATED: &str = "the JPEG XL file is cut short"; const LIMIT: &str = "the image is too large";"#);
    let pcx = str_consts(r#"const TRUNCATED: &str = "the PCX file is cut short";"#);
    let menu = str_consts(r#"const TRUNCATED: &str = "Some scripts were not listed"; const SHARED: &str = "a message of another crate";"#);
    let gather = |files: &[&Vec<(String, String)>]| {
        let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, value) in files.iter().copied().flatten() {
            map.entry(name.clone()).or_default().push(value.clone());
        }
        map
    };
    let engine = gather(&[&jxl, &pcx]);
    let everywhere = gather(&[&jxl, &pcx, &menu]);
    let code =
        "fn f() -> Result<()> { Err(TRUNCATED.into()) }\nfn g() -> Result<()> { Err(LIMIT.into()) }\nfn h() -> Result<()> { Err(SHARED.into()) }";
    let found = |own: &[(String, String)], in_crate: &BTreeMap<String, Vec<String>>| -> BTreeSet<String> {
        source_messages(code, &[], &scoped_consts(own, in_crate, &everywhere)).into_iter().collect()
    };
    let in_jxl = ["a message of another crate", "the JPEG XL file is cut short", "the image is too large"];
    assert_eq!(found(&jxl, &engine).iter().collect::<Vec<_>>(), in_jxl);
    let in_engine = ["a message of another crate", "the JPEG XL file is cut short", "the PCX file is cut short", "the image is too large"];
    assert_eq!(found(&[], &engine).iter().collect::<Vec<_>>(), in_engine);
    assert_eq!(found(&[], &BTreeMap::new()).len(), 5);
    let scanned = message_literals();
    for header in [crate::scripts_menu::TRUNCATED, crate::scripts_menu::UNREADABLE] {
        assert!(!scanned.contains(header), "{header}");
    }
}

/// The scan reads the errors the script interpreter throws at a script with a method of any
/// receiver, `&format!` strings included, and not a call of a longer name.
#[test]
fn message_scan_reads_script_interpreter_errors() {
    let code = r#"
fn f(i: &mut Interp, interp: &mut Interp) -> JsResult<Value> {
    let _ = i.type_error(&format!("{what}: the object is not a File or a Folder"));
    let _ = interp.throw(ErrorType::Range, "File.seek: the mode must be 0, 1 or 2");
    let _ = self.error("the path has an empty step");
    let _ = self.throw_host(t, "not an error: a longer name");
    let _ = throw_type_error(interp, "not an error: a function");
    let _ = self.reject_bool(throw, &k, "the object is not extensible");
    let _ = self.reject(throw, &key("length"), "it is read-only");
    Ok(Value::Undefined)
}
"#;
    let js: Vec<_> = ERROR_HELPERS.iter().filter(|(c, _, _)| *c == "js").map(|&(_, call, before)| (call, before)).collect();
    let found = source_messages(code, &js, &Default::default());
    assert_eq!(
        found,
        [
            "{what}: the object is not a File or a Folder",
            "File.seek: the mode must be 0, 1 or 2",
            "the path has an empty step",
            "the object is not extensible",
            "it is read-only"
        ]
    );
}

/// The scan reads the message a `.map_err` closure returns, a literal, a `format!` string or a
/// constant, whatever the closure's parameter, from each branch of an `if` and each arm of a
/// `match`, and given to a variant of an enum whose name ends in `Fail`, and not a closure that
/// passes the error on, a `json!` value's keys, nor the steps of a script result's path
/// ([`NOT_MESSAGES`]).
#[test]
fn message_scan_reads_map_err_closures() {
    let code = r#"
fn f(text: &str, n: usize) -> Result<u32, String> {
    let doc = parse(text).map_err(|e| format!("the filter isn't valid XML: {e}"))?;
    let a = at(doc).map_err(|place| format!("[{i}]{place}"))?;
    let b = at(doc).map_err(|place| format!(".{k}{place}"))?;
    let c = at(doc).map_err(|e| format!("{name}: {e}"))?;
    let n = u32::try_from(n).map_err(|_| "too many blocks".to_string())?;
    let m = u16::try_from(n).map_err(move |(a, b)| LIMIT.into())?;
    let k = read(doc).map_err(|e| e.to_string())?;
    let j = read(doc).map_err(|e| bad(C, e))?;
    let i = read(doc).map_err(|e| if e == "{x}" { CUT_SHORT.into() } else { e })?;
    let h = read(doc).map_err(|e| match e {
        Error::Io(_) => "the file can't be read".to_string(),
        other => fail(json!({"objects": [other.to_string()]})),
    })?;
    let g = i32::try_from(n).map_err(|_| CompileFail::Syntax("the pattern is too large".into()))?;
    let f = read(doc).map_err(|e| {
        self.published = None;
        format!("the clipboard can't be read: {e}")
    })?;
    Ok(n)
}
"#;
    let consts =
        [("LIMIT", "the name is too long"), ("CUT_SHORT", "the book is cut short")].map(|(k, v)| (k.to_string(), vec![v.to_string()])).into();
    let want = [
        "the pattern is too large",
        "the filter isn't valid XML: {e}",
        "{name}: {e}",
        "too many blocks",
        "the name is too long",
        "the book is cut short",
        "the file can't be read",
        "the clipboard can't be read: {e}",
    ];
    assert_eq!(source_messages(code, &[], &consts), want);
    assert!(message_literals_in(&["color"]).contains("the color book is cut short"));
    assert!(message_literals_in(&["color"]).contains("not a swatch library: {e}"));
    assert!(message_literals_in(&["js"]).contains("regular expression too large"));
    assert!(message_literals_in(&["effects"]).contains("the SVG filter isn't valid XML: {e}"));
}

/// The scan reads the message a `.map_err` closure gives with `return`, also from inside an `if`
/// branch or a `let` statement but not from a closure inside it, the last expression after a block
/// statement with no `;`, and the values a `let` statement gives a local that the closure returns,
/// and not those of a local the closure only passes elsewhere or writes into a format string, by
/// name or as a `format!` argument.
#[test]
fn message_scan_reads_returns_and_tails_and_skips_locals() {
    let code = r#"
fn f(text: &str) -> Result<u32, String> {
    let a = read(text).map_err(|_| {
        return "the file cannot be read".to_string();
    })?;
    let b = read(text).map_err(|e| {
        if e.is_io() {
            return format!("the disk failed: {e}");
        }
        e.to_string()
    })?;
    let c = read(text).map_err(|e| {
        if e.is_io() { cleanup(); }
        format!("the file is cut short: {e}")
    })?;
    let d = read(text).map_err(|e| {
        if e.is_io() { cleanup(); }
        let level = if e.is_io() { "warning" } else { "error" };
        log_failure(level, &e);
        format!("the file is not a library: {e}")
    })?;
    let g = read(text).map_err(|e| {
        let kind = match e.kind() {
            Kind::Gone => return GONE.into(),
            _ => { "some other kind" }
        };
        format!("the {kind} file is wrong")
    })?;
    let h = read(text).map_err(|e| {
        let name = e.name().unwrap_or_else(|| return "unnamed source");
        let note = e.note().map(move |n| { return "noted source" });
        format!("the source {name} is gone")
    })?;
    let k = read(text).map_err(|e| {
        let why = if e.is_deep() { format!("its objects are nested too deep ({e})") } else { e.to_string() };
        FormatError::NotVectorcraft(why)
    })?;
    let m = read(text).map_err(|e| {
        let level = if e.is_io() { "warning" } else { "error" };
        format!("the file failed with a {}", level)
    })?;
    Ok(a)
}
"#;
    let consts = [("GONE", "the file is gone")].map(|(k, v)| (k.to_string(), vec![v.to_string()])).into();
    let want = [
        "the file cannot be read",
        "the disk failed: {e}",
        "the file is cut short: {e}",
        "the file is not a library: {e}",
        "the file is gone",
        "the {kind} file is wrong",
        "the source {name} is gone",
        "its objects are nested too deep ({e})",
        "the file failed with a {_1}",
    ];
    assert_eq!(source_messages(code, &[], &consts), want);
}

/// The scan reads the messages of an error handler that a `.map_err(name)` names: a closure bound
/// to that name, up to its `;`, and a function of that name, each of its statements.
#[test]
fn message_scan_reads_named_map_err_handlers() {
    let code = r#"
fn encode(img: &Image) -> Result<Vec<u8>, String> {
    let fail = |e: gif::EncodingError| format!("GIF encoding failed: {e}");
    let gone = move |_| GONE.to_string();
    let size = if img.is_empty() { WRONG.into() } else { 0 };
    let mut enc = Encoder::new(img).map_err(fail)?;
    enc.write(img).map_err(fail)?;
    enc.check().map_err(gone)?;
    enc.close().map_err(unreadable)?;
    enc.into_inner().map_err(Error::Io)
}

fn unreadable(e: impl std::fmt::Display) -> String {
    let e = e.to_string();
    format!("the file can't be read: {e}")
}
"#;
    let consts = [("GONE", "the encoder is gone"), ("WRONG", "the image is wrong")].map(|(k, v)| (k.to_string(), vec![v.to_string()])).into();
    let want = ["GIF encoding failed: {e}", "the encoder is gone", "the file can't be read: {e}"];
    assert_eq!(source_messages(code, &[], &consts), want);
    let render = message_literals_in(&["render"]);
    assert!(render.contains("GIF encoding failed: {e}") && render.contains("JPEG encoding failed: {e}"));
    assert!(message_literals_in(&["engine"]).contains("Data Recovery can't keep copies in {path}: {e}"));
}

/// The scan stops at a test module whose `cfg` is `test` or `all(test, …)`, and reads on past a
/// declaration of a test file and a module that is not only for tests.
#[test]
fn message_scan_skips_test_modules_with_any_test_cfg() {
    let code = "fn a() {}\n#[cfg(test)]\nmod tests_x;\n#[cfg(any(test, feature = \"x\"))]\nmod extra {}\nfn b() {}\n";
    for module in ["#[cfg(test)]\nmod tests {", "#[cfg(all(test, not(target_arch = \"wasm32\")))]\nmod tests {", "#[cfg(all( test , unix))] mod t {"]
    {
        let text = format!("{code}{module} fn c() {{}} }}\n");
        assert_eq!(without_test_module(&text), code, "{module}");
    }
    assert_eq!(without_test_module(code), code);
}

/// The scan reads the refusals `script.run`'s `param(msg)` gives, and not the arguments of a type's
/// own function of that name.
#[test]
fn message_scan_reads_script_run_param_refusals() {
    let code = r#"
fn f(p: &Value) -> Result<()> {
    let grays = Grays::param(C, p)?;
    let pick = Pick::param(p, C)?;
    let _ = cmd::bad(C, "the name is too long");
    Err(param("`path` is empty"))
}
"#;
    let engine: Vec<_> = ERROR_HELPERS.iter().filter(|(c, _, _)| *c == "engine").map(|&(_, call, before)| (call, before)).collect();
    let consts = [("C".to_string(), vec!["view.proofSetup".to_string()])].into();
    assert_eq!(source_messages(code, &engine, &consts), ["the name is too long", "`path` is empty"]);
}

/// The phrases a parameter schema's error uses for what a value must be, as `@msg` sources: one
/// for each type, and for a number each range and unit form.
const SCHEMA_PHRASES: &[&str] = &[
    "a number",
    "a number >= {_1}",
    "a number > {_1}",
    "a number <= {_1}",
    "a number < {_1}",
    "a number {_1} to {_2}",
    "a number >= {_1} and < {_2}",
    "a number > {_1} and <= {_2}",
    "a number > {_1} and < {_2}",
    "a number ({_1})",
    "a number >= {_1} ({_2})",
    "a number > {_1} ({_2})",
    "a number <= {_1} ({_2})",
    "a number < {_1} ({_2})",
    "a number {_1} to {_2} ({_3})",
    "a number >= {_1} and < {_2} ({_3})",
    "a number > {_1} and <= {_2} ({_3})",
    "a number > {_1} and < {_2} ({_3})",
    "an integer",
    "an integer >= {_1}",
    "an integer <= {_1}",
    "an integer {_1} to {_2}",
    "true or false",
    "a string",
    "one of {_1}",
    "an object id (a non-negative integer)",
    "a list of object ids",
    "a color (a string, [r, g, b] or an object)",
    "[x, y] (two numbers)",
    "an object",
    "a list",
];

/// Whether `text` holds `word` with no ASCII letter, digit or underscore right before or after it,
/// so `true` counts in `true이거나` but not in `untrue`.
fn holds_ascii_word(text: &str, word: &str) -> bool {
    let joins = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back().is_none_or(|c| !joins(c));
        let after = text[at + word.len()..].chars().next().is_none_or(|c| !joins(c));
        before && after
    })
}

/// The command ids that `s` names outside its backticked spans, such as `assets.list` or
/// `window.panel.dock`: words of two or more dot-separated parts, each an ASCII letter or digit
/// run that starts with a lowercase letter.
fn command_ids(s: &str) -> Vec<&str> {
    let mut ids = Vec::new();
    for part in s.split('`').step_by(2) {
        for word in part.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_')) {
            let word = word.trim_end_matches('.');
            let segments: Vec<&str> = word.split('.').collect();
            let dotted = segments.len() > 1
                && segments
                    .iter()
                    .all(|seg| seg.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && seg.chars().all(|c| c.is_ascii_alphanumeric()));
            if dotted && word != "e.g" && word != "i.e" {
                ids.push(word);
            }
        }
    }
    ids
}

/// Languages whose catalogs translate status and error messages; [`untranslated_report`] lists
/// the messages each one lacks.
const COMPLETE_MESSAGES: &[&str] = &[
    "cs", "de", "el", "es", "fr", "hi", "hu", "it", "ja", "ka", "ko", "pl", "pt-br", "ru", "sv", "uk", "vi", "zh-hans", "zh-hant", "fi", "tr", "ca",
    "nl", "da", "id",
];

/// Crates whose error and status messages reach the status bar, the crates whose errors the
/// engine and the effects pass on included (effects, swatch libraries, raster sizes and encoders,
/// 3D solids, path operations).
const MESSAGE_CRATES: &[&str] = &[
    "ui-egui", "engine", "doc", "format", "svg", "pdf", "eps", "text", "plugins", "metafile", "cad", "trace", "js", "effects", "color", "render",
    "three-d", "pathops",
];

/// Where a message literal starts: a status, an error value, a `thiserror` message.
const MESSAGE_MARKERS: &[&str] = &[".status(", "ui.status = ", "status = ", "Other(", "Err(", "ok_or(", "ok_or_else(|| ", "#[error("];

/// Helpers that make an error from a message, by crate, with the number of arguments before the
/// message: the engine's `bad(cmd, msg)` (`EngineError::BadParams`), `err(msg)` (`cmd/fileio`)
/// and `error(msg)` (`cmd/dimension.rs`), the script host's `fail(msg)` (`script/files.rs`,
/// `script/dollar.rs`) and `value(msg)`, which gives the messages the script `File` object's own
/// code throws (`script/files.rs`), `.over(limit)` for a limit a run passes (`script/mod.rs`) and
/// `param(msg)` for `script.run`'s parameters (`cmd/scripts.rs`), the PDF writer's `bad(why)` (`forms.rs`) and `failed(why)` (`encrypt.rs`, `linearize.rs`), the
/// plug-in host's `bad(what)` (a parameter's spec and dialog keys), `wrong(what)` (a parameter's
/// value), `out_err(at, msg)` (plug-in output), `in_range(x, lo, hi, at, what)` and
/// `unit(x, at, what)` (a painted effect's numbers) and `.read_packed(packed, what, max)` (the block
/// a plug-in returned), and the script interpreter's methods that throw an error at a script:
/// `.type_error(msg)`, `.throw(kind, msg)` and the XPath evaluator's `.error(msg)`, with the reasons
/// it gives for a property it cannot define or assign (`.reject_bool(throw, key, why)` and
/// `.reject(throw, key, why)`), and the interface's `unavailable(tool, reason)` (a tool's place that
/// no toolbar can show, which Help puts in the status bar). A helper that starts with `.` is a method of any receiver; any other
/// is a function, not a type's own function of the same name (`Grays::param(C, p)`).
const ERROR_HELPERS: &[(&str, &str, usize)] = &[
    ("engine", "bad(", 1),
    ("engine", "err(", 0),
    ("engine", "error(", 0),
    ("engine", "fail(", 0),
    ("engine", "value(", 0),
    ("engine", ".over(", 0),
    ("engine", "param(", 0),
    ("pdf", "bad(", 0),
    ("pdf", "failed(", 0),
    ("plugins", "bad(", 0),
    ("plugins", "wrong(", 0),
    ("plugins", "out_err(", 1),
    ("plugins", "in_range(", 4),
    ("plugins", "unit(", 2),
    ("plugins", ".read_packed(", 1),
    ("js", ".type_error(", 0),
    ("js", ".throw(", 1),
    ("js", ".error(", 0),
    ("js", ".reject_bool(", 2),
    ("js", ".reject(", 2),
    ("ui-egui", "unavailable(", 1),
];

/// Error variants whose first argument is a name, not a message: a command id
/// (`EngineError::UnknownCommand`) and a PostScript error name (`PsError::Ps`).
const NAME_VARIANTS: &[&str] = &["UnknownCommand", "Ps"];

/// Error variants whose message follows other arguments, with the number of arguments before it:
/// `EngineError::Disabled(command id, reason)`.
const LATER_MESSAGE_VARIANTS: &[(&str, usize)] = &[("Disabled", 1)];

/// Messages that are signals, not text for people, the name of a plug-in export that an error
/// message holds (`vc_manifest points outside memory`), and the steps of the path to a script
/// result's part that an error message holds (`vc.run: the result{place} is too large for a script`).
const NOT_MESSAGES: &[&str] = &["quit", "cancelled", "vc_manifest", "[{i}]{place}", ".{k}{place}"];

/// Every status and error message literal in [`MESSAGE_CRATES`] (test modules aside), as catalog
/// templates: `{}` becomes `{_1}`, `{_2}` … and format specs are dropped (`{n:?}` → `{n}`), and
/// the braces of a message that is not a format string are doubled.
pub fn message_literals() -> std::collections::BTreeSet<String> {
    message_literals_in(MESSAGE_CRATES)
}

/// The message literals of [`message_literals`] in the crates `names` alone.
fn message_literals_in(names: &[&str]) -> std::collections::BTreeSet<String> {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = std::collections::BTreeSet::new();
    let mut files = vec![];
    let mut stack: Vec<_> = names.iter().map(|c| crates.join(c).join("src")).collect();
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if name != "tests" {
                    stack.push(path);
                }
            } else if name.ends_with(".rs")
                && !name.starts_with("tests")
                && !name.ends_with("_tests.rs")
                && !(d == crates.join("ui-egui").join("src") && name == "control.rs")
            {
                // Control replies are English protocol data, never status-bar messages
                // (see i18n's module contract). Engine messages are scanned in their own crates.
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                files.push((path, text));
            }
        }
    }
    let crate_of = |path: &std::path::Path| names.iter().copied().find(|c| path.starts_with(crates.join(c))).unwrap_or("");
    let declared: Vec<_> = files.iter().map(|(path, text)| (crate_of(path), str_consts(without_test_module(text)))).collect();
    let mut by_crate: std::collections::BTreeMap<&str, std::collections::BTreeMap<String, Vec<String>>> = std::collections::BTreeMap::new();
    let mut everywhere: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for (krate, consts) in &declared {
        let in_crate = by_crate.entry(*krate).or_default();
        for (name, value) in consts {
            in_crate.entry(name.clone()).or_default().push(value.clone());
            everywhere.entry(name.clone()).or_default().push(value.clone());
        }
    }
    let none = std::collections::BTreeMap::new();
    let crate_consts: std::collections::BTreeMap<&str, _> =
        by_crate.iter().map(|(krate, in_crate)| (*krate, scoped_consts(&[], in_crate, &everywhere).into_owned())).collect();
    for ((path, text), (krate, own)) in files.iter().zip(&declared) {
        let helpers: Vec<_> =
            ERROR_HELPERS.iter().filter(|(c, _, _)| path.starts_with(crates.join(c))).map(|&(_, call, before)| (call, before)).collect();
        let consts = scoped_consts(own, crate_consts.get(krate).unwrap_or(&none), &none);
        out.extend(source_messages(without_test_module(text), &helpers, &consts));
    }
    out
}

/// The `&str` constants a file can name, from the constants it declares (`own`), those the files
/// of its crate declare (`in_crate`) and those every scanned crate declares (`everywhere`): a name
/// the file declares has the file's own text only, a name its crate declares has every text the
/// crate's files give it, and any other name has every text the scanned crates give it.
fn scoped_consts<'a>(
    own: &[(String, String)],
    in_crate: &'a std::collections::BTreeMap<String, Vec<String>>,
    everywhere: &std::collections::BTreeMap<String, Vec<String>>,
) -> std::borrow::Cow<'a, std::collections::BTreeMap<String, Vec<String>>> {
    let elsewhere = everywhere.keys().any(|name| !in_crate.contains_key(name));
    let own_differs = own.iter().any(|(name, _)| {
        let mine: Vec<&String> = own.iter().filter(|(n, _)| n == name).map(|(_, v)| v).collect();
        in_crate.get(name).is_none_or(|values| values.iter().collect::<Vec<_>>() != mine)
    });
    if !elsewhere && !own_differs {
        return std::borrow::Cow::Borrowed(in_crate);
    }
    let mut consts = everywhere.clone();
    for (name, values) in in_crate {
        consts.insert(name.clone(), values.clone());
    }
    for (name, _) in own {
        consts.remove(name);
    }
    for (name, value) in own {
        consts.entry(name.clone()).or_default().push(value.clone());
    }
    std::borrow::Cow::Owned(consts)
}

/// The `&str` constants `code` declares outside its comments (`const NAME: &str = "…";`), with
/// their text, escapes resolved.
fn str_consts(code: &str) -> Vec<(String, String)> {
    let code = &without_comments(code);
    code.match_indices("const ")
        .filter(|&(at, _)| !code[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
        .filter_map(|(at, m)| {
            let rest = &code[at + m.len()..];
            let (name, rest) = rest.split_at(rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_')?);
            let rest = rest.trim_start().strip_prefix(':')?.trim_start();
            let rest = rest.strip_prefix("&'static str").or_else(|| rest.strip_prefix("&str"))?.trim_start();
            let value = string_literal(rest.strip_prefix('=')?.trim_start().strip_prefix('"')?)?;
            Some((name.to_string(), value))
        })
        .collect()
}

/// The text of the constant that `rest` starts with, by name or by path (`TIME_BUDGET`,
/// `crate::runtime::SELECTION_TOO_LARGE.into()`), when `consts` holds it: every text, when crates
/// declare constants of that name with different texts.
fn named_const<'a>(rest: &str, consts: &'a std::collections::BTreeMap<String, Vec<String>>) -> &'a [String] {
    let path = &rest[..rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != ':').unwrap_or(rest.len())];
    let after = &rest[path.len()..];
    let used = [")", ",", ".into()", ".to_string()", ".to_owned()"].iter().any(|end| after.starts_with(end));
    let name = path.rsplit("::").next().unwrap_or("");
    let upper = name.starts_with(|c: char| c.is_ascii_uppercase()) && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if used && upper { consts.get(name).map_or(&[], Vec::as_slice) } else { &[] }
}

/// The message literals in `code` outside its comments, as catalog templates (see
/// [`message_literals`]): the literal after each of [`MESSAGE_MARKERS`], the message argument of an
/// error enum's variant, an enum whose name ends in `Error` or `Fail` (the first argument, as in
/// `PsError::Limit(…)`, `Error::Manifest(…)` and `CompileFail::Syntax(…)`, or the one
/// [`LATER_MESSAGE_VARIANTS`] gives; not [`NAME_VARIANTS`]),
/// the messages a `.map_err` closure returns (`.map_err(|e| format!("…: {e}"))`, see
/// [`closure_returns`]) or the handler a `.map_err(name)` names returns (see [`handler_returns`]),
/// and the message argument of each call in `helpers` (a call and the number of arguments before
/// its message), a literal, a `format!` string or the name of one of `consts` (see
/// [`str_consts`]), each one with or without a `&` before it. A key's name alone (`` `tint` ``)
/// is not a message. A `format!` string and an `#[error(…)]` string are format strings (see
/// [`as_template`]); in any other literal or constant every brace is literal (see
/// [`as_literal_key`]).
fn source_messages(code: &str, helpers: &[(&str, usize)], consts: &std::collections::BTreeMap<String, Vec<String>>) -> Vec<String> {
    let code = &without_comments(code);
    let after_markers =
        MESSAGE_MARKERS.iter().flat_map(|marker| code.match_indices(marker).map(|(at, _)| (&code[at + marker.len()..], *marker == "#[error(")));
    let in_variants = ["Error::", "Fail::"].into_iter().flat_map(|enum_end| code.match_indices(enum_end)).filter_map(|(at, m)| {
        let rest = &code[at + m.len()..];
        let (variant, args) = rest.split_at(rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(rest.len()));
        let named = variant.starts_with(|c: char| c.is_ascii_uppercase()) && !NAME_VARIANTS.contains(&variant);
        let before = LATER_MESSAGE_VARIANTS.iter().find(|(v, _)| *v == variant).map_or(0, |&(_, n)| n);
        if named { skip_args(args.strip_prefix('(')?, before).map(|rest| (rest, false)) } else { None }
    });
    let in_map_err = code.match_indices(".map_err(").flat_map(|(at, m)| {
        let rest = code[at + m.len()..].trim_start();
        let params = rest.strip_prefix("move").map_or(rest, str::trim_start).strip_prefix('|');
        params.and_then(|p| p.find('|').map(|end| &p[end + 1..])).map(closure_returns).unwrap_or_default()
    });
    let mut handlers = vec![];
    for (at, m) in code.match_indices(".map_err(") {
        let rest = &code[at + m.len()..];
        let name = &rest[..rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(rest.len())];
        if name.starts_with(|c: char| c.is_ascii_lowercase()) && rest[name.len()..].starts_with(')') && !handlers.contains(&name) {
            handlers.push(name);
        }
    }
    let in_handlers = handlers.into_iter().flat_map(|name| handler_returns(code, name));
    let in_helpers = helpers.iter().flat_map(|&(call, before)| {
        code.match_indices(call)
            .filter(|&(at, _)| call.starts_with('.') || !code[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '.'))
            .filter(|&(at, _)| {
                let owner = code[..at].strip_suffix("::").map(|p| p.rsplit(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or(""));
                !owner.is_some_and(|o| o.starts_with(|c: char| c.is_ascii_uppercase()))
            })
            // `format!(` given as a helper, and an `error(` helper that is an `#[error(…)]` attribute, take a format string.
            .filter_map(move |(at, _)| {
                skip_args(&code[at + call.len()..], before).map(|rest| (rest, call == "format!(" || code[..at].ends_with("#[")))
            })
    });
    let mut out = vec![];
    let in_map_err = in_map_err.map(|rest| (rest, false));
    let in_handlers = in_handlers.map(|rest| (rest, false));
    for (rest, takes_format) in after_markers.chain(in_variants).chain(in_map_err).chain(in_handlers).chain(in_helpers) {
        let rest = rest.trim_start();
        let rest = rest.strip_prefix('&').map_or(rest, str::trim_start);
        let formatted = rest.strip_prefix("format!(").map(str::trim_start);
        let format = takes_format || formatted.is_some();
        let rest = formatted.unwrap_or(rest);
        let lits = match rest.strip_prefix('"') {
            Some(quoted) => string_literal(quoted).into_iter().collect(),
            None => named_const(rest, consts).to_vec(),
        };
        for lit in lits {
            let has_word = lit.as_bytes().windows(3).any(|w| w.iter().all(u8::is_ascii_lowercase));
            let key_name = lit.len() > 1 && lit.starts_with('`') && lit.ends_with('`') && !lit.contains(' ');
            if has_word && !key_name && !NOT_MESSAGES.contains(&lit.as_str()) {
                out.push(if format { as_template(&lit) } else { as_literal_key(&lit) });
            }
        }
    }
    out
}

/// The arguments of a call whose opening parenthesis was just read, past the first `n` of them;
/// `None` when `n` is 1 or more and the call has `n` arguments or fewer.
fn skip_args(args: &str, n: usize) -> Option<&str> {
    if n == 0 {
        return Some(args);
    }
    let (mut left, mut depth, mut chars) = (n, 0usize, args.char_indices());
    while let Some((at, c)) = chars.next() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.checked_sub(1)?,
            '"' => loop {
                match chars.next()?.1 {
                    '\\' => {
                        chars.next()?;
                    }
                    '"' => break,
                    _ => {}
                }
            },
            ',' if depth == 0 => {
                left -= 1;
                if left == 0 {
                    return Some(&args[at + 1..]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Where each value the error handler `name` of a `.map_err(name)` in `code` can return starts:
/// the closure the file binds to that name (`let fail = |e| format!("…: {e}");`, up to its `;`)
/// or the body of the function of that name (`fn unreadable(e: …) -> String { … }`), read as
/// [`closure_returns`] reads a closure.
fn handler_returns<'a>(code: &'a str, name: &str) -> Vec<&'a str> {
    let (bound, declared) = (format!("let {name} = "), format!("fn {name}("));
    let mut out = vec![];
    for (at, m) in code.match_indices(bound.as_str()).chain(code.match_indices(declared.as_str())) {
        if code[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let rest = &code[at + m.len()..];
        if m == bound {
            let rest = rest.strip_prefix("move").map_or(rest, str::trim_start);
            if let Some(body) = rest.strip_prefix('|').and_then(|p| p.find('|').map(|end| &p[end + 1..])) {
                out.extend(closure_returns(statement(body)));
            }
        } else if let Some(open) = rest.find('{') {
            out.extend(closure_returns(&rest[open + 1..]));
        }
    }
    out
}

/// The statement `code` starts with, up to its `;` or the bracket that closes the block it is in.
fn statement(code: &str) -> &str {
    let (mut depth, mut chars) = (0usize, code.char_indices());
    while let Some((at, c)) = chars.next() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => match depth.checked_sub(1) {
                Some(d) => depth = d,
                None => return &code[..at],
            },
            ';' if depth == 0 => return &code[..at],
            '"' => loop {
                match chars.next() {
                    Some((_, '\\')) => {
                        chars.next();
                    }
                    Some((_, '"')) | None => break,
                    _ => {}
                }
            },
            _ => {}
        }
    }
    code
}

/// Where each value a closure can return starts, `body` being the text after its parameters: the
/// body itself, each block or `if`/`else` branch in it (`{ ACB_CUT_SHORT.into() }`, not the
/// `{` of a `json!({…})` value), each `match` arm (`=>`), each statement after a `;` or after a
/// block statement's `}` (the last expression of a block or a function body) and the value of
/// each `return`, up to the bracket that closes the call the closure is given to or the function.
/// The blocks, arms and statements inside a `let` statement give a local its value, so they count
/// only when the local's name is used, outside a string literal and a `format!`, in a value the closure returns
/// (`let why = if … { format!("…") } else { … }; Error(why)`), and not when the local only goes
/// elsewhere (`let level = if … { "warning" } else { "error" }; log(level);`). A `return` counts
/// wherever it is, except inside a closure the body holds (`unwrap_or_else(|| return …)`).
fn closure_returns(body: &str) -> Vec<&str> {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    // The name a `let` statement starting at `at` binds (empty for a pattern), if it is one.
    let let_at = |at: usize| {
        let rest = body[at..].trim_start().strip_prefix("let").filter(|r| r.starts_with(char::is_whitespace))?.trim_start();
        let rest = rest.strip_prefix("mut ").map_or(rest, str::trim_start);
        Some(&rest[..rest.find(|c: char| !ident(c)).unwrap_or(rest.len())])
    };
    // Each place a value starts, with the `let` whose local it gives a value to, if any, and
    // whether a `return` gives it.
    let mut found: Vec<(&str, Option<&str>, bool)> = vec![(body, None, false)];
    // For each open bracket, the `quiet` and `in_let` of the text around it: `quiet` names the
    // `let` that a value inside the bracket belongs to, `in_let` the `let` the statement is.
    let mut outer = vec![];
    let (mut quiet, mut in_let, mut chars) = (None, let_at(0), body.char_indices());
    // Where the innermost closure in the body that a `return` would leave ends.
    let mut inner_closure_end = 0;
    while let Some((at, c)) = chars.next() {
        match c {
            '|' if at >= inner_closure_end => {
                let before = body[..at].trim_end();
                let after_operator = before.ends_with(['(', ',', '=']) && !["==", "!=", "<=", ">="].iter().any(|op| before.ends_with(op));
                let opens = after_operator || (before.ends_with("move") && !before[..before.len() - 4].ends_with(ident));
                let params_end = if body[at + 1..].starts_with('|') { Some(at + 1) } else { body[at + 1..].find('|').map(|end| at + 1 + end) };
                if let Some(params_end) = params_end.filter(|_| opens) {
                    inner_closure_end = params_end + 1 + statement(&body[params_end + 1..]).len();
                }
            }
            '(' | '[' | '{' => {
                outer.push((quiet, in_let));
                if c == '{' && !body[..at].trim_end().ends_with('(') {
                    found.push((&body[at + 1..], quiet.or(in_let), false));
                }
                quiet = quiet.or(in_let);
                in_let = if c == '{' { let_at(at + 1) } else { None };
            }
            ')' | ']' | '}' => {
                let Some(around) = outer.pop() else { break };
                (quiet, in_let) = around;
                if c == '}' {
                    found.push((&body[at + 1..], quiet.or(in_let), false));
                    if in_let.is_none() {
                        in_let = let_at(at + 1);
                    }
                }
            }
            ';' => {
                found.push((&body[at + 1..], quiet, false));
                in_let = let_at(at + 1);
            }
            '=' if body[at + 1..].starts_with('>') => found.push((&body[at + 2..], quiet.or(in_let), false)),
            'r' if at >= inner_closure_end
                && body[at..].starts_with("return")
                && !body[..at].ends_with(ident)
                && !body[at + "return".len()..].starts_with(ident) =>
            {
                found.push((&body[at + "return".len()..], None, true));
            }
            '"' => loop {
                match chars.next() {
                    Some((_, '\\')) => {
                        chars.next();
                    }
                    Some((_, '"')) | None => break,
                    _ => {}
                }
            },
            _ => {}
        }
    }
    // The values the closure returns: each `return`, and each expression that ends its block
    // rather than a `;` (a block's start, which spans the whole block, aside).
    let returned: Vec<String> = found
        .iter()
        .filter(|&&(start, held, by_return)| {
            let tail = !start.trim_start().starts_with('{') && !start[statement(start).len()..].starts_with(';');
            held.is_none() && (by_return || tail)
        })
        .map(|(start, ..)| without_format_args(&without_strings(statement(start))))
        .collect();
    let used = |name: &str| !name.is_empty() && returned.iter().any(|value| holds_ascii_word(value, name));
    found.into_iter().filter(|&(_, held, _)| held.is_none_or(used)).map(|(start, ..)| start).collect()
}

/// `code`, its string literals already emptied, with the arguments of each `format!(…)` removed:
/// a value written into a format string is part of a message, not one of its own.
fn without_format_args(code: &str) -> String {
    let (mut out, mut rest) = (String::new(), code);
    while let Some(at) = rest.find("format!(") {
        out.push_str(&rest[..at + "format!(".len()]);
        rest = &rest[at + "format!(".len()..];
        let mut depth = 0usize;
        let end = rest.char_indices().find_map(|(i, c)| match c {
            '(' | '[' | '{' => {
                depth += 1;
                None
            }
            ')' | ']' | '}' => match depth.checked_sub(1) {
                Some(d) => {
                    depth = d;
                    None
                }
                None => Some(i),
            },
            _ => None,
        });
        rest = &rest[end.unwrap_or(rest.len())..];
    }
    out.push_str(rest);
    out
}

/// `code` with the text of its string literals removed, the quotes kept.
fn without_strings(code: &str) -> String {
    let (mut out, mut chars) = (String::new(), code.chars());
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '"' {
            loop {
                match chars.next() {
                    Some('\\') => {
                        chars.next();
                    }
                    Some('"') => {
                        out.push('"');
                        break;
                    }
                    Some(_) => {}
                    None => break,
                }
            }
        }
    }
    out
}

/// Source up to its inline test module (`#[cfg(test)] mod tests {`, or a `cfg(all(test, …))` one);
/// declarations of test files (`#[cfg(test)] mod tests_x;`) don't end it.
fn without_test_module(text: &str) -> &str {
    for (at, m) in text.match_indices("#[cfg(") {
        let rest = &text[at + m.len()..];
        let Some(end) = rest.find(")]") else { continue };
        let cond: String = rest[..end].chars().filter(|c| !c.is_whitespace()).collect();
        if cond != "test" && !cond.starts_with("all(test,") {
            continue;
        }
        let after = rest[end + 2..].trim_start();
        if let Some(rest) = after.strip_prefix("mod ")
            && rest.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_').trim_start().starts_with('{')
        {
            return &text[..at];
        }
    }
    text
}

/// The text of a string literal whose opening quote was just read, escapes resolved as Rust resolves
/// them (https://doc.rust-lang.org/reference/tokens.html#character-escapes): `\n`, `\r`, `\t`, `\0`,
/// `\\`, `\'`, `\"`, `\x` with two hex digits up to `7F`, and `\u{…}` with one to six hex digits
/// (underscores allowed after the first) naming a Unicode scalar value. A `\` at the end of a line
/// drops the line break and the whitespace that starts the next line. A literal with any other
/// escape, or one left unclosed, is `None`.
fn string_literal(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut it = s.chars();
    loop {
        match it.next()? {
            '"' => return Some(out),
            '\\' => match it.next()? {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '0' => out.push('\0'),
                c @ ('\\' | '\'' | '"') => out.push(c),
                'x' => {
                    let (high, low) = (it.next()?.to_digit(16)?, it.next()?.to_digit(16)?);
                    out.push(char::from_u32(high * 16 + low).filter(char::is_ascii)?);
                }
                'u' => {
                    if it.next()? != '{' {
                        return None;
                    }
                    let (mut value, mut digits) = (0u32, 0);
                    loop {
                        match it.next()? {
                            '}' if digits > 0 => break,
                            '_' if digits > 0 => {}
                            c => {
                                value = value * 16 + c.to_digit(16)?;
                                digits += 1;
                                if digits > 6 {
                                    return None;
                                }
                            }
                        }
                    }
                    out.push(char::from_u32(value)?);
                }
                '\n' | '\r' => it = it.as_str().trim_start_matches([' ', '\t', '\n', '\r']).chars(),
                _ => return None,
            },
            c => out.push(c),
        }
    }
}

/// A format string as a catalog template (see [`message_literals`]). `{{` and `}}` stay as they
/// are, the catalogs' literal braces.
fn as_template(s: &str) -> String {
    let mut out = String::new();
    let mut unnamed = 0;
    let mut rest = s;
    while let Some(a) = rest.find('{') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        if let Some(escaped) = after.strip_prefix('{') {
            out.push_str("{{");
            rest = escaped;
            continue;
        }
        let Some(b) = after.find('}') else {
            out.push_str(&rest[a..]);
            return out;
        };
        let name = after[..b].split(':').next().unwrap_or("");
        if name.is_empty() {
            unnamed += 1;
            out.push_str(&format!("{{_{unnamed}}}"));
        } else {
            out.push_str(&format!("{{{name}}}"));
        }
        rest = &after[b + 1..];
    }
    out.push_str(rest);
    out
}

/// A message that is not a format string as a catalog key: each of its braces is literal, so it is
/// written `{{` or `}}`.
fn as_literal_key(s: &str) -> String {
    s.replace('{', "{{").replace('}', "}}")
}

/// (context, English source) of the labels [`untranslated_report`] lists for a language without
/// their rows: the file dialogs' filter names, the status bar's timings, New Document's preset
/// name, the contributor tooltips, the gradient kind in a swatch's description, the printer mark
/// styles and the label of the 3D gizmo's light.
const RENDER_SITE_ROWS: &[(&str, &str)] = &[
    ("file-filter", "All readable files"),
    ("file-filter", "All placeable files"),
    ("file-filter", "Swatch libraries"),
    ("file-filter", "Flattener presets"),
    ("file-filter", "PDF presets"),
    ("file-filter", "Print presets"),
    ("file-filter", "Export presets"),
    ("file-filter", "SVG Compressed"),
    ("file-filter", "PDF-compatible .ai"),
    ("file-filter", "VectorCraft Template"),
    ("file-filter", "Templates"),
    ("anchor", "Points:"),
    ("diagnostics", "render"),
    ("diagnostics", "ui"),
    ("", "Preset {number}"),
    ("", "PRs"),
    ("", "@{login}: {prs}, {commits}, +{added} / −{deleted} lines (Δ {delta}), +{binary_added} / −{binary_deleted} binary assets, {first} – {last}"),
    ("@plural", "{n} PR|{n} PRs"),
    ("@plural", "{n} commit|{n} commits"),
    ("gradient-kind", "Radial"),
    ("mark-style", "Japanese"),
    ("", "Roman"),
    ("lighting", "Light"),
];
