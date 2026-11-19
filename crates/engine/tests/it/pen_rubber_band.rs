//! The Pen's rubber band after a smooth anchor is the segment the next click adds: a new corner,
//! a close on the first anchor, or a join with an end of another open path.

use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::{CubicBez, PathData, PathEl, Point};
use vectorcraft_tools::{Overlay, PointerEvent, PointerKind};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn view() -> ViewInfo {
    ViewInfo { zoom: 1.0, smart_guides: false, snap_to_grid: false, snap_to_pixel: false, snap_to_point: false, ..Default::default() }
}

fn at(s: &mut Session, v: ViewInfo, kind: PointerKind, p: Point) {
    s.pointer(&PointerEvent::new(kind, p.x, p.y), v).unwrap();
}

/// A press at `p` dragged to `out`: a smooth anchor with its outgoing handle at `out`.
fn smooth(s: &mut Session, v: ViewInfo, p: Point, out: Point) {
    at(s, v, PointerKind::Down, p);
    at(s, v, PointerKind::Drag, out);
    at(s, v, PointerKind::Up, out);
}

fn click(s: &mut Session, v: ViewInfo, p: Point) {
    at(s, v, PointerKind::Down, p);
    at(s, v, PointerKind::Up, p);
}

/// The rubber band shown with the pointer hovering at `h`, as one cubic segment.
fn band(s: &mut Session, v: ViewInfo, h: Point) -> CubicBez {
    at(s, v, PointerKind::Move, h);
    let paths: Vec<Vec<PathEl>> =
        s.overlays(v).into_iter().filter_map(|o| if let Overlay::Path { path, .. } = o { Some(path.elements().to_vec()) } else { None }).collect();
    match paths.as_slice() {
        [els] => match els.as_slice() {
            [PathEl::MoveTo(p0), PathEl::CurveTo(p1, p2, p3)] => CubicBez::new(*p0, *p1, *p2, *p3),
            other => panic!("the rubber band is one curve, got {other:?}"),
        },
        other => panic!("one rubber band, got {other:?}"),
    }
}

/// How many rubber bands show with the pointer hovering at `h` with Alt held as `alt` says.
fn bands(s: &mut Session, v: ViewInfo, h: Point, alt: bool) -> usize {
    let mut ev = PointerEvent::new(PointerKind::Move, h.x, h.y);
    ev.mods.alt = alt;
    s.pointer(&ev, v).unwrap();
    s.overlays(v).into_iter().filter(|o| matches!(o, Overlay::Path { .. })).count()
}

/// The measurement label shown with the pointer hovering at `h`.
fn label(s: &mut Session, v: ViewInfo, h: Point) -> String {
    at(s, v, PointerKind::Move, h);
    let labels: Vec<String> =
        s.overlays(v).into_iter().filter_map(|o| if let Overlay::Measure { text, .. } = o { Some(text) } else { None }).collect();
    match labels.as_slice() {
        [text] => text.clone(),
        other => panic!("one measurement label, got {other:?}"),
    }
}

fn paths(s: &Session) -> Vec<PathData> {
    let mut v = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if let NodeKind::Path { path, .. } = &n.kind {
            v.push(path.clone());
        }
    });
    v
}

/// An open path whose first anchor, at `end`, is smooth with its incoming handle at `h_in`.
fn smooth_ended_path(s: &mut Session, end: Point, h_in: Point, other: Point) {
    s.execute(
        "path.create",
        &json!({"anchors": [{"x": end.x, "y": end.y, "in": [h_in.x, h_in.y], "out": [2.0 * end.x - h_in.x, 2.0 * end.y - h_in.y]}, {"x": other.x, "y": other.y}]}),
    )
    .unwrap();
    s.execute("select.none", &json!({})).unwrap();
}

#[test]
fn rubber_band_is_the_corner_a_click_adds() {
    let (mut s, v) = (session(), view());
    s.select_tool("pen", v).unwrap();
    smooth(&mut s, v, Point::new(100.0, 100.0), Point::new(150.0, 60.0));
    let h = Point::new(300.0, 200.0);
    let preview = band(&mut s, v, h);
    click(&mut s, v, h);
    let [path] = paths(&s).try_into().unwrap();
    assert_eq!(path.subpaths[0].segment(0), preview);
}

#[test]
fn rubber_band_on_the_first_anchor_is_the_closing_segment() {
    let (mut s, v) = (session(), view());
    s.select_tool("pen", v).unwrap();
    let first = Point::new(100.0, 100.0);
    smooth(&mut s, v, first, Point::new(150.0, 60.0));
    click(&mut s, v, Point::new(300.0, 100.0));
    smooth(&mut s, v, Point::new(300.0, 300.0), Point::new(350.0, 340.0));
    // Within the pick tolerance of the first anchor: the band ends on the anchor, not the pointer.
    let preview = band(&mut s, v, Point::new(102.0, 101.0));
    assert_eq!(preview.p3, first);
    click(&mut s, v, Point::new(102.0, 101.0));
    let [path] = paths(&s).try_into().unwrap();
    let sp = &path.subpaths[0];
    assert!(sp.closed);
    assert_eq!(sp.segment(2), preview);
}

#[test]
fn rubber_band_on_another_paths_end_is_the_joining_segment() {
    let (mut s, v) = (session(), view());
    let end = Point::new(400.0, 300.0);
    smooth_ended_path(&mut s, end, Point::new(430.0, 250.0), Point::new(500.0, 300.0));
    s.select_tool("pen", v).unwrap();
    smooth(&mut s, v, Point::new(100.0, 100.0), Point::new(150.0, 60.0));
    let preview = band(&mut s, v, Point::new(402.0, 301.0));
    assert_eq!((preview.p2, preview.p3), (Point::new(430.0, 250.0), end));
    click(&mut s, v, Point::new(402.0, 301.0));
    let [path] = paths(&s).try_into().unwrap();
    assert_eq!(path.subpaths[0].anchors.len(), 3);
    assert_eq!(path.subpaths[0].segment(0), preview);
}

#[test]
fn rubber_band_picks_the_end_at_the_pointer_before_grid_snapping() {
    let mut s = session();
    let mut prefs = s.prefs.clone();
    prefs.gridline_every = 100.0;
    prefs.grid_subdivisions = 1;
    s.apply_prefs(prefs);
    let v = ViewInfo { snap_to_grid: true, ..view() };
    // The end is off the grid: the pointer next to it snaps to (100, 300), out of its pick tolerance,
    // and a press there still joins at the pointer.
    let end = Point::new(63.0, 303.0);
    smooth_ended_path(&mut s, end, Point::new(90.0, 250.0), Point::new(13.0, 303.0));
    s.select_tool("pen", v).unwrap();
    smooth(&mut s, v, Point::new(300.0, 500.0), Point::new(350.0, 450.0));
    let pointer = Point::new(64.0, 303.0);
    let preview = band(&mut s, v, pointer);
    assert_eq!((preview.p2, preview.p3), (Point::new(90.0, 250.0), end));
    click(&mut s, v, pointer);
    let [path] = paths(&s).try_into().unwrap();
    assert_eq!(path.subpaths[0].anchors.len(), 3);
    assert_eq!(path.subpaths[0].segment(0), preview);
}

#[test]
fn rubber_band_joins_an_end_the_grid_snaps_to_from_a_corner() {
    let mut s = session();
    let mut prefs = s.prefs.clone();
    prefs.gridline_every = 100.0;
    prefs.grid_subdivisions = 1;
    s.apply_prefs(prefs);
    let v = ViewInfo { snap_to_grid: true, ..view() };
    // The end is on the grid and the pointer is out of its pick tolerance: only the snapped point
    // joins. The path ends on a corner, so only the end's own handle makes the segment a curve.
    let end = Point::new(100.0, 300.0);
    smooth_ended_path(&mut s, end, Point::new(130.0, 250.0), Point::new(13.0, 300.0));
    s.select_tool("pen", v).unwrap();
    click(&mut s, v, Point::new(300.0, 500.0));
    let pointer = Point::new(88.0, 300.0);
    let preview = band(&mut s, v, pointer);
    assert_eq!((preview.p1, preview.p2, preview.p3), (Point::new(300.0, 500.0), Point::new(130.0, 250.0), end));
    click(&mut s, v, pointer);
    let [path] = paths(&s).try_into().unwrap();
    assert_eq!(path.subpaths[0].anchors.len(), 3);
    assert_eq!(path.subpaths[0].segment(0), preview);
}

#[test]
fn measurement_label_measures_the_segment_a_click_adds() {
    let mut s = session();
    let mut prefs = s.prefs.clone();
    prefs.gridline_every = 100.0;
    prefs.grid_subdivisions = 1;
    s.apply_prefs(prefs);
    let grid = ViewInfo { snap_to_grid: true, ..view() };
    let end = Point::new(63.0, 303.0);
    smooth_ended_path(&mut s, end, Point::new(90.0, 250.0), Point::new(13.0, 303.0));
    s.select_tool("pen", grid).unwrap();
    smooth(&mut s, grid, Point::new(300.0, 500.0), Point::new(350.0, 450.0));
    // The pointer snaps to (100, 300), and the click joins at the end: the label measures to the end.
    let joining = label(&mut s, grid, Point::new(64.0, 303.0));
    assert_eq!(joining, label(&mut s, view(), end));
}

#[test]
fn no_rubber_band_over_the_last_anchor() {
    let (mut s, v) = (session(), view());
    s.select_tool("pen", v).unwrap();
    smooth(&mut s, v, Point::new(100.0, 100.0), Point::new(150.0, 60.0));
    smooth(&mut s, v, Point::new(300.0, 300.0), Point::new(350.0, 340.0));
    // A click there retracts the last anchor's outgoing handle and adds no segment.
    let h = Point::new(303.0, 300.0);
    assert_eq!(bands(&mut s, v, h, false), 0);
    click(&mut s, v, h);
    let [path] = paths(&s).try_into().unwrap();
    assert_eq!(path.subpaths[0].anchors.len(), 2);
}

#[test]
fn no_rubber_band_where_alt_converts_an_anchor() {
    let (mut s, v) = (session(), view());
    // Another path, in front, starts on an inner anchor of the path being drawn.
    let shared = Point::new(300.0, 100.0);
    let drawn = s
        .execute(
            "path.create",
            &json!({"anchors": [{"x": 100.0, "y": 100.0, "out": [150.0, 60.0]}, {"x": shared.x, "y": shared.y}, {"x": 300.0, "y": 300.0, "in": [250.0, 260.0], "out": [350.0, 340.0]}]}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    smooth_ended_path(&mut s, shared, Point::new(330.0, 50.0), Point::new(400.0, 50.0));
    s.execute("select.set", &json!({"ids": [drawn]})).unwrap();
    s.select_tool("pen", v).unwrap();
    // A click on the last anchor goes on drawing the path.
    click(&mut s, v, Point::new(300.0, 300.0));
    let paths_before = paths(&s).len();
    let h = Point::new(301.0, 100.0);
    assert_eq!(band(&mut s, v, h).p3, shared);
    // With Alt the click hands the anchor under the pointer to the Anchor Point tool and joins nothing.
    assert_eq!(bands(&mut s, v, h, true), 0);
    let mut down = PointerEvent::new(PointerKind::Down, h.x, h.y);
    down.mods.alt = true;
    s.pointer(&down, v).unwrap();
    let mut up = PointerEvent::new(PointerKind::Up, h.x, h.y);
    up.mods.alt = true;
    s.pointer(&up, v).unwrap();
    assert_eq!(paths(&s).len(), paths_before);
}

#[test]
fn no_rubber_band_where_a_join_merges_the_ends() {
    let (mut s, v) = (session(), view());
    // Another path's end lies less than the join's merge distance from where this path ends.
    let drawn = s
        .execute("path.create", &json!({"anchors": [{"x": 100.0, "y": 100.0, "out": [150.0, 60.0]}, {"x": 300.0, "y": 300.0, "in": [250.0, 260.0], "out": [350.0, 340.0]}]}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    smooth_ended_path(&mut s, Point::new(300.000_000_9, 300.0), Point::new(330.0, 250.0), Point::new(400.0, 300.0));
    s.execute("select.set", &json!({"ids": [drawn]})).unwrap();
    s.select_tool("pen", v).unwrap();
    // A click on the last anchor goes on drawing the path.
    click(&mut s, v, Point::new(300.0, 300.0));
    // Just out of the pick tolerance of the last anchor and in it of the other end: the click
    // merges the two ends into one anchor and adds no segment.
    let h = Point::new(305.000_000_5, 300.0);
    assert_eq!(bands(&mut s, v, h, false), 0);
    click(&mut s, v, h);
    let [path] = paths(&s).try_into().unwrap();
    assert_eq!(path.subpaths[0].anchors.len(), 3);
}
