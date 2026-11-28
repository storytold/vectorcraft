//! `.cmx` files: Open and Place, by extension and by content, mapped to artboards, layers, groups,
//! paths, solid paint and strokes, with their warnings and refusals; and export, read back.

use super::*;
use craft_cdr::synth_cmx::*;
use serde_json::json;
use vectorcraft_doc::{Node, NodeKind};

fn paths(nodes: &[std::sync::Arc<Node>], out: &mut Vec<Node>) {
    for n in nodes {
        match &n.kind {
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => paths(children, out),
            _ => out.push((**n).clone()),
        }
    }
}

fn warnings(r: &serde_json::Value) -> Vec<String> {
    r["warnings"].as_array().map(|w| w.iter().filter_map(|w| w.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

#[test]
fn a_cmx_file_opens_as_editable_art() {
    let mut s = Session::new();
    let r = open_bytes(&mut s, "drawing.cmx", &sample(), None).unwrap();
    assert_eq!(r["format"], "cmx");
    assert!(warnings(&r).is_empty(), "{r}");
    let doc = &s.doc().unwrap().doc;
    // One 210 x 297 mm page.
    assert_eq!(doc.artboards.len(), 1);
    let ab = doc.artboards[0].rect;
    assert_eq!((ab.width().round(), ab.height().round()), (595.0, 842.0));
    // Bottom to top: the layer drawn first is at the bottom.
    let names: Vec<String> = doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["Art", "Top"]);
    let art = doc.layers[0].children().unwrap();
    assert_eq!(art.len(), 2);
    assert!(matches!(art[1].kind, NodeKind::Group { .. }), "the group drawn after the red square is above it");
    let mut all = Vec::new();
    paths(&doc.layers, &mut all);
    assert_eq!(all.len(), 4);
    // The red square: 100 mm on a side, centered on the page, with a fill and a stroke.
    let red = &all[0];
    let b = doc.bounds_of(&[red.id], false).unwrap();
    let mm = 72.0 / 25.4;
    assert!((b.width() - 100.0 * mm).abs() < 0.01 && (b.center().x - ab.center().x).abs() < 0.01, "{b:?}");
    assert_eq!(red.appearance.items.len(), 2);
    // The square whose line type draws no outline has a fill only.
    assert_eq!(all[1].appearance.items.len(), 1);
}

#[test]
fn detection_by_content_and_place() {
    let bytes = sample();
    let mut s = Session::new();
    // A renamed file is found by its content, and a .cmx extension alone reads it as one.
    assert_eq!(open_bytes(&mut s, "renamed.bin", &bytes, None).unwrap()["format"], "cmx");
    assert_eq!(detect("drawing.png", &bytes).map(|f| f.id), Some("cmx"));
    assert_eq!(detect("drawing.cmx", b"").map(|f| f.id), Some("cmx"));
    s.execute("file.new", &json!({"width":100,"height":100})).unwrap();
    s.execute("file.place", &json!({"name":"art.cmx", "dataBase64":vectorcraft_format::base64_encode(&bytes)})).unwrap();
    assert!(s.doc().unwrap().doc.layers.iter().any(|l| !l.children().unwrap().is_empty()));
    let f = format("cmx").unwrap();
    assert!(f.read && f.write);
    assert!(OPEN_EXTS.contains(&"cmx") && PLACE_EXTS.contains(&"cmx") && UNASSOCIATED_EXTS.contains(&"cmx"));
    assert!(!SAVE_FORMATS.contains(&"cmx"));
}

#[test]
fn what_is_left_out_is_named_in_the_warnings() {
    let mut children = vec![cont(b'4', 1e-7)];
    children.push(page(&[
        begin_page(None),
        begin_layer("L", None),
        instr(66, &fields(&[field(1, &[0; 8])])),
        square(&attrs_fill_type(2), 0, 0, 1000),
        square(&attrs(Some(1), Some(1), true), 0, 0, 1000),
    ]));
    children.extend(tables().into_iter().filter(|t| !t.starts_with(b"rota")));
    children.push(arrowheads(&[(1, 0)]));
    let mut s = Session::new();
    let w = warnings(&open_bytes(&mut s, "drawing.cmx", &riff(&children), None).unwrap());
    assert_eq!(
        w,
        [
            "CMX text, images, rectangles, ellipses, clipping paths and other objects VectorCraft doesn't read yet were left out",
            "CMX fills other than uniform colors were left out",
            "CMX outline arrowheads were left out",
            "CMX lenses were not applied",
        ]
    );
}

#[test]
fn a_file_the_open_left_things_out_of_is_not_written_over_unasked() {
    let d = std::env::temp_dir().join(format!("vc-cmx-loss-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let src = d.join("drawing.cmx");
    let mut children = vec![cont(b'4', 1e-7), page(&[begin_page(None), square(&attrs_fill_type(2), 0, 0, 1000)])];
    children.extend(tables());
    let original = riff(&children);
    std::fs::write(&src, &original).unwrap();
    let mut s = Session::new();
    s.execute("document.open", &json!({ "path": src.to_string_lossy() })).unwrap();
    let e = s.execute("document.export", &json!({ "path": src.to_string_lossy(), "format": "svg" })).unwrap_err().to_string();
    assert!(e.contains("acknowledgeLoss"), "{e}");
    assert_eq!(std::fs::read(&src).unwrap(), original);
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn files_that_are_not_read_say_why() {
    let mut s = Session::new();
    let mut rifx = sample();
    rifx[..4].copy_from_slice(b"RIFX");
    let e = open_bytes(&mut s, "a.cmx", &rifx, None).unwrap_err().to_string();
    assert!(e.contains("this CMX file can't be read (big-endian CMX files are not read)"), "{e}");
    let e = open_bytes(&mut s, "b.cmx", &riff(&[cont(b'2', 1e-3)]), None).unwrap_err().to_string();
    assert!(e.contains("this CMX file can't be read (CMX files with 16-bit coordinates are not read)"), "{e}");
    let e = open_bytes(&mut s, "c.cmx", &riff(&[]), None).unwrap_err().to_string();
    assert!(e.contains("this CMX file can't be read (damaged CMX header)"), "{e}");
    let e = open_bytes(&mut s, "d.cmx", b"not a cmx file", None).unwrap_err().to_string();
    assert!(e.contains("this CMX file can't be read (not a CMX file)"), "{e}");
}

fn b64(v: &serde_json::Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// Two artboards of 200 x 100 pt. The first layer holds a CMYK rectangle with a dashed gray stroke on
/// the first; layer "Front" a 40 % tint of spot ink "Gold" on the first and an RGB stroke on the
/// second.
fn drawn() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 2, "colorMode": "cmyk"})).unwrap();
    s.execute("swatch.new", &json!({"name": "Gold", "color": {"model": "cmyk", "c": 0.0, "m": 0.2, "y": 0.8, "k": 0.1}, "spot": true})).unwrap();
    let second = s.doc().unwrap().doc.artboards[1].rect.x0;
    s.execute("shape.rectangle", &json!({"x": 10, "y": 20, "width": 40, "height": 30})).unwrap();
    s.execute("paint.setFill", &json!({"color": {"model": "cmyk", "c": 0.6, "m": 0.1, "y": 0.0, "k": 0.2}})).unwrap();
    s.execute("paint.setStroke", &json!({"color": {"model": "gray", "k": 0.5}})).unwrap();
    s.execute("stroke.set", &json!({"weight": 2, "dash": [6, 2]})).unwrap();
    s.execute("layer.new", &json!({"name": "Front"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("paint.setFill", &json!({"swatch": "Gold", "tint": 40})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("shape.rectangle", &json!({"x": second + 5.0, "y": 5, "width": 10, "height": 10})).unwrap();
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#3366cc"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 1})).unwrap();
    s
}

/// Every path of a document with its bounds relative to the artboard it is on.
fn placed(s: &Session) -> Vec<(Node, vectorcraft_geom::Rect)> {
    let doc = &s.doc().unwrap().doc;
    let mut all = Vec::new();
    paths(&doc.layers, &mut all);
    all.into_iter()
        .map(|n| {
            let b = doc.bounds_of(&[n.id], false).unwrap();
            let board = doc.artboards.iter().find(|a| a.rect.contains(b.center())).map_or(vectorcraft_geom::Rect::ZERO, |a| a.rect);
            (n, b - board.origin().to_vec2())
        })
        .collect()
}

#[test]
fn an_exported_cmx_file_opens_with_its_pages_layers_colors_and_strokes() {
    let mut s = drawn();
    let r = s.execute("document.export", &json!({"format": "cmx", "useArtboards": true})).unwrap();
    assert_eq!(r["format"], "cmx");
    assert!(warnings(&r).is_empty(), "{r}");
    let bytes = b64(&r);
    assert!(craft_cdr::is_cmx(&bytes), "one file holds both pages");
    let mut back = Session::new();
    let opened = open_bytes(&mut back, "drawing.cmx", &bytes, None).unwrap();
    assert!(warnings(&opened).is_empty(), "{opened}");
    let doc = &back.doc().unwrap().doc;
    let sizes: Vec<(f64, f64)> = doc.artboards.iter().map(|a| (a.rect.width().round(), a.rect.height().round())).collect();
    assert_eq!(sizes, [(200.0, 100.0), (200.0, 100.0)]);
    let names: Vec<String> = doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    let was: Vec<String> = s.doc().unwrap().doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, was, "the layers keep their names and order");
    let was = placed(&s);
    let now = placed(&back);
    assert_eq!(was.len(), now.len());
    for ((_, a), (_, b)) in was.iter().zip(&now) {
        assert!((a.x0 - b.x0).abs() < 0.01 && (a.y0 - b.y0).abs() < 0.01 && (a.x1 - b.x1).abs() < 0.01, "{a:?} {b:?}");
    }
    let fill = |n: &Node| n.appearance.fill().map(|f| f.paint.clone());
    let stroke = |n: &Node| n.appearance.stroke().cloned();
    use vectorcraft_color::{Color, Paint};
    let cmyk = &now[0].0;
    assert!(matches!(fill(cmyk).and_then(|p| p.color()), Some(Color::Cmyk { c, .. }) if (c - 0.6).abs() < 0.006), "{:?}", fill(cmyk));
    let st = stroke(cmyk).unwrap();
    assert!((st.width - 2.0).abs() < 1e-3 && matches!(st.paint.color(), Some(Color::Gray { k }) if (k - 0.5).abs() < 0.005));
    // Widths are stored to 0.1 µm.
    let dash = st.dash.as_ref().map(|d| d.pattern.clone()).unwrap_or_default();
    assert!(dash.len() == 2 && (dash[0] - 6.0).abs() < 1e-3 && (dash[1] - 2.0).abs() < 1e-3, "{dash:?}");
    let gold = &now[1].0;
    assert!(matches!(fill(gold), Some(Paint::Solid { swatch: Some(n), tint, .. }) if n == "Gold" && (tint - 0.4).abs() < 0.006), "{:?}", fill(gold));
    assert!(doc.swatch("Gold").is_some_and(|g| g.spot));
    let line = &now[2].0;
    assert!(fill(line).is_none_or(|p| p == Paint::None));
    // The document converted the RGB color to its CMYK mode; the file keeps the CMYK values.
    let (Some(Color::Cmyk { c, m, y, k }), Some(Color::Cmyk { c: c2, m: m2, y: y2, k: k2 })) =
        (stroke(&was[2].0).unwrap().paint.color(), stroke(line).unwrap().paint.color())
    else {
        panic!("not CMYK: {:?}", stroke(line))
    };
    assert!([c - c2, m - m2, y - y2, k - k2].iter().all(|d| d.abs() < 0.006));
}

#[test]
fn one_artboard_is_one_page_and_what_cmx_leaves_out_is_named() {
    let mut s = drawn();
    let bytes = b64(&s.execute("document.export", &json!({"format": "cmx", "artboard": 1})).unwrap());
    let d = craft_cdr::read_cmx(&bytes, craft_cdr::Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    let shapes: usize = d.pages[0].layers.iter().map(|l| l.objects.len()).sum();
    assert_eq!(shapes, 1, "only the art over the second artboard");
    assert_eq!(b64(&s.execute("document.serialize", &json!({"format": "cmx", "artboard": 1})).unwrap()), bytes);
    // A gradient is written as one color, and a placed image is left out.
    s.execute("shape.rectangle", &json!({"x": 10, "y": 60, "width": 10, "height": 10})).unwrap();
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    let png = vectorcraft_format::base64_encode(&tiny_png());
    s.execute("file.place", &json!({"name": "dot.png", "dataBase64": png, "x": 150, "y": 60})).unwrap();
    let w = warnings(&s.execute("document.export", &json!({"format": "cmx"})).unwrap());
    assert!(w.iter().any(|w| w.contains("gradients are written as one colour")), "{w:?}");
    assert!(w.iter().any(|w| w == "placed images are left out of CMX files"), "{w:?}");
    assert!(w.iter().all(|w| !w.contains("edge of a page")), "{w:?}");
    // Art across the artboard's edge is written whole, and the warnings say so.
    s.execute("shape.rectangle", &json!({"x": 190, "y": 90, "width": 20, "height": 20})).unwrap();
    let w = warnings(&s.execute("document.export", &json!({"format": "cmx"})).unwrap());
    assert!(w.iter().any(|w| w.contains("edge of a page")), "{w:?}");
}

#[test]
fn a_nonzero_fill_that_would_fill_differently_even_odd_is_named() {
    let nonzero = |w: &[String]| w.iter().any(|w| w.contains("nonzero"));
    let export = |d: &str, rule: &str| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        s.execute("path.create", &json!({"d": d})).unwrap();
        s.execute("paint.setFill", &json!({"color": "#cc3333"})).unwrap();
        s.execute("path.setFillRule", &json!({"rule": rule})).unwrap();
        let r = s.execute("document.export", &json!({"format": "cmx"})).unwrap();
        let shapes: Vec<craft_cdr::model::Shape> = craft_cdr::read_cmx(&b64(&r), craft_cdr::Limits::default())
            .unwrap()
            .pages
            .into_iter()
            .flat_map(|p| p.layers)
            .flat_map(|l| l.objects)
            .filter_map(|o| match o {
                craft_cdr::model::Object::Shape(s) => Some(*s),
                _ => None,
            })
            .collect();
        (warnings(&r), shapes)
    };
    // Two squares that overlap and turn the same way leave a hole where they overlap even-odd.
    let overlapping = "M10 10 L50 10 L50 50 L10 50 Z M30 30 L70 30 L70 70 L30 70 Z";
    let (w, shapes) = export(overlapping, "nonZero");
    assert!(nonzero(&w), "{w:?}");
    assert!(shapes.len() == 1 && shapes[0].fill.is_some(), "{shapes:?}");
    // The same shape filled even-odd is written as it fills.
    assert!(!nonzero(&export(overlapping, "evenOdd").0));
    // A counter that turns the other way inside its shape fills the same either way.
    let counter = "M10 10 L50 10 L50 50 L10 50 Z M20 20 L20 40 L40 40 L40 20 Z";
    assert!(!nonzero(&export(counter, "nonZero").0));
    // Type exported from a document gives no warning about the fill rule.
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Bob 80"})).unwrap();
    let w = warnings(&s.execute("document.export", &json!({"format": "cmx"})).unwrap());
    assert!(!nonzero(&w), "{w:?}");
}

#[test]
fn a_miter_limit_the_file_cannot_hold_is_named() {
    let miter = |w: &[String]| w.iter().any(|w| w.contains("miter limit"));
    let export = |join: &str, limit: f64| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 20, "width": 40, "height": 30})).unwrap();
        s.execute("paint.setStroke", &json!({"color": "#3366cc"})).unwrap();
        s.execute("stroke.set", &json!({"weight": 4, "join": join, "miterLimit": limit})).unwrap();
        warnings(&s.execute("document.export", &json!({"format": "cmx"})).unwrap())
    };
    assert!(miter(&export("miter", 2.0)));
    // The limit a stroke opens with again, and corners that are not mitered, lose nothing.
    assert!(!miter(&export("miter", 10.0)));
    assert!(!miter(&export("round", 2.0)));
}

/// A 1 x 1 PNG.
fn tiny_png() -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

#[test]
fn a_layer_without_a_name_is_named_by_its_place() {
    // An empty layer, which is left out, then a layer named only by spaces: it comes in unnamed,
    // and the importer numbers it by its place among the layers it keeps, as for a `.cdr` drawing.
    let p = page(&[
        begin_page(None),
        begin_layer("Grid", None),
        end_layer(),
        begin_layer("  ", None),
        square(&attrs(Some(2), None, false), 0, 0, 1000),
        end_layer(),
    ]);
    let mut children = vec![cont(b'4', 1e-7), p];
    children.extend(tables());
    let mut s = Session::new();
    let r = open_bytes(&mut s, "drawing.cmx", &riff(&children), None).unwrap();
    assert!(warnings(&r).is_empty(), "{r}");
    let names: Vec<String> = s.doc().unwrap().doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["Layer 1"]);
}
