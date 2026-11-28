//! Import Shortcut Set: the entry points that hand a shortcut set another app saved as text to
//! Keyboard Shortcuts' review. Every set here is written for the test, with invented names and codes.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::shortcut_import::{foreign_set, remember};
use crate::{VectorcraftApp, io, shortcut_editor};

fn app() -> VectorcraftApp {
    VectorcraftApp::new(Session::new(), Default::default())
}

/// One frame of a window, which builds its keymap, then a pass of Keyboard Shortcuts in it, which
/// takes up a pending review.
fn frame(app: &mut VectorcraftApp) {
    let ctx = crate::testing::ui_context();
    crate::tests::synthetic::key_frame(app, &ctx, vec![]);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| shortcut_editor::show(app, ui));
    out.textures_delta.clear();
}

/// Keyboard Shortcuts' keymap, once a frame has given it one.
fn dialog_keymap(app: &mut VectorcraftApp) -> &mut craft::keymap::Keymap {
    frame(app);
    app.shortcuts_dialog.as_mut().and_then(|d| d.keymap.as_mut()).expect("the dialog's keymap")
}

/// A set's name and each entry's item, line and target.
type Review = (String, Vec<(String, Option<usize>, Option<craft::keymap::CommandRef>)>);

/// The review the dialog shows.
fn review(app: &VectorcraftApp) -> Option<Review> {
    let d = app.shortcuts_dialog.as_ref()?;
    let r = d.state.snapshot(d.keymap.as_ref()?)?.review?;
    Some((r.name, r.entries.into_iter().map(|e| (e.item, e.line, e.target)).collect()))
}

fn remember_matches(app: &mut VectorcraftApp, matches: Value) {
    app.run("importMatches.set", json!({ "matches": matches })).unwrap();
}

const SET: &str = "/Menus {\n /Edit {\n  /Acme\\ Undo { /Context 0 /Modifiers 11 /Represent 0 /Key 902 }\n  /Acme\\ Copy { /Modifiers 11 /Key 902 }\n }\n /Acme\\ Pen { /Modifiers 11 /Key 901 }\n /Acme\\ Gadget { /Modifiers 0 /Key 48 }\n}\n";

/// The answers an earlier import left for [`SET`].
fn earlier_answers(app: &mut VectorcraftApp) {
    remember_matches(
        app,
        json!([
            {"kind": "name", "name": "Acme Undo", "target": "edit.undo"},
            {"kind": "name", "name": "Acme Copy", "target": "edit.copy"},
            {"kind": "name", "name": "Acme Pen", "target": "tool:pen"},
            {"kind": "name", "name": "Acme Gadget", "target": ""},
            {"kind": "key", "name": "901", "target": "K"},
            {"kind": "key", "name": "902", "target": "J"},
            {"kind": "modifiers", "name": "11", "target": "Cmd"},
        ]),
    );
}

/// What `shortcutSet.read` makes of `text`.
fn read(app: &mut VectorcraftApp, text: &str) -> Value {
    app.run("shortcutSet.read", json!({ "name": "Set.kys", "data": text })).unwrap()
}

#[test]
fn kys_files_open_the_review() {
    let mut app = app();
    earlier_answers(&mut app);
    io::open_bytes(&mut app, "Acme Keys.kys", SET.as_bytes(), None).unwrap();
    frame(&mut app);
    let (name, entries) = review(&app).expect("a review is open");
    assert_eq!(name, "Acme Keys");
    let places: Vec<(&str, Option<usize>)> = entries.iter().map(|(item, line, _)| (item.as_str(), *line)).collect();
    let lines = [("Menus > Edit > Acme Undo", 3), ("Menus > Edit > Acme Copy", 4), ("Menus > Acme Pen", 6), ("Menus > Acme Gadget", 7)];
    assert_eq!(places, lines.map(|(item, line)| (item, Some(line))));
    let targets: Vec<Option<String>> = entries.iter().map(|(.., t)| t.as_ref().map(crate::keymap::entry_key)).collect();
    assert_eq!(targets, [Some("edit.undo".into()), Some("edit.copy".into()), Some("tool:pen".into()), None], "saved targets come first");

    // The format's two described codes are decided; a code nobody answered is left out.
    let (_, options) = foreign_set(&read(&mut app, "/Menus { /Acme\\ Odd { /Modifiers 77 /Key 48 } /Acme\\ Even { /Modifiers 0 /Key 78 } }"));
    assert_eq!((options.codes.get("key:48"), options.codes.get("mods:0")), (Some(&Some("0".to_string())), Some(&Some(String::new()))));
    assert!(!options.codes.contains_key("key:78") && !options.codes.contains_key("mods:77"));

    // A file in another form says so, and opens nothing.
    let mut fresh = self::app();
    let e = io::open_bytes(&mut fresh, "Other.kys", b"<?xml version=\"1.0\"?><shortcuts/>", None).unwrap_err();
    assert_eq!(e, vectorcraft_engine::cmd::shortcutset::NOT_READABLE);
    frame(&mut fresh);
    assert!(review(&fresh).is_none());
}

/// A set dropped on the window opens the review when no document is open, from its path (desktop)
/// or from its bytes (web).
#[test]
fn a_set_dropped_with_no_document_open_opens_the_review() {
    let mut app = app();
    let dir = vectorcraft_testkit::temp_dir("shortcut-import-drop");
    let kys = dir.join("Dropped.kys");
    std::fs::write(&kys, SET).unwrap();
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    let target = app.drop_target("Dropped.kys", crate::place::DropSpot::Unknown, false);
    assert_eq!(target, crate::place::DropTarget::Open);
    crate::place::drop_files(&mut app, vec![(target, ("Dropped.kys".into(), Some(kys.to_string_lossy().into_owned()), vec![]))]);
    frame(&mut app);
    let (name, entries) = review(&app).expect("a review is open");
    assert_eq!((name.as_str(), entries.len()), ("Dropped", 4));
    app.shortcuts_dialog = None;
    app.services.read = None;
    crate::place::drop_files(&mut app, vec![(target, ("Sent.kys".into(), None, SET.as_bytes().to_vec()))]);
    frame(&mut app);
    let (name, entries) = review(&app).expect("a review is open");
    assert_eq!((name.as_str(), entries.len()), ("Sent", 4));
    let _ = std::fs::remove_dir_all(dir);
}

/// What the review reports the user answered is kept and answers the next import.
#[test]
fn remembered_choices_answer_the_next_import() {
    let mut app = app();
    let kys = "/Menus {\n /Acme\\ Undo { /Modifiers 21 /Key 931 }\n /Acme\\ Thing { /Modifiers 21 /Key 932 }\n}\n";
    let (set, options) = foreign_set(&read(&mut app, kys));
    assert!(options.remembered.is_empty() && !options.codes.contains_key("key:931"));
    let [undo, thing] = [&set.entries[0], &set.entries[1]];
    let targets = BTreeMap::from([(undo.remember_key(), Some(crate::keymap::command_of("edit.undo"))), (thing.remember_key(), None)]);
    let codes = BTreeMap::from([("key:931".to_string(), Some("Z".to_string())), ("mods:21".to_string(), Some("Cmd+Alt".to_string()))]);
    shortcut_editor::open(&mut app);
    remember(&mut app, &set, &targets, &codes);
    let kept = app.run("importMatches.list", json!({})).unwrap();
    let kept: Vec<(&str, &str, &str)> =
        kept.as_array().unwrap().iter().map(|m| (m["kind"].as_str().unwrap(), m["name"].as_str().unwrap(), m["target"].as_str().unwrap())).collect();
    assert_eq!(kept, [("name", "Acme Undo", "edit.undo"), ("name", "Acme Thing", ""), ("key", "931", "Z"), ("modifiers", "21", "Cmd+Alt")]);
    // The next import of the file has every answer.
    let (set, options) = foreign_set(&read(&mut app, kys));
    assert_eq!(options.remembered.len(), 2);
    assert_eq!(options.remembered.get(&set.entries[1].remember_key()), Some(&None), "No equivalent is kept as an answer");
    assert_eq!((options.codes.get("key:931"), options.codes.get("mods:21")), (Some(&Some("Z".to_string())), Some(&Some("Cmd+Alt".to_string()))));
}

#[test]
fn keyboard_shortcuts_import_reads_both_kinds_of_set() {
    let mut app = app();
    let dir = vectorcraft_testkit::temp_dir("shortcut-import-path");
    let kys = dir.join("On Disk.kys");
    std::fs::write(&kys, SET).unwrap();
    let kys = kys.to_string_lossy().to_string();
    let files: BTreeMap<&str, Vec<u8>> = BTreeMap::from([
        (
            "/sets/Mine.json",
            serde_json::to_vec(&json!({"format": "vectorcraft-shortcuts", "version": 1, "set": "Mine", "overrides": {"tool:pen": "Shift+9"}}))
                .unwrap(),
        ),
        ("/sets/Text.json", b"\xEF\xBB\xBF  /Menus { /Acme\\ Undo { /Key 1 } }".to_vec()),
    ]);
    let reads = std::rc::Rc::new(std::cell::Cell::new(0));
    let counted = reads.clone();
    app.services.read = Some(Box::new(move |p| {
        counted.set(counted.get() + 1);
        files.get(p).cloned().ok_or_else(|| format!("no file {p}"))
    }));
    app.run("shortcuts.import", json!({"path": "/sets/Mine.json"})).unwrap();
    assert_eq!(app.ui.shortcut_sets.active(), "Mine");
    // A `.kys` file is read by `shortcutSet.read`, up to its cap, and not through the app's file reader.
    let before = reads.get();
    app.run("shortcuts.import", json!({ "path": kys })).unwrap();
    frame(&mut app);
    assert_eq!((review(&app).map(|r| (r.0, r.1.len())), reads.get()), (Some(("On Disk".into(), 4)), before));
    // The import waits while Keyboard Shortcuts is open.
    assert!(app.run("shortcuts.import", json!({"path": "/sets/Text.json"})).is_err());
    app.shortcuts_dialog = None;
    app.run("shortcuts.import", json!({"path": "/sets/Text.json"})).unwrap();
    frame(&mut app);
    assert_eq!(review(&app).map(|r| r.0).as_deref(), Some("Text"), "text that starts with a slash name is a text set");
    // File › Open and the command open it from a path, the command also from its bytes.
    app.shortcuts_dialog = None;
    let before = reads.get();
    app.run("file.open", json!({ "path": kys })).unwrap();
    frame(&mut app);
    assert_eq!((review(&app).map(|r| r.1.len()), reads.get()), (Some(4), before));
    assert_eq!(app.ui.recent_files.first(), Some(&kys), "File › Open lists it in Open Recent Files");
    app.shortcuts_dialog = None;
    app.run("ui.shortcutImportDialog", json!({ "path": kys })).unwrap();
    frame(&mut app);
    assert_eq!(review(&app).map(|r| (r.0, r.1.len())), Some(("On Disk".into(), 4)));
    app.shortcuts_dialog = None;
    let b64 = vectorcraft_format::base64_encode(SET.as_bytes());
    let r = app.run("ui.shortcutImportDialog", json!({"dataBase64": b64, "name": "Sent.kys"})).unwrap();
    frame(&mut app);
    assert_eq!((r["entries"].as_u64(), review(&app).map(|r| r.0)), (Some(4), Some("Sent".into())));
    // A file past the reader's cap opens with what fits, marked truncated.
    app.shortcuts_dialog = None;
    let big = format!("/Menus {{ /Acme\\ Undo {{ /Key 1 }} {} }}", " ".repeat(5 << 20));
    io::open_bytes(&mut app, "Big.kys", big.as_bytes(), None).unwrap();
    let pending = app.shortcuts_dialog.as_ref().and_then(|d| d.pending_review.as_ref()).expect("the review waits for its first frame");
    assert_eq!((pending.0.entries.len(), pending.0.truncated), (1, 1));
    let _ = std::fs::remove_dir_all(dir);
}

/// Importing F2 onto another command takes the key from the command that had it.
#[test]
fn an_imported_function_key_goes_to_its_target() {
    if crate::keymap::platform().is_mac() {
        return;
    }
    let mut app = app();
    shortcut_editor::open(&mut app);
    let kys = json!({
        "entries": [{"name": "Acme Duplicate", "section": ["Menus"], "line": 2, "key": 1000, "modifiers": 0, "target": "edit.duplicate"}],
        "codes": {"keys": [{"value": 1000, "key": "F2"}], "modifiers": []},
    });
    let (set, options) = foreign_set(&kys);
    let keymap = dialog_keymap(&mut app);
    let planned = craft::keymap::plan(&set, keymap, &options);
    planned.apply(keymap);
    let f2 = crate::keymap::stored::parse_vc("F2").unwrap();
    assert_eq!(keymap.holder(&f2), Some(crate::keymap::command_of("edit.duplicate")));
}

/// A name of 1,500 characters and 14 repeats of a label stay within what `plan` reads.
#[test]
fn long_names_stay_within_what_plan_reads() {
    use craft::keymap::import::{MAX_IMPORT_TEXT, MAX_PATH_SEGMENTS};
    let mut app = app();
    shortcut_editor::open(&mut app);
    let name = "Acme Copy ".repeat(150);
    let section: Vec<String> = (0..40).map(|i| format!("Block {i}")).collect();
    let read = json!({"entries": [{"name": name, "section": section, "line": 2, "key": 901}]});
    let (set, options) = foreign_set(&read);
    let path = &set.entries[0].path;
    assert!(path.len() <= MAX_PATH_SEGMENTS && path.iter().all(|s| s.chars().count() <= MAX_IMPORT_TEXT));
    let _ = craft::keymap::plan(&set, dialog_keymap(&mut app), &options);
}

/// A plug-in row picked as a target stores under its `plugin.dialog {id}` key and reads back under
/// the filter's id; a removed plug-in is no target.
#[test]
fn an_imported_shortcut_reaches_a_filter_plugin_picked_as_its_target() {
    use vectorcraft_plugins::wat as tpl;
    let id = "test.UI.import_filter-one";
    let key = format!("plugin:{id}");
    let manifest = format!(r#"{{"id":"{id}","name":"Acme Import Filter","version":"1","kind":"filter","params":{{}}}}"#);
    vectorcraft_plugins::registry::install_bytes(&wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap()).unwrap();

    let mut app = app();
    shortcut_editor::open(&mut app);
    let kys = json!({
        "entries": [{"name": "Anything", "section": ["Menus"], "line": 2, "key": 1000, "modifiers": 1000, "target": key}],
        "codes": {"keys": [{"value": 1000, "key": "F11"}], "modifiers": [{"value": 1000, "mods": "Cmd+Alt+Shift"}]},
    });
    let (set, options) = foreign_set(&kys);
    let keymap = dialog_keymap(&mut app);
    let planned = craft::keymap::plan(&set, keymap, &options);
    planned.apply(keymap);
    let stored = crate::keymap::stored_key(&key);
    let saved = keymap.overrides().get(&stored).map(<[String]>::to_vec).expect("a binding of the filter's item");
    assert!(keymap.overrides().get("plugin.dialog").is_none(), "not a binding of plugin.dialog without its id");
    assert_eq!(crate::keymap::entry_of_stored(&stored), key);
    let reloaded = crate::keymap::stored::from_value(json!({ key.as_str(): saved[0] }));
    assert_eq!(reloaded.get(&stored).map(<[String]>::len), Some(1), "read back under the filter's id");

    vectorcraft_plugins::registry::remove(id);
    assert!(shortcut_editor::entries().iter().all(|e| e.key != key), "a removed filter is no target");
}
