use serde_json::json;
use vectorcraft_engine::Session;

use crate::tests_labels::painted_text;
use crate::{VectorcraftApp, menus};

/// Object › Plug-ins › Cutting Tools › Stencil Bridges… opens the command dialog on a 2 pt width, one
/// bridge per island, vertical bridges and Keep Original on; the direction is a list of its three
/// ways; OK cuts the bridges as one undo step, and a message shows when there are no islands.
#[test]
fn stencil_bridges_opens_its_dialog_and_runs_on_ok() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let outer = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
    let inner = app.run("shape.rectangle", json!({"x": 120, "y": 120, "width": 60, "height": 60})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [outer, inner]})).unwrap();
    let o = app.run("object.pathfinder.minusFront", json!({})).unwrap()["ids"][0].clone();
    app.run("select.set", json!({"ids": [o]})).unwrap();
    let steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let n = steps(&app);
    let open = |app: &mut VectorcraftApp| {
        let entries = menus::menu_entries(app);
        let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Cutting Tools"] && e.label == "Stencil Bridges…").expect("listed");
        assert!(item.enabled);
        let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
        menus::invoke(app, &cmd, p);
    };

    open(&mut app);
    let d = app.ui.dialog.as_ref().expect("a dialog opens");
    assert_eq!((d.str("__command"), d.str("__label")), (super::BRIDGES.to_string(), "Stencil Bridges".to_string()));
    assert_eq!((d.f64("width", 0.0), d.f64("count", 0.0), d.str("direction"), d.bool("keepOriginal")), (2.0, 1.0, "vertical".to_string(), true));
    let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for s in ["Stencil Bridges", "Width:", "Count:", "Direction:", "Vertical", "Keep Original"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    // Each way the list offers is one the command takes.
    for (_, way) in super::BRIDGE_DIRECTIONS {
        assert!(app.session.execute(super::BRIDGES, &json!({"direction": way, "width": 1000})).is_ok(), "{way}");
    }
    assert_eq!(steps(&app), n, "no bridge fits that wide: nothing changed");

    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(steps(&app), n + 1, "one undo step");
    let layers = &app.session.doc().unwrap().doc.layers;
    assert!(layers.iter().any(|l| l.name.as_deref() == Some("Stencil Original") && !l.visible), "the original is on a hidden layer");

    // Run again on the result, which has no islands left: a message, and nothing changes.
    open(&mut app);
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(steps(&app), n + 1);
    assert!(app.ui.status.contains("no islands"), "{}", app.ui.status);
}
