//! Import Shortcut Set: shortcut sets another app saved as text, reviewed and applied to Keyboard
//! Shortcuts' working set. Every set here is written for the test, with invented names and codes.

use std::collections::BTreeMap;

use craft_keymap::import::namematch::Candidate;
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::shortcut_import::{Code, KIND, Review, Row, Status, report, target_picker};
use super::*;
use crate::{io, shortcut_editor, theme};

fn app() -> VectorcraftApp {
    VectorcraftApp::new(Session::new(), Default::default())
}

/// One headless frame of the dialog layer, with `events`.
fn frame(app: &mut VectorcraftApp, events: Vec<egui::Event>) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn press(key: egui::Key) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
}

fn dialog(app: &VectorcraftApp) -> &Dialog {
    app.ui.dialog.as_ref().unwrap()
}

fn set(app: &mut VectorcraftApp, key: &str, v: Value) {
    app.ui.dialog.as_mut().unwrap().fields.insert(key.into(), v);
}

fn remember(app: &mut VectorcraftApp, matches: Value) {
    app.run("importMatches.set", json!({ "matches": matches })).unwrap();
}

fn row(name: &str, target: &str, key: i64, modifiers: Option<i64>) -> Row {
    Row { name: name.into(), target: Some(target.into()), from: "matched".into(), key: Some(key), modifiers, on: true, ..Default::default() }
}

/// A review of `rows` with every code unknown, decoded.
fn decoded(rows: Vec<Row>) -> Review {
    let mut r = Review { rows: rows.clone(), ..Default::default() };
    for row in rows {
        r.keys.insert(row.key.unwrap(), Code::default());
        if let Some(m) = row.modifiers {
            r.mods.insert(m, Code::default());
        }
    }
    r.decode();
    r
}

fn text(codes: &BTreeMap<i64, Code>, code: i64) -> Option<&str> {
    codes[&code].text.as_deref()
}

#[test]
fn agreement_decodes_codes_that_defaults_share() {
    for (id, default) in [("edit.undo", "Cmd+Z"), ("edit.redo", "Cmd+Shift+Z"), ("edit.copy", "Cmd+C"), ("edit.cut", "Cmd+X"), ("tool:pen", "P")] {
        assert_eq!(shortcut_editor::default_of(id), Some(default), "{id}");
    }
    let r = decoded(vec![
        row("Acme Undo", "edit.undo", 901, Some(11)),
        row("Acme Redo", "edit.redo", 901, Some(12)),
        row("Acme Copy", "edit.copy", 902, Some(11)),
        row("Acme Cut", "edit.cut", 903, Some(11)),
        row("Acme Pen", "tool:pen", 904, Some(10)),
    ]);
    assert_eq!(text(&r.keys, 901), Some("Z"), "two entries agree");
    assert_eq!((r.keys[&901].from.as_str(), r.keys[&901].votes), ("agreement", 2));
    assert_eq!(text(&r.keys, 902), None, "one vote isn't enough");
    assert_eq!(text(&r.mods, 11), Some("Cmd"));
    assert_eq!((text(&r.mods, 12), text(&r.mods, 10)), (None, None));
    let statuses = r.statuses();
    assert_eq!(statuses[0], (Status::Ready, Some("Cmd+Z".into())));
    assert_eq!(statuses[1].0, Status::NeedsKey, "its modifiers aren't known");
    assert_eq!(statuses[2].0, Status::NeedsKey);
}

#[test]
fn disagreement_leaves_codes_undecided() {
    // Three different keys for one code: nothing is proposed.
    let r = decoded(vec![row("A", "edit.undo", 905, None), row("B", "edit.copy", 905, None), row("C", "edit.cut", 905, None)]);
    assert_eq!(text(&r.keys, 905), None);
    // Four in five agree: proposed. Three in four: not.
    let five = ["edit.undo", "edit.redo", "edit.undo", "edit.redo", "edit.copy"];
    let r = decoded(five.iter().enumerate().map(|(i, t)| row(&format!("E{i}"), t, 906, None)).collect());
    assert_eq!((text(&r.keys, 906), r.keys[&906].votes), (Some("Z"), 4));
    let four = ["edit.undo", "edit.redo", "edit.undo", "edit.copy"];
    let r = decoded(four.iter().enumerate().map(|(i, t)| row(&format!("E{i}"), t, 907, None)).collect());
    assert_eq!(text(&r.keys, 907), None);
    // Rows without a target, or with no equivalent, don't vote; an answer that isn't agreement stays.
    let mut r = decoded(vec![row("A", "edit.undo", 908, None), row("B", "", 908, None)]);
    assert_eq!(text(&r.keys, 908), None);
    r.keys.insert(908, Code { text: Some("Q".into()), from: "you".into(), ..Default::default() });
    r.rows.push(row("C", "edit.redo", 908, None));
    r.decode();
    assert_eq!(text(&r.keys, 908), Some("Q"));
}

#[test]
fn a_key_with_no_equivalent_needs_no_modifiers() {
    let mut r = decoded(vec![row("A", "edit.undo", 940, Some(41))]);
    assert_eq!((text(&r.keys, 940), text(&r.mods, 41)), (None, None));
    r.keys.insert(940, Code { text: Some(String::new()), from: "you".into(), ..Default::default() });
    assert_eq!(r.statuses()[0], (Status::NoEquivalent, None), "the unknown modifiers don't matter");
}

const SET: &str = "/Menus {\n /Edit {\n  /Acme\\ Undo { /Context 0 /Modifiers 11 /Represent 0 /Key 902 }\n  /Acme\\ Copy { /Modifiers 11 /Key 902 }\n }\n /Acme\\ Pen { /Modifiers 11 /Key 901 }\n /Acme\\ Gadget { /Modifiers 0 /Key 48 }\n}\n";

/// The answers an earlier import left for [`SET`].
fn earlier_answers(app: &mut VectorcraftApp) {
    remember(
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

#[test]
fn kys_files_open_the_review() {
    let mut app = app();
    earlier_answers(&mut app);
    io::open_bytes(&mut app, "Acme Keys.kys", SET.as_bytes(), None).unwrap();
    let d = dialog(&app);
    assert_eq!((d.kind.as_str(), d.str("name"), d.str("start"), d.bool("remember")), (KIND, "Acme Keys".into(), "defaults".into(), true));
    let r = Review::of(d);
    let names: Vec<(&str, &str, Option<&str>)> = r.rows.iter().map(|r| (r.name.as_str(), r.section.as_str(), r.target.as_deref())).collect();
    assert_eq!(
        names,
        [
            ("Acme Undo", "Menus › Edit", Some("edit.undo")),
            ("Acme Copy", "Menus › Edit", Some("edit.copy")),
            ("Acme Pen", "Menus", Some("tool:pen")),
            ("Acme Gadget", "Menus", Some(""))
        ]
    );
    assert!(r.rows.iter().all(|r| r.from == "saved"));
    assert_eq!((text(&r.keys, 48), r.keys[&48].from.as_str()), (Some("0"), "described"), "the format's description");
    assert_eq!((text(&r.mods, 0), r.mods[&0].from.as_str()), (Some(""), "described"));
    let statuses: Vec<Status> = r.statuses().into_iter().map(|s| s.0).collect();
    assert_eq!(statuses, [Status::SameShortcut, Status::Ready, Status::ToolKey, Status::NoEquivalent]);
    frame(&mut app, vec![]);
    assert_eq!(dialog(&app).kind, KIND);
    // A file in another form says so, and opens nothing.
    app.ui.dialog = None;
    let e = io::open_bytes(&mut app, "Other.kys", b"<?xml version=\"1.0\"?><shortcuts/>", None).unwrap_err();
    assert_eq!(e, vectorcraft_engine::cmd::shortcutset::NOT_READABLE);
    assert!(app.ui.dialog.is_none());
}

#[test]
fn kys_rows_go_through_assign() {
    let mut app = app();
    earlier_answers(&mut app);
    io::open_bytes(&mut app, "Acme Keys.kys", SET.as_bytes(), None).unwrap();
    let d = dialog(&app).clone();
    assert_eq!(confirm(&mut app).unwrap(), json!({"applied": 1, "rows": 4}));
    // Keyboard Shortcuts shows the set: the later row took ⌘J from Join's default, and the tool kept P.
    assert_eq!(shortcut_editor::default_of("path.join"), Some("Cmd+J"));
    let k = dialog(&app);
    assert_eq!((k.kind.as_str(), k.str("set")), ("shortcuts", "Acme Keys".into()));
    let ov: BTreeMap<String, String> = serde_json::from_value(k.fields["overrides"].clone()).unwrap();
    assert_eq!(ov, BTreeMap::from([("edit.copy".to_string(), "Cmd+J".to_string()), ("path.join".to_string(), String::new())]));
    assert_eq!(k.str("__message"), "Imported. Press OK to keep the imported set.");
    assert!(app.ui.shortcut_overrides.is_empty(), "nothing is kept before OK");
    // The rows left out are reported, with their line.
    let r = Review::of(&d);
    let text = report(&d, &r, &r.statuses());
    assert!(text.starts_with("Acme Keys\nLine 3: Acme Undo: A later row takes this shortcut\n"), "{text}");
    assert!(text.contains("Line 6: Acme Pen: Tools take a single key (optionally with Shift)\n") && !text.contains("Acme Copy"), "{text}");
}

#[test]
fn rows_for_the_same_item_leave_the_earlier_out() {
    let mut app = app();
    earlier_answers(&mut app);
    remember(&mut app, json!([{"kind": "name", "name": "Acme Copy Again", "target": "edit.copy"}]));
    let kys = "/Menus {\n /Acme\\ Copy { /Modifiers 11 /Key 902 }\n /Acme\\ Copy\\ Again { /Modifiers 11 /Key 901 }\n}\n";
    io::open_bytes(&mut app, "Twice.kys", kys.as_bytes(), None).unwrap();
    let r = Review::of(dialog(&app));
    assert_eq!(r.statuses().iter().map(|s| s.0).collect::<Vec<_>>(), [Status::SameTarget, Status::Ready]);
    assert_eq!(confirm(&mut app).unwrap()["applied"], 1);
    let ov = &dialog(&app).fields["overrides"];
    assert_eq!(ov["edit.copy"], "Cmd+K");
    assert!(ov.get("path.join").is_none(), "Join keeps ⌘J: the earlier row isn't applied");
}

#[test]
fn cancel_and_apply_go_back_to_keyboard_shortcuts() {
    let mut app = app();
    earlier_answers(&mut app);
    shortcut_editor::open(&mut app);
    set(&mut app, "overrides", json!({"edit.preferences": "Cmd+Alt+Shift+9"}));
    super::shortcut_import::open(&mut app, "/sets/Acme Keys.kys", SET.as_bytes()).unwrap();
    assert_eq!(dialog(&app).kind, KIND);
    cancel(&mut app);
    assert_eq!((dialog(&app).kind.as_str(), dialog(&app).fields["overrides"].clone()), ("shortcuts", json!({"edit.preferences": "Cmd+Alt+Shift+9"})));
    // Escape goes back the same way, and closes Keyboard Shortcuts after that.
    super::shortcut_import::open(&mut app, "/sets/Acme Keys.kys", SET.as_bytes()).unwrap();
    let escape = |app: &mut VectorcraftApp| {
        let ctx = egui::Context::default();
        let mut out =
            ctx.run_ui(egui::RawInput { events: vec![press(egui::Key::Escape)], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
        out.textures_delta.clear();
    };
    escape(&mut app);
    assert_eq!((dialog(&app).kind.as_str(), dialog(&app).fields["overrides"].clone()), ("shortcuts", json!({"edit.preferences": "Cmd+Alt+Shift+9"})));
    escape(&mut app);
    assert!(app.ui.dialog.is_none());
    shortcut_editor::open(&mut app);
    set(&mut app, "overrides", json!({"edit.preferences": "Cmd+Alt+Shift+9"}));
    // Added to the current shortcuts: the working set keeps its own change.
    super::shortcut_import::open(&mut app, "/sets/Acme Keys.kys", SET.as_bytes()).unwrap();
    set(&mut app, "start", json!("current"));
    confirm(&mut app).unwrap();
    assert_eq!(dialog(&app).fields["overrides"], json!({"edit.copy": "Cmd+J", "edit.preferences": "Cmd+Alt+Shift+9", "path.join": ""}));
    // OK keeps it.
    confirm(&mut app).unwrap();
    assert_eq!((app.ui.shortcut_set.as_str(), app.ui.shortcut_overrides.len()), ("Acme Keys", 3));
}

/// A document's Missing Fonts dialog that Keyboard Shortcuts replaced waits while Cancel goes back
/// from Import Shortcut Set to Keyboard Shortcuts, and opens again once Keyboard Shortcuts closes.
#[test]
fn cancel_goes_back_to_keyboard_shortcuts_before_a_waiting_missing_fonts_dialog() {
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 40, "text": "Type", "font": "No Such Font Family"})).unwrap();
    app.run("ui.missingFontsDialog", json!({})).unwrap();
    assert_eq!(dialog(&app).kind, missing_fonts::KIND);
    shortcut_editor::open(&mut app);
    set(&mut app, "overrides", json!({"edit.preferences": "Cmd+Alt+Shift+9"}));
    super::shortcut_import::open(&mut app, "/sets/Acme Keys.kys", SET.as_bytes()).unwrap();
    cancel(&mut app);
    assert_eq!((dialog(&app).kind.as_str(), dialog(&app).fields["overrides"].clone()), ("shortcuts", json!({"edit.preferences": "Cmd+Alt+Shift+9"})));
    cancel(&mut app);
    assert_eq!(dialog(&app).kind, missing_fonts::KIND);
    assert_eq!(dialog(&app).fields["fonts"][0]["family"], "No Such Font Family");
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
    let target = app.drop_target("Dropped.kys", None, false);
    assert_eq!(target, crate::place::DropTarget::Open);
    crate::place::drop_files(&mut app, vec![(target, ("Dropped.kys".into(), Some(kys.to_string_lossy().into_owned()), vec![]))]);
    assert_eq!((dialog(&app).kind.as_str(), dialog(&app).str("name"), Review::of(dialog(&app)).rows.len()), (KIND, "Dropped".into(), 4));
    app.ui.dialog = None;
    app.services.read = None;
    crate::place::drop_files(&mut app, vec![(target, ("Sent.kys".into(), None, SET.as_bytes().to_vec()))]);
    assert_eq!((dialog(&app).kind.as_str(), dialog(&app).str("name")), (KIND, "Sent".into()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn remembered_choices_answer_the_next_import() {
    let mut app = app();
    let kys = "/Menus {\n /Acme\\ Undo { /Modifiers 21 /Key 931 }\n /Acme\\ Thing { /Modifiers 21 /Key 932 }\n}\n";
    io::open_bytes(&mut app, "Fresh.kys", kys.as_bytes(), None).unwrap();
    let r = Review::of(dialog(&app));
    assert_eq!(r.rows[0].target.as_deref(), Some("edit.undo"), "proposed from the name");
    assert_eq!(r.rows[0].from, "matched");
    assert_eq!(r.statuses().iter().map(|s| s.0).collect::<Vec<_>>(), [Status::NeedsKey, Status::NeedsChoice]);
    assert!(dialog(&app).bool("__keysOpen"), "codes to answer open the Keys section");
    // The user answers the codes and picks the second row's target.
    let mut r = r;
    r.keys.insert(931, Code { text: Some("Z".into()), from: "you".into(), ..r.keys[&931].clone() });
    r.mods.insert(21, Code { text: Some("Cmd+Alt".into()), from: "you".into(), ..r.mods[&21].clone() });
    r.rows[1].target = Some(String::new());
    r.rows[1].from = "you".into();
    set(&mut app, "rows", serde_json::to_value(&r.rows).unwrap());
    set(&mut app, "keys", serde_json::to_value(&r.keys).unwrap());
    set(&mut app, "mods", serde_json::to_value(&r.mods).unwrap());
    confirm(&mut app).unwrap();
    let kept = app.run("importMatches.list", json!({})).unwrap();
    let kept: Vec<(&str, &str, &str)> =
        kept.as_array().unwrap().iter().map(|m| (m["kind"].as_str().unwrap(), m["name"].as_str().unwrap(), m["target"].as_str().unwrap())).collect();
    assert_eq!(kept, [("name", "Acme Undo", "edit.undo"), ("name", "Acme Thing", ""), ("key", "931", "Z"), ("modifiers", "21", "Cmd+Alt")]);
    assert_eq!(app.run("importMatches.list", json!({"kind": "name"})).unwrap()[0]["label"], "Undo");
    // The next import asks nothing.
    app.ui.dialog = None;
    io::open_bytes(&mut app, "Fresh.kys", kys.as_bytes(), None).unwrap();
    let r = Review::of(dialog(&app));
    assert!(r.rows.iter().all(|r| r.from == "saved"));
    assert_eq!(r.statuses().iter().map(|s| s.0).collect::<Vec<_>>(), [Status::Ready, Status::NoEquivalent]);
    assert_eq!(r.statuses()[0].1.as_deref(), Some("Cmd+Alt+Z"));
    // The saved codes stay listed, in a Keys section that starts closed, and can be answered again.
    assert!(!dialog(&app).bool("__keysOpen"));
    set(&mut app, "__keysOpen", json!(true));
    frame(&mut app, vec![]);
    let mut r = r;
    r.keys.insert(931, Code { text: Some("Y".into()), from: "you".into(), ..r.keys[&931].clone() });
    set(&mut app, "keys", serde_json::to_value(&r.keys).unwrap());
    confirm(&mut app).unwrap();
    assert_eq!(importmatches_target(&app, "key", "931"), Some("Y".to_string()));
    // Without Remember My Choices nothing more is kept.
    app.ui.dialog = None;
    io::open_bytes(&mut app, "Fresh.kys", kys.as_bytes(), None).unwrap();
    set(&mut app, "remember", json!(false));
    let before = app.session.prefs.import_matches.clone();
    let mut r = Review::of(dialog(&app));
    r.keys.insert(931, Code { text: Some("W".into()), from: "you".into(), ..r.keys[&931].clone() });
    set(&mut app, "keys", serde_json::to_value(&r.keys).unwrap());
    confirm(&mut app).unwrap();
    assert_eq!(app.session.prefs.import_matches, before);
}

/// What the import matches say `name` of `kind` stands for.
fn importmatches_target(app: &VectorcraftApp, kind: &str, name: &str) -> Option<String> {
    vectorcraft_engine::cmd::importmatches::targets(&app.session.prefs).get(&(kind, name)).map(|t| t.to_string())
}

#[test]
fn a_key_press_answers_the_code_being_recorded() {
    let mut app = app();
    let kys = "/Menus {\n /Acme\\ Undo { /Modifiers 21 /Key 931 }\n /Acme\\ Redo { /Modifiers 21 /Key 931 }\n}\n";
    io::open_bytes(&mut app, "Keys.kys", kys.as_bytes(), None).unwrap();
    frame(&mut app, vec![]);
    set(&mut app, "__recording", json!("931"));
    assert!(shortcut_editor::is_recording(&app), "Escape stops the recording, not the dialog");
    // A modifier's own key press waits for the key.
    frame(&mut app, vec![press(egui::Key::ShiftLeft)]);
    assert_eq!(dialog(&app).str("__recording"), "931");
    frame(&mut app, vec![press(egui::Key::K)]);
    let r = Review::of(dialog(&app));
    assert_eq!((text(&r.keys, 931), r.keys[&931].from.as_str(), dialog(&app).str("__recording")), (Some("K"), "you", String::new()));
    // Escape stops recording and keeps the answer.
    set(&mut app, "__recording", json!("931"));
    frame(&mut app, vec![press(egui::Key::Escape)]);
    assert_eq!((text(&Review::of(dialog(&app)).keys, 931), dialog(&app).str("__recording")), (Some("K"), String::new()));
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
            serde_json::to_vec(&shortcut_editor::export_json("Mine", &crate::keymap::overrides_from(&[("tool:pen", "Shift+9")]))).unwrap(),
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
    assert_eq!(app.ui.shortcut_set, "Mine");
    // A `.kys` file is read by `shortcutSet.read`, up to its cap, and not through the app's file reader.
    let before = reads.get();
    app.run("shortcuts.import", json!({ "path": kys })).unwrap();
    assert_eq!((dialog(&app).kind.as_str(), dialog(&app).str("name"), reads.get()), (KIND, "On Disk".into(), before));
    app.run("shortcuts.import", json!({"path": "/sets/Text.json"})).unwrap();
    assert_eq!(dialog(&app).str("name"), "Text", "text that starts with a slash name is a text set");
    // File › Open and the command open it from a path, the command also from its bytes.
    app.ui.dialog = None;
    let before = reads.get();
    app.run("file.open", json!({ "path": kys })).unwrap();
    assert_eq!((Review::of(dialog(&app)).rows.len(), reads.get()), (4, before));
    assert_eq!(app.ui.recent_files.first(), Some(&kys), "File › Open lists it in Open Recent Files");
    app.ui.dialog = None;
    app.run("ui.shortcutImportDialog", json!({ "path": kys })).unwrap();
    assert_eq!((Review::of(dialog(&app)).rows.len(), dialog(&app).str("name")), (4, "On Disk".into()));
    let b64 = vectorcraft_format::base64_encode(SET.as_bytes());
    let r = app.run("ui.shortcutImportDialog", json!({"dataBase64": b64, "name": "Sent.kys"})).unwrap();
    assert_eq!((r["entries"].as_u64(), dialog(&app).str("name")), (Some(4), "Sent".into()));
    // A file past the reader's cap opens with what fits, marked truncated.
    let big = format!("/Menus {{ /Acme\\ Undo {{ /Key 1 }} {} }}", " ".repeat(5 << 20));
    io::open_bytes(&mut app, "Big.kys", big.as_bytes(), None).unwrap();
    assert_eq!((Review::of(dialog(&app)).rows.len(), dialog(&app).bool("truncated")), (1, true));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn many_rows_draw_and_filter() {
    let mut app = app();
    let body: String = (0..2000).map(|i| format!(" /Acme\\ Entry\\ {i} {{ /Modifiers {} /Key {} }}\n", i % 7, 1000 + i % 90)).collect();
    io::open_bytes(&mut app, "Big.kys", format!("/Menus {{\n{body}}}\n").as_bytes(), None).unwrap();
    frame(&mut app, vec![]);
    set(&mut app, "__attention", json!(true));
    set(&mut app, "__query", json!("entry 19"));
    frame(&mut app, vec![]);
    assert_eq!(Review::of(dialog(&app)).rows.len(), 2000);
    // Notes on the parts not read are kept for the report.
    app.ui.dialog = None;
    io::open_bytes(&mut app, "Odd.kys", b"/Menus { /Acme\\ Undo { /Key 1 /Bogus 2 } junk }", None).unwrap();
    let d = dialog(&app).clone();
    let r = Review::of(&d);
    let text = report(&d, &r, &r.statuses());
    assert!(text.contains("Line 1: /Bogus 2: Not read\n") && text.contains("Line 1: junk: Not read\n"), "{text}");
    frame(&mut app, vec![]);
}

/// The texts the sixth frame of the dialog layer painted on `screen`, each with its rectangle and
/// the rectangle it was clipped to.
fn painted_on(app: &mut VectorcraftApp, screen: egui::Rect) -> Vec<(String, egui::Rect, egui::Rect)> {
    fn collect(s: &egui::Shape, clip: egui::Rect, v: &mut Vec<(String, egui::Rect, egui::Rect)>) {
        match s {
            egui::Shape::Text(t) => v.push((t.galley.text().to_string(), t.visual_bounding_rect(), clip)),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, clip, v)),
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let mut texts = vec![];
    for _ in 0..6 {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
        texts.clear();
        for c in &out.shapes {
            collect(&c.shape, c.clip_rect, &mut texts);
        }
    }
    texts
}

/// On the app's smallest window (800 × 500 points), a large set shows the heading, Copy Report,
/// Cancel and Apply, with the Keys section open or closed: its lists get shorter.
#[test]
fn a_large_set_keeps_its_buttons_on_a_small_window() {
    let mut app = app();
    let body: String = (0..400).map(|i| format!(" /Acme\\ Entry\\ {i} {{ /Modifiers {} /Key {} }}\n", i % 7, 1000 + i % 90)).collect();
    io::open_bytes(&mut app, "Big.kys", format!("/Menus {{\n{body}}}\n").as_bytes(), None).unwrap();
    // Every row has a target, so the Keys section lists the set's 97 codes.
    let mut r = Review::of(dialog(&app));
    for row in &mut r.rows {
        row.target = Some("edit.undo".into());
    }
    set(&mut app, "rows", serde_json::to_value(&r.rows).unwrap());
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 500.0));
    for open in [true, false] {
        set(&mut app, "__keysOpen", json!(open));
        let texts = painted_on(&mut app, screen);
        for label in ["Import Shortcut Set", "Copy Report", "Cancel", "Apply"] {
            let (_, rect, clip) = texts.iter().find(|(t, ..)| t == label).unwrap_or_else(|| panic!("Keys open: {open}: no {label}"));
            assert!(screen.contains_rect(*rect) && clip.contains_rect(*rect), "Keys open: {open}: {label} at {rect:?}, clipped to {clip:?}");
        }
    }
}

/// Applies `rows` (name, section, target, shortcut) through the review, every code answered, starting
/// from `start` → Apply's result and the working set Keyboard Shortcuts shows.
fn apply_rows(app: &mut VectorcraftApp, start: &str, rows: &[(&str, &str, &str, &str)]) -> (Value, Value) {
    super::shortcut_import::open(app, "Set.kys", SET.as_bytes()).unwrap();
    let mut r = Review::default();
    for (i, (name, section, target, chord)) in rows.iter().enumerate() {
        let code = 1000 + i as i64;
        let (mods, key) = chord.rsplit_once('+').unwrap_or(("", chord));
        r.keys.insert(code, Code { text: Some(key.into()), from: "you".into(), ..Default::default() });
        r.mods.insert(code, Code { text: Some(mods.into()), from: "you".into(), ..Default::default() });
        let target = Some(target.to_string());
        r.rows.push(Row {
            name: name.to_string(),
            section: section.to_string(),
            line: i + 2,
            key: Some(code),
            modifiers: Some(code),
            target,
            from: "you".into(),
            on: true,
            ..Default::default()
        });
    }
    assert!(r.statuses().iter().all(|s| s.0 == Status::Ready), "{:?}", r.statuses());
    set(app, "rows", serde_json::to_value(&r.rows).unwrap());
    set(app, "keys", serde_json::to_value(&r.keys).unwrap());
    set(app, "mods", serde_json::to_value(&r.mods).unwrap());
    set(app, "start", json!(start));
    set(app, "remember", json!(false));
    let out = confirm(app).unwrap();
    (out, dialog(app).fields["overrides"].clone())
}

/// Cmd and Ctrl are one key off the Mac: the review lets both rows through, and the later one takes
/// the shortcut.
#[test]
fn a_later_row_takes_a_shortcut_that_reads_the_same() {
    let mut app = app();
    let rows = [("Acme Copy", "Menus", "edit.copy", "Cmd+Alt+Shift+F10"), ("Acme Undo", "Menus", "edit.undo", "Ctrl+Alt+Shift+F10")];
    let (out, ov) = apply_rows(&mut app, "defaults", &rows);
    assert_eq!(ov["edit.undo"], "Ctrl+Alt+Shift+F10");
    if craft_keymap::Platform::current() == craft_keymap::Platform::Mac {
        assert_eq!((out["applied"].clone(), ov["edit.copy"].clone()), (json!(2), json!("Cmd+Alt+Shift+F10")));
    } else {
        assert_eq!(out["applied"], 1);
        assert!(ov.get("edit.copy").is_none(), "Copy keeps its default: {ov}");
    }
}

/// A shortcut the set gives is taken from a default outside the set and from the working set's own
/// change, while the working set's other changes stay with Start: Current.
#[test]
fn the_set_takes_its_shortcuts_from_outside_it() {
    let mut app = app();
    assert_eq!(shortcut_editor::default_of("path.join"), Some("Cmd+J"));
    let rows = [("Acme Copy", "Menus › Edit", "edit.copy", "Cmd+J"), ("Acme Undo", "Menus › Edit", "edit.undo", "Cmd+Alt+Shift+9")];
    let (out, ov) = apply_rows(&mut app, "defaults", &rows);
    assert_eq!(out["applied"], 2);
    assert_eq!(ov, json!({"edit.copy": "Cmd+J", "edit.undo": "Cmd+Alt+Shift+9", "path.join": ""}));
    // From the current shortcuts: the parent dialog's working set.
    app.ui.dialog = None;
    shortcut_editor::open(&mut app);
    set(&mut app, "overrides", json!({"edit.preferences": "Cmd+Alt+Shift+9", "edit.redo": "Cmd+Alt+Shift+F11"}));
    let (_, ov) = apply_rows(&mut app, "current", &rows);
    assert_eq!(
        ov,
        json!({"edit.copy": "Cmd+J", "edit.undo": "Cmd+Alt+Shift+9", "edit.preferences": "", "edit.redo": "Cmd+Alt+Shift+F11", "path.join": ""})
    );
    // From the defaults: the working set's changes are dropped.
    set(&mut app, "overrides", json!({"edit.redo": "Cmd+Alt+Shift+F11"}));
    let (_, ov) = apply_rows(&mut app, "defaults", &rows);
    assert_eq!(ov, json!({"edit.copy": "Cmd+J", "edit.undo": "Cmd+Alt+Shift+9", "path.join": ""}));
}

#[test]
fn tool_rows_go_to_tools_and_command_rows_to_commands() {
    let mut app = app();
    let rows = [("Pen", "Tools", "tool:pen", "Shift+9"), ("Pen Tool", "Menus", "edit.copy", "Cmd+Alt+Shift+F12")];
    let (out, ov) = apply_rows(&mut app, "defaults", &rows);
    assert_eq!(out["applied"], 2);
    assert_eq!(ov, json!({"tool:pen": "Shift+9", "edit.copy": "Cmd+Alt+Shift+F12"}));
}

/// Two entries with one name in one block can be answered with different items.
#[test]
fn rows_with_one_name_keep_their_own_targets() {
    let mut app = app();
    let rows = [("Acme Twin", "Menus", "edit.copy", "Cmd+Alt+Shift+F11"), ("acme twin…", "Menus", "edit.undo", "Cmd+Alt+Shift+F12")];
    let (out, ov) = apply_rows(&mut app, "defaults", &rows);
    assert_eq!(out["applied"], 2);
    assert_eq!(ov, json!({"edit.copy": "Cmd+Alt+Shift+F11", "edit.undo": "Cmd+Alt+Shift+F12"}));
}

#[test]
fn long_names_match_and_apply() {
    let label = "Acme Long Label ".repeat(14);
    assert!(label.chars().count() > 200);
    let read = json!({"entries": [{"name": format!("{label}and more words after it"), "section": ["Menus"], "line": 2, "key": 901}]});
    let r = Review::build(
        &read,
        &[Candidate { id: "edit.copy".into(), labels: vec![label] }, Candidate { id: "edit.undo".into(), labels: vec!["Undo".into()] }],
    );
    assert_eq!(r.rows[0].target.as_deref(), Some("edit.copy"), "the first 200 characters decide");
    // Past the longest text the plan reads.
    let mut app = app();
    let name = "Acme Copy ".repeat(150);
    let (out, ov) = apply_rows(&mut app, "defaults", &[(&name, "Menus", "edit.copy", "Cmd+Alt+Shift+F11")]);
    assert_eq!((out["applied"].clone(), ov), (json!(1), json!({"edit.copy": "Cmd+Alt+Shift+F11"})));
}

/// A search in a row's target picker that matches nothing shrinks its menu to the search field
/// and "No equivalent"; clearing the search lists every entry again, and the menu grows back to
/// its height before the search (egui#5225). The search field stays where it was throughout.
#[test]
fn the_target_picker_grows_back_when_a_search_that_matched_nothing_is_cleared() {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let r = Row { name: "Acme Undo".into(), ..Default::default() };
    let frame = |events: Vec<egui::Event>| {
        let input =
            egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))), ..Default::default() };
        let mut at = egui::Pos2::ZERO;
        let mut out = ctx.run_ui(input, |ui| {
            at = ui.next_widget_position();
            target_picker(ui, 0, &r);
        });
        out.textures_delta.clear();
        let text = |label: &str| {
            out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos),
                _ => None,
            })
        };
        // The row under the search field: it shows whatever the field holds.
        let none_row = text("No equivalent");
        let menu = none_row.and_then(|p| ctx.layer_id_at(p)).and_then(|l| ctx.memory(|m| m.area_rect(l.id)));
        (at, none_row, text("Search"), menu.map(|r| r.height()))
    };
    let settle = |events: Vec<egui::Event>| {
        let mut last = frame(events);
        for _ in 0..5 {
            last = frame(vec![]);
        }
        last
    };
    let (at, ..) = frame(vec![]);
    let click = at + egui::vec2(40.0, 10.0);
    let button = |pressed| egui::Event::PointerButton { pos: click, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(vec![egui::Event::PointerMoved(click), button(true)]);
    let (_, none_row, search, full) = settle(vec![button(false)]);
    let (none_row, search, full) = (none_row.expect("the menu is open"), search.expect("the search field shows"), full.expect("the menu's area"));
    let nothing = "qqzzxxqqzz";
    let (_, n, _, small) = settle(vec![egui::Event::Text(nothing.into())]);
    assert_eq!(n, Some(none_row), "the search field stays put while nothing matches");
    let small = small.expect("the menu stays open");
    assert!(small + 40.0 < full, "the menu shrinks to its search field: {small} of {full}");
    for _ in nothing.chars() {
        frame(vec![press(egui::Key::Backspace)]);
    }
    let (_, n, s, grown) = settle(vec![]);
    assert_eq!((n, s), (Some(none_row), Some(search)), "the search field is where it was");
    assert_eq!(grown, Some(full), "the menu grows back to its height before the search");
}
