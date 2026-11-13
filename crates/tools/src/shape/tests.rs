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
