//! Spot colours defined in Lab: Lab swatches through Swatch Options, the Spot Colors options
//! (Lab values or CMYK equivalents for display and separations), Lab alternates in PDF, and the
//! saved setting.

use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Model};
use vectorcraft_color::{Color, Paint};
use vectorcraft_render::proof;

use crate::tests_colormgmt::GLOBAL;
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

fn fill(s: &Session, id: NodeId) -> Paint {
    doc(s).node(id).unwrap().appearance.fill_paint()
}

const LAB: Color = Color::Lab { l: 55.0, a: 60.0, b: 40.0 };

fn linked(color: Color, tint: f32) -> Paint {
    Paint::Solid { color, swatch: Some("Lab Ink".into()), tint }
}

/// The working-CMYK equivalent of a Lab colour, what CMYK spot options show.
fn cmyk_of(c: Color) -> Color {
    let cms = cms::active();
    cms.convert(&c, Model::Cmyk, cms.settings().intent)
}

/// A document with the Lab spot swatch "Lab Ink" and a rectangle filled with 40 % of it.
fn lab_ink() -> (Session, NodeId) {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Lab Ink", "color": {"l": 55, "a": 60, "b": 40}, "spot": true}));
    let id = NodeId(run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30}))["id"].as_u64().unwrap());
    run(&mut s, "paint.setFill", json!({"ids": [id.0], "swatch": "Lab Ink", "tint": 40}));
    (s, id)
}

#[test]
fn swatch_options_define_spot_colours_in_lab() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 80, "y": 60, "k": 0}, "spot": true}));
    let id = NodeId(run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].as_u64().unwrap());
    run(&mut s, "paint.setFill", json!({"ids": [id.0], "swatch": "Ink"}));
    // Lab mode converts the swatch's colour through the CMS; linked art follows.
    run(&mut s, "swatch.edit", json!({"name": "Ink", "mode": "lab"}));
    let lab = doc(&s).swatch("Ink").unwrap().paint.color().unwrap();
    assert_eq!(lab.model(), Model::Lab);
    let back = cmyk_of(lab);
    let Color::Cmyk { c, m, y, k } = back else { panic!("{back:?}") };
    assert!(c < 0.03 && (m - 0.8).abs() < 0.03 && (y - 0.6).abs() < 0.03 && k < 0.03, "Lab of C0 M80 Y60 K0 separates back: {back:?}");
    assert_eq!(fill(&s, id), Paint::Solid { color: lab, swatch: Some("Ink".into()), tint: 1.0 });
    // Lab values given directly; the swatch list reports the model.
    let r = run(&mut s, "swatch.edit", json!({"name": "Ink", "color": {"l": 55, "a": 60, "b": 40}}));
    assert_eq!(r["relinked"], 1);
    assert_eq!(fill(&s, id).color(), Some(LAB));
    let list = run(&mut s, "swatch.list", json!({}));
    let ink = list["swatches"].as_array().unwrap().iter().find(|w| w["name"] == "Ink").unwrap();
    assert_eq!((ink["color"]["model"].as_str(), ink["spot"].as_bool()), (Some("lab"), Some(true)));
    assert!(s.execute("swatch.edit", &json!({"name": "Ink", "mode": "xyz"})).unwrap_err().to_string().contains("lab"));
}

#[test]
fn spot_colors_options_switch_lab_spots_between_lab_and_cmyk() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut s, id) = lab_ink();
    assert!(doc(&s).spot_use_lab, "Lab values by default");
    assert_eq!(fill(&s, id), linked(LAB.tinted(0.4), 0.4));
    // A gradient stop and a tint swatch of the ink follow too; a process swatch doesn't.
    run(&mut s, "swatch.new", json!({"swatch": "Lab Ink", "tint": 70}));
    let g = NodeId(run(&mut s, "shape.rectangle", json!({"x": 50, "y": 10, "width": 30, "height": 30}))["id"].as_u64().unwrap());
    run(
        &mut s,
        "paint.setFill",
        json!({"ids": [g.0], "gradient": {"stops": [{"offset": 0, "swatch": "Lab Ink"}, {"offset": 1, "color": "#ffffff"}]}}),
    );
    assert_eq!(run(&mut s, "swatch.spotOptions", json!({})), json!({"useLab": true, "relinked": 0}), "no change: a query");

    let r = run(&mut s, "swatch.spotOptions", json!({"useLab": false}));
    assert_eq!(r, json!({"useLab": false, "relinked": 3}));
    let cmyk = cmyk_of(LAB);
    assert!(matches!(cmyk, Color::Cmyk { .. }));
    assert_eq!(fill(&s, id), linked(cmyk.tinted(0.4), 0.4), "the CMYK equivalent at the same tint");
    assert_eq!(doc(&s).swatch("Lab Ink 70%").unwrap().paint, linked(cmyk.tinted(0.7), 0.7));
    let Paint::Gradient(gp) = fill(&s, g) else { panic!("gradient") };
    assert_eq!((gp.gradient.stops[0].color, gp.gradient.stops[0].swatch.as_deref()), (cmyk, Some("Lab Ink")));
    assert_eq!(doc(&s).swatch("Lab Ink").unwrap().paint.color(), Some(LAB), "the definition stays Lab");
    assert_eq!(doc(&s).global_color("Lab Ink"), Some(cmyk));
    // New art takes the CMYK equivalent; the Separations plate shows it.
    run(&mut s, "paint.setFill", json!({"ids": [id.0], "swatch": "Lab Ink", "tint": 50}));
    assert_eq!(fill(&s, id), linked(cmyk.tinted(0.5), 0.5));
    let plate = |s: &Session| proof::plates(doc(s)).into_iter().find(|p| p.name == "Lab Ink").unwrap().rgb;
    assert_eq!(plate(&s), cmyk.to_rgb());
    // One undo step back to Lab values.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(doc(&s).spot_use_lab);
    assert_eq!(fill(&s, id), linked(LAB.tinted(0.4), 0.4));
    assert_eq!(plate(&s), LAB.to_rgb());
    assert!(s.execute("swatch.spotOptions", &json!({"useLab": "yes"})).is_err());
}

#[test]
fn colour_values_parse_lab() {
    let mut s = session();
    let id = NodeId(run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].as_u64().unwrap());
    run(&mut s, "paint.setFill", json!({"ids": [id.0], "color": {"l": 50, "a": -20, "b": 30.5}}));
    assert_eq!(fill(&s, id).color(), Some(Color::lab(50.0, -20.0, 30.5)));
    run(&mut s, "paint.setFill", json!({"ids": [id.0], "color": {"model": "lab", "l": 20, "a": 1, "b": 2}}));
    assert_eq!(fill(&s, id).color(), Some(Color::lab(20.0, 1.0, 2.0)));
    // Lab colours keep their model through colour operations.
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "paint.complement", json!({}));
    assert_eq!(fill(&s, id).color(), Some(Color::lab(20.0, -1.0, -2.0))); // A Lab swatch is named by its values.
    let r = run(&mut s, "swatch.new", json!({"color": {"l": 50.4, "a": -20, "b": -0.2}}));
    assert_eq!(r["name"], "L=50 a=-20 b=0");
}

#[test]
fn spot_options_and_lab_colours_round_trip() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut s, id) = lab_ink();
    let saved = vectorcraft_format::save(doc(&s), false);
    assert!(!String::from_utf8_lossy(&saved).contains("spot_use_lab"), "the default isn't written");
    run(&mut s, "swatch.spotOptions", json!({"useLab": false}));
    let d = doc(&s).clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&d, false)).unwrap();
    assert!(!back.spot_use_lab);
    assert_eq!(back.swatch("Lab Ink").unwrap().paint.color(), Some(LAB));
    assert_eq!(back.node(id).unwrap().appearance.fill_paint(), d.node(id).unwrap().appearance.fill_paint());
    // Files from before the option use Lab values.
    let mut v: Value = serde_json::to_value(&d).unwrap();
    v.as_object_mut().unwrap().remove("spot_use_lab");
    assert!(serde_json::from_value::<vectorcraft_doc::Document>(v).unwrap().spot_use_lab);
}

fn pdf_bytes(s: &Session) -> Vec<u8> {
    let opts = vectorcraft_pdf::PdfOptions { created: Some(0), ..vectorcraft_pdf::PdfOptions::uncompressed() };
    vectorcraft_pdf::export(doc(s), &opts).unwrap()
}

/// Every in-use cross-reference entry points at its object, and `startxref` at the table.
fn assert_xref_valid(pdf: &[u8]) {
    let sx = pdf.windows(9).rposition(|w| w == b"startxref").unwrap();
    let xref: usize = std::str::from_utf8(&pdf[sx + 9..]).unwrap().split_whitespace().next().unwrap().parse().unwrap();
    assert!(pdf[xref..].starts_with(b"xref\n0 "), "startxref → the table");
    // The table and trailer are ASCII.
    let mut lines = std::str::from_utf8(&pdf[xref..]).unwrap().lines().skip(1);
    let count: usize = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    for (id, e) in lines.take(count).enumerate() {
        if e.trim_end().ends_with('n') {
            let off: usize = e[..10].parse().unwrap();
            assert!(pdf[off..].starts_with(format!("{id} 0 obj").as_bytes()), "object {id} at {off}");
        }
    }
}

#[test]
fn lab_spots_export_with_a_lab_alternate() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut s, _) = lab_ink();
    // A tint gradient of the ink is a Separation shading of the same space.
    let g = NodeId(run(&mut s, "shape.rectangle", json!({"x": 50, "y": 10, "width": 30, "height": 30}))["id"].as_u64().unwrap());
    run(
        &mut s,
        "paint.setFill",
        json!({"ids": [g.0], "gradient": {"stops": [{"offset": 0, "swatch": "Lab Ink"}, {"offset": 1, "color": "#ffffff"}]}}),
    );
    let pdf = pdf_bytes(&s);
    let text = String::from_utf8_lossy(&pdf);
    assert!(
        text.contains("[/Separation/Lab#20Ink[/Lab<</WhitePoint[0.9642 1 0.8252]/Range[-128 127 -128 127]>>]<</FunctionType 2/Domain[0 1]/Range[0 100 -128 127 -128 127]/C0[100 0 0]/C1[55 60 40]/N 1>>]"),
        "a Lab alternate running from paper white to the Lab values"
    );
    assert!(!text.contains("/Separation/Lab#20Ink/DeviceCMYK"));
    assert!(text.contains("/ShadingType 2"), "the gradient is still a shading");
    assert_xref_valid(&pdf);
    // The file reads back: the 40 % tint shows as that tint of the Lab colour.
    let back = vectorcraft_pdf::import(&pdf).unwrap();
    let mut fills = vec![];
    back.visit_paints(&mut |p| fills.extend(p.color()));
    let want = LAB.tinted(0.4).to_rgba8(1.0);
    assert!(fills.iter().any(|c| c.to_rgba8(1.0).iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 3)), "{fills:?} has {want:?}");
    // CMYK values: a DeviceCMYK alternate, as for process-defined spots.
    run(&mut s, "swatch.spotOptions", json!({"useLab": false}));
    let pdf = pdf_bytes(&s);
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/Separation/Lab#20Ink/DeviceCMYK") && !text.contains("/Lab<<"));
    assert_xref_valid(&pdf);
}
