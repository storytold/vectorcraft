//! Save As › `.ai` with `aiVersion: "7"`: legacy `.ai` files of version 7, one per artboard, that open with
//! their layers and groups (#868).

use serde_json::{Value, json};

use crate::tests_links::Folder;
use crate::tests_menucmds::{place_undecodable_png, place_undecodable_png_sized};
use crate::*;

/// Three artboards (named A, B and C) with a rectangle on each, on two layers: the first
/// rectangle's group (with a second one) on `Layer 1`, the others on `Top`.
fn three_boards() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 0, "name": "A"})).unwrap();
    for (i, name) in ["B", "C"].iter().enumerate() {
        let x = 200.0 * (i + 1) as f64;
        s.execute("artboard.new", &json!({"x": x, "y": 0, "width": 100, "height": 100, "name": name})).unwrap();
    }
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 50, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("layer.new", &json!({"name": "Top"})).unwrap();
    for x in [210, 410] {
        s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 30, "height": 30})).unwrap();
    }
    s
}

fn layer_names(d: &vectorcraft_doc::Document) -> Vec<String> {
    d.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect()
}

fn warnings(r: &Value) -> Vec<String> {
    r["warnings"].as_array().map(|w| w.iter().filter_map(|w| w.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

#[test]
fn each_artboard_is_a_file_that_opens_with_its_layers() {
    let dir = Folder::new("ai8");
    let mut s = three_boards();
    let path = dir.file("Poster.ai");
    let r = s.execute("file.saveAs", &json!({"path": path, "format": "ai", "aiVersion": "7"})).unwrap();
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert_eq!(files, [dir.file("Poster-A.ai"), dir.file("Poster-B.ai"), dir.file("Poster-C.ai")], "{r}");
    assert!(warnings(&r).iter().any(|w| w.contains("file of its own")), "{r}");
    for (f, objects) in [(files[0], 1), (files[1], 1), (files[2], 1)] {
        let bytes = std::fs::read(f).unwrap();
        assert!(bytes.starts_with(b"%!PS-Adobe-3.0\n"), "{f}");
        s.execute("document.open", &json!({"path": f})).unwrap();
        let d = &s.doc().unwrap().doc;
        assert_eq!(d.artboards.len(), 1, "{f}");
        assert_eq!(layer_names(d), ["Layer 1", "Top"], "{f}");
        let count: usize = d.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum();
        assert_eq!(count, objects, "{f}");
    }
    // Artboard A's file: the group of two rectangles in Layer 1, nothing on Top.
    s.execute("document.open", &json!({"path": files[0]})).unwrap();
    let d = &s.doc().unwrap().doc;
    let group = &d.layers[0].children().unwrap()[0];
    assert!(matches!(&group.kind, vectorcraft_doc::NodeKind::Group { children, clip: false } if children.len() == 2), "{:?}", group.kind);
    assert!(d.layers[1].children().unwrap().is_empty());
}

#[test]
fn one_artboard_is_the_file_itself_and_save_writes_it_again() {
    let dir = Folder::new("ai8-one");
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    let path = dir.file("One.ai");
    let r = s.execute("file.saveAs", &json!({"path": path, "format": "ai", "aiVersion": "7"})).unwrap();
    assert!(r["files"].is_null(), "{r}");
    assert!(std::fs::read(&path).unwrap().starts_with(b"%!PS"));
    // Save remembers the version.
    std::fs::remove_file(&path).unwrap();
    s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 30, "height": 30})).unwrap();
    s.execute("document.save", &json!({})).unwrap();
    assert!(std::fs::read(&path).unwrap().starts_with(b"%!PS"));
}

#[test]
fn the_default_stays_pdf_compatible_and_other_versions_are_refused() {
    let mut s = three_boards();
    for p in [json!({"format": "ai"}), json!({"format": "ai", "aiVersion": "pdf"})] {
        let r = s.execute("file.saveCopy", &p).unwrap();
        let bytes = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
        assert!(bytes.starts_with(b"%PDF"), "{p}");
    }
    assert!(s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "3"})).is_err());
    let v = s.execute("file.formatOptions", &json!({"format": "ai"})).unwrap();
    assert_eq!(v["options"]["aiVersion"]["value"], "pdf", "{v}");
}

#[test]
fn transparency_and_appearances_are_written_as_plain_art_and_type_as_text() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 30, "y": 30, "width": 50, "height": 50})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 120, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    s.execute("text.create", &json!({"x": 100, "y": 80, "text": "Hi", "size": 24})).unwrap();
    let r = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7"})).unwrap();
    let w = warnings(&r);
    assert!(w.iter().any(|w| w.contains("written opaque")), "{w:?}");
    assert!(!w.iter().any(|w| w.contains("type") && w.contains("outlines")), "{w:?}");
    let bytes = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    // No transparency operators: version 7 has none.
    assert!(!text.contains(" Xy\n"), "{text}");
    s.execute("document.open", &json!({"name": "t.ai", "dataBase64": r["dataBase64"]})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.layers.iter().all(|l| l.children().is_some_and(|c| !c.is_empty())));
    let mut opaque = true;
    d.walk(|n| opaque &= n.opacity == 1.0);
    assert!(opaque);
    assert_eq!(texts(d), ["Hi"]);
}

/// The plain text of every text object of `d`.
fn texts(d: &vectorcraft_doc::Document) -> Vec<String> {
    let mut out = vec![];
    d.walk(|n| {
        if let vectorcraft_doc::NodeKind::Text(t) = &n.kind {
            out.push(t.plain_text());
        }
    });
    out
}

#[test]
fn type_can_be_written_as_outlines() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("text.create", &json!({"x": 20, "y": 50, "text": "Café", "size": 24})).unwrap();
    let r = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7", "outlineText": true})).unwrap();
    s.execute("document.open", &json!({"name": "t.ai", "dataBase64": r["dataBase64"]})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(texts(d).is_empty());
    let mut paths = 0;
    d.walk(|n| paths += usize::from(matches!(n.kind, vectorcraft_doc::NodeKind::Compound { .. } | vectorcraft_doc::NodeKind::Path { .. })));
    assert!(paths > 0);
    assert!(s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7", "outlineText": "yes"})).is_err());
    let v = s.execute("file.formatOptions", &json!({"format": "ai"})).unwrap();
    assert_eq!(v["options"]["outlineText"]["value"], false, "{v}");
}

#[test]
fn hidden_objects_go_to_a_hidden_layer_or_are_left_out_or_shown() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 60, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("object.hide", &json!({})).unwrap();
    let saved = |s: &mut Session, hidden: Value| {
        let r = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7", "hiddenObjects": hidden})).unwrap();
        s.execute("document.open", &json!({"name": "t.ai", "dataBase64": r["dataBase64"]})).unwrap();
        let d = s.doc().unwrap().doc.clone();
        s.execute("file.close", &json!({})).unwrap();
        (d, warnings(&r))
    };
    let (d, w) = saved(&mut s, Value::Null);
    assert_eq!(layer_names(&d), ["Layer 1", "Layer 1 (hidden)"], "{w:?}");
    assert!(d.layers[0].visible && !d.layers[1].visible);
    assert!(w.iter().any(|w| w.contains("hidden layer")), "{w:?}");
    let (d, _) = saved(&mut s, json!("omit"));
    assert_eq!(layer_names(&d), ["Layer 1"]);
    assert_eq!(d.layers[0].children().map(Vec::len), Some(1));
    let (d, _) = saved(&mut s, json!("visible"));
    assert_eq!(layer_names(&d), ["Layer 1"]);
    assert_eq!(d.layers[0].children().map(|c| c.iter().filter(|n| n.visible).count()), Some(2));
    assert!(s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7", "hiddenObjects": "behind"})).is_err());
    let v = s.execute("file.formatOptions", &json!({"format": "ai"})).unwrap();
    assert_eq!(v["options"]["hiddenObjects"]["value"], "layer", "{v}");
}

#[test]
fn art_under_transparent_art_stays_in_its_own_layer() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    s.execute("layer.new", &json!({"name": "Shade"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50, "blend": "multiply"})).unwrap();
    let r = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7"})).unwrap();
    s.execute("document.open", &json!({"name": "t.ai", "dataBase64": r["dataBase64"]})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(layer_names(d), ["Layer 1", "Shade"]);
    assert!(d.layers.iter().all(|l| l.children().is_some_and(|c| c.len() == 1)), "{:?}", d.layers);
}

/// #2687's file: a circle filled with a radial gradient from white to #1b2a6b, and a square filled
/// with a linear one.
const RADIAL_AND_LINEAR: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="200" viewBox="0 0 400 200">
  <defs>
    <radialGradient id="r" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#1b2a6b"/></radialGradient>
    <linearGradient id="l" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#1b2a6b"/></linearGradient>
  </defs>
  <circle cx="100" cy="100" r="80" fill="url(#r)"/>
  <rect x="220" y="20" width="160" height="160" fill="url(#l)"/>
</svg>"##;

/// A radial gradient is written with its highlight, `0 0 0 0 Bh` at the center (§3.5.5, §3.5.6).
/// #2687's file had none. The linear one has none. Both open again with their type, stops and
/// place.
#[test]
fn a_radial_gradient_is_written_with_its_highlight() {
    let mut s = Session::new();
    let svg = vectorcraft_format::base64_encode(RADIAL_AND_LINEAR.as_bytes());
    s.execute("document.open", &json!({"name": "t_radial.svg", "dataBase64": svg})).unwrap();
    let r = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7"})).unwrap();
    let text = String::from_utf8(vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(text.contains("\n%AI5_BeginGradient: (Gradient)\n(Gradient) 1 2 Bd\n[\n"), "{text}");
    assert!(text.contains("\nBb\n0 0 0 0 Bh\n1 (Gradient) 100 100 0 80 1 0 0 1 0 0 Bg\nf\n0 BB\n"), "{text}");
    assert!(text.contains("\nBb\n1 (Gradient 2) 220 180 0 160 1 0 0 1 0 0 Bg\nf\n0 BB\n"), "{text}");
    assert_eq!(text.matches(" Bh\n").count(), 1, "{text}");
    s.execute("document.open", &json!({"name": "t.ai", "dataBase64": r["dataBase64"]})).unwrap();
    let d = &s.doc().unwrap().doc;
    let mut found = vec![];
    d.walk(|n| {
        if let vectorcraft_color::Paint::Gradient(g) = n.appearance.fill_paint() {
            found.push(*g);
        }
    });
    let [radial, linear] = found.as_slice() else { panic!("{found:#?}") };
    let navy = vectorcraft_color::Color::rgb(27.0 / 255.0, 42.0 / 255.0, 107.0 / 255.0);
    use vectorcraft_color::GradientKind::{Linear, Radial};
    for (g, kind, start, end) in [(radial, Radial, (100.0, 100.0), (180.0, 100.0)), (linear, Linear, (220.0, 20.0), (380.0, 20.0))] {
        assert_eq!(g.gradient.kind, kind);
        let [a, b] = g.gradient.stops.as_slice() else { panic!("{g:?}") };
        assert_eq!((a.offset, b.offset, a.color), (0.0, 1.0, vectorcraft_color::Color::rgb(1.0, 1.0, 1.0)), "{g:?}");
        assert!(b.color.to_rgb().iter().zip(navy.to_rgb()).all(|(x, y)| (x - y).abs() < 1e-3), "{g:?}");
        let geom = g.geom.unwrap();
        assert!((geom.start.x - start.0).abs() < 1e-3 && (geom.start.y - start.1).abs() < 1e-3, "{geom:?}");
        assert!((geom.end.x - end.0).abs() < 1e-3 && (geom.end.y - end.1).abs() < 1e-3 && geom.focal.is_none(), "{geom:?}");
    }
    // White at the center, #1b2a6b at the edge.
    for ((x, y), want) in [((100.0, 100.0), [255, 255, 255]), ((100.0, 21.0), [27, 42, 107])] {
        let got = colour_at(d, x, y);
        assert!(got.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 8), "{got:?} at {x}, {y}");
    }
}

/// #2687's radial gradient lists its stops from white at ramp point 0 to #1b2a6b at 100, as
/// §3.5.2's radial definition lists them (10, then 100); the linear one lists them from 100 to 0
/// (§1.4.3). The reference app read the radial gradient's two stops listed from 100 both at ramp
/// point 100 and drew the circle in #1b2a6b (reported on #2807).
#[test]
fn a_radial_gradient_lists_its_stops_from_the_first() {
    let mut s = Session::new();
    let svg = vectorcraft_format::base64_encode(RADIAL_AND_LINEAR.as_bytes());
    s.execute("document.open", &json!({"name": "t_radial.svg", "dataBase64": svg})).unwrap();
    let r = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7"})).unwrap();
    let text = String::from_utf8(vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    // Each `%_Bs` line of a definition as its RGB color (0-255) and ramp point.
    let listed = |head: &str| -> Vec<([u8; 3], String)> {
        let def = text.split(head).nth(1).and_then(|t| t.split("BD\n").next()).unwrap_or_else(|| panic!("no {head:?} in\n{text}"));
        def.lines()
            .map(|l| {
                let n: Vec<&str> = l.split_whitespace().collect();
                let [_, _, _, _, r, g, b, "2", "50", ramp, "%_Bs"] = n.as_slice() else { panic!("{l}") };
                let c = [r, g, b].map(|v| (v.parse::<f64>().unwrap() * 255.0).round() as u8);
                (c, (*ramp).to_string())
            })
            .collect()
    };
    let (white, navy) = ([255, 255, 255], [27, 42, 107]);
    assert_eq!(listed("(Gradient) 1 2 Bd\n[\n"), [(white, "0".to_string()), (navy, "100".to_string())], "{text}");
    assert_eq!(listed("(Gradient 2) 0 2 Bd\n[\n"), [(navy, "100".to_string()), (white, "0".to_string())], "{text}");
}

/// A `w`×`h` PNG of one colour (`rgba`), 72 ppi.
fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut out = vec![];
    image::RgbaImage::from_pixel(w, h, image::Rgba(rgba)).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// A red square (10–50 × 10–50) half under a half-transparent black image (30–70 × 10–50), and
/// an opaque blue image (140–160 × 40–60).
fn shadow_over_square() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    for (bytes, at) in [(png(40, 40, [0, 0, 0, 128]), [50, 30]), (png(20, 20, [0, 0, 255, 255]), [150, 50])] {
        let p = json!({"name": "i.png", "dataBase64": vectorcraft_format::base64_encode(&bytes), "at": at});
        s.execute("file.place", &p).unwrap();
    }
    s
}

/// The colour `d` shows at (`x`, `y`), over white.
fn colour_at(d: &vectorcraft_doc::Document, x: f64, y: f64) -> [u8; 3] {
    let px = vectorcraft_render::Renderer::new().render_region(d, vectorcraft_geom::Rect::new(x, y, x + 1.0, y + 1.0), 1.0, true).pixels;
    [px[0], px[1], px[2]]
}

fn images(d: &vectorcraft_doc::Document) -> (usize, usize) {
    let (mut all, mut clipped) = (0, 0);
    d.walk(|n| {
        if let vectorcraft_doc::NodeKind::Group { children, clip: true } = &n.kind {
            clipped += children.iter().filter(|c| matches!(c.kind, vectorcraft_doc::NodeKind::Image(_))).count();
        }
        all += usize::from(matches!(n.kind, vectorcraft_doc::NodeKind::Image(_)));
    });
    (all, clipped)
}

#[test]
fn a_transparent_image_is_flattened_with_the_art_under_it_which_stays() {
    let mut s = shadow_over_square();
    let before = [colour_at(&s.doc().unwrap().doc, 40.0, 30.0), colour_at(&s.doc().unwrap().doc, 60.0, 30.0)];
    let r = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7"})).unwrap();
    assert!(warnings(&r).iter().any(|w| w.contains("flattened with the art under them")), "{r}");
    assert!(!warnings(&r).iter().any(|w| w.contains("transparent image pixels are written over white")), "{r}");
    s.execute("document.open", &json!({"name": "t.ai", "dataBase64": r["dataBase64"]})).unwrap();
    let d = &s.doc().unwrap().doc;
    // The square is still there, under an opaque image clipped to the transparent one's place;
    // the opaque image stays as it was.
    let mut squares = 0;
    d.walk(|n| squares += usize::from(matches!(n.kind, vectorcraft_doc::NodeKind::Path { clipping: false, .. })));
    assert_eq!(squares, 1);
    assert_eq!(images(d), (2, 1));
    let after = [colour_at(d, 40.0, 30.0), colour_at(d, 60.0, 30.0)];
    for (a, b) in before.iter().zip(&after) {
        assert!(a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 3), "{before:?} {after:?}");
    }
    assert_eq!(colour_at(d, 20.0, 30.0), [255, 0, 0], "the square outside the image");
    assert_eq!(colour_at(d, 80.0, 30.0), [255, 255, 255], "nothing beyond the image");
    // Run as a PostScript program (the file's prolog draws it), it prints the same: two images
    // and the square, on a page that is the art's box (10 pt in from the artboard's top left).
    s.execute("document.open", &json!({"name": "t.ai", "dataBase64": r["dataBase64"], "editingData": false})).unwrap();
    let page = &s.doc().unwrap().doc;
    assert_eq!(images(page).0, 2);
    let printed = [colour_at(page, 30.0, 20.0), colour_at(page, 50.0, 20.0)];
    for (a, b) in before.iter().zip(&printed) {
        assert!(a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 3), "{before:?} {printed:?}");
    }
    assert_eq!(colour_at(page, 10.0, 20.0), [255, 0, 0], "the square outside the image");
}

/// How many of reply `r`'s warnings say that an image couldn't be decoded, as raster exports do.
fn undecoded_notes(r: &Value) -> usize {
    warnings(r).iter().filter(|w| *w == "an image couldn't be decoded and is missing from the picture").count()
}

#[test]
fn a_flattened_image_that_cant_be_decoded_is_noted_once() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("artboard.new", &json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "B"})).unwrap();
    let cut = place_undecodable_png(&mut s);
    s.execute("transparency.set", &json!({"ids": [cut.0], "opacity": 50})).unwrap();
    // Both artboards flatten it.
    let r = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7"})).unwrap();
    assert_eq!(r["files"].as_array().map(Vec::len), Some(2), "{r}");
    assert!(warnings(&r).iter().any(|w| w.contains("flattened with the art under them")), "{r}");
    assert_eq!(undecoded_notes(&r), 1, "{r}");
}

/// Images that can't be decoded and are written as they are get the same note as one that is
/// flattened. All of them give one note, and the note names no image's key.
#[test]
fn images_written_or_flattened_that_cant_be_decoded_are_noted_once() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    place_undecodable_png(&mut s);
    place_undecodable_png_sized(&mut s, 48, 24);
    let keys: Vec<String> = s.doc().unwrap().doc.images.keys().cloned().collect();
    assert_eq!(keys.len(), 2);
    let decode_notes = |r: &Value| warnings(r).iter().filter(|w| w.contains("decoded")).count();
    let named = |r: &Value| warnings(r).iter().any(|w| keys.iter().any(|k| w.contains(k.as_str())));
    let r = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7"})).unwrap();
    assert_eq!((undecoded_notes(&r), decode_notes(&r), named(&r)), (1, 1, false), "{r}");
    // A third image, at 50% opacity, is flattened. The engine's note and the writer's note come out
    // as one warning.
    let cut = place_undecodable_png_sized(&mut s, 32, 16);
    s.execute("transparency.set", &json!({"ids": [cut.0], "opacity": 50})).unwrap();
    let r = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7"})).unwrap();
    assert!(warnings(&r).iter().any(|w| w.contains("flattened with the art under them")), "{r}");
    assert_eq!((undecoded_notes(&r), decode_notes(&r), named(&r)), (1, 1, false), "{r}");
}

#[test]
fn an_image_that_cant_be_decoded_with_an_expanded_effect_is_noted() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    let cut = place_undecodable_png(&mut s);
    s.execute("effect.apply", &json!({"effect": "blur.gaussian", "params": {"radius": 3}, "ids": [cut.0]})).unwrap();
    // Left out rather than flattened: only expanding the blur draws it.
    let r = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7", "transparentImages": "omit"})).unwrap();
    assert_eq!(undecoded_notes(&r), 1, "{r}");
}

#[test]
fn images_that_decode_give_no_decode_note() {
    let mut s = shadow_over_square();
    s.execute("effect.apply", &json!({"effect": "blur.gaussian", "params": {"radius": 3}})).unwrap(); // the blue image
    for mode in ["flatten", "omit"] {
        let r = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7", "transparentImages": mode})).unwrap();
        assert!(warnings(&r).iter().any(|w| w.contains("transparent images")), "{r}");
        assert_eq!(undecoded_notes(&r), 0, "{r}");
    }
}

#[test]
fn transparent_images_can_be_left_out_and_opacity_makes_one_transparent() {
    let mut s = shadow_over_square();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap(); // the blue image
    let r = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7", "transparentImages": "omit"})).unwrap();
    assert!(warnings(&r).iter().any(|w| w.contains("left out")), "{r}");
    s.execute("document.open", &json!({"name": "t.ai", "dataBase64": r["dataBase64"]})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(images(d), (0, 0));
    assert_eq!(colour_at(d, 40.0, 30.0), [255, 0, 0]);
    assert!(s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7", "transparentImages": "blur"})).is_err());
    let v = s.execute("file.formatOptions", &json!({"format": "ai"})).unwrap();
    assert_eq!(v["options"]["transparentImages"]["value"], "flatten", "{v}");
}

/// #2692's file, an image clipped to a circle inside a circle's stroke, and #2830's, an image with
/// see-through pixels (which the save flattens into an image inside a clipping group), each with
/// an orange square on the next layer.
const CLIPPED_IMAGE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="400" height="200" viewBox="0 0 400 200">
  <defs><clipPath id="c1"><circle cx="100" cy="100" r="70"/></clipPath></defs>
  <g id="clipped-image">
    <circle cx="100" cy="100" r="78" fill="none" stroke="#777777" stroke-width="4"/>
    <g clip-path="url(#c1)"><image x="20" y="20" width="160" height="160" xlink:href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAGklEQVR42mPMy8tjIAWwcHBwjGoY1TBMNQAAdtYC8NwTJdoAAAAASUVORK5CYII="/></g>
  </g>
  <g id="above"><rect x="220" y="20" width="160" height="160" fill="#f7941d"/></g>
</svg>"##;
const SEE_THROUGH_IMAGE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="400" height="200" viewBox="0 0 400 200">
  <g id="alpha-image">
    <image x="20" y="20" width="160" height="160" xlink:href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAeElEQVR42mNgoBAwoguUlZX9x6ehq6uLEacBdXV1/4mxtampiRHDgLa2tv+kOL2qqooRbkBfX99/cvxfVFTEyMLAwMDAwcFBdiCOGjAYDIAnpHPnzpGUFoyMjBgxkvK1a9eIMkRLS4sRZ2a6d+8eXkOUlJQYGagJAOoJHMzSF7FXAAAAAElFTkSuQmCC"/>
  </g>
  <g id="above"><rect x="220" y="20" width="160" height="160" fill="#f7941d"/></g>
</svg>"##;

/// #2692 and #2830: the image inside a clipping group, clipped in the file or flattened by the
/// save, is written as the version 7 specification shows one (§8.1, §8.4): the matrix with a space
/// inside each bracket and the samples on comment lines of 60 hex digits, between `W n` and `Q`.
/// A line `F`, which the specification's examples do not show, follows `%AI5_EndRaster`. The next
/// layer follows with the square's exact lines, and the file opens again with both layers, the
/// clipping group, the image, the square and, besides the clipping path, only the paths the SVG
/// holds.
#[test]
fn a_clipped_or_flattened_image_is_written_as_the_specification_shows_it() {
    for (svg, layer, paths) in [(CLIPPED_IMAGE, "clipped-image", 2), (SEE_THROUGH_IMAGE, "alpha-image", 1)] {
        let mut s = Session::new();
        s.execute("document.open", &json!({"name": "t.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
        let r = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7"})).unwrap();
        let bytes = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let xi: Vec<usize> = (0..lines.len()).filter(|i| lines[*i].ends_with(" XI")).collect();
        assert_eq!(xi.len(), 1, "{text}");
        let at = xi[0];
        assert_eq!(lines[at - 2..at], ["%AI5_File:", "%AI5_BeginRaster"], "{layer}");
        assert!(lines[at].starts_with("[ ") && lines[at].contains(" ] 0 0 "), "{}", lines[at]);
        let end = at + lines[at..].iter().position(|l| *l == "%AI5_EndRaster").unwrap();
        let (last, full) = lines[at + 1..end].split_last().unwrap();
        assert!(full.iter().all(|l| l.len() == 61) && (2..=61).contains(&last.len()), "{layer}");
        assert_eq!(lines[end + 1], "F", "{layer}");
        assert!(text.contains("\nW\nn\n%AI5_File:\n") && text.contains("\n%AI5_EndRaster\nF\nQ\nLB\n%AI5_EndLayer--\n"), "{text}");
        let square = "%AI5_BeginLayer\n1 1 1 1 0 0 1 255 79 79 Lb\n(above) Ln\n0 A\n0.9686 0.5804 0.1137 Xa\n0 O\n0 XR\n\
                      220 180 m\n380 180 L\n380 20 L\n220 20 L\n220 180 L\nf\nLB\n%AI5_EndLayer--\n%%PageTrailer\n";
        assert!(text.contains(square), "{text}");
        s.execute("document.open", &json!({"name": "t.ai", "dataBase64": r["dataBase64"]})).unwrap();
        let d = &s.doc().unwrap().doc;
        assert_eq!(layer_names(d), [layer, "above"]);
        assert_eq!(images(d), (1, 1), "{layer}");
        let mut drawn = 0;
        d.walk(|n| drawn += usize::from(matches!(n.kind, vectorcraft_doc::NodeKind::Path { clipping: false, .. })));
        assert_eq!(drawn, paths, "{layer}");
        assert_eq!(colour_at(d, 300.0, 100.0), [247, 148, 29], "{layer}");
        assert_ne!(colour_at(d, 100.0, 100.0), [255, 255, 255], "{layer}");
    }
}

#[test]
fn a_range_saves_those_artboards_and_save_writes_them_again() {
    let dir = Folder::new("ai7-range");
    let mut s = three_boards();
    let path = dir.file("Poster.ai");
    let r = s.execute("file.saveAs", &json!({"path": path, "format": "ai", "aiVersion": "7", "range": "1, 3"})).unwrap();
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert_eq!(files, [dir.file("Poster-A.ai"), dir.file("Poster-C.ai")], "{r}");
    // One artboard: the file itself. An empty range: every artboard.
    let r = s.execute("file.saveAs", &json!({"path": path, "format": "ai", "aiVersion": "7", "range": "2"})).unwrap();
    assert!(r["files"].is_null(), "{r}");
    let r = s.execute("file.saveAs", &json!({"path": path, "format": "ai", "aiVersion": "7", "range": ""})).unwrap();
    assert_eq!(r["files"].as_array().map(Vec::len), Some(3), "{r}");
    // Save keeps the range it was saved with.
    s.execute("file.saveAs", &json!({"path": path, "format": "ai", "aiVersion": "7", "range": "1-2"})).unwrap();
    let r = s.execute("document.save", &json!({})).unwrap();
    assert_eq!(r["files"].as_array().map(Vec::len), Some(2), "{r}");
}

/// A 32×32 PNG checkerboard of 16-pixel squares: `a` top left and bottom right, `b` elsewhere.
fn checker(a: [u8; 4], b: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(32, 32, |x, y| image::Rgba(if (x < 16) == (y < 16) { a } else { b }));
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// A blue square (40–168) under a square at `opacity` filled with a pattern whose 32 pt tile is the
/// image `tile`, grouped.
fn pattern_over_blue(tile: &[u8], opacity: f32) -> Session {
    let href = format!("data:image/png;base64,{}", vectorcraft_format::base64_encode(tile));
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="200" height="200" viewBox="0 0 200 200"><defs><pattern id="p" patternUnits="userSpaceOnUse" width="32" height="32"><image width="32" height="32" xlink:href="{href}"/></pattern></defs><g><rect x="40" y="40" width="128" height="128" fill="#0000ff"/><rect x="40" y="40" width="128" height="128" fill="url(#p)" opacity="{opacity}"/></g></svg>"##
    );
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "p.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
    s
}

/// #3666: a pattern fill whose tile is an image, under opacity, is flattened with the art under it
/// like a transparent image, instead of being written opaque over white.
#[test]
fn a_transparent_image_pattern_is_flattened_with_the_art_under_it() {
    for (tile, what) in
        [(checker([0, 200, 0, 255], [255, 230, 0, 255]), "opaque tile"), (checker([0, 200, 0, 255], [255, 255, 255, 0]), "see-through tile")]
    {
        let mut s = pattern_over_blue(&tile, 0.6);
        let points = [(72.0, 72.0), (88.0, 72.0), (136.0, 152.0), (152.0, 152.0)];
        let before: Vec<[u8; 3]> = points.iter().map(|&(x, y)| colour_at(&s.doc().unwrap().doc, x, y)).collect();
        let p = json!({"format": "ai", "aiVersion": "7", "transparentImages": "flatten", "hiddenObjects": "layer"});
        let r = s.execute("file.saveCopy", &p).unwrap();
        assert!(warnings(&r).iter().any(|w| w.contains("flattened with the art under them")), "{what}: {:?}", warnings(&r));
        let r = s.execute("document.open", &json!({"name": "t.ai", "dataBase64": r["dataBase64"]})).unwrap();
        assert!(!r.to_string().contains("left out"), "{what}: {:?}", warnings(&r));
        let d = &s.doc().unwrap().doc;
        let after: Vec<[u8; 3]> = points.iter().map(|&(x, y)| colour_at(d, x, y)).collect();
        for (a, b) in before.iter().zip(&after) {
            assert!(a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 3), "{what}: before {before:?} after {after:?}");
        }
    }
}

/// #3666: an opaque pattern fill whose tile is an opaque image is still written as its tiles by the
/// writer, not flattened.
#[test]
fn an_opaque_image_pattern_is_written_as_its_tiles() {
    let mut s = pattern_over_blue(&checker([0, 200, 0, 255], [255, 230, 0, 255]), 1.0);
    let points = [(72.0, 72.0), (88.0, 72.0), (136.0, 152.0), (152.0, 152.0)];
    let before: Vec<[u8; 3]> = points.iter().map(|&(x, y)| colour_at(&s.doc().unwrap().doc, x, y)).collect();
    let p = json!({"format": "ai", "aiVersion": "7", "transparentImages": "flatten", "hiddenObjects": "layer"});
    let r = s.execute("file.saveCopy", &p).unwrap();
    let w = warnings(&r);
    assert!(w.iter().any(|w| w.contains("pattern fills are written as their tiles")), "{w:?}");
    assert!(!w.iter().any(|w| w.contains("flattened with the art under them")), "{w:?}");
    s.execute("document.open", &json!({"name": "t.ai", "dataBase64": r["dataBase64"]})).unwrap();
    let after: Vec<[u8; 3]> = points.iter().map(|&(x, y)| colour_at(&s.doc().unwrap().doc, x, y)).collect();
    for (a, b) in before.iter().zip(&after) {
        assert!(a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 3), "before {before:?} after {after:?}");
    }
}

/// A square (40–168) with two pattern fills of 32 pt image tiles: a red and blue checkerboard,
/// with a green checkerboard with see-through squares over it.
fn two_pattern_fills() -> Session {
    let see_through = vectorcraft_format::base64_encode(&checker([0, 200, 0, 255], [255, 255, 255, 0]));
    let opaque = vectorcraft_format::base64_encode(&checker([200, 0, 0, 255], [0, 0, 200, 255]));
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="200" height="200" viewBox="0 0 200 200"><defs><pattern id="p" patternUnits="userSpaceOnUse" width="32" height="32"><image width="32" height="32" xlink:href="data:image/png;base64,{see_through}"/></pattern><pattern id="q" patternUnits="userSpaceOnUse" width="32" height="32"><image width="32" height="32" xlink:href="data:image/png;base64,{opaque}"/></pattern></defs><rect x="40" y="40" width="128" height="128" fill="url(#q)"/><rect x="40" y="40" width="128" height="128" fill="url(#p)"/></svg>"##
    );
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "p.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
    // Move the opaque pattern fill under the see-through one, on the second rectangle.
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let ids: Vec<NodeId> = d.layers[0].children().unwrap().iter().map(|n| n.id).collect();
    let under = d.node(ids[0]).unwrap().appearance.items.clone();
    assert!(!under.is_empty());
    let n = d.node_mut(ids[1]).unwrap();
    let mut items = under;
    items.append(&mut n.appearance.items);
    n.appearance.items = items;
    let _ = d.remove(ids[0]);
    s
}

/// #3666: on a shape with two pattern fills, only the one that paints a transparent image is
/// flattened; the opaque image pattern under it is still written as its tiles.
#[test]
fn only_the_transparent_image_pattern_fill_of_a_shape_is_flattened() {
    let mut s = two_pattern_fills();
    let points = [(72.0, 72.0), (88.0, 72.0), (136.0, 152.0), (152.0, 152.0)];
    let before: Vec<[u8; 3]> = points.iter().map(|&(x, y)| colour_at(&s.doc().unwrap().doc, x, y)).collect();
    assert_eq!(before[1], [0, 0, 200], "{before:?}");
    let p = json!({"format": "ai", "aiVersion": "7", "transparentImages": "flatten", "hiddenObjects": "layer"});
    let r = s.execute("file.saveCopy", &p).unwrap();
    let w = warnings(&r);
    assert!(w.iter().any(|w| w.contains("flattened with the art under them")), "{w:?}");
    assert!(w.iter().any(|w| w.contains("pattern fills are written as their tiles")), "{w:?}");
    s.execute("document.open", &json!({"name": "t.ai", "dataBase64": r["dataBase64"]})).unwrap();
    let after: Vec<[u8; 3]> = points.iter().map(|&(x, y)| colour_at(&s.doc().unwrap().doc, x, y)).collect();
    for (a, b) in before.iter().zip(&after) {
        assert!(a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 3), "before {before:?} after {after:?}");
    }
}

/// #3666: with `omit`, a pattern fill whose tile image is see-through is not expanded, so its tiles
/// aren't left out: the writer still draws them.
#[test]
fn omit_keeps_a_transparent_image_pattern() {
    let mut s = pattern_over_blue(&checker([0, 200, 0, 255], [255, 255, 255, 0]), 1.0);
    let p = json!({"format": "ai", "aiVersion": "7", "transparentImages": "omit", "hiddenObjects": "layer"});
    let r = s.execute("file.saveCopy", &p).unwrap();
    let w = warnings(&r);
    assert!(w.iter().any(|w| w.contains("pattern fills are written as their tiles")), "{w:?}");
    assert!(!w.iter().any(|w| w.contains("are left out")), "{w:?}");
    s.execute("document.open", &json!({"name": "t.ai", "dataBase64": r["dataBase64"]})).unwrap();
    assert!(images(&s.doc().unwrap().doc).0 > 0);
}

/// #3666: expanding the transparent image patterns of a shape with two pattern fills expands only
/// the fill that paints a transparent image; the opaque image pattern stays a pattern fill.
#[test]
fn only_the_transparent_pattern_fill_of_a_leaf_is_expanded() {
    let mut s = two_pattern_fills();
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let keys = crate::cmd::flatten::see_through_images(d);
    let layer = d.layers[0].id;
    assert_eq!(crate::cmd::expand::expand_transparent_image_patterns(d, layer, &keys).unwrap(), 1);
    let mut patterns = vec![];
    let mut images = 0;
    d.walk(|n| {
        for i in &n.appearance.items {
            if let vectorcraft_doc::appearance::AppearanceItem::Fill(f) = i
                && let vectorcraft_color::Paint::Pattern { pattern, .. } = &f.paint
            {
                patterns.push(pattern.clone());
            }
        }
        images += usize::from(matches!(n.kind, vectorcraft_doc::NodeKind::Image(_)));
    });
    assert_eq!(patterns.len(), 1, "{patterns:?}");
    assert!(images > 0);
}
