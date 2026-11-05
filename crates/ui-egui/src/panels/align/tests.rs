use egui::accesskit::{Role, Toggled};

use super::super::tests_appearance::{click, frame_events, text_rect};
use super::*;
use crate::toolbar::tests::widget_rects;
use crate::widgets::accessibility_tests::{role_and_state, tree};

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
    let ctx = egui::Context::default();
    let ([_, key, _], left) = panel_buttons(&mut app, &ctx);
    click(&ctx, &mut app, key.center(), show);
    assert_eq!(app.session.active().unwrap().selection.key_guide, Some(0), "{}", app.ui.status);
    assert_eq!(align_to(&app), AlignTo::Key);
    click(&ctx, &mut app, left.center(), show);
    assert_eq!(left_edge(&app, &a), 120.0, "on the guide, not the artboard: {}", app.ui.status);
    assert_eq!(app.session.active().unwrap().doc.guides[0].pos, 120.0);
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
    let ctx = egui::Context::default();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.run("select.key", json!({"id": b})).unwrap();
    app.run("select.toggle", json!({"id": a})).unwrap();
    let id = vectorcraft_doc::NodeId(b.as_u64().unwrap());
    let bounds = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    // `align_buttons` (the Control bar and the Properties panel): one frame finds the first
    // button, and the next clicks Horizontal Align Right, the third.
    let at = std::cell::Cell::new(egui::Pos2::ZERO);
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            ui.horizontal(|ui| {
                at.set(ui.cursor().min);
                align_buttons(app, ui, 24.0);
            });
        });
        out.textures_delta.clear();
    };
    frame(&mut app, vec![]);
    let right = at.get() + egui::vec2(2.0 * (24.0 + ctx.global_style().spacing.item_spacing.x) + 12.0, 12.0);
    let press = |pressed| egui::Event::PointerButton { pos: right, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, vec![egui::Event::PointerMoved(right), press(true)]);
    frame(&mut app, vec![press(false)]);
    assert_eq!(bounds(&app).x1, 200.0, "the artboard's right edge");
    // The Align panel's Horizontal Align Left.
    let p = with_align_to(&app, json!({"horizontal": "left", "bounds": "geometric"}));
    app.run("object.align", p).unwrap();
    assert_eq!(bounds(&app).x0, 0.0, "the artboard's left edge");
}

#[test]
fn align_both_centers_params() {
    let p = align_params(json!({"horizontal": "center", "vertical": "center"}), AlignTo::Selection, 2);
    assert_eq!(p["horizontal"], "center");
    assert_eq!(p["vertical"], "center");
    assert_eq!(p["to"], "selection");
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

/// One frame of the Align panel → its Align To buttons (Selection, Key Object, Artboard) and
/// its Horizontal Align Left button.
fn panel_buttons(app: &mut VectorcraftApp, ctx: &egui::Context) -> ([egui::Rect; 3], egui::Rect) {
    frame_events(ctx, app, vec![], show);
    // The 28-point buttons are Distribute Spacing's two and then Align To's three.
    let mut small = widget_rects(ctx, egui::vec2(28.0, 28.0));
    small.sort_by(|a, b| a.left().total_cmp(&b.left()));
    let [.., selection, key, artboard] = small[..] else { panic!("no Align To buttons in {small:?}") };
    let objects = widget_rects(ctx, egui::vec2(32.0, 32.0));
    let left = objects.iter().copied().filter(|r| r.top() == objects[0].top()).min_by(|a, b| a.left().total_cmp(&b.left())).unwrap();
    ([selection, key, artboard], left)
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
    let (mut app, back, front) = back_and_front();
    let ctx = egui::Context::default();
    let ([_, key, _], left) = panel_buttons(&mut app, &ctx);
    click(&ctx, &mut app, key.center(), show);
    let key_object = |app: &VectorcraftApp| app.session.active().unwrap().selection.key.map(|k| k.0);
    assert_eq!(key_object(&app), front.as_u64(), "the front rectangle, selected first");
    app.run("select.set", json!({"ids": [back, front]})).unwrap();
    click(&ctx, &mut app, key.center(), show);
    assert_eq!(key_object(&app), front.as_u64(), "the front rectangle, selected last");
    assert_eq!(align_to(&app), AlignTo::Key);
    click(&ctx, &mut app, left.center(), show);
    assert_eq!((left_edge(&app, &back), left_edge(&app, &front)), (80.0, 80.0), "to the key's left edge: {}", app.ui.status);
    app.run("select.key", json!({"id": back})).unwrap();
    click(&ctx, &mut app, key.center(), show);
    assert_eq!(key_object(&app), back.as_u64(), "a key already set");
}

/// Align to Key Object stores no Align To choice: once the key object is cleared, by Cancel
/// Key Object, by a click on the key or by another selection, Align To shows the stored
/// choice again.
#[test]
fn align_to_shows_its_choice_again_once_the_key_is_cleared() {
    let (mut app, back, front) = back_and_front();
    let ctx = egui::Context::default();
    let ([_, key, artboard], _) = panel_buttons(&mut app, &ctx);
    click(&ctx, &mut app, artboard.center(), show);
    click(&ctx, &mut app, key.center(), show);
    assert_eq!(align_to(&app), AlignTo::Key);
    let cancel = text_rect(&frame_events(&ctx, &mut app, vec![], menu), "   Cancel Key Object");
    click(&ctx, &mut app, cancel.center(), menu);
    assert!(!has_key(&app));
    assert_eq!(align_to(&app), AlignTo::Artboard, "Cancel Key Object");
    // The Selection tool's click on the key object runs `select.key {}`.
    click(&ctx, &mut app, key.center(), show);
    assert!(has_key(&app));
    app.run("select.key", json!({})).unwrap();
    assert_eq!(align_to(&app), AlignTo::Artboard, "a click on the key");
    click(&ctx, &mut app, key.center(), show);
    assert!(has_key(&app));
    app.run("select.set", json!({"ids": [back, front]})).unwrap();
    assert_eq!(align_to(&app), AlignTo::Artboard, "another selection");
}

/// The Align To choice is kept with the UI state, which the app saves with its preferences and
/// `ui.inspect` reports. A saved value other than "artboard" reads as Selection, and the rest of
/// the state still loads.
#[test]
fn the_align_to_choice_is_kept_with_the_ui_state() {
    let (mut app, _, _) = back_and_front();
    let ctx = egui::Context::default();
    let ([_, _, artboard], _) = panel_buttons(&mut app, &ctx);
    click(&ctx, &mut app, artboard.center(), show);
    assert_eq!(crate::control::inspect(&app, &ctx)["ui"]["align_to"], "artboard");
    let back: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
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
    let ctx = egui::Context::default();
    let ([_, key, _], left) = panel_buttons(&mut app, &ctx);
    let clickable = ctx.viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).any(|w| w.rect == key && w.sense.senses_click()));
    assert!(!clickable, "disabled");
    click(&ctx, &mut app, key.center(), show);
    assert_eq!(app.session.active().unwrap().selection.key, None);
    assert_eq!(align_to(&app), AlignTo::Selection);
    click(&ctx, &mut app, left.center(), show);
    assert_eq!(left_edge(&app, &front), 0.0, "the artboard's left edge");
}

/// One frame of the Align panel → the rects of the Align To buttons drawn as chosen.
fn chosen_align_to(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<egui::Rect> {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
    out.textures_delta.clear();
    let fill = crate::theme::Tokens::get(ctx).tool_active;
    let chosen = |s: &egui::Shape| match s {
        egui::Shape::Rect(r) if r.fill == fill && r.rect.size() == egui::vec2(28.0, 28.0) => Some(r.rect),
        _ => None,
    };
    out.shapes.iter().filter_map(|c| chosen(&c.shape)).collect()
}

/// With one object selected the align buttons align it to the artboard, and Align To shows
/// Artboard; with two, it shows the stored choice.
#[test]
fn align_to_shows_artboard_while_one_object_is_selected() {
    let (mut app, _, front) = back_and_front();
    let ctx = egui::Context::default();
    let ([selection, _, artboard], _) = panel_buttons(&mut app, &ctx);
    assert_eq!(chosen_align_to(&mut app, &ctx), [selection], "two objects");
    app.run("select.set", json!({"ids": [front]})).unwrap();
    assert_eq!(chosen_align_to(&mut app, &ctx), [artboard], "one object");
    assert_eq!(align_to(&app), AlignTo::Selection, "the stored choice");
}

/// Screen readers name the align buttons, with no pressed state, and the Align To buttons, the
/// chosen one pressed.
#[test]
fn screen_readers_name_the_buttons_and_the_align_to_choice() {
    let (mut app, _, _) = back_and_front();
    let tree = tree(|ui| show(&mut app, ui));
    for (_, label, _, _) in ALIGN.iter().chain(DISTRIBUTE.iter()) {
        assert_eq!(role_and_state(&tree, label), (Role::Button, None), "{label}");
    }
    assert_eq!(role_and_state(&tree, "Align to Selection"), (Role::Button, Some(Toggled::True)));
    assert_eq!(role_and_state(&tree, "Align to Key Object"), (Role::Button, None));
    assert_eq!(role_and_state(&tree, "Align to Artboard"), (Role::Button, None));
}

/// #789: the Control bar's and the Properties panel's align buttons follow Align To too.
#[test]
fn the_other_align_buttons_align_to_the_align_to_choice() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
    let b = app.run("shape.rectangle", json!({"x": 80, "y": 50, "width": 30, "height": 30})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let ctx = egui::Context::default();
    app.ui.align_to = AlignTo::Artboard;
    // One frame finds the first button (Horizontal Align Right is third); the next clicks it.
    let at = std::cell::Cell::new(egui::Pos2::ZERO);
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            ui.horizontal(|ui| {
                at.set(ui.cursor().min);
                align_buttons(app, ui, 24.0);
            });
        });
        out.textures_delta.clear();
    };
    frame(&mut app, vec![]);
    let right = at.get() + egui::vec2(2.0 * (24.0 + ctx.global_style().spacing.item_spacing.x) + 12.0, 12.0);
    let press = |pressed| egui::Event::PointerButton { pos: right, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, vec![egui::Event::PointerMoved(right), press(true)]);
    frame(&mut app, vec![press(false)]);
    let st = app.session.active().unwrap();
    let x1 = |id: &serde_json::Value| st.doc.bounds_of(&[vectorcraft_doc::NodeId(id.as_u64().unwrap())], false).unwrap().x1;
    assert_eq!((x1(&a), x1(&b)), (200.0, 200.0), "both to the artboard's right edge");
}

/// The Align panel's buttons, in the panel's order: (command id, label).
const BUTTON_ENTRIES: [(&str, &str); 15] = [
    ("align.horizontalLeft", "Horizontal Align Left"),
    ("align.horizontalCenter", "Horizontal Align Center"),
    ("align.horizontalRight", "Horizontal Align Right"),
    ("align.horizontalVerticalCenter", "Horizontal & Vertical Align Center"),
    ("align.verticalTop", "Vertical Align Top"),
    ("align.verticalCenter", "Vertical Align Center"),
    ("align.verticalBottom", "Vertical Align Bottom"),
    ("align.distributeVerticalTop", "Vertical Distribute Top"),
    ("align.distributeVerticalCenter", "Vertical Distribute Center"),
    ("align.distributeVerticalBottom", "Vertical Distribute Bottom"),
    ("align.distributeHorizontalLeft", "Horizontal Distribute Left"),
    ("align.distributeHorizontalCenter", "Horizontal Distribute Center"),
    ("align.distributeHorizontalRight", "Horizontal Distribute Right"),
    ("align.distributeVerticalSpace", "Vertical Distribute Space"),
    ("align.distributeHorizontalSpace", "Horizontal Distribute Space"),
];

/// The chord the tests below give an Align panel button.
const CHORD: &str = "Cmd+Alt+Shift+F2";

/// One frame pressing [`CHORD`] through the shortcut handler.
fn press_chord(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    let modifiers = egui::Modifiers::COMMAND | egui::Modifiers::ALT | egui::Modifiers::SHIFT;
    let ev = egui::Event::Key { key: egui::Key::F2, physical_key: None, pressed: true, repeat: false, modifiers };
    let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
    out.textures_delta.clear();
}

/// Edit › Keyboard Shortcuts lists each of the Align panel's Align Objects, Distribute Objects
/// and Distribute Spacing buttons under Align, with the button's label and no default shortcut,
/// and each can be given one.
#[test]
fn keyboard_shortcuts_lists_each_align_panel_button() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    let panel = {
        let (mut app, _, _) = back_and_front();
        tree(|ui| show(&mut app, ui))
    };
    let list = app.run("shortcuts.list", json!({})).unwrap();
    for (id, label) in BUTTON_ENTRIES {
        assert_eq!(role_and_state(&panel, label), (Role::Button, None), "the panel's {label} button");
        let e = list.as_array().unwrap().iter().find(|e| e["id"] == id).unwrap_or_else(|| panic!("{id} is not listed"));
        assert_eq!(
            (e["label"].as_str(), e["group"].as_str(), &e["default"], &e["shortcut"]),
            (Some(label), Some("Align"), &Value::Null, &Value::Null),
            "{id}"
        );
        let set = app.run("shortcuts.set", json!({"id": id, "shortcut": CHORD, "force": true})).unwrap();
        assert_eq!(set["shortcut"], CHORD, "{id}");
    }
}

/// A shortcut on an Align panel button's entry runs what a click on the button runs: Horizontal
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
    app.run("shortcuts.set", json!({"id": "align.horizontalRight", "shortcut": CHORD})).unwrap();
    press_chord(&mut app);
    assert_eq!(edges(&app, true), [140.0; 3], "Align to Selection: {}", app.ui.status);
    app.run("edit.undo", json!({})).unwrap();
    app.ui.align_to = AlignTo::Artboard;
    press_chord(&mut app);
    assert_eq!(edges(&app, true), [200.0; 3], "Align to Artboard: {}", app.ui.status);
    app.run("edit.undo", json!({})).unwrap();
    app.run("select.key", json!({"id": ids[1]})).unwrap();
    press_chord(&mut app);
    assert_eq!(edges(&app, true), [90.0; 3], "the key object's right edge: {}", app.ui.status);
    app.run("edit.undo", json!({})).unwrap();
    // Distribute Objects with the key object and a Distribute Spacing value of 15 pt.
    app.run("shortcuts.set", json!({"id": "align.distributeHorizontalLeft", "shortcut": CHORD, "force": true})).unwrap();
    app.ui.align_spacing = 15.0;
    press_chord(&mut app);
    assert_eq!(sent(&app), ("object.distribute".to_string(), json!("key"), Some(json!(15.0))));
    assert_eq!(edges(&app, false), [35.0, 50.0, 65.0], "15 pt apart: {}", app.ui.status);
    app.run("edit.undo", json!({})).unwrap();
    // Distribute Spacing with Align to Selection: the outer objects stay, and the gaps are equal.
    app.run("select.key", json!({})).unwrap();
    app.ui.align_to = AlignTo::Selection;
    app.run("shortcuts.set", json!({"id": "align.distributeHorizontalSpace", "shortcut": CHORD, "force": true})).unwrap();
    press_chord(&mut app);
    assert_eq!(sent(&app), ("object.distributeSpacing".to_string(), json!("selection"), Some(Value::Null)));
    assert_eq!(edges(&app, false), [10.0, 55.0, 120.0], "25 pt gaps: {}", app.ui.status);
}

/// A shortcut on an Align panel button's entry is kept in the set Export… writes, and Import…
/// of that set gives it back after a reset; it is kept in the saved preferences too.
#[test]
fn an_align_panel_button_shortcut_is_kept_in_a_saved_set() {
    let (mut app, back, front) = back_and_front();
    let written: std::rc::Rc<std::cell::RefCell<Vec<u8>>> = Default::default();
    let w = written.clone();
    app.services.write = Some(Box::new(move |_: &str, b: &[u8]| {
        *w.borrow_mut() = b.to_vec();
        Ok(())
    }));
    let top = |app: &VectorcraftApp, id: &Value| {
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().geometric_bounds().unwrap().y0
    };
    app.run("shortcuts.set", json!({"id": "align.verticalTop", "shortcut": CHORD})).unwrap();
    app.run("shortcuts.export", json!({"path": "/sets/Align.json"})).unwrap();
    // The preferences save the UI state, and the next launch reads it back.
    let saved = serde_json::to_value(&app.ui).unwrap();
    let loaded: crate::UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(serde_json::to_value(&loaded).unwrap()["shortcut_overrides"]["align.verticalTop"], CHORD, "the saved preferences");
    app.run("shortcuts.reset", json!({})).unwrap();
    press_chord(&mut app);
    assert_eq!(top(&app, &front), 50.0, "no shortcut after the reset: {}", app.ui.status);
    let set: Value = serde_json::from_slice(&written.borrow()).unwrap();
    app.run("shortcuts.import", json!({"data": set})).unwrap();
    press_chord(&mut app);
    assert_eq!((top(&app, &back), top(&app, &front)), (10.0, 10.0), "the imported shortcut: {}", app.ui.status);
}

/// One frame of the Align panel → its Horizontal Distribute Left and Horizontal Distribute
/// Space buttons.
fn distribute_buttons(app: &mut VectorcraftApp, ctx: &egui::Context) -> (egui::Rect, egui::Rect) {
    frame_events(ctx, app, vec![], show);
    // The lower row of 32-point buttons is Distribute Objects': Vertical Distribute Top, Center
    // and Bottom, then Horizontal Distribute Left, Center and Right.
    let objects = widget_rects(ctx, egui::vec2(32.0, 32.0));
    let bottom = objects.last().unwrap().top();
    let mut row: Vec<egui::Rect> = objects.into_iter().filter(|r| r.top() == bottom).collect();
    row.sort_by(|a, b| a.left().total_cmp(&b.left()));
    // The 28-point buttons are Distribute Spacing's two (Vertical, then Horizontal) and then
    // Align To's three.
    let mut small = widget_rects(ctx, egui::vec2(28.0, 28.0));
    small.sort_by(|a, b| a.left().total_cmp(&b.left()));
    (row[3], small[1])
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
    let ctx = egui::Context::default();
    let (left, space) = distribute_buttons(&mut app, &ctx);
    click(&ctx, &mut app, left.center(), show);
    assert_eq!(lefts(&app), [10.0, 65.0, 120.0], "Align to Selection: {}", app.ui.status);
    app.run("edit.undo", json!({})).unwrap();
    // The objects are on artboard 0, and artboard 1 is the active one.
    app.ui.align_to = AlignTo::Artboard;
    app.view_mut().unwrap().artboard = 1;
    let (x0, x1) = (boards[1].x0, boards[1].x1);
    click(&ctx, &mut app, left.center(), show);
    assert_eq!(lefts(&app), [x0, x0 + 90.0, x1 - 20.0], "the active artboard's edges: {}", app.ui.status);
    app.run("edit.undo", json!({})).unwrap();
    click(&ctx, &mut app, space.center(), show);
    assert_eq!(lefts(&app), [x0, x0 + 80.0, x1 - 20.0], "60 pt gaps: {}", app.ui.status);
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
    let ctx = egui::Context::default();
    let (left, _) = distribute_buttons(&mut app, &ctx);
    click(&ctx, &mut app, left.center(), show);
    assert_eq!(sent(&app), ("object.distribute".to_string(), json!("selection"), None), "no key object");
    assert_eq!(lefts(&app), [10.0, 65.0, 120.0], "{}", app.ui.status);
    app.run("edit.undo", json!({})).unwrap();
    app.run("select.key", json!({"id": ids[1]})).unwrap();
    click(&ctx, &mut app, left.center(), show);
    assert_eq!(sent(&app), ("object.distribute".to_string(), json!("key"), Some(json!(0.0))));
    assert_eq!(lefts(&app), [50.0; 3], "on the key's left edge: {}", app.ui.status);
    app.run("edit.undo", json!({})).unwrap();
    app.ui.align_spacing = 15.0;
    click(&ctx, &mut app, left.center(), show);
    assert_eq!(lefts(&app), [35.0, 50.0, 65.0], "15 pt apart: {}", app.ui.status);
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
    let ctx = egui::Context::default();
    let (left, space) = distribute_buttons(&mut app, &ctx);
    click(&ctx, &mut app, left.center(), show);
    assert_eq!(sent(&app), ("object.distribute".to_string(), json!("key"), Some(json!(0.0))));
    assert_eq!(lefts(&app), [100.0; 3], "on the guide: {}", app.ui.status);
    app.run("edit.undo", json!({})).unwrap();
    app.ui.align_spacing = 15.0;
    click(&ctx, &mut app, left.center(), show);
    assert_eq!(lefts(&app), [70.0, 85.0, 115.0], "15 pt apart, outward from the guide: {}", app.ui.status);
    assert_eq!(app.session.active().unwrap().doc.guides[0].pos, 100.0, "the guide stays put");
    // Horizontal Distribute Space with 15: the object before the guide ends 15 pt before it,
    // the one after it starts 15 pt after it.
    click(&ctx, &mut app, space.center(), show);
    assert_eq!(sent(&app), ("object.distributeSpacing".to_string(), json!("key"), Some(json!(15.0))));
    assert_eq!(lefts(&app), [10.0, 45.0, 115.0], "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().doc.guides[0].pos, 100.0, "the guide stays put");
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
    let ctx = egui::Context::default();
    frame_events(&ctx, &mut app, vec![], show);
    let row = widget_rects(&ctx, egui::vec2(32.0, 32.0));
    let top = row.iter().map(|r| r.top()).fold(f32::MAX, f32::min);
    let mut row: Vec<egui::Rect> = row.into_iter().filter(|r| r.top() == top).collect();
    row.sort_by(|a, b| a.left().total_cmp(&b.left()));
    assert!(row[0].width() > 0.0, "Horizontal Align Left is enabled");
    click(&ctx, &mut app, row[0].center(), show);
    let pos = |app: &VectorcraftApp| app.session.active().unwrap().doc.guides.iter().map(|g| g.pos).collect::<Vec<_>>();
    assert_eq!(pos(&app), [0.0, 0.0, 0.0], "{}", app.ui.status);
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
    let ctx = egui::Context::default();
    frame_events(&ctx, &mut app, vec![], show);
    let row = widget_rects(&ctx, egui::vec2(32.0, 32.0));
    let bottom = row.iter().map(|r| r.top()).fold(f32::MIN, f32::max);
    let mut row: Vec<egui::Rect> = row.into_iter().filter(|r| r.top() == bottom).collect();
    row.sort_by(|a, b| a.left().total_cmp(&b.left()));
    assert_eq!(row.len(), 6, "the Distribute row");
    // Its fifth button is Horizontal Distribute Center.
    click(&ctx, &mut app, row[4].center(), show);
    let pos = |app: &VectorcraftApp| app.session.active().unwrap().doc.guides.iter().map(|g| g.pos).collect::<Vec<_>>();
    assert_eq!(pos(&app), [20.0, 100.0, 180.0], "{}", app.ui.status);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(pos(&app), [20.0, 50.0, 180.0], "one undo step");
    // With Align to Artboard, the outermost guides go to its edges: equal columns across it.
    app.ui.align_to = AlignTo::Artboard;
    click(&ctx, &mut app, row[4].center(), show);
    assert_eq!(pos(&app), [0.0, 100.0, 200.0], "{}", app.ui.status);
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
    let ctx = egui::Context::default();
    let ([selection, _, artboard], left) = panel_buttons(&mut app, &ctx);
    click(&ctx, &mut app, left.center(), show);
    assert_eq!(xs(&app), [100.0, 100.0, 150.0], "to the points' left edge: {}", app.ui.status);
    assert_eq!(chosen_align_to(&mut app, &ctx), [selection], "the stored choice");
    app.run("edit.undo", json!({})).unwrap();
    click(&ctx, &mut app, artboard.center(), show);
    click(&ctx, &mut app, left.center(), show);
    assert_eq!(xs(&app), [0.0, 0.0, 150.0], "to the artboard's left edge: {}", app.ui.status);
}

/// #1636: the Distribute buttons work on the anchor points of one path (one object selected).
#[test]
fn distribute_buttons_space_the_anchor_points_of_one_path() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = app.run("path.create", json!({"d": "M10 10 L20 50 L100 30"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 0], [0, 1], [0, 2]], "mode": "set"})).unwrap();
    let ctx = egui::Context::default();
    frame_events(&ctx, &mut app, vec![], show);
    // The 32-point buttons are the Align row and, below it, the Distribute row, whose fifth
    // button is Horizontal Distribute Center.
    let all = widget_rects(&ctx, egui::vec2(32.0, 32.0));
    let top = all.iter().map(|r| r.top()).fold(f32::MIN, f32::max);
    let mut row: Vec<_> = all.into_iter().filter(|r| r.top() == top).collect();
    row.sort_by(|a, b| a.left().total_cmp(&b.left()));
    assert_eq!(row.len(), 6, "the Distribute row");
    let hit = row[4].center();
    click(&ctx, &mut app, hit, show);
    let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
    let xs: Vec<f64> = node.path_data().unwrap().anchors().map(|(_, _, a)| a.p.x).collect();
    assert_eq!(xs, [10.0, 55.0, 100.0], "{}", app.ui.status);
}

/// #4231: the panel menu's Align to Glyph Bounds is a submenu (no longer dimmed) whose Point Text
/// and Area Text each toggle their own preference and show a check while it is on.
#[test]
fn align_to_glyph_bounds_toggles_point_and_area_text() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    assert!(frame_events(&ctx, &mut app, vec![], menu).iter().any(|(t, _)| t.trim() == "Align to Glyph Bounds"));
    let checked = |app: &mut VectorcraftApp| {
        let texts = frame_events(&ctx, app, vec![], glyph_bounds_items);
        texts.iter().filter_map(|(t, _)| t.strip_prefix("✓ ")).map(str::to_string).collect::<Vec<_>>()
    };
    let choose = |app: &mut VectorcraftApp, label: &str| {
        let texts = frame_events(&ctx, app, vec![], glyph_bounds_items);
        let r = texts.iter().find(|(t, _)| t.trim_start_matches('✓').trim() == label).unwrap_or_else(|| panic!("{label} in {texts:?}")).1;
        click(&ctx, app, r.center(), glyph_bounds_items);
    };
    let prefs = |app: &VectorcraftApp| (app.session.prefs.align_glyph_bounds_point, app.session.prefs.align_glyph_bounds_area);
    assert!(checked(&mut app).is_empty(), "both off by default");
    choose(&mut app, "Point Text");
    assert_eq!((prefs(&app), checked(&mut app)), ((true, false), vec!["Point Text".to_string()]));
    choose(&mut app, "Area Text");
    assert_eq!((prefs(&app), checked(&mut app)), ((true, true), vec!["Point Text".to_string(), "Area Text".to_string()]));
    choose(&mut app, "Point Text");
    assert_eq!((prefs(&app), checked(&mut app)), ((false, true), vec!["Area Text".to_string()]));
}
