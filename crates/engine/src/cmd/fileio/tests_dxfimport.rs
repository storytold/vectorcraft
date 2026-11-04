//! DXF through the commands: `document.open` with the DXF options, `document.dxfInfo`,
//! File → Place of a drawing (fitted, centred or at its origin), the format table, binary files,
//! and a drawing `document.export` wrote opened again.

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Rect;

use super::*;

fn b64(bytes: &[u8]) -> String {
    vectorcraft_format::base64_encode(bytes)
}

/// A millimetre drawing: a 200 × 100 rectangle on layer Walls, a circle on layer Notes, a block
/// "Mark" (a short line) inserted twice, and a paper layout holding a line.
fn plan() -> Vec<u8> {
    let pairs: &[(i32, &str)] = &[
        (0, "SECTION"),
        (2, "HEADER"),
        (9, "$ACADVER"),
        (1, "AC1015"),
        (9, "$INSUNITS"),
        (70, "4"),
        (0, "ENDSEC"),
        (0, "SECTION"),
        (2, "TABLES"),
        (0, "TABLE"),
        (2, "LAYER"),
        (0, "LAYER"),
        (2, "Walls"),
        (70, "0"),
        (62, "1"),
        (0, "LAYER"),
        (2, "Notes"),
        (70, "4"),
        (62, "5"),
        (0, "ENDTAB"),
        (0, "ENDSEC"),
        (0, "SECTION"),
        (2, "BLOCKS"),
        (0, "BLOCK"),
        (2, "Mark"),
        (10, "0"),
        (20, "0"),
        (0, "LINE"),
        (8, "0"),
        (10, "0"),
        (20, "0"),
        (11, "10"),
        (21, "0"),
        (0, "ENDBLK"),
        (0, "ENDSEC"),
        (0, "SECTION"),
        (2, "ENTITIES"),
        (0, "LWPOLYLINE"),
        (8, "Walls"),
        (90, "4"),
        (70, "1"),
        (10, "0"),
        (20, "0"),
        (10, "200"),
        (20, "0"),
        (10, "200"),
        (20, "100"),
        (10, "0"),
        (20, "100"),
        (0, "CIRCLE"),
        (8, "Notes"),
        (10, "100"),
        (20, "50"),
        (40, "20"),
        (0, "INSERT"),
        (8, "Walls"),
        (2, "Mark"),
        (10, "20"),
        (20, "20"),
        (0, "INSERT"),
        (8, "Walls"),
        (2, "Mark"),
        (10, "150"),
        (20, "20"),
        (0, "LINE"),
        (67, "1"),
        (10, "0"),
        (20, "0"),
        (11, "50"),
        (21, "0"),
        (0, "ENDSEC"),
        (0, "EOF"),
    ];
    pairs.iter().map(|(c, v)| format!("{c:>3}\n{v}\n")).collect::<String>().into_bytes()
}

fn open(s: &mut Session, bytes: &[u8], extra: Value) -> crate::Result<Value> {
    let mut p = json!({"name": "plan.dxf", "dataBase64": b64(bytes)});
    if let (Some(p), Value::Object(e)) = (p.as_object_mut(), extra) {
        p.extend(e);
    }
    s.execute("document.open", &p)
}

fn layer_names(s: &Session) -> Vec<String> {
    s.doc().unwrap().doc.layers.iter().filter_map(|l| l.name.clone()).collect()
}

fn leaves(s: &Session) -> Vec<Node> {
    let mut out = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if n.children().is_none() {
            out.push(n.clone());
        }
    });
    out
}

const MM: f64 = 72.0 / 25.4;

#[test]
fn document_open_reads_a_dxf_drawing() {
    let mut s = Session::new();
    let r = open(&mut s, &plan(), json!({})).unwrap();
    assert_eq!((r["format"].as_str(), r["title"].as_str()), (Some("dxf"), Some("plan.dxf")));
    assert_eq!(layer_names(&s), ["Walls", "Notes"]);
    let st = s.doc().unwrap();
    // Millimetres at 1:1, the artboard the art's size.
    let board = st.doc.artboards[0].rect;
    assert!((board.width() - 200.0 * MM).abs() < 1e-6 && (board.height() - 100.0 * MM).abs() < 1e-6, "{board:?}");
    assert_eq!(st.doc.units, vectorcraft_doc::Unit::Millimeters);
    assert_eq!(st.doc.symbols.len(), 1, "the block is a symbol");
    assert_eq!(leaves(&s).iter().filter(|n| matches!(n.kind, NodeKind::SymbolInstance { .. })).count(), 2);
    assert!(st.doc.layers[1].locked, "Notes is locked");
    // Saving asks for a name: DXF opens as artwork, it isn't written back.
    assert!(st.path.is_none());
}

#[test]
fn document_open_takes_the_dxf_options() {
    let mut s = Session::new();
    open(&mut s, &plan(), json!({"dxf": {"fit": true, "mergeLayers": true}})).unwrap();
    assert_eq!(layer_names(&s), ["Layer 1"]);
    let board = s.doc().unwrap().doc.artboards[0].rect;
    assert_eq!((board.width(), board.height()), (792.0, 612.0), "fitted to a landscape letter page");
    open(&mut s, &plan(), json!({"dxf": {"unit": "cm", "scale": 1}})).unwrap();
    let board = s.doc().unwrap().doc.artboards[0].rect;
    assert!((board.width() - 200.0 * MM * 10.0).abs() < 1e-6, "1 cm = 1 unit: {board:?}");
    open(&mut s, &plan(), json!({"dxf": {"layout": "Layout1"}})).unwrap();
    assert_eq!(leaves(&s).len(), 1, "the paper layout's line");
    for bad in [json!({"dxf": {"unit": "furlong"}}), json!({"dxf": {"scale": 0}}), json!({"dxf": {"layout": "Sheet 9"}}), json!({"dxf": 3})] {
        assert!(open(&mut s, &plan(), bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn dxf_info_lists_layouts_layers_and_units() {
    let mut s = Session::new();
    let r = s.execute("document.dxfInfo", &json!({"dataBase64": b64(&plan())})).unwrap();
    assert_eq!(r["version"], "2000");
    assert_eq!(r["units"], "Millimeters");
    assert_eq!(r["layouts"], json!(["Model", "Layout1"]));
    assert_eq!(r["layers"], json!(["Walls", "Notes"]));
    assert_eq!((r["unit"].as_str(), r["scale"].as_f64()), (Some("Millimeters"), Some(1.0)));
}

#[test]
fn binary_dxf_says_to_save_it_as_ascii() {
    let mut s = Session::new();
    let mut bytes = vectorcraft_cad::import::BINARY_SENTINEL.to_vec();
    bytes.extend([0u8; 64]);
    let e = open(&mut s, &bytes, json!({})).unwrap_err().to_string();
    assert!(e.contains("binary DXF") && e.contains("ASCII"), "{e}");
    let e = s.execute("document.dxfInfo", &json!({"dataBase64": b64(&bytes)})).unwrap_err().to_string();
    assert!(e.contains("binary DXF"), "{e}");
}

#[test]
fn dxf_is_readable_and_detected_by_content() {
    let f = format("dxf").unwrap();
    assert!(f.read && f.write);
    assert!(OPEN_EXTS.contains(&"dxf") && PLACE_EXTS.contains(&"dxf"));
    assert_eq!(detect("drawing.txt", &plan()).map(|f| f.id), Some("dxf"), "the content tells");
    let mut s = Session::new();
    let r = s.execute("document.formats", &json!({})).unwrap();
    assert!(r["readable"].as_array().unwrap().contains(&json!("dxf")));
}

/// A document of one 600 × 400 artboard.
fn page() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    s
}

fn placed(s: &mut Session, dxf: Value) -> (Value, Rect) {
    let mut p = json!({"name": "plan.dxf", "dataBase64": b64(&plan())});
    if !dxf.is_null() {
        p["dxf"] = dxf;
    }
    let undo = s.doc().unwrap().history.undo.len();
    let r = s.execute("file.place", &p).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    let id = vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap());
    let b = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    (r, b)
}

#[test]
fn file_place_puts_a_drawing_in_as_one_group() {
    let mut s = page();
    let (r, b) = placed(&mut s, Value::Null);
    assert_eq!(r["format"], "dxf");
    let st = s.doc().unwrap();
    let id = vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap());
    assert!(matches!(st.doc.node(id).unwrap().kind, NodeKind::Group { clip: false, .. }));
    assert_eq!(st.doc.symbols.len(), 1, "its symbol joins the document");
    // Centred on the artboard at 1:1.
    assert!((b.center().x - 300.0).abs() < 0.5 && (b.center().y - 200.0).abs() < 0.5, "{b:?}");
    assert!((b.width() - 200.0 * MM).abs() < 1.0, "{b:?}");
    // Fit: the artboard's size.
    let (_, b) = placed(&mut s, json!({"fit": true}));
    assert!((b.width() - 600.0).abs() < 1.0 && (b.center().y - 200.0).abs() < 0.5, "{b:?}");
    // Not centred: the drawing's origin on the artboard's bottom-left corner.
    let (_, b) = placed(&mut s, json!({"center": false, "unit": "pt", "scale": 1}));
    assert!((b.x0 - 0.0).abs() < 0.5 && (b.y1 - 400.0).abs() < 0.5, "{b:?}");
}

#[test]
fn a_drawing_export_writes_opens_again() {
    let mut s = page();
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 350, "y": 100, "width": 100, "height": 100})).unwrap();
    let r = s.execute("document.export", &json!({"format": "dxf", "unit": "pt"})).unwrap();
    let bytes = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
    let r = open(&mut s, &bytes, json!({"dxf": {"unit": "pt", "scale": 1, "center": false}})).unwrap();
    assert_eq!(r["format"], "dxf");
    let all: Vec<Rect> = leaves(&s).iter().filter_map(|n| n.geometric_bounds()).collect();
    assert_eq!(all.len(), 2, "{all:?}");
    let h = s.doc().unwrap().doc.artboards[0].rect.height();
    // The export's origin is the artboard's bottom-left: the art keeps its place above it.
    let union = all.iter().copied().reduce(|a, b| a.union(b)).unwrap();
    assert!((union.width() - 350.0).abs() < 2.0 && (union.y1 - h + 200.0).abs() < 2.0, "{union:?} on {h}");
}
