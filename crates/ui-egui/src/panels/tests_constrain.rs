//! The W/H link (Constrain Width and Height Proportions) in the Transform panel, the Properties
//! panel's Transform section and the Control bar: one preference, passed on as `proportional`.
//! Alt+Enter in their fields applies the value to a copy of the selection, passed on as `copy`, and
//! Ctrl/Cmd+Enter in W or H keeps the proportions whether the link is on or off.

use egui::Modifiers;
use serde_json::json;

use super::tests_appearance::run;
use super::*;
use crate::tests::panel_harness::Panel;

/// A selected 100 × 50 pt rectangle.
fn rect_app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 400, "height": 400}));
    let id = run(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 50}))["id"].clone();
    run(&mut app, "select.set", json!({ "ids": [id] }));
    app
}

fn size(app: &VectorcraftApp) -> (f64, f64) {
    let st = app.session.active().unwrap();
    let b = st.doc.bounds_of(&st.selection.objects, false).unwrap();
    (b.width().round(), b.height().round())
}

/// The paths of the document, bottom to top: their size and angle (rounded), and whether each is
/// selected.
fn paths(app: &VectorcraftApp) -> Vec<((f64, f64), f64, bool)> {
    let st = app.session.active().unwrap();
    let mut out = vec![];
    st.doc.walk(|n| {
        if matches!(n.kind, vectorcraft_doc::NodeKind::Path { .. }) {
            let b = st.doc.oriented_bounds(&[n.id], false).unwrap();
            out.push(((b.rect.width().round(), b.rect.height().round()), b.angle.round(), st.selection.objects.contains(&n.id)));
        }
    });
    out
}

/// One Enter with modifiers in a field: Alt makes a copy unless anchors are selected (they move),
/// and Ctrl/Cmd scales proportionally in W and H only.
#[test]
fn enter_modifiers_set_copy_and_proportional() {
    // (case, field, typed, modifiers, anchors selected, a copy is made, selection size, angle of the last path)
    let rows = [
        ("Alt+Enter on W", "Width", "200", Modifiers::ALT, false, true, Some((200.0, 50.0)), 0.0),
        ("Alt+Enter on X", "X position", "150", Modifiers::ALT, false, true, Some((100.0, 50.0)), 0.0),
        ("Alt+Enter on Rotate", "Rotate", "45", Modifiers::ALT, false, true, None, 45.0),
        ("Alt+Enter on X, anchors", "X position", "150", Modifiers::ALT, true, false, Some((140.0, 50.0)), 0.0),
        ("Cmd+Enter on W", "Width", "200", Modifiers::COMMAND, false, false, Some((200.0, 100.0)), 0.0),
        ("Cmd+Enter on X", "X position", "200", Modifiers::COMMAND, false, false, Some((100.0, 50.0)), 0.0),
    ];
    for (case, field, text, mods, anchors, copy, want_size, angle) in rows {
        let mut app = rect_app();
        if anchors {
            let id = app.session.active().unwrap().selection.objects[0].0;
            run(&mut app, "select.anchors", json!({"id": id, "anchors": [[0, 1]]}));
        }
        let undo = app.session.doc().unwrap().history.undo.len();
        let mut p = Panel::new(app, transform::show);
        p.type_into_with(field, text, mods);
        let app = p.app();
        let all = paths(&app);
        assert_eq!(all.len(), 1 + usize::from(copy), "{case}: copy");
        if let Some(s) = want_size {
            assert_eq!(size(&app), s, "{case}: size");
        }
        assert_eq!(all.last().map(|a| a.1), Some(angle), "{case}: angle");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "{case}: one undo step");
        if copy {
            assert_eq!(all.last().map(|a| a.2), Some(true), "{case}: the copy is selected");
        }
    }
    // The Artboard tool's W: Cmd+Enter keeps the artboard's proportions.
    let board = |mods| {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 200, "height": 150}));
        app.run("tool.select", json!({"tool": "artboard"})).unwrap();
        // With the Artboard tool the panel shows the artboard's fields.
        let mut p = Panel::new(app, transform::show);
        p.type_into_with("Width", "400", mods);
        let r = p.app().session.active().unwrap().doc.artboards[0].rect;
        (r.width().round(), r.height().round())
    };
    assert_eq!(board(Modifiers::NONE), (400.0, 150.0));
    assert_eq!(board(Modifiers::COMMAND), (400.0, 300.0));
}
