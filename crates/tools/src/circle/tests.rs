use super::*;
use crate::testutil::*;
use vectorcraft_doc::Selection;

fn ev(kind: PointerKind, (x, y): (f64, f64), mods: Mods) -> PointerEvent {
    PointerEvent::new(kind, x, y).with_mods(mods)
}

fn click(t: &mut CircleTool, cx: &ToolContext, p: (f64, f64)) -> Vec<Action> {
    let mut acts = t.pointer(cx, &ev(PointerKind::Down, p, Mods::default()));
    acts.extend(t.pointer(cx, &ev(PointerKind::Up, p, Mods::default())));
    acts
}

/// A press at `from`, dragged through `path` and let go at its last point.
fn drag(t: &mut CircleTool, cx: &ToolContext, from: (f64, f64), path: &[(f64, f64)], mods: Mods) -> Vec<Action> {
    let mut acts = t.pointer(cx, &ev(PointerKind::Down, from, mods));
    for p in path {
        acts.extend(t.pointer(cx, &ev(PointerKind::Drag, *p, mods)));
    }
    let end = path.last().copied().unwrap_or(from);
    acts.extend(t.pointer(cx, &ev(PointerKind::Up, end, mods)));
    acts
}

fn circle(points: &[(f64, f64)]) -> Vec<Action> {
    let pairs: Vec<Value> = points.iter().map(|(x, y)| json!([x, y])).collect();
    vec![Action::Exec("shape.circleThroughPoints".into(), json!({ "points": pairs }))]
}

/// The dashed circle the tool shows: its bounding box.
fn preview(o: &[Overlay]) -> Option<Rect> {
    o.iter().find_map(|o| match o {
        Overlay::Path { path, dashed: true, .. } => Some(vectorcraft_geom::Shape::bounding_box(path)),
        _ => None,
    })
}

fn near(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(p, q)| (p - q).abs() < 1e-9)
}

/// A drag from one point to another draws the circle with those two as the ends of a diameter.
#[test]
fn a_drag_draws_the_circle_on_its_diameter() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    assert_eq!(drag(&mut t, &cx, (300.0, 300.0), &[(340.0, 330.0), (380.0, 360.0)], Mods::default()), circle(&[(300.0, 300.0), (380.0, 360.0)]));
    assert!(!t.ends_on_escape(&cx) && t.overlays(&cx).is_empty());
}

/// A click places the first point and a second click the other end of the diameter. In between,
/// the tool shows the circle through the first point and the pointer, its center cross, its radii
/// and its diameter.
#[test]
fn two_clicks_draw_the_circle_on_their_diameter() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    assert_eq!(click(&mut t, &cx, (300.0, 300.0)), vec![]);
    assert!(t.ends_on_escape(&cx) && t.claims_key(&cx, ToolKey::Escape));
    t.pointer(&cx, &ev(PointerKind::Move, (380.0, 360.0), Mods::default()));
    let o = t.overlays(&cx);
    // Center (340, 330), radius 50.
    assert!(near(preview(&o).expect("the circle is shown"), Rect::new(290.0, 280.0, 390.0, 380.0)), "{o:?}");
    let center = Point::new(340.0, 330.0);
    let lines = o.iter().filter(|o| matches!(o, Overlay::Line { a, .. } if *a == center)).count();
    assert_eq!(lines, 2, "a radius to each point: {o:?}");
    assert!(o.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text.starts_with("D: 100"))), "{o:?}");
    let without_labels = ToolContext { measurement_labels: false, ..cx };
    assert!(!t.overlays(&without_labels).iter().any(|o| matches!(o, Overlay::Measure { .. })));
    assert_eq!(click(&mut t, &cx, (380.0, 360.0)), circle(&[(300.0, 300.0), (380.0, 360.0)]));
    assert!(!t.ends_on_escape(&cx));
}

/// A click, then a press on a second point dragged to a third, draws the circle through the three.
#[test]
fn a_click_and_a_drag_draw_the_circle_through_three_points() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    click(&mut t, &cx, (300.0, 300.0));
    t.pointer(&cx, &ev(PointerKind::Down, (380.0, 300.0), Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Drag, (300.0, 360.0), Mods::default()));
    // (300, 300), (380, 300) and (300, 360): the right angle stands on the diameter, center
    // (340, 330), radius 50.
    let o = t.overlays(&cx);
    assert!(near(preview(&o).expect("the circle is shown"), Rect::new(290.0, 280.0, 390.0, 380.0)), "{o:?}");
    assert_eq!(o.iter().filter(|o| matches!(o, Overlay::Anchor { .. })).count(), 2, "{o:?}");
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (300.0, 360.0), Mods::default())), circle(&[(300.0, 300.0), (380.0, 300.0), (300.0, 360.0)]));
}

/// The lines the tool shows from the center `c`: (radius lines, arms of the center cross), each
/// with its color.
fn annotations(o: &[Overlay], c: Point) -> (Vec<[u8; 3]>, Vec<[u8; 3]>) {
    let (mut radii, mut cross) = (vec![], vec![]);
    for o in o {
        if let Overlay::Line { a, b, color, .. } = o {
            if *a == c {
                radii.push(*color);
            } else if a.midpoint(*b).distance(c) < 1e-9 {
                cross.push(*color);
            }
        }
    }
    (radii, cross)
}

/// `center`, `radii` and `diameter` show or hide the center cross, the radius lines and the
/// diameter label, each on its own, and `color` is the color of the cross and the lines; the
/// outline stays in the feedback color. Each option lasts in [`Tool::options`].
#[test]
fn the_options_show_or_hide_the_center_radii_and_diameter_in_their_color() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    click(&mut t, &cx, (300.0, 300.0));
    t.pointer(&cx, &ev(PointerKind::Move, (380.0, 360.0), Mods::default()));
    let c = Point::new(340.0, 330.0);
    let blue = crate::draw2::FEEDBACK;
    assert_eq!(annotations(&t.overlays(&cx), c), (vec![blue; 2], vec![blue; 2]));
    let diameter = |t: &CircleTool| t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Measure { .. }));
    assert!(diameter(&t));
    t.set_option("center", &json!(false));
    assert_eq!(annotations(&t.overlays(&cx), c), (vec![blue; 2], vec![]));
    t.set_option("radii", &json!(false));
    assert_eq!(annotations(&t.overlays(&cx), c), (vec![], vec![]));
    assert!(diameter(&t));
    t.set_option("diameter", &json!(false));
    assert!(!diameter(&t));
    t.set_option("center", &json!(true));
    t.set_option("radii", &json!(true));
    t.set_option("color", &json!("red"));
    let red = [0xe5, 0x2d, 0x27];
    let o = t.overlays(&cx);
    assert_eq!(annotations(&o, c), (vec![red; 2], vec![red; 2]));
    assert!(o.iter().any(|o| matches!(o, Overlay::Path { color, .. } if *color == blue)), "{o:?}");
    assert_eq!(t.options(), json!({"center": true, "radii": true, "diameter": false, "color": "red", "standardAnchors": true, "dragThird": true}));
    // A value an option does not take leaves it as it was.
    for (key, value) in [("color", json!("orange")), ("center", json!("no")), ("radii", json!(0))] {
        t.set_option(key, &value);
    }
    assert_eq!(t.options()["color"], json!("red"));
    assert_eq!((&t.options()["center"], &t.options()["radii"]), (&json!(true), &json!(true)));
}

/// With `standardAnchors` off, the circle's anchors go at the points it was drawn through.
#[test]
fn anchors_at_the_points_asks_for_them() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    t.set_option("standardAnchors", &json!(false));
    let acts = drag(&mut t, &cx, (300.0, 300.0), &[(380.0, 360.0)], Mods::default());
    let points = json!([[300.0, 300.0], [380.0, 360.0]]);
    assert_eq!(acts, vec![Action::Exec("shape.circleThroughPoints".into(), json!({"points": points, "anchors": "points"}))]);
}

/// With `standardAnchors` off, the tool shows the path with an anchor at each point, the one its
/// release draws.
#[test]
fn anchors_at_the_points_show_the_path_they_draw() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    t.set_option("standardAnchors", &json!(false));
    click(&mut t, &cx, (300.0, 300.0));
    t.pointer(&cx, &ev(PointerKind::Down, (380.0, 300.0), Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Drag, (310.0, 340.0), Mods::default()));
    let points = [Point::new(300.0, 300.0), Point::new(380.0, 300.0), Point::new(310.0, 340.0)];
    let (c, r) = circle_of(&points).expect("a circle");
    let want = shapes::circle_with_anchors_at(c, r, &points).to_bezpath();
    let o = t.overlays(&cx);
    assert!(o.iter().any(|o| matches!(o, Overlay::Path { path, .. } if *path == want)), "{o:?}");
}

/// With `standardAnchors` off, a circle within 1 pt of the canvas edge whose path with anchors at
/// the points curves past that edge is not shown and not drawn; with standard anchors it is.
#[test]
fn anchors_at_the_points_keep_the_path_on_the_canvas() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let r = MAX_COORD - 1.0;
    let at = |deg: f64| (r * deg.to_radians().cos(), r * deg.to_radians().sin());
    let points = [at(0.0), at(70.0), at(180.0)];
    let geom: Vec<Point> = points.iter().map(|(x, y)| Point::new(*x, *y)).collect();
    assert!(drawn_circle(&geom, true).is_some());
    assert_eq!(drawn_circle(&geom, false), None);
    let mut t = CircleTool::default();
    t.set_option("standardAnchors", &json!(false));
    click(&mut t, &cx, points[0]);
    t.pointer(&cx, &ev(PointerKind::Down, points[1], Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Drag, points[2], Mods::default()));
    // No outline at all: at this size the tool would draw it solid, not dashed.
    let o = t.overlays(&cx);
    assert!(!o.iter().any(|o| matches!(o, Overlay::Path { .. })), "{o:?}");
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, points[2], Mods::default())), vec![]);
}

/// With `dragThird` off, a click in reach of both placed points is on the nearer one: nearer the
/// first point it places nothing, nearer the second it draws the circle on the two.
#[test]
fn with_third_point_by_dragging_off_a_click_near_both_points_takes_the_nearer() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    t.set_option("dragThird", &json!(false));
    click(&mut t, &cx, (100.0, 100.0));
    assert_eq!(click(&mut t, &cx, (104.0, 100.0)), vec![]);
    assert_eq!(click(&mut t, &cx, (101.0, 100.0)), vec![]);
    assert_eq!(click(&mut t, &cx, (103.0, 100.0)), circle(&[(100.0, 100.0), (104.0, 100.0)]));
}

/// With `dragThird` off, a press on the second point shows the circle on the two points, which its
/// release draws, and a press on the first point shows none.
#[test]
fn with_third_point_by_dragging_off_a_press_shows_the_circle_its_release_draws() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    t.set_option("dragThird", &json!(false));
    click(&mut t, &cx, (100.0, 100.0));
    click(&mut t, &cx, (200.0, 100.0));
    t.pointer(&cx, &ev(PointerKind::Down, (101.0, 101.0), Mods::default()));
    assert_eq!(preview(&t.overlays(&cx)), None);
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (101.0, 101.0), Mods::default())), vec![]);
    t.pointer(&cx, &ev(PointerKind::Down, (202.0, 101.0), Mods::default()));
    let o = t.overlays(&cx);
    assert!(near(preview(&o).expect("the circle is shown"), Rect::new(100.0, 50.0, 200.0, 150.0)), "{o:?}");
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (202.0, 101.0), Mods::default())), circle(&[(100.0, 100.0), (200.0, 100.0)]));
}

/// With `dragThird` off, a press that moves less than 2 screen points keeps the point where it was
/// pressed, for the second point and for the third.
#[test]
fn with_third_point_by_dragging_off_a_short_move_keeps_the_pressed_point() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let (down, drag, up) = (PointerKind::Down, PointerKind::Drag, PointerKind::Up);
    let mut t = CircleTool::default();
    t.set_option("dragThird", &json!(false));
    click(&mut t, &cx, (100.0, 100.0));
    t.pointer(&cx, &ev(down, (200.0, 100.0), Mods::default()));
    t.pointer(&cx, &ev(drag, (201.0, 100.0), Mods::default()));
    assert_eq!(t.pointer(&cx, &ev(up, (200.0, 100.0), Mods::default())), vec![]);
    t.pointer(&cx, &ev(down, (150.0, 200.0), Mods::default()));
    t.pointer(&cx, &ev(drag, (151.0, 200.0), Mods::default()));
    let three = circle(&[(100.0, 100.0), (200.0, 100.0), (150.0, 200.0)]);
    assert_eq!(t.pointer(&cx, &ev(up, (150.0, 200.0), Mods::default())), three);
}

/// With `dragThird` off, a second click places the second point and a third click draws the circle
/// through the three; a click on the second point again (as a double-click there does) draws the
/// circle on the two; a second point let go on the first places nothing; a drag from the first
/// point still draws the circle on its diameter.
#[test]
fn with_third_point_by_dragging_off_three_clicks_draw_the_circle() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    t.set_option("dragThird", &json!(false));
    click(&mut t, &cx, (300.0, 300.0));
    assert_eq!(click(&mut t, &cx, (380.0, 300.0)), vec![]);
    t.pointer(&cx, &ev(PointerKind::Move, (300.0, 360.0), Mods::default()));
    let o = t.overlays(&cx);
    assert!(near(preview(&o).expect("the circle through the three is shown"), Rect::new(290.0, 280.0, 390.0, 380.0)), "{o:?}");
    assert_eq!(o.iter().filter(|o| matches!(o, Overlay::Anchor { .. })).count(), 2, "{o:?}");
    // A click on the first point places nothing and keeps both points.
    assert_eq!(click(&mut t, &cx, (300.0, 300.0)), vec![]);
    assert!(t.busy());
    assert_eq!(click(&mut t, &cx, (300.0, 360.0)), circle(&[(300.0, 300.0), (380.0, 300.0), (300.0, 360.0)]));
    assert!(!t.busy());
    // The events of a double-click on the second point: its first click places it, the second
    // draws the circle on the two points, and the double-click event that follows adds nothing.
    click(&mut t, &cx, (100.0, 100.0));
    assert_eq!(click(&mut t, &cx, (200.0, 100.0)), vec![]);
    assert_eq!(click(&mut t, &cx, (200.0, 100.0)), circle(&[(100.0, 100.0), (200.0, 100.0)]));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::DoubleClick, (200.0, 100.0), Mods::default())), vec![]);
    assert!(!t.busy());
    // A drag that moves the second point places it where it is let go.
    click(&mut t, &cx, (100.0, 100.0));
    assert_eq!(drag(&mut t, &cx, (150.0, 100.0), &[(180.0, 100.0), (200.0, 100.0)], Mods::default()), vec![]);
    assert_eq!(click(&mut t, &cx, (100.0, 200.0)), circle(&[(100.0, 100.0), (200.0, 100.0), (100.0, 200.0)]));
    // Three points on one line keep the two placed: the next click elsewhere draws.
    click(&mut t, &cx, (100.0, 100.0));
    click(&mut t, &cx, (200.0, 100.0));
    assert_eq!(click(&mut t, &cx, (300.0, 100.0)), vec![]);
    assert_eq!(click(&mut t, &cx, (100.0, 200.0)), circle(&[(100.0, 100.0), (200.0, 100.0), (100.0, 200.0)]));
    // A second press elsewhere, dragged back and let go on the first point, places nothing: the
    // next click places the second point.
    click(&mut t, &cx, (100.0, 100.0));
    assert_eq!(drag(&mut t, &cx, (200.0, 100.0), &[(150.0, 100.0), (100.0, 100.0)], Mods::default()), vec![]);
    assert!(t.busy());
    assert_eq!(click(&mut t, &cx, (200.0, 100.0)), vec![]);
    assert_eq!(click(&mut t, &cx, (100.0, 200.0)), circle(&[(100.0, 100.0), (200.0, 100.0), (100.0, 200.0)]));
    // A drag from the first point.
    assert_eq!(drag(&mut t, &cx, (300.0, 300.0), &[(380.0, 360.0)], Mods::default()), circle(&[(300.0, 300.0), (380.0, 360.0)]));
    // Switching the option drops the points placed so far.
    click(&mut t, &cx, (100.0, 100.0));
    t.set_option("dragThird", &json!(true));
    assert!(!t.busy());
}

/// With `dragThird` off, a drag of the second or third point ends where it is let go, also when the
/// release is away from the last drag.
#[test]
fn with_third_point_by_dragging_off_a_drag_ends_where_it_is_let_go() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let (down, drag, up) = (PointerKind::Down, PointerKind::Drag, PointerKind::Up);
    let mut t = CircleTool::default();
    t.set_option("dragThird", &json!(false));
    click(&mut t, &cx, (300.0, 300.0));
    t.pointer(&cx, &ev(down, (350.0, 300.0), Mods::default()));
    t.pointer(&cx, &ev(drag, (380.0, 300.0), Mods::default()));
    assert_eq!(t.pointer(&cx, &ev(up, (400.0, 300.0), Mods::default())), vec![]);
    t.pointer(&cx, &ev(down, (350.0, 350.0), Mods::default()));
    t.pointer(&cx, &ev(drag, (320.0, 380.0), Mods::default()));
    assert_eq!(t.pointer(&cx, &ev(up, (300.0, 400.0), Mods::default())), circle(&[(300.0, 300.0), (400.0, 300.0), (300.0, 400.0)]));
}

/// With `dragThird` off, an Alt-click places the second point where Alt's rounding of the diameter
/// puts it (88 pt rounds to 90 pt at 100%), and the tool shows it there: the marker, the preview,
/// the circle on the two points and the circle through three all use it.
#[test]
fn an_alt_click_places_the_second_point_where_it_is_shown() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let alt = Mods { alt: true, ..Mods::default() };
    let mut t = CircleTool::default();
    t.set_option("dragThird", &json!(false));
    click(&mut t, &cx, (300.0, 300.0));
    t.pointer(&cx, &ev(PointerKind::Down, (388.0, 300.0), alt));
    t.pointer(&cx, &ev(PointerKind::Up, (388.0, 300.0), alt));
    t.pointer(&cx, &ev(PointerKind::Move, (300.0, 390.0), Mods::default()));
    let o = t.overlays(&cx);
    let second = Point::new(390.0, 300.0);
    assert!(o.iter().any(|o| matches!(o, Overlay::Anchor { p, .. } if p.distance(second) < 1e-9)), "{o:?}");
    // (300, 300), (390, 300) and (300, 390): center (345, 345), radius 45·√2.
    let r = 45.0 * 2f64.sqrt();
    assert!(near(preview(&o).unwrap(), Rect::new(345.0 - r, 345.0 - r, 345.0 + r, 345.0 + r)), "{o:?}");
    assert_eq!(click(&mut t, &cx, (300.0, 390.0)), circle(&[(300.0, 300.0), (390.0, 300.0), (300.0, 390.0)]));
    click(&mut t, &cx, (300.0, 300.0));
    t.pointer(&cx, &ev(PointerKind::Down, (388.0, 300.0), alt));
    t.pointer(&cx, &ev(PointerKind::Up, (388.0, 300.0), alt));
    assert_eq!(click(&mut t, &cx, (390.0, 300.0)), circle(&[(300.0, 300.0), (390.0, 300.0)]));
    // In inches the ticks are farther apart, and the rounding can move the point more than the
    // 3 screen points a click reaches: the second click of a double-click, where the first let go,
    // still draws the circle on the two points, at the rounded point.
    let inches = ToolContext { unit: vectorcraft_doc::Unit::Inches, ..cx };
    let d = 94.4;
    let rounded = vectorcraft_doc::Unit::Inches.snap_to_ruler_tick(d, 1.0);
    assert!((rounded - d).abs() > 3.0, "{rounded}");
    click(&mut t, &inches, (300.0, 300.0));
    t.pointer(&inches, &ev(PointerKind::Down, (300.0 + d, 300.0), alt));
    t.pointer(&inches, &ev(PointerKind::Up, (300.0 + d, 300.0), alt));
    assert_eq!(click(&mut t, &inches, (300.0 + d, 300.0)), circle(&[(300.0, 300.0), (300.0 + rounded, 300.0)]));
}

/// `tool.setOption`'s check takes the values the options take and nothing else.
#[test]
fn check_options_takes_only_the_values_the_options_take() {
    let ok = json!({"center": false, "radii": true, "diameter": false, "color": "gray", "standardAnchors": false, "dragThird": false});
    assert_eq!(check_options(ok.as_object().unwrap()), Ok(()));
    // The tool's own check, which `tool.setOption` also runs, takes the same values.
    let tool = CircleTool::default();
    assert_eq!(tool.check_options(ok.as_object().unwrap()), Ok(()));
    for (bad, error) in [
        (json!({"center": 1}), "`center` must be true or false"),
        (json!({"dragThird": null}), "`dragThird` must be true or false"),
        (json!({"color": "orange"}), "`color` must be one of red, blue, magenta, green, black, gray"),
        (json!({"colour": "red"}), "unknown Circle tool option `colour`"),
    ] {
        assert_eq!(check_options(bad.as_object().unwrap()), Err(error.to_string()), "{bad}");
        assert_eq!(tool.check_options(bad.as_object().unwrap()), Err(error.to_string()), "{bad}");
    }
}

/// A drag ends where it is let go, also when the release is away from the last drag: from a first
/// press, from a second press, and from a press on the first point. A release within 2 screen
/// points of a second press is still a click and keeps the point of the press.
#[test]
fn a_drag_ends_where_it_is_let_go() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let (down, drag, up) = (PointerKind::Down, PointerKind::Drag, PointerKind::Up);
    let mut t = CircleTool::default();
    t.pointer(&cx, &ev(down, (300.0, 300.0), Mods::default()));
    t.pointer(&cx, &ev(drag, (380.0, 300.0), Mods::default()));
    assert_eq!(t.pointer(&cx, &ev(up, (400.0, 300.0), Mods::default())), circle(&[(300.0, 300.0), (400.0, 300.0)]));
    click(&mut t, &cx, (300.0, 300.0));
    t.pointer(&cx, &ev(down, (380.0, 300.0), Mods::default()));
    t.pointer(&cx, &ev(drag, (300.0, 340.0), Mods::default()));
    assert_eq!(t.pointer(&cx, &ev(up, (300.0, 360.0), Mods::default())), circle(&[(300.0, 300.0), (380.0, 300.0), (300.0, 360.0)]));
    click(&mut t, &cx, (300.0, 300.0));
    t.pointer(&cx, &ev(down, (300.0, 300.0), Mods::default()));
    t.pointer(&cx, &ev(drag, (300.0, 380.0), Mods::default()));
    assert_eq!(t.pointer(&cx, &ev(up, (300.0, 400.0), Mods::default())), circle(&[(300.0, 300.0), (300.0, 400.0)]));
    click(&mut t, &cx, (300.0, 300.0));
    t.pointer(&cx, &ev(down, (380.0, 360.0), Mods::default()));
    assert_eq!(t.pointer(&cx, &ev(up, (381.0, 360.0), Mods::default())), circle(&[(300.0, 300.0), (380.0, 360.0)]));
}

/// A release within 2 screen points of the press is a click also after a drag that came back to
/// it: a second press draws the circle on the diameter to the press, a press on the first point
/// places nothing, and a first press places the first point.
#[test]
fn a_release_near_the_press_is_a_click_after_a_drag() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    click(&mut t, &cx, (300.0, 300.0));
    assert_eq!(drag(&mut t, &cx, (380.0, 300.0), &[(300.0, 360.0), (381.0, 300.0)], Mods::default()), circle(&[(300.0, 300.0), (380.0, 300.0)]));
    click(&mut t, &cx, (300.0, 300.0));
    assert_eq!(drag(&mut t, &cx, (300.0, 300.0), &[(300.0, 380.0), (301.0, 300.0)], Mods::default()), vec![]);
    assert!(t.ends_on_escape(&cx), "the first point stays");
    assert_eq!(click(&mut t, &cx, (380.0, 360.0)), circle(&[(300.0, 300.0), (380.0, 360.0)]));
    assert_eq!(drag(&mut t, &cx, (300.0, 300.0), &[(380.0, 300.0), (301.0, 300.0)], Mods::default()), vec![]);
    assert!(t.ends_on_escape(&cx), "the press placed the first point");
    assert_eq!(click(&mut t, &cx, (380.0, 360.0)), circle(&[(300.0, 300.0), (380.0, 360.0)]));
    // Let go near the press straight from far away, the tool shows no circle to the last drag.
    t.pointer(&cx, &ev(PointerKind::Down, (300.0, 300.0), Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Drag, (380.0, 300.0), Mods::default()));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (301.0, 300.0), Mods::default())), vec![]);
    assert_eq!(preview(&t.overlays(&cx)), None);
    t.pointer(&cx, &ev(PointerKind::Down, (300.0, 300.0), Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Drag, (300.0, 380.0), Mods::default()));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (301.0, 300.0), Mods::default())), vec![]);
    assert_eq!(preview(&t.overlays(&cx)), None);
    assert!(t.ends_on_escape(&cx));
}

/// A click within 3 screen points of the first point, also one dragged away and let go where it
/// was pressed, places nothing and leaves no circle shown.
#[test]
fn a_click_near_the_first_point_shows_no_circle() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    click(&mut t, &cx, (300.0, 300.0));
    assert_eq!(drag(&mut t, &cx, (302.0, 300.0), &[(302.0, 380.0), (302.0, 300.0)], Mods::default()), vec![]);
    assert_eq!(preview(&t.overlays(&cx)), None);
    assert_eq!(click(&mut t, &cx, (302.0, 300.0)), vec![]);
    assert_eq!(preview(&t.overlays(&cx)), None);
    assert!(t.ends_on_escape(&cx), "the first point stays");
    assert_eq!(click(&mut t, &cx, (380.0, 360.0)), circle(&[(300.0, 300.0), (380.0, 360.0)]));
}

/// A press on the first point, dragged to snap to the rectangle's corner anchor and let go near
/// the press, leaves no guides of the drag.
#[test]
fn a_release_near_the_press_hides_the_guides_of_the_drag() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut t = CircleTool::default();
    click(&mut t, &cx, (300.0, 300.0));
    t.pointer(&cx, &ev(PointerKind::Down, (300.0, 300.0), Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Drag, (201.0, 201.0), Mods::default()));
    assert!(!t.snap.guides().is_empty(), "the drag snapped to the corner anchor");
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (301.0, 300.0), Mods::default())), vec![]);
    assert!(t.snap.guides().is_empty(), "{:?}", t.snap.guides());
}

/// Three points on one line show no circle, and the release draws nothing and starts afresh. A
/// circle past the canvas is not shown either.
#[test]
fn points_on_one_line_show_no_circle() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    click(&mut t, &cx, (100.0, 100.0));
    t.pointer(&cx, &ev(PointerKind::Down, (200.0, 200.0), Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Drag, (300.0, 300.0), Mods::default()));
    assert_eq!(preview(&t.overlays(&cx)), None);
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (300.0, 300.0), Mods::default())), vec![]);
    assert!(!t.ends_on_escape(&cx) && t.overlays(&cx).is_empty());
    assert_eq!(circle_of(&[Point::new(0.0, 0.0), Point::new(4.1e6, 0.0)]), None);
    assert_eq!(circle_of(&[Point::new(0.0, 0.0), Point::new(200.0, 0.0), Point::new(100.0, 0.001)]), None);
}

/// Shift keeps the second point at 45° steps from the first.
#[test]
fn shift_keeps_the_diameter_at_45_degree_steps() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let shift = Mods { shift: true, ..Mods::default() };
    let mut t = CircleTool::default();
    let acts = drag(&mut t, &cx, (300.0, 300.0), &[(400.0, 310.0)], shift);
    let [Action::Exec(_, v)] = acts.as_slice() else { panic!("one circle: {acts:?}") };
    let end = &v["points"][1];
    assert!((end[1].as_f64().unwrap() - 300.0).abs() < 1e-9, "level with the first point: {v}");
}

/// Alt rounds the diameter to the nearest ruler tick at the current zoom: every 5 pt at 100% and
/// every 2.5 pt at 200% in points, keeping the direction from the first point; for a drag, for a
/// second click and in the preview.
#[test]
fn alt_rounds_the_diameter_to_ruler_ticks() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let alt = Mods { alt: true, ..Mods::default() };
    // The second point of the one circle `acts` draws.
    let second = |acts: Vec<Action>| {
        let [Action::Exec(_, v)] = acts.as_slice() else { panic!("one circle: {acts:?}") };
        Point::new(v["points"][1][0].as_f64().unwrap(), v["points"][1][1].as_f64().unwrap())
    };
    let mut t = CircleTool::default();
    let end = second(drag(&mut t, &cx, (300.0, 300.0), &[(373.0, 300.0)], alt));
    assert!(end.distance(Point::new(375.0, 300.0)) < 1e-9, "{end:?}");
    // (31.2, 41.6) is 52 from the origin, and Alt makes it 50: (30, 40).
    click(&mut t, &cx, (0.0, 0.0));
    t.pointer(&cx, &ev(PointerKind::Move, (31.2, 41.6), alt));
    assert!(near(preview(&t.overlays(&cx)).expect("the circle is shown"), Rect::new(-10.0, -5.0, 40.0, 45.0)), "{:?}", t.overlays(&cx));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, (31.2, 41.6), alt)), vec![]);
    let end = second(t.pointer(&cx, &ev(PointerKind::Up, (31.2, 41.6), alt)));
    assert!(end.distance(Point::new(30.0, 40.0)) < 1e-9, "{end:?}");
    let zoomed = ToolContext { zoom: 2.0, ..cx };
    for (to, want) in [(51.0, 50.0), (51.5, 52.5)] {
        let end = second(drag(&mut t, &zoomed, (0.0, 0.0), &[(0.0, to)], alt));
        assert!(end.distance(Point::new(0.0, want)) < 1e-9, "{to}: {end:?}");
    }
    // Without Alt the pointer is used as it is.
    assert_eq!(drag(&mut t, &cx, (300.0, 300.0), &[(373.0, 300.0)], Mods::default()), circle(&[(300.0, 300.0), (373.0, 300.0)]));
}

/// Alt rounds only a diameter: a second press dragged to a third point keeps the second point
/// where it was pressed, and a second press 2.6 pt from the first point, which Alt's rounding
/// would move to 5 pt, is still a press on the first point and places nothing.
#[test]
fn alt_leaves_three_points_and_presses_on_the_first_point_alone() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let alt = Mods { alt: true, ..Mods::default() };
    let mut t = CircleTool::default();
    click(&mut t, &cx, (0.0, 0.0));
    assert_eq!(drag(&mut t, &cx, (31.2, 41.6), &[(45.0, 20.0), (60.0, 0.0)], alt), circle(&[(0.0, 0.0), (31.2, 41.6), (60.0, 0.0)]));
    click(&mut t, &cx, (300.0, 300.0));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, (302.6, 300.0), alt)), vec![]);
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (302.6, 300.0), alt)), vec![]);
    assert!(t.ends_on_escape(&cx), "the first point stays");
}

/// Three different points on one line show "Circle too large" by the pointer instead of a circle;
/// a pointer back on the first point does not.
#[test]
fn points_on_one_line_say_the_circle_is_too_large() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let label = |t: &CircleTool| {
        t.overlays(&cx).iter().find_map(|o| match o {
            Overlay::Label { p, text, .. } => Some((*p, text.clone())),
            _ => None,
        })
    };
    let mut t = CircleTool::default();
    click(&mut t, &cx, (100.0, 100.0));
    t.pointer(&cx, &ev(PointerKind::Down, (200.0, 200.0), Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Drag, (300.0, 300.0), Mods::default()));
    assert_eq!(label(&t), Some((Point::new(300.0, 300.0), "Circle too large".to_string())));
    t.pointer(&cx, &ev(PointerKind::Drag, (300.0, 290.0), Mods::default()));
    assert_eq!(label(&t), None, "a circle is shown");
    t.pointer(&cx, &ev(PointerKind::Drag, (200.0, 200.0), Mods::default()));
    assert_eq!(label(&t), None, "the third point is on the second");
    assert!(too_large(&[Point::new(0.0, 0.0), Point::new(4.1e6, 0.0)], true));
    assert!(!too_large(&[Point::new(0.0, 0.0)], true) && !too_large(&[Point::new(1.0, 0.0), Point::new(1.0, 0.0)], true));
}

/// Escape drops the first point; so do a dropped press and an edit made outside the tool. A
/// second click on the first point places nothing.
#[test]
fn escape_and_outside_edits_drop_the_first_point() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    click(&mut t, &cx, (300.0, 300.0));
    assert_eq!(click(&mut t, &cx, (300.5, 300.0)), vec![], "a double-click places one point");
    assert!(t.ends_on_escape(&cx));
    t.key(&cx, ToolKey::Escape, Mods::default());
    assert!(!t.ends_on_escape(&cx) && t.overlays(&cx).is_empty());
    click(&mut t, &cx, (300.0, 300.0));
    t.history_moved();
    assert!(!t.ends_on_escape(&cx));
    click(&mut t, &cx, (300.0, 300.0));
    t.pointer(&cx, &ev(PointerKind::Down, (380.0, 300.0), Mods::default()));
    t.drop_press(&cx);
    assert!(!t.ends_on_escape(&cx));
    // After all of that, two clicks start a new circle.
    click(&mut t, &cx, (10.0, 10.0));
    assert_eq!(click(&mut t, &cx, (30.0, 10.0)), circle(&[(10.0, 10.0), (30.0, 10.0)]));
}

/// The points snap to Smart Guides: a press near the rectangle's corner anchor at (100, 100) places
/// the point on it.
#[test]
fn points_snap_to_smart_guides() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut t = CircleTool::default();
    t.pointer(&cx, &ev(PointerKind::Move, (101.0, 101.5), Mods::default()));
    click(&mut t, &cx, (101.0, 101.5));
    assert_eq!(click(&mut t, &cx, (300.0, 100.0)), circle(&[(100.0, 100.0), (300.0, 100.0)]));
}

/// A click is measured from where the pointer was pressed, not from the point it snapped to: a
/// second press pulled 1.8 pt onto the corner anchor at (100, 100) and moved 0.5 pt before the
/// release still draws the circle on that diameter.
#[test]
fn a_snapped_second_click_is_not_a_drag() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut t = CircleTool::default();
    click(&mut t, &cx, (300.0, 100.0));
    t.pointer(&cx, &ev(PointerKind::Down, (101.0, 101.5), Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Drag, (101.5, 101.5), Mods::default()));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (101.5, 101.5), Mods::default())), circle(&[(300.0, 100.0), (100.0, 100.0)]));
}

/// A second press within 3 screen points of the first point is a press on it: dragged out, it
/// draws the circle on the diameter it drags out, whether it landed on the first point or next to
/// it; a press farther away and dragged places the second and third points.
#[test]
fn a_drag_from_the_first_point_draws_the_circle_on_its_diameter() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    click(&mut t, &cx, (300.0, 300.0));
    let acts = drag(&mut t, &cx, (301.0, 300.0), &[(300.0, 350.0), (300.0, 400.0)], Mods::default());
    assert_eq!(acts, circle(&[(300.0, 300.0), (300.0, 400.0)]));
    click(&mut t, &cx, (300.0, 300.0));
    let acts = drag(&mut t, &cx, (304.0, 300.0), &[(300.0, 350.0), (300.0, 400.0)], Mods::default());
    assert_eq!(acts, circle(&[(300.0, 300.0), (304.0, 300.0), (300.0, 400.0)]));
    click(&mut t, &cx, (300.0, 300.0));
    t.pointer(&cx, &ev(PointerKind::Down, (300.0, 300.0), Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Drag, (300.0, 400.0), Mods::default()));
    assert!(near(preview(&t.overlays(&cx)).expect("the circle is shown"), Rect::new(250.0, 300.0, 350.0, 400.0)), "{:?}", t.overlays(&cx));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (300.0, 400.0), Mods::default())), circle(&[(300.0, 300.0), (300.0, 400.0)]));
}

/// A second press farther than 3 screen points from the first point that snaps to it is a press on
/// it too: the first point is the rectangle's corner anchor at (100, 100), and a press 3.2 pt away
/// snaps back to that anchor.
#[test]
fn a_second_press_that_snaps_to_the_first_point_is_a_press_on_it() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut t = CircleTool::default();
    click(&mut t, &cx, (100.0, 100.0));
    t.pointer(&cx, &ev(PointerKind::Down, (103.2, 100.0), Mods::default()));
    assert_eq!(t.second(), None, "the press snapped to the first point: {:?}", t.press);
    t.pointer(&cx, &ev(PointerKind::Drag, (100.0, 150.0), Mods::default()));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (100.0, 150.0), Mods::default())), circle(&[(100.0, 100.0), (100.0, 150.0)]));
}

/// A circle more than 50,000 screen points around is previewed solid, not dashed: three points
/// nearly on one line give a radius of about 1,250,000 pt.
#[test]
fn a_huge_circle_is_previewed_solid() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CircleTool::default();
    click(&mut t, &cx, (0.0, 0.0));
    t.pointer(&cx, &ev(PointerKind::Down, (1000.0, 0.0), Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Drag, (500.0, 0.1), Mods::default()));
    let o = t.overlays(&cx);
    assert_eq!(preview(&o), None, "{o:?}");
    assert!(o.iter().any(|o| matches!(o, Overlay::Path { dashed: false, .. })), "{o:?}");
    // At 100% a circle 100 pt across is dashed.
    t.pointer(&cx, &ev(PointerKind::Drag, (500.0, 300.0), Mods::default()));
    assert!(preview(&t.overlays(&cx)).is_some());
}

/// The tool is in the Ellipse tool's group, after the Ellipse tool.
#[test]
fn the_circle_tool_follows_the_ellipse_tool() {
    let tool = crate::create(ID);
    assert_eq!(tool.id(), ID);
    let group = crate::catalog::group_of(ID).unwrap();
    assert_eq!(crate::catalog::group_of("ellipse"), Some(group));
    let ids: Vec<&str> = crate::TOOL_GROUPS[group].iter().map(|t| t.id).collect();
    let ellipse = ids.iter().position(|id| *id == "ellipse").unwrap();
    assert_eq!(ids.get(ellipse + 1), Some(&ID));
}
