use vectorcraft_doc::corners::set_corners;
use vectorcraft_doc::{Appearance, Node, Selection};
use vectorcraft_geom::{PathData, SubPath};

use super::*;
use crate::testutil::*;

/// A square (100, 100)–(300, 300), plain: anchors from the top left, clockwise.
fn doc_with_square() -> (Document, NodeId) {
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let pts = [(100.0, 100.0), (300.0, 100.0), (300.0, 300.0), (100.0, 300.0)].map(|(x, y)| Point::new(x, y));
    d.insert(Some(l), 0, Node::path(id, PathData::single(SubPath::polyline(&pts, true)), Appearance::default_art())).unwrap();
    (d, id)
}

/// [`doc_with_square`] with a chamfer of radius 20 on corner 1, whose marker is at (280, 120).
fn doc_with_corner() -> (Document, NodeId) {
    let (mut d, id) = doc_with_square();
    let Some(NodeKind::Path { path, live, .. }) = d.node_mut(id).map(|n| &mut n.kind) else { panic!() };
    set_corners(path, live, &[1].into(), Some(20.0), Some(CornerKind::Chamfer));
    (d, id)
}

fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
    PointerEvent::new(kind, x, y)
}

fn with(mut e: PointerEvent, f: impl Fn(&mut Mods)) -> PointerEvent {
    f(&mut e.mods);
    e
}

fn radius_of(a: Option<&Action>) -> f64 {
    let Some(Action::Preview(_, p)) = a else { panic!("{a:?}") };
    p["radius"].as_f64().unwrap()
}

#[test]
fn hover_shows_the_anchor_and_a_click_cuts_with_the_tools_values() {
    let (d, id) = doc_with_square();
    let (sel, p) = (Selection::default(), paint());
    let cx = cx(&d, &sel, &p);
    let mut t = CornerTool::default();
    t.set_option("radius", &json!(15));
    t.set_option("kind", &json!("invertedRound"));
    assert!(t.pointer(&cx, &ev(PointerKind::Move, 301.0, 101.0)).is_empty());
    assert!(matches!(t.overlays(&cx).as_slice(), [Overlay::Anchor { filled: true, .. }]), "{:?}", t.overlays(&cx));
    t.pointer(&cx, &ev(PointerKind::Move, 200.0, 200.0));
    assert!(t.overlays(&cx).is_empty(), "nothing there");
    assert!(t.pointer(&cx, &ev(PointerKind::Down, 300.0, 100.0)).is_empty());
    assert!(t.busy());
    let up = t.pointer(&cx, &ev(PointerKind::Up, 300.0, 100.0));
    assert_eq!(up, vec![Action::Exec(SET.into(), json!({"items": [{"id": id.0, "anchors": [1]}], "radius": 15.0, "kind": "invertedRound"}))]);
    assert!(!t.busy());
}

#[test]
fn a_drag_from_an_anchor_sizes_the_corner_and_stops_at_what_fits() {
    let (d, id) = doc_with_square();
    let (sel, p) = (Selection::default(), paint());
    let cx = cx(&d, &sel, &p);
    let mut t = CornerTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, 300.0, 100.0));
    let drag = t.pointer(&cx, &ev(PointerKind::Drag, 280.0, 120.0));
    assert_eq!(drag.first(), Some(&Action::Begin(LABEL.into())));
    assert!((radius_of(drag.get(1)) - 20.0).abs() < 1e-9, "{drag:?}");
    let Some(Action::Preview(cmd, params)) = drag.get(1) else { panic!() };
    assert_eq!((cmd.as_str(), params["items"][0]["id"].clone(), params["kind"].clone()), (SET, json!(id.0), json!("round")));
    // Shift: whole values.
    let shifted = t.pointer(&cx, &with(ev(PointerKind::Drag, 276.6, 123.4), |m| m.shift = true));
    assert_eq!(radius_of(shifted.first()), 23.0);
    // Past what fits: as far as the sharp corners next to it, marked at the limit.
    let far = t.pointer(&cx, &ev(PointerKind::Drag, 0.0, 400.0));
    assert!((radius_of(far.first()) - 200.0).abs() < 1e-9, "{far:?}");
    assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::CurvePoint { color, .. } if *color == HIGHLIGHT_RED)));
    assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text.contains("Radius: 200"))));
    let up = t.pointer(&cx, &ev(PointerKind::Up, 280.0, 120.0));
    assert!(matches!(up.as_slice(), [Action::Preview(..), Action::Commit]), "{up:?}");
    // Back on the anchor: nothing to keep. Escape cancels.
    t.pointer(&cx, &ev(PointerKind::Down, 300.0, 100.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 280.0, 120.0));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 300.0, 100.0)), vec![Action::Cancel]);
    t.pointer(&cx, &ev(PointerKind::Down, 300.0, 100.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 280.0, 120.0));
    assert!(t.claims_key(&cx, ToolKey::Escape));
    assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
    assert!(!t.busy());
}

#[test]
fn a_cut_corners_marker_selects_loads_resizes_and_removes() {
    let (d, id) = doc_with_corner();
    let (sel, p) = (Selection::default(), paint());
    let cx = cx(&d, &sel, &p);
    let mut t = CornerTool::default();
    assert!(matches!(t.overlays(&cx).as_slice(), [Overlay::CurvePoint { filled: false, .. }]), "the marker shows");
    t.pointer(&cx, &ev(PointerKind::Move, 281.0, 119.0));
    assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::CurvePoint { filled: true, .. })), "hovered");
    // A click selects the corner's anchors and loads its values.
    t.pointer(&cx, &ev(PointerKind::Down, 280.0, 120.0));
    let up = t.pointer(&cx, &ev(PointerKind::Up, 280.0, 120.0));
    let Some(Action::Exec(cmd, params)) = up.first() else { panic!("{up:?}") };
    assert_eq!((cmd.as_str(), params["id"].clone(), params["mode"].clone()), ("select.anchors", json!(id.0), json!("set")));
    assert_eq!(params["anchors"], json!([[0, 1], [0, 2]]));
    assert_eq!(t.options(), json!({"radius": 20.0, "kind": "chamfer"}));
    // A drag resizes it, its kind kept; to nothing it goes.
    t.pointer(&cx, &ev(PointerKind::Down, 280.0, 120.0));
    let drag = t.pointer(&cx, &ev(PointerKind::Drag, 270.0, 130.0));
    assert!((radius_of(drag.get(1)) - 30.0).abs() < 1e-9, "{drag:?}");
    assert!(matches!(drag.get(1), Some(Action::Preview(_, p)) if p["kind"] == json!("chamfer")));
    let gone = t.pointer(&cx, &ev(PointerKind::Up, 310.0, 90.0));
    assert!(matches!(gone.as_slice(), [Action::Preview(c, _), Action::Commit] if c == REMOVE), "{gone:?}");
    // A double-click removes it.
    let dbl = t.pointer(&cx, &ev(PointerKind::DoubleClick, 280.0, 120.0));
    assert_eq!(dbl, vec![Action::Exec(REMOVE.into(), json!({"items": [{"id": id.0, "anchors": [1]}]}))]);
    assert!(t.pointer(&cx, &ev(PointerKind::DoubleClick, 300.0, 300.0)).is_empty(), "a sharp corner has nothing to remove");
}

#[test]
fn an_alt_marquee_cuts_every_corner_inside_and_a_plain_one_selects() {
    let (d, id) = doc_with_corner();
    let (sel, p) = (Selection::default(), paint());
    let cx = cx(&d, &sel, &p);
    let mut t = CornerTool::default();
    // Alt over an anchor starts a marquee too; the chamfered corner counts, the far side doesn't.
    let alt = |e| with(e, |m| m.alt = true);
    t.pointer(&cx, &alt(ev(PointerKind::Down, 50.0, 50.0)));
    t.pointer(&cx, &alt(ev(PointerKind::Drag, 350.0, 200.0)));
    assert!(matches!(t.overlays(&cx).as_slice(), [Overlay::Marquee(_)]));
    let up = t.pointer(&cx, &alt(ev(PointerKind::Up, 350.0, 200.0)));
    assert_eq!(up, vec![Action::Exec(SET.into(), json!({"items": [{"id": id.0, "anchors": [0, 1]}], "radius": RADIUS, "kind": "round"}))]);
    t.pointer(&cx, &alt(ev(PointerKind::Down, 400.0, 400.0)));
    assert!(t.pointer(&cx, &alt(ev(PointerKind::Up, 450.0, 450.0))).is_empty(), "nothing inside");
    // Without Alt the corners inside are selected; a click away deselects.
    t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0));
    let up = t.pointer(&cx, &ev(PointerKind::Up, 200.0, 200.0));
    let Some(Action::Exec(cmd, params)) = up.first() else { panic!("{up:?}") };
    assert_eq!((cmd.as_str(), params["anchors"].clone(), params["mode"].clone()), ("select.anchors", json!([[0, 0]]), json!("set")));
    t.pointer(&cx, &ev(PointerKind::Down, 400.0, 400.0));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 400.0, 400.0)), vec![Action::Exec("select.none".into(), json!({}))]);
}

#[test]
fn locked_paths_are_left_alone_and_options_are_checked() {
    let (mut d, id) = doc_with_corner();
    d.node_mut(id).unwrap().locked = true;
    let (sel, p) = (Selection::default(), paint());
    let cx = cx(&d, &sel, &p);
    let mut t = CornerTool::default();
    assert!(t.overlays(&cx).is_empty());
    t.pointer(&cx, &ev(PointerKind::Move, 100.0, 100.0));
    assert!(t.overlays(&cx).is_empty());
    for bad in [json!(-1), json!(1e308), json!("x")] {
        t.set_option("radius", &bad);
    }
    t.set_option("kind", &json!("spiky"));
    assert_eq!(t.options(), json!({"radius": RADIUS, "kind": "round"}));
    assert_eq!(t.cursor(&cx, Point::ZERO, Mods::default()), Cursor::CornerTool);
}
