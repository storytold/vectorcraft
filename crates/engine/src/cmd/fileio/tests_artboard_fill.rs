//! Artboard background colors in output (#4937): raster, PDF, SVG and EPS exports, Export for
//! Screens and print draw each artboard's color behind its art, inside the artboard;
//! `includeArtboardFill: false` (Include Artboard Fill) leaves it out; the editing data keeps it as
//! the artboard's color, not as art.

use serde_json::{Value, json};

use super::*;

const RED: [u8; 4] = [255, 0, 0, 255];

/// Two 60×40 pt artboards, a 20 pt square (white fill, black stroke) on the first, which is red.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 60, "height": 40, "artboards": 2})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 0, "color": "#ff0000"})).unwrap();
    s
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

fn with(mut p: Value, more: &Value) -> Value {
    if let (Some(o), Some(m)) = (p.as_object_mut(), more.as_object()) {
        o.extend(m.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    p
}

/// The pixels of the PNG `document.export` writes of the active document with `p`, on transparency.
fn png(s: &mut Session, p: Value) -> image::RgbaImage {
    let v = s.execute("document.export", &with(json!({"format": "png", "background": "transparent"}), &p)).unwrap();
    image::load_from_memory(&b64(&v)).unwrap().to_rgba8()
}

/// The file `document.export` writes with `p`.
fn file(s: &mut Session, p: Value) -> Vec<u8> {
    b64(&s.execute("document.export", &p).unwrap())
}

/// The file `bytes` named `name`, opened in a new session.
fn opened(name: &str, bytes: &[u8]) -> Session {
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)})).unwrap();
    s
}

/// What the file `bytes` named `name` shows near its top-left corner, opened and drawn.
fn corner(name: &str, bytes: &[u8]) -> [u8; 4] {
    png(&mut opened(name, bytes), json!({"artboard": 0})).get_pixel(2, 2).0
}

#[test]
fn raster_exports_paint_the_color_behind_the_art() {
    let mut s = session();
    let on = png(&mut s, json!({"artboard": 0}));
    assert_eq!(on.get_pixel(2, 2).0, RED, "a transparent background still shows the color");
    assert_eq!(on.get_pixel(20, 20).0, [255, 255, 255, 255], "the art over it");
    assert_eq!(png(&mut s, json!({"artboard": 0, "includeArtboardFill": false})).get_pixel(2, 2)[3], 0, "left out");
    assert_eq!(png(&mut s, json!({"artboard": 1})).get_pixel(2, 2)[3], 0, "an artboard without a color");
    let jpg = file(&mut s, json!({"format": "jpg", "artboard": 0, "quality": 100}));
    let px = image::load_from_memory(&jpg).unwrap().to_rgb8().get_pixel(2, 2).0;
    assert!(px[0] > 240 && px[1] < 16 && px[2] < 16, "JPEG too: {px:?}");
    for bad in [json!("no"), json!(1)] {
        assert!(s.execute("document.export", &json!({"format": "png", "includeArtboardFill": bad})).is_err(), "{bad}");
    }
    assert_eq!(s.doc().unwrap().doc.layers.len(), 1, "the document gains no layer");
}

#[test]
fn the_color_fills_its_artboard_alone_behind_the_art() {
    let s = session();
    let doc = &s.doc().unwrap().doc;
    let board = doc.artboards[0].rect;
    let filled = doc.with_artboard_fills([0, 1]).unwrap();
    let img = vectorcraft_render::Renderer::new().render_region(&filled, board.inflate(10.0, 10.0), 1.0, false);
    let rgba = img.to_straight();
    let px = |x: usize, y: usize| rgba[(y * img.width as usize + x) * 4..][..4].to_vec();
    assert_eq!(px(5, 5)[3], 0, "outside the artboard");
    assert_eq!(px(15, 15), RED, "inside it");
    assert_eq!(px(30, 30), [255, 255, 255, 255], "the art over it");
}

#[test]
fn pdf_and_svg_show_the_color_and_their_editing_data_keeps_it_as_the_artboards() {
    let mut s = session();
    for (format, name) in [("pdf", "a.pdf"), ("svg", "a.svg")] {
        let p = json!({"format": format, "artboard": 0, "preserveEditing": false});
        assert_eq!(corner(name, &file(&mut s, p.clone())), RED, "{format}");
        assert_eq!(corner(name, &file(&mut s, with(p, &json!({"includeArtboardFill": false}))))[3], 0, "{format}");
    }
    let reopened = opened("a.pdf", &file(&mut s, json!({"format": "pdf", "preserveEditing": true})));
    let (back, doc) = (&reopened.doc().unwrap().doc, &s.doc().unwrap().doc);
    assert_eq!(back.artboards[0].color, doc.artboards[0].color);
    assert_eq!(back.layers.len(), doc.layers.len(), "no art made of it");
}

#[test]
fn eps_and_postscript_print_draw_the_color() {
    let mut s = session();
    // The only red the document has, the first artboard's color: RGB, or CMYK PostScript's (its
    // values depend on the color conversion the platform offers, so only its hue is checked).
    let red = |t: &str| {
        t.lines().any(|l| {
            let mut w: Vec<&str> = l.split_whitespace().collect();
            let Some(op) = w.pop() else { return false };
            let v: Vec<f64> = w.iter().filter_map(|n| n.parse().ok()).collect();
            match (op, v.as_slice()) {
                ("rg" | "setrgbcolor", [r, g, b]) => *r > 0.9 && *g < 0.1 && *b < 0.1,
                ("k" | "setcmykcolor", [c, m, y, k]) => *c < 0.1 && *m > 0.8 && *y > 0.8 && *k < 0.1,
                _ => false,
            }
        })
    };
    let eps = |s: &mut Session, more: Value| {
        String::from_utf8_lossy(&file(s, with(json!({"format": "eps", "useArtboards": true, "artboard": 0}), &more))).into_owned()
    };
    assert!(red(&eps(&mut s, json!({}))), "EPS");
    assert!(!red(&eps(&mut s, json!({"includeArtboardFill": false}))), "EPS without it");
    let ps = |s: &mut Session, more: Value| {
        String::from_utf8_lossy(&b64(&s.execute("file.print", &with(json!({"format": "postscript"}), &more)).unwrap())).into_owned()
    };
    assert!(red(&ps(&mut s, json!({}))), "print");
    assert!(!red(&ps(&mut s, json!({"includeArtboardFill": false}))), "print without it");
}

#[test]
fn export_for_screens_includes_the_fill_unless_unchecked() {
    let mut s = session();
    let screens = |s: &mut Session, more: Value| {
        let p = with(json!({"artboards": [0], "formats": [{"format": "png"}], "settings": {"png": {"background": "transparent"}}}), &more);
        s.execute("document.exportForScreens", &p).unwrap()
    };
    let corner = |r: &Value| image::load_from_memory(&b64(&r["files"][0])).unwrap().to_rgba8().get_pixel(2, 2).0;
    assert_eq!(corner(&screens(&mut s, json!({}))), RED, "on by default");
    assert_eq!(corner(&screens(&mut s, json!({"includeArtboardFill": false})))[3], 0, "unchecked");
    assert_eq!(s.execute("document.exportSettings", &json!({})).unwrap()["settings"]["includeArtboardFill"], false, "remembered");
    assert!(s.execute("document.exportForScreens", &json!({"includeArtboardFill": "no", "formats": [{"format": "png"}]})).is_err());
    // A document row's artboard keeps its color, as a color.
    let r = screens(&mut s, json!({"formats": [{"format": "vectorcraft"}]}));
    let back = vectorcraft_format::load(&b64(&r["files"][0])).unwrap();
    assert_eq!(back.artboards[0].color, s.doc().unwrap().doc.artboards[0].color);
    assert_eq!(back.layers.len(), 1, "not as art");
}
