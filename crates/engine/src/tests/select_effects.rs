//! Clicks, hovers and marquees with the selection tools leave out the area that raster effects
//! (drop shadows, glows, blurs) paint beyond an object: there they reach the object under the
//! effect, or nothing.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::{Point, Rect};
use vectorcraft_tools::{Cursor, Mods, PointerEvent, PointerKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A rectangle filled with `color`.
fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, color: &str) -> NodeId {
    let id = id_of(&run(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h})));
    run(s, "select.set", json!({"ids": [id.0]}));
    run(s, "paint.setFill", json!({"color": color}));
    id
}

/// A drop shadow on the whole of `id`, `dx` to the right, with no blur.
fn shadow(s: &mut Session, id: NodeId, dx: f64) {
    run(s, "effect.apply", json!({"effect": "stylize.dropShadow", "ids": [id.0], "params": {"x": dx, "y": 0, "blur": 0}, "item": null}));
}

fn pixel(s: &Session, x: u32, y: u32) -> [u8; 4] {
    let doc = s.doc().unwrap().doc.clone();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    vectorcraft_render::Renderer::new().render(&doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts).pixel(x, y)
}

fn press(s: &mut Session, tool: &str, from: (f64, f64), to: (f64, f64), mods: Mods) -> Vec<NodeId> {
    let v = ViewInfo::default();
    s.select_tool(tool, v).unwrap();
    for (k, (x, y)) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
        if k == PointerKind::Drag && from == to {
            continue;
        }
        s.pointer(&PointerEvent::new(k, x, y).with_mods(mods), v).unwrap();
    }
    s.active().unwrap().selection.objects.clone()
}

/// Click with the Selection tool at (x, y) on a 100% view: what is selected afterwards.
fn click(s: &mut Session, x: f64, y: f64) -> Vec<NodeId> {
    press(s, "selection", (x, y), (x, y), Mods::default())
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

const WHITE: [u8; 4] = [255; 4];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// Blue B at x 100 to 160, red A over its left edge's side at x 20 to 80, and A's shadow
/// 40 pt to the right, from x 60 to 120: over the canvas from 80 to 100 and over B from 100 to 120.
fn shadow_over_b(s: &mut Session) -> (NodeId, NodeId) {
    let b = rect(s, 100.0, 20.0, 60.0, 60.0, "#0000ff");
    let a = rect(s, 20.0, 20.0, 60.0, 60.0, "#ff0000");
    shadow(s, a, 40.0);
    (a, b)
}

#[test]
fn a_click_on_a_drop_shadow_selects_the_object_under_it() {
    let mut s = session();
    let (a, b) = shadow_over_b(&mut s);
    // The shadow paints over B and over the canvas.
    assert_ne!(pixel(&s, 110, 50), BLUE);
    assert_ne!(pixel(&s, 90, 50), WHITE);
    run(&mut s, "select.none", json!({}));
    assert_eq!(click(&mut s, 110.0, 50.0), [b]);
    assert_eq!(click(&mut s, 50.0, 50.0), [a]);
}

#[test]
fn a_click_on_a_drop_shadow_over_the_canvas_selects_nothing_and_clears_the_selection() {
    let mut s = session();
    let (a, _) = shadow_over_b(&mut s);
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    assert!(click(&mut s, 90.0, 50.0).is_empty());
}

#[test]
fn a_click_on_an_outer_glow_or_a_gaussian_blur_outside_the_path_selects_what_is_under_it() {
    let mut s = session();
    let a = rect(&mut s, 20.0, 20.0, 60.0, 60.0, "#ff0000");
    run(&mut s, "effect.apply", json!({"effect": "stylize.outerGlow", "ids": [a.0], "params": {"blur": 20, "mode": "normal"}, "item": null}));
    assert_ne!(pixel(&s, 88, 50), WHITE);
    run(&mut s, "select.none", json!({}));
    assert!(click(&mut s, 88.0, 50.0).is_empty());
    // A red square whose blur reaches over the canvas and over the left edge of a blue one.
    let mut s = session();
    let b = rect(&mut s, 92.0, 20.0, 60.0, 60.0, "#0000ff");
    let a = rect(&mut s, 20.0, 20.0, 60.0, 60.0, "#ff0000");
    run(&mut s, "effect.apply", json!({"effect": "blur.gaussian", "ids": [a.0], "params": {"radius": 20}, "item": null}));
    assert_ne!(pixel(&s, 98, 50), BLUE);
    assert_ne!(pixel(&s, 87, 50), WHITE);
    run(&mut s, "select.none", json!({}));
    assert_eq!(click(&mut s, 98.0, 50.0), [b]);
    // (With B deselected, so that the press does not take its bounding box handle.)
    run(&mut s, "select.none", json!({}));
    assert!(click(&mut s, 87.0, 50.0).is_empty());
}

/// The first point of `area` (x0, y0, x1, y1) that the canvas paints, scanning rows.
fn painted_in(s: &Session, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> Option<(f64, f64)> {
    let doc = s.doc().unwrap().doc.clone();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let img = vectorcraft_render::Renderer::new().render(&doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts);
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .find(|&(x, y)| img.pixel(x, y) != WHITE)
        .map(|(x, y)| (f64::from(x) + 0.5, f64::from(y) + 0.5))
}

#[test]
fn a_click_on_the_drop_shadow_of_a_group_or_of_point_type_selects_neither() {
    let mut s = session();
    let a = rect(&mut s, 20.0, 20.0, 30.0, 30.0, "#ff0000");
    let b = rect(&mut s, 60.0, 20.0, 30.0, 30.0, "#ff0000");
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let g = id_of(&run(&mut s, "object.group", json!({})));
    shadow(&mut s, g, 80.0);
    // The group's shadow lies from x 100 to 170.
    assert_ne!(pixel(&s, 110, 35), WHITE);
    run(&mut s, "select.none", json!({}));
    assert!(click(&mut s, 110.0, 35.0).is_empty());
    assert_eq!(click(&mut s, 35.0, 35.0), [g]);
    // Type 40 pt high at y 150 with its shadow 90 pt to the right.
    let t = id_of(&run(&mut s, "text.create", json!({"x": 20, "y": 150, "text": "HH", "size": 40})));
    shadow(&mut s, t, 90.0);
    let right = bounds(&s, t).x1;
    assert!(right < 100.0, "{right}");
    let at = painted_in(&s, (right.ceil() as u32 + 10, 110, 199, 160)).expect("the type's shadow paints");
    run(&mut s, "select.none", json!({}));
    assert!(click(&mut s, at.0, at.1).is_empty(), "{at:?}");
}

#[test]
fn a_click_on_the_drop_shadow_of_a_symbols_art_does_not_select_the_instance() {
    let mut s = session();
    let a = rect(&mut s, 20.0, 20.0, 60.0, 60.0, "#ff0000");
    shadow(&mut s, a, 40.0);
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let inst = id_of(&run(&mut s, "symbol.new", json!({"name": "Shadowed"})));
    assert_ne!(pixel(&s, 100, 50), WHITE);
    run(&mut s, "select.none", json!({}));
    assert!(click(&mut s, 100.0, 50.0).is_empty());
    assert_eq!(click(&mut s, 50.0, 50.0), [inst]);
}

#[test]
fn a_click_on_the_drop_shadow_of_an_image_outside_its_frame_does_not_select_it() {
    let mut s = session();
    let mut png = vec![];
    image::RgbaImage::from_pixel(30, 30, image::Rgba([255, 0, 0, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let r = run(&mut s, "file.place", json!({"name": "red.png", "dataBase64": vectorcraft_format::base64_encode(&png), "at": [65, 65]}));
    let im = NodeId(r["ids"][0].as_u64().unwrap());
    // The frame lies from x 50 to 80 and the shadow from 90 to 120.
    shadow(&mut s, im, 40.0);
    assert_ne!(pixel(&s, 105, 65), WHITE);
    run(&mut s, "select.none", json!({}));
    assert!(click(&mut s, 105.0, 65.0).is_empty());
    assert_eq!(click(&mut s, 65.0, 65.0), [im]);
}

#[test]
fn shift_click_direct_selection_and_hover_on_a_drop_shadow_pass_through_it() {
    let mut s = session();
    let (a, b) = shadow_over_b(&mut s);
    // Shift-click on the shadow over B adds B and keeps A.
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let shift = Mods { shift: true, ..Mods::default() };
    let mut sel = press(&mut s, "selection", (110.0, 50.0), (110.0, 50.0), shift);
    sel.sort_by_key(|n| n.0);
    let mut both = vec![a, b];
    both.sort_by_key(|n| n.0);
    assert_eq!(sel, both);
    // Direct Selection takes B through the shadow, and nothing over the canvas.
    run(&mut s, "select.none", json!({}));
    let sel = press(&mut s, "directSelection", (110.0, 50.0), (110.0, 50.0), Mods::default());
    assert!(sel.contains(&b) && !sel.contains(&a), "{sel:?}");
    run(&mut s, "select.none", json!({}));
    assert!(press(&mut s, "directSelection", (90.0, 50.0), (90.0, 50.0), Mods::default()).is_empty());
    // With A selected, the pointer over its shadow is the plain arrow, and over A the move pointer.
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    assert_eq!(s.cursor(Point::new(50.0, 50.0), Mods::default(), v), Cursor::Move);
    assert_eq!(s.cursor(Point::new(110.0, 50.0), Mods::default(), v), Cursor::Arrow);
    assert_eq!(s.cursor(Point::new(92.0, 50.0), Mods::default(), v), Cursor::Arrow);
}

#[test]
fn a_drag_from_the_drop_shadow_of_a_selected_object_does_not_move_it() {
    let mut s = session();
    let (a, b) = shadow_over_b(&mut s);
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let before = bounds(&s, a);
    // From the shadow over the canvas: a marquee that touches nothing.
    assert!(press(&mut s, "selection", (92.0, 50.0), (96.0, 60.0), Mods::default()).is_empty());
    assert_eq!(bounds(&s, a), before);
    // From the shadow over B: B moves, A stays.
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let b_before = bounds(&s, b);
    assert_eq!(press(&mut s, "selection", (110.0, 50.0), (110.0, 60.0), Mods::default()), [b]);
    assert_eq!(bounds(&s, a), before);
    assert!((bounds(&s, b).y0 - b_before.y0 - 10.0).abs() < 1e-6, "{:?}", bounds(&s, b));
}

#[test]
fn a_marquee_that_touches_only_a_drop_shadow_does_not_select_its_object() {
    let mut s = session();
    let (_, b) = shadow_over_b(&mut s);
    run(&mut s, "select.none", json!({}));
    assert!(press(&mut s, "selection", (85.0, 40.0), (95.0, 60.0), Mods::default()).is_empty());
    // From above B, over B and the shadow: B only.
    assert_eq!(press(&mut s, "selection", (105.0, 5.0), (115.0, 30.0), Mods::default()), [b]);
}

/// The selection after a drag with `tool` from `from` to `to`.
fn marquee(s: &mut Session, tool: &str, from: (f64, f64), to: (f64, f64)) -> vectorcraft_doc::Selection {
    press(s, tool, from, to, Mods::default());
    s.doc().unwrap().selection.clone()
}

/// The point of anchor `(si, ai)` of path `id`.
fn anchor_point(s: &Session, id: NodeId, (si, ai): vectorcraft_doc::AnchorRef) -> Point {
    let path = s.doc().unwrap().doc.node(id).and_then(|n| n.path_data()).unwrap();
    path.anchors().find(|(s, i, _)| (*s, *i) == (si, ai)).map(|(.., a)| a.p).unwrap()
}

/// The anchors a marquee or a click picked on `id`, as points.
fn picked_points(s: &Session, id: NodeId) -> Vec<Point> {
    let refs: Vec<_> = s.doc().unwrap().selection.partial(id).into_iter().flatten().copied().collect();
    refs.into_iter().map(|r| anchor_point(s, id, r)).collect()
}

/// The anchors the tool marks as hovered.
fn anchor_marks(s: &mut Session) -> Vec<Point> {
    s.overlays(ViewInfo::default())
        .into_iter()
        .filter_map(|o| if let vectorcraft_tools::Overlay::Anchor { p, .. } = o { Some(p) } else { None })
        .collect()
}

fn hover(s: &mut Session, x: f64, y: f64) -> Cursor {
    let v = ViewInfo::default();
    s.pointer(&PointerEvent::new(PointerKind::Move, x, y), v).unwrap();
    s.cursor(Point::new(x, y), Mods::default(), v)
}

#[test]
fn a_direct_selection_marquee_leaves_out_a_drop_shadow() {
    let mut s = session();
    let (a, b) = shadow_over_b(&mut s);
    let (a_before, b_before) = (bounds(&s, a), bounds(&s, b));
    assert_ne!(pixel(&s, 90, 50), WHITE);
    run(&mut s, "select.none", json!({}));
    // Over the shadow alone: no object, no anchor, no segment.
    let sel = marquee(&mut s, "directSelection", (85.0, 40.0), (95.0, 60.0));
    assert!(sel.objects.is_empty() && sel.anchors.is_empty() && sel.segments.is_empty(), "{sel:?}");
    // Over the shadow and B's upper left corner: that anchor of B, and nothing of A.
    let sel = marquee(&mut s, "directSelection", (90.0, 5.0), (105.0, 30.0));
    assert_eq!(sel.objects, [b]);
    assert_eq!(picked_points(&s, b), [Point::new(100.0, 20.0)]);
    // Across B's top edge under the shadow, with neither of its anchors inside: that segment.
    run(&mut s, "select.none", json!({}));
    assert_ne!(pixel(&s, 112, 25), BLUE);
    let sel = marquee(&mut s, "directSelection", (110.0, 5.0), (115.0, 30.0));
    assert_eq!(sel.objects, [b]);
    assert!(sel.segments.contains_key(&b) && !sel.segments.contains_key(&a), "{sel:?}");
    assert_eq!((bounds(&s, a), bounds(&s, b)), (a_before, b_before));
}

#[test]
fn direct_selection_hover_leaves_out_a_drop_shadow_and_marks_the_anchor_under_it() {
    let mut s = session();
    let (_, b) = shadow_over_b(&mut s);
    // B's upper left corner lies under the shadow.
    assert_ne!(pixel(&s, 101, 21), BLUE);
    run(&mut s, "select.none", json!({}));
    s.select_tool("directSelection", ViewInfo::default()).unwrap();
    for (x, y) in [(90.0, 50.0), (110.0, 50.0)] {
        assert_eq!(hover(&mut s, x, y), Cursor::ArrowHollow, "({x}, {y})");
        assert!(anchor_marks(&mut s).is_empty(), "({x}, {y})");
    }
    assert_eq!(hover(&mut s, 101.0, 21.0), Cursor::DirectAnchor);
    assert_eq!(anchor_marks(&mut s), [Point::new(100.0, 20.0)]);
    // A press there picks that anchor of B.
    assert_eq!(press(&mut s, "directSelection", (101.0, 21.0), (101.0, 21.0), Mods::default()), [b]);
    assert_eq!(picked_points(&s, b), [Point::new(100.0, 20.0)]);
}

#[test]
fn a_group_selection_click_on_a_drop_shadow_selects_the_object_under_it() {
    let mut s = session();
    let (a, b) = shadow_over_b(&mut s);
    run(&mut s, "select.none", json!({}));
    assert_eq!(press(&mut s, "groupSelection", (110.0, 50.0), (110.0, 50.0), Mods::default()), [b]);
    assert!(press(&mut s, "groupSelection", (90.0, 50.0), (90.0, 50.0), Mods::default()).is_empty());
    assert_eq!(press(&mut s, "groupSelection", (50.0, 50.0), (50.0, 50.0), Mods::default()), [a]);
}

#[test]
fn group_selection_clicks_on_a_drop_shadow_climb_the_groups_of_the_object_under_it() {
    let mut s = session();
    // B and C in one group, A and D in another above it, and A's shadow over B.
    let b = rect(&mut s, 100.0, 20.0, 60.0, 60.0, "#0000ff");
    let c = rect(&mut s, 170.0, 20.0, 20.0, 20.0, "#0000ff");
    run(&mut s, "select.set", json!({"ids": [b.0, c.0]}));
    let gb = id_of(&run(&mut s, "object.group", json!({})));
    let a = rect(&mut s, 20.0, 20.0, 60.0, 60.0, "#ff0000");
    let d = rect(&mut s, 20.0, 120.0, 20.0, 20.0, "#ff0000");
    run(&mut s, "select.set", json!({"ids": [a.0, d.0]}));
    let ga = id_of(&run(&mut s, "object.group", json!({})));
    shadow(&mut s, a, 40.0);
    assert_ne!(pixel(&s, 110, 50), BLUE);
    assert_ne!(pixel(&s, 90, 50), WHITE);
    run(&mut s, "select.none", json!({}));
    let click = |s: &mut Session, x, y| {
        let mut sel = press(s, "groupSelection", (x, y), (x, y), Mods::default());
        sel.sort_by_key(|n| n.0);
        sel
    };
    assert_eq!(click(&mut s, 110.0, 50.0), [b]);
    let mut both = vec![b, gb];
    both.sort_by_key(|n| n.0);
    assert_eq!(click(&mut s, 110.0, 50.0), both);
    // With A selected, a click on its shadow over the canvas does not add A's group.
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    assert!(click(&mut s, 90.0, 50.0).is_empty());
    // On A itself the second click adds it.
    assert_eq!(click(&mut s, 50.0, 50.0), [a]);
    let mut both = vec![a, ga];
    both.sort_by_key(|n| n.0);
    assert_eq!(click(&mut s, 50.0, 50.0), both);
}

#[test]
fn a_group_selection_marquee_leaves_out_a_drop_shadow() {
    let mut s = session();
    let (a, b) = shadow_over_b(&mut s);
    run(&mut s, "select.none", json!({}));
    let sel = marquee(&mut s, "groupSelection", (85.0, 40.0), (95.0, 60.0));
    assert!(sel.objects.is_empty() && sel.anchors.is_empty(), "{sel:?}");
    // Over the shadow and B's upper left corner: that anchor of B.
    let sel = marquee(&mut s, "groupSelection", (90.0, 5.0), (105.0, 30.0));
    assert_eq!(sel.objects, [b]);
    assert_eq!(picked_points(&s, b), [Point::new(100.0, 20.0)]);
    // With E, a marquee around B that crosses A's shadow selects B whole, and not A.
    run(&mut s, "select.none", json!({}));
    let v = ViewInfo::default();
    s.select_tool("groupSelection", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 95.0, 10.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 130.0, 50.0), v).unwrap();
    s.tool_key(vectorcraft_tools::ToolKey::Letter('e'), Mods::default(), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 165.0, 90.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 165.0, 90.0), v).unwrap();
    let sel = s.doc().unwrap().selection.clone();
    assert_eq!(sel.objects, [b]);
    assert!(sel.partial(b).is_none() && sel.partial(a).is_none(), "{sel:?}");
}

#[test]
fn group_selection_hover_over_a_drop_shadow_marks_nothing() {
    let mut s = session();
    let (_, b) = shadow_over_b(&mut s);
    run(&mut s, "select.none", json!({}));
    s.select_tool("groupSelection", ViewInfo::default()).unwrap();
    for (x, y) in [(90.0, 50.0), (110.0, 50.0), (101.0, 21.0)] {
        assert_eq!(hover(&mut s, x, y), Cursor::GroupSelection, "({x}, {y})");
        assert!(anchor_marks(&mut s).is_empty(), "({x}, {y})");
    }
    assert_eq!(press(&mut s, "groupSelection", (110.0, 50.0), (110.0, 50.0), Mods::default()), [b]);
}
