use super::*;
use crate::testutil::*;
use vectorcraft_color::{GradientPaint, Paint};
use vectorcraft_doc::{Document, Selection};

fn ev(kind: PointerKind, p: Point) -> PointerEvent {
    PointerEvent::new(kind, p.x, p.y)
}

fn with(m: Mods, e: PointerEvent) -> PointerEvent {
    e.with_mods(m)
}

const SHIFT: Mods = Mods { shift: true, alt: false, cmd: false, ctrl: false, space: false };
const ALT: Mods = Mods { shift: false, alt: true, cmd: false, ctrl: false, space: false };

/// The test rectangle (100..200) with a horizontal gradient of `stops` from (100,150) to (200,150).
fn graded(stops: Vec<GradientStop>) -> (Document, Selection) {
    let (mut d, id) = doc_with_rect();
    let mut g = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    g.geom = Some(GradientGeom { start: Point::new(100.0, 150.0), end: Point::new(200.0, 150.0), aspect: 1.0, focal: None });
    d.node_mut(id).unwrap().appearance.set_fill(Paint::Gradient(Box::new(g)));
    let mut s = Selection::default();
    s.add(id);
    (d, s)
}

fn two() -> Vec<GradientStop> {
    Gradient::default().stops
}

fn three() -> Vec<GradientStop> {
    insert_stop(&Gradient::default(), 0.5).0
}

fn preview(a: &[Action]) -> &Value {
    a.iter().find_map(|a| if let Action::Preview(_, v) = a { Some(v) } else { None }).unwrap_or_else(|| panic!("no preview in {a:?}"))
}

fn exec(a: &Action) -> (&str, &Value) {
    match a {
        Action::Exec(c, v) => (c.as_str(), v),
        a => panic!("not an Exec: {a:?}"),
    }
}

fn point(v: &Value) -> Point {
    Point::new(v[0].as_f64().unwrap(), v[1].as_f64().unwrap())
}

/// Stop offsets in percent (rounded: offsets travel as f32).
fn offsets(v: &Value) -> Vec<f64> {
    v["stops"].as_array().unwrap().iter().map(|s| (s["offset"].as_f64().unwrap() * 100.0).round()).collect()
}

#[test]
fn drag_sets_gradient_vector_and_constrains() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GradientTool::default();
    assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 110.0, 150.0)).is_empty());
    let a = t.pointer(&cx, &with(SHIFT, PointerEvent::new(PointerKind::Drag, 190.0, 152.0)));
    assert_eq!(a[0], Action::Begin("Gradient".into()));
    let Action::Preview(c, v) = &a[1] else { panic!() };
    assert_eq!(c, "paint.setGradientGeom");
    assert_eq!(v["start"], json!([110.0, 150.0]));
    assert!((v["end"][1].as_f64().unwrap() - 150.0).abs() < 1e-9);
    // While dragging a solid-filled object the annotator follows the drag.
    assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Handle { p, .. } if p.x == 110.0)));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 190.0, 150.0)), vec![Action::Commit]);
}

#[test]
fn stroke_proxy_drags_and_annotates_the_stroke() {
    let (mut d, id) = doc_with_rect();
    d.node_mut(id).unwrap().appearance.set_stroke(Paint::Gradient(Box::new(GradientPaint::new(Gradient::default()))));
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    // The fill is solid: with the Fill proxy in front there is no annotator.
    assert!(GradientTool::default().overlays(&cx).is_empty());
    cx.fill_active = false;
    // The stroke's annotator spans the stroke-inflated box (1 pt stroke: 99.5..200.5).
    let a = Annotator::of(&cx).unwrap();
    assert_eq!((a.geom.start, a.geom.end, a.gradient.stops.len()), (Point::new(99.5, 150.0), Point::new(200.5, 150.0), 2));
    let mut t = GradientTool::default();
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 110.0, 120.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 190.0, 120.0));
    assert_eq!(preview(&a)["stroke"], json!(true));
}

#[test]
fn empty_selection_targets_hit_object() {
    let (d, id) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GradientTool::default();
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
    assert_eq!(a, vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
    assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 450.0, 450.0)).is_empty());
}

#[test]
fn the_end_handle_changes_only_the_end_and_the_start_moves_both() {
    let (d, s) = graded(two());
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GradientTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, Point::new(200.0, 150.0)));
    let v = preview(&t.pointer(&cx, &ev(PointerKind::Drag, Point::new(220.0, 170.0)))).clone();
    assert_eq!((point(&v["start"]), point(&v["end"])), (Point::new(100.0, 150.0), Point::new(220.0, 170.0)));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, Point::new(220.0, 170.0))), vec![Action::Commit]);
    // The start handle moves the whole vector.
    t.pointer(&cx, &ev(PointerKind::Down, Point::new(100.0, 150.0)));
    let v = preview(&t.pointer(&cx, &ev(PointerKind::Drag, Point::new(110.0, 140.0)))).clone();
    assert_eq!((point(&v["start"]), point(&v["end"])), (Point::new(110.0, 140.0), Point::new(210.0, 140.0)));
}

#[test]
fn past_the_end_rotates_and_shift_snaps() {
    let (d, s) = graded(two());
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GradientTool::default();
    assert_eq!(t.cursor(&cx, Point::new(210.0, 151.0), Mods::default()), Cursor::Rotate);
    t.pointer(&cx, &ev(PointerKind::Down, Point::new(210.0, 151.0)));
    let end = point(&preview(&t.pointer(&cx, &with(SHIFT, ev(PointerKind::Drag, Point::new(103.0, 60.0)))))["end"]);
    // Snapped to straight up, keeping the 100 pt length.
    assert!(end.distance(Point::new(100.0, 50.0)) < 1e-9, "{end:?}");
}

#[test]
fn handles_snap_to_anchors_and_shift_uses_the_constrain_angle() {
    let (d, s) = graded(two());
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    let mut t = GradientTool::default();
    // The end handle dragged near the rectangle's corner lands on it, with a guide label.
    t.pointer(&cx, &ev(PointerKind::Down, Point::new(200.0, 150.0)));
    let v = preview(&t.pointer(&cx, &ev(PointerKind::Drag, Point::new(202.0, 197.0)))).clone();
    assert_eq!(point(&v["end"]), Point::new(200.0, 200.0));
    assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "anchor")));
    t.pointer(&cx, &ev(PointerKind::Up, Point::new(202.0, 197.0)));
    assert!(!t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Label { .. })), "the guides go with the drag");
    // A vector drawn from near a corner starts on it.
    let (d2, id) = doc_with_rect();
    let mut s2 = Selection::default();
    s2.add(id);
    let cx2 = crate::testutil::cx(&d2, &s2, &p);
    t.pointer(&cx2, &ev(PointerKind::Down, Point::new(101.0, 102.0)));
    let v = preview(&t.pointer(&cx2, &ev(PointerKind::Drag, Point::new(150.0, 130.0)))).clone();
    assert_eq!(point(&v["start"]), Point::new(100.0, 100.0));
    t.pointer(&cx2, &ev(PointerKind::Up, Point::new(150.0, 130.0)));
    // Constrain Angle 30: Shift gives 30 and 75 degrees (and no snapping).
    cx.constrain_angle = 30.0;
    let angle = |v: &Value| GradientGeom { start: point(&v["start"]), end: point(&v["end"]), aspect: 1.0, focal: None }.angle_deg();
    for (to, want) in [(Point::new(190.0, 100.0), 30.0), (Point::new(125.0, 60.0), 75.0)] {
        t.pointer(&cx, &ev(PointerKind::Down, Point::new(200.0, 150.0)));
        let v = preview(&t.pointer(&cx, &with(SHIFT, ev(PointerKind::Drag, to)))).clone();
        assert!((angle(&v) - want).abs() < 1e-9, "{want}: {v}");
        t.pointer(&cx, &ev(PointerKind::Up, to));
    }
}

#[test]
fn a_bar_click_adds_a_stop_and_selects_it() {
    let (d, s) = graded(two());
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GradientTool::default();
    assert_eq!(t.cursor(&cx, Point::new(130.0, 150.0), Mods::default()), Cursor::AddStop);
    assert!(t.pointer(&cx, &ev(PointerKind::Down, Point::new(130.0, 150.0))).is_empty());
    let a = t.pointer(&cx, &ev(PointerKind::Up, Point::new(130.0, 150.0)));
    let (c, v) = exec(&a[0]);
    assert_eq!((c, offsets(v)), ("paint.editGradient", vec![0.0, 30.0, 100.0]));
    assert_eq!(v["keepModel"], json!(true), "the sampled color keeps its model");
    assert_eq!(a[1], GradientTool::select_stop(1));
}

#[test]
fn stops_drag_along_the_bar_off_it_to_delete_and_alt_copies() {
    let (d, s) = graded(two());
    let p = paint();
    let cx = cx(&d, &s, &p);
    let chip = Annotator::of(&cx).unwrap().stop_point(&cx, 0).unwrap();
    let mut t = GradientTool::default();
    // Pressing a stop selects it; dragging moves it.
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, chip)), vec![GradientTool::select_stop(0)]);
    assert_eq!(offsets(preview(&t.pointer(&cx, &ev(PointerKind::Drag, Point::new(140.0, chip.y))))), vec![40.0, 100.0]);
    assert_eq!(t.cursor(&cx, chip, Mods::default()), Cursor::Move);
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, Point::new(140.0, chip.y))), vec![Action::Commit, GradientTool::select_stop(0)]);
    // Two stops never lose one: dragged off the bar, the stop stays.
    t.pointer(&cx, &ev(PointerKind::Down, chip));
    assert_eq!(offsets(preview(&t.pointer(&cx, &ev(PointerKind::Drag, Point::new(100.0, 260.0))))), vec![0.0, 100.0]);
    t.pointer(&cx, &ev(PointerKind::Up, Point::new(100.0, 260.0)));
    // Alt-drag leaves the original and drags a copy.
    t.pointer(&cx, &with(ALT, ev(PointerKind::Down, chip)));
    assert_eq!(offsets(preview(&t.pointer(&cx, &with(ALT, ev(PointerKind::Drag, Point::new(150.0, chip.y)))))), vec![0.0, 50.0, 100.0]);
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, Point::new(150.0, chip.y))), vec![Action::Commit, GradientTool::select_stop(1)]);

    // With three stops, dragging the middle one off the bar deletes it.
    let (d, s) = graded(three());
    let cx = crate::testutil::cx(&d, &s, &p);
    let chip = Annotator::of(&cx).unwrap().stop_point(&cx, 1).unwrap();
    t.pointer(&cx, &ev(PointerKind::Down, chip));
    assert_eq!(offsets(preview(&t.pointer(&cx, &ev(PointerKind::Drag, Point::new(150.0, 200.0))))), vec![0.0, 100.0]);
    assert_eq!(t.cursor(&cx, Point::new(150.0, 200.0), Mods::default()), Cursor::RemoveStop);
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, Point::new(150.0, 200.0))), vec![Action::Commit, GradientTool::select_stop(1)]);
}

#[test]
fn diamonds_move_midpoints() {
    let (d, s) = graded(two());
    let p = paint();
    let cx = cx(&d, &s, &p);
    let diamond = Annotator::of(&cx).unwrap().mid_point(&cx, 0).unwrap();
    let mut t = GradientTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, diamond));
    let v = preview(&t.pointer(&cx, &ev(PointerKind::Drag, Point::new(130.0, diamond.y)))).clone();
    assert!((v["stops"][0]["midpoint"].as_f64().unwrap() - 0.3).abs() < 1e-6, "{v}");
}

#[test]
fn delete_and_arrows_act_on_the_selected_stop() {
    let (d, s) = graded(two());
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    let mut t = GradientTool::default();
    assert!(!t.claims_key(&cx, ToolKey::Delete), "no stop selected");
    cx.gradient_stop = Some(1);
    assert!(t.claims_key(&cx, ToolKey::Delete) && t.claims_key(&cx, ToolKey::Backspace) && !t.claims_key(&cx, ToolKey::Enter));
    // Two stops: Delete is claimed but does nothing.
    assert!(t.key(&cx, ToolKey::Delete, Mods::default()).is_empty());
    let a = t.key(&cx, ToolKey::Left, SHIFT);
    let (c, v) = exec(&a[0]);
    assert_eq!((c, offsets(v)), ("paint.editGradient", vec![0.0, 90.0]));
    assert_eq!(a[1], GradientTool::select_stop(1));
    // With three stops Delete removes the selected one and selects its successor.
    let (d, s) = graded(three());
    let mut cx = crate::testutil::cx(&d, &s, &p);
    cx.gradient_stop = Some(1);
    let a = t.key(&cx, ToolKey::Backspace, Mods::default());
    assert_eq!(offsets(exec(&a[0]).1), vec![0.0, 100.0]);
    assert_eq!(a[1], GradientTool::select_stop(1));
}

#[test]
fn a_click_inside_the_art_applies_the_gradient_from_there() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = GradientTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, Point::new(150.0, 120.0)));
    let a = t.pointer(&cx, &ev(PointerKind::Up, Point::new(150.0, 120.0)));
    let (c, v) = exec(&a[0]);
    assert_eq!(c, "paint.setGradientGeom");
    // The default fit's 100 pt horizontal vector, starting at the click.
    assert_eq!((point(&v["start"]), point(&v["end"]), &v["ids"]), (Point::new(150.0, 120.0), Point::new(250.0, 120.0), &json!([id.0])));
    // Outside the art a click does nothing.
    t.pointer(&cx, &ev(PointerKind::Down, Point::new(400.0, 400.0)));
    assert!(t.pointer(&cx, &ev(PointerKind::Up, Point::new(400.0, 400.0))).is_empty());
}

#[test]
fn double_click_on_a_stop_opens_its_popover() {
    let (d, s) = graded(two());
    let p = paint();
    let cx = cx(&d, &s, &p);
    let chip = Annotator::of(&cx).unwrap().stop_point(&cx, 1).unwrap();
    let a = GradientTool::default().pointer(&cx, &ev(PointerKind::DoubleClick, chip));
    assert_eq!(a[0], GradientTool::select_stop(1));
    assert_eq!(a[1], Action::Dialog("gradientStop".into(), json!({"index": 1, "x": chip.x, "y": chip.y})));
}

#[test]
fn overlays_draw_a_chip_per_stop_marking_the_selected_one() {
    let (d, s) = graded(two());
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    cx.gradient_stop = Some(1);
    let o = GradientTool::default().overlays(&cx);
    let chips: Vec<bool> = o.iter().filter_map(|o| if let Overlay::Swatch { selected, .. } = o { Some(*selected) } else { None }).collect();
    assert_eq!(chips, vec![false, true]);
    assert_eq!(o.iter().filter(|o| matches!(o, Overlay::Path { .. })).count(), 1, "one midpoint diamond");
}

#[test]
fn annotator_follows_the_active_appearance_item() {
    let (mut d, id) = doc_with_rect();
    let geom = GradientGeom { start: Point::new(100.0, 120.0), end: Point::new(200.0, 120.0), aspect: 1.0, focal: None };
    let mut gp = GradientPaint::new(Default::default());
    gp.geom = Some(geom);
    d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().paint = Paint::Gradient(Box::new(gp));
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let mut cx = cx(&d, &s, &p);
    // The fill is solid: no annotator until the stroke row (item 1) is the active item, which
    // brings the Stroke proxy forward.
    assert!(Annotator::of(&cx).is_none());
    cx.appearance_item = Some(1);
    assert!(Annotator::of(&cx).is_none(), "a stroke row does not stand in for the fill");
    cx.fill_active = false;
    assert_eq!(Annotator::of(&cx).unwrap().geom, geom);
    // A click inside the art edits that item.
    let mut t = GradientTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, Point::new(150.0, 180.0)));
    let a = t.pointer(&cx, &ev(PointerKind::Up, Point::new(150.0, 180.0)));
    assert_eq!(exec(&a[0]).1["item"], 1);
}

#[test]
fn a_radial_shows_its_extent_ellipse_aspect_handle_and_focal_dot() {
    let (mut d, s) = graded(two());
    let n = d.node_mut(s.objects[0]).unwrap();
    let Paint::Gradient(mut g) = n.appearance.fill_paint() else { panic!("a gradient fill") };
    g.gradient.kind = GradientKind::Radial;
    let geom = g.geom.as_mut().unwrap();
    geom.aspect = 0.5;
    geom.set_focal(Some(Point::new(120.0, 145.0)));
    n.appearance.set_fill(Paint::Gradient(g));
    let p = paint();
    let cx = cx(&d, &s, &p);
    let o = GradientTool::default().overlays(&cx);
    assert_eq!(o.iter().filter(|o| matches!(o, Overlay::Path { dashed: true, .. })).count(), 1, "the extent ellipse");
    // The aspect handle 50 pt above the centre, the focal dot where the focal point is.
    assert!(
        o.contains(&Overlay::Handle { p: Point::new(100.0, 100.0), color: BAR })
            && o.contains(&Overlay::Handle { p: Point::new(120.0, 145.0), color: BAR })
    );
    let a = Annotator::of(&cx).unwrap();
    let hit = |x: f64, y: f64| a.hit(&cx, Point::new(x, y));
    assert_eq!((hit(120.0, 145.0), hit(104.0, 150.0), hit(100.0, 100.0)), (Some(Part::Focal), Some(Part::Start), Some(Part::Aspect)));
    assert_eq!((hit(100.0, 200.0), hit(0.0, 150.0), hit(100.0, 220.0)), (Some(Part::Rotate), Some(Part::Rotate), None));
    assert_eq!(GradientTool::default().cursor(&cx, Point::new(100.0, 200.0), Mods::default()), Cursor::Rotate);
}
