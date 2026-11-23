//! VectorCraft's keymap on [`craft_keymap`]: the catalogue of everything a shortcut can run, the
//! bridge between VectorCraft's chord spellings and the crate's, and the loader of the user's
//! saved shortcut sets.
//!
//! Entry keys are VectorCraft's everywhere outside this module: a command id, `tool:<id>`, or
//! `panel:<id>` for the Window menu item showing a panel (`window.panel {panel}`), `effect:<id>`
//! for an Effect menu item, or `plugin:<id>` for a filter plug-in's Object › Plug-ins item
//! (`plugin.dialog {id}`). Chord texts read by [`stored`] are VectorCraft's (`Cmd+Shift+]`,
//! `Comma`, `Period`, `~` for the backtick key, `=` for Plus with modifiers). Inside an
//! [`Overrides`] keys and texts are the crate's.

use std::sync::RwLock;

use craft::keymap::{Binding, Chord, CommandDef, CommandKind, CommandRef, Key, Mods, Overrides, Platform, Scope};
use craft::ui::runtime::commands::{ChromeCommand, CommandSet as _};
use serde_json::{Value, json};

use crate::shortcut_editor::{self, EffectItem, Entry, Snapshot};

/// Menu commands whose label flips between Show and Hide (see `menus::dynamic_label`).
pub const TOGGLES: &[&str] = &[
    "view.edges",
    "view.template",
    "view.cornerWidget",
    "view.shapeWidgets",
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
    if key.starts_with("effect:") {
        return command_in(key, &shortcut_editor::effect_items());
    }
    command_in(key, &[])
}

/// The command an entry key runs while the Effect menu holds `effects`.
fn command_in(key: &str, effects: &[EffectItem]) -> CommandRef {
    if let Some(panel) = key.strip_prefix("panel:") {
        return CommandRef::with_params("window.panel", json!({ "panel": panel }));
    }
    // An Effect menu item (#3296): what its item runs, its dialog unless it has no options. An
    // effect the menu does not hold (a plug-in removed for now) is named by its dialog.
    if let Some(effect) = key.strip_prefix("effect:") {
        let command = effects.iter().find(|e| e.effect == effect).map_or("effect.dialog", |e| e.command);
        return CommandRef::with_params(command, json!({ "effect": effect }));
    }
    // A filter plug-in's Object › Plug-ins item, installed or not: what its item runs.
    if let Some(id) = key.strip_prefix("plugin:") {
        return CommandRef::with_params("plugin.dialog", json!({ "id": id }));
    }
    // Tab's panels toggle is the framework's chrome command: only its chord toggles the panels instead of moving focus.
    if key == DOCK {
        return ChromeCommand::TogglePanels.into();
    }
    CommandRef::new(key)
}

/// The entry key of Tab's Panels toggle (`window.dock`), which runs the framework's `ChromeCommand::TogglePanels`.
const DOCK: &str = "window.dock";

/// The app command whose state the row of `command` shows: `window.dock` for the framework's Panels toggle.
pub(crate) fn state_id(command: &CommandRef) -> &str {
    if ChromeCommand::from_ref(command) == Some(ChromeCommand::TogglePanels) { DOCK } else { &command.id }
}

/// `overrides` with each Effect menu item's shortcut under the command its item runs while the
/// menu holds `effects`: a shortcut saved while a plug-in effect was missing, or while it had
/// options it no longer has, stays that effect's.
pub(crate) fn effect_keys_in(overrides: &Overrides, effects: &[EffectItem]) -> Overrides {
    let mut out = overrides.clone();
    for (k, texts) in overrides.iter() {
        let entry = entry_of_stored(k);
        if !entry.starts_with("effect:") {
            continue;
        }
        let now = command_in(&entry, effects).key();
        if now != k && out.get(&now).is_none() {
            out.remove(k);
            let _ = out.insert(now, texts.to_vec());
        }
    }
    out
}

/// The entry key of a command: `panel:<id>` for `window.panel {panel}`, `effect:<id>` for an
/// Effect menu item and `plugin:<id>` for `plugin.dialog {id}`.
pub fn entry_key(command: &CommandRef) -> String {
    match (command.id.as_str(), command.params.get("panel").and_then(Value::as_str)) {
        ("window.panel", Some(panel)) => format!("panel:{panel}"),
        ("effect.dialog" | "effect.apply", _) if command.params.get("effect").and_then(Value::as_str).is_some() => {
            format!("effect:{}", command.params.get("effect").and_then(Value::as_str).unwrap_or_default())
        }
        ("plugin.dialog", _) if command.params.get("id").and_then(Value::as_str).is_some() => {
            format!("plugin:{}", command.params.get("id").and_then(Value::as_str).unwrap_or_default())
        }
        _ if ChromeCommand::from_ref(command) == Some(ChromeCommand::TogglePanels) => DOCK.to_string(),
        _ => command.key(),
    }
}

/// The [`Overrides`] key of an entry key.
pub fn stored_key(key: &str) -> String {
    if key.starts_with("panel:") || key.starts_with("effect:") || key.starts_with("plugin:") || key == DOCK {
        command_of(key).key()
    } else {
        key.to_string()
    }
}

/// An Effect menu item's [`Overrides`] key under the command its item does not run now
/// (`effect.apply` for one that opens its dialog, and the other way round).
pub(crate) fn other_stored_key(key: &str) -> Option<String> {
    let effect = key.strip_prefix("effect:")?;
    let other = if command_of(key).id == "effect.apply" { "effect.dialog" } else { "effect.apply" };
    Some(CommandRef::with_params(other, json!({ "effect": effect })).key())
}

/// The entry key of an [`Overrides`] key.
pub fn entry_of_stored(key: &str) -> String {
    CommandRef::parse_key(key)
        .filter(|c| {
            matches!(c.id.as_str(), "window.panel" | "effect.dialog" | "effect.apply" | "plugin.dialog") || ChromeCommand::from_ref(c).is_some()
        })
        .map_or_else(|| key.to_string(), |c| entry_key(&c))
}

// ---------- chord texts ----------

/// Plus alone is Plus; with modifiers the key is Equals (`+` is Shift+`=` on many layouts, so a
/// `Cmd+=` chord answers `+` however it is typed). Every reader of a chord's key goes through it.
fn plus_rule(mods: Mods, key: Key) -> Key {
    match key {
        Key::Plus | Key::Equals if mods.is_none() => Key::Plus,
        Key::Plus | Key::Equals => Key::Equals,
        k => k,
    }
}

/// The chord a key press with `modifiers` makes on `platform`; `None` for a key the crate does
/// not name.
pub fn chord_of_press(key: egui::Key, modifiers: egui::Modifiers, platform: Platform) -> Option<Chord> {
    let key = Key::from_egui(key)?;
    let mods = Mods::from_egui(&modifiers, platform);
    Some(Chord::new(mods, plus_rule(mods, key)))
}

/// A VectorCraft chord text as the menus show it on this platform; text that is no chord is
/// shown as written.
pub(crate) fn chord_text(s: &str) -> String {
    stored::parse_vc(s).map_or_else(|| s.to_string(), |c| c.display(platform()))
}

// ---------- catalogue ----------

/// A default shortcut of the catalogue.
fn vc_binding(text: &str) -> Option<Binding> {
    stored::parse_vc(text).map(Binding::new)
}

/// The default bindings of an entry on `p`: its default text, then the aliases the registry
/// spec does not carry, without repeats.
fn defaults_of(key: &str, p: Platform) -> Vec<Binding> {
    let mut texts: Vec<&str> = shortcut_editor::default_on(key, p).into_iter().collect();
    texts.extend(default_aliases(key, p).iter().copied());
    let mut out: Vec<Binding> = vec![];
    for b in texts.into_iter().filter_map(vc_binding) {
        if !out.contains(&b) {
            out.push(b);
        }
    }
    out
}

/// Every listed entry as the crate sees it, in dispatch priority: commands (engine, then UI),
/// panels, tools. Each tool is its own group: tools never cycle.
pub(crate) fn catalog_defs(platform: Platform, snapshot: &Snapshot) -> Vec<CommandDef> {
    let rank = |e: &Entry| {
        if e.is_tool {
            2
        } else if e.key.starts_with("panel:") {
            1
        } else {
            0
        }
    };
    let mut entries: Vec<&Entry> = snapshot.entries.iter().collect();
    entries.sort_by_key(|e| rank(e));
    entries
        .into_iter()
        .map(|e| {
            let mut path: Vec<String> = e.group.split(" › ").filter(|s| !s.is_empty()).map(str::to_string).collect();
            path.push(e.label.clone());
            let mut def = CommandDef::new(command_in(&e.key, &snapshot.effects), e.label.clone(), Scope::app()).with_path(path);
            if let Some(tool) = e.key.strip_prefix("tool:") {
                def = def.with_kind(CommandKind::Tool { group: Some(tool.to_string()) });
            }
            if TOGGLES.contains(&e.key.as_str()) {
                def = def.toggle();
            }
            defaults_of(&e.key, platform).into_iter().fold(def, CommandDef::with_default)
        })
        .collect()
}

/// Additional defaults for commands whose primary shortcut remains in their command spec.
fn default_aliases(key: &str, p: Platform) -> &'static [&'static str] {
    match key {
        "edit.cut" if !p.is_mac() => &["F2"],
        "edit.copy" if !p.is_mac() => &["F3"],
        "edit.paste" if !p.is_mac() => &["F4"],
        "paint.lastColor" => &["Shift+,"],
        "paint.lastGradient" => &["Shift+."],
        "edit.clear" => &["Backspace"],
        _ => &[],
    }
}

/// Cut, Copy and Paste with their primary shortcut and the function key [`default_aliases`] adds
/// outside macOS.
pub(crate) const EDIT_FUNCTION_KEYS: [(&str, &str, &str); 3] =
    [("edit.cut", "Cmd+X", "F2"), ("edit.copy", "Cmd+C", "F3"), ("edit.paste", "Cmd+V", "F4")];

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
        let p = craft::keymap::egui::platform(ctx);
        if let Ok(mut web) = WEB_PLATFORM.write()
            && *web != Some(p)
        {
            *web = Some(p);
        }
    }
}

/// The loader of saved shortcut sets. [`profiles`] is the form this version writes; [`legacy`]
/// reads the three keys earlier versions wrote (`shortcut_overrides`, `shortcut_set`,
/// `shortcut_holds`), in VectorCraft's own spelling. Loading never fails: what cannot be used is
/// dropped or kept for `Keymap::problems`, and only an entry's first text is kept.
pub(crate) mod stored {
    use craft::keymap::{Binding, Chord, Key, Mods, Overrides, Platform, Profiles};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::{Map, Value};

    use super::{EDIT_FUNCTION_KEYS, entry_of_stored, plus_rule, stored_key, vc_binding};
    use crate::shortcut_editor;

    /// Serde `with` module of the saved [`Profiles`]: the crate's own form.
    pub(crate) mod profiles {
        use super::{Deserialize, Deserializer, Profiles, Serialize, Serializer};

        /// Writes the sets.
        pub(crate) fn serialize<S: Serializer>(profiles: &Profiles, s: S) -> Result<S::Ok, S::Error> {
            profiles.serialize(s)
        }

        /// Reads the sets.
        pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Profiles, D::Error> {
            Profiles::deserialize(d)
        }
    }

    /// The sets that the three keys of earlier versions hold: the overrides on top of the set in
    /// use become a user set, through the crate's own name and size checks.
    pub(crate) fn legacy(overrides: &Value, set: Option<&str>, holds: &Value) -> Profiles {
        legacy_on(overrides, set, holds, super::platform())
    }

    /// [`legacy`] on platform `p`.
    pub(super) fn legacy_on(overrides: &Value, set: Option<&str>, holds: &Value, p: Platform) -> Profiles {
        let mut ov = from_value(overrides.clone());
        add_holds(&mut ov, holds);
        pin_edit_primaries(&mut ov, p);
        let mut profiles = Profiles::default();
        if let Some(name) = set {
            // A name no built-in set has (a custom one) leaves the default set in use.
            let _ = profiles.select(shortcut_editor::set_name(name), &shortcut_editor::BUILTINS);
        }
        if !ov.is_empty() {
            // Refused only over the crate's limits, which drop the entries that exceed them.
            let _ = profiles.store(ov, &shortcut_editor::BUILTINS, shortcut_editor::CUSTOM);
        }
        profiles
    }

    /// Adds each old hold shortcut (`"P"` under an entry) to that entry's list as `Hold P`. An
    /// entry with no override keeps its default shortcuts beside the hold.
    pub(crate) fn add_holds(overrides: &mut Overrides, holds: &Value) {
        let Some(holds) = holds.as_object() else { return };
        for (entry, text) in holds {
            let Some(text) = text.as_str() else { continue };
            let key = stored_key(entry);
            let mut texts: Vec<String> = match overrides.get(&key) {
                Some(t) => t.to_vec(),
                None => super::defaults_of(entry, super::platform()).iter().map(ToString::to_string).collect(),
            };
            texts.push(format!("Hold {}", parse_vc(text).map_or_else(|| text.to_string(), |c| c.to_string())));
            // Refused only over the crate's limits.
            let _ = overrides.insert(key, texts);
        }
    }

    /// When a saved override assigns F2, F3 or F4 elsewhere without changing its edit command,
    /// keep only that edit command's primary shortcut so the saved assignment runs.
    fn pin_edit_primaries(overrides: &mut Overrides, p: Platform) {
        if p.is_mac() {
            return;
        }
        for (key, primary, alias) in EDIT_FUNCTION_KEYS {
            if overrides.get(key).is_some() {
                continue;
            }
            let Some(binding) = vc_binding(alias) else { continue };
            let assigned_elsewhere = overrides.iter().any(|(other, texts)| {
                other != key
                    && shortcut_editor::entry(&entry_of_stored(other)).is_some()
                    && texts.iter().any(|text| Binding::parse(text).is_ok_and(|b| b == binding))
            });
            if assigned_elsewhere {
                let _ = overrides.insert(key, vec![vc_to_crate(primary)]);
            }
        }
    }

    /// Reads VectorCraft chord text. `~` is the backtick key; `+` and `=` are one key: Plus alone,
    /// Equals with modifiers.
    pub(crate) fn parse_vc(text: &str) -> Option<Chord> {
        let text = text.trim();
        let (prefix, key) = if text == "+" {
            ("", "+")
        } else if let Some(mods) = text.strip_suffix("++") {
            (mods.trim_end_matches('+'), "+")
        } else {
            text.rsplit_once('+').unwrap_or(("", text))
        };
        let with = |k: &str| if prefix.is_empty() { k.to_string() } else { format!("{prefix}+{k}") };
        let chord = match key {
            "+" | "=" => {
                let mods =
                    if prefix.is_empty() { Mods { cmd: false, ctrl: false, alt: false, shift: false } } else { Chord::parse(&with("A")).ok()?.mods };
                Chord::new(mods, Key::Equals)
            }
            "~" => Chord::parse(&with("`")).ok()?,
            _ => Chord::parse(text).ok()?,
        };
        Some(Chord::new(chord.mods, plus_rule(chord.mods, chord.key)))
    }

    /// VectorCraft text → the crate's. Text both read as one chord stays as written (`Comma`,
    /// `Ctrl+Cmd+H`), so it is saved back as the user wrote it; text neither reads is kept for
    /// `Keymap::problems`.
    fn vc_to_crate(text: &str) -> String {
        if text.trim().is_empty() {
            return String::new();
        }
        match parse_vc(text) {
            Some(c) if Binding::parse(text).is_ok_and(|b| b == Binding::new(c)) => text.to_string(),
            Some(c) => c.to_string(),
            None => text.to_string(),
        }
    }

    /// The overrides of the map form (`entry key → one text`) or the crate's list form
    /// (`[{command, keys}]`).
    pub(crate) fn from_value(v: Value) -> Overrides {
        let entries: Vec<(String, Value)> = match v {
            Value::Object(m) => m.into_iter().collect(),
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
    use craft::keymap::{Active, Keymap, Lookup, Scopes, Trigger};

    /// The settings of an earlier version: the three old keys in place of `shortcut_sets`.
    fn old_file(keys: Value, set: Option<&str>) -> UiState {
        let mut raw = serde_json::to_value(UiState::default()).unwrap();
        if let Some(o) = raw.as_object_mut() {
            o.remove("shortcut_sets");
        }
        raw["shortcut_overrides"] = keys;
        if let Some(set) = set {
            raw["shortcut_set"] = json!(set);
        }
        // A loaded file's old keys become the sets as the settings are sanitized.
        serde_json::from_value::<UiState>(raw).unwrap().sanitized()
    }

    fn active(ui: &UiState) -> &Overrides {
        ui.shortcut_sets.active_overrides(&shortcut_editor::BUILTINS)
    }

    fn texts<'a>(ov: &'a Overrides, key: &str) -> Option<Vec<&'a str>> {
        ov.get(key).map(|t| t.iter().map(String::as_str).collect())
    }

    /// A settings file in each earlier form loads into the same sets, and this version's file
    /// loads back equal.
    #[test]
    fn older_versions_read_the_saved_overrides() {
        let map = json!({
            "tool:pen": "Shift+9",
            "edit.preferences": "",
            "panel:layers": "Cmd+Alt+Shift+F7",
            "view.grid": "Cmd+Period",
            "tool:curvature": "Shift+~",
        });
        let list = json!([
            {"command": "tool:pen", "keys": ["Shift+9"]},
            {"command": "edit.preferences", "keys": []},
            {"command": "panel:layers", "keys": ["Cmd+Alt+Shift+F7"]},
            {"command": "view.grid", "keys": ["Cmd+Period"]},
            {"command": "tool:curvature", "keys": ["Shift+~"]},
        ]);
        let old_name = shortcut_editor::LEGACY_PRESETS[0].0;
        let named = old_file(map.clone(), Some(old_name));
        for ui in [old_file(map, None), old_file(list, None), named.clone()] {
            let ov = active(&ui);
            assert_eq!(texts(ov, "tool:pen"), Some(vec!["Shift+9"]));
            assert!(ov.get("edit.preferences").is_none_or(|t| t.iter().all(String::is_empty)));
            assert_eq!(texts(ov, &stored_key("panel:layers")), Some(vec!["Cmd+Alt+Shift+F7"]));
            assert_eq!(texts(ov, "tool:curvature"), Some(vec!["Shift+`"]));
        }
        let back: UiState = serde_json::from_value(serde_json::to_value(&named).unwrap()).unwrap();
        assert_eq!(back.shortcut_sets.active(), named.shortcut_sets.active());
        assert_eq!(active(&back), active(&named));
    }

    /// A hand-edited value that is not text drops that entry, not every preference.
    #[test]
    fn a_bad_override_value_keeps_the_other_preferences() {
        let mut raw = serde_json::to_value(UiState { control_bar: true, ..Default::default() }).unwrap();
        if let Some(o) = raw.as_object_mut() {
            o.remove("shortcut_sets");
        }
        raw["shortcut_overrides"] = json!({"x": 5, "tool:pen": "Shift+9"});
        let ui = serde_json::from_value::<UiState>(raw).unwrap().sanitized();
        assert!(ui.control_bar);
        assert!(active(&ui).get("x").is_none());
        assert_eq!(texts(active(&ui), "tool:pen"), Some(vec!["Shift+9"]));
    }

    #[test]
    fn curvature_keeps_shift_tilde_and_plus_is_one_key() {
        let ov = stored::from_value(json!({"tool:curvature": "Shift+~", "tool:addAnchor": "=", "view.zoomIn": "Cmd++"}));
        assert_eq!(texts(&ov, "tool:curvature"), Some(vec!["Shift+`"]));
        assert_eq!(stored::parse_vc("Shift+~"), Some(Chord::parse("Shift+`").unwrap()));
        for t in ["+", "="] {
            assert_eq!(stored::parse_vc(t), Some(Chord::plain(Key::Plus)), "{t}");
        }
        for t in ["Cmd++", "Cmd+="] {
            assert_eq!(stored::parse_vc(t), Some(Chord::new(Mods::CMD, Key::Equals)), "{t}");
        }
        let one = |key: &str| ov.get(key).and_then(|t| t.first()).and_then(|t| stored::parse_vc(t));
        assert_eq!(one("tool:addAnchor"), Some(Chord::plain(Key::Plus)));
        assert_eq!(one("view.zoomIn"), Some(Chord::new(Mods::CMD, Key::Equals)));
        assert_eq!(chord_text("Cmd+,"), Chord::new(Mods::CMD, Key::Comma).display(platform()));
    }

    #[test]
    fn the_default_shortcuts_do_not_conflict() {
        // One snapshot: tests running beside this one install and remove plug-ins.
        let snapshot = shortcut_editor::snapshot();
        for p in [Platform::Windows, Platform::Linux, Platform::Mac] {
            let km = Keymap::new(catalog_defs(p, &snapshot), Scopes::new(), p).with_reserved(craft::keymap::reserved::system(p));
            assert_eq!(km.conflicts(), vec![], "{p:?}");
            assert_eq!(km.problems(), vec![], "{p:?}");
            assert_eq!(km.catalog().len(), snapshot.entries.len(), "{p:?}: every entry is bindable");
            for e in snapshot.entries.iter() {
                // A default that no longer reads would show in the menus but never run.
                assert!(shortcut_editor::default_on(&e.key, p).is_none_or(|d| stored::parse_vc(d).is_some()), "{p:?} {}", e.key);
                let want = defaults_of(&e.key, p);
                let got = km.default_bindings(&command_in(&e.key, &snapshot.effects));
                assert_eq!(got, want, "{p:?} {}", e.key);
                assert!(want.iter().all(|b| !km.is_reserved(&b.trigger.chord)), "{p:?} {}: a default on a reserved chord", e.key);
            }
            let clear = km.default_bindings(&command_of("edit.clear"));
            assert!(clear.contains(&vc_binding("Backspace").unwrap()), "{p:?}: Clear has Backspace");
        }
    }

    /// The alias rule of the loader: Cut, Copy and Paste carry F2, F3 and F4 off macOS, unless a
    /// saved assignment gives the key to another command without touching them.
    #[test]
    fn edit_function_keys_follow_platform_and_reset_restores_both_bindings() {
        let snapshot = shortcut_editor::snapshot();
        let run = |km: &Keymap, text: &str| match km.lookup(&stored::parse_vc(text).unwrap(), false, &Active::NONE) {
            Lookup::Run(c) => Some(c),
            _ => None,
        };
        for p in [Platform::Windows, Platform::Linux, Platform::Mac] {
            for (alias, edit, primary) in [("F2", "edit.cut", "Cmd+X"), ("F3", "edit.copy", "Cmd+C"), ("F4", "edit.paste", "Cmd+V")] {
                let keymap = |sets: &craft::keymap::Profiles| {
                    Keymap::new(catalog_defs(p, &snapshot), Scopes::new(), p)
                        .with_overrides(sets.active_overrides(&shortcut_editor::BUILTINS).clone())
                };
                let kept = keymap(&stored::legacy_on(&json!({}), None, &Value::Null, p));
                assert_eq!(run(&kept, alias), (!p.is_mac()).then(|| command_of(edit)), "{p:?} {alias} default");
                assert_eq!(run(&kept, primary), Some(command_of(edit)), "{p:?} {primary}");
                let moved = keymap(&stored::legacy_on(&json!({"tool:pen": alias}), None, &Value::Null, p));
                assert_eq!(run(&moved, alias), Some(command_of("tool:pen")), "{p:?} saved {alias}");
                assert_eq!(run(&moved, primary), Some(command_of(edit)), "{p:?} primary {edit} stays");
            }
        }
    }

    #[test]
    fn old_hold_shortcuts_load_as_hold_bindings() {
        let sets = stored::legacy(&json!({}), Some(shortcut_editor::LEGACY_PRESETS[0].0), &json!({"tool:pen": "P"}));
        let ov = sets.active_overrides(&shortcut_editor::BUILTINS);
        let pen = texts(ov, "tool:pen").unwrap();
        assert!(pen.len() > 1, "the single binding stays beside the hold: {pen:?}");
        assert_eq!(pen.last(), Some(&"Hold P"));
        assert!(Trigger::parse("Hold P").is_ok_and(|t| t.is_hold()));
    }
}
