use super::*;
use crate::testutil::*;
use vectorcraft_doc::Selection;
use vectorcraft_geom::Rect;

fn ev(kind: PointerKind, (x, y): (f64, f64), mods: Mods) -> PointerEvent {
    PointerEvent::new(kind, x, y).with_mods(mods)
}

fn click_with(t: &mut ThreePointArcTool, cx: &ToolContext, p: (f64, f64), mods: Mods) -> Vec<Action> {
    let mut acts = t.pointer(cx, &ev(PointerKind::Down, p, mods));
    acts.extend(t.pointer(cx, &ev(PointerKind::Up, p, mods)));
    acts
}

fn click(t: &mut ThreePointArcTool, cx: &ToolContext, p: (f64, f64)) -> Vec<Action> {
    click_with(t, cx, p, Mods::default())
}

fn hover(t: &mut ThreePointArcTool, cx: &ToolContext, p: (f64, f64), mods: Mods) {
    assert!(t.pointer(cx, &ev(PointerKind::Move, p, mods)).is_empty());
}

/// A press at `from`, dragged through `path` and let go at its last point.
fn drag(t: &mut ThreePointArcTool, cx: &ToolContext, from: (f64, f64), path: &[(f64, f64)], mods: Mods) -> Vec<Action> {
    let mut acts = t.pointer(cx, &ev(PointerKind::Down, from, mods));
    for p in path {
        acts.extend(t.pointer(cx, &ev(PointerKind::Drag, *p, mods)));
    }
    let end = path.last().copied().unwrap_or(from);
    acts.extend(t.pointer(cx, &ev(PointerKind::Up, end, mods)));
    acts
}

fn through(start: (f64, f64), end: (f64, f64), via: (f64, f64)) -> Vec<Action> {
    let pair = |(x, y): (f64, f64)| json!([x, y]);
    vec![Action::Exec("shape.threePointArc".into(), json!({ "start": pair(start), "end": pair(end), "through": pair(via) }))]
}

fn toward(start: (f64, f64), end: (f64, f64), to: (f64, f64), at: &str) -> Vec<Action> {
    let pair = |(x, y): (f64, f64)| json!([x, y]);
    vec![Action::Exec("shape.threePointArc".into(), json!({ "start": pair(start), "end": pair(end), "toward": pair(to), "at": at }))]
}

/// The dashed arc the tool shows: its bounding box.
fn preview(o: &[Overlay]) -> Option<Rect> {
    o.iter().find_map(|o| match o {
        Overlay::Path { path, dashed: true, .. } => Some(vectorcraft_geom::Shape::bounding_box(path)),
        _ => None,
    })
}

fn too_large(o: &[Overlay]) -> bool {
    o.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == TOO_LARGE))
}

/// A drag places the two ends; a click on a third point then draws the arc through it. Until
/// then, nothing is drawn.
#[test]
fn a_drag_and_a_click_draw_the_arc_through_the_third_point() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ThreePointArcTool::default();
    assert!(drag(&mut t, &cx, (300.0, 300.0), &[(340.0, 300.0), (380.0, 300.0)], Mods::default()).is_empty());
    assert!(t.busy() && t.ends_on_escape(&cx));
    assert_eq!(click(&mut t, &cx, (340.0, 260.0)), through((300.0, 300.0), (380.0, 300.0), (340.0, 260.0)));
    assert!(!t.busy() && t.overlays(&cx).is_empty(), "the tool starts afresh");
}

/// Two clicks place the ends. Between the second click and the third, the tool shows the arc
/// through the pointer (a half circle here, bulging up the page), its center cross and its
/// radius; the third click draws it.
#[test]
fn three_clicks_draw_the_arc_and_the_tool_shows_it_on_the_way() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ThreePointArcTool::default();
    assert!(click(&mut t, &cx, (300.0, 300.0)).is_empty());
    hover(&mut t, &cx, (380.0, 300.0), Mods::default());
    assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Line { dashed: true, .. })), "the chord so far");
    assert!(click(&mut t, &cx, (380.0, 300.0)).is_empty());
    hover(&mut t, &cx, (340.0, 260.0), Mods::default());
    let o = t.overlays(&cx);
    let b = preview(&o).expect("the arc");
    assert!((b.x0 - 300.0).abs() < 0.1 && (b.x1 - 380.0).abs() < 0.1 && (b.y0 - 260.0).abs() < 0.1 && (b.y1 - 300.0).abs() < 1e-9, "{b:?}");
    let radius = format!("R: {}", cx.len(40.0));
    assert!(o.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if *text == radius)), "{o:?}");
    // The center cross at (340, 300).
    assert!(o.iter().any(|o| matches!(o, Overlay::Line { a, b, dashed: false, .. } if a.midpoint(*b).distance(Point::new(340.0, 300.0)) < 1e-9)));
    let quiet = ToolContext { measurement_labels: false, ..cx };
    assert!(!t.overlays(&quiet).iter().any(|o| matches!(o, Overlay::Measure { .. })));
    assert_eq!(click(&mut t, &cx, (340.0, 260.0)), through((300.0, 300.0), (380.0, 300.0), (340.0, 260.0)));
}

/// A third point on the line through the ends, an arc reaching past the canvas, or a third point
/// on an end gives no arc: a click draws nothing and keeps the ends, and a click on a point that
/// fits then draws the arc. The tool shows "Arc too large" by the pointer, except on an end.
#[test]
fn a_third_point_in_line_with_the_ends_draws_nothing() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ThreePointArcTool::default();
    drag(&mut t, &cx, (300.0, 300.0), &[(380.0, 300.0)], Mods::default());
    // Nearly in line beyond the end: the circle's center is about 1.2·10⁷ pt away.
    for (bad, label) in [((420.0, 300.0), true), ((250.0, 300.0), true), ((340.0, 300.0), true), ((500.0, 300.001), true), ((380.0, 300.0), false)] {
        hover(&mut t, &cx, bad, Mods::default());
        let o = t.overlays(&cx);
        assert!(too_large(&o) == label && preview(&o).is_none(), "{bad:?}: {o:?}");
        assert!(click(&mut t, &cx, bad).is_empty(), "{bad:?}");
        assert!(t.busy());
    }
    assert_eq!(click(&mut t, &cx, (340.0, 340.0)), through((300.0, 300.0), (380.0, 300.0), (340.0, 340.0)));
}

/// An arc near the canvas edge fits while its curve stays inside, even where a handle reaches past
/// the edge, and gives no arc once the curve reaches past it.
#[test]
fn an_arc_near_the_canvas_edge_is_measured_by_its_curve() {
    // A 60° arc of radius 1000 whose rightmost point is `edge`: the handles at its ends reach about
    // 45 pt further right.
    let arc = |edge: f64| {
        let (r, c) = (1000.0, edge - 1000.0);
        let (dx, dy) = (r * 30f64.to_radians().cos(), r * 30f64.to_radians().sin());
        let start = Point::new(c + dx, -dy);
        let end = Point::new(c + dx, dy);
        arc_of(start, end, start + Vec2::new(0.5, 3f64.sqrt() / 2.0) * 100.0, true, false)
    };
    let Some((_, path)) = arc(MAX_COORD - 1.0) else { panic!("an arc inside the canvas fits") };
    assert!(path.control_bounds().is_some_and(|r| r.x1 > MAX_COORD), "a handle reaches past the edge");
    assert!(path.bounds().is_some_and(|r| r.x1 <= MAX_COORD && (r.x1 - (MAX_COORD - 1.0)).abs() < 1e-6));
    assert!(arc(MAX_COORD + 1.0).is_none(), "an arc reaching past the canvas does not fit");
}

/// In direction mode, Shift keeps the direction line at 45° steps from the end it starts at.
#[test]
fn shift_keeps_the_direction_at_45_degree_steps() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ThreePointArcTool::default();
    t.set_option("direction", &json!(true));
    drag(&mut t, &cx, (300.0, 300.0), &[(380.0, 300.0)], Mods::default());
    let shift = Mods { shift: true, ..Mods::default() };
    let acts = click_with(&mut t, &cx, (303.0, 250.0), shift);
    let [Action::Exec(_, params)] = acts.as_slice() else { panic!("{acts:?}") };
    assert!(params["toward"][0].as_f64().is_some_and(|x| (x - 300.0).abs() < 1e-9), "{params}");
    // With Alt, from the second end.
    drag(&mut t, &cx, (300.0, 300.0), &[(380.0, 300.0)], Mods::default());
    let both = Mods { shift: true, alt: true, ..Mods::default() };
    let acts = click_with(&mut t, &cx, (383.0, 250.0), both);
    let [Action::Exec(_, params)] = acts.as_slice() else { panic!("{acts:?}") };
    assert!(params["toward"][0].as_f64().is_some_and(|x| (x - 380.0).abs() < 1e-9) && params["at"] == "end", "{params}");
}

/// With the `direction` option, the third point gives the direction the arc leaves its first
/// end in, and the tool shows the line from that end to the pointer. Alt moves the direction to
/// the second end. `]` switches the option while the ends are placed, and only then.
#[test]
fn in_direction_mode_the_third_point_sets_the_tangent_at_an_end() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ThreePointArcTool::default();
    assert!(!t.claims_key(&cx, ToolKey::BracketRight), "no ends: ] is left to the app");
    t.key(&cx, ToolKey::BracketRight, Mods::default());
    assert_eq!(t.options()["direction"], json!(false));
    drag(&mut t, &cx, (300.0, 300.0), &[(380.0, 300.0)], Mods::default());
    assert!(t.claims_key(&cx, ToolKey::BracketRight));
    t.key(&cx, ToolKey::BracketRight, Mods::default());
    assert_eq!(t.options()["direction"], json!(true));
    // Straight up the page from (300, 300): the upper half circle about (340, 300).
    hover(&mut t, &cx, (300.0, 250.0), Mods::default());
    let o = t.overlays(&cx);
    let b = preview(&o).expect("the arc");
    assert!((b.y0 - 260.0).abs() < 0.1 && (b.y1 - 300.0).abs() < 1e-9, "{b:?}");
    // The direction line: solid, from the end it starts at.
    let line = |o: &[Overlay], from: Point| o.iter().any(|o| matches!(o, Overlay::Line { a, dashed: false, .. } if *a == from));
    assert!(line(&o, Point::new(300.0, 300.0)), "{o:?}");
    assert_eq!(click(&mut t, &cx, (300.0, 250.0)), toward((300.0, 300.0), (380.0, 300.0), (300.0, 250.0), "start"));
    // Alt: the direction line starts at the second end.
    t.set_option("direction", &json!(true));
    drag(&mut t, &cx, (300.0, 300.0), &[(380.0, 300.0)], Mods::default());
    let alt = Mods { alt: true, ..Mods::default() };
    hover(&mut t, &cx, (380.0, 250.0), alt);
    assert!(line(&t.overlays(&cx), Point::new(380.0, 300.0)));
    assert!(preview(&t.overlays(&cx)).is_some());
    assert_eq!(click_with(&mut t, &cx, (380.0, 250.0), alt), toward((300.0, 300.0), (380.0, 300.0), (380.0, 250.0), "end"));
    // ] switches back to passing through the third point.
    drag(&mut t, &cx, (300.0, 300.0), &[(380.0, 300.0)], Mods::default());
    t.key(&cx, ToolKey::BracketRight, Mods::default());
    assert_eq!(click(&mut t, &cx, (340.0, 260.0)), through((300.0, 300.0), (380.0, 300.0), (340.0, 260.0)));
}

/// The `direction` option takes a bool; anything else leaves it as it was.
#[test]
fn the_direction_option_takes_a_bool() {
    let mut t = ThreePointArcTool::default();
    t.set_option("direction", &json!(true));
    for v in [json!("false"), json!(0), json!(null)] {
        t.set_option("direction", &v);
        assert_eq!(t.options()["direction"], json!(true), "{v}");
    }
    t.set_option("direction", &json!(false));
    assert_eq!(t.options(), json!({ "direction": false, "join": false, "tangentSnap": true }));
}

/// `]` pressed while the third point is held down places the point as the new mode keeps it: the
/// point Shift kept on the direction line goes back to where the pointer is, and the other way.
#[test]
fn bracket_during_a_press_places_the_third_point_in_the_new_mode() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ThreePointArcTool::default();
    let shift = Mods { shift: true, ..Mods::default() };
    t.set_option("direction", &json!(true));
    drag(&mut t, &cx, (300.0, 300.0), &[(380.0, 300.0)], Mods::default());
    t.pointer(&cx, &ev(PointerKind::Down, (303.0, 250.0), shift));
    t.key(&cx, ToolKey::BracketRight, Mods::default());
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (303.0, 250.0), shift)), through((300.0, 300.0), (380.0, 300.0), (303.0, 250.0)));
    // Through mode first, switched to direction mode during the press: Shift keeps the line upright.
    drag(&mut t, &cx, (300.0, 300.0), &[(380.0, 300.0)], Mods::default());
    t.pointer(&cx, &ev(PointerKind::Down, (303.0, 250.0), shift));
    t.key(&cx, ToolKey::BracketRight, Mods::default());
    let acts = t.pointer(&cx, &ev(PointerKind::Up, (303.0, 250.0), shift));
    let [Action::Exec(_, params)] = acts.as_slice() else { panic!("{acts:?}") };
    assert!(params["toward"][0].as_f64().is_some_and(|x| (x - 300.0).abs() < 1e-9) && params["at"] == "start", "{params}");
}

/// `]` pressed while the pointer hovers moves the third point the tool shows to where the new mode
/// keeps it, so the arc shown is the arc a click there draws.
#[test]
fn bracket_while_hovering_shows_the_third_point_in_the_new_mode() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ThreePointArcTool::default();
    let shift = Mods { shift: true, ..Mods::default() };
    drag(&mut t, &cx, (300.0, 300.0), &[(380.0, 300.0)], Mods::default());
    hover(&mut t, &cx, (303.0, 250.0), shift);
    t.key(&cx, ToolKey::BracketRight, Mods::default());
    // The direction line from the first end: Shift keeps it upright.
    let start = Point::new(300.0, 300.0);
    let line = t.overlays(&cx).into_iter().find_map(|o| match o {
        Overlay::Line { a, b, dashed: false, .. } if a == start => Some(b),
        _ => None,
    });
    assert!(line.is_some_and(|b| (b.x - 300.0).abs() < 1e-9), "{line:?}");
    let acts = click_with(&mut t, &cx, (303.0, 250.0), shift);
    let [Action::Exec(_, params)] = acts.as_slice() else { panic!("{acts:?}") };
    assert!(params["toward"][0].as_f64().is_some_and(|x| (x - 300.0).abs() < 1e-9), "{params}");
    // Back to through mode while hovering: the point goes back to the pointer.
    drag(&mut t, &cx, (300.0, 300.0), &[(380.0, 300.0)], Mods::default());
    hover(&mut t, &cx, (303.0, 250.0), shift);
    assert!(t.pointer.is_some_and(|q| (q.x - 300.0).abs() < 1e-9), "{:?}", t.pointer);
    t.key(&cx, ToolKey::BracketRight, Mods::default());
    assert_eq!(t.pointer, Some(Point::new(303.0, 250.0)));
    assert_eq!(click_with(&mut t, &cx, (303.0, 250.0), shift), through((300.0, 300.0), (380.0, 300.0), (303.0, 250.0)));
}

/// With Snap to Point, a first press that snaps to the rectangle's corner, 1.5 pt from the
/// pointer, and moves 1.9 pt before it is let go is a click: the drag is measured from the press,
/// not from the corner, so only the first end is placed.
#[test]
fn a_short_move_from_a_snapped_press_is_not_a_drag() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, snap_to_point: true, ..cx(&d, &s, &p) };
    let mut t = ThreePointArcTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, (201.5, 200.0), Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Drag, (203.4, 200.0), Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Up, (203.4, 200.0), Mods::default()));
    assert_eq!((t.start, t.end), (Some(Point::new(200.0, 200.0)), None));
}

/// A release away from the last drag event places the point where the pointer is let go, for
/// the second end and for the third point.
#[test]
fn a_point_is_placed_where_the_press_is_let_go() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ThreePointArcTool::default();
    let none = Mods::default();
    t.pointer(&cx, &ev(PointerKind::Down, (300.0, 300.0), none));
    t.pointer(&cx, &ev(PointerKind::Drag, (340.0, 300.0), none));
    t.pointer(&cx, &ev(PointerKind::Up, (380.0, 300.0), none));
    assert_eq!(t.end, Some(Point::new(380.0, 300.0)));
    t.pointer(&cx, &ev(PointerKind::Down, (340.0, 250.0), none));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (340.0, 260.0), none)), through((300.0, 300.0), (380.0, 300.0), (340.0, 260.0)));
    // A press that only moves when it is let go still counts as a drag for the ends.
    t.pointer(&cx, &ev(PointerKind::Down, (300.0, 300.0), none));
    t.pointer(&cx, &ev(PointerKind::Up, (380.0, 300.0), none));
    assert_eq!(t.end, Some(Point::new(380.0, 300.0)));
}

/// A release with Shift or Alt changed since the press places the point as the keys at the
/// release keep it, even where the pointer did not move.
#[test]
fn a_release_places_the_point_with_the_keys_held_then() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ThreePointArcTool::default();
    let (none, shift) = (Mods::default(), Mods { shift: true, ..Mods::default() });
    // The second end: pressed without Shift, let go with it.
    click(&mut t, &cx, (300.0, 300.0));
    t.pointer(&cx, &ev(PointerKind::Down, (380.0, 306.0), none));
    t.pointer(&cx, &ev(PointerKind::Up, (380.0, 306.0), shift));
    assert_eq!(t.end.map(|e| e.y), Some(300.0), "{:?}", t.end);
    // The third point in direction mode: pressed with Shift, let go with Shift and Alt, so the
    // direction line keeps to 45° steps from the second end.
    t.set_option("direction", &json!(true));
    let both = Mods { alt: true, ..shift };
    t.pointer(&cx, &ev(PointerKind::Down, (303.0, 250.0), shift));
    let acts = t.pointer(&cx, &ev(PointerKind::Up, (303.0, 250.0), both));
    let [Action::Exec(_, params)] = acts.as_slice() else { panic!("{acts:?}") };
    let at = |key: &str, i: usize| params[key][i].as_f64().unwrap_or(f64::NAN);
    let (dx, dy) = (at("toward", 0) - at("end", 0), at("toward", 1) - at("end", 1));
    assert!(params["at"] == "end" && (dx.abs() - dy.abs()).abs() < 1e-9, "{params}");
}

/// A first click that snaps to the rectangle's corner, 3 pt from the pointer, and is let go with
/// Shift changed places only the first end: a change of keys alone is not a drag.
#[test]
fn a_click_let_go_with_shift_changed_is_not_a_drag() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let g = Point::new(200.0, 200.0);
    let off = (203.0, 201.0);
    let mut t = ThreePointArcTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, off, Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Up, off, Mods { shift: true, ..Mods::default() }));
    assert_eq!((t.start, t.end), (Some(g), None));
}

/// Shift keeps the second end at 45° steps from the first.
#[test]
fn shift_keeps_the_second_end_at_45_degree_steps() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ThreePointArcTool::default();
    let shift = Mods { shift: true, ..Mods::default() };
    drag(&mut t, &cx, (300.0, 300.0), &[(340.0, 303.0), (380.0, 306.0)], shift);
    assert_eq!(t.end.map(|e| e.y), Some(300.0), "{:?}", t.end);
}

/// Escape, an edit made outside the tool and a dropped press drop the ends; a second click on
/// the first end (a double-click) places nothing.
#[test]
fn escape_history_and_a_double_click_drop_or_keep_the_ends() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ThreePointArcTool::default();
    click(&mut t, &cx, (300.0, 300.0));
    assert!(t.claims_key(&cx, ToolKey::Escape));
    t.key(&cx, ToolKey::Escape, Mods::default());
    assert!(!t.busy() && !t.claims_key(&cx, ToolKey::Escape));
    click(&mut t, &cx, (300.0, 300.0));
    assert!(click(&mut t, &cx, (300.5, 300.0)).is_empty());
    assert_eq!((t.start, t.end), (Some(Point::new(300.0, 300.0)), None), "the double-click's second click places nothing");
    click(&mut t, &cx, (380.0, 300.0));
    t.history_moved();
    assert!(!t.busy());
    drag(&mut t, &cx, (300.0, 300.0), &[(380.0, 300.0)], Mods::default());
    t.drop_press(&cx);
    assert!(!t.busy() && t.overlays(&cx).is_empty());
}

/// A document holding an open straight path from (100, 100) to (200, 100), unselected.
fn doc_with_open_line() -> vectorcraft_doc::Document {
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let (l, id) = (d.layers[0].id, d.alloc_id());
    let path = vectorcraft_geom::shapes::line(Point::new(100.0, 100.0), Point::new(200.0, 100.0));
    d.insert(Some(l), 0, vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art())).unwrap();
    d
}

/// The arc command's params in `acts`, its only action.
fn params_of(acts: &[Action]) -> Value {
    let [Action::Exec(name, params)] = acts else { panic!("{acts:?}") };
    assert_eq!(name, "shape.threePointArc");
    params.clone()
}

/// With `join` on, an end placed near an open path's end lands on it and the command asks for the
/// join; with it off, the end stays where it was placed and the command asks for none. J while
/// the ends are placed switches `join`, moving a placed end onto the path's end.
#[test]
fn with_join_an_end_near_an_open_path_end_lands_on_it() {
    let d = doc_with_open_line();
    let (s, p) = (Selection::default(), paint());
    // Snap to Point off, so only `join` moves an end onto the path's end.
    let isolated = ToolContext { smart_guides: false, snap_to_point: false, isolation: Some(vectorcraft_doc::NodeId(u64::MAX)), ..cx(&d, &s, &p) };
    let cx = ToolContext { smart_guides: false, snap_to_point: false, ..cx(&d, &s, &p) };
    let mut t = ThreePointArcTool::default();
    assert!(!t.claims_key(&cx, ToolKey::Letter('j')), "no ends: J is left to the app");
    click(&mut t, &cx, (198.5, 100.5));
    click(&mut t, &cx, (280.0, 180.0));
    let off = params_of(&click(&mut t, &cx, (280.0, 100.0)));
    assert_eq!((off["start"].clone(), off.get("join")), (json!([198.5, 100.5]), None), "{off}");
    t.set_option("join", &json!(true));
    click(&mut t, &cx, (198.5, 100.5));
    click(&mut t, &cx, (280.0, 180.0));
    let on = params_of(&click(&mut t, &cx, (280.0, 100.0)));
    assert_eq!((on["start"].clone(), on["join"].clone()), (json!([200.0, 100.0]), json!(true)), "{on}");
    // The second end lands on the path's first end.
    click(&mut t, &cx, (20.0, 180.0));
    click(&mut t, &cx, (101.5, 100.5));
    let on = params_of(&click(&mut t, &cx, (20.0, 100.0)));
    assert_eq!(on["end"], json!([100.0, 100.0]), "{on}");
    // In isolation mode, a path outside the isolated node is left alone.
    click(&mut t, &isolated, (198.5, 100.5));
    assert_eq!(t.start, Some(Point::new(198.5, 100.5)));
    t.key(&cx, ToolKey::Escape, Mods::default());
    // Past the path's end, off its stroke, but within reach of the end.
    click(&mut t, &cx, (203.5, 102.0));
    assert_eq!(t.start, Some(Point::new(200.0, 100.0)));
    t.key(&cx, ToolKey::Escape, Mods::default());
    // A second end that would land on the first end is not placed, by a click or by a drag.
    click(&mut t, &cx, (200.0, 100.0));
    click(&mut t, &cx, (196.0, 100.0));
    assert_eq!((t.start, t.end), (Some(Point::new(200.0, 100.0)), None));
    t.key(&cx, ToolKey::Escape, Mods::default());
    drag(&mut t, &cx, (200.0, 100.0), &[(196.0, 100.0)], Mods::default());
    assert_eq!((t.start, t.end), (Some(Point::new(200.0, 100.0)), None));
    t.key(&cx, ToolKey::Escape, Mods::default());
    // J switches `join` off and on again while the ends are placed.
    t.set_option("join", &json!(false));
    click(&mut t, &cx, (198.5, 100.5));
    assert!(t.claims_key(&cx, ToolKey::Letter('j')));
    t.key(&cx, ToolKey::Letter('j'), Mods::default());
    assert_eq!((t.options()["join"].clone(), t.start), (json!(true), Some(Point::new(200.0, 100.0))));
    click(&mut t, &cx, (280.0, 180.0));
    let on = params_of(&click(&mut t, &cx, (280.0, 100.0)));
    assert_eq!((on["start"].clone(), on["join"].clone()), (json!([200.0, 100.0]), json!(true)), "{on}");
}

/// In direction mode, with the first end on an open path's end, a third point near the line along
/// which the arc carries the path on (here the line y = 100) snaps onto it, the same way or turned
/// back, and the tool shows the line dashed; farther away, with Shift, with `tangentSnap` off, or
/// away from a path's end, the point stays where it is.
#[test]
fn in_direction_mode_the_third_point_snaps_to_carry_the_path_on() {
    let d = doc_with_open_line();
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = ThreePointArcTool::default();
    t.set_option("direction", &json!(true));
    let dashed_through = |o: &[Overlay]| {
        o.iter().any(|o| matches!(o, Overlay::Line { a, b, dashed: true, .. } if (a.y - 100.0).abs() < 1e-9 && (b.y - 100.0).abs() < 1e-9 && a.midpoint(*b) == Point::new(200.0, 100.0)))
    };
    for (third, want) in [((260.0, 103.0), (260.0, 100.0)), ((140.0, 97.0), (140.0, 100.0)), ((260.0, 120.0), (260.0, 120.0))] {
        click(&mut t, &cx, (200.0, 100.0));
        click(&mut t, &cx, (200.0, 200.0));
        hover(&mut t, &cx, third, Mods::default());
        assert_eq!(dashed_through(&t.overlays(&cx)), third != want, "{third:?}");
        assert_eq!(params_of(&click(&mut t, &cx, third))["toward"], json!([want.0, want.1]), "{third:?}");
    }
    // Shift keeps the line at 45° steps instead: the pointer just off 45° goes onto 45°.
    click(&mut t, &cx, (200.0, 100.0));
    click(&mut t, &cx, (200.0, 200.0));
    let shift = Mods { shift: true, ..Mods::default() };
    let toward = params_of(&click_with(&mut t, &cx, (240.0, 63.0), shift))["toward"].clone();
    assert!((toward[0].as_f64().unwrap() - 200.0 + toward[1].as_f64().unwrap() - 100.0).abs() < 1e-9, "{toward}");
    // From an end away from the path, or with the snap off, no snap.
    for (first, snap) in [((200.0, 140.0), true), ((200.0, 100.0), false)] {
        t.set_option("tangentSnap", &json!(snap));
        click(&mut t, &cx, first);
        click(&mut t, &cx, (200.0, 220.0));
        let toward = params_of(&click(&mut t, &cx, (260.0, first.1 + 3.0)))["toward"].clone();
        assert_eq!(toward, json!([260.0, first.1 + 3.0]), "{first:?}");
    }
    // With Alt, the line from the second end, on the path's first end, carries the path on to the
    // left.
    t.set_option("tangentSnap", &json!(true));
    click(&mut t, &cx, (100.0, 200.0));
    click(&mut t, &cx, (100.0, 100.0));
    let alt = Mods { alt: true, ..Mods::default() };
    let acts = click_with(&mut t, &cx, (40.0, 98.0), alt);
    assert_eq!((params_of(&acts)["toward"].clone(), params_of(&acts)["at"].clone()), (json!([40.0, 100.0]), json!("end")));
}

/// The catalog lists the tool in the Line Segment tool's group, after the Arc tool, with no
/// default shortcut, and `create` makes it.
#[test]
fn the_tool_is_in_the_line_segment_group_after_the_arc_tool() {
    let group = crate::TOOL_GROUPS.iter().find(|g| g.iter().any(|t| t.id == ID)).expect("in a toolbar group");
    let ids: Vec<_> = group.iter().map(|t| t.id).collect();
    let arc = ids.iter().position(|id| *id == "arc").expect("the Arc tool's group");
    assert_eq!(ids.first(), Some(&"lineSegment"));
    assert_eq!(ids.get(arc + 1), Some(&ID));
    let info = crate::tool_info(ID).expect("catalogued");
    assert_eq!((info.label, info.shortcut), ("Three-Point Arc Tool", None));
    assert_eq!(crate::create(ID).id(), ID);
}
