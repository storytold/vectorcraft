use super::*;
use crate::testutil::*;
use vectorcraft_doc::Selection;
use vectorcraft_geom::{ParamCurve, PathData, PathSeg};

fn pt(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn start(p: Point) -> SubPath {
    SubPath::new(vec![Anchor::corner(p)], false)
}

fn piece(x: f64, y: f64, stroke: Stroke) -> Piece {
    Piece { to: pt(x, y), stroke, turn: DEFAULT_TURN, flip: false }
}

/// Every sampled point of the segments from anchor `from` on lies `r` from `center`.
fn on_circle(sp: &SubPath, from: usize, center: Point, r: f64) -> bool {
    (from..sp.segment_count()).all(|i| (0..=16).all(|k| (sp.segment(i).eval(k as f64 / 16.0).distance(center) - r).abs() < r * 1e-3))
}

#[test]
fn a_line_piece_adds_a_corner_point_and_no_handles() {
    let mut sp = start(pt(0.0, 0.0));
    assert!(append(&mut sp, &piece(100.0, 0.0, Stroke::Line)));
    assert_eq!(sp.anchors.len(), 2);
    assert!(sp.anchors.iter().all(|a| a.h_in == a.p && a.h_out == a.p && a.kind == AnchorKind::Corner));
}

/// A sharp-corner arc turns 180° by default: a half circle on its chord, on the left of the
/// direction drawn (above a chord drawn to the right, since y runs down), in two quarter segments.
#[test]
fn a_sharp_corner_arc_is_a_half_circle_bulging_left() {
    let mut sp = start(pt(0.0, 0.0));
    append(&mut sp, &piece(100.0, 0.0, Stroke::SharpArc));
    assert_eq!(sp.anchors.len(), 3);
    assert!(on_circle(&sp, 0, pt(50.0, 0.0), 50.0));
    assert!(sp.anchors.get(1).unwrap().p.distance(pt(50.0, -50.0)) < 1e-9);
    assert_eq!(sp.anchors.last().unwrap().p, pt(100.0, 0.0));
    // Flipped, it bulges the other way.
    let mut sp = start(pt(0.0, 0.0));
    append(&mut sp, &Piece { flip: true, ..piece(100.0, 0.0, Stroke::SharpArc) });
    assert!(sp.anchors.get(1).unwrap().p.distance(pt(50.0, 50.0)) < 1e-9);
}

/// The turn sets how much of a circle the arc is: 90° is one segment, 270° three.
#[test]
fn the_turn_sets_the_arc_and_its_segments() {
    let mut sp = start(pt(0.0, 0.0));
    append(&mut sp, &Piece { turn: 90.0, ..piece(100.0, 0.0, Stroke::SharpArc) });
    assert_eq!(sp.anchors.len(), 2);
    let r = 100.0 / (2.0 * (45f64).to_radians().sin());
    // Bulging up, so its center is below the chord.
    assert!(on_circle(&sp, 0, pt(50.0, 50.0), r));
    let mut sp = start(pt(0.0, 0.0));
    append(&mut sp, &Piece { turn: 270.0, ..piece(100.0, 0.0, Stroke::SharpArc) });
    assert_eq!(sp.anchors.len(), 4);
    assert!(sp.anchors.iter().skip(1).take(2).all(|a| a.kind == AnchorKind::Smooth));
}

/// A continuous arc leaves the end of the path in the direction the path arrives in: after a
/// line drawn to the right, an arc to a point below and to the right starts heading right.
#[test]
fn a_continuous_arc_goes_on_in_the_paths_direction() {
    let mut sp = start(pt(0.0, 0.0));
    append(&mut sp, &piece(100.0, 0.0, Stroke::Line));
    append(&mut sp, &piece(200.0, 100.0, Stroke::ContinuousArc));
    let corner = sp.anchors.get(1).unwrap();
    let out = corner.h_out - corner.p;
    assert!(out.y.abs() < 1e-9 && out.x > 0.0, "{out:?}");
    // The quarter circle centered at (100, 100).
    assert!(on_circle(&sp, 1, pt(100.0, 100.0), 100.0));
    // Another continuous arc goes on smoothly from its end.
    append(&mut sp, &piece(100.0, 200.0, Stroke::ContinuousArc));
    let joint = sp.anchors.get(2).unwrap();
    assert!((joint.p - joint.h_in).cross(joint.h_out - joint.p).abs() < 1e-6);
    assert_eq!(joint.kind, AnchorKind::Smooth);
}

/// With nothing to follow (the path's first piece) a continuous arc is drawn as a sharp-corner
/// arc; an end straight ahead is a line.
#[test]
fn a_continuous_arc_without_a_direction_or_straight_ahead() {
    let mut a = start(pt(0.0, 0.0));
    let mut b = start(pt(0.0, 0.0));
    append(&mut a, &piece(100.0, 0.0, Stroke::ContinuousArc));
    append(&mut b, &piece(100.0, 0.0, Stroke::SharpArc));
    assert_eq!(a, b);
    let mut sp = start(pt(0.0, 0.0));
    append(&mut sp, &piece(100.0, 0.0, Stroke::Line));
    append(&mut sp, &piece(300.0, 0.0, Stroke::ContinuousArc));
    assert_eq!(sp.anchors.len(), 3);
    assert_eq!(sp.anchors.get(1).unwrap().h_out, pt(100.0, 0.0));
}

/// A line or a sharp-corner arc leaves a corner behind it, also after a continuous arc and from
/// the smooth end of a path it goes on from; a continuous arc leaves a smooth point.
#[test]
fn only_a_continuous_arc_leaves_a_smooth_point() {
    let mut sp = start(pt(0.0, 0.0));
    append(&mut sp, &piece(100.0, 0.0, Stroke::Line));
    append(&mut sp, &piece(200.0, 100.0, Stroke::ContinuousArc));
    append(&mut sp, &piece(100.0, 200.0, Stroke::ContinuousArc));
    assert_eq!(sp.anchors.get(2).unwrap().kind, AnchorKind::Smooth);
    let smooth_end = sp.anchors.len() - 1;
    let mut a = sp.clone();
    a.anchors.last_mut().unwrap().kind = AnchorKind::Smooth;
    let mut b = a.clone();
    append(&mut a, &piece(300.0, 300.0, Stroke::SharpArc));
    append(&mut b, &piece(300.0, 300.0, Stroke::Line));
    for sp in [a, b] {
        assert_eq!(sp.anchors.get(smooth_end).unwrap().kind, AnchorKind::Corner);
    }
}

#[test]
fn a_piece_to_the_last_point_adds_nothing() {
    let mut sp = start(pt(5.0, 5.0));
    assert!(!append(&mut sp, &piece(5.0, 5.0, Stroke::SharpArc)));
    assert_eq!(sp.anchors.len(), 1);
}

/// Closing on the first point joins the two; elsewhere a straight segment closes the path.
#[test]
fn closing_joins_a_last_point_on_the_first() {
    let pieces = [piece(100.0, 0.0, Stroke::Line), piece(100.0, 100.0, Stroke::Line), piece(0.0, 0.0, Stroke::SharpArc)];
    let sp = build(&start(pt(0.0, 0.0)), &pieces, true);
    assert!(sp.closed);
    assert_eq!(sp.anchors.first().unwrap().p, pt(0.0, 0.0));
    assert!(sp.anchors.iter().filter(|a| a.p == pt(0.0, 0.0)).count() == 1);
    assert_ne!(sp.anchors.first().unwrap().h_in, pt(0.0, 0.0), "the first point takes the arc's handle");
    let sp = build(&start(pt(0.0, 0.0)), &pieces[..2], true);
    assert!(sp.closed && sp.anchors.len() == 3);
}

#[test]
fn actions_cycle_and_name_themselves() {
    assert_eq!(Stroke::Line.next(), Stroke::ContinuousArc);
    assert_eq!(Stroke::ContinuousArc.next(), Stroke::SharpArc);
    assert_eq!(Stroke::SharpArc.next(), Stroke::Line);
    for s in Stroke::ALL {
        assert_eq!(Stroke::parse(s.name()), Some(s));
    }
    assert_eq!(Stroke::parse("arc"), None);
}

#[test]
fn options_are_checked_and_set() {
    let mut t = QuickPenTool::default();
    assert_eq!(t.options(), json!({"action": "line", "turn": 180.0}));
    assert!(t.check_option("action", &json!("sharpArc")).is_ok());
    assert!(t.check_option("action", &json!("arc")).is_err());
    assert!(t.check_option("turn", &json!(0)).is_err());
    assert!(t.check_option("turn", &json!(360)).is_err());
    assert!(t.check_option("size", &json!(1)).is_err());
    t.set_option("action", &json!("continuousArc"));
    t.set_option("turn", &json!(90));
    assert_eq!(t.options(), json!({"action": "continuousArc", "turn": 90.0}));
}

/// Alt tapped alone switches to the next action; [ and ] change the arc size by 15° within 1° to
/// 359°, with no path being drawn.
#[test]
fn alt_tap_cycles_and_brackets_size_the_next_arc() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = QuickPenTool::default();
    assert!(t.claims_key(&cx, ToolKey::AltTap));
    for want in ["continuousArc", "sharpArc", "line"] {
        assert!(t.key(&cx, ToolKey::AltTap, Mods::default()).is_empty());
        assert_eq!(t.options()["action"], want);
    }
    assert!(t.key(&cx, ToolKey::BracketRight, Mods::default()).is_empty());
    assert_eq!(t.options()["turn"], 195.0);
    t.set_option("turn", &json!(10));
    t.key(&cx, ToolKey::BracketLeft, Mods::default());
    assert_eq!(t.options()["turn"], 10.0, "stays at or above 1°");
    // C is the Scissors' shortcut while no arc is being drawn.
    assert!(!t.claims_key(&cx, ToolKey::Letter('c')));
}

/// The first click starts a new path and the tool picks it up once the engine made it.
#[test]
fn the_first_click_starts_a_path() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = QuickPenTool::default();
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
    assert!(matches!(&a[0], Action::Exec(c, v) if c == COMMAND && v["start"] == json!([10.0, 10.0]) && v["pieces"] == json!([])), "{a:?}");
    assert_eq!(a[1], Action::Notify("created".into()));
    assert_eq!(t.cursor(&cx, pt(400.0, 400.0), Mods::default()), Cursor::Pen);
}

/// The circle `arc_circle` gives is the one `append` draws the arc on, for sharp-corner arcs of
/// less and more than a half circle, flipped, and continuous arcs; a line has none.
#[test]
fn the_arc_circle_is_the_circle_the_arc_is_drawn_on() {
    let line = build(&start(pt(0.0, 0.0)), &[piece(100.0, 0.0, Stroke::Line)], false);
    let cases = [
        (start(pt(0.0, 0.0)), Piece { turn: 90.0, ..piece(100.0, 0.0, Stroke::SharpArc) }),
        (start(pt(0.0, 0.0)), Piece { turn: 270.0, flip: true, ..piece(100.0, 30.0, Stroke::SharpArc) }),
        (line.clone(), piece(200.0, 100.0, Stroke::ContinuousArc)),
        (line.clone(), piece(40.0, 60.0, Stroke::ContinuousArc)),
    ];
    for (sp, pc) in cases {
        let arc = arc_circle(&sp, &pc).unwrap();
        let from = sp.anchors.len() - 1;
        let mut drawn = sp.clone();
        append(&mut drawn, &pc);
        assert!(on_circle(&drawn, from, arc.center, arc.radius), "{pc:?} {arc:?}");
        assert_eq!(arc.start, sp.anchors.last().unwrap().p);
    }
    assert!(arc_circle(&line, &piece(200.0, 100.0, Stroke::Line)).is_none());
}

/// A continuous arc's end goes to the protractor tick nearest it: on the same circle, where the
/// sweep is a whole number of 15° steps, so an end that sweeps 61° goes to the drawn 60° tick. A
/// sharp-corner arc and a sweep that rounds to a whole circle have no tick to go to.
#[test]
fn a_continuous_arc_end_goes_to_the_nearest_tick() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let sp = build(&start(pt(0.0, 0.0)), &[piece(100.0, 0.0, Stroke::Line)], false);
    let half = 30.5f64.to_radians();
    let to = pt(100.0 + 80.0 * half.cos(), 80.0 * half.sin());
    let pc = Piece { to, ..piece(0.0, 0.0, Stroke::ContinuousArc) };
    let arc = arc_circle(&sp, &pc).unwrap();
    assert!((arc.sweep.to_degrees() - 61.0).abs() < 1e-9, "{arc:?}");
    let b = tick_end(&sp, &pc).unwrap();
    let after = arc_circle(&sp, &Piece { to: b, ..pc }).unwrap();
    assert!((after.sweep.to_degrees() - 60.0).abs() < 1e-9, "{after:?}");
    assert!(after.center.distance(arc.center) < 1e-9 && (after.radius - arc.radius).abs() < 1e-9, "the same circle");
    let ticks = protractor(&cx, &arc);
    let on_a_tick = ticks.segments().any(|seg| (seg.eval(1.0) - seg.eval(0.0)).cross(b - seg.eval(0.0)).abs() < 1e-6);
    assert!(on_a_tick, "{b:?} lies across a drawn tick");
    assert!(tick_end(&sp, &Piece { stroke: Stroke::SharpArc, ..pc }).is_none());
    // Nearly straight behind: the sweep rounds to 360°.
    assert!(tick_end(&sp, &Piece { to: pt(20.0, 0.5), ..pc }).is_none());
}

/// An end inside the document's coordinate limit whose tick lies past it has no tick to go to: a
/// 59° sweep whose 60° tick is 0.4 pt past the limit.
#[test]
fn a_tick_past_the_coordinate_limit_is_not_used() {
    let x = vectorcraft_doc::MAX_COORD - 87.5;
    let sp = build(&start(pt(x - 100.0, 0.0)), &[piece(x, 0.0, Stroke::Line)], false);
    let half = 29.5f64.to_radians();
    let pc = Piece { to: pt(x + 100.0 * half.cos(), 100.0 * half.sin()), ..piece(0.0, 0.0, Stroke::ContinuousArc) };
    assert!(pc.to.x < vectorcraft_doc::MAX_COORD);
    assert!(tick_end(&sp, &pc).is_none());
    let inside = Piece { to: pt(pc.to.x - 10.0, pc.to.y), ..pc };
    assert!(tick_end(&sp, &inside).is_some(), "a tick inside the limit is used");
}

/// A continuous arc that starts a path has no direction to follow and sweeps the arc size, so its
/// end has no tick to go to: with a 100° arc size, a tick at 105° would move the end off the arc
/// the click draws.
#[test]
fn a_continuous_arc_without_a_direction_has_no_tick() {
    let sp = start(pt(0.0, 0.0));
    let pc = Piece { turn: 100.0, ..piece(100.0, 10.0, Stroke::ContinuousArc) };
    let arc = arc_circle(&sp, &pc).unwrap();
    assert!((arc.sweep.to_degrees() - 100.0).abs() < 1e-9, "{arc:?}");
    assert!(tick_end(&sp, &pc).is_none());
    // After a line, the same end has a tick.
    let sp = build(&start(pt(-100.0, 0.0)), &[piece(0.0, 0.0, Stroke::Line)], false);
    assert!(tick_end(&sp, &pc).is_some());
}

/// The protractor has a tick every 15° of the sweep, each across the arc's circle, the first
/// through the arc's start; a sweep that is not a whole number of steps adds a tick at its end.
#[test]
fn the_protractor_ticks_lie_across_the_arc() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let sp = start(pt(0.0, 0.0));
    for (turn, ticks) in [(90.0, 7), (100.0, 8)] {
        let pc = Piece { turn, ..piece(100.0, 0.0, Stroke::SharpArc) };
        let arc = arc_circle(&sp, &pc).unwrap();
        let segs: Vec<_> = protractor(&cx, &arc).segments().collect();
        assert_eq!(segs.len(), ticks, "{turn}");
        for seg in &segs {
            let (a, b) = (seg.eval(0.0), seg.eval(1.0));
            assert!((a.midpoint(b).distance(arc.center) - arc.radius).abs() < 1e-9);
            assert!((b - a).cross(arc.center - a).abs() < 1e-6, "points at the center");
        }
        let through = |seg: &PathSeg, q: Point| (seg.eval(1.0) - seg.eval(0.0)).cross(q - seg.eval(0.0)).abs() < 1e-6;
        assert!(through(segs.first().unwrap(), arc.start));
        assert!(through(segs.last().unwrap(), pc.to));
    }
}

/// The rubber band previews exactly the piece a click adds.
#[test]
fn the_rubber_band_is_the_next_piece() {
    let t = QuickPenTool { action: Stroke::SharpArc, ..Default::default() };
    let sp = build(&start(pt(0.0, 0.0)), &[piece(100.0, 0.0, Stroke::Line)], false);
    let band = t.rubber_band(&sp, pt(200.0, 0.0)).unwrap();
    let mut added = sp.clone();
    append(&mut added, &t.piece_to(pt(200.0, 0.0)));
    let whole = PathData::single(added);
    for seg in band.segments() {
        for k in 0..=8 {
            let q = seg.eval(k as f64 / 8.0);
            assert!(whole.nearest(q).is_some_and(|n| n.4 < 1e-6), "{q:?}");
        }
    }
}
