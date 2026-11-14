//! Rotated bounding boxes (#39): a rotation keeps the objects' angle, the bounding box and its
//! handles turn with them, sizes are measured along their own axes, and Reset Bounding Box squares
//! the box to the page again.

use serde_json::{Value, json};
use vectorcraft_doc::{Node, OrientedBox};
use vectorcraft_geom::{Affine, Point, Rect};
use vectorcraft_tools::bbox::Handle;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn sel(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn angle(s: &Session, id: NodeId) -> f64 {
    s.doc().unwrap().doc.node(id).unwrap().bbox_angle
}

fn selection_box(s: &Session) -> OrientedBox {
    s.transform_box(&s.doc().unwrap().selection.objects).unwrap()
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// A 100 × 50 rectangle centred on (200, 200), selected and turned to 45°.
fn turned_rect(s: &mut Session) -> NodeId {
    let a = rect(s, 150.0, 175.0, 100.0, 50.0);
    sel(s, &[a]);
    s.execute("object.rotate", &json!({"angle": 45, "absolute": true})).unwrap();
    a
}

#[test]
fn rotating_to_45_keeps_45() {
    let mut s = session();
    let n = undo_len(&s);
    let a = turned_rect(&mut s);
    assert_eq!(undo_len(&s), n + 2, "the rectangle, then one step for the rotation");
    // The panel's angle and the inspector's.
    let b = selection_box(&s);
    assert!(near(b.angle, 45.0) && near(angle(&s, a), 45.0), "{b:?}");
    assert!(near(s.execute("document.inspect", &json!({})).unwrap()["selectionRotation"].as_f64().unwrap(), 45.0));
    // The box hugs the rectangle: its own sides, its own centre.
    assert!(near(b.rect.width(), 100.0) && near(b.rect.height(), 50.0), "{b:?}");
    assert!(b.center().distance(Point::new(200.0, 200.0)) < 1e-6);
    // Absolute angles: entering 45 again changes nothing, 90 turns it by 45 more.
    s.execute("object.rotate", &json!({"angle": 45, "absolute": true})).unwrap();
    assert!(near(angle(&s, a), 45.0));
    s.execute("object.rotate", &json!({"angle": 90, "absolute": true})).unwrap();
    assert!(near(angle(&s, a), 90.0));
    let b = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    assert!(near(b.width(), 50.0) && near(b.height(), 100.0), "{b:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(angle(&s, a), 45.0));
}

#[test]
fn handles_turn_with_the_object_and_scale_along_its_axes() {
    let mut s = session();
    let a = turned_rect(&mut s);
    let b = selection_box(&s);
    // The Right handle sits on the turned box, 50 pt from the centre along 45° (up and right).
    let right = b.to_doc() * Handle::Right.pos(b.rect);
    let d = std::f64::consts::FRAC_1_SQRT_2 * 50.0;
    assert!(right.distance(Point::new(200.0 + d, 200.0 - d)) < 1e-6, "{right:?}");
    // Drag it 50 pt further out along the object's own x axis.
    let to = Point::new(right.x + d, right.y - d);
    let n = undo_len(&s);
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    for (kind, p) in [(PointerKind::Down, right), (PointerKind::Drag, to), (PointerKind::Up, to)] {
        s.pointer(&PointerEvent::new(kind, p.x, p.y).with_mods(Mods::default()), v).unwrap();
    }
    assert_eq!(undo_len(&s), n + 1);
    // 150 × 50 along its own axes, still at exactly 45°, the left side where it was.
    assert_eq!(angle(&s, a), b.angle);
    let nb = selection_box(&s);
    assert!(near(nb.rect.width(), 150.0) && near(nb.rect.height(), 50.0), "{nb:?}");
    let left = |b: &OrientedBox| b.to_doc() * Handle::Left.pos(b.rect);
    assert!(left(&nb).distance(left(&b)) < 1e-6);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(selection_box(&s).rect.width(), 100.0));
}

/// Presses, drags and lets go with the current tool along `pts`, holding `mods`.
fn drag_with(s: &mut Session, pts: &[Point], mods: Mods) {
    let v = ViewInfo::default();
    let kinds = std::iter::once(PointerKind::Down).chain(std::iter::repeat_n(PointerKind::Drag, pts.len().saturating_sub(2)));
    for (kind, p) in kinds.chain([PointerKind::Up]).zip(pts) {
        s.pointer(&PointerEvent::new(kind, p.x, p.y).with_mods(mods), v).unwrap();
    }
}

/// The anchors of path `id`, on the page.
fn anchors(s: &Session, id: NodeId) -> Vec<Point> {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().anchors().map(|(_, _, an)| an.p).collect()
}

fn has_point(pts: &[Point], p: Point) -> bool {
    pts.iter().any(|q| q.distance(p) < 1e-6)
}

#[test]
fn free_transform_handles_turn_with_the_object_and_scale_along_its_axes() {
    let mut s = session();
    let a = turned_rect(&mut s);
    let b = selection_box(&s);
    // The Right handle of the turned box, dragged 50 pt further out along the object's own x axis.
    let right = b.to_doc() * Handle::Right.pos(b.rect);
    let to = b.to_doc() * (Handle::Right.pos(b.rect) + vectorcraft_geom::Vec2::new(50.0, 0.0));
    let n = undo_len(&s);
    s.select_tool("freeTransform", ViewInfo::default()).unwrap();
    drag_with(&mut s, &[right, to, to], Mods::default());
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Scale"));
    // 150 × 50 along its own axes, still at exactly 45°, the left side where it was.
    assert_eq!(angle(&s, a), b.angle);
    let nb = selection_box(&s);
    assert!(near(nb.rect.width(), 150.0) && near(nb.rect.height(), 50.0), "{nb:?}");
    let left = |b: &OrientedBox| b.to_doc() * Handle::Left.pos(b.rect);
    assert!(left(&nb).distance(left(&b)) < 1e-6);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(selection_box(&s).rect.width(), 100.0));
}

#[test]
fn free_transform_shears_along_the_turned_side() {
    let mut s = session();
    let a = turned_rect(&mut s);
    let b = selection_box(&s);
    let [tl, tr, br, bl] = b.corners();
    // Dragging the Top handle 20 pt along the object's own top side with Cmd held once the drag
    // started (a Cmd press is the temporary Selection tool) slides that side along itself; the
    // bottom side stays.
    let top = b.to_doc() * Handle::Top.pos(b.rect);
    let along = b.to_doc() * (Handle::Top.pos(b.rect) + vectorcraft_geom::Vec2::new(20.0, 0.0));
    let shift = along - top;
    let v = ViewInfo::default();
    s.select_tool("freeTransform", v).unwrap();
    let cmd = Mods { cmd: true, ..Default::default() };
    for (kind, p, m) in [(PointerKind::Down, top, Mods::default()), (PointerKind::Drag, along, cmd), (PointerKind::Up, along, cmd)] {
        s.pointer(&PointerEvent::new(kind, p.x, p.y).with_mods(m), v).unwrap();
    }
    assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Shear"));
    let pts = anchors(&s, a);
    for p in [tl + shift, tr + shift, br, bl] {
        assert!(has_point(&pts, p), "{p:?} not in {pts:?}");
    }
}

#[test]
fn free_transform_distort_starts_from_the_turned_box() {
    let mut s = session();
    let a = turned_rect(&mut s);
    let b = selection_box(&s);
    let [tl, tr, br, bl] = b.corners();
    s.select_tool("freeTransform", ViewInfo::default()).unwrap();
    s.set_tool_option("mode", &json!("distort"));
    let to = br + vectorcraft_geom::Vec2::new(10.0, 20.0);
    let n = undo_len(&s);
    drag_with(&mut s, &[br, to, to], Mods::default());
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!(s.journal.last().map(|j| j.0.as_str()), Some("object.distort"));
    // The dragged corner of the turned box lands under the pointer; the other three stay.
    let pts = anchors(&s, a);
    for p in [tl, tr, to, bl] {
        assert!(has_point(&pts, p), "{p:?} not in {pts:?}");
    }
}

/// Where the line through `a` and `b` crosses the line through `c` and `d`.
fn cross(a: Point, b: Point, c: Point, d: Point) -> Point {
    let (r, q) = (b - a, d - c);
    let t = ((c - a).x * q.y - (c - a).y * q.x) / (r.x * q.y - r.y * q.x);
    a + r * t
}

/// The counter-clockwise angle of the direction from `from` to `to`.
fn direction(from: Point, to: Point) -> f64 {
    -(to.y - from.y).atan2(to.x - from.x).to_degrees()
}

fn near_deg(a: f64, b: f64) -> bool {
    vectorcraft_geom::normalize_deg(a - b).abs() < 1e-6
}

fn json_points(pts: [Point; 4]) -> Value {
    json!(pts.map(|p| [p.x, p.y]))
}

/// `pts` turned by `deg` (counter-clockwise) about `c`.
fn turned(pts: [Point; 4], c: Point, deg: f64) -> [Point; 4] {
    let a = Affine::translate(c.to_vec2()) * Affine::rotate(-deg.to_radians()) * Affine::translate(-c.to_vec2());
    pts.map(|p| a * p)
}

/// A perspective quad for the 100 × 100 box at the origin: its top-right corner raised 20 pt.
const RAISED: [(f64, f64); 4] = [(0.0, 0.0), (100.0, -20.0), (100.0, 100.0), (0.0, 100.0)];

fn raised() -> [Point; 4] {
    RAISED.map(|(x, y)| Point::new(x, y))
}

/// The angle a warp onto `quad` (from a box square to the page) turns an upright object's x axis
/// to, where the object's center lands at `center`: every horizontal line of the box is drawn
/// through the point where the lines of the quad's top and bottom sides cross.
fn upright_angle_after(quad: [Point; 4], center: Point) -> f64 {
    direction(cross(quad[0], quad[1], quad[3], quad[2]), center)
}

/// Where a warped rectangle's center lands: where the diagonals of its warped corners cross.
fn warped_center(s: &Session, id: NodeId) -> Point {
    let p = anchors(s, id);
    assert_eq!(p.len(), 4, "{p:?}");
    cross(p[0], p[2], p[1], p[3])
}

#[test]
fn distort_turns_an_upright_box_with_the_warp_at_its_center() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    let quad = raised();
    s.execute("object.distort", &json!({ "corners": json_points(quad) })).unwrap();
    // The top tilts up to the right: a small counter-clockwise angle (atan 0.1 for this quad).
    let want = upright_angle_after(quad, cross(quad[0], quad[2], quad[1], quad[3]));
    assert!(near(want, 0.1_f64.atan().to_degrees()), "{want}");
    assert!(near(angle(&s, a), want), "{} vs {want}", angle(&s, a));
    // The Selection and Free Transform tools' box stands at it.
    assert!(near(selection_box(&s).angle, want), "{:?}", selection_box(&s));
    // Reset Bounding Box squares it to the page and leaves the art.
    let pts = anchors(&s, a);
    assert_eq!(s.execute("object.resetBoundingBox", &json!({})).unwrap()["changed"], 1);
    assert_eq!(angle(&s, a), 0.0);
    assert_eq!(anchors(&s, a), pts);

    // A warp that only stretches a turned object along its own axes keeps its angle exactly.
    let mut s = session();
    let t = turned_rect(&mut s);
    let b = selection_box(&s);
    let wide = OrientedBox { angle: b.angle, rect: Rect::new(b.rect.x0, b.rect.y0, b.rect.x1 + 50.0, b.rect.y1) };
    let r = b.rect;
    s.execute("object.distort", &json!({"corners": json_points(wide.corners()), "from": [r.x0, r.y0, r.x1, r.y1], "angle": b.angle})).unwrap();
    assert_eq!(angle(&s, t), b.angle);
    assert!(near(selection_box(&s).rect.width(), 150.0), "{:?}", selection_box(&s));
}

/// Every object's angle in `n`'s subtree, opacity mask art included, each with whether it is the
/// art of an unlinked mask (or inside one).
fn angles_with_links(n: &Node, unlinked: bool, out: &mut Vec<(f64, bool)>) {
    out.push((n.bbox_angle, unlinked));
    for c in n.children().into_iter().flatten() {
        angles_with_links(c, unlinked, out);
    }
    if let Some(m) = &n.extras().mask {
        angles_with_links(&m.art, unlinked || !m.linked, out);
    }
}

fn group_angles(s: &Session, g: NodeId) -> Vec<(f64, bool)> {
    let mut out = vec![];
    angles_with_links(s.doc().unwrap().doc.node(g).unwrap(), false, &mut out);
    out
}

fn selected(s: &Session) -> NodeId {
    s.doc().unwrap().selection.objects[0]
}

#[test]
fn distort_turns_each_member_and_linked_mask_by_its_own_warp() {
    let mut s = session();
    // A path turned 20° with a linked mask turned with it.
    let a = rect(&mut s, 100.0, 100.0, 60.0, 40.0);
    let am = rect(&mut s, 100.0, 100.0, 60.0, 40.0);
    sel(&mut s, &[a, am]);
    s.execute("object.rotate", &json!({"angle": 20})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    // A path with an unlinked mask.
    let u = rect(&mut s, 200.0, 100.0, 40.0, 40.0);
    let um = rect(&mut s, 200.0, 100.0, 40.0, 40.0);
    sel(&mut s, &[u, um]);
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    s.execute("transparency.unlinkOpacityMask", &json!({})).unwrap();
    // A compound path turned 10°.
    let c1 = rect(&mut s, 100.0, 200.0, 40.0, 40.0);
    let c2 = rect(&mut s, 120.0, 220.0, 40.0, 40.0);
    sel(&mut s, &[c1, c2]);
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    let compound = selected(&s);
    s.execute("object.rotate", &json!({"angle": 10})).unwrap();
    // A clip group.
    let k1 = rect(&mut s, 250.0, 200.0, 50.0, 50.0);
    let k2 = rect(&mut s, 260.0, 210.0, 50.0, 50.0);
    sel(&mut s, &[k1, k2]);
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    let clip = selected(&s);
    sel(&mut s, &[a, u, compound, clip]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    let before = group_angles(&s, g);
    assert!(before.len() >= 10 && before.iter().any(|(a, _)| near(*a, 20.0)) && before.iter().any(|(_, u)| *u), "{before:?}");
    // A warp that turns the group 30° about its center turns every object 30° more, except the
    // art of the unlinked mask, which stays.
    let b = s.doc().unwrap().doc.bounds_of(&[g], false).unwrap();
    let n = undo_len(&s);
    s.execute("object.distort", &json!({ "corners": json_points(turned(OrientedBox::aligned(b).corners(), b.center(), 30.0)) })).unwrap();
    assert_eq!(undo_len(&s), n + 1, "one step");
    let after = group_angles(&s, g);
    assert_eq!(after.len(), before.len());
    for ((old, unlinked), (new, _)) in before.iter().zip(&after) {
        if *unlinked {
            assert_eq!(new, old, "{before:?} → {after:?}");
        } else {
            assert!(near_deg(*new, old + 30.0), "{before:?} → {after:?}");
        }
    }

    // Under a perspective warp each object follows the warp at its own center: two members of a
    // group, one above the other, tilt by different amounts, and the group by the warp at its own
    // center.
    let mut s = session();
    let p = rect(&mut s, 0.0, 0.0, 100.0, 50.0);
    let q = rect(&mut s, 0.0, 50.0, 100.0, 50.0);
    sel(&mut s, &[p, q]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    let quad = raised();
    s.execute("object.distort", &json!({ "corners": json_points(quad) })).unwrap();
    let (want_p, want_q) = (upright_angle_after(quad, warped_center(&s, p)), upright_angle_after(quad, warped_center(&s, q)));
    assert!(near(angle(&s, p), want_p) && near(angle(&s, q), want_q), "{} {} vs {want_p} {want_q}", angle(&s, p), angle(&s, q));
    assert!(want_p > want_q + 1.0, "{want_p} {want_q}");
    let want_g = upright_angle_after(quad, cross(quad[0], quad[2], quad[1], quad[3]));
    assert!(near(angle(&s, g), want_g), "{} vs {want_g}", angle(&s, g));
}

#[test]
fn mirrored_and_collapsing_distorts_keep_valid_angles() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 50.0);
    sel(&mut s, &[a]);
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    // Corners swapped left for right mirror the art: the box stands at the mirrored angle.
    let b = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    let [tl, tr, br, bl] = OrientedBox::aligned(b).corners();
    s.execute("object.distort", &json!({ "corners": json_points([tr, tl, bl, br]) })).unwrap();
    assert!(near(angle(&s, a), -30.0), "{}", angle(&s, a));
    // A warp that collapses the art to a point leaves the angle as it was.
    let n = undo_len(&s);
    let c = b.center();
    match s.execute("object.distort", &json!({ "corners": json_points([c; 4]) })) {
        Ok(_) => assert!(near(angle(&s, a), -30.0), "{}", angle(&s, a)),
        Err(_) => {
            assert_eq!(undo_len(&s), n);
            assert!(near(angle(&s, a), -30.0));
        }
    }
}

#[test]
fn distort_angles_undo_redo_and_reopen_together() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    let before = anchors(&s, a);
    let n = undo_len(&s);
    let b = Rect::new(0.0, 0.0, 100.0, 100.0);
    s.execute("object.distort", &json!({ "corners": json_points(turned(OrientedBox::aligned(b).corners(), b.center(), 30.0)) })).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Free Distort"));
    assert!(near(angle(&s, a), 30.0), "{}", angle(&s, a));
    let warped = anchors(&s, a);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((anchors(&s, a), angle(&s, a)), (before, 0.0));
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(anchors(&s, a), warped);
    assert!(near(angle(&s, a), 30.0));
    // A native file keeps the angle.
    let data = s.execute("document.save", &json!({})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    s.execute("document.open", &json!({"name": "warped.vectorcraft", "dataBase64": data})).unwrap();
    assert!(near(angle(&s, a), 30.0));
    sel(&mut s, &[a]);
    let b = selection_box(&s);
    assert!(near(b.angle, 30.0) && near(b.rect.width(), 100.0) && near(b.rect.height(), 100.0), "{b:?}");
    // The next handle drag scales along the turned box: the Right handle 50 pt further out.
    let right = b.to_doc() * Handle::Right.pos(b.rect);
    let to = b.to_doc() * (Handle::Right.pos(b.rect) + vectorcraft_geom::Vec2::new(50.0, 0.0));
    s.select_tool("selection", ViewInfo::default()).unwrap();
    drag_with(&mut s, &[right, to, to], Mods::default());
    let nb = selection_box(&s);
    assert!(near(nb.angle, 30.0) && near(nb.rect.width(), 150.0) && near(nb.rect.height(), 100.0), "{nb:?}");
}

#[test]
fn transform_panel_sizes_follow_the_turned_box() {
    let mut s = session();
    let a = turned_rect(&mut s);
    let n = undo_len(&s);
    s.execute("object.setBounds", &json!({"width": 200, "reference": 4})).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    let b = selection_box(&s);
    assert!(near(b.rect.width(), 200.0) && near(b.rect.height(), 50.0) && near(b.angle, 45.0), "{b:?}");
    assert!(b.center().distance(Point::new(200.0, 200.0)) < 1e-6);
    // X/Y place the box's own reference point on the page.
    s.execute("object.setBounds", &json!({"x": 300, "y": 100, "reference": 0})).unwrap();
    let b = selection_box(&s);
    assert!(b.reference_point(0).distance(Point::new(300.0, 100.0)) < 1e-6, "{b:?}");
    assert!(near(angle(&s, a), 45.0));
}

#[test]
fn reset_bounding_box_squares_the_box_in_one_step() {
    let mut s = session();
    let a = turned_rect(&mut s);
    let geometry = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    let n = undo_len(&s);
    assert_eq!(s.execute("object.resetBoundingBox", &json!({})).unwrap()["changed"], 1);
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!(angle(&s, a), 0.0);
    // The art stays; the box is square to the page around it.
    let b = selection_box(&s);
    assert_eq!(b.angle, 0.0);
    assert_eq!(b.rect, geometry);
    // Nothing left to reset: no step.
    assert_eq!(s.execute("object.resetBoundingBox", &json!({})).unwrap()["changed"], 0);
    assert_eq!(undo_len(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(angle(&s, a), 45.0));
}

/// The angles of every object in `n`'s subtree, opacity mask art included, layers left out.
fn subtree_angles(n: &Node, out: &mut Vec<f64>) {
    if !n.is_layer() {
        out.push(n.bbox_angle);
    }
    for c in n.children().into_iter().flatten() {
        subtree_angles(c, out);
    }
    if let Some(m) = &n.extras().mask {
        subtree_angles(&m.art, out);
    }
}

/// The angles of every object in the document, opacity mask art included, layers left out.
fn object_angles(s: &Session) -> Vec<f64> {
    let mut out = vec![];
    for l in &s.doc().unwrap().doc.layers {
        subtree_angles(l, &mut out);
    }
    out
}

#[test]
fn reset_bounding_box_squares_the_objects_inside_a_group() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    let b = rect(&mut s, 300.0, 100.0, 100.0, 50.0);
    sel(&mut s, &[a, b]);
    s.execute("object.rotate", &json!({"angle": 20})).unwrap();
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("object.rotate", &json!({"angle": 10})).unwrap();
    assert!(near(angle(&s, g), 10.0) && near(angle(&s, a), 30.0) && near(angle(&s, b), 30.0));
    let n = undo_len(&s);
    // The group and both members, in one step.
    assert_eq!(s.execute("object.resetBoundingBox", &json!({})).unwrap()["changed"], 3);
    assert_eq!(undo_len(&s), n + 1);
    assert!(object_angles(&s).iter().all(|a| *a == 0.0), "{:?}", object_angles(&s));
    s.execute("object.ungroup", &json!({})).unwrap();
    assert_eq!(selection_box(&s).angle, 0.0);
    assert_eq!(s.execute("object.resetBoundingBox", &json!({})).unwrap()["changed"], 0);
    // Undo brings every angle back.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(angle(&s, g), 10.0) && near(angle(&s, a), 30.0) && near(angle(&s, b), 30.0));
}

#[test]
fn reset_bounding_box_squares_the_turned_members_of_a_square_group() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    let b = rect(&mut s, 300.0, 100.0, 100.0, 50.0);
    sel(&mut s, &[a, b]);
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    assert_eq!(selection_box(&s).angle, 0.0);
    let n = undo_len(&s);
    assert_eq!(s.execute("object.resetBoundingBox", &json!({})).unwrap()["changed"], 2);
    assert_eq!(undo_len(&s), n + 1);
    s.execute("object.ungroup", &json!({})).unwrap();
    assert_eq!(selection_box(&s).angle, 0.0);
}

/// Two rectangles turned 20°, put in a container by `make`, turned 10° with it and reset, then
/// released by `release`: nothing stands at an angle before or after the release.
fn reset_then_release(make: &str, release: &str) -> Session {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    let b = rect(&mut s, 150.0, 120.0, 100.0, 50.0);
    sel(&mut s, &[a, b]);
    s.execute("object.rotate", &json!({"angle": 20})).unwrap();
    s.execute(make, &json!({})).unwrap();
    s.execute("object.rotate", &json!({"angle": 10})).unwrap();
    assert!(object_angles(&s).iter().any(|a| near(*a, 30.0)), "{make}: {:?}", object_angles(&s));
    s.execute("object.resetBoundingBox", &json!({})).unwrap();
    assert!(object_angles(&s).iter().all(|a| *a == 0.0), "{make}: {:?}", object_angles(&s));
    s.execute(release, &json!({})).unwrap();
    assert!(object_angles(&s).iter().all(|a| *a == 0.0), "{release}: {:?}", object_angles(&s));
    s.execute("select.all", &json!({})).unwrap();
    assert_eq!(s.execute("object.resetBoundingBox", &json!({})).unwrap()["changed"], 0, "{release}");
    s
}

#[test]
fn reset_bounding_box_then_release_clipping_mask() {
    reset_then_release("object.clippingMask.make", "object.clippingMask.release");
}

#[test]
fn reset_bounding_box_then_release_compound_path() {
    reset_then_release("object.compoundPath.make", "object.compoundPath.release");
}

#[test]
fn reset_bounding_box_then_expand_blend() {
    reset_then_release("object.blend.make", "object.blend.expand");
}

#[test]
fn reset_bounding_box_then_release_envelope() {
    reset_then_release("object.envelope.makeWithWarp", "object.envelope.release");
}

#[test]
fn reset_bounding_box_then_release_repeat() {
    reset_then_release("object.repeat.grid", "object.repeat.release");
}

#[test]
fn reset_bounding_box_then_release_intertwine() {
    reset_then_release("object.intertwine.make", "object.intertwine.release");
}

/// A symbol of two rectangles, the first turned `inner` degrees before the symbol is made, placed
/// as an instance that is then turned 20°.
fn turned_instance(inner: f64) -> (Session, NodeId) {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    let b = rect(&mut s, 150.0, 220.0, 100.0, 50.0);
    if inner != 0.0 {
        sel(&mut s, &[a]);
        s.execute("object.rotate", &json!({"angle": inner})).unwrap();
    }
    sel(&mut s, &[a, b]);
    let inst = id_of(&s.execute("symbol.new", &json!({"name": "Pair"})).unwrap());
    sel(&mut s, &[inst]);
    s.execute("object.rotate", &json!({"angle": 20})).unwrap();
    assert!(near(angle(&s, inst), 20.0));
    (s, inst)
}

/// Break Link and Expand with the parameters that turn instance `inst` into its art.
fn expand_instance(s: &mut Session, command: &str, inst: NodeId) {
    sel(s, &[inst]);
    let params = if command == "symbol.breakLink" { json!({"ids": [inst.0]}) } else { json!({"object": true, "fill": false, "stroke": false}) };
    s.execute(command, &params).unwrap();
    assert!(s.doc().unwrap().doc.node(inst).is_none_or(|n| !matches!(n.kind, vectorcraft_doc::NodeKind::SymbolInstance { .. })), "{command}");
}

/// The angles of the objects inside the document's only top-level object, that object included.
fn art_angles(s: &Session) -> Vec<f64> {
    let d = &s.doc().unwrap().doc;
    let layer = d.layers.first().unwrap();
    let top = layer.children().unwrap();
    assert_eq!(top.len(), 1);
    let mut out = vec![];
    subtree_angles(&top[0], &mut out);
    out
}

#[test]
fn reset_bounding_box_then_expand_symbol_instance() {
    for command in ["symbol.breakLink", "object.expand"] {
        let (mut s, inst) = turned_instance(0.0);
        assert_eq!(s.execute("object.resetBoundingBox", &json!({})).unwrap()["changed"], 1);
        assert_eq!(angle(&s, inst), 0.0);
        expand_instance(&mut s, command, inst);
        let angles = art_angles(&s);
        assert!(angles.len() >= 2 && angles.iter().all(|a| near(*a, 0.0)), "{command}: {angles:?}");
        s.execute("select.all", &json!({})).unwrap();
        assert_eq!(selection_box(&s).angle, 0.0, "{command}");
    }
}

#[test]
fn expanding_a_turned_symbol_instance_keeps_its_angle() {
    for command in ["symbol.breakLink", "object.expand"] {
        let (mut s, inst) = turned_instance(0.0);
        expand_instance(&mut s, command, inst);
        let angles = art_angles(&s);
        assert!(angles.len() >= 2 && angles.iter().all(|a| near(*a, 20.0)), "{command}: {angles:?}");
    }
}

#[test]
fn reset_bounding_box_then_expand_keeps_the_angles_inside_the_symbol() {
    for command in ["symbol.breakLink", "object.expand"] {
        let (mut s, inst) = turned_instance(30.0);
        s.execute("object.resetBoundingBox", &json!({})).unwrap();
        expand_instance(&mut s, command, inst);
        // The turned rectangle keeps the 30° it had in the symbol; the rest stand square.
        let mut angles = art_angles(&s);
        angles.sort_by(f64::total_cmp);
        assert!(near(*angles.last().unwrap(), 30.0) && angles[..angles.len() - 1].iter().all(|a| near(*a, 0.0)), "{command}: {angles:?}");
    }
}

/// The group that an instance's opacity adds around the art of a one-object symbol stands as the
/// instance's box does, turned or reset.
#[test]
fn expanding_a_faded_symbol_instance_gives_its_group_the_instance_box() {
    for command in ["symbol.breakLink", "object.expand"] {
        for reset in [false, true] {
            let mut s = session();
            let a = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
            sel(&mut s, &[a]);
            let inst = id_of(&s.execute("symbol.new", &json!({"name": "One"})).unwrap());
            sel(&mut s, &[inst]);
            s.execute("object.rotate", &json!({"angle": 20})).unwrap();
            s.execute("object.setProps", &json!({"ids": [inst.0], "opacity": 50})).unwrap();
            if reset {
                s.execute("object.resetBoundingBox", &json!({})).unwrap();
            }
            expand_instance(&mut s, command, inst);
            let want = if reset { 0.0 } else { 20.0 };
            let angles = art_angles(&s);
            assert!(angles.len() == 2 && angles.iter().all(|a| near(*a, want)), "{command}, reset {reset}: {angles:?}");
        }
    }
}

/// A mirrored instance: turned 170° and then reflected, its box stands 180° from the box its
/// transform gives in one step. The art takes the instance's box, and stands square after a reset.
#[test]
fn expanding_a_reflected_symbol_instance_keeps_its_box() {
    for command in ["symbol.breakLink", "object.expand"] {
        for reset in [false, true] {
            let (mut s, inst) = turned_instance(0.0);
            s.execute("object.rotate", &json!({"angle": 150})).unwrap();
            s.execute("object.reflect", &json!({"axis": "horizontal"})).unwrap();
            assert!(near(angle(&s, inst), -170.0), "{}", angle(&s, inst));
            if reset {
                s.execute("object.resetBoundingBox", &json!({})).unwrap();
            }
            let want = angle(&s, inst);
            expand_instance(&mut s, command, inst);
            let angles = art_angles(&s);
            assert!(angles.len() >= 2 && angles.iter().all(|a| near(*a, want)), "{command}, reset {reset}: {angles:?}");
        }
    }
}

#[test]
fn reset_bounding_box_then_release_linked_opacity_mask() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    let b = rect(&mut s, 150.0, 120.0, 100.0, 50.0);
    sel(&mut s, &[a, b]);
    s.execute("object.rotate", &json!({"angle": 20})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    let mask = s.doc().unwrap().doc.node(a).unwrap().extras().mask.clone().unwrap();
    assert!(mask.linked && near(mask.art.bbox_angle, 20.0));
    drop(s);
    reset_then_release("transparency.makeOpacityMask", "transparency.releaseOpacityMask");
}

#[test]
fn reset_bounding_box_leaves_the_art_of_an_unlinked_opacity_mask() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    let b = rect(&mut s, 150.0, 120.0, 100.0, 50.0);
    sel(&mut s, &[a, b]);
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    s.execute("object.rotate", &json!({"angle": 20})).unwrap();
    s.execute("transparency.unlinkOpacityMask", &json!({})).unwrap();
    let mask_angle = |s: &Session| s.doc().unwrap().doc.node(a).unwrap().extras().mask.as_ref().unwrap().art.bbox_angle;
    assert!(near(angle(&s, a), 20.0) && near(mask_angle(&s), 20.0));
    // Only the object: its mask's art no longer turns with it.
    assert_eq!(s.execute("object.resetBoundingBox", &json!({})).unwrap()["changed"], 1);
    assert_eq!(angle(&s, a), 0.0);
    assert!(near(mask_angle(&s), 20.0));
}

#[test]
fn page_flips_keep_an_upright_box_upright() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 50.0);
    sel(&mut s, &[a]);
    s.execute("object.reflect", &json!({"axis": "vertical"})).unwrap();
    s.execute("object.reflect", &json!({"axis": "horizontal"})).unwrap();
    assert_eq!(angle(&s, a), 0.0);
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    // A named axis is the turned box's own: the box keeps its angle.
    s.execute("object.reflect", &json!({"axis": "vertical"})).unwrap();
    assert!(near(angle(&s, a), 30.0));
    s.execute("object.reflect", &json!({"axis": "vertical", "basis": "page"})).unwrap();
    assert!(near(angle(&s, a), -30.0));
}

fn same_matrix(a: Affine, b: Affine) -> bool {
    a.as_coeffs().iter().zip(b.as_coeffs()).all(|(x, y)| near(*x, y))
}

/// After a rotation, Scale and a named Reflect act along the turned bounding box until Reset
/// Bounding Box; `basis: "page"` keeps the page's axes, and Transform Again repeats the matrix the
/// transform applied on the page.
#[test]
fn scale_and_reflect_follow_the_turned_box() {
    let mut s = session();
    let a = turned_rect(&mut s);
    let page = |s: &Session, ids: &[NodeId]| s.doc().unwrap().doc.bounds_of(ids, false).unwrap();
    let c = Point::new(200.0, 200.0);
    let frame = OrientedBox { angle: 45.0, rect: Rect::ZERO };
    s.execute("object.scale", &json!({"sx": 200, "sy": 100})).unwrap();
    let b = selection_box(&s);
    assert!(near(b.angle, 45.0) && near(angle(&s, a), 45.0), "{b:?}");
    assert!(near(b.rect.width(), 200.0) && near(b.rect.height(), 50.0), "{b:?}");
    assert!(b.center().distance(c) < 1e-6, "{b:?}");
    let want = Affine::translate(c.to_vec2()) * frame.conjugate(Affine::scale_non_uniform(2.0, 1.0)) * Affine::translate(-c.to_vec2());
    let Some(LastTransform::Matrix(m, _)) = s.doc().unwrap().last_transform.clone() else { panic!("no transform to repeat") };
    assert!(same_matrix(m, want), "{m:?}");
    // The recorded step names its basis, so a replay acts in the same frame.
    let steps = s.journal_for_action(0);
    assert!(steps.iter().any(|(id, p)| id == "object.scale" && p["basis"] == "box"), "{steps:?}");

    // Reflected across the box's vertical axis through its left side: the box keeps its angle and
    // size, and its centre moves a width to the left along the box.
    let left = b.reference_point(3);
    s.execute("object.reflect", &json!({"axis": "vertical", "origin": [left.x, left.y]})).unwrap();
    let r = selection_box(&s);
    let moved = c + (frame.to_doc() * Point::new(-200.0, 0.0)).to_vec2();
    assert!(near(r.angle, 45.0) && near(r.rect.width(), 200.0) && near(r.rect.height(), 50.0), "{r:?}");
    assert!(r.center().distance(moved) < 1e-6, "{r:?}");
    // Across its horizontal axis through its centre: the box stays where it is.
    s.execute("object.reflect", &json!({"axis": "horizontal"})).unwrap();
    assert!(selection_box(&s).center().distance(moved) < 1e-6 && near(angle(&s, a), 45.0));

    // Unknown bases are errors and change nothing.
    let n = undo_len(&s);
    assert!(s.execute("object.scale", &json!({"sx": 50, "basis": "turned"})).is_err());
    assert!(s.execute("object.reflect", &json!({"axis": "vertical", "basis": 1})).is_err());
    assert!(s.execute("object.reflect", &json!({"axis": 30, "basis": "turned"})).is_err());
    assert_eq!(undo_len(&s), n);

    // On the page's axes, the box no longer stands at 45°.
    s.execute("object.scale", &json!({"sx": 50, "sy": 100, "basis": "page"})).unwrap();
    assert!(!near(angle(&s, a), 45.0));
    // Transform Again repeats the page matrix after Reset Bounding Box too.
    s.execute("object.scale", &json!({"sx": 200, "sy": 100})).unwrap();
    let Some(LastTransform::Matrix(m, _)) = s.doc().unwrap().last_transform.clone() else { panic!("no transform to repeat") };
    s.execute("object.resetBoundingBox", &json!({})).unwrap();
    let before = s.doc().unwrap().doc.node(a).unwrap().clone();
    s.execute("object.transformAgain", &json!({})).unwrap();
    let mut want = before;
    want.transform(m, false);
    let (got, want) = (page(&s, &[a]), want.geometric_bounds_with(&s.doc().unwrap().doc.symbol_boxes()).unwrap());
    assert!([got.x0 - want.x0, got.y0 - want.y0, got.x1 - want.x1, got.y1 - want.y1].iter().all(|d| d.abs() < 1e-6), "{got:?} {want:?}");
}

/// Once Reset Bounding Box squares the box, and for objects standing at different angles, Scale
/// acts along the page's axes.
#[test]
fn scale_follows_the_page_after_a_reset_and_for_mixed_angles() {
    let mut s = session();
    let a = turned_rect(&mut s);
    s.execute("object.resetBoundingBox", &json!({})).unwrap();
    let page = |s: &Session, ids: &[NodeId]| s.doc().unwrap().doc.bounds_of(ids, false).unwrap();
    let w = page(&s, &[a]);
    s.execute("object.scale", &json!({"sx": 200, "sy": 100})).unwrap();
    let r = page(&s, &[a]);
    assert!(near(r.width(), 2.0 * w.width()) && near(r.height(), w.height()), "{w:?} {r:?}");
    assert_eq!(angle(&s, a), 0.0);
    // A turned rectangle with an upright one: the box is square to the page.
    let b = turned_rect(&mut s);
    let u = rect(&mut s, 400.0, 100.0, 50.0, 50.0);
    sel(&mut s, &[b, u]);
    let w = page(&s, &[b, u]);
    s.execute("object.scale", &json!({"sx": 100, "sy": 200})).unwrap();
    let r = page(&s, &[b, u]);
    assert!(near(r.width(), w.width()) && near(r.height(), 2.0 * w.height()), "{w:?} {r:?}");
}

/// With anchors selected, Scale and a named Reflect move the anchors along the page's axes, but
/// with Transform Objects off they move the whole objects' pattern tiles, along the turned box.
#[test]
fn pattern_only_transforms_of_selected_anchors_follow_the_turned_box() {
    let c = Point::new(200.0, 200.0);
    let frame = OrientedBox { angle: 45.0, rect: Rect::ZERO };
    let about = |m: Affine| Affine::translate(c.to_vec2()) * m * Affine::translate(-c.to_vec2());
    let last = |s: &Session| match s.doc().unwrap().last_transform.clone() {
        Some(LastTransform::Matrix(m, _)) => m,
        other => panic!("no matrix to repeat: {other:?}"),
    };
    let scale = Affine::scale_non_uniform(2.0, 1.0);
    let flip = Affine::scale_non_uniform(-1.0, 1.0);
    for (cmd, p, m) in [("object.scale", json!({"sx": 200, "sy": 100}), scale), ("object.reflect", json!({"axis": "vertical"}), flip)] {
        for objects in [false, true] {
            let mut s = session();
            let a = turned_rect(&mut s);
            s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0]], "mode": "set"})).unwrap();
            let mut p = p.clone();
            p["origin"] = json!([c.x, c.y]);
            p["objects"] = json!(objects);
            p["patterns"] = json!(true);
            s.execute(cmd, &p).unwrap();
            let want = if objects { about(m) } else { about(frame.conjugate(m)) };
            assert!(same_matrix(last(&s), want), "{cmd} objects {objects}: {:?}", last(&s));
        }
    }
}

#[test]
fn groups_type_and_multiple_selections_keep_their_angle() {
    let mut s = session();
    // Objects turned together share the box.
    let a = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    let b = rect(&mut s, 300.0, 100.0, 100.0, 50.0);
    sel(&mut s, &[a, b]);
    s.execute("object.rotate", &json!({"angle": 20})).unwrap();
    assert!(near(selection_box(&s).angle, 20.0) && near(angle(&s, a), 20.0) && near(angle(&s, b), 20.0));
    // Mixed angles: the box is square to the page.
    let c = rect(&mut s, 100.0, 300.0, 50.0, 50.0);
    sel(&mut s, &[a, c]);
    assert_eq!(selection_box(&s).angle, 0.0);
    // A group turns as one; its members keep their own angles inside it.
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    assert_eq!(selection_box(&s).angle, 0.0);
    s.execute("object.rotate", &json!({"angle": 30, "absolute": true})).unwrap();
    assert!(near(angle(&s, g), 30.0) && near(angle(&s, a), 50.0) && near(angle(&s, c), 30.0));
    s.execute("object.ungroup", &json!({})).unwrap();
    sel(&mut s, &[c]);
    let cb = selection_box(&s);
    assert!(near(cb.angle, 30.0) && near(cb.rect.width(), 50.0) && near(cb.rect.height(), 50.0), "{cb:?}");
    // Type keeps its angle too.
    let t = id_of(&s.execute("text.create", &json!({"x": 50, "y": 500, "text": "Turn"})).unwrap());
    sel(&mut s, &[t]);
    let before = selection_box(&s).rect;
    s.execute("object.rotate", &json!({"angle": 60, "absolute": true})).unwrap();
    let tb = selection_box(&s);
    assert!(near(tb.angle, 60.0), "{tb:?}");
    assert!(near(tb.rect.width(), before.width()) && near(tb.rect.height(), before.height()), "{tb:?} vs {before:?}");
}

#[test]
fn native_files_keep_the_angle() {
    let mut s = session();
    let plain = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let a = turned_rect(&mut s);
    let data = s.execute("document.save", &json!({})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    let text = String::from_utf8(vectorcraft_format::base64_decode(&data).unwrap()).unwrap();
    // Only turned objects write it.
    assert_eq!(text.matches("bbox_angle").count(), 1);
    s.execute("document.open", &json!({"name": "turned.vectorcraft", "dataBase64": data})).unwrap();
    assert!(near(angle(&s, a), 45.0));
    assert_eq!(angle(&s, plain), 0.0);
    sel(&mut s, &[a]);
    assert!(near(selection_box(&s).rect.width(), 100.0));
}

#[test]
fn rotated_box_rect_matches_page_bounds_when_upright() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 20.0, 30.0, 40.0);
    sel(&mut s, &[a]);
    assert_eq!(selection_box(&s), OrientedBox::aligned(Rect::new(10.0, 20.0, 40.0, 60.0)));
}

/// The Selection tool says when a drag moves, scales or rotates the selection (the canvas hides the
/// bounding box meanwhile, #712), and not on a press that hasn't moved or a marquee.
#[test]
fn the_selection_tool_reports_a_transforming_drag() {
    let mut s = session();
    let id = id_of(&s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 50})).unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let ev = |kind, x: f64, y: f64| PointerEvent::new(kind, x, y).with_mods(Mods::default());
    s.pointer(&ev(PointerKind::Down, 150.0, 125.0), v).unwrap();
    assert!(!s.tool_transforming(), "a press that hasn't moved");
    s.pointer(&ev(PointerKind::Drag, 190.0, 160.0), v).unwrap();
    assert!(s.tool_transforming(), "moving");
    s.pointer(&ev(PointerKind::Up, 190.0, 160.0), v).unwrap();
    assert!(!s.tool_transforming());
    // A marquee on empty canvas isn't a transform.
    s.pointer(&ev(PointerKind::Down, 500.0, 400.0), v).unwrap();
    s.pointer(&ev(PointerKind::Drag, 550.0, 450.0), v).unwrap();
    assert!(s.tool_busy() && !s.tool_transforming());
    s.pointer(&ev(PointerKind::Up, 550.0, 450.0), v).unwrap();
}

/// With General › Show Bounding Box While Moving and Drawing on, a move drag's bounding box is the
/// box measured once, at the drag's start, offset by each step's move: with Real-time Drawing on
/// and off, the later steps do not measure the selection again. Off, only the Selection tool with
/// Real-time Drawing off gives a box (the art waits for the release).
#[test]
fn a_move_drag_measures_the_bounding_box_once() {
    for (real_time, while_moving) in [(true, true), (false, true), (true, false), (false, false)] {
        let what = format!("real time {real_time}, box while moving {while_moving}");
        let mut s = session();
        let id = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
        sel(&mut s, &[id]);
        s.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": real_time})).unwrap();
        s.execute("prefs.set", &json!({"key": "boundingBoxWhileMovingAndDrawing", "value": while_moving})).unwrap();
        let v = ViewInfo::default();
        s.select_tool("selection", v).unwrap();
        let ev = |kind, x: f64, y: f64| PointerEvent::new(kind, x, y).with_mods(Mods::default());
        s.pointer(&ev(PointerKind::Down, 150.0, 125.0), v).unwrap();
        assert_eq!(s.move_drag_box(), None, "{what}: not before the drag begins");
        s.pointer(&ev(PointerKind::Drag, 170.0, 135.0), v).unwrap();
        if real_time && !while_moving {
            assert_eq!(s.move_drag_box(), None, "{what}: no box during the move");
            s.pointer(&ev(PointerKind::Up, 170.0, 135.0), v).unwrap();
            continue;
        }
        assert_eq!(s.move_drag_box(), Some(OrientedBox::aligned(Rect::new(120.0, 110.0, 220.0, 160.0))), "{what}");
        // The measurement made at the drag's start is the one each step offsets: replaced by a
        // stand-in, the next step's box is the stand-in moved.
        let it = s.doc_mut().unwrap().interaction.as_mut().unwrap();
        assert_eq!(it.move_box.get(), Some(&Some(OrientedBox::aligned(Rect::new(100.0, 100.0, 200.0, 150.0)))), "{what}: measured at the start");
        it.move_box = std::sync::OnceLock::from(Some(OrientedBox::aligned(Rect::new(0.0, 0.0, 10.0, 10.0))));
        s.pointer(&ev(PointerKind::Drag, 200.0, 165.0), v).unwrap();
        assert_eq!(s.move_drag_box(), Some(OrientedBox::aligned(Rect::new(50.0, 40.0, 60.0, 50.0))), "{what}: not measured again");
        s.pointer(&ev(PointerKind::Up, 200.0, 165.0), v).unwrap();
        assert_eq!(s.move_drag_box(), None, "{what}: none after the release");
        assert_eq!(selection_box(&s), OrientedBox::aligned(Rect::new(150.0, 140.0, 250.0, 190.0)), "{what}: moved");
    }
}

/// A shape being drawn has no preview applied after a step that failed (with Real-time Drawing on,
/// also when the failed command left a document other than the interaction's snapshot), and with
/// Real-time Drawing off, where the document stays at the snapshot. A successful step with Real-time
/// Drawing on applies it.
#[test]
fn a_shape_being_drawn_reports_whether_a_preview_is_applied() {
    for real_time in [true, false] {
        let what = format!("real time {real_time}");
        let mut s = session();
        s.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": real_time})).unwrap();
        let v = ViewInfo::default();
        s.select_tool("rectangle", v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, 160.0, 140.0), v).unwrap();
        assert_eq!(s.interaction_unapplied(), !real_time, "{what}: after a step that succeeded");
        assert!(s.preview("shape.rectangle", &json!({"x": "left"})).is_err(), "{what}: the preview fails");
        assert!(s.interaction_unapplied(), "{what}: after a step that failed");
        if real_time {
            // A failed command that copied the document on write leaves another document than the
            // snapshot; no preview is applied all the same.
            let st = s.doc_mut().unwrap();
            st.doc = std::sync::Arc::new((*st.doc).clone());
            assert!(s.interaction_unapplied(), "{what}: a failed step with a copied document");
        }
        s.pointer(&PointerEvent::new(PointerKind::Drag, 180.0, 150.0), v).unwrap();
        assert_eq!(s.interaction_unapplied(), !real_time, "{what}: the next step succeeded");
        s.pointer(&PointerEvent::new(PointerKind::Up, 180.0, 150.0), v).unwrap();
        assert!(!s.interaction_unapplied(), "{what}: no interaction after the release");
    }
}
