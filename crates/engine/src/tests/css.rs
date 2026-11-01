//! CSS Properties: `css.selection` (rectangles' backgrounds, borders and radii, type's fonts,
//! gradients), `css.generate` over the document and `css.export` writing the style sheet and the
//! pictures of rasterized art.

use serde_json::{Value, json};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn css(v: &Value) -> &str {
    v["css"].as_str().unwrap()
}

#[test]
fn a_rounded_rectangle_gives_background_border_and_radius() {
    let mut s = session();
    let id = run(&mut s, "shape.rectangle", json!({"x": 10, "y": 20, "width": 120, "height": 60, "radius": 6}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"color": "#3366cc"}));
    run(&mut s, "paint.setStroke", json!({"color": "#000000"}));
    run(&mut s, "layer.setProps", json!({"id": id, "name": "Button"}));
    let r = run(&mut s, "css.selection", json!({"position": true}));
    let text = css(&r);
    for decl in ["background-color: #3366cc;", "border: 1px solid #000000;", "border-radius: 6px;", "left: 10px;", "top: 20px;", "width: 120px;"] {
        assert!(text.contains(decl), "{decl} in {text}");
    }
    assert!(text.starts_with(".Button {\n"), "{text}");
    assert_eq!(r["rules"][0]["id"], json!(id));
    assert_eq!(r["rules"][0]["selector"], json!(".Button"));
    assert!(r["rules"][0].get("unsupported").is_none());
    // A query: no undo step, no change.
    let rev = s.doc().unwrap().revision;
    run(&mut s, "css.selection", json!({"units": "mm"}));
    assert_eq!(s.doc().unwrap().revision, rev);
    assert!(s.execute("css.selection", &json!({"units": "furlong"})).is_err(), "unknown units are refused");
}

#[test]
fn type_gives_font_properties_and_gradients_css_gradients() {
    let mut s = session();
    let t = run(&mut s, "text.create", json!({"x": 20, "y": 50, "text": "Title", "size": 24}))["id"].as_u64().unwrap();
    let text = css(&run(&mut s, "css.selection", json!({"ids": [t]}))).to_string();
    for decl in ["font-family: 'Source Sans 3', sans-serif;", "font-size: 24px;", "color: #"] {
        assert!(text.contains(decl), "{decl} in {text}");
    }
    run(&mut s, "shape.rectangle", json!({"x": 0, "y": 100, "width": 100, "height": 50}));
    run(&mut s, "paint.setFill", json!({"gradient": {"kind": "linear"}}));
    let text = css(&run(&mut s, "css.selection", json!({}))).to_string();
    assert!(text.contains("background-image: linear-gradient(90deg, "), "{text}");
}

#[test]
fn generate_covers_the_document_and_an_empty_selection_has_none() {
    let mut s = session();
    run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}));
    run(&mut s, "shape.ellipse", json!({"x": 20, "y": 0, "width": 10, "height": 10}));
    run(&mut s, "select.none", json!({}));
    let none = run(&mut s, "css.selection", json!({}));
    assert_eq!(css(&none), "");
    let all = run(&mut s, "css.generate", json!({}));
    let sel: Vec<&str> = all["rules"].as_array().unwrap().iter().map(|r| r["selector"].as_str().unwrap()).collect();
    assert_eq!(sel, [".rectangle", ".ellipse"]);
    assert!(css(&all).contains("border-radius: 50%;"), "{}", css(&all));
    let named = run(&mut s, "css.generate", json!({"unnamed": false}));
    assert_eq!((named["rules"].as_array().unwrap().len(), named["skipped"].as_u64()), (0, Some(2)));
}

#[test]
fn export_writes_the_sheet_and_rasterized_art() {
    let mut s = session();
    run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 40}));
    run(&mut s, "shape.star", json!({"cx": 100, "cy": 100, "radius1": 40, "radius2": 20}));
    // Without a path: the CSS as data, the pictures as base64.
    let r = run(&mut s, "css.export", json!({"scope": "all", "rasterize": true}));
    let data = r["data"].as_str().unwrap();
    assert!(data.contains(".rectangle {") && data.contains(".path {\n") && data.contains("background-image: url(path.png);"), "{data}");
    assert!(data.ends_with("}\n"));
    assert_eq!(r["rules"], json!(2));
    let images = r["images"].as_array().unwrap();
    assert_eq!(images.len(), 1, "only the star is rasterized");
    assert_eq!(images[0]["name"], json!("path.png"));
    let png = vectorcraft_format::base64_decode(images[0]["dataBase64"].as_str().unwrap()).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    // The selection alone (the star, selected last).
    let r = run(&mut s, "css.export", json!({}));
    assert_eq!(r["rules"], json!(1));
    assert!(r["images"].as_array().unwrap().is_empty(), "no rasterize: no pictures");
    assert!(s.execute("css.export", &json!({"scope": "page"})).is_err());
    run(&mut s, "select.none", json!({}));
    assert!(s.execute("css.export", &json!({})).is_err(), "nothing selected: nothing to export");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn export_to_a_path_puts_the_pictures_next_to_the_sheet() {
    let mut s = session();
    run(&mut s, "shape.star", json!({"cx": 100, "cy": 100, "radius1": 40, "radius2": 20}));
    let dir = std::env::temp_dir().join(format!("vc-css-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("site.css");
    let r = run(&mut s, "css.export", json!({"path": path.to_string_lossy(), "rasterize": true}));
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("url(path.png)"), "{written}");
    assert_eq!(r["bytes"], json!(written.len()));
    let png = dir.join("path.png");
    assert_eq!(r["images"], json!([png.to_string_lossy()]));
    assert!(std::fs::read(&png).unwrap().starts_with(b"\x89PNG"));
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn export_says_once_when_a_rasterized_image_could_not_be_decoded() {
    const NOTE: &str = "an image couldn't be decoded and is missing from the picture";
    let notes = |r: &Value| r["warnings"].as_array().unwrap().iter().filter(|w| w.as_str() == Some(NOTE)).count();
    let mut s = session();
    run(&mut s, "shape.star", json!({"cx": 100, "cy": 100, "radius1": 40, "radius2": 20}));
    assert_eq!(notes(&run(&mut s, "css.export", json!({"scope": "all", "rasterize": true}))), 0);
    // Two images the renderer can't decode, each rasterized to a picture of its own.
    crate::tests_menucmds::place_undecodable_png(&mut s);
    crate::tests_menucmds::place_undecodable_png(&mut s);
    let r = run(&mut s, "css.export", json!({"scope": "all", "rasterize": true}));
    assert_eq!(r["images"].as_array().unwrap().len(), 3, "{r}");
    assert_eq!(notes(&r), 1, "{}", r["warnings"]);
    let dir = std::env::temp_dir().join(format!("vc-css-undecoded-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let r = run(&mut s, "css.export", json!({"path": dir.join("site.css").to_string_lossy(), "scope": "all", "rasterize": true}));
    assert_eq!(r["images"].as_array().unwrap().len(), 3, "the pictures are still written");
    assert_eq!(notes(&r), 1, "{}", r["warnings"]);
    std::fs::remove_dir_all(&dir).ok();
}
