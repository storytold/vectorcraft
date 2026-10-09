//! Tests for live compound shapes (`object.compoundShape.*`).

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{NodeKind, ShapeMode};
use vectorcraft_geom::{FillRule, PathData, Rect};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn node(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

fn set_fill(s: &mut Session, id: NodeId, c: Color) {
    s.edit("t", |d, _| {
        d.node_mut(id).unwrap().appearance.set_fill(Paint::solid(c));
        Ok(())
    })
    .unwrap();
}

fn red() -> Color {
    Color::rgb(1.0, 0.0, 0.0)
}
fn blue() -> Color {
    Color::rgb(0.0, 0.0, 1.0)
}

/// Two overlapping 100×100 squares (red at the origin, blue at 50,50), both selected.
fn two_rects(s: &mut Session) -> (NodeId, NodeId) {
    let a = rect(s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(s, 50.0, 50.0, 100.0, 100.0);
    set_fill(s, a, red());
    set_fill(s, b, blue());
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    (a, b)
}

fn outline(s: &Session, id: NodeId) -> PathData {
    vectorcraft_render::effects::compound_shape_path(&node(s, id)).unwrap()
}

fn area(p: &PathData) -> f64 {
    vectorcraft_pathops::area(p, FillRule::NonZero)
}

fn make(s: &mut Session, mode: &str) -> NodeId {
    id_of(&s.execute("object.compoundShape.make", &json!({"mode": mode})).unwrap())
}

#[test]
fn each_mode_matches_the_destructive_shape_mode() {
    for (mode, op, want) in
        [("add", "unite", 17500.0), ("subtract", "minusFront", 7500.0), ("intersect", "intersect", 2500.0), ("exclude", "exclude", 15000.0)]
    {
        let mut s = session();
        let (a, b) = two_rects(&mut s);
        let c = make(&mut s, mode);
        let n = node(&s, c);
        let NodeKind::CompoundShape { children } = &n.kind else { panic!("{mode}: not a compound shape") };
        // The members stay, with their own paint.
        assert_eq!(children.iter().map(|c| c.id).collect::<Vec<_>>(), vec![a, b]);
        assert_eq!(children[1].appearance.fill_paint(), Paint::solid(blue()));
        assert_eq!(s.doc().unwrap().selection.objects, vec![c]);
        let live = area(&outline(&s, c));
        assert!((live - want).abs() < 1.0, "{mode}: area {live}");
        let paint = n.appearance.fill_paint();
        // Same paint and outline as the plain Shape Mode.
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
        let r = s.execute(&format!("object.pathfinder.{op}"), &json!({})).unwrap();
        let r = NodeId(r["ids"][0].as_u64().unwrap());
        assert_eq!(node(&s, r).appearance.fill_paint(), paint, "{mode}: paint");
        assert_eq!(paint, Paint::solid(if mode == "subtract" { red() } else { blue() }));
    }
}

#[test]
fn subtract_keeps_the_bottom_member_adding() {
    let mut s = session();
    two_rects(&mut s);
    let c = make(&mut s, "subtract");
    let n = node(&s, c);
    let modes: Vec<ShapeMode> = n.children().unwrap().iter().map(|c| c.shape_mode).collect();
    assert_eq!(modes, vec![ShapeMode::Add, ShapeMode::Subtract]);
}

#[test]
fn editing_a_member_updates_the_outline() {
    let mut s = session();
    let (_, b) = two_rects(&mut s);
    let c = make(&mut s, "subtract");
    assert!((area(&outline(&s, c)) - 7500.0).abs() < 1.0);
    // Move the front member away: nothing is cut out any more.
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    s.execute("object.move", &json!({"dx": 200, "dy": 0})).unwrap();
    assert!((area(&outline(&s, c)) - 10000.0).abs() < 1.0);
    // Bounds follow the members (subtract doesn't grow them).
    let bb = node(&s, c).geometric_bounds().unwrap();
    assert!((bb.x1 - 100.0).abs() < 1e-6 && (bb.y1 - 100.0).abs() < 1e-6, "{bb:?}");
}

#[test]
fn set_mode_on_a_member() {
    let mut s = session();
    let (_, b) = two_rects(&mut s);
    let c = make(&mut s, "add");
    s.execute("object.compoundShape.setMode", &json!({"ids": [b.0], "mode": "intersect"})).unwrap();
    assert_eq!(node(&s, b).shape_mode, ShapeMode::Intersect);
    assert!((area(&outline(&s, c)) - 2500.0).abs() < 1.0);
    // Undo restores the mode.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, b).shape_mode, ShapeMode::Add);
    // Not a member: refused.
    let lone = rect(&mut s, 300.0, 300.0, 10.0, 10.0);
    assert!(s.execute("object.compoundShape.setMode", &json!({"ids": [lone.0], "mode": "add"})).is_err());
    assert!(s.execute("object.compoundShape.setMode", &json!({"ids": [b.0], "mode": "nope"})).is_err());
}

#[test]
fn a_shape_mode_on_selected_members_sets_their_mode() {
    let mut s = session();
    let (a, b) = two_rects(&mut s);
    let c = make(&mut s, "add");
    // Direct-selected member: Minus Front sets its mode instead of cutting.
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    s.execute("object.pathfinder.minusFront", &json!({})).unwrap();
    assert_eq!(node(&s, b).shape_mode, ShapeMode::Subtract);
    assert!(s.doc().unwrap().doc.node(a).is_some() && s.doc().unwrap().doc.node(c).is_some());
    // Alt-click (make) on a member does the same.
    s.execute("object.compoundShape.make", &json!({"mode": "exclude"})).unwrap();
    assert_eq!(node(&s, b).shape_mode, ShapeMode::Exclude);
}

#[test]
fn release_gives_the_members_back() {
    let mut s = session();
    let (a, b) = two_rects(&mut s);
    let c = make(&mut s, "subtract");
    let r = s.execute("object.compoundShape.release", &json!({})).unwrap();
    let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids, vec![a.0, b.0]);
    assert!(s.doc().unwrap().doc.node(c).is_none());
    assert_eq!(node(&s, a).appearance.fill_paint(), Paint::solid(red()));
    assert_eq!(node(&s, b).appearance.fill_paint(), Paint::solid(blue()));
    assert_eq!(node(&s, b).shape_mode, ShapeMode::Add);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(node(&s, c).kind, NodeKind::CompoundShape { .. }));
}

#[test]
fn expand_bakes_the_outline() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 25.0, 25.0, 50.0, 50.0);
    set_fill(&mut s, a, red());
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let c = make(&mut s, "subtract");
    s.edit("t", |d, _| {
        d.node_mut(c).unwrap().opacity = 0.5;
        Ok(())
    })
    .unwrap();
    s.execute("object.compoundShape.expand", &json!({})).unwrap();
    let n = node(&s, c);
    // A square with a hole: a compound path, painted and blended like the compound shape.
    let NodeKind::Compound { children, .. } = &n.kind else { panic!("not a compound path: {:?}", n.kind) };
    assert_eq!(children.len(), 2);
    assert_eq!(n.appearance.fill_paint(), Paint::solid(red()));
    assert_eq!(n.opacity, 0.5);
    let doc = &s.doc().unwrap().doc;
    assert!(doc.node(a).is_none() && doc.node(b).is_none());
}

#[test]
fn make_needs_two_objects_that_cover_a_region() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    assert!(s.execute("object.compoundShape.make", &json!({})).is_err());
    assert!(s.execute("object.compoundShape.make", &json!({"mode": 3})).is_err());
    assert!(s.execute("object.compoundShape.release", &json!({})).is_err());
}

#[test]
fn groups_type_and_nested_compound_shapes_are_members() {
    let mut s = session();
    let (a, b) = two_rects(&mut s);
    let inner = make(&mut s, "add");
    let t = id_of(&s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap());
    let g1 = rect(&mut s, 400.0, 0.0, 10.0, 10.0);
    let g2 = rect(&mut s, 420.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [g1.0, g2.0]})).unwrap();
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("select.set", &json!({"ids": [inner.0, t.0, g.0]})).unwrap();
    let c = make(&mut s, "add");
    let p = outline(&s, c);
    // The squares (17500), the group's two boxes (200) and the glyphs' area (none without fonts).
    let a_ = area(&p);
    assert!(a_ > 17699.0 && a_ < 17700.0 + 5000.0, "{a_}");
    if !vectorcraft_text::CRAFT_FONTS.is_empty() {
        assert!(a_ > 17710.0, "{a_}");
    }
    let _ = (a, b);
}

#[test]
fn renders_and_exports_the_evaluated_outline() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 25.0, 25.0, 50.0, 50.0);
    set_fill(&mut s, a, red());
    set_fill(&mut s, b, blue());
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    make(&mut s, "subtract");
    let doc = &s.doc().unwrap().doc;
    // SVG: one path with the hole, in the compound's red; the blue member doesn't paint.
    let svg = vectorcraft_svg::export(doc, &Default::default());
    assert_eq!(svg.matches("<path").count(), 1, "{svg}");
    assert!(!svg.to_lowercase().contains("#0000ff"), "{svg}");
    // Baked documents hold no compound shapes.
    let baked = vectorcraft_render::effects::bake_document(doc).unwrap();
    let mut any = false;
    for l in &baked.layers {
        l.walk(&mut |n| any |= matches!(n.kind, NodeKind::CompoundShape { .. }));
    }
    assert!(!any);
    let _ = Rect::ZERO;
}

#[test]
fn saves_and_reopens_live() {
    let mut s = session();
    two_rects(&mut s);
    let c = make(&mut s, "subtract");
    let doc = &s.doc().unwrap().doc;
    let bytes = vectorcraft_format::save(doc, false);
    let back = vectorcraft_format::load(&bytes).unwrap();
    let n = back.node(c).unwrap();
    assert!(matches!(n.kind, NodeKind::CompoundShape { .. }));
    assert_eq!(n.children().unwrap()[1].shape_mode, ShapeMode::Subtract);
}

#[test]
fn object_expand_bakes_compound_shapes() {
    let mut s = session();
    two_rects(&mut s);
    let c = make(&mut s, "intersect");
    s.execute("object.expand", &json!({"object": true, "fill": false, "stroke": false})).unwrap();
    let n = node(&s, c);
    let NodeKind::Path { path, .. } = &n.kind else { panic!("not a path: {:?}", n.kind) };
    assert!((area(path) - 2500.0).abs() < 1.0);
    assert_eq!(n.appearance.fill_paint(), Paint::solid(blue()));
}

#[test]
fn older_versions_get_groups_of_the_members() {
    let mut s = session();
    two_rects(&mut s);
    let c = make(&mut s, "subtract");
    let doc = &s.doc().unwrap().doc;
    let o = vectorcraft_format::SaveOptions { version: 4, ..Default::default() };
    let back = vectorcraft_format::load(&vectorcraft_format::save_with(doc, &o).unwrap()).unwrap();
    assert!(!back.has_compound_shapes());
    assert!(matches!(back.node(c).unwrap().kind, NodeKind::Group { .. }));
}

#[test]
fn clicks_hit_the_outline_and_pick_members() {
    use vectorcraft_doc::hit::{HitOptions, hit_test};
    use vectorcraft_geom::Point;
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 25.0, 25.0, 50.0, 50.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let c = make(&mut s, "subtract");
    let doc = &s.doc().unwrap().doc;
    let opt = HitOptions::default();
    // In the hole: nothing.
    assert!(hit_test(doc, Point::new(50.0, 50.0), opt).is_none());
    // On the ring: the compound shape for the Selection tool, the member under it for Direct
    // Selection.
    let h = hit_test(doc, Point::new(10.0, 10.0), opt).unwrap();
    assert_eq!(h.top_object(None), c);
    assert_eq!(h.leaf, a);
    // The hole's edge belongs to the front member.
    let h = hit_test(doc, Point::new(25.0, 50.0), opt).unwrap();
    assert_eq!((h.top_object(None), h.leaf), (c, b));
}

#[test]
fn a_compound_shape_clips_by_its_outline() {
    let mut s = session();
    let art = rect(&mut s, -50.0, -50.0, 300.0, 300.0);
    two_rects(&mut s);
    let c = make(&mut s, "intersect");
    s.execute("select.set", &json!({"ids": [art.0, c.0]})).unwrap();
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    let g = s.doc().unwrap().doc.parent_of(c).unwrap();
    let clip = vectorcraft_render::effects::clip_outline(&node(&s, c)).unwrap();
    assert!(node(&s, g).clips());
    let b = vectorcraft_geom::Shape::bounding_box(&clip.0);
    assert!((b.x0 - 50.0).abs() < 1e-6 && (b.x1 - 100.0).abs() < 1e-6, "{b:?}");
}

/// Regression: in isolation a subtracted member could only be clicked on the compound's outline
/// where it ran along the member; elsewhere (outside the filled part, or in the hole it cuts) the
/// click hit the background. Isolated, members are objects of their own.
#[test]
fn an_isolated_compound_shapes_members_click_drag_and_resize_anywhere_in_them() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let v = crate::tooling::ViewInfo { smart_guides: false, ..Default::default() };
    let mut s = session();
    // A filled square with a square subtracted over its top-left corner (an L shape).
    let a = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
    let b = rect(&mut s, 0.0, 0.0, 150.0, 150.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let c = make(&mut s, "subtract");
    s.select_tool("selection", v).unwrap();
    let press = |s: &mut Session, kind, x, y| {
        s.pointer(&PointerEvent::new(kind, x, y), v).unwrap();
    };
    let click = |s: &mut Session, x, y| {
        press(s, PointerKind::Down, x, y);
        press(s, PointerKind::Up, x, y);
    };
    // 1–2. Select the compound shape, double-click it: isolation.
    click(&mut s, 250.0, 250.0);
    assert_eq!(s.doc().unwrap().selection.objects, vec![c]);
    press(&mut s, PointerKind::DoubleClick, 250.0, 250.0);
    assert_eq!(s.doc().unwrap().isolation, Some(c), "double-click isolates the compound shape");
    // 3. The subtracted member: outside the filled part, and inside the hole it cuts.
    for (x, y) in [(40.0, 40.0), (125.0, 125.0)] {
        s.execute("select.none", &json!({})).unwrap();
        click(&mut s, x, y);
        assert_eq!(s.doc().unwrap().selection.objects, vec![b], "a click at ({x}, {y}) picks the subtracted member");
    }
    // The filled member where it shows.
    click(&mut s, 250.0, 250.0);
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    // 4. Drag the subtracted member from inside the hole…
    click(&mut s, 125.0, 125.0);
    press(&mut s, PointerKind::Down, 125.0, 125.0);
    for x in [130.0, 135.0] {
        press(&mut s, PointerKind::Drag, x, 125.0);
    }
    press(&mut s, PointerKind::Up, 135.0, 125.0);
    assert!((node(&s, a).geometric_bounds().unwrap().x0 - 100.0).abs() < 1e-6, "the filled member stays");
    let bb = node(&s, b).geometric_bounds().unwrap();
    assert!((bb.x0 - 10.0).abs() < 1e-6 && (bb.x1 - 160.0).abs() < 1e-6, "moved: {bb:?}");
    // …and resize it by its top-left handle (outside the filled part).
    press(&mut s, PointerKind::Down, 10.0, 0.0);
    for x in [0.0, -10.0] {
        press(&mut s, PointerKind::Drag, x, -20.0);
    }
    press(&mut s, PointerKind::Up, -10.0, -20.0);
    let bb = node(&s, b).geometric_bounds().unwrap();
    assert!((bb.x0 + 10.0).abs() < 1e-6 && (bb.y0 + 20.0).abs() < 1e-6 && (bb.x1 - 160.0).abs() < 1e-6, "resized: {bb:?}");
    assert_eq!(s.doc().unwrap().selection.objects, vec![b]);
    // The outline follows: the L's notch grew with the member.
    assert!((area(&outline(&s, c)) - (40000.0 - 60.0 * 50.0)).abs() < 1.0);
}

/// Regression: a selected subtracted member is a hole in the outline, so a press inside it hit the
/// background, dropped the selection and moved nothing.
#[test]
fn a_selected_member_drags_from_inside_its_hole() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let v = crate::tooling::ViewInfo { smart_guides: false, ..Default::default() };
    // (Group Selection adds the compound shape when its selected member is pressed, as with groups.)
    for tool in ["selection", "directSelection"] {
        let mut s = session();
        let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
        let b = rect(&mut s, 25.0, 25.0, 50.0, 50.0);
        s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
        make(&mut s, "subtract");
        s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
        s.select_tool(tool, v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 50.0), v).unwrap();
        for x in [55.0, 60.0] {
            s.pointer(&PointerEvent::new(PointerKind::Drag, x, 50.0), v).unwrap();
        }
        s.pointer(&PointerEvent::new(PointerKind::Up, 60.0, 50.0), v).unwrap();
        assert_eq!(s.doc().unwrap().selection.objects, vec![b], "{tool}: the member stays selected");
        let bb = node(&s, b).geometric_bounds().unwrap();
        assert!((bb.x0 - 35.0).abs() < 1e-6, "{tool}: the member moved: {bb:?}");
        assert!((node(&s, a).geometric_bounds().unwrap().x0).abs() < 1e-6, "{tool}: the other member stayed");
    }
}

#[test]
fn layers_drags_add_and_remove_members() {
    let mut s = session();
    let (_, b) = two_rects(&mut s);
    let c = make(&mut s, "subtract");
    let x = rect(&mut s, 300.0, 300.0, 10.0, 10.0);
    s.execute("layer.move", &json!({"ids": [x.0], "target": c.0, "place": "inside"})).unwrap();
    let n = node(&s, c);
    assert_eq!(n.children().unwrap().last().map(|m| (m.id, m.shape_mode)), Some((x, ShapeMode::Add)));
    // Out of it again: back to Add, and the compound shape keeps the others.
    let layer = s.doc().unwrap().doc.layers[0].id;
    s.execute("layer.move", &json!({"ids": [b.0], "target": layer.0, "place": "inside"})).unwrap();
    assert_eq!(node(&s, b).shape_mode, ShapeMode::Add);
    assert_eq!(node(&s, c).children().unwrap().len(), 2);
}
