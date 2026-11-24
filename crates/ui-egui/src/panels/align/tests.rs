use super::*;
use crate::tests::panel_harness::Panel;

#[test]
fn single_object_aligns_to_artboard() {
    let p = align_params(json!({"horizontal": "left"}), AlignTo::Selection, 1);
    assert_eq!(p["to"], "artboard");
    let p = align_params(json!({"vertical": "top"}), AlignTo::Selection, 3);
    assert_eq!(p["to"], "selection");
    // One object and a key: the key is a guide (#2143), which it aligns to.
    assert_eq!(align_params(json!({}), AlignTo::Key, 1)["to"], "key");
}

/// #2143: with one object and a guide selected, Align to Key Object is on and makes the guide
/// the key; Align then lines the object up on it rather than on the artboard.
#[test]
fn align_to_key_object_makes_a_selected_guide_the_key() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
    app.run("guide.add", json!({"vertical": true, "pos": 120})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    app.run("guide.select", json!({"indexes": [0], "toggle": true})).unwrap();
    let mut panel = Panel::new(app, show);
    panel.click("Align to Key Object");
    {
        let app = panel.app();
        let st = app.session.active().unwrap();
        let key = st.selection.key_guide.map(|k| vectorcraft_engine::cmd::guide_indexes(&st.doc, &[k]));
        assert_eq!(key, Some(vec![0]), "{}", app.ui.status);
        assert_eq!(align_to(&app), AlignTo::Key);
    }
    panel.click("Horizontal Align Left");
    let app = panel.app();
    assert_eq!(left_edge(&app, &a), 120.0, "on the guide, not the artboard: {}", app.ui.status);
    assert_eq!(app.session.active().unwrap().doc.ruler_guides()[0].pos, 120.0);
}

/// A Shift-click that leaves the key object selected on its own lets the key go, and the align
/// buttons of the Control bar and the Properties panel, and then the Align panel's, align that
/// object to the artboard.
#[test]
fn a_lone_object_aligns_to_the_artboard() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
    let b = app.run("shape.rectangle", json!({"x": 80, "y": 50, "width": 30, "height": 30})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.run("select.key", json!({"id": b})).unwrap();
    app.run("select.toggle", json!({"id": a})).unwrap();
    let id = vectorcraft_doc::NodeId(b.as_u64().unwrap());
    let bounds = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    // `align_buttons` (the Control bar and the Properties panel).
    let mut buttons = Panel::new(app, align_buttons);
    buttons.click("Horizontal Align Right");
    assert_eq!(bounds(&buttons.app()).x1, 200.0, "the artboard's right edge");
    // The Align panel's Horizontal Align Left.
    let p = with_align_to(&buttons.app(), json!({"horizontal": "left", "bounds": "geometric"}));
    buttons.app().run("object.align", p).unwrap();
    assert_eq!(bounds(&buttons.app()).x0, 0.0, "the artboard's left edge");
}

/// #541: while the selection has a key object Align aligns to it, whatever Align To says;
/// choosing Align to Selection lets the key go.
#[test]
fn a_key_object_makes_align_align_to_it() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
    let b = app.run("shape.rectangle", json!({"x": 80, "y": 50, "width": 30, "height": 30})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    assert_eq!(align_to(&app), AlignTo::Selection);
    app.run("select.key", json!({"id": b})).unwrap();
    assert_eq!(align_to(&app), AlignTo::Key);
    app.run("select.key", json!({})).unwrap();
    assert_eq!(align_to(&app), AlignTo::Selection, "no key: the choice again");
}

/// Two rectangles, the back one at x 10 and the front one at x 80, selected front first →
/// (the app, the back one's id, the front one's id).
fn back_and_front() -> (VectorcraftApp, Value, Value) {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let back = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
    let front = app.run("shape.rectangle", json!({"x": 80, "y": 50, "width": 30, "height": 30})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [front, back]})).unwrap();
    (app, back, front)
}

/// The left edge of object `id`.
fn left_edge(app: &VectorcraftApp, id: &Value) -> f64 {
    let id = vectorcraft_doc::NodeId(id.as_u64().unwrap());
    app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0
}

/// With two objects selected and no key object, Align to Key Object makes the topmost one the
/// key, whatever order they were selected in, and Align then aligns to it. A key already set
/// stays the key.
#[test]
fn align_to_key_object_makes_the_topmost_object_the_key() {
    let (app, back, front) = back_and_front();
    let mut panel = Panel::new(app, show);
    panel.click("Align to Key Object");
    let key_object = |panel: &Panel| panel.app().session.active().unwrap().selection.key.map(|k| k.0);
    assert_eq!(key_object(&panel), front.as_u64(), "the front rectangle, selected first");
    panel.app().run("select.set", json!({"ids": [back, front]})).unwrap();
    panel.click("Align to Key Object");
    assert_eq!(key_object(&panel), front.as_u64(), "the front rectangle, selected last");
    assert_eq!(align_to(&panel.app()), AlignTo::Key);
    panel.click("Horizontal Align Left");
    let app = panel.app();
    assert_eq!((left_edge(&app, &back), left_edge(&app, &front)), (80.0, 80.0), "to the key's left edge: {}", app.ui.status);
    drop(app);
    panel.app().run("select.key", json!({"id": back})).unwrap();
    panel.click("Align to Key Object");
    assert_eq!(key_object(&panel), back.as_u64(), "a key already set");
}

/// Align to Key Object stores no Align To choice: once the key object is cleared, by Cancel
/// Key Object, by a click on the key or by another selection, Align To shows the stored
/// choice again.
#[test]
fn align_to_shows_its_choice_again_once_the_key_is_cleared() {
    let (app, back, front) = back_and_front();
    let mut panel = Panel::new(app, show);
    panel.click("Align to Artboard");
    panel.click("Align to Key Object");
    assert_eq!(align_to(&panel.app()), AlignTo::Key);
    panel.app().run("ui.align.cancelKeyObject", json!({})).unwrap();
    assert!(!has_key(&panel.app()));
    assert_eq!(align_to(&panel.app()), AlignTo::Artboard, "Cancel Key Object");
    // The Selection tool's click on the key object runs `select.key {}`.
    panel.click("Align to Key Object");
    assert!(has_key(&panel.app()));
    panel.app().run("select.key", json!({})).unwrap();
    assert_eq!(align_to(&panel.app()), AlignTo::Artboard, "a click on the key");
    panel.click("Align to Key Object");
    assert!(has_key(&panel.app()));
    panel.app().run("select.set", json!({"ids": [back, front]})).unwrap();
    assert_eq!(align_to(&panel.app()), AlignTo::Artboard, "another selection");
}

/// The Align To choice is kept with the UI state, which the app saves with its preferences and
/// `ui.inspect` reports. A saved value other than "artboard" reads as Selection, and the rest of
/// the state still loads.
#[test]
fn the_align_to_choice_is_kept_with_the_ui_state() {
    let (app, _, _) = back_and_front();
    let mut panel = Panel::new(app, show);
    panel.click("Align to Artboard");
    assert_eq!(serde_json::to_value(&panel.app().ui).unwrap()["align_to"], "artboard");
    let back: crate::UiState = serde_json::from_value(serde_json::to_value(&panel.app().ui).unwrap()).unwrap();
    assert_eq!(back.align_to, AlignTo::Artboard);
    for v in [json!("key"), json!("Artboard"), json!(3), Value::Null] {
        let ui: crate::UiState = serde_json::from_value(json!({"align_to": v, "place_link": false})).unwrap();
        assert_eq!((ui.align_to, ui.place_link), (AlignTo::Selection, false), "{v}");
    }
    let old: crate::UiState = serde_json::from_value(json!({})).unwrap();
    assert_eq!(old.align_to, AlignTo::Selection, "preferences saved without it");
}

/// With one object selected, Align to Key Object is disabled: a click sets no key and keeps the
/// Align To choice, and Horizontal Align Left moves the object to the artboard's left edge.
#[test]
fn align_to_key_object_needs_two_selected_objects() {
    let (mut app, _, front) = back_and_front();
    app.run("select.set", json!({"ids": [front]})).unwrap();
    let mut panel = Panel::new(app, show);
    assert!(!panel.enabled("Align to Key Object"), "disabled");
    panel.click("Align to Key Object");
    assert_eq!(panel.app().session.active().unwrap().selection.key, None);
    assert_eq!(align_to(&panel.app()), AlignTo::Selection);
    panel.click("Horizontal Align Left");
    assert_eq!(left_edge(&panel.app(), &front), 0.0, "the artboard's left edge");
}

/// With one object selected the align buttons align it to the artboard, and Align To shows
/// Artboard; with two, it shows the stored choice.
#[test]
fn align_to_shows_artboard_while_one_object_is_selected() {
    let (mut app, _, front) = back_and_front();
    assert_eq!(align_to_checks(&app), [Some(true), Some(false), Some(false)], "two objects");
    app.run("select.set", json!({"ids": [front]})).unwrap();
    assert_eq!(align_to_checks(&app), [Some(false), Some(false), Some(true)], "one object");
    assert_eq!(align_to(&app), AlignTo::Selection, "the stored choice");
}

/// #789: the Control bar's and the Properties panel's align buttons follow Align To too.
#[test]
fn the_other_align_buttons_align_to_the_align_to_choice() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
    let b = app.run("shape.rectangle", json!({"x": 80, "y": 50, "width": 30, "height": 30})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.ui.align_to = AlignTo::Artboard;
    let mut panel = Panel::new(app, align_buttons);
    panel.click("Horizontal Align Right");
    let app = panel.app();
    let st = app.session.active().unwrap();
    let x1 = |id: &serde_json::Value| st.doc.bounds_of(&[vectorcraft_doc::NodeId(id.as_u64().unwrap())], false).unwrap().x1;
    assert_eq!((x1(&a), x1(&b)), (200.0, 200.0), "both to the artboard's right edge");
}

/// The command of an Align panel button runs what a click on the button runs: Horizontal
/// Align Right aligns to the selection, to the active artboard with Align to Artboard, and to
/// the key object while there is one. Horizontal Distribute Left sends the key and the
/// Distribute Spacing value, and Horizontal Distribute Space makes the gaps equal.
#[test]
fn a_shortcut_runs_its_align_panel_button() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let ids: Vec<Value> = [(10, 20), (50, 40), (120, 20)]
        .into_iter()
        .map(|(x, w)| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": w, "height": 10})).unwrap()["id"].clone())
        .collect();
    app.run("select.set", json!({"ids": ids})).unwrap();
    let edges = |app: &VectorcraftApp, right: bool| {
        let st = app.session.active().unwrap();
        ids.iter()
            .map(|id| {
                let b = st.doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().geometric_bounds().unwrap();
                if right { b.x1 } else { b.x0 }
            })
            .collect::<Vec<_>>()
    };
    let sent = |app: &VectorcraftApp| {
        let (id, p) = app.session.journal.last().unwrap();
        (id.clone(), p["to"].clone(), p.get("spacing").cloned())
    };
    app.run("align.horizontalRight", json!({})).unwrap();
    assert_eq!(edges(&app, true), [140.0; 3], "Align to Selection: {}", app.ui.status);
    app.run("edit.undo", json!({})).unwrap();
    app.ui.align_to = AlignTo::Artboard;
    app.run("align.horizontalRight", json!({})).unwrap();
    assert_eq!(edges(&app, true), [200.0; 3], "Align to Artboard: {}", app.ui.status);
    app.run("edit.undo", json!({})).unwrap();
    app.run("select.key", json!({"id": ids[1]})).unwrap();
    app.run("align.horizontalRight", json!({})).unwrap();
    assert_eq!(edges(&app, true), [90.0; 3], "the key object's right edge: {}", app.ui.status);
    app.run("edit.undo", json!({})).unwrap();
    // Distribute Objects with the key object and a Distribute Spacing value of 15 pt.
    app.ui.align_spacing = 15.0;
    app.run("align.distributeHorizontalLeft", json!({})).unwrap();
    assert_eq!(sent(&app), ("object.distribute".to_string(), json!("key"), Some(json!(15.0))));
    assert_eq!(edges(&app, false), [35.0, 50.0, 65.0], "15 pt apart: {}", app.ui.status);
    app.run("edit.undo", json!({})).unwrap();
    // Distribute Spacing with Align to Selection: the outer objects stay, and the gaps are equal.
    app.run("select.key", json!({})).unwrap();
    app.ui.align_to = AlignTo::Selection;
    app.ui.align_to = AlignTo::Selection;
    app.run("align.distributeHorizontalSpace", json!({})).unwrap();
    assert_eq!(sent(&app), ("object.distributeSpacing".to_string(), json!("selection"), Some(Value::Null)));
    assert_eq!(edges(&app, false), [10.0, 55.0, 120.0], "25 pt gaps: {}", app.ui.status);
}

/// With Align to Artboard chosen, the Align panel's Horizontal Distribute Left puts the
/// outermost of three objects (20, 40 and 20 pt wide) on the active artboard's left and right
/// edges and spaces their left edges evenly, and Horizontal Distribute Space makes the gaps
/// between them equal. With Align to Selection chosen, the outermost objects stay put.
#[test]
fn the_distribute_buttons_follow_align_to_artboard() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200, "artboards": 2})).unwrap();
    let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    let ids: Vec<Value> = [(10, 20), (50, 40), (120, 20)]
        .into_iter()
        .map(|(x, w)| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": w, "height": 10})).unwrap()["id"].clone())
        .collect();
    app.run("select.set", json!({"ids": ids})).unwrap();
    let lefts = |app: &VectorcraftApp| ids.iter().map(|id| left_edge(app, id)).collect::<Vec<_>>();
    let mut panel = Panel::new(app, show);
    panel.click("Horizontal Distribute Left");
    assert_eq!(lefts(&panel.app()), [10.0, 65.0, 120.0], "Align to Selection: {}", panel.app().ui.status);
    panel.app().run("edit.undo", json!({})).unwrap();
    // The objects are on artboard 0, and artboard 1 is the active one.
    panel.app().ui.align_to = AlignTo::Artboard;
    panel.app().view_mut().unwrap().artboard = 1;
    let (x0, x1) = (boards[1].x0, boards[1].x1);
    panel.click("Horizontal Distribute Left");
    assert_eq!(lefts(&panel.app()), [x0, x0 + 90.0, x1 - 20.0], "the active artboard's edges: {}", panel.app().ui.status);
    panel.app().run("edit.undo", json!({})).unwrap();
    panel.click("Horizontal Distribute Space");
    assert_eq!(lefts(&panel.app()), [x0, x0 + 80.0, x1 - 20.0], "60 pt gaps: {}", panel.app().ui.status);
}

/// With a key object, the Align panel's Distribute Objects buttons send the Distribute Spacing
/// value: Horizontal Distribute Left puts the others' left edges on the key's left edge with
/// the default value of 0, and puts them 15 pt apart from it with 15. Without a key object
/// they send no spacing.
#[test]
fn the_distribute_objects_buttons_send_the_spacing_with_a_key_object() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let ids: Vec<Value> = [(10, 20), (50, 40), (120, 20)]
        .into_iter()
        .map(|(x, w)| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": w, "height": 10})).unwrap()["id"].clone())
        .collect();
    app.run("select.set", json!({"ids": ids})).unwrap();
    let lefts = |app: &VectorcraftApp| ids.iter().map(|id| left_edge(app, id)).collect::<Vec<_>>();
    // The command and its `to` and `spacing` params (the app adds the active artboard).
    let sent = |app: &VectorcraftApp| {
        let (id, p) = app.session.journal.last().unwrap();
        (id.clone(), p["to"].clone(), p.get("spacing").cloned())
    };
    let mut panel = Panel::new(app, show);
    panel.click("Horizontal Distribute Left");
    assert_eq!(sent(&panel.app()), ("object.distribute".to_string(), json!("selection"), None), "no key object");
    assert_eq!(lefts(&panel.app()), [10.0, 65.0, 120.0], "{}", panel.app().ui.status);
    panel.app().run("edit.undo", json!({})).unwrap();
    panel.app().run("select.key", json!({"id": ids[1]})).unwrap();
    panel.click("Horizontal Distribute Left");
    assert_eq!(sent(&panel.app()), ("object.distribute".to_string(), json!("key"), Some(json!(0.0))));
    assert_eq!(lefts(&panel.app()), [50.0; 3], "on the key's left edge: {}", panel.app().ui.status);
    panel.app().run("edit.undo", json!({})).unwrap();
    panel.app().ui.align_spacing = 15.0;
    panel.click("Horizontal Distribute Left");
    assert_eq!(lefts(&panel.app()), [35.0, 50.0, 65.0], "15 pt apart: {}", panel.app().ui.status);
}

/// #2143: with a key guide, the Distribute Objects and Distribute Spacing buttons send the
/// Distribute Spacing value too, and the objects are spaced from the guide, which stays put.
#[test]
fn the_distribute_buttons_send_the_spacing_with_a_key_guide() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let ids: Vec<Value> = [(10, 20), (50, 40), (120, 20)]
        .into_iter()
        .map(|(x, w)| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": w, "height": 10})).unwrap()["id"].clone())
        .collect();
    app.run("guide.add", json!({"vertical": true, "pos": 100})).unwrap();
    app.run("select.set", json!({"ids": ids})).unwrap();
    app.run("guide.select", json!({"indexes": [0], "toggle": true})).unwrap();
    app.run("select.key", json!({"guide": 0})).unwrap();
    let lefts = |app: &VectorcraftApp| ids.iter().map(|id| left_edge(app, id)).collect::<Vec<_>>();
    let sent = |app: &VectorcraftApp| {
        let (id, p) = app.session.journal.last().unwrap();
        (id.clone(), p["to"].clone(), p.get("spacing").cloned())
    };
    let mut panel = Panel::new(app, show);
    panel.click("Horizontal Distribute Left");
    assert_eq!(sent(&panel.app()), ("object.distribute".to_string(), json!("key"), Some(json!(0.0))));
    assert_eq!(lefts(&panel.app()), [100.0; 3], "on the guide: {}", panel.app().ui.status);
    panel.app().run("edit.undo", json!({})).unwrap();
    panel.app().ui.align_spacing = 15.0;
    panel.click("Horizontal Distribute Left");
    assert_eq!(lefts(&panel.app()), [70.0, 85.0, 115.0], "15 pt apart, outward from the guide: {}", panel.app().ui.status);
    assert_eq!(panel.app().session.active().unwrap().doc.ruler_guides()[0].pos, 100.0, "the guide stays put");
    // Horizontal Distribute Space with 15: the object before the guide ends 15 pt before it,
    // the one after it starts 15 pt after it.
    panel.click("Horizontal Distribute Space");
    assert_eq!(sent(&panel.app()), ("object.distributeSpacing".to_string(), json!("key"), Some(json!(15.0))));
    assert_eq!(lefts(&panel.app()), [10.0, 45.0, 115.0], "{}", panel.app().ui.status);
    assert_eq!(panel.app().session.active().unwrap().doc.ruler_guides()[0].pos, 100.0, "the guide stays put");
}

/// Three vertical ruler guides selected without objects: Align Objects buttons are on, and
/// Horizontal Align Left puts every guide on the artboard's left edge (#3463).
#[test]
fn the_align_buttons_align_guides_selected_alone_to_artboard() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    for pos in [20, 50, 180] {
        app.run("guide.add", json!({"vertical": true, "pos": pos})).unwrap();
    }
    app.run("guide.select", json!({"indexes": [0, 1, 2]})).unwrap();
    app.ui.align_to = AlignTo::Artboard;
    let mut panel = Panel::new(app, show);
    assert!(panel.enabled("Horizontal Align Left"), "Horizontal Align Left is enabled");
    panel.click("Horizontal Align Left");
    let pos = |app: &VectorcraftApp| app.session.active().unwrap().doc.ruler_guides().iter().map(|g| g.pos).collect::<Vec<_>>();
    assert_eq!(pos(&panel.app()), [0.0, 0.0, 0.0], "{}", panel.app().ui.status);
}

/// Three vertical ruler guides selected without objects: the Distribute buttons are on, and
/// Horizontal Distribute Center puts the middle guide halfway between the other two, or with
/// Align to Artboard spaces them from its left edge to its right edge, as a public forum answer
/// describes for equal columns (https://community.adobe.com/questions-652/illustrator-equal-guides-822924).
#[test]
fn the_distribute_buttons_space_guides_selected_alone() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    for pos in [20, 50, 180] {
        app.run("guide.add", json!({"vertical": true, "pos": pos})).unwrap();
    }
    app.run("guide.select", json!({"indexes": [0, 1, 2]})).unwrap();
    let mut panel = Panel::new(app, show);
    panel.click("Horizontal Distribute Center");
    let pos = |app: &VectorcraftApp| app.session.active().unwrap().doc.ruler_guides().iter().map(|g| g.pos).collect::<Vec<_>>();
    assert_eq!(pos(&panel.app()), [20.0, 100.0, 180.0], "{}", panel.app().ui.status);
    panel.app().run("edit.undo", json!({})).unwrap();
    assert_eq!(pos(&panel.app()), [20.0, 50.0, 180.0], "one undo step");
    // With Align to Artboard, the outermost guides go to its edges: equal columns across it.
    panel.app().ui.align_to = AlignTo::Artboard;
    panel.click("Horizontal Distribute Center");
    assert_eq!(pos(&panel.app()), [0.0, 100.0, 200.0], "{}", panel.app().ui.status);
}

/// Runs the Object → Align item `label` as a click on it does.
fn object_align(app: &mut VectorcraftApp, label: &str) {
    let entry = crate::menus::menu_entries(app).into_iter().find(|e| e.path == ["Object", "Align"] && e.label == label).expect("menu item");
    let (id, p) = crate::menus::click_target(&entry.label, entry.command.as_deref().unwrap(), &entry.params);
    crate::menus::invoke(app, &id, p);
}

/// Object → Align aligns as the align buttons do: it aligns one object to the active artboard
/// whatever Align To says, and with Align to Artboard chosen it aligns two objects to the active
/// artboard too.
#[test]
fn object_align_follows_align_to() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200, "artboards": 2})).unwrap();
    let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
    let b = app.run("shape.rectangle", json!({"x": 80, "y": 50, "width": 30, "height": 30})).unwrap()["id"].clone();
    let bounds = |app: &VectorcraftApp, id: &Value| {
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().geometric_bounds().unwrap()
    };
    app.run("select.set", json!({"ids": [b]})).unwrap();
    object_align(&mut app, "Horizontal Align Left");
    assert_eq!(bounds(&app, &b).x0, 0.0, "one object, Align to Selection: the artboard's left edge");
    app.ui.align_to = AlignTo::Artboard;
    app.view_mut().unwrap().artboard = 1;
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    object_align(&mut app, "Horizontal Align Right");
    assert_eq!((bounds(&app, &a).x1, bounds(&app, &b).x1), (boards[1].x1, boards[1].x1), "the active artboard's right edge");
}

/// The Align To choices of Object → Align ([`ALIGN_TO_COMMANDS`]), in their order: (command id,
/// label).
const ALIGN_TO_ENTRIES: [(&str, &str); 3] =
    [("align.toSelection", "Align to Selection"), ("align.toKeyObject", "Align to Key Object"), ("align.toArtboard", "Align to Artboard")];

/// Which of Object → Align's Align To items are checked, in their order.
fn align_to_checks(app: &VectorcraftApp) -> Vec<Option<bool>> {
    ALIGN_TO_ENTRIES.iter().map(|(id, _)| crate::menus::checked(app, id, &json!({}))).collect()
}

/// Object → Align lists the Align To choices after its align items, the one the Align panel shows
/// checked, and a click on one chooses it as the panel's button does; its command chooses it too.
#[test]
fn object_align_lists_the_align_to_choices() {
    let (mut app, back, front) = back_and_front();
    let items: Vec<(String, Option<String>)> =
        crate::menus::menu_entries(&app).into_iter().filter(|e| e.path == ["Object", "Align"]).map(|e| (e.label, e.command)).skip(7).collect();
    let expected: Vec<(String, Option<String>)> = ALIGN_TO_ENTRIES.iter().map(|(id, l)| (l.to_string(), Some(id.to_string()))).collect();
    assert_eq!(items, expected, "after the seven align items");
    assert_eq!(align_to_checks(&app), [Some(true), Some(false), Some(false)]);
    object_align(&mut app, "Align to Artboard");
    assert_eq!(app.ui.align_to, AlignTo::Artboard);
    assert_eq!(align_to_checks(&app), [Some(false), Some(false), Some(true)]);
    // Align to Key Object by its command: the topmost selected object becomes the key, and the
    // stored choice stays.
    app.run("align.toKeyObject", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().selection.key.map(|k| k.0), front.as_u64(), "{}", app.ui.status);
    assert_eq!(align_to_checks(&app), [Some(false), Some(true), Some(false)]);
    assert_eq!(app.ui.align_to, AlignTo::Artboard);
    // Align to Selection lets the key go and stores Selection.
    assert_eq!(app.run("align.toSelection", json!({})).unwrap(), json!({"alignTo": "selection"}));
    assert!(!has_key(&app));
    assert_eq!(app.ui.align_to, AlignTo::Selection);
    // One object: Align to Key Object is off and fails, and Align To shows Artboard checked.
    app.run("select.set", json!({"ids": [back]})).unwrap();
    let key_item = crate::menus::menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some("align.toKeyObject")).unwrap();
    assert!(!key_item.enabled, "one object");
    assert_eq!(app.run("align.toKeyObject", json!({})).unwrap_err(), "no key object");
    assert!(!has_key(&app));
    assert_eq!(align_to_checks(&app), [Some(false), Some(false), Some(true)]);
    // Align to Selection with one object stores Selection and returns what the panel shows.
    assert_eq!(app.run("align.toSelection", json!({})).unwrap(), json!({"alignTo": "artboard"}));
    assert_eq!(app.ui.align_to, AlignTo::Selection);
}

/// With anchor points of one path selected (Direct Selection), Align to Selection returns
/// `"selection"`: the points align to each other, so the lone path does not make it Artboard.
#[test]
fn align_to_selection_with_anchor_points_returns_selection() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = app.run("path.create", json!({"d": "M100 40 L200 60 L150 160 Z"})).unwrap()["id"].as_u64().unwrap();
    app.ui.align_to = AlignTo::Artboard;
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
    assert_eq!(app.run("align.toSelection", json!({})).unwrap(), json!({"alignTo": "selection"}));
    assert_eq!(app.ui.align_to, AlignTo::Selection);
    assert_eq!(align_to_checks(&app), [Some(true), Some(false), Some(false)]);
}

/// With the Artboard tool chosen, Align to Key Object is off and fails, and Align to Artboard and
/// Align to Selection store the choice while the selected objects keep their key.
#[test]
fn align_to_commands_with_the_artboard_tool() {
    let (mut app, _, front) = back_and_front();
    app.run("align.toKeyObject", json!({})).unwrap();
    app.select_tool("artboard");
    assert!(has_key(&app), "choosing the Artboard tool keeps the key");
    let key_item = crate::menus::menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some("align.toKeyObject")).unwrap();
    assert!(!key_item.enabled, "the Artboard tool");
    assert_eq!(app.run("align.toKeyObject", json!({})).unwrap_err(), "Align to Key Object isn't available with the Artboard tool");
    assert_eq!(app.run("align.toArtboard", json!({})).unwrap(), json!({"alignTo": "artboard"}));
    assert_eq!(app.ui.align_to, AlignTo::Artboard);
    assert_eq!(align_to_checks(&app), [Some(false), Some(false), Some(true)]);
    assert_eq!(app.run("align.toSelection", json!({})).unwrap(), json!({"alignTo": "selection"}));
    assert_eq!(app.ui.align_to, AlignTo::Selection);
    assert_eq!(align_to_checks(&app), [Some(true), Some(false), Some(false)]);
    assert_eq!(app.session.active().unwrap().selection.key.map(|k| k.0), front.as_u64(), "the key stays");
}

/// With the Artboard tool chosen and one object selected with no anchor points, Align to Selection
/// returns `"selection"`, the stored choice, not `"artboard"`.
#[test]
fn align_to_selection_with_one_object_and_the_artboard_tool_returns_selection() {
    let (mut app, back, _) = back_and_front();
    app.run("select.set", json!({"ids": [back]})).unwrap();
    app.ui.align_to = AlignTo::Artboard;
    app.select_tool("artboard");
    assert_eq!(app.run("align.toSelection", json!({})).unwrap(), json!({"alignTo": "selection"}));
    assert_eq!(app.ui.align_to, AlignTo::Selection);
    assert_eq!(align_to_checks(&app), [Some(true), Some(false), Some(false)]);
}

/// Align to Key Object is off and fails when the topmost selected object can't be the key: a group
/// selected with an object inside it.
#[test]
fn align_to_key_object_fails_when_the_topmost_object_cannot_be_the_key() {
    let (mut app, _, front) = back_and_front();
    let group = app.run("object.group", json!({})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [group, front]})).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects.len(), 2, "the group and its member");
    let key_item = crate::menus::menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some("align.toKeyObject")).unwrap();
    assert!(!key_item.enabled, "the topmost object is inside the group");
    assert!(!can_choose(&app, AlignTo::Key), "the panel's button is off too");
    assert_eq!(app.run("align.toKeyObject", json!({})).unwrap_err(), "no key object");
    assert!(!has_key(&app));
}

/// With a key anchor, Object → Align shows Align to Key Object checked under its menu name, and
/// the command keeps the key anchor → `{alignTo: "key"}`.
#[test]
fn align_to_key_object_with_a_key_anchor() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = app.run("path.create", json!({"d": "M100 40 L200 60 L150 160 Z"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 0], [0, 2]], "mode": "set"})).unwrap();
    app.run("select.key", json!({"id": id, "anchor": [0, 2]})).unwrap();
    assert_eq!(align_to_checks(&app), [Some(false), Some(true), Some(false)]);
    let key_item = crate::menus::menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some("align.toKeyObject")).unwrap();
    assert_eq!((key_item.label.as_str(), key_item.enabled), ("Align to Key Object", true));
    assert_eq!(app.run("align.toKeyObject", json!({})).unwrap(), json!({"alignTo": "key"}));
    let key_anchor = app.session.active().unwrap().selection.key_anchor.map(|(k, a)| (k.0, a));
    assert_eq!(key_anchor, Some((id, (0, 2))), "the key anchor stays");
}

/// With anchor points selected, Object → Align sends its params unchanged: two anchor points of
/// one path align to their own bounds with Align to Selection and with Align to Artboard chosen.
#[test]
fn object_align_aligns_anchor_points_to_each_other() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = app.run("path.create", json!({"d": "M100 40 L200 60 L150 160 Z"})).unwrap()["id"].as_u64().unwrap();
    let anchors = |app: &VectorcraftApp| -> Vec<(f64, f64)> {
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
        node.path_data().unwrap().anchors().map(|(_, _, a)| (a.p.x, a.p.y)).collect()
    };
    for to in [AlignTo::Selection, AlignTo::Artboard] {
        app.ui.align_to = to;
        app.run("select.anchors", json!({"id": id, "anchors": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
        object_align(&mut app, "Horizontal Align Left");
        assert_eq!(anchors(&app), [(100.0, 40.0), (100.0, 60.0), (150.0, 160.0)], "{to:?}: {}", app.ui.status);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(anchors(&app)[1], (200.0, 60.0), "undone");
    }
}

/// #1636: with two anchor points of one path selected, the Align panel's buttons align the
/// points to each other, not to the artboard as they align a lone object, and Align To shows
/// the stored choice. With Align to Artboard chosen the points go to the artboard.
#[test]
fn anchor_points_of_one_path_align_to_each_other() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = app.run("path.create", json!({"d": "M100 40 L200 60 L150 160 Z"})).unwrap()["id"].as_u64().unwrap();
    let xs = |app: &VectorcraftApp| -> Vec<f64> {
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
        node.path_data().unwrap().anchors().map(|(_, _, a)| a.p.x).collect()
    };
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
    let mut panel = Panel::new(app, show);
    panel.click("Horizontal Align Left");
    assert_eq!(xs(&panel.app()), [100.0, 100.0, 150.0], "to the points' left edge: {}", panel.app().ui.status);
    assert_eq!(align_to_checks(&panel.app()), [Some(true), Some(false), Some(false)], "the stored choice");
    panel.app().run("edit.undo", json!({})).unwrap();
    panel.click("Align to Artboard");
    panel.click("Horizontal Align Left");
    assert_eq!(xs(&panel.app()), [0.0, 0.0, 150.0], "to the artboard's left edge: {}", panel.app().ui.status);
}

/// #1636: with a key anchor, the Align panel shows the key choice chosen and named Align to Key
/// Anchor, its align button moves the other points to the key anchor, which stays, and Align to
/// Selection lets go of it.
#[test]
fn a_key_anchor_is_chosen_named_and_aligned_to() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = app.run("path.create", json!({"d": "M100 40 L200 60 L150 160 Z"})).unwrap()["id"].as_u64().unwrap();
    let xs = |app: &VectorcraftApp| -> Vec<f64> {
        let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
        node.path_data().unwrap().anchors().map(|(_, _, a)| a.p.x).collect()
    };
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 0], [0, 2]], "mode": "set"})).unwrap();
    app.run("select.key", json!({"id": id, "anchor": [0, 2]})).unwrap();
    let mut panel = Panel::new(app, show);
    panel.frame();
    assert_eq!(
        {
            let app = panel.app();
            (align_to(&app), align_to_checks(&app))
        },
        (AlignTo::Key, vec![Some(false), Some(true), Some(false)]),
        "chosen automatically"
    );
    assert!(panel.shows("Align to Key Anchor"));
    panel.click("Horizontal Align Left");
    assert_eq!(xs(&panel.app()), [150.0, 200.0, 150.0], "to the key anchor's x: {}", panel.app().ui.status);
    panel.click("Align to Selection");
    assert!(!has_key(&panel.app()), "Align to Selection lets go of the key anchor");
    assert_eq!(panel.app().ui.align_to, AlignTo::Selection);
}

/// #1636: the Distribute buttons work on the anchor points of one path (one object selected).
#[test]
fn distribute_buttons_space_the_anchor_points_of_one_path() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = app.run("path.create", json!({"d": "M10 10 L20 50 L100 30"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 0], [0, 1], [0, 2]], "mode": "set"})).unwrap();
    let mut panel = Panel::new(app, show);
    panel.click("Horizontal Distribute Center");
    let app = panel.app();
    let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
    let xs: Vec<f64> = node.path_data().unwrap().anchors().map(|(_, _, a)| a.p.x).collect();
    assert_eq!(xs, [10.0, 55.0, 100.0], "{}", app.ui.status);
}

/// Three artboards, 0 at (0, 0) 100 × 100, 1 at (150, 40) 60 × 80 and 2 at (400, 10) 100 × 50, with
/// the Artboard tool chosen and all three selected, artboard 1 active.
fn selected_artboards() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("artboard.new", json!({"x": 150, "y": 40, "width": 60, "height": 80})).unwrap();
    app.run("artboard.new", json!({"x": 400, "y": 10, "width": 100, "height": 50})).unwrap();
    app.select_tool("artboard");
    app.session.set_tool_option("selected", &json!([1, 0, 2]));
    app
}

/// The top-left corner of each artboard.
fn artboard_corners(app: &VectorcraftApp) -> Vec<(f64, f64)> {
    app.session.active().unwrap().doc.artboards.iter().map(|a| (a.rect.x0, a.rect.y0)).collect()
}

/// The command the app ran last, with its `to` and `spacing`.
fn last_sent(app: &VectorcraftApp) -> (String, Value, Option<Value>) {
    let (id, p) = app.session.journal.last().unwrap();
    (id.clone(), p["to"].clone(), p.get("spacing").cloned())
}

/// With the Artboard tool chosen, the Align Objects buttons align the selected artboards: to
/// their bounds with Align to Selection, and to the active artboard, which stays in place, with
/// Align to Artboard. Align to Key Object cannot be chosen.
#[test]
fn the_align_buttons_align_the_selected_artboards() {
    let app = selected_artboards();
    let steps = app.session.active().unwrap().history.undo.len();
    let mut panel = Panel::new(app, show);
    panel.click("Horizontal Align Left");
    assert_eq!(artboard_corners(&panel.app()), [(0.0, 0.0), (0.0, 40.0), (0.0, 10.0)], "{}", panel.app().ui.status);
    assert_eq!(last_sent(&panel.app()), ("artboard.align".to_string(), json!("selection"), None));
    assert_eq!(panel.app().session.active().unwrap().history.undo.len(), steps + 1, "one undo step");
    panel.app().run("edit.undo", json!({})).unwrap();
    panel.click("Align to Key Object");
    assert!(
        {
            let app = panel.app();
            app.ui.align_to == AlignTo::Selection && !has_key(&app)
        },
        "Align to Key Object is off"
    );
    panel.click("Align to Artboard");
    assert_eq!(panel.app().ui.align_to, AlignTo::Artboard);
    panel.click("Horizontal Align Left");
    assert_eq!(artboard_corners(&panel.app()), [(150.0, 0.0), (150.0, 40.0), (150.0, 10.0)], "artboard 1 stays: {}", panel.app().ui.status);
    assert_eq!(last_sent(&panel.app()), ("artboard.align".to_string(), json!("artboard"), None));
}

/// With the Artboard tool chosen, the Distribute Objects and Distribute Spacing buttons space the
/// selected artboards. With Align to Artboard they send the Distribute Spacing value and the
/// active artboard stays in place. They are off for one artboard.
#[test]
fn the_distribute_buttons_space_the_selected_artboards() {
    let app = selected_artboards();
    let mut panel = Panel::new(app, show);
    panel.click("Horizontal Distribute Left");
    assert_eq!(artboard_corners(&panel.app()), [(0.0, 0.0), (200.0, 40.0), (400.0, 10.0)], "{}", panel.app().ui.status);
    assert_eq!(last_sent(&panel.app()), ("artboard.distribute".to_string(), json!("selection"), None));
    panel.app().run("edit.undo", json!({})).unwrap();
    panel.click("Horizontal Distribute Space");
    assert_eq!(artboard_corners(&panel.app()), [(0.0, 0.0), (220.0, 40.0), (400.0, 10.0)], "gaps of 120: {}", panel.app().ui.status);
    panel.app().run("edit.undo", json!({})).unwrap();
    panel.app().ui.align_to = AlignTo::Artboard;
    panel.app().ui.align_spacing = 20.0;
    panel.click("Horizontal Distribute Space");
    assert_eq!(artboard_corners(&panel.app()), [(30.0, 0.0), (150.0, 40.0), (230.0, 10.0)], "gaps of 20: {}", panel.app().ui.status);
    assert_eq!(last_sent(&panel.app()), ("artboard.distributeSpacing".to_string(), json!("artboard"), Some(json!(20.0))));
    panel.app().run("edit.undo", json!({})).unwrap();
    panel.click("Horizontal Distribute Left");
    assert_eq!(artboard_corners(&panel.app()), [(130.0, 0.0), (150.0, 40.0), (170.0, 10.0)], "left edges 20 apart: {}", panel.app().ui.status);
    panel.app().run("edit.undo", json!({})).unwrap();
    panel.app().session.set_tool_option("selected", &json!([1]));
    panel.frame();
    for name in ["Horizontal Distribute Left", "Horizontal Distribute Space"] {
        assert!(!panel.enabled(name), "one artboard: {name} is off");
    }
}

/// With Align to Selection chosen, two selected artboards are the ends of the distribution
/// already, so the Distribute buttons are off; with Align to Artboard and the active artboard
/// selected, the spacing places the second from it, and they are on.
#[test]
fn the_distribute_buttons_need_three_artboards_without_a_spacing() {
    let mut app = selected_artboards();
    // Making an artboard active selects it alone, so the selection is set after.
    crate::panels::artboards::select(&mut app, 1);
    app.session.set_tool_option("selected", &json!([1, 2]));
    let mut panel = Panel::new(app, show);
    panel.frame();
    let on = |panel: &Panel| ["Horizontal Distribute Left", "Horizontal Distribute Space"].map(|name| panel.enabled(name));
    assert_eq!(on(&panel), [false, false], "Align to Selection: off for two artboards");
    panel.app().ui.align_to = AlignTo::Artboard;
    panel.frame();
    assert_eq!(on(&panel), [true, true], "Align to Artboard: on for two artboards");
    panel.app().ui.align_spacing = 20.0;
    panel.click("Horizontal Distribute Space");
    assert_eq!(artboard_corners(&panel.app()), [(0.0, 0.0), (150.0, 40.0), (230.0, 10.0)], "a gap of 20: {}", panel.app().ui.status);
}

/// With Align to Artboard chosen, the spacing is measured from the active artboard, which is
/// always one of the selected ones: selecting artboards 0 and 2 makes artboard 0 active, and
/// Horizontal Distribute Space with 20 keeps it in place.
#[test]
fn the_artboard_spacing_is_measured_from_a_selected_artboard() {
    let mut app = selected_artboards();
    app.session.set_tool_option("selected", &json!([0, 2]));
    assert_eq!((app.session.tool_options()["active"].clone(), artboards_selected(&app)), (json!(0), 2));
    app.ui.align_to = AlignTo::Artboard;
    app.ui.align_spacing = 20.0;
    let mut panel = Panel::new(app, show);
    panel.click("Horizontal Distribute Space");
    assert_eq!(last_sent(&panel.app()), ("artboard.distributeSpacing".to_string(), json!("artboard"), Some(json!(20.0))));
    assert_eq!(artboard_corners(&panel.app()), [(0.0, 0.0), (150.0, 40.0), (120.0, 10.0)], "a gap of 20 after artboard 0: {}", panel.app().ui.status);
}

/// With the Artboard tool chosen, the Distribute Spacing value can be negative, as
/// `artboard.distributeSpacing` takes it: the artboards then overlap. With another tool its
/// lowest value is 0.
#[test]
fn the_artboard_spacing_can_be_negative() {
    let mut app = selected_artboards();
    assert_eq!(spacing_min(&app), f64::NEG_INFINITY);
    app.ui.align_to = AlignTo::Artboard;
    app.ui.align_spacing = -10.0;
    run_button(&mut app, Button::Spacing("horizontal")).unwrap();
    assert_eq!(artboard_corners(&app), [(60.0, 0.0), (150.0, 40.0), (200.0, 10.0)], "10 pt overlaps: {}", app.ui.status);
    app.select_tool("selection");
    assert_eq!(spacing_min(&app), 0.0);
}

/// A button's shortcut entry is enabled by, and runs, the artboard command with the Artboard tool
/// chosen and the object command with another tool.
#[test]
fn a_button_runs_the_artboard_command_only_with_the_artboard_tool() {
    let mut app = selected_artboards();
    let b = Button::Spacing("horizontal");
    assert_eq!(b.command_now(&app), "artboard.distributeSpacing");
    assert_eq!(artboards_selected(&app), 3);
    app.select_tool("selection");
    assert_eq!(b.command_now(&app), "object.distributeSpacing");
    let before = artboard_corners(&app);
    run_button(&mut app, Button::Align("horizontal", "left")).ok();
    assert_eq!(artboard_corners(&app), before, "with the Selection tool the buttons leave the artboards alone");
}
