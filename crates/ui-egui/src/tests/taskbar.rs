//! The Contextual Task Bar's actions for each kind of selection: open paths offer Join and Edit
//! Path, compound paths and clipping masks their Release, images selected with vector art Make
//! Clipping Mask, type converts between point and area type, and a basic shape shows its
//! properties in place of Lock.

use egui::{Event, PointerButton, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_doc::text::TextKind;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::Session;

use crate::tests_removeanchors::{at, has, shapes_text};
use crate::{VectorcraftApp, canvas};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

fn id(v: serde_json::Value) -> u64 {
    v["id"].as_u64().unwrap()
}

fn kind(app: &VectorcraftApp, id: u64) -> NodeKind {
    app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind.clone()
}

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
    shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
}

/// The task bar's labels for the current selection (its area settles on the second frame).
fn bar(app: &mut VectorcraftApp) -> (egui::Context, Vec<(String, Rect)>) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(app, &ctx, vec![]);
    let texts = frame(app, &ctx, vec![]);
    (ctx, texts)
}

/// Click the task bar's `label`.
fn click(app: &mut VectorcraftApp, label: &str) {
    let (ctx, texts) = bar(app);
    let p = at(&texts, label);
    let press = |pressed| Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, &ctx, vec![Event::PointerMoved(p)]);
    frame(app, &ctx, vec![press(true)]);
    frame(app, &ctx, vec![press(false)]);
    frame(app, &ctx, vec![]);
}

fn select(app: &mut VectorcraftApp, ids: &[u64]) {
    app.run("select.set", json!({"ids": ids})).unwrap();
}

#[test]
fn an_open_path_offers_join_and_edit_path() {
    let mut app = app();
    let line = id(app.run("shape.line", json!({"x1": 50, "y1": 50, "x2": 200, "y2": 120})).unwrap());
    select(&mut app, &[line]);
    let (_, texts) = bar(&mut app);
    for label in ["Join", "Edit Path", "Offset Path", "Simplify", "Duplicate"] {
        assert!(has(&texts, label), "{label}: {texts:?}");
    }
    // Join closes the single open path.
    click(&mut app, "Join");
    assert!(matches!(kind(&app, line), NodeKind::Path { path, .. } if path.is_closed()));
    // A closed path offers Edit Path but not Join.
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Edit Path") && !has(&texts, "Join"), "{texts:?}");
    // Edit Path switches to the Direct Selection tool, keeping the selection.
    click(&mut app, "Edit Path");
    assert_eq!(app.session.tool_id(), "directSelection");
    assert_eq!(app.session.active().unwrap().selection.objects, [NodeId(line)]);
}

#[test]
fn a_compound_path_offers_edit_path_and_release() {
    let mut app = app();
    let a = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 120, "height": 100})).unwrap());
    let b = id(app.run("shape.ellipse", json!({"x": 70, "y": 60, "width": 50, "height": 50})).unwrap());
    select(&mut app, &[a, b]);
    let c = id(app.run("object.compoundPath.make", json!({})).unwrap());
    assert!(matches!(kind(&app, c), NodeKind::Compound { .. }));
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Edit Path") && has(&texts, "Release Compound Path"), "{texts:?}");
    assert!(!has(&texts, "Offset Path"), "{texts:?}");
    click(&mut app, "Release Compound Path");
    let st = app.session.active().unwrap();
    assert_eq!(st.selection.objects.len(), 2, "the paths come apart, selected");
    assert!(st.selection.objects.iter().all(|i| matches!(st.doc.node(*i).unwrap().kind, NodeKind::Path { .. })));
}

#[test]
fn a_clipping_mask_offers_edit_clipping_path_and_release() {
    let mut app = app();
    let art = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 160, "height": 120})).unwrap());
    let mask = id(app.run("shape.ellipse", json!({"x": 60, "y": 50, "width": 100, "height": 90})).unwrap());
    select(&mut app, &[art, mask]);
    let g = id(app.run("object.clippingMask.make", json!({})).unwrap());
    assert!(matches!(kind(&app, g), NodeKind::Group { clip: true, .. }));
    select(&mut app, &[g]);
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Edit Clipping Path") && has(&texts, "Release Clipping Mask"), "{texts:?}");
    assert!(!has(&texts, "Ungroup"), "a clip group isn't offered Ungroup: {texts:?}");
    click(&mut app, "Release Clipping Mask");
    assert!(!matches!(kind(&app, g), NodeKind::Group { clip: true, .. }), "no longer clipped");
    // Undo puts the mask back.
    app.run("edit.undo", json!({})).unwrap();
    assert!(matches!(kind(&app, g), NodeKind::Group { clip: true, .. }));
}

#[test]
fn an_image_with_vector_art_offers_make_clipping_mask() {
    let mut app = app();
    let mut png = vec![];
    image::RgbaImage::from_pixel(20, 20, image::Rgba([0, 128, 255, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let dir = std::env::temp_dir().join(format!("vectorcraft-ui-taskbar-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("blue.png");
    std::fs::write(&path, &png).unwrap();
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    app.run("file.place", json!({"path": path.to_string_lossy(), "link": false})).unwrap();
    let img = app.session.active().unwrap().selection.objects[0].0;
    assert!(matches!(kind(&app, img), NodeKind::Image(_)));
    let shape = id(app.run("shape.ellipse", json!({"x": 150, "y": 100, "width": 60, "height": 60})).unwrap());
    select(&mut app, &[img, shape]);
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Make Clipping Mask") && has(&texts, "Group"), "{texts:?}");
    assert!(!has(&texts, "Unite"), "{texts:?}");
    click(&mut app, "Make Clipping Mask");
    let st = app.session.active().unwrap();
    let g = st.selection.objects[0];
    assert!(matches!(st.doc.node(g).unwrap().kind, NodeKind::Group { clip: true, .. }));
    // Vector art alone still offers Unite.
    let a = id(app.run("shape.rectangle", json!({"x": 10, "y": 200, "width": 40, "height": 40})).unwrap());
    let b = id(app.run("shape.rectangle", json!({"x": 30, "y": 210, "width": 40, "height": 40})).unwrap());
    select(&mut app, &[a, b]);
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Unite") && !has(&texts, "Make Clipping Mask"), "{texts:?}");
}

#[test]
fn type_converts_between_point_and_area_type() {
    let mut app = app();
    let t = id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Hello"})).unwrap());
    select(&mut app, &[t]);
    let text_kind = |app: &VectorcraftApp| match kind(app, t) {
        NodeKind::Text(t) => t.kind.clone(),
        other => panic!("{other:?}"),
    };
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Create Outlines") && has(&texts, "Convert To Area Type"), "{texts:?}");
    click(&mut app, "Convert To Area Type");
    assert!(matches!(text_kind(&app), TextKind::Area { .. }));
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Convert To Point Type") && !has(&texts, "Convert To Area Type"), "{texts:?}");
    click(&mut app, "Convert To Point Type");
    assert!(matches!(text_kind(&app), TextKind::Point));
}

/// The path's anchors (its first subpath).
fn anchors(app: &VectorcraftApp, id: u64) -> Vec<vectorcraft_geom::Anchor> {
    match kind(app, id) {
        NodeKind::Path { path, .. } => path.subpaths[0].anchors.clone(),
        k => panic!("not a path: {k:?}"),
    }
}

/// With the Direct Selection tool, a path selected whole offers its anchor points' actions instead
/// of Edit Path and Offset Path (#4469), as after the bar's own Edit Path.
#[test]
fn direct_selection_of_a_whole_path_offers_its_anchor_actions() {
    let mut app = app();
    let line = id(app.run("path.create", json!({"anchors": [{"x": 50, "y": 200}, {"x": 150, "y": 80}, {"x": 250, "y": 200}]})).unwrap());
    select(&mut app, &[line]);
    click(&mut app, "Edit Path");
    assert_eq!(app.session.tool_id(), "directSelection");
    assert!(app.session.active().unwrap().selection.anchors.is_empty(), "the path is selected whole");
    let (_, texts) = bar(&mut app);
    for label in ["Connect", "Corner", "Smooth", "Simplify", "Duplicate"] {
        assert!(has(&texts, label), "{label}: {texts:?}");
    }
    for label in ["Edit Path", "Offset Path", "Join", "Remove Anchor Points", "Cut Path"] {
        assert!(!has(&texts, label), "{label}: {texts:?}");
    }
    // Smooth and Corner convert every anchor of the path, one undo step each.
    click(&mut app, "Smooth");
    let a = anchors(&app, line);
    assert_eq!(a[1].kind, vectorcraft_geom::AnchorKind::Smooth, "{a:?}");
    assert!(a[1].has_in() && a[1].has_out(), "{a:?}");
    click(&mut app, "Corner");
    assert!(anchors(&app, line).iter().all(|a| !a.has_in() && !a.has_out()), "{:?}", anchors(&app, line));
    app.run("edit.undo", json!({})).unwrap();
    assert!(anchors(&app, line)[1].has_in(), "Corner undoes in one step");
    // Connect closes the open path; a closed one isn't offered it.
    click(&mut app, "Connect");
    assert!(matches!(kind(&app, line), NodeKind::Path { path, .. } if path.is_closed()));
    let (_, texts) = bar(&mut app);
    assert!(!has(&texts, "Connect") && has(&texts, "Corner"), "{texts:?}");
}

/// Anchors picked out with the Direct Selection tool keep the full set: Remove and Cut need them.
#[test]
fn direct_selected_anchors_still_offer_remove_and_cut() {
    let mut app = app();
    let line = id(app.run("path.create", json!({"anchors": [{"x": 50, "y": 200}, {"x": 150, "y": 80}, {"x": 250, "y": 200}]})).unwrap());
    app.run("tool.select", json!({"tool": "directSelection"})).unwrap();
    app.run("select.anchors", json!({"id": line, "anchors": [[0, 1]]})).unwrap();
    let (_, texts) = bar(&mut app);
    for label in ["Remove Anchor Points", "Connect", "Cut Path", "Corner", "Smooth"] {
        assert!(has(&texts, label), "{label}: {texts:?}");
    }
    assert!(!has(&texts, "Edit Path"), "{texts:?}");
}

/// Click the task bar's button left of More Options (Lock, or a basic shape's Shape Properties),
/// and return the context and what the frame after the click draws.
fn click_lock_slot(app: &mut VectorcraftApp) -> (egui::Context, Vec<(String, Rect)>) {
    let (ctx, _) = bar(app);
    let r = canvas::task_bar_rect(&ctx).expect("the task bar shows");
    // Right to left: the frame's margin, More Options, then the button.
    let p = Pos2::new(r.right() - 8.0 - 30.0 - 6.0 - 15.0, r.center().y);
    let press = |pressed| Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, &ctx, vec![Event::PointerMoved(p)]);
    frame(app, &ctx, vec![press(true)]);
    frame(app, &ctx, vec![press(false)]);
    let texts = frame(app, &ctx, vec![]);
    (ctx, texts)
}

/// Type `value` into the field showing `field` and press Enter.
fn type_into(app: &mut VectorcraftApp, ctx: &egui::Context, texts: &[(String, Rect)], field: &str, value: &str) {
    let p = at(texts, field);
    let press = |pressed| Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, vec![Event::PointerMoved(p), press(true), press(false)]);
    // Focus selected the whole field: typing replaces it.
    frame(app, ctx, vec![]);
    frame(app, ctx, vec![Event::Text(value.into())]);
    let enter = Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    frame(app, ctx, vec![enter]);
    frame(app, ctx, vec![]);
}

fn live(app: &VectorcraftApp, id: u64) -> vectorcraft_doc::LiveShape {
    match kind(app, id) {
        NodeKind::Path { live: Some(l), .. } => *l,
        k => panic!("not a live shape: {k:?}"),
    }
}

fn locked(app: &VectorcraftApp, id: u64) -> bool {
    app.session.active().unwrap().doc.node(NodeId(id)).unwrap().locked
}

/// A basic shape selected with the Selection tool shows Shape Properties in place of Lock: a
/// rectangle, rounded rectangle, ellipse or polygon, each under its own heading.
#[test]
fn a_basic_shape_shows_its_properties_in_place_of_lock() {
    let mut app = app();
    let shapes = [
        (id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap()), "Rectangle Properties:"),
        (id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60, "radius": 10})).unwrap()), "Rectangle Properties:"),
        (id(app.run("shape.ellipse", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap()), "Ellipse Properties:"),
        (id(app.run("shape.polygon", json!({"cx": 100, "cy": 100, "radius": 40, "sides": 6})).unwrap()), "Polygon Properties:"),
    ];
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    for (shape, heading) in shapes {
        select(&mut app, &[shape]);
        let (_, texts) = bar(&mut app);
        assert!(!has(&texts, heading), "the popup starts closed: {texts:?}");
        let (_, texts) = click_lock_slot(&mut app);
        assert!(has(&texts, heading), "{heading}: {texts:?}");
        assert!(!locked(&app, shape), "the button doesn't lock the shape");
    }
}

/// The popup's fields edit the shape through `object.setLiveShape`, and Undo restores it.
#[test]
fn the_shape_properties_popup_edits_the_shape() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let corners = |app: &VectorcraftApp, id: u64| match live(app, id) {
        vectorcraft_doc::LiveShape::Rectangle { radii, .. } => radii,
        l => panic!("{l:?}"),
    };
    let rect = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60, "radius": 10})).unwrap());
    select(&mut app, &[rect]);
    let (ctx, texts) = click_lock_slot(&mut app);
    type_into(&mut app, &ctx, &texts, "10 pt", "20");
    assert_eq!(corners(&app, rect), [20.0; 4]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(corners(&app, rect), [10.0; 4]);

    let ellipse = id(app.run("shape.ellipse", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap());
    select(&mut app, &[ellipse]);
    let pie = |app: &VectorcraftApp| match live(app, ellipse) {
        vectorcraft_doc::LiveShape::Ellipse { pie, .. } => pie,
        l => panic!("{l:?}"),
    };
    let (ctx, texts) = click_lock_slot(&mut app);
    type_into(&mut app, &ctx, &texts, "360°", "180");
    assert_eq!(pie(&app), (0.0, 180.0));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(pie(&app), (0.0, 360.0));

    let polygon = id(app.run("shape.polygon", json!({"cx": 100, "cy": 100, "radius": 40, "sides": 6})).unwrap());
    select(&mut app, &[polygon]);
    let sides = |app: &VectorcraftApp| match live(app, polygon) {
        vectorcraft_doc::LiveShape::Polygon { sides, .. } => sides,
        l => panic!("{l:?}"),
    };
    let (ctx, texts) = click_lock_slot(&mut app);
    type_into(&mut app, &ctx, &texts, "6", "5");
    assert_eq!(sides(&app), 5);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(sides(&app), 6);
}

/// The popup stays open while a polygon's Corner dropdown is open, so a corner type can be picked
/// from it.
#[test]
fn the_shape_properties_popup_keeps_its_corner_dropdown() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let polygon = id(app.run("shape.polygon", json!({"cx": 100, "cy": 100, "radius": 40, "sides": 6})).unwrap());
    select(&mut app, &[polygon]);
    let (ctx, texts) = click_lock_slot(&mut app);
    let click = |app: &mut VectorcraftApp, p: Pos2| {
        let press = |pressed| Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, &ctx, vec![Event::PointerMoved(p), press(true), press(false)]);
        frame(app, &ctx, vec![])
    };
    let texts = click(&mut app, at(&texts, "Round"));
    assert!(has(&texts, "Polygon Properties:") && has(&texts, "Chamfer"), "the popup and its dropdown: {texts:?}");
    click(&mut app, at(&texts, "Chamfer"));
    let vectorcraft_doc::LiveShape::Polygon { kinds, .. } = live(&app, polygon) else { panic!("not a polygon") };
    assert!(!kinds.is_empty() && kinds.iter().all(|k| *k == vectorcraft_geom::shapes::CornerKind::Chamfer), "{kinds:?}");
}

/// Every other selection keeps Lock: a plain closed path, a star (a plain path), a path with live
/// corners, a group and several objects. The Direct Selection tool keeps it for a basic shape.
#[test]
fn other_selections_keep_lock() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let closed = json!({"anchors": [{"x": 50, "y": 200}, {"x": 150, "y": 80}, {"x": 250, "y": 200}], "closed": true});
    let path = id(app.run("path.create", closed).unwrap());
    let star = id(app.run("shape.star", json!({"cx": 200, "cy": 150, "radius1": 40})).unwrap());
    let rounded = id(app.run("shape.star", json!({"cx": 300, "cy": 150, "radius1": 40})).unwrap());
    select(&mut app, &[rounded]);
    app.run("object.setLiveShape", json!({"radius": 4})).unwrap();
    assert!(matches!(live(&app, rounded), vectorcraft_doc::LiveShape::Path { .. }));
    let a = id(app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 30, "height": 30})).unwrap());
    let b = id(app.run("shape.ellipse", json!({"x": 60, "y": 20, "width": 30, "height": 30})).unwrap());
    select(&mut app, &[a, b]);
    let several = [a, b];
    let c = id(app.run("shape.rectangle", json!({"x": 20, "y": 250, "width": 30, "height": 30})).unwrap());
    let d = id(app.run("shape.rectangle", json!({"x": 60, "y": 250, "width": 30, "height": 30})).unwrap());
    select(&mut app, &[c, d]);
    let group = id(app.run("object.group", json!({})).unwrap());
    let headings = ["Rectangle Properties:", "Ellipse Properties:", "Polygon Properties:"];
    for ids in [vec![path], vec![star], vec![rounded], vec![group], several.to_vec()] {
        select(&mut app, &ids);
        let (_, texts) = click_lock_slot(&mut app);
        assert!(!headings.iter().any(|h| has(&texts, h)), "{ids:?}: {texts:?}");
        assert!(ids.iter().all(|i| locked(&app, *i)), "{ids:?} locks");
        app.run("object.unlockAll", json!({})).unwrap();
    }
    let shape = id(app.run("shape.rectangle", json!({"x": 300, "y": 40, "width": 30, "height": 30})).unwrap());
    for tool in ["directSelection", "groupSelection"] {
        app.run("tool.select", json!({ "tool": tool })).unwrap();
        select(&mut app, &[shape]);
        let (_, texts) = click_lock_slot(&mut app);
        assert!(!has(&texts, "Rectangle Properties:"), "{tool}: {texts:?}");
        assert!(locked(&app, shape), "{tool} keeps Lock");
        app.run("object.unlockAll", json!({})).unwrap();
    }
}

/// A compound path under the Direct Selection tool isn't offered Edit Path, which would only switch
/// to the tool already in use; the Selection tool still offers it.
#[test]
fn a_compound_path_under_direct_selection_drops_edit_path() {
    let mut app = app();
    let a = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 120, "height": 100})).unwrap());
    let b = id(app.run("shape.ellipse", json!({"x": 70, "y": 60, "width": 50, "height": 50})).unwrap());
    select(&mut app, &[a, b]);
    let c = id(app.run("object.compoundPath.make", json!({})).unwrap());
    click(&mut app, "Edit Path");
    assert_eq!(app.session.tool_id(), "directSelection");
    select(&mut app, &[c]);
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Release Compound Path") && !has(&texts, "Edit Path"), "{texts:?}");
}
