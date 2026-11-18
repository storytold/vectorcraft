use super::*;
use crate::testutil::*;
use vectorcraft_doc::Selection;

#[test]
fn rect_drag() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeTool::new("rectangle");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 50.0, 30.0));
    assert_eq!(a[0], Action::Begin("Rectangle".into()));
    assert_eq!(a[1], Action::Preview("shape.rectangle".into(), json!({"x": 10.0, "y": 10.0, "width": 40.0, "height": 20.0})));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 50.0, 30.0)), vec![Action::Commit]);
}

#[test]
fn space_moves_the_shape_being_drawn() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let space = Mods { space: true, ..Default::default() };
    let mut t = ShapeTool::new("rectangle");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 120.0));
    // Space held: the 100 × 70 rectangle follows the pointer.
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 250.0, 170.0).with_mods(space));
    assert_eq!(a.last(), Some(&Action::Preview("shape.rectangle".into(), json!({"x": 150.0, "y": 100.0, "width": 100.0, "height": 70.0}))));
    // Let go of Space: it grows again, from where it was moved to.
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 270.0, 190.0));
    assert_eq!(a.last(), Some(&Action::Preview("shape.rectangle".into(), json!({"x": 150.0, "y": 100.0, "width": 120.0, "height": 90.0}))));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 270.0, 190.0)), vec![Action::Commit]);
    // A star (drawn from its centre) moves its centre.
    let mut t = ShapeTool::new("star");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 80.0, 50.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 90.0, 60.0).with_mods(space));
    assert!(matches!(a.last(), Some(Action::Preview(_, v)) if v["cx"] == 60.0 && v["cy"] == 60.0), "{a:?}");
}

#[test]
fn star_drag_measure_label_shows_outer_and_inner_radii() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = ToolContext { measurement_labels: true, smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ShapeTool::new("star");
    t.star_ratio = 0.5;
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 50.0, 10.0));
    let want = cx.star_radii_label(40.0, 20.0);
    let label = t
        .overlays(&cx)
        .into_iter()
        .find_map(|o| match o {
            Overlay::Measure { text, .. } => Some(text),
            _ => None,
        })
        .unwrap_or_default();
    assert_eq!(label, want, "{label}");
}

/// The star label shows the radii the preview draws, also with Ctrl/Cmd + Alt and when a
/// Ctrl/Cmd drag shrinks the outer radius below the held inner one (the ratio is clamped).
#[test]
fn star_measure_label_matches_the_preview_with_modifiers_and_shrink() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = ToolContext { measurement_labels: true, smart_guides: false, ..cx(&d, &s, &p) };
    let label = |t: &ShapeTool| {
        t.overlays(&cx)
            .into_iter()
            .find_map(|o| match o {
                Overlay::Measure { text, .. } => Some(text),
                _ => None,
            })
            .unwrap_or_default()
    };
    let radii = |a: &[Action]| match a.last() {
        Some(Action::Preview(_, v)) => (v["radius1"].as_f64().unwrap(), v["radius2"].as_f64().unwrap()),
        other => panic!("{other:?}"),
    };
    let cmd = Mods { cmd: true, ..Default::default() };
    let cmd_alt = Mods { cmd: true, alt: true, ..Default::default() };
    let mut t = ShapeTool::new("star");
    t.star_ratio = 0.5;
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 50.0, 10.0));
    // Ctrl/Cmd holds the 20 inner radius while the outer one grows.
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 70.0, 10.0).with_mods(cmd));
    let (r1, r2) = radii(&a);
    assert_eq!(label(&t), cx.star_radii_label(r1, r2));
    // Ctrl/Cmd + Alt: the preview straightens the shoulders, and so does the label.
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 70.0, 10.0).with_mods(cmd_alt));
    let (r1, r2) = radii(&a);
    assert!((r2 - 20.0).abs() > 1e-6, "Alt should override the held radius: {r2}");
    assert_eq!(label(&t), cx.star_radii_label(r1, r2));
    // Shrinking below the held inner radius clamps the ratio; the label follows the preview.
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 20.0, 10.0).with_mods(cmd));
    let (r1, r2) = radii(&a);
    assert!(r2 <= r1 + 1e-9, "{r1} {r2}");
    assert_eq!(label(&t), cx.star_radii_label(r1, r2));
}

/// During a star drag, Ctrl/Cmd keeps the inner radius the star showed when the key went down
/// while the outer one follows the pointer (the new ratio stays once the key is let go), and Alt
/// keeps the shoulders straight (#812). The canvas sends a drag only once the pointer moves, so
/// the key first arrives with a new position.
#[test]
fn a_star_drag_holds_its_inner_radius_with_ctrl_and_straightens_with_alt() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let radii = |a: &[Action]| match a.last() {
        Some(Action::Preview(_, v)) => (v["radius1"].as_f64().unwrap(), v["radius2"].as_f64().unwrap()),
        other => panic!("{other:?}"),
    };
    let ctrl = Mods { cmd: true, ..Default::default() };
    let mut t = ShapeTool::new("star");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
    let (r1, r2) = radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 90.0, 50.0)));
    assert_eq!((r1, r2), (40.0, 20.0));
    // Ctrl, pressed with the pointer still: the inner radius stays 20 as the outer one grows to 80.
    let (r1, r2) = radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 130.0, 50.0).with_mods(ctrl)));
    assert!((r1 - 80.0).abs() < 1e-9 && (r2 - 20.0).abs() < 1e-9, "{r1} {r2}");
    // Let go: the ratio (a quarter) stays.
    let (r1, r2) = radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 50.0)));
    assert!((r2 / r1 - 0.25).abs() < 1e-9, "{r1} {r2}");
    // Alt: straight shoulders, about 0.382 for five points.
    let alt = Mods { alt: true, ..Default::default() };
    let (r1, r2) = radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 50.0).with_mods(alt)));
    assert!((r2 / r1 - 0.381_966).abs() < 1e-5, "{}", r2 / r1);
    assert_eq!(straight_ratio(4), None, "four points can't line up");
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 50.0)), vec![Action::Commit]);
}

/// Ctrl/Cmd pressed while Alt straightens the shoulders keeps the straightened inner radius, a drag
/// canceled with Escape leaves no held radius for the next star, and Ctrl/Cmd pressed inside the
/// drag threshold keeps the inner radius of the first star the drag shows. The canvas lends a
/// selection tool to a press with Ctrl/Cmd held, so every press here is without it.
#[test]
fn a_star_drag_holds_the_inner_radius_it_shows_when_ctrl_goes_down() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let radii = |a: &[Action]| match a.last() {
        Some(Action::Preview(_, v)) => (v["radius1"].as_f64().unwrap(), v["radius2"].as_f64().unwrap()),
        other => panic!("{other:?}"),
    };
    let close = |(r1, r2): (f64, f64), (w1, w2): (f64, f64)| assert!((r1 - w1).abs() < 1e-9 && (r2 - w2).abs() < 1e-9, "{r1} {r2}, not {w1} {w2}");
    let ctrl = Mods { cmd: true, ..Default::default() };
    let alt = Mods { alt: true, ..Default::default() };
    let ctrl_alt = Mods { cmd: true, alt: true, ..Default::default() };
    // Alt shows straight shoulders; Ctrl/Cmd pressed then keeps that inner radius.
    let mut t = ShapeTool::new("star");
    let straight = straight_ratio(5).unwrap_or(0.0);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
    close(radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 90.0, 50.0).with_mods(alt))), (40.0, 40.0 * straight));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 130.0, 50.0).with_mods(ctrl_alt));
    close(radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 170.0, 50.0).with_mods(ctrl))), (120.0, 40.0 * straight));

    // Escape with the key held, then a new press and the key before the first move: the next star
    // has the ratio from before the canceled drag and holds the inner radius it shows, not the
    // canceled one.
    let mut t = ShapeTool::new("star");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 90.0, 50.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 130.0, 50.0).with_mods(ctrl));
    assert_eq!(t.key(&cx, ToolKey::Escape, ctrl), vec![Action::Cancel]);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
    close(radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 60.0, 50.0).with_mods(ctrl))), (10.0, 5.0));
    close(radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 70.0, 50.0).with_mods(ctrl))), (20.0, 5.0));

    // Ctrl/Cmd pressed inside the drag threshold, where no star is shown yet.
    let mut t = ShapeTool::new("star");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 51.0, 50.0)), vec![]);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 90.0, 50.0).with_mods(ctrl));
    assert_eq!(a[0], Action::Begin("Star".into()));
    close(radii(&a), (40.0, 20.0));
    close(radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 130.0, 50.0).with_mods(ctrl))), (80.0, 20.0));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 130.0, 50.0).with_mods(ctrl)), vec![Action::Commit]);
}

#[test]
fn shift_square_alt_center() {
    let t = ShapeTool::new("ellipse");
    let (c, v) = t.command(Point::new(0.0, 0.0), Point::new(10.0, 4.0), Mods { shift: true, alt: true, ..Default::default() });
    assert_eq!(c, "shape.ellipse");
    assert_eq!(v, json!({"x": -10.0, "y": -10.0, "width": 20.0, "height": 20.0}));
}

/// A 500 × 500 document holding `paths`.
fn doc_holding_paths(paths: impl IntoIterator<Item = vectorcraft_geom::PathData>) -> vectorcraft_doc::Document {
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    for (i, path) in paths.into_iter().enumerate() {
        let id = d.alloc_id();
        d.insert(Some(l), i, vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art())).unwrap();
    }
    d
}

/// The box of the last preview of a drag: x, y, width, height.
fn previewed_box(a: &[Action]) -> [f64; 4] {
    let Some(Action::Preview(_, v)) = a.last() else { panic!("no preview: {a:?}") };
    ["x", "y", "width", "height"].map(|k| v[k].as_f64().unwrap_or(f64::NAN))
}

/// An Alt+Shift drag with the Ellipse tool from `center` to `to`: the circle's box as previewed,
/// and the overlays it shows.
fn circle_drag(c: &ToolContext, center: Point, to: Point) -> ([f64; 4], Vec<Overlay>) {
    let both = Mods { alt: true, shift: true, ..Mods::default() };
    let mut t = ShapeTool::new("ellipse");
    t.pointer(c, &PointerEvent::new(PointerKind::Down, center.x, center.y).with_mods(both));
    let a = t.pointer(c, &PointerEvent::new(PointerKind::Drag, to.x, to.y).with_mods(both));
    (previewed_box(&a), t.overlays(c))
}

fn tangent_label_at(o: &[Overlay], at: Point, within: f64) -> bool {
    o.iter().any(|o| matches!(o, Overlay::Label { text, p, .. } if text == "tangent" && p.distance(at) <= within))
}

/// Snap to Tangent: a circle drawn from its center takes the radius at which it touches a nearby
/// straight path, with the "tangent" label at the touch point, also when the touch point is the
/// middle of the segment; the size label gives the circle as drawn.
#[test]
fn a_circle_from_its_center_snaps_its_radius_to_touch_a_line() {
    // The line's ends and middle line up with none of the corners below.
    let d = doc_holding_paths([vectorcraft_geom::shapes::line(Point::new(60.0, 300.0), Point::new(380.0, 300.0))]);
    let (s, p) = (Selection::default(), paint());
    let c = ToolContext { measurement_labels: true, ..cx(&d, &s, &p) };
    // The pointer makes the radius 98; the line is 100 below the center.
    let (b, o) = circle_drag(&c, Point::new(200.0, 200.0), Point::new(203.0, 102.0));
    assert_eq!(b, [100.0, 100.0, 200.0, 200.0]);
    assert!(tangent_label_at(&o, Point::new(200.0, 300.0), 1e-9), "{o:?}");
    assert!(o.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &c.size_label(200.0, 200.0))), "{o:?}");
    // From the other side of the touching radius too (102).
    let (b, _) = circle_drag(&c, Point::new(200.0, 200.0), Point::new(205.0, 98.0));
    assert_eq!(b, [100.0, 100.0, 200.0, 200.0]);
    // Space moves the snapped circle and keeps its size.
    let both = Mods { alt: true, shift: true, ..Mods::default() };
    let mut t = ShapeTool::new("ellipse");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 200.0, 200.0).with_mods(both));
    t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 203.0, 102.0).with_mods(both));
    // Space pressed with the pointer still moves the circle by no more than the snap moved its
    // corner.
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 203.0, 102.0).with_mods(Mods { space: true, ..both }));
    let b = previewed_box(&a);
    assert!(b[2] == 200.0 && b[3] == 200.0 && (b[0] - 100.0).abs() <= 4.0 && (b[1] - 100.0).abs() <= 4.0, "{b:?}");
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 253.0, 152.0).with_mods(Mods { space: true, ..both }));
    let b = previewed_box(&a);
    assert!(b[2] == 200.0 && b[3] == 200.0 && (b[0], b[1]) != (100.0, 100.0), "moved, same size: {b:?}");
    assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &c.size_label(200.0, 200.0))));
    // Letting go of Space where the moved circle no longer touches the line keeps its size, and
    // moving on grows it from there.
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 253.0, 152.0).with_mods(both));
    assert_eq!(previewed_box(&a), b, "Space let go");
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 243.0, 142.0).with_mods(both));
    let (grown, moved) = (previewed_box(&a), Point::new(b[0] + 100.0, b[1] + 100.0));
    let radius = (243.0 - moved.x).abs().max((142.0 - moved.y).abs());
    assert!(grown[2] == 2.0 * radius && grown[3] == 2.0 * radius && radius > 105.0, "{grown:?} from {moved:?}");
    // Out of reach (radius 90): the pointer sizes it.
    let (b, o) = circle_drag(&c, Point::new(200.0, 200.0), Point::new(203.0, 110.0));
    assert_eq!(b, [110.0, 110.0, 180.0, 180.0]);
    assert!(!o.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "tangent")), "{o:?}");
}

/// A circle drawn around a center outside another circle touches it from outside at the nearer
/// side and takes it inside at the farther side, the radii those of the closed form (the distance
/// between the centers less or plus the other radius) within the error of the curves that draw a
/// circle. A center inside a circle gives the inscribed circle; its touch point is an anchor of
/// the circle, where two segments join smoothly. A center at the circle's own center gets no snap:
/// every point is square to it.
#[test]
fn a_circle_from_its_center_snaps_to_touch_another_circle_from_outside_and_inside() {
    use vectorcraft_geom::shapes::ellipse;
    // A circle of radius 50 around (350, 300), its center 50√13 from (200, 200).
    let d = doc_holding_paths([ellipse(Rect::new(300.0, 250.0, 400.0, 350.0))]);
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let (center, between) = (Point::new(200.0, 200.0), 50.0 * 13f64.sqrt());
    let toward = Vec2::new(150.0, 100.0) / between;
    for (radius, pointer) in [(between - 50.0, Vec2::new(20.0, -(between - 52.0))), (between + 50.0, Vec2::new(-20.0, between + 48.0))] {
        let (b, o) = circle_drag(&c, center, center + pointer);
        assert!((b[2] / 2.0 - radius).abs() < 0.02 && b[2] == b[3], "{b:?}, want radius {radius}");
        assert!(tangent_label_at(&o, center + toward * radius, 0.2), "{o:?}");
    }
    // Concentric: no tangent snap.
    let (_, o) = circle_drag(&c, Point::new(350.0, 300.0), Point::new(374.0, 252.0));
    assert!(!o.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "tangent")), "{o:?}");
    // Ellipses with radii 10010 and 10000, or 1001 and 1000, around the center are nearly square
    // to it all round, but they are not circles: a circle touches each at the ends of its axes.
    for (rx, ry) in [(10_010.0, 10_000.0), (1001.0, 1000.0)] {
        let d = doc_holding_paths([ellipse(Rect::new(250.0 - rx, 250.0 - ry, 250.0 + rx, 250.0 + ry))]);
        let c = cx(&d, &s, &p);
        let (_, o) = circle_drag(&c, Point::new(250.0, 250.0), Point::new(280.0, 252.0 - ry));
        assert!(tangent_label_at(&o, Point::new(250.0, 250.0 - ry), 1e-6), "{rx} × {ry}: {o:?}");
    }
    let near = |b: [f64; 4], want: [f64; 4]| b.iter().zip(want).all(|(a, w)| (a - w).abs() < 1e-6);
    // Inside a circle of radius 150 around (250, 250), 50 from its center: the inscribed circle has
    // radius 100.
    let d = doc_holding_paths([ellipse(Rect::new(100.0, 100.0, 400.0, 400.0))]);
    let c = cx(&d, &s, &p);
    let (b, o) = circle_drag(&c, Point::new(200.0, 250.0), Point::new(203.0, 152.0));
    assert!(near(b, [100.0, 150.0, 200.0, 200.0]), "{b:?}");
    assert!(tangent_label_at(&o, Point::new(100.0, 250.0), 1e-6), "{o:?}");
}

/// The touch point can fall inside a curved segment: the radius is the distance to the nearest
/// point of the curve, measured here by sampling the path's own segments densely.
#[test]
fn a_circle_from_its_center_snaps_to_touch_a_curve() {
    use vectorcraft_geom::ParamCurve;
    let path = vectorcraft_geom::shapes::ellipse(Rect::new(300.0, 100.0, 500.0, 200.0));
    let center = Point::new(250.0, 300.0);
    let sp = &path.subpaths[0];
    let (want, at) = (0..sp.segment_count())
        .flat_map(|i| (0..=20_000).map(move |k| (i, f64::from(k) / 20_000.0)))
        .map(|(i, t)| sp.segment(i).eval(t))
        .map(|q| (q.distance(center), q))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap();
    let d = doc_holding_paths([path]);
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let r = want - 2.0;
    let (b, o) = circle_drag(&c, center, center + Vec2::new(-r * 0.3, r));
    assert!((b[2] / 2.0 - want).abs() < 1e-4 && (b[3] / 2.0 - want).abs() < 1e-4, "radius {} want {want}", b[2] / 2.0);
    assert!(tangent_label_at(&o, at, 1e-2), "{at:?} {o:?}");
    // Straight out from a symmetric curve's middle (t = 0.5, (200, 262.5)), 162.5 from (200, 100).
    let mut bump = vectorcraft_geom::BezPath::new();
    bump.move_to((100.0, 300.0));
    bump.curve_to((150.0, 250.0), (250.0, 250.0), (300.0, 300.0));
    let d = doc_holding_paths([vectorcraft_geom::PathData::from_bezpath(&bump)]);
    let c = cx(&d, &s, &p);
    let (b, o) = circle_drag(&c, Point::new(200.0, 100.0), Point::new(40.0, 40.0));
    assert!((b[2] - 325.0).abs() < 1e-9 && (b[3] - 325.0).abs() < 1e-9, "{b:?}");
    assert!(tangent_label_at(&o, Point::new(200.0, 262.5), 1e-9), "{o:?}");
}

/// The snap is Snap to Tangent's, under Smart Guides: off, the pointer sizes the circle. An
/// ellipse (no Shift) and a box drawn from a corner (no Alt) do not snap. An open path's end and
/// a corner square to the center are where a circle meets the path rather than touches it, so
/// neither is offered.
#[test]
fn a_circle_from_its_center_keeps_its_radius_without_the_snap_or_a_smooth_touch() {
    let line = vectorcraft_geom::shapes::line(Point::new(60.0, 300.0), Point::new(380.0, 300.0));
    let d = doc_holding_paths([line]);
    let (s, p) = (Selection::default(), paint());
    // The pointer makes the radius 98, 2 short of touching the line.
    let (center, to) = (Point::new(200.0, 200.0), Point::new(215.0, 102.0));
    for off in [ToolContext { snap_tangent: false, ..cx(&d, &s, &p) }, ToolContext { smart_guides: false, ..cx(&d, &s, &p) }] {
        assert_eq!(circle_drag(&off, center, to).0, [102.0, 102.0, 196.0, 196.0]);
    }
    let c = cx(&d, &s, &p);
    assert_eq!(circle_drag(&c, center, to).0, [100.0, 100.0, 200.0, 200.0], "on, it snaps");
    for (mods, want) in
        [(Mods { alt: true, ..Mods::default() }, [185.0, 102.0, 30.0, 196.0]), (Mods { shift: true, ..Mods::default() }, [200.0, 102.0, 98.0, 98.0])]
    {
        let mut t = ShapeTool::new("ellipse");
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, center.x, center.y).with_mods(mods));
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, to.x, to.y).with_mods(mods));
        assert_eq!(previewed_box(&a), want, "{mods:?}");
    }
    // The line's left end is square below (60, 200), but it is an open end, also with a second
    // line starting there the same way, which overlaps it rather than carrying it on.
    assert_eq!(circle_drag(&c, Point::new(60.0, 200.0), Point::new(63.0, 102.0)).0, [-38.0, 102.0, 196.0, 196.0]);
    let line = |a: (f64, f64), b: (f64, f64)| vectorcraft_geom::shapes::line(Point::new(a.0, a.1), Point::new(b.0, b.1));
    let d = doc_holding_paths([line((60.0, 300.0), (380.0, 300.0)), line((60.0, 300.0), (200.0, 300.0))]);
    let c = cx(&d, &s, &p);
    assert_eq!(circle_drag(&c, Point::new(60.0, 200.0), Point::new(63.0, 102.0)).0, [-38.0, 102.0, 196.0, 196.0]);
    // Two lines end to end in one direction meet smoothly under (200, 200): the circle touches there.
    let d = doc_holding_paths([line((60.0, 300.0), (200.0, 300.0)), line((200.0, 300.0), (380.0, 300.0))]);
    let c = cx(&d, &s, &p);
    assert_eq!(circle_drag(&c, center, to).0, [100.0, 100.0, 200.0, 200.0]);
    // A rectangle's corner (200, 200) is 100√2 from (300, 300), diagonal to both its sides.
    let (d, _) = doc_with_rect();
    let c = cx(&d, &s, &p);
    let r = 100.0 * 2f64.sqrt() - 2.0;
    let (b, _) = circle_drag(&c, Point::new(300.0, 300.0), Point::new(300.0 + r, 300.0 + r * 0.5));
    assert!((b[2] - 2.0 * r).abs() < 1e-9, "{b:?}");
}

/// An anchor the dragged corner snapped to wins over the tangent radius, as it does for a drawn
/// point. With more segments near the radius than one move measures, the ones nearest the pointer
/// are measured.
#[test]
fn a_circle_from_its_center_prefers_an_anchor_and_measures_the_segments_nearest_the_pointer() {
    let line = |a: (f64, f64), b: (f64, f64)| vectorcraft_geom::shapes::line(Point::new(a.0, a.1), Point::new(b.0, b.1));
    let (s, p) = (Selection::default(), paint());
    let center = Point::new(200.0, 200.0);
    // Shift keeps the corner on the diagonal, and it slides onto the anchor (297, 103): radius 97.
    let d = doc_holding_paths([line((60.0, 300.0), (380.0, 300.0)), line((297.0, 103.0), (297.0, 40.0))]);
    let c = cx(&d, &s, &p);
    let (b, o) = circle_drag(&c, center, Point::new(203.0, 102.0));
    assert_eq!(b, [103.0, 103.0, 194.0, 194.0]);
    assert!(!o.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "tangent")), "{o:?}");
    // A corner that only lines up with an anchor near it, (300, 104), still takes the tangent.
    let d = doc_holding_paths([line((60.0, 300.0), (380.0, 300.0)), line((300.0, 104.0), (300.0, 40.0))]);
    let c = cx(&d, &s, &p);
    let (b, o) = circle_drag(&c, center, Point::new(203.0, 102.0));
    assert_eq!(b, [100.0, 100.0, 200.0, 200.0]);
    assert!(tangent_label_at(&o, Point::new(200.0, 300.0), 1e-9), "{o:?}");
    // 300 short segments pointing at the center cross the ring above it, ahead of the line below.
    let spokes = (0..300).map(|i| {
        let a = (-55.0 + 110.0 * f64::from(i) / 299.0_f64).to_radians();
        let at = |r: f64| (200.0 + r * a.sin(), 200.0 - r * a.cos());
        line(at(95.0), at(105.0))
    });
    let d = doc_holding_paths(spokes.chain([line((60.0, 300.0), (380.0, 300.0))]));
    let c = cx(&d, &s, &p);
    // The press lines up with a spoke's anchor, so the center moves by less than a point.
    let (b, o) = circle_drag(&c, center, Point::new(215.0, 298.0));
    assert!((b[2] - 200.0).abs() < 1e-9 && (b[3] - 200.0).abs() < 1e-9 && (b[1] - 100.0).abs() < 1e-9, "{b:?}");
    assert!(o.iter().any(|o| matches!(o, Overlay::Label { text, p, .. } if text == "tangent" && (p.y - 300.0).abs() < 1e-9)), "{o:?}");
}

/// With a Constrain Angle the radius is measured, and the snapped box rebuilt, along the turned
/// axes: the circle still touches the line.
#[test]
fn a_circle_snapped_to_touch_keeps_to_the_constrain_angle() {
    let d = doc_holding_paths([vectorcraft_geom::shapes::line(Point::new(0.0, 300.0), Point::new(450.0, 300.0))]);
    let (s, p) = (Selection::default(), paint());
    let c = ToolContext { constrain_angle: 30.0, ..cx(&d, &s, &p) };
    let axes = vectorcraft_geom::constrain_axes(30.0).unwrap();
    let center = Point::new(200.0, 200.0);
    let both = Mods { alt: true, shift: true, ..Mods::default() };
    let mut t = ShapeTool::new("ellipse");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, center.x, center.y).with_mods(both));
    let to = center + (axes * Point::new(-40.0, -97.0)).to_vec2();
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, to.x, to.y).with_mods(both));
    let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
    let (w, h) = (v["width"].as_f64().unwrap(), v["height"].as_f64().unwrap());
    assert!((w - 200.0).abs() < 1e-9 && (h - 200.0).abs() < 1e-9, "{v}");
    assert!((v["rotation"].as_f64().unwrap() + 30.0).abs() < 1e-9, "{v}");
    let mid = Point::new(v["x"].as_f64().unwrap() + w / 2.0, v["y"].as_f64().unwrap() + h / 2.0);
    assert!(mid.distance(center) < 1e-9, "centered on the press: {mid:?}");
    // Space pressed with the pointer still keeps the size and moves the circle by no more than
    // the snap moved its corner.
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, to.x, to.y).with_mods(Mods { space: true, ..both }));
    let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
    let (w, h) = (v["width"].as_f64().unwrap(), v["height"].as_f64().unwrap());
    assert!((w - 200.0).abs() < 1e-9 && (h - 200.0).abs() < 1e-9, "{v}");
    let mid = Point::new(v["x"].as_f64().unwrap() + w / 2.0, v["y"].as_f64().unwrap() + h / 2.0);
    assert!(mid.distance(center) <= 4.0, "{mid:?}");
}

/// Snap to Grid and pixel snapping place the drawn point instead of the tangent snap, also when
/// one is turned on while the circle is dragged out. Snap to Tangent turned off while dragging
/// stops the snap.
#[test]
fn a_circle_from_its_center_does_not_snap_to_touch_with_the_grid_or_pixels() {
    let d = doc_holding_paths([vectorcraft_geom::shapes::line(Point::new(60.0, 300.0), Point::new(380.0, 300.0))]);
    let (s, p) = (Selection::default(), paint());
    let on = cx(&d, &s, &p);
    let grid = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
    let pixels = ToolContext { snap_to_pixel: true, pixel_snap: crate::PixelSnap { drawing: true, ..on.pixel_snap }, ..cx(&d, &s, &p) };
    let no_tangent = ToolContext { snap_tangent: false, ..cx(&d, &s, &p) };
    let both = Mods { alt: true, shift: true, ..Mods::default() };
    let drag = |press: &ToolContext, moved: &ToolContext, to: Point| {
        let mut t = ShapeTool::new("ellipse");
        t.pointer(press, &PointerEvent::new(PointerKind::Down, 200.0, 200.0).with_mods(both));
        let a = t.pointer(moved, &PointerEvent::new(PointerKind::Drag, to.x, to.y).with_mods(both));
        (previewed_box(&a), t.overlays(moved))
    };
    let tangent = |o: &[Overlay]| o.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "tangent"));
    // The pointer makes the radius 98: the snap would make it 100.
    let to = Point::new(203.0, 102.0);
    assert_eq!(drag(&on, &on, to).0, [100.0, 100.0, 200.0, 200.0], "the snap applies");
    // Pixels round (203.2, 102.2) to (203, 102): radius 98.
    let to = Point::new(203.2, 102.2);
    for (press, moved) in [(&pixels, &pixels), (&on, &pixels)] {
        let (b, o) = drag(press, moved, to);
        assert_eq!(b, [102.0, 102.0, 196.0, 196.0]);
        assert!(!tangent(&o), "{o:?}");
    }
    // The grid (every 9 points) puts the corner on (207, 99): radius 101 from (200, 200), or 99
    // from (198, 198), where the grid puts a press made with it on.
    for (press, moved, want) in [(&grid, &grid, [99.0, 99.0, 198.0, 198.0]), (&on, &grid, [99.0, 99.0, 202.0, 202.0])] {
        let (b, o) = drag(press, moved, to);
        assert_eq!(b, want);
        assert!(!tangent(&o), "{o:?}");
    }
    let (b, o) = drag(&on, &no_tangent, Point::new(203.0, 102.0));
    assert_eq!(b, [102.0, 102.0, 196.0, 196.0]);
    assert!(!tangent(&o), "{o:?}");
}

/// A press and a small jitter is a click, not a drag, even when the tangent snap would size the
/// circle at the jittered pointer large enough to begin a drag.
#[test]
fn a_jittered_alt_shift_press_beside_a_path_is_a_click() {
    // With a Constrain Angle, Shift does not hold the corner on the page's diagonal, which would
    // take it onto the line.
    let d = doc_holding_paths([vectorcraft_geom::shapes::line(Point::new(60.0, 300.0), Point::new(380.0, 300.0))]);
    let (s, p) = (Selection::default(), paint());
    let c = ToolContext { constrain_angle: 30.0, ..cx(&d, &s, &p) };
    let both = Mods { alt: true, shift: true, ..Mods::default() };
    let mut t = ShapeTool::new("ellipse");
    // 5.5 above the line, beyond the snapping tolerance of 4. The jitter of 1.9 makes the radius
    // 1.9 cos 30°, about 1.65, within 4 of the touching radius 5.5.
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 200.0, 294.5).with_mods(both));
    assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 201.9, 294.5).with_mods(both)), vec![]);
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Up, 201.9, 294.5).with_mods(both));
    assert!(!a.contains(&Action::Commit), "{a:?}");
}

/// An anchor or a crossing within the snapping tolerance of the pointer takes the point, and the
/// circle is not sized to touch the line.
#[test]
fn a_circle_from_its_center_prefers_an_anchor_or_a_crossing_at_the_pointer() {
    let line = |a: (f64, f64), b: (f64, f64)| vectorcraft_geom::shapes::line(Point::new(a.0, a.1), Point::new(b.0, b.1));
    let (s, p) = (Selection::default(), paint());
    let center = Point::new(200.0, 200.0);
    let base = line((60.0, 300.0), (380.0, 300.0));
    // An anchor at (205, 104), and two lines crossing there.
    for extra in [vec![line((205.0, 104.0), (260.0, 40.0))], vec![line((185.0, 84.0), (225.0, 124.0)), line((185.0, 124.0), (225.0, 84.0))]] {
        let d = doc_holding_paths(std::iter::once(base.clone()).chain(extra));
        let c = cx(&d, &s, &p);
        let (b, o) = circle_drag(&c, center, Point::new(203.0, 102.0));
        assert!(b[2] != 200.0, "{b:?}");
        assert!(!o.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "tangent")), "{o:?}");
    }
}

/// A path whose anchor below the center joins two straight curves with their handles drawn back
/// onto that anchor runs smoothly through it: the circle touches it there.
#[test]
fn a_circle_from_its_center_touches_a_path_at_an_anchor_with_retracted_handles() {
    let mut run = vectorcraft_geom::BezPath::new();
    run.move_to((60.0, 300.0));
    run.curve_to((100.0, 300.0), (200.0, 300.0), (200.0, 300.0));
    run.curve_to((200.0, 300.0), (300.0, 300.0), (380.0, 300.0));
    let d = doc_holding_paths([vectorcraft_geom::PathData::from_bezpath(&run)]);
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let (b, o) = circle_drag(&c, Point::new(200.0, 200.0), Point::new(215.0, 102.0));
    assert_eq!(b, [100.0, 100.0, 200.0, 200.0]);
    assert!(tangent_label_at(&o, Point::new(200.0, 300.0), 1e-9), "{o:?}");
}

/// General › Constrain Angle: a Shift-drag squares along the turned axes, and the shape turns
/// by the angle about its centre.
#[test]
fn constrain_angle_squares_along_the_turned_axes() {
    let t = ShapeTool::new("rectangle");
    let shift = Mods { shift: true, ..Default::default() };
    // 45°: the drag straight right is a diagonal of the turned axes, so a square on it.
    let (_, v) = t.command_at(Point::new(0.0, 0.0), Point::new(20.0, 0.0), shift, 45.0);
    let side = 20.0 / 2f64.sqrt();
    let near = |k: &str, want: f64| (v[k].as_f64().unwrap() - want).abs() < 1e-9;
    assert!(near("width", side) && near("height", side) && near("rotation", -45.0), "{v}");
    assert!(near("x", 10.0 - side / 2.0) && near("y", -side / 2.0), "centred on the drag's middle: {v}");
    // Whole turns are the page's axes exactly: the same box as 0°, unturned.
    let (a, b) = (Point::new(3.0, 4.0), Point::new(41.0, 17.0));
    let flat = t.command_at(a, b, Mods::default(), 0.0);
    for turn in [360.0, -360.0, 720.0] {
        assert_eq!(t.command_at(a, b, Mods::default(), turn), flat, "{turn}°");
    }
    // A quarter turn is exact (no floating-point noise in the box).
    let (_, v) = t.command_at(Point::new(0.0, 0.0), Point::new(20.0, 10.0), Mods::default(), 90.0);
    assert_eq!((v["width"].as_f64(), v["height"].as_f64(), v["rotation"].as_f64()), (Some(10.0), Some(20.0), Some(-90.0)), "{v}");
    let (_, w) = t.command_at(Point::new(0.0, 0.0), Point::new(20.0, 10.0), Mods::default(), 450.0);
    assert_eq!(v, w, "450° is 90°");
}

/// The size label gives the box as drawn: measured along the Constrain Angle's axes.
#[test]
fn constrain_angle_size_label_measures_the_turned_box() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.constrain_angle = 45.0;
    let mut t = ShapeTool::new("rectangle");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 340.0, 300.0));
    let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
    let (w, h) = (v["width"].as_f64().unwrap(), v["height"].as_f64().unwrap());
    assert!((w - h).abs() < 1e-9 && (w - 40.0 / 2f64.sqrt()).abs() < 1e-9, "{v}");
    let o = t.overlays(&cx);
    assert!(o.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &cx.size_label(w, h))), "{o:?}");
    assert!(!o.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &cx.size_label(40.0, 0.0))), "not the page box");
}

#[test]
fn click_opens_dialog() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeTool::new("star");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 10.0, 10.0)), vec![Action::Dialog("star".into(), json!({"x": 10.0, "y": 10.0}))]);
}

/// With the Ellipse tool, the first two Shift-clicks place points (shown as marks, and with two
/// placed, the circle through them and the pointer is previewed); the third runs the circle
/// command through the three. A Shift-click with another shape tool still opens its dialog.
#[test]
fn three_shift_clicks_with_the_ellipse_tool_run_the_circle_through_them() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = ShapeTool::new("ellipse");
    let click = |t: &mut ShapeTool, x: f64, y: f64| {
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, x, y).with_mods(shift));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, x, y).with_mods(shift))
    };
    assert_eq!(click(&mut t, 10.0, 10.0), vec![]);
    assert_eq!(click(&mut t, 90.0, 10.0), vec![]);
    assert!(t.ends_on_escape(&cx) && t.claims_key(&cx, ToolKey::Escape));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 10.0, 70.0));
    let o = t.overlays(&cx);
    assert_eq!(o.iter().filter(|o| matches!(o, Overlay::Anchor { .. })).count(), 2, "{o:?}");
    let preview = o.iter().find_map(|o| match o {
        Overlay::Path { path, dashed: true, .. } => Some(path.clone()),
        _ => None,
    });
    let b = vectorcraft_geom::Shape::bounding_box(&preview.expect("the circle is previewed"));
    assert!(b.x0.abs() < 1e-9 && (b.y0 + 10.0).abs() < 1e-9 && (b.x1 - 100.0).abs() < 1e-9 && (b.y1 - 90.0).abs() < 1e-9, "{b:?}");
    assert_eq!(
        click(&mut t, 10.0, 70.0),
        vec![Action::Exec("shape.circleThroughPoints".into(), json!({"points": [[10.0, 10.0], [90.0, 10.0], [10.0, 70.0]]}))]
    );
    assert!(!t.ends_on_escape(&cx) && t.overlays(&cx).is_empty());
    // Another shape tool keeps its dialog on a Shift-click.
    let mut r = ShapeTool::new("rectangle");
    assert_eq!(click(&mut r, 10.0, 10.0), vec![Action::Dialog("rectangle".into(), json!({"x": 10.0, "y": 10.0}))]);
}

/// With Smart Guides on, the preview passes through the point the pointer snaps to, which is where
/// the third Shift-click lands.
#[test]
fn the_circle_preview_passes_through_the_snapped_pointer() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = ShapeTool::new("ellipse");
    for (x, y) in [(300.0, 0.0), (300.0, 200.0)] {
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, x, y).with_mods(shift));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, x, y).with_mods(shift));
    }
    // Near the rectangle's corner anchor at (100, 100): the circle through (300, 0), (300, 200)
    // and (100, 100) has its center at (225, 100) and radius 125.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 98.0, 102.0));
    let preview = t.overlays(&cx).into_iter().find_map(|o| match o {
        Overlay::Path { path, dashed: true, .. } => Some(path),
        _ => None,
    });
    let b = vectorcraft_geom::Shape::bounding_box(&preview.expect("the circle is previewed"));
    let want = [100.0, -25.0, 350.0, 225.0];
    assert!([b.x0, b.y0, b.x1, b.y1].iter().zip(want).all(|(a, w)| (a - w).abs() < 1e-9), "{b:?}");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 98.0, 102.0).with_mods(shift));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 98.0, 102.0).with_mods(shift));
    assert!(matches!(&a[..], [Action::Exec(_, v)] if v["points"][2] == json!([100.0, 100.0])), "{a:?}");
}

/// Escape, a plain click and a drag drop the points Shift-clicks placed, so the next Shift-click
/// starts a new circle.
#[test]
fn escape_a_click_or_a_drag_drops_the_shift_clicked_points() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = ShapeTool::new("ellipse");
    let shift_click = |t: &mut ShapeTool, x: f64, y: f64| {
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, x, y).with_mods(shift));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, x, y).with_mods(shift))
    };
    shift_click(&mut t, 10.0, 10.0);
    shift_click(&mut t, 90.0, 10.0);
    assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![]);
    assert!(!t.ends_on_escape(&cx));
    shift_click(&mut t, 10.0, 10.0);
    shift_click(&mut t, 90.0, 10.0);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
    assert_eq!(
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 50.0, 50.0)),
        vec![Action::Dialog("ellipse".into(), json!({"x": 50.0, "y": 50.0}))]
    );
    assert!(!t.ends_on_escape(&cx), "a plain click drops them");
    shift_click(&mut t, 10.0, 10.0);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0).with_mods(shift));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 90.0, 90.0).with_mods(shift));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 90.0, 90.0).with_mods(shift)), vec![Action::Commit]);
    assert!(!t.ends_on_escape(&cx), "a Shift-drag draws a circle and drops them");
    // A move of the selection by its centre point drops them too.
    let (d, s) = unfilled_live_rect();
    let c = ToolContext { smart_guides: false, ..crate::testutil::cx(&d, &s, &p) };
    let mut t = ShapeTool::new("ellipse");
    for (x, y) in [(10.0, 10.0), (90.0, 10.0)] {
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, x, y).with_mods(shift));
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, x, y).with_mods(shift));
    }
    assert!(t.ends_on_escape(&c));
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
    t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 170.0, 160.0));
    assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 170.0, 160.0)), vec![Action::Commit]);
    assert!(!t.ends_on_escape(&c), "a move of the selection drops them");
}

/// Escape pressed while the third Shift-click is held drops the points and the press, so the
/// release draws no circle.
#[test]
fn escape_during_the_third_shift_click_drops_the_points_and_the_press() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = ShapeTool::new("ellipse");
    for (x, y) in [(10.0, 10.0), (90.0, 10.0)] {
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, x, y).with_mods(shift));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, x, y).with_mods(shift));
    }
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 70.0).with_mods(shift));
    assert!(t.claims_key(&cx, ToolKey::Escape));
    assert_eq!(t.key(&cx, ToolKey::Escape, shift), vec![]);
    assert!(!t.ends_on_escape(&cx) && t.overlays(&cx).is_empty());
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 10.0, 70.0).with_mods(shift)), vec![]);
    assert!(!t.ends_on_escape(&cx), "the release places no point");
}

/// Once a point is placed, a Shift-click on the selection's side handle or centre point places a
/// point instead of scaling or moving the selection, and the cursor over them is the crosshair.
#[test]
fn shift_clicks_on_the_selection_place_points_once_one_is_placed() {
    let (d, s) = unfilled_live_rect();
    let p = paint();
    let c = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = ShapeTool::new("ellipse");
    let centre = Point::new(150.0, 150.0);
    assert_eq!(t.cursor(&c, centre, shift), Cursor::Move, "no point placed yet: the centre point moves");
    let click = |t: &mut ShapeTool, x: f64, y: f64| {
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, x, y).with_mods(shift));
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, x, y).with_mods(shift))
    };
    assert_eq!(click(&mut t, 10.0, 10.0), vec![]);
    assert_eq!(t.cursor(&c, centre, shift), Cursor::Crosshair);
    assert_eq!(t.cursor(&c, centre, Mods::default()), Cursor::Move, "without Shift a press still moves the selection");
    assert_eq!(click(&mut t, 100.0, 150.0), vec![], "the left side handle");
    assert!(!t.busy());
    assert_eq!(
        click(&mut t, centre.x, centre.y),
        vec![Action::Exec("shape.circleThroughPoints".into(), json!({"points": [[10.0, 10.0], [100.0, 150.0], [150.0, 150.0]]}))]
    );
}

/// The pointer leaving the canvas removes the circle's preview, and an edit, Undo or Redo made
/// outside the tool drops the points placed so far.
#[test]
fn leaving_the_canvas_or_an_outside_edit_clears_the_circle_in_progress() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = ShapeTool::new("ellipse");
    let click = |t: &mut ShapeTool, x: f64, y: f64| {
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, x, y).with_mods(shift));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, x, y).with_mods(shift))
    };
    let dashed = |t: &ShapeTool| t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Path { dashed: true, .. }));
    click(&mut t, 10.0, 10.0);
    click(&mut t, 90.0, 10.0);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 10.0, 70.0));
    assert!(dashed(&t));
    t.hover_ended(&cx);
    assert!(!dashed(&t), "no preview through a pointer off the canvas");
    assert!(t.ends_on_escape(&cx), "the points stay");
    t.history_moved();
    assert!(!t.ends_on_escape(&cx) && t.overlays(&cx).is_empty());
    assert_eq!(click(&mut t, 10.0, 70.0), vec![], "the next Shift-click starts a new circle");
}

/// While the perspective grid shows with an active plane, a Shift-click with the Ellipse tool opens
/// the size dialog, whose shape goes on the plane, as before.
#[test]
fn a_shift_click_on_an_active_perspective_plane_opens_the_dialog() {
    let (mut d, _) = doc_with_rect();
    crate::distort::perspective::PerspectiveGrid::preset(2, Rect::new(0.0, 0.0, 612.0, 792.0)).store(&mut d);
    let s = Selection::default();
    let p = paint();
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let shift = Mods { shift: true, ..Default::default() };
    let mut t = ShapeTool::new("ellipse");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0).with_mods(shift));
    assert_eq!(
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 10.0, 10.0).with_mods(shift)),
        vec![Action::Dialog("ellipse".into(), json!({"x": 10.0, "y": 10.0}))]
    );
    assert!(!t.ends_on_escape(&cx));
}

#[test]
fn alt_click_passes_draw_from_centre_to_the_dialog() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Default::default() };
    let mut t = ShapeTool::new("rectangle");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0).with_mods(alt));
    assert_eq!(
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 10.0, 10.0).with_mods(alt)),
        vec![Action::Dialog("rectangle".into(), json!({"x": 10.0, "y": 10.0, "alt": true}))]
    );
}

#[test]
fn arrow_keys_change_sides() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeTool::new("polygon");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 50.0, 10.0));
    let a = t.key(&cx, ToolKey::Up, Mods::default());
    assert!(matches!(&a[0], Action::Preview(_, v) if v["sides"] == 7));
}

/// The Rounded Rectangle radius stops at General › Corner Radius's range on both of its input
/// paths: a Tool Options value is clamped to it, and ↑ at the top stays there.
#[test]
fn rounded_rectangle_radius_is_clamped_to_its_range() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeTool::new("roundedRectangle");
    t.set_option("cornerRadius", &json!(5000));
    assert_eq!(t.options()["cornerRadius"], json!(MAX_CORNER_RADIUS));
    t.set_option("cornerRadius", &json!(f64::INFINITY));
    assert_eq!(t.options()["cornerRadius"], json!(12.0), "a non-finite value is the default");
    t.set_option("cornerRadius", &json!(-3));
    assert_eq!(t.options()["cornerRadius"], json!(0.0));
    t.set_option("cornerRadius", &json!(MAX_CORNER_RADIUS));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 50.0, 40.0));
    let a = t.key(&cx, ToolKey::Up, Mods::default());
    assert!(matches!(&a[0], Action::Preview(_, v) if v["radius"] == json!(MAX_CORNER_RADIUS)), "{a:?}");
    assert_eq!(t.options()["cornerRadius"], json!(MAX_CORNER_RADIUS));
}

/// Escape during a polygon, star or rounded rectangle drag puts back the side count, point count
/// or corner radius its arrows changed, for the next drag and the tool's options; a drag that is
/// let go keeps them.
#[test]
fn escape_puts_back_the_options_a_drag_changed() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    for (id, field) in [("polygon", "sides"), ("star", "points"), ("roundedRectangle", "radius")] {
        let mut t = ShapeTool::new(id);
        let value = |a: Vec<Action>| match a.last() {
            Some(Action::Preview(_, v)) => v[field].clone(),
            _ => panic!("{id}: {a:?}"),
        };
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        let before = value(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 60.0, 40.0)));
        let options = t.options();
        t.key(&cx, ToolKey::Up, Mods::default());
        let changed = value(t.key(&cx, ToolKey::Up, Mods::default()));
        assert_ne!(changed, before, "{id}: the arrows change the option");
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel], "{id}");
        assert_eq!(t.options(), options, "{id}: the tool's options");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        assert_eq!(value(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 60.0, 40.0))), before, "{id}: the next drag");
        // A drag that is let go keeps what its arrows changed.
        t.key(&cx, ToolKey::Up, Mods::default());
        t.key(&cx, ToolKey::Up, Mods::default());
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 60.0, 40.0)), vec![Action::Commit]);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        assert_eq!(value(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 60.0, 40.0))), changed, "{id}: a finished drag keeps it");
        t.key(&cx, ToolKey::Down, Mods::default());
        t.key(&cx, ToolKey::Escape, Mods::default());
        let kept = t.options();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        // Escape puts back only what this drag changed.
        assert_eq!(value(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 60.0, 40.0))), changed, "{id}: only this drag");
        assert_ne!(kept, options, "{id}");
    }
}

/// Ctrl/Cmd changes a star's inner-to-outer ratio as it is dragged out; Escape puts it back, and
/// an option set during the drag stays.
#[test]
fn escape_puts_back_the_star_ratio_and_keeps_an_option_set_during_the_drag() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.smart_guides = false;
    let ctrl = Mods { cmd: true, ..Default::default() };
    let mut t = ShapeTool::new("star");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 90.0, 50.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 130.0, 50.0).with_mods(ctrl));
    assert!((t.star_ratio - 0.25).abs() < 1e-9, "{}", t.star_ratio);
    t.key(&cx, ToolKey::Escape, ctrl);
    assert_eq!(t.star_ratio, 0.5);

    // Options set with `tool.setOption` during the drag stay; the arrows' change goes back.
    for (id, key, set) in [("polygon", "sides", json!(9)), ("star", "points", json!(8)), ("roundedRectangle", "cornerRadius", json!(30.0))] {
        let mut t = ShapeTool::new(id);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 60.0, 40.0));
        t.key(&cx, ToolKey::Up, Mods::default());
        t.set_option(key, &set);
        t.set_option("starRatio", &json!(0.3));
        t.key(&cx, ToolKey::Up, Mods::default());
        t.key(&cx, ToolKey::Escape, Mods::default());
        let o = t.options();
        assert_eq!((o[key].clone(), o["starRatio"].clone()), (set, json!(0.3)), "{id}");
    }
}

/// The start and the dragged corner land on another object's anchors (#506), hovering shows
/// it before the press, and the measurement label gives the size drawn.
#[test]
fn corners_snap_to_smart_guides() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeTool::new("rectangle");
    let anchor = |o: &[Overlay], at: Point| o.iter().any(|o| matches!(o, Overlay::Label { text, p, .. } if text == "anchor" && *p == at));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 98.0, 102.0));
    assert!(anchor(&t.overlays(&cx), Point::new(100.0, 100.0)));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 98.0, 102.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 198.0, 202.0));
    assert_eq!(a[1], Action::Preview("shape.rectangle".into(), json!({"x": 100.0, "y": 100.0, "width": 100.0, "height": 100.0})));
    let o = t.overlays(&cx);
    assert!(anchor(&o, Point::new(200.0, 200.0)), "{o:?}");
    assert!(o.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &cx.size_label(100.0, 100.0))));
    // Alt draws from the centre: the label gives the whole size.
    let alt = Mods { alt: true, ..Mods::default() };
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 198.0, 202.0).with_mods(alt));
    assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &cx.size_label(200.0, 200.0))));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 198.0, 202.0));
    assert!(t.overlays(&cx).is_empty());
    // A line with Shift keeps to 45° steps, sliding into line with the rect's right side.
    let mut t = ShapeTool::new("lineSegment");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
    let shift = Mods { shift: true, ..Mods::default() };
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 203.0, 302.0).with_mods(shift));
    assert!(matches!(&a[..], [_, Action::Preview(_, v)] if v["x2"] == 200.0 && (v["y2"].as_f64().unwrap() - 300.0).abs() < 1e-9), "{a:?}");
    let from = Point::new(300.0, 300.0);
    let to = Point::new(200.0, 300.0);
    assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &cx.segment_measure_label(from, to))));
}

/// Line Segment Tool Options: Length and Angle are options (for a click's dialog), and Fill
/// Line makes the drag's line take the current fill.
#[test]
fn line_segment_options_and_fill_line() {
    let mut t = ShapeTool::new("lineSegment");
    let (a, b) = (Point::new(0.0, 0.0), Point::new(30.0, 40.0));
    assert!(t.command(a, b, Mods::default()).1.get("fill").is_none());
    for (k, v) in [("length", json!(-80.0)), ("angle", json!(-90.0)), ("fillLine", json!(true))] {
        t.set_option(k, &v);
    }
    t.set_option("length", &json!(f64::NAN));
    assert_eq!(
        (t.options()["length"].clone(), t.options()["angle"].clone(), t.options()["fillLine"].clone()),
        (json!(80.0), json!(270.0), json!(true))
    );
    assert_eq!(t.command(a, b, Mods::default()).1["fill"], json!(true));
}

/// A document holding the closed paths `paths`, unselected.
fn doc_with_paths(paths: Vec<vectorcraft_geom::PathData>) -> vectorcraft_doc::Document {
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    for (k, path) in paths.into_iter().enumerate() {
        let id = d.alloc_id();
        d.insert(Some(l), k, vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art())).unwrap();
    }
    d
}

/// The circle of radius `r` about `c`, as the Ellipse tool draws it.
fn circle(c: Point, r: f64) -> vectorcraft_geom::PathData {
    vectorcraft_geom::shapes::ellipse(Rect::new(c.x - r, c.y - r, c.x + r, c.y + r))
}

/// The ends of the line a Line Segment drag from `press` through `drag` to `to` (with `mods`)
/// previews last, and the drag's actions.
fn line_drag(c: &ToolContext, press: Point, drag: Point, to: Point, mods: Mods) -> ((Point, Point), Vec<Action>) {
    let mut t = ShapeTool::new("lineSegment");
    t.pointer(c, &PointerEvent::new(PointerKind::Down, press.x, press.y).with_mods(mods));
    let mut all = t.pointer(c, &PointerEvent::new(PointerKind::Drag, drag.x, drag.y).with_mods(mods));
    all.extend(t.pointer(c, &PointerEvent::new(PointerKind::Drag, to.x, to.y).with_mods(mods)));
    let Some(Action::Preview(cmd, v)) = all.last().cloned() else { panic!("a preview: {all:?}") };
    assert_eq!(cmd, "shape.line");
    let f = |k: &str| v[k].as_f64().unwrap();
    let line = (Point::new(f("x1"), f("y1")), Point::new(f("x2"), f("y2")));
    all.extend(t.pointer(c, &PointerEvent::new(PointerKind::Up, to.x, to.y).with_mods(mods)));
    (line, all)
}

/// Snap to Tangent: a line pressed on one circle and dragged near another lands on a common
/// tangent of the two, both ends moving to where it touches them. Near the second circle's top it
/// is the outer tangent, near its bottom left the one crossing between them, each against the
/// closed form (the drawn circles are within 0.03% of round). The guides show the line and a
/// "tangent" label at each end, and the length label measures the tangent line.
#[test]
fn a_line_from_one_circle_to_another_lands_on_their_common_tangent() {
    let (c1, r1, c2, r2) = (Point::new(100.0, 250.0), 40.0, Point::new(300.0, 250.0), 60.0);
    let d = doc_with_paths(vec![circle(c1, r1), circle(c2, r2)]);
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    // A drawn circle's radius is off by up to 0.03%, which moves the touch points along the line by
    // up to about 0.1 pt here; the line's distance from each center stays within 0.03 pt.
    let near = |a: Point, b: Point| (a - b).hypot() < 0.15;
    let off_by = |(a, b): (Point, Point), c: Point| ((b - a).cross(c - a) / (b - a).hypot()).abs();
    let touches = |line: (Point, Point)| (off_by(line, c1) - r1).abs() < 0.03 && (off_by(line, c2) - r2).abs() < 0.03;
    // A normal n with n · (c2 - c1) / |c2 - c1| = k: the line touches the first circle at
    // c1 + r1 n, and the second at c2 + r2 n (outer, k = (r1 - r2) / d) or c2 - r2 n (crossing,
    // k = (r1 + r2) / d).
    let normal = |k: f64, up: bool| Vec2::new(k, if up { -1.0 } else { 1.0 } * (1.0 - k * k).sqrt());
    let outer = normal((r1 - r2) / 200.0, true);
    let (on_a, on_b) = (c1 + outer * r1, c2 + outer * r2);
    let press = Point::new(100.0, 210.0);
    let ((a, b), actions) = line_drag(&c, press, Point::new(200.0, 200.0), on_b + Vec2::new(0.5, 0.7), Mods::default());
    assert!(near(a, on_a) && near(b, on_b) && touches((a, b)), "{a:?} {b:?}, want {on_a:?} {on_b:?}");
    // One gesture: one undo step.
    assert_eq!(actions.iter().filter(|a| matches!(a, Action::Begin(_))).count(), 1, "{actions:?}");
    assert_eq!(actions.last(), Some(&Action::Commit));
    let crossing = normal((r1 + r2) / 200.0, false);
    let (on_a, on_b) = (c1 + crossing * r1, c2 - crossing * r2);
    let ((a, b), _) = line_drag(&c, press, Point::new(200.0, 260.0), on_b + Vec2::new(-0.6, 0.4), Mods::default());
    assert!(near(a, on_a) && near(b, on_b) && touches((a, b)), "{a:?} {b:?}, want {on_a:?} {on_b:?}");
    // The guides and the length label while dragging.
    let mut t = ShapeTool::new("lineSegment");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, press.x, press.y));
    let to = on_b + Vec2::new(-0.6, 0.4);
    t.pointer(&c, &PointerEvent::new(PointerKind::Drag, to.x, to.y));
    let o = t.overlays(&c);
    let tangents: Vec<Point> = o
        .iter()
        .filter_map(|o| match o {
            Overlay::Label { p, text, .. } if text == "tangent" => Some(*p),
            _ => None,
        })
        .collect();
    assert_eq!(tangents.len(), 2, "{o:?}");
    assert!(near(tangents[0], on_a) && near(tangents[1], on_b), "{o:?}");
    let (a, b) = t.tangent_line.line().unwrap();
    assert!(o.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &c.segment_measure_label(a, b))), "{o:?}");
    // Escape ends it.
    assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
    assert!(t.tangent_line.line().is_none() && t.overlays(&c).is_empty());
}

/// The tangent of a turned ellipse and a circle, against the ellipse's own closed form: the
/// support value of the ellipse x = c + M u (|u| = 1) along n is c · n + |Mᵀ n|, touched at
/// c + M Mᵀ n / |Mᵀ n|; the outer tangents are the n where it equals the circle's c₂ · n + r.
#[test]
fn a_line_from_a_turned_ellipse_lands_on_its_tangent_with_a_circle() {
    let ce = Point::new(100.0, 250.0);
    let turn = Affine::rotate_about(30f64.to_radians(), ce);
    let ellipse = vectorcraft_geom::shapes::ellipse(Rect::new(40.0, 220.0, 160.0, 280.0)).transformed(turn);
    let (c2, r2) = (Point::new(320.0, 260.0), 40.0);
    let d = doc_with_paths(vec![ellipse, circle(c2, r2)]);
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    // M maps the unit circle onto the ellipse's axes: 60 and 30 long, turned 30°.
    let (cos, sin) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
    let m = [[60.0 * cos, -30.0 * sin], [60.0 * sin, 30.0 * cos]];
    let mt_n = |n: Vec2| Vec2::new(m[0][0] * n.x + m[1][0] * n.y, m[0][1] * n.x + m[1][1] * n.y);
    let g = |a: f64| {
        let n = Vec2::new(a.cos(), a.sin());
        ce.to_vec2().dot(n) + mt_n(n).hypot() - c2.to_vec2().dot(n) - r2
    };
    // The roots of g over the angle, by sampling and bisection; the one with n pointing up.
    let mut roots = vec![];
    let steps = 3600;
    for k in 0..steps {
        let (mut lo, mut hi) = (k as f64 * std::f64::consts::TAU / steps as f64, (k + 1) as f64 * std::f64::consts::TAU / steps as f64);
        if g(lo).signum() == g(hi).signum() {
            continue;
        }
        for _ in 0..80 {
            let mid = (lo + hi) / 2.0;
            if g(mid).signum() == g(lo).signum() { lo = mid } else { hi = mid }
        }
        roots.push(lo);
    }
    let n = roots.iter().map(|a| Vec2::new(a.cos(), a.sin())).find(|n| n.y < 0.0).unwrap();
    let w = mt_n(n);
    let on_a = ce + Vec2::new(m[0][0] * w.x + m[0][1] * w.y, m[1][0] * w.x + m[1][1] * w.y) / w.hypot();
    let on_b = c2 + n * r2;
    // The press on the ellipse's turned top anchor.
    let press = turn * Point::new(100.0, 220.0);
    let ((a, b), _) = line_drag(&c, press, Point::new(200.0, 200.0), on_b + Vec2::new(0.4, -0.5), Mods::default());
    assert!((a - on_a).hypot() < 0.05 && (b - on_b).hypot() < 0.05, "{a:?} {b:?}, want {on_a:?} {on_b:?}");
}

/// The line keeps its start where it was pressed, as before, when Snap to Tangent is off, Smart
/// Guides are off, Shift or Alt is held, or the pressed path bends inward somewhere (it has no
/// tangent of this kind). Where the tangent touches the second circle at one of its anchors, the
/// tangent still wins over that anchor.
#[test]
fn a_line_keeps_its_start_without_a_tangent_snap() {
    use vectorcraft_geom::{BezPath, PathData};
    let (c1, c2, r) = (Point::new(100.0, 250.0), Point::new(300.0, 250.0), 50.0);
    let d = doc_with_paths(vec![circle(c1, r), circle(c2, r)]);
    let (s, p) = (Selection::default(), paint());
    let on = cx(&d, &s, &p);
    // The pointer next to the second circle's top anchor, where the outer tangent touches it.
    let to = Point::new(300.6, 200.4);
    // A press on the first circle 30° round from its top, away from its anchors.
    let pressed = c1 + Vec2::new(r * 0.5, -r * 3f64.sqrt() / 2.0);
    // With Snap to Tangent on, the start moves to the first circle's top.
    let ((a, b), _) = line_drag(&on, pressed, Point::new(200.0, 190.0), to, Mods::default());
    assert!((a - Point::new(100.0, 200.0)).hypot() < 0.05 && (b - Point::new(300.0, 200.0)).hypot() < 0.05, "{a:?} {b:?}");
    let off = [ToolContext { snap_tangent: false, ..cx(&d, &s, &p) }, ToolContext { smart_guides: false, ..cx(&d, &s, &p) }];
    for c in &off {
        let ((a, _), _) = line_drag(c, pressed, Point::new(200.0, 190.0), to, Mods::default());
        assert!((a - pressed).hypot() < 0.05, "{a:?}");
    }
    for mods in [Mods { shift: true, ..Mods::default() }, Mods { alt: true, ..Mods::default() }] {
        let ((a, b), _) = line_drag(&on, pressed, Point::new(200.0, 190.0), to, mods);
        // Alt draws the line from its middle.
        let start = if mods.alt { a.midpoint(b) } else { a };
        assert!((start - pressed).hypot() < 0.05, "{a:?} {b:?}");
    }
    // Space moves the line being drawn, its start off the pressed circle: the snap lets go.
    let mut t = ShapeTool::new("lineSegment");
    let space = Mods { space: true, ..Mods::default() };
    t.pointer(&on, &PointerEvent::new(PointerKind::Down, pressed.x, pressed.y));
    t.pointer(&on, &PointerEvent::new(PointerKind::Drag, 200.0, 170.0));
    t.pointer(&on, &PointerEvent::new(PointerKind::Drag, 210.0, 160.0).with_mods(space));
    let a = t.pointer(&on, &PointerEvent::new(PointerKind::Drag, to.x, to.y));
    let Some(Action::Preview(_, v)) = a.last() else { panic!("a preview: {a:?}") };
    let start = Point::new(v["x1"].as_f64().unwrap(), v["y1"].as_f64().unwrap());
    assert!((start - (pressed + Vec2::new(10.0, -10.0))).hypot() < 1.0, "{start:?}");
    // A crescent: closed and curved, but its lower side bends inward. The press on its top
    // keeps its start.
    let mut bp = BezPath::new();
    bp.move_to((60.0, 250.0));
    bp.curve_to((60.0, 190.0), (140.0, 190.0), (140.0, 250.0));
    bp.curve_to((120.0, 220.0), (80.0, 220.0), (60.0, 250.0));
    bp.close_path();
    let d = doc_with_paths(vec![PathData::from_bezpath(&bp), circle(c2, r)]);
    let c = cx(&d, &s, &p);
    let crest = Point::new(100.0, 205.0);
    let ((a, _), _) = line_drag(&c, crest, Point::new(200.0, 190.0), to, Mods::default());
    assert!((a - crest).hypot() < 1e-6, "{a:?}");
}

/// The upper outer common tangent of the circles about `c1` and `c2` with radii `r1` and `r2`
/// (`c2` to the right): where it touches each.
fn upper_outer_tangent(c1: Point, r1: f64, c2: Point, r2: f64) -> (Point, Point) {
    let d = c2 - c1;
    let u = d / d.hypot();
    let k = (r1 - r2) / d.hypot();
    // n · u = k, with n pointing up (y down).
    let n = u * k + Vec2::new(u.y, -u.x) * (1.0 - k * k).sqrt();
    (c1 + n * r1, c2 + n * r2)
}

/// The pointer only has to come near the second path's outline: anywhere along it, the line takes
/// the tangent whose touch point is nearest the pointer, here one about 30 pt away. A press on a
/// straight side of a convex path with curves (a rounded rectangle's top edge) moves the start
/// onto the corner the tangent touches.
#[test]
fn a_line_near_the_second_path_takes_the_tangent_touching_it_nearest() {
    let (c1, r1, c2, r2) = (Point::new(100.0, 250.0), 40.0, Point::new(300.0, 250.0), 60.0);
    let d = doc_with_paths(vec![circle(c1, r1), circle(c2, r2)]);
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let (on_a, on_b) = upper_outer_tangent(c1, r1, c2, r2);
    // On the second circle's outline, 25° round from its top.
    let to = c2 + Vec2::new(r2 * 25f64.to_radians().sin(), -r2 * 25f64.to_radians().cos());
    assert!((to - on_b).hypot() > 25.0);
    let ((a, b), _) = line_drag(&c, Point::new(100.0, 210.0), Point::new(200.0, 200.0), to, Mods::default());
    assert!((a - on_a).hypot() < 0.15 && (b - on_b).hypot() < 0.15, "{a:?} {b:?}, want {on_a:?} {on_b:?}");

    // A rounded rectangle whose top right corner is a quarter circle of radius 20 about (140, 220).
    let rounded = vectorcraft_geom::shapes::rounded_rectangle(Rect::new(40.0, 200.0, 160.0, 300.0), 20.0);
    let (c2, r2) = (Point::new(320.0, 260.0), 40.0);
    let d = doc_with_paths(vec![rounded, circle(c2, r2)]);
    let c = cx(&d, &s, &p);
    let (on_a, on_b) = upper_outer_tangent(Point::new(140.0, 220.0), 20.0, c2, r2);
    let press = Point::new(90.0, 200.0);
    let ((a, b), _) = line_drag(&c, press, Point::new(200.0, 200.0), Point::new(320.0, 220.5), Mods::default());
    assert!((a - on_a).hypot() < 0.15 && (b - on_b).hypot() < 0.15, "{a:?} {b:?}, want {on_a:?} {on_b:?}");
}

/// No tangent snap when the second path bends inward somewhere (a crescent), or when the press
/// snapped to an anchor of other art lying beside the first circle: the start stays where the
/// press put it.
#[test]
fn a_line_to_a_crescent_or_from_an_anchor_beside_the_curve_keeps_its_start() {
    use vectorcraft_geom::{BezPath, PathData};
    let (c1, r) = (Point::new(100.0, 250.0), 50.0);
    let mut bp = BezPath::new();
    bp.move_to((260.0, 250.0));
    bp.curve_to((260.0, 190.0), (340.0, 190.0), (340.0, 250.0));
    bp.curve_to((320.0, 220.0), (280.0, 220.0), (260.0, 250.0));
    bp.close_path();
    let d = doc_with_paths(vec![circle(c1, r), PathData::from_bezpath(&bp)]);
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let pressed = c1 + Vec2::new(r * 0.5, -r * 3f64.sqrt() / 2.0);
    // Beside the crescent's top, which lies at (300, 205).
    let to = Point::new(300.4, 205.3);
    let ((a, b), _) = line_drag(&c, pressed, Point::new(200.0, 190.0), to, Mods::default());
    assert!((a - pressed).hypot() < 0.05 && (b - to).hypot() < 1.0, "{a:?} {b:?}");

    // A square whose corner (99, 199) lies about 1 pt outside the first circle's top; a second
    // circle to the right. The press snaps to that corner and the line starts there.
    let (c2, r2) = (Point::new(300.0, 250.0), 50.0);
    let square = vectorcraft_geom::shapes::rectangle(Rect::new(80.0, 180.0, 99.0, 199.0));
    let d = doc_with_paths(vec![circle(c1, r), square, circle(c2, r2)]);
    let c = cx(&d, &s, &p);
    let corner = Point::new(99.0, 199.0);
    let ((a, _), _) = line_drag(&c, Point::new(99.3, 199.3), Point::new(200.0, 190.0), Point::new(300.6, 200.4), Mods::default());
    assert!((a - corner).hypot() < 1e-6, "{a:?}");
}

/// #5477: with a shape tool, a drag on a selected live rectangle's corner widget rounds its
/// corners (one undo step) and a drag on a live ellipse's pie widget sets its pie angle, as with
/// the Selection tool; the pointer over a widget is the Selection tool's. A press anywhere else, or
/// with Line Segment, draws.
#[test]
fn a_shape_tool_drags_the_selected_shapes_widgets() {
    use vectorcraft_doc::{Appearance, LiveShape, Node, NodeKind};
    let live = |shape: LiveShape| {
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let (l, id) = (d.layers[0].id, d.alloc_id());
        let mut n = Node::path(id, shape.to_path(), Appearance::default_art());
        if let NodeKind::Path { live: slot, .. } = &mut n.kind {
            *slot = Some(Box::new(shape));
        }
        d.insert(Some(l), 0, n).unwrap();
        let mut s = Selection::default();
        s.add(id);
        (d, s)
    };
    let xf = Affine::translate((100.0, 100.0));
    let (d, s) = live(LiveShape::Rectangle { w: 100.0, h: 100.0, radii: [0.0; 4], kinds: Default::default(), radii_relative: false, xf });
    let p = paint();
    let c = cx(&d, &s, &p);
    let at = Point::new(110.0, 110.0);
    for id in ["rectangle", "roundedRectangle", "polygon", "star", "ellipse"] {
        let mut t = ShapeTool::new(id);
        assert_eq!(t.cursor(&c, at, Mods::default()), Cursor::CornerRadius, "{id}");
        // Hovering a widget shows no Smart Guide for a new shape.
        t.pointer(&c, &PointerEvent::new(PointerKind::Move, at.x, at.y));
        assert!(t.overlays(&c).is_empty(), "{id}");
        assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, at.x, at.y)).is_empty(), "{id}");
        assert!(t.busy(), "{id}");
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 130.0, 130.0));
        assert_eq!(a.first(), Some(&Action::Begin("Corner Radius".into())), "{id}: {a:?}");
        assert!(
            matches!(a.last(), Some(Action::Preview(cmd, v)) if cmd == "object.setLiveShape" && v["radius"].as_f64().is_some_and(|r| r > 0.0)),
            "{id}: {a:?}"
        );
        assert!(t.transforming() && !t.drawing_boxed() && !t.moving(), "{id}");
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 130.0, 130.0)), vec![Action::Commit], "{id}");
        assert!(!t.busy() && !t.transforming(), "{id}");
        // Escape cancels a widget drag.
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, at.x, at.y));
        t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 130.0, 130.0));
        assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel], "{id}");
        assert!(!t.busy(), "{id}");
        // Away from the widgets the tool draws.
        assert_eq!(t.cursor(&c, Point::new(300.0, 300.0), Mods::default()), Cursor::Crosshair, "{id}");
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 350.0, 340.0));
        assert!(matches!(a.last(), Some(Action::Preview(cmd, _)) if cmd.starts_with("shape.")), "{id}: {a:?}");
    }
    // Line Segment draws from a widget, as it always did.
    let mut t = ShapeTool::new("lineSegment");
    assert_eq!(t.cursor(&c, at, Mods::default()), Cursor::Crosshair);
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, at.x, at.y));
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 130.0, 130.0));
    assert!(matches!(a.last(), Some(Action::Preview(cmd, _)) if cmd == "shape.line"), "{a:?}");
    // A whole live ellipse's pie widgets sit 12 px right of (300, 150).
    let (d, s) = live(LiveShape::Ellipse { w: 200.0, h: 100.0, pie: (0.0, 360.0), xf });
    let c = cx(&d, &s, &p);
    let mut t = ShapeTool::new("ellipse");
    assert_eq!(t.cursor(&c, Point::new(312.0, 150.0), Mods::default()), Cursor::ArrowHollow);
    assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 312.0, 150.0)).is_empty());
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 200.0, 20.0));
    assert_eq!(a.first(), Some(&Action::Begin("Live Shape".into())), "{a:?}");
    assert!(matches!(a.last(), Some(Action::Preview(cmd, v)) if cmd == "object.setLiveShape" && v.get("pieEnd").is_some()), "{a:?}");
    assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 200.0, 20.0)), vec![Action::Commit]);
}

/// A double-click on a corner widget of a selected live shape, with a shape tool that shows the
/// widgets, asks for the Transform panel and nothing else: neither click draws, edits or begins an
/// undo step, and the tool stays idle. Line Segment, a plain path, a pie widget, a hidden or
/// angle-excluded corner widget and a spot off the widgets ask for nothing.
#[test]
fn a_double_click_on_a_corner_widget_shows_the_transform_panel() {
    use vectorcraft_doc::{Appearance, LiveShape, Node, NodeKind};
    let live = |shape: LiveShape| {
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let (l, id) = (d.layers[0].id, d.alloc_id());
        let mut n = Node::path(id, shape.to_path(), Appearance::default_art());
        if let NodeKind::Path { live: slot, .. } = &mut n.kind {
            *slot = Some(Box::new(shape));
        }
        d.insert(Some(l), 0, n).unwrap();
        let mut s = Selection::default();
        s.add(id);
        (d, s)
    };
    let double_click = |t: &mut ShapeTool, c: &ToolContext, at: Point| {
        let mut out = vec![];
        for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
            out.push(t.pointer(c, &PointerEvent::new(kind, at.x, at.y)));
        }
        out
    };
    let panel = vec![Action::Panel("transform".into())];
    let xf = Affine::translate((100.0, 100.0));
    let (d, s) = live(LiveShape::Rectangle { w: 100.0, h: 100.0, radii: [0.0; 4], kinds: Default::default(), radii_relative: false, xf });
    let p = paint();
    let c = cx(&d, &s, &p);
    let at = Point::new(110.0, 110.0);
    for id in ["rectangle", "roundedRectangle", "ellipse", "polygon", "star"] {
        let mut t = ShapeTool::new(id);
        let out = double_click(&mut t, &c, at);
        assert!(out[..4].iter().all(Vec::is_empty), "{id}: {out:?}");
        assert_eq!(out[4], panel, "{id}");
        assert!(!t.busy() && !t.transforming(), "{id}");
        // Off the widgets, a double-click asks for nothing.
        assert!(t.pointer(&c, &PointerEvent::new(PointerKind::DoubleClick, 300.0, 300.0)).is_empty(), "{id}");
    }
    // Line Segment has no shape widgets.
    let mut t = ShapeTool::new("lineSegment");
    assert!(t.pointer(&c, &PointerEvent::new(PointerKind::DoubleClick, at.x, at.y)).is_empty());
    // Show Corner Widget off, or the corner wider than the angle limit: the widget is hidden.
    let mut hidden = cx(&d, &s, &p);
    hidden.corner_widgets = false;
    let mut t = ShapeTool::new("rectangle");
    assert!(t.pointer(&hidden, &PointerEvent::new(PointerKind::DoubleClick, at.x, at.y)).is_empty());
    let mut narrow = cx(&d, &s, &p);
    narrow.corner_widget_max_angle = 80.0;
    assert!(t.pointer(&narrow, &PointerEvent::new(PointerKind::DoubleClick, at.x, at.y)).is_empty());
    // A live polygon's corner widget asks for the panel too.
    let (d, s) = live(LiveShape::Polygon {
        radius: 100.0,
        sides: 6,
        xf: Affine::translate((250.0, 250.0)),
        radii: vec![],
        kinds: vec![],
        radii_relative: false,
    });
    let c = cx(&d, &s, &p);
    let w = corners::CornerWidgets::for_tool(&c, false).and_then(|w| w.visible().next()).unwrap();
    let mut t = ShapeTool::new("polygon");
    assert_eq!(double_click(&mut t, &c, w)[4], panel);
    // A plain path shows no corner widget to a shape tool.
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let c = cx(&d, &s, &p);
    let mut t = ShapeTool::new("rectangle");
    assert!(t.pointer(&c, &PointerEvent::new(PointerKind::DoubleClick, at.x, at.y)).is_empty());
    // A live ellipse's pie widget, 12 px right of (300, 150), is not a corner widget.
    let (d, s) = live(LiveShape::Ellipse { w: 200.0, h: 100.0, pie: (0.0, 360.0), xf });
    let c = cx(&d, &s, &p);
    let mut t = ShapeTool::new("ellipse");
    assert!(double_click(&mut t, &c, Point::new(312.0, 150.0)).iter().all(Vec::is_empty));
    assert!(!t.busy());
}

/// A selected live 100 × 100 rectangle at (100, 100)–(200, 200) with no paint: its centre point
/// (Attributes › Show Center) is at (150, 150) and its corners at the box's.
fn unfilled_live_rect() -> (vectorcraft_doc::Document, Selection) {
    let (d, id) = doc_with_live_rect();
    let mut s = Selection::default();
    s.add(id);
    (d, s)
}

/// #5477: with a shape tool the selection's centre point moves it and the zone just outside a
/// corner of its box turns it, as the Selection tool's do; anywhere else a press draws a new
/// shape. The centre is where no art is painted, so without this a press there would draw one.
#[test]
fn a_shape_tool_moves_the_selection_by_its_centre_point_and_turns_it_from_a_corner() {
    let (d, s) = unfilled_live_rect();
    let p = paint();
    let c = cx(&d, &s, &p);
    let centre = Point::new(150.0, 150.0);
    for id in ["rectangle", "roundedRectangle", "polygon", "star", "ellipse"] {
        let mut t = ShapeTool::new(id);
        assert_eq!(t.cursor(&c, centre, Mods::default()), Cursor::Move, "{id}");
        // No Smart Guide for a new shape's start over the centre point.
        t.pointer(&c, &PointerEvent::new(PointerKind::Move, centre.x, centre.y));
        assert!(t.overlays(&c).is_empty(), "{id}");
        assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, centre.x, centre.y)).is_empty(), "{id}");
        assert!(t.busy(), "{id}");
        // A drag moves the selection by the pointer's offset.
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 170.0, 160.0));
        assert_eq!(a.first(), Some(&Action::Begin("Move".into())), "{id}: {a:?}");
        // A move, which the canvas can show the bounding box for.
        assert!(t.transforming() && t.moving() && !t.resizing() && !t.drawing_boxed(), "{id}");
        assert_eq!(
            a.last(),
            Some(&Action::Preview("object.transform".into(), json!({"matrix": [1.0, 0.0, 0.0, 1.0, 20.0, 10.0], "copy": false}))),
            "{id}"
        );
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 170.0, 160.0)), vec![Action::Commit], "{id}");
        assert!(!t.busy() && !t.transforming() && !t.moving(), "{id}");
    }
    // Alt pressed once the drag has begun copies, and the copy cursor shows, as the Selection
    // tool's do.
    let mut t = ShapeTool::new("rectangle");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, centre.x, centre.y));
    t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 160.0, 150.0));
    let alt = Mods { alt: true, ..Mods::default() };
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 170.0, 150.0).with_mods(alt));
    assert!(matches!(a.last(), Some(Action::Preview(_, v)) if v["copy"] == json!(true)), "{a:?}");
    assert_eq!(t.cursor(&c, Point::new(170.0, 150.0), alt), Cursor::SelectionCopy);
    // View › Hide Bounding Box hides the box, not the centre point: a press there still moves.
    let hidden = ToolContext { show_bbox: false, ..cx(&d, &s, &p) };
    let mut t = ShapeTool::new("rectangle");
    assert_eq!(t.cursor(&hidden, centre, Mods::default()), Cursor::Move);
    t.pointer(&hidden, &PointerEvent::new(PointerKind::Down, centre.x, centre.y));
    let a = t.pointer(&hidden, &PointerEvent::new(PointerKind::Drag, 170.0, 160.0));
    assert_eq!(a.first(), Some(&Action::Begin("Move".into())), "{a:?}");
    // On the corner's bounding-box handle, and on the corner widget itself, no turn: those keep
    // their own gestures (the widget rounds the corner, the handle scales with the Selection tool).
    for on_handle in [Point::new(205.0, 205.0), Point::new(200.0, 200.0)] {
        let t = ShapeTool::new("rectangle");
        assert_ne!(t.cursor(&c, on_handle, Mods::default()), Cursor::rotate_about(centre, on_handle), "{on_handle:?}");
    }
    // Just outside the bottom-right corner, past the handle's 5 pt square and within the 18 pt
    // zone (8.49 pt away): a press there turns the selection about its centre.
    let outside = Point::new(206.0, 206.0);
    let mut t = ShapeTool::new("rectangle");
    assert_eq!(t.cursor(&c, outside, Mods::default()), Cursor::rotate_about(centre, outside));
    // A turn begins with the press, as the Selection tool's does (its drags preview into it).
    assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, outside.x, outside.y)), vec![Action::Begin("Rotate".into())]);
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 150.0, 205.0));
    let (want, _) = rotate_for_drag(centre, outside, Point::new(150.0, 205.0), false);
    assert_eq!(a.last(), Some(&Action::Preview("object.transform".into(), json!({"matrix": matrix_json(want), "copy": false}))), "{a:?}");
    assert!(t.transforming());
    // Escape turns it back.
    assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
    assert!(!t.busy());
    // Away from the centre point and the corners, the tool draws a new shape.
    let away = Point::new(300.0, 300.0);
    let mut t = ShapeTool::new("rectangle");
    assert_eq!(t.cursor(&c, away, Mods::default()), Cursor::Crosshair);
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, away.x, away.y));
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 350.0, 340.0));
    assert_eq!(a.first(), Some(&Action::Begin("Rectangle".into())), "{a:?}");
    assert!(t.drawing_boxed() && !t.moving() && !t.transforming());
}

/// The shape tools that show the selection's bounding box report a shape being drawn once the drag
/// has begun, until the release or Escape. The Line Segment tool shows no box and reports none.
#[test]
fn a_shape_being_drawn_has_its_bounding_box_drawn_by_the_tools_that_show_one() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let c = cx(&d, &s, &p);
    for id in ["rectangle", "roundedRectangle", "ellipse", "polygon", "star", "lineSegment"] {
        let mut t = ShapeTool::new(id);
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        assert!(!t.drawing_boxed(), "{id}: not before the drag begins");
        t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 160.0, 140.0));
        assert_eq!(t.drawing_boxed(), id != "lineSegment", "{id}");
        assert!(!t.moving() && !t.resizing(), "{id}");
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 160.0, 140.0));
        assert!(!t.drawing_boxed(), "{id}: not after the release");
        // Escape ends the drag.
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 160.0, 140.0));
        t.key(&c, ToolKey::Escape, Mods::default());
        assert!(!t.drawing_boxed(), "{id}: not after Escape");
    }
}

/// With a shape tool, a drag on one of the eight handles of the selection's bounding box scales the
/// selection, as with the Selection tool, instead of drawing a new shape from the handle.
#[test]
fn a_shape_tool_scales_the_selection_by_its_bounding_box_handles() {
    let (d, s) = unfilled_live_rect();
    let p = paint();
    let c = cx(&d, &s, &p);
    let corner = Point::new(200.0, 200.0);
    for id in ["rectangle", "roundedRectangle", "polygon", "star", "ellipse"] {
        let mut t = ShapeTool::new(id);
        assert_eq!(t.cursor(&c, corner, Mods::default()), handle_cursor(Handle::BottomRight, 0.0), "{id}");
        // No Smart Guide for a new shape's start over a handle.
        t.pointer(&c, &PointerEvent::new(PointerKind::Move, corner.x, corner.y));
        assert!(t.overlays(&c).is_empty(), "{id}");
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, corner.x, corner.y)), vec![Action::Begin("Scale".into())], "{id}");
        // A resize: the canvas keeps the box on show at the changing bounds.
        assert!(t.busy() && t.transforming() && t.resizing() && !t.moving(), "{id}");
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 220.0, 230.0));
        let want = scale_for_drag(Rect::new(100.0, 100.0, 200.0, 200.0), Handle::BottomRight, Point::new(220.0, 230.0), false, false);
        assert_eq!(a, vec![Action::Preview("object.transform".into(), json!({"matrix": matrix_json(want), "copy": false}))], "{id}");
        assert_eq!(t.cursor(&c, Point::new(220.0, 230.0), Mods::default()), handle_cursor(Handle::BottomRight, 0.0), "{id}");
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 220.0, 230.0)), vec![Action::Commit], "{id}");
        assert!(!t.busy() && !t.transforming() && !t.resizing(), "{id}");
    }
    // The side handles scale along one axis; Escape turns the scale back.
    let mut t = ShapeTool::new("rectangle");
    assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 100.0)), vec![Action::Begin("Scale".into())]);
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 150.0, 80.0));
    let want = scale_for_drag(Rect::new(100.0, 100.0, 200.0, 200.0), Handle::Top, Point::new(150.0, 80.0), false, false);
    assert_eq!(a, vec![Action::Preview("object.transform".into(), json!({"matrix": matrix_json(want), "copy": false}))]);
    assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
    assert!(!t.busy());
    // Backtick pressed after the last drag: the release previews the scale again for the patterns
    // only, then commits.
    let mut t = ShapeTool::new("rectangle");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, corner.x, corner.y));
    t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 220.0, 230.0));
    let a = t.pointer(&c, &PointerEvent { backquote: true, ..PointerEvent::new(PointerKind::Up, 220.0, 230.0) });
    let want = scale_for_drag(Rect::new(100.0, 100.0, 200.0, 200.0), Handle::BottomRight, Point::new(220.0, 230.0), false, false);
    let patterns = json!({"matrix": matrix_json(want), "copy": false, "objects": false, "patterns": true});
    assert_eq!(a, vec![Action::Preview("object.transform".into(), patterns), Action::Commit]);
    assert!(!t.busy());
    // View › Hide Bounding Box hides the handles, and a press there draws a new shape.
    let hidden = ToolContext { show_bbox: false, ..cx(&d, &s, &p) };
    let mut t = ShapeTool::new("rectangle");
    assert_eq!(t.cursor(&hidden, corner, Mods::default()), Cursor::Crosshair);
    t.pointer(&hidden, &PointerEvent::new(PointerKind::Down, corner.x, corner.y));
    let a = t.pointer(&hidden, &PointerEvent::new(PointerKind::Drag, 240.0, 240.0));
    assert_eq!(a.first(), Some(&Action::Begin("Rectangle".into())), "{a:?}");
    // The Line Segment tool shows no box, so its press draws.
    let mut t = ShapeTool::new("lineSegment");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, corner.x, corner.y));
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 240.0, 240.0));
    assert_eq!(a.first(), Some(&Action::Begin("Line".into())), "{a:?}");
}

/// A shape tool's press scales the selection only on the drawn handle square: 3 screen points from
/// the handle's centre along each axis at the default Anchor Size, at any zoom. A press 4 points
/// from it draws a new shape; at Anchor Size 7 the square is 10 points wide and that press scales.
#[test]
fn a_shape_tool_draws_from_a_press_just_beside_a_handle() {
    let (d, s) = unfilled_live_rect();
    let p = paint();
    for zoom in [1.0, 4.0] {
        let c = ToolContext { zoom, ..cx(&d, &s, &p) };
        let on = Point::new(200.0 + 3.0 / zoom, 200.0 - 3.0 / zoom);
        let mut t = ShapeTool::new("rectangle");
        assert_eq!(t.cursor(&c, on, Mods::default()), handle_cursor(Handle::BottomRight, 0.0), "{zoom}");
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, on.x, on.y)), vec![Action::Begin("Scale".into())], "{zoom}");
        let beside = Point::new(200.0 + 2.0 / zoom, 200.0 + 4.0 / zoom);
        let mut t = ShapeTool::new("rectangle");
        assert_eq!(t.cursor(&c, beside, Mods::default()), Cursor::Crosshair, "{zoom}");
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, beside.x, beside.y));
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 260.0, 260.0));
        assert_eq!(a.first(), Some(&Action::Begin("Rectangle".into())), "{zoom}: {a:?}");
        let big = ToolContext { anchor_size: 7, ..c };
        let mut t = ShapeTool::new("rectangle");
        assert_eq!(t.cursor(&big, beside, Mods::default()), handle_cursor(Handle::BottomRight, 0.0), "{zoom}");
        assert_eq!(t.pointer(&big, &PointerEvent::new(PointerKind::Down, beside.x, beside.y)), vec![Action::Begin("Scale".into())], "{zoom}");
    }
    // In a view turned 45°, the drawn square stays square to the screen: a press 3 screen points
    // right of and below the handle's centre, 4.24 pt along the box's x axis, scales.
    let screen = turned_view(45.0);
    let c = ToolContext { screen, ..cx(&d, &s, &p) };
    let f = screen.unwrap_or_else(|| panic!("a turned view"));
    let corner = Point::new(200.0, 200.0) + f.right * 3.0 + f.down * 3.0;
    let mut t = ShapeTool::new("rectangle");
    assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, corner.x, corner.y)), vec![Action::Begin("Scale".into())]);
}

/// Two selected live rectangles, the second placed so that its top-left Live Corners widget (10 pt
/// in along the corner's bisector) lands on the first one's centre point.
fn two_live_rects() -> (vectorcraft_doc::Document, Selection) {
    use vectorcraft_doc::{Appearance, LiveShape, Node, NodeKind};
    let (mut d, first) = doc_with_live_rect();
    let inside = 10.0 / std::f64::consts::SQRT_2;
    let shape = LiveShape::Rectangle {
        w: 100.0,
        h: 100.0,
        radii: [0.0; 4],
        kinds: Default::default(),
        radii_relative: false,
        xf: Affine::translate((150.0 - inside, 150.0 - inside)),
    };
    let (layer, id) = (d.layers[0].id, d.alloc_id());
    let mut n = Node::path(id, shape.to_path(), Appearance::default());
    if let NodeKind::Path { live: slot, .. } = &mut n.kind {
        *slot = Some(Box::new(shape));
    }
    d.insert(Some(layer), 1, n).unwrap();
    let mut s = Selection::default();
    s.add(first);
    s.add(id);
    (d, s)
}

/// A press takes a Live Corners widget before a centre point, as the Selection tool does: with two
/// shapes selected, one's corner widget may sit on another's centre, and it rounds rather than
/// moving the selection.
#[test]
fn a_live_corner_widget_wins_over_another_shapes_centre_point() {
    let (d, s) = two_live_rects();
    let p = paint();
    let c = cx(&d, &s, &p);
    let at = Point::new(150.0, 150.0);
    assert!(corners::over_widget(&c, at, false), "a widget is on the centre point");
    assert!(hit_center(&c, at, c.point_tol()).is_some(), "and so is the centre point");
    let mut t = ShapeTool::new("rectangle");
    assert_eq!(t.cursor(&c, at, Mods::default()), Cursor::CornerRadius, "the widget takes the pointer");
    assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, at.x, at.y)).is_empty());
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 160.0, 160.0));
    assert_eq!(a.first(), Some(&Action::Begin("Corner Radius".into())), "{a:?}");
}

/// A move of the selection from the centre point snaps like the Selection tool's, and the arrow
/// turns hollow while it does.
#[test]
fn a_move_from_the_centre_point_snaps_and_shows_the_hollow_arrow() {
    use vectorcraft_doc::{Appearance, Node};
    let (mut d, s) = unfilled_live_rect();
    // Another rectangle whose left edge is at x = 300: the moved selection's right edge lands 4 pt
    // short of it, within the Smart Guides' reach.
    let (layer, id) = (d.layers[0].id, d.alloc_id());
    d.insert(Some(layer), 1, Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(300.0, 100.0, 350.0, 180.0)), Appearance::default_art()))
        .unwrap();
    let p = paint();
    let c = cx(&d, &s, &p);
    let mut t = ShapeTool::new("rectangle");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 246.0, 150.0));
    assert!(matches!(a.last(), Some(Action::Preview(_, v)) if v["matrix"][4] == json!(100.0)), "snapped to the edge: {a:?}");
    assert_eq!(t.cursor(&c, Point::new(246.0, 150.0), Mods::default()), Cursor::ArrowHollow);
    // Away from the neighbour, the move is the pointer's own and the arrow is plain.
    let mut t = ShapeTool::new("rectangle");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 160.0, 150.0));
    assert!(matches!(a.last(), Some(Action::Preview(_, v)) if v["matrix"][4] == json!(10.0)), "{a:?}");
    assert_eq!(t.cursor(&c, Point::new(160.0, 150.0), Mods::default()), Cursor::Arrow);
}

/// `tool.setOption` takes an ellipse's anchor count from 2 to 1000, as the tool keeps it.
#[test]
fn the_anchor_count_is_checked_against_its_range() {
    let t = ShapeTool::new("ellipse");
    for n in [2, 4, 1000] {
        assert_eq!(t.check_option("anchors", &json!(n)), Ok(()), "{n}");
    }
    for bad in [json!(1), json!(1001), json!(4.5), json!("4")] {
        assert_eq!(t.check_option("anchors", &bad), Err("`anchors` must be a whole number from 2 to 1000".into()), "{bad}");
    }
}

/// A drag event to `(x, y)` with Backtick held or not.
fn quote_drag(x: f64, y: f64, held: bool) -> PointerEvent {
    PointerEvent { backquote: held, ..PointerEvent::new(PointerKind::Drag, x, y) }
}

/// The shapes the last preview in `a` draws, as their commands and params: the steps of a
/// `command.batch`, or the one shape. The bool says whether it was a batch.
fn previewed_shapes(a: &[Action]) -> (bool, Vec<(String, Value)>) {
    match a.last() {
        Some(Action::Preview(c, v)) if c == "command.batch" => {
            assert!(v.get("commands").and_then(Value::as_array).is_some(), "{v}");
            let steps = v["commands"].as_array().into_iter().flatten();
            (true, steps.map(|s| (s["command"].as_str().unwrap_or_default().to_owned(), s["params"].clone())).collect())
        }
        Some(Action::Preview(c, v)) => (false, vec![(c.clone(), v.clone())]),
        _ => panic!("no preview: {a:?}"),
    }
}

/// The widths of the rectangles the last preview in `a` draws, and whether it was a batch.
fn previewed_widths(a: &[Action]) -> (bool, Vec<f64>) {
    let (batch, shapes) = previewed_shapes(a);
    (batch, shapes.iter().map(|(_, p)| p["width"].as_f64().unwrap_or(f64::NAN)).collect())
}

/// Backtick held while a rectangle is dragged out leaves a copy every 8 screen points the pointer
/// travels from where the key was first held or the last copy was left; the shape being drawn
/// still follows the pointer. A key press without pointer travel leaves none, letting go stops
/// the copies and keeps those left, and pressing it again starts measuring afresh. The release
/// commits once.
#[test]
fn backtick_drag_leaves_copies_of_the_rectangle_being_drawn() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let c = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ShapeTool::new("rectangle");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
    assert_eq!(previewed_widths(&t.pointer(&c, &quote_drag(50.0, 30.0, false))), (false, vec![40.0]));
    // The key goes down with the pointer still: no copy.
    assert_eq!(previewed_widths(&t.pointer(&c, &quote_drag(50.0, 30.0, true))), (false, vec![40.0]));
    // 4 pt of travel is short of the spacing.
    assert_eq!(previewed_widths(&t.pointer(&c, &quote_drag(54.0, 30.0, true))), (false, vec![44.0]));
    // 10 pt: a copy, the same as the shape shown, so the batch draws it once.
    assert_eq!(previewed_widths(&t.pointer(&c, &quote_drag(60.0, 30.0, true))), (true, vec![50.0]));
    assert_eq!(previewed_widths(&t.pointer(&c, &quote_drag(70.0, 30.0, true))), (true, vec![50.0, 60.0]));
    // 5 pt past the last copy: none left, and the live shape follows.
    assert_eq!(previewed_widths(&t.pointer(&c, &quote_drag(75.0, 30.0, true))), (true, vec![50.0, 60.0, 65.0]));
    // Let go of the key: the copies stay and no more are left.
    assert_eq!(previewed_widths(&t.pointer(&c, &quote_drag(90.0, 30.0, false))), (true, vec![50.0, 60.0, 80.0]));
    assert_eq!(previewed_widths(&t.pointer(&c, &quote_drag(120.0, 30.0, false))), (true, vec![50.0, 60.0, 110.0]));
    // Pressed again: measured from where it went down.
    assert_eq!(previewed_widths(&t.pointer(&c, &quote_drag(125.0, 30.0, true))), (true, vec![50.0, 60.0, 115.0]));
    assert_eq!(previewed_widths(&t.pointer(&c, &quote_drag(135.0, 30.0, true))), (true, vec![50.0, 60.0, 125.0]));
    assert_eq!(t.pointer(&c, &PointerEvent { backquote: true, ..PointerEvent::new(PointerKind::Up, 135.0, 30.0) }), vec![Action::Commit]);
    // The next drag starts with no copies.
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
    assert_eq!(previewed_widths(&t.pointer(&c, &quote_drag(50.0, 30.0, false))), (false, vec![40.0]));
}

/// Backtick already held at the press measures the first copy from the pressed spot: the first
/// drag 10 pt away leaves one, and 5 pt more leaves none.
#[test]
fn backtick_held_at_the_press_measures_copies_from_there() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let c = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ShapeTool::new("rectangle");
    t.pointer(&c, &PointerEvent { backquote: true, ..PointerEvent::new(PointerKind::Down, 0.0, 0.0) });
    assert_eq!(previewed_widths(&t.pointer(&c, &quote_drag(10.0, 10.0, true))), (true, vec![10.0]));
    assert_eq!(previewed_widths(&t.pointer(&c, &quote_drag(15.0, 10.0, true))), (true, vec![10.0, 15.0]));
    assert_eq!(t.pointer(&c, &PointerEvent { backquote: true, ..PointerEvent::new(PointerKind::Up, 15.0, 10.0) }), vec![Action::Commit]);
}

/// Every shape tool leaves copies of its own shape, drawn with the modifiers held when each copy
/// was left: Shift's square stays square, and Alt's from the centre stays centred.
#[test]
fn backtick_drag_leaves_copies_with_every_shape_tool_and_its_modifiers() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let c = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    for (id, cmd) in [
        ("rectangle", "shape.rectangle"),
        ("roundedRectangle", "shape.rectangle"),
        ("ellipse", "shape.ellipse"),
        ("polygon", "shape.polygon"),
        ("star", "shape.star"),
        ("lineSegment", "shape.line"),
    ] {
        let mut t = ShapeTool::new(id);
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&c, &quote_drag(140.0, 130.0, true));
        t.pointer(&c, &quote_drag(160.0, 130.0, true));
        let (batch, shapes) = previewed_shapes(&t.pointer(&c, &quote_drag(180.0, 140.0, true)));
        assert!(batch, "{id}");
        assert_eq!(shapes.len(), 2, "{id}: two copies, the second the shape shown: {shapes:?}");
        assert!(shapes.iter().all(|(c, _)| c == cmd), "{id}: {shapes:?}");
        assert_ne!(shapes[0].1, shapes[1].1, "{id}: each copy as the drag showed it then");
    }
    // Shift and Alt: each copy is the square drawn from the centre at that moment.
    let shift_alt = Mods { shift: true, alt: true, ..Default::default() };
    let mut t = ShapeTool::new("rectangle");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
    t.pointer(&c, &quote_drag(130.0, 120.0, true).with_mods(shift_alt));
    let (_, shapes) = previewed_shapes(&t.pointer(&c, &quote_drag(150.0, 120.0, true).with_mods(shift_alt)));
    let (_, r) = &shapes[0];
    assert_eq!(r["width"], r["height"], "square: {r}");
    assert_eq!(r["x"].as_f64().unwrap_or_default() + r["width"].as_f64().unwrap_or_default() / 2.0, 100.0, "centred: {r}");
}

/// ↑ during a polygon drag with copies left changes only the shape being drawn; Escape cancels
/// the copies with it, and the next drag starts without them.
#[test]
fn backtick_copies_keep_their_sides_and_escape_cancels_them() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let c = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ShapeTool::new("polygon");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
    t.pointer(&c, &quote_drag(140.0, 100.0, true));
    t.pointer(&c, &quote_drag(150.0, 100.0, true));
    let (batch, shapes) = previewed_shapes(&t.key(&c, ToolKey::Up, Mods::default()));
    assert!(batch);
    let sides: Vec<_> = shapes.iter().map(|(_, p)| p["sides"].clone()).collect();
    assert_eq!(sides, vec![json!(6), json!(7)], "the copy keeps its sides");
    assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
    let (batch, _) = previewed_shapes(&t.pointer(&c, &quote_drag(140.0, 100.0, false)));
    assert!(!batch, "no copies after Escape");
}

/// No copy is left of a shape without a height or a length, and one drag leaves at most
/// [`MAX_COPIES`] copies while the shape being drawn still follows the pointer.
#[test]
fn backtick_copies_skip_empty_shapes_and_stop_at_the_limit() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let c = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    // A rectangle with no height: nothing is left.
    let mut t = ShapeTool::new("rectangle");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
    for x in [30.0, 40.0, 50.0, 60.0] {
        assert!(!previewed_shapes(&t.pointer(&c, &quote_drag(x, 10.0, true))).0, "{x}");
    }
    // A line back on its start: no copy there.
    let mut t = ShapeTool::new("lineSegment");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
    t.pointer(&c, &quote_drag(30.0, 10.0, true));
    assert!(!previewed_shapes(&t.pointer(&c, &quote_drag(10.0, 10.0, true))).0);
    // The limit.
    let mut t = ShapeTool::new("rectangle");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
    t.pointer(&c, &quote_drag(10.0, 10.0, true));
    let mut last = vec![];
    for i in 1..=(MAX_COPIES as u32 + 20) {
        let at = 10.0 + f64::from(i) * 10.0;
        last = t.pointer(&c, &quote_drag(at, at, true));
    }
    let (batch, widths) = previewed_widths(&last);
    assert!(batch);
    assert_eq!(widths.len(), MAX_COPIES + 1, "the copies and the shape being drawn");
    assert_eq!(widths.last(), Some(&(10.0 + f64::from(MAX_COPIES as u32 + 20) * 10.0)), "the shape follows the pointer");
}

/// The spacing is measured on screen: at 400% zoom, 2 pt of travel is 8 screen points.
#[test]
fn backtick_copy_spacing_is_measured_on_screen() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let c = ToolContext { smart_guides: false, zoom: 4.0, ..cx(&d, &s, &p) };
    let mut t = ShapeTool::new("rectangle");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
    t.pointer(&c, &quote_drag(20.0, 20.0, true));
    assert!(!previewed_shapes(&t.pointer(&c, &quote_drag(21.5, 20.0, true))).0, "6 screen points");
    assert!(previewed_shapes(&t.pointer(&c, &quote_drag(22.0, 20.0, true))).0, "8 screen points");
}
