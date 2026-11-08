//! Smart Guides for the drawing tools, driven through the session (#506): where the Rectangle
//! tool's corners and the Pen's anchors land, and the guides they show.

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_geom::{Point, Rect};
use vectorcraft_tools::{Mods, Overlay, PointerEvent, PointerKind};

use crate::tooling::ViewInfo;
use crate::*;

/// An 800 × 600 document with a 100 × 100 square at (100, 100), nothing selected.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s
}

fn pointer(s: &mut Session, kind: PointerKind, x: f64, y: f64) {
    s.pointer(&PointerEvent::new(kind, x, y), ViewInfo::default()).unwrap();
}

fn labels(s: &mut Session) -> Vec<(String, Point)> {
    s.overlays(ViewInfo::default()).into_iter().filter_map(|o| if let Overlay::Label { text, p, .. } = o { Some((text, p)) } else { None }).collect()
}

/// The bounds of the object the last gesture made (it is selected).
fn made(s: &Session) -> Rect {
    let d = s.doc().unwrap();
    let id: NodeId = d.selection.objects[0];
    d.doc.node(id).unwrap().geometric_bounds().unwrap()
}

#[test]
fn a_rectangle_corner_lands_on_another_objects_anchor() {
    let mut s = session();
    s.select_tool("rectangle", ViewInfo::default()).unwrap();
    // Hovering near the square's corner already says where the press goes.
    pointer(&mut s, PointerKind::Move, 302.0, 297.0);
    pointer(&mut s, PointerKind::Move, 202.0, 203.0);
    assert_eq!(labels(&mut s), [("anchor".to_string(), Point::new(200.0, 200.0))]);
    pointer(&mut s, PointerKind::Down, 302.0, 297.0);
    pointer(&mut s, PointerKind::Drag, 250.0, 250.0);
    pointer(&mut s, PointerKind::Drag, 202.0, 203.0);
    assert_eq!(labels(&mut s), [("anchor".to_string(), Point::new(200.0, 200.0))]);
    let measure = s.overlays(ViewInfo::default()).into_iter().find_map(|o| if let Overlay::Measure { text, .. } = o { Some(text) } else { None });
    assert!(measure.as_deref().is_some_and(|t| t.starts_with("W: 102") && t.contains("H: 100")), "{measure:?}");
    pointer(&mut s, PointerKind::Up, 202.0, 203.0);
    // The start lined up with the artboard's centre (y = 300), the corner on the anchor.
    assert_eq!(made(&s), Rect::new(200.0, 200.0, 302.0, 300.0));
}

/// The corner being dragged never snaps to the rectangle's own preview: dragged slowly, it follows
/// the pointer instead of sticking where the last step left it.
#[test]
fn a_rectangle_does_not_snap_to_itself_while_drawn() {
    let mut s = session();
    s.select_tool("rectangle", ViewInfo::default()).unwrap();
    pointer(&mut s, PointerKind::Down, 320.0, 420.0);
    for x in 370..=380 {
        pointer(&mut s, PointerKind::Drag, f64::from(x), 470.0 + f64::from(x - 370));
    }
    pointer(&mut s, PointerKind::Up, 380.0, 480.0);
    assert_eq!(made(&s), Rect::new(320.0, 420.0, 380.0, 480.0));
}

#[test]
fn a_pen_anchor_lines_up_with_another_objects_centre() {
    let mut s = session();
    s.select_tool("pen", ViewInfo::default()).unwrap();
    pointer(&mut s, PointerKind::Down, 300.0, 420.0);
    pointer(&mut s, PointerKind::Up, 300.0, 420.0);
    // Hovering: in line with the square's centre (150, 150), the guide running from it.
    pointer(&mut s, PointerKind::Move, 151.5, 431.0);
    let ov = s.overlays(ViewInfo::default());
    assert!(ov.iter().any(|o| matches!(o, Overlay::Line { a, b, .. } if *a == Point::new(150.0, 150.0) && *b == Point::new(150.0, 431.0))), "{ov:?}");
    assert_eq!(labels(&mut s), [("align".to_string(), Point::new(150.0, 431.0))]);
    pointer(&mut s, PointerKind::Down, 151.5, 431.0);
    pointer(&mut s, PointerKind::Up, 151.5, 431.0);
    // Shift: along the 45° step from it, slid into line with the square's left side (x = 100).
    s.pointer(&PointerEvent::new(PointerKind::Down, 103.0, 386.0).with_mods(Mods { shift: true, ..Mods::default() }), ViewInfo::default()).unwrap();
    pointer(&mut s, PointerKind::Up, 103.0, 386.0);
    let d = s.doc().unwrap();
    let path = d.doc.node(d.selection.objects[0]).unwrap().path_data().unwrap();
    let pts: Vec<Point> = path.subpaths[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(pts[..2], [Point::new(300.0, 420.0), Point::new(150.0, 431.0)]);
    assert!((pts[2] - Point::new(100.0, 381.0)).hypot() < 1e-9, "{pts:?}");
}

/// Preferences › Smart Guides › Construction Guides: on (the default), a Pen anchor near the
/// 45° guide through the last one lands on it; off, it stays where it was clicked.
#[test]
fn construction_guides_follow_the_preference() {
    for on in [true, false] {
        let mut s = session();
        s.execute("prefs.set", &json!({"key": "constructionGuides", "value": on})).unwrap();
        s.select_tool("pen", ViewInfo::default()).unwrap();
        pointer(&mut s, PointerKind::Down, 500.0, 500.0);
        pointer(&mut s, PointerKind::Up, 500.0, 500.0);
        pointer(&mut s, PointerKind::Down, 561.0, 438.0);
        pointer(&mut s, PointerKind::Up, 561.0, 438.0);
        let d = s.doc().unwrap();
        let last = d.doc.node(d.selection.objects[0]).unwrap().path_data().unwrap().subpaths[0].anchors[1].p;
        let want = if on { Point::new(561.5, 438.5) } else { Point::new(561.0, 438.0) };
        assert!((last - want).hypot() < 1e-9, "{on}: {last:?}");
    }
}
