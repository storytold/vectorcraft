use serde_json::json;
use vectorcraft_engine::Session;

use super::*;

fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app
}

fn doc_sets(app: &VectorcraftApp) -> Vec<KinsokuSet> {
    app.session.active().unwrap().doc.kinsoku_sets.to_vec()
}

fn steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// Runs `f` on the open dialog.
fn dialog<R>(app: &VectorcraftApp, f: impl FnOnce(&mut KinsokuDialog) -> R) -> R {
    app.dialogs().unwrap().with(|h| f(h.get_mut::<KinsokuDialog>().expect("Kinsoku Shori Settings open")))
}

fn is_open(app: &VectorcraftApp) -> bool {
    app.dialogs().unwrap().with(|h| h.get::<KinsokuDialog>().is_some())
}

/// Sets a field as automation does.
fn set(app: &VectorcraftApp, field: &str, value: Value) {
    dialog(app, |d| d.set_json_field(&crate::testing::ui_context(), field, value)).unwrap();
}

/// The picked set's name, as the dialog lists it.
fn picked(app: &VectorcraftApp) -> String {
    dialog(app, |d| d.json_fields().unwrap()["name"].as_str().unwrap().to_owned())
}

/// OK as the button does it, then the frames whose pump runs the command and whose next pass closes an accepted OK. A
/// second OK in between would file the answer again, which a refused name's next OK then waits behind.
fn pump(app: &mut VectorcraftApp) {
    crate::dialogs::tests::confirm(app);
}

fn cancel(app: &VectorcraftApp) {
    let ctx = app.ui_ctx.clone().unwrap();
    app.dialogs().unwrap().with(|h| h.cancel::<KinsokuDialog>(&ctx));
}

/// Type › Kinsoku Shori Settings… opens on the document's sets; New adds one based on Hard, its
/// categories edited in their fields and Add putting in a character by its code point; nothing
/// changes until OK, which sets them as one undo step; Cancel leaves them.
#[test]
fn kinsoku_sets_change_on_ok_as_one_undo_step() {
    let mut app = app();
    crate::menus::invoke(&mut app, COMMAND, json!({}));
    assert!(is_open(&app));
    dialog(&app, |d| d.create("Kinsoku Set 1")).unwrap();
    assert_eq!(picked(&app), "Kinsoku Set 1");
    assert!(dialog(&app, |d| d.cats[0].contains('ー')), "based on Hard");
    set(&app, "name", json!("UI Tight"));
    set(&app, "cannotEnd", json!("「 （"));
    set(&app, "inseparable", json!("あい  ——"));
    set(&app, "code", json!("U+FF5D"));
    set(&app, "into", json!("cannotEnd"));
    dialog(&app, |d| d.add_code()).unwrap();
    assert_eq!(dialog(&app, |d| d.cats[1].clone()), "「 （｝");
    assert!(doc_sets(&app).is_empty());
    let before = steps(&app);
    pump(&mut app);
    assert!(!is_open(&app));
    let sets = doc_sets(&app);
    assert_eq!(sets.len(), 1);
    assert_eq!((sets[0].name.as_str(), sets[0].cannot_end.clone()), ("UI Tight", vec!['「', '（', '｝']));
    assert_eq!(sets[0].inseparable, ["——", "あい"]);
    assert_eq!(steps(&app), before + 1);
    // OK with nothing changed adds no step; Cancel leaves the sets.
    open(&mut app).unwrap();
    pump(&mut app);
    assert_eq!(steps(&app), before + 1);
    open(&mut app).unwrap();
    dialog(&app, |d| d.delete("UI Tight")).unwrap();
    cancel(&app);
    assert_eq!(doc_sets(&app).len(), 1);
    // Renamed in the dialog, the set keeps its paragraphs; Delete and OK takes it out.
    let id = app.run("text.create", json!({"x": 10, "y": 50, "text": "あいう"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("text.setFormat", json!({"kinsoku": "UI Tight"})).unwrap();
    open(&mut app).unwrap();
    set(&app, "name", json!("UI Strict"));
    pump(&mut app);
    let para = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => t.para_at(0).clone(),
        _ => panic!("not text"),
    };
    assert_eq!(para(&app).kinsoku_name(), "UI Strict");
    open(&mut app).unwrap();
    dialog(&app, |d| d.delete("UI Strict")).unwrap();
    assert_eq!(picked(&app), "");
    pump(&mut app);
    assert!(doc_sets(&app).is_empty());
    assert_eq!(para(&app).kinsoku_name(), "Hard");
}

/// New is based on the choice in Based on, the dialog's own sets included, and takes the next free
/// name; a name two sets share, a built-in name or one left empty is refused in the name prompt, and
/// one that reaches OK keeps the dialog open and changes nothing.
#[test]
fn shared_reserved_or_empty_names_keep_the_dialog_open() {
    let mut app = app();
    open(&mut app).unwrap();
    set(&app, "base", json!("none"));
    dialog(&app, |d| d.create("Kinsoku Set 1")).unwrap();
    assert_eq!(dialog(&app, |d| d.cats[0].clone()), "");
    set(&app, "cannotStart", json!("ゝ"));
    set(&app, "base", json!("Kinsoku Set 1"));
    let next = dialog(&app, |d| new_name(&d.list(), &d.opened));
    dialog(&app, |d| d.create(&next)).unwrap();
    assert_eq!((picked(&app), dialog(&app, |d| d.cats[0].clone())), ("Kinsoku Set 2".to_string(), "ゝ".to_string()));
    for bad in ["kinsoku set 1", "  ", "Soft"] {
        assert!(dialog(&app, |d| name_problem(&d.list(), bad)).is_some(), "{bad:?}");
        set(&app, "name", json!(bad));
        pump(&mut app);
        assert!(is_open(&app), "{bad:?}");
        assert!(doc_sets(&app).is_empty());
    }
    set(&app, "name", json!("UI Loose"));
    pump(&mut app);
    assert!(!is_open(&app));
    assert_eq!(doc_sets(&app).iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), ["Kinsoku Set 1", "UI Loose"]);
}

/// Based on follows a set of the dialog through a rename, so New starts from that set's
/// characters as edited, not from the document's copy under its old name.
#[test]
fn based_on_follows_a_renamed_set() {
    let mut app = app();
    app.run("type.kinsokuSets.set", json!({"sets": [{"name": "Doc Set", "cannotStart": "ゝ"}]})).unwrap();
    open(&mut app).unwrap();
    set(&app, "base", json!("Doc Set"));
    set(&app, "cannotStart", json!("ゞ"));
    set(&app, "name", json!("Renamed"));
    assert_eq!(dialog(&app, |d| d.base.clone()), "Renamed");
    dialog(&app, |d| d.create("Kinsoku Set 1")).unwrap();
    assert_eq!((picked(&app), dialog(&app, |d| d.cats[0].clone())), ("Kinsoku Set 1".to_string(), "ゞ".to_string()));
}

/// Delete then New does not give the new set the deleted set's name, so on OK the deleted set's
/// paragraphs take Hard rather than the new set's characters.
#[test]
fn new_after_delete_does_not_take_the_deleted_name() {
    let mut app = app();
    app.run("type.kinsokuSets.set", json!({"sets": [{"name": "Kinsoku Set 1", "cannotStart": "ゝ"}]})).unwrap();
    let id = app.run("text.create", json!({"x": 10, "y": 50, "text": "あいう"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("text.setFormat", json!({"kinsoku": "Kinsoku Set 1"})).unwrap();
    open(&mut app).unwrap();
    dialog(&app, |d| d.delete("Kinsoku Set 1")).unwrap();
    set(&app, "base", json!("none"));
    let next = dialog(&app, |d| new_name(&d.list(), &d.opened));
    assert_eq!(next, "Kinsoku Set 2");
    dialog(&app, |d| d.create(&next)).unwrap();
    pump(&mut app);
    assert_eq!(doc_sets(&app).iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), ["Kinsoku Set 2"]);
    let para = match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => t.para_at(0).clone(),
        _ => panic!("not text"),
    };
    assert_eq!(para.kinsoku_name(), "Hard");
}

/// OK sets the sets of the document the dialog opened on, also when another document became the
/// active one while it was open; with that document closed, OK closes the dialog and changes no
/// document's sets.
#[test]
fn ok_sets_the_document_the_dialog_opened_on() {
    let mut app = app();
    let first = app.session.active().unwrap().uid;
    app.run("type.kinsokuSets.set", json!({ "sets": [{"name": "First", "cannotStart": "ゝ"}] })).unwrap();
    open(&mut app).unwrap();
    assert!(dialog(&app, |d| d.set_json_field(&crate::testing::ui_context(), "__doc", json!(1))).is_err(), "automation cannot change the document");
    set(&app, "name", json!("Soft"));
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let other = app.session.active().unwrap().uid;
    assert_ne!(other, first);
    // A name OK refuses keeps the dialog open on its document, now the active one.
    pump(&mut app);
    assert!(is_open(&app));
    assert_eq!(app.session.active().unwrap().uid, first);
    assert_eq!(doc_sets(&app)[0].name, "First");
    app.run("document.activate", json!({ "index": app.session.documents().iter().position(|d| d.uid == other).unwrap() })).unwrap();
    set(&app, "name", json!("Kinsoku Set 1"));
    pump(&mut app);
    assert!(!is_open(&app));
    assert_eq!(app.session.active().unwrap().uid, first);
    assert_eq!(doc_sets(&app)[0].name, "Kinsoku Set 1");
    let second = app.session.documents().iter().find(|d| d.uid != first).unwrap();
    assert!(second.doc.kinsoku_sets.is_empty(), "the other document keeps its sets");

    // New skips the names of the dialog's document's sets, not of the active document's.
    open(&mut app).unwrap();
    dialog(&app, |d| d.delete("Kinsoku Set 1")).unwrap();
    app.run("document.activate", json!({ "index": app.session.documents().iter().position(|d| d.uid == other).unwrap() })).unwrap();
    let next = dialog(&app, |d| new_name(&d.list(), &d.opened));
    assert_eq!(next, "Kinsoku Set 2");
    dialog(&app, |d| d.create(&next)).unwrap();
    let i = app.session.documents().iter().position(|d| d.uid == first).unwrap();
    app.session.execute("file.close", &json!({ "index": i })).unwrap();
    pump(&mut app);
    assert!(!is_open(&app));
    assert!(app.session.documents().iter().all(|d| d.doc.kinsoku_sets.is_empty()));
}

/// OK keeps a set the document gained while the dialog was open, after the dialog's sets, and a
/// set deleted in the dialog stays deleted. A `sets` field that is not a list of sets keeps the
/// dialog open and leaves the document's sets as they are, and New refuses to start from it.
#[test]
fn ok_keeps_sets_added_while_open_and_refuses_malformed_sets() {
    let mut app = app();
    app.run("type.kinsokuSets.set", json!({ "sets": [{"name": "First", "cannotStart": "ゝ"}] })).unwrap();
    open(&mut app).unwrap();
    let both = json!([{"name": "First", "cannotStart": "ゝ"}, {"name": "Later", "cannotStart": "ー"}]);
    app.run("type.kinsokuSets.set", json!({ "sets": both })).unwrap();
    set(&app, "cannotStart", json!("ゞ"));
    pump(&mut app);
    let sets = doc_sets(&app);
    let named = sets.iter().map(|k| (k.name.as_str(), k.cannot_start.clone())).collect::<Vec<_>>();
    assert_eq!(named, [("First", vec!['ゞ']), ("Later", vec!['ー'])]);
    open(&mut app).unwrap();
    dialog(&app, |d| d.delete("First")).unwrap();
    pump(&mut app);
    let names = |app: &VectorcraftApp| doc_sets(app).iter().map(|k| k.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&app), ["Later"]);
    for bad in [Value::Null, json!({}), json!("Later"), json!([1]), json!([{"name": "Later", "cannotStart": "ー"}, 1])] {
        open(&mut app).unwrap();
        set(&app, "sets", bad.clone());
        assert!(dialog(&app, |d| d.create("New one")).is_err(), "{bad}");
        assert_eq!(dialog(&app, |d| d.sets.clone()), bad, "New keeps {bad} as it is");
        pump(&mut app);
        assert!(is_open(&app), "{bad}");
        assert_eq!(names(&app), ["Later"], "{bad}");
    }
}

/// Add takes a code point as U+XXXX, u+xxxx or bare hexadecimal, and never a control or space
/// character or a value that is no character.
#[test]
fn add_reads_unicode_code_points() {
    assert_eq!(code_char("U+3001"), Some('、'));
    assert_eq!(code_char(" u+ff5d "), Some('｝'));
    assert_eq!(code_char("300c"), Some('「'));
    for bad in ["", "U+", "U+0007", "U+0020", "U+D800", "U+110000", "zz"] {
        assert_eq!(code_char(bad), None, "{bad:?}");
    }
}

/// The East Asian menu items show only with Show East Asian Options on, and need a document.
#[test]
fn east_asian_menu_rows_show_with_the_option() {
    for id in [crate::dialogs::composite_fonts::COMMAND, COMMAND] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.prefs.show_east_asian_options = true;
        assert_eq!(crate::menus::shown_state(&app, id, &json!({})).map(|s| s.0), Some(false), "{id}");
        app.run("file.new", json!({})).unwrap();
        assert_eq!(crate::menus::shown_state(&app, id, &json!({})).map(|s| s.0), Some(true), "{id}");
        app.session.prefs.show_east_asian_options = false;
        assert_eq!(crate::menus::shown_state(&app, id, &json!({})), None, "{id}");
    }
}
