use serde_json::json;
use vectorcraft_engine::Session;

use super::*;

/// Filled target circles drawn by one headless frame of the panel.
fn filled_targets(app: &mut VectorcraftApp, ctx: &egui::Context) -> usize {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
    out.textures_delta.clear();
    out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Circle(cs) if cs.radius == 3.2 && cs.fill != Color32::TRANSPARENT)).count()
}

/// A generated `<Kind>` name is translated where painted; an unnamed text object's text never
/// is, even when it reads like one.
#[test]
fn only_generated_names_are_translated() {
    let zh = crate::i18n::Lang::from_code("zh-hant").unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let rect = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    let text = run(&mut app, "text.create", json!({"x": 10, "y": 40, "text": "<Path>"}))["id"].as_u64().unwrap();
    let doc = &app.session.active().unwrap().doc;
    let names = |id: u64| {
        let n = doc.node(NodeId(id)).unwrap();
        (n.display_name(), painted_name(n, &n.display_name(), zh))
    };
    assert_eq!(names(text), ("<Path>".to_string(), "<Path>".to_string()));
    let (stored, painted) = names(rect);
    let inner = stored.trim_start_matches('<').trim_end_matches('>');
    assert_eq!(painted, format!("<{}>", crate::i18n::tr(zh, inner)));
}

#[test]
fn target_circle_fills_for_non_default_transparency() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let id = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].clone();
    run(&mut app, "select.set", json!({ "ids": [id] }));
    let ctx = egui::Context::default();
    assert_eq!(filled_targets(&mut app, &ctx), 0);
    run(&mut app, "transparency.set", json!({"blend": "multiply"}));
    assert_eq!(filled_targets(&mut app, &ctx), 1, "a Multiply object");
    run(&mut app, "transparency.set", json!({"blend": "normal", "knockout": true}));
    assert_eq!(filled_targets(&mut app, &ctx), 1, "a knockout group");
    run(&mut app, "transparency.set", json!({"knockout": false}));
    run(&mut app, "appearance.addStroke", json!({}));
    assert_eq!(filled_targets(&mut app, &ctx), 1, "two strokes");
}

#[test]
fn a_layer_clipping_mask_underlines_its_clipping_path() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100}));
    run(&mut app, "shape.rectangle", json!({"x": 25, "y": 25, "width": 50, "height": 50}));
    run(&mut app, "select.none", json!({}));
    let ctx = egui::Context::default();
    let text = Tokens::get(&ctx).text;
    // Line segments in the text colour: the underlines.
    let underlines = |app: &mut VectorcraftApp| {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::LineSegment { stroke, .. } if stroke.color == text)).count()
    };
    assert_eq!(underlines(&mut app), 0);
    assert_eq!(run(&mut app, "layer.clippingMask.toggle", json!({}))["clip"], true);
    assert_eq!(underlines(&mut app), 1);
    run(&mut app, "layer.clippingMask.toggle", json!({}));
    assert_eq!(underlines(&mut app), 0);
}

/// One headless frame of the panel with `events` (Alt held when `alt`): the centres of the
/// target circles, top row first, and the text-coloured line segments (underlines) drawn.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, alt: bool) -> (Vec<egui::Pos2>, usize) {
    let mut events = events;
    events.insert(0, egui::Event::ModifiersChanged(egui::Modifiers { alt, ..Default::default() }));
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui));
    out.textures_delta.clear();
    let text = Tokens::get(ctx).text;
    let mut circles = vec![];
    let mut lines = 0;
    for c in &out.shapes {
        match &c.shape {
            egui::Shape::Circle(cs) if cs.radius == 5.0 => circles.push(cs.center),
            egui::Shape::LineSegment { stroke, .. } if stroke.color == text => lines += 1,
            _ => {}
        }
    }
    circles.sort_by(|a, b| a.y.total_cmp(&b.y));
    (circles, lines)
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// Drag from `a` to `b` over a few frames (Alt held when `alt`).
fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, a: egui::Pos2, b: egui::Pos2, alt: bool) {
    let steps = [
        vec![egui::Event::PointerMoved(a)],
        vec![button(a, true)],
        vec![egui::Event::PointerMoved(a + vec2(0.0, 6.0))],
        vec![egui::Event::PointerMoved(b)],
        vec![button(b, false)],
        vec![],
    ];
    for e in steps {
        frame(app, ctx, e, alt);
    }
}

#[test]
fn alt_clicking_an_eye_or_lock_toggles_the_other_layers() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    run(&mut app, "layer.new", json!({}));
    run(&mut app, "layer.new", json!({}));
    let ctx = egui::Context::default();
    // Rows top first: Layers 3, 2, 1.
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    assert_eq!(circles.len(), 3);
    let state =
        |app: &VectorcraftApp| -> Vec<(bool, bool)> { app.session.active().unwrap().doc.layers.iter().map(|l| (l.visible, l.locked)).collect() };
    let click = |app: &mut VectorcraftApp, at: egui::Pos2, alt: bool| {
        frame(app, &ctx, vec![egui::Event::PointerMoved(at)], alt);
        frame(app, &ctx, vec![button(at, true)], alt);
        frame(app, &ctx, vec![button(at, false)], alt);
    };
    let (eye, lock) = (egui::pos2(12.0, circles[1].y), egui::pos2(37.0, circles[1].y));
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let before = undo(&app);
    // Alt-click Layer 2's eye: Layers 1 and 3 hide in one step; again, they show.
    click(&mut app, eye, true);
    assert_eq!(state(&app), [(false, false), (true, false), (false, false)]);
    assert_eq!(undo(&app), before + 1);
    click(&mut app, eye, true);
    assert_eq!(state(&app), [(true, false); 3]);
    // Alt-click its lock: the others lock; again, every layer unlocks.
    click(&mut app, lock, true);
    assert_eq!(state(&app), [(true, true), (true, false), (true, true)]);
    assert_eq!(app.session.active().unwrap().history.undo.last().map(|u| u.label.as_str()), Some("Lock Others"));
    // A plain click still toggles only that layer.
    click(&mut app, eye, false);
    assert_eq!(state(&app), [(true, true), (false, false), (true, true)]);
    click(&mut app, lock, true);
    assert_eq!(state(&app), [(true, false), (false, false), (true, false)]);
    // A sublayer's row stands for its top-level layer: the other layers hide.
    click(&mut app, eye, false);
    let layer2 = app.session.active().unwrap().doc.layers[1].id.0;
    let sub = run(&mut app, "layer.newSublayer", json!({"parent": layer2}));
    // Rows: Layer 3, Layer 2, its sublayer, Layer 1.
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    assert_eq!(circles.len(), 4);
    click(&mut app, egui::pos2(12.0, circles[2].y), true);
    assert_eq!(state(&app), [(false, false), (true, false), (false, false)]);
    let doc = &app.session.active().unwrap().doc;
    assert!(doc.node(NodeId(sub["id"].as_u64().unwrap())).unwrap().visible);
}

#[test]
fn ctrl_alt_clicking_an_eye_switches_the_other_layers_between_outline_and_preview() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    run(&mut app, "layer.new", json!({}));
    run(&mut app, "layer.new", json!({}));
    let ctx = egui::Context::default();
    // Rows top first: Layers 3, 2, 1.
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    assert_eq!(circles.len(), 3);
    // Each layer bottom first: (visible, preview).
    let state = |app: &VectorcraftApp| -> Vec<(bool, bool)> {
        let doc = &app.session.active().unwrap().doc;
        doc.layers.iter().map(|l| (l.visible, !matches!(l.kind, NodeKind::Layer { preview: false, .. }))).collect()
    };
    let last = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.last().map(|u| u.label.clone());
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let eye = |i: usize| egui::pos2(12.0, circles[i].y);
    let ctrl_alt = egui::Modifiers { alt: true, command: true, ..Default::default() };
    let before = undo(&app);
    // Ctrl/Cmd+Alt-click Layer 1's eye: Layers 2 and 3 go to Outline in one step, and stay visible.
    click_with(&mut app, &ctx, eye(2), ctrl_alt);
    assert_eq!(state(&app), [(true, true), (true, false), (true, false)]);
    assert_eq!(undo(&app), before + 1);
    assert_eq!(last(&app).as_deref(), Some("Outline Others"));
    // Again: every layer previews, in one step.
    click_with(&mut app, &ctx, eye(2), ctrl_alt);
    assert_eq!(state(&app), [(true, true); 3]);
    assert_eq!(undo(&app), before + 2);
    assert_eq!(last(&app).as_deref(), Some("Preview All Layers"));
    // On a layer in Outline whose others preview: the others go to Outline and it previews.
    click_with(&mut app, &ctx, eye(0), egui::Modifiers::COMMAND);
    assert_eq!(state(&app), [(true, true), (true, true), (true, false)]);
    click_with(&mut app, &ctx, eye(0), ctrl_alt);
    assert_eq!(state(&app), [(true, false), (true, false), (true, true)]);
}

/// A document with two rectangles on one layer → (app, layer, [bottom, top]).
fn two_rects() -> (VectorcraftApp, NodeId, [NodeId; 2]) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let a = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    let b = run(&mut app, "shape.rectangle", json!({"x": 60, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    run(&mut app, "select.none", json!({}));
    let layer = app.session.active().unwrap().doc.layers[0].id;
    (app, layer, [NodeId(a), NodeId(b)])
}

#[test]
fn row_thumbnails_wait_for_the_end_of_a_drag() {
    let (mut app, _, _) = two_rects();
    app.session.execute("select.all", &json!({})).unwrap();
    let ctx = egui::Context::default();
    let renders = || THUMBS.with(|c| c.borrow().renders);
    frame(&mut app, &ctx, vec![], false);
    let first = renders();
    assert!(first >= 1, "the rows' thumbnails render at once");
    frame(&mut app, &ctx, vec![], false);
    assert_eq!(renders(), first, "and stay");
    // A move drag: the layer (and the moved objects) change on every preview.
    app.session.begin_interaction("Move").unwrap();
    for i in 1..=10 {
        app.session.preview("object.transform", &json!({"matrix": [1, 0, 0, 1, i, 0]})).unwrap();
        frame(&mut app, &ctx, vec![], false);
    }
    assert_eq!(renders(), first, "nothing renders while the drag goes on");
    app.session.commit_interaction().unwrap();
    for _ in 0..12 {
        frame(&mut app, &ctx, vec![], false);
    }
    assert!(renders() > first, "the changed rows render after it");
    let after = renders();
    frame(&mut app, &ctx, vec![], false);
    assert_eq!(renders(), after, "once");
}

#[test]
fn clicking_a_target_circle_targets_the_layer() {
    let (mut app, layer, [a, b]) = two_rects();
    let ctx = egui::Context::default();
    // Rows top down: the layer, the top rectangle, the bottom one.
    let (c, _) = frame(&mut app, &ctx, vec![], false);
    assert_eq!(c.len(), 3);
    for e in [egui::Event::PointerMoved(c[0]), button(c[0], true), button(c[0], false)] {
        frame(&mut app, &ctx, vec![e], false);
    }
    let st = app.session.active().unwrap();
    assert_eq!(st.selection.target, Some(layer));
    assert_eq!(st.selection.objects, vec![a, b]);
    assert_eq!(super::super::appearance::object_label(&app), "Layer", "the Appearance panel lists the layer");
    app.session.execute("transparency.set", &json!({"opacity": 25})).unwrap();
    assert!((app.session.active().unwrap().doc.node(layer).unwrap().opacity - 0.25).abs() < 1e-6);
    // The layer's circle is filled now; clicking an object's circle targets that object.
    assert_eq!(filled_targets(&mut app, &ctx), 1);
    for e in [egui::Event::PointerMoved(c[2]), button(c[2], true), button(c[2], false)] {
        frame(&mut app, &ctx, vec![e], false);
    }
    let st = app.session.active().unwrap();
    assert_eq!((st.selection.target, st.selection.objects.clone()), (Some(a), vec![a]));
    // Shift-clicking another object's circle adds it to the selection (#901), and again takes
    // it out; nothing is targeted meanwhile.
    click_with(&mut app, &ctx, c[1], egui::Modifiers::SHIFT);
    let st = app.session.active().unwrap();
    assert_eq!((st.selection.target, st.selection.objects.clone()), (None, vec![a, b]));
    click_with(&mut app, &ctx, c[1], egui::Modifiers::SHIFT);
    assert_eq!(app.session.active().unwrap().selection.objects, vec![a]);
}

#[test]
fn dragging_a_target_circle_moves_or_copies_the_appearance_and_the_trash_clears_it() {
    let (mut app, layer, [a, b]) = two_rects();
    app.session.execute("transparency.set", &json!({"ids": [a.0], "opacity": 30})).unwrap();
    let ctx = egui::Context::default();
    let (c, _) = frame(&mut app, &ctx, vec![], false);
    let opacity = |app: &VectorcraftApp, id: NodeId| app.session.active().unwrap().doc.node(id).unwrap().opacity;
    // Alt-drag the bottom rectangle's circle onto the top one's: both have it.
    drag(&mut app, &ctx, c[2], c[1], true);
    assert!((opacity(&app, b) - 0.3).abs() < 1e-6 && (opacity(&app, a) - 0.3).abs() < 1e-6);
    // A plain drag onto the layer's circle moves it there.
    drag(&mut app, &ctx, c[1], c[0], false);
    assert!((opacity(&app, layer) - 0.3).abs() < 1e-6 && opacity(&app, b) == 1.0);
    // Dropped on the trash, the layer's appearance is cleared (and nothing is deleted).
    let st = app.session.active().unwrap();
    let count = st.doc.node_count();
    let trash = trash_pos(&mut app, &ctx);
    drag(&mut app, &ctx, c[0], trash, false);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.node(layer).unwrap().opacity, 1.0);
    assert_eq!(st.doc.node_count(), count);
}

/// The trash button: the bottom bar's rightmost button.
fn trash_pos(app: &mut VectorcraftApp, ctx: &egui::Context) -> egui::Pos2 {
    bar_buttons(app, ctx)[0]
}

#[test]
fn dragging_rows_onto_the_trash_deletes_them() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("layer.new", &json!({})).unwrap();
    let ctx = egui::Context::default();
    // Rows top first: Layer 2, Layer 1. Grab Layer 2 by its name, left of its target circle.
    let (c, _) = frame(&mut app, &ctx, vec![], false);
    let top = app.session.active().unwrap().doc.layers[1].id;
    let trash = trash_pos(&mut app, &ctx);
    drag(&mut app, &ctx, c[0] - vec2(100.0, 0.0), trash, false);
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 1);
    assert!(doc.node(top).is_none());
}

#[test]
fn renaming_a_layer_commits_on_enter_and_escape_cancels() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let layer = app.session.doc().unwrap().doc.layers[0].id;
    let name = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].display_name();
    let ctx = egui::Context::default();
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let select_all = egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
    // What a double-click on the row does: the name field opens with the name.
    let open = |app: &mut VectorcraftApp| {
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique("layers-rename"), (layer.0, name(app))));
        frame(app, &ctx, vec![], false);
        frame(app, &ctx, vec![], false);
    };
    open(&mut app);
    frame(&mut app, &ctx, vec![select_all.clone(), egui::Event::Text("Sky".into())], false);
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)], false);
    frame(&mut app, &ctx, vec![], false);
    assert_eq!(name(&app), "Sky", "Enter keeps the new name");
    assert!(ctx.data(|d| d.get_temp::<(u64, String)>(egui::Id::unique("layers-rename"))).is_none(), "and closes the field");
    open(&mut app);
    frame(&mut app, &ctx, vec![select_all, egui::Event::Text("Ground".into())], false);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)], false);
    frame(&mut app, &ctx, vec![], false);
    assert_eq!(name(&app), "Sky", "Escape cancels");
}

/// Shift+Enter in the name field renames the layer and keeps the field open, the new name
/// selected; Enter then renames it again and closes the field.
#[test]
fn shift_enter_renames_a_layer_and_keeps_the_field_open() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let layer = app.session.doc().unwrap().doc.layers[0].id;
    let name = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].display_name();
    let ctx = egui::Context::default();
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let field = rename_id().with("field");
    ctx.data_mut(|d| d.insert_temp(rename_id(), (layer.0, name(&app))));
    frame(&mut app, &ctx, vec![], false);
    frame(&mut app, &ctx, vec![], false);
    frame(&mut app, &ctx, vec![key(egui::Key::A, egui::Modifiers::COMMAND), egui::Event::Text("Sky".into())], false);
    frame(&mut app, &ctx, vec![key(egui::Key::Enter, egui::Modifiers::SHIFT)], false);
    assert_eq!(name(&app), "Sky", "Shift+Enter applies the name");
    assert!(ctx.memory(|m| m.has_focus(field)), "the field keeps the focus");
    assert!(ctx.data(|d| d.get_temp::<(u64, String)>(rename_id())).is_some(), "and stays open");
    frame(&mut app, &ctx, vec![], false);
    let selected = egui::TextEdit::load_state(&ctx, field).and_then(|st| st.cursor.char_range()).map(|r| r.as_sorted_char_range());
    assert_eq!(selected.map(|r| (r.start.0, r.end.0)), Some((0, 3)), "the name shown is selected");
    frame(&mut app, &ctx, vec![egui::Event::Text("Sea".into())], false);
    frame(&mut app, &ctx, vec![key(egui::Key::Enter, egui::Modifiers::NONE)], false);
    frame(&mut app, &ctx, vec![], false);
    assert_eq!(name(&app), "Sea", "typing replaced it, and Enter applies that");
    assert!(ctx.data(|d| d.get_temp::<(u64, String)>(rename_id())).is_none(), "and closes the field");
}

#[test]
fn an_open_rename_does_not_follow_into_another_document() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let layer = app.session.doc().unwrap().doc.layers[0].id;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![], false);
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("layers-rename"), (layer.0, "Renamed in the first".to_string())));
    frame(&mut app, &ctx, vec![], false);
    // A second document, whose first layer has the same node id, comes to the front.
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc.layers[0].id, layer);
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    frame(&mut app, &ctx, vec![], false);
    frame(&mut app, &ctx, vec![enter], false);
    frame(&mut app, &ctx, vec![], false);
    assert_eq!(app.session.doc().unwrap().doc.layers[0].display_name(), "Layer 1");
    assert!(ctx.data(|d| d.get_temp::<(u64, String)>(egui::Id::unique("layers-rename"))).is_none());
}

#[test]
fn masks_underline_dashed_and_mask_editing_lists_only_the_mask() {
    let (mut app, _, [a, _]) = two_rects();
    let ctx = egui::Context::default();
    let (_, before) = frame(&mut app, &ctx, vec![], false);
    app.session.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    app.session.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    // Editing the new (empty) mask: one row, `<Opacity Mask>`.
    let (c, _) = frame(&mut app, &ctx, vec![], false);
    assert_eq!(c.len(), 1);
    let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], show);
    assert!(texts.iter().any(|(t, _)| t == "<Opacity Mask>"), "{texts:?}");
    app.session.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
    let (c, after) = frame(&mut app, &ctx, vec![], false);
    assert_eq!(c.len(), 3);
    assert!(after > before + 1, "a dashed underline: {before} → {after}");
}

// ---- rows at every depth (layers, sublayers, groups, objects) ----

/// Layer 1 holding rectangle `c` and a sublayer holding rectangle `a` and a group of
/// rectangle `b`, every row open, nothing selected → (app, [layer, sub, g, b, a, c]): the rows
/// top down.
fn nested() -> (VectorcraftApp, [NodeId; 6]) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    let id = |v: serde_json::Value| NodeId(v["id"].as_u64().unwrap());
    run(&mut app, "file.new", json!({"width": 400, "height": 400}));
    let layer = app.session.active().unwrap().doc.layers[0].id;
    let c = id(run(&mut app, "shape.rectangle", json!({"x": 300, "y": 0, "width": 20, "height": 20})));
    let sub = id(run(&mut app, "layer.newSublayer", json!({"name": "Sub"})));
    let a = id(run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})));
    let b = id(run(&mut app, "shape.rectangle", json!({"x": 100, "y": 0, "width": 20, "height": 20})));
    run(&mut app, "select.set", json!({"ids": [b.0]}));
    let g = id(run(&mut app, "object.group", json!({})));
    run(&mut app, "select.none", json!({}));
    expand(&mut app, &json!({})).unwrap();
    (app, [layer, sub, g, b, a, c])
}

/// Press and release at `at` with `mods` held, over three frames.
fn click_with(app: &mut VectorcraftApp, ctx: &egui::Context, at: egui::Pos2, mods: egui::Modifiers) {
    for e in [egui::Event::PointerMoved(at), button(at, true), button(at, false)] {
        let mut out = ctx.run_ui(egui::RawInput { events: vec![egui::Event::ModifiersChanged(mods), e], ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
    }
}

fn rows_of(app: &VectorcraftApp) -> Vec<NodeId> {
    app.session.active().unwrap().highlighted_rows()
}

#[test]
fn rows_at_every_depth_are_clickable() {
    let (mut app, [layer, sub, g, b, a, c]) = nested();
    let ctx = egui::Context::default();
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    assert_eq!(circles.len(), 6, "a row for the layer, the sublayer, the group, both rectangles in it and the one on the layer");
    let name = |i: usize| egui::pos2(150.0, circles[i].y);
    for (i, row, current) in [(1, sub, sub), (2, g, sub), (3, b, sub), (4, a, sub), (5, c, layer), (0, layer, layer)] {
        click_with(&mut app, &ctx, name(i), egui::Modifiers::NONE);
        let st = app.session.active().unwrap();
        assert_eq!((st.highlighted_rows(), st.current_layer()), (vec![row], Some(current)), "row {i}");
    }
    // New art goes into the current sublayer once its row is clicked.
    click_with(&mut app, &ctx, name(1), egui::Modifiers::NONE);
    let n = app.session.execute("shape.ellipse", &json!({"x": 0, "y": 100, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(app.session.active().unwrap().doc.parent_of(NodeId(n)), Some(sub));
    app.session.execute("edit.undo", &json!({})).unwrap();
    // Shift-click highlights the range down to the row; Ctrl/Cmd-click toggles one.
    click_with(&mut app, &ctx, name(4), egui::Modifiers::SHIFT);
    assert_eq!(rows_of(&app), vec![sub, g, b, a]);
    click_with(&mut app, &ctx, name(2), egui::Modifiers::COMMAND);
    assert_eq!(rows_of(&app), vec![sub, b, a]);
    click_with(&mut app, &ctx, name(5), egui::Modifiers::COMMAND);
    assert_eq!(rows_of(&app), vec![sub, b, a, c]);
    // Delete with the panel's keyboard: the highlighted rows go.
    let del = egui::Event::Key { key: egui::Key::Delete, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    frame(&mut app, &ctx, vec![del], false);
    let d = &app.session.active().unwrap().doc;
    assert!(d.node(sub).is_none() && d.node(c).is_none());
    assert_eq!(d.layers.len(), 1);
}

/// A layer's ruler guides are rows in it, above its art: a click selects the guide, a drag onto
/// another layer's row moves it there, and a guide on a locked layer is left alone.
#[test]
fn guide_rows_select_and_move_their_guides() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}));
    run(&mut app, "guide.add", json!({"vertical": true, "pos": 50}));
    run(&mut app, "select.none", json!({}));
    let second = NodeId(run(&mut app, "layer.new", json!({}))["id"].as_u64().unwrap());
    expand(&mut app, &json!({})).unwrap();
    let ctx = egui::Context::default();
    // Rows top down: Layer 2, Layer 1, its guide (no target circle), the rectangle.
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    assert_eq!(circles.len(), 3);
    let guide_row = egui::pos2(150.0, (circles[1].y + circles[2].y) / 2.0);
    let guides = |app: &VectorcraftApp| app.session.active().unwrap().selection.guides.clone();
    click_with(&mut app, &ctx, guide_row, egui::Modifiers::NONE);
    assert_eq!(guides(&app), [0], "its row selects it");
    // Dragged onto Layer 2's row, it goes on Layer 2, in one undo step.
    let undo = app.session.active().unwrap().history.undo.len();
    drag(&mut app, &ctx, guide_row, egui::pos2(150.0, circles[0].y), false);
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.guides[0].layer, st.history.undo.len()), (Some(second), undo + 1));
    // Now under Layer 2: locked, its row neither selects nor drags it.
    run(&mut app, "layer.setProps", json!({"ids": [second.0], "locked": true}));
    assert!(guides(&app).is_empty(), "deselected with its layer locked");
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    let guide_row = egui::pos2(150.0, (circles[0].y + circles[1].y) / 2.0);
    click_with(&mut app, &ctx, guide_row, egui::Modifiers::NONE);
    assert!(guides(&app).is_empty());
    drag(&mut app, &ctx, guide_row, egui::pos2(150.0, circles[2].y), false);
    assert_eq!(app.session.active().unwrap().doc.guides[0].layer, Some(second));
}

/// The selection squares drawn: (big ones on rows whose object is selected, small ones on rows
/// holding selected art).
fn squares(app: &mut VectorcraftApp, ctx: &egui::Context) -> (usize, usize) {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
    out.textures_delta.clear();
    let size = |w: f32| {
        out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Rect(r) if (r.rect.width() - w).abs() < 0.01 && (r.rect.height() - w).abs() < 0.01 && r.fill != Color32::TRANSPARENT)).count()
    };
    (size(7.0), size(4.0))
}

#[test]
fn selection_squares_show_and_select_a_rows_art() {
    let (mut app, [layer, sub, g, b, a, c]) = nested();
    let ctx = egui::Context::default();
    assert_eq!(squares(&mut app, &ctx), (0, 0));
    app.session.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    assert_eq!(squares(&mut app, &ctx), (1, 2), "the group's square, and small ones on the sublayer and the layer");
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    let square = |i: usize| circles[i] + vec2(TARGET_X - SQUARE_X, 0.0);
    let sel = |app: &VectorcraftApp| app.session.active().unwrap().selection.objects.clone();
    click_with(&mut app, &ctx, square(4), egui::Modifiers::NONE);
    assert_eq!(sel(&app), vec![a]);
    click_with(&mut app, &ctx, square(5), egui::Modifiers::SHIFT);
    assert_eq!(sel(&app), vec![a, c]);
    click_with(&mut app, &ctx, square(1), egui::Modifiers::NONE);
    assert_eq!(sel(&app), vec![a, g], "a sublayer's square selects its art");
    click_with(&mut app, &ctx, square(0), egui::Modifiers::NONE);
    assert_eq!(sel(&app), vec![c, a, g], "a layer's, its sublayers' too");
    let _ = (layer, sub, b);
}

#[test]
fn eye_and_lock_toggle_nested_rows_and_drag_down_the_column() {
    let (mut app, [_, sub, g, b, a, _]) = nested();
    let ctx = egui::Context::default();
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    let (eye, lock) = (|i: usize| egui::pos2(12.0, circles[i].y), |i: usize| egui::pos2(37.0, circles[i].y));
    let node = |app: &VectorcraftApp, id: NodeId| app.session.active().unwrap().doc.node(id).unwrap().clone();
    click_with(&mut app, &ctx, eye(3), egui::Modifiers::NONE);
    assert!(!node(&app, b).visible, "an object in a group in a sublayer hides");
    click_with(&mut app, &ctx, eye(3), egui::Modifiers::NONE);
    click_with(&mut app, &ctx, lock(2), egui::Modifiers::NONE);
    assert!(node(&app, g).locked, "a group locks");
    click_with(&mut app, &ctx, lock(2), egui::Modifiers::NONE);
    // Ctrl/Cmd-click a layer's eye: Outline, and back.
    click_with(&mut app, &ctx, eye(1), egui::Modifiers::COMMAND);
    assert!(matches!(node(&app, sub).kind, NodeKind::Layer { preview: false, .. }));
    click_with(&mut app, &ctx, eye(1), egui::Modifiers::COMMAND);
    assert!(matches!(node(&app, sub).kind, NodeKind::Layer { preview: true, .. }));
    // Drag down the eye column from the group to the rectangle below: all three hide, one step.
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let before = undo(&app);
    let steps = [
        vec![egui::Event::PointerMoved(eye(2))],
        vec![button(eye(2), true)],
        vec![egui::Event::PointerMoved(eye(2) + vec2(0.0, 6.0))],
        vec![egui::Event::PointerMoved(eye(3))],
        vec![egui::Event::PointerMoved(eye(4))],
        vec![button(eye(4), false)],
        vec![],
    ];
    for e in steps {
        frame(&mut app, &ctx, e, false);
    }
    assert!(!node(&app, g).visible && !node(&app, b).visible && !node(&app, a).visible);
    assert!(node(&app, sub).visible, "rows it didn't pass stay");
    assert_eq!(undo(&app), before + 1, "one undo step");
    assert!(!app.session.in_interaction());
    app.session.execute("edit.undo", &json!({})).unwrap();
    assert!(node(&app, g).visible && node(&app, b).visible && node(&app, a).visible);
}

#[test]
fn triangles_open_and_close_rows_and_alt_does_everything_inside() {
    let (mut app, [layer, sub, ..]) = nested();
    let ctx = egui::Context::default();
    let count = |app: &mut VectorcraftApp| frame(app, &ctx, vec![], false).0.len();
    assert_eq!(count(&mut app), 6);
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    let triangle = |depth: f32, y: f32| egui::pos2(2.0 * COLUMN + 8.0 + depth * INDENT + 7.0, y);
    // Close the sublayer: its group, the group's rectangle and its own rectangle go.
    click_with(&mut app, &ctx, triangle(1.0, circles[1].y), egui::Modifiers::NONE);
    assert_eq!(count(&mut app), 3);
    // Alt-click the layer's: everything inside closes, and opens again.
    click_with(&mut app, &ctx, triangle(0.0, circles[0].y), egui::Modifiers::ALT);
    assert_eq!(count(&mut app), 1);
    click_with(&mut app, &ctx, triangle(0.0, circles[0].y), egui::Modifiers::ALT);
    assert_eq!(count(&mut app), 6);
    // Show Layers Only (Panel Options).
    app.ui.layers_panel.layers_only = true;
    assert_eq!(count(&mut app), 2);
    app.ui.layers_panel = PanelOptions { row_size: ROW_LARGE, ..Default::default() };
    let (c, _) = frame(&mut app, &ctx, vec![], false);
    assert!((c[1].y - c[0].y - ROW_LARGE).abs() < 0.5, "large rows");
    let _ = (layer, sub);
}

// ---- Search All and the Filter ----

/// Layer 1 holding a path, a rectangle, type reading "Sky line", a group of an ellipse with an
/// effect, a compound path, a guide, an embedded and a linked image and a ruler guide, and on top
/// a sublayer "Sub" holding a hidden path named "Sky" and a locked path; only Layer 1 open and
/// nothing selected → (app, the rows' ids by name).
fn kinds() -> (VectorcraftApp, HashMap<&'static str, NodeId>) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    let id = |v: serde_json::Value| NodeId(v["id"].as_u64().unwrap());
    let star = |app: &mut VectorcraftApp, x: u32| id(run(app, "shape.star", json!({"cx": x, "cy": 50, "radius1": 20, "radius2": 10})));
    run(&mut app, "file.new", json!({"width": 400, "height": 400}));
    let mut ids = HashMap::from([("layer", app.session.active().unwrap().doc.layers[0].id)]);
    ids.insert("path", star(&mut app, 50));
    ids.insert("rect", id(run(&mut app, "shape.rectangle", json!({"x": 0, "y": 100, "width": 20, "height": 20}))));
    ids.insert("text", id(run(&mut app, "text.create", json!({"x": 10, "y": 200, "text": "Sky line"}))));
    let ellipse = id(run(&mut app, "shape.ellipse", json!({"x": 100, "y": 100, "width": 20, "height": 20})));
    run(&mut app, "effect.apply", json!({"effect": "distort.twist", "ids": [ellipse.0]}));
    run(&mut app, "select.set", json!({"ids": [ellipse.0]}));
    ids.insert("group", id(run(&mut app, "object.group", json!({}))));
    ids.insert("ellipse", ellipse);
    let parts = [star(&mut app, 150), star(&mut app, 160)];
    run(&mut app, "select.set", json!({"ids": [parts[0].0, parts[1].0]}));
    ids.insert("compound", id(run(&mut app, "object.compoundPath.make", json!({}))));
    let guide = id(run(&mut app, "shape.arc", json!({"x1": 0, "y1": 300, "x2": 50, "y2": 350})));
    run(&mut app, "select.set", json!({"ids": [guide.0]}));
    run(&mut app, "view.guides.make", json!({}));
    ids.insert("guide", guide);
    run(&mut app, "guide.add", json!({"vertical": true, "pos": 50}));
    let mut png = vec![];
    image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 40, 40, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let placed = |app: &mut VectorcraftApp, p: serde_json::Value| NodeId(run(app, "file.place", p)["ids"][0].as_u64().unwrap());
    ids.insert("embedded", placed(&mut app, json!({"name": "pic.png", "dataBase64": vectorcraft_format::base64_encode(&png)})));
    // One file for every test that calls this, written by the first.
    static LINKED: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    let file = LINKED.get_or_init(|| {
        let file = vectorcraft_testkit::temp_dir("layers-filter").join("linked.png");
        std::fs::write(&file, &png).unwrap();
        file
    });
    ids.insert("linked", placed(&mut app, json!({"path": file.to_string_lossy()})));
    ids.insert("sub", id(run(&mut app, "layer.newSublayer", json!({"name": "Sub"}))));
    let hidden = star(&mut app, 250);
    run(&mut app, "layer.setProps", json!({"ids": [hidden.0], "name": "Sky", "visible": false}));
    let locked = star(&mut app, 300);
    run(&mut app, "layer.setProps", json!({"ids": [locked.0], "locked": true}));
    ids.extend([("hidden", hidden), ("locked", locked)]);
    run(&mut app, "select.none", json!({}));
    (app, ids)
}

/// The rows [`list_rows`] lists for `query` and `filter`, by their names in `ids` ("ruler guide"
/// for a guide's row).
fn listed(app: &VectorcraftApp, ids: &HashMap<&'static str, NodeId>, query: &str, filter: &Filter) -> Vec<&'static str> {
    let st = app.session.active().unwrap();
    let rows = list_rows(&st.doc, &st.layers_open, &PanelOptions::default(), query, filter);
    let name = |r: &Row| match r.guide {
        Some(_) => "ruler guide",
        None => ids.iter().find(|(_, id)| **id == r.node.id).map_or("?", |(name, _)| *name),
    };
    rows.iter().map(name).collect()
}

fn only(k: ObjectKind) -> Filter {
    Filter { objects: vec![k], layers: vec![] }
}

/// Each object kind keeps exactly the rows of that kind, with the rows holding them, open or
/// closed.
#[test]
fn each_object_kind_keeps_its_rows_and_the_rows_holding_them() {
    let (app, ids) = kinds();
    for (k, want) in [
        (ObjectKind::Paths, vec!["layer", "sub", "locked", "hidden", "compound", "path"]),
        (ObjectKind::Shapes, vec!["layer", "group", "ellipse", "rect"]),
        (ObjectKind::Text, vec!["layer", "text"]),
        (ObjectKind::Images, vec!["layer", "linked", "embedded"]),
        (ObjectKind::Effects, vec!["layer", "group", "ellipse"]),
        (ObjectKind::Groups, vec!["layer", "group"]),
        (ObjectKind::LinkedFiles, vec!["layer", "linked"]),
        (ObjectKind::Guides, vec!["layer", "ruler guide", "guide"]),
    ] {
        assert_eq!(listed(&app, &ids, "", &only(k)), want, "{k:?}");
    }
}

/// By Layer keeps the rows that are locked or hidden themselves: in a hidden, locked sublayer a
/// row stays only for its own lock or eye. Two choices in a section take either; a choice in
/// each section takes rows that are both.
#[test]
fn row_states_keep_locked_or_hidden_rows_and_combine_with_kinds() {
    let (mut app, ids) = kinds();
    let f = |objects: Vec<ObjectKind>, layers: Vec<RowState>| Filter { objects, layers };
    assert_eq!(listed(&app, &ids, "", &f(vec![], vec![RowState::Hidden])), ["layer", "sub", "hidden"]);
    assert_eq!(listed(&app, &ids, "", &f(vec![], vec![RowState::Locked])), ["layer", "sub", "locked"]);
    assert_eq!(listed(&app, &ids, "", &f(vec![], vec![RowState::Locked, RowState::Hidden])), ["layer", "sub", "locked", "hidden"]);
    assert_eq!(listed(&app, &ids, "", &f(vec![ObjectKind::Text, ObjectKind::Images], vec![])), ["layer", "linked", "embedded", "text"]);
    assert_eq!(listed(&app, &ids, "", &f(vec![ObjectKind::Paths], vec![RowState::Hidden])), ["layer", "sub", "hidden"]);
    assert!(listed(&app, &ids, "", &f(vec![ObjectKind::Shapes], vec![RowState::Hidden])).is_empty());
    // Once Sub is hidden and locked, the rows in it stay only for their own lock or eye.
    app.session.execute("layer.setProps", &json!({"ids": [ids["sub"].0], "visible": false, "locked": true})).unwrap();
    assert_eq!(listed(&app, &ids, "", &f(vec![], vec![RowState::Hidden])), ["layer", "sub", "hidden"]);
    assert_eq!(listed(&app, &ids, "", &f(vec![], vec![RowState::Locked])), ["layer", "sub", "locked"]);
}

/// The filter and the search combine: a row stays when its name holds the text and the filter
/// keeps it, or when it holds such a row.
#[test]
fn the_filter_combines_with_the_search() {
    let (app, ids) = kinds();
    assert_eq!(listed(&app, &ids, "sky", &Filter::default()), ["layer", "sub", "hidden", "text"]);
    assert_eq!(listed(&app, &ids, "sky", &only(ObjectKind::Text)), ["layer", "text"]);
    assert_eq!(listed(&app, &ids, "sky", &Filter { objects: vec![], layers: vec![RowState::Hidden] }), ["layer", "sub", "hidden"]);
    assert!(listed(&app, &ids, "sky", &only(ObjectKind::Groups)).is_empty());
}

/// (target circles, lit 24 pt buttons) one frame of the panel draws.
fn circles_and_lit_buttons(app: &mut VectorcraftApp, ctx: &egui::Context) -> (usize, usize) {
    let lit = Tokens::get(ctx).tool_active;
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
    out.textures_delta.clear();
    let circles = out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Circle(cs) if cs.radius == 5.0)).count();
    let buttons =
        out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == lit && (r.rect.width() - 24.0).abs() < 0.01)).count();
    (circles, buttons)
}

/// `ui.layersFilter` has the panel list its rows again at once and lights the Filter button
/// while a filter is on; clearing it lists every row again. The document, its open rows and the
/// selection stay as they were.
#[test]
fn the_panel_lists_its_rows_again_when_the_filter_changes() {
    let (mut app, ids) = kinds();
    app.session.execute("select.set", &json!({"ids": [ids["rect"].0]})).unwrap();
    let ctx = egui::Context::default();
    let before = circles_and_lit_buttons(&mut app, &ctx);
    assert_eq!(before.1, 0, "the Filter button is not lit");
    let st = app.session.active().unwrap();
    let (revision, open) = (st.revision, st.layers_open.generation());
    let r = app.run("ui.layersFilter", json!({"objects": ["text"]})).unwrap();
    assert_eq!(r, json!({"objects": ["text"], "layers": []}));
    assert_eq!(circles_and_lit_buttons(&mut app, &ctx), (2, 1), "Layer 1 and the type; the button is lit");
    app.run("ui.layersFilter", json!({"layers": ["hidden"]})).unwrap();
    assert_eq!(circles_and_lit_buttons(&mut app, &ctx), (0, 1), "no type is hidden");
    app.run("ui.layersFilter", json!({"objects": [], "layers": []})).unwrap();
    assert_eq!(circles_and_lit_buttons(&mut app, &ctx), before, "every row again");
    let st = app.session.active().unwrap();
    assert_eq!((st.revision, st.layers_open.generation(), st.selection.objects.clone()), (revision, open, vec![ids["rect"]]));
}

/// Clicks in the Filter button's popup tick a kind and a state, and a second click clears the
/// kind, while the popup stays open; Clear All turns the filter off.
#[test]
fn the_filter_popup_ticks_kinds_and_states_and_clear_all_turns_the_filter_off() {
    let (mut app, _) = kinds();
    let ctx = egui::Context::default();
    // The Filter button: the 24 pt square lit while a filter is on.
    app.run("ui.layersFilter", json!({"layers": ["locked"]})).unwrap();
    let lit = Tokens::get(&ctx).tool_active;
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui));
    out.textures_delta.clear();
    let filter = out
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.fill == lit && (r.rect.width() - 24.0).abs() < 0.01 => Some(r.rect.center()),
            _ => None,
        })
        .unwrap();
    app.run("ui.layersFilter", json!({"layers": []})).unwrap();
    click_with(&mut app, &ctx, filter, egui::Modifiers::NONE);
    // Where the popup draws `label` (after a frame for it to settle).
    let at = |app: &mut VectorcraftApp, label: &str| {
        super::super::tests_appearance::frame_events(&ctx, app, vec![], show);
        let texts = super::super::tests_appearance::frame_events(&ctx, app, vec![], show);
        texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("{label} not in {texts:?}"))
    };
    let text = at(&mut app, "Text");
    click_with(&mut app, &ctx, text, egui::Modifiers::NONE);
    assert_eq!(app.ui.layers_filter, Filter { objects: vec![ObjectKind::Text], layers: vec![] });
    let hidden = at(&mut app, "Hidden");
    click_with(&mut app, &ctx, hidden, egui::Modifiers::NONE);
    assert_eq!(app.ui.layers_filter, Filter { objects: vec![ObjectKind::Text], layers: vec![RowState::Hidden] });
    click_with(&mut app, &ctx, text, egui::Modifiers::NONE);
    assert_eq!(app.ui.layers_filter, Filter { objects: vec![], layers: vec![RowState::Hidden] }, "a second click takes it out");
    let clear = at(&mut app, "Clear All");
    click_with(&mut app, &ctx, clear, egui::Modifiers::NONE);
    assert!(!app.ui.layers_filter.is_on());
}

/// `ui.layersFilter` reads its lists by name in the popup's order, keeps a list it is not given,
/// and changes nothing when a list holds anything else.
#[test]
fn the_filter_command_reads_its_lists_by_name() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let r = app.run("ui.layersFilter", json!({"objects": ["guides", "paths", "paths"], "layers": ["hidden"]})).unwrap();
    assert_eq!(r, json!({"objects": ["paths", "guides"], "layers": ["hidden"]}));
    let r = app.run("ui.layersFilter", json!({"layers": []})).unwrap();
    assert_eq!(r, json!({"objects": ["paths", "guides"], "layers": []}));
    for bad in [json!({"objects": ["paths", "layers"]}), json!({"objects": "paths"}), json!({"objects": [], "layers": ["visible"]})] {
        let e = app.run("ui.layersFilter", bad.clone()).unwrap_err();
        let key = if bad["layers"].is_null() { "objects" } else { "layers" };
        assert!(e.starts_with(&format!("`{key}` must be an array of these names: ")), "{bad}: {e}");
    }
    assert_eq!(app.run("ui.layersFilter", json!({})).unwrap(), json!({"objects": ["paths", "guides"], "layers": []}), "unchanged");
}

#[test]
fn dragging_a_layer_into_the_empty_space_under_the_rows_moves_it_to_the_bottom() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("layer.new", &json!({"top": true})).unwrap();
    let ids = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers.iter().map(|l| l.id).collect::<Vec<_>>();
    let [bottom, top] = ids(&app)[..] else { panic!("two layers") };
    let ctx = egui::Context::default();
    // Rows: the top layer, the bottom layer, then empty space.
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    assert_eq!(circles.len(), 2);
    drag(&mut app, &ctx, egui::pos2(150.0, circles[0].y), egui::pos2(150.0, circles[1].y + 3.0 * ROW_MEDIUM), false);
    assert_eq!(ids(&app), vec![top, bottom], "the top layer is now the bottom one");
}

#[test]
fn dragging_rows_moves_them_into_other_layers_and_alt_copies() {
    let (mut app, [layer, sub, g, _, a, c]) = nested();
    let ctx = egui::Context::default();
    let l2 = NodeId(app.session.execute("layer.new", &json!({"top": true, "name": "Top"})).unwrap()["id"].as_u64().unwrap());
    // Rows: Top, Layer 1, Sub, g, b, a, c.
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    assert_eq!(circles.len(), 7);
    let name = |i: usize| egui::pos2(150.0, circles[i].y);
    let parent = |app: &VectorcraftApp, id: NodeId| app.session.active().unwrap().doc.parent_of(id);
    // Rectangle a onto the middle of the Top layer's row: into it.
    drag(&mut app, &ctx, name(5), name(0), false);
    assert_eq!(parent(&app, a), Some(l2));
    // Rows: Top, a, Layer 1, Sub, g, b, c. The sublayer onto the upper edge of Layer 1's row:
    // above it, a top-level layer now.
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    assert_eq!(circles.len(), 7);
    drag(&mut app, &ctx, egui::pos2(150.0, circles[3].y), egui::pos2(150.0, circles[2].y - ROW_MEDIUM * 0.4), false);
    assert_eq!(parent(&app, sub), None);
    let d = &app.session.active().unwrap().doc;
    assert_eq!(d.layers.iter().map(|l| l.id).collect::<Vec<_>>(), vec![layer, sub, l2]);
    // Rows: Top, a, Sub, g, b, Layer 1, c. Alt-drag the group into Layer 1: a copy goes there,
    // the group stays.
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    let count = app.session.active().unwrap().doc.node_count();
    drag(&mut app, &ctx, egui::pos2(150.0, circles[3].y), egui::pos2(150.0, circles[5].y), true);
    assert_eq!(parent(&app, g), Some(sub));
    assert_eq!(app.session.active().unwrap().doc.node_count(), count + 2, "the group and its rectangle, copied");
    let _ = (layer, c);
}

/// The centres of the bottom bar's buttons, right to left from under the right end of its top
/// divider: Delete, New Layer, New Sublayer, Make/Release Clipping Mask, Locate Object.
fn bar_buttons(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<egui::Pos2> {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
    out.textures_delta.clear();
    let divider = Tokens::get(ctx).divider;
    let corner = out
        .shapes
        .iter()
        .rev()
        .find_map(|c| match &c.shape {
            egui::Shape::LineSegment { points, stroke } if stroke.color == divider => Some(points[1]),
            _ => None,
        })
        .unwrap();
    let step = 24.0 + ctx.global_style().spacing.item_spacing.x;
    (0..5).map(|k| corner + vec2(-12.0 - k as f32 * step, 15.0)).collect()
}

/// How many texts reading `s` the frame after `events` paints.
fn texts(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, s: &str) -> usize {
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui));
    out.textures_delta.clear();
    out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Text(t) if t.galley.text() == s)).count()
}

/// Drag the row at `a` to `b` and drop it there. Returns how many times its `name` was painted
/// while it hung over `b`.
fn drag_row(app: &mut VectorcraftApp, ctx: &egui::Context, a: egui::Pos2, b: egui::Pos2, name: &str) -> usize {
    frame(app, ctx, vec![egui::Event::PointerMoved(a)], false);
    frame(app, ctx, vec![button(a, true)], false);
    frame(app, ctx, vec![egui::Event::PointerMoved(a + vec2(0.0, 6.0))], false);
    let shown = texts(app, ctx, vec![egui::Event::PointerMoved(b)], name);
    frame(app, ctx, vec![button(b, false)], false);
    frame(app, ctx, vec![], false);
    shown
}

#[test]
fn dropping_rows_on_new_layer_duplicates_them() {
    let (mut app, first, _) = two_rects();
    let second = app.session.execute("layer.new", &json!({"name": "Top"})).unwrap()["id"].as_u64().map(NodeId).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let layers = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers.iter().map(|l| l.id).collect::<Vec<_>>();
    // Rows top down: Top, Layer 1 (expanded: its two rectangles).
    let (c, _) = frame(&mut app, &ctx, vec![], false);
    let buttons = bar_buttons(&mut app, &ctx);
    // Layer 1 dropped on New Layer: a copy of it right above it.
    let shown = drag_row(&mut app, &ctx, egui::pos2(150.0, c[1].y), buttons[1], "Layer 1");
    assert_eq!(shown, 2, "the row and its ghost under the pointer");
    let after = layers(&app);
    assert_eq!(after.len(), 3);
    assert_eq!((after[0], after[2]), (first, second));
    assert_eq!(app.session.active().unwrap().doc.node(after[1]).unwrap().children().map(Vec::len), Some(2));
    assert_eq!(texts(&mut app, &ctx, vec![], "Layer 1"), 1, "the ghost is gone after the drop");
    // One undo step takes the copy away.
    app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(layers(&app), vec![first, second]);
    // The other bottom-bar buttons are no drop targets.
    drag_row(&mut app, &ctx, egui::pos2(150.0, c[0].y), buttons[2], "Top");
    assert_eq!(layers(&app), vec![first, second]);
}

/// Cmd+Alt+L is New Layer Options… by default. On Windows and Linux the key is Ctrl+Alt+L, and on
/// macOS it is Option+Command+L. Cmd+L is still New Layer. No other command, tool or panel has
/// Cmd+Alt+L by default.
#[test]
fn cmd_alt_l_is_new_layer_options() {
    use crate::shortcut_editor::{command_for_key, default_command_shortcut};
    assert_eq!(default_command_shortcut("ui.newLayer"), Some("Cmd+Alt+L"));
    assert_eq!(command_for_key("Cmd+Alt+L"), Some("ui.newLayer"));
    assert_eq!(command_for_key("Cmd+L"), Some("layer.new"));
    let chord = crate::shortcuts::parse("Cmd+Alt+L").expect("parses");
    assert_eq!((chord.modifiers, chord.logical_key), (egui::Modifiers::COMMAND | egui::Modifiers::ALT, egui::Key::L));
    let shown = crate::menus::pretty_shortcut("Cmd+Alt+L");
    assert_eq!(shown, if cfg!(target_os = "macos") { "⌥⌘L" } else { "Ctrl+Alt+L" });
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let listed = app.run("shortcuts.list", json!({"query": "ui.newLayer"})).unwrap();
    let rows: Vec<_> = listed.as_array().expect("rows").iter().map(|r| (r["label"].clone(), r["default"].clone())).collect();
    assert_eq!(rows, [(json!("New Layer Options…"), json!("Cmd+Alt+L"))], "Keyboard Shortcuts lists it");
    assert_eq!(app.run("shortcuts.conflicts", json!({})).unwrap(), json!([]), "no other default has the key");
}

/// Cmd+Alt+L, pressed through the shortcut handler, opens Layer Options for a new layer as an
/// Alt-click on the panel's New Layer button does, and no layer is added until OK. OK adds the
/// layer above the current layer, as after the Alt-click, and Cancel adds none. Cmd+L still adds
/// a layer without the dialog.
#[test]
fn cmd_alt_l_opens_new_layer_options_as_alt_clicking_new_layer_does() {
    fn press(app: &mut VectorcraftApp, modifiers: egui::Modifiers) {
        let ctx = egui::Context::default();
        let ev = egui::Event::Key { key: egui::Key::L, physical_key: None, pressed: true, repeat: false, modifiers };
        let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
        out.textures_delta.clear();
    }
    /// Layer 1 with two rectangles under a layer named Top, with Layer 1 current.
    fn two_layers() -> VectorcraftApp {
        let (mut app, first, _) = two_rects();
        app.session.execute("layer.new", &json!({"name": "Top"})).unwrap();
        app.session.execute("layer.setCurrent", &json!({"id": first.0})).unwrap();
        app
    }
    let names = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers.iter().map(|l| l.display_name()).collect::<Vec<_>>();
    let dialog = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| (d.kind.clone(), d.fields.clone()));
    let mut keyed = two_layers();
    press(&mut keyed, egui::Modifiers::COMMAND | egui::Modifiers::ALT);
    let mut clicked = two_layers();
    let ctx = egui::Context::default();
    let new_layer = bar_buttons(&mut clicked, &ctx)[1];
    click_with(&mut clicked, &ctx, new_layer, egui::Modifiers::ALT);
    let opened = dialog(&keyed).expect("the key opens a dialog");
    assert_eq!((opened.0.as_str(), opened.1.get("mode")), ("layerOptions", Some(&json!("new"))));
    assert_eq!(dialog(&clicked), Some(opened), "the key opens what the Alt-click opens");
    assert_eq!(names(&keyed), ["Layer 1", "Top"], "no layer is added until OK");
    for app in [&mut keyed, &mut clicked] {
        crate::dialogs::confirm(app).unwrap();
    }
    assert_eq!(names(&keyed), ["Layer 1", "Layer 3", "Top"]);
    assert_eq!(names(&clicked), names(&keyed));
    // Cancel adds no layer.
    press(&mut keyed, egui::Modifiers::COMMAND | egui::Modifiers::ALT);
    assert!(keyed.ui.dialog.is_some());
    crate::dialogs::cancel(&mut keyed);
    assert!(keyed.ui.dialog.is_none());
    assert_eq!(names(&keyed), ["Layer 1", "Layer 3", "Top"]);
    press(&mut keyed, egui::Modifiers::COMMAND);
    assert!(keyed.ui.dialog.is_none(), "Cmd+L opens no dialog");
    assert_eq!(names(&keyed).len(), 4);
}

#[test]
fn the_selected_art_square_drags_the_selection_to_another_layer() {
    let (mut app, [_, sub, g, _, a, c]) = nested();
    let ctx = egui::Context::default();
    app.session.execute("select.set", &json!({"ids": [a.0, g.0]})).unwrap();
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    let square = |i: usize| circles[i] + vec2(TARGET_X - SQUARE_X, 0.0);
    // From the sublayer's (small) square onto Layer 1's row.
    drag(&mut app, &ctx, square(1), egui::pos2(150.0, circles[0].y), false);
    let d = &app.session.active().unwrap().doc;
    assert_eq!(d.node(d.layers[0].id).unwrap().children().unwrap().iter().map(|n| n.id).collect::<Vec<_>>(), vec![c, sub, a, g]);
    assert_eq!(app.session.active().unwrap().selection.objects.len(), 2, "still selected");
}

#[test]
fn the_panel_menu_lists_every_operation() {
    let (mut app, _) = nested();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], menu);
    for label in [
        "New Layer…",
        "New Sublayer…",
        "Duplicate Selection",
        "Delete Selection",
        "Options for Selection…",
        "Make/Release Clipping Mask",
        "Enter Isolation Mode",
        "Locate Object",
        "Save Selection",
        "Merge Selected",
        "Flatten Artwork",
        "Collect in New Layer",
        "Collect for Export",
        "Release to Layers (Sequence)",
        "Release to Layers (Build)",
        "Reverse Order",
        "Template",
        "Show All Layers",
        "Preview All Layers",
        "Unlock All Layers",
        "Paste Remembers Layers",
        "Panel Options…",
    ] {
        assert!(texts.iter().any(|(t, _)| t.trim() == label), "{label} missing from {texts:?}");
    }
}

/// ≡ › Save Selection lists the saved selections: choosing one recalls it, Update Selection
/// makes it hold the selected objects, and Save Selection… opens the name dialog.
#[test]
fn the_save_selection_submenu_recalls_updates_and_saves_selections() {
    let (mut app, _, [a, b]) = two_rects();
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    app.run("select.save", json!({"name": "Left"})).unwrap();
    app.run("select.set", json!({"ids": [b.0]})).unwrap();
    app.run("select.save", json!({"name": "Right"})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| super::super::tests_appearance::frame_events(&ctx, app, events, menu);
    // Opens the submenu (when closed) and clicks the item labeled `label` in it.
    let choose = |app: &mut VectorcraftApp, label: &str| {
        let at = |texts: &[(String, egui::Rect)], label: &str| texts.iter().find(|(t, _)| t.trim() == label).map(|(_, r)| r.center());
        let mut texts = frame(app, vec![]);
        if at(&texts, label).is_none() {
            let sub = at(&texts, "Save Selection").expect("the Save Selection submenu");
            for e in [egui::Event::PointerMoved(sub), button(sub, true), button(sub, false)] {
                frame(app, vec![e]);
            }
            texts = frame(app, vec![]);
        }
        let item = at(&texts, label).unwrap_or_else(|| panic!("{label} missing from {texts:?}"));
        for e in [egui::Event::PointerMoved(item), button(item, true), button(item, false)] {
            frame(app, vec![e]);
        }
    };
    let selected = |app: &VectorcraftApp| app.session.active().unwrap().selection.objects.clone();
    choose(&mut app, "Left");
    assert_eq!(selected(&app), vec![a], "choosing a name recalls it");
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    choose(&mut app, "Update Selection");
    let saved = app.session.active().unwrap().doc.saved_selections.clone();
    assert_eq!(saved.iter().map(|s| (s.name.as_str(), s.objects.len())).collect::<Vec<_>>(), vec![("Left", 2), ("Right", 1)]);
    assert!(app.ui.dialog.is_none());
    choose(&mut app, "Save Selection…");
    let d = app.ui.dialog.clone().expect("Save Selection dialog");
    assert_eq!((d.str("__command"), d.str("name")), ("select.save".into(), "Selection 1".into()));
}

/// With the dock collapsed, Layers shows in a floating flyout: its rows work there too.
#[test]
fn rows_work_in_the_floating_flyout() {
    let (mut app, [_, sub, ..]) = nested();
    app.ui.dock_collapsed = true;
    app.ui.open_panel = Some("layers".into());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 1000.0));
    let mut time = 0.0;
    let mut frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        time += 0.05;
        let input = egui::RawInput { time: Some(time), events, screen_rect: Some(screen), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| crate::panel_docking::show(app, ui));
        out.textures_delta.clear();
        let mut c: Vec<egui::Pos2> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Circle(cs) if cs.radius == 5.0 => Some(cs.center),
                _ => None,
            })
            .collect();
        c.sort_by(|a, b| a.y.total_cmp(&b.y));
        c
    };
    for _ in 0..3 {
        frame(&mut app, vec![]);
    }
    let circles = frame(&mut app, vec![]);
    assert_eq!(circles.len(), 6, "every row shows in the flyout");
    assert!(circles.iter().all(|c| screen.contains(*c)));
    let at = circles[1] - vec2(120.0, 0.0);
    frame(&mut app, vec![egui::Event::PointerMoved(at), button(at, true)]);
    frame(&mut app, vec![button(at, false)]);
    frame(&mut app, vec![]);
    assert_eq!(rows_of(&app), vec![sub]);
    assert_eq!(app.session.active().unwrap().current_layer(), Some(sub));
}

// ---- Locate Selected Objects (Panel Options) ----

/// A layer of `n` groups of one path each, with only the layer open, as a document opens →
/// (app, layer, the paths in stacking order, so the first is in the bottom row). The path in
/// group `i` is named `Path {i}`.
fn groups_of_one(n: usize) -> (VectorcraftApp, NodeId, Vec<NodeId>) {
    use vectorcraft_doc::Appearance;
    use vectorcraft_geom::{Rect, shapes};
    let mut d = Document::new(1000.0, 1000.0);
    let mut groups = vec![];
    let mut paths = vec![];
    for i in 0..n {
        let r = Rect::new(0.0, i as f64, 1.0, i as f64 + 1.0);
        let mut path = Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::default());
        path.name = Some(format!("Path {i}"));
        paths.push(path.id);
        groups.push(Arc::new(Node::group(d.alloc_id(), vec![Arc::new(path)])));
    }
    *Arc::make_mut(&mut d.layers[0]).children_mut().unwrap() = groups;
    let layer = d.layers[0].id;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.add_document(d, None);
    (app, layer, paths)
}

/// One frame of the panel in a 300 × 800 window, one second after the previous frame: the
/// centers of the target circles, top row first, and whether `name` is drawn.
fn tall_frame(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str) -> (Vec<egui::Pos2>, bool) {
    let time = ctx.input(|i| i.time) + 1.0;
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, 800.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), ..Default::default() }, |ui| show(app, ui));
    out.textures_delta.clear();
    let found = out.shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Text(t) if t.galley.text() == name));
    let mut circles: Vec<egui::Pos2> = out
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Circle(cs) if cs.radius == 5.0 => Some(cs.center),
            _ => None,
        })
        .collect();
    circles.sort_by(|a, b| a.y.total_cmp(&b.y));
    (circles, found)
}

/// Without Locate Selected Objects (the default), an object selected outside the panel leaves
/// the open rows and the scroll position as they were.
#[test]
fn without_the_option_a_selection_leaves_the_rows_and_the_scroll_as_they_were() {
    let (mut app, layer, paths) = groups_of_one(500);
    let ctx = egui::Context::default();
    let (before, _) = tall_frame(&mut app, &ctx, "");
    // The bottom row's path: out of view, in a closed group.
    app.session.execute("select.set", &json!({"ids": [paths[0].0]})).unwrap();
    for _ in 0..4 {
        let (circles, found) = tall_frame(&mut app, &ctx, "Path 0");
        assert!(!found);
        assert_eq!(circles, before, "nothing opens or scrolls");
    }
    let st = app.session.active().unwrap();
    let group = st.doc.parent_of(paths[0]).unwrap();
    assert!(st.layers_open.contains(layer) && !st.layers_open.contains(group));
}

/// With Locate Selected Objects on, an object selected outside the panel is located as Locate
/// Object locates it: the layer and the group around it open and its row is scrolled into
/// view. The highlighted row stays as it was.
#[test]
fn with_the_option_a_selected_object_is_opened_and_scrolled_into_view() {
    let (mut app, layer, paths) = groups_of_one(500);
    app.ui.layers_panel.locate_selected = true;
    let ctx = egui::Context::default();
    let top = app.session.active().unwrap().doc.parent_of(paths[499]).unwrap();
    app.session.execute("layer.highlight", &json!({"ids": [top.0]})).unwrap();
    tall_frame(&mut app, &ctx, "");
    expand(&mut app, &json!({"ids": [layer.0], "open": false})).unwrap();
    assert_eq!(tall_frame(&mut app, &ctx, "").0.len(), 1, "only the layer's row");
    app.session.execute("select.set", &json!({"ids": [paths[0].0]})).unwrap();
    let found = (0..4).any(|_| tall_frame(&mut app, &ctx, "Path 0").1);
    let st = app.session.active().unwrap();
    let group = st.doc.parent_of(paths[0]).unwrap();
    assert!(st.layers_open.contains(layer) && st.layers_open.contains(group), "the layer and the group open");
    assert!(found, "the row is scrolled into view");
    assert_eq!(st.highlighted_rows(), [top], "the highlighted row stays");
}

/// With Locate Selected Objects on, the panel's own clicks locate nothing: a Shift-click
/// highlights the rows down to the one it clicks, and Shift-clicking selection squares selects
/// art in a closed sublayer without opening it. An object then selected outside the panel is
/// located, and the highlighted rows stay.
#[test]
fn with_the_option_the_panels_own_clicks_locate_nothing() {
    let (mut app, [layer, sub, g, b, _, c]) = nested();
    app.ui.layers_panel.locate_selected = true;
    expand(&mut app, &json!({"ids": [sub.0, g.0], "open": false})).unwrap();
    let ctx = egui::Context::default();
    // Rows top down: Layer 1, Sub (closed), c.
    let (circles, _) = frame(&mut app, &ctx, vec![], false);
    assert_eq!(circles.len(), 3);
    let name = |i: usize| egui::pos2(150.0, circles[i].y);
    let square = |i: usize| circles[i] + vec2(TARGET_X - SQUARE_X, 0.0);
    click_with(&mut app, &ctx, name(0), egui::Modifiers::NONE);
    click_with(&mut app, &ctx, name(2), egui::Modifiers::SHIFT);
    assert_eq!(rows_of(&app), [layer, sub, c], "both rows clicked and the one between them");
    click_with(&mut app, &ctx, square(2), egui::Modifiers::SHIFT);
    click_with(&mut app, &ctx, square(1), egui::Modifiers::SHIFT);
    let st = app.session.active().unwrap();
    assert!(st.selection.objects.len() >= 3 && st.selection.objects.contains(&c), "c and Sub's art are selected");
    let (after, _) = frame(&mut app, &ctx, vec![], false);
    assert_eq!(after, circles, "no row opens and nothing scrolls");
    assert_eq!(rows_of(&app), [layer, sub, c]);
    // Selected outside the panel: b's sublayer and group open, and the rows stay highlighted.
    app.session.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    assert_eq!(frame(&mut app, &ctx, vec![], false).0.len(), 6);
    assert_eq!(rows_of(&app), [layer, sub, c]);
}

/// With Locate Selected Objects on, a selection the panel menu changes is not located: Delete
/// Selection deletes the highlighted object, and the object left selected stays in its closed
/// sublayer and group.
#[test]
fn with_the_option_the_panel_menu_locates_nothing() {
    let (mut app, [_, sub, g, b, _, c]) = nested();
    app.ui.layers_panel.locate_selected = true;
    app.session.execute("select.set", &json!({"ids": [b.0, c.0]})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, vec![], false);
    expand(&mut app, &json!({"ids": [sub.0, g.0], "open": false})).unwrap();
    app.session.execute("layer.highlight", &json!({"ids": [c.0]})).unwrap();
    assert_eq!(frame(&mut app, &ctx, vec![], false).0.len(), 3, "Layer 1, Sub (closed) and c");
    let menu_frame = |app: &mut VectorcraftApp, events| super::super::tests_appearance::frame_events(&ctx, app, events, menu);
    let at = menu_frame(&mut app, vec![]).iter().find(|(t, _)| t.trim() == "Delete Selection").map(|(_, r)| r.center()).unwrap();
    for e in [egui::Event::PointerMoved(at), button(at, true), button(at, false)] {
        menu_frame(&mut app, vec![e]);
    }
    assert_eq!(app.session.active().unwrap().selection.objects, [b], "c is deleted and b stays selected");
    assert_eq!(frame(&mut app, &ctx, vec![], false).0.len(), 2, "Sub stays closed");
}

/// With Locate Selected Objects on, a saved selection recalled from the panel menu's Save
/// Selection submenu is not located: the recalled object stays in its closed sublayer and group.
#[test]
fn with_the_option_a_selection_recalled_from_the_panel_menu_is_not_located() {
    let (mut app, [_, sub, g, b, _, c]) = nested();
    app.ui.layers_panel.locate_selected = true;
    app.run("select.set", json!({"ids": [b.0]})).unwrap();
    app.run("select.save", json!({"name": "Kept"})).unwrap();
    app.run("select.set", json!({"ids": [c.0]})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, vec![], false);
    expand(&mut app, &json!({"ids": [sub.0, g.0], "open": false})).unwrap();
    assert_eq!(frame(&mut app, &ctx, vec![], false).0.len(), 3, "Layer 1, Sub (closed) and c");
    let menu_frame = |app: &mut VectorcraftApp, events| super::super::tests_appearance::frame_events(&ctx, app, events, menu);
    let at = |texts: &[(String, egui::Rect)], label: &str| texts.iter().find(|(t, _)| t.trim() == label).map(|(_, r)| r.center());
    let sub_menu = at(&menu_frame(&mut app, vec![]), "Save Selection").expect("the Save Selection submenu");
    for e in [egui::Event::PointerMoved(sub_menu), button(sub_menu, true), button(sub_menu, false)] {
        menu_frame(&mut app, vec![e]);
    }
    let item = at(&menu_frame(&mut app, vec![]), "Kept").expect("the saved selection's item");
    for e in [egui::Event::PointerMoved(item), button(item, true), button(item, false)] {
        menu_frame(&mut app, vec![e]);
    }
    assert_eq!(app.session.active().unwrap().selection.objects, [b], "the saved selection is recalled");
    assert_eq!(frame(&mut app, &ctx, vec![], false).0.len(), 3, "Sub stays closed");
}

/// With Locate Selected Objects on, the object selected in a document that becomes active is
/// not located; an object selected after that is. Neither document has highlighted rows, so
/// the switch reveals no row of its own.
#[test]
fn with_the_option_a_document_becoming_active_locates_nothing() {
    let (mut app, [_, sub, g, b, a, _]) = nested();
    app.ui.layers_panel.locate_selected = true;
    app.session.execute("layer.highlight", &json!({"ids": []})).unwrap();
    app.session.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![], false);
    expand(&mut app, &json!({"ids": [sub.0, g.0], "open": false})).unwrap();
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    frame(&mut app, &ctx, vec![], false);
    app.session.execute("document.activate", &json!({"index": 0})).unwrap();
    assert_eq!(frame(&mut app, &ctx, vec![], false).0.len(), 3, "b's sublayer and group stay closed");
    app.session.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    assert_eq!(frame(&mut app, &ctx, vec![], false).0.len(), 5, "a's sublayer opens");
}

/// With Locate Selected Objects on, the copies an Alt-drag selects are located once the drag
/// ends, not while it lasts.
#[test]
fn with_the_option_a_selection_made_during_a_drag_is_located_when_it_ends() {
    let (mut app, [_, sub, g, b, ..]) = nested();
    app.ui.layers_panel.locate_selected = true;
    app.session.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![], false);
    expand(&mut app, &json!({"ids": [sub.0, g.0], "open": false})).unwrap();
    app.session.begin_interaction("Move").unwrap();
    for i in 1..=3 {
        app.session.preview("object.transform", &json!({"matrix": [1, 0, 0, 1, i * 10, 0], "copy": true})).unwrap();
        assert_eq!(frame(&mut app, &ctx, vec![], false).0.len(), 3, "nothing opens while the drag lasts");
    }
    app.session.commit_interaction().unwrap();
    let st = app.session.active().unwrap();
    let copy = st.selection.objects.last().copied().unwrap();
    assert!(copy != b && st.doc.parent_of(copy) == Some(g), "the copy is selected, in b's group");
    assert_eq!(frame(&mut app, &ctx, vec![], false).0.len(), 7, "Sub and the group open on the copy's row");
}

/// With Locate Selected Objects on, art selected outside the panel in another layer is
/// located: the selection makes that layer the active one, and the panel scrolls to the art's
/// row, not to the layer's row.
#[test]
fn with_the_option_the_located_object_is_revealed_instead_of_its_active_layer() {
    let (mut app, layer, paths) = groups_of_one(500);
    app.ui.layers_panel.locate_selected = true;
    app.session.execute("layer.new", &json!({})).unwrap();
    assert_ne!(app.session.active().unwrap().current_layer(), Some(layer));
    let ctx = egui::Context::default();
    tall_frame(&mut app, &ctx, "");
    app.session.execute("select.set", &json!({"ids": [paths[0].0]})).unwrap();
    assert_eq!(app.session.active().unwrap().current_layer(), Some(layer), "the path's layer is the active one");
    let found = (0..4).any(|_| tall_frame(&mut app, &ctx, "Path 0").1);
    let st = app.session.active().unwrap();
    let group = st.doc.parent_of(paths[0]).unwrap();
    assert!(st.layers_open.contains(group), "the group opens");
    assert!(found, "the path's row is scrolled into view");
}

/// With Locate Selected Objects and Show Layers Only on, art selected outside the panel has no
/// row, so the panel scrolls to the row of the layer that the selection makes the active one.
#[test]
fn with_the_option_and_show_layers_only_the_active_layer_is_revealed() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.ui.layers_panel.locate_selected = true;
    app.ui.layers_panel.layers_only = true;
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let rect = |app: &mut VectorcraftApp| run(app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))["id"].as_u64().unwrap();
    let bottom = rect(&mut app);
    for _ in 0..59 {
        run(&mut app, "layer.new", json!({}));
        rect(&mut app);
    }
    run(&mut app, "select.none", json!({}));
    let doc = app.session.active().unwrap().doc.clone();
    let name = doc.node(doc.layer_of(NodeId(bottom)).unwrap()).unwrap().display_name();
    let ctx = egui::Context::default();
    assert!(!(0..4).any(|_| tall_frame(&mut app, &ctx, &name).1), "the bottom layer's row is out of view");
    run(&mut app, "select.set", json!({"ids": [bottom]}));
    assert!((0..4).any(|_| tall_frame(&mut app, &ctx, &name).1), "the active layer's row is scrolled into view");
}

/// With Locate Selected Objects on, art selected outside the panel that Show Layers Only or the
/// filter leaves without a row opens nothing, so its group stays closed once the panel lists
/// every row again. Art the filter keeps has its group opened.
#[test]
fn with_the_option_art_the_panel_does_not_list_opens_nothing() {
    let (mut app, _, paths) = groups_of_one(500);
    app.ui.layers_panel.locate_selected = true;
    let ctx = egui::Context::default();
    let open = |app: &VectorcraftApp, i: usize| {
        let st = app.session.active().unwrap();
        st.layers_open.contains(st.doc.parent_of(paths[i]).unwrap())
    };
    let locate = |app: &mut VectorcraftApp, i: usize| {
        app.session.execute("select.set", &json!({"ids": [paths[i].0]})).unwrap();
        for _ in 0..4 {
            tall_frame(app, &ctx, "");
        }
    };
    tall_frame(&mut app, &ctx, "");
    app.ui.layers_panel.layers_only = true;
    locate(&mut app, 0);
    app.ui.layers_panel.layers_only = false;
    tall_frame(&mut app, &ctx, "");
    assert!(!open(&app, 0), "Show Layers Only: the group stays closed");
    app.ui.layers_filter = Filter { objects: vec![ObjectKind::Text], layers: vec![] };
    locate(&mut app, 1);
    assert!(!open(&app, 1), "a filter that leaves the path out: the group stays closed");
    app.ui.layers_filter = Filter { objects: vec![ObjectKind::Paths], layers: vec![] };
    locate(&mut app, 2);
    assert!(open(&app, 2), "a filter that keeps the path: the group opens");
}

/// A layer of 2,000 groups of 10 paths with every row open: one frame lays out only the rows
/// in view, and a row highlighted far below them is scrolled to.
#[test]
fn only_the_rows_in_view_are_laid_out() {
    use vectorcraft_doc::Appearance;
    use vectorcraft_geom::{Rect, shapes};
    let mut d = Document::new(1000.0, 1000.0);
    let mut groups = vec![];
    let mut needle = None;
    for g in 0..2000 {
        let kids = (0..10)
            .map(|k| {
                let r = Rect::new(k as f64, g as f64 * 0.1, k as f64 + 1.0, g as f64 * 0.1 + 1.0);
                let mut n = Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::default());
                if g == 0 && k == 0 {
                    n.name = Some("Needle".into());
                    needle = Some(n.id);
                }
                Arc::new(n)
            })
            .collect();
        groups.push(Arc::new(Node::group(d.alloc_id(), kids)));
    }
    *Arc::make_mut(&mut d.layers[0]).children_mut().unwrap() = groups;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.add_document(d, None);
    expand(&mut app, &json!({})).unwrap();
    let ctx = egui::Context::default();
    // (shapes drawn, whether the needle's name is among them); a second passes per frame, so
    // scrolling ends.
    let mut time = 0.0;
    let mut frame = |app: &mut VectorcraftApp| {
        time += 1.0;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, 800.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        let found = out.shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Text(t) if t.galley.text() == "Needle"));
        (out.shapes.len(), found)
    };
    let (shapes, found) = frame(&mut app);
    // 22,001 rows; about 30 fit.
    assert!(shapes < 1000 && !found, "{shapes} shapes");
    let needle = needle.unwrap();
    app.session.execute("layer.highlight", &json!({"ids": [needle.0], "mode": "set"})).unwrap();
    let found = (0..4).any(|_| frame(&mut app).1);
    assert_eq!(app.session.active().unwrap().highlighted_rows(), [needle]);
    assert!(found, "the highlighted row is scrolled into view");
}

/// Selecting art on the canvas scrolls its layer (now the active one) into view, but not away
/// from a row highlighted in the panel.
#[test]
fn a_canvas_selection_reveals_its_active_layer() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let mut rects = vec![run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))["id"].as_u64().unwrap()];
    for _ in 0..59 {
        run(&mut app, "layer.new", json!({}));
        rects.push(run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))["id"].as_u64().unwrap());
    }
    run(&mut app, "select.none", json!({}));
    let doc = app.session.active().unwrap().doc.clone();
    let name = |id: u64| doc.node(doc.layer_of(NodeId(id)).unwrap()).unwrap().display_name();
    let (bottom, top) = (name(rects[0]), name(rects[59]));
    let ctx = egui::Context::default();
    let mut time = 0.0;
    // Whether a row with this name is drawn after a few frames.
    let mut shown = |app: &mut VectorcraftApp, s: &str| {
        let mut found = false;
        for _ in 0..4 {
            time += 1.0;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, 400.0));
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
            found = out.shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Text(t) if t.galley.text() == s));
        }
        found
    };
    assert!(shown(&mut app, &top) && !shown(&mut app, &bottom));
    run(&mut app, "select.set", json!({"ids": [rects[0]]}));
    assert_eq!(app.session.active().unwrap().current_layer(), doc.layer_of(NodeId(rects[0])));
    assert!(shown(&mut app, &bottom), "the active layer is scrolled into view");
    // A row highlighted in the panel stays in view when the active layer changes.
    let top_layer = doc.layer_of(NodeId(rects[59])).unwrap();
    run(&mut app, "layer.highlight", json!({"ids": [top_layer.0], "mode": "set"}));
    assert!(shown(&mut app, &top));
    app.session.active_mut().unwrap().active_layer = doc.layer_of(NodeId(rects[0]));
    assert!(shown(&mut app, &top) && !shown(&mut app, &bottom), "the highlighted row keeps the view");
}
