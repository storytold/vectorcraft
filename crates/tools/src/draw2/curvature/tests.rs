use super::*;
use crate::testutil::*;
use vectorcraft_doc::{Appearance, Document, Node, Selection};
use vectorcraft_geom::{ParamCurve, PathData};

/// The Curvature tool has its own cursor, not the Pen's start-a-path one.
#[test]
fn curvature_tool_shows_its_cursor() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    assert_eq!(CurvatureTool::default().cursor(&cx, Point::new(10.0, 10.0), Mods::default()), Cursor::Curvature);
}

#[test]
fn first_click_creates_then_notify_tracks_path() {
    let (d, id) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = CurvatureTool::default();
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
    assert!(matches!(&a[0], Action::Exec(c, v) if c == "path.curvature" && v.get("id").is_none() && v["points"].as_array().unwrap().len() == 1));
    assert_eq!(a[1], Action::Notify("created".into()));
    // Pretend the engine created `id` with one anchor: it is a rect (4 anchors) so it doesn't match.
    let mut s2 = Selection::default();
    s2.set([id]);
    let cx2 = crate::testutil::cx(&d, &s2, &p);
    t.notify(&cx2, "created");
    assert_eq!(t.id, Some(id));
    assert!(t.active(&cx2).is_none());
    // With four points it matches, so a click appends a fifth.
    t.pts = vec![(Point::new(100.0, 100.0), false); 4];
    let a = t.pointer(&cx2, &PointerEvent::new(PointerKind::Down, 400.0, 400.0));
    assert!(matches!(&a[0], Action::Exec(_, v) if v["id"] == id.0 && v["points"].as_array().unwrap().len() == 5));
}

/// Points snap to Smart Guides (#506): the first lands on another object's anchor and says so.
#[test]
fn points_snap_to_smart_guides() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = CurvatureTool::default();
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 202.0, 99.0));
    assert!(matches!(&a[0], Action::Exec(_, v) if v["points"][0]["x"] == 200.0 && v["points"][0]["y"] == 100.0), "{a:?}");
    assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "anchor")));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 202.0, 99.0));
    assert!(t.overlays(&cx).is_empty());
}

fn pt(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

/// An open pen-style path: a corner start, two smooth anchors with uneven handles, a cusp and
/// an end with only an incoming handle.
fn wave() -> SubPath {
    SubPath::new(
        vec![
            Anchor::corner(pt(0.0, 100.0)),
            Anchor::with_handles(pt(100.0, 0.0), pt(60.0, 10.0), pt(160.0, -15.0)),
            Anchor::with_handles(pt(250.0, 80.0), pt(220.0, 30.0), pt(280.0, 130.0)),
            Anchor::with_handles(pt(350.0, 150.0), pt(320.0, 160.0), pt(380.0, 100.0)),
            Anchor::with_handles(pt(450.0, 60.0), pt(420.0, 90.0), pt(450.0, 60.0)),
        ],
        false,
    )
}

/// Every point of `a` lies on `b` (within 1e-6), sampled.
fn lies_on(a: &SubPath, b: &SubPath) -> bool {
    let b = PathData::single(b.clone());
    (0..a.segment_count()).all(|i| (0..=20).all(|k| b.nearest(a.segment(i).eval(k as f64 / 20.0)).is_some_and(|n| n.4 < 1e-6)))
}

fn collinear(a: Vec2, b: Vec2) -> bool {
    a.cross(b).abs() < 1e-6 * a.hypot().max(1.0) * b.hypot().max(1.0)
}

#[test]
fn moving_a_point_recurves_only_its_two_segments() {
    let before = wave();
    let mut sp = before.clone();
    assert_eq!(sp.anchors[2].kind, AnchorKind::Smooth);
    assert!(curvature_move(&mut sp, 2, pt(260.0, 140.0)));
    assert_eq!(sp.anchors[2].p, pt(260.0, 140.0));
    // The segments beyond its neighbours are bit-identical.
    assert_eq!(sp.segment(0), before.segment(0));
    assert_eq!(sp.segment(3), before.segment(3));
    // The neighbours keep their handle directions on its segments, and their other handles.
    assert!(collinear(sp.anchors[1].h_out - sp.anchors[1].p, before.anchors[1].h_out - before.anchors[1].p));
    assert!(collinear(sp.anchors[3].h_in - sp.anchors[3].p, before.anchors[3].h_in - before.anchors[3].p));
    assert_eq!((sp.anchors[1].h_in, sp.anchors[3].h_out), (before.anchors[1].h_in, before.anchors[3].h_out));
    // The moved smooth point takes the Curvature tool's handles, along the circle through it
    // and its neighbours, whatever the spacing: between a third and two thirds of its chord.
    let a = sp.anchors[2];
    let (prev, next) = (sp.anchors[1].p, sp.anchors[3].p);
    assert_eq!(a.kind, AnchorKind::Smooth);
    assert_eq!(Some((a.h_in, a.h_out)), smooth_handles(prev, a.p, next));
    assert!(collinear(a.h_in - a.p, a.h_out - a.p) && (a.h_in - a.p).dot(a.h_out - a.p) < 0.0);
    for (h, to) in [(a.h_in, prev), (a.h_out, next)] {
        let r = a.p.distance(h) / a.p.distance(to);
        assert!((1.0 / 3.0 - 1e-9..=2.0 / 3.0 + 1e-9).contains(&r), "{r}");
    }
    // A cusp keeps its handle directions; a moved end changes its one segment only.
    assert_eq!(sp.anchors[3].kind, AnchorKind::Corner);
    assert!(curvature_move(&mut sp, 3, pt(360.0, 170.0)));
    assert!(collinear(sp.anchors[3].h_out - sp.anchors[3].p, before.anchors[3].h_out - before.anchors[3].p));
    let mut end = before.clone();
    assert!(curvature_move(&mut end, 4, pt(470.0, 20.0)));
    assert_eq!(end.segment(2), before.segment(2));
    assert!(!curvature_move(&mut end, 5, pt(0.0, 0.0)));
}

/// A smooth end of an open path has one segment: its one handle into the curve, mirroring the
/// neighbour's tangent in their chord, and nothing on the outer side.
#[test]
fn moving_a_smooth_end_gives_it_the_end_handle() {
    let mut sp = curvature_spline(&[(pt(0.0, 0.0), false), (pt(100.0, 60.0), false), (pt(200.0, 0.0), false), (pt(300.0, 50.0), false)], false);
    sp.anchors[0].kind = AnchorKind::Smooth;
    let before = sp.clone();
    assert!(curvature_move(&mut sp, 0, pt(-20.0, 30.0)));
    let (e, b) = (sp.anchors[0], sp.anchors[1]);
    assert!(!e.has_in() && e.has_out());
    assert_eq!(Some(e.h_out), end_handle(e.p, &b, true));
    assert_eq!(sp.segment(2), before.segment(2));
    assert_eq!(sp.anchors[2], before.anchors[2]);
}

#[test]
fn moving_a_corner_without_handles_keeps_its_sides_straight() {
    let (d, id) = doc_with_rect();
    let mut sp = d.node(id).unwrap().path_data().unwrap().subpaths[0].clone();
    assert!(curvature_move(&mut sp, 0, pt(80.0, 70.0)));
    assert!((0..sp.segment_count()).all(|i| sp.segment_is_line(i)));
    assert_eq!(sp.anchors[0], Anchor::corner(pt(80.0, 70.0)));
}

#[test]
fn going_on_from_an_end_keeps_the_old_segments() {
    let before = wave();
    let mut sp = before.clone();
    curvature_extend(&mut sp, false, pt(550.0, 120.0), EndCurve::Keep, false);
    assert_eq!(sp.anchors.len(), 6);
    assert!((0..4).all(|i| sp.segment(i) == before.segment(i)));
    // The old end turns smooth, going on along its incoming handle, a third of the chord out.
    let e = sp.anchors[4];
    assert_eq!(e.kind, AnchorKind::Smooth);
    assert!(collinear(e.h_out - e.p, e.p - e.h_in) && (e.h_out - e.p).dot(e.p - e.h_in) > 0.0);
    assert!((e.h_out.distance(e.p) - e.p.distance(pt(550.0, 120.0)) / 3.0).abs() < 1e-9);
    // The new end has no outer handle and one into the curve, mirroring the old end's tangent.
    let n = sp.anchors[5];
    assert_eq!((n.p, n.has_out(), n.kind), (pt(550.0, 120.0), false, AnchorKind::Corner));
    assert_eq!(Some(n.h_in), end_handle(n.p, &e, false));
    // The point placed then curves through its neighbours as a drawn path's points do.
    curvature_extend(&mut sp, false, pt(650.0, 60.0), EndCurve::Smooth, false);
    let drawn = curvature_spline(&[(pt(450.0, 60.0), false), (pt(550.0, 120.0), false), (pt(650.0, 60.0), false)], false);
    assert_eq!((sp.anchors[5].h_in, sp.anchors[5].h_out), (drawn.anchors[1].h_in, drawn.anchors[1].h_out));
    assert!((0..4).all(|i| sp.segment(i) == before.segment(i)));
    // A corner stays one.
    curvature_extend(&mut sp, false, pt(700.0, 100.0), EndCurve::Corner, false);
    assert!(!sp.anchors[6].has_in() && !sp.anchors[6].has_out());
    // From the start: the old segments come one later, and the new end is the first anchor.
    let mut sp = before.clone();
    curvature_extend(&mut sp, true, pt(-100.0, 50.0), EndCurve::Keep, false);
    assert!(sp.anchors[0].p == pt(-100.0, 50.0) && !sp.anchors[0].has_in() && sp.anchors[0].has_out());
    assert!((0..4).all(|i| sp.segment(i + 1) == before.segment(i)));
    // A corner end without handles goes on along its segment's tangent.
    let old = sp.anchors[1];
    assert!(old.has_in() && !old.has_out());
    assert!(collinear(old.h_in - old.p, old.p - before.anchors[1].h_in) && (old.h_in - old.p).dot(old.p - before.anchors[1].h_in) > 0.0);
}

#[test]
fn closing_keeps_the_old_segments() {
    let before = wave();
    let mut sp = before.clone();
    curvature_close(&mut sp, false, EndCurve::Keep);
    assert!(sp.closed && sp.anchors.len() == 5);
    assert!((0..4).all(|i| sp.segment(i) == before.segment(i)));
    // The first anchor gains an incoming handle along its tangent: no kink there.
    let f = sp.anchors[0];
    assert!(f.has_in() && collinear(f.p - f.h_in, before.anchors[1].h_in - f.p));
}

#[test]
fn inserting_a_point_keeps_the_shape() {
    let before = wave();
    let mut sp = before.clone();
    assert_eq!(curvature_insert(&mut sp, 1, 0.3), Some(2));
    assert_eq!(sp.anchors.len(), 6);
    assert_eq!(sp.anchors[2].kind, AnchorKind::Smooth);
    assert!(lies_on(&sp, &before) && lies_on(&before, &sp));
    // On a straight side the point is smooth too, its handles along the side.
    let (d, id) = doc_with_rect();
    let rect = d.node(id).unwrap().path_data().unwrap().subpaths[0].clone();
    let mut sp = rect.clone();
    let i = curvature_insert(&mut sp, 0, 0.5).unwrap();
    assert_eq!(sp.anchors[i].kind, AnchorKind::Smooth);
    assert!(sp.anchors[i].has_in() && sp.anchors[i].has_out());
    assert!(lies_on(&sp, &rect) && lies_on(&rect, &sp));
    assert_eq!(curvature_insert(&mut sp, 99, 0.5), None);
}

/// On a straight side the inserted point's handles lie along it, each a third of its own part
/// of the side; a drag then moves it as a smooth Curvature point.
#[test]
fn a_point_inserted_on_a_line_takes_the_chord_rule_and_drags_smooth() {
    let (d, id) = doc_with_rect();
    let rect = d.node(id).unwrap().path_data().unwrap().subpaths[0].clone();
    let mut sp = rect.clone();
    let i = curvature_insert(&mut sp, 0, 0.25).unwrap();
    let (a, prev, next) = (sp.anchors[i], sp.anchors[i - 1].p, sp.anchors[i + 1].p);
    assert!((a.p.distance(a.h_in) - a.p.distance(prev) / 3.0).abs() < 1e-9);
    assert!((a.p.distance(a.h_out) - a.p.distance(next) / 3.0).abs() < 1e-9);
    assert!(collinear(a.h_out - a.p, next - prev) && (a.h_out - a.p).dot(next - prev) > 0.0);
    assert!(sp.segment_is_line(0) || lies_on(&sp, &rect));
    // A drag bends it as a smooth point; the sides beyond stay straight.
    let q = pt(a.p.x, a.p.y - 40.0);
    assert!(curvature_move(&mut sp, i, q));
    let m = sp.anchors[i];
    assert_eq!(Some((m.h_in, m.h_out)), smooth_handles(prev, q, next));
    assert!((2..sp.segment_count()).all(|s| sp.segment_is_line(s)));
}

#[test]
fn curvature_smooth_uses_the_tool_rule() {
    let mut sp = curvature_spline(&[(pt(0.0, 0.0), true), (pt(100.0, 60.0), true), (pt(200.0, 0.0), false), (pt(300.0, 50.0), false)], false);
    // A corner in the middle takes the bisector handles.
    assert!(curvature_smooth(&mut sp, 1));
    let a = sp.anchors[1];
    assert_eq!((a.kind, Some((a.h_in, a.h_out))), (AnchorKind::Smooth, smooth_handles(pt(0.0, 0.0), a.p, pt(200.0, 0.0))));
    // An end curves into its segment, mirroring its neighbour's tangent.
    assert!(curvature_smooth(&mut sp, 0));
    let e = sp.anchors[0];
    assert!(!e.has_in() && Some(e.h_out) == end_handle(e.p, &sp.anchors[1], true));
    // No direction: a fold, or an end whose neighbour has no handle there, stays as it is.
    let mut fold = SubPath::new(vec![Anchor::corner(pt(0.0, 0.0)), Anchor::corner(pt(100.0, 0.0)), Anchor::corner(pt(0.0, 0.0))], false);
    let before = fold.clone();
    assert!(!curvature_smooth(&mut fold, 1) && !curvature_smooth(&mut fold, 0) && !curvature_smooth(&mut fold, 9));
    assert_eq!(fold, before);
}

/// A curved end counts as smooth for the toggle: Alt-click turns it into a corner.
#[test]
fn alt_click_on_a_curved_end_makes_it_a_corner() {
    let sp = curvature_spline(&[(pt(100.0, 300.0), false), (pt(200.0, 200.0), false), (pt(300.0, 300.0), false)], false);
    let (d, id) = doc_with(sp);
    let (s, p) = (selected(id, None), paint());
    let cx = quiet(&d, &s, &p);
    let a = press(&mut CurvatureTool::default(), &cx, 100.0, 300.0, true);
    assert!(a.iter().any(|a| matches!(a, Action::Exec(c, v) if c == "path.convertAnchor" && v["anchor"] == 0 && v["to"] == "corner")), "{a:?}");
}

/// A document with one path, `sp`.
fn doc_with(sp: SubPath) -> (Document, NodeId) {
    let mut d = Document::new(800.0, 600.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    d.insert(Some(l), 0, Node::path(id, PathData::single(sp), Appearance::default_art())).unwrap();
    (d, id)
}

/// Path `id` selected, anchor `current` of its first subpath direct-selected (or none).
fn selected(id: NodeId, current: Option<usize>) -> Selection {
    let mut s = Selection::default();
    s.set([id]);
    if let Some(ai) = current {
        s.anchors.insert(id, [(0, ai)].into());
    }
    s
}

/// A context without snapping.
fn quiet<'a>(d: &'a Document, s: &'a Selection, p: &'a crate::PaintDefaults) -> ToolContext<'a> {
    ToolContext { smart_guides: false, snap_to_point: false, ..cx(d, s, p) }
}

fn press(t: &mut CurvatureTool, cx: &ToolContext, x: f64, y: f64, alt: bool) -> Vec<Action> {
    let ev = PointerEvent::new(PointerKind::Down, x, y).with_mods(Mods { alt, ..Mods::default() });
    let mut a = t.pointer(cx, &ev);
    a.extend(t.pointer(cx, &PointerEvent::new(PointerKind::Up, x, y)));
    a
}

/// The op and params of a `path.curvatureEdit` action.
fn edit_op(a: &Action) -> Option<(&str, &Value)> {
    match a {
        Action::Exec(c, v) | Action::Preview(c, v) if c == EDIT => Some((v["op"].as_str()?, v)),
        _ => None,
    }
}

#[test]
fn a_press_on_a_point_selects_it_and_a_drag_moves_it() {
    let (d, id) = doc_with(wave());
    let (s, p) = (selected(id, None), paint());
    let cx = quiet(&d, &s, &p);
    let mut t = CurvatureTool::default();
    assert_eq!(t.cursor(&cx, pt(250.0, 81.0), Mods::default()), Cursor::Move);
    assert_eq!(t.cursor(&cx, pt(450.0, 61.0), Mods::default()), Cursor::PenContinue);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 251.0, 81.0));
    assert_eq!(a, vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 2]], "mode": "set"}))]);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 260.0, 140.0));
    assert_eq!(a[0], Action::Begin("Curvature".into()));
    let moved = edit_op(&a[1]).map(|(op, v)| (op, v["anchor"].clone(), v["x"].clone(), v["y"].clone()));
    assert_eq!(moved, Some(("move", json!(2), json!(260.0), json!(140.0))));
    assert!(t.busy());
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 260.0, 140.0)), vec![Action::Commit]);
    // A press without a drag only selects.
    assert_eq!(press(&mut t, &cx, 100.0, 0.0, false).len(), 1);
}

/// #2120: the dragged end of a slanted straight path keeps to its extension (Smart Guides).
#[test]
fn a_dragged_end_of_a_straight_path_keeps_to_its_extension() {
    let (d, id) = doc_with(SubPath::polyline(&[pt(300.0, 400.0), pt(380.0, 340.0)], false));
    let (s, p) = (selected(id, None), paint());
    let cx = cx(&d, &s, &p);
    let mut t = CurvatureTool::default();
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 380.0, 340.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 421.5, 312.0));
    let moved = a.get(1).and_then(edit_op).map(|(op, v)| (op, v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap()));
    assert!(matches!(moved, Some(("move", x, y)) if (x - 420.0).abs() < 1e-9 && (y - 310.0).abs() < 1e-9), "{a:?}");
}

#[test]
fn alt_click_or_double_click_toggles_and_delete_removes() {
    let (d, id) = doc_with(wave());
    let p = paint();
    let s = selected(id, None);
    let cx = quiet(&d, &s, &p);
    let mut t = CurvatureTool::default();
    let to = |a: &[Action]| a.iter().find(|a| matches!(a, Action::Exec(c, _) if c == "path.convertAnchor")).cloned();
    let a = press(&mut t, &cx, 100.0, 0.0, true);
    assert!(matches!(to(&a), Some(Action::Exec(_, v)) if v["anchor"] == 1 && v["to"] == "corner"), "{a:?}");
    assert_eq!(t.cursor(&cx, pt(100.0, 0.0), Mods { alt: true, ..Mods::default() }), Cursor::PenConvert);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::DoubleClick, 0.0, 100.0));
    assert!(matches!(to(&a), Some(Action::Exec(_, v)) if v["anchor"] == 0 && v["to"] == "smooth"), "{a:?}");
    // Delete is the tool's while a point is current, and removes it.
    assert!(!t.claims_key(&cx, ToolKey::Delete));
    let s = selected(id, Some(2));
    let cx = quiet(&d, &s, &p);
    assert!(t.claims_key(&cx, ToolKey::Backspace));
    assert_eq!(t.key(&cx, ToolKey::Delete, Mods::default()), vec![remove_anchor((id, 0, 2))]);
    // Esc lets go of the point.
    assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
}

#[test]
fn a_press_on_a_segment_adds_a_point_and_a_drag_moves_it_in_one_step() {
    let (d, id) = doc_with(wave());
    let (s, p) = (selected(id, None), paint());
    let cx = quiet(&d, &s, &p);
    let mut t = CurvatureTool::default();
    let on = wave().segment(1).eval(0.5);
    assert_eq!(t.cursor(&cx, on, Mods::default()), Cursor::PenAdd);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, on.x, on.y));
    assert_eq!(a[0], Action::Begin("Curvature".into()));
    assert!(
        matches!(edit_op(&a[1]), Some(("insert", v)) if v["segment"] == 1 && (v["t"].as_f64().unwrap() - 0.5).abs() < 1e-3 && v.get("x").is_none())
    );
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, on.x + 1.0, on.y + 40.0));
    assert!(matches!(edit_op(&a[0]), Some(("insert", v)) if v["segment"] == 1 && v["y"] == on.y + 40.0), "{a:?}");
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, on.x, on.y + 40.0)), vec![Action::Commit]);
}

#[test]
fn clicks_go_on_from_a_current_end_and_the_other_end_closes() {
    let p = paint();
    let mut sp = wave();
    let mut t = CurvatureTool::default();
    let (d, id) = doc_with(sp.clone());
    let s = selected(id, Some(4));
    let a = press(&mut t, &quiet(&d, &s, &p), 550.0, 120.0, false);
    assert!(matches!(edit_op(&a[0]), Some(("extend", v)) if v["end"] == "end" && v["from"] == "keep" && v["x"] == 550.0), "{a:?}");
    // As the engine does it: the new end is current, and the next click curves through it.
    curvature_extend(&mut sp, false, pt(550.0, 120.0), EndCurve::Keep, false);
    let (d, id) = doc_with(sp.clone());
    let s = selected(id, Some(5));
    let cx = quiet(&d, &s, &p);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 650.0, 60.0));
    let ovs = t.overlays(&cx);
    assert!(ovs.iter().any(|o| matches!(o, Overlay::Path { .. })), "the rubber band shows");
    let from = pt(550.0, 120.0);
    let to = pt(650.0, 60.0);
    assert!(ovs.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &cx.segment_measure_label(from, to))));
    let a = press(&mut t, &cx, 650.0, 60.0, true);
    assert!(matches!(edit_op(&a[0]), Some(("extend", v)) if v["from"] == "smooth"), "{a:?}");
    curvature_extend(&mut sp, false, pt(650.0, 60.0), EndCurve::Smooth, false);
    let (d, id) = doc_with(sp.clone());
    let s = selected(id, Some(6));
    let cx = quiet(&d, &s, &p);
    // The Alt-clicked point stays a corner; a click on the first point closes.
    assert_eq!(t.cursor(&cx, pt(0.0, 100.0), Mods::default()), Cursor::PenClose);
    let a = press(&mut t, &cx, 0.0, 100.0, false);
    assert!(matches!(edit_op(&a[0]), Some(("close", v)) if v["from"] == "corner" && v["end"] == "end"), "{a:?}");
    // Any other end keeps its curve; the start goes on from the start.
    let s = selected(id, Some(0));
    let a = press(&mut t, &quiet(&d, &s, &p), -50.0, 150.0, false);
    assert!(matches!(edit_op(&a[0]), Some(("extend", v)) if v["end"] == "start" && v["from"] == "keep"), "{a:?}");
}
