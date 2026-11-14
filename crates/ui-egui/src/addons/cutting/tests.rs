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

/// Object › Plug-ins › Cutting Tools › Weed Box and Lines… opens the command dialog on its
/// defaults with Preview off. With Preview on the canvas shows the weed box and lines while the
/// dialog is open; Cancel takes them away with no step of history, and OK keeps them as one step.
#[test]
fn weed_box_and_lines_previews_cancels_and_runs_on_ok() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node_count();
    let (start, n, nodes) = (app.session.doc().unwrap().doc.clone(), steps(&app), count(&app));
    let open = |app: &mut VectorcraftApp| {
        let entries = menus::menu_entries(app);
        let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Cutting Tools"] && e.label == "Weed Box and Lines…").unwrap();
        assert!(item.enabled);
        let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
        menus::invoke(app, &cmd, p);
    };

    open(&mut app);
    let d = app.ui.dialog.as_ref().expect("a dialog opens");
    assert_eq!((d.str("__command"), d.str("__label")), (super::WEED.to_string(), "Weed Box and Lines".to_string()));
    assert_eq!((d.str("direction"), d.bool("box"), d.bool("preview")), ("both".to_string(), true, false));
    let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for s in ["Weed Box and Lines", "Margin", "Box", "Direction", "Both", "Count", "Spacing", "Gap", "Weight", "Swatch", "Preview"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    // Every field label is translated.
    crate::i18n::set_current(crate::i18n::Lang::from_code("de").expect("de registered"));
    let german = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    crate::i18n::set_current(crate::i18n::Lang::EN);
    for s in ["Rand:", "Rahmen:", "Richtung:", "Farbfeld:"] {
        assert!(german.contains(s), "{s} in {german}");
    }
    assert_eq!(count(&app), nodes, "no preview without the checkbox");

    // Preview on: the group with the box and four lines is on the canvas.
    crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "preview".into(), json!(true));
    painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert_eq!(count(&app), nodes + 6, "previewed: a group, the box and four lines");

    // Cancel: as it was, with no step of history.
    crate::dialogs::cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, start, "Cancel takes the preview away");
    assert_eq!(steps(&app), n, "and adds no undo step");

    // OK: one step.
    open(&mut app);
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(steps(&app), n + 1, "one undo step");
    assert_eq!(count(&app), nodes + 6);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, start);

    // A Spacing above 0 is used instead of the Count: the 68 pt wide box in strips at most 20 pt
    // wide takes three vertical lines, each cut in two by the rectangle.
    app.run("select.set", json!({"ids": [id]})).unwrap();
    open(&mut app);
    let d = app.ui.dialog.as_mut().unwrap();
    crate::dialogs::set_field(d, "direction".into(), json!("vertical"));
    crate::dialogs::set_field(d, "spacing".into(), json!(20));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(count(&app), nodes + 8, "a group, the box and six pieces of line");

    // A Spacing below 0 is refused, not swapped for the Count.
    app.run("edit.undo", json!({})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    open(&mut app);
    let n = steps(&app);
    crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "spacing".into(), json!(-5));
    let e = crate::dialogs::confirm(&mut app).unwrap_err().to_string();
    assert!(e.contains("`spacing` must be more than 0"), "{e}");
    assert_eq!(steps(&app), n, "no undo step");

    // A Spacing with no value, or of 0 written with its unit, leaves the Count in use: a Count of 0
    // draws the box alone.
    crate::dialogs::cancel(&mut app);
    for spacing in [serde_json::Value::Null, json!("0 pt")] {
        app.run("select.set", json!({"ids": [id]})).unwrap();
        open(&mut app);
        let d = app.ui.dialog.as_mut().unwrap();
        crate::dialogs::set_field(d, "spacing".into(), spacing.clone());
        crate::dialogs::set_field(d, "count".into(), json!(0));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(count(&app), nodes + 2, "{spacing}: a group and the box");
        app.run("edit.undo", json!({})).unwrap();
    }
}

/// Object › Plug-ins › Cutting Tools › Banner Hems and Grommets… opens the command dialog on its
/// defaults with Preview off, previews the marks, takes them away on Cancel and keeps them as one
/// step on OK. With nothing selected it marks the artboard.
#[test]
fn banner_hems_and_grommets_previews_cancels_and_runs_on_ok() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node_count();
    let (start, n, nodes) = (app.session.doc().unwrap().doc.clone(), steps(&app), count(&app));
    let open = |app: &mut VectorcraftApp| {
        let entries = menus::menu_entries(app);
        let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Cutting Tools"] && e.label == "Banner Hems and Grommets…").unwrap();
        assert!(item.enabled);
        let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
        menus::invoke(app, &cmd, p);
    };

    open(&mut app);
    let d = app.ui.dialog.as_ref().expect("a dialog opens");
    assert_eq!((d.str("__command"), d.str("__label")), (super::BANNER.to_string(), "Banner Hems and Grommets".to_string()));
    assert_eq!((d.bool("grommets"), d.bool("top"), d.bool("preview")), (true, true, false));
    let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for s in [
        "Banner Hems and Grommets",
        "Hem",
        "Top",
        "Right",
        "Bottom",
        "Left",
        "Grommets",
        "Spacing",
        "Grommet Inset",
        "Diameter",
        "Weight",
        "Swatch",
        "Preview",
    ] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    crate::i18n::set_current(crate::i18n::Lang::from_code("de").expect("de registered"));
    let german = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    crate::i18n::set_current(crate::i18n::Lang::EN);
    for s in ["Bannersäume und Ösen", "Saum:", "Ösen:", "Randabstand der Ösen:", "Durchmesser:"] {
        assert!(german.contains(s), "{s} in {german}");
    }
    assert_eq!(count(&app), nodes, "no preview without the checkbox");

    // 600 × 400 pt with grommets at most 1728 pt apart: one at each corner. The group holds four
    // hem lines and four grommets of three paths each.
    let added = 1 + 4 + 4 * 4;
    crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "preview".into(), json!(true));
    painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert_eq!(count(&app), nodes + added, "previewed");
    crate::dialogs::cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, start, "Cancel takes the preview away");
    assert_eq!(steps(&app), n, "and adds no undo step");

    open(&mut app);
    crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "spacing".into(), json!("100 pt"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(steps(&app), n + 1, "one undo step");
    // 528 pt along the top and bottom need 6 gaps, 328 pt down the sides 4: 20 grommets.
    assert_eq!(count(&app), nodes + 1 + 4 + 20 * 4);
}
