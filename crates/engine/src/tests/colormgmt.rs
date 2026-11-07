//! Colour management: document colour mode through the CMS, gamut checks, profiles, proofing,
//! overprint preview, separations and CMYK/spot PDF output.

use std::sync::Mutex;

use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Intent, Lab, delta_e2000};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{ColorMode, Document};
use vectorcraft_geom::Affine;
use vectorcraft_render::proof::{ProofSetup, ProofTarget};
use vectorcraft_render::{RenderOptions, Renderer};

use crate::*;

/// Every test here reads or changes the process-wide proof view or colour settings: one at a time.
/// Tests that switch the Color Settings away from the defaults live in `tests/color_settings.rs`
/// (their own process): every colour shown or rendered goes through them, so no lock could cover
/// all the tests that would see the change.
pub(crate) static GLOBAL: Mutex<()> = Mutex::new(());

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, color: Value) -> NodeId {
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [id.0], "color": color})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [id.0], "none": true})).unwrap();
    id
}

fn fill_of(s: &Session, id: NodeId) -> Paint {
    s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint()
}

fn lab_of_rgb(rgb: [f32; 3]) -> Lab {
    cms::lab::srgb_to_lab(rgb)
}

fn render(doc: &Document, opts: RenderOptions) -> vectorcraft_render::Rendered {
    let mut r = Renderer::new();
    r.threads = 0;
    r.render(doc, 100, 100, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..opts })
}

#[test]
fn document_mode_converts_through_cms() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0, json!("#ff0000"));
    let r = s.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk"})).unwrap();
    assert!(r["changed"].as_u64().unwrap() >= 1);
    assert_eq!(s.doc().unwrap().doc.color_mode, ColorMode::Cmyk);
    let Paint::Solid { color: Color::Cmyk { c, m, y, k }, .. } = fill_of(&s, a) else { panic!("{:?}", fill_of(&s, a)) };
    // A press red: lots of M and Y, no cyan or black — not the naive (0, 1, 1, 0) necessarily.
    assert!(m > 0.8 && y > 0.8 && c < 0.05 && k < 0.05, "{c} {m} {y} {k}");
    // Swatches are converted too.
    assert!(s.doc().unwrap().doc.swatches.iter().filter_map(|w| w.paint.color()).all(|c| !matches!(c, Color::Rgb { .. })));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.color_mode, ColorMode::Rgb);
    assert_eq!(fill_of(&s, a), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    assert!(s.execute("object.convertDocumentColorMode", &json!({"mode": "lab"})).is_err());
}

#[test]
fn document_mode_conversion_is_idempotent() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 10.0, 10.0, json!("#336699"));
    rect(&mut s, 20.0, 0.0, 10.0, 10.0, json!({"gray": 40}));
    s.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk"})).unwrap();
    let once = s.doc().unwrap().doc.clone();
    let r = s.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk"})).unwrap();
    assert_eq!(r["changed"], 0);
    assert_eq!(*s.doc().unwrap().doc, *once);
    s.execute("object.convertDocumentColorMode", &json!({"mode": "rgb"})).unwrap();
    let twice = s.doc().unwrap().doc.clone();
    assert_eq!(s.execute("object.convertDocumentColorMode", &json!({"mode": "rgb"})).unwrap()["changed"], 0);
    assert_eq!(*s.doc().unwrap().doc, *twice);
}

#[test]
fn rgb_cmyk_rgb_document_roundtrip_within_tolerance() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    let cols = ["#cc9980", "#808080", "#4d7359", "#e6d94d"];
    let ids: Vec<NodeId> = cols.iter().enumerate().map(|(i, h)| rect(&mut s, i as f64 * 20.0, 0.0, 10.0, 10.0, json!(h))).collect();
    s.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk", "intent": "relative"})).unwrap();
    s.execute("object.convertDocumentColorMode", &json!({"mode": "rgb"})).unwrap();
    for (id, h) in ids.iter().zip(cols) {
        let orig = Color::from_hex(h).unwrap().to_rgb();
        let back = fill_of(&s, *id).color().unwrap().to_rgb();
        let de = delta_e2000(lab_of_rgb(orig), lab_of_rgb(back));
        // Black-point compensation shifts shadows a little; everything stays within ~3 ΔE00.
        assert!(de < 3.0, "{h}: ΔE {de}");
    }
}

#[test]
fn swatch_links_survive_mode_conversion() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#2a6fb0", "global": true})).unwrap();
    let a = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [a.0], "swatch": "Brand"})).unwrap();
    s.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk"})).unwrap();
    let Paint::Solid { color, swatch, .. } = fill_of(&s, a) else { panic!() };
    assert_eq!(swatch.as_deref(), Some("Brand"));
    let sw = s.doc().unwrap().doc.swatches.iter().find(|w| w.name == "Brand").unwrap().paint.color().unwrap();
    assert!(matches!(color, Color::Cmyk { .. }));
    assert_eq!(color, sw, "object and swatch converted identically");
}

/// The process color the active color settings give `c` in CMYK (the default intent).
fn to_cmyk(c: Color) -> Color {
    let cms = cms::active();
    cms.convert(&c, cms::Model::Cmyk, cms.settings().intent)
}

/// A document with colors that no 8-bit hex value holds: a plain fill, a three-stop gradient, a
/// pattern fill, a global swatch on one object's fill and at a 50% tint on another's stroke, and a
/// spot swatch on a third object → (session, plain, gradient, patterned, the swatch's two users,
/// the spot user).
fn reusable_paints_document() -> (Session, [NodeId; 6]) {
    let mut s = session();
    let plain = rect(&mut s, 0.0, 0.0, 10.0, 10.0, json!(Color::rgb(0.123_457, 0.765_432, 0.333_333)));
    let grad = rect(&mut s, 20.0, 0.0, 10.0, 10.0, json!("#000000"));
    let stops = json!([
        {"offset": 0, "color": Color::rgb(0.912_345, 0.1, 0.2)},
        {"offset": 0.37, "color": Color::rgb(0.2, 0.876_543, 0.3)},
        {"offset": 1, "color": Color::rgb(0.3, 0.4, 0.654_321)},
    ]);
    s.execute("paint.setFill", &json!({"ids": [grad.0], "gradient": {"kind": "linear", "stops": stops}})).unwrap();
    let tile = rect(&mut s, 40.0, 0.0, 10.0, 10.0, json!(Color::rgb(0.6, 0.111_111, 0.9)));
    s.execute("object.pattern.make", &json!({"ids": [tile.0], "name": "Odd Tile", "edit": false})).unwrap();
    let patterned = rect(&mut s, 60.0, 0.0, 10.0, 10.0, json!("#000000"));
    s.execute("paint.setFill", &json!({"ids": [patterned.0], "swatch": "Odd Tile"})).unwrap();
    s.execute("swatch.new", &json!({"name": "Shared", "color": Color::rgb(0.271_937, 0.314_773, 0.583_119), "global": true})).unwrap();
    let filled = rect(&mut s, 0.0, 20.0, 10.0, 10.0, json!("#000000"));
    s.execute("paint.setFill", &json!({"ids": [filled.0], "swatch": "Shared"})).unwrap();
    let stroked = rect(&mut s, 20.0, 20.0, 10.0, 10.0, json!("#000000"));
    s.execute("paint.setStroke", &json!({"ids": [stroked.0], "swatch": "Shared", "tint": 50})).unwrap();
    s.execute("swatch.new", &json!({"name": "Spot Lab", "color": Color::Lab { l: 61.5, a: 42.25, b: -17.75 }, "global": true})).unwrap();
    s.execute("swatch.setSpot", &json!({"name": "Spot Lab"})).unwrap();
    let spot = rect(&mut s, 40.0, 20.0, 10.0, 10.0, json!("#000000"));
    s.execute("paint.setFill", &json!({"ids": [spot.0], "swatch": "Spot Lab"})).unwrap();
    (s, [plain, grad, patterned, filled, stroked, spot])
}

/// RGB → CMYK → Undo gives back the document bit for bit (no conversion back through the
/// profile), and Redo gives back the converted one.
#[test]
fn rgb_cmyk_undo_restores_every_color_exactly() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut s, _) = reusable_paints_document();
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk"})).unwrap();
    let converted = s.doc().unwrap().doc.clone();
    assert_ne!(*converted, *before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one Undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *converted);
}

/// Each stop of a gradient converts on its own and keeps its offset.
#[test]
fn rgb_cmyk_converts_each_gradient_stop() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut s, [_, grad, ..]) = reusable_paints_document();
    let Paint::Gradient(old) = fill_of(&s, grad) else { panic!("not a gradient") };
    s.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk"})).unwrap();
    let Paint::Gradient(new) = fill_of(&s, grad) else { panic!("not a gradient") };
    assert_eq!(new.gradient.stops.len(), 3);
    for (o, n) in old.gradient.stops.iter().zip(&new.gradient.stops) {
        assert_eq!(n.offset, o.offset);
        assert!(matches!(n.color, Color::Cmyk { .. }), "{:?}", n.color);
        assert_eq!(n.color, to_cmyk(o.color));
    }
}

/// A pattern's tile art converts, and the art filled with the pattern keeps it.
#[test]
fn rgb_cmyk_converts_a_pattern_tile() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut s, [_, _, patterned, ..]) = reusable_paints_document();
    let tile = |s: &Session| s.doc().unwrap().doc.pattern("Odd Tile").unwrap().art[0].appearance.fill_paint().color().unwrap();
    let old = tile(&s);
    s.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk"})).unwrap();
    assert!(matches!(tile(&s), Color::Cmyk { .. }), "{:?}", tile(&s));
    assert_eq!(tile(&s), to_cmyk(old));
    assert!(matches!(fill_of(&s, patterned), Paint::Pattern { ref pattern, .. } if pattern == "Odd Tile"), "{:?}", fill_of(&s, patterned));
}

/// A global swatch used in two places converts once: both uses keep the link and take the
/// converted swatch color (the 50% tint at that tint), not a conversion of their own.
#[test]
fn rgb_cmyk_converts_a_shared_swatch_once() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut s, [.., filled, stroked, _]) = reusable_paints_document();
    let shared = |s: &Session| s.doc().unwrap().doc.swatch("Shared").unwrap().paint.color().unwrap();
    let old = shared(&s);
    s.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk"})).unwrap();
    let new = shared(&s);
    assert_eq!(new, to_cmyk(old));
    assert_eq!(fill_of(&s, filled), Paint::Solid { color: new, swatch: Some("Shared".into()), tint: 1.0 });
    let stroke = s.doc().unwrap().doc.node(stroked).unwrap().appearance.stroke_paint();
    assert_eq!(stroke, Paint::Solid { color: new.tinted(0.5), swatch: Some("Shared".into()), tint: 0.5 });
}

/// A spot color defined in Lab stays a spot color: its Lab definition, its link and its own
/// plate are kept.
#[test]
fn rgb_cmyk_keeps_a_spot_color() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut s, [.., spot]) = reusable_paints_document();
    let swatch = |s: &Session| s.doc().unwrap().doc.swatch("Spot Lab").unwrap().clone();
    let (old_swatch, old_fill) = (swatch(&s), fill_of(&s, spot));
    s.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk"})).unwrap();
    assert_eq!(swatch(&s), old_swatch);
    assert!(swatch(&s).spot);
    assert_eq!(fill_of(&s, spot), old_fill);
    let plates = s.execute("color.plates", &json!({})).unwrap();
    assert!(plates["plates"].as_array().unwrap().iter().any(|p| p["name"] == "Spot Lab"), "{plates}");
}

/// A spot color defined in RGB stays a spot color with its own plate, and its definition
/// converts like any other swatch: the fill that uses it and a 40% tint stroke both take the
/// converted definition.
#[test]
fn rgb_cmyk_converts_an_rgb_spot_definition() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Spot RGB", "color": Color::rgb(0.812_345, 0.234_567, 0.456_789), "global": true})).unwrap();
    s.execute("swatch.setSpot", &json!({"name": "Spot RGB"})).unwrap();
    let filled = rect(&mut s, 0.0, 0.0, 10.0, 10.0, json!("#000000"));
    s.execute("paint.setFill", &json!({"ids": [filled.0], "swatch": "Spot RGB"})).unwrap();
    let stroked = rect(&mut s, 20.0, 0.0, 10.0, 10.0, json!("#000000"));
    s.execute("paint.setStroke", &json!({"ids": [stroked.0], "swatch": "Spot RGB", "tint": 40})).unwrap();
    let swatch = |s: &Session| s.doc().unwrap().doc.swatch("Spot RGB").unwrap().clone();
    let old = swatch(&s).paint.color().unwrap();
    s.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk"})).unwrap();
    let sw = swatch(&s);
    assert!(sw.spot);
    let new = sw.paint.color().unwrap();
    assert!(matches!(new, Color::Cmyk { .. }), "{new:?}");
    assert_eq!(new, to_cmyk(old));
    assert_eq!(fill_of(&s, filled), Paint::Solid { color: new, swatch: Some("Spot RGB".into()), tint: 1.0 });
    let stroke = s.doc().unwrap().doc.node(stroked).unwrap().appearance.stroke_paint();
    assert_eq!(stroke, Paint::Solid { color: new.tinted(0.4), swatch: Some("Spot RGB".into()), tint: 0.4 });
    let plates = s.execute("color.plates", &json!({})).unwrap();
    assert!(plates["plates"].as_array().unwrap().iter().any(|p| p["name"] == "Spot RGB" && p["spot"] == true), "{plates}");
}

#[test]
fn gamut_check_flags_unprintable_colours() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 10.0, 10.0, json!("#0000ff"));
    rect(&mut s, 20.0, 0.0, 10.0, 10.0, json!("#cc9980"));
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let r = s.execute("color.gamutCheck", &json!({})).unwrap();
    let hexes: Vec<&str> = r["outOfGamut"].as_array().unwrap().iter().map(|v| v["hex"].as_str().unwrap()).collect();
    assert!(hexes.contains(&"#0000ff"), "{r}");
    assert!(!hexes.contains(&"#cc9980"), "{r}");
    let r = s.execute("color.gamutCheck", &json!({"colors": ["#00ff00", {"c": 100, "m": 0, "y": 0, "k": 0}]})).unwrap();
    assert_eq!(r["checked"], 2);
    assert_eq!(r["outOfGamut"].as_array().unwrap().len(), 1);
    // A color it can't read, a hex color with an alpha part included, is an error, not skipped.
    for bad in ["#0000ff80", "nope"] {
        assert!(s.execute("color.gamutCheck", &json!({"colors": ["#00ff00", bad]})).is_err(), "{bad}");
    }
}

#[test]
fn convert_query_and_intents() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    let q = |s: &mut Session, intent: &str| s.execute("color.convert", &json!({"color": "#0000ff", "to": "cmyk", "intent": intent})).unwrap();
    let p = q(&mut s, "perceptual");
    let r = q(&mut s, "relative");
    assert_ne!(p["values"], r["values"], "intents differ");
    assert_eq!(r["outOfGamut"], true);
    let lab = s.execute("color.convert", &json!({"color": "#ff0000", "to": "lab"})).unwrap();
    let l = lab["values"][0].as_f64().unwrap();
    assert!((l - 54.3).abs() < 1.0, "sRGB red L* (D50) ≈ 54.3: {lab}");
    let g = s.execute("color.convert", &json!({"color": "#808080", "to": "gray"})).unwrap();
    assert!((g["values"][0].as_f64().unwrap() - 0.5).abs() < 0.02, "{g}");
    assert!(s.execute("color.convert", &json!({"color": "#808080", "to": "hsv"})).is_err());
    assert!(s.execute("color.convert", &json!({"color": "#808080", "intent": "vivid"})).is_err());
}

#[test]
fn assign_profile_is_stored_and_undoable() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    let r = s.execute("edit.assignProfile", &json!({"cmyk": cms::GENERIC_CMYK})).unwrap();
    assert_eq!(r["cmyk"], cms::GENERIC_CMYK);
    assert_eq!(cmd::colormgmt::doc_profiles(&s.doc().unwrap().doc).1.as_deref(), Some(cms::GENERIC_CMYK));
    assert!(s.execute("edit.assignProfile", &json!({"cmyk": "No Such Profile"})).is_err());
    assert!(s.execute("edit.assignProfile", &json!({"rgb": cms::GENERIC_CMYK})).is_err(), "kind must match");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(cmd::colormgmt::doc_profiles(&s.doc().unwrap().doc), (None, None));
}

#[test]
fn proof_colors_simulate_press_on_screen() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 100.0, 100.0, json!("#0000ff"));
    let doc = s.doc().unwrap().doc.clone();
    assert_eq!(render(&doc, RenderOptions::default()).pixel(50, 50), [0, 0, 255, 255]);
    let proofed = render(&doc, RenderOptions { proof: Some(ProofSetup::default()), ..Default::default() }).pixel(50, 50);
    assert_ne!(proofed, [0, 0, 255, 255]);
    assert!(proofed[2] > proofed[0] && proofed[2] > proofed[1], "still blue: {proofed:?}");
    let de = delta_e2000(lab_of_rgb([0.0, 0.0, 1.0]), lab_of_rgb(proofed.map(|v| v as f32 / 255.0)[..3].try_into().unwrap()));
    assert!(de > 5.0, "out-of-gamut blue visibly changes: ΔE {de}");
    // Colour-blindness proof.
    let pr = render(&doc, RenderOptions { proof: Some(ProofSetup { target: ProofTarget::Protanopia, ..Default::default() }), ..Default::default() });
    assert_ne!(pr.pixel(50, 50), [0, 0, 255, 255]);
}

#[test]
fn separation_plates_render_ink_coverage() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 50.0, 100.0, json!({"c": 100, "m": 0, "y": 0, "k": 0}));
    rect(&mut s, 50.0, 0.0, 50.0, 100.0, json!({"c": 0, "m": 50, "y": 0, "k": 0}));
    let doc = s.doc().unwrap().doc.clone();
    let plate = |name: &str| {
        render(&doc, RenderOptions { proof: Some(ProofSetup { separations: Some(vec![name.into()]), ..Default::default() }), ..Default::default() })
    };
    let c = plate("Cyan");
    assert_eq!(c.pixel(25, 50), [0, 0, 0, 255], "100% cyan is solid on the cyan plate");
    assert_eq!(c.pixel(75, 50), [255, 255, 255, 255], "no cyan in the magenta object");
    let m = plate("Magenta");
    assert_eq!(m.pixel(25, 50), [255, 255, 255, 255]);
    let v = m.pixel(75, 50)[0] as i32;
    assert!((v - 128).abs() <= 2, "50% magenta renders as 50% grey: {v}");
    assert_eq!(plate("Black").pixel(25, 50), [255, 255, 255, 255]);
    // Composite of C+M only shows the magenta object in magenta ink.
    let cm = render(
        &doc,
        RenderOptions { proof: Some(ProofSetup { separations: Some(vec!["Magenta".into()]), ..Default::default() }), ..Default::default() },
    );
    assert_eq!(cm.pixel(25, 50)[0], 255);
    let all = render(
        &doc,
        RenderOptions {
            proof: Some(ProofSetup {
                separations: Some(vec!["Cyan".into(), "Magenta".into(), "Yellow".into(), "Black".into()]),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let p = all.pixel(25, 50);
    assert!(p[0] < 40 && p[2] > 200, "composite cyan: {p:?}");
}

#[test]
fn rgb_art_separates_with_gcr() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 100.0, 100.0, json!("#000000"));
    let doc = s.doc().unwrap().doc.clone();
    let k = render(
        &doc,
        RenderOptions { proof: Some(ProofSetup { separations: Some(vec!["Black".into()]), ..Default::default() }), ..Default::default() },
    )
    .pixel(50, 50);
    assert!(k[0] < 20, "RGB black is mostly K: {k:?}");
    let c = render(
        &doc,
        RenderOptions { proof: Some(ProofSetup { separations: Some(vec!["Cyan".into()]), ..Default::default() }), ..Default::default() },
    )
    .pixel(50, 50);
    assert!(c[0] > 40 && c[0] < 200, "rich black has some cyan: {c:?}");
}

#[test]
fn spot_colours_get_their_own_plate() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Brand Orange", "color": {"c": 0, "m": 60, "y": 100, "k": 0}, "global": true})).unwrap();
    s.execute("swatch.setSpot", &json!({"name": "Brand Orange"})).unwrap();
    assert!(s.execute("swatch.setSpot", &json!({"name": "Nope"})).is_err());
    let a = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [a.0], "swatch": "Brand Orange"})).unwrap();
    let plates = s.execute("color.plates", &json!({})).unwrap();
    let names: Vec<&str> = plates["plates"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Cyan", "Magenta", "Yellow", "Black", "Brand Orange"]);
    let doc = s.doc().unwrap().doc.clone();
    let plate = |n: &str| {
        render(&doc, RenderOptions { proof: Some(ProofSetup { separations: Some(vec![n.into()]), ..Default::default() }), ..Default::default() })
            .pixel(50, 50)
    };
    assert_eq!(plate("Brand Orange"), [0, 0, 0, 255]);
    assert_eq!(plate("Magenta"), [255, 255, 255, 255], "spot ink isn't on the process plates");
    // Panel commands: only / toggle.
    let r = s.execute("view.separationsPreview", &json!({"only": "Brand Orange"})).unwrap();
    let vis: Vec<bool> = r["plates"].as_array().unwrap().iter().map(|p| p["visible"].as_bool().unwrap()).collect();
    assert_eq!(vis, [false, false, false, false, true]);
    let r = s.execute("view.separationsPreview", &json!({"toggle": "Cyan"})).unwrap();
    assert_eq!(r["plates"][0]["visible"], true);
    assert_eq!(vectorcraft_render::proof::active_proof().unwrap().separations.unwrap().len(), 2);
    assert!(vectorcraft_render::proof::overprint_preview_on(), "separations imply overprint preview");
    assert!(s.execute("view.separationsPreview", &json!({"only": "Mauve"})).is_err());
    s.execute("view.separationsPreview", &json!({"on": false})).unwrap();
    assert!(vectorcraft_render::proof::active_proof().is_none_or(|p| p.separations.is_none()));
}

#[test]
fn overprint_black_multiplies_in_preview() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 100.0, 100.0, json!({"c": 100, "m": 0, "y": 0, "k": 0}));
    let k = rect(&mut s, 25.0, 25.0, 50.0, 50.0, json!({"c": 0, "m": 0, "y": 0, "k": 100}));
    let rgbk = rect(&mut s, 0.0, 0.0, 10.0, 10.0, json!("#000000"));
    s.execute("select.set", &json!({"ids": [k.0, rgbk.0]})).unwrap();
    let r = s.execute("edit.colors.overprintBlack", &json!({})).unwrap();
    assert_eq!(r["changed"], 1, "only the 100% K object (not RGB black) overprints");
    let doc = s.doc().unwrap().doc.clone();
    let knock = render(&doc, RenderOptions::default()).pixel(50, 50);
    let over = render(&doc, RenderOptions { overprint_preview: true, ..Default::default() }).pixel(50, 50);
    assert!((knock[0] as i32 - knock[2] as i32).abs() < 8, "knockout shows plain black: {knock:?}");
    assert!(over[2] as i32 > over[0] as i32 + 10, "overprint lets the cyan through: {over:?}");
    assert!(over[2] < knock[2] + 60 && over[2] <= 128, "and stays dark: {over:?}");
    s.execute("edit.colors.overprintBlack", &json!({"remove": true})).unwrap();
    assert!(!s.doc().unwrap().doc.node(k).unwrap().has_overprint());
}

#[test]
fn view_toggles_redraw_without_dirtying() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    let rev = s.doc().unwrap().revision;
    let r = s.execute("view.proofColors", &json!({"on": true})).unwrap();
    assert_eq!(r["proofColors"], true);
    assert!(s.doc().unwrap().revision > rev, "canvas redraws");
    assert!(!s.doc().unwrap().is_dirty(), "not marked modified");
    let r = s.execute("view.proofSetup", &json!({"target": "deuteranopia", "intent": "perceptual"})).unwrap();
    assert_eq!(r["target"], "deuteranopia");
    assert_eq!(vectorcraft_render::proof::active_proof().unwrap().target, ProofTarget::Deuteranopia);
    assert!(s.execute("view.proofSetup", &json!({"target": "cmyk:Nope"})).is_err());
    assert!(s.execute("view.proofSetup", &json!({"target": format!("cmyk:{}", cms::WIDE_GAMUT_RGB)})).is_err(), "an RGB profile");
    s.execute("view.proofSetup", &json!({"target": format!("cmyk:{}", cms::DEVICE_CMYK)})).unwrap();
    s.execute("view.proofSetup", &json!({"target": "workingCmyk", "intent": "relative"})).unwrap();
    let r = s.execute("view.proofColors", &json!({"on": false})).unwrap();
    assert_eq!(r["proofColors"], false);
    let r = s.execute("view.overprintPreview", &json!({"on": true})).unwrap();
    assert_eq!(r["overprintPreview"], true);
    s.execute("view.overprintPreview", &json!({"on": false})).unwrap();
}

#[test]
fn proof_setup_simulates_tritanopia() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 100.0, 100.0, json!("#0000ff"));
    let r = s.execute("view.proofSetup", &json!({"target": "tritanopia", "proof": true})).unwrap();
    assert_eq!(r["target"], "tritanopia");
    let setup = vectorcraft_render::proof::active_proof().unwrap();
    assert_eq!(setup.target, ProofTarget::Tritanopia);
    let doc = s.doc().unwrap().doc.clone();
    let px = render(&doc, RenderOptions { proof: Some(setup), ..Default::default() }).pixel(50, 50);
    // A tritanope sees blue as a dark teal.
    assert!(px[0] < 8 && px[1] > 80 && px[2] > 110 && px[2] < 150, "{px:?}");
    s.execute("view.proofSetup", &json!({"target": "workingCmyk", "proof": false})).unwrap();
}

/// Whether the content has a DeviceCMYK fill/stroke operator (`c m y k k` / `K`).
fn has_cmyk_op(pdf: &str) -> bool {
    let toks: Vec<&str> = pdf.split_ascii_whitespace().collect();
    toks.windows(5).any(|w| (w[4] == "k" || w[4] == "K") && w[..4].iter().all(|t| t.parse::<f32>().is_ok()))
}

fn pdf_text(doc: &Document) -> String {
    let bytes =
        vectorcraft_pdf::export(doc, &vectorcraft_pdf::PdfOptions { created: Some(0), ..vectorcraft_pdf::PdfOptions::uncompressed() }).unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn pdf_cmyk_document_writes_device_cmyk() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 10.0, 10.0, json!("#ff0000"));
    let rgb_pdf = pdf_text(&s.doc().unwrap().doc);
    assert!(!has_cmyk_op(&rgb_pdf) && rgb_pdf.contains(" rg"));
    s.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk"})).unwrap();
    let pdf = pdf_text(&s.doc().unwrap().doc);
    assert!(has_cmyk_op(&pdf), "CMYK colours are written with the DeviceCMYK `k` operator");
    // RGB colours pasted into a CMYK document are separated on export.
    let mut s2 = session();
    rect(&mut s2, 0.0, 0.0, 10.0, 10.0, json!("#336699"));
    let mut d = (*s2.doc().unwrap().doc).clone();
    d.color_mode = ColorMode::Cmyk;
    assert!(has_cmyk_op(&pdf_text(&d)));
}

#[test]
fn pdf_spot_colour_is_a_separation() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "PMSish 123", "color": {"c": 0, "m": 20, "y": 90, "k": 0}, "global": true})).unwrap();
    s.execute("swatch.setSpot", &json!({"name": "PMSish 123"})).unwrap();
    let a = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [a.0], "swatch": "PMSish 123"})).unwrap();
    let pdf = pdf_text(&s.doc().unwrap().doc);
    assert!(pdf.contains("/Separation"), "spot → Separation colour space");
    assert!(pdf.contains("PMSish#20123"), "colorant name");
    assert!(pdf.contains("DeviceCMYK"), "CMYK alternate space");
}

#[test]
fn intents_reach_the_document_conversion() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut a = session();
    rect(&mut a, 0.0, 0.0, 10.0, 10.0, json!("#00ff00"));
    let mut b = session();
    rect(&mut b, 0.0, 0.0, 10.0, 10.0, json!("#00ff00"));
    a.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk", "intent": "perceptual"})).unwrap();
    b.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk", "intent": "relative"})).unwrap();
    let first = |s: &Session| {
        let mut out = None;
        s.doc().unwrap().doc.walk(|n| {
            if out.is_none()
                && let Some(c) = n.appearance.fill_paint().color()
                && n.children().is_none()
            {
                out = Some(c);
            }
        });
        out.unwrap()
    };
    assert_ne!(first(&a), first(&b));
    let _ = Intent::Perceptual;
}

/// The error `cmd` gives for `params`: it must name `msg`.
fn refused(s: &mut Session, cmd: &str, params: Value, msg: &str) {
    let e = match s.execute(cmd, &params) {
        Ok(r) => panic!("{cmd} {params} gave {r}"),
        Err(e) => e.to_string(),
    };
    assert!(e.contains(msg), "{cmd} {params}: {e}");
}

#[test]
fn color_settings_refuse_a_param_of_the_wrong_type() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    let before = cms::active_settings();
    let rev = s.doc().unwrap().revision;
    let c = "edit.colorSettings";
    refused(&mut s, c, json!({"bpc": "no"}), "`bpc` must be true or false");
    refused(&mut s, c, json!({"rgb": 5}), "`rgb` must be a string");
    refused(&mut s, c, json!({"cmyk": true}), "`cmyk` must be a string");
    refused(&mut s, c, json!({"intent": 2}), "`intent` must be a string");
    assert_eq!(cms::active_settings(), before, "the settings are unchanged");
    assert_eq!(s.doc().unwrap().revision, rev, "nothing redraws");
    // Omitted and null params keep the settings.
    s.execute(c, &json!({"bpc": null, "rgb": null})).unwrap();
    assert_eq!(cms::active_settings(), before);
    refused(&mut s, "color.convert", json!({"color": "#808080", "to": 1}), "`to` must be a string");
    refused(&mut s, "color.convert", json!({"color": "#808080", "intent": ["relative"]}), "`intent` must be a string");
    assert_eq!(s.execute("color.convert", &json!({"color": "#808080", "to": null})).unwrap()["model"], "rgb");
    refused(&mut s, "color.loadProfile", json!({"path": 7}), "`path` must be a string");
    refused(&mut s, "color.removeProfile", json!({"name": 7}), "`name` must be a string");
}

#[test]
fn document_color_mode_refuses_a_param_of_the_wrong_type() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0, json!("#ff0000"));
    let fill = fill_of(&s, a);
    let undo = s.doc().unwrap().history.undo.len();
    for c in ["file.documentColorMode", "object.convertDocumentColorMode"] {
        refused(&mut s, c, json!({"mode": "cmyk", "convert": "no"}), "`convert` must be true or false");
        refused(&mut s, c, json!({"mode": "rgb", "convert": 0}), "`convert` must be true or false");
        refused(&mut s, c, json!({"mode": "cmyk", "grays": 1}), "`grays` must be a string");
        refused(&mut s, c, json!({"mode": "cmyk", "intent": false}), "`intent` must be a string");
    }
    let st = s.doc().unwrap();
    assert_eq!(st.doc.color_mode, ColorMode::Rgb);
    assert_eq!(st.history.undo.len(), undo, "no undo step");
    assert_eq!(fill_of(&s, a), fill, "the colors are unchanged");
}

#[test]
fn proof_views_refuse_a_param_of_the_wrong_type() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    let view = vectorcraft_render::proof::view();
    let rev = s.doc().unwrap().revision;
    refused(&mut s, "view.proofSetup", json!({"simulatePaper": "yes"}), "`simulatePaper` must be true or false");
    refused(&mut s, "view.proofSetup", json!({"proof": 1}), "`proof` must be true or false");
    refused(&mut s, "view.proofSetup", json!({"target": 3}), "`target` must be a string");
    refused(&mut s, "view.proofColors", json!({"on": "true"}), "`on` must be true or false");
    refused(&mut s, "view.overprintPreview", json!({"on": 1}), "`on` must be true or false");
    refused(&mut s, "view.separationsPreview", json!({"on": "false"}), "`on` must be true or false");
    refused(&mut s, "view.separationsPreview", json!({"plates": "Cyan"}), "`plates` must be an array of these names: Cyan, Magenta");
    refused(&mut s, "view.separationsPreview", json!({"plates": ["Cyan", 2]}), "`plates` must be an array of these names");
    refused(&mut s, "view.separationsPreview", json!({"only": 0}), "`only` must be a string");
    refused(&mut s, "view.separationsPreview", json!({"toggle": ["Cyan"]}), "`toggle` must be a string");
    assert_eq!(vectorcraft_render::proof::view(), view, "the proof view is unchanged");
    assert_eq!(s.doc().unwrap().revision, rev, "nothing redraws");
    // A null `on` toggles, as an omitted one does.
    let r = s.execute("view.proofColors", &json!({"on": null})).unwrap();
    assert_eq!(r["proofColors"], !view.proof_colors);
    s.execute("view.proofColors", &json!({"on": view.proof_colors})).unwrap();
}

#[test]
fn set_spot_refuses_a_param_of_the_wrong_type() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Ink", "color": {"c": 0, "m": 60, "y": 100, "k": 0}, "global": true})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    refused(&mut s, "swatch.setSpot", json!({"name": "Ink", "spot": "true"}), "`spot` must be true or false");
    refused(&mut s, "swatch.setSpot", json!({"name": 1}), "`name` must be a string");
    assert!(!s.doc().unwrap().doc.swatch("Ink").unwrap().spot, "the swatch stays a process color");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "no undo step");
    s.execute("swatch.setSpot", &json!({"name": "Ink", "spot": null})).unwrap();
    assert!(s.doc().unwrap().doc.swatch("Ink").unwrap().spot, "a null `spot` is the default, true");
}

#[test]
fn gamut_check_refuses_colors_that_are_not_a_list() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 10.0, 10.0, json!("#0000ff"));
    s.execute("select.set", &json!({"ids": []})).unwrap();
    refused(&mut s, "color.gamutCheck", json!({"colors": "#00ff00"}), "bad color \"#00ff00\"");
    refused(&mut s, "color.gamutCheck", json!({"colors": 5}), "bad color 5");
    refused(&mut s, "color.gamutCheck", json!({"colors": {"c": 100, "m": 0, "y": 0, "k": 0}}), "bad color");
    // A null list checks the document, as an omitted one does.
    let r = s.execute("color.gamutCheck", &json!({"colors": null})).unwrap();
    assert_eq!(r, s.execute("color.gamutCheck", &json!({})).unwrap());
    assert!(r["outOfGamut"].as_array().unwrap().iter().any(|v| v["hex"] == "#0000ff"), "{r}");
}
