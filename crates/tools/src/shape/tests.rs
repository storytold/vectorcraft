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
    // holds what it shows, not the canceled radius.
    let mut t = ShapeTool::new("star");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 90.0, 50.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 130.0, 50.0).with_mods(ctrl));
    assert_eq!(t.key(&cx, ToolKey::Escape, ctrl), vec![Action::Cancel]);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
    close(radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 60.0, 50.0).with_mods(ctrl))), (10.0, 2.5));
    close(radii(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 70.0, 50.0).with_mods(ctrl))), (20.0, 2.5));

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
