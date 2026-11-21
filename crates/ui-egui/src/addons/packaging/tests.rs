use std::sync::Arc;

use serde_json::json;
use vectorcraft_engine::Session;

use super::{CHECK, ID, SET_ROLE, follow_role, last_check, rows, run_check, select_issue};
use crate::i18n::LangExt as _;
use crate::{VectorcraftApp, menus};

/// An app with two rectangles, nothing selected → (app, their ids).
fn app() -> (VectorcraftApp, u64, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let rect = |app: &mut VectorcraftApp, x: f64| {
        app.run("shape.rectangle", json!({"x": x, "y": 20, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b) = (rect(&mut app, 20.0), rect(&mut app, 100.0));
    app.run("select.none", json!({})).unwrap();
    (app, a, b)
}

/// The count of each role the panel's rows show.
fn counts(app: &mut VectorcraftApp) -> Vec<(String, u64)> {
    rows(app).into_iter().map(|row| (row.0, row.3)).collect()
}

#[test]
fn the_rows_count_the_roles_for_the_document_then_for_the_selection() {
    let (mut app, a, b) = app();
    app.run(SET_ROLE, json!({"role": "cut", "ids": [a, b]})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let roles = ["cut", "crease", "partialCut", "perforation"];
    let expect = |cut: u64| roles.iter().map(|role| (role.to_string(), if *role == "cut" { cut } else { 0 })).collect::<Vec<_>>();
    assert_eq!(counts(&mut app), expect(2), "the document's two cut lines");
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(counts(&mut app), expect(1), "the selection's one cut line");
}

#[test]
fn role_swatches_shows_the_swatch_of_the_role_picked() {
    let (mut app, a, _) = app();
    app.run(SET_ROLE, json!({"role": "crease", "ids": [a]})).unwrap();
    app.run("swatch.edit", json!({"name": "Crease", "newName": "Score"})).unwrap();
    let mut filled_for = None;
    assert_eq!(follow_role(&mut app, "cut", &mut filled_for).as_deref(), Some("Cut"));
    assert_eq!(follow_role(&mut app, "crease", &mut filled_for).as_deref(), Some("Score"), "the Crease role's swatch");
    // The same role again: a swatch typed for it stays.
    assert_eq!(follow_role(&mut app, "crease", &mut filled_for), None);
    assert_eq!(filled_for.as_deref(), Some("crease"));
}

/// An app with a square blank cut on its outline, a crease across it and a cut line that stops
/// inside it, nothing selected → (app, the cut line that stops).
fn blank() -> (VectorcraftApp, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let path = |app: &mut VectorcraftApp, d: &str| app.run("path.create", json!({ "d": d })).unwrap()["id"].as_u64().unwrap();
    let outline = path(&mut app, "M50 50 L250 50 L250 250 L50 250 Z");
    let crease = path(&mut app, "M50 150 L250 150");
    let slit = path(&mut app, "M100 50 L100 100");
    app.run(SET_ROLE, json!({"role": "cut", "ids": [outline, slit]})).unwrap();
    app.run(SET_ROLE, json!({"role": "crease", "ids": [crease]})).unwrap();
    app.run("select.none", json!({})).unwrap();
    (app, slit)
}

#[test]
fn check_fold_lines_keeps_its_issues_for_the_panel_until_the_document_changes() {
    let (mut app, _) = blank();
    assert!(!crate::panel_docking::showing(&app.ui, ID));
    assert!(last_check(&app).is_none(), "nothing is listed before a check");
    menus::invoke(&mut app, CHECK, json!(null));
    assert!(crate::panel_docking::showing(&app.ui, ID), "the panel opens");
    let checked = last_check(&app).expect("the check's issues");
    assert_eq!((checked.issues.len(), checked.issues[0].error), (1, true), "one stopped cut line");
    assert_eq!((checked.panels, checked.hinges), (2, 1));
    assert_eq!(app.ui.status, checked.summary());
    // Any change to the document takes the list away.
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    assert!(last_check(&app).is_none());
}

#[test]
fn an_issue_selects_its_lines_and_the_check_stays() {
    let (mut app, slit) = blank();
    let checked = run_check(&mut app, json!({})).expect("a check");
    assert!(app.session.active().unwrap().selection.is_empty());
    // An issue that does not exist names no lines.
    select_issue(&mut app, &checked, checked.issues.len());
    assert!(app.session.active().unwrap().selection.is_empty());
    select_issue(&mut app, &checked, 0);
    let selected: Vec<u64> = app.session.active().unwrap().selection.objects.iter().map(|id| id.0).collect();
    assert_eq!(selected, [slit]);
    assert!(last_check(&app).is_some_and(|kept| Arc::ptr_eq(&kept, &checked)), "selecting keeps the list");
    // A clean blank: no issues.
    app.run("edit.clear", json!({})).unwrap();
    let clean = run_check(&mut app, json!({})).expect("a check");
    assert_eq!((clean.issues.len(), clean.panels, clean.hinges), (0, 2, 1));
}

#[test]
fn each_document_keeps_its_own_check() {
    let (mut app, _) = blank();
    let first = run_check(&mut app, json!({})).expect("a check");
    // A second document, checked too: it has no cut lines.
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let second = run_check(&mut app, json!({})).expect("a check");
    assert!(!Arc::ptr_eq(&first, &second));
    // Back on the first document, unchanged since its check: its issues are still listed.
    assert!(app.session.set_active(0));
    assert!(last_check(&app).is_some_and(|kept| Arc::ptr_eq(&kept, &first)));
    // And the second document's check stays its own.
    assert!(app.session.set_active(1));
    assert!(last_check(&app).is_some_and(|kept| Arc::ptr_eq(&kept, &second)));
}

/// The messages `addon.packaging.check` gives its issues, one for each code (three for `noCut`).
const ISSUE_MESSAGES: [&str; 17] = [
    "No cut lines are in the selection.",
    "No cut lines are in the document.",
    "No cut lines are among the given objects.",
    "The cut lines do not enclose an area.",
    "A cut line ends without meeting another cut line.",
    "A gap between cut lines was closed.",
    "The fold lines are too complex to check.",
    "A cut line and a crease lie on top of each other.",
    "Two fold lines lie on top of each other.",
    "A crease ends inside a panel.",
    "A crease does not divide two panels.",
    "A crease lies outside the cut outline, wholly or in part.",
    "The pieces of a hinge are not on one straight line.",
    "A gap between cut lines is too narrow to tell apart.",
    "A cut-out area holds other fold lines or another blank.",
    "A panel is not joined to the rest by a crease.",
    "A fold line is on a non-printing layer.",
];

#[test]
fn every_issue_message_translates_in_every_complete_message_catalog() {
    for code in [
        "cs", "de", "el", "es", "fr", "hi", "hu", "it", "ja", "ka", "ko", "pl", "pt-br", "ru", "sv", "uk", "vi", "zh-hans", "zh-hant", "fi", "tr",
        "ca", "nl", "da",
    ] {
        let lang = crate::i18n::Lang::from_code(code).unwrap();
        for m in ISSUE_MESSAGES.into_iter().chain(["Panels: 3, hinges: 2, errors: 1, warnings: 0"]) {
            assert_ne!(crate::i18n::message(lang, m), m, "{code}: {m}");
        }
    }
}
