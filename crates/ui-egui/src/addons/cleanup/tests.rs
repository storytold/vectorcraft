use serde_json::json;
use vectorcraft_engine::Session;

use super::{check, current, fix};
use crate::VectorcraftApp;

/// An app with a rectangle and a stray point (a path of one anchor), nothing selected.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    app.run("path.create", json!({"anchors": [{"x": 80, "y": 80}]})).unwrap();
    app.run("select.none", json!({})).unwrap();
    app
}

#[test]
fn check_lists_what_it_found_and_fix_clears_it() {
    let mut app = app();
    assert!(app.addon_panels.cleanup.is_none(), "nothing is listed before a check");
    check(&mut app, None);
    let uid = app.session.active().unwrap().uid;
    let r = app.addon_panels.cleanup.clone().expect("the check's results");
    assert_eq!(r.scope, "document");
    assert_eq!((r.doc, r.rows.as_slice()), (uid, &[("strayPoints".to_string(), "Stray Points".to_string(), 1)][..]));

    // Another document is active: the results are not its own, and come back with the first.
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    assert!(current(&app).is_none(), "another document's results are not shown");
    assert!(app.session.set_active(0));
    assert_eq!(current(&app).map(|r| r.doc), Some(uid), "the results stay with their document");

    let steps = app.session.doc().unwrap().history.undo.len();
    fix(&mut app, vec!["strayPoints".into()]);
    let after = app.addon_panels.cleanup.clone().expect("the check after the fix");
    assert!(after.rows.is_empty() && after.status.contains('1'), "{}", after.status);
    assert_eq!(after.doc, uid);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
}

#[test]
fn a_fix_with_no_results_does_nothing() {
    let mut app = app();
    let steps = app.session.doc().unwrap().history.undo.len();
    fix(&mut app, vec!["strayPoints".into()]);
    assert!(app.addon_panels.cleanup.is_none());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), steps, "nothing was fixed without a check");
}
