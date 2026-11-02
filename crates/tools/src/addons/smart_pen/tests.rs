use super::*;
use crate::testutil::{cx, paint};
use vectorcraft_doc::{Appearance, Document, Node, Selection};
use vectorcraft_geom::{PathData, SubPath};

const NO: Mods = Mods { shift: false, alt: false, cmd: false, ctrl: false, space: false };
const ALT: Mods = Mods { alt: true, ..NO };
const SHIFT: Mods = Mods { shift: true, ..NO };

/// A document with the open path `pts` (selected): the document, the path and the selection.
fn doc(sp: SubPath) -> (Document, NodeId, Selection) {
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    d.insert(Some(l), 0, Node::path(id, PathData::single(sp), Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.set([id]);
    (d, id, s)
}

fn line() -> SubPath {
    SubPath::polyline(&[Point::new(100.0, 100.0), Point::new(200.0, 100.0)], false)
}

fn ev(kind: PointerKind, x: f64, y: f64, m: Mods) -> PointerEvent {
    PointerEvent::new(kind, x, y).with_mods(m)
}

/// A context without Smart Guides, so points land where they are pressed.
fn plain<'a>(d: &'a Document, s: &'a Selection, p: &'a crate::PaintDefaults) -> ToolContext<'a> {
    ToolContext { smart_guides: false, snap_to_point: false, ..cx(d, s, p) }
}

/// A click: press and release at (x, y).
fn click(t: &mut SmartPenTool, cx: &ToolContext, x: f64, y: f64, m: Mods) -> Vec<Action> {
    let mut out = t.pointer(cx, &ev(PointerKind::Down, x, y, m));
    out.extend(t.pointer(cx, &ev(PointerKind::Up, x, y, m)));
    out
}

/// A tool drawing on from the last anchor of the selected path.
fn drawing(cx: &ToolContext, end: Point) -> SmartPenTool {
    let mut t = SmartPenTool::default();
    click(&mut t, cx, end.x, end.y, NO);
    assert!(t.active(cx).is_some(), "the end clicked is drawn on from");
    t
}

/// The last preview among `acts`: its command and params.
fn preview(acts: &[Action]) -> (String, Value) {
    match acts.iter().rev().find(|a| matches!(a, Action::Preview(..))) {
        Some(Action::Preview(c, p)) => (c.clone(), p.clone()),
        _ => panic!("no preview in {acts:?}"),
    }
}

#[test]
fn a_click_away_from_paths_starts_a_new_path() {
    let (d, s, p) = (Document::new(500.0, 500.0), Selection::default(), paint());
    let cx = plain(&d, &s, &p);
    let mut t = SmartPenTool::default();
    assert_eq!(t.cursor(&cx, Point::new(50.0, 50.0), NO), Cursor::SmartPen);
    let down = t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0, NO));
    assert_eq!(down.first(), Some(&Action::Begin("Smart Pen".into())));
    assert_eq!(preview(&down), ("path.create".into(), json!({"anchors": [{"x": 50.0, "y": 50.0}]})));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 50.0, 50.0, NO)), vec![Action::Commit]);
    assert!(t.drawing && !t.busy());
}

#[test]
fn clicks_add_straight_or_curved_segments_as_the_option_says() {
    let (d, _, s) = doc(line());
    let p = paint();
    let cx = plain(&d, &s, &p);
    let mut t = drawing(&cx, Point::new(200.0, 100.0));
    assert_eq!(t.cursor(&cx, Point::new(300.0, 200.0), NO), Cursor::PenDrawing);
    let acts = click(&mut t, &cx, 300.0, 200.0, NO);
    assert_eq!(acts.first(), Some(&Action::Begin("Smart Pen".into())));
    let (c, params) = preview(&acts);
    assert_eq!((c.as_str(), params["segment"].as_str(), params["x"].as_f64()), ("addon.pen.append", Some("straight"), Some(300.0)));
    assert!(params.get("in").is_none(), "a click gives the anchor no handles");
    assert_eq!(acts.last(), Some(&Action::Commit));
    t.set_option("next", &json!("curved"));
    assert_eq!(t.options(), json!({"next": "curved"}));
    let (_, params) = preview(&click(&mut t, &cx, 300.0, 200.0, NO));
    assert_eq!(params["segment"], json!("curved"));
    t.set_option("next", &json!("wavy"));
    assert_eq!(t.options(), json!({"next": "curved"}), "an unknown value changes nothing");
}

#[test]
fn a_drag_makes_a_smooth_point_and_alt_breaks_the_outgoing_handle() {
    let (d, _, s) = doc(line());
    let p = paint();
    let cx = plain(&d, &s, &p);
    let mut t = drawing(&cx, Point::new(200.0, 100.0));
    t.pointer(&cx, &ev(PointerKind::Down, 300.0, 200.0, NO));
    // Inside the drag threshold nothing happens yet.
    assert!(t.pointer(&cx, &ev(PointerKind::Drag, 301.0, 200.0, NO)).is_empty());
    let acts = t.pointer(&cx, &ev(PointerKind::Drag, 340.0, 210.0, NO));
    let (c, params) = preview(&acts);
    assert_eq!(c, "addon.pen.append");
    assert_eq!((params["out"].clone(), params["in"].clone()), (json!([340.0, 210.0]), json!([260.0, 190.0])));
    assert!(params.get("segment").is_none(), "a drag shapes the segment with its handles");
    // Alt: the incoming handle stays, only the outgoing one follows.
    let acts = t.pointer(&cx, &ev(PointerKind::Drag, 300.0, 250.0, ALT));
    let (_, params) = preview(&acts);
    assert_eq!((params["out"].clone(), params["in"].clone()), (json!([300.0, 250.0]), json!([260.0, 190.0])));
    // Shift: the handle in 45° steps.
    let acts = t.pointer(&cx, &ev(PointerKind::Drag, 340.0, 205.0, SHIFT));
    let (_, params) = preview(&acts);
    let out = params["out"].as_array().unwrap();
    assert!((out[1].as_f64().unwrap() - 200.0).abs() < 1e-9, "{out:?}");
    // Space moves the anchor with the pointer.
    let acts = t.pointer(&cx, &ev(PointerKind::Drag, 350.0, 215.0, Mods { space: true, ..NO }));
    let (_, params) = preview(&acts);
    assert_eq!((params["x"].as_f64(), params["y"].as_f64()), (Some(310.0), Some(210.0)));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 350.0, 215.0, NO)), vec![Action::Commit]);
}

#[test]
fn tab_switches_the_segment_being_placed_and_the_last_one_drawn() {
    let (d, id, s) = doc(line());
    let p = paint();
    let cx = plain(&d, &s, &p);
    let mut t = drawing(&cx, Point::new(200.0, 100.0));
    assert!(t.claims_key(&cx, ToolKey::Tab));
    t.pointer(&cx, &ev(PointerKind::Down, 300.0, 200.0, NO));
    t.pointer(&cx, &ev(PointerKind::Drag, 340.0, 210.0, NO));
    for want in ["straight", "curved", "straight"] {
        let acts = t.key(&cx, ToolKey::Tab, NO);
        assert_eq!(preview(&acts).1["segment"], json!(want));
    }
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 340.0, 210.0, NO)), vec![Action::Commit]);
    // Between anchors, Tab switches the last segment drawn: the line here becomes curved.
    assert_eq!(
        t.key(&cx, ToolKey::Tab, NO),
        vec![Action::Exec("addon.pen.segment".into(), json!({"id": id.0, "subpath": 0, "segment": 0, "curved": true}))]
    );
    // Not drawing, Tab is not the tool's.
    let mut idle = SmartPenTool::default();
    assert!(!idle.claims_key(&cx, ToolKey::Tab) && idle.key(&cx, ToolKey::Tab, NO).is_empty());
}

#[test]
fn a_press_on_the_first_anchor_closes_the_path() {
    let (d, id, s) = doc(SubPath::polyline(&[Point::new(100.0, 100.0), Point::new(200.0, 100.0), Point::new(200.0, 200.0)], false));
    let p = paint();
    let cx = plain(&d, &s, &p);
    let mut t = drawing(&cx, Point::new(200.0, 200.0));
    assert_eq!(t.cursor(&cx, Point::new(101.0, 100.0), NO), Cursor::PenClose);
    let down = t.pointer(&cx, &ev(PointerKind::Down, 101.0, 100.0, NO));
    assert_eq!(
        down,
        vec![Action::Begin("Close Path".into()), Action::Preview("addon.pen.close".into(), json!({"id": id.0, "subpath": 0, "segment": "straight"}))]
    );
    // A drag shapes the closing curve.
    let (c, params) = preview(&t.pointer(&cx, &ev(PointerKind::Drag, 130.0, 100.0, NO)));
    assert_eq!((c.as_str(), params["in"].clone()), ("path.close", json!([70.0, 100.0])));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 130.0, 100.0, NO)), vec![Action::Commit]);
    assert!(!t.drawing, "the path is finished");
}

#[test]
fn point_edits_without_switching_tools() {
    let (d, id, s) = doc(SubPath::polyline(&[Point::new(100.0, 100.0), Point::new(200.0, 100.0), Point::new(300.0, 100.0)], false));
    let p = paint();
    let cx = plain(&d, &s, &p);
    let mut t = SmartPenTool::default();
    let select = |mode: &str| Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 1]], "mode": mode}));
    assert_eq!(t.cursor(&cx, Point::new(200.0, 100.0), NO), Cursor::DirectAnchorSelected);
    assert_eq!(t.cursor(&cx, Point::new(200.0, 100.0), ALT), Cursor::PenDelete);
    // A click selects the point (an inner one: nothing is drawn on from it); Shift toggles it.
    assert_eq!(click(&mut t, &cx, 200.0, 100.0, NO), vec![select("set")]);
    assert!(!t.drawing);
    assert_eq!(click(&mut t, &cx, 200.0, 100.0, SHIFT), vec![select("toggle")]);
    // Alt-click removes it keeping the shape; Shift+Alt-click deletes it plainly.
    let remove = |plain: bool| vec![Action::Exec("addon.pen.removePoint".into(), json!({"id": id.0, "subpath": 0, "anchor": 1, "plain": plain}))];
    assert_eq!(click(&mut t, &cx, 200.0, 100.0, ALT), remove(false));
    assert_eq!(click(&mut t, &cx, 200.0, 100.0, Mods { shift: true, ..ALT }), remove(true));
    // A drag moves it (Shift: 45° steps from where it was).
    t.pointer(&cx, &ev(PointerKind::Down, 200.0, 100.0, NO));
    let acts = t.pointer(&cx, &ev(PointerKind::Drag, 230.0, 104.0, SHIFT));
    assert_eq!(acts.first(), Some(&Action::Begin("Move Anchor Point".into())));
    let (c, params) = preview(&acts);
    assert_eq!((c.as_str(), params["anchor"].as_u64(), params["y"].as_f64()), ("path.moveAnchor", Some(1), Some(100.0)));
    assert!((params["x"].as_f64().unwrap() - 230.0).abs() < 0.5, "{params}");
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 230.0, 104.0, NO)), vec![Action::Commit]);
    // Alt-drag pulls out the outgoing handle.
    t.pointer(&cx, &ev(PointerKind::Down, 200.0, 100.0, ALT));
    let acts = t.pointer(&cx, &ev(PointerKind::Drag, 220.0, 120.0, ALT));
    assert_eq!(acts.first(), Some(&Action::Begin("Move Direction Handle".into())));
    assert_eq!(preview(&acts), ("path.setHandle".into(), json!({"id": id.0, "subpath": 0, "anchor": 1, "which": "out", "x": 220.0, "y": 120.0})));
    t.pointer(&cx, &ev(PointerKind::Up, 220.0, 120.0, ALT));
    // A double-click toggles smooth and corner.
    assert_eq!(
        t.pointer(&cx, &ev(PointerKind::DoubleClick, 200.0, 100.0, NO)),
        vec![Action::Exec("addon.pen.togglePoint".into(), json!({"id": id.0, "subpath": 0, "anchor": 1}))]
    );
    // A click on the first anchor of the open path draws on from it, the path reversed.
    let acts = click(&mut t, &cx, 100.0, 100.0, NO);
    assert_eq!(acts.get(1), Some(&Action::Exec("path.reverse".into(), json!({"ids": [id.0], "subpath": 0}))));
    assert_eq!(acts.last(), Some(&Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 2]], "mode": "set"}))));
    assert!(t.drawing);
}

#[test]
fn handles_drag_and_keep_their_angle_with_alt() {
    let sp = SubPath::new(
        vec![
            Anchor::corner(Point::new(100.0, 100.0)),
            Anchor::smooth(Point::new(200.0, 100.0), Point::new(240.0, 100.0)),
            Anchor::corner(Point::new(300.0, 200.0)),
        ],
        false,
    );
    let (d, id, mut s) = doc(sp);
    s.anchors.insert(id, [(0, 1)].into_iter().collect());
    let p = paint();
    let cx = plain(&d, &s, &p);
    let mut t = SmartPenTool::default();
    assert_eq!(t.cursor(&cx, Point::new(240.0, 100.0), NO), Cursor::AnchorPoint);
    assert!(t.pointer(&cx, &ev(PointerKind::Down, 240.0, 100.0, NO)).is_empty());
    let acts = t.pointer(&cx, &ev(PointerKind::Drag, 250.0, 130.0, ALT));
    assert_eq!(acts.first(), Some(&Action::Begin("Move Direction Handle".into())));
    // Alt keeps the handle's direction: only its length follows the pointer.
    assert_eq!(preview(&acts), ("path.setHandle".into(), json!({"id": id.0, "subpath": 0, "anchor": 1, "which": "out", "x": 250.0, "y": 100.0})));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 250.0, 130.0, NO)), vec![Action::Commit]);
    // A click without a drag changes nothing; a double-click toggles its point.
    assert!(click(&mut t, &cx, 160.0, 100.0, NO).is_empty());
    assert_eq!(
        t.pointer(&cx, &ev(PointerKind::DoubleClick, 160.0, 100.0, NO)),
        vec![Action::Exec("addon.pen.togglePoint".into(), json!({"id": id.0, "subpath": 0, "anchor": 1}))]
    );
}

#[test]
fn segments_reshape_by_dragging_and_alt_click_adds_a_point() {
    let (d, id, s) = doc(line());
    let p = paint();
    let cx = plain(&d, &s, &p);
    let mut t = SmartPenTool::default();
    assert_eq!(t.cursor(&cx, Point::new(150.0, 100.0), NO), Cursor::ReshapeSegment);
    assert_eq!(t.cursor(&cx, Point::new(150.0, 100.0), ALT), Cursor::PenAdd);
    assert!(t.pointer(&cx, &ev(PointerKind::Down, 150.0, 100.0, NO)).is_empty());
    for (m, mode) in [(NO, "free"), (ALT, "keepAngles"), (SHIFT, "symmetric")] {
        let acts = t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 130.0, m));
        let (c, params) = preview(&acts);
        assert_eq!(
            (c.as_str(), params["mode"].as_str(), params["dy"].as_f64(), params["segment"].as_u64()),
            ("addon.pen.reshape", Some(mode), Some(30.0), Some(0))
        );
        assert!((params["t"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    }
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 150.0, 130.0, NO)), vec![Action::Commit]);
    let acts = click(&mut t, &cx, 125.0, 100.0, ALT);
    let [Action::Exec(c, params)] = acts.as_slice() else { panic!("{acts:?}") };
    assert_eq!((c.as_str(), params["id"].as_u64(), params["segment"].as_u64()), ("path.insertAnchor", Some(id.0), Some(0)));
    assert!(click(&mut t, &cx, 150.0, 100.0, NO).is_empty(), "a click on a selected path's segment does nothing");
    // A click on a path that isn't selected selects it.
    let none = Selection::default();
    let cx = plain(&d, &none, &p);
    assert_eq!(click(&mut t, &cx, 150.0, 100.0, NO), vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
}

#[test]
fn ends_of_other_open_paths_continue_or_join() {
    let (mut d, id, s) = doc(line());
    let l = d.layers[0].id;
    let other = d.alloc_id();
    let sp = SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0)], false);
    d.insert(Some(l), 1, Node::path(other, PathData::single(sp), Appearance::default_art())).unwrap();
    let p = paint();
    // Not drawing, nothing selected: a click on an end draws on from it.
    let none = Selection::default();
    let cx = plain(&d, &none, &p);
    let mut t = SmartPenTool::default();
    assert_eq!(t.cursor(&cx, Point::new(200.0, 300.0), NO), Cursor::PenContinue);
    assert_eq!(click(&mut t, &cx, 200.0, 300.0, NO), vec![Action::Exec("select.set".into(), json!({"ids": [other.0]}))]);
    assert!(t.drawing);
    // Drawing the selected path, a press on the other's end joins the two.
    let cx = plain(&d, &s, &p);
    let mut t = drawing(&cx, Point::new(200.0, 100.0));
    assert_eq!(t.cursor(&cx, Point::new(100.0, 300.0), NO), Cursor::PenJoin);
    assert_eq!(
        t.pointer(&cx, &ev(PointerKind::Down, 100.0, 300.0, NO)),
        vec![Action::Exec("path.join".into(), json!({"ids": [id.0, other.0], "ends": ["last", "first"], "subpaths": [0, 0]}))]
    );
    assert!(!t.drawing);
}

#[test]
fn the_rubber_band_shows_the_next_segment_and_ending_the_path() {
    let (d, _, s) = doc(line());
    let p = paint();
    let cx = plain(&d, &s, &p);
    let mut t = drawing(&cx, Point::new(200.0, 100.0));
    t.pointer(&cx, &ev(PointerKind::Move, 300.0, 200.0, NO));
    assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Path { .. })));
    // Curved, the band leaves the last anchor along the straight segment before it.
    t.set_option("next", &json!("curved"));
    let band = t.overlays(&cx).into_iter().find_map(|o| if let Overlay::Path { path, .. } = o { Some(path) } else { None }).unwrap();
    assert!(
        band.elements().iter().any(|e| matches!(e, vectorcraft_geom::PathEl::CurveTo(p1, ..) if (p1.y - 100.0).abs() < 1e-9 && p1.x > 200.0)),
        "{band:?}"
    );
    assert!(t.key(&cx, ToolKey::Escape, NO).is_empty());
    assert!(t.active(&cx).is_none());
    assert!(t.overlays(&cx).is_empty());
}

#[test]
fn an_alt_drag_away_from_paths_selects_the_points_inside_a_marquee() {
    let (d, id, s) = doc(SubPath::polyline(&[Point::new(100.0, 100.0), Point::new(200.0, 100.0), Point::new(300.0, 100.0)], false));
    let p = paint();
    let cx = plain(&d, &s, &p);
    let mut t = SmartPenTool::default();
    assert!(t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0, ALT)).is_empty());
    assert!(t.busy());
    t.pointer(&cx, &ev(PointerKind::Drag, 250.0, 150.0, ALT));
    assert_eq!(t.overlays(&cx).last(), Some(&Overlay::Marquee(Rect::from_points(Point::new(50.0, 50.0), Point::new(250.0, 150.0)))));
    let acts = t.pointer(&cx, &ev(PointerKind::Up, 250.0, 150.0, ALT));
    assert_eq!(acts, [Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": id.0, "anchors": [[0, 0], [0, 1]]}], "mode": "set"}))]);
    assert!(!t.busy(), "the marquee is over");
    // A click is no marquee; Shift toggles the points inside it.
    assert!(click(&mut t, &cx, 50.0, 50.0, ALT).is_empty());
    t.pointer(&cx, &ev(PointerKind::Down, 280.0, 50.0, Mods { shift: true, ..ALT }));
    t.pointer(&cx, &ev(PointerKind::Drag, 320.0, 150.0, Mods { shift: true, ..ALT }));
    let acts = t.pointer(&cx, &ev(PointerKind::Up, 320.0, 150.0, Mods { shift: true, ..ALT }));
    assert_eq!(acts, [Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": id.0, "anchors": [[0, 2]]}], "mode": "toggle"}))]);
}
