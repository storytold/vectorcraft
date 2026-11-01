//! A reflected object is still clicked by its fill, not only by its outline.

use serde_json::{Value, json};
use vectorcraft_geom::Point;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A click with the Selection tool at (x, y): what it selects.
fn click(s: &mut Session, x: f64, y: f64) -> Vec<NodeId> {
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, x, y).with_mods(Mods::default()), v).unwrap();
    }
    s.doc().unwrap().selection.objects.clone()
}

#[test]
fn a_reflected_shape_is_clicked_by_its_fill() {
    const HEART: &str = "M200 290C190 280 110 230 110 170C110 140 130 120 155 120C175 120 192 132 200 148C208 132 225 120 245 120C270 120 290 140 290 170C290 230 210 280 200 290Z";
    let makers: [(&str, Value); 5] = [
        ("shape.rectangle", json!({"x": 150, "y": 150, "width": 100, "height": 100})),
        ("shape.ellipse", json!({"x": 150, "y": 150, "width": 100, "height": 100})),
        ("shape.polygon", json!({"cx": 200, "cy": 200, "radius": 60, "sides": 5})),
        ("shape.star", json!({"cx": 200, "cy": 200, "radius1": 70, "radius2": 40, "points": 5})),
        ("path.create", json!({"d": HEART})),
    ];
    for (cmd, params) in makers {
        for axis in ["horizontal", "vertical"] {
            let mut s = session();
            let id = id_of(&s.execute(cmd, &params).unwrap());
            s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
            s.execute("object.reflect", &json!({"axis": axis})).unwrap();
            let b = s.doc().unwrap().doc.bounds_of(&[id], false).unwrap();
            s.execute("select.set", &json!({"ids": []})).unwrap();
            let c = b.center();
            assert_eq!(click(&mut s, c.x, c.y), vec![id], "{cmd} reflected {axis}: a click in its fill at {c:?} (bounds {b:?})");
        }
    }
}

/// Hearts as SVG files bring them: a plain path, a compound path, inside a group, under a
/// transform. Reflected with the button and with a bounding-box handle dragged across.
#[test]
fn a_reflected_imported_heart_is_clicked_by_its_fill() {
    const D: &str = "M200 290C190 280 110 230 110 170C110 140 130 120 155 120C175 120 192 132 200 148C208 132 225 120 245 120C270 120 290 140 290 170C290 230 210 280 200 290Z";
    let files = [
        format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="400"><path fill="#e2384d" d="{D}"/></svg>"##),
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="400"><path fill="#e2384d" fill-rule="evenodd" d="{D} M190 200h20v20h-20z"/></svg>"##
        ),
        format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="400"><g><g fill="#e2384d"><path d="{D}"/></g></g></svg>"##),
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="400"><g transform="translate(10 0) scale(1.2)"><path fill="#e2384d" transform="matrix(1 0 0 1 -20 -20)" d="{D}"/></g></svg>"##
        ),
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="400" viewBox="0 0 200 200"><path style="fill:#e2384d;stroke:none" d="{D}" transform="scale(0.5)"/></svg>"##
        ),
    ];
    for (i, svg) in files.iter().enumerate() {
        for how in ["horizontal", "vertical", "handle"] {
            let mut s = Session::new();
            s.add_document(vectorcraft_svg::import(svg).unwrap(), None);
            s.execute("select.all", &json!({})).unwrap();
            let ids = s.doc().unwrap().selection.objects.clone();
            assert!(!ids.is_empty(), "file {i}: nothing imported");
            let b = s.doc().unwrap().doc.bounds_of(&ids, false).unwrap();
            if how == "handle" {
                // Drag the top handle down past the bottom: a top-to-bottom flip.
                let v = ViewInfo::default();
                s.select_tool("selection", v).unwrap();
                let (from, to) = (Point::new(b.center().x, b.y0), Point::new(b.center().x, b.y1 + b.height()));
                for (kind, p) in
                    [(PointerKind::Down, from), (PointerKind::Drag, Point::new(from.x, b.center().y)), (PointerKind::Drag, to), (PointerKind::Up, to)]
                {
                    s.pointer(&PointerEvent::new(kind, p.x, p.y).with_mods(Mods::default()), v).unwrap();
                }
            } else {
                s.execute("object.reflect", &json!({"axis": how})).unwrap();
            }
            let b = s.doc().unwrap().doc.bounds_of(&ids, false).unwrap();
            s.execute("select.set", &json!({"ids": []})).unwrap();
            // Inside the heart away from the compound's square hole: a third of the way down.
            let c = Point::new(b.center().x, if how == "horizontal" || how == "handle" { b.y1 - b.height() * 0.3 } else { b.y0 + b.height() * 0.3 });
            let picked = click(&mut s, c.x, c.y);
            assert_eq!(picked, ids, "file {i} flipped by {how}: a click in its fill at {c:?} (bounds {b:?})");
        }
    }
}

/// Still selected after the flip, a drag from inside the fill moves it.
#[test]
fn a_reflected_selected_shape_drags_by_its_fill() {
    const HEART: &str = "M200 290C190 280 110 230 110 170C110 140 130 120 155 120C175 120 192 132 200 148C208 132 225 120 245 120C270 120 290 140 290 170C290 230 210 280 200 290Z";
    let makers: [(&str, Value); 4] = [
        ("shape.rectangle", json!({"x": 150, "y": 150, "width": 100, "height": 100})),
        ("shape.polygon", json!({"cx": 200, "cy": 200, "radius": 60, "sides": 3})),
        ("shape.star", json!({"cx": 200, "cy": 200, "radius1": 70, "radius2": 40, "points": 5})),
        ("path.create", json!({"d": HEART})),
    ];
    for (cmd, params) in makers {
        for axis in ["horizontal", "vertical"] {
            let mut s = session();
            let id = id_of(&s.execute(cmd, &params).unwrap());
            s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
            s.execute("object.reflect", &json!({"axis": axis})).unwrap();
            let b = s.doc().unwrap().doc.bounds_of(&[id], false).unwrap();
            let c = b.center();
            let v = ViewInfo::default();
            s.select_tool("selection", v).unwrap();
            for (kind, x) in
                [(PointerKind::Down, c.x), (PointerKind::Drag, c.x + 20.0), (PointerKind::Drag, c.x + 40.0), (PointerKind::Up, c.x + 40.0)]
            {
                s.pointer(&PointerEvent::new(kind, x, c.y).with_mods(Mods::default()), v).unwrap();
            }
            let after = s.doc().unwrap().doc.bounds_of(&[id], false).unwrap();
            assert!((after.x0 - b.x0 - 40.0).abs() < 1e-6, "{cmd} reflected {axis}: a drag from {c:?} left it at {after:?} (was {b:?})");
        }
    }
}

/// An open path (an anchor deleted from a closed one) still fills as if closed, and its fill is
/// clicked on both sides of that closing line, reflected or not.
#[test]
fn an_open_filled_path_is_clicked_by_its_fill_either_way_round() {
    // The sliver of a strawberry, open, its closing line down its left side.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200"><path fill="#c22a3f" d="M86.311 69.612C100.544 78.261 110.506 88.64 110.506 100.748C110.506 111.127 100.544 118.046 86.311 118.046C93.214 102.045 91.407 102.403 91.407 97.61"/></svg>"##;
    for axis in [None, Some("vertical"), Some("horizontal")] {
        let mut s = Session::new();
        s.add_document(vectorcraft_svg::import(svg).unwrap(), None);
        s.execute("select.all", &json!({})).unwrap();
        let ids = s.doc().unwrap().selection.objects.clone();
        if let Some(axis) = axis {
            s.execute("object.reflect", &json!({"axis": axis})).unwrap();
        }
        let b = s.doc().unwrap().doc.bounds_of(&ids, false).unwrap();
        s.execute("select.set", &json!({"ids": []})).unwrap();
        // The middle of the sliver's widest part, well inside it.
        let x = if axis == Some("vertical") { b.x0 + b.width() * 0.4 } else { b.x0 + b.width() * 0.6 };
        assert_eq!(click(&mut s, x, b.center().y), ids, "reflected {axis:?}: a click in the fill (bounds {b:?})");
    }
}
