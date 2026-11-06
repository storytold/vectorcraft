//! The built-in [Registration] swatch (every plate, PDF `/Separation /All`), Object → Create Trim
//! Marks (Registration, every artboard, Japanese marks) and Effect → Crop Marks.

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_doc::{Document, Node};
use vectorcraft_geom::Affine;
use vectorcraft_render::proof::ProofSetup;
use vectorcraft_render::{RenderOptions, Renderer};

use crate::tests_colormgmt::GLOBAL;
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

/// The 4× render of `doc` (400×400 px), on white, showing `plates` only when given.
fn render(doc: &Document, plates: Option<&[&str]>) -> vectorcraft_render::Rendered {
    let proof = plates.map(|p| ProofSetup { separations: Some(p.iter().map(|n| n.to_string()).collect()), ..Default::default() });
    let mut r = Renderer::new();
    r.threads = 0;
    r.render(doc, 400, 400, Affine::scale(4.0), &RenderOptions { background: Some([255, 255, 255, 255]), proof, ..Default::default() })
}

fn dark(p: [u8; 4]) -> bool {
    p[0] < 90 && p[1] < 90 && p[2] < 90
}

#[test]
fn registration_is_a_reserved_swatch() {
    let mut s = session();
    let a = id_of(&run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})));
    run(&mut s, "paint.setStroke", json!({"swatch": REGISTRATION}));
    let st = node(&s, a).appearance.stroke().unwrap().paint.clone();
    assert!(st.is_registration() && st == Paint::registration(), "{st:?}");
    // Listed after None, global; it can't be deleted, edited, moved, duplicated or merged.
    let list = run(&mut s, "swatch.list", json!({}));
    assert_eq!(list["swatches"][0]["kind"], "none");
    assert_eq!((list["swatches"][1]["name"].clone(), list["swatches"][1]["global"].clone()), (json!(REGISTRATION), json!(true)));
    for (cmd, p) in [
        ("swatch.delete", json!({"name": REGISTRATION})),
        ("swatch.edit", json!({"name": REGISTRATION, "color": "#ff0000"})),
        ("swatch.move", json!({"name": REGISTRATION, "to": 5})),
        ("swatch.duplicate", json!({"name": REGISTRATION})),
        ("swatch.merge", json!({"names": ["White", REGISTRATION]})),
        ("swatch.new", json!({"name": REGISTRATION, "color": "#ff0000"})),
    ] {
        let r = s.execute(cmd, &p);
        assert!(r.is_err() || cmd == "swatch.new", "{cmd}");
        if cmd == "swatch.new" {
            assert_ne!(r.unwrap()["name"], REGISTRATION, "the name is taken");
        }
    }
    assert!(s.doc().unwrap().doc.swatch(REGISTRATION).is_some());
    // Unused swatches never list it.
    assert!(!run(&mut s, "swatch.unused", json!({}))["names"].as_array().unwrap().iter().any(|n| n == REGISTRATION));
}

#[test]
fn trim_marks_print_on_every_plate() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Spot Ink", "color": {"c": 0, "m": 60, "y": 90, "k": 0}, "global": true}));
    run(&mut s, "swatch.setSpot", json!({"name": "Spot Ink"}));
    run(&mut s, "shape.rectangle", json!({"x": 30, "y": 30.125, "width": 40, "height": 40}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    let g = id_of(&run(&mut s, "object.createTrimMarks", json!({"weight": 1})));
    let n = node(&s, g);
    assert_eq!(n.children().unwrap().len(), 8);
    assert!(n.children().unwrap().iter().all(|c| c.appearance.stroke().unwrap().paint.is_registration()));
    let doc = s.doc().unwrap().doc.clone();
    // The top-left mark runs from x = 21 to 3 at y = 30.125: 100% ink on each plate.
    for plate in ["Cyan", "Magenta", "Yellow", "Black", "Spot Ink"] {
        let p = render(&doc, Some(&[plate])).pixel(48, 120);
        assert!(dark(p), "{plate}: {p:?}");
    }
    // A plain black mark would print on the black plate only.
    run(&mut s, "paint.setStroke", json!({"ids": [n.children().unwrap()[0].id.0], "color": {"c": 0, "m": 0, "y": 0, "k": 100}}));
    let doc = s.doc().unwrap().doc.clone();
    assert!(!dark(render(&doc, Some(&["Cyan"])).pixel(48, 120)));
    assert!(!dark(render(&doc, Some(&["Spot Ink"])).pixel(48, 120)));
}

#[test]
fn pdf_writes_registration_as_separation_all() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    run(&mut s, "shape.rectangle", json!({"x": 30, "y": 30, "width": 40, "height": 40}));
    run(&mut s, "object.createTrimMarks", json!({}));
    let bytes = vectorcraft_pdf::export(
        &s.doc().unwrap().doc,
        &vectorcraft_pdf::PdfOptions { created: Some(0), ..vectorcraft_pdf::PdfOptions::uncompressed() },
    )
    .unwrap();
    let pdf = String::from_utf8_lossy(&bytes);
    assert!(pdf.contains("/Separation/All/DeviceCMYK"), "Registration prints on every plate");
}

#[test]
fn trim_marks_cover_every_artboard_and_take_the_japanese_style() {
    let mut s = session();
    run(&mut s, "artboard.new", json!({"x": 200, "y": 0, "width": 100, "height": 100}));
    let r = run(&mut s, "object.createTrimMarks", json!({}));
    let ids = r["ids"].as_array().unwrap().clone();
    assert_eq!(ids.len(), 2, "one group per artboard when nothing is selected");
    let b = s.doc().unwrap().doc.bounds_of(&[NodeId(ids[1].as_u64().unwrap())], false).unwrap();
    assert!(b.x0 < 200.0 && b.x0 > 150.0 && b.x1 > 300.0, "{b:?}");
    assert_eq!(s.doc().unwrap().selection.objects.len(), 2);
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().doc.layers[0].children().unwrap().is_empty(), "one undo step");
    // Japanese marks: double lines at the trim and bleed edges, and centre marks.
    run(&mut s, "prefs.set", json!({"key": "japaneseCropMarks", "value": true}));
    assert_eq!(s.crop_mark_style(), vectorcraft_doc::marks::MarkStyle::Japanese);
    assert!(s.execute("object.createTrimMarks", &json!({"allArtboards": false})).is_err(), "nothing selected to mark");
    let r = run(&mut s, "object.createTrimMarks", json!({}));
    let first = node(&s, NodeId(r["ids"][0].as_u64().unwrap()));
    let lines = first.children().unwrap();
    assert_eq!(lines.len(), 24);
    // At the top-left corner of the first artboard: horizontal lines at y = 0 and y = -bleed.
    let ys: Vec<f64> = lines[..4].iter().filter_map(|l| l.path_data().and_then(|p| p.bounds())).filter(|b| b.height() < 1e-9).map(|b| b.y0).collect();
    assert_eq!(ys.len(), 2);
    assert!(ys.iter().any(|y| y.abs() < 1e-9) && ys.iter().any(|y| (y + 3.0 * 72.0 / 25.4).abs() < 1e-6), "{ys:?}");
    // An explicit style wins over the preference.
    let r = run(&mut s, "object.createTrimMarks", json!({"style": "roman"}));
    assert_eq!(node(&s, NodeId(r["ids"][0].as_u64().unwrap())).children().unwrap().len(), 8);
    assert!(s.execute("object.createTrimMarks", &json!({"style": "western"})).is_err());
}

#[test]
fn crop_marks_effect_follows_the_object() {
    let mut s = session();
    let a = id_of(&run(&mut s, "shape.rectangle", json!({"x": 30, "y": 30.125, "width": 40, "height": 40})));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    run(&mut s, "effect.apply", json!({"effect": "cropMarks"}));
    let fx = node(&s, a).appearance.effects[0].clone();
    assert_eq!((fx.id.as_str(), fx.params["style"].as_str()), ("cropMarks", Some("roman")), "the style comes from the preferences");
    // The marks render outside the object (0.3 pt at 4×: about a pixel).
    let mark = |s: &Session, x: u32| render(&s.doc().unwrap().doc, None).pixel(x, 120);
    assert!(mark(&s, 48)[0] < 200, "{:?}", mark(&s, 48));
    // Moved, the marks move with it.
    run(&mut s, "object.move", json!({"dx": 10, "dy": 0}));
    assert_eq!(mark(&s, 48)[0], 255, "nothing left behind");
    assert!(mark(&s, 88)[0] < 200);
    // SVG export and Expand Appearance draw the marks as shapes.
    let svg = vectorcraft_svg::export(&s.doc().unwrap().doc, &Default::default());
    assert_eq!(vectorcraft_testkit::svg_shapes(&svg), 9, "{svg}");
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, a);
    assert!(n.appearance.effects.is_empty() && n.children().unwrap().len() == 2, "a group of the object and its marks");
    assert_eq!(n.children().unwrap()[1].children().unwrap().len(), 8);
    // Japanese crop marks.
    let b = id_of(&run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})));
    run(&mut s, "effect.apply", json!({"ids": [b.0], "effect": "cropMarks", "params": {"style": "japanese"}}));
    let art = vectorcraft_render::effects::crop_marks_art(&node(&s, b)).unwrap();
    assert_eq!(art.children().unwrap()[1].children().unwrap().len(), 24);
}
