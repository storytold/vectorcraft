use serde_json::json;
use vectorcraft_engine::Session;

use super::{ID, STATE, check, fix, show};
use crate::panels::set_pstate;
use crate::tests_labels::painted_text;
use crate::{VectorcraftApp, menus};

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
fn the_plugins_submenu_opens_the_panel_and_lists_the_tools() {
    let mut app = app();
    let entries = menus::menu_entries(&app);
    let under = |label: &str| entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Artwork Cleanup"] && e.label == label);
    let item = under("Document Health Check").expect("listed");
    assert_eq!((item.command.as_deref(), &item.params), (Some("window.panel"), &json!({ "panel": ID })));
    for (label, id) in [
        ("Simplify Anchors (Keep Shape)…", "addon.cleanup.simplify"),
        ("Rejoin Path Pieces…", "addon.cleanup.rejoin"),
        ("Deep Ungroup…", "addon.cleanup.deepUngroup"),
        ("Fix All Issues", "addon.cleanup.fix"),
    ] {
        assert_eq!(under(label).and_then(|e| e.command.as_deref()), Some(id), "{label}");
    }
    // Install Plug-in… still follows.
    assert!(entries.iter().any(|e| e.path == ["Object", "Plug-ins"] && e.label == "Install Plug-in…"));
    assert!(!crate::panel_docking::showing(&app.ui, ID));
    menus::invoke(&mut app, "window.panel", item.params.clone());
    assert!(crate::panel_docking::showing(&app.ui, ID), "the panel opens");
    // The tools open their options first, as dialogs.
    let (target, p) = menus::click_target("Deep Ungroup…", "addon.cleanup.deepUngroup", &json!({"includeClipGroups": false}));
    assert_eq!((target.as_str(), &p["command"]), ("ui.paramDialog", &json!("addon.cleanup.deepUngroup")));
}

#[test]
fn check_lists_what_it_found_and_fix_clears_it() {
    let mut app = app();
    let text = painted_text(&mut app, show);
    assert!(text.contains("Check Document") && !text.contains("Stray Points"), "{text}");
    let r = check(&mut app, None);
    assert_eq!(r.scope, "document");
    assert_eq!(r.rows, [("strayPoints".to_string(), "Stray Points".to_string(), 1)]);
    let shown = r.clone();
    let text = painted_text(&mut app, |app, ui| {
        set_pstate(ui.ctx(), STATE, shown.clone());
        show(app, ui);
    });
    assert!(text.contains("Stray Points") && text.contains("Fix Selected") && text.contains("Fix All"), "{text}");
    let steps = app.session.doc().unwrap().history.undo.len();
    let after = fix(&mut app, &r, vec!["strayPoints".into()]);
    assert!(after.rows.is_empty() && after.status.contains('1'), "{}", after.status);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
    let text = painted_text(&mut app, |app, ui| {
        set_pstate(ui.ctx(), STATE, after.clone());
        show(app, ui);
    });
    assert!(text.contains("No issues found."), "{text}");
}
