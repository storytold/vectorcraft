//! Import Shortcut Set: entry points for a shortcut set another app saved as text (`.kys`, read by
//! `shortcutSet.read`). The set is handed to Keyboard Shortcuts, whose review matches, decodes and
//! plans it ([`craft::keymap::import`]); this module maps the read value to a [`ForeignSet`] with the
//! user's earlier answers (`importMatches`), and keeps the answers the review reports.
//!
//! The two codes the format's public description gives (`/Key 48` is the 0 key and `/Modifiers 0` is
//! none) are decided here; every other code is decided by the framework's agreement rule or by the user.

use std::collections::BTreeMap;

use craft::host::{PickRequest, Picked};
use craft::keymap::import::{MAX_IMPORT_TEXT, MAX_PATH_SEGMENTS, NotRead};
use craft::keymap::{CommandRef, ForeignEntry, ForeignKey, ForeignSet, ImportOptions, NoneDefined, Start};
use serde_json::{Value, json};

use crate::file_asks::{self, Owner};
use crate::{VectorcraftApp, shortcut_editor};

/// The files Keyboard Shortcuts' Import… lists: VectorCraft's own sets and sets another app saved as
/// text.
pub const IMPORT_EXTS: &[&str] = &["json", "kys"];

/// The codes the format's public description gives: `/Key 48` is the 0 key.
const DESCRIBED_KEYS: &[(i64, &str)] = &[(48, "0")];
/// `/Modifiers 0` is none.
const DESCRIBED_MODS: &[(i64, &str)] = &[(0, "")];

/// The file dialog that opens a shortcut set: one of [`IMPORT_EXTS`].
pub(crate) fn open_request() -> PickRequest {
    PickRequest::open(shortcut_editor::SHORTCUT_SETS).filter("Shortcut sets", IMPORT_EXTS)
}

/// A file named as a shortcut set another app saves (`.kys`).
pub fn is_kys(path: &str) -> bool {
    vectorcraft_engine::cmd::shortcutset::EXTS.contains(&vectorcraft_engine::cmd::fileio::extension(path).as_str())
}

/// A set another app saved as text: a `.kys` file, or text that starts with a slash name.
pub fn is_text_set(path: &str, bytes: &[u8]) -> bool {
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    is_kys(path) || text.trim_ascii_start().starts_with(b"/")
}

/// Open the review of the shortcut set in `bytes`, a file called `name` (a path or a file name) →
/// `{entries, unread}`.
pub fn open(app: &mut VectorcraftApp, name: &str, bytes: &[u8]) -> Result<Value, String> {
    // Only the part `shortcutSet.read` takes in.
    let head = bytes.get(..vectorcraft_engine::cmd::shortcutset::READ_LIMIT).unwrap_or(bytes);
    open_read(app, json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(head) }))
}

/// Open the review of the set `shortcutSet.read` reads with `params`, in Keyboard Shortcuts (opened
/// when it is not).
fn open_read(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    let read = app.run("shortcutSet.read", params)?;
    let count = |field: &str| read[field].as_array().map_or(0, Vec::len);
    let summary = json!({ "entries": count("entries"), "unread": count("unread") });
    let review = foreign_set(&read);
    if app.shortcuts_dialog.is_none() {
        shortcut_editor::open(app);
    }
    if let Some(dialog) = app.shortcuts_dialog.as_mut() {
        dialog.pending_review = Some(review);
    }
    Ok(summary)
}

/// `ui.shortcutImportDialog`: the review of the file at `path`, or in `dataBase64` (called `name`),
/// else of a picked one.
pub fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    if let Some(b64) = s("dataBase64") {
        return open_read(app, json!({ "name": s("name").unwrap_or_default(), "dataBase64": b64 }));
    }
    match s("path") {
        Some(path) => open_read(app, json!({ "path": path })),
        None => {
            // A shortcut set belongs to no document.
            Ok(file_asks::ask(app, open_request(), Owner::Free, |app, picked| match picked {
                Picked::Contents(files) => {
                    let (name, bytes) = files.into_iter().next().ok_or("cancelled")?;
                    open(app, &name, &bytes)
                }
                picked => open_read(app, json!({ "path": picked.first_path().ok_or("cancelled")?.to_string_lossy() })),
            }))
        }
    }
}

/// Text cut to what [`craft::keymap::plan`] reads, so an entry's [`ForeignEntry::remember_key`] is
/// the one `plan` looks up.
fn within_cap(s: &str) -> String {
    s.chars().take(MAX_IMPORT_TEXT).collect()
}

/// `shortcutSet.read`'s result as a [`ForeignSet`] and the options that carry the user's earlier
/// answers: each entry's saved target, and each code's saved or described text.
pub(crate) fn foreign_set(read: &Value) -> (ForeignSet, ImportOptions) {
    let mut entries = vec![];
    let mut remembered = BTreeMap::new();
    for e in read["entries"].as_array().into_iter().flatten() {
        // Room for the name within the segments `plan` reads.
        let mut path: Vec<String> = e["section"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(MAX_PATH_SEGMENTS.saturating_sub(2))
            .map(within_cap)
            .collect();
        path.push(within_cap(e["name"].as_str().unwrap_or_default()));
        let code = |field: &str| e[field].as_u64().and_then(|n| u32::try_from(n).ok());
        let (key, mods) = (code("key"), code("modifiers"));
        let text = [key.map(|k| format!("/Key {k}")), mods.map(|m| format!("/Modifiers {m}"))].into_iter().flatten().collect::<Vec<_>>().join(" ");
        let mut entry = ForeignEntry::new(None, path).with_key(ForeignKey::code(None, &text, key, mods));
        if let Some(line) = e["line"].as_u64().and_then(|n| usize::try_from(n).ok()) {
            entry = entry.at_line(line);
        }
        if let Some(target) = e["target"].as_str() {
            // An entry answered with a tool selects one: the review matches it among the tools.
            if target.starts_with("tool:") {
                entry = entry.tool();
            }
            remembered.insert(entry.remember_key(), (!target.is_empty()).then(|| crate::keymap::command_of(target)));
        }
        entries.push(entry);
    }
    let mut codes = BTreeMap::new();
    for (list, field, kind) in [("keys", "key", "key"), ("modifiers", "mods", "mods")] {
        for c in read["codes"][list].as_array().into_iter().flatten() {
            let (Some(value), Some(text)) = (c["value"].as_i64(), c[field].as_str()) else { continue };
            // A key with no equivalent is answered `None`; no modifiers is the empty text.
            codes.insert(format!("{kind}:{value}"), (kind == "mods" || !text.is_empty()).then(|| text.to_string()));
        }
    }
    for (kind, described) in [("key", DESCRIBED_KEYS), ("mods", DESCRIBED_MODS)] {
        for (value, text) in described {
            codes.entry(format!("{kind}:{value}")).or_insert_with(|| Some(text.to_string()));
        }
    }
    let not_read = read["unread"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|n| {
            let line = n["line"].as_u64().and_then(|l| usize::try_from(l).ok());
            NotRead::new(line, n["text"].as_str().unwrap_or_default().to_string())
        })
        .collect();
    let mut set = ForeignSet::new(entries).with_truncated(usize::from(read["truncated"].as_bool().unwrap_or(false))).with_not_read(not_read);
    set.name = read["set"].as_str().map(str::to_string);
    let options =
        ImportOptions { start: Start::Defaults, none_defined: NoneDefined::Keep, suggest: true, remembered, codes, ..ImportOptions::default() };
    (set, options)
}

/// Keep for later imports the answers the review reported: `targets` by remember key (`None` is "No
/// equivalent") and `codes` by `key:N` / `mods:N`, through `importMatches.set`. A name match is keyed
/// by the entry's name as the file writes it, which `set` supplies for each remember key.
pub(crate) fn remember(
    app: &mut VectorcraftApp,
    set: &ForeignSet,
    targets: &BTreeMap<String, Option<CommandRef>>,
    codes: &BTreeMap<String, Option<String>>,
) {
    let mut matches = vec![];
    for entry in &set.entries {
        let (Some(answer), Some(name)) = (targets.get(&entry.remember_key()), entry.path.last()) else { continue };
        let mut m = json!({ "kind": "name", "name": name, "target": answer.as_ref().map(crate::keymap::entry_key).unwrap_or_default() });
        let label = answer.as_ref().and_then(|c| app.shortcuts_dialog.as_ref()?.keymap.as_ref()?.bindable(c)).map(|d| d.label.clone());
        if let Some(label) = label {
            m["label"] = json!(label);
        }
        matches.push(m);
    }
    for (code, text) in codes {
        let (kind, value) = match (code.strip_prefix("key:"), code.strip_prefix("mods:")) {
            (Some(v), _) => ("key", v),
            (_, Some(v)) => ("modifiers", v),
            _ => continue,
        };
        matches.push(json!({ "kind": kind, "name": value, "target": text.clone().unwrap_or_default() }));
    }
    if matches.is_empty() {
        return;
    }
    if let Err(e) = app.run("importMatches.set", json!({ "matches": matches }))
        && let Some(dialog) = app.shortcuts_dialog.as_mut()
    {
        dialog.state.show_error(e);
    }
}
