//! Overprint per fill and stroke (`object.setOverprint`, `attributes.info`), its preview, the
//! legacy Overprint Black list, and Edit → Edit Colors → Overprint Black.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_geom::Affine;
use vectorcraft_render::{RenderOptions, Renderer};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s
}

/// A CMYK colour param (percentages).
fn ink(c: u8, m: u8, y: u8, k: u8) -> Value {
    json!({"c": c, "m": m, "y": y, "k": k})
}
fn cyan_ink() -> Value {
    ink(100, 0, 0, 0)
}
fn yellow_ink() -> Value {
    ink(0, 0, 100, 0)
}
fn black_ink() -> Value {
    ink(0, 0, 0, 100)
}

/// A rectangle with this fill and stroke (`None`: no stroke) of weight `weight`.
fn rect(s: &mut Session, xywh: [f64; 4], fill: Value, stroke: Option<Value>, weight: f64) -> NodeId {
    let [x, y, width, height] = xywh;
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": width, "height": height})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [id.0], "color": fill})).unwrap();
    match stroke {
        Some(c) => {
            s.execute("paint.setStroke", &json!({"ids": [id.0], "color": c})).unwrap();
            s.execute("stroke.set", &json!({"ids": [id.0], "weight": weight})).unwrap();
        }
        None => {
            s.execute("paint.setStroke", &json!({"ids": [id.0], "none": true})).unwrap();
        }
    }
    id
}

fn pixel(doc: &Document, preview: bool, x: u32, y: u32) -> [u8; 4] {
    let mut r = Renderer::new();
    r.threads = 0;
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), overprint_preview: preview, ..Default::default() };
    r.render(doc, 100, 100, Affine::IDENTITY, &opts).pixel(x, y)
}

fn green(p: [u8; 4]) -> bool {
    p[1] as i32 > p[0] as i32 + 60 && p[1] as i32 > p[2] as i32 + 60
}

fn cyan(p: [u8; 4]) -> bool {
    p[2] as i32 > p[0] as i32 + 60 && p[2] > 150 && !green(p)
}

fn node(s: &Session, id: NodeId) -> &Node {
    s.doc().unwrap().doc.node(id).unwrap()
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn cyan_stroke_overprinting_yellow_previews_green_under_the_stroke_only() {
    let mut s = session();
    rect(&mut s, [0.0, 0.0, 100.0, 100.0], yellow_ink(), None, 0.0);
    let c = rect(&mut s, [20.0, 20.0, 60.0, 60.0], cyan_ink(), Some(cyan_ink()), 10.0);
    let r = s.execute("object.setOverprint", &json!({"stroke": true, "ids": [c.0]})).unwrap();
    assert_eq!(r["changed"], 1);
    let a = node(&s, c).appearance.clone();
    assert!(a.stroke().unwrap().overprint && !a.fill().unwrap().overprint);

    let doc = s.doc().unwrap().doc.clone();
    // The outer half of the stroke lies over yellow only; the fill knocks out.
    let (stroke, fill) = ((17, 50), (50, 50));
    assert!(cyan(pixel(&doc, false, stroke.0, stroke.1)), "no preview: the stroke knocks out {:?}", pixel(&doc, false, stroke.0, stroke.1));
    assert!(green(pixel(&doc, true, stroke.0, stroke.1)), "overprint preview: cyan over yellow {:?}", pixel(&doc, true, stroke.0, stroke.1));
    assert!(cyan(pixel(&doc, true, fill.0, fill.1)), "the fill still knocks out {:?}", pixel(&doc, true, fill.0, fill.1));
    // Separations Preview implies overprint preview.
    let mut r = Renderer::new();
    r.threads = 0;
    let seps = vectorcraft_render::proof::ProofSetup { separations: Some(vec!["Yellow".into()]), ..Default::default() };
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), proof: Some(seps), ..Default::default() };
    let px = r.render(&doc, 100, 100, Affine::IDENTITY, &opts).pixel(stroke.0, stroke.1);
    assert!(px[0] < 100, "the yellow plate keeps its ink under the overprinting stroke: {px:?}");
    let px = r.render(&doc, 100, 100, Affine::IDENTITY, &opts).pixel(fill.0, fill.1);
    assert!(px[0] > 200, "and is knocked out under the fill: {px:?}");
}

#[test]
fn set_overprint_is_one_undo_step_and_reports_through_attributes_info() {
    let mut s = session();
    let a = rect(&mut s, [0.0, 0.0, 10.0, 10.0], cyan_ink(), Some(black_ink()), 1.0);
    let b = rect(&mut s, [20.0, 0.0, 10.0, 10.0], cyan_ink(), Some(black_ink()), 1.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let info = s.execute("attributes.info", &json!({})).unwrap();
    assert_eq!((info["overprintFill"].clone(), info["overprintStroke"].clone()), (json!(false), json!(false)));
    assert_eq!(info["ids"], json!([a.0, b.0]));

    let before = undo_len(&s);
    s.execute("object.setOverprint", &json!({"fill": true, "stroke": true, "ids": [a.0]})).unwrap();
    assert_eq!(undo_len(&s), before + 1, "fill and stroke in one step");
    let info = s.execute("attributes.info", &json!({})).unwrap();
    assert_eq!((info["overprintFill"].clone(), info["overprintStroke"].clone()), (Value::Null, Value::Null), "mixed");
    assert_eq!(s.attributes_info().overprint_fill, None);
    let info = s.execute("attributes.info", &json!({"ids": [a.0]})).unwrap();
    assert_eq!((info["overprintFill"].clone(), info["overprintStroke"].clone()), (json!(true), json!(true)));

    let r = s.execute("object.setOverprint", &json!({"fill": true, "ids": [a.0]})).unwrap();
    assert_eq!((r["changed"].clone(), undo_len(&s)), (json!(0), before + 1), "no change, no undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!node(&s, a).has_overprint());

    assert!(s.execute("object.setOverprint", &json!({"ids": [a.0]})).is_err(), "needs fill or stroke");
    assert!(s.execute("object.setOverprint", &json!({"fill": "yes", "ids": [a.0]})).is_err());
}

#[test]
fn set_overprint_reaches_groups_items_and_type() {
    let mut s = session();
    let a = rect(&mut s, [0.0, 0.0, 10.0, 10.0], cyan_ink(), Some(black_ink()), 1.0);
    let b = rect(&mut s, [20.0, 0.0, 10.0, 10.0], cyan_ink(), None, 0.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.setOverprint", &json!({"fill": true, "ids": [g.0]})).unwrap();
    assert!(node(&s, a).appearance.fill().unwrap().overprint && node(&s, b).appearance.fill().unwrap().overprint, "a group's contents");

    // An explicit appearance item: only that one.
    s.execute("appearance.addFill", &json!({"ids": [a.0]})).unwrap();
    let items = node(&s, a).appearance.items.len();
    let fills: Vec<usize> = (0..items).filter(|i| node(&s, a).appearance.items[*i].is_fill()).collect();
    s.execute("object.setOverprint", &json!({"fill": false, "ids": [a.0]})).unwrap();
    s.execute("object.setOverprint", &json!({"fill": true, "item": fills[0], "ids": [a.0]})).unwrap();
    let ov: Vec<bool> = fills.iter().map(|i| node(&s, a).appearance.items[*i].overprint()).collect();
    assert_eq!(ov, [true, false]);
    let stroke = (0..items).find(|i| !node(&s, a).appearance.items[*i].is_fill()).unwrap();
    assert!(s.execute("object.setOverprint", &json!({"fill": true, "item": stroke, "ids": [a.0]})).is_err(), "not a fill");

    // Type: its characters.
    let t = NodeId(s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hi"})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.setOverprint", &json!({"fill": true, "ids": [t.0]})).unwrap();
    let NodeKind::Text(tx) = &node(&s, t).kind else { panic!() };
    assert!(tx.runs.iter().all(|r| r.style.overprint_fill && !r.style.overprint_stroke));
    assert_eq!(s.execute("attributes.info", &json!({"ids": [t.0]})).unwrap()["overprintFill"], true);
}

#[test]
fn overprinting_type_previews_through() {
    let mut s = session();
    rect(&mut s, [0.0, 0.0, 100.0, 100.0], yellow_ink(), None, 0.0);
    let t = NodeId(s.execute("text.create", &json!({"x": 5, "y": 90, "text": "H", "size": 90})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [t.0], "color": cyan_ink()})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let b = vectorcraft_render::Renderer::new().render(&doc, 100, 100, Affine::IDENTITY, &RenderOptions::default());
    // A pixel the glyph covers.
    let (x, y) = (0..100u32).flat_map(|y| (0..100u32).map(move |x| (x, y))).find(|&(x, y)| cyan(b.pixel(x, y))).expect("the glyph draws");
    s.execute("object.setOverprint", &json!({"fill": true, "ids": [t.0]})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    assert!(cyan(pixel(&doc, false, x, y)) && green(pixel(&doc, true, x, y)), "{:?}", pixel(&doc, true, x, y));
}

#[test]
fn old_documents_with_an_overprint_black_list_still_preview() {
    let mut s = session();
    rect(&mut s, [0.0, 0.0, 100.0, 100.0], cyan_ink(), None, 0.0);
    let k = rect(&mut s, [25.0, 25.0, 50.0, 50.0], black_ink(), Some(cyan_ink()), 1.0);
    // How older versions stored Overprint Black.
    let mut old = (*s.doc().unwrap().doc).clone();
    old.unknown.insert("overprintBlack".into(), json!([k.0]));
    let bytes = vectorcraft_format::save(&old, false);
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains("\"overprintBlack\"") && !text.contains("\"overprint\""));

    let doc = vectorcraft_format::load(&bytes).unwrap();
    assert!(!doc.unknown.contains_key("overprintBlack"), "migrated on load");
    let n = doc.node(k).unwrap();
    assert!(n.appearance.fill().unwrap().overprint && !n.appearance.stroke().unwrap().overprint, "the black fill, not the cyan stroke");
    let over = pixel(&doc, true, 50, 50);
    assert!(over[2] as i32 > over[0] as i32 + 10 && over[2] <= 128, "black overprints the cyan: {over:?}");
    let knock = pixel(&doc, false, 50, 50);
    assert!((knock[0] as i32 - knock[2] as i32).abs() < 8, "knockout without the preview: {knock:?}");

    // The flags round-trip through the native format.
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert_eq!(back.node(k).unwrap().appearance, n.appearance);
}

fn overprints(s: &Session, id: NodeId) -> (bool, bool) {
    let a = &node(s, id).appearance;
    (a.fill().unwrap().overprint, a.stroke().unwrap().overprint)
}

#[test]
fn overprint_black_options() {
    let mut s = session();
    let k = rect(&mut s, [0.0, 0.0, 10.0, 10.0], black_ink(), Some(black_ink()), 1.0);
    let rich = rect(&mut s, [20.0, 0.0, 10.0, 10.0], ink(60, 40, 40, 100), Some(ink(0, 0, 0, 80)), 1.0);
    let rgb = rect(&mut s, [40.0, 0.0, 10.0, 10.0], json!("#000000"), Some(cyan_ink()), 1.0);
    let ids = json!([k.0, rich.0, rgb.0]);

    let r = s.execute("edit.colors.overprintBlack", &json!({"ids": ids, "fill": false})).unwrap();
    assert_eq!(r["changed"], 1);
    assert_eq!(overprints(&s, k), (false, true), "fill: false marks strokes only");
    s.execute("edit.colors.overprintBlack", &json!({"ids": ids})).unwrap();
    assert_eq!(overprints(&s, k), (true, true));
    assert_eq!(overprints(&s, rich), (false, false), "rich black and 80% K stay out at 100%");
    assert_eq!(overprints(&s, rgb), (false, false), "RGB black is not black ink");

    let before = undo_len(&s);
    s.execute("edit.colors.overprintBlack", &json!({"ids": ids, "includeCmyBlacks": true})).unwrap();
    assert_eq!(overprints(&s, rich), (true, false), "rich black included with CMY");
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    s.execute("edit.colors.overprintBlack", &json!({"ids": ids, "percentage": 75})).unwrap();
    assert_eq!(overprints(&s, rich), (true, true), "80% K at 75%");

    s.execute("edit.colors.overprintBlack", &json!({"ids": ids, "remove": true, "stroke": false, "includeCmyBlacks": true})).unwrap();
    assert_eq!((overprints(&s, k), overprints(&s, rich)), ((false, true), (false, true)), "remove from fills only");
    let before = undo_len(&s);
    let r = s.execute("edit.colors.overprintBlack", &json!({"ids": ids, "remove": true, "stroke": false})).unwrap();
    assert_eq!((r["changed"].clone(), undo_len(&s)), (json!(0), before), "nothing to change, no undo step");
}

#[test]
fn overprint_black_covers_type_gradients_spots_and_groups() {
    let mut s = session();
    let t = NodeId(s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hi"})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [t.0], "color": black_ink()})).unwrap();
    let g = rect(&mut s, [0.0, 0.0, 10.0, 10.0], black_ink(), None, 0.0);
    let stops = json!([{"offset": 0, "color": black_ink()}, {"offset": 1, "color": ink(50, 0, 0, 100)}]);
    s.execute("paint.setFill", &json!({"ids": [g.0], "gradient": {"stops": stops}})).unwrap();
    let spot = rect(&mut s, [20.0, 0.0, 10.0, 10.0], black_ink(), None, 0.0);
    let name =
        s.execute("swatch.new", &json!({"name": "Press Black", "color": black_ink(), "spot": true})).unwrap()["name"].as_str().unwrap().to_string();
    s.execute("paint.setFill", &json!({"ids": [spot.0], "swatch": name})).unwrap();
    s.execute("select.set", &json!({"ids": [t.0, g.0, spot.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();

    s.execute("edit.colors.overprintBlack", &json!({})).unwrap();
    let NodeKind::Text(tx) = &node(&s, t).kind else { panic!() };
    assert!(tx.runs.iter().all(|r| r.style.overprint_fill), "type characters, inside the selected group");
    let fill = |s: &Session, id| node(s, id).appearance.fill().unwrap().overprint;
    assert!(!fill(&s, g), "a gradient with a rich-black stop");
    assert!(!fill(&s, spot), "spot blacks need includeSpotBlacks");
    s.execute("edit.colors.overprintBlack", &json!({"includeCmyBlacks": true, "includeSpotBlacks": true})).unwrap();
    assert!(fill(&s, g) && fill(&s, spot));
}
