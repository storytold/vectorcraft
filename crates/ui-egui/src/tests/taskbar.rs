//! The Contextual Task Bar's actions for each kind of selection: open paths offer Join and Edit
//! Path, compound paths and clipping masks their Release, images selected with vector art Make
//! Clipping Mask, type converts between point and area type, type on a path opens its options, and
//! a basic shape shows its properties in place of Lock. Type, and the Type tools while they edit it,
//! show the font family, font style and Font Size controls.

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

/// The names and places of the buttons the task bar draws, from the accessibility tree: an
/// icon-only button is named by its hover text.
fn buttons(app: &mut VectorcraftApp) -> (egui::Context, Vec<(String, Rect)>) {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    frame(app, &ctx, vec![]);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
    let named = nodes
        .iter()
        .filter(|(_, n)| n.role() == egui::accesskit::Role::Button)
        .filter_map(|(_, n)| {
            let b = n.bounds()?;
            Some((n.label()?.to_string(), Rect::from_min_max(Pos2::new(b.x0 as f32, b.y0 as f32), Pos2::new(b.x1 as f32, b.y1 as f32))))
        })
        .collect();
    (ctx, named)
}

/// Click the task bar's button named `name` (its label, or an icon-only button's hover text).
fn click_button(app: &mut VectorcraftApp, name: &str) {
    let (ctx, named) = buttons(app);
    let p = named.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no button named {name:?} in {named:?}")).1.center();
    let press = |pressed| Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, &ctx, vec![Event::PointerMoved(p)]);
    frame(app, &ctx, vec![press(true)]);
    frame(app, &ctx, vec![press(false)]);
    frame(app, &ctx, vec![]);
}

const TO_CORNER: &str = "Convert Selected Anchor Points to Corner";
const TO_SMOOTH: &str = "Convert Selected Anchor Points to Smooth";

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
    let dir = crate::tests_folder::Folder::new("ui-taskbar");
    let path = dir.file("blue.png", &png);
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    app.run("file.place", json!({"path": path, "link": false})).unwrap();
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

#[test]
fn type_on_a_path_offers_its_options() {
    let mut app = app();
    let path = id(app.run("path.create", json!({"d": "M60 150 L340 150"})).unwrap());
    let t = id(app.run("text.createInPath", json!({"path": path, "mode": "onPath", "text": "Along", "at": [200, 160]})).unwrap());
    assert!(matches!(kind(&app, t), NodeKind::Text(ref t) if matches!(t.kind, TextKind::OnPath { .. })));
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    select(&mut app, &[t]);
    // A non-default effect, so the dialog shows the selected type's options rather than defaults.
    app.run("type.pathOptions", json!({"effect": "skew"})).unwrap();
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Create Outlines") && has(&texts, "Type on a Path Options"), "{texts:?}");
    assert!(!has(&texts, "Convert To Area Type") && !has(&texts, "Convert To Point Type"), "{texts:?}");
    click(&mut app, "Type on a Path Options");
    let d = app.ui.dialog.as_ref().expect("the options dialog opens");
    assert_eq!(d.str("__command"), "type.pathOptions");
    assert_eq!(d.fields.get("effect"), Some(&json!("skew")));
}

/// Type on a path offers Dynamic Text, which turns it on and off; type in a thread does not.
#[test]
fn type_on_a_path_offers_dynamic_text() {
    let mut app = app();
    let path = id(app.run("path.create", json!({"d": "M60 150 L340 150"})).unwrap());
    let t = id(app.run("text.createInPath", json!({"path": path, "mode": "onPath", "text": "Along", "at": [60, 150]})).unwrap());
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    select(&mut app, &[t]);
    let dynamic = |app: &VectorcraftApp| matches!(kind(app, t), NodeKind::Text(ref t) if t.dynamic_text);
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Dynamic Text"), "{texts:?}");
    click(&mut app, "Dynamic Text");
    assert!(dynamic(&app));
    click(&mut app, "Dynamic Text");
    assert!(!dynamic(&app));
    // Threaded, it is not offered.
    let frame = id(app.run("shape.rectangle", json!({"x": 60, "y": 200, "width": 200, "height": 100})).unwrap());
    select(&mut app, &[t, frame]);
    app.run("text.thread.create", json!({})).unwrap();
    select(&mut app, &[t]);
    let (_, texts) = bar(&mut app);
    assert!(!has(&texts, "Dynamic Text"), "{texts:?}");
}

/// Type on a closed path offers Center on Top and Center on Bottom; type on an open path does not.
#[test]
fn type_on_a_closed_path_offers_center_on_top_and_bottom() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let line = id(app.run("path.create", json!({"d": "M60 150 L340 150"})).unwrap());
    let open = id(app.run("text.createInPath", json!({"path": line, "mode": "onPath", "text": "Along", "at": [60, 150]})).unwrap());
    select(&mut app, &[open]);
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Type on a Path Options") && !has(&texts, "Center on Top") && !has(&texts, "Center on Bottom"), "{texts:?}");
    let circle = id(app.run("path.create", json!({"d": "M100 300 A100 100 0 1 1 300 300 A100 100 0 1 1 100 300 Z"})).unwrap());
    let t = id(app.run("text.createInPath", json!({"path": circle, "mode": "onPath", "text": "Round", "at": [90, 300]})).unwrap());
    select(&mut app, &[t]);
    let start = |app: &VectorcraftApp| match kind(app, t) {
        NodeKind::Text(t) => match t.kind {
            TextKind::OnPath { start, .. } => start,
            k => panic!("{k:?}"),
        },
        k => panic!("{k:?}"),
    };
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Center on Top") && has(&texts, "Center on Bottom"), "{texts:?}");
    click(&mut app, "Center on Top");
    assert!((start(&app) - 0.75).abs() < 1e-3, "{}", start(&app));
    click(&mut app, "Center on Bottom");
    assert!((start(&app) - 0.25).abs() < 1e-3, "{}", start(&app));
}

/// Area type offers Dynamic Text beside Convert To Point Type, which turns Block on and off; area
/// type in a thread does not.
#[test]
fn area_type_offers_dynamic_text() {
    let mut app = app();
    let t = id(app.run("text.create", json!({"x": 60, "y": 80, "area": {"width": 200, "height": 100}, "text": "Fit me"})).unwrap());
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    select(&mut app, &[t]);
    let dynamic = |app: &VectorcraftApp| matches!(kind(app, t), NodeKind::Text(ref t) if t.dynamic_text);
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Convert To Point Type") && has(&texts, "Dynamic Text"), "{texts:?}");
    click(&mut app, "Dynamic Text");
    assert!(dynamic(&app));
    click(&mut app, "Dynamic Text");
    assert!(!dynamic(&app));
    // Threaded, it is not offered.
    let frame = id(app.run("shape.rectangle", json!({"x": 60, "y": 300, "width": 200, "height": 100})).unwrap());
    select(&mut app, &[t, frame]);
    app.run("text.thread.create", json!({})).unwrap();
    select(&mut app, &[t]);
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Convert To Point Type") && !has(&texts, "Dynamic Text"), "{texts:?}");
}

/// Point type offers Dynamic Text beside Convert To Area Type, which turns it into area type with
/// Block on; the bar then offers what area type does.
#[test]
fn point_type_offers_dynamic_text() {
    let mut app = app();
    let t = id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Fit\nme in a frame"})).unwrap());
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    select(&mut app, &[t]);
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Convert To Area Type") && has(&texts, "Dynamic Text"), "{texts:?}");
    click(&mut app, "Dynamic Text");
    assert!(matches!(kind(&app, t), NodeKind::Text(ref t) if t.dynamic_text && matches!(t.kind, TextKind::Area { .. })));
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Convert To Point Type") && has(&texts, "Dynamic Text"), "{texts:?}");
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
    let (_, named) = buttons(&mut app);
    let names: Vec<&str> = named.iter().map(|(n, _)| n.as_str()).collect();
    for name in ["Connect", TO_CORNER, TO_SMOOTH, "Simplify", "Smooth", "Duplicate"] {
        assert!(names.contains(&name), "{name}: {names:?}");
    }
    for name in ["Edit Path", "Offset Path", "Join", "Remove Anchor Points", "Cut Path"] {
        assert!(!names.contains(&name), "{name}: {names:?}");
    }
    // The convert buttons show their icons alone: the bar draws no convert text, and one "Smooth".
    let (_, texts) = bar(&mut app);
    assert!(!has(&texts, "Corner") && !texts.iter().any(|(t, _)| t.starts_with("Convert")), "{texts:?}");
    assert_eq!(texts.iter().filter(|(t, _)| t == "Smooth").count(), 1, "{texts:?}");
    // The convert buttons convert every anchor of the path, one undo step each.
    click_button(&mut app, TO_SMOOTH);
    let a = anchors(&app, line);
    assert_eq!(a[1].kind, vectorcraft_geom::AnchorKind::Smooth, "{a:?}");
    assert!(a[1].has_in() && a[1].has_out(), "{a:?}");
    click_button(&mut app, TO_CORNER);
    assert!(anchors(&app, line).iter().all(|a| !a.has_in() && !a.has_out()), "{:?}", anchors(&app, line));
    app.run("edit.undo", json!({})).unwrap();
    assert!(anchors(&app, line)[1].has_in(), "Corner undoes in one step");
    // Connect closes the open path; a closed one isn't offered it.
    click(&mut app, "Connect");
    assert!(matches!(kind(&app, line), NodeKind::Path { path, .. } if path.is_closed()));
    let (_, named) = buttons(&mut app);
    assert!(!named.iter().any(|(n, _)| n == "Connect") && named.iter().any(|(n, _)| n == TO_CORNER), "{named:?}");
}

/// With the Direct Selection tool, the bar's Smooth smooths the path's curvature: it opens the
/// Smooth slider, as Object › Path › Smooth… does, and changes nothing until the slider is
/// confirmed (#6784). Simplify opens its slider in the same way.
#[test]
fn direct_selection_smooth_opens_the_smooth_slider() {
    for (name, kind) in [("Smooth", crate::dialogs::smooth_slider::KIND), ("Simplify", crate::dialogs::simplify_slider::KIND)] {
        let mut app = app();
        // A jagged line of 40 anchor points, which Smooth changes at its opening level.
        let anchors_in: Vec<_> = (0..40).map(|i| json!({"x": 50 + i * 5, "y": 150 + (i % 2) * 6})).collect();
        let line = id(app.run("path.create", json!({"anchors": anchors_in})).unwrap());
        app.run("tool.select", json!({"tool": "directSelection"})).unwrap();
        select(&mut app, &[line]);
        let before = anchors(&app, line);
        let undo = app.session.active().unwrap().history.undo.len();
        click_button(&mut app, name);
        let d = app.ui.dialog.as_ref().unwrap_or_else(|| panic!("{name} opens its slider: {}", app.ui.status));
        assert_eq!(d.kind, kind, "{name}");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "{name} adds no undo step before it is confirmed");
        crate::dialogs::cancel(&mut app);
        assert_eq!(anchors(&app, line), before, "{name}: canceling leaves the path as it was");
        if name != "Smooth" {
            continue;
        }
        // Confirmed, the slider smooths the path as one undo step, which is not the anchor convert.
        click_button(&mut app, name);
        crate::dialogs::confirm(&mut app).unwrap();
        let history = &app.session.active().unwrap().history.undo;
        assert_eq!(history.len(), undo + 1, "one undo step");
        assert_ne!(history.last().unwrap().label, "Convert Anchor Points");
        assert_ne!(anchors(&app, line), before, "Smooth changes the path's curvature");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(anchors(&app, line), before, "undo restores the path");
    }
}

/// Anchors picked out with the Direct Selection tool keep the full set: Remove and Cut need them.
#[test]
fn direct_selected_anchors_still_offer_remove_and_cut() {
    let mut app = app();
    let line = id(app.run("path.create", json!({"anchors": [{"x": 50, "y": 200}, {"x": 150, "y": 80}, {"x": 250, "y": 200}]})).unwrap());
    app.run("tool.select", json!({"tool": "directSelection"})).unwrap();
    app.run("select.anchors", json!({"id": line, "anchors": [[0, 1]]})).unwrap();
    let (_, named) = buttons(&mut app);
    let names: Vec<&str> = named.iter().map(|(n, _)| n.as_str()).collect();
    for name in ["Remove Anchor Points", "Connect", "Cut Path", TO_CORNER, TO_SMOOTH] {
        assert!(names.contains(&name), "{name}: {names:?}");
    }
    assert!(!names.contains(&"Edit Path"), "{names:?}");
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

/// After a switch to another document and back, the task bar shows the actions for the active
/// document's selection: an open path's Join in the first, a rectangle's Shape Properties in the
/// second.
#[test]
fn the_task_bar_follows_the_active_document() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let line = id(app.run("shape.line", json!({"x1": 50, "y1": 50, "x2": 200, "y2": 120})).unwrap());
    select(&mut app, &[line]);
    assert!(has(&bar(&mut app).1, "Join"));
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let rect = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap());
    select(&mut app, &[rect]);
    let (_, texts) = bar(&mut app);
    assert!(!has(&texts, "Join") && has(&texts, "Duplicate"), "{texts:?}");
    app.session.set_active(0);
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Join"), "{texts:?}");
    app.session.set_active(1);
    let (_, texts) = bar(&mut app);
    assert!(!has(&texts, "Join"), "{texts:?}");
    let (_, texts) = click_lock_slot(&mut app);
    assert!(has(&texts, "Rectangle Properties:"), "{texts:?}");
    assert!(!locked(&app, rect), "the button doesn't lock the shape");
}

/// The Type tools whose bar shows the font and Fill controls.
const TYPE_TOOLS: [&str; 6] = ["type", "areaType", "typeOnPath", "verticalType", "verticalAreaType", "verticalTypeOnPath"];

/// The labels drawn inside the task bar (none while it isn't shown), with the context.
fn bar_texts(app: &mut VectorcraftApp) -> (egui::Context, Vec<(String, Rect)>) {
    let (ctx, texts) = bar(app);
    let Some(r) = canvas::task_bar_rect(&ctx) else { return (ctx, vec![]) };
    (ctx, texts.into_iter().filter(|(_, t)| r.contains(t.center())).collect())
}

/// Point type "Hello" at 20 pt, made with `tool` and edited by it as after a click with it.
fn edited_type(app: &mut VectorcraftApp, tool: &str) -> u64 {
    app.run("tool.select", json!({ "tool": tool })).unwrap();
    let t = id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Hello", "size": 20})).unwrap());
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    assert!(crate::panels::character::text_editing(app).is_some(), "{tool} edits the type");
    t
}

/// The font family and style of type `t`'s first character.
fn font_of(app: &VectorcraftApp, t: u64) -> (String, String) {
    match kind(app, t) {
        NodeKind::Text(t) => (t.first_style().font_family, t.first_style().font_style),
        k => panic!("{k:?}"),
    }
}

/// The size of each run of type `t`, with its text.
fn sizes(app: &VectorcraftApp, t: u64) -> Vec<(String, f64)> {
    match kind(app, t) {
        NodeKind::Text(t) => t.runs.iter().map(|r| (r.text.clone(), r.style.size)).collect(),
        k => panic!("{k:?}"),
    }
}

/// Each Type tool that adds or edits type shows the font family, font style and Font Size controls
/// and the Fill, without the selection tools' object actions, the Stroke chip or Lock.
#[test]
fn the_type_tools_show_font_and_fill_controls() {
    for tool in TYPE_TOOLS {
        let mut app = app();
        let t = edited_type(&mut app, tool);
        let (family, style) = font_of(&app, t);
        let (_, texts) = bar_texts(&mut app);
        for label in [family.as_str(), style.as_str(), "20 pt"] {
            assert!(has(&texts, label), "{tool}: {label} in {texts:?}");
        }
        for label in ["Create Outlines", "Convert To Area Type", "Dynamic Text", "Duplicate"] {
            assert!(!has(&texts, label), "{tool}: {label} in {texts:?}");
        }
        let tree = crate::widgets::accessibility_tests::tree(|ui| canvas::show(&mut app, ui));
        let named = |label: &str| tree.iter().any(|n| n.label().is_some_and(|l| l.starts_with(label)));
        assert!(named("Fill"), "{tool}: the Fill shows");
        for label in ["Stroke", "Recolor Artwork", "Lock"] {
            assert!(!named(label), "{tool}: {label} is left out");
        }
    }
}

/// A Type tool shows no bar before it adds type, nor with other art selected; the Touch Type tool
/// shows none with type selected.
#[test]
fn the_type_tools_show_no_bar_without_type() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "type"})).unwrap();
    let (ctx, _) = bar(&mut app);
    assert!(canvas::task_bar_rect(&ctx).is_none(), "nothing to format");
    let rect = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap());
    select(&mut app, &[rect]);
    app.run("tool.select", json!({"tool": "type"})).unwrap();
    let (ctx, _) = bar(&mut app);
    assert!(canvas::task_bar_rect(&ctx).is_none(), "a rectangle is not type");
    let t = id(app.run("text.create", json!({"x": 60, "y": 180, "text": "Hello", "size": 20})).unwrap());
    app.run("tool.select", json!({"tool": "touchType"})).unwrap();
    select(&mut app, &[t]);
    let (ctx, _) = bar(&mut app);
    assert!(canvas::task_bar_rect(&ctx).is_none(), "Touch Type has its own handles");
}

/// Selected type keeps its actions and gains the font controls, for one type object or several;
/// type selected with other art does not.
#[test]
fn selected_type_shows_font_controls_beside_its_actions() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let a = id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Hello", "size": 20})).unwrap());
    let b = id(app.run("text.create", json!({"x": 60, "y": 160, "text": "World", "size": 20})).unwrap());
    let rect = id(app.run("shape.rectangle", json!({"x": 200, "y": 40, "width": 80, "height": 60})).unwrap());
    let (family, _) = font_of(&app, a);
    select(&mut app, &[a]);
    let (_, texts) = bar_texts(&mut app);
    for label in ["Create Outlines", "Convert To Area Type", "Duplicate", family.as_str(), "20 pt"] {
        assert!(has(&texts, label), "{label} in {texts:?}");
    }
    select(&mut app, &[a, b]);
    let (_, texts) = bar_texts(&mut app);
    assert!(has(&texts, family.as_str()) && has(&texts, "20 pt"), "{texts:?}");
    select(&mut app, &[a, rect]);
    let (_, texts) = bar_texts(&mut app);
    assert!(has(&texts, "Group") && !has(&texts, family.as_str()) && !has(&texts, "20 pt"), "{texts:?}");
    select(&mut app, &[rect]);
    let (_, texts) = bar_texts(&mut app);
    assert!(!has(&texts, family.as_str()) && !has(&texts, "20 pt"), "{texts:?}");
}

/// The bar's Font Size sets the characters a Type tool selects, one undo step; with type selected
/// it sets every selected type object, as the Character panel's does.
#[test]
fn the_task_bars_font_size_sets_what_the_character_panel_would() {
    let mut app = app();
    let t = edited_type(&mut app, "type");
    app.session.set_tool_option("select", &json!({"start": 0, "end": 2}));
    let (ctx, texts) = bar_texts(&mut app);
    type_into(&mut app, &ctx, &texts, "20 pt", "30");
    assert_eq!(sizes(&app, t), [("He".to_string(), 30.0), ("llo".to_string(), 20.0)]);
    // Mixed sizes leave the field blank, as in the Character panel.
    app.session.set_tool_option("select", &json!({"start": 0, "end": 5}));
    let (_, texts) = bar_texts(&mut app);
    assert!(!has(&texts, "20 pt") && !has(&texts, "30 pt"), "{texts:?}");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(sizes(&app, t), [("Hello".to_string(), 20.0)]);

    let mut app = self::app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let a = id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Hello", "size": 20})).unwrap());
    let b = id(app.run("text.create", json!({"x": 60, "y": 160, "text": "World", "size": 20})).unwrap());
    select(&mut app, &[a, b]);
    let (ctx, texts) = bar_texts(&mut app);
    type_into(&mut app, &ctx, &texts, "20 pt", "14");
    assert_eq!(sizes(&app, a), [("Hello".to_string(), 14.0)]);
    assert_eq!(sizes(&app, b), [("World".to_string(), 14.0)]);
}

/// At a caret, with no characters selected, the bar's Font Size sets the edited type as the
/// Character panel's does, and the characters typed next take it.
#[test]
fn the_task_bars_font_size_at_a_caret_sets_what_is_typed_next() {
    let mut app = app();
    let t = edited_type(&mut app, "type");
    app.session.set_tool_option("select", &json!({"start": 5, "end": 5}));
    assert_eq!(crate::panels::character::text_editing(&app), Some((NodeId(t), 5, 5)), "a caret after the type");
    let (ctx, texts) = bar_texts(&mut app);
    type_into(&mut app, &ctx, &texts, "20 pt", "30");
    app.session.tool_typed_text("!", app.view_info()).unwrap();
    assert_eq!(sizes(&app, t), [("Hello!".to_string(), 30.0)]);
}

/// The Type tool's Fill chip makes the Fill active and brings up the Swatches panel.
#[test]
fn the_type_tools_fill_chip_activates_the_fill() {
    let mut app = app();
    edited_type(&mut app, "type");
    app.session.fill_active = false;
    let (ctx, _) = bar_texts(&mut app);
    let r = canvas::task_bar_rect(&ctx).expect("the task bar shows");
    // Right to left: the frame's margin, More Options, then the Fill chip.
    let p = Pos2::new(r.right() - 8.0 - 30.0 - 6.0 - 13.0, r.center().y);
    let press = |pressed| Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![Event::PointerMoved(p)]);
    frame(&mut app, &ctx, vec![press(true)]);
    frame(&mut app, &ctx, vec![press(false)]);
    assert!(app.session.fill_active, "the Fill is active");
    assert!(crate::panels::character::text_editing(&app).is_some(), "the type is still edited");
}

/// Screen readers name the Type tool bar's controls, and the bar stays on the canvas for type at
/// its lower right corner.
#[test]
fn the_type_tools_bar_is_named_and_stays_on_the_canvas() {
    use crate::widgets::accessibility_tests::{named, tree};
    let mut app = app();
    edited_type(&mut app, "type");
    let tree = tree(|ui| canvas::show(&mut app, ui));
    for label in ["Font Family", "Style", "Font Size", "Fill"] {
        named(&tree, label);
    }
    let mut app = self::app();
    app.run("tool.select", json!({"tool": "type"})).unwrap();
    app.run("text.create", json!({"x": 380, "y": 290, "text": "Corner", "size": 20})).unwrap();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    // The type near the canvas's lower edge, so the bar's place under it is off the canvas.
    let view = app.view_mut().unwrap();
    (view.center, view.zoom) = (vectorcraft_geom::Point::new(200.0, 20.0), 1.0);
    let (ctx, _) = bar(&mut app);
    let r = canvas::task_bar_rect(&ctx).expect("the task bar shows");
    let canvas = app.canvas_rect.expect("the canvas is laid out");
    assert!(canvas.contains_rect(r), "{r:?} in {canvas:?}");
}

/// The task bar's rect after a few frames on a `width` by 700 screen, and the canvas's rect.
fn bar_on_screen(app: &mut VectorcraftApp, width: f32) -> (Rect, Rect) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, 700.0));
    for _ in 0..3 {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| canvas::show(app, ui));
        out.textures_delta.clear();
    }
    (canvas::task_bar_rect(&ctx).expect("the task bar shows"), app.canvas_rect.expect("the canvas is laid out"))
}

/// On a canvas narrower than one row of the bar, the font controls go on a second row, narrowed to
/// fit, and the bar stays whole on the canvas: selected type's actions, and a Type tool's Fill.
#[test]
fn selected_types_font_controls_take_a_second_row_on_a_narrow_canvas() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let t = id(app.run("text.create", json!({"x": 20, "y": 60, "text": "Hello", "size": 20})).unwrap());
    select(&mut app, &[t]);
    let (r, canvas) = bar_on_screen(&mut app, 760.0);
    assert!(canvas.contains_rect(r), "{r:?} in {canvas:?}");
    assert!(r.height() > 60.0, "the font controls take a second row: {r:?}");

    let mut app = self::app();
    app.run("tool.select", json!({"tool": "type"})).unwrap();
    app.run("text.create", json!({"x": 20, "y": 60, "text": "Hello", "size": 20})).unwrap();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    // Narrower than the font controls at their full widths too.
    let (r, canvas) = bar_on_screen(&mut app, 300.0);
    assert!(canvas.contains_rect(r), "{r:?} in {canvas:?}");
    assert!(r.height() > 60.0, "the font controls take a second row: {r:?}");
    let (r, _) = bar_on_screen(&mut app, 1000.0);
    assert!(r.height() < 60.0, "one row where it fits: {r:?}");
    // The Type tool's bar has only the Fill and More Options beside the controls: one row fits.
    let (r, canvas) = bar_on_screen(&mut app, 460.0);
    assert!(canvas.contains_rect(r) && r.height() < 60.0, "one row on a 460 px screen: {r:?} in {canvas:?}");
    // Too narrow for the controls at their narrowest: the bar leaves them out.
    let (r, canvas) = bar_on_screen(&mut app, 160.0);
    assert!(canvas.contains_rect(r) && r.height() < 60.0, "no font controls on a 160 px screen: {r:?} in {canvas:?}");
}

/// When the canvas narrows so far that the bar leaves its font controls out, the font family
/// field it drew before no longer takes the mouse wheel.
#[test]
fn a_task_bar_without_its_font_controls_leaves_their_wheel() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "type"})).unwrap();
    app.run("text.create", json!({"x": 20, "y": 60, "text": "Hello", "size": 20})).unwrap();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let draw = |app: &mut VectorcraftApp, width: f32| {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, 700.0));
        for _ in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| canvas::show(app, ui));
            out.textures_delta.clear();
        }
    };
    draw(&mut app, 1000.0);
    let bar = canvas::task_bar_rect(&ctx).expect("the task bar shows");
    let viewport = egui::ViewportId::ROOT;
    let points = (0..(bar.width() / 4.0) as usize).flat_map(|x| (0..(bar.height() / 4.0) as usize).map(move |y| (x, y)));
    let field = points
        .map(|(x, y)| bar.min + vec2(x as f32 * 4.0, y as f32 * 4.0))
        .find(|p| crate::font_menu::wheel_field_layer_for_test(&ctx, viewport, *p).is_some())
        .expect("the bar's font family field takes the wheel");
    draw(&mut app, 160.0);
    assert!(canvas::task_bar_rect(&ctx).is_some(), "the bar still shows, without its font controls");
    crate::font_menu::sync_wheel_target(&app, &ctx, viewport);
    assert!(crate::font_menu::wheel_field_layer_for_test(&ctx, viewport, field).is_none(), "the left-out field at {field:?} still takes the wheel");

    // Hiding the bar (Window › Contextual Task Bar) releases the field before the next layout.
    draw(&mut app, 1000.0);
    assert!(crate::font_menu::wheel_field_layer_for_test(&ctx, viewport, field).is_some(), "the field is back");
    app.run("window.taskBar", json!({})).unwrap();
    crate::font_menu::sync_wheel_target(&app, &ctx, viewport);
    assert!(
        crate::font_menu::wheel_field_layer_for_test(&ctx, viewport, field).is_none(),
        "the hidden bar's field at {field:?} still takes the wheel"
    );
}

/// While a Type tool edits type, the Fill chip shows the selected characters' fill, and "?" where
/// their fills differ.
#[test]
fn the_type_tools_fill_chip_shows_the_edited_characters_fill() {
    let mut app = app();
    let t = edited_type(&mut app, "type");
    app.session.set_tool_option("select", &json!({"start": 2, "end": 5}));
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    let fills: Vec<_> = match kind(&app, t) {
        NodeKind::Text(t) => t.runs.iter().map(|r| (r.text.clone(), r.style.fill.clone())).collect(),
        k => panic!("{k:?}"),
    };
    let [(he, he_fill), (llo, llo_fill)] = &fills[..] else { panic!("{fills:?}") };
    assert_eq!((he.as_str(), llo.as_str()), ("He", "llo"));
    assert_ne!(he_fill, llo_fill);
    let ctx = egui::Context::default();
    let (fill, _, mixed, _) = canvas::chip_paints(&app, &ctx);
    assert!(fill == *llo_fill && !mixed, "{fill:?} {mixed}");
    app.session.set_tool_option("select", &json!({"start": 0, "end": 2}));
    let (fill, _, mixed, _) = canvas::chip_paints(&app, &ctx);
    assert!(fill == *he_fill && !mixed, "{fill:?} {mixed}");
    app.session.set_tool_option("select", &json!({"start": 0, "end": 5}));
    let (_, _, mixed, _) = canvas::chip_paints(&app, &ctx);
    assert!(mixed, "the fills differ");
}

/// The task bar shows the selection as it is after an edit and after Undo: Join leaves once the
/// open path is closed and comes back with Undo, and Shape Properties shows a rectangle's new
/// corner radius and then its old one.
#[test]
fn the_task_bar_follows_edits_and_undo() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let line = id(app.run("shape.line", json!({"x1": 50, "y1": 50, "x2": 200, "y2": 120})).unwrap());
    select(&mut app, &[line]);
    assert!(has(&bar(&mut app).1, "Join"));
    app.run("path.join", json!({})).unwrap();
    let (_, texts) = bar(&mut app);
    assert!(!has(&texts, "Join"), "{texts:?}");
    app.run("edit.undo", json!({})).unwrap();
    let (_, texts) = bar(&mut app);
    assert!(has(&texts, "Join"), "{texts:?}");

    let rect = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60, "radius": 10})).unwrap());
    select(&mut app, &[rect]);
    let (_, texts) = click_lock_slot(&mut app);
    assert!(has(&texts, "10 pt") && !has(&texts, "20 pt"), "{texts:?}");
    app.run("object.setLiveShape", json!({"radius": 20})).unwrap();
    let (_, texts) = click_lock_slot(&mut app);
    assert!(has(&texts, "20 pt") && !has(&texts, "10 pt"), "{texts:?}");
    app.run("edit.undo", json!({})).unwrap();
    let (_, texts) = click_lock_slot(&mut app);
    assert!(has(&texts, "10 pt") && !has(&texts, "20 pt"), "{texts:?}");
    assert!(!locked(&app, rect), "the button doesn't lock the shape");
}
