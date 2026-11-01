//! VectorCraft's keymap on [`craft_keymap`]: the catalogue of everything a shortcut can run, the
//! bridge between VectorCraft's chord spellings and the crate's, and the stored form of the
//! user's overrides.
//!
//! Entry keys are VectorCraft's everywhere outside this module: a command id, `tool:<id>`, or
//! `panel:<id>` for the Window menu item showing a panel (`window.panel {panel}`). Chord texts
//! outside it are VectorCraft's too (`Cmd+Shift+]`, `Comma`, `Period`, `~` for the backtick key,
//! `=` for Plus with modifiers). Inside an [`Overrides`] keys and texts are the crate's.

use std::sync::{OnceLock, RwLock};

use craft_keymap::{Bindable, Binding, Chord, CommandRef, Key, Keymap, Kind, Lookup, Mods, Overrides, Platform, Scope, Scopes};
use serde_json::{Value, json};

use crate::shortcut_editor::{self, Entry};

/// Menu commands whose label flips between Show and Hide (see `menus::dynamic_label`).
pub const TOGGLES: &[&str] = &[
    "view.edges",
    "view.template",
    "view.cornerWidget",
    "view.textThreads",
    "type.hiddenCharacters",
    "view.gradientAnnotator",
    "view.artboards",
    "view.rulers",
    "view.boundingBox",
    "view.transparencyGrid",
    "view.guides",
    "view.grid",
    "view.slices.hide",
    "view.printTiling",
    "perspective.grid.show",
    "perspective.grid.rulers",
];

// ---------- entry keys ----------

/// The command an entry key runs.
pub fn command_of(key: &str) -> CommandRef {
    match key.strip_prefix("panel:") {
        Some(panel) => CommandRef::with_params("window.panel", json!({ "panel": panel })),
        None => CommandRef::new(key),
    }
}

/// The entry key of a command: `panel:<id>` for `window.panel {panel}`.
pub fn entry_key(command: &CommandRef) -> String {
    match (command.id.as_str(), command.params.get("panel").and_then(Value::as_str)) {
        ("window.panel", Some(panel)) => format!("panel:{panel}"),
        _ => command.key(),
    }
}

/// The [`Overrides`] key of an entry key.
pub fn stored_key(key: &str) -> String {
    if key.starts_with("panel:") { command_of(key).key() } else { key.to_string() }
}

/// The entry key of an [`Overrides`] key.
pub fn entry_of_stored(key: &str) -> String {
    CommandRef::parse_key(key).filter(|c| c.id == "window.panel").map_or_else(|| key.to_string(), |c| entry_key(&c))
}

// ---------- chord texts ----------

/// Reads VectorCraft chord text. `+` and `=` are one key: Plus alone, Equals with modifiers.
pub fn vc_chord(text: &str) -> Option<Chord> {
    let sc = crate::shortcuts::parse(text.trim())?;
    let key = Key::from_egui(sc.logical_key)?;
    let m = sc.modifiers;
    let mods = Mods { cmd: m.command || m.mac_cmd, ctrl: m.ctrl, alt: m.alt, shift: m.shift };
    let key = match key {
        Key::Plus | Key::Equals if mods.is_none() => Key::Plus,
        Key::Plus | Key::Equals => Key::Equals,
        k => k,
    };
    Some(Chord::new(mods, key))
}

/// VectorCraft's text of a chord, as `shortcut_editor::format` writes it.
pub fn vc_format(chord: &Chord) -> String {
    let m = chord.mods;
    let mods = egui::Modifiers { alt: m.alt, ctrl: m.ctrl, shift: m.shift, mac_cmd: false, command: m.cmd };
    shortcut_editor::format(&egui::KeyboardShortcut::new(mods, chord.key.to_egui()))
}

/// VectorCraft's text of a binding; a scoped or double binding (hand-written) keeps the crate's.
pub fn binding_text(b: &Binding) -> String {
    if b.scope.is_none() && !b.trigger.double { vc_format(&b.trigger.chord) } else { b.to_string() }
}

/// VectorCraft text → the crate's. Text both read as one chord stays as written (`Comma`,
/// `Ctrl+Cmd+H`), so it is saved back as the user wrote it; text neither reads is kept for
/// `Keymap::problems`.
pub fn vc_to_crate(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    match vc_chord(text) {
        Some(c) if Binding::parse(text).is_ok_and(|b| b == Binding::new(c)) => text.to_string(),
        Some(c) => c.to_string(),
        None => text.to_string(),
    }
}

/// The crate's text → VectorCraft's. The crate's canonical text is rewritten; other text is
/// what [`vc_to_crate`] kept as written, or text that does not parse.
pub fn crate_to_vc(text: &str) -> String {
    match Binding::parse(text) {
        Ok(b) if b.to_string() == text => binding_text(&b),
        _ => text.to_string(),
    }
}

/// A default shortcut of the catalogue.
fn vc_binding(text: &str) -> Option<Binding> {
    vc_chord(text).map(Binding::new)
}

// ---------- catalogue ----------

/// Every listed entry as the crate sees it, in dispatch priority: commands (engine, then UI),
/// panels, tools. Each tool is its own group: tools never cycle.
fn catalogue() -> Vec<Bindable> {
    let rank = |e: &Entry| {
        if e.is_tool {
            2
        } else if e.key.starts_with("panel:") {
            1
        } else {
            0
        }
    };
    let mut entries: Vec<&Entry> = shortcut_editor::entries().iter().collect();
    entries.sort_by_key(|e| rank(e));
    entries
        .into_iter()
        .map(|e| {
            let mut path: Vec<String> = e.group.split(" › ").filter(|s| !s.is_empty()).map(str::to_string).collect();
            path.push(e.label.clone());
            let mut b = Bindable::new(command_of(&e.key), e.label.clone(), Scope::app()).with_path(path);
            if let Some(tool) = e.key.strip_prefix("tool:") {
                b = b.with_kind(Kind::Tool { group: Some(tool.to_string()) });
            }
            if TOGGLES.contains(&e.key.as_str()) {
                b = b.toggle();
            }
            if let Some(d) = shortcut_editor::default_of(&e.key).and_then(vc_binding) {
                b = b.with_default(d);
            }
            for alias in default_aliases(&e.key) {
                if let Some(d) = vc_binding(alias) {
                    b = b.with_default(d);
                }
            }
            b
        })
        .collect()
}

/// Additional defaults for commands whose primary shortcut remains in their command spec.
fn default_aliases(key: &str) -> &'static [&'static str] {
    match key {
        "paint.lastColor" => &["Shift+,"],
        "paint.lastGradient" => &["Shift+."],
        _ => &[],
    }
}

/// On the web, the browser's operating system once egui reports it ([`note_os`]).
static WEB_PLATFORM: RwLock<Option<Platform>> = RwLock::new(None);

/// The platform shortcuts follow: the build's, and on the web the browser's operating system, so
/// Cmd shortcuts work in a browser on a Mac and Windows' reserved chords stay free on Windows.
pub fn platform() -> Platform {
    let web = if cfg!(target_arch = "wasm32") { WEB_PLATFORM.read().ok().and_then(|p| *p) } else { None };
    web.unwrap_or_else(Platform::current)
}

/// Records the operating system egui reports, which the web build reads from the browser.
pub fn note_os(ctx: &egui::Context) {
    if cfg!(target_arch = "wasm32") {
        let p = craft_keymap::egui::platform(ctx);
        if let Ok(mut web) = WEB_PLATFORM.write()
            && *web != Some(p)
        {
            *web = Some(p);
        }
    }
}

/// The value for `p` in `cells`, made on first use. Each platform has its own cell, because the
/// web build learns the browser's platform only after the first frame.
pub(crate) fn per_platform<T>(cells: &'static [OnceLock<T>; 3], p: Platform, make: impl FnOnce() -> T) -> &'static T {
    let [mac, windows, other] = cells;
    let cell = match p {
        Platform::Mac => mac,
        Platform::Windows => windows,
        _ => other,
    };
    cell.get_or_init(make)
}

/// The keymap with the default shortcuts for [`platform`], built once per platform.
pub fn template() -> &'static Keymap {
    static TEMPLATES: [OnceLock<Keymap>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let p = platform();
    per_platform(&TEMPLATES, p, || keymap_for(p))
}

/// The default keymap of `p`. The chords the operating system takes can't be assigned.
fn keymap_for(p: Platform) -> Keymap {
    Keymap::new(catalogue(), Scopes::new(), p).with_reserved(craft_keymap::reserved::system(p))
}

/// The keymap with `overrides` applied.
pub fn keymap_with(overrides: &Overrides) -> Keymap {
    template().clone().with_overrides(overrides.clone())
}

/// The command a chord runs, the first for tools sharing it.
pub fn lookup(km: &Keymap, chord: &Chord) -> Option<CommandRef> {
    match km.lookup(chord, false, &craft_keymap::Active::NONE) {
        Lookup::None => None,
        Lookup::Run(c) => Some(c),
        Lookup::Cycle(v) => v.into_iter().next(),
        _ => None,
    }
}

/// Effective shortcut text of an entry key under `km`: the default's own text while the entry
/// has its defaults. An entry the catalogue lacks reads its stored text.
pub fn effective_with(km: &Keymap, key: &str) -> Option<String> {
    let command = command_of(key);
    if km.bindable(&command).is_none() {
        return match km.overrides().get(&stored_key(key)) {
            Some(texts) => texts.first().filter(|t| !t.trim().is_empty()).map(|t| crate_to_vc(t)),
            None => shortcut_editor::default_of(key).map(str::to_string),
        };
    }
    let now = km.bindings(&command);
    if now == km.default_bindings(&command) {
        return shortcut_editor::default_of(key).map(str::to_string);
    }
    let first = now.first()?;
    // The text as stored, when it is this binding's: the user's spelling.
    let stored = km.overrides().get(&stored_key(key)).and_then(|t| t.first()).filter(|t| Binding::parse(t).is_ok_and(|b| b == *first));
    Some(stored.map_or_else(|| binding_text(first), |t| crate_to_vc(t)))
}

// ---------- overrides ----------

/// Sets an entry's shortcut in VectorCraft's terms: `""` for none.
pub fn set_override(overrides: &mut Overrides, key: &str, text: &str) {
    let texts = if text.trim().is_empty() { vec![] } else { vec![vc_to_crate(text)] };
    // Only an over-long key or text is refused; such an entry could not be stored anyway.
    let _ = overrides.insert(stored_key(key), texts);
}

/// Overrides holding `(entry key, shortcut)` pairs in VectorCraft's terms.
pub fn overrides_from(pairs: &[(&str, &str)]) -> Overrides {
    let mut out = Overrides::new();
    for (k, t) in pairs {
        set_override(&mut out, k, t);
    }
    out
}

/// An entry's stored shortcut in VectorCraft's terms (`""` for none), when it has an override.
pub fn override_text(overrides: &Overrides, key: &str) -> Option<String> {
    overrides.get(&stored_key(key)).map(|t| t.first().map(|t| crate_to_vc(t)).unwrap_or_default())
}

pub fn has_override(overrides: &Overrides, key: &str) -> bool {
    overrides.get(&stored_key(key)).is_some()
}

/// Serde for [`Overrides`] in VectorCraft's own form, which every earlier version reads: a map
/// of entry key → one shortcut text (`""` for none), with `panel:` keys and VectorCraft's
/// spellings. Loading never fails: what cannot be used is dropped or kept for
/// `Keymap::problems`, and only an entry's first text is kept.
pub mod stored {
    use craft_keymap::Overrides;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::{Map, Value};

    use super::{crate_to_vc, entry_of_stored, stored_key, vc_to_crate};

    pub fn serialize<S: Serializer>(overrides: &Overrides, s: S) -> Result<S::Ok, S::Error> {
        s.collect_map(overrides.iter().map(|(k, texts)| (entry_of_stored(k), texts.first().map(|t| crate_to_vc(t)).unwrap_or_default())))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Overrides, D::Error> {
        Ok(from_value(Value::deserialize(d)?))
    }

    pub fn to_value(overrides: &Overrides) -> Value {
        struct Stored<'a>(&'a Overrides);
        impl Serialize for Stored<'_> {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                serialize(self.0, s)
            }
        }
        serde_json::to_value(Stored(overrides)).unwrap_or_else(|_| Value::Object(Map::new()))
    }

    pub fn from_value(v: Value) -> Overrides {
        let entries: Vec<(String, Value)> = match v {
            Value::Object(m) => m.into_iter().collect(),
            // The crate's list form, `[{command, keys}]`, as a newer version might write it.
            Value::Array(items) => items
                .into_iter()
                .filter_map(|item| {
                    let command = item.get("command").and_then(Value::as_str)?.to_string();
                    Some((command, item.get("keys").cloned().unwrap_or(Value::Array(vec![]))))
                })
                .collect(),
            other => return Overrides::deserialize(other).unwrap_or_default(),
        };
        let map: Map<String, Value> = entries.into_iter().map(|(k, v)| (stored_key(&k), first_text(v))).collect();
        Overrides::deserialize(Value::Object(map)).unwrap_or_default()
    }

    /// One text in the crate's spelling; values that are not text are left for the crate to
    /// report.
    fn first_text(v: Value) -> Value {
        match v {
            Value::String(s) => Value::String(vc_to_crate(&s)),
            Value::Array(items) => match items.into_iter().next() {
                None => Value::Array(vec![]),
                Some(Value::String(s)) => Value::String(vc_to_crate(&s)),
                Some(other) => Value::Array(vec![other]),
            },
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::UiState;
    use std::collections::BTreeMap;

    /// Earlier versions read the overrides as a map of one string per entry; a file this
    /// version writes must stay that.
    #[test]
    fn older_versions_read_the_saved_overrides() {
        let mut ui = UiState::default();
        set_override(&mut ui.shortcut_overrides, "tool:pen", "Shift+9");
        set_override(&mut ui.shortcut_overrides, "edit.preferences", "");
        set_override(&mut ui.shortcut_overrides, "panel:layers", "Cmd+Alt+Shift+F7");
        set_override(&mut ui.shortcut_overrides, "view.grid", "Cmd+Period");
        // An entry holding two texts (the crate keeps texts it cannot use after the binding).
        ui.shortcut_overrides.insert("tool:curvature", vec!["Shift+`".into(), "Hyper+K".into()]);
        let saved = serde_json::to_value(&ui).unwrap();
        let old: BTreeMap<String, String> = serde_json::from_value(saved["shortcut_overrides"].clone()).unwrap();
        let want: BTreeMap<String, String> = [
            ("tool:pen", "Shift+9"),
            ("edit.preferences", ""),
            ("panel:layers", "Cmd+Alt+Shift+F7"),
            ("view.grid", "Cmd+Period"),
            ("tool:curvature", "Shift+~"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(old, want);
        let back: UiState = serde_json::from_value(saved).unwrap();
        assert_eq!(override_text(&back.shortcut_overrides, "panel:layers").as_deref(), Some("Cmd+Alt+Shift+F7"));
        assert_eq!(override_text(&back.shortcut_overrides, "tool:curvature").as_deref(), Some("Shift+~"));
    }

    /// A hand-edited value that is not text drops that entry, not every preference.
    #[test]
    fn a_bad_override_value_keeps_the_other_preferences() {
        let mut saved = serde_json::to_value(UiState { toolbar: false, ..Default::default() }).unwrap();
        saved["shortcut_overrides"] = json!({"x": 5, "tool:pen": "Shift+9"});
        let ui: UiState = serde_json::from_value(saved).unwrap();
        assert!(!ui.toolbar);
        assert!(!has_override(&ui.shortcut_overrides, "x"));
        assert_eq!(override_text(&ui.shortcut_overrides, "tool:pen").as_deref(), Some("Shift+9"));
    }

    #[test]
    fn curvature_keeps_shift_tilde_and_plus_is_one_key() {
        let km = template();
        let curvature = vc_chord("Shift+~").unwrap();
        assert_eq!(curvature, Chord::parse("Shift+`").unwrap());
        assert_eq!(lookup(km, &curvature), Some(CommandRef::new("tool:curvature")));
        assert_eq!(effective_with(km, "tool:curvature").as_deref(), Some("Shift+~"));
        // Plus alone is the Add Anchor Point tool's, however it is written; with modifiers the
        // key is `=`.
        for t in ["+", "="] {
            assert_eq!(vc_chord(t), Some(Chord::plain(Key::Plus)), "{t}");
        }
        for t in ["Cmd++", "Cmd+="] {
            assert_eq!(vc_chord(t), Some(Chord::new(Mods::CMD, Key::Equals)), "{t}");
        }
        assert_eq!(lookup(km, &Chord::plain(Key::Plus)), Some(CommandRef::new("tool:addAnchor")));
        assert_eq!(lookup(km, &Chord::new(Mods::CMD, Key::Equals)), Some(CommandRef::new("view.zoomIn")));
        assert_eq!(vc_format(&Chord::new(Mods::CMD, Key::Comma)), "Cmd+Comma");
    }

    #[test]
    fn the_default_shortcuts_do_not_conflict() {
        assert_eq!(template().conflicts(), vec![]);
        assert_eq!(template().problems(), vec![]);
        assert_eq!(template().catalog().len(), shortcut_editor::entries().len(), "every entry is bindable");
        // A default the bridge can't read would show in the menus but never run.
        let unbound: Vec<(&str, &str)> = shortcut_editor::entries()
            .iter()
            .filter_map(|e| shortcut_editor::default_of(&e.key).map(|d| (e.key.as_str(), d)))
            .filter(|(key, d)| {
                let mut expected: Vec<Binding> = vc_binding(d).into_iter().collect();
                expected.extend(default_aliases(key).iter().filter_map(|alias| vc_binding(alias)));
                expected != template().default_bindings(&command_of(key))
            })
            .collect();
        assert_eq!(unbound, vec![], "every default text becomes the entry's binding");
    }

    #[test]
    fn paint_defaults_have_two_bindings_and_reset_restores_both() {
        for (key, plain, shifted) in [("paint.lastColor", ",", "Shift+,"), ("paint.lastGradient", ".", "Shift+.")] {
            let command = command_of(key);
            let defaults = template().default_bindings(&command);
            assert_eq!(defaults, vec![vc_binding(plain).unwrap(), vc_binding(shifted).unwrap()], "{key}");
            assert_eq!(
                shortcut_editor::entry(key).map(|e| e.label.as_str()),
                Some(if key == "paint.lastColor" { "Apply Last Color" } else { "Apply Last Gradient" })
            );
            assert_eq!(effective_with(template(), key).as_deref(), Some(plain));
            let mut overrides = Overrides::new();
            set_override(&mut overrides, key, "Shift+;");
            assert_eq!(keymap_with(&overrides).bindings(&command), vec![vc_binding("Shift+;").unwrap()]);
            shortcut_editor::reset_one(&mut overrides, key);
            assert_eq!(keymap_with(&overrides).bindings(&command), defaults);
        }
        assert!(template().conflicts().is_empty());
    }

    /// The chords the operating system takes are refused, and no default uses one.
    #[test]
    fn system_shortcuts_are_refused() {
        for (p, os_chord) in [(Platform::Mac, "Cmd+Tab"), (Platform::Windows, "Alt+F4"), (Platform::Linux, "Ctrl+Alt+F2")] {
            let mut km = keymap_for(p);
            assert_eq!((km.conflicts(), km.problems()), (vec![], vec![]), "{p:?}");
            let reserved: Vec<String> = shortcut_editor::entries()
                .iter()
                .filter(|e| km.bindings(&command_of(&e.key)).iter().any(|b| km.is_reserved(&b.trigger.chord)))
                .map(|e| e.key.clone())
                .collect();
            assert_eq!(reserved, Vec::<String>::new(), "{p:?}: defaults on reserved chords");
            let err = shortcut_editor::assign_in(&mut km, "edit.undo", Some(os_chord), true).unwrap_err();
            assert!(err.ends_with("is reserved by the system"), "{p:?}: {err}");
            // A saved override on a reserved chord keeps the default.
            let mut ov = Overrides::new();
            let _ = ov.insert(stored_key("edit.undo"), vec![os_chord.to_string()]);
            let km = keymap_for(p).with_overrides(ov);
            assert_eq!(km.bindings(&command_of("edit.undo")), km.default_bindings(&command_of("edit.undo")), "{p:?}");
        }
    }

    #[test]
    fn panel_entries_bridge_to_window_panel() {
        let c = command_of("panel:layers");
        assert_eq!((c.id.as_str(), c.params.clone()), ("window.panel", json!({"panel": "layers"})));
        assert_eq!(entry_key(&c), "panel:layers");
        assert_eq!(entry_of_stored(&stored_key("panel:layers")), "panel:layers");
        assert_eq!(stored_key("tool:pen"), "tool:pen");
        let paths = &template().bindable(&c).unwrap().paths;
        assert_eq!(paths, &vec![vec!["Window".to_string(), "Layers".to_string()]], "import matches panels by name");
    }

    /// Each toggle's menu label reads Show or Hide.
    #[test]
    fn toggles_show_and_hide() {
        let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        for id in TOGGLES {
            let label = crate::menus::dynamic_label(&app, id, "");
            assert!(label.starts_with("Show ") || label.starts_with("Hide "), "{id}: {label}");
            let listed = template().bindable(&CommandRef::new(*id));
            assert!(listed.is_none_or(|b| b.is_toggle), "{id}");
        }
    }
}
