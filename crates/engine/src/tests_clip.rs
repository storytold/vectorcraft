//! Clipping sets with any clip shape (M3.26): Make Clipping Mask takes a compound path or text on
//! top and Release gives it back; compound holes, even-odd fills, text outlines and imported
//! multi-shape clip paths clip on screen, and the SVG and PDF output clip the same way.

use serde_json::{Value, json};
use vectorcraft_render::Rendered;

use super::*;

pub(crate) fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s
}

pub(crate) fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn ellipse(s: &mut Session, x: f64, w: f64) -> NodeId {
    id_of(&s.execute("shape.ellipse", &json!({"x": x, "y": x, "width": w, "height": w})).unwrap())
}

pub(crate) fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

/// The active document's 100×100 artboard, rendered transparent.
pub(crate) fn render(s: &Session) -> Rendered {
    vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 100, 100, Affine::IDENTITY, &Default::default())
}

pub(crate) fn opaque(r: &Rendered, x: u32, y: u32) -> bool {
    r.pixel(x, y)[3] > 128
}

/// The document exported as `format` and opened again, rendered.
fn reopened(s: &mut Session, format: &str) -> Rendered {
    let data = s.execute("document.export", &json!({"format": format})).unwrap()["dataBase64"].clone();
    let mut o = Session::new();
    o.execute("document.open", &json!({"name": format!("clip.{format}"), "dataBase64": data})).unwrap();
    render(&o)
}

/// The same probes are opaque on screen and in the SVG and PDF output.
pub(crate) fn assert_outputs_agree(s: &mut Session, probes: &[(u32, u32)]) {
    let screen = render(s);
    for format in ["svg", "pdf"] {
        let out = reopened(s, format);
        for &(x, y) in probes {
            assert_eq!(opaque(&out, x, y), opaque(&screen, x, y), "{format} at ({x}, {y})");
        }
    }
}

/// The leaf a click at (`x`, `y`) hits.
pub(crate) fn hit(s: &Session, x: f64, y: f64) -> Option<NodeId> {
    vectorcraft_doc::hit::hit_test(&s.doc().unwrap().doc, vectorcraft_geom::Point::new(x, y), Default::default()).map(|h| h.leaf)
}

/// Art under the whole artboard, then a donut (compound path with a real hole) on top.
fn art_and_donut(s: &mut Session) -> (NodeId, NodeId) {
    let art = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let (outer, inner) = (ellipse(s, 10.0, 80.0), ellipse(s, 35.0, 30.0));
    select(s, &[inner]);
    s.execute("path.reverse", &json!({})).unwrap();
    select(s, &[outer, inner]);
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    let donut = s.doc().unwrap().selection.objects[0];
    (art, donut)
}

#[test]
fn compound_clip_keeps_its_hole_and_release_gives_the_compound_back() {
    let mut s = session();
    let (art, donut) = art_and_donut(&mut s);
    select(&mut s, &[art, donut]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    let d = &s.doc().unwrap().doc;
    let NodeKind::Group { children, clip: true } = &d.node(g).unwrap().kind else { panic!("a clip group") };
    assert_eq!(children[0].id, donut);
    assert!(matches!(children[0].kind, NodeKind::Compound { .. }) && children[0].appearance.fill_paint().is_none());
    let r = render(&s);
    assert!(opaque(&r, 20, 50), "the ring shows the art");
    assert!(!opaque(&r, 50, 50), "the hole stays transparent");
    assert!(!opaque(&r, 3, 3), "outside the ring");
    assert_outputs_agree(&mut s, &[(20, 50), (50, 50), (3, 3), (80, 50)]);
    // Clicks hit the art only where it shows.
    assert_eq!(hit(&s, 20.0, 50.0), Some(art));
    assert_eq!(hit(&s, 50.0, 50.0), None, "the hole");
    // Release: a plain group, the compound path back on top of the art, unpainted.
    select(&mut s, &[g]);
    s.execute("object.clippingMask.release", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    let NodeKind::Group { children, clip: false } = &d.node(g).unwrap().kind else { panic!("a plain group") };
    assert!(matches!(children[0].kind, NodeKind::Compound { .. }) && children[0].id == donut);
    assert!(opaque(&render(&s), 50, 50), "nothing clips any more");
}

#[test]
fn text_on_top_clips_by_its_glyphs() {
    let mut s = session();
    let art = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let t = id_of(&s.execute("text.create", &json!({"x": 5, "y": 80, "text": "O", "size": 90})).unwrap());
    let b = s.doc().unwrap().doc.node(t).unwrap().geometric_bounds().unwrap();
    select(&mut s, &[art, t]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    let d = &s.doc().unwrap().doc;
    let NodeKind::Text(text) = &d.node(t).unwrap().kind else { panic!("still text") };
    assert!(text.runs.iter().all(|r| r.style.fill.is_none() && r.style.stroke.is_none()), "the clipping path loses its paint");
    // Across the middle of the "O": clear, ring, the counter (clear), ring, clear.
    let cy = ((b.y0 + b.y1) / 2.0) as u32;
    let r = render(&s);
    let row: Vec<bool> = (0..100).map(|x| opaque(&r, x, cy)).collect();
    assert_eq!(row.windows(2).filter(|w| w[0] != w[1]).count(), 4, "{row:?}");
    let probes: Vec<(u32, u32)> = (0..100).step_by(3).map(|x| (x, cy)).collect();
    assert_outputs_agree(&mut s, &probes);
    // Text clips clicks by its frame.
    assert_eq!(hit(&s, (b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0), Some(art));
    assert!(b.x1 < 95.0, "{b:?}");
    assert_eq!(hit(&s, b.x1 + 3.0, (b.y0 + b.y1) / 2.0), None, "beside the text");
    // Release gives the text back.
    select(&mut s, &[g]);
    s.execute("object.clippingMask.release", &json!({})).unwrap();
    assert!(matches!(s.doc().unwrap().doc.node(t).unwrap().kind, NodeKind::Text(_)));
}

#[test]
fn make_refuses_a_top_object_that_cannot_clip() {
    let mut s = session();
    let art = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let (a, b) = (ellipse(&mut s, 10.0, 20.0), ellipse(&mut s, 50.0, 20.0));
    select(&mut s, &[a, b]);
    s.execute("object.group", &json!({})).unwrap();
    let group = s.doc().unwrap().selection.objects[0];
    select(&mut s, &[art, group]);
    let err = s.execute("object.clippingMask.make", &json!({})).unwrap_err();
    assert!(err.to_string().contains("path, compound path, compound shape or text"), "{err}");
}

/// An SVG clip path made of several shapes (imported as one multi-subpath clipping path) and an
/// even-odd star clip path.
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100">
<defs>
  <clipPath id="two"><rect x="0" y="0" width="30" height="40"/><rect x="60" y="0" width="30" height="40"/></clipPath>
  <clipPath id="star"><path clip-rule="evenodd" d="M70 49 L84.7 94.2 L46.2 66.3 L93.8 66.3 L55.3 94.2 Z"/></clipPath>
</defs>
<g clip-path="url(#two)"><rect x="0" y="0" width="100" height="45" fill="#000"/></g>
<g clip-path="url(#star)"><rect x="40" y="45" width="60" height="55" fill="#000"/></g>
</svg>"##;

#[test]
fn imported_multi_shape_and_even_odd_clip_paths_clip() {
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "clips.svg", "dataBase64": vectorcraft_format::base64_encode(SVG.as_bytes())})).unwrap();
    let d = &s.doc().unwrap().doc;
    let mut kinds = vec![];
    d.walk(|n| {
        if let NodeKind::Group { children, clip: true } = &n.kind {
            kinds.push(children[0].kind_label());
        }
    });
    assert_eq!(kinds.len(), 2, "two clip groups");
    let r = render(&s);
    assert!(opaque(&r, 15, 20) && opaque(&r, 75, 20), "inside both shapes of the clip path");
    assert!(!opaque(&r, 45, 20), "between them");
    assert!(!opaque(&r, 15, 43), "below them");
    // The star's arms cross: the centre of the crossing is outside under even-odd.
    assert!(opaque(&r, 70, 56), "an arm");
    assert!(!opaque(&r, 70, 75), "the even-odd centre");
    assert_outputs_agree(&mut s, &[(15, 20), (75, 20), (45, 20), (15, 43), (70, 56), (70, 75)]);
}
