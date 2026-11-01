//! View → Show Print Tiling (`view.printTiling`, `print.tiling`) and the Print Tiling tool
//! (`print.tiling.set`): the pages drawn are print.preview's, and where the tool puts them is
//! saved with the print settings.

use serde_json::{Value, json};
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::tooling::ViewInfo;
use crate::*;

/// A 300 × 200 document with a rectangle.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    s
}

fn rect(v: &Value) -> [f64; 4] {
    let a: Vec<f64> = v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
    [a[0], a[1], a[2], a[3]]
}

fn close(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
}

/// `t` (a print.preview transform) applied to `(x, y)`.
fn apply(t: &Value, (x, y): (f64, f64)) -> (f64, f64) {
    let c: Vec<f64> = t.as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    (c[0] * x + c[2] * y + c[4], c[1] * x + c[3] * y + c[5])
}

fn tiling(s: &mut Session, settings: Value) -> Value {
    s.execute("print.tiling", &json!({ "settings": settings })).unwrap()
}

#[test]
fn show_print_tiling_is_a_per_document_view_toggle() {
    let mut s = session();
    let undo = s.doc().unwrap().history.undo.len();
    assert_eq!(s.execute("view.printTiling", &json!({})).unwrap()["on"], true);
    assert!(s.doc().unwrap().print_tiling);
    assert_eq!(s.execute("view.printTiling", &json!({})).unwrap()["on"], false);
    assert_eq!(s.execute("view.printTiling", &json!({"on": true})).unwrap()["on"], true);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "a view toggle is no undo step");
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    assert!(!s.doc().unwrap().print_tiling, "each document has its own");
}

#[test]
fn tiling_pages_are_print_preview_tiles() {
    let mut s = session();
    let set = json!({"scaling": "tileImageable", "scale": {"width": 600, "height": 600}, "margin": 18, "overlap": 9, "tileRange": "1-2, 5"});
    let pv = s.execute("print.preview", &json!({ "settings": set })).unwrap();
    let grid = &pv["tiles"][0];
    let tiles = grid["tiles"].as_array().unwrap();
    let pages = tiling(&mut s, set.clone())["pages"].as_array().unwrap().clone();
    assert!(tiles.len() > 4);
    assert_eq!(pages.len(), tiles.len(), "one page per tile");
    for (i, (p, t)) in pages.iter().zip(tiles).enumerate() {
        assert_eq!(p["number"], i + 1);
        assert!(close(rect(&p["imageable"]), rect(t)), "tiles are imageable areas");
        let printed = grid["printed"].as_array().unwrap().contains(&json!(i + 1));
        assert_eq!(p["printed"], printed);
        // The paper reaches the margin (18 pt on paper, 3 pt of the document at 600%) further.
        let (pg, im) = (rect(&p["page"]), rect(&p["imageable"]));
        assert!((im[0] - pg[0] - 3.0).abs() < 1e-6 && (pg[3] - im[3] - 3.0).abs() < 1e-6);
    }
    // Full pages: the tiles are the paper, the imageable area inside it.
    let full = json!({"scaling": "tileFull", "scale": {"width": 400, "height": 400}, "margin": 18});
    let pv = s.execute("print.preview", &json!({ "settings": full })).unwrap();
    let pages = tiling(&mut s, full)["pages"].as_array().unwrap().clone();
    assert_eq!(pages.len(), pv["tiles"][0]["tiles"].as_array().unwrap().len());
    assert!(close(rect(&pages[0]["page"]), rect(&pv["tiles"][0]["tiles"][0])));
}

#[test]
fn untiled_pages_come_from_the_sheets_once_per_page() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 400, "y": 0, "width": 300, "height": 200})).unwrap();
    // Letter turned landscape for the wide artboards, centred on each.
    let set = json!({"output": {"mode": "separations"}, "margin": 36});
    let pv = s.execute("print.preview", &json!({ "settings": set })).unwrap();
    assert!(pv["sheets"].as_array().unwrap().len() >= 8, "a page per ink");
    let pages = tiling(&mut s, set)["pages"].as_array().unwrap().clone();
    assert_eq!(pages.len(), 2, "each page once, not once per ink");
    for (p, cx) in pages.iter().zip([150.0, 550.0]) {
        let pg = rect(&p["page"]);
        assert!(close(pg, [cx - 396.0, 100.0 - 306.0, cx + 396.0, 100.0 + 306.0]), "{pg:?}");
        assert!(close(rect(&p["imageable"]), [pg[0] + 36.0, pg[1] + 36.0, pg[2] - 36.0, pg[3] - 36.0]));
    }
}

#[test]
fn the_tool_puts_the_first_page_where_it_is_dragged_as_one_undo_step() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("printTiling", v).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 45.0, 40.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 40.0, 30.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 40.0, 30.0), v).unwrap();
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.history.undo.last().unwrap().label, "Print Tiling");
    let set = s.execute("print.setup", &json!({})).unwrap()["settings"].clone();
    assert_eq!(set["tileOrigin"], json!({"placed": true, "x": 40.0, "y": 30.0}));
    // The page's imageable corner is the point dragged to, on the paper and on the canvas.
    let pv = s.execute("print.preview", &json!({"settings": {"margin": 12}})).unwrap();
    let (x, y) = apply(&pv["sheets"][0]["transform"], (40.0, 30.0));
    assert!((x - 12.0).abs() < 1e-6 && (y - 12.0).abs() < 1e-6, "({x}, {y})");
    let page = &tiling(&mut s, json!({"margin": 12}))["pages"][0];
    assert!((rect(&page["imageable"])[0] - 40.0).abs() < 1e-6 && (rect(&page["imageable"])[1] - 30.0).abs() < 1e-6);
    // Saved in the native file.
    let doc = &s.doc().unwrap().doc;
    assert_eq!(vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap().print_setup, doc.print_setup);
    // A double click puts the pages back where the placement puts them.
    s.pointer(&PointerEvent::new(PointerKind::DoubleClick, 10.0, 10.0), v).unwrap();
    assert_eq!(s.execute("print.setup", &json!({})).unwrap()["settings"]["tileOrigin"]["placed"], false);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.execute("print.setup", &json!({})).unwrap()["settings"]["tileOrigin"]["placed"], true);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.print_setup.is_none());
}

#[test]
fn tiles_start_at_the_tile_origin_and_cover_the_art() {
    let mut s = session();
    let tiles = json!({"scaling": "tileImageable", "scale": {"width": 400, "height": 400}});
    s.execute("print.setup", &json!({ "settings": tiles })).unwrap();
    let before = tiling(&mut s, json!({}))["pages"].as_array().unwrap().len();
    s.execute("print.tiling.set", &json!({"origin": [60, 35]})).unwrap();
    let pages = tiling(&mut s, json!({}))["pages"].as_array().unwrap().clone();
    let pv = s.execute("print.preview", &json!({})).unwrap();
    assert_eq!(pages.len(), pv["tiles"][0]["tiles"].as_array().unwrap().len(), "tile count equals print.preview");
    assert_eq!(pages.len(), pv["sheets"].as_array().unwrap().len());
    assert!(pages.len() > before, "the grid moved off the corner takes more tiles");
    let rects: Vec<[f64; 4]> = pages.iter().map(|p| rect(&p["imageable"])).collect();
    assert!(rects.iter().any(|r| (r[0] - 60.0).abs() < 1e-6 && (r[1] - 35.0).abs() < 1e-6), "a tile starts at the origin");
    let all = rects.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |a, r| [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])]);
    assert!(all[0] <= 0.0 && all[1] <= 0.0 && all[2] >= 300.0 && all[3] >= 200.0, "the tiles cover the artboard: {all:?}");
    // Bad input is refused.
    assert!(s.execute("print.tiling.set", &json!({})).is_err());
    assert!(s.execute("print.tiling.set", &json!({"origin": [1, 2], "artboard": 7})).is_err());
    // A 32-bit build read 2^32 as artboard 0.
    assert!(s.execute("print.tiling.set", &json!({"origin": [1, 2], "artboard": 1u64 << 32})).is_err());
    assert!(s.execute("print.tiling.set", &json!({"origin": [1e9, 2]})).is_err());
}

#[test]
fn an_origin_on_an_artboards_right_or_bottom_edge_is_measured_from_that_artboard() {
    // Artboard 0 is (0, 0) to (300, 200). Artboard 1 is (300, 0) to (600, 200), and its left edge is artboard 0's right edge.
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 300, "y": 0, "width": 300, "height": 200})).unwrap();
    let mut placed = |x: f64, y: f64| {
        let o = s.execute("print.tiling.set", &json!({"origin": [x, y]})).unwrap()["tileOrigin"].clone();
        (o["x"].as_f64().unwrap(), o["y"].as_f64().unwrap())
    };
    assert_eq!(placed(600.0, 50.0), (300.0, 50.0), "artboard 1's right edge");
    assert_eq!(placed(450.0, 200.0), (150.0, 200.0), "artboard 1's bottom edge");
    assert_eq!(placed(600.0, 200.0), (300.0, 200.0), "artboard 1's bottom right corner");
    assert_eq!(placed(150.0, 200.0), (150.0, 200.0), "artboard 0's bottom edge");
    assert_eq!(placed(300.0, 50.0), (0.0, 50.0), "the shared edge: the artboard to its right");
    let (x, _) = placed(600.0 + 1e-9, 50.0);
    assert!((x - 300.0).abs() < 1e-6, "roundoff past the edge: {x}");
    // Beyond every artboard the origin is measured from the first.
    assert_eq!(placed(600.001, 50.0), (600.001, 50.0), "beyond the edge");
}

#[test]
fn settings_saved_before_the_tile_origin_still_load() {
    let mut s = session();
    let mut old = serde_json::to_value(vectorcraft_pdf::PrintSettings::default()).unwrap();
    old.as_object_mut().unwrap().remove("tileOrigin");
    old["copies"] = json!(2);
    let mut doc = (*s.doc().unwrap().doc).clone();
    doc.print_setup = Some(old);
    s.doc_mut().unwrap().doc = std::sync::Arc::new(doc);
    let set = s.execute("print.setup", &json!({})).unwrap()["settings"].clone();
    assert_eq!((set["copies"].clone(), set["tileOrigin"]["placed"].clone()), (json!(2), json!(false)));
    assert_eq!(tiling(&mut s, json!({}))["pages"].as_array().unwrap().len(), 1);
}
