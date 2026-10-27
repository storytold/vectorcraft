//! Import Shortcut Set: a shortcut set another app saved as text (`.kys`, read by `shortcutSet.read`),
//! reviewed before it fills Edit › Keyboard Shortcuts' working set.
//!
//! Each entry's name is matched against the tools, commands and panels Keyboard Shortcuts lists
//! ([`craft_keymap::import::namematch`]). Its key and modifier codes are decoded from the user's earlier
//! answers (`importMatches`), from the two codes the format's public description gives (`/Key 48` is
//! the 0 key and `/Modifiers 0` is none), and by agreement with VectorCraft's default shortcuts: each
//! entry whose match has a default votes for what its codes stand for, and a code is proposed when at
//! least two entries vote for it and four votes in five agree. The user answers the rest in the Keys
//! section. Apply hands the rows that are ready, in the file's order, to [`craft_keymap::plan`], each
//! row's target given as the user's answer for it: when two rows want one shortcut, the later row
//! takes it, and the tools, commands and panels outside the set lose the shortcuts the set gives.
//! Keyboard Shortcuts then shows the set, and its OK keeps it.
//!
//! Fields: `name`, `start` (`defaults` or `current`), `remember`, `rows` ([`Row`]s), `keys` and `mods`
//! ([`Code`]s by code), `notes` (`[{line, text}]`: the parts of the file not read), `truncated`,
//! `__parent` (the Keyboard Shortcuts dialog it was opened from, which Apply, Cancel and Escape go back
//! to), `__recording` (the key code waiting for a key press), `__keysOpen` (the Keys section is open:
//! at first when a code is not known), and `__query` and `__attention` (the filter of the rows shown).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;

use craft_keymap::import::namematch::{Candidate, Matcher, Proposal};
use craft_keymap::import::{MAX_IMPORT_TEXT, MAX_PATH_SEGMENTS};
use craft_keymap::{CommandRef, ForeignEntry, ForeignKey, ForeignSet, ImportOptions, Keymap, NoneDefined, Start, Trigger, plan};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::shortcut_editor::{self, Entry};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{FilePick, VectorcraftApp, menus, widgets};

/// The dialog kind.
pub const KIND: &str = "shortcutImport";

/// The files Keyboard Shortcuts' Import… lists: VectorCraft's own sets and sets another app saved as
/// text.
pub const IMPORT_EXTS: &[&str] = &["json", "kys"];

/// The codes the format's public description gives: `/Key 48` is the 0 key.
const DESCRIBED_KEYS: &[(i64, &str)] = &[(48, "0")];
/// `/Modifiers 0` is none.
const DESCRIBED_MODS: &[(i64, &str)] = &[(0, "")];

/// The least votes a code needs for agreement to propose it.
const MIN_VOTES: usize = 2;
/// The share of the votes that must agree: four in five.
const AGREE: (usize, usize) = (4, 5);
/// The most entry names a code lists.
const NAMES_SHOWN: usize = 3;

/// The modifiers the Keys section offers, as Edit › Keyboard Shortcuts writes them ("" is none).
const MODIFIERS: &[&str] = &[
    "",
    "Cmd",
    "Ctrl",
    "Alt",
    "Shift",
    "Cmd+Ctrl",
    "Cmd+Alt",
    "Cmd+Shift",
    "Ctrl+Alt",
    "Ctrl+Shift",
    "Alt+Shift",
    "Cmd+Ctrl+Alt",
    "Cmd+Ctrl+Shift",
    "Cmd+Alt+Shift",
    "Ctrl+Alt+Shift",
    "Cmd+Ctrl+Alt+Shift",
];

/// The table's row height and column widths (points).
const ROW_H: f32 = 22.0;
const NAME_W: f32 = 200.0;
const TARGET_W: f32 = 230.0;
const CHORD_W: f32 = 90.0;
/// The tallest the Keys and Shortcuts lists get (points).
const KEYS_LIST_H: f32 = 140.0;
const ROWS_LIST_H: f32 = 280.0;

/// An entry of the file and what it becomes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Row {
    /// As the file writes it.
    pub name: String,
    /// The names of the blocks around it (`Menus › Edit`).
    pub section: String,
    pub line: usize,
    pub key: Option<i64>,
    pub modifiers: Option<i64>,
    /// A key of [`shortcut_editor::entries`], or "" for no equivalent; none until chosen.
    pub target: Option<String>,
    /// Where the target came from: `saved` (an earlier import), `matched` (the name) or `you`.
    pub from: String,
    /// The candidates to choose from when the name matched none clearly, best first.
    pub choices: Vec<String>,
    /// Checked: Apply assigns it.
    pub on: bool,
}

/// What a key or modifier code stands for.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Code {
    /// A key, or modifiers, as Edit › Keyboard Shortcuts writes them: "" for a key with no equivalent
    /// or for no modifiers. None until known.
    pub text: Option<String>,
    /// `saved` (an earlier import), `described` (the format's description), `agreement` or `you`; ""
    /// while unknown.
    pub from: String,
    /// For `agreement`: the entries whose default shortcut agrees.
    pub votes: usize,
    /// The first few entries that use it.
    pub names: Vec<String>,
}

/// What happens to a row on Apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ready,
    /// Unchecked.
    Off,
    NeedsChoice,
    NeedsKey,
    NoEquivalent,
    /// A tool's shortcut with Cmd, Alt or Ctrl.
    ToolKey,
    /// A later row takes the same shortcut.
    SameShortcut,
    /// A later row gives the same tool, command or panel its shortcut.
    SameTarget,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => tl!("Ready"),
            Self::Off => tl!("Off"),
            Self::NeedsChoice => tl!("Needs a choice"),
            Self::NeedsKey => tl!("Needs a key"),
            Self::NoEquivalent => tl!("No equivalent"),
            Self::ToolKey => tl!("Tools take a single key (optionally with Shift)"),
            Self::SameShortcut => tl!("A later row takes this shortcut"),
            Self::SameTarget => tl!("Same item as a later row"),
        }
    }

    /// The user has something to answer or change.
    pub fn needs_attention(self) -> bool {
        matches!(self, Self::NeedsChoice | Self::NeedsKey | Self::ToolKey | Self::SameShortcut | Self::SameTarget)
    }
}

/// The rows and codes of a review.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Review {
    pub rows: Vec<Row>,
    pub keys: BTreeMap<i64, Code>,
    pub mods: BTreeMap<i64, Code>,
}

impl Review {
    /// The review of `read`, `shortcutSet.read`'s result, with names matched against `candidates`
    /// ([`candidates`]).
    pub fn build(read: &Value, candidates: &[Candidate]) -> Self {
        let matcher = Matcher::new(candidates);
        let id = |i: usize| candidates.get(i).map(|c| c.id.clone());
        let mut rows = vec![];
        for e in read["entries"].as_array().into_iter().flatten() {
            let name = e["name"].as_str().unwrap_or_default().to_string();
            let (target, from, choices) = match e["target"].as_str() {
                Some(t) => (Some(t.to_string()), "saved", vec![]),
                None => match matcher.propose(&name) {
                    Proposal::Match(i, _) => (id(i), "matched", vec![]),
                    Proposal::Choice(best) => (None, "", best.into_iter().filter_map(id).collect()),
                },
            };
            rows.push(Row {
                section: e["section"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(" › "),
                line: e["line"].as_u64().unwrap_or(0) as usize,
                key: e["key"].as_i64(),
                modifiers: e["modifiers"].as_i64(),
                target,
                from: from.into(),
                choices,
                on: true,
                name,
            });
        }
        let codes = |list: &Value, field: &str, described: &[(i64, &str)], of: fn(&Row) -> Option<i64>| -> BTreeMap<i64, Code> {
            // The first few names for each code, in one pass over the rows: a scan per code made a
            // file of many distinct codes cost codes × rows.
            let mut examples: BTreeMap<i64, Vec<String>> = BTreeMap::new();
            for r in &rows {
                if let Some(value) = of(r) {
                    let names = examples.entry(value).or_default();
                    if names.len() < NAMES_SHOWN {
                        names.push(r.name.clone());
                    }
                }
            }
            let mut out = BTreeMap::new();
            for c in list.as_array().into_iter().flatten() {
                let Some(value) = c["value"].as_i64() else { continue };
                let names = examples.get(&value).cloned().unwrap_or_default();
                let (text, from) = match (c[field].as_str(), described.iter().find(|(v, _)| *v == value)) {
                    (Some(t), _) => (Some(t.to_string()), "saved"),
                    (None, Some((_, t))) => (Some(t.to_string()), "described"),
                    (None, None) => (None, ""),
                };
                out.insert(value, Code { text, from: from.into(), votes: 0, names });
            }
            out
        };
        let keys = codes(&read["codes"]["keys"], "key", DESCRIBED_KEYS, |r| r.key);
        let mods = codes(&read["codes"]["modifiers"], "mods", DESCRIBED_MODS, |r| r.modifiers);
        let mut review = Self { rows, keys, mods };
        review.decode();
        review
    }

    /// The review `d` holds.
    pub fn of(d: &Dialog) -> Self {
        fn field<T: serde::de::DeserializeOwned + Default>(d: &Dialog, k: &str) -> T {
            d.fields.get(k).and_then(|v| T::deserialize(v).ok()).unwrap_or_default()
        }
        Self { rows: field(d, "rows"), keys: field(d, "keys"), mods: field(d, "mods") }
    }

    fn store(&self, d: &mut Dialog) {
        d.fields.insert("rows".into(), serde_json::to_value(&self.rows).unwrap_or_else(|_| json!([])));
        d.fields.insert("keys".into(), serde_json::to_value(&self.keys).unwrap_or_else(|_| json!({})));
        d.fields.insert("mods".into(), serde_json::to_value(&self.mods).unwrap_or_else(|_| json!({})));
    }

    /// Codes that no earlier import, description or user answered take what agreement with
    /// VectorCraft's default shortcuts proposes for them now.
    pub fn decode(&mut self) {
        let mut defaults: HashMap<&str, Option<(String, String)>> = HashMap::new();
        let mut keys: BTreeMap<i64, HashMap<String, usize>> = BTreeMap::new();
        let mut mods: BTreeMap<i64, HashMap<String, usize>> = BTreeMap::new();
        for r in &self.rows {
            let Some(t) = r.target.as_deref().filter(|t| !t.is_empty()) else { continue };
            let default =
                defaults.entry(t).or_insert_with(|| shortcut_editor::default_of(t).and_then(shortcut_editor::normalize).map(|c| split_chord(&c)));
            let Some((m, k)) = default else { continue };
            if let Some(code) = r.key {
                *keys.entry(code).or_default().entry(k.clone()).or_default() += 1;
            }
            if let Some(code) = r.modifiers {
                *mods.entry(code).or_default().entry(m.clone()).or_default() += 1;
            }
        }
        for (codes, votes) in [(&mut self.keys, keys), (&mut self.mods, mods)] {
            for (code, c) in codes.iter_mut().filter(|(_, c)| c.from.is_empty() || c.from == "agreement") {
                let names = std::mem::take(&mut c.names);
                *c = match votes.get(code).and_then(agreed) {
                    Some((text, votes)) => Code { text: Some(text), from: "agreement".into(), votes, names },
                    None => Code { names, ..Code::default() },
                };
            }
        }
    }

    /// Row `r`'s shortcut as Edit › Keyboard Shortcuts writes it: `Some(None)` when its key has no
    /// equivalent, none while one of its codes isn't known. An entry without `/Modifiers` takes none.
    pub fn chord(&self, r: &Row) -> Option<Option<String>> {
        let key = self.keys.get(&r.key?)?.text.as_deref()?;
        // A key with no equivalent needs no modifiers.
        if key.is_empty() {
            return Some(None);
        }
        let mods = match r.modifiers {
            Some(m) => self.mods.get(&m)?.text.as_deref()?,
            None => "",
        };
        Some(shortcut_editor::normalize(&if mods.is_empty() { key.to_string() } else { format!("{mods}+{key}") }))
    }

    fn status(&self, r: &Row) -> (Status, Option<String>) {
        if !r.on {
            return (Status::Off, None);
        }
        let target = match r.target.as_deref() {
            None => return (Status::NeedsChoice, None),
            Some("") => return (Status::NoEquivalent, None),
            Some(t) if !known(t) => return (Status::NeedsChoice, None),
            Some(t) => t,
        };
        match self.chord(r) {
            None => (Status::NeedsKey, None),
            Some(None) => (Status::NoEquivalent, None),
            Some(Some(c)) if target.starts_with("tool:") && !single_key(&c) => (Status::ToolKey, Some(c)),
            Some(Some(c)) => (Status::Ready, Some(c)),
        }
    }

    /// Each row's status, with the shortcut it takes. When ready rows give the same tool, command or
    /// panel a shortcut, or want the same shortcut, the last of them does.
    pub fn statuses(&self) -> Vec<(Status, Option<String>)> {
        let mut out: Vec<(Status, Option<String>)> = self.rows.iter().map(|r| self.status(r)).collect();
        let (mut targets, mut chords) = (HashSet::new(), HashSet::new());
        for (r, (status, chord)) in self.rows.iter().zip(out.iter_mut()).rev() {
            let (Status::Ready, Some(t), Some(c)) = (*status, r.target.as_deref(), chord.as_ref()) else { continue };
            if targets.contains(t) {
                *status = Status::SameTarget;
            } else if chords.contains(c) {
                *status = Status::SameShortcut;
            } else {
                targets.insert(t);
                chords.insert(c.clone());
            }
        }
        out
    }

    /// Some code the Keys section lists is not known yet.
    fn asks(&self) -> bool {
        let (keys, mods) = self.asked();
        keys.iter().any(|k| self.keys.get(k).is_some_and(|c| c.text.is_none()))
            || mods.iter().any(|m| self.mods.get(m).is_some_and(|c| c.text.is_none()))
    }

    /// The codes the Keys section lists: those of the checked rows with a target, the unknown ones
    /// first.
    fn asked(&self) -> (Vec<i64>, Vec<i64>) {
        let (mut keys, mut mods) = (BTreeSet::new(), BTreeSet::new());
        for r in self.rows.iter().filter(|r| r.on && r.target.as_deref().is_some_and(|t| !t.is_empty())) {
            keys.extend(r.key.filter(|k| self.keys.contains_key(k)));
            mods.extend(r.modifiers.filter(|m| self.mods.contains_key(m)));
        }
        let unknown_first = |set: BTreeSet<i64>, codes: &BTreeMap<i64, Code>| {
            let mut v: Vec<i64> = set.into_iter().collect();
            v.sort_by_key(|c| codes.get(c).is_some_and(|c| c.text.is_some()));
            v
        };
        (unknown_first(keys, &self.keys), unknown_first(mods, &self.mods))
    }
}

/// The text most votes agree on, with its votes, when at least [`MIN_VOTES`] voted and four votes in
/// five agree.
fn agreed(votes: &HashMap<String, usize>) -> Option<(String, usize)> {
    let total: usize = votes.values().sum();
    let (text, n) = votes.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))?;
    (total >= MIN_VOTES && n * AGREE.1 >= total * AGREE.0).then(|| (text.clone(), *n))
}

/// A shortcut as written (`Cmd+Shift+K`) split into its modifiers (`Cmd+Shift`, "" for none) and key.
fn split_chord(chord: &str) -> (String, String) {
    match chord.rsplit_once('+') {
        Some((m, k)) => (m.to_string(), k.to_string()),
        None => (String::new(), chord.to_string()),
    }
}

/// A shortcut a tool can take: one key, optionally with Shift.
fn single_key(chord: &str) -> bool {
    crate::shortcuts::parse(chord).is_some_and(|sc| !(sc.modifiers.command || sc.modifiers.alt || sc.modifiers.ctrl))
}

/// What a name can stand for: Keyboard Shortcuts' entries, by their English labels and the labels in
/// the interface language.
pub fn candidates() -> Vec<Candidate> {
    shortcut_editor::entries()
        .iter()
        .map(|e| {
            let shown = tl!(&e.label);
            let labels = if shown == e.label { vec![e.label.clone()] } else { vec![e.label.clone(), shown.to_string()] };
            Candidate { id: e.key.clone(), labels }
        })
        .collect()
}

/// Keyboard Shortcuts' entry `key`.
fn entry(key: &str) -> Option<&'static Entry> {
    static BY_KEY: OnceLock<HashMap<&'static str, &'static Entry>> = OnceLock::new();
    BY_KEY.get_or_init(|| shortcut_editor::entries().iter().map(|e| (e.key.as_str(), e)).collect()).get(key).copied()
}

/// A tool, command or panel that Keyboard Shortcuts can give a shortcut.
fn known(key: &str) -> bool {
    entry(key).is_some() || shortcut_editor::default_of(key).is_some()
}

/// How Keyboard Shortcuts names entry `e`: a tool by its label, the others after their menu.
fn shown(e: &Entry) -> String {
    if e.is_tool || e.group.is_empty() { tl!(&e.label).to_string() } else { format!("{} › {}", tl!(&e.group), tl!(&e.label)) }
}

/// What a row's target shows.
fn target_label(target: Option<&str>) -> String {
    match target {
        None => tl!("Choose…").to_string(),
        Some("") => tl!("No equivalent").to_string(),
        Some(t) => entry(t).map_or_else(|| t.to_string(), shown),
    }
}

/// Modifiers as written (`Cmd+Shift`, "" for none) as the menus show them.
fn pretty_mods(mods: &str) -> String {
    if mods.is_empty() {
        return tl!("None").to_string();
    }
    let shown = menus::pretty_shortcut(&format!("{mods}+K"));
    shown.strip_suffix("+K").or_else(|| shown.strip_suffix('K')).unwrap_or(&shown).to_string()
}

/// Where a code's answer came from, as the Keys section says it.
fn source(c: &Code) -> String {
    match c.from.as_str() {
        "saved" => tl!("from your earlier imports").to_string(),
        "described" => tl!("from the format's public description").to_string(),
        "agreement" => crate::i18n::tn(c.votes as u64, "agrees with {n} default shortcut", "agrees with {n} default shortcuts"),
        "you" => String::new(),
        _ => tl!("not known yet").to_string(),
    }
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
/// `{entries, unread}`. An open Keyboard Shortcuts dialog is kept to go back to.
pub fn open(app: &mut VectorcraftApp, name: &str, bytes: &[u8]) -> Result<Value, String> {
    // Only the part `shortcutSet.read` takes in.
    let head = bytes.get(..vectorcraft_engine::cmd::shortcutset::READ_LIMIT).unwrap_or(bytes);
    open_read(app, json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(head) }))
}

/// Open the review of the set `shortcutSet.read` reads with `params`.
fn open_read(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    let read = app.run("shortcutSet.read", params)?;
    let review = Review::build(&read, &candidates());
    let back = app.ui.dialog.take().filter(|d| d.kind == "shortcuts");
    let mut d = Dialog::new(
        KIND,
        json!({
            "name": read["set"],
            "start": "defaults",
            "remember": true,
            "notes": read["unread"],
            "truncated": read["truncated"],
            super::PARENT: back.and_then(|b| serde_json::to_value(b).ok()),
            "__recording": "",
            "__query": "",
            "__attention": false,
            "__keysOpen": review.asks(),
        }),
    );
    review.store(&mut d);
    app.ui.dialog = Some(d);
    Ok(json!({ "entries": review.rows.len(), "unread": read["unread"].as_array().map_or(0, Vec::len) }))
}

/// `ui.shortcutImportDialog`: the review of the file at `path`, or in `dataBase64` (called `name`),
/// else of a picked one. On the web the browser's file picker opens, and File › Open opens the
/// review of a `.kys`.
pub fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    if let Some(b64) = s("dataBase64") {
        return open_read(app, json!({ "name": s("name").unwrap_or_default(), "dataBase64": b64 }));
    }
    let path = match s("path") {
        Some(path) => path,
        None if app.services.open_async.is_some() => return crate::io::open_dialog(app).map(|_| Value::Null),
        None => {
            let pick =
                FilePick { filters: vec![FilePick::filter("Shortcut sets", vectorcraft_engine::cmd::shortcutset::EXTS)], ..Default::default() };
            crate::picks::open(app, &pick).ok_or("cancelled")?
        }
    };
    open_read(app, json!({ "path": path }))
}

/// Apply: the ready rows applied as one set ([`apply_ready`]) over VectorCraft's defaults (or the current
/// shortcuts), and Keyboard Shortcuts shown with the result as its working set; its OK keeps it. With
/// Remember My Choices on, the matches and codes used are kept for later imports (`importMatches.set`).
/// → `{applied, rows}`
pub fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let review = Review::of(d);
    let statuses = review.statuses();
    let back = super::parent(d);
    let current = d.str("start") == "current";
    let ov = match (current, &back) {
        (true, Some(b)) => b.fields.get("overrides").map(|v| crate::keymap::stored::from_value(v.clone())).unwrap_or_default(),
        (true, None) => app.ui.shortcut_overrides.clone(),
        (false, _) => craft_keymap::Overrides::new(),
    };
    let mut km = crate::keymap::keymap_with(&ov);
    let applied = apply_ready(&mut km, &review, &statuses, if current { Start::Current } else { Start::Defaults });
    let remembered = if d.bool("remember") { remember(app, &review, &statuses) } else { Ok(()) };
    let name = d.str("name");
    let set = Some(name.trim()).filter(|n| !n.is_empty()).unwrap_or(shortcut_editor::CUSTOM).to_string();
    app.ui.dialog = back;
    if app.ui.dialog.as_ref().is_none_or(|b| b.kind != "shortcuts") {
        shortcut_editor::open(app);
    }
    if let Some(k) = app.ui.dialog.as_mut() {
        k.fields.insert("overrides".into(), crate::keymap::stored::to_value(km.overrides()));
        k.fields.insert("set".into(), json!(set));
        k.fields.insert("__message".into(), json!(tl!("Imported. Press OK to keep the imported set.")));
    }
    if let Err(e) = remembered {
        app.status(e);
    }
    Ok(json!({ "applied": applied, "rows": review.rows.len() }))
}

/// Text cut to what [`craft_keymap::plan`] reads, so an entry's [`ForeignEntry::remember_key`] is
/// the one `plan` looks up.
fn within_cap(s: &str) -> String {
    s.chars().take(MAX_IMPORT_TEXT).collect()
}

/// The ready rows applied to `km` as one set through [`craft_keymap::plan`]. Each row's target is
/// passed as the user's answer for its entry, so the review's matching decides; a binding the plan
/// gives any other command is left out. → the number of rows applied.
fn apply_ready(km: &mut Keymap, review: &Review, statuses: &[(Status, Option<String>)], start: Start) -> usize {
    let mut entries: Vec<ForeignEntry> = vec![];
    // Per entry: its row's line and target.
    let mut rows: Vec<(usize, &str)> = vec![];
    let mut remembered: BTreeMap<String, CommandRef> = BTreeMap::new();
    for (r, (status, chord)) in review.rows.iter().zip(statuses) {
        let (Status::Ready, Some(target), Some(text)) = (*status, r.target.as_deref(), chord.as_deref()) else { continue };
        let Some(chord) = crate::keymap::vc_chord(text) else { continue };
        // Room for the name and a `#line` segment within the segments `plan` reads.
        let mut path: Vec<String> =
            r.section.split(" › ").filter(|s| !s.is_empty()).take(MAX_PATH_SEGMENTS.saturating_sub(2)).map(within_cap).collect();
        path.push(within_cap(&r.name));
        let key = ForeignKey { context: None, text: text.to_string(), trigger: Ok(Trigger::single(chord)), notes: vec![] };
        let mut entry = ForeignEntry::new(None, path.clone()).with_key(key).at_line(r.line);
        if target.starts_with("tool:") {
            entry = entry.tool();
        }
        let command = crate::keymap::command_of(target);
        // Rows with one name in one section can stand for different items: each keeps its own answer.
        let mut n = 0usize;
        while remembered.get(&entry.remember_key()).is_some_and(|c| *c != command) {
            n += 1;
            entry.path = path.clone();
            entry.path.push(if n == 1 { format!("#{}", r.line) } else { format!("#{}-{n}", r.line) });
        }
        remembered.insert(entry.remember_key(), command);
        entries.push(entry);
        rows.push((r.line, target));
    }
    let options = ImportOptions { remembered, start, none_defined: NoneDefined::Keep, suggest: false, ..ImportOptions::default() };
    let mut planned = plan(&ForeignSet::new(entries), km, &options);
    for p in &mut planned.commands {
        let key = crate::keymap::entry_key(&p.command);
        p.bindings.retain(|b| rows.get(b.order).is_some_and(|(_, t)| *t == key));
    }
    let report = planned.apply(km);
    let planned_rows: HashSet<usize> = planned.commands.iter().flat_map(|p| p.bindings.iter().map(|b| b.order)).collect();
    let left_out = |line: usize, target: &str| {
        report.skipped.iter().any(|s| s.line == Some(line) && crate::keymap::entry_key(&s.command) == target)
            || report.failed.iter().any(|f| crate::keymap::entry_key(&f.command) == target)
    };
    rows.iter().enumerate().filter(|(i, (line, target))| planned_rows.contains(i) && !left_out(*line, target)).count()
}

/// Keep for later imports the targets and codes the user chose, and those of the rows Apply assigns.
fn remember(app: &mut VectorcraftApp, review: &Review, statuses: &[(Status, Option<String>)]) -> Result<(), String> {
    let mut matches = vec![];
    let (mut used_keys, mut used_mods) = (BTreeSet::new(), BTreeSet::new());
    for (r, (status, _)) in review.rows.iter().zip(statuses) {
        let ready = *status == Status::Ready;
        if let Some(t) = &r.target
            && r.from != "saved"
            && (ready || r.from == "you")
        {
            let mut m = json!({ "kind": "name", "name": r.name, "target": t });
            if let Some(e) = entry(t) {
                m["label"] = json!(e.label);
            }
            matches.push(m);
        }
        if ready {
            used_keys.extend(r.key);
            used_mods.extend(r.modifiers);
        }
    }
    for (kind, codes, used) in [("key", &review.keys, &used_keys), ("modifiers", &review.mods, &used_mods)] {
        for (code, c) in codes {
            if let Some(text) = &c.text
                && (c.from == "you" || (c.from == "agreement" && used.contains(code)))
            {
                matches.push(json!({ "kind": kind, "name": code.to_string(), "target": text }));
            }
        }
    }
    if !matches.is_empty() {
        app.run("importMatches.set", json!({ "matches": matches }))?;
    }
    Ok(())
}

/// The rows Apply leaves out and the parts of the file not read, a line each, after the set's name.
pub fn report(d: &Dialog, review: &Review, statuses: &[(Status, Option<String>)]) -> String {
    let line = |n: usize, name: &str, status: &str| {
        crate::i18n::fmt(tl!("Line {line}: {name}: {status}"), &[("line", &n.to_string()), ("name", name), ("status", status)])
    };
    let mut lines = vec![d.str("name")];
    for (r, (status, _)) in review.rows.iter().zip(statuses) {
        if *status != Status::Ready {
            lines.push(line(r.line, &r.name, status.label()));
        }
    }
    for n in d.fields.get("notes").and_then(Value::as_array).into_iter().flatten() {
        lines.push(line(n["line"].as_u64().unwrap_or(0) as usize, n["text"].as_str().unwrap_or_default(), tl!("Not read")));
    }
    lines.join("\n") + "\n"
}

// ---------- dialog ----------

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    let mut review = Review::of(&d);
    let before = review.clone();
    let t = Tokens::get(ctx);
    // A key press answers the key code being recorded, before the widgets see it; Escape stops.
    if let Ok(code) = d.str("__recording").parse::<i64>() {
        let ev = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, .. } if !crate::shortcuts::is_modifier(*key) => Some(*key),
                _ => None,
            })
        });
        if let Some(key) = ev {
            ctx.input_mut(|i| i.events.clear());
            d.fields.insert("__recording".into(), json!(""));
            if key != egui::Key::Escape
                && let Some(text) = shortcut_editor::chord_from_event(key, egui::Modifiers::NONE)
                && let Some(c) = review.keys.get_mut(&code)
            {
                c.text = Some(text);
                c.from = "you".into();
            }
        }
    }
    let statuses = review.statuses();
    // The Keys and Shortcuts lists get the window's height less the rest of the dialog, as the frame
    // before drew it.
    let id = egui::Id::unique("dialog-shortcut-import");
    let rest = ctx.data(|m| m.get_temp::<f32>(id.with("rest"))).unwrap_or(0.0);
    let (keys, mods) = review.asked();
    let keys_open = d.bool("__keysOpen") && (!keys.is_empty() || !mods.is_empty());
    let (keys_h, rows_h) = list_heights(ctx.content_rect().height() - rest, keys_open);
    let (mut apply, mut cancel, mut copy, mut lists) = (false, false, false, 0.0);
    crate::dialogs::modal::show(ctx, tl!("Import Shortcut Set"), id, -20.0, 20, |ui| {
        ui.set_width(NAME_W + TARGET_W + CHORD_W + 220.0);
        crate::dialogs::modal::heading(ui, tl!("Import Shortcut Set"));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Name:")).color(t.text_dim));
            let mut name = d.str("name");
            if ui.add(egui::TextEdit::singleline(&mut name).desired_width(260.0)).changed() {
                d.fields.insert("name".into(), json!(name));
            }
        });
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(tl!(
                "Each entry of the file is matched to a VectorCraft tool, command or panel. Check the matches and answer what is left."
            ))
            .color(t.text_dim)
            .size(11.5),
        );
        lists = keys_section(ui, &mut d, &mut review, keys_h) + rows_section(ui, &mut d, &mut review, &statuses, rows_h);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let current = d.str("start") == "current";
            if widgets::radio(ui, tl!("Start from VectorCraft Defaults"), !current, true) {
                d.fields.insert("start".into(), json!("defaults"));
            }
            ui.add_space(12.0);
            if widgets::radio(ui, tl!("Add to the Current Shortcuts"), current, true) {
                d.fields.insert("start".into(), json!("current"));
            }
        });
        let remember = d.bool("remember");
        if widgets::check(ui, tl!("Remember My Choices"), remember, true) {
            d.fields.insert("remember".into(), json!(!remember));
        }
        ui.add_space(4.0);
        let ready = statuses.iter().filter(|(s, _)| *s == Status::Ready).count();
        let mut summary =
            crate::i18n::fmt(tl!("{ready} of {total} ready"), &[("ready", &ready.to_string()), ("total", &review.rows.len().to_string())]);
        let notes = d.fields.get("notes").and_then(Value::as_array).map_or(0, Vec::len);
        if notes > 0 {
            summary.push_str(" · ");
            summary.push_str(&crate::i18n::tn(notes as u64, "{n} part of the file wasn't read", "{n} parts of the file weren't read"));
        }
        if d.bool("truncated") {
            summary.push_str(" · ");
            summary.push_str(tl!("The file is too large to read in full, so only its first part is listed."));
        }
        ui.label(egui::RichText::new(summary).color(t.text_dim).size(11.5));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if widgets::secondary_button(ui, tl!("Copy Report")).clicked() {
                copy = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::primary_button(ui, tl!("Apply")).clicked() {
                    apply = true;
                }
                ui.add_space(8.0);
                if widgets::secondary_button(ui, tl!("Cancel")).clicked() {
                    cancel = true;
                }
            });
        });
    });
    if let Some(window) = ctx.memory(|m| m.area_rect(id)) {
        ctx.data_mut(|m| m.insert_temp(id.with("rest"), window.height() - lists));
    }
    if review != before {
        if review.rows.iter().map(|r| &r.target).ne(before.rows.iter().map(|r| &r.target)) {
            review.decode();
        }
        review.store(&mut d);
    }
    if copy {
        app.clipboard_out = Some(report(&d, &review, &review.statuses()));
        app.status(tl!("Report copied"));
    }
    app.ui.dialog = Some(d);
    if cancel {
        crate::dialogs::cancel(app);
    } else if apply && let Err(e) = crate::dialogs::confirm(app) {
        app.status(e);
    }
}

/// The heights of the Keys list (when `keys_open`) and the Shortcuts list when `room` points of the
/// window are left for them: their tallest, or both shorter by the same share, down to a tenth.
fn list_heights(room: f32, keys_open: bool) -> (f32, f32) {
    let tallest = if keys_open { KEYS_LIST_H + ROWS_LIST_H } else { ROWS_LIST_H };
    let share = (room / tallest).clamp(0.1, 1.0);
    (KEYS_LIST_H * share, ROWS_LIST_H * share)
}

/// The Keys section: the codes to answer or check, each with a field for its key or modifiers, where
/// its answer came from and the first entries that use it, in a list at most `height` points tall.
/// Not shown when there are none. → the list's height (0 while the section is closed).
fn keys_section(ui: &mut egui::Ui, d: &mut Dialog, review: &mut Review, height: f32) -> f32 {
    let (keys, mods) = review.asked();
    if keys.is_empty() && mods.is_empty() {
        return 0.0;
    }
    let t = Tokens::get(ui.ctx());
    ui.add_space(8.0);
    let open = d.bool("__keysOpen");
    if widgets::section_toggle(ui, tl!("Keys"), open) {
        d.fields.insert("__keysOpen".into(), json!(!open));
    }
    if !open {
        return 0.0;
    }
    ui.label(
        egui::RichText::new(tl!(
            "The file writes keys and modifiers as codes. Check the proposed meanings and answer the codes that are not known yet."
        ))
        .color(t.text_dim)
        .size(11.5),
    );
    let recording = d.str("__recording");
    let list = egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
        scrolling_list("shortcut-import-keys", height).auto_shrink([false, true]).show(ui, |ui| {
            for code in keys {
                let Some(c) = review.keys.get_mut(&code) else { continue };
                let waiting = recording == code.to_string();
                ui.horizontal(|ui| {
                    cell(ui, 110.0, |ui| ui.label(egui::RichText::new(format!("/Key {code}")).color(t.text_dim)));
                    let label = match (waiting, c.text.as_deref()) {
                        (true, _) => tl!("Press a key…").to_string(),
                        (false, None) => "?".to_string(),
                        (false, Some("")) => tl!("No equivalent").to_string(),
                        (false, Some(k)) => menus::pretty_shortcut(k),
                    };
                    let b = ui
                        .add_sized([120.0, ROW_H], egui::Button::new(label).selected(waiting))
                        .on_hover_text(tl!("Click, then press the key this code stands for (Esc cancels)"));
                    if b.clicked() {
                        d.fields.insert("__recording".into(), json!(code.to_string()));
                    }
                    if widgets::icon_button(ui, "x", tl!("No equivalent"), c.text.as_deref() == Some(""), 18.0).clicked() {
                        c.text = Some(String::new());
                        c.from = "you".into();
                    }
                    code_notes(ui, c);
                });
            }
            for code in mods {
                let Some(c) = review.mods.get_mut(&code) else { continue };
                ui.horizontal(|ui| {
                    cell(ui, 110.0, |ui| ui.label(egui::RichText::new(format!("/Modifiers {code}")).color(t.text_dim)));
                    let current = c.text.as_deref().map_or_else(|| "?".to_string(), pretty_mods);
                    let options: Vec<String> = MODIFIERS.iter().map(|m| pretty_mods(m)).collect();
                    let names: Vec<&str> = options.iter().map(String::as_str).collect();
                    if let Some(m) =
                        widgets::dropdown_names(ui, ("shortcut-import-mods", code), &current, &names, 144.0).and_then(|i| MODIFIERS.get(i))
                    {
                        c.text = Some(m.to_string());
                        c.from = "you".into();
                    }
                    code_notes(ui, c);
                });
            }
        })
    });
    list.inner.inner_rect.height()
}

/// A vertical list that scrolls when its contents are taller than `height` points, and is then
/// `height` points tall, also under egui's 64-point minimum.
fn scrolling_list(id: &str, height: f32) -> egui::ScrollArea {
    egui::ScrollArea::vertical().id_salt(id).max_height(height).min_scrolled_height(height)
}

/// A cell `w` points wide with its contents on the left.
fn cell<R>(ui: &mut egui::Ui, w: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(egui::vec2(w, ROW_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(w);
        add(ui)
    })
    .inner
}

/// Where code `c`'s answer came from, and the first entries that use it.
fn code_notes(ui: &mut egui::Ui, c: &Code) {
    let t = Tokens::get(ui.ctx());
    let from = source(c);
    if !from.is_empty() {
        ui.label(egui::RichText::new(from).color(t.text_dim).size(11.0));
    }
    let names = c.names.join(", ");
    ui.add(egui::Label::new(egui::RichText::new(names).color(t.text_dim).size(11.0).italics()).truncate());
}

/// The rows: a check box, the name as the file writes it, what it stands for in VectorCraft (a
/// searchable picker), the shortcut and the status, in a list `height` points tall. → the list's
/// height.
fn rows_section(ui: &mut egui::Ui, d: &mut Dialog, review: &mut Review, statuses: &[(Status, Option<String>)], height: f32) -> f32 {
    let t = Tokens::get(ui.ctx());
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        widgets::section_header(ui, tl!("Shortcuts"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let attention = d.bool("__attention");
            if widgets::check(ui, tl!("Needs Attention Only"), attention, true) {
                d.fields.insert("__attention".into(), json!(!attention));
            }
            ui.add_space(8.0);
            let mut q = d.str("__query");
            if ui.add(egui::TextEdit::singleline(&mut q).hint_text(tl!("Search")).desired_width(180.0)).changed() {
                d.fields.insert("__query".into(), json!(q));
            }
        });
    });
    let query = d.str("__query").to_lowercase();
    let attention = d.bool("__attention");
    let visible: Vec<usize> = review
        .rows
        .iter()
        .zip(statuses)
        .enumerate()
        .filter(|(_, (r, (status, _)))| {
            (!attention || status.needs_attention())
                && (query.is_empty() || r.name.to_lowercase().contains(&query) || target_label(r.target.as_deref()).to_lowercase().contains(&query))
        })
        .map(|(i, _)| i)
        .collect();
    let list = egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            for (label, w) in [(tl!("In the File"), NAME_W), (tl!("In VectorCraft"), TARGET_W), (tl!("Shortcut"), CHORD_W)] {
                cell(ui, w, |ui| ui.label(egui::RichText::new(label).color(t.text_dim).size(11.0)));
            }
            ui.label(egui::RichText::new(tl!("Status")).color(t.text_dim).size(11.0));
        });
        scrolling_list("shortcut-import-rows", height).auto_shrink([false, false]).show_rows(ui, ROW_H, visible.len(), |ui, range| {
            for i in range.filter_map(|v| visible.get(v).copied()) {
                let (status, chord) = statuses.get(i).cloned().unwrap_or((Status::NeedsChoice, None));
                let Some(r) = review.rows.get_mut(i) else { continue };
                ui.horizontal(|ui| {
                    if widgets::check_box(ui, &r.name, r.on, true) {
                        r.on = !r.on;
                    }
                    let place = crate::i18n::fmt(tl!("Line {line}"), &[("line", &r.line.to_string())]);
                    let place = if r.section.is_empty() { place } else { format!("{} · {place}", r.section) };
                    cell(ui, NAME_W, |ui| ui.add(egui::Label::new(egui::RichText::new(&r.name).color(t.text)).truncate()).on_hover_text(place));
                    if let Some(target) = target_picker(ui, i, r) {
                        r.target = Some(target);
                        r.from = "you".into();
                    }
                    let shortcut = chord.as_deref().map_or_else(|| "—".to_string(), menus::pretty_shortcut);
                    cell(ui, CHORD_W, |ui| ui.label(egui::RichText::new(shortcut).color(t.text)));
                    let color = match status {
                        Status::Ready => t.text,
                        Status::Off | Status::NoEquivalent => t.text_dim,
                        _ => t.warning,
                    };
                    ui.add(egui::Label::new(egui::RichText::new(status.label()).color(color).size(11.5)).truncate());
                });
            }
        })
    });
    list.inner.inner_rect.height()
}

/// Row `r`'s target picker: "No equivalent" and the row's choices, then Keyboard Shortcuts' entries
/// that match the search. → the target picked ("" for no equivalent).
fn target_picker(ui: &mut egui::Ui, i: usize, r: &Row) -> Option<String> {
    let current = target_label(r.target.as_deref());
    widgets::combo(ui, ("shortcut-import-target", i), &current, TARGET_W, true, |ui| {
        let field = ui.scope_id().with("query");
        if ui.memory(|m| m.focused().is_none()) {
            ui.memory_mut(|m| m.request_focus(field.with("edit")));
        }
        let q = widgets::search_field(ui, field, tl!("Search")).to_lowercase();
        let mut picked = None;
        let mut option = |ui: &mut egui::Ui, key: &str, label: &str| {
            if ui.add(egui::Button::selectable(r.target.as_deref() == Some(key), label)).clicked() {
                picked = Some(key.to_string());
            }
        };
        option(ui, "", tl!("No equivalent"));
        let list: Vec<&Entry> = if q.is_empty() {
            for e in r.choices.iter().filter_map(|c| entry(c)) {
                option(ui, &e.key, &shown(e));
            }
            ui.separator();
            shortcut_editor::entries().iter().collect()
        } else {
            shortcut_editor::entries().iter().filter(|e| shown(e).to_lowercase().contains(&q) || e.key.to_lowercase().contains(&q)).collect()
        };
        egui::ScrollArea::vertical().max_height(300.0).show_rows(ui, ROW_H, list.len(), |ui, range| {
            for e in range.filter_map(|k| list.get(k)) {
                option(ui, &e.key, &shown(e));
            }
        });
        if picked.is_some() {
            ui.close();
        }
        picked
    })
}
