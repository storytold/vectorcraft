use serde_json::json;
use vectorcraft_engine::Session;

use super::*;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app
}

fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn dialog(app: &mut VectorcraftApp) -> &mut Dialog {
    app.ui.dialog.as_mut().expect("Kinsoku Shori Settings open")
}

fn doc_sets(app: &VectorcraftApp) -> Vec<KinsokuSet> {
    app.session.active().unwrap().doc.kinsoku_sets.to_vec()
}

fn steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// Do `act` with the dialog as it is, as its button does.
fn press(app: &mut VectorcraftApp, act: Action) {
    let mut d = app.ui.dialog.clone().unwrap();
    run(app, &mut d, act).unwrap();
    app.ui.dialog = Some(d);
}

/// Type › Kinsoku Shori Settings… opens on the document's sets; New adds one based on Hard, its
/// categories edited in their fields and Add putting in a character by its code point; nothing
/// changes until OK, which sets them as one undo step; Cancel leaves them.
#[test]
fn kinsoku_sets_change_on_ok_as_one_undo_step() {
    let mut app = app();
    crate::menus::invoke(&mut app, COMMAND, json!({}));
    assert_eq!(dialog(&mut app).kind, KIND);
    frame(&mut app);
    press(&mut app, Action::New);
    frame(&mut app);
    let d = dialog(&mut app);
    assert_eq!(d.str("name"), "Kinsoku Set 1");
    assert!(d.str("cannotStart").contains('ー'), "based on Hard");
    d.fields.insert("name".into(), json!("UI Tight"));
    d.fields.insert("cannotEnd".into(), json!("「 （"));
    d.fields.insert("inseparable".into(), json!("あい  ——"));
    d.fields.insert("code".into(), json!("U+FF5D"));
    d.fields.insert("into".into(), json!("cannotEnd"));
    frame(&mut app);
    press(&mut app, Action::Add);
    assert_eq!(dialog(&mut app).str("cannotEnd"), "「 （｝");
    frame(&mut app);
    assert!(doc_sets(&app).is_empty());
    let before = steps(&app);
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let sets = doc_sets(&app);
    assert_eq!(sets.len(), 1);
    assert_eq!((sets[0].name.as_str(), sets[0].cannot_end.clone()), ("UI Tight", vec!['「', '（', '｝']));
    assert_eq!(sets[0].inseparable, ["——", "あい"]);
    assert_eq!(steps(&app), before + 1);
    // OK with nothing changed adds no step; Cancel leaves the sets.
    open(&mut app).unwrap();
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert_eq!(steps(&app), before + 1);
    open(&mut app).unwrap();
    press(&mut app, Action::Delete);
    super::super::cancel(&mut app);
    assert_eq!(doc_sets(&app).len(), 1);
    // Renamed in the dialog, the set keeps its paragraphs; Delete and OK takes it out.
    let id = app.run("text.create", json!({"x": 10, "y": 50, "text": "あいう"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("text.setFormat", json!({"kinsoku": "UI Tight"})).unwrap();
    open(&mut app).unwrap();
    dialog(&mut app).fields.insert("name".into(), json!("UI Strict"));
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    let para = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => t.para_at(0).clone(),
        _ => panic!("not text"),
    };
    assert_eq!(para(&app).kinsoku_name(), "UI Strict");
    open(&mut app).unwrap();
    press(&mut app, Action::Delete);
    assert_eq!(dialog(&mut app).str("name"), "");
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert!(doc_sets(&app).is_empty());
    assert_eq!(para(&app).kinsoku_name(), "Hard");
}

/// New is based on the choice in Based on, the dialog's own sets included, and takes the next free
/// name; a name two sets share, a built-in name or one left empty keeps the dialog open and changes
/// nothing.
#[test]
fn shared_reserved_or_empty_names_keep_the_dialog_open() {
    let mut app = app();
    open(&mut app).unwrap();
    dialog(&mut app).fields.insert("base".into(), json!("none"));
    press(&mut app, Action::New);
    assert_eq!(dialog(&mut app).str("cannotStart"), "");
    dialog(&mut app).fields.insert("cannotStart".into(), json!("ゝ"));
    frame(&mut app);
    dialog(&mut app).fields.insert("base".into(), json!("Kinsoku Set 1"));
    press(&mut app, Action::New);
    let d = dialog(&mut app);
    assert_eq!((d.str("name"), d.str("cannotStart")), ("Kinsoku Set 2".to_string(), "ゝ".to_string()));
    for bad in ["kinsoku set 1", "  ", "Soft"] {
        dialog(&mut app).fields.insert("name".into(), json!(bad));
        frame(&mut app);
        assert!(super::super::confirm(&mut app).is_err(), "{bad:?}");
        assert!(app.ui.dialog.is_some());
        assert!(doc_sets(&app).is_empty());
    }
    dialog(&mut app).fields.insert("name".into(), json!("UI Loose"));
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert_eq!(doc_sets(&app).iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), ["Kinsoku Set 1", "UI Loose"]);
}

/// Based on follows a set of the dialog through a rename, so New starts from that set's
/// characters as edited, not from the document's copy under its old name.
#[test]
fn based_on_follows_a_renamed_set() {
    let mut app = app();
    app.run("type.kinsokuSets.set", json!({"sets": [{"name": "Doc Set", "cannotStart": "ゝ"}]})).unwrap();
    open(&mut app).unwrap();
    dialog(&mut app).fields.insert("base".into(), json!("Doc Set"));
    frame(&mut app);
    let d = dialog(&mut app);
    d.fields.insert("cannotStart".into(), json!("ゞ"));
    d.fields.insert("name".into(), json!("Renamed"));
    frame(&mut app);
    assert_eq!(dialog(&mut app).str("base"), "Renamed");
    press(&mut app, Action::New);
    let d = dialog(&mut app);
    assert_eq!((d.str("name"), d.str("cannotStart")), ("Kinsoku Set 1".to_string(), "ゞ".to_string()));
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

/// The menu item shows only with Show East Asian Options on, and needs a document.
#[test]
fn the_menu_item_shows_with_east_asian_options() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.show_east_asian_options = true;
    assert_eq!(crate::menus::shown_state(&app, COMMAND, &json!({})).map(|s| s.0), Some(false));
    app.run("file.new", json!({})).unwrap();
    assert_eq!(crate::menus::shown_state(&app, COMMAND, &json!({})).map(|s| s.0), Some(true));
    app.session.prefs.show_east_asian_options = false;
    assert_eq!(crate::menus::shown_state(&app, COMMAND, &json!({})), None);
}
