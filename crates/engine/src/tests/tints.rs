//! Tints of global and spot colours: applying a swatch at a tint, tints following swatch edits,
//! tint swatches, spot tints on their plate and in PDF, Adjust Color Balance's Global mode.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_render::proof;

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn doc(s: &Session) -> &vectorcraft_doc::Document {
    &s.doc().unwrap().doc
}

/// A selected rectangle.
fn rect(s: &mut Session) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30}))["id"].as_u64().unwrap());
    run(s, "select.set", json!({"ids": [id.0]}));
    id
}

fn fill(s: &Session, id: NodeId) -> Paint {
    doc(s).node(id).unwrap().appearance.fill_paint()
}

const MAGENTA: Color = Color::Cmyk { c: 0.0, m: 1.0, y: 0.0, k: 0.2 };

/// A document with a spot swatch "Ink" (magenta + 20 % black) and a global process "Brand".
fn inks() -> Session {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Ink", "color": MAGENTA, "spot": true}));
    run(&mut s, "swatch.new", json!({"name": "Brand", "color": "#3366cc", "global": true}));
    s
}

fn linked(color: Color, name: &str, tint: f32) -> Paint {
    Paint::Solid { color, swatch: Some(name.into()), tint }
}

#[test]
fn a_swatch_applies_at_a_tint_and_the_tint_follows_swatch_edits() {
    let mut s = inks();
    let a = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Brand", "tint": 40}));
    let brand = Color::from_hex("#3366cc").unwrap();
    assert_eq!(fill(&s, a), linked(brand.tinted(0.4), "Brand", 0.4));
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "swatch.edit", json!({"name": "Brand", "color": "#cc3300"}));
    assert_eq!(r["relinked"], 1);
    let red = Color::from_hex("#cc3300").unwrap();
    assert_eq!(fill(&s, a), linked(red.tinted(0.4), "Brand", 0.4), "the 40% tint follows the edit");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    // Renaming keeps the tint; deleting unlinks it and the art keeps its colour.
    run(&mut s, "swatch.edit", json!({"name": "Brand", "newName": "Logo"}));
    assert_eq!(fill(&s, a), linked(red.tinted(0.4), "Logo", 0.4));
    run(&mut s, "swatch.delete", json!({"name": "Logo"}));
    assert_eq!(fill(&s, a), Paint::solid(red.tinted(0.4)), "unlinked: tint 100% of its own colour");
    // The proxies report the tint; process colours have none.
    run(&mut s, "paint.setFill", json!({"swatch": "Ink", "tint": 25}));
    assert_eq!(run(&mut s, "paint.proxies", json!({}))["fill"]["tint"], json!(0.25));
    assert!(s.execute("paint.setFill", &json!({"swatch": "Black", "tint": 50})).unwrap_err().to_string().contains("process"));
    assert!(s.execute("paint.setFill", &json!({"color": "#ff0000", "tint": 50})).is_err(), "a tint needs a swatch");
}

#[test]
fn new_swatch_saves_a_tint_swatch_linked_to_its_base() {
    let mut s = inks();
    let a = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Ink", "tint": 40}));
    let r = run(&mut s, "swatch.new", json!({}));
    assert_eq!(r["name"], "Ink 40%");
    let w = doc(&s).swatch("Ink 40%").unwrap();
    assert_eq!((w.tint_of(), w.global, w.spot), (Some(("Ink", 0.4)), false, false), "its kind is its base's");
    let listed = run(&mut s, "swatch.list", json!({}));
    let row = listed["swatches"].as_array().unwrap().iter().find(|w| w["name"] == "Ink 40%").unwrap().clone();
    assert_eq!((row["tintOf"].clone(), row["tint"].clone()), (json!("Ink"), json!(40.0)));
    // `tint` alone: that tint of the fill's swatch.
    assert_eq!(run(&mut s, "swatch.new", json!({"tint": 70}))["name"], "Ink 70%");
    assert_eq!(run(&mut s, "swatch.new", json!({"swatch": "Brand", "tint": 10}))["name"], "Brand 10%");
    // Applying a tint swatch links to the base at that tint.
    let b = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Ink 70%"}));
    assert_eq!(fill(&s, b), linked(MAGENTA.tinted(0.7), "Ink", 0.7));
    // Editing the base recolours the tint swatch and the art; the tint swatch only takes a name.
    run(&mut s, "swatch.edit", json!({"name": "Ink", "color": {"c": 1, "m": 0, "y": 0, "k": 0}}));
    let cyan = Color::cmyk(1.0, 0.0, 0.0, 0.0);
    assert_eq!(doc(&s).swatch("Ink 70%").unwrap().paint, linked(cyan.tinted(0.7), "Ink", 0.7));
    assert_eq!(fill(&s, a), linked(cyan.tinted(0.4), "Ink", 0.4));
    assert!(s.execute("swatch.edit", &json!({"name": "Ink 70%", "color": "#ff0000"})).is_err());
    run(&mut s, "swatch.edit", json!({"name": "Ink 70%", "newName": "Pale Ink"}));
    // A tint swatch is used where its tint of its base is.
    let unused = run(&mut s, "swatch.unused", json!({}));
    let names: Vec<&str> = unused["names"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(!names.contains(&"Pale Ink") && !names.contains(&"Ink 40%") && names.contains(&"Brand 10%"), "{names:?}");
    // A tint asked for in another mode, or as a global colour, is a process copy.
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let r = run(&mut s, "swatch.new", json!({"global": true}));
    assert!(doc(&s).swatch(r["name"].as_str().unwrap()).unwrap().tint_of().is_none());
}

#[test]
fn spot_tints_print_their_percentage_of_the_plate() {
    let mut s = inks();
    rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Ink", "tint": 40}));
    let d = doc(&s);
    let c = vectorcraft_color::cms::active();
    let Paint::Solid { color, .. } = fill(&s, s.doc().unwrap().selection.objects[0]) else { panic!() };
    let ink = proof::inks(d, &c, &color, Some(("Ink", 0.4)), proof::Intent::RelativeColorimetric);
    assert_eq!((ink.cmyk, ink.spot), ([0.0; 4], Some(("Ink".to_string(), 0.4))));
    // A process colour linked to a global swatch separates into process inks.
    assert!(proof::inks(d, &c, &color, Some(("Brand", 0.4)), proof::Intent::RelativeColorimetric).spot.is_none());
    // PDF: the Separation value is the tint (102 / 255 = 0.4).
    let bytes = vectorcraft_pdf::export(d, &vectorcraft_pdf::PdfOptions { created: Some(0), ..vectorcraft_pdf::PdfOptions::uncompressed() }).unwrap();
    let pdf = String::from_utf8_lossy(&bytes);
    assert!(pdf.contains("/Separation/Ink/DeviceCMYK"), "a spot tint is a Separation");
    assert!(pdf.contains("cs 0.4 scn"), "the tint is the Separation value");
}

#[test]
fn global_mode_shifts_tints_of_linked_colours_only() {
    let mut s = inks();
    let a = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Ink", "tint": 50}));
    run(&mut s, "paint.setStroke", json!({"color": "#808080"}));
    let b = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Brand"}));
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "edit.colors.adjustBalance", json!({"tint": -30}));
    assert_eq!(r["changed"], 2, "two linked fills; the grey stroke isn't global");
    let tint_of = |p: Paint| match p {
        Paint::Solid { color, swatch, tint } => (color, swatch, tint),
        _ => panic!("{p:?}"),
    };
    let (color, swatch, tint) = tint_of(fill(&s, a));
    assert!((tint - 0.2).abs() < 1e-6 && swatch.as_deref() == Some("Ink") && color == MAGENTA.tinted(tint), "{tint}");
    let brand = Color::from_hex("#3366cc").unwrap();
    let (color, swatch, tint) = tint_of(fill(&s, b));
    assert!((tint - 0.7).abs() < 1e-6 && swatch.as_deref() == Some("Brand") && color == brand.tinted(tint), "{tint}");
    assert_eq!(doc(&s).node(a).unwrap().appearance.stroke_paint(), Paint::solid(Color::from_hex("#808080").unwrap()));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    // Clamped at 100%; nothing left to change leaves the document alone.
    let r = run(&mut s, "edit.colors.adjustBalance", json!({"mode": "global", "tint": 100}));
    assert_eq!(r["changed"], 2);
    assert_eq!(fill(&s, a), linked(MAGENTA, "Ink", 1.0));
    let r = run(&mut s, "edit.colors.adjustBalance", json!({"mode": "global", "tint": 10}));
    assert_eq!(r["changed"], 0);
}

#[test]
fn tints_round_trip_through_the_native_format() {
    let mut s = inks();
    rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Ink", "tint": 35}));
    run(&mut s, "swatch.new", json!({}));
    let d = doc(&s).clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&d, false)).unwrap();
    let id = s.doc().unwrap().selection.objects[0];
    assert_eq!(back.node(id).unwrap().appearance.fill_paint(), fill(&s, id));
    assert_eq!(back.swatch("Ink 35%"), d.swatch("Ink 35%"));
}

#[test]
fn converting_the_document_colour_mode_keeps_tints_exact() {
    let mut s = inks();
    let a = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"swatch": "Brand", "tint": 30}));
    run(&mut s, "object.convertDocumentColorMode", json!({"mode": "cmyk"}));
    let base = doc(&s).global_color("Brand").unwrap();
    assert!(matches!(base, Color::Cmyk { .. }));
    assert_eq!(fill(&s, a), linked(base.tinted(0.3), "Brand", 0.3), "30 % of the converted swatch's inks");
}
