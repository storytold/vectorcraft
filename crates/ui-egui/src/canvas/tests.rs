use craft::pages::ui::marks::{HiddenMark, MarkAnchor, PlacedMark};
use craft::ui::dialog::StackDialog;
use egui::{Color32, Pos2, Shape, pos2, vec2};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_geom::Shape as _;
use vectorcraft_geom::{Affine, Point};
use vectorcraft_tools::{Cursor, Mods, Overlay, PointerEvent, PointerKind};

use super::artboards::names_an_artboard;
use super::cursor::still_wait;
use super::fill::{fill_view, polygon_area};
use super::marks::{HiddenMarks, thread_overlays};
use super::outline::{node_outline, walk_drawn};
use super::raster::{frame_view, pixel_region, placed_changes, placed_key_at, placed_since, poll_worker};
use super::*;
use crate::CacheKey;
use crate::panels::actions::tests_support::{cancel_dialog, is_open};
use crate::tests::taskbar::{BAR, click, offer, on_canvas};

#[test]
fn shaper_selection_highlights_only_the_visible_result() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.ellipse", &json!({"x":100,"y":100,"width":100,"height":100})).unwrap();
    let line = vectorcraft_doc::NodeId(s.execute("shape.line", &json!({"x1":50,"y1":150,"x2":250,"y2":150})).unwrap()["id"].as_u64().unwrap());
    for x in [0.0, 150.0] {
        s.execute("shaper.scribble", &json!({"points":[[60.0+x,140],[70.0+x,160],[80.0+x,140],[90.0+x,160]]})).unwrap();
    }
    let st = s.active().unwrap();
    let g = st.doc.node(st.selection.objects[0]).unwrap();
    assert!(g.extras().shaper.is_some());
    let symbols = st.doc.symbol_boxes();
    let bounds = node_outline(g, &symbols).bounding_box();
    assert!((bounds.x0 - 100.0).abs() < 0.01 && (bounds.x1 - 200.0).abs() < 0.01, "{bounds:?}");
    assert_eq!(node_outline(st.doc.node(line).unwrap(), &symbols).bounding_box().x0, 50.0, "an isolated original still shows its full outline");
}

#[test]
fn a_symbol_instance_is_outlined_around_its_art() {
    // A symbol holding the square of the SVG import tests' `Star` symbol, 20 × 20 from (0, 0),
    // and an instance of it at (20, 30).
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><defs><symbol id="Star"><path d="M0 0H20V20H0Z" fill="#e0a000"/></symbol></defs><use href="#Star" x="20" y="30"/></svg>"##;
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "star.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
    let st = s.active().unwrap();
    let mut inst = None;
    st.doc.walk(|n| {
        if matches!(n.kind, NodeKind::SymbolInstance { .. }) {
            inst = Some(n);
        }
    });
    assert_eq!(node_outline(inst.unwrap(), &st.doc.symbol_boxes()).bounding_box(), Rect::new(20.0, 30.0, 40.0, 50.0));
}

/// The points of `bp`'s move and line elements, in order.
fn outline_points(bp: &vectorcraft_geom::BezPath) -> Vec<Point> {
    use vectorcraft_geom::kurbo::PathEl;
    bp.elements()
        .iter()
        .filter_map(|el| match el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => Some(*p),
            _ => None,
        })
        .collect()
}

/// Whether `got` are the points `want`, each within 1e-9, a last point of `got` that repeats its
/// first left out.
fn same_points(got: &[Point], want: &[Point]) -> bool {
    let near = |a: &Point, b: &Point| a.distance(*b) < 1e-9;
    let got = match got {
        [first, rest @ .., last] if !rest.is_empty() && near(first, last) => &got[..got.len() - 1],
        _ => got,
    };
    got.len() == want.len() && got.iter().zip(want).all(|(a, b)| near(a, b))
}

/// Puts `n` in perspective, drawn through `h`.
fn in_perspective(n: &mut Node, h: vectorcraft_geom::Homography) {
    let at = vectorcraft_doc::PerspectiveAttachment { projection: Some(h.to_array()), ..vectorcraft_doc::PerspectiveAttachment::new("left", 0.0) };
    n.extras_mut().perspective = Some(Box::new(at));
}

#[test]
fn a_turned_or_skewed_symbol_instance_is_outlined_along_its_frame() {
    // The 20 × 20 square symbol of the test above, its instance at (20, 30).
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><defs><symbol id="Star"><path d="M0 0H20V20H0Z" fill="#e0a000"/></symbol></defs><use href="#Star" x="20" y="30"/></svg>"##;
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "star.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
    let st = s.active().unwrap();
    let mut inst = None;
    st.doc.walk(|n| {
        if matches!(n.kind, NodeKind::SymbolInstance { .. }) {
            inst = Some(n.clone());
        }
    });
    let inst = inst.unwrap();
    let symbols = st.doc.symbol_boxes();
    let square = [(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)].map(|(x, y)| Point::new(x, y));
    let placed = |a: Affine| {
        let mut n = inst.clone();
        let NodeKind::SymbolInstance { xf, .. } = &mut n.kind else { panic!("an instance") };
        *xf = a * *xf;
        let xf = *xf;
        (n, square.map(|p| xf * p))
    };
    for (case, a) in [
        ("unturned", Affine::IDENTITY),
        ("turned 30 degrees", Affine::rotate_about(30f64.to_radians(), Point::new(30.0, 40.0))),
        ("skewed", Affine::skew(0.5, 0.0)),
    ] {
        let (n, frame) = placed(a);
        let got = outline_points(&node_outline(&n, &symbols));
        assert!(same_points(&got, &frame), "{case}: {got:?} is not the frame {frame:?}");
    }
    // In perspective, the frame's corners go through the projection.
    let h = vectorcraft_geom::Homography([[1.0, 0.0, 0.0], [0.1, 1.0, 0.0], [0.004, 0.0, 1.0]]);
    let mut n = inst.clone();
    in_perspective(&mut n, h);
    let (_, flat) = placed(Affine::IDENTITY);
    let drawn = flat.map(|p| h.apply(p).unwrap());
    let got = outline_points(&node_outline(&n, &symbols));
    assert!(same_points(&got, &drawn), "in perspective: {got:?} is not the projected frame {drawn:?}");
}

#[test]
fn type_in_perspective_is_outlined_by_its_projected_box() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let r = s.execute("text.create", &json!({"x": 100, "y": 150, "text": "Hello world", "size": 24})).unwrap();
    let id = vectorcraft_doc::NodeId(r["id"].as_u64().unwrap());
    let st = s.active().unwrap();
    let mut n = st.doc.node(id).unwrap().clone();
    let NodeKind::Text(t) = &n.kind else { panic!("type") };
    let b = t.bounds().unwrap();
    let h = vectorcraft_geom::Homography([[1.0, 0.0, 0.0], [0.1, 1.0, 0.0], [0.002, 0.0, 1.0]]);
    in_perspective(&mut n, h);
    let drawn = [(b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1), (b.x0, b.y1)].map(|(x, y)| h.apply(Point::new(x, y)).unwrap());
    let got = outline_points(&node_outline(&n, &st.doc.symbol_boxes()));
    assert!(same_points(&got, &drawn), "{got:?} is not the projected box {drawn:?}");
}

/// One headless canvas frame on an 800 × 600 window.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
}

/// A press between a Live Paint Bucket triple-click's clicks that the view tools take (a
/// middle-button pan, a pan with Space held) counts as a press that paints nothing: the
/// triple-click matches the fill the face had before the first click.
#[test]
fn bucket_triple_click_with_a_view_tool_press_between_matches_the_fill_before() {
    use egui::{Event, Modifiers, PointerButton};
    use vectorcraft_color::{Color, Paint};
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    for case in ["middle", "space"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let ids: Vec<u64> = [0.0, 50.0, 100.0]
            .iter()
            .map(|x| app.session.execute("shape.rectangle", &json!({"x": x, "y": 100, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap())
            .collect();
        app.session.execute("select.set", &json!({"ids": ids})).unwrap();
        let g = app.session.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap();
        for (x, hex) in [(25, "#ff0000"), (75, "#ff0000"), (125, "#0000ff")] {
            app.session.execute("livePaint.fill", &json!({"group": g, "point": [x, 125], "color": hex})).unwrap();
        }
        app.session.execute("select.none", &json!({})).unwrap();
        app.select_tool("livePaintBucket");
        app.session.paint.fill = green.clone();
        if let Some(v) = app.view_mut() {
            v.center = Point::new(200.0, 150.0);
            v.zoom = 1.0;
            v.fitted = true;
        }
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx, vec![]);
        let xf = app.canvas_view().unwrap();
        let at = xf.to_screen(Point::new(25.0, 125.0));
        let click = |app: &mut VectorcraftApp, button, pos, modifiers| {
            let b = |pressed| Event::PointerButton { pos, button, pressed, modifiers };
            frame(app, &ctx, vec![Event::PointerMoved(pos), b(true)]);
            frame(app, &ctx, vec![b(false)]);
        };
        click(&mut app, PointerButton::Primary, at, Modifiers::NONE);
        if case == "middle" {
            click(&mut app, PointerButton::Middle, at, Modifiers::NONE);
        } else {
            let space = |pressed| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
            frame(&mut app, &ctx, vec![space(true)]);
            click(&mut app, PointerButton::Primary, at, Modifiers::NONE);
            frame(&mut app, &ctx, vec![space(false)]);
        }
        click(&mut app, PointerButton::Primary, at, Modifiers::NONE);
        frame(&mut app, &ctx, vec![]);
        let group = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(g)).unwrap().clone();
        let fill = |x: f64| vectorcraft_tools::builder::face_at(&group, Point::new(x, 125.0)).unwrap().appearance.fill_paint();
        assert_eq!([fill(25.0), fill(75.0)], [green.clone(), green.clone()], "{case}: the faces that were red");
        assert_eq!(fill(125.0), Paint::solid(Color::from_hex("#0000ff").unwrap()), "{case}: the blue face");
    }
}

/// The Image Trace panel's eye-button hold ends when the pointer is released, even when the
/// panel stopped drawing while the button was held, so a later press on the canvas does not
/// show the source image.
#[test]
fn a_release_ends_the_image_trace_hold_without_the_panel() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let made = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    let id = vectorcraft_doc::NodeId(made["id"].as_u64().unwrap());
    let ctx = crate::testing::ui_context();
    let at = pos2(400.0, 300.0);
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), press(true)]);
    app.ui.trace_hold_source = Some(std::sync::Arc::new(vec![id]));
    frame(&mut app, &ctx, vec![]);
    assert!(app.ui.trace_hold_source.is_some(), "held while the button is down");
    frame(&mut app, &ctx, vec![press(false)]);
    assert_eq!(app.ui.trace_hold_source, None, "the release ends the hold");
    frame(&mut app, &ctx, vec![press(true)]);
    assert_eq!(app.ui.trace_hold_source, None, "a later press does not show the source image");
    frame(&mut app, &ctx, vec![press(false)]);
}

/// The canvas decides which tool's marks and widgets to show for the pointer where it is in the same frame: over the
/// selected blend a Cmd press selects behind, so the Selection tool's stay; moved away in the next frame, where Cmd
/// lends Direct Selection, they are Direct Selection's in that frame.
#[test]
fn the_shown_tool_follows_the_pointer_in_the_frame_it_moves() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = |v: serde_json::Value| v["id"].as_u64().unwrap();
    // Behind the blend, an object to select behind.
    app.session.execute("shape.rectangle", &json!({"x": 20, "y": 120, "width": 360, "height": 160})).unwrap();
    let a = id(app.session.execute("shape.rectangle", &json!({"x": 60, "y": 160, "width": 80, "height": 80})).unwrap());
    let b = id(app.session.execute("shape.rectangle", &json!({"x": 260, "y": 160, "width": 80, "height": 80})).unwrap());
    app.session.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let blend = id(app.session.execute("object.blend.make", &json!({})).unwrap());
    app.select_tool("selection");
    app.session.execute("select.set", &json!({"ids": [blend]})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let xf = app.canvas_view().unwrap();
    let cmd_frame = |app: &mut VectorcraftApp, at: Point| {
        frame(app, &ctx, vec![egui::Event::ModifiersChanged(egui::Modifiers::COMMAND), egui::Event::PointerMoved(xf.to_screen(at))]);
        app.session.hover_tool_id(&Mods { cmd: true, ..Default::default() })
    };
    // Inside the first key object, away from its anchors and the spine.
    assert_eq!(cmd_frame(&mut app, Point::new(80.0, 175.0)), "selection", "over the selected blend, the press selects behind");
    assert_eq!(cmd_frame(&mut app, Point::new(200.0, 380.0)), "directSelection", "away from it, in the same frame");
}

/// A click with the Selection tool inside an artboard makes it the active one, and Paste in
/// Place then pastes onto it, where the objects were on their own artboard (#693).
#[test]
fn a_click_activates_its_artboard_and_paste_in_place_goes_onto_it() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    app.session.execute("shape.rectangle", &json!({"x": boards[0].x0 + 10.0, "y": boards[0].y0 + 15.0, "width": 20, "height": 20})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("selection");
    let view = app.view_info();
    let at = boards[2].center();
    for kind in [PointerKind::Down, PointerKind::Up] {
        dispatch(&mut app, &PointerEvent { kind, pos: at, mods: Default::default(), backquote: false, pressure: 1.0, pen: None }, view);
    }
    assert_eq!(app.view().unwrap().artboard, 2, "the clicked artboard is the active one");
    let ids: Vec<vectorcraft_doc::NodeId> = app.run("edit.pasteInPlace", json!({})).unwrap()["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| vectorcraft_doc::NodeId(v.as_u64().unwrap()))
        .collect();
    let b = app.session.active().unwrap().doc.bounds_of(&ids, false).unwrap();
    assert_eq!((b.x0, b.y0), (boards[2].x0 + 10.0, boards[2].y0 + 15.0));
}

/// A Shift-click with the Selection tool on an empty spot inside an artboard makes it the active
/// one and keeps the selection, and Align to Artboard then aligns the objects to it unless the
/// call names another artboard.
#[test]
fn a_shift_click_activates_its_artboard_and_align_to_artboard_aligns_to_it() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    let mut rect = |x: f64, y: f64| {
        let r = app.session.execute("shape.rectangle", &json!({"x": boards[0].x0 + x, "y": boards[0].y0 + y, "width": 20, "height": 20}));
        vectorcraft_doc::NodeId(r.unwrap()["id"].as_u64().unwrap())
    };
    let ids = [rect(10.0, 15.0), rect(50.0, 40.0)];
    app.session.execute("select.set", &json!({"ids": [ids[0].0, ids[1].0]})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("selection");
    let view = app.view_info();
    let shift = Mods { shift: true, ..Default::default() };
    for kind in [PointerKind::Down, PointerKind::Up] {
        dispatch(&mut app, &PointerEvent { kind, pos: boards[2].center(), mods: shift, backquote: false, pressure: 1.0, pen: None }, view);
    }
    assert_eq!(app.view().unwrap().artboard, 2, "the clicked artboard is the active one");
    assert_eq!(app.session.active().unwrap().selection.objects, ids, "the selection stays");
    let lefts = |app: &VectorcraftApp| ids.map(|id| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0);
    app.run("object.align", json!({"horizontal": "left", "to": "artboard"})).unwrap();
    assert_eq!(lefts(&app), [boards[2].x0; 2], "the left edges are on the active artboard's");
    app.run("object.align", json!({"horizontal": "left", "to": "artboard", "artboard": 1})).unwrap();
    assert_eq!(lefts(&app), [boards[1].x0; 2], "an artboard the caller names is kept");
}

/// In the Touch workspace a two-finger tap switches between the Selection tool and the drawing tool
/// chosen last, and undoes nothing; three fingers still redo. Outside it, two fingers undo again.
#[test]
fn two_finger_tap_in_touch_switches_selection_and_drawing_modes() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 40, "height": 40})).unwrap();
    crate::workspaces::switch(&mut app, crate::workspaces::TOUCH).unwrap();
    app.select_tool("rectangle");
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let count = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers[0].children().unwrap().len();
    let c = app.canvas_view().unwrap().rect.center();
    let touch = |n: u64, phase, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(n), phase, pos, force: None };
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let tap = |app: &mut VectorcraftApp, fingers: u64| {
        let at = |n: u64| c + vec2(n as f32 * 50.0, 0.0);
        frame(app, &ctx, vec![touch(0, egui::TouchPhase::Start, at(0)), egui::Event::PointerMoved(at(0)), button(at(0), true)]);
        frame(app, &ctx, vec![touch(0, egui::TouchPhase::Move, at(0) + vec2(6.0, 6.0)), egui::Event::PointerMoved(at(0) + vec2(6.0, 6.0))]);
        frame(app, &ctx, (1..fingers).map(|n| touch(n, egui::TouchPhase::Start, at(n))).collect());
        frame(app, &ctx, vec![touch(0, egui::TouchPhase::End, at(0)), button(at(0) + vec2(6.0, 6.0), false), egui::Event::PointerGone]);
        frame(app, &ctx, (1..fingers).map(|n| touch(n, egui::TouchPhase::End, at(n))).collect());
        frame(app, &ctx, vec![]);
    };
    tap(&mut app, 2);
    assert_eq!((app.session.tool_id(), count(&app)), ("selection", 1), "selection mode, nothing undone or drawn");
    tap(&mut app, 2);
    assert_eq!((app.session.tool_id(), count(&app)), ("rectangle", 1), "back to the drawing tool");
    // A tool chosen by hand becomes the drawing tool a switch returns to.
    app.select_tool("ellipse");
    tap(&mut app, 2);
    tap(&mut app, 2);
    assert_eq!((app.session.tool_id(), count(&app)), ("ellipse", 1));
    // The switches added no undo step: undo still takes the rectangle away, and three fingers redo it.
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(count(&app), 0);
    tap(&mut app, 3);
    assert_eq!(count(&app), 1, "three fingers redo in Touch too");
    // Presentation Mode runs neither.
    app.ui.screen_mode = 3;
    frame(&mut app, &ctx, vec![]);
    tap(&mut app, 2);
    assert_eq!(app.session.tool_id(), "ellipse", "Presentation Mode: no switch");
    app.ui.screen_mode = 0;
    // Enable Touch Gestures off: no switch.
    app.session.prefs.touch_gestures = false;
    app.select_tool("selection");
    frame(&mut app, &ctx, vec![]);
    tap(&mut app, 2);
    assert_eq!(app.session.tool_id(), "selection", "off: no switch");
    app.session.prefs.touch_gestures = true;
    // Another workspace: two fingers undo again and switch no tool.
    crate::workspaces::switch(&mut app, crate::workspaces::ESSENTIALS).unwrap();
    app.select_tool("rectangle");
    frame(&mut app, &ctx, vec![]);
    let before = count(&app);
    tap(&mut app, 2);
    assert_eq!(app.session.tool_id(), "rectangle");
    assert_eq!(count(&app), before - 1, "Essentials: undone");
}

/// Palm rejection: once a pen has been seen, a finger's drag on the canvas pans the view with any tool and draws
/// nothing; the pen still draws.
#[test]
fn a_finger_after_a_pen_pans_and_draws_nothing() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("rectangle");
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let count = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers[0].children().unwrap().len();
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let drag = |app: &mut VectorcraftApp, a: Pos2, b: Pos2| {
        for ev in
            [egui::Event::PointerMoved(a), button(a, true), egui::Event::PointerMoved(a.lerp(b, 0.5)), egui::Event::PointerMoved(b), button(b, false)]
        {
            frame(app, &ctx, vec![ev]);
        }
        frame(app, &ctx, vec![]);
    };
    let c = app.canvas_view().unwrap().rect.center();
    let origin = |app: &VectorcraftApp| app.canvas_view().unwrap().to_screen(Point::ZERO);
    crate::touch::set_source(crate::touch::Source::Pen);
    crate::touch::set_source(crate::touch::Source::Finger);
    let before = origin(&app);
    drag(&mut app, c, c + vec2(60.0, 40.0));
    assert_eq!(count(&app), 0, "the finger drew nothing");
    assert!(origin(&app) != before, "the finger panned the view");
    crate::touch::set_source(crate::touch::Source::Pen);
    drag(&mut app, c, c + vec2(60.0, 40.0));
    assert_eq!(count(&app), 1, "the pen draws");
    crate::touch::set_pen_seen(false);
    crate::touch::set_source(crate::touch::Source::Mouse);
}

/// Switch Selection and Drawing Modes from the command: Pencil when no drawing tool was chosen yet,
/// a view tool is left for Selection without becoming the drawing tool, and every selection tool
/// switches to drawing mode.
#[test]
fn switch_selection_and_drawing_modes_command() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    assert!(app.ui.touch_drawing_tool.is_none());
    app.select_tool("selection");
    let r = app.run(crate::touch::TOGGLE_MODE, json!({})).unwrap();
    assert_eq!((r, app.session.tool_id()), (json!({"tool": "pencil"}), "pencil"), "no drawing tool chosen yet: Pencil");
    app.select_tool("hand");
    assert_eq!(app.run(crate::touch::TOGGLE_MODE, json!({})).unwrap(), json!({"tool": "selection"}));
    assert_eq!(app.run(crate::touch::TOGGLE_MODE, json!({})).unwrap(), json!({"tool": "pencil"}), "the Hand tool is no drawing tool");
    app.select_tool("pen");
    for id in ["directSelection", "groupSelection", "magicWand", "lasso"] {
        app.select_tool(id);
        assert_eq!(app.run(crate::touch::TOGGLE_MODE, json!({})).unwrap(), json!({"tool": "pen"}), "{id}");
    }
}

/// Select Last Selection Tool: the Selection tool before any selection tool is chosen, then the
/// one of Selection, Direct Selection and Group Selection chosen last; Magic Wand and Lasso don't
/// count. Running it again keeps that tool, and it changes no art, selection or undo history.
#[test]
fn select_last_selection_tool_command() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let last = |app: &mut VectorcraftApp| app.run("tool.selectLastSelection", json!({})).unwrap();
    let state = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        (st.doc.clone(), st.selection.clone(), st.history.undo.len())
    };
    app.select_tool("pen");
    let before = state(&app);
    assert_eq!((last(&mut app), app.session.tool_id()), (json!({"tool": "selection"}), "selection"), "none chosen yet: Selection");
    for id in ["directSelection", "groupSelection", "selection"] {
        app.select_tool(id);
        app.select_tool("pen");
        assert_eq!(last(&mut app), json!({"tool": id}), "{id}");
    }
    app.select_tool("directSelection");
    for other in ["magicWand", "lasso", "rectangle", "hand"] {
        app.select_tool(other);
    }
    assert_eq!(last(&mut app), json!({"tool": "directSelection"}), "Magic Wand and Lasso are not remembered");
    assert_eq!(last(&mut app), json!({"tool": "directSelection"}), "again: the same tool, no cycling");
    assert!(state(&app) == before, "no art, selection or undo step changes");
}

/// A Switch Selection and Drawing Modes whose tool choice fails returns the error, in both
/// directions: the tool stays, the drawing tool to return to stays, and no undo step is added.
#[test]
fn a_failed_mode_switch_returns_its_error() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 30, "height": 20})).unwrap();
    let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    let steps = undo(&app);
    app.select_tool("rectangle");
    app.ui.touch_drawing_tool = Some("pen".into());
    app.fail_next_tool_choice = Some("no switch".into());
    let r = app.run(crate::touch::TOGGLE_MODE, json!({}));
    assert!(r.as_ref().is_err_and(|e| e.contains("no switch")), "{r:?}");
    assert_eq!((app.session.tool_id(), app.ui.touch_drawing_tool.as_deref(), undo(&app)), ("rectangle", Some("pen"), steps), "drawing mode");
    app.select_tool("selection");
    app.fail_next_tool_choice = Some("no switch".into());
    let r = app.run(crate::touch::TOGGLE_MODE, json!({}));
    assert!(r.as_ref().is_err_and(|e| e.contains("no switch")), "{r:?}");
    assert_eq!((app.session.tool_id(), app.ui.touch_drawing_tool.as_deref(), undo(&app)), ("selection", Some("pen"), steps), "selection mode");
    // The next switch works again.
    assert_eq!(app.run(crate::touch::TOGGLE_MODE, json!({})).unwrap(), json!({"tool": "pen"}));
    assert_eq!(undo(&app), steps);
}

/// The commands that choose a tool (Select Tool, Crop Image, and the Path Precision Tools' Corner
/// Tool and Shared Edge Merge Tool) return the error when the choice fails, and the tool and the
/// toolbar group's tool stay as they were.
#[test]
fn tool_commands_return_a_failed_choice() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let img = image::RgbaImage::from_pixel(20, 10, image::Rgba([200, 120, 40, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    app.run("file.place", json!({"name": "texture.png", "dataBase64": data, "link": false, "at": [150, 150]})).unwrap();
    app.select_tool("selection");
    let group = vectorcraft_tools::catalog::group_of("ellipse").unwrap();
    let commands = [
        ("tool.select", json!({"tool": "ellipse"})),
        ("ui.cropImage", json!({})),
        (crate::addons::pathtools::corner::COMMAND, json!({})),
        (crate::addons::pathtools::shared_edge::COMMAND, json!({})),
    ];
    for (command, params) in commands {
        let slot = app.ui.group_tool[group].clone();
        app.fail_next_tool_choice = Some("no switch".into());
        let r = app.run(command, params.clone());
        assert!(r.as_ref().is_err_and(|e| e.contains("no switch")), "{command}: {r:?}");
        assert_eq!(app.session.tool_id(), "selection", "{command}");
        assert_eq!(app.ui.group_tool[group], slot, "{command}");
        app.run(command, params).unwrap();
        assert_ne!(app.session.tool_id(), "selection", "{command} works again");
        app.select_tool("selection");
    }
}

/// The Artboard tool's label holds the artboard's name (never translated); the tools' own
/// labels are interface text.
#[test]
fn artboard_labels_name_the_artboard() {
    assert!(names_an_artboard("01 - Layers") && names_an_artboard("12 - Artboard 12 - copy"));
    assert!(!names_an_artboard("anchor") && !names_an_artboard("1 - x") && !names_an_artboard("Off the mesh - move closer"));
}

/// The box-drawing shape tools show the bounding box of the shape just drawn (#5477). A drag of
/// its bottom-right handle with the tool still active scales that shape, as with the Selection
/// tool, rather than drawing a second shape from the handle.
#[test]
fn a_shape_tool_scales_the_shape_just_drawn_by_its_bounding_box_handle() {
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for tool in ["rectangle", "roundedRectangle", "ellipse", "polygon", "star"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.select_tool(tool);
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx, vec![]);
        let drag = |app: &mut VectorcraftApp, a: Pos2, b: Pos2| {
            for ev in [
                egui::Event::PointerMoved(a),
                button(a, true),
                egui::Event::PointerMoved(a.lerp(b, 0.5)),
                egui::Event::PointerMoved(b),
                button(b, false),
            ] {
                frame(app, &ctx, vec![ev]);
            }
            frame(app, &ctx, vec![]);
        };
        let xf = app.canvas_view().unwrap();
        drag(&mut app, xf.to_screen(Point::new(150.0, 120.0)), xf.to_screen(Point::new(210.0, 170.0)));
        let objects = |app: &VectorcraftApp| {
            let mut n = 0;
            app.session.active().unwrap().doc.walk(|node| n += usize::from(node.path_data().is_some()));
            n
        };
        assert_eq!(objects(&app), 1, "{tool}: the drag draws one shape");
        let before = app.selection_box().unwrap_or_else(|| panic!("{tool}: the shape just drawn shows its box"));
        // The bottom-right handle, dragged 40 pt along the box's width and 30 pt along its height
        // (a polygon's or star's box turns with the shape).
        let (corner, to) = (Point::new(before.rect.x1, before.rect.y1), Point::new(before.rect.x1 + 40.0, before.rect.y1 + 30.0));
        drag(&mut app, xf.to_screen(before.to_doc() * corner), xf.to_screen(before.to_doc() * to));
        let after = app.selection_box().unwrap_or_else(|| panic!("{tool}: the shape stays selected"));
        let label = app.session.active().and_then(|st| st.history.undo.last().map(|e| e.label.clone()));
        assert_eq!(objects(&app), 1, "{tool}: no second shape, last step {label:?}");
        assert!(
            (after.rect.width() - before.rect.width() - 40.0).abs() < 0.5 && (after.rect.height() - before.rect.height() - 30.0).abs() < 0.5,
            "{tool}: {:?} -> {:?}, last step {label:?}",
            before.rect,
            after.rect
        );
        assert_eq!(label.as_deref(), Some("Scale"), "{tool}");
    }
}

#[test]
fn cmd_with_another_tool_drags_with_the_selection_tool_used_last() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 50})).unwrap()["id"].as_u64().unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let xf = app.canvas_view().unwrap();
    let cmd_frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let events = std::iter::once(egui::Event::ModifiersChanged(egui::Modifiers::COMMAND)).chain(events).collect();
        let raw = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    let button =
        |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::COMMAND };
    // A Cmd drag from `a` to `b` with the Pen tool; → the tool that did it.
    let cmd_drag = |app: &mut VectorcraftApp, a: Pos2, b: Pos2| {
        app.select_tool("pen");
        cmd_frame(app, vec![egui::Event::PointerMoved(a), button(a, true)]);
        let used = app.session.tool_id().to_string();
        cmd_frame(app, vec![egui::Event::PointerMoved(b)]);
        cmd_frame(app, vec![button(b, false)]);
        assert_eq!(app.session.tool_id(), "pen", "back to the Pen tool");
        used
    };
    let bounds = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    // Direct Selection used last: the drag moves the rectangle's bottom-right corner only (once
    // the rectangle isn't selected whole, which would move all its anchors).
    app.select_tool("directSelection");
    app.session.execute("select.none", &json!({})).unwrap();
    let corner = xf.to_screen(Point::new(200.0, 150.0));
    assert_eq!(cmd_drag(&mut app, corner, corner + vec2(20.0, 20.0)), "directSelection");
    let b = bounds(&app);
    assert_eq!((b.x0, b.y0), (100.0, 100.0));
    assert!((b.x1 - xf.to_doc(corner + vec2(20.0, 20.0)).x).abs() < 1e-6, "{b:?}");
    // Group Selection, then Selection: each is the one used.
    app.select_tool("groupSelection");
    let inside = xf.to_screen(Point::new(130.0, 120.0));
    assert_eq!(cmd_drag(&mut app, inside, inside), "groupSelection");
    app.select_tool("selection");
    assert_eq!(cmd_drag(&mut app, inside, inside), "selection");
}

/// #6369: only the selection box remains over the temporary halftone, and the original
/// selection's outlines come back when Preview is turned off or the dialog is cancelled.
#[test]
fn halftone_preview_hides_dot_outlines_and_keeps_selection_bounds() {
    use crate::dialogs::{halftone, host};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.bounding_box = true;
    let original = app.run("shape.ellipse", json!({"x": 50, "y": 50, "width": 120, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let original = vectorcraft_doc::NodeId(original);
    app.run("paint.setFill", json!({"color": "#000000"})).unwrap();
    let ctx = crate::testing::ui_context();
    app.ui_ctx = Some(ctx.clone());
    // Whether the selection shows its objects' outlines; the box around it shows throughout.
    let outlined = |app: &mut VectorcraftApp| {
        let tool = app.session.tool_id();
        assert!(super::widgets::bounding_box(app, tool).is_some(), "the selection box is still drawn");
        let app = &*app;
        let st = app.session.active().unwrap();
        !super::selection::outlines(app, st).is_empty()
    };
    let set_preview = |app: &mut VectorcraftApp, on: bool| {
        let id = app.effect_dialogs.halftone.as_ref().map(|o| o.id).expect("Vector Halftone is open");
        app.dialogs().unwrap().with(|h| h.set_json_field(&ctx, id, "preview", json!(on))).unwrap();
        host::pump(app, &ctx);
    };
    let top = |app: &VectorcraftApp| app.dialogs().unwrap().with(|h| h.open_dialogs().last().map(|d| d.id)).expect("a dialog is open");
    assert!(outlined(&mut app), "the selected ellipse shows its outline");

    app.run("ui.menuDialog", json!({"command": halftone::CMD})).unwrap();
    host::pump(&mut app, &ctx);
    assert!(halftone::previewing(&app));
    assert_ne!(app.session.active().unwrap().selection.objects, vec![original], "the halftone is previewed and selected");
    assert!(!outlined(&mut app), "no outlines over the halftone dots");

    set_preview(&mut app, false);
    assert_eq!(app.session.active().unwrap().selection.objects, vec![original]);
    assert!(outlined(&mut app), "turning Preview off brings the outline back");

    set_preview(&mut app, true);
    assert!(!outlined(&mut app), "turning Preview back on hides the dot outlines again");
    let id = top(&app);
    assert!(app.dialogs().unwrap().with(|h| h.cancel_by_id(&ctx, id)));
    host::pump(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().selection.objects, vec![original]);
    assert!(outlined(&mut app), "Cancel brings the outline back");

    app.run("ui.menuDialog", json!({"command": "object.move"})).unwrap();
    let id = top(&app);
    app.dialogs().unwrap().with(|h| h.set_json_field(&ctx, id, "preview", json!(true))).unwrap();
    host::pump(&mut app, &ctx);
    assert!(app.session.in_interaction());
    assert!(outlined(&mut app), "another dialog's preview keeps the outlines");
    assert!(app.dialogs().unwrap().with(|h| h.cancel_by_id(&ctx, id)));
    host::pump(&mut app, &ctx);

    app.run("ui.menuDialog", json!({"command": halftone::CMD})).unwrap();
    host::pump(&mut app, &ctx);
    assert!(!outlined(&mut app));
    // Vector Halftone holds its OK: the pump answers it.
    let id = top(&app);
    crate::tests::picks::answer_dialog(&mut app, id, &[], true);
    assert!(!app.any_dialog_open() && !app.session.in_interaction());
    assert!(outlined(&mut app), "OK brings ordinary outlines back on the applied dots");
}

// One headless canvas frame → the shapes drawn, `Shape::Vec`s flattened.
fn shapes(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<Shape> {
    let (shapes, mut delta) = frame_output(app, ctx);
    delta.clear();
    shapes
}

/// The shapes of a frame's output, `Shape::Vec`s flattened: the canvas's picture slot holds the art's.
fn flat_shapes(clipped: Vec<egui::epaint::ClippedShape>) -> Vec<Shape> {
    fn flat(s: Shape, out: &mut Vec<Shape>) {
        match s {
            Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
            s => out.push(s),
        }
    }
    let mut v = vec![];
    clipped.into_iter().for_each(|c| flat(c.shape, &mut v));
    v
}

/// One headless canvas frame → the shapes drawn ([`flat_shapes`]) and the textures uploaded by it
/// (the art's raster among them, when it was re-rendered).
fn frame_output(app: &mut VectorcraftApp, ctx: &egui::Context) -> (Vec<Shape>, egui::TexturesDelta) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let out = ctx.run_ui(raw, |ui| show(app, ui));
    (flat_shapes(out.shapes), out.textures_delta)
}

/// An edit of one object renders the canvas again only where it changed, into its texture; a
/// change of the view renders all of it.
#[test]
fn an_edit_renders_only_its_part_of_the_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":400,"height":300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.canvas.worker_started = true; // every frame on this thread, however long it takes
    let id = app.run("shape.rectangle", json!({"x":50,"y":50,"width":40,"height":30})).unwrap()["id"].as_u64().unwrap();
    app.run("shape.rectangle", json!({"x":250,"y":200,"width":40,"height":30})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let ctx = crate::testing::ui_context();
    // Where the canvas texture changed in a frame, and how much of it.
    let canvas = |app: &VectorcraftApp, mut delta: egui::TexturesDelta| {
        let tex = app.canvas.texture.as_ref().map(|t| t.id());
        let set = delta.set.iter().find(|(t, _)| tex == Some(**t)).and_then(|(_, d)| d.last()).map(|d| (d.pos, d.image.size()));
        delta.clear();
        set
    };
    let (_, delta) = frame_output(&mut app, &ctx);
    assert!(canvas(&app, delta).is_some_and(|(pos, _)| pos.is_none()), "the first frame renders all of it");
    app.run("paint.setFill", json!({"color": "#ff0000", "ids": [id]})).unwrap();
    let (_, delta) = frame_output(&mut app, &ctx);
    let (pos, size) = canvas(&app, delta).expect("the edit shows");
    let whole = app.canvas.key.as_ref().map(|k| (k.w as usize, k.h as usize)).unwrap();
    assert!(pos.is_some() && size[0] * size[1] * 10 < whole.0 * whole.1, "a small part of it: {pos:?} {size:?} of {whole:?}");
    // An edit with other render options than the frame shown's renders all of it.
    app.session.prefs.pattern_tile_edge_color = "#ff00ff".into();
    app.run("paint.setFill", json!({"color": "#00ff00", "ids": [id]})).unwrap();
    let (_, delta) = frame_output(&mut app, &ctx);
    assert!(canvas(&app, delta).is_some_and(|(pos, _)| pos.is_none()), "other options render all of it");
    app.run("view.setZoom", json!({"zoom": 2})).unwrap();
    let (_, delta) = frame_output(&mut app, &ctx);
    assert!(canvas(&app, delta).is_some_and(|(pos, _)| pos.is_none()), "a new view renders all of it");
}

/// A canvas more device pixels across than the largest texture side the display supports renders
/// its art at fewer pixels a point, within that size, and draws it over the whole canvas. Pixel
/// Preview's region of document pixels stays within it too.
#[test]
fn the_art_renders_within_the_largest_texture_side() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":400,"height":300})).unwrap();
    app.canvas.worker_started = true; // every frame on this thread, however long it takes
    let ctx = crate::testing::ui_context();
    ctx.set_pixels_per_point(2.0);
    // The first frame sets the pixels a point, the second draws at them.
    for _ in 0..2 {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            max_texture_side: Some(1024),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| show(&mut app, ui));
        out.textures_delta.clear();
    }
    let rect = app.canvas_view().unwrap().rect;
    assert!(rect.width() * ctx.pixels_per_point() > 1024.0, "the canvas is {rect:?} at {} pixels a point", ctx.pixels_per_point());
    let size = app.canvas.texture.as_ref().map(|t| t.size()).unwrap();
    assert_eq!(size[0].max(size[1]), 1024, "the frame is {size:?} pixels");
    let k = app.canvas.key.clone().unwrap();
    assert!(
        (k.w as f32 / k.ppp - rect.width()).abs() < 1.0 && (k.h as f32 / k.ppp - rect.height()).abs() < 1.0,
        "the frame covers the canvas {rect:?}: {} × {} pixels at {} a point",
        k.w,
        k.h,
        k.ppp
    );

    let view = CanvasView::new(egui::Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 500.0)), Point::new(0.0, 0.0), 0.5);
    let [x0, _, x1, _] = pixel_region(&view, 4096).expect("a region 2,000 document pixels wide");
    assert_eq!(x1 - x0, 2000);
    assert_eq!(pixel_region(&view, 1024), None, "more than the display's largest texture");
}

/// `rgb` as the display transform `t` shows it.
fn convert8(t: &craft::color_ui::display::DisplayTransform, rgb: [u8; 3]) -> [u8; 3] {
    let mut px = [rgb[0], rgb[1], rgb[2], 255];
    t.apply_rgba8(&mut px);
    [px[0], px[1], px[2]]
}

/// Run a test that compares two renders with the color settings that are active when it starts,
/// and the default proof view. A test on another thread can change the process's active settings;
/// this thread keeps its own. The proof view is the whole process's, so it is held still.
fn keeping_color_settings(test: fn()) {
    let _view = crate::testing::DefaultProofView::new();
    vectorcraft_color::cms::with_active(vectorcraft_color::cms::active(), test);
}

/// A placed document's bitmap made in the background renders the canvas again only where the
/// objects showing its file are: both copies of the file are in the part rendered again.
#[test]
fn a_ready_placed_document_bitmap_renders_only_its_part_of_the_canvas() {
    keeping_color_settings(a_ready_placed_bitmap);
}

fn a_ready_placed_bitmap() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":400,"height":300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.canvas.worker_started = true; // every frame on this thread, however long it takes
    // A document of one 60 × 30 artboard with a rectangle, placed linked twice.
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 60, "height": 30})).unwrap();
    src.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 60, "height": 30})).unwrap();
    let dir = std::env::temp_dir().join(format!("vectorcraft-placed-redraw-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("card.vectorcraft");
    std::fs::write(&path, vectorcraft_format::save_file(&src.active().unwrap().doc)).unwrap();
    let path = path.to_string_lossy().into_owned();
    let ids: Vec<u64> = [[60, 50], [140, 50]]
        .iter()
        .map(|at| {
            let r = app.session.execute("file.place", &json!({"path": path, "at": at})).unwrap();
            assert_eq!(r["linked"], true, "{r}");
            r["ids"][0].as_u64().unwrap()
        })
        .collect();
    app.run("shape.rectangle", json!({"x":250,"y":200,"width":40,"height":30})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let ctx = crate::testing::ui_context();
    // Where the canvas texture changed in a frame, and how much of it.
    let canvas = |app: &VectorcraftApp, mut delta: egui::TexturesDelta| {
        let tex = app.canvas.texture.as_ref().map(|t| t.id());
        let set = delta.set.iter().find(|(t, _)| tex == Some(**t)).and_then(|(_, d)| d.last()).map(|d| (d.pos, d.image.size()));
        delta.clear();
        set
    };
    let (_, delta) = frame_output(&mut app, &ctx);
    assert!(canvas(&app, delta).is_some_and(|(pos, _)| pos.is_none()), "the first frame renders all of it");
    while vectorcraft_render::placed_document::busy() {
        std::thread::yield_now();
    }
    let (_, delta) = frame_output(&mut app, &ctx);
    let (pos, size) = canvas(&app, delta).expect("the bitmap shows");
    let [px, py] = pos.expect("a part of the canvas");
    let key = app.canvas.key.clone().unwrap();
    let view = Affine::translate((f64::from(key.w) / 2.0, f64::from(key.h) / 2.0))
        * Affine::rotate(key.rot.to_radians())
        * Affine::scale(key.zoom * f64::from(key.ppp))
        * Affine::translate((-key.cx, -key.cy));
    let doc = &app.session.active().unwrap().doc;
    for id in ids {
        let b = doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
        let [x0, y0, x1, y1] = vectorcraft_render::view_pixels(b, view, key.w, key.h).unwrap();
        let inside = x0 as usize >= px && y0 as usize >= py && x1 as usize <= px + size[0] && y1 as usize <= py + size[1];
        assert!(inside, "object {id} at {:?} in the part at {:?} of size {size:?}", [x0, y0, x1, y1], [px, py]);
    }
    assert!(size[0] * size[1] * 2 <= key.w as usize * key.h as usize, "a part of it: {size:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// While an opacity mask is viewed, an edit renders all of the canvas: the mask view shows the
/// mask's coverage wherever its art reaches, past its object's bounds.
#[test]
fn an_edit_while_a_mask_is_viewed_renders_all_of_the_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":400,"height":300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.canvas.worker_started = true; // every frame on this thread, however long it takes
    let rect =
        |app: &mut VectorcraftApp, x: f64| app.run("shape.rectangle", json!({"x": x, "y": 50, "width": 60, "height": 40})).unwrap()["id"].clone();
    let (obj, mask) = (rect(&mut app, 50.0), rect(&mut app, 70.0));
    app.run("select.set", json!({"ids": [obj, mask]})).unwrap();
    app.run("transparency.makeOpacityMask", json!({})).unwrap();
    app.run("transparency.viewOpacityMask", json!({})).unwrap();
    let ctx = crate::testing::ui_context();
    let (_, mut delta) = frame_output(&mut app, &ctx);
    delta.clear();
    app.run("paint.setFill", json!({"color": "#ff0000", "ids": [obj]})).unwrap();
    let (_, mut delta) = frame_output(&mut app, &ctx);
    let tex = app.canvas.texture.as_ref().map(|t| t.id());
    let pos = delta.set.iter().find(|(t, _)| tex == Some(**t)).and_then(|(_, d)| d.last()).map(|d| d.pos);
    delta.clear();
    assert_eq!(pos, Some(None), "all of it");
}

/// View › Pixel Preview: magnified, the art shows as the document pixels it rasterizes to (one
/// per point, hard-edged), not as smooth vectors; zoomed out or out of Pixel Preview the canvas
/// renders for the screen again.
#[test]
fn pixel_preview_shows_the_document_pixels() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // Its 1 pt stroke covers a quarter of document pixel 99 and most of pixel 100.
    app.session.execute("shape.rectangle", &json!({"x": 100.25, "y": 50, "width": 200, "height": 200})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let ctx = crate::testing::ui_context();
    // The art's raster uploaded by one frame: (width, height, hard-edged, pixels).
    let upload = |app: &mut VectorcraftApp| {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let up = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            (img.width(), img.height(), d.options.magnification == egui::TextureFilter::Nearest, img.pixels.clone())
        });
        delta.clear();
        up.expect("the art re-rendered")
    };
    let screen = upload(&mut app); // the first frame fits the view to the artboard
    assert!(screen.0 > 600 && !screen.2, "out of Pixel Preview: the screen's pixels, smoothed");
    app.ui.view.pixel_preview = true;
    let view = app.view_mut().unwrap();
    (view.zoom, view.center) = (8.0, Point::new(100.0, 150.0));
    let (w, h, hard, px) = upload(&mut app);
    assert!(hard, "hard-edged pixels");
    assert!(w <= screen.0 / 8 + 2 && h <= screen.1 / 8 + 2, "one pixel per point: {w} × {h}");
    let [x0, y0, x1, y1] = app.canvas.key.as_ref().and_then(|k| k.pixel).expect("the pixels rendered");
    assert_eq!(((x1 - x0) as usize, (y1 - y0) as usize), (w, h));
    // Each document pixel is one texel, anti-aliased by how much of it the art covers.
    let alpha = |x: i64| px[((150 - y0) * w as i64 + x - x0) as usize].a();
    assert!((40..=90).contains(&alpha(99)), "a quarter of pixel 99: {}", alpha(99));
    assert!(alpha(100) > 200, "most of pixel 100: {}", alpha(100));
    assert_eq!((alpha(98), alpha(101)), (0, 255));
    // The pixels are placed over the document rect they cover.
    let s = shapes(&mut app, &ctx);
    let art = app.canvas.texture.as_ref().unwrap().id();
    let Some(Shape::Mesh(mesh)) = s.iter().find(|s| matches!(s, Shape::Mesh(m) if m.texture_id == art)) else { panic!("the art") };
    let xf = app.canvas_view().unwrap();
    let at = xf.to_screen(Point::new(x0 as f64, y0 as f64));
    assert!((mesh.vertices[0].pos - at).length() < 0.01, "{:?} at {at:?}", mesh.vertices[0].pos);
    // Zoomed out, a document pixel is no bigger than a screen pixel: rendered for the screen.
    app.view_mut().unwrap().zoom = 0.5;
    let (w, _, hard, _) = upload(&mut app);
    assert!(w > 600 && !hard, "zoomed out: the screen's pixels");
}

/// Preferences › Performance › Use Monitor Profile for Canvas: with a display profile read, the
/// art is drawn converted into it, and drawn again when the profile or the preference changes.
#[test]
fn the_canvas_is_drawn_through_the_display_profile_while_the_preference_is_on() {
    use crate::display_profile::{Found, Reading};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    app.session.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    assert!(app.session.prefs.monitor_profile_display, "on by default");
    let ctx = crate::testing::ui_context();
    // How many pixels of the art's raster one frame uploads are `c`, None when it was not re-rendered.
    let count = |app: &mut VectorcraftApp, c: egui::Color32| -> Option<usize> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let n = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            img.pixels.iter().filter(|p| **p == c).count()
        });
        delta.clear();
        n
    };
    let red = egui::Color32::RED;
    let srgb = count(&mut app, red).expect("the first frame");
    assert!(srgb > 1000, "the rectangle in sRGB: {srgb}");
    let p3 = crate::display_profile::tests::p3();
    let t = crate::display_profile::tests::transform();
    let [r, g, b] = convert8(&t, [255, 0, 0]);
    let shown = egui::Color32::from_rgb(r, g, b);
    assert_ne!(shown, red, "sRGB red is less saturated on a wide-gamut display");
    let reading = |found| Reading { display: Some("DP-1".into()), found };
    assert!(app.display_profile.report(reading(Found::Profile { icc: p3.clone(), from: "test".into() })));
    assert_eq!(count(&mut app, shown), Some(srgb), "drawn again, converted");
    // The same profile again: nothing is drawn again.
    assert!(!app.display_profile.report(reading(Found::Profile { icc: p3, from: "test".into() })));
    assert_eq!(count(&mut app, shown), None);
    // Off: drawn again in sRGB.
    app.session.prefs.monitor_profile_display = false;
    assert_eq!(count(&mut app, red), Some(srgb));
    // On again, then the system takes over: sRGB.
    app.session.prefs.monitor_profile_display = true;
    assert_eq!(count(&mut app, shown), Some(srgb));
    app.display_profile.report(reading(Found::System("test".into())));
    assert_eq!(count(&mut app, red), Some(srgb));
}

/// With a display profile, in Overprint Preview translucent art around the artboards is blended
/// with the white canvas before the conversion, as art over the paper is. With the preview off it
/// converts alone, over the pasteboard.
#[test]
fn translucent_art_around_the_artboards_is_blended_with_overprint_previews_white_canvas() {
    keeping_color_settings(translucent_art_around_the_artboards);
}

fn translucent_art_around_the_artboards() {
    use crate::display_profile::{Found, Reading};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.canvas.worker_started = true; // every frame on this thread
    // Left of the artboard.
    let outside = app.run("shape.rectangle", json!({"x": -60, "y": 100, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [outside], "color": "#00ff00"})).unwrap();
    app.run("transparency.set", json!({"ids": [outside], "opacity": 50})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let ctx = crate::testing::ui_context();
    // The whole art raster one frame uploads: its width and its pixels.
    let upload = |app: &mut VectorcraftApp| -> Option<(usize, Vec<egui::Color32>)> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref()?.id();
        let up = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).filter(|d| d.pos.is_none()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            (img.width(), img.pixels.clone())
        });
        delta.clear();
        up
    };
    upload(&mut app).expect("the first frame");
    let v = app.view_mut().unwrap();
    (v.zoom, v.center) = (1.0, Point::new(120.0, 150.0));
    upload(&mut app).expect("the view at 100%");
    let p3 = crate::display_profile::tests::p3();
    let t = crate::display_profile::tests::transform();
    app.display_profile.report(Reading { display: Some("DP-1".into()), found: Found::Profile { icc: p3, from: "test".into() } });
    upload(&mut app).expect("drawn again, converted");
    let white = convert8(&t, [255; 3]);
    // What the canvas shows of pixel `c` over the converted paper `under`, as egui blends it.
    let over = |c: egui::Color32, under: [u8; 3]| {
        let k = 255 - u32::from(c.a());
        [0, 1, 2].map(|i| u32::from(c.to_array()[i]) + (u32::from(under[i]) * k + 127) / 255)
    };
    let plain = |s: egui::Color32| {
        let mut px = s.to_array();
        t.apply_premultiplied_rgba8(&mut px);
        egui::Color32::from_rgba_premultiplied(px[0], px[1], px[2], px[3])
    };
    // The pixel at the translucent rectangle's center, in sRGB (the display profile's preference
    // off) and as the canvas shows it (on).
    let pixel = |app: &mut VectorcraftApp| {
        app.session.prefs.monitor_profile_display = false;
        let (width, srgb) = upload(app).expect("drawn in sRGB");
        let key = app.canvas.key.clone().unwrap();
        let (_, _, view) =
            frame_view(None, (key.w, key.h), &CanvasView::new(egui::Rect::NOTHING, Point::new(key.cx, key.cy), key.zoom).rotation(key.rot), key.ppp);
        let at = view * Point::new(-40.0, 120.0);
        let at = at.y as usize * width + at.x as usize;
        app.session.prefs.monitor_profile_display = true;
        let (_, shown) = upload(app).expect("drawn again, converted");
        (srgb[at], shown[at], app.canvas.key.clone().unwrap())
    };
    let (s, c, key) = pixel(&mut app);
    assert!((50..=205).contains(&s.a()), "the 50% fill: {s:?}");
    assert!(key.backdrop.as_ref().is_some_and(|b| b.everywhere.is_none()), "Preview: the pasteboard is no paper");
    assert_eq!(c, plain(s), "over the pasteboard the art converts alone");
    app.run("view.overprintPreview", json!({"on": true})).unwrap();
    let (s, c, key) = pixel(&mut app);
    assert!((50..=205).contains(&s.a()), "the 50% fill: {s:?}");
    assert_eq!(key.backdrop.as_ref().and_then(|b| b.everywhere), Some([255; 3]), "the white canvas is paper");
    let k = 255 - u32::from(s.a());
    let mix = [0, 1, 2].map(|j| (u32::from(s.to_array()[j]) + (255 * k + 127) / 255).min(255) as u8);
    let (want, got) = (convert8(&t, mix), over(c, white));
    assert!((0..3).all(|j| got[j].abs_diff(u32::from(want[j])) <= 1), "{got:?} shown, {want:?} wanted");
    // The test tells the two apart: the fill converted alone shows another color over the white canvas.
    let alone = over(plain(s), white);
    assert!((0..3).any(|j| alone[j].abs_diff(got[j]) > 2), "{alone:?} and {got:?}");
}

/// With a display profile, translucent art and the anti-aliased edges of art over the paper are
/// blended with the paper in sRGB before the conversion: drawn over the converted paper, each such
/// pixel shows the converted blend. Opaque art, and art over the pasteboard, convert as before. The
/// part an edit renders again and the render worker's frame do the same, a change of the
/// transparency grid renders the art again, and Outline converts the art alone.
#[test]
fn translucent_art_over_the_paper_is_blended_before_the_display_profile() {
    keeping_color_settings(translucent_art_over_the_paper);
}

fn translucent_art_over_the_paper() {
    use crate::display_profile::{Found, Reading};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.canvas.worker_started = true; // every frame on this thread
    let half = app.run("shape.rectangle", json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [half], "color": "#ff0000"})).unwrap();
    app.run("transparency.set", json!({"ids": [half], "opacity": 50})).unwrap();
    let disc = app.run("shape.ellipse", json!({"x": 20, "y": 20, "width": 60, "height": 60})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [disc], "color": "#2040c0"})).unwrap();
    // Left of the artboard, over the pasteboard.
    let outside = app.run("shape.rectangle", json!({"x": -60, "y": 100, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [outside], "color": "#00ff00"})).unwrap();
    app.run("transparency.set", json!({"ids": [outside], "opacity": 50})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let ctx = crate::testing::ui_context();
    // The art's raster one frame uploads: where it goes ([x, y] for a part), its width and its pixels.
    let upload = |app: &mut VectorcraftApp| -> Option<(Option<[usize; 2]>, usize, Vec<egui::Color32>)> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref()?.id();
        let up = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            (d.pos, img.width(), img.pixels.clone())
        });
        delta.clear();
        up
    };
    upload(&mut app).expect("the first frame");
    let v = app.view_mut().unwrap();
    (v.zoom, v.center) = (1.0, Point::new(120.0, 150.0));
    let (_, width, srgb) = upload(&mut app).expect("the view at 100%");
    let key = app.canvas.key.clone().unwrap();
    assert!(key.backdrop.is_none(), "no display profile: nothing to blend");
    let p3 = crate::display_profile::tests::p3();
    let t = crate::display_profile::tests::transform();
    app.display_profile.report(Reading { display: Some("DP-1".into()), found: Found::Profile { icc: p3, from: "test".into() } });
    let (_, _, shown) = upload(&mut app).expect("drawn again, converted");
    let key = app.canvas.key.clone().unwrap();
    assert!(key.backdrop.is_some());
    let white = convert8(&t, [255; 3]);
    // What the canvas shows of pixel `c` over the converted paper `under`, as egui blends it.
    let over = |c: egui::Color32, under: [u8; 3]| {
        let k = 255 - u32::from(c.a());
        [0, 1, 2].map(|i| u32::from(c.to_array()[i]) + (u32::from(under[i]) * k + 127) / 255)
    };
    let plain = |s: egui::Color32| {
        let mut px = s.to_array();
        t.apply_premultiplied_rgba8(&mut px);
        egui::Color32::from_rgba_premultiplied(px[0], px[1], px[2], px[3])
    };
    let (view_w, view_h) = (key.w, key.h);
    let (_, _, view) =
        frame_view(None, (view_w, view_h), &CanvasView::new(egui::Rect::NOTHING, Point::new(key.cx, key.cy), key.zoom).rotation(key.rot), key.ppp);
    let to_doc = view.inverse();
    let (mut blended, mut beside) = (0, 0);
    for (i, (s, c)) in srgb.iter().zip(&shown).enumerate() {
        let p = to_doc * Point::new((i % width) as f64 + 0.5, (i / width) as f64 + 0.5);
        if s.a() == 0 || s.a() == 255 {
            assert_eq!(*c, plain(*s), "opaque and empty pixels convert as before, at {p:?}");
        } else if (1.0..399.0).contains(&p.x) && (1.0..299.0).contains(&p.y) {
            let k = 255 - u32::from(s.a());
            let mix = [0, 1, 2].map(|j| (u32::from(s.to_array()[j]) + (255 * k + 127) / 255).min(255) as u8);
            let want = convert8(&t, mix);
            let got = over(*c, white);
            assert!((0..3).all(|j| got[j].abs_diff(u32::from(want[j])) <= 1), "at {p:?}: {got:?} shown, {want:?} wanted");
            blended += 1;
        } else if p.x < -1.0 {
            assert_eq!(*c, plain(*s), "over the pasteboard the art converts alone, at {p:?}");
            beside += 1;
        }
    }
    assert!(blended > 1000 && beside > 500, "{blended} pixels over the paper, {beside} over the pasteboard");
    // The test tells the two apart: the fill converted alone, over the converted paper, shows another color.
    let at = view * Point::new(200.0, 150.0);
    let at = at.y as usize * width + at.x as usize;
    let (s, c) = (srgb[at], shown[at]);
    assert!((50..=205).contains(&s.a()), "the 50% fill at the center: {s:?}");
    let alone = over(plain(s), white);
    assert!((0..3).any(|j| alone[j].abs_diff(over(c, white)[j]) > 2), "{alone:?} and {:?}", over(c, white));
    // The render worker's frame of the same key is the same.
    let opts = app.canvas.shown.as_ref().unwrap().1.clone();
    let doc = app.session.active().unwrap().doc.clone();
    let display = app.display_profile.transform(true);
    let job = |key: &CacheKey, doc: &std::sync::Arc<vectorcraft_doc::Document>| crate::render_worker::Job {
        key: key.clone(),
        doc: doc.clone(),
        w: view_w,
        h: view_h,
        view,
        opts: opts.clone(),
        scope: vectorcraft_engine::file_access::ScopeHandle::capture(),
        effects: Default::default(),
        may_copy: false,
        may_keep: false,
        display: display.clone(),
    };
    let worker = crate::canvas::render_job(&mut vectorcraft_render::Renderer::new(), &mut None, &job(&key, &doc)).img;
    let near = |a: &[u8], b: egui::Color32| a.iter().zip(b.to_array()).all(|(x, y)| x.abs_diff(y) <= 1);
    assert!(worker.pixels.as_chunks::<4>().0.iter().zip(&shown).all(|(a, b)| near(a, *b)), "the worker's frame matches");
    // An edit renders its part again, blended too.
    app.run("paint.setFill", json!({"ids": [half], "color": "#0000ff"})).unwrap();
    let (pos, part_w, part) = upload(&mut app).expect("the edit shows");
    let [x0, y0] = pos.expect("only its part");
    let key = app.canvas.key.clone().unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let whole = crate::canvas::render_job(&mut vectorcraft_render::Renderer::new(), &mut None, &job(&key, &doc)).img;
    let whole = whole.pixels.as_chunks::<4>().0;
    for (i, c) in part.iter().enumerate() {
        let at = (y0 + i / part_w) * view_w as usize + x0 + i % part_w;
        assert!(near(&whole[at], *c), "the part matches the whole frame at {at}: {c:?}, {:?}", whole[at]);
    }
    assert!(part.iter().filter(|c| c.a() != 0 && c.a() != 255).count() > 1000, "the blue fill is in the part");
    // The transparency grid replaces the paper under the art: drawn again.
    app.run("view.transparencyGrid", json!({"on": true})).unwrap();
    assert!(upload(&mut app).is_some_and(|(pos, ..)| pos.is_none()), "the grid renders the art again");
    assert!(app.canvas.key.as_ref().and_then(|k| k.backdrop.as_ref()).is_some_and(|b| b.grid.is_some()));
    // Outline: the white canvas is the interface's, so the art converts alone.
    app.run("view.outline", json!({})).unwrap();
    upload(&mut app).expect("Outline renders again");
    assert!(app.canvas.key.as_ref().is_some_and(|k| k.outline && k.backdrop.is_none()));
}

/// View › Hide Template (#2000): a template layer's art leaves the canvas, and Show Template
/// draws it again, dimmed as before; each re-renders at once.
#[test]
fn hide_template_leaves_template_layers_off_the_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    let layer = app.session.active().unwrap().doc.layers[0].id.0;
    app.session.execute("layer.template", &json!({"ids": [layer], "on": true})).unwrap();
    let ctx = crate::testing::ui_context();
    // How many pixels of the art's raster one frame uploads are painted, None when it was not
    // re-rendered.
    let painted = |app: &mut VectorcraftApp| -> Option<usize> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let n = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            img.pixels.iter().filter(|c| c.a() != 0).count()
        });
        delta.clear();
        n
    };
    let shown = painted(&mut app).expect("the first frame renders the art");
    assert!(shown > 1000, "the template layer's art is drawn: {shown} pixels");
    assert_eq!(app.run("view.template", json!({})).unwrap(), json!(false));
    assert_eq!(painted(&mut app), Some(0), "Hide Template re-renders without the template layer");
    assert_eq!(painted(&mut app), None, "nothing changed: no re-render");
    assert_eq!(app.run("view.template", json!({})).unwrap(), json!(true));
    assert_eq!(painted(&mut app), Some(shown), "Show Template draws it again as before");
}

/// General › Anti-aliased Artwork (#394): on (the default), the art's edges are smoothed on
/// screen; off, every pixel is either painted or not, and turning it either way re-renders.
#[test]
fn anti_aliased_artwork_preference_smooths_the_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // Render on this thread: the worker's frame would land later than the test looks.
    app.canvas.worker_started = true;
    // A disc: its edge crosses pixels at every angle.
    app.session.execute("shape.ellipse", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    let ctx = crate::testing::ui_context();
    // The alphas of the art's raster uploaded by one frame, None when it was not re-rendered.
    let alphas = |app: &mut VectorcraftApp| -> Option<Vec<u8>> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let alphas = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            img.pixels.iter().map(|c| c.a()).collect()
        });
        delta.clear();
        alphas
    };
    let partial = |a: &[u8]| a.iter().filter(|&&a| a != 0 && a != 255).count();
    let on = alphas(&mut app).expect("the first frame renders the art");
    assert!(partial(&on) > 50, "anti-aliased: edge pixels partly covered ({} of them)", partial(&on));
    assert!(alphas(&mut app).is_none(), "nothing changed: no re-render");
    app.session.execute("prefs.set", &json!({"key": "antiAliasedArtwork", "value": false})).unwrap();
    let off = alphas(&mut app).expect("the preference change re-renders");
    assert_eq!(partial(&off), 0, "hard edges: every pixel painted or not");
    assert!(off.iter().filter(|&&a| a == 255).count() > 1000, "the disc is still painted");
    app.session.execute("prefs.set", &json!({"key": "antiAliasedArtwork", "value": true})).unwrap();
    assert!(partial(&alphas(&mut app).expect("re-rendered")) > 50, "smooth again");
}

/// Type › Highlight Substituted Fonts: on (the default), type in a missing font has pink
/// behind it on the canvas; off, it does not, and turning it either way re-renders.
#[test]
fn highlight_substituted_fonts_preference_reaches_the_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    app.session.execute("text.create", &json!({"x": 20, "y": 120, "text": "Hamburg", "size": 60, "font": "Missing Family"})).unwrap();
    let ctx = crate::testing::ui_context();
    // How many pink pixels the art's raster uploaded by one frame has, None when it was not
    // re-rendered.
    let pink = |app: &mut VectorcraftApp| -> Option<usize> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let n = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            img.pixels.iter().filter(|c| c.a() != 0 && c.r() > c.g().saturating_add(30) && c.b() > c.g()).count()
        });
        delta.clear();
        n
    };
    let on = pink(&mut app).expect("the first frame renders the art");
    assert!(on > 100, "on by default: the text has pink behind it ({on} pixels)");
    assert!(app.canvas.shown.as_ref().is_some_and(|(_, o)| o.highlight_substituted_fonts));
    app.session.execute("prefs.set", &json!({"key": "highlightSubstitutedFonts", "value": false})).unwrap();
    assert_eq!(pink(&mut app), Some(0), "the preference off re-renders without the highlight");
    assert!(app.canvas.key.as_ref().is_some_and(|k| !k.highlight_substituted_fonts));
    app.session.execute("prefs.set", &json!({"key": "highlightSubstitutedFonts", "value": true})).unwrap();
    assert_eq!(pink(&mut app), Some(on), "on again, as before");
}

/// Images whose pixels can't be decoded draw as crossed boxes on the canvas, and the status
/// bar reports each of them once: two such images give one message, and later frames that
/// draw them again give none.
#[test]
fn undecodable_images_are_marked_and_reported_once() {
    use vectorcraft_doc::{ImageBlob, ImageObject, NodeKind};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    // PNGs of noisy pixels cut off halfway: their headers read, their pixels don't decode.
    let cut = |w: u32| {
        let img = image::RgbaImage::from_fn(w, 41, |x, y| image::Rgba([(x * 37 + y * 101) as u8, (x * y) as u8, (x ^ y) as u8, 255]));
        let mut png = vec![];
        {
            let __dyn: image::DynamicImage = img.into();
            let (__w, __h) = (__dyn.width(), __dyn.height());
            let (__layout, __sample, __raw) = match __dyn {
                image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
                image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
                image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
                image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
                image::DynamicImage::ImageLuma16(b) => (
                    craft_image_codecs::ChannelLayout::Gray,
                    craft_image_codecs::SampleType::U16,
                    b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
                ),
                image::DynamicImage::ImageLumaA16(b) => (
                    craft_image_codecs::ChannelLayout::GrayA,
                    craft_image_codecs::SampleType::U16,
                    b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
                ),
                image::DynamicImage::ImageRgb16(b) => (
                    craft_image_codecs::ChannelLayout::Rgb,
                    craft_image_codecs::SampleType::U16,
                    b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
                ),
                image::DynamicImage::ImageRgba16(b) => (
                    craft_image_codecs::ChannelLayout::Rgba,
                    craft_image_codecs::SampleType::U16,
                    b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
                ),
                other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
            };
            let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
            craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
                .map(|__b| png.extend_from_slice(&__b))
        }
        .unwrap();
        png.truncate(png.len() / 2);
        png
    };
    {
        let st = app.session.doc_mut().unwrap();
        let d = std::sync::Arc::make_mut(&mut st.doc);
        for (i, w) in [53u32, 59].into_iter().enumerate() {
            let key = format!("cut{i}");
            d.images.insert(key.clone(), ImageBlob::new("image/png", cut(w)));
            let im = ImageObject {
                key,
                width: w,
                height: 41,
                xf: Affine::translate((40.0 + 150.0 * i as f64, 100.0)),
                link: None,
                placement: Default::default(),
                pixels: None,
            };
            let (id, layer) = (d.alloc_id(), d.layers[0].id);
            d.insert(Some(layer), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
        }
        st.revision += 1;
    }
    let ctx = crate::testing::ui_context();
    app.status("");
    frame(&mut app, &ctx, vec![]);
    assert!(app.canvas.shown.as_ref().is_some_and(|(_, o)| o.mark_undecoded_images), "the canvas marks them");
    assert_eq!(app.ui.status, "2 images couldn't be decoded and show as crossed boxes");
    app.status("");
    app.session.doc_mut().unwrap().revision += 1;
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.ui.status, "", "said once");
}

/// The plug-in effects a document's first view leaves out past its budget are reported in the
/// status bar once, in the singular or the plural, and Effect › Refresh Deferred Effects is
/// enabled until it has run them.
#[test]
fn the_first_view_reports_the_plug_in_effects_it_deferred() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use vectorcraft_plugins::wat as tpl;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.canvas.worker_started = true; // render on this thread
    let module = tpl::module(&tpl::manifest("test.ui.deferred", "effect", "{}"), tpl::ECHO, "", 1);
    app.session.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wat::parse_str(module).unwrap())})).unwrap();
    // Each reading of the clock is 3 s later: the first run's result comes back after the budget,
    // and no later run starts.
    let step = std::sync::Arc::new(AtomicU64::new(3_000));
    let ctx = crate::testing::ui_context();
    for (count, said) in [
        (2, "Opened with 2 plug-in effects deferred. Effect › Refresh Deferred Effects runs them."),
        (1, "Opened with 1 plug-in effect deferred. Effect › Refresh Deferred Effects runs it."),
    ] {
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        // Art the plug-in has not been given before: a result it gave is kept and drawn at once.
        for i in 0..count {
            let x = 31.0 + 97.0 * f64::from(i) + 13.0 * f64::from(count);
            let id = app.session.execute("shape.rectangle", &json!({"x": x, "y": 47, "width": 23, "height": 29})).unwrap()["id"].clone();
            app.session.execute("effect.apply", &json!({"ids": [id], "effect": "plugin.test.ui.deferred"})).unwrap();
        }
        app.session.execute("select.none", &json!({})).unwrap();
        let (now, by) = (std::sync::Arc::new(AtomicU64::new(0)), step.clone());
        let clock: vectorcraft_plugins::deferred::Clock =
            std::sync::Arc::new(move || Some(now.fetch_add(by.load(Ordering::Relaxed), Ordering::Relaxed)));
        app.session.doc_mut().unwrap().plugin_effects = vectorcraft_plugins::deferred::Deferrals::with_clock(clock);
        step.store(3_000, Ordering::Relaxed);
        app.status("");
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.ui.status, said);
        assert!(crate::menus::enabled(&app, "effect.refreshDeferred"));
        app.status("");
        app.session.doc_mut().unwrap().revision += 1;
        frame(&mut app, &ctx, vec![]);
        assert_eq!(app.ui.status, "", "said once");
        step.store(0, Ordering::Relaxed);
        app.session.execute("effect.refreshDeferred", &json!({})).unwrap();
        assert!(!crate::menus::enabled(&app, "effect.refreshDeferred"), "nothing is left deferred");
    }
}

/// The render worker's side of a controlled worker: the test thread renders the jobs the canvas
/// queues, with [`crate::render_worker::render`] and the worker's own renderer and last frame.
struct WorkerSide {
    jobs: std::sync::mpsc::Receiver<crate::render_worker::Job>,
    done: std::sync::mpsc::Sender<crate::render_worker::Done>,
    renderer: vectorcraft_render::Renderer,
    kept: Option<crate::canvas::KeptFrame>,
}

impl WorkerSide {
    /// A controlled worker for the canvas, and its side.
    fn start() -> (crate::render_worker::Worker, Self) {
        let (worker, jobs, done) = crate::render_worker::Worker::test_channel();
        (worker, Self { jobs, done, renderer: vectorcraft_render::Renderer::new(), kept: None })
    }

    /// Render the newest job queued, as the worker thread does, and send its frame with a time of
    /// 50 ms: the canvas then keeps sending the document's frames to the worker. False when no job
    /// is queued.
    fn render(&mut self) -> bool {
        let Some(job) = self.jobs.try_iter().last() else { return false };
        let done = crate::render_worker::render(&mut self.renderer, &mut self.kept, job);
        self.done.send(crate::render_worker::Done { ms: 50.0, ..done }).is_ok()
    }
}

/// A drag of an object with raster effects (#1257): the drag's frames render while
/// interacting and draw the object's drop shadow and filtered content from the rasters of the
/// frame before; the frame after the drag renders without `interacting`, on the render worker,
/// and draws what a fresh renderer draws. The worker is a controlled one ([`WorkerSide`]).
#[test]
fn dragging_an_object_with_raster_effects_draws_its_rasters_moved() {
    keeping_color_settings(dragging_with_raster_effects);
}

fn dragging_with_raster_effects() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 80, "width": 120, "height": 90})).unwrap()["id"].clone();
    for effect in ["stylize.dropShadow", "texture.craquelure"] {
        app.run("effect.apply", json!({"effect": effect, "ids": [id]})).unwrap();
    }
    let ctx = crate::testing::ui_context();
    // The art's raster uploaded by one frame, None when it was not re-rendered.
    let upload = |app: &mut VectorcraftApp| -> Option<Vec<Color32>> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let px = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            img.pixels.clone()
        });
        delta.clear();
        px
    };
    upload(&mut app).expect("the first frame renders the art");
    app.select_tool("selection");
    let view = app.view_info();
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 160.0, 125.0), view).unwrap();
    for step in [1.0, 2.0, 3.0] {
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, 160.0 + 7.3 * step, 125.0 + 2.1 * step), view).unwrap();
        upload(&mut app).expect("each frame of the drag renders");
        assert!(app.canvas.key.as_ref().is_some_and(|k| k.interacting), "step {step}");
        assert_eq!(app.canvas.renderer.stats.moved, 2, "the shadow and the filtered content, step {step}");
        assert!(app.canvas.moved);
    }
    // The frame after the drag runs the effects again: on the worker, while the canvas shows
    // the drag's last frame.
    let (worker, mut side) = WorkerSide::start();
    app.canvas.worker = Some(worker);
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 181.9, 131.3), view).unwrap();
    assert!(upload(&mut app).is_none(), "not rendered on this thread");
    assert!(app.canvas.worker.as_ref().and_then(|w| w.pending.as_ref()).is_some_and(|k| !k.interacting), "sent to the worker");
    assert!(side.render(), "the worker renders the frame");
    let after = upload(&mut app).expect("the worker's frame arrives");
    assert!(app.canvas.key.as_ref().is_some_and(|k| !k.interacting));
    assert!(!app.canvas.moved);
    app.canvas.worker = None;
    app.canvas.renderer = vectorcraft_render::Renderer::new();
    app.canvas.key = None;
    assert!(upload(&mut app) == Some(after), "the frame after the drag is what a fresh renderer draws");
}

/// A tool's drag on the canvas renders with `dragging` (#2558). At 3200% a rectangle with
/// Texturizer covers more than the raster budget, so no raster is kept for it, and each frame
/// of a Selection tool drag draws its filtered content at the drag's lower resolution. The
/// frame after the drag draws what a fresh renderer draws. A dialog's preview, an interaction
/// without a drag, renders without `dragging` and draws the same.
#[test]
fn a_drag_on_the_canvas_draws_raster_effects_at_a_lower_resolution() {
    keeping_color_settings(a_drag_at_a_lower_resolution);
}

fn a_drag_at_a_lower_resolution() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 80, "width": 200, "height": 150})).unwrap()["id"].clone();
    app.run("effect.apply", json!({"effect": "texture.texturizer", "ids": [id]})).unwrap();
    app.select_tool("selection");
    let ctx = crate::testing::ui_context();
    // One headless frame on a 1920 × 1200 window (a view larger than the drag's budget) with
    // `events` → the art's raster it uploaded, None when it was not re-rendered.
    let run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| -> Option<Vec<Color32>> {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1200.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        let art = app.canvas.texture.as_ref().map(|t| t.id());
        let px = out.textures_delta.set.iter().find(|e| Some(*e.0) == art).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            img.pixels.clone()
        });
        out.textures_delta.clear();
        px
    };
    run(&mut app, vec![]);
    if let Some(vm) = app.view_mut() {
        (vm.zoom, vm.center) = (32.0, Point::new(200.0, 155.0));
    }
    let still = run(&mut app, vec![]).expect("the zoomed view renders");
    assert_eq!(app.canvas.renderer.stats.previewed, 0, "a still frame");
    let at = app.canvas_view().unwrap().rect.center();
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    run(&mut app, vec![egui::Event::PointerMoved(at)]);
    run(&mut app, vec![button(at, true)]);
    for step in [1.0, 2.0] {
        let drawn = run(&mut app, vec![egui::Event::PointerMoved(at + vec2(13.0, 7.0) * step)]).expect("each frame of the drag renders");
        assert!(app.canvas.key.as_ref().is_some_and(|k| k.interacting && k.dragging), "step {step}");
        assert_eq!(app.canvas.renderer.stats.previewed, 1, "the filtered content, step {step}");
        assert!(app.canvas.moved);
        assert!(drawn != still, "the rectangle moved, step {step}");
    }
    let after = run(&mut app, vec![button(at + vec2(26.0, 14.0), false)]).expect("the frame after the drag renders");
    assert!(app.canvas.key.as_ref().is_some_and(|k| !k.interacting && !k.dragging));
    assert_eq!(app.canvas.renderer.stats.previewed, 0, "the frame after the drag");
    assert!(!app.canvas.moved);
    app.canvas.renderer = vectorcraft_render::Renderer::new();
    app.canvas.key = None;
    let fresh = run(&mut app, vec![]);
    assert!(fresh.as_ref() == Some(&after), "the frame after the drag is what a fresh renderer draws: {}", differing(&after, fresh.as_deref()));
    // A dialog's preview: an interaction, and no drag.
    app.session.begin_interaction("Texturizer").unwrap();
    assert!(run(&mut app, vec![]) == Some(after), "a dialog's preview draws the effect at full resolution");
    assert!(app.canvas.key.as_ref().is_some_and(|k| k.interacting && !k.dragging));
    assert_eq!(app.canvas.renderer.stats.previewed, 0);
}

/// Where frame `b` differs from `a`: how many pixels, the first, and the largest channel difference.
fn differing(a: &[Color32], b: Option<&[Color32]>) -> String {
    let Some(b) = b else { return "not rendered".into() };
    if a.len() != b.len() {
        return format!("{} pixels against {}", b.len(), a.len());
    }
    let diff: Vec<(usize, u8)> = a
        .iter()
        .zip(b)
        .enumerate()
        .filter(|(_, (p, q))| p != q)
        .map(|(i, (p, q))| (i, p.to_array().iter().zip(q.to_array()).map(|(x, y)| x.abs_diff(y)).max().unwrap_or(0)))
        .collect();
    let first = diff.first().map(|&(i, _)| (i, a[i], b[i]));
    let most = diff.iter().map(|d| d.1).max().unwrap_or(0);
    format!("{} of {} pixels, first {first:?}, at most {most} apart", diff.len(), a.len())
}

/// Dragging a small square over an A4 page filled by a rectangle blurred by 500 pt, at 200%:
/// the blur reaches far past the raster budget, so the still frame keeps the backdrop's raster
/// cropped to the view, and each frame of a Selection tool drag draws it from there instead of
/// previewing it at the drag's lower resolution: on this thread, and on the worker, where the
/// frames of a heavy document render. The worker is a controlled one ([`WorkerSide`]).
#[test]
fn a_drag_over_a_wide_blur_draws_the_blur_from_the_still_frame() {
    keeping_color_settings(a_drag_over_a_wide_blur);
}

fn a_drag_over_a_wide_blur() {
    for on_worker in [false, true] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 595, "height": 842})).unwrap();
        app.canvas.worker_started = true; // no worker unless `on_worker`
        let back = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 595, "height": 842})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [back], "color": "#1a4ce6"})).unwrap();
        app.run("effect.apply", json!({"effect": "blur.gaussian", "params": {"radius": 500}, "ids": [back]})).unwrap();
        let square = app.run("shape.rectangle", json!({"x": 260, "y": 380, "width": 60, "height": 60})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [square]})).unwrap();
        app.select_tool("selection");
        let ctx = crate::testing::ui_context();
        // One headless frame with `events`, then, with a worker, its frames rendered and shown
        // until none is pending.
        let run = |app: &mut VectorcraftApp, side: &mut Option<WorkerSide>, events: Vec<egui::Event>| {
            let frame = |app: &mut VectorcraftApp, events| {
                let raw =
                    egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 870.0))), events, ..Default::default() };
                let mut out = ctx.run_ui(raw, |ui| show(app, ui));
                out.textures_delta.clear();
            };
            frame(app, events);
            for _ in 0..8 {
                if !side.as_mut().is_some_and(WorkerSide::render) {
                    break;
                }
                frame(app, vec![]);
            }
            assert!(app.canvas.worker.as_ref().is_none_or(|w| w.pending.is_none()), "the worker's frame is shown");
        };
        let moved = |app: &VectorcraftApp| if on_worker { app.canvas.moved } else { app.canvas.renderer.stats.moved > 0 };
        let mut side = None;
        run(&mut app, &mut side, vec![]);
        if let Some(vm) = app.view_mut() {
            (vm.zoom, vm.center) = (2.0, Point::new(290.0, 410.0));
        }
        if on_worker {
            let (worker, worker_side) = WorkerSide::start();
            app.canvas.worker = Some(worker);
            app.canvas.last_ms = 50.0;
            side = Some(worker_side);
        }
        run(&mut app, &mut side, vec![]);
        assert_eq!(app.canvas.from_worker, on_worker);
        assert!(!moved(&app), "a still frame, on the worker: {on_worker}");
        let at = app.canvas_view().unwrap().rect.center();
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run(&mut app, &mut side, vec![egui::Event::PointerMoved(at)]);
        run(&mut app, &mut side, vec![button(at, true)]);
        for step in [1.0, 2.0, 3.0] {
            run(&mut app, &mut side, vec![egui::Event::PointerMoved(at + vec2(13.0, 7.0) * step)]);
            assert!(app.canvas.key.as_ref().is_some_and(|k| k.interacting && k.dragging), "step {step}, on the worker: {on_worker}");
            assert_eq!(app.canvas.from_worker, on_worker, "step {step}");
            assert!(!moved(&app), "the blur drawn from the still frame's raster, step {step}, on the worker: {on_worker}");
        }
        run(&mut app, &mut side, vec![button(at + vec2(39.0, 21.0), false)]);
        let info = app.run("document.inspect", json!({})).unwrap();
        let children = info["layers"][0]["children"].as_array().unwrap().clone();
        let x0 = |id: &serde_json::Value| children.iter().find(|c| c["id"] == *id).unwrap()["bounds"]["x"].as_f64().unwrap();
        assert!(x0(&square) > 270.0 && x0(&back) == 0.0, "the square moved, the backdrop did not");
        app.canvas.worker = None;
    }
}

/// An interaction's frames stay on the worker once it drew the frame shown, however fast that
/// frame was: the worker's renderer keeps the effect rasters that made it fast, and this
/// thread's renderer, without them, would draw the next frame slowly. Without a frame from the
/// worker, a fast frame renders on this thread as before.
#[test]
fn an_interaction_stays_on_the_worker_that_drew_the_frame_shown() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    app.canvas.worker_started = true; // the first frame renders on this thread
    let ctx = crate::testing::ui_context();
    shapes(&mut app, &ctx);
    for from_worker in [true, false] {
        app.canvas.worker = crate::render_worker::Worker::spawn(ctx.clone());
        (app.canvas.from_worker, app.canvas.last_ms) = (from_worker, 1.0);
        app.session.begin_interaction("Preview").unwrap();
        shapes(&mut app, &ctx);
        let pending = app.canvas.worker.as_ref().and_then(|w| w.pending.as_ref()).is_some_and(|k| k.interacting);
        assert_eq!(pending, from_worker, "sent to the worker: {from_worker}");
        if !from_worker {
            assert!(app.canvas.key.as_ref().is_some_and(|k| k.interacting), "rendered on this thread");
        }
        app.session.cancel_interaction().unwrap();
        app.canvas.worker = None;
        shapes(&mut app, &ctx);
    }
}

/// The frame after an interaction renders without `interacting` also when the interaction
/// leaves the document as it was (#1257): here a drag back to where it started.
#[test]
fn a_drag_back_to_its_start_renders_again_after_it() {
    keeping_color_settings(a_drag_back_to_its_start);
}

fn a_drag_back_to_its_start() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 80, "width": 120, "height": 90})).unwrap()["id"].clone();
    app.run("effect.apply", json!({"effect": "texture.craquelure", "ids": [id]})).unwrap();
    let ctx = crate::testing::ui_context();
    shapes(&mut app, &ctx);
    app.select_tool("selection");
    let view = app.view_info();
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 160.0, 125.0), view).unwrap();
    for (x, y) in [(173.3, 129.1), (160.0, 125.0)] {
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, x, y), view).unwrap();
        shapes(&mut app, &ctx);
        assert_eq!(app.canvas.renderer.stats.moved, 1, "the filtered content at ({x}, {y})");
    }
    let revision = app.session.active().unwrap().revision;
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 160.0, 125.0), view).unwrap();
    assert_eq!(app.session.active().unwrap().revision, revision, "the drag left the document as it was");
    shapes(&mut app, &ctx);
    assert!(app.canvas.key.as_ref().is_some_and(|k| !k.interacting), "rendered again");
    assert_eq!(app.canvas.renderer.stats.moved, 0);
    assert!(!app.canvas.moved);
}

/// Cache metadata, worker requests and view transforms use the renderer's bounded raster size,
/// on a display that reports a larger texture side than that.
/// Keep the worker controlled so huge virtual viewports never allocate huge test images.
#[test]
fn canvas_render_jobs_use_renderer_limited_dimensions() {
    let ctx = crate::testing::ui_context();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true;
    app.ui.view.rulers = false;
    let frame = |app: &mut VectorcraftApp, size, ppp| {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), size)),
            max_texture_side: Some(1 << 20),
            ..Default::default()
        };
        input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(ppp);
        let mut output = ctx.run_ui(input, |ui| show(app, ui));
        output.textures_delta.clear();
    };
    frame(&mut app, vec2(400.0, 300.0), 1.0);
    let (worker, jobs, _completed) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    app.canvas.last_ms = 50.0;
    // A component past the framework's size guard (`MAX_EXTENT` points) shows nothing, so the windows stay inside it
    // and their pixel density takes them past the renderer's side.
    let side = craft::ui::contract::MAX_EXTENT - 0.75;
    for (size, ppp, wide) in [(vec2(side, 32.0), 2.5, true), (vec2(32.0, side), 2.5, false), (vec2(side, 32.0), 3.0, true)] {
        frame(&mut app, size, ppp);
        let job = jobs.try_recv().expect("oversized viewport queued on controlled worker");
        assert_eq!((job.key.w, job.key.h), (job.w, job.h));
        assert_eq!(if wide { job.w } else { job.h }, u32::from(u16::MAX));
        assert!((1..=u32::from(u16::MAX)).contains(&job.w));
        assert!((1..=u32::from(u16::MAX)).contains(&job.h));
        let center = job.view * Point::new(job.key.cx, job.key.cy);
        assert!((center.x - f64::from(job.w) / 2.0).abs() < 0.001);
        assert!((center.y - f64::from(job.h) / 2.0).abs() < 0.001);
    }
}

/// A cached texture keeps the renderer's document mapping when the viewport changes. The
/// controlled worker cannot finish early, even if egui repeats a layout pass.
#[test]
fn cached_frames_keep_their_scale_during_resize_rotation_and_dpi_changes() {
    keeping_color_settings(cached_frames_keep_their_scale);
}

fn cached_frames_keep_their_scale() {
    for (old_rot, new_rot, old_ppp, new_ppp, pixel, resize_only) in [
        // Isolate the original panel/window resize: zoom, pan, rotation and DPI stay fixed.
        (0.0, 0.0, 1.0, 1.0, false, true),
        (30.0, 30.0, 1.25, 1.25, false, true),
        (0.0, 0.0, 1.0, 1.0, false, false),
        (30.0, 30.0, 1.25, 1.25, false, false),
        (30.0, 75.0, 1.25, 1.25, false, false),
        (30.0, 75.0, 1.25, 2.0, false, false),
        (30.0, 30.0, 2.0, 1.25, false, false),
        (30.0, 75.0, 1.25, 2.0, true, false),
    ] {
        let ctx = crate::testing::ui_context();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.canvas.worker_started = true;
        app.ui.view.pixel_preview = pixel;
        let frame = |app: &mut VectorcraftApp, size: egui::Vec2, ppp: f32| {
            let mut input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(pos2(31.25, 19.5), size)), ..Default::default() };
            // Monitor DPI is viewport input, not egui's UI zoom (which rescales the last
            // screen rect and would override the simultaneous resize under test).
            input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(ppp);
            let mut out = ctx.run_ui(input, |ui| show(app, ui));
            assert!((out.pixels_per_point - ppp).abs() < 0.001);
            out.textures_delta.clear();
            let tex = app.canvas.texture.as_ref().expect("rendered canvas texture").id();
            flat_shapes(out.shapes)
                .into_iter()
                .find_map(|s| match s {
                    Shape::Mesh(m) if m.texture_id == tex => Some(m),
                    _ => None,
                })
                .expect("the canvas texture is drawn")
        };
        frame(&mut app, vec2(800.25, 600.35), old_ppp);
        {
            let v = app.view_mut().unwrap();
            v.rotation = old_rot;
            v.zoom = 2.5;
            v.center = Point::new(210.0, 160.0);
        }
        frame(&mut app, vec2(800.25, 600.35), old_ppp);
        let old = app.canvas.key.clone().unwrap();
        let (worker, jobs, completed) = crate::render_worker::Worker::test_channel();
        app.canvas.worker = Some(worker);
        app.canvas.last_ms = 50.0;
        {
            let v = app.view_mut().unwrap();
            v.rotation = new_rot;
            if !resize_only {
                v.zoom = 3.25;
                v.center = Point::new(225.0, 170.0);
            }
        }
        let size = vec2(602.5, 720.1);
        let mesh = frame(&mut app, size, new_ppp);
        assert_eq!(app.canvas.key.as_ref(), Some(&old), "the cached frame is still displayed");
        let job = jobs.try_recv().expect("the replacement frame was submitted");
        assert_eq!(job.key.ppp, new_ppp);
        assert_eq!(job.key.rot, new_rot);
        if resize_only {
            assert_eq!((job.key.zoom, job.key.cx, job.key.cy, job.key.rot, job.key.ppp), (old.zoom, old.cx, old.cy, old.rot, old.ppp));
        }
        assert_ne!((job.key.w, job.key.h), (old.w, old.h));
        let xf = app.canvas_view().unwrap();
        let render_view = Affine::translate((old.w as f64 / 2.0, old.h as f64 / 2.0))
            * Affine::rotate(old.rot.to_radians())
            * Affine::scale(old.zoom * old.ppp as f64)
            * Affine::translate((-old.cx, -old.cy));
        assert_eq!(mesh.vertices.len(), 4);
        for vertex in &mesh.vertices {
            assert!([pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)].contains(&vertex.uv));
            let doc = if let Some([x0, y0, x1, y1]) = old.pixel {
                Point::new(x0 as f64 + vertex.uv.x as f64 * (x1 - x0) as f64, y0 as f64 + vertex.uv.y as f64 * (y1 - y0) as f64)
            } else {
                render_view.inverse() * Point::new(vertex.uv.x as f64 * old.w as f64, vertex.uv.y as f64 * old.h as f64)
            };
            assert!(
                (vertex.pos - xf.to_screen(doc)).length() < 0.002,
                "{old_rot}° -> {new_rot}°, DPI {old_ppp} -> {new_ppp}, pixel={pixel}: {vertex:?}"
            );
        }
        // Deliver the actual replacement and verify that it replaces the cached key.
        let img = app.canvas.renderer.render(&job.doc, job.w, job.h, job.view, &job.opts);
        let expected = job.key.clone();
        completed.send(crate::render_worker::Done { key: job.key, doc: job.doc, opts: job.opts, img, ms: 1.0, moved: 0, reused: false }).unwrap();
        frame(&mut app, size, new_ppp);
        assert_eq!(app.canvas.key.as_ref(), Some(&expected), "the worker's frame arrived");
    }
}

/// When the view turns while the worker renders the frame at the new angle (Reset Rotate View on
/// a heavy document), the canvas draws the last frame turned until that frame arrives: each
/// corner goes where the new view puts the document point that the corner showed. The new frame
/// is then drawn unturned. In Pixel Preview the last frame's pixels stay over the document rect
/// they cover. The worker is a controlled one ([`WorkerSide`]).
#[test]
fn a_turned_view_draws_the_last_frame_turned_until_its_own_arrives() {
    keeping_color_settings(a_turned_view);
}

fn a_turned_view() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let ctx = crate::testing::ui_context();
    shapes(&mut app, &ctx); // the first frame fits the view to the artboard
    app.view_mut().unwrap().rotation = 30.0;
    shapes(&mut app, &ctx);
    let rect = app.canvas_view().unwrap().rect;
    let old = app.canvas_view().unwrap();
    // `art` runs one frame and finds the mesh that draws the art's texture.
    let art = |app: &mut VectorcraftApp| {
        let s = shapes(app, &ctx);
        let tex = app.canvas.texture.as_ref().unwrap().id();
        s.into_iter().find_map(|s| match s {
            Shape::Mesh(m) if m.texture_id == tex => Some(m),
            _ => None,
        })
    };
    // With a worker and a last frame that took 50 ms, the next frame renders on the worker.
    let (worker, mut side) = WorkerSide::start();
    app.canvas.worker = Some(worker);
    app.canvas.last_ms = 50.0;
    app.run("view.rotateReset", json!({})).unwrap();
    let mesh = art(&mut app).expect("the last frame, turned");
    assert!(app.canvas.worker.as_ref().and_then(|w| w.pending.as_ref()).is_some_and(|k| k.rot == 0.0), "sent to the worker");
    let new = app.canvas_view().unwrap();
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    assert_eq!(mesh.vertices.len(), 4);
    for ((v, corner), uv) in mesh.vertices.iter().zip(corners).zip([pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)]) {
        let at = new.to_screen(old.to_doc(corner));
        assert!((v.pos - at).length() < 0.01 && v.uv == uv, "{v:?}: texture corner {uv:?} at {at:?}");
    }
    // The point where the last frame drew the rectangle's top-left corner lands where the new view puts that corner.
    let p = Point::new(100.0, 50.0);
    let uv = (old.to_screen(p) - rect.min) / rect.size();
    let [a, b, _, d] = [0, 1, 2, 3].map(|i| mesh.vertices[i].pos);
    assert!((a + (b - a) * uv.x + (d - a) * uv.y - new.to_screen(p)).length() < 0.01, "the rectangle's corner");
    // The worker's frame at 0° then covers the canvas unturned.
    assert!(side.render(), "the worker renders the frame at 0°");
    shapes(&mut app, &ctx);
    assert!(app.canvas.key.as_ref().is_some_and(|k| k.rot == 0.0), "the worker's frame arrives");
    let b = art(&mut app).expect("the worker's frame").calc_bounds();
    assert!((b.min - rect.min).length() < 0.01 && (b.max - rect.max).length() < 0.01, "{b:?} over {rect:?}");
    // In Pixel Preview the last frame's pixels are drawn over the document rect they cover.
    app.canvas.worker = None;
    app.ui.view.pixel_preview = true;
    app.view_mut().unwrap().zoom = 8.0;
    shapes(&mut app, &ctx);
    let [x0, y0, ..] = app.canvas.key.as_ref().and_then(|k| k.pixel).expect("the pixels rendered");
    let (worker, _side) = WorkerSide::start();
    app.canvas.worker = Some(worker);
    app.canvas.last_ms = 50.0;
    app.view_mut().unwrap().rotation = 30.0;
    let mesh = art(&mut app).expect("the last frame's pixels");
    let at = app.canvas_view().unwrap().to_screen(Point::new(x0 as f64, y0 as f64));
    assert!((mesh.vertices[0].pos - at).length() < 0.01, "{:?} at {at:?}", mesh.vertices[0].pos);
    app.canvas.worker = None;
}

/// While the worker renders the frame for a panned view, the fill worker renders a
/// low-resolution frame, and the part of the canvas the last frame no longer covers shows the
/// art from it, drawn only there. When the worker's frame arrives, it covers the canvas and the
/// fill is dropped.
#[test]
fn a_pan_fills_the_area_the_last_frame_leaves_until_the_worker_frame_arrives() {
    keeping_color_settings(a_pan_fills_the_area);
}

fn a_pan_fills_the_area() {
    let ctx = crate::testing::ui_context();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 300, "y": 100, "width": 80, "height": 100})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    app.ui.view.rulers = false;
    app.ui.screen_mode = 2; // no scroll bars: the canvas is the window
    // One frame → the meshes drawn and the fill's pixels, when this frame uploaded them.
    let frame = |app: &mut VectorcraftApp| {
        let (shapes, mut delta) = frame_output(app, &ctx);
        let fill = app.canvas.fill.as_ref().map(|f| f.tex.id());
        let pixels = delta.set.iter().find(|(id, _)| Some(**id) == fill).and_then(|(_, d)| d.last()).map(|d| match &d.image {
            egui::ImageData::Color(img) => img.clone(),
        });
        delta.clear();
        let meshes: Vec<egui::Mesh> = shapes
            .into_iter()
            .filter_map(|s| match s {
                Shape::Mesh(m) => Some(std::sync::Arc::unwrap_or_clone(m)),
                _ => None,
            })
            .collect();
        (meshes, pixels)
    };
    frame(&mut app);
    {
        let v = app.view_mut().unwrap();
        v.zoom = 4.0;
        v.center = Point::new(100.0, 150.0);
    }
    frame(&mut app); // the art from about x 0 to 200, without the rectangle
    let old = app.canvas.key.clone().unwrap();
    let before = app.canvas_view().unwrap();
    assert!(app.canvas.fill.is_none());
    let (worker, jobs, completed) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    let (fill_worker, fill_jobs, fill_completed) = crate::render_worker::Worker::test_channel();
    app.canvas.fill_worker = Some(fill_worker);
    app.canvas.last_ms = 50.0;
    // Pan right by 150 pt: the last frame covers the canvas's left quarter, and the rectangle
    // comes into its right quarter.
    app.view_mut().unwrap().center = Point::new(250.0, 150.0);
    frame(&mut app);
    assert_eq!(app.canvas.key.as_ref(), Some(&old), "the last frame is still shown");
    let job = jobs.try_recv().expect("the frame for the new view is on the worker");
    let fill_job = fill_jobs.try_recv().expect("the fill is on the fill worker");
    let (fw, fh, _) = fill_view(&job.key);
    assert!(fill_job.opts.kept_images_only && (fill_job.w, fill_job.h) == (fw, fh), "{:?}", (fill_job.w, fill_job.h));
    assert!(!fill_job.may_copy && !fill_job.may_keep, "the fill worker copies nothing and holds no frame");
    assert!(app.canvas.fill.is_none(), "nothing to draw until the fill arrives");
    // The fill arrives; the next frame draws it, and asks for no other fill.
    let img = app.canvas.renderer.render(&fill_job.doc, fill_job.w, fill_job.h, fill_job.view, &fill_job.opts);
    let (doc, opts) = (fill_job.doc, fill_job.opts);
    fill_completed.send(crate::render_worker::Done { key: fill_job.key, doc, opts, img, ms: 5.0, moved: 0, reused: false }).unwrap();
    let (meshes, pixels) = frame(&mut app);
    assert!(fill_jobs.try_recv().is_err(), "the fill covers the canvas");
    assert_eq!(app.canvas.key.as_ref(), Some(&old), "the last frame is still shown");
    let fill = app.canvas.fill.as_ref().expect("the fill is kept");
    let mesh = meshes.iter().find(|m| m.texture_id == fill.tex.id()).expect("the fill is drawn");
    let rect = app.canvas_view().unwrap().rect;
    let xf = app.canvas_view().unwrap();
    let edge = xf.to_screen(before.to_doc(rect.right_top())).x; // the last frame's right edge
    assert!(edge > rect.left() + 100.0 && edge < xf.to_screen(Point::new(300.0, 0.0)).x, "{edge} in {rect:?}");
    assert!(
        mesh.vertices.iter().all(|v| v.pos.x >= edge - 1e-3 && rect.expand(1e-3).contains(v.pos)),
        "only right of the last frame: {:?}",
        mesh.vertices
    );
    let area: f32 = mesh.indices.as_chunks::<3>().0.iter().map(|t| polygon_area(&t.map(|i| mesh.vertices[i as usize].pos))).sum();
    let uncovered = (rect.right() - edge) * rect.height();
    assert!((area - uncovered).abs() < 1.0, "the whole uncovered area, {uncovered} square points: {area}");
    // The fill's pixel under a point of the canvas, from the uv the mesh gives that point.
    let img = pixels.expect("the fill's pixels were uploaded");
    let to_uv = Affine::scale_non_uniform(1.0 / fill.size.0 as f64, 1.0 / fill.size.1 as f64) * fill.view * xf.affine().inverse();
    let alpha = |p: Pos2| {
        let uv = to_uv * Point::new(p.x as f64, p.y as f64);
        img.pixels[(uv.y * img.size[1] as f64) as usize * img.size[0] + (uv.x * img.size[0] as f64) as usize].a()
    };
    assert!(alpha(xf.to_screen(Point::new(340.0, 150.0))) > 0, "the rectangle, in the uncovered area");
    assert_eq!(alpha(xf.to_screen(Point::new(250.0, 150.0))), 0, "no art there");
    for v in &mesh.vertices {
        assert!((to_uv * Point::new(v.pos.x as f64, v.pos.y as f64) - Point::new(v.uv.x as f64, v.uv.y as f64)).hypot() < 1e-4);
    }
    // Panned until the fill reaches only the canvas's left half: it is drawn only there, and the
    // fill for the new view is asked for.
    let fill_tex = fill.tex.id();
    let reach = fill.view.inverse() * Point::new(fill.size.0 as f64, 0.0); // the fill's right edge
    app.view_mut().unwrap().center = Point::new(reach.x, 150.0);
    let (meshes, _) = frame(&mut app);
    let right = app.canvas_view().unwrap().to_screen(reach).x;
    assert!((right - rect.center().x).abs() < 1.0, "{right} in {rect:?}");
    let mesh = meshes.iter().find(|m| m.texture_id == fill_tex).expect("the fill is drawn where it reaches");
    assert!(mesh.vertices.iter().all(|v| v.pos.x <= right + 1e-3), "only where the fill reaches: {:?}", mesh.vertices);
    let next = fill_jobs.try_recv().expect("the next fill");
    assert_eq!(Some(&next.key), app.canvas.worker.as_ref().unwrap().pending.as_ref());
    app.view_mut().unwrap().center = Point::new(250.0, 150.0);
    // That fill finishes after the worker's frame: it is taken and not kept.
    let img = app.canvas.renderer.render(&next.doc, next.w, next.h, next.view, &next.opts);
    fill_completed.send(crate::render_worker::Done { key: next.key, doc: next.doc, opts: next.opts, img, ms: 5.0, moved: 0, reused: false }).unwrap();
    // The worker's frame arrives: it covers the canvas, and the fill is no longer drawn or kept.
    let img = app.canvas.renderer.render(&job.doc, job.w, job.h, job.view, &job.opts);
    let expected = job.key.clone();
    completed.send(crate::render_worker::Done { key: job.key, doc: job.doc, opts: job.opts, img, ms: 50.0, moved: 0, reused: false }).unwrap();
    let (meshes, _) = frame(&mut app);
    assert_eq!(app.canvas.key.as_ref(), Some(&expected), "the worker's frame is shown");
    assert!(app.canvas.fill.is_none() && meshes.iter().all(|m| m.texture_id != fill_tex), "the fill is dropped");
    assert!(app.canvas.fill_worker.as_ref().unwrap().pending.is_none(), "the late fill was taken");
    // A pan with Outline turned on: the frame shown's options are not the ones asked for, and
    // no fill is asked for.
    app.view_mut().unwrap().outline = true;
    app.view_mut().unwrap().center = Point::new(400.0, 150.0);
    frame(&mut app);
    assert!(jobs.try_iter().last().is_some_and(|j| j.key.outline), "the outline frame is on the worker");
    assert!(fill_jobs.try_recv().is_err() && app.canvas.fill.is_none(), "no fill");
    app.canvas.worker = None;
    app.canvas.fill_worker = None;
}

/// With the web build's setting (`waits_for_still`, no render worker), a heavy document's canvas
/// is not rendered while the view moves: each frame draws the frame shown moved and scaled with
/// the view and asks for a frame within STILL_SECS. The canvas renders once the view has been
/// still for STILL_SECS. An edit while the view moves draws the part it changed into the frame
/// shown, and one that changes most of the frame renders it whole at once. With a worker's build settings the same frames render at once, as
/// before.
#[test]
fn without_a_worker_a_heavy_canvas_is_reprojected_until_the_view_is_still() {
    keeping_color_settings(without_a_worker_a_heavy_canvas);
}

fn without_a_worker_a_heavy_canvas() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    assert!(!app.canvas.waits_for_still, "off the web build");
    app.canvas.worker_started = true; // no worker, as in the web build
    app.canvas.waits_for_still = true;
    let ctx = crate::testing::ui_context();
    // Frames at 60 fps: a frame's passes share its time.
    let clock = std::cell::Cell::new(0.0);
    // One frame → (the bounds of the mesh that draws the art's texture, the repaint delay asked for).
    let frame = |app: &mut VectorcraftApp| {
        clock.set(clock.get() + 1.0 / 60.0);
        let screen_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
        let raw = egui::RawInput { screen_rect, time: Some(clock.get()), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
        let tex = app.canvas.texture.as_ref().unwrap().id();
        let art = flat_shapes(std::mem::take(&mut out.shapes)).into_iter().find_map(|s| match s {
            Shape::Mesh(m) if m.texture_id == tex => Some(m.calc_bounds()),
            _ => None,
        });
        (art.expect("the art's texture is drawn"), out.viewport_output[&egui::ViewportId::ROOT].repaint_delay)
    };
    frame(&mut app); // the first frame fits the view to the artboard
    frame(&mut app);
    let rect = app.canvas_view().unwrap().rect;
    // A light document renders every change of view at once.
    app.canvas.last_ms = 1.0;
    app.view_mut().unwrap().center.x += 10.0;
    frame(&mut app);
    let shown = app.canvas.key.clone().unwrap();
    assert_eq!(shown.cx, app.view().unwrap().center.x, "rendered at once");
    // A heavy one draws the frame shown moved and scaled with the view.
    app.canvas.last_ms = 50.0;
    let old = app.canvas_view().unwrap();
    for (dx, zoom) in [(10.0, 1.0), (10.0, 1.5), (-25.0, 1.5)] {
        let v = app.view_mut().unwrap();
        v.center.x += dx;
        v.zoom *= zoom;
        let (b, delay) = frame(&mut app);
        assert_eq!(app.canvas.key.as_ref(), Some(&shown), "not rendered while the view moves");
        let new = app.canvas_view().unwrap();
        let (a, z) = (new.to_screen(old.to_doc(rect.min)), new.to_screen(old.to_doc(rect.max)));
        assert!((b.min - a).length() < 0.01 && (b.max - z).length() < 0.01, "{b:?}: the frame shown at {a:?} to {z:?}");
        assert!(delay <= std::time::Duration::from_secs_f64(STILL_SECS), "a frame within STILL_SECS: {delay:?}");
    }
    // The view stays still: frames at 60 fps wait until STILL_SECS has passed, and the next one
    // renders.
    let mut frames = 0;
    let b = loop {
        frames += 1;
        let (b, _) = frame(&mut app);
        if app.canvas.key.as_ref() != Some(&shown) || frames == 60 {
            break b;
        }
    };
    assert!((12..=13).contains(&frames), "rendered after about 12 frames: {frames}");
    let now = app.canvas.key.clone().unwrap();
    assert_eq!((now.cx, now.zoom), (app.view().unwrap().center.x, app.view().unwrap().zoom), "rendered for the view");
    assert!((b.min - rect.min).length() < 0.01 && (b.max - rect.max).length() < 0.01, "{b:?} over {rect:?}");
    // An edit while the view moves shows at once: the part it changed is drawn into the frame
    // shown, at the view that frame was rendered for, and the frame stays drawn reprojected until
    // the view is still (#8688).
    app.canvas.last_ms = 50.0;
    app.view_mut().unwrap().center.y += 10.0;
    frame(&mut app);
    assert_eq!(app.canvas.key.as_ref(), Some(&now), "waiting");
    app.session.execute("shape.rectangle", &json!({"x": 110, "y": 60, "width": 20, "height": 20})).unwrap();
    let render_ms = app.canvas.last_ms;
    let (b, delay) = frame(&mut app);
    let k = app.canvas.key.clone().unwrap();
    assert_eq!(k.revision, app.session.active().unwrap().revision, "the edit is drawn");
    assert_eq!((k.cx, k.cy, k.zoom), (now.cx, now.cy, now.zoom), "at the view of the frame shown");
    assert_eq!(app.canvas.last_ms, render_ms, "the frame was not rendered whole");
    let new = app.canvas_view().unwrap();
    let mut old = new;
    old.center = Point::new(now.cx, now.cy);
    let (a, z) = (new.to_screen(old.to_doc(rect.min)), new.to_screen(old.to_doc(rect.max)));
    assert!((b.min - a).length() < 0.01 && (b.max - z).length() < 0.01, "{b:?}: the frame shown at {a:?} to {z:?}");
    assert!(delay <= std::time::Duration::from_secs_f64(STILL_SECS), "a frame within STILL_SECS: {delay:?}");
    let mut frames = 0;
    while app.canvas.key.as_ref() == Some(&k) && frames < 60 {
        frames += 1;
        frame(&mut app);
    }
    let k = app.canvas.key.clone().unwrap();
    assert_eq!((k.revision, k.cy), (app.session.active().unwrap().revision, app.view().unwrap().center.y), "rendered once still");
    // An edit that changes more than half of the frame renders whole at once, view and all.
    app.canvas.last_ms = 50.0;
    app.view_mut().unwrap().center.y += 10.0;
    frame(&mut app);
    app.session.execute("shape.rectangle", &json!({"x": -1000, "y": -1000, "width": 3000, "height": 3000})).unwrap();
    frame(&mut app);
    let k = app.canvas.key.clone().unwrap();
    assert_eq!((k.revision, k.cy), (app.session.active().unwrap().revision, app.view().unwrap().center.y), "rendered whole after the edit");
    // With a worker's build settings and no worker (VECTORCRAFT_SYNC_RENDER), a heavy document
    // renders every change of view at once.
    app.canvas.waits_for_still = false;
    app.canvas.last_ms = 50.0;
    app.view_mut().unwrap().center.x += 10.0;
    frame(&mut app);
    assert_eq!(app.canvas.key.as_ref().map(|k| k.cx), Some(app.view().unwrap().center.x), "rendered at once");
}

/// With the web build's setting (`waits_for_still`, no render worker), a heavy document's view
/// that stops moving while a drag goes on editing the document renders whole once the view has
/// been still for STILL_SECS: each edit is drawn into the frame shown and does not start the wait
/// again.
#[test]
fn without_a_worker_edits_while_the_view_is_still_do_not_hold_off_the_render() {
    keeping_color_settings(edits_while_the_view_is_still);
}

fn edits_while_the_view_is_still() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    app.canvas.worker_started = true; // no worker, as in the web build
    app.canvas.waits_for_still = true;
    let ctx = crate::testing::ui_context();
    // Frames at 60 fps: a frame's passes share its time.
    let clock = std::cell::Cell::new(0.0);
    let frame = |app: &mut VectorcraftApp| {
        clock.set(clock.get() + 1.0 / 60.0);
        let screen_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
        let raw = egui::RawInput { screen_rect, time: Some(clock.get()), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    frame(&mut app); // the first frame fits the view to the artboard
    frame(&mut app);
    app.canvas.last_ms = 50.0;
    // The view moves once (a drag at an edge of the canvas scrolls it) and then stays still.
    app.view_mut().unwrap().center.y += 10.0;
    frame(&mut app);
    let shown = app.canvas.key.clone().unwrap();
    assert_ne!(shown.cy, app.view().unwrap().center.y, "waiting");
    // The drag edits the document on every frame.
    let mut frames = 0;
    while app.canvas.key.as_ref().is_none_or(|k| k.cy != app.view().unwrap().center.y) && frames < 60 {
        frames += 1;
        let x = 110 + (frames % 10) * 5;
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 60, "width": 4, "height": 4})).unwrap();
        frame(&mut app);
        let k = app.canvas.key.as_ref().unwrap();
        assert_eq!(k.revision, app.session.active().unwrap().revision, "each edit is drawn");
    }
    assert!((11..=13).contains(&frames), "rendered whole about STILL_SECS after the view stopped: {frames} frames");
}

/// [`still_wait`] waits while only the view changes on a slow canvas: each change starts
/// STILL_SECS again. It does not wait for a light document, a still view, a clock that went
/// back, an edit, a switch into Pixel Preview or a canvas with no frame shown.
#[test]
fn the_canvas_waits_only_while_the_view_of_a_slow_frame_changes() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    shapes(&mut app, &crate::testing::ui_context());
    let shown = app.canvas.key.clone().unwrap();
    let mut still = Some((shown.clone(), 0.0));
    let moved = CacheKey { cx: shown.cx + 5.0, zoom: shown.zoom * 2.0, ..shown.clone() };
    assert_eq!(still_wait(&mut still, Some(&shown), &moved, true, 10.0), Some(STILL_SECS), "the view moved");
    assert_eq!(still_wait(&mut still, Some(&shown), &moved, true, 10.0625), Some(STILL_SECS - 0.0625), "still for 62.5 ms");
    let turned = CacheKey { rot: 15.0, w: shown.w + 40, ppp: 2.0, ..moved.clone() };
    assert_eq!(still_wait(&mut still, Some(&shown), &turned, true, 11.0), Some(STILL_SECS), "turned and resized: again");
    assert_eq!(still_wait(&mut still, Some(&shown), &turned, true, 11.25), None, "still for longer than STILL_SECS");
    assert_eq!(still_wait(&mut still, Some(&shown), &turned, false, 11.0), None, "a light document");
    assert_eq!(still_wait(&mut still, Some(&shown), &turned, true, 10.5), None, "the clock went back");
    let edited = CacheKey { revision: shown.revision + 1, ..turned.clone() };
    assert_eq!(still_wait(&mut still, Some(&shown), &edited, true, 12.0), None, "an edit");
    let pixels = CacheKey { pixel: Some([0, 0, 10, 10]), ..moved.clone() };
    assert_eq!(still_wait(&mut still, Some(&shown), &pixels, true, 13.0), None, "into Pixel Preview");
    let shown_pixels = CacheKey { pixel: Some([5, 5, 20, 20]), ..shown.clone() };
    assert_eq!(still_wait(&mut still, Some(&shown_pixels), &pixels, true, 13.0), Some(STILL_SECS), "Pixel Preview, moved");
    assert_eq!(still_wait(&mut still, None, &moved, true, 14.0), None, "no frame shown");
    assert_eq!(still_wait(&mut still, Some(&shown), &shown, true, 15.0), None, "the frame shown");
}

/// A bitmap made in the background for a file that only another document holds leaves this
/// document's canvas key as it is. Without a render worker (the web build's setting), a pan of a
/// heavy canvas keeps drawing the frame shown until the view is still. A light canvas renders the
/// pan at once, and its key differs from the frame shown only in the view: the pan fill and the
/// worker's pixel copy ask for that.
#[test]
fn a_bitmap_made_for_another_documents_file_leaves_a_pan_waiting() {
    keeping_color_settings(a_bitmap_for_another_documents_file);
}

fn a_bitmap_for_another_documents_file() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    app.canvas.worker_started = true; // no worker, as in the web build
    app.canvas.waits_for_still = true;
    // Another window's document places a file, which its canvas asks a bitmap of.
    let mut other = VectorcraftApp::new(Session::new(), Default::default());
    other.canvas.worker_started = true; // render on this thread
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 70, "height": 40})).unwrap();
    src.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 70, "height": 40})).unwrap();
    let dir = std::env::temp_dir().join(format!("vectorcraft-placed-pan-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("badge.vectorcraft");
    std::fs::write(&path, vectorcraft_format::save_file(&src.active().unwrap().doc)).unwrap();
    other.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    other.session.execute("file.place", &json!({"path": path.to_string_lossy(), "at": [100, 100]})).unwrap();
    let ctx = crate::testing::ui_context();
    let frame = |app: &mut VectorcraftApp| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    frame(&mut app); // the first frame fits the view to the artboard
    frame(&mut app);
    app.canvas.last_ms = 1.0;
    app.view_mut().unwrap().center.x += 10.0;
    frame(&mut app);
    let shown = app.canvas.key.clone().unwrap();
    // The other canvas asks for the bitmap, and it is made: the generation changes before the
    // bitmap stops being pending.
    let generation = vectorcraft_render::placed_document::generation();
    shapes(&mut other, &crate::testing::ui_context());
    while vectorcraft_render::placed_document::busy() {
        std::thread::yield_now();
    }
    assert_ne!(vectorcraft_render::placed_document::generation(), generation, "a bitmap was made");
    app.canvas.last_ms = 50.0;
    app.view_mut().unwrap().center.x += 10.0;
    frame(&mut app);
    assert_eq!(app.canvas.key.as_ref(), Some(&shown), "a heavy canvas is not rendered while the view moves");
    app.canvas.last_ms = 1.0;
    app.view_mut().unwrap().center.x += 10.0;
    frame(&mut app);
    let next = app.canvas.key.clone().unwrap();
    assert_eq!(next.cx, app.view().unwrap().center.x, "a light canvas renders at once");
    assert!(super::cursor::only_view_differs(&shown, &next), "only the view differs: {shown:?} {next:?}");
    std::fs::remove_dir_all(&dir).ok();
}

/// A document's [`CacheKey::placed`] moves to the current placed-document bitmap generation only
/// when a bitmap of a file it shows in a placed document was made, or when the files made are no
/// longer known and it shows a placed document. A bitmap of another document's file, or of a file
/// it keeps only for Undo, leaves it, so a pan of a heavy
/// document keeps waiting for the view to stop. Each change keeps the value before it and the
/// last generation that value was checked at. A closed document's value is dropped.
#[test]
fn a_documents_placed_key_moves_only_for_bitmaps_of_its_own_files() {
    let mut session = Session::new();
    session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    let plain = session.active().unwrap().uid;
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 60, "height": 30})).unwrap();
    src.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 60, "height": 30})).unwrap();
    let dir = std::env::temp_dir().join(format!("vectorcraft-placed-key-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("card.vectorcraft");
    std::fs::write(&path, vectorcraft_format::save_file(&src.active().unwrap().doc)).unwrap();
    session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = session.execute("file.place", &json!({"path": path.to_string_lossy(), "at": [60, 50]})).unwrap();
    let st = session.active().unwrap();
    let placing = st.uid;
    let file = match &st.doc.node(vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap())).unwrap().kind {
        vectorcraft_doc::NodeKind::PlacedDocument(p) => p.key.clone(),
        other => panic!("a placed document: {other:?}"),
    };
    type Made<'a> = &'a dyn Fn(u64, u64) -> Option<std::collections::HashSet<String>>;
    fn key(keys: &mut std::collections::HashMap<u64, crate::PlacedKey>, session: &Session, uid: u64, now: u64, made: Made<'_>) -> u64 {
        let st = session.documents().iter().find(|d| d.uid == uid).unwrap();
        placed_key_at(keys, session, st, now, made)
    }
    let files = |names: &[&str]| Some(names.iter().map(|n| n.to_string()).collect::<std::collections::HashSet<_>>());
    let unasked = |_: u64, _: u64| -> Option<std::collections::HashSet<String>> { panic!("no bitmaps to look up") };
    let mut keys = std::collections::HashMap::new();
    assert_eq!(key(&mut keys, &session, placing, 5, &unasked), 5, "checked for the first time");
    assert_eq!(key(&mut keys, &session, placing, 5, &unasked), 5, "no bitmap made since");
    let other = |from: u64, to: u64| {
        assert_eq!((from, to), (5, 6), "the bitmaps made since the last check");
        files(&["another document's file"])
    };
    assert_eq!(key(&mut keys, &session, placing, 6, &other), 5, "another document's file");
    assert_eq!(key(&mut keys, &session, placing, 7, &|_, _| files(&["another document's file", file.as_str()])), 7, "its own file");
    assert_eq!(keys[&placing], crate::PlacedKey { checked: 7, key: 7, before: (5, 6) });
    assert_eq!(key(&mut keys, &session, placing, 8, &|_, _| None), 8, "files no longer known");
    assert_eq!(keys[&placing].before, (7, 7));
    assert_eq!(key(&mut keys, &session, plain, 8, &unasked), 8, "the other document, checked for the first time");
    assert_eq!(key(&mut keys, &session, plain, 9, &|_, _| None), 8, "no images: files no longer known");
    assert_eq!(key(&mut keys, &session, plain, 10, &|_, _| files(&[file.as_str()])), 8, "no images: a file another document holds");
    // The same document holding an image and showing no placed document.
    let mut raster = session.documents().iter().find(|d| d.uid == plain).unwrap().clone();
    let blob = vectorcraft_doc::ImageBlob { mime: "image/png".into(), bytes: Default::default(), proxy: None, file_font: Default::default() };
    std::sync::Arc::make_mut(&mut raster.doc).images.insert("an image".into(), blob);
    assert_eq!(placed_key_at(&mut keys, &session, &raster, 11, |_, _| None), 8, "an image and no placed document: files no longer known");
    assert_eq!(keys.len(), 2);
    let index = session.documents().iter().position(|d| d.uid == plain).unwrap();
    session.execute("file.close", &json!({"index": index})).unwrap();
    assert_eq!(key(&mut keys, &session, placing, 12, &|_, _| files(&[])), 8, "no bitmaps of its files");
    assert_eq!(keys.keys().copied().collect::<Vec<_>>(), [placing], "the closed document's value is dropped");
    // The placed document deleted: its file stays in the images for Undo, and no longer shows.
    assert_eq!(session.active().unwrap().uid, placing);
    session.execute("edit.clear", &json!({"ids": [r["ids"][0]]})).unwrap();
    assert!(session.active().unwrap().doc.images.contains_key(&file), "kept for Undo");
    assert_eq!(key(&mut keys, &session, placing, 13, &|_, _| files(&[file.as_str()])), 8, "a file kept only for Undo");
    std::fs::remove_dir_all(&dir).ok();
}

/// The canvas key takes the document's value from [`placed_key`], which a bitmap made for a test
/// running beside this one does not move: this document holds no images.
#[test]
fn the_canvas_key_takes_the_documents_placed_value() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let uid = app.session.active().unwrap().uid;
    let generation = vectorcraft_render::placed_document::generation();
    let marked = generation.wrapping_add(1000);
    app.canvas.placed_keys.insert(uid, crate::PlacedKey { checked: generation, key: marked, before: (marked, generation) });
    shapes(&mut app, &crate::testing::ui_context());
    assert_eq!(app.canvas.key.as_ref().map(|k| k.placed), Some(marked));
}

/// A frame shown at one [`CacheKey::placed`] value and drawn again at the next looks at the
/// bitmaps made since the last check before the change: those made earlier were of other
/// documents' files, and past 256 bitmaps they are no longer known ([`placed_since`]).
#[test]
fn a_frame_drawn_again_after_one_change_of_its_placed_value_looks_at_the_bitmaps_since_the_last_check() {
    let entry = crate::PlacedKey { checked: 400, key: 400, before: (5, 399) };
    assert_eq!(placed_since(Some(&entry), 5, 400), 399, "the value changed once since the frame shown");
    assert_eq!(placed_since(Some(&entry), 3, 400), 3, "it changed more than once");
    assert_eq!(placed_since(Some(&entry), 5, 401), 5, "a value the document no longer has");
    assert_eq!(placed_since(None, 5, 400), 5, "a document not checked");
}

/// A frame drawn again at a new [`CacheKey::placed`] value draws again where the document shows
/// the files whose bitmaps were made after [`placed_since`]. After one change of the value, the
/// bitmaps made before the last check of the old value are not looked up: past 256 bitmaps they
/// are no longer known, and the whole frame would render. At the same value nothing is looked up.
#[test]
fn a_frame_drawn_again_at_a_new_placed_value_draws_where_the_bitmaps_since_the_last_check_show() {
    let mut session = Session::new();
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 60, "height": 30})).unwrap();
    src.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 60, "height": 30})).unwrap();
    let dir = std::env::temp_dir().join(format!("vectorcraft-placed-changes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("card.vectorcraft");
    std::fs::write(&path, vectorcraft_format::save_file(&src.active().unwrap().doc)).unwrap();
    session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = session.execute("file.place", &json!({"path": path.to_string_lossy(), "at": [60, 50]})).unwrap();
    session.execute("shape.rectangle", &json!({"x": 250, "y": 200, "width": 40, "height": 30})).unwrap();
    let doc = &session.active().unwrap().doc;
    let placed = doc.node(vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap())).unwrap();
    let file = match &placed.kind {
        vectorcraft_doc::NodeKind::PlacedDocument(p) => p.key.clone(),
        other => panic!("a placed document: {other:?}"),
    };
    let bounds = placed.geometric_bounds().unwrap();
    let entry = crate::PlacedKey { checked: 400, key: 400, before: (5, 399) };
    // Only the bitmaps made after generation 399 are known: one of the placed file.
    let made = |from: u64, to: u64| (from >= 399 && to == 400).then(|| std::collections::HashSet::from([file.clone()]));
    let unasked = |_: u64, _: u64| -> Option<std::collections::HashSet<String>> { panic!("no bitmaps to look up") };
    assert_eq!(placed_changes(Some(&entry), 400, 400, doc, unasked), Some(vectorcraft_render::Changed::Nothing), "the same value");
    match placed_changes(Some(&entry), 5, 400, doc, made) {
        Some(vectorcraft_render::Changed::Area(a)) => {
            assert!(a.x0 <= bounds.x0 && a.y0 <= bounds.y0 && a.x1 >= bounds.x1 && a.y1 >= bounds.y1, "{a:?} holds {bounds:?}");
            assert!(a.x1 < 250.0, "{a:?}: not the rectangle");
        }
        other => panic!("the placed document's part: {other:?}"),
    }
    assert_eq!(placed_changes(Some(&entry), 3, 400, doc, made), None, "the value changed more than once: no longer known");
    let none = |_: u64, _: u64| Some(std::collections::HashSet::new());
    assert_eq!(placed_changes(Some(&entry), 5, 400, doc, none), Some(vectorcraft_render::Changed::Nothing), "no bitmaps since");
    std::fs::remove_dir_all(&dir).ok();
}

/// File Handling › Display Bitmaps as Anti-aliased Images in Pixel Preview (#394): off, Pixel
/// Preview draws images with each pixel taking its nearest image pixel; on, smoothly, as the
/// canvas always does out of Pixel Preview.
#[test]
fn pixel_preview_smooths_bitmaps_only_when_asked() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let ctx = crate::testing::ui_context();
    let smooth = |app: &mut VectorcraftApp| {
        let (_, mut delta) = frame_output(app, &ctx);
        delta.clear();
        app.canvas.key.as_ref().map(|k| k.smooth_images).unwrap()
    };
    assert!(smooth(&mut app), "out of Pixel Preview");
    app.ui.view.pixel_preview = true;
    app.view_mut().unwrap().zoom = 8.0;
    assert!(!smooth(&mut app), "Pixel Preview, off by default");
    app.run("prefs.set", json!({"key": "antiAliasedBitmaps", "value": true})).unwrap();
    assert!(smooth(&mut app), "Pixel Preview, on");
}

/// Appearance of Black › On Screen (#394): Display All Blacks Accurately (the default) shows
/// pure CMYK black through the CMYK profile, a dark grey; Display All Blacks as Rich Black
/// shows it as RGB black. A rich black shows the same either way.
#[test]
fn appearance_of_black_on_screen_shows_pure_black_as_rgb_black() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300, "colorMode": "cmyk"})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    // Larger than the view: the middle of the art's raster is its fill.
    let id = app.run("shape.rectangle", json!({"x": -5000, "y": -5000, "width": 10000, "height": 10000})).unwrap()["id"].clone();
    let fill = |app: &mut VectorcraftApp, cmyk: [u8; 4]| {
        let [c, m, y, k] = cmyk;
        app.run("paint.setFill", json!({"ids": [id], "color": {"c": c, "m": m, "y": y, "k": k}})).unwrap();
    };
    let ctx = crate::testing::ui_context();
    // The middle pixel of the art's raster uploaded by one frame, None when it was not re-rendered.
    let middle = |app: &mut VectorcraftApp| -> Option<[u8; 4]> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let px = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            let [w, h] = img.size;
            img.pixels[h / 2 * w + w / 2].to_array()
        });
        delta.clear();
        px
    };
    let shown = |cmyk: [f32; 4]| vectorcraft_color::Color::cmyk(cmyk[0], cmyk[1], cmyk[2], cmyk[3]).to_rgba8(1.0);
    let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 1);

    fill(&mut app, [0, 0, 0, 100]);
    let accurate = middle(&mut app).expect("the first frame renders the art");
    assert!(near(accurate, shown([0.0, 0.0, 0.0, 1.0])), "accurate: through the CMYK profile, {accurate:?}");
    assert!(accurate[..3].iter().all(|v| *v > 10), "accurate: a dark grey, {accurate:?}");
    app.run("prefs.set", json!({"key": "blackOnScreen", "value": "rich"})).unwrap();
    assert_eq!(middle(&mut app).expect("the preference change re-renders"), [0, 0, 0, 255], "rich: RGB black");

    fill(&mut app, [60, 40, 40, 100]);
    let rich = middle(&mut app).expect("re-rendered");
    assert!(near(rich, shown([0.6, 0.4, 0.4, 1.0])), "a rich black is not pure black: {rich:?}");

    fill(&mut app, [0, 0, 0, 100]);
    app.run("prefs.set", json!({"key": "blackOnScreen", "value": "accurate"})).unwrap();
    assert!(near(middle(&mut app).expect("re-rendered"), accurate), "accurate again");
}

/// A change of the color settings, and a profile in use loaded again under its name, re-render the
/// canvas through the new conversion. They raise the documents' revisions but leave the art as it
/// is, so [`super::raster::changed_pixels`] finds no part of the frame shown that changed: the
/// frame's key holds the color settings and the loaded profiles' generation. A frame with nothing
/// changed renders nothing.
#[test]
fn color_settings_and_a_reloaded_profile_render_the_canvas_again() {
    const CHILD: &str = "VECTORCRAFT_TEST_CANVAS_COLOR_SETTINGS";
    if std::env::var_os(CHILD).is_none() {
        // Color settings and loaded profiles are process-wide: change them in a separate test process.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "canvas::tests::color_settings_and_a_reloaded_profile_render_the_canvas_again", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
            "isolated canvas color test failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    use vectorcraft_color::cms;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300, "colorMode": "cmyk"})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    // Larger than the view: the middle of the art's raster is its fill.
    let id = app.run("shape.rectangle", json!({"x": -5000, "y": -5000, "width": 10000, "height": 10000})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [id], "color": {"c": 60, "m": 10, "y": 80, "k": 0}})).unwrap();
    let ctx = crate::testing::ui_context();
    // The middle pixel of the art's raster uploaded by one frame, None when it was not re-rendered.
    let middle = |app: &mut VectorcraftApp| -> Option<[u8; 4]> {
        let (_, mut delta) = frame_output(app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let px = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            let [w, h] = img.size;
            img.pixels[h / 2 * w + w / 2].to_array()
        });
        delta.clear();
        px
    };
    let ink = vectorcraft_color::Color::cmyk(0.6, 0.1, 0.8, 0.0);
    let through = |cmyk: &str| {
        let c = cms::Cms::new(&cms::ColorSettings { cmyk: cmyk.into(), ..cms::working_settings() }).unwrap().display_rgb(&ink);
        c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
    };
    let near = |a: [u8; 4], b: [u8; 3]| a[..3].iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3);
    let (generic, device) = (through(cms::GENERIC_CMYK), through(cms::DEVICE_CMYK));
    assert!(!near([generic[0], generic[1], generic[2], 255], device), "the two CMYK spaces draw the ink differently: {generic:?}, {device:?}");

    let first = middle(&mut app).expect("the first frame renders the art");
    assert!(near(first, generic), "through the working CMYK space: {first:?}, not {generic:?}");
    assert_eq!(middle(&mut app), None, "nothing changed: no re-render");
    app.run("edit.colorSettings", json!({"cmyk": cms::DEVICE_CMYK})).unwrap();
    let shown = middle(&mut app).expect("Color Settings re-render the canvas");
    assert!(near(shown, device), "through the new working CMYK space: {shown:?}, not {device:?}");
    assert_eq!(middle(&mut app), None, "nothing changed since: no re-render");
    app.run("edit.colorSettings", json!({"cmyk": cms::GENERIC_CMYK})).unwrap();
    assert!(near(middle(&mut app).expect("re-rendered"), generic), "back to the first working space");

    // A built-in CMYK profile's file renamed `name`: padded with spaces, which names drop.
    let renamed = |builtin: &str, name: &str| {
        let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
        let mut bytes = cms::icc_bytes(builtin).unwrap().to_vec();
        let (from, to) = (utf16(builtin), utf16(&format!("{name:<width$}", width = builtin.len())));
        let at = bytes.windows(from.len()).position(|w| w == from).unwrap();
        bytes[at..at + to.len()].copy_from_slice(&to);
        bytes
    };
    let name = "Canvas Reload CMYK";
    let dir = vectorcraft_testkit::temp_dir("canvas-reload-cmyk");
    let load = |app: &mut VectorcraftApp, builtin: &str| {
        let path = dir.join(format!("{}.icc", builtin.len()));
        std::fs::write(&path, renamed(builtin, name)).unwrap();
        let r = app.run("color.loadProfile", json!({"path": path.to_str().unwrap()})).unwrap();
        assert_eq!(r["name"], name);
    };
    load(&mut app, cms::GENERIC_CMYK);
    app.run("edit.assignProfile", json!({"cmyk": name})).unwrap();
    let assigned = middle(&mut app).expect("assigning a profile re-renders the canvas");
    assert!(near(assigned, generic), "through the assigned copy of the working space: {assigned:?}, not {generic:?}");
    assert_eq!(middle(&mut app), None, "nothing changed: no re-render");
    // The same name, the settings as they were: only the profile's file is new.
    load(&mut app, cms::DEVICE_CMYK);
    let reloaded = middle(&mut app).expect("the profile loaded again re-renders the canvas");
    assert!(near(reloaded, device), "through the profile's new file: {reloaded:?}, not {device:?}");
    assert_eq!(middle(&mut app), None, "nothing changed since: no re-render");

    // A proof of the working CMYK space converts through the working settings. With the document's
    // assigned profile in place of the working CMYK space, a new working CMYK space leaves the
    // active settings as they were and changes only the proof.
    app.run("view.proofSetup", json!({"target": "workingCmyk"})).unwrap();
    app.run("view.proofColors", json!({"on": true})).unwrap();
    let proofed = middle(&mut app).expect("Proof Colors re-renders the canvas");
    assert_eq!(middle(&mut app), None, "nothing changed: no re-render");
    let active = cms::active_settings();
    app.run("edit.colorSettings", json!({"cmyk": cms::DEVICE_CMYK})).unwrap();
    let reproofed = middle(&mut app).expect("a new working CMYK space re-renders the proof");
    assert_eq!(cms::active_settings(), active, "the assigned profile keeps the active settings");
    assert!(!near(reproofed, [proofed[0], proofed[1], proofed[2]]), "the proof through the new working CMYK space: {reproofed:?}, not {proofed:?}");
    assert_eq!(middle(&mut app), None, "nothing changed since: no re-render");
}

/// A frame from the render worker whose color key is not the current one is not shown, and the
/// canvas asks for the frame again. The worker converts colors with the settings in force while it
/// renders, so a frame asked for before a change of the color settings may hold colors of the new
/// settings under the old key, and a return to the old settings would keep it.
#[test]
fn a_worker_frame_of_other_color_settings_is_not_shown() {
    keeping_color_settings(a_worker_frame_of_other_color_settings);
}

fn a_worker_frame_of_other_color_settings() {
    use vectorcraft_color::cms;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true;
    let (worker, jobs, done_tx) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    let ctx = crate::testing::ui_context();
    let frame = |app: &mut VectorcraftApp| {
        let raw =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time: Some(1.0), ..Default::default() };
        ctx.run_ui(raw, |ui| show(app, ui)).textures_delta.clear();
    };
    frame(&mut app);
    done_tx.send(finished(&jobs.try_recv().expect("the first frame"))).unwrap();
    frame(&mut app);
    let first = app.canvas.key.clone().expect("the first frame is shown");

    // A pan's frame goes to the worker, and the color settings change before it is finished.
    app.view_mut().unwrap().center.x += 5.0;
    frame(&mut app);
    let pan = jobs.try_recv().expect("the pan's frame goes to the worker");
    let settings = cms::ColorSettings { cmyk: cms::DEVICE_CMYK.into(), ..cms::active_settings() };
    assert_ne!(settings, cms::active_settings(), "other color settings");
    cms::with_active(std::sync::Arc::new(cms::Cms::new(&settings).unwrap()), || {
        done_tx.send(finished(&pan)).unwrap();
        frame(&mut app);
        assert_eq!(app.canvas.key.as_ref(), Some(&first), "the pan's frame, of other color settings, is not shown");
        let again = jobs.try_recv().expect("the pan's frame is asked for with the current color settings");
        assert_ne!(again.key.color, pan.key.color);
    });
    // Back to the settings in the pan's key: the pan's frame is asked for again.
    frame(&mut app);
    let back = jobs.try_recv().expect("the pan's frame is asked for again");
    assert_eq!(back.key, pan.key);
}

#[test]
fn top_bounding_box_handle_scales_a_default_mirror_repeat() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    app.run("object.repeat.mirror", json!({})).unwrap();
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);

    let initial = app.selection_bounds().unwrap();
    let bx = app.selection_box().unwrap();
    let top = bx.to_doc() * vectorcraft_tools::bbox::Handle::Top.pos(bx.rect);
    let xf = app.canvas_view().unwrap();
    let from = xf.to_screen(top);
    let to = from + vec2(0.0, -30.0);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };

    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from), button(from, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(to)]);
    frame(&mut app, &ctx, vec![button(to, false)]);

    let scaled = app.selection_bounds().unwrap();
    assert!(scaled.height() > initial.height() + 10.0, "Top handle should scale the repeat: {initial:?} -> {scaled:?}");
    assert!((scaled.width() - initial.width()).abs() < 0.01, "Top handle should not scale width: {initial:?} -> {scaled:?}");
}

/// #3219: a selected path made into a guide is deselected when guides are hidden, so the
/// selection overlay cannot reveal the hidden path.
#[test]
fn hide_guides_deselects_selected_guide_paths() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("view.guides.make", json!({})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(id)]);

    app.run("view.guides", json!({})).unwrap();

    assert!(app.session.active().unwrap().selection.objects.is_empty());
}

/// View › Pixel Preview turns Smart Guides off in the tools: a square moved to 3 pt below another
/// square's top lines up with it only while Pixel Preview is off.
#[test]
fn pixel_preview_stops_smart_guides_snapping_a_move() {
    for pixel in [false, true] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 400, "y": 300, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        if pixel {
            app.run("view.pixelPreview", json!({})).unwrap();
        }
        assert!(app.ui.view.smart_guides);
        assert_eq!(app.ui.view.pixel_preview, pixel);
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.select_tool("selection");
        let view = app.view_info();
        let mut at = |kind: PointerKind, y: f64| crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, 425.0, y), view);
        at(PointerKind::Down, 325.0);
        for y in [300.0, 250.0, 200.0, 150.0, 128.0] {
            at(PointerKind::Drag, y);
        }
        at(PointerKind::Up, 128.0);
        let top = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().y0;
        assert_eq!(top, if pixel { 103.0 } else { 100.0 }, "Pixel Preview {pixel}");
    }
}

/// View › Hide Edges hides a live rectangle's corner widgets, and the Selection tool then
/// neither shows the corner cursor over one's place nor rounds the corners from it. With
/// edges shown, a drag from the same widget rounds them.
#[test]
fn hidden_corner_widgets_do_not_answer_the_pointer() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let id = vectorcraft_doc::NodeId(id);
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("selection");
    let widget = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        vectorcraft_tools::corners::CornerWidgets::of(&st.doc, &st.selection, app.view_info().zoom, false).unwrap().visible().next().unwrap()
    };
    let radius = |app: &VectorcraftApp| {
        let c = vectorcraft_doc::LiveCorners::of(app.session.active().unwrap().doc.node(id).unwrap()).unwrap();
        c.style(&c.all()).0
    };
    let drag = |app: &mut VectorcraftApp, from: Point| {
        let (view, to) = (app.view_info(), from + vectorcraft_geom::Vec2::new(10.0, 10.0));
        for (kind, at) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
            app.session.pointer(&PointerEvent::new(kind, at.x, at.y), view).unwrap();
        }
    };
    app.ui.view.edges = false;
    let at = widget(&app);
    let view = app.view_info();
    assert_ne!(app.session.cursor(at, Mods::default(), view), Cursor::CornerRadius);
    drag(&mut app, at);
    assert_eq!(radius(&app), Some(0.0), "a press on a hidden widget does not round the corners");
    app.ui.view.edges = true;
    let at = widget(&app);
    let view = app.view_info();
    assert_eq!(app.session.cursor(at, Mods::default(), view), Cursor::CornerRadius);
    drag(&mut app, at);
    assert!(radius(&app).is_some_and(|r| r > 0.0), "{:?}", radius(&app));
}

/// View › Hide Edges hides the bounding box and the type widget beside it, and the Selection
/// tool then answers neither: a double-click on the widget's place leaves point type as it is,
/// and a drag from a corner handle's place does not scale. With edges shown, both answer.
#[test]
fn a_hidden_bounding_box_and_type_widget_do_not_answer_the_pointer() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type"})).unwrap()["id"].as_u64().unwrap();
    let id = vectorcraft_doc::NodeId(id);
    app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("selection");
    let bx = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        st.doc.oriented_bounds(&st.selection.objects, app.session.prefs.use_preview_bounds).unwrap()
    };
    let widget = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        vectorcraft_tools::typewidget::TypeWidget::of(&st.doc, &st.selection, &bx(app), app.view_info().zoom).unwrap()
    };
    let area = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => matches!(t.kind, vectorcraft_doc::TextKind::Area { .. }),
        k => panic!("not type: {k:?}"),
    };
    let double_click = |app: &mut VectorcraftApp, at: Point| {
        let view = app.view_info();
        for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
            app.session.pointer(&PointerEvent::new(kind, at.x, at.y), view).unwrap();
        }
    };
    let width = |app: &VectorcraftApp| bx(app).rect.width();
    let scale = |app: &mut VectorcraftApp| {
        let b = bx(app);
        let corner = b.to_doc() * Point::new(b.rect.x1, b.rect.y1);
        let (view, to) = (app.view_info(), corner + vectorcraft_geom::Vec2::new(30.0, 0.0));
        for (kind, at) in [(PointerKind::Down, corner), (PointerKind::Drag, to), (PointerKind::Up, to)] {
            app.session.pointer(&PointerEvent::new(kind, at.x, at.y), view).unwrap();
        }
        app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    };
    let w = widget(&app);
    app.ui.view.edges = false;
    let view = app.view_info();
    assert_ne!(app.session.cursor(w.at, Mods::default(), view), Cursor::TypeWidget);
    double_click(&mut app, w.at);
    assert!(!area(&app), "a double-click on the hidden widget leaves point type");
    app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let before = width(&app);
    scale(&mut app);
    assert!((width(&app) - before).abs() < 1e-6, "a drag from a hidden handle does not scale: {before} -> {}", width(&app));
    app.ui.view.edges = true;
    let w = widget(&app);
    let view = app.view_info();
    assert_eq!(app.session.cursor(w.at, Mods::default(), view), Cursor::TypeWidget);
    scale(&mut app);
    assert!(width(&app) > before + 1.0, "a drag from the handle scales: {before} -> {}", width(&app));
    let w = widget(&app);
    double_click(&mut app, w.at);
    assert!(area(&app), "a double-click on the widget converts to area type");
}

/// #3797: View › Outline is a setting of each document's view. A document opened or made after
/// another was put in Outline opens in Preview, where a click inside its fill picks it, and each
/// document keeps its own setting.
#[test]
fn outline_is_per_document_and_new_documents_start_in_preview() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 100, "height": 100})).unwrap();
    app.sync_views();
    let inside = Point::new(70.0, 70.0);
    let picks_fill = |app: &VectorcraftApp| super::outline::hit_at(app, inside, 1.0).is_some();
    assert!(picks_fill(&app), "Preview picks the fill");
    app.run("view.outline", json!({})).unwrap();
    assert!(app.outline_view() && app.view_info().outline);
    assert!(!picks_fill(&app), "Outline picks only the path");
    // An SVG opened now opens in Preview, its fill drawn and picked.
    let svg =
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200"><rect x="20" y="20" width="100" height="100" fill="#ff0000"/></svg>"##;
    crate::io::open_bytes(&mut app, "b.svg", svg, None).unwrap();
    assert_eq!(app.session.active_index(), Some(1));
    assert!(!app.outline_view() && !app.view_info().outline, "the opened document is in Preview");
    assert!(picks_fill(&app));
    assert_eq!(crate::menus::dynamic_label(&app, "view.outline", "Outline"), "Outline");
    assert_eq!(crate::menus::checked(&app, "view.outline", &json!({})), Some(false));
    // A new document opens in Preview too.
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    assert!(!app.outline_view());
    // The first document is still in Outline.
    app.session.execute("document.activate", &json!({"index": 0})).unwrap();
    assert!(app.outline_view() && !picks_fill(&app));
    assert_eq!(crate::menus::dynamic_label(&app, "view.outline", "Outline"), "Preview");
    assert_eq!(crate::menus::checked(&app, "view.outline", &json!({})), Some(true));
    // Without a document there is nothing to put in Outline.
    while !app.session.documents().is_empty() {
        app.session.execute("file.close", &json!({})).unwrap();
    }
    app.sync_views();
    assert!(!app.outline_view() && !crate::menus::enabled(&app, "view.outline"));
    assert!(app.run("view.outline", json!({})).is_err());
}

/// #3797: a preferences file saved while View › Outline was an interface setting still loads, with
/// its other view settings, and its Outline value does not put a document in Outline.
#[test]
fn old_outline_preference_loads_but_does_not_restore_outline() {
    let mut saved = serde_json::to_value(crate::UiState::default()).unwrap();
    assert!(saved["view"].get("outline").is_none(), "Outline is not saved with the interface");
    saved["view"]["outline"] = json!(true);
    saved["view"]["rulers"] = json!(true);
    let ui = serde_json::from_value::<crate::UiState>(saved).unwrap().sanitized();
    assert!(ui.view.rulers, "the other view settings load");
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.ui = ui;
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    assert!(!app.outline_view(), "a new document is in Preview");
}

/// The kinds of `marks` other than spaces and line ends, in order.
fn char_marks(marks: &[PlacedMark]) -> Vec<HiddenMark> {
    use HiddenMark::{LineBreak, ParagraphEnd, Space, StoryEnd};
    marks.iter().map(|m| m.mark).filter(|k| !matches!(k, Space | LineBreak | ParagraphEnd | StoryEnd)).collect()
}

/// The marks of `marks` of kind `kind`.
fn marks_of_kind(marks: &[PlacedMark], kind: HiddenMark) -> Vec<PlacedMark> {
    marks.iter().filter(|m| m.mark == kind).copied().collect()
}

/// The paragraph and story ends of `marks`, in order.
fn line_ends(marks: &[PlacedMark]) -> Vec<PlacedMark> {
    marks.iter().filter(|m| matches!(m.mark, HiddenMark::ParagraphEnd | HiddenMark::StoryEnd)).copied().collect()
}

/// Show Hidden Characters marks each tab, nonbreaking space, narrow nonbreaking space, space of
/// a set width and discretionary hyphen with a mark of its kind, a plain space with a dot, and
/// a tab's mark starts where the tab does.
#[test]
fn hidden_characters_mark_tabs_nonbreaking_spaces_and_discretionary_hyphens() {
    use HiddenMark::*;
    let text = "a b\tc\u{00A0}d\u{202F}e\u{2003}f\u{2009}g\u{200A}h\u{00AD}i";
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(10.0, 50.0), text, vectorcraft_doc::CharStyle::default());
    let marks = HiddenMarks::of(&t);
    assert_eq!(char_marks(&marks), [Tab, NoBreakSpace, NarrowNoBreakSpace, FixedSpace, FixedSpace, FixedSpace, DiscretionaryHyphen]);
    assert_eq!(marks_of_kind(&marks, Space).len(), 1, "only the plain space has a dot");
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    let tab = lay.glyphs.iter().find(|g| text[g.byte..].starts_with('\t')).unwrap();
    let [mark] = marks_of_kind(&marks, Tab)[..] else { panic!("one tab mark: {marks:?}") };
    assert!((mark.at - t.xf * tab.origin).hypot() < 1e-9 && mark.anchor == MarkAnchor::Start, "{mark:?} at {:?}", tab.origin);
}

/// Show Hidden Characters marks a line that a forced line break (U+2028) ends with its own mark
/// and no ¶; the line a paragraph return ends gets ¶, and the story's end #.
#[test]
fn hidden_characters_mark_forced_line_breaks() {
    use HiddenMark::*;
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "ab\u{2028}cd\nef", vectorcraft_doc::CharStyle::default());
    let marks = HiddenMarks::of(&t);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    assert_eq!(lay.lines.len(), 3);
    let [brk] = marks_of_kind(&marks, LineBreak)[..] else { panic!("one line break mark: {marks:?}") };
    let first = &lay.lines[0];
    assert_eq!(brk.at, t.xf * vectorcraft_geom::Point::new(first.x1 + 1.0, first.baseline));
    let ends: Vec<_> = line_ends(&marks).iter().map(|e| (e.mark, e.at.y)).collect();
    assert_eq!(ends, [(ParagraphEnd, lay.lines[1].baseline), (StoryEnd, lay.lines[2].baseline)]);
    // A story that ends with a forced line break: its mark, and # on the empty line after it.
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "ab\u{2028}", vectorcraft_doc::CharStyle::default());
    let marks = HiddenMarks::of(&t);
    let ends: Vec<_> = line_ends(&marks).iter().map(|e| e.mark).collect();
    assert_eq!((marks_of_kind(&marks, LineBreak).len(), ends), (1, vec![StoryEnd]));
}

/// A discretionary hyphen gets one mark also where the line breaks at it and the layout draws a
/// hyphen there, which stands for no character of the text.
#[test]
fn hidden_characters_mark_a_discretionary_hyphen_at_a_break_once() {
    let style = vectorcraft_doc::CharStyle::default();
    let db = vectorcraft_text::FontDb::global();
    let word = vectorcraft_text::layout(db, &vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "coco-", style.clone()));
    let width = word.lines[0].x1 - word.lines[0].x0 + 2.0;
    let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "coco\u{00AD}co", style);
    t.kind = vectorcraft_doc::text::TextKind::Area {
        frame: vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::Rect::new(0.0, 0.0, width, 400.0).to_path(0.1)),
    };
    let lay = vectorcraft_text::layout(db, &t);
    assert_eq!(lay.lines.len(), 2, "the line breaks at the discretionary hyphen");
    assert!(lay.glyphs.iter().any(|g| g.len == 0), "the layout draws a hyphen at the break");
    assert_eq!(char_marks(&HiddenMarks::of(&t)), [HiddenMark::DiscretionaryHyphen]);
}

/// In vertical type the marks follow the column: a space's dot sits halfway down the space, and ¶
/// hangs below the column's last character.
#[test]
fn hidden_characters_follow_vertical_type() {
    let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "a b\nc", vectorcraft_doc::CharStyle::default());
    t.vertical = true;
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    let space = lay.glyphs.iter().find(|g| g.byte == 1).unwrap();
    let marks = HiddenMarks::of(&t);
    let [dot] = marks_of_kind(&marks, HiddenMark::Space)[..] else { panic!("one dot: {marks:?}") };
    let middle = t.xf * vectorcraft_geom::Point::new(space.origin.x, space.origin.y + space.advance / 2.0);
    assert!((dot.at.y - middle.y).abs() < 1e-9, "{dot:?} {space:?}");
    let end = line_ends(&marks)[0];
    assert_eq!((end.mark, end.anchor), (HiddenMark::ParagraphEnd, MarkAnchor::Top));
    let b = lay.glyphs.iter().find(|g| g.byte == 2).unwrap();
    assert!(end.at.y > (t.xf * b.origin).y, "{end:?} below {:?}", b.origin);
}

/// In a right-to-left paragraph ¶ goes at the line's left end, where the paragraph ends.
#[test]
fn hidden_characters_end_a_right_to_left_line_at_its_left() {
    let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "אב גד\nהו", vectorcraft_doc::CharStyle::default());
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    assert!(lay.lines[0].rtl);
    let end = line_ends(&HiddenMarks::of(&t))[0];
    assert_eq!((end.mark, end.anchor), (HiddenMark::ParagraphEnd, MarkAnchor::End));
    assert_eq!(end.at.x, (t.xf * vectorcraft_geom::Point::new(lay.lines[0].x0 - 1.0, lay.lines[0].baseline)).x);
}

/// Type on a path has the marks over its characters and # after its last one.
#[test]
fn hidden_characters_mark_type_on_a_path() {
    let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "a b\tc", vectorcraft_doc::CharStyle::default());
    let path = vectorcraft_geom::Line::new((0.0, 100.0), (600.0, 100.0)).to_path(0.1);
    t.kind = vectorcraft_doc::text::TextKind::OnPath { path: vectorcraft_geom::PathData::from_bezpath(&path), start: 0.0, end: None };
    let marks = HiddenMarks::of(&t);
    assert_eq!(marks_of_kind(&marks, HiddenMark::Space).len(), 1);
    assert_eq!(char_marks(&marks), [HiddenMark::Tab]);
    assert_eq!(line_ends(&marks).iter().map(|e| e.mark).collect::<Vec<_>>(), [HiddenMark::StoryEnd]);
}

/// Show Hidden Characters puts a dot over each space of a list paragraph and none over its
/// bullet, which stands for no character.
#[test]
fn hidden_characters_mark_no_space_at_a_bullet() {
    let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, " Item two", vectorcraft_doc::CharStyle::default());
    let dots = |t: &vectorcraft_doc::TextObject| marks_of_kind(&HiddenMarks::of(t), HiddenMark::Space).len();
    let plain = dots(&t);
    t.para.list = Some(vectorcraft_doc::text::ParaList::bulleted(vectorcraft_doc::text::Bullet::Dot));
    assert_eq!((plain, dots(&t)), (2, 2));
}

/// General › Display Print Size at 100% Zoom (#394, #9051): Actual Size shows the document larger
/// with it on (an inch at 96 screen points where no display was measured) than off (a point at a
/// point), and reads 100% either way.
#[test]
fn view_actual_size_follows_display_print_size() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    app.session.execute("prefs.set", &json!({"key": "displayPrintSize", "value": false})).unwrap();
    let ctx = crate::testing::ui_context();
    // Frames as the hosts run them: the zoom stops come from the preferences through the app's frame input.
    let frame = |app: &mut VectorcraftApp, ctx: &egui::Context, _: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        crate::testing::part_frame(app, ctx, raw, show);
    };
    frame(&mut app, &ctx, vec![]);
    // The view's zoom after Actual Size from 250%, and the percent the app's zoom scheme reads it as.
    let actual = |app: &mut VectorcraftApp| {
        app.run("view.setZoom", json!({"zoom": 250})).unwrap();
        frame(app, &ctx, vec![]);
        app.run("view.actualSize", json!({})).unwrap();
        frame(app, &ctx, vec![]);
        let view = app.canvas_view().unwrap();
        let scheme = crate::app::view::zoom_scheme(app.print_ppi(), ctx.pixels_per_point() / ctx.zoom_factor());
        (view.zoom, scheme.percent(craft::behavior::canvas::zoom::device(&view, ctx.pixels_per_point())))
    };
    let (off, off_percent) = actual(&mut app);
    app.session.execute("prefs.set", &json!({"key": "displayPrintSize", "value": true})).unwrap();
    let (on, on_percent) = actual(&mut app);
    assert!(on > off, "an inch at 96 screen points shows the document larger: {off} → {on}");
    assert!((off_percent - 100.0).abs() < 1e-9 && (on_percent - 100.0).abs() < 1e-9, "both read 100%: {off_percent}, {on_percent}");
}

/// #9051: with Display Print Size on, Actual Size shows a document inch in an inch of the display
/// the window is on, from the density the host measured for it. When the window moves to a display
/// of another density, the view keeps its zoom percentage, so the art keeps its physical size there.
#[test]
fn view_actual_size_fills_an_inch_of_the_measured_display() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    app.session.execute("prefs.set", &json!({"key": "displayPrintSize", "value": true})).unwrap();
    let ctx = crate::testing::ui_context();
    let frame = |app: &mut VectorcraftApp, ctx: &egui::Context| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        crate::testing::part_frame(app, ctx, raw, show);
    };
    let run = |app: &mut VectorcraftApp, id: &str| {
        app.run(id, json!({})).unwrap();
        frame(app, &ctx);
    };
    frame(&mut app, &ctx);
    run(&mut app, "view.fitArtboard");
    let fitted = app.view().unwrap().zoom;
    assert!(app.set_screen_ppi(Some(127.0)));
    assert_eq!(app.view().unwrap().zoom, fitted, "the first measurement leaves a fitted view alone");
    run(&mut app, "view.actualSize");
    assert!((app.view().unwrap().zoom * 72.0 - 127.0).abs() < 1e-6, "72 pt take the display's 127 points to an inch");
    assert!(app.set_screen_ppi(Some(91.0)), "the window moved to a sparser display");
    assert!(!app.set_screen_ppi(Some(91.0)), "the same reading changes nothing");
    assert!((app.view().unwrap().zoom * 72.0 - 91.0).abs() < 1e-6, "100% still fills an inch");
    assert_eq!(crate::state::zoom_label(app.view().unwrap().zoom, app.print_ppi()), "100%");
    assert!(app.set_screen_ppi(None), "the window moved to a display that reports no size");
    assert!((app.view().unwrap().zoom * 72.0 - 96.0).abs() < 1e-6, "100% there is 96 points to an inch");
    app.session.execute("prefs.set", &json!({"key": "displayPrintSize", "value": false})).unwrap();
    let zoom = app.view().unwrap().zoom;
    assert!(app.set_screen_ppi(Some(110.0)));
    assert_eq!(app.view().unwrap().zoom, zoom, "off, a move leaves the zoom alone");
}

/// Fit Artboard in Window fits the active artboard as a turned view shows it: with the second of
/// two artboards active, at 45° all of it is on the canvas, and the two together are not.
#[test]
fn fit_artboard_fits_the_artboard_turned_with_the_view() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 612, "height": 792, "artboards": 2})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    app.view_mut().unwrap().artboard = 1;
    app.run("view.rotate", json!({"angle": 45})).unwrap();
    app.run("view.fitArtboard", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let view = app.canvas_view().unwrap();
    let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    let on_canvas = |r: vectorcraft_geom::Rect| view.quad(r).iter().all(|c| view.rect.contains(*c));
    assert!(on_canvas(boards[1]), "the turned active artboard is on the canvas: {:?} in {:?}", view.quad(boards[1]), view.rect);
    assert!(!on_canvas(boards[0].union(boards[1])), "fitted to the active artboard, not to both");
}

// While the Type tool edits text, Space types a space: held, it shows no hand and a drag with it
/// does not pan the view. Cmd+Space still zooms.
#[test]
fn space_types_a_space_while_editing_text_and_does_not_pan() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    let ctx = crate::testing::ui_context();
    let id = start_typing(&mut app, &ctx);
    let at = app.canvas_view().unwrap().to_screen(Point::new(100.0, 100.0));
    let space = |pressed, modifiers| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers };
    let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    // The cursor the canvas asked for in its last pass.
    let shown = |app: &mut VectorcraftApp| {
        typing_frame(app, &ctx, vec![]);
        craft::ui::runtime::cursor::current(&ctx)
    };
    typing_frame(&mut app, &ctx, vec![Event::Text("a".into()), Event::PointerMoved(at)]);
    let editing = shown(&mut app);
    typing_frame(&mut app, &ctx, vec![space(true, Modifiers::NONE), Event::Text(" ".into())]);
    assert_eq!(plain(&app, id), "a ", "Space types a space");
    assert_eq!(shown(&mut app), editing, "held while editing, Space lends no hand");
    let before = *app.view().unwrap();
    typing_frame(&mut app, &ctx, vec![button(at, true, Modifiers::NONE)]);
    typing_frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(30.0, 0.0))]);
    typing_frame(&mut app, &ctx, vec![button(at + vec2(30.0, 0.0), false, Modifiers::NONE)]);
    let after = *app.view().unwrap();
    assert_eq!((after.center, after.zoom, after.rotation), (before.center, before.zoom, before.rotation), "a drag with Space held does not pan");
    assert!(app.session.tool_wants_text(), "editing goes on");
    typing_frame(&mut app, &ctx, vec![space(false, Modifiers::NONE)]);
    // Cmd+Space still lends the Zoom tool.
    typing_frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::COMMAND), space(true, Modifiers::COMMAND), Event::PointerMoved(at)]);
    assert_ne!(shown(&mut app), editing, "Cmd+Space shows the magnifier");
    typing_frame(&mut app, &ctx, vec![button(at, true, Modifiers::COMMAND)]);
    typing_frame(&mut app, &ctx, vec![button(at, false, Modifiers::COMMAND)]);
    assert_eq!(app.view().unwrap().zoom, crate::state::next_zoom(before.zoom, true, app.print_ppi()), "Cmd+Space click zooms in");
    typing_frame(&mut app, &ctx, vec![space(false, Modifiers::COMMAND), Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(plain(&app, id), "a ", "the text is as typed");
}

/// While the Type tool edits text, a zoom marquee that Cmd+Space started keeps its Space rules once
/// Cmd is let go: Space held since the press sizes it, and Space pressed again moves it on every
/// frame it stays held, without typing a space. With no marquee, Space types a space again, and
/// during an animated zoom drag it types none.
#[test]
fn space_moves_a_text_edit_zoom_marquee_after_cmd_is_let_go() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    app.run("prefs.set", json!({"key": "animatedZoom", "value": false})).unwrap();
    let ctx = crate::testing::ui_context();
    ctx.enable_accesskit();
    let id = start_typing(&mut app, &ctx);
    let rect = app.canvas_view().unwrap().rect;
    let space = |pressed, modifiers| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers };
    let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    let out = typing_frame(&mut app, &ctx, vec![Event::Text("a".into())]);
    // The press starts on the canvas under the task bar: a press on the bar drags the bar.
    let at = under_task_bar(&out, rect);
    let z0 = app.view().unwrap().zoom;
    // Space held since the Cmd+Space press sizes the marquee, also once Cmd is let go.
    typing_frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::COMMAND), space(true, Modifiers::COMMAND), Event::PointerMoved(at)]);
    typing_frame(&mut app, &ctx, vec![button(at, true, Modifiers::COMMAND)]);
    typing_frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(50.0, 25.0))]);
    typing_frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerMoved(at + vec2(80.0, 40.0))]);
    typing_frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(100.0, 50.0))]);
    // Let go and pressed again, Space moves the marquee on this frame and the frames after it.
    typing_frame(&mut app, &ctx, vec![space(false, Modifiers::NONE)]);
    typing_frame(&mut app, &ctx, vec![space(true, Modifiers::NONE), Event::Text(" ".into()), Event::PointerMoved(at + vec2(120.0, 60.0))]);
    typing_frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(140.0, 70.0))]);
    assert_eq!(plain(&app, id), "a", "the Space that moves the marquee types nothing");
    typing_frame(&mut app, &ctx, vec![space(false, Modifiers::NONE)]);
    typing_frame(&mut app, &ctx, vec![space(true, Modifiers::NONE), Event::Ime(egui::ImeEvent::Commit(" ".into()))]);
    assert_eq!(plain(&app, id), "a", "nor does a space an input method commits for it");
    // Let go, the drag sizes it again, and the release zooms to the moved marquee.
    typing_frame(&mut app, &ctx, vec![space(false, Modifiers::NONE), Event::PointerMoved(at + vec2(180.0, 95.0))]);
    typing_frame(&mut app, &ctx, vec![button(at + vec2(180.0, 95.0), false, Modifiers::NONE)]);
    // Moved by Space, the marquee spans 140 × 75 points; never moved, it would span 180 × 95.
    let to = |w: f32, h: f32| z0 * f64::from((rect.width() / w).min(rect.height() / h));
    let (moved, unmoved, zoom) = (to(140.0, 75.0), to(180.0, 95.0), app.view().unwrap().zoom);
    assert!((zoom - moved).abs() < (zoom - unmoved).abs(), "{z0} → {zoom}: zoomed to the moved marquee ({moved}), not {unmoved}");
    assert!(app.session.tool_wants_text(), "editing goes on");
    // With no marquee, Space types a space.
    typing_frame(&mut app, &ctx, vec![space(true, Modifiers::NONE), Event::Text(" ".into())]);
    typing_frame(&mut app, &ctx, vec![space(false, Modifiers::NONE)]);
    assert_eq!(plain(&app, id), "a ", "Space types a space");
    // With Animated Zoom, Space during the Cmd+Space drag does not type a space either.
    app.run("prefs.set", json!({"key": "animatedZoom", "value": true})).unwrap();
    // The zoom moved the text and its task bar.
    let at = under_task_bar(&typing_frame(&mut app, &ctx, vec![]), app.canvas_view().unwrap().rect);
    typing_frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::COMMAND), space(true, Modifiers::COMMAND), Event::PointerMoved(at)]);
    typing_frame(&mut app, &ctx, vec![button(at, true, Modifiers::COMMAND)]);
    typing_frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::NONE), space(false, Modifiers::NONE)]);
    typing_frame(&mut app, &ctx, vec![space(true, Modifiers::NONE), Event::Text(" ".into())]);
    typing_frame(&mut app, &ctx, vec![button(at, false, Modifiers::NONE)]);
    typing_frame(&mut app, &ctx, vec![space(false, Modifiers::NONE)]);
    assert_eq!(plain(&app, id), "a ", "the text is as typed");
}

#[test]
fn space_pressed_mid_drag_moves_what_the_tool_draws() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.smart_guides = false;
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let xf = app.canvas_view().unwrap();
    let space = |pressed| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    // Press at `a`, drag to `b`, then hold Space while moving on by `by` and let go of both.
    let draw = |app: &mut VectorcraftApp, a: Pos2, b: Pos2, by: egui::Vec2| {
        frame(app, &ctx, vec![Event::PointerMoved(a)]);
        frame(app, &ctx, vec![button(a, true)]);
        frame(app, &ctx, vec![Event::PointerMoved(b)]);
        frame(app, &ctx, vec![space(true)]);
        frame(app, &ctx, vec![Event::PointerMoved(b + by)]);
        frame(app, &ctx, vec![space(false)]);
        frame(app, &ctx, vec![button(b + by, false)]);
    };
    let near = |p: Point, q: Point| p.distance(q) < 1e-3;
    let at = app.canvas_view().unwrap().rect.center() - vec2(100.0, 60.0);
    let center = app.view().unwrap().center;
    // The rectangle moves at its size instead of growing, and the view doesn't pan.
    app.select_tool("rectangle");
    draw(&mut app, at, at + vec2(80.0, 40.0), vec2(30.0, 20.0));
    assert_eq!(app.view().unwrap().center, center, "Space during a drag doesn't pan");
    let st = app.session.active().unwrap();
    let r = st.doc.node(st.selection.objects[0]).unwrap().path_data().unwrap().bounds().unwrap();
    assert!(
        near(Point::new(r.x0, r.y0), xf.to_doc(at + vec2(30.0, 20.0))) && near(Point::new(r.x1, r.y1), xf.to_doc(at + vec2(110.0, 60.0))),
        "{r:?}"
    );
    // The Pen's anchor being dragged out moves, handles and all.
    app.session.execute("select.none", &json!({})).unwrap();
    app.select_tool("pen");
    let a = at + vec2(0.0, 120.0);
    draw(&mut app, a, a + vec2(40.0, 0.0), vec2(20.0, 10.0));
    let st = app.session.active().unwrap();
    let anchor = st.doc.node(st.selection.objects[0]).unwrap().path_data().unwrap().subpaths[0].anchors[0];
    assert!(near(anchor.p, xf.to_doc(a + vec2(20.0, 10.0))) && near(anchor.h_out, xf.to_doc(a + vec2(60.0, 10.0))), "{anchor:?}");
}

#[test]
fn double_clicking_type_with_the_selection_tool_puts_the_caret_there() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 100, "y": 150, "text": "Hello world", "size": 24})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    app.select_tool("selection");
    let ctx = crate::testing::ui_context();
    let timed = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    timed(&mut app, 0.0, vec![]);
    let xf = app.canvas_view().unwrap();
    let text = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    };
    // Double-click the middle of the text.
    let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    let at = xf.to_screen(b.center());
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    timed(&mut app, 1.0, vec![egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
    // The framework holds a frame's second press for the next frame; the double-click it completes hands the text
    // to the Type tool two frames after that.
    timed(&mut app, 1.05, vec![]);
    timed(&mut app, 1.1, vec![]);
    timed(&mut app, 1.15, vec![]);
    // The Type tool edits the text, with the caret inside it, where it was clicked: typing goes
    // into the text, not to tool shortcuts (X would swap fill and stroke).
    assert_eq!(app.session.tool_id(), "type");
    assert!(app.session.tool_wants_text());
    let o = app.session.tool_options();
    assert_eq!(o["editing"], json!(id));
    let caret = o["caret"].as_u64().unwrap();
    assert!((1..11).contains(&caret), "caret {caret}");
    app.session.tool_text("X", app.view_info()).unwrap();
    let t = text(&app);
    assert_eq!((t.len(), t.find('X')), (12, Some(caret as usize)), "{t}");
}

/// #2119: type copied with an Alt-drag, double-clicked, all of its text selected and typed over
/// keeps its font, size and colour: through an IME (the marked text cleared, then committed by
/// Return, as Windows sends them) and with Backspace then typing. Whole app frames, one input
/// event per frame.
fn retype_all_of_a_copied_type(ime: bool) {
    use egui::{Event, Key, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let original = app
        .session
        .execute("text.create", &json!({"x": 60, "y": 100, "text": "Chair 123", "size": 36, "font": "Georgia", "color": "#808080"}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    app.select_tool("selection");
    let ctx = crate::testing::ui_context();
    let time = std::cell::Cell::new(0.0);
    let frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
        time.set(time.get() + 0.03);
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 850.0));
        crate::testing::app_frame(app, &ctx, egui::RawInput { screen_rect: Some(screen), time: Some(time.get()), events, ..Default::default() });
    };
    for _ in 0..3 {
        frame(&mut app, vec![]);
    }
    let xf = app.canvas_view().unwrap();
    let node = |app: &VectorcraftApp, id: u64| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    let styles = |app: &VectorcraftApp, id: u64| match &node(app, id).kind {
        NodeKind::Text(t) => {
            t.runs.iter().map(|r| (r.text.clone(), r.style.font_family.clone(), r.style.size, r.style.fill.clone())).collect::<Vec<_>>()
        }
        _ => panic!("not text"),
    };
    let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    let key = |key, pressed, modifiers| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
    // 1. Alt-drag the type: a copy, selected.
    let from = xf.to_screen(node(&app, original).geometric_bounds().unwrap().center());
    let to = from + vec2(0.0, 120.0 * xf.zoom as f32);
    frame(&mut app, vec![Event::PointerMoved(from), Event::ModifiersChanged(Modifiers::ALT)]);
    frame(&mut app, vec![button(from, true, Modifiers::ALT)]);
    for k in 1..=6 {
        frame(&mut app, vec![Event::PointerMoved(from + (to - from) * (k as f32 / 6.0))]);
    }
    frame(&mut app, vec![button(to, false, Modifiers::ALT)]);
    frame(&mut app, vec![Event::ModifiersChanged(Modifiers::NONE)]);
    let copy = app.session.active().unwrap().selection.objects[0].0;
    assert_ne!(copy, original);
    let style = styles(&app, original)[0].clone();
    // 2. Double-click it at once: the Type tool edits it.
    let at = xf.to_screen(node(&app, copy).geometric_bounds().unwrap().center());
    frame(&mut app, vec![Event::PointerMoved(at)]);
    for pressed in [true, false, true, false] {
        frame(&mut app, vec![button(at, pressed, Modifiers::NONE)]);
    }
    frame(&mut app, vec![]);
    // Without Georgia on this computer, the Type tool asks first; OK starts the edit.
    if app.dialogs().unwrap().with(|h| h.confirm::<crate::dialogs::confirm::MissingFontQuestion>(&ctx)) {
        frame(&mut app, vec![]);
    }
    assert_eq!(app.session.tool_options()["editing"], json!(copy));
    // 3. Select all of its text and type over it.
    frame(&mut app, vec![key(Key::A, true, Modifiers::COMMAND)]);
    frame(&mut app, vec![key(Key::A, false, Modifiers::COMMAND)]);
    if ime {
        frame(&mut app, vec![preedit("t", 1)]);
        frame(&mut app, vec![preedit("table", 5)]);
        let marked = styles(&app, copy);
        assert_eq!(marked.iter().map(|r| (&r.1, r.2, &r.3)).collect::<Vec<_>>(), vec![(&style.1, style.2, &style.3)], "composing: {marked:?}");
        // Return commits the composition.
        frame(&mut app, vec![preedit("", 0), Event::Ime(egui::ImeEvent::Commit("table".into()))]);
    } else {
        frame(&mut app, vec![key(Key::Backspace, true, Modifiers::NONE)]);
        frame(&mut app, vec![key(Key::Backspace, false, Modifiers::NONE)]);
        frame(&mut app, vec![Event::Text("table".into())]);
        frame(&mut app, vec![key(Key::Enter, true, Modifiers::NONE)]);
        frame(&mut app, vec![key(Key::Enter, false, Modifiers::NONE)]);
    }
    frame(&mut app, vec![]);
    let after = styles(&app, copy);
    assert!(after.iter().any(|r| r.0.starts_with("table")), "ime={ime}: typed over: {after:?}");
    assert!(
        after.iter().all(|r| (&r.1, r.2, &r.3) == (&style.1, style.2, &style.3)),
        "ime={ime}: the copy keeps {:?} {} {:?}, got {after:?}",
        style.1,
        style.2,
        style.3
    );
}

#[test]
fn retyping_all_of_a_copied_type_through_an_ime_keeps_its_style() {
    retype_all_of_a_copied_type(true);
}

#[test]
fn retyping_all_of_a_copied_type_after_backspace_keeps_its_style() {
    retype_all_of_a_copied_type(false);
}

#[test]
fn double_clicking_type_with_the_direct_selection_tool_puts_the_caret_there() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 100, "y": 150, "text": "Hello world", "size": 24})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    app.select_tool("directSelection");
    let ctx = crate::testing::ui_context();
    let timed = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() }, |ui| {
            show(app, ui);
            crate::shortcuts::escape(app, ui.ctx());
        });
        out.textures_delta.clear();
    };
    timed(&mut app, 0.0, vec![]);
    let xf = app.canvas_view().unwrap();
    let text = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    };
    // Double-click the middle of the text with Direct Selection.
    let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    let at = xf.to_screen(b.center());
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    timed(&mut app, 1.0, vec![egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
    // The framework holds a frame's second press for the next frame; the double-click it completes hands the text
    // to the Type tool two frames after that.
    timed(&mut app, 1.05, vec![]);
    timed(&mut app, 1.1, vec![]);
    timed(&mut app, 1.15, vec![]);
    // The Type tool edits the text, with the caret inside it, where it was clicked: typing goes
    // into the text, not to tool shortcuts.
    assert_eq!(app.session.tool_id(), "type");
    assert!(app.session.tool_wants_text());
    let o = app.session.tool_options();
    assert_eq!(o["editing"], json!(id));
    let caret = o["caret"].as_u64().unwrap();
    assert!((1..11).contains(&caret), "caret {caret}");
    app.session.tool_text("X", app.view_info()).unwrap();
    let t = text(&app);
    assert_eq!((t.len(), t.find('X')), (12, Some(caret as usize)), "{t}");

    // Pressing Escape exits text editing and returns to Direct Selection tool.
    let esc = |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    timed(&mut app, 1.2, vec![esc(true), esc(false)]);
    assert_eq!(app.session.tool_id(), "directSelection");
    assert!(!app.session.tool_wants_text());
}

/// Double-clicking type set in a family that isn't available asks before the Type tool edits it:
/// Cancel leaves the document, its undo history and the selection as they were, with nothing
/// edited; OK puts the caret where the type was clicked, and typing edits the type, which keeps
/// the family it names.
#[test]
fn double_clicking_type_in_a_missing_font_asks_before_editing() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let params = json!({"x": 100, "y": 150, "text": "Hello world", "size": 24, "font": "Missing Test Family 7258"});
    let id = app.session.execute("text.create", &params).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    app.select_tool("selection");
    let ctx = crate::testing::ui_context();
    app.ui_ctx = Some(ctx.clone());
    let timed = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    timed(&mut app, 0.0, vec![]);
    let xf = app.canvas_view().unwrap();
    let node = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    // Answers the open question as OK or Cancel would, and runs what the answer does.
    let answer = |app: &mut VectorcraftApp, ok: bool| {
        use crate::dialogs::confirm::MissingFontQuestion;
        let host = app.dialogs().unwrap();
        assert!(
            host.with(|h| if ok { h.confirm::<MissingFontQuestion>(&ctx) } else { h.cancel::<MissingFontQuestion>(&ctx) }),
            "the question is open"
        );
        crate::dialogs::host::pump(app, &ctx);
    };
    let b = node(&app).geometric_bounds().unwrap();
    // A quarter of the way in, away from the middle of the text.
    let at = xf.to_screen(Point::new(b.x0 + b.width() * 0.25, b.center().y));
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let double_click = |app: &mut VectorcraftApp, time: f64| {
        timed(app, time, vec![egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
        // The framework holds a frame's second press for the next frame; the double-click it completes asks two
        // frames after that.
        timed(app, time + 0.05, vec![]);
        timed(app, time + 0.1, vec![]);
        timed(app, time + 0.15, vec![]);
    };
    let asking = |app: &VectorcraftApp| app.dialogs().unwrap().with(|h| h.is_open::<crate::dialogs::confirm::MissingFontQuestion>());
    double_click(&mut app, 1.0);
    assert!(asking(&app), "the question about the missing font");
    assert_eq!(app.session.tool_options()["editing"], json!(null));
    // Cancel: nothing is edited, and the document, its undo history and the selection stay.
    let (before, undo, selected) =
        (node(&app), app.session.doc().unwrap().history.undo.len(), app.session.active().unwrap().selection.objects.clone());
    answer(&mut app, false);
    timed(&mut app, 1.5, vec![]);
    assert_eq!(app.session.tool_options()["editing"], json!(null));
    assert!(!app.session.tool_wants_text());
    assert_eq!(node(&app), before);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
    assert_eq!(app.session.active().unwrap().selection.objects, selected);
    // A new double-click asks again; OK edits with the caret where the type was clicked.
    double_click(&mut app, 3.0);
    assert!(asking(&app), "asked again");
    answer(&mut app, true);
    timed(&mut app, 3.5, vec![]);
    assert!(!asking(&app));
    let o = app.session.tool_options();
    assert_eq!(o["editing"], json!(id));
    let caret = o["caret"].as_u64().unwrap();
    assert!((1..11).contains(&caret), "caret {caret}");
    // The caret is the one a click at that point puts in the type being edited.
    timed(&mut app, 5.0, vec![button(true), button(false)]);
    timed(&mut app, 5.1, vec![]);
    assert_eq!(app.session.tool_options()["caret"], json!(caret), "the caret where the type was double-clicked");
    app.session.tool_text("X", app.view_info()).unwrap();
    let NodeKind::Text(t) = &node(&app).kind else { panic!("not text") };
    assert_eq!(t.plain_text().find('X'), Some(caret as usize), "{}", t.plain_text());
    assert!(t.runs.iter().all(|r| r.style.font_family == "Missing Test Family 7258"));
}

/// A Type tool press that reaches type in a missing font while another dialog is open (as one sent
/// with `ui.pointer` does) leaves that dialog open and starts no edit.
#[test]
fn a_press_on_type_in_a_missing_font_keeps_an_open_dialog() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let params = json!({"x": 100, "y": 150, "text": "Hello world", "size": 24, "font": "Missing Test Family 8408"});
    let id = app.session.execute("text.create", &params).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    app.select_tool("type");
    let ctx = crate::testing::ui_context();
    // A frame of the whole window opens the question the app queued.
    let window = |app: &mut VectorcraftApp| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))), ..Default::default() };
        crate::testing::app_frame(app, &ctx, raw);
    };
    window(&mut app);
    let doc = app.session.active().unwrap().uid;
    crate::dialogs::confirm::ask_replace_files(&mut app, doc, &["/tmp/a.svg".to_string()], &[], "file.export", json!({}));
    window(&mut app);
    let replacing = |app: &VectorcraftApp| crate::dialogs::confirm::open_replace_files(app).is_some();
    assert!(replacing(&app), "the question about replacing files is open");
    let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    let view = app.view_info();
    for kind in [PointerKind::Down, PointerKind::Up] {
        crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, b.center().x, b.center().y), view);
    }
    assert!(replacing(&app), "that question stays");
    assert!(!app.dialogs().unwrap().with(|h| h.is_open::<crate::dialogs::confirm::MissingFontQuestion>()), "no question about the font");
    assert!(app.dialogs().unwrap().with(|h| h.cancel::<crate::dialogs::confirm::ReplaceFilesPrompt>(&ctx)));
    window(&mut app);
    let r = app.session.tool_poll(view, false);
    crate::canvas::apply_requests(&mut app, r);
    assert!(!replacing(&app));
    assert_eq!(app.session.tool_options()["editing"], json!(null));
}

#[test]
fn escape_while_editing_in_isolation_returns_to_selection_tool_and_keeps_isolation_until_second_escape() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let tid = app.session.execute("text.create", &json!({"x": 100, "y": 150, "text": "Nested hello", "size": 24})).unwrap()["id"].as_u64().unwrap();
    // Group the text inside a group.
    app.session.execute("select.all", &json!({})).unwrap();
    app.session.execute("object.group", &json!({})).unwrap();
    app.select_tool("selection");
    let ctx = crate::testing::ui_context();
    let timed = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() }, |ui| {
            show(app, ui);
            crate::shortcuts::escape(app, ui.ctx());
        });
        out.textures_delta.clear();
    };
    timed(&mut app, 0.0, vec![]);
    let xf = app.canvas_view().unwrap();
    let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(tid)).unwrap().geometric_bounds().unwrap();
    let at = xf.to_screen(b.center());
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let esc = |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };

    // 1st double-click on group enters isolation mode.
    timed(&mut app, 1.0, vec![egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
    // The framework holds a frame's second press for the next frame; the double-click it completes isolates the group
    // two frames after that.
    timed(&mut app, 1.05, vec![]);
    timed(&mut app, 1.1, vec![]);
    timed(&mut app, 1.15, vec![]);
    assert!(app.session.active().unwrap().isolation.is_some(), "should be in isolation mode");
    assert_eq!(app.session.tool_id(), "selection");

    // 2nd double-click on the text inside the isolated group enters text editing.
    timed(&mut app, 2.0, vec![egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
    timed(&mut app, 2.05, vec![]);
    timed(&mut app, 2.1, vec![]);
    assert_eq!(app.session.tool_id(), "type");
    assert!(app.session.tool_wants_text());
    assert!(app.session.active().unwrap().isolation.is_some(), "isolation mode still active while editing text");

    // 1st Escape exits text edit mode, returns to Selection tool, but keeps isolation mode.
    timed(&mut app, 3.0, vec![esc(true), esc(false)]);
    timed(&mut app, 3.1, vec![]);
    assert_eq!(app.session.tool_id(), "selection");
    assert!(!app.session.tool_wants_text());
    assert!(app.session.active().unwrap().isolation.is_some(), "1st Escape must NOT end isolation mode");

    // 2nd Escape exits isolation mode.
    timed(&mut app, 4.0, vec![esc(true), esc(false)]);
    timed(&mut app, 4.1, vec![]);
    assert_eq!(app.session.tool_id(), "selection");
    assert!(app.session.active().unwrap().isolation.is_none(), "2nd Escape must exit isolation mode");
}

/// One Escape leaves isolation mode from a sublayer two levels down, where Exit Isolation Mode goes
/// up one layer at a time.
#[test]
fn escape_leaves_an_isolated_sublayer_in_one_step() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let top = app.session.active().unwrap().doc.layers[0].id;
    let mid = app.session.execute("layer.newSublayer", &json!({"parent": top.0})).unwrap()["id"].as_u64().unwrap();
    let deep = app.session.execute("layer.newSublayer", &json!({"parent": mid})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("layer.setCurrent", &json!({"id": deep})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
    app.session.execute("object.isolate", &json!({"id": deep})).unwrap();
    app.select_tool("selection");
    let ctx = crate::testing::ui_context();
    let timed = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() }, |ui| {
            show(app, ui);
            crate::shortcuts::escape(app, ui.ctx());
        });
        out.textures_delta.clear();
    };
    let esc = |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    timed(&mut app, 0.0, vec![]);
    assert_eq!(app.session.active().unwrap().isolation, Some(vectorcraft_doc::NodeId(deep)));
    timed(&mut app, 1.0, vec![esc(true), esc(false)]);
    timed(&mut app, 1.1, vec![]);
    assert_eq!(app.session.active().unwrap().isolation, None, "one Escape leaves every level");
}

/// The art the canvas hands the panels while a move is carried off it.
fn carried(ctx: &egui::Context) -> Option<Vec<vectorcraft_doc::NodeId>> {
    match craft::ui::runtime::drag::active::<crate::panel_drag::PanelDrag>(ctx).as_deref() {
        Some(crate::panel_drag::PanelDrag::Art { ids, .. }) => Some(ids.clone()),
        _ => None,
    }
}

/// A move with the Selection tool, or one the Direct or Group Selection tool makes of art it moves whole (#4233),
/// waits where it left the canvas while the panels get the art; released over a panel, the move goes back and
/// nothing is recorded.
#[test]
fn art_moved_off_the_canvas_waits_and_drops_on_the_panel() {
    for tool in ["selection", "directSelection", "groupSelection"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        let id = vectorcraft_doc::NodeId(id);
        app.session.execute("select.none", &json!({})).unwrap();
        app.select_tool(tool);
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx, vec![]);
        let view = app.canvas_view().unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let start = view.to_screen(Point::new(100.0, 100.0));
        let left = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0;
        let off = pos2(view.rect.right() + 50.0, view.rect.center().y);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, true)]);
        assert_eq!(app.session.active().unwrap().selection.whole(), [id], "{tool}: selected whole");
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(40.0, 0.0))]);
        assert!(app.session.active().unwrap().interaction.is_some(), "{tool}: the move is live");
        let moved = left(&app);
        assert!(moved > 50.0, "{tool}: the art moved with the pointer");
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(off)]);
        frame(&mut app, &ctx, vec![]);
        assert!(app.session.active().unwrap().interaction.is_some(), "{tool}: the move waits, not cancels");
        assert_eq!(left(&app), moved, "{tool}: the art stays where it last moved");
        assert_eq!(carried(&ctx), Some(vec![id]), "{tool}");
        frame(&mut app, &ctx, vec![button(off, false)]);
        frame(&mut app, &ctx, vec![]);
        let st = app.session.active().unwrap();
        assert!(st.interaction.is_none(), "{tool}");
        assert_eq!(*st.doc, *before, "{tool}: the move goes back");
        assert_eq!(st.history.undo.len(), undo, "{tool}: nothing recorded");
    }
}

/// Art Alt-dragged off the canvas (a copy while it moves there) reaches the panels as the art the
/// drag started with, which is what stays when the release over a panel takes the copy back.
#[test]
fn art_alt_dragged_off_the_canvas_reaches_the_panels_as_the_original() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let id = vectorcraft_doc::NodeId(id);
    app.select_tool("selection");
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let view = app.canvas_view().unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let button = |pos: Pos2, pressed: bool| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::ALT };
    let start = view.to_screen(Point::new(100.0, 100.0));
    let off = pos2(view.rect.right() + 50.0, view.rect.center().y);
    frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::ALT), Event::PointerMoved(start), button(start, true)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(start + vec2(40.0, 0.0))]);
    let st = app.session.active().unwrap();
    assert_eq!(st.interaction.as_ref().map(|i| i.label.as_str()), Some("Copy"));
    assert!(!st.selection.objects.contains(&id), "the copy is selected while it moves");
    frame(&mut app, &ctx, vec![Event::PointerMoved(off)]);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(carried(&ctx), Some(vec![id]), "the panels get the original");
    frame(&mut app, &ctx, vec![button(off, false)]);
    frame(&mut app, &ctx, vec![]);
    let st = app.session.active().unwrap();
    assert!(st.interaction.is_none());
    assert_eq!(*st.doc, *before, "the copy goes, the original stays");
    assert!(st.doc.node(id).is_some());
}

/// #2826: a move taken off the canvas waits where it left it, and goes on with the pointer when it
/// comes back: it isn't dropped or put back.
#[test]
fn art_moved_off_the_canvas_resumes_when_the_pointer_returns() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("selection");
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let view = app.canvas_view().unwrap();
    let undo = app.session.active().unwrap().history.undo.len();
    let button =
        |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let start = view.to_screen(Point::new(100.0, 100.0));
    let off = pos2(view.rect.right() + 50.0, view.rect.center().y);
    let left = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0;
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(40.0, 0.0))]);
    let at_40 = left(&app);
    assert!(at_40 > 50.0, "the move is live");
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(off)]);
    assert!(app.session.active().unwrap().interaction.is_some(), "the move waits, not cancels");
    assert_eq!(left(&app), at_40, "the art stays where it last moved");
    // Back on the canvas it goes on with the pointer, further right.
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(60.0, 0.0))]);
    assert!(app.session.active().unwrap().interaction.is_some(), "the move is live again");
    let at_60 = left(&app);
    assert!(at_60 > at_40, "the move resumes with the pointer: {at_40} → {at_60}");
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(70.0, 0.0))]);
    frame(&mut app, &ctx, vec![button(start + vec2(70.0, 0.0), false)]);
    assert!(left(&app) > at_60, "released where the pointer was");
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one move, one undo step");
}

// One frame of the canvas as the app runs it, with Escape taken last in the pass; the commands the
/// frame's keys fired then run, as the next frame's input takes them first.
fn typing_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    app.ui_ctx = Some(ctx.clone());
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        show(app, ui);
        crate::shortcuts::escape(app, ui.ctx());
    });
    out.textures_delta.clear();
    craft::ui::shell::embed::before_input(ctx, |output| app.frame_input(output));
    out
}

/// A point on the canvas of `rect` just under the Contextual Task Bar of the frame `out`, or `rect`'s centre with no bar.
fn under_task_bar(out: &egui::FullOutput, rect: egui::Rect) -> Pos2 {
    let nodes = out.platform_output.accesskit_update.as_ref().map_or(&[][..], |u| &u.nodes[..]);
    let bar = nodes.iter().find(|(_, n)| n.label() == Some(BAR)).and_then(|(_, n)| n.bounds());
    bar.map_or(rect.center(), |b| pos2(rect.center().x - 100.0, b.y1 as f32 + 20.0))
}

fn preedit(t: &str, chars: usize) -> egui::Event {
    egui::Event::Ime(egui::ImeEvent::Preedit { text: t.into(), active_range_chars: Some(chars..chars) })
}

fn plain(app: &VectorcraftApp, id: u64) -> String {
    match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    }
}

/// Starts editing new point type with the Type tool; returns its id.
fn start_typing(app: &mut VectorcraftApp, ctx: &egui::Context) -> u64 {
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    typing_frame(app, ctx, vec![]);
    app.select_tool("type");
    let xf = app.canvas_view().unwrap();
    let at = xf.to_screen(Point::new(100.0, 100.0));
    let click =
        |p: Pos2, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    typing_frame(app, ctx, vec![egui::Event::PointerMoved(at), click(at, true)]);
    typing_frame(app, ctx, vec![click(at, false)]);
    assert!(app.session.tool_wants_text());
    app.session.active().unwrap().selection.objects[0].0
}

/// Returns a key press and its release.
fn tap(key: egui::Key) -> [egui::Event; 2] {
    [true, false].map(|pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() })
}

/// Arrows, Home, End, Tab, Enter and clipboard events go to the Type tool in order with the text
/// of the same frame, and each Enter types a line break.
#[test]
fn the_type_tool_takes_text_and_editing_keys_in_the_order_they_came() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    let ctx = crate::testing::ui_context();
    let id = start_typing(&mut app, &ctx);
    let text = |t: &str| egui::Event::Text(t.into());
    typing_frame(&mut app, &ctx, vec![text("ax")]);
    // ← moves the caret before the text that follows it goes in.
    typing_frame(&mut app, &ctx, [tap(egui::Key::ArrowLeft).to_vec(), vec![text("b")]].concat());
    assert_eq!(plain(&app, id), "abx");
    // Home and End move the caret before the text that follows each goes in.
    typing_frame(&mut app, &ctx, [tap(egui::Key::Home).to_vec(), vec![text("0")], tap(egui::Key::End).to_vec(), vec![text("9")]].concat());
    assert_eq!(plain(&app, id), "0abx9");
    // Tab and a paste go in between the texts around them.
    let paste = egui::Event::Paste("p".into());
    typing_frame(&mut app, &ctx, [vec![text("t")], tap(egui::Key::Tab).to_vec(), vec![text("u"), paste, text("q")]].concat());
    assert_eq!(plain(&app, id), "0abx9t\tupq");
    // Enter types its line break between the texts around it, once for each press.
    typing_frame(&mut app, &ctx, [vec![text("1")], tap(egui::Key::Enter).to_vec(), tap(egui::Key::Enter).to_vec(), vec![text("2")]].concat());
    assert_eq!(plain(&app, id), "0abx9t\tupq1\n\n2");
    // A key that comes before a composition starts in the same frame still moves the caret.
    typing_frame(&mut app, &ctx, [tap(egui::Key::Home).to_vec(), vec![preedit("k", 1)]].concat());
    typing_frame(&mut app, &ctx, vec![preedit("", 0), egui::Event::Ime(egui::ImeEvent::Commit("k".into()))]);
    assert_eq!(plain(&app, id), "0abx9t\tupq1\n\nk2");
}

/// Enter keeps each key event's modifiers while sharing a frame with typed text.
#[test]
fn shift_enter_types_a_soft_return_in_order_with_plain_enter() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    let ctx = crate::testing::ui_context();
    let id = start_typing(&mut app, &ctx);
    let text = |t: &str| egui::Event::Text(t.into());
    let enter =
        |modifiers| [true, false].map(|pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers });
    typing_frame(
        &mut app,
        &ctx,
        [vec![text("a")], enter(egui::Modifiers::SHIFT).to_vec(), vec![text("b")], enter(egui::Modifiers::NONE).to_vec(), vec![text("c")]].concat(),
    );
    assert_eq!(plain(&app, id), "a\u{2028}b\nc");
}

/// Within one frame, Home between two texts, Tab inside a line, and Backspace after a paste each
/// act on the text as the events before them left it.
#[test]
fn home_tab_and_backspace_act_on_the_text_before_them_in_the_frame() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    let ctx = crate::testing::ui_context();
    let id = start_typing(&mut app, &ctx);
    let text = |t: &str| egui::Event::Text(t.into());
    typing_frame(&mut app, &ctx, vec![text("abc")]);
    typing_frame(&mut app, &ctx, [vec![text("d")], tap(egui::Key::Home).to_vec(), vec![text("0")]].concat());
    assert_eq!(plain(&app, id), "0abcd");
    typing_frame(&mut app, &ctx, [tap(egui::Key::ArrowRight).to_vec(), tap(egui::Key::Tab).to_vec(), vec![text("t")]].concat());
    assert_eq!(plain(&app, id), "0a\ttbcd");
    typing_frame(&mut app, &ctx, [tap(egui::Key::End).to_vec(), vec![egui::Event::Paste("pq".into())], tap(egui::Key::Backspace).to_vec()].concat());
    assert_eq!(plain(&app, id), "0a\ttbcdp");
}

/// Cmd/Ctrl+Enter bound to a command runs the command and types no line break, also when a plain
/// Enter comes in the same frame. Unbound, it ends editing and keeps the text.
#[test]
fn a_command_on_cmd_enter_runs_without_typing_a_line_break() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    let ctx = crate::testing::ui_context();
    let id = start_typing(&mut app, &ctx);
    let cmd_enter = [true, false].map(|pressed| egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    typing_frame(&mut app, &ctx, vec![egui::Event::Text("a".into())]);
    app.run("shortcuts.set", json!({"id": "view.grid", "shortcut": "Cmd+Enter"})).unwrap();
    typing_frame(&mut app, &ctx, vec![]);
    let grid = app.ui.view.grid;
    typing_frame(&mut app, &ctx, cmd_enter.to_vec());
    let ran = (plain(&app, id), app.ui.view.grid, app.session.tool_wants_text());
    typing_frame(&mut app, &ctx, [tap(egui::Key::Enter).to_vec(), cmd_enter.to_vec()].concat());
    app.run("shortcuts.reset", json!({})).unwrap();
    typing_frame(&mut app, &ctx, vec![]);
    assert_eq!((ran.0.as_str(), ran.1, ran.2), ("a", !grid, true), "Cmd+Enter runs Show Grid only");
    assert_eq!((plain(&app, id).as_str(), app.ui.view.grid), ("a\n", grid), "Enter types a line break and Cmd+Enter runs Show Grid");
    assert!(app.session.tool_wants_text(), "a bound Cmd+Enter does not end editing");
    typing_frame(&mut app, &ctx, cmd_enter.to_vec());
    assert_eq!(plain(&app, id), "a\n", "nothing is bound to Cmd+Enter: no line break");
    assert!(!app.session.tool_wants_text(), "unbound, Cmd+Enter ends editing");
    assert_eq!(app.session.tool_id(), "selection");
}

/// Cmd+Enter (Ctrl+Enter on Windows and Linux) ends editing like Escape: a repeat of it, and text,
/// keys and a paste after it in the same frame, leave the type as it was. Enter, Shift+Enter and
/// Ctrl+Enter on macOS still type line breaks.
#[test]
fn cmd_enter_ends_text_editing_and_keeps_the_text() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    let ctx = crate::testing::ui_context();
    let id = start_typing(&mut app, &ctx);
    let key = |pressed, repeat, modifiers| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat, modifiers };
    let mac_ctrl = egui::Modifiers { ctrl: true, ..Default::default() };
    typing_frame(&mut app, &ctx, vec![egui::Event::Text("a".into())]);
    typing_frame(&mut app, &ctx, vec![key(true, false, egui::Modifiers::NONE), key(false, false, egui::Modifiers::NONE)]);
    typing_frame(&mut app, &ctx, vec![key(true, false, egui::Modifiers::SHIFT), key(false, false, egui::Modifiers::SHIFT)]);
    // Off macOS Ctrl is the command key, so a Ctrl-only Enter is Cmd+Enter there.
    let mac = cfg!(target_os = "macos");
    if mac {
        typing_frame(&mut app, &ctx, vec![key(true, false, mac_ctrl), key(false, false, mac_ctrl)]);
    }
    let typed = if mac { "a\n\u{2028}\n" } else { "a\n\u{2028}" };
    assert_eq!(plain(&app, id), typed);
    assert!(app.session.tool_wants_text());
    let held = vec![
        key(true, false, egui::Modifiers::COMMAND),
        key(true, true, egui::Modifiers::COMMAND),
        egui::Event::Text("b".into()),
        egui::Event::Key { key: egui::Key::Backspace, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
        egui::Event::Paste("c".into()),
        egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
    ];
    typing_frame(&mut app, &ctx, held);
    assert!(!app.session.tool_wants_text(), "editing ended");
    assert_eq!(app.session.tool_id(), "selection");
    assert_eq!(plain(&app, id), typed, "the text is kept, and nothing after the chord changes it");
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(id)], "the type stays selected");
    typing_frame(&mut app, &ctx, vec![key(true, true, egui::Modifiers::COMMAND)]);
    typing_frame(&mut app, &ctx, vec![key(false, false, egui::Modifiers::COMMAND)]);
    assert!(!app.any_dialog_open(), "the repeats of the chord open no dialog");
    assert_eq!(plain(&app, id), typed);
}

/// While the IME composes, Cmd+Enter is the IME's: it does not end editing, drop the marked text or
/// type a line break, in the frame that starts the composition or in a later one.
#[test]
fn cmd_enter_while_composing_keeps_the_composition_and_editing() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    let ctx = crate::testing::ui_context();
    let id = start_typing(&mut app, &ctx);
    let cmd_enter = [true, false].map(|pressed| egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    typing_frame(&mut app, &ctx, vec![egui::Event::Text("a".into())]);
    typing_frame(&mut app, &ctx, [vec![preedit("か", 1)], cmd_enter.to_vec()].concat());
    assert!(app.session.tool_composing(), "the same frame keeps the composition");
    assert!(app.session.tool_wants_text(), "the same frame keeps editing");
    assert_eq!(plain(&app, id), "aか", "the same frame keeps the marked text and types no line break");
    typing_frame(&mut app, &ctx, cmd_enter.to_vec());
    assert!(app.session.tool_composing(), "a later frame keeps the composition");
    assert!(app.session.tool_wants_text(), "a later frame keeps editing");
    assert_eq!(plain(&app, id), "aか", "a later frame keeps the marked text and types no line break");
}

/// #4711: Shift pressed and released while a corner handle is held still scales proportionally and
/// then freely again at once, without waiting for the pointer to move.
#[test]
fn a_modifier_changed_mid_scale_applies_with_the_pointer_held_still() {
    use egui::{Event, Modifiers};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.smart_guides = false;
    app.ui.view.snap_to_point = false;
    let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("selection");
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let xf = app.canvas_view().unwrap();
    let with = |app: &mut VectorcraftApp, mut events: Vec<Event>, modifiers: Modifiers| {
        events.insert(0, Event::ModifiersChanged(modifiers));
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let height =
        |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().height();
    // The pointer's screen points are f32: the scaled height is within their precision of the exact one.
    let near = |h: f64, want: f64| (h - want).abs() < 1e-4;
    let corner = xf.to_screen(Point::new(200.0, 150.0));
    let to = xf.to_screen(Point::new(300.0, 150.0));
    with(&mut app, vec![Event::PointerMoved(corner)], Modifiers::NONE);
    with(&mut app, vec![button(corner, true)], Modifiers::NONE);
    with(&mut app, vec![Event::PointerMoved(to)], Modifiers::NONE);
    assert!(near(height(&app), 50.0), "a free scale keeps the height: {}", height(&app));
    with(&mut app, vec![], Modifiers::SHIFT);
    assert!(near(height(&app), 100.0), "Shift scales proportionally at once: {}", height(&app));
    with(&mut app, vec![], Modifiers::NONE);
    assert!(near(height(&app), 50.0), "releasing Shift frees it at once: {}", height(&app));
    with(&mut app, vec![button(to, false)], Modifiers::NONE);
    assert!(app.session.active().unwrap().interaction.is_none(), "committed");
}

/// Backtick switches a moved Selection drag's live preview while the pointer stays still.
#[test]
fn backtick_changed_mid_selection_drag_applies_with_the_pointer_held_still() {
    use egui::Event;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let tile = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    app.run("object.pattern.make", json!({"ids": [tile], "name": "Dots", "width": 20, "height": 20})).unwrap();
    app.run("object.pattern.done", json!({})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"ids": [id], "swatch": "Dots"})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.select_tool("selection");
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.ui.view.smart_guides = false;
    app.ui.view.snap_to_point = false;
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let xf = app.canvas_view().unwrap();
    let from = xf.to_screen(Point::new(125.0, 125.0));
    let to = xf.to_screen(Point::new(145.0, 125.0));
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let key = |pressed| Event::Key { key: egui::Key::Backtick, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    let node = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    let pattern_x = |node: &vectorcraft_doc::Node| {
        let vectorcraft_color::Paint::Pattern { xf, .. } = node.appearance.fill_paint() else { panic!("pattern fill expected") };
        xf.as_coeffs()[4]
    };
    let before = node(&app);
    frame(&mut app, &ctx, vec![Event::PointerMoved(from), button(from, true)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(to)]);
    let moved = node(&app);
    assert!((moved.geometric_bounds().unwrap().x0 - before.geometric_bounds().unwrap().x0 - 20.0).abs() < 0.001);
    frame(&mut app, &ctx, vec![key(true)]);
    let pattern_only = node(&app);
    assert_eq!(pattern_only.geometric_bounds(), before.geometric_bounds(), "Backtick restores geometry in the live preview");
    assert!((pattern_x(&pattern_only) - pattern_x(&before) - 20.0).abs() < 0.001, "Backtick moves the pattern in the live preview");
    frame(&mut app, &ctx, vec![key(false)]);
    let ordinary = node(&app);
    assert_eq!(ordinary.geometric_bounds(), moved.geometric_bounds(), "releasing Backtick restores the ordinary move in the live preview");
    assert!((pattern_x(&ordinary) - pattern_x(&before)).abs() < 0.001, "the ordinary move restores the pattern matrix");
    frame(&mut app, &ctx, vec![button(to, false)]);
}

/// Backtick pressed while a flare's rings' end point is placed places the rings anew, with the
/// pointer held still.
#[test]
fn backtick_places_a_flares_rings_anew_with_the_pointer_held_still() {
    use egui::Event;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("flare");
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.ui.view.smart_guides = false;
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let xf = app.canvas_view().unwrap();
    let from = xf.to_screen(Point::new(100.0, 100.0));
    let to = xf.to_screen(Point::new(130.0, 100.0));
    let end = xf.to_screen(Point::new(180.0, 160.0));
    assert!(app.canvas_view().unwrap().rect.contains(end));
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let key = |pressed| Event::Key { key: egui::Key::Backtick, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    let rings = |app: &VectorcraftApp| {
        let st = app.session.doc().unwrap();
        let g = st.doc.node(st.selection.objects[0]).unwrap();
        let vectorcraft_doc::NodeKind::Group { children, .. } = &g.kind else { panic!("flare is a group") };
        children.iter().filter(|c| c.name.as_deref() == Some("Ring")).map(|c| c.geometric_bounds().unwrap()).collect::<Vec<_>>()
    };
    // Frames a sixtieth of a second apart while the pointer moves, and a second apart with it
    // still, so it counts as still once it stops.
    let mut time = 0.0;
    let mut step = |app: &mut VectorcraftApp, events: Vec<Event>| {
        time += if events.iter().any(|e| matches!(e, Event::PointerMoved(_))) { 1.0 / 60.0 } else { 1.0 };
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    step(&mut app, vec![Event::PointerMoved(from), button(from, true)]);
    step(&mut app, vec![Event::PointerMoved(to)]);
    step(&mut app, vec![button(to, false)]);
    step(&mut app, vec![Event::PointerMoved(end - vec2(5.0, 5.0))]);
    step(&mut app, vec![Event::PointerMoved(end)]);
    step(&mut app, vec![]);
    step(&mut app, vec![]);
    let before = rings(&app);
    assert_eq!(before.len(), 10);
    assert!(before.iter().all(|r| r.center().y > 90.0 && r.center().x > 90.0), "the rings run towards the pointer: {before:?}");
    step(&mut app, vec![key(true)]);
    let after = rings(&app);
    assert_eq!(after.len(), 10);
    assert_ne!(after, before, "~ places the rings anew");
    step(&mut app, vec![key(false)]);
    assert_eq!(rings(&app), after, "letting ~ go keeps the placement");
}

/// The events eframe's winit integration sends for a pen (Windows Ink) or a finger: a touch
/// with its pressure, and the mouse it stands in for (#491).
fn pen(phase: egui::TouchPhase, pos: Pos2, force: f32) -> Vec<egui::Event> {
    let touch = egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase, pos, force: Some(force) };
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    match phase {
        egui::TouchPhase::Start => vec![touch, egui::Event::PointerMoved(pos), button(true)],
        egui::TouchPhase::Move => vec![touch, egui::Event::PointerMoved(pos)],
        egui::TouchPhase::End | egui::TouchPhase::Cancel => vec![touch, button(false), egui::Event::PointerGone],
    }
}

/// An app with an 800 × 600 document whose host reads the pen itself (`Services::pen`), after its
/// first frame, with the pointer in the window.
fn pen_app() -> (VectorcraftApp, craft::ui::runtime::pen::PenSource, egui::Context) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let stylus = craft::ui::runtime::pen::PenSource::default();
    app.services.pen = Some(stylus.clone());
    app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(Pos2::new(400.0, 300.0))]);
    (app, stylus, ctx)
}

/// The pen's tip, with `pressure`.
fn tip(pressure: f32) -> Option<craft::behavior::PointerSample> {
    Some(craft::behavior::PointerSample::new(pressure))
}

/// The pen turned to its eraser end, with `pressure`.
fn eraser_end(pressure: f32) -> Option<craft::behavior::PointerSample> {
    let mut sample = craft::behavior::PointerSample::new(pressure);
    sample.eraser = true;
    Some(sample)
}

/// #764: turning the pen to its eraser end selects the Eraser, and turning it back to the tip
/// selects the tool before it, as the reference app's help describes ("Erase parts of a path
/// using a stylus pen"). Each turn acts once.
#[test]
fn the_eraser_end_selects_the_eraser_and_the_tip_brings_back_the_tool() {
    let (mut app, stylus, ctx) = pen_app();
    app.select_tool("paintbrush");
    stylus.set(tip(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "paintbrush", "the tip hovers");
    stylus.set(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser", "the eraser end hovers");
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser");
    stylus.set(tip(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "paintbrush", "the tip brings back the tool before");
    assert_eq!(app.ui.group_tool.get(vectorcraft_tools::catalog::group_of("paintbrush").unwrap()).map(String::as_str), Some("paintbrush"));

    // A turn acts once: a tool that becomes active while the eraser end stays in stays, and
    // the tip doesn't replace a tool other than the Eraser.
    stylus.set(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser");
    app.session.select_tool("pencil", app.view_info()).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "pencil");
    stylus.set(tip(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "pencil");
}

/// #764: the eraser end erases what it crosses, also when it comes in and presses in one
/// frame; the tool before it does not paint a stroke.
#[test]
fn the_eraser_end_erases() {
    let (mut app, stylus, ctx) = pen_app();
    app.session.execute("shape.rectangle", &json!({"x": 300, "y": 200, "width": 100, "height": 100})).unwrap();
    app.select_tool("paintbrush");
    let steps = app.session.journal.len();
    let xf = app.canvas_view().unwrap();
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let [a, b, c] = [280.0, 350.0, 420.0].map(|x| xf.to_screen(Point::new(x, 250.0)));
    stylus.set(eraser_end(0.6));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(a), button(a, true)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(b)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(c)]);
    frame(&mut app, &ctx, vec![button(c, false)]);
    let cmds: Vec<&str> = app.session.journal.since(steps).iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(cmds, ["path.eraseRegion"], "one Eraser step, no Paintbrush stroke");
    assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), 2, "the rectangle cut in two");
}

/// #764: a turn of the pen while a press is held takes effect after the release.
#[test]
fn a_flip_during_a_press_waits_for_the_release() {
    let (mut app, stylus, ctx) = pen_app();
    app.select_tool("paintbrush");
    let xf = app.canvas_view().unwrap();
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let [a, b, c] = [100.0, 150.0, 200.0].map(|x| xf.to_screen(Point::new(x, 100.0)));
    stylus.set(tip(0.5));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(a), button(a, true)]);
    stylus.set(eraser_end(0.5));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(b)]);
    assert_eq!(app.session.tool_id(), "paintbrush");
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(c), button(c, false)]);
    assert_eq!(app.session.tool_id(), "paintbrush");
    assert_eq!(app.session.journal.last().unwrap().0, "path.freehand", "the stroke ends with the Paintbrush");
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser", "the frame after the release");
}

/// #764: a tool chosen while the eraser end stands in (the toolbar and shortcuts call
/// `select_tool`; menus and MCP run `tool.select`) stays when the tip comes back.
#[test]
fn a_tool_chosen_while_the_eraser_end_stands_in_stays() {
    let (mut app, stylus, ctx) = pen_app();
    app.select_tool("paintbrush");
    stylus.set(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser");
    app.select_tool("rectangle");
    stylus.set(tip(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "rectangle");

    stylus.set(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser");
    app.run("tool.select", json!({"tool": "eraser"})).unwrap();
    stylus.set(tip(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser", "the Eraser chosen again stays too");
}

/// #764: a mouse moving while the eraser end stands in does not change the tool, and an Eraser
/// chosen before the eraser end comes in stays when the tip comes back.
#[test]
fn a_mouse_and_an_explicit_eraser_change_nothing() {
    let (mut app, stylus, ctx) = pen_app();
    app.select_tool("paintbrush");
    stylus.set(None);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "paintbrush", "a mouse");
    stylus.set(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    stylus.set(None);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser", "a mouse between the turns");
    stylus.set(tip(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "paintbrush");

    app.select_tool("eraser");
    stylus.set(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    stylus.set(tip(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser", "chosen before the eraser end came in");
}

/// #764: the host reads the pen over every window. A turn made while the pointer is outside
/// this window takes effect when the pointer comes in, and a turn there and back does not
/// change the tool.
#[test]
fn a_turn_over_another_window_waits_for_the_pointer() {
    let (mut app, stylus, ctx) = pen_app();
    app.select_tool("paintbrush");
    stylus.set(tip(0.0));
    frame(&mut app, &ctx, vec![]);
    let inside = egui::Event::PointerMoved(Pos2::new(400.0, 300.0));
    frame(&mut app, &ctx, vec![egui::Event::PointerGone]);
    stylus.set(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "paintbrush", "the pointer is in another window");
    frame(&mut app, &ctx, vec![inside.clone()]);
    assert_eq!(app.session.tool_id(), "eraser", "the pointer came in");

    frame(&mut app, &ctx, vec![egui::Event::PointerGone]);
    stylus.set(tip(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser");
    frame(&mut app, &ctx, vec![inside.clone()]);
    assert_eq!(app.session.tool_id(), "paintbrush");

    frame(&mut app, &ctx, vec![egui::Event::PointerGone]);
    stylus.set(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    stylus.set(tip(0.0));
    frame(&mut app, &ctx, vec![inside]);
    assert_eq!(app.session.tool_id(), "paintbrush", "turned there and back outside");
}

/// #764: the tip brings back the Perspective Grid tool as it was, with a grid hidden by Hide
/// Grid still hidden (choosing the tool afresh shows the grid).
#[test]
fn the_tip_brings_back_the_perspective_grid_tool_without_showing_the_grid() {
    let (mut app, stylus, ctx) = pen_app();
    let visible =
        |app: &VectorcraftApp| vectorcraft_tools::distort::perspective::PerspectiveGrid::current(&app.session.active().unwrap().doc).visible;
    app.select_tool("perspectiveGrid");
    assert!(visible(&app));
    app.session.execute("perspective.grid.show", &json!({"visible": false})).unwrap();
    stylus.set(tip(0.0));
    frame(&mut app, &ctx, vec![]);
    stylus.set(eraser_end(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "eraser");
    stylus.set(tip(0.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.tool_id(), "perspectiveGrid");
    assert!(!visible(&app), "the grid stays hidden");
}

/// #491: a pen tap whose press and lift come in one frame selects what it tapped, and the
/// lift (the pointer gone with it) doesn't drag the selection to the canvas's centre.
#[test]
fn a_pen_tap_within_one_frame_clicks_where_it_tapped() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    app.select_tool("selection");
    let before = app.session.active().unwrap().doc.art_bounds();
    let at = app.canvas_view().unwrap().to_screen(Point::new(150.0, 150.0));
    let mut tap = pen(egui::TouchPhase::Start, at, 0.5);
    tap.extend(pen(egui::TouchPhase::End, at, 0.5));
    frame(&mut app, &ctx, tap);
    frame(&mut app, &ctx, vec![]);
    let doc = app.session.active().unwrap();
    assert_eq!(doc.selection.objects, vec![vectorcraft_doc::NodeId(id)]);
    assert_eq!(doc.doc.art_bounds(), before, "the tap moved nothing");
}

/// The Contextual Task Bar shows Recolor Artwork (a palette) when the selection has colours,
/// and it opens the Recolor Artwork dialog; art painted with None has no button.
#[test]
fn the_task_bar_recolors_coloured_art() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 180, "y": 130, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.session.execute("paint.setFill", &json!({"ids": [id], "color": "#cc3322"})).unwrap();
    app.select_tool("selection");
    let st = app.session.active().unwrap();
    assert!(has_colors(&st.doc, &st.selection.objects));
    click(&mut app, RECOLOR);
    assert!(is_open::<crate::dialogs::recolor::RecolorDialog>(&app));

    app.session.execute("paint.setFill", &json!({"ids": [id], "none": true})).unwrap();
    app.session.execute("paint.setStroke", &json!({"ids": [id], "none": true})).unwrap();
    let st = app.session.active().unwrap();
    assert!(!has_colors(&st.doc, &st.selection.objects), "None paints have no colours");
    assert!(on_canvas(&mut app, |rig| rig.target(RECOLOR).is_none()), "no Recolor button without colours");
}

/// The Contextual Task Bar's Recolor Artwork button, by its name.
const RECOLOR: &str = "Recolor Artwork";

/// Recolor Artwork shows in the bars when any selected object has a color, whichever of them it is,
/// and not for an empty selection or ids that are gone.
#[test]
fn has_colors_looks_at_every_selected_object() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let mut rect = |x: i32| {
        vectorcraft_doc::NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap())
    };
    let (blank, colored) = (rect(10), rect(80));
    s.execute("paint.setFill", &json!({"ids": [blank.0], "none": true})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [blank.0], "none": true})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [colored.0], "color": "#cc3322"})).unwrap();
    let doc = &s.active().unwrap().doc;
    let gone = vectorcraft_doc::NodeId(u64::MAX);
    assert!(!has_colors(doc, &[]), "nothing selected");
    assert!(!has_colors(doc, &[gone]), "an id that is gone");
    assert!(!has_colors(doc, &[blank, gone]));
    assert!(has_colors(doc, &[blank, colored]), "the color is on the second object");
    assert!(has_colors(doc, &[colored, blank, gone]), "the color is on the first object");
}

/// The Contextual Task Bar's Lock button locks the selected member in isolation mode and the
/// selected group outside isolation mode.
#[test]
fn the_task_bar_lock_button_locks_selected_art_in_isolation_mode() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // Plain paths: a basic shape shows its properties in Lock's place.
    let square =
        |x: f64| json!({"anchors": [{"x": x, "y": 130}, {"x": x + 40.0, "y": 130}, {"x": x + 40.0, "y": 170}, {"x": x, "y": 170}], "closed": true});
    let a = app.session.execute("path.create", &square(150.0)).unwrap()["id"].clone();
    let b = app.session.execute("path.create", &square(210.0)).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [a.clone(), b.clone()]})).unwrap();
    let g = app.session.execute("object.group", &json!({})).unwrap()["id"].clone();
    app.select_tool("selection");
    let click_lock = |app: &mut VectorcraftApp| click(app, "Lock (⌘2)");
    let locked = |app: &VectorcraftApp, id: &serde_json::Value| {
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().locked
    };
    app.session.execute("object.isolate", &json!({"id": g.clone()})).unwrap();
    app.session.execute("select.set", &json!({"ids": [a.clone()]})).unwrap();
    click_lock(&mut app);
    assert!(locked(&app, &a), "the selected member is locked in isolation mode");
    assert!(!locked(&app, &b), "the other member stays unlocked");
    assert_eq!(app.ui.status, "", "no error");
    app.session.execute("object.exitIsolation", &json!({})).unwrap();
    app.session.execute("select.set", &json!({"ids": [g.clone()]})).unwrap();
    click_lock(&mut app);
    assert!(locked(&app, &g), "the group is locked outside isolation mode");
}

/// The Contextual Task Bar shows Image Trace first for a selected image: it traces the image
/// with the Default preset and leaves the Image Trace panel as it was. For the Image Trace
/// object it made, the first button is Expand, which keeps only the traced paths.
#[test]
fn the_task_bar_traces_an_image_then_expands_the_trace() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let img = image::RgbaImage::from_fn(20, 20, |x, _| image::Rgba(if x < 10 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
    let mut png = vec![];
    {
        let __dyn: image::DynamicImage = img.into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    app.run("file.place", json!({"name": "half.png", "dataBase64": data, "link": false, "at": [200, 150]})).unwrap();
    app.select_tool("selection");
    let first = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        st.selection.objects.first().and_then(|id| st.doc.node(*id)).cloned().unwrap()
    };
    let first_action = |app: &mut VectorcraftApp| offer(app).and_then(|o| o.actions.first().map(|a| a.0));
    assert!(matches!(first(&app).kind, NodeKind::Image(_)));
    assert_eq!(first_action(&mut app), Some("Image Trace"));
    click(&mut app, "Image Trace");
    assert_eq!(first(&app).name.as_deref(), Some("Image Trace"), "the image was traced");
    assert_eq!(app.ui.open_panel, None, "the Image Trace panel is left as it was");

    assert_eq!(first_action(&mut app), Some("Expand"));
    click(&mut app, "Expand");
    let expanded = first(&app);
    assert_ne!(expanded.name.as_deref(), Some("Image Trace"), "the trace was expanded");
    let mut images = 0;
    expanded.walk(&mut |n| images += usize::from(matches!(n.kind, NodeKind::Image(_))));
    assert_eq!(images, 0, "only the traced paths are kept");
}

/// For a selected image the Contextual Task Bar offers Remove Background, which clears the
/// background in the image's pixels at once (no dialog) with the command's defaults.
#[test]
fn the_task_bar_removes_a_selected_images_background() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let img =
        image::RgbaImage::from_fn(20, 20, |x, y| image::Rgba(if (6..14).contains(&x) && (6..14).contains(&y) { [200, 0, 0, 255] } else { [255; 4] }));
    let mut png = vec![];
    {
        let __dyn: image::DynamicImage = img.into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    app.run("file.place", json!({"name": "dot.png", "dataBase64": data, "link": false, "at": [200, 150]})).unwrap();
    app.select_tool("selection");
    click(&mut app, "Remove Background");
    assert!(!app.any_dialog_open(), "it runs at once");
    let st = app.session.active().unwrap();
    let Some(NodeKind::Image(im)) = st.selection.objects.first().and_then(|id| st.doc.node(*id)).map(|n| &n.kind) else { panic!("an image") };
    let px = craft_image_codecs::decode(&st.doc.images[&im.key].bytes)
        .map(|__cimg| {
            let (__w, __h) = __cimg.dimensions();
            match (__cimg.layout(), __cimg.sample_type()) {
                (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
            }
        })
        .unwrap()
        .to_rgba8();
    assert_eq!((px.get_pixel(0, 0).0[3], px.get_pixel(10, 10).0[3]), (0, 255), "the white background is cleared, the red dot kept");
}

/// A selected group's Contextual Task Bar offers Ungroup and Isolate. While an opacity mask is
/// edited, when Enter Isolation Mode is off, a group in the mask art gets Ungroup without Isolate.
#[test]
fn the_task_bar_leaves_isolate_out_while_an_opacity_mask_is_edited() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("selection");
    let rect = |app: &mut VectorcraftApp, x: f64| {
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 100, "width": 40, "height": 40})).unwrap()["id"].clone()
    };
    let group = |app: &mut VectorcraftApp, x: f64| {
        let (a, b) = (rect(app, x), rect(app, x + 50.0));
        app.session.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        app.session.execute("object.group", &json!({})).unwrap();
    };
    group(&mut app, 20.0);
    assert_eq!(task_bar_actions(&mut app), ["Ungroup", "Isolate", "Duplicate"]);
    let (obj, m) = (rect(&mut app, 200.0), rect(&mut app, 200.0));
    app.session.execute("transparency.makeOpacityMask", &json!({"ids": [obj, m]})).unwrap();
    app.session.execute("transparency.editOpacityMask", &json!({"id": obj})).unwrap();
    group(&mut app, 200.0);
    assert_eq!(task_bar_actions(&mut app), ["Ungroup", "Duplicate"]);
}

/// The names of the actions the Contextual Task Bar shows, in order; none while it isn't shown.
fn task_bar_actions(app: &mut VectorcraftApp) -> Vec<&'static str> {
    offer(app).map(|o| o.actions.iter().map(|a| a.0).collect()).unwrap_or_default()
}

/// The Artboard tool's Contextual Task Bar buttons act on its selected artboards: Resize opens
/// Artboard Options for the active one, Rearrange opens Rearrange for the selected artboards,
/// Export opens Export for Screens with the selected artboards checked, and Duplicate copies each
/// selected artboard in one undo step and selects the copies.
#[test]
fn the_artboard_tools_task_bar_buttons_act_on_the_selected_artboards() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    crate::panels::artboards::select(&mut app, 2);
    app.session.set_tool_option("selected", &json!([2, 0]));
    // The fields of the open dialog `D`, which the test then closes so the canvas takes the next click.
    fn fields_then_cancel<D: StackDialog>(app: &mut VectorcraftApp) -> serde_json::Value {
        let fields = app.dialogs().and_then(|d| d.with(|h| h.get::<D>().and_then(|d| d.json_fields()))).unwrap_or_default();
        cancel_dialog::<D>(app);
        fields
    }
    click(&mut app, "Resize");
    assert_eq!(fields_then_cancel::<crate::dialogs::artboard_options::ArtboardOptionsDialog>(&mut app)["index"], json!(2), "Artboard Options");
    click(&mut app, "Rearrange");
    assert_eq!(fields_then_cancel::<crate::dialogs::rearrange_artboards::RearrangeArtboardsDialog>(&mut app)["indices"], json!([0, 2]), "Rearrange");
    click(&mut app, "Export");
    assert_eq!(fields_then_cancel::<crate::dialogs::ExportForScreensDialog>(&mut app)["boards"], json!([true, false, true]), "Export for Screens");

    let boards =
        |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| (a.name.clone(), a.rect.size())).collect::<Vec<_>>();
    let before = boards(&app);
    click(&mut app, "Duplicate");
    assert!(!app.any_dialog_open());
    let after = boards(&app);
    assert_eq!(after[..3], before[..]);
    assert_eq!(after[3..], [("Artboard 1 copy".to_string(), before[0].1), ("Artboard 3 copy".to_string(), before[2].1)]);
    assert_eq!(app.session.tool_options()["selected"], json!([4, 3]), "the copies are selected, artboard 3's copy active");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(boards(&app), before, "one undo step");
}

/// With anchors direct-selected, the Contextual Task Bar offers Remove Anchor Points, Connect,
/// Cut Path and the two convert buttons. Converting to smooth and to
/// corner changes the selected anchor, Cut Path opens the path there and Connect closes it again,
/// as the Control bar's anchor buttons do.
#[test]
fn the_task_bar_connects_cuts_and_converts_direct_selected_anchors() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 150, "y": 100, "width": 100, "height": 80})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("directSelection");
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 1]]})).unwrap();
    let buttons = task_bar_actions(&mut app);
    let expected = [
        "Remove Anchor Points",
        "Connect",
        "Cut Path",
        "Convert Selected Anchor Points to Corner",
        "Convert Selected Anchor Points to Smooth",
        "Duplicate",
    ];
    assert_eq!(buttons[..expected.len()], expected, "{buttons:?}");
    let subpath =
        |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().path_data().unwrap().subpaths[0].clone();
    let last_undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.last().unwrap().label.clone();
    click(&mut app, "Convert Selected Anchor Points to Smooth");
    let a = subpath(&app).anchors[1];
    assert!(a.has_in() && a.has_out(), "smooth: {a:?}");
    assert_eq!(last_undo(&app), "Convert Anchor Points");
    click(&mut app, "Convert Selected Anchor Points to Corner");
    let a = subpath(&app).anchors[1];
    assert!(!a.has_in() && !a.has_out(), "corner: {a:?}");
    click(&mut app, "Cut Path");
    assert_eq!(last_undo(&app), "Cut Path");
    let sp = subpath(&app);
    assert!(!sp.closed && sp.anchors.len() == 5, "the rectangle opens at the anchor: {sp:?}");
    click(&mut app, "Connect");
    assert_eq!(last_undo(&app), "Join");
    let sp = subpath(&app);
    assert!(sp.closed && sp.anchors.len() == 4, "the ends join again: {sp:?}");
}

#[test]
fn ruler_guide_origin_display_fallback_accepts_only_bounded_finite_pairs() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    // The global zero point as stored (artboard rulers check their own, see `ruler_origin_for`).
    app.run("view.globalRulers", json!({"global": true})).unwrap();
    let max = vectorcraft_engine::MAX_COORD;
    for (origin, expected) in [
        (Point::ZERO, Point::ZERO),
        (Point::new(17.0, -23.0), Point::new(17.0, -23.0)),
        (Point::new(max, -max), Point::new(max, -max)),
        (Point::new(f64::NAN, 23.0), Point::ZERO),
        (Point::new(17.0, f64::INFINITY), Point::ZERO),
        (Point::new(max + 1.0, 23.0), Point::ZERO),
        (Point::new(17.0, -max - 1.0), Point::ZERO),
    ] {
        std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).ruler_origin = origin;
        assert_eq!(app.ruler_origin(), expected);
    }
}

#[test]
fn live_trace_selection_highlight_excludes_the_retained_source_rectangle() {
    use std::sync::Arc;
    use vectorcraft_doc::{Appearance, ImageObject, NodeId, TraceView};
    let mut source = Node::new(
        NodeId(2),
        NodeKind::Image(Box::new(ImageObject {
            key: "synthetic".into(),
            width: 200,
            height: 200,
            xf: Affine::IDENTITY,
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    source.visible = false;
    let art = Node::path(NodeId(3), vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(20.0, 30.0, 80.0, 90.0)), Appearance::default());
    let mut group = Node::group(NodeId(1), vec![Arc::new(source), Arc::new(art)]);
    let doc = vectorcraft_doc::Document::new(300.0, 300.0);
    let symbols = doc.symbol_boxes();
    for view in TraceView::ALL {
        group.extras_mut().trace = Some(Box::new(json!({"view":view.id()})));
        let mut visited = vec![];
        walk_drawn(&group, &mut |n| visited.push(n.id));
        assert_eq!(visited.contains(&NodeId(2)), view.shows_source());
        let want = if view.shows_source() {
            vectorcraft_geom::Rect::new(0.0, 0.0, 200.0, 200.0)
        } else {
            vectorcraft_geom::Rect::new(20.0, 30.0, 80.0, 90.0)
        };
        assert_eq!(node_outline(&group, &symbols).bounding_box(), want);
    }
    group.extras_mut().trace = None;
    assert_eq!(node_outline(&group, &symbols).bounding_box(), vectorcraft_geom::Rect::new(0.0, 0.0, 200.0, 200.0));
}

/// #2572: Show Text Threads links a thread's frames from the out port of one to the in port of
/// the next, where the tools draw those ports, each with the arrow of a threaded port; nothing
/// is drawn while no frame of the thread is selected.
#[test]
fn text_threads_run_from_each_out_port_to_the_next_in_port() {
    use vectorcraft_tools::text::{Port, port_centre};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let text = "Words that fill more than the frame holds. ".repeat(10);
    let a = app.run("text.create", json!({"x": 20, "y": 20, "text": text, "area": {"width": 150, "height": 60}})).unwrap()["id"].clone();
    let b = app.run("text.thread.add", json!({"from": a, "at": [300, 200]})).unwrap()["id"].clone();
    let frame = |app: &VectorcraftApp, id: &serde_json::Value| match &app
        .session
        .active()
        .unwrap()
        .doc
        .node(vectorcraft_doc::NodeId(id.as_u64().unwrap()))
        .unwrap()
        .kind
    {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    };
    let zoom = 2.0;
    let (out, input) = (port_centre(&frame(&app, &a), Port::Out, 0.5).unwrap(), port_centre(&frame(&app, &b), Port::In, 0.5).unwrap());
    let o = thread_overlays(&app, zoom);
    assert!(o.iter().any(|x| matches!(x, Overlay::Line { a, b, .. } if *a == out && *b == input)), "the thread line: {o:?}");
    let boxes: Vec<Point> =
        o.iter().filter_map(|x| if let Overlay::Highlight { quad, .. } = x { Some(quad[0].midpoint(quad[2])) } else { None }).collect();
    assert_eq!(boxes.len(), 2);
    assert!(boxes[0].distance(out) < 1e-9 && boxes[1].distance(input) < 1e-9, "{boxes:?}");
    // Two arrows: three strokes each, beside the thread line.
    assert_eq!(o.iter().filter(|x| matches!(x, Overlay::Line { .. })).count(), 7);
    app.run("select.none", json!({})).unwrap();
    assert!(thread_overlays(&app, zoom).is_empty());
}

#[test]
fn text_thread_overlays_keep_document_order_with_mixed_selection() {
    use vectorcraft_doc::NodeId;
    use vectorcraft_tools::text::{Port, port_centre};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let text = "Words that fill more than the frame holds. ".repeat(10);
    let id = |value: serde_json::Value| NodeId(value["id"].as_u64().unwrap());
    let first = id(app.run("text.create", json!({"x": 20, "y": 20, "text": text, "area": {"width": 150, "height": 60}})).unwrap());
    let first_next = id(app.run("text.thread.add", json!({"from": first.0, "at": [300, 20]})).unwrap());
    let second = id(app
        .run(
            "text.create",
            json!({"x": 20, "y": 200, "text": "Words that fill more than the frame holds. ".repeat(10), "area": {"width": 150, "height": 60}}),
        )
        .unwrap());
    let second_next = id(app.run("text.thread.add", json!({"from": second.0, "at": [300, 200]})).unwrap());
    let unselected = id(app
        .run("text.create", json!({"x": 20, "y": 100, "text": "An unselected story. ".repeat(20), "area": {"width": 150, "height": 60}}))
        .unwrap());
    let unselected_next = id(app.run("text.thread.add", json!({"from": unselected.0, "at": [300, 100]})).unwrap());
    let unrelated_a = id(app.run("shape.rectangle", json!({"x": 450, "y": 300, "width": 20, "height": 20})).unwrap());
    let unrelated_b = id(app.run("shape.rectangle", json!({"x": 480, "y": 300, "width": 20, "height": 20})).unwrap());
    app.run("select.set", json!({"ids": [unrelated_a.0, unrelated_b.0, second_next.0, first.0]})).unwrap();

    let state = app.session.active().unwrap();
    assert_eq!(state.selection.objects, [unrelated_a, unrelated_b, second_next, first]);
    assert_eq!(state.doc.text_threads, [vec![first, first_next], vec![second, second_next], vec![unselected, unselected_next]]);
    let frame = |id| match &state.doc.node(id).unwrap().kind {
        NodeKind::Text(text) => (**text).clone(),
        _ => panic!("thread member is text"),
    };
    let px = 1.0;
    let expected = [
        (port_centre(&frame(first), Port::Out, px).unwrap(), port_centre(&frame(first_next), Port::In, px).unwrap()),
        (port_centre(&frame(second), Port::Out, px).unwrap(), port_centre(&frame(second_next), Port::In, px).unwrap()),
    ];
    let overlays = thread_overlays(&app, 1.0);
    let lines: Vec<_> = overlays
        .iter()
        .filter_map(|overlay| match overlay {
            Overlay::Line { a, b, .. } if expected.iter().any(|(from, to)| *a == *from && *b == *to) => Some((*a, *b)),
            _ => None,
        })
        .collect();
    assert_eq!(lines, expected, "thread lines follow document order, not selection order");
    assert_eq!(overlays.iter().filter(|overlay| matches!(overlay, Overlay::Line { .. })).count(), 14, "two thread links and their port arrows only");
    let ports: Vec<_> = overlays
        .iter()
        .filter_map(|overlay| match overlay {
            Overlay::Highlight { quad, .. } => Some(quad[0].midpoint(quad[2])),
            _ => None,
        })
        .collect();
    assert_eq!(ports, expected.into_iter().flat_map(|(from, to)| [from, to]).collect::<Vec<_>>());
}

/// Two document-order threads hit the first two selected IDs among 512 unrelated paths.
/// Their real port geometry and document-order output remain unchanged.
#[test]
fn text_thread_overlays_keep_two_early_hits_with_many_unrelated_paths() {
    use std::sync::Arc;
    use vectorcraft_doc::NodeId;
    use vectorcraft_tools::text::{Port, port_centre, ports_shown};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let text = "Words that fill more than the frame holds. ".repeat(10);
    let id = |value: serde_json::Value| NodeId(value["id"].as_u64().unwrap());
    let mut threads = vec![];
    for y in [20, 200] {
        let first = id(app.run("text.create", json!({"x": 20, "y": y, "text": text, "area": {"width": 150, "height": 60}})).unwrap());
        let next = id(app.run("text.thread.add", json!({"from": first.0, "at": [300, y]})).unwrap());
        threads.push([first, next]);
    }
    let path = id(app.run("shape.rectangle", json!({"x": 450, "y": 300, "width": 20, "height": 20})).unwrap());
    let mut selected = vec![threads[0][0], threads[1][0], path];
    {
        let st = app.session.active_mut().unwrap();
        let doc = Arc::make_mut(&mut st.doc);
        let template = doc.node(path).unwrap().clone();
        let mut additions = Vec::with_capacity(511);
        for _ in 1..512 {
            let mut p = template.clone();
            p.id = doc.alloc_id();
            selected.push(p.id);
            additions.push(Arc::new(p));
        }
        Arc::make_mut(&mut doc.layers[0]).children_mut().unwrap().extend(additions);
        doc.drop_lookup_cache();
    }
    app.run("select.set", json!({"ids": selected.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.text_threads, threads.iter().map(|t| t.to_vec()).collect::<Vec<_>>());
    assert_eq!(st.selection.objects, selected);
    assert_eq!(st.selection.objects.len(), 514);
    assert!(ports_shown(&st.doc, &st.selection.objects));
    let frame = |id| match &st.doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("thread member is text"),
    };
    let expected: Vec<_> =
        threads.iter().map(|[a, b]| (port_centre(&frame(*a), Port::Out, 1.0).unwrap(), port_centre(&frame(*b), Port::In, 1.0).unwrap())).collect();
    let overlays = thread_overlays(&app, 1.0);
    let links: Vec<_> = overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Line { a, b, .. } if expected.contains(&(*a, *b)) => Some((*a, *b)),
            _ => None,
        })
        .collect();
    assert_eq!(links, expected, "the two thread links stay in document order");
    assert_eq!(overlays.iter().filter(|o| matches!(o, Overlay::Line { .. })).count(), 14, "two links and their port arrows only");
    let ports: Vec<_> = overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Highlight { quad, .. } => Some(quad[0].midpoint(quad[2])),
            _ => None,
        })
        .collect();
    assert_eq!(ports, expected.into_iter().flat_map(|(a, b)| [a, b]).collect::<Vec<_>>());
}

/// One two-frame thread misses a selection of 512 unrelated paths.
/// This checks ordinary empty output, not elapsed time or allocation.
#[test]
fn text_thread_overlays_skip_one_unselected_thread_with_many_unrelated_paths() {
    use std::sync::Arc;
    use vectorcraft_doc::NodeId;
    use vectorcraft_tools::text::{ported, ports_shown};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let text = "Words that fill more than the frame holds. ".repeat(10);
    let id = |value: serde_json::Value| NodeId(value["id"].as_u64().unwrap());
    let first = id(app.run("text.create", json!({"x": 20, "y": 20, "text": text, "area": {"width": 150, "height": 60}})).unwrap());
    let next = id(app.run("text.thread.add", json!({"from": first.0, "at": [300, 20]})).unwrap());
    let path = id(app.run("shape.rectangle", json!({"x": 450, "y": 300, "width": 20, "height": 20})).unwrap());
    let mut selected = vec![path];
    {
        let st = app.session.active_mut().unwrap();
        let doc = Arc::make_mut(&mut st.doc);
        let template = doc.node(path).unwrap().clone();
        let mut additions = Vec::with_capacity(511);
        for _ in 1..512 {
            let mut p = template.clone();
            p.id = doc.alloc_id();
            selected.push(p.id);
            additions.push(Arc::new(p));
        }
        Arc::make_mut(&mut doc.layers[0]).children_mut().unwrap().extend(additions);
        doc.drop_lookup_cache();
    }
    app.run("select.set", json!({"ids": selected.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.text_threads, [vec![first, next]]);
    assert_eq!(st.selection.objects, selected);
    assert_eq!(st.selection.objects.len(), 512);
    assert!(selected.iter().all(|id| *id != first && *id != next));
    assert!([first, next].into_iter().all(|id| ported(&st.doc, id).is_some()), "both thread frames have ports");
    assert!(ports_shown(&st.doc, &st.selection.objects));
    assert!(thread_overlays(&app, 1.0).is_empty());
}

/// A double-click with the Selection tool on the out port of a threaded frame breaks the
/// thread there: no new frame is threaded on, and the story flows back into the first frame.
#[test]
fn a_double_click_on_a_threaded_out_port_breaks_the_thread() {
    use vectorcraft_tools::text::{Port, port_centre};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let text = "Words that fill more than the frame holds. ".repeat(10);
    let a = app.run("text.create", json!({"x": 20, "y": 20, "text": text, "area": {"width": 150, "height": 60}})).unwrap()["id"].as_u64().unwrap();
    app.run("text.thread.add", json!({"from": a, "at": [300, 200]})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    app.select_tool("selection");
    let doc = |app: &VectorcraftApp| app.session.active().unwrap().doc.clone();
    let before = doc(&app);
    let NodeKind::Text(t) = &before.node(vectorcraft_doc::NodeId(a)).unwrap().kind else { panic!("not text") };
    let view = app.view_info();
    let at = port_centre(t, Port::Out, 1.0 / view.zoom).unwrap();
    let count = doc(&app).layers[0].children().map_or(0, |c| c.len());
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
        crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, at.x, at.y), view);
    }
    let d = doc(&app);
    assert!(d.text_threads.is_empty(), "the thread is broken");
    assert_eq!(d.layers[0].children().map_or(0, |c| c.len()), count, "no new frame");
    let NodeKind::Text(t) = &d.node(vectorcraft_doc::NodeId(a)).unwrap().kind else { panic!("not text") };
    assert_eq!(t.plain_text(), text, "the story flows into the first frame");
    assert_eq!(app.session.tool_id(), "selection");
}

/// With more than `PORTS_MAX` type objects selected the tools draw no ports, and Show Text
/// Threads draws only its lines, no port boxes.
#[test]
fn text_threads_of_a_huge_selection_have_lines_but_no_ports() {
    use vectorcraft_tools::text::PORTS_MAX;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let text = "Words that fill more than the frame holds. ".repeat(10);
    let a = app.run("text.create", json!({"x": 20, "y": 20, "text": text, "area": {"width": 150, "height": 60}})).unwrap()["id"].clone();
    app.run("text.thread.add", json!({"from": a, "at": [300, 200]})).unwrap();
    app.run("text.create", json!({"x": 20, "y": 300, "text": "x", "area": {"width": 50, "height": 20}})).unwrap();
    while app.session.active().unwrap().doc.layers[0].children().map_or(0, |c| c.len()) <= PORTS_MAX {
        app.run("select.all", json!({})).unwrap();
        app.run("edit.duplicate", json!({})).unwrap();
    }
    app.run("select.all", json!({})).unwrap();
    let o = thread_overlays(&app, 1.0);
    assert!(o.iter().any(|x| matches!(x, Overlay::Line { .. })), "the thread lines");
    assert!(!o.iter().any(|x| matches!(x, Overlay::Highlight { .. } | Overlay::Path { .. })), "no port boxes");
}

/// A frame of a thread in perspective has no ports: Show Text Threads draws no port box on it
/// and no line to or from it; the links between frames with ports keep theirs.
#[test]
fn text_threads_skip_the_links_of_a_frame_in_perspective() {
    use vectorcraft_tools::text::{Port, port_centre};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let text = "Words that fill more than the frame holds. ".repeat(20);
    let id = |v: serde_json::Value| vectorcraft_doc::NodeId(v["id"].as_u64().unwrap());
    let a = id(app.run("text.create", json!({"x": 20, "y": 20, "text": text, "area": {"width": 150, "height": 60}})).unwrap());
    let b = id(app.run("text.thread.add", json!({"from": a.0, "at": [200, 20]})).unwrap());
    let c = id(app.run("text.thread.add", json!({"from": b.0, "at": [20, 200]})).unwrap());
    let d = id(app.run("text.thread.add", json!({"from": c.0, "at": [200, 200]})).unwrap());
    app.session
        .edit("Perspective", |doc, _| {
            if let Some(n) = doc.node_mut(b) {
                n.extras_mut().perspective = Some(Box::new(vectorcraft_doc::PerspectiveAttachment::new("left", 0.0)));
            }
            Ok(())
        })
        .unwrap();
    app.run("select.set", json!({"ids": [a.0, b.0, c.0, d.0]})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.text_threads, [vec![a, b, c, d]], "b is still threaded");
    let frame = |id| match &st.doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    };
    let (out, input) = (port_centre(&frame(c), Port::Out, 1.0).unwrap(), port_centre(&frame(d), Port::In, 1.0).unwrap());
    let o = thread_overlays(&app, 1.0);
    let boxes: Vec<Point> =
        o.iter().filter_map(|x| if let Overlay::Highlight { quad, .. } = x { Some(quad[0].midpoint(quad[2])) } else { None }).collect();
    assert_eq!(boxes.len(), 2, "only c's out port and d's in port: {boxes:?}");
    assert!(boxes[0].distance(out) < 1e-9 && boxes[1].distance(input) < 1e-9, "{boxes:?}");
    // One thread line (c to d) and its two arrows of three strokes each.
    assert_eq!(o.iter().filter(|x| matches!(x, Overlay::Line { .. })).count(), 7);
}

/// #4681: a click on another object moves the Contextual Task Bar to it without hiding it for the
/// press (it flashed); a drag hides it until the drag ends.
#[test]
fn the_task_bar_stays_up_through_a_click_and_gives_way_to_a_drag() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 40, "y": 60, "width": 60, "height": 60})).unwrap();
    app.run("shape.rectangle", json!({"x": 260, "y": 60, "width": 60, "height": 60})).unwrap();
    app.select_tool("selection");
    on_canvas(&mut app, |rig| {
        let first = rig.target(BAR).expect("the task bar shows under the second rectangle").rect;
        let at = rig.app().canvas_view().unwrap().to_screen(Point::new(70.0, 90.0));
        let press = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() };
        rig.fx.frame(vec![egui::Event::PointerMoved(at), press]);
        rig.fx.frame(vec![]);
        assert!(rig.app().session.tool_busy(), "the press is under way");
        let pressed = rig.fx.by_label(BAR).expect("the task bar stays up through the press").rect;
        assert!(pressed.center().x < first.center().x - 50.0, "it moved under the rectangle pressed: {pressed:?} from {first:?}");
        // Dragged, it gives way until the drag ends.
        let to = at + vec2(40.0, 30.0);
        rig.fx.frame(vec![egui::Event::PointerMoved(at + vec2(20.0, 15.0))]);
        rig.fx.frame(vec![egui::Event::PointerMoved(to)]);
        rig.fx.frame(vec![]);
        assert!(rig.fx.by_label(BAR).is_none(), "hidden while the object is dragged");
        rig.fx.release_at(to);
        assert!(rig.fx.by_label(BAR).is_some(), "back once the drag ends");
    });
}

/// The fade is drawn only on the canvas: exports with the Artboard tool active and the fade on are
/// the same as with the Selection tool and the fade off.
#[test]
fn the_pasteboard_fade_leaves_exports_unchanged() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": -40, "y": 20, "width": 120, "height": 40})).unwrap();
    let exports = |app: &mut VectorcraftApp| -> Vec<serde_json::Value> {
        ["svg", "png"].iter().map(|f| app.run("document.export", json!({"format": f})).unwrap()).collect()
    };
    app.run("prefs.set", json!({"key": "fadeRegionOutsideArtboard", "value": false})).unwrap();
    let plain = exports(&mut app);
    app.run("prefs.set", json!({"key": "fadeRegionOutsideArtboard", "value": true})).unwrap();
    app.select_tool("artboard");
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    assert!(super::artboards::fades_outside_artboards(&app));
    assert_eq!(exports(&mut app), plain);
}

/// Finished worker renders are uploaded whenever they are not older than what is currently
/// displayed on screen. During continuous edit streams (drag previews, held arrow nudges, or
/// command bursts) where the document revision advances every frame, intermediate worker frames
/// are uploaded progressively to prevent the canvas from freezing. Obsolete frames older than what
/// is already displayed are discarded.
#[test]
fn poll_worker_accepts_frames_not_older_than_displayed_and_discards_obsolete_frames() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true;
    let ctx = crate::testing::ui_context();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw.clone(), |ui| show(&mut app, ui));
    out.textures_delta.clear();

    let doc = app.session.active().unwrap().doc.clone();
    let initial_key = app.canvas.key.clone().expect("first frame initializes key");
    let key_doc = initial_key.doc;

    // Displayed canvas texture is initially at revision 2.
    let mut displayed_key = initial_key.clone();
    displayed_key.revision = 2;
    app.canvas.key = Some(displayed_key);

    let (worker, _jobs, done_tx) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);

    // Worker completes revision 5 (intermediate frame from an edit stream).
    // It is newer than displayed revision 2: it MUST be accepted and uploaded.
    let mut rev5_key = initial_key.clone();
    rev5_key.revision = 5;
    let img5 = vectorcraft_render::Rendered { width: 10, height: 10, pixels: vec![200; 400] };
    done_tx
        .send(crate::render_worker::Done {
            key: rev5_key,
            doc: doc.clone(),
            opts: vectorcraft_render::RenderOptions::default(),
            img: img5,
            ms: 2.5,
            moved: 0,
            reused: false,
        })
        .unwrap();

    let mut out = ctx.run_ui(raw.clone(), |ui| poll_worker(&mut app, ui, key_doc));
    let set_count = out.textures_delta.set.len();
    out.textures_delta.clear();
    assert_eq!(set_count, 1, "intermediate frame 5 is uploaded to texture");
    assert_eq!(app.canvas.key.as_ref().unwrap().revision, 5, "intermediate frame 5 is uploaded");
    assert!(app.canvas.from_worker);

    // Worker completes an older revision 3 (out of order). It is older than displayed 5, so discarded.
    let mut rev3_key = initial_key.clone();
    rev3_key.revision = 3;
    let img3 = vectorcraft_render::Rendered { width: 10, height: 10, pixels: vec![150; 400] };
    done_tx
        .send(crate::render_worker::Done {
            key: rev3_key,
            doc: doc.clone(),
            opts: vectorcraft_render::RenderOptions::default(),
            img: img3,
            ms: 2.5,
            moved: 0,
            reused: false,
        })
        .unwrap();

    let mut out = ctx.run_ui(raw.clone(), |ui| poll_worker(&mut app, ui, key_doc));
    let set_count = out.textures_delta.set.len();
    out.textures_delta.clear();
    assert_eq!(set_count, 0, "older out-of-order frame 3 emits NO texture upload delta, leaving displayed texture intact");
    assert_eq!(app.canvas.key.as_ref().unwrap().revision, 5, "older out-of-order frame 3 is discarded");

    // Worker completes another intermediate revision 8. Newer than displayed 5, so uploaded.
    let mut rev8_key = initial_key.clone();
    rev8_key.revision = 8;
    let img8 = vectorcraft_render::Rendered { width: 10, height: 10, pixels: vec![180; 400] };
    done_tx
        .send(crate::render_worker::Done {
            key: rev8_key,
            doc: doc.clone(),
            opts: vectorcraft_render::RenderOptions::default(),
            img: img8,
            ms: 2.5,
            moved: 0,
            reused: false,
        })
        .unwrap();

    let mut out = ctx.run_ui(raw.clone(), |ui| poll_worker(&mut app, ui, key_doc));
    let set_count = out.textures_delta.set.len();
    out.textures_delta.clear();
    assert_eq!(set_count, 1, "newer frame 8 is uploaded to texture");
    assert_eq!(app.canvas.key.as_ref().unwrap().revision, 8, "newer frame 8 is uploaded");

    // Finally, the settling frame at revision 10 completes: it is accepted and uploaded.
    let mut rev10_key = initial_key;
    rev10_key.revision = 10;
    let img10 = vectorcraft_render::Rendered { width: 10, height: 10, pixels: vec![255; 400] };
    done_tx
        .send(crate::render_worker::Done {
            key: rev10_key,
            doc: doc.clone(),
            opts: vectorcraft_render::RenderOptions::default(),
            img: img10,
            ms: 2.5,
            moved: 0,
            reused: false,
        })
        .unwrap();

    let mut out = ctx.run_ui(raw, |ui| poll_worker(&mut app, ui, key_doc));
    let set_count = out.textures_delta.set.len();
    out.textures_delta.clear();
    assert_eq!(set_count, 1, "settled frame 10 is uploaded to texture");
    assert_eq!(app.canvas.key.as_ref().unwrap().revision, 10, "settled frame 10 is uploaded");
}

/// The Contour bar's corner styles and Bake, by name.
const CONTOUR_CONTROLS: [&str; 4] = ["Sharp", "Round", "Square", "Bake"];

/// The Contour bar shows with the Contour tool and a shape selected: Sharp, Round and Square set the
/// corner style (of a selected Contour at once, else of the next one), and Bake turns a Contour into
/// plain paths.
#[test]
fn the_contour_bar_sets_corners_and_bakes() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 80, "height": 60})).unwrap();
    let shown = |app: &mut VectorcraftApp| on_canvas(app, |rig| CONTOUR_CONTROLS.into_iter().filter(|n| rig.target(n).is_some()).collect::<Vec<_>>());

    // Only the Contour tool shows it.
    app.select_tool("selection");
    assert!(shown(&mut app).is_empty());
    app.select_tool("contour");
    assert_eq!(shown(&mut app), CONTOUR_CONTROLS);
    let offsets = |app: &VectorcraftApp| {
        let mut found = vec![];
        app.session.active().unwrap().doc.walk(|n| {
            if let Some(e) = n.appearance.effects.iter().find(|e| e.id == "path.offsetPath") {
                found.push(e.params["joins"].as_str().unwrap_or_default().to_string());
            }
        });
        found
    };

    // No Contour yet: Round is kept for the next one, and Bake does nothing.
    let steps = app.session.active().unwrap().history.undo.len();
    click(&mut app, "Round");
    assert_eq!(app.session.tool_options()["corners"], json!("round"));
    click(&mut app, "Bake");
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps, "nothing to bake");
    assert!(offsets(&app).is_empty());

    // A Contour made now has round corners; Square changes the selected Contour in place.
    app.session.execute("addon.contour.set", &json!({"offset": 8, "corners": "round"})).unwrap();
    assert_eq!(offsets(&app), ["round"]);
    click(&mut app, "Square");
    assert_eq!(offsets(&app), ["bevel"], "the selected Contour changed, none was added");

    // Bake: the live effect is gone and the art stays.
    click(&mut app, "Bake");
    assert!(offsets(&app).is_empty(), "baked");
}

/// A frame the render worker finished for `job`: white pixels of the job's size.
fn finished(job: &crate::render_worker::Job) -> crate::render_worker::Done {
    let img = vectorcraft_render::Rendered { width: job.w, height: job.h, pixels: vec![255; job.w as usize * job.h as usize * 4] };
    crate::render_worker::Done { key: job.key.clone(), doc: job.doc.clone(), opts: job.opts.clone(), img, ms: 40.0, moved: 0, reused: false }
}

/// A job for the render worker to pan over a document of `doc`: a view at 100% moved by `(dx, dy)`
/// device pixels, in a 256×128 frame.
fn pan_job(doc: &std::sync::Arc<vectorcraft_doc::Document>, key: &CacheKey, (dx, dy): (f64, f64)) -> crate::render_worker::Job {
    crate::render_worker::Job {
        key: CacheKey { cx: 128.0 - dx, cy: 64.0 - dy, w: 256, h: 128, ..key.clone() },
        doc: doc.clone(),
        w: 256,
        h: 128,
        view: Affine::translate((dx, dy)),
        opts: vectorcraft_render::RenderOptions::default(),
        scope: vectorcraft_engine::file_access::ScopeHandle::capture(),
        effects: Default::default(),
        may_copy: true,
        may_keep: true,
        display: None,
    }
}

/// The document and the canvas key the render worker tests start from: rectangles on whole pixels.
fn pan_doc() -> (std::sync::Arc<vectorcraft_doc::Document>, CacheKey) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 256, "height": 128})).unwrap();
    for (x, color) in [(16, "#c02040"), (96, "#2040c0"), (176, "#20a040")] {
        let id = app.run("shape.rectangle", json!({"x": x, "y": 16, "width": 64, "height": 96})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [id], "color": color})).unwrap();
    }
    app.canvas.worker_started = true;
    shapes(&mut app, &crate::testing::ui_context());
    let key = app.canvas.key.clone().expect("a frame was rendered");
    (app.session.active().unwrap().doc.clone(), CacheKey { zoom: 1.0, rot: 0.0, ppp: 1.0, pixel: None, ..key })
}

/// A pan by whole device pixels takes the pixels that stay on screen from the last frame: a mark
/// put in that frame moves with the pan, and the frame matches a whole render where the mark is not.
#[test]
fn a_whole_pixel_pan_copies_the_pixels_that_stay_on_screen() {
    keeping_color_settings(a_whole_pixel_pan);
}

fn a_whole_pixel_pan() {
    let (doc, key) = pan_doc();
    for pan in [(10.0, 0.0), (-10.0, 0.0), (0.0, 7.0), (0.0, -7.0), (-13.0, 5.0)] {
        let mut r = vectorcraft_render::Renderer::new();
        let mut kept = None;
        let first = render_job(&mut r, &mut kept, &pan_job(&doc, &key, (0.0, 0.0)));
        assert!(!first.reused, "the first frame renders whole");
        let k = kept.as_mut().and_then(|k| k.pixels.as_mut()).expect("the frame holds its pixels");
        // The middle of the frame, which stays on screen.
        k[(64 * 256 + 128) * 4..(64 * 256 + 128) * 4 + 4].copy_from_slice(&[1, 2, 3, 4]);
        let job = pan_job(&doc, &key, pan);
        let frame = render_job(&mut r, &mut kept, &job);
        assert!(frame.reused, "{pan:?}");
        let at = ((64 + pan.1 as i64) * 256 + 128 + pan.0 as i64) as usize * 4;
        assert_eq!(frame.img.pixels[at..at + 4], [1, 2, 3, 4], "{pan:?}: the marked pixel moved with the art");
        let whole = vectorcraft_render::Renderer::new().render(&doc, 256, 128, job.view, &job.opts);
        let mut img = frame.img.pixels;
        img[at..at + 4].copy_from_slice(&whole.pixels[at..at + 4]);
        assert_eq!(img, whole.pixels, "{pan:?}: the same picture as a whole render");
    }
}

/// Installing, replacing or removing a plug-in draws a document with a plug-in effect again, and a
/// pan from the frame drawn before the change renders whole instead of copying its pixels. The key
/// of a document without a plug-in effect does not follow the plug-ins. The other tests share the
/// plug-in registry, and the plug-in is removed after.
#[test]
fn a_plug_in_change_draws_the_document_s_plug_in_effects_again() {
    keeping_color_settings(a_plug_in_change);
}

fn a_plug_in_change() {
    use vectorcraft_plugins::wat as tpl;
    let id = "test.canvas.redraw";
    let module = vectorcraft_format::base64_encode(&wat::parse_str(tpl::module(&tpl::manifest(id, "effect", "{}"), tpl::ECHO, "", 1)).unwrap());
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("plugin.install", json!({"dataBase64": module})).unwrap();
    app.session.execute("file.new", &json!({"width": 256, "height": 128})).unwrap();
    let rect = app.run("shape.rectangle", json!({"x": 16, "y": 16, "width": 64, "height": 96})).unwrap()["id"].clone();
    app.canvas.worker_started = true; // render on this thread
    let ctx = crate::testing::ui_context();
    shapes(&mut app, &ctx);
    // Other keys' fields (placed documents' bitmaps) follow what other tests do; this one's don't.
    let plugins = |app: &VectorcraftApp| app.canvas.key.as_ref().expect("a frame was rendered").plugins;
    let plain = plugins(&app);
    app.run("plugin.install", json!({"dataBase64": module})).unwrap();
    shapes(&mut app, &ctx);
    assert_eq!(plugins(&app), plain, "no plug-in effect, so nothing to draw again");
    app.run("effect.apply", json!({"effect": vectorcraft_plugins::effect::effect_id(id), "ids": [rect]})).unwrap();
    shapes(&mut app, &ctx);
    let before = app.canvas.key.clone().expect("a frame was rendered");
    app.run("plugin.install", json!({"dataBase64": module})).unwrap();
    shapes(&mut app, &ctx);
    let after = plugins(&app);
    assert_ne!(after, before.plugins, "drawn again once the plug-in was replaced");
    // The worker's frame from before the change gives no pixels to the next pan.
    let doc = app.session.active().unwrap().doc.clone();
    let at_100 = CacheKey { zoom: 1.0, rot: 0.0, ppp: 1.0, pixel: None, ..before };
    let mut r = vectorcraft_render::Renderer::new();
    let mut kept = None;
    render_job(&mut r, &mut kept, &pan_job(&doc, &at_100, (0.0, 0.0)));
    let pan = pan_job(&doc, &CacheKey { plugins: after, ..at_100.clone() }, (10.0, 0.0));
    assert!(!render_job(&mut r, &mut kept, &pan).reused, "copied from the frame before the change");
    app.run("plugin.remove", json!({"id": id})).unwrap();
    shapes(&mut app, &ctx);
    assert_ne!(plugins(&app), after, "drawn again once the plug-in was removed");
}

/// A frame does not copy from the last one when the view moves by part of a pixel or over half the
/// frame, when copying is not allowed, or when a listed key field, render option, zoom, rotation,
/// canvas size or device scale differs. Each such frame has the job's size. The session test
/// compares such frames with a fresh render pixel by pixel.
#[test]
fn a_frame_renders_whole_when_anything_but_a_whole_pixel_pan_changed() {
    keeping_color_settings(a_frame_renders_whole);
}

fn a_frame_renders_whole() {
    let (doc, key) = pan_doc();
    let other_opts = |f: &dyn Fn(&mut vectorcraft_render::RenderOptions)| {
        let mut job = pan_job(&doc, &key, (10.0, 0.0));
        f(&mut job.opts);
        job
    };
    let other_key = |f: &dyn Fn(&mut CacheKey)| {
        let mut job = pan_job(&doc, &key, (10.0, 0.0));
        f(&mut job.key);
        job
    };
    let mut other_doc = (*doc).clone();
    other_doc.layers.clear();
    let scaled = {
        let mut job = pan_job(&doc, &key, (10.0, 0.0));
        job.key.zoom = 2.0;
        job.view = Affine::translate((10.0, 0.0)) * Affine::scale(2.0);
        job
    };
    let turned = {
        let mut job = pan_job(&doc, &key, (10.0, 0.0));
        job.key.rot = 30.0;
        job.view = Affine::translate((10.0, 0.0)) * Affine::rotate(30f64.to_radians());
        job
    };
    let resized = {
        let mut job = pan_job(&doc, &key, (10.0, 0.0));
        (job.key.w, job.w) = (200, 200);
        job
    };
    let denser = other_key(&|k| k.ppp = 2.0);
    let not_allowed = crate::render_worker::Job { may_copy: false, ..pan_job(&doc, &key, (10.0, 0.0)) };
    let cases: Vec<(&str, crate::render_worker::Job)> = vec![
        ("part of a pixel across", pan_job(&doc, &key, (10.5, 0.0))),
        ("part of a pixel down", pan_job(&doc, &key, (0.0, 0.25))),
        ("too far to keep half", pan_job(&doc, &key, (250.0, 0.0))),
        ("zoom", scaled),
        ("rotation", turned),
        ("canvas size", resized),
        ("device scale", denser),
        ("not allowed", not_allowed),
        ("document", other_key(&|k| k.doc += 1)),
        ("revision", crate::render_worker::Job { doc: std::sync::Arc::new(other_doc), ..other_key(&|k| k.revision += 1) }),
        ("interaction", other_key(&|k| k.interacting = true)),
        ("outline key", other_key(&|k| k.outline = true)),
        ("Pixel Preview", other_key(&|k| k.pixel = Some([0, 0, 256, 128]))),
        ("placed bitmaps", other_key(&|k| k.placed += 1)),
        ("plug-ins", other_key(&|k| k.plugins += 1)),
        ("outline", other_opts(&|o| o.outline = true)),
        ("proof", other_opts(&|o| o.proof = Some(Default::default()))),
        ("overprint preview", other_opts(&|o| o.overprint_preview = true)),
        ("rich black", other_opts(&|o| o.rich_black = true)),
        ("trim", other_opts(&|o| o.trim = true)),
        ("anti-alias", other_opts(&|o| o.anti_alias = vectorcraft_render::AntiAlias::None)),
        ("smooth images", other_opts(&|o| o.smooth_images = false)),
        ("opacity mask view", other_opts(&|o| o.mask_view = Some(vectorcraft_doc::NodeId(1)))),
    ];
    let mut copied = vec![];
    for (name, job) in cases {
        let mut r = vectorcraft_render::Renderer::new();
        let mut kept = None;
        render_job(&mut r, &mut kept, &pan_job(&doc, &key, (0.0, 0.0)));
        let frame = render_job(&mut r, &mut kept, &job);
        if frame.reused {
            copied.push(name);
        }
        let whole = vectorcraft_render::Renderer::new().render(&job.doc, job.w, job.h, job.view, &job.opts);
        assert_eq!((frame.img.width, frame.img.height), (whole.width, whole.height), "{name}");
    }
    assert!(copied.is_empty(), "copied from the last frame: {copied:?}");
}

/// The worker keeps a frame only when a later one can copy from it: a frame that does not allow
/// keeping, a frame over the limit and a frame that drew a raster from before its object moved are
/// not kept. A frame that may not copy can still be kept.
#[test]
fn the_render_worker_keeps_one_frame_up_to_its_limit() {
    keeping_color_settings(the_render_worker_keeps_one_frame);
}

fn the_render_worker_keeps_one_frame() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 256, "height": 128})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 16, "y": 16, "width": 64, "height": 64})).unwrap()["id"].clone();
    app.run("effect.apply", json!({"effect": "stylize.dropShadow", "ids": [id]})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("object.move", json!({"dx": 8, "dy": 0})).unwrap();
    let after = app.session.active().unwrap().doc.clone();
    let (_, key) = pan_doc();
    let mut r = vectorcraft_render::Renderer::new();
    let mut kept = None;
    render_job(&mut r, &mut kept, &pan_job(&before, &key, (0.0, 0.0)));
    assert!(kept.is_some(), "kept");
    let mut moved = pan_job(&after, &CacheKey { revision: key.revision + 1, interacting: true, ..key.clone() }, (0.0, 0.0));
    moved.opts.interacting = true;
    let frame = render_job(&mut r, &mut kept, &moved);
    assert!(frame.moved > 0, "the shadow drew from the raster made before the move");
    let mut wrong = vec![];
    if kept.is_some() {
        wrong.push("a frame that drew a raster from before its object moved");
    }

    let (doc, key) = pan_doc();
    render_job(&mut r, &mut kept, &pan_job(&doc, &key, (0.0, 0.0)));
    assert!(kept.is_some(), "kept");
    let off = crate::render_worker::Job { may_keep: false, ..pan_job(&doc, &key, (0.0, 0.0)) };
    render_job(&mut r, &mut kept, &off);
    if kept.is_some() {
        wrong.push("a frame that does not allow keeping");
    }
    let whole = crate::render_worker::Job { may_copy: false, ..pan_job(&doc, &key, (0.0, 0.0)) };
    render_job(&mut r, &mut kept, &whole);
    assert!(kept.is_some(), "a frame that may not copy is kept");
    let mut big = pan_job(&doc, &key, (0.0, 0.0));
    (big.key.w, big.key.h, big.w, big.h) = (4097, 4097, 4097, 4097);
    render_job(&mut r, &mut kept, &big);
    if kept.is_some() {
        wrong.push("a frame over 64 MiB");
    }
    assert!(wrong.is_empty(), "kept: {wrong:?}");
}

/// Through a pan, an edit, an undo, a zoom, a resize, a view mode and a color setting, each frame the
/// worker renders whole matches a fresh render, and each copied frame is within one level of it.
#[test]
fn worker_frames_match_a_fresh_render_through_a_session() {
    keeping_color_settings(worker_frames_match_a_fresh_render);
}

fn worker_frames_match_a_fresh_render() {
    let (doc, key) = pan_doc();
    let mut edited = (*doc).clone();
    edited.layers.reverse();
    let edited = std::sync::Arc::new(edited);
    let pan = |at: (f64, f64)| pan_job(&doc, &key, at);
    let changed = |f: &dyn Fn(&mut crate::render_worker::Job)| {
        let mut job = pan((4.5, -4.0));
        f(&mut job);
        job
    };
    let steps: Vec<(&str, crate::render_worker::Job, bool)> = vec![
        ("still", pan((0.0, 0.0)), false),
        ("pan", pan((10.0, 0.0)), true),
        ("pan back", pan((3.0, -4.0)), true),
        ("part of a pixel", pan((3.5, -4.0)), false),
        ("whole again", pan((4.5, -4.0)), true),
        ("settled", changed(&|j| j.may_copy = false), false),
        ("pan after settling", pan((5.5, -4.0)), true),
        ("back before the edit", pan((4.5, -4.0)), true),
        (
            "edit",
            changed(&|j| {
                j.doc = edited.clone();
                j.key.revision += 1;
            }),
            false,
        ),
        ("undo", pan((4.5, -4.0)), false),
        (
            "zoom",
            changed(&|j| {
                j.key.zoom = 2.0;
                j.view = Affine::translate((4.5, -4.0)) * Affine::scale(2.0);
            }),
            false,
        ),
        (
            "resize",
            changed(&|j| {
                (j.key.w, j.key.h, j.w, j.h) = (200, 100, 200, 100);
            }),
            false,
        ),
        (
            "outline",
            changed(&|j| {
                j.key.outline = true;
                j.opts.outline = true;
            }),
            false,
        ),
        (
            "rich black",
            changed(&|j| {
                j.key.rich_black = true;
                j.opts.rich_black = true;
            }),
            false,
        ),
        ("setting back", pan((4.5, -4.0)), false),
        ("first pan after the setting", pan((5.5, -4.0)), false),
        ("next pan", pan((6.5, -4.0)), true),
    ];
    let mut r = vectorcraft_render::Renderer::new();
    let mut kept = None;
    for (name, job, copies) in steps {
        let frame = render_job(&mut r, &mut kept, &job);
        assert_eq!(frame.reused, copies, "{name}");
        let whole = vectorcraft_render::Renderer::new().render(&job.doc, job.w, job.h, job.view, &job.opts);
        if copies {
            let off = frame.img.pixels.iter().zip(&whole.pixels).map(|(a, b)| a.abs_diff(*b)).max();
            assert!(off.is_some_and(|d| d <= 1), "{name}: {off:?}");
        } else {
            assert_eq!(frame.img.pixels, whole.pixels, "{name}");
        }
    }
}

/// A pan over art the frame's size decides stays within one level of a fresh render of the frame,
/// because it renders whole: a full-page raster effect whose raster a part of the frame would make
/// at another scale, and a group drawn offscreen across the exposed part's edge with an Inner Glow
/// child inside the part, whose filter layer starts at another pixel in the part. The worker does
/// not keep such a frame for the pan to copy from.
#[test]
fn a_pan_over_art_the_frame_size_decides_renders_whole() {
    keeping_color_settings(a_pan_over_art);
}

fn a_pan_over_art() {
    let (_, key) = pan_doc();
    let effect = |effect: &str, params: serde_json::Value| {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 595, "height": 842})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 595, "height": 842})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [id], "color": "#1a4de6"})).unwrap();
        app.run("effect.apply", json!({"effect": effect, "ids": [id], "params": params})).unwrap();
        app.session.active().unwrap().doc.clone()
    };
    let group_glow = {
        use vectorcraft_doc::Appearance;
        use vectorcraft_doc::color::{BlendMode, Color, Paint};
        use vectorcraft_geom::{Rect, shapes::rectangle};
        let mut d = vectorcraft_doc::Document::new(1600.0, 1200.0);
        let basic = |c: Color| Appearance::basic(Paint::solid(c), Paint::None, 0.0);
        let wide = Node::path(d.alloc_id(), rectangle(Rect::new(100.0, 100.0, 1590.0, 900.0)), basic(Color::rgb(0.9, 0.8, 0.3)));
        let mut child = Node::path(d.alloc_id(), rectangle(Rect::new(1453.0, 200.0, 1583.0, 500.0)), basic(Color::rgb(0.1, 0.3, 0.9)));
        child.appearance.effects = vec![vectorcraft_doc::Effect { id: "stylize.innerGlow".into(), params: json!({"blur": 40}), visible: true }];
        child.blend = BlendMode::Multiply;
        let mut group = Node::group(d.alloc_id(), vec![std::sync::Arc::new(wide), std::sync::Arc::new(child)]);
        group.isolate = true;
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, group).unwrap();
        std::sync::Arc::new(d)
    };
    let cases = [
        ("Gaussian Blur 500 pt", effect("blur.gaussian", json!({"radius": 500})), (-200.0, -200.0), 2.0, (1440, 870)),
        ("Craquelure", effect("texture.craquelure", json!({})), (-200.0, -200.0), 10.0, (3840, 2160)),
        ("Inner Glow in a group drawn offscreen", group_glow, (0.0, 0.0), 1.0, (1600, 1200)),
    ];
    for (name, doc, (x, y), zoom, (w, h)) in cases {
        let job = |dx: f64| {
            let mut job = pan_job(&doc, &key, (dx, 0.0));
            (job.key.w, job.key.h, job.w, job.h, job.key.zoom) = (w, h, w, h, zoom);
            job.view = Affine::translate((x + dx, y)) * Affine::scale(zoom);
            job
        };
        let mut r = vectorcraft_render::Renderer::new();
        let mut kept = None;
        render_job(&mut r, &mut kept, &job(0.0));
        let pan = job(-40.0);
        let frame = render_job(&mut r, &mut kept, &pan);
        let whole = vectorcraft_render::Renderer::new().render(&pan.doc, pan.w, pan.h, pan.view, &pan.opts);
        let off = frame.img.pixels.iter().zip(&whole.pixels).map(|(a, b)| a.abs_diff(*b)).max();
        assert!(off.is_some_and(|d| d <= 1), "{name}: a channel differs by {off:?}");
        assert!(!frame.reused, "{name}: the pan renders whole");
        assert!(kept.is_none(), "{name}: the frame is not kept");
    }
}

/// The canvas asks the worker for a frame that may copy pixels when the view moves, keeps the whole
/// frame's time, and renders the frame whole once the view has been still for STILL_SECS, not
/// before and only once.
#[test]
fn a_frame_with_copied_pixels_renders_whole_once_the_view_stops() {
    keeping_color_settings(a_frame_with_copied_pixels);
}

fn a_frame_with_copied_pixels() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true;
    let (worker, jobs, done_tx) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    let ctx = crate::testing::ui_context();
    let frame = |app: &mut VectorcraftApp, t: f64| {
        let raw =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time: Some(t), ..Default::default() };
        ctx.run_ui(raw, |ui| show(app, ui)).textures_delta.clear();
    };
    frame(&mut app, 1.0);
    done_tx.send(finished(&jobs.try_recv().expect("the first frame"))).unwrap();
    frame(&mut app, 1.0);
    assert_eq!(app.canvas.last_ms, 40.0);
    assert!(app.canvas.reused.is_none());

    app.view_mut().unwrap().center.x += 5.0;
    frame(&mut app, 2.0);
    let pan = jobs.try_recv().expect("the pan's frame goes to the worker");
    assert!(pan.may_copy, "a new view may copy");
    done_tx.send(crate::render_worker::Done { ms: 2.0, reused: true, ..finished(&pan) }).unwrap();
    frame(&mut app, 3.0);
    assert_eq!(app.canvas.reused, Some(3.0));
    assert_eq!(app.canvas.last_ms, 40.0, "a frame with copied pixels keeps the whole frame's time");
    assert_eq!(app.perf.render_ms, 2.0);

    frame(&mut app, 3.0 + STILL_SECS / 2.0);
    assert!(jobs.try_recv().is_err(), "the view has not been still long enough");
    frame(&mut app, 3.0 + STILL_SECS);
    let settle = jobs.try_recv().expect("the frame renders whole");
    assert!(!settle.may_copy, "the settling frame copies nothing");
    assert_eq!(settle.key, pan.key);
    frame(&mut app, 3.0 + STILL_SECS * 2.0);
    assert!(jobs.try_recv().is_err(), "queued once");
    done_tx.send(finished(&settle)).unwrap();
    frame(&mut app, 4.0);
    assert!(app.canvas.reused.is_none());
    frame(&mut app, 5.0);
    assert!(jobs.try_recv().is_err(), "nothing to render after it");
}

/// The frame that renders whole after a pan stops copies nothing and stays on the worker, so the
/// first step of the next pan copies from it. The jobs are the ones the canvas asks for.
#[test]
fn a_pan_after_the_settling_frame_copies() {
    keeping_color_settings(a_pan_after_the_settling_frame);
}

fn a_pan_after_the_settling_frame() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true;
    let (worker, jobs, done_tx) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    let ctx = crate::testing::ui_context();
    let frame = |app: &mut VectorcraftApp, t: f64| {
        let raw =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time: Some(t), ..Default::default() };
        ctx.run_ui(raw, |ui| show(app, ui)).textures_delta.clear();
    };
    let mut r = vectorcraft_render::Renderer::new();
    let mut kept = None;
    let mut render = |name: &str, copies: bool| {
        let job = jobs.try_recv().unwrap_or_else(|_| panic!("{name}: the canvas asks for a frame"));
        let f = render_job(&mut r, &mut kept, &job);
        assert_eq!(f.reused, copies, "{name}");
        let done = crate::render_worker::Done { key: job.key, doc: job.doc, opts: job.opts, img: f.img, ms: 40.0, moved: f.moved, reused: f.reused };
        done_tx.send(done).unwrap();
    };
    frame(&mut app, 1.0);
    render("the first frame", false);
    frame(&mut app, 1.0);
    // At 100% the view moves by whole device pixels.
    app.view_mut().unwrap().zoom = 1.0;
    frame(&mut app, 1.5);
    render("the zoom", false);
    frame(&mut app, 1.5);

    app.view_mut().unwrap().center.x += 5.0;
    frame(&mut app, 1.75);
    render("the first pan after the zoom", false);
    frame(&mut app, 1.75);

    app.view_mut().unwrap().center.x += 5.0;
    frame(&mut app, 2.0);
    render("the pan", true);
    frame(&mut app, 3.0);
    frame(&mut app, 3.0 + STILL_SECS);
    render("the settling frame", false);
    frame(&mut app, 4.0);

    app.view_mut().unwrap().center.x += 5.0;
    frame(&mut app, 5.0);
    render("the next pan", true);
}

/// A frame after a zoom or an edit holds no pixels, and the frame before it lets its pixels go. The
/// first pan step after it renders whole and holds its pixels, and the next step copies them.
#[test]
fn a_frame_after_a_zoom_or_an_edit_holds_no_pixels() {
    let (doc, key) = pan_doc();
    let mut edited = (*doc).clone();
    edited.layers.reverse();
    let edited = std::sync::Arc::new(edited);
    let zoomed = |key: &CacheKey, at: f64, zoom: f64| {
        let mut job = pan_job(&doc, key, (at, 0.0));
        job.key.zoom = zoom;
        job.view = Affine::translate((at, 0.0)) * Affine::scale(zoom);
        job
    };
    let revised = CacheKey { revision: key.revision + 1, ..key.clone() };
    let steps: Vec<(&str, crate::render_worker::Job, bool, bool)> = vec![
        ("the first frame", pan_job(&doc, &key, (0.0, 0.0)), false, true),
        ("a zoom", zoomed(&key, 0.0, 2.0), false, false),
        ("another zoom", zoomed(&key, 0.0, 1.5), false, false),
        ("the first pan after the zoom", zoomed(&key, 10.0, 1.5), false, true),
        ("the next pan", zoomed(&key, 20.0, 1.5), true, true),
        ("an edit", crate::render_worker::Job { doc: edited.clone(), ..pan_job(&doc, &revised, (0.0, 0.0)) }, false, false),
        ("the first pan after the edit", crate::render_worker::Job { doc: edited.clone(), ..pan_job(&doc, &revised, (10.0, 0.0)) }, false, true),
        ("the next pan after the edit", crate::render_worker::Job { doc: edited.clone(), ..pan_job(&doc, &revised, (20.0, 0.0)) }, true, true),
    ];
    let mut r = vectorcraft_render::Renderer::new();
    let mut kept = None;
    for (name, job, copies, holds) in steps {
        assert_eq!(render_job(&mut r, &mut kept, &job).reused, copies, "{name}: copied");
        assert_eq!(kept.as_ref().is_some_and(|k| k.pixels.is_some()), holds, "{name}: holds its pixels");
    }
}

/// The worker thread itself copies pixels on a pan and reports it.
#[test]
fn the_render_worker_thread_copies_a_pan() {
    let (doc, key) = pan_doc();
    let mut worker = crate::render_worker::Worker::spawn(crate::testing::ui_context()).expect("a worker");
    // Blocks until the worker sends its frame. A worker thread that stopped gives None.
    let wait = |worker: &mut crate::render_worker::Worker| worker.wait(std::time::Duration::MAX).expect("the worker sends the frame");
    worker.submit(pan_job(&doc, &key, (0.0, 0.0)));
    assert!(!wait(&mut worker).reused);
    worker.submit(pan_job(&doc, &key, (10.0, 0.0)));
    assert!(wait(&mut worker).reused);
}

/// #4155: a document's first frame renders on the render worker, not on the UI thread, where a
/// heavy one froze the window for seconds while it opened. The canvas shows the document's paper
/// meanwhile, does not queue the frame again while it renders, and shows it once it lands.
#[test]
fn a_documents_first_frame_renders_on_the_worker() {
    keeping_color_settings(a_documents_first_frame);
}

fn a_documents_first_frame() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true;
    let (worker, jobs, done_tx) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    let ctx = crate::testing::ui_context();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let frame = |app: &mut VectorcraftApp| ctx.run_ui(raw.clone(), |ui| show(app, ui)).textures_delta.clear();

    frame(&mut app);
    let job = jobs.try_recv().expect("the first frame is queued on the worker");
    assert!(app.canvas.texture.is_none(), "not rendered on the UI thread");
    frame(&mut app);
    assert!(jobs.try_recv().is_err(), "not queued again while it renders");
    assert!(app.canvas.texture.is_none());

    done_tx.send(finished(&job)).unwrap();
    frame(&mut app);
    assert!(app.canvas.texture.is_some(), "shown once it lands");
    assert_eq!(app.canvas.key.as_ref(), Some(&job.key));
    assert!(app.canvas.from_worker);
}

/// The canvas waits for a document's first frame only briefly (`FIRST_FRAME_GRACE`): `wait`
/// returns a frame finished before or during the wait, clearing the pending job, and nothing once
/// the time is up.
#[test]
fn the_render_worker_waits_for_a_frame_only_as_long_as_asked() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.canvas.worker_started = true;
    let (worker, jobs, _) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    let ctx = crate::testing::ui_context();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(200.0, 150.0))), ..Default::default() };
    ctx.run_ui(raw, |ui| show(&mut app, ui)).textures_delta.clear();
    let job = jobs.try_recv().expect("a job to finish");

    let (mut w, _, done_tx) = crate::render_worker::Worker::test_channel();
    let t0 = std::time::Instant::now();
    assert!(w.wait(std::time::Duration::from_millis(20)).is_none());
    assert!(t0.elapsed() >= std::time::Duration::from_millis(20), "waited the time asked");

    w.pending = Some(job.key.clone());
    done_tx.send(finished(&job)).unwrap();
    assert_eq!(w.wait(std::time::Duration::from_secs(5)).map(|d| d.key), Some(job.key.clone()), "finished before the wait");
    assert!(w.pending.is_none());

    let late = finished(&job);
    let sender = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(30));
        done_tx.send(late).unwrap();
    });
    assert!(w.wait(std::time::Duration::from_secs(5)).is_some(), "finished during the wait");
    sender.join().unwrap();
}

/// An edit to a 3D effect renders the part it changed on this thread, measured by the renderer
/// that drew the frame shown ([`vectorcraft_render::Renderer::changed`]): it draws the part with
/// the 3D art it made to measure it.
#[test]
fn an_edit_to_a_3d_effect_redraws_with_the_art_measured() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    app.canvas.worker_started = true; // render on this thread
    let id = app.run("shape.rectangle", json!({"x": 380, "y": 280, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "ids": [id], "params": {"depth": 60.0}})).unwrap();
    let ctx = crate::testing::ui_context();
    shapes(&mut app, &ctx);
    assert_eq!(app.canvas.renderer.stats.three_d_built, 1, "the whole frame made the art");
    app.run("effect.setParams", json!({"ids": [id], "index": 0, "item": null, "params": {"depth": 5.0}})).unwrap();
    // Drawing only a part keeps the whole frame's time; drawing it whole measures it again.
    app.canvas.last_ms = 7.25;
    shapes(&mut app, &ctx);
    assert!(!app.canvas.from_worker);
    assert_eq!(app.canvas.last_ms, 7.25, "only the part that changed was drawn");
    assert_eq!(app.canvas.renderer.stats.three_d_built, 0, "the part was drawn with the art made to measure the edit");
}

/// An edit to a 3D effect off the view, and one the render worker draws, leave none of the art made
/// to measure them held by this thread's renderer.
#[test]
fn art_measured_for_an_edit_not_drawn_here_is_let_go() {
    for worker in [false, true] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
        app.canvas.worker_started = true; // render on this thread
        // Off the view, or filling it, for the worker to draw.
        let (x, size) = if worker { (0, 800) } else { (5000, 40) };
        let id = app.run("shape.rectangle", json!({"x": x, "y": 0, "width": size, "height": size})).unwrap()["id"].clone();
        app.run("effect.apply", json!({"effect": "threeD.extrude", "ids": [id], "params": {"depth": 60.0}})).unwrap();
        let ctx = crate::testing::ui_context();
        shapes(&mut app, &ctx);
        // A worker whose jobs the test receives: none finishes unless the test says so.
        let mut jobs = None;
        if worker {
            let (w, rx, done) = crate::render_worker::Worker::test_channel();
            app.canvas.worker = Some(w);
            jobs = Some((rx, done));
            app.canvas.last_ms = 100.0;
        }
        app.run("effect.setParams", json!({"ids": [id], "index": 0, "item": null, "params": {"depth": 5.0}})).unwrap();
        // Measured through the canvas's renderer, the edit holds the art made for it.
        let (shown, opts) = app.canvas.shown.clone().unwrap();
        let now = app.session.active().unwrap().doc.clone();
        app.canvas.renderer.changed(&shown, &now, &opts);
        assert!(app.canvas.renderer.measured_bytes() > 0, "worker {worker}: art held");
        if !worker {
            app.canvas.last_ms = 7.25;
        }
        shapes(&mut app, &ctx);
        if !worker {
            // Nothing in view to draw: no frame, whole or part, began since the measure.
            assert_eq!(app.canvas.last_ms, 7.25, "not drawn whole");
            assert!(app.canvas.renderer.stats.three_d_built > 0, "no part drawn");
        }
        if let Some((rx, _)) = &jobs {
            assert!(rx.try_recv().is_ok(), "the worker draws it");
        }
        assert_eq!(app.canvas.renderer.measured_bytes(), 0, "worker {worker}");
        app.canvas.worker = None;
    }
}

/// #8363: a selected guide path is deselected when Trim View hides it, as Hide Guides does (#3219).
#[test]
fn trim_view_deselects_selected_guide_paths() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("view.guides.make", json!({})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(id)]);

    app.run("view.trimView", json!({})).unwrap();

    assert!(app.session.active().unwrap().selection.objects.is_empty());
}

/// #8363: a Selection move begun with the guides shown gathers the ruler guides to snap to. When
/// they stop showing during the move (Trim View or Hide Guides turned on), it no longer snaps to
/// them; with them shown throughout, it does.
#[test]
fn a_move_stops_snapping_to_guides_hidden_during_it() {
    // What hides the guides mid-move (none: they stay shown), and whether the move then snaps.
    for (hide, snaps) in [(None, true), (Some("trim"), false), (Some("guides"), false)] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 50, "height": 50})).unwrap();
        app.session.execute("guide.add", &json!({"vertical": true, "pos": 200})).unwrap();
        app.select_tool("selection");
        app.ui.view.smart_guides = false;
        app.ui.view.snap_to_point = true;
        app.ui.view.guides = true;
        let ctx = crate::testing::ui_context();
        frame(&mut app, &ctx, vec![]);
        let xf = app.canvas_view().unwrap();
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        // Grabbed at its centre, moved first well away from the guide, then 1.5 px short of it.
        let grab = xf.to_screen(Point::new(75.0, 75.0));
        let away = xf.to_screen(Point::new(120.0, 75.0));
        let near = xf.to_screen(Point::new(200.0, 75.0)) - vec2(1.5, 0.0);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(grab)]);
        frame(&mut app, &ctx, vec![button(grab, true)]);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(away)]);
        match hide {
            Some("trim") => app.ui.view.trim_view = true,
            Some(_) => app.ui.view.guides = false,
            None => {}
        }
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(near)]);
        frame(&mut app, &ctx, vec![button(near, false)]);
        let b = app.session.active().unwrap().doc.art_bounds().unwrap();
        let centre = (b.x0 + b.x1) / 2.0;
        assert_eq!((centre - 200.0).abs() < 1e-6, snaps, "{hide:?}: the grabbed centre at {centre}");
        // Within the precision of the pointer's f32 screen points.
        assert!((centre - xf.to_doc(near).x).abs() < 1e-4 || snaps, "{hide:?}: unsnapped, it follows the pointer: {centre}");
    }
}
