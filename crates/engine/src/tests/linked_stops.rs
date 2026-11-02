//! Gradient swatches and gradient stops linked to global and spot swatches: the recorded links,
//! stops following swatch edits, spot stops on their plate and as Separation and DeviceN shadings in PDF.

use serde_json::{Value, json};
use vectorcraft_color::{Color, GradientPaint, GradientStop, Paint};
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

const INK: Color = Color::Cmyk { c: 0.0, m: 1.0, y: 0.0, k: 0.0 };

/// A document with spot swatch "Ink" and a selected rectangle.
fn setup() -> (Session, NodeId) {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Ink", "color": INK, "spot": true}));
    let id = NodeId(run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    (s, id)
}

fn gradient(s: &Session, id: NodeId) -> GradientPaint {
    match doc(s).node(id).unwrap().appearance.fill_paint() {
        Paint::Gradient(g) => *g,
        p => panic!("not a gradient: {p:?}"),
    }
}

/// Ink at `tint` % → white.
fn ink_to_white(tint: f64) -> Value {
    json!({"stops": [{"offset": 0, "swatch": "Ink", "tint": tint}, {"offset": 1, "color": "#ffffff"}]})
}

#[test]
fn applying_a_gradient_swatch_records_the_link_until_the_stops_change() {
    let (mut s, id) = setup();
    run(&mut s, "paint.setFill", json!({"swatch": "Sunset"}));
    assert_eq!(gradient(&s, id).swatch.as_deref(), Some("Sunset"));
    let used = run(&mut s, "swatch.unused", json!({}));
    assert!(!used["names"].as_array().unwrap().contains(&json!("Sunset")));
    run(&mut s, "paint.editGradient", json!({"angle": 45}));
    assert_eq!(gradient(&s, id).swatch.as_deref(), Some("Sunset"), "moving the vector keeps it");
    run(&mut s, "paint.editGradient", ink_to_white(100.0));
    assert_eq!(gradient(&s, id).swatch, None, "new stops are another gradient");
}

#[test]
fn stops_link_to_swatches_and_follow_their_edits() {
    let (mut s, id) = setup();
    run(&mut s, "paint.editGradient", ink_to_white(50.0));
    let st = gradient(&s, id).gradient.stops[0].clone();
    assert_eq!((st.swatch.as_deref(), st.tint, st.color), (Some("Ink"), 0.5, INK.tinted(0.5)), "the colour comes from the swatch");
    assert_eq!(gradient(&s, id).gradient.stops[1].swatch, None);
    // A tint swatch links the stop to its base at its tint; a process swatch just gives its colour.
    run(&mut s, "paint.setFill", json!({"swatch": "Ink", "tint": 40}));
    run(&mut s, "swatch.new", json!({}));
    run(&mut s, "paint.setFill", json!({"gradient": {"stops": [{"offset": 0, "swatch": "Ink 40%"}, {"offset": 1, "swatch": "Black"}]}}));
    let stops = gradient(&s, id).gradient.stops;
    assert_eq!((stops[0].swatch.as_deref(), stops[0].tint), (Some("Ink"), 0.4));
    assert_eq!((stops[1].swatch.as_deref(), stops[1].tint), (None, 1.0));
    assert!(s.execute("paint.editGradient", &json!({"stops": [{"offset": 0, "swatch": "Nope"}, {"offset": 1, "color": "#fff"}]})).is_err());
    // Editing the swatch recolours the stops (and the tint swatch) as one undo step.
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "swatch.edit", json!({"name": "Ink", "color": {"c": 1, "m": 0, "y": 0, "k": 0}}));
    let cyan = Color::cmyk(1.0, 0.0, 0.0, 0.0);
    assert_eq!(gradient(&s, id).gradient.stops[0].color, cyan.tinted(0.4));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    // Global mode of Adjust Color Balance shifts stop tints too.
    run(&mut s, "edit.colors.adjustBalance", json!({"tint": 60}));
    assert_eq!(gradient(&s, id).gradient.stops[0].color, cyan);
    // Recolouring a stop unlinks it; deleting the swatch unlinks the rest, which keep their colour.
    run(&mut s, "paint.editGradient", ink_to_white(25.0));
    run(&mut s, "swatch.delete", json!({"name": "Ink"}));
    let st = gradient(&s, id).gradient.stops[0].clone();
    assert_eq!((st.swatch, st.tint, st.color), (None, 1.0, cyan.tinted(0.25)));
}

#[test]
fn gradient_swatches_keep_their_stop_links_and_follow_edits() {
    let (mut s, id) = setup();
    run(&mut s, "swatch.new", json!({"name": "Glow", "gradient": ink_to_white(100.0)}));
    run(&mut s, "paint.setFill", json!({"swatch": "Glow"}));
    run(&mut s, "swatch.edit", json!({"name": "Ink", "color": {"c": 0, "m": 0, "y": 1, "k": 0}}));
    let yellow = Color::cmyk(0.0, 0.0, 1.0, 0.0);
    let Paint::Gradient(sw) = doc(&s).swatch("Glow").unwrap().paint.clone() else { panic!() };
    assert_eq!(sw.gradient.stops[0].color, yellow, "the gradient swatch's stop follows");
    let g = gradient(&s, id);
    assert_eq!((g.gradient.stops[0].color, g.swatch.as_deref()), (yellow, Some("Glow")), "so does the art, still linked to its swatch");
    assert_eq!(g.gradient, sw.gradient);
}

#[test]
fn spot_stops_separate_on_their_plate() {
    let (mut s, id) = setup();
    run(&mut s, "paint.editGradient", ink_to_white(60.0));
    let mut node = doc(&s).node(id).unwrap().clone();
    let mut links = vec![];
    proof::map_node_colors(&mut node, &mut |c, link| {
        links.push(link.map(|(n, t)| (n.to_string(), t)));
        *c
    });
    assert!(links.contains(&Some(("Ink".to_string(), 0.6))), "{links:?}");
    let c = vectorcraft_color::cms::active();
    let st = gradient(&s, id).gradient.stops[0].clone();
    let ink = proof::inks(doc(&s), &c, &st.color, Some(("Ink", st.tint)), proof::Intent::RelativeColorimetric);
    assert_eq!((ink.cmyk, ink.spot), ([0.0; 4], Some(("Ink".to_string(), 0.6))));
}

fn pdf_of(s: &Session) -> (String, Vec<String>) {
    let opts = vectorcraft_pdf::PdfOptions { created: Some(0), ..vectorcraft_pdf::PdfOptions::uncompressed() };
    let r = vectorcraft_pdf::export_with_report(doc(s), &opts).unwrap();
    (String::from_utf8_lossy(&r.bytes).into_owned(), r.warnings)
}

#[test]
fn spot_gradients_export_as_separation_shadings() {
    let (mut s, _) = setup();
    run(&mut s, "paint.editGradient", ink_to_white(80.0));
    let (pdf, _) = pdf_of(&s);
    assert!(pdf.contains("/ShadingType 2") && pdf.contains("/Separation/Ink/DeviceCMYK"), "an Ink → paper gradient is a Separation shading");
    // Its function runs over the tints: 80 % (204 / 255) to 0 %.
    assert!(pdf.contains("/C0[0.8]/C1[0]"), "the shading's tints");
    // Mixing the spot ink with an RGB color writes the gradient without the spot ink, with a warning.
    run(&mut s, "paint.editGradient", json!({"stops": [{"offset": 0, "swatch": "Ink"}, {"offset": 1, "color": "#00ff00"}]}));
    let (pdf, warnings) = pdf_of(&s);
    assert!(!pdf.contains("/Separation") && !pdf.contains("/DeviceN"), "no spot ink for a gradient with an RGB stop");
    assert!(warnings.iter().any(|w| w.contains("spot")), "{warnings:?}");
}

#[test]
fn spot_gradients_mixed_with_other_inks_export_as_device_n_shadings() {
    let (mut s, _) = setup();
    // Ink → process yellow: a DeviceN shading of Yellow and Ink, with no warning.
    run(&mut s, "paint.editGradient", json!({"stops": [{"offset": 0, "swatch": "Ink"}, {"offset": 1, "color": {"c": 0, "m": 0, "y": 0.6, "k": 0}}]}));
    let (pdf, warnings) = pdf_of(&s);
    assert!(pdf.contains("/ShadingType 2") && pdf.contains("/DeviceN[/Yellow/Ink]/DeviceCMYK"), "a DeviceN shading of Yellow and Ink");
    assert!(pdf.contains("/C0[0 1]/C1[0.6 0]"), "the shading's tints: Yellow and Ink");
    assert!(!warnings.iter().any(|w| w.contains("spot")), "{warnings:?}");
    // Two spot inks, with an off-center midpoint: a stop halfway between the two inks.
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": {"c": 1, "m": 0, "y": 1, "k": 0}, "spot": true}));
    run(
        &mut s,
        "paint.editGradient",
        json!({"stops": [{"offset": 0, "swatch": "Ink", "midpoint": 30}, {"offset": 1, "swatch": "Leaf", "tint": 50}]}),
    );
    let (pdf, warnings) = pdf_of(&s);
    assert!(pdf.contains("/DeviceN[/Ink/Leaf]/DeviceCMYK"), "a DeviceN shading of both spot inks");
    let functions: Vec<_> = pdf.match_indices("/C0[").filter_map(|(i, _)| pdf.get(i..i + 32)).collect();
    assert!(
        pdf.contains("/C0[1 0]/C1[0.5019608 0.2509804]") && pdf.contains("/C0[0.5019608 0.2509804]/C1[0 0.5019608]"),
        "the tints through the midpoint: {functions:?}"
    );
    assert!(!warnings.iter().any(|w| w.contains("spot")), "{warnings:?}");
}

#[test]
fn radial_spot_gradients_mixed_with_other_inks_export_as_device_n_shadings() {
    let (mut s, _) = setup();
    run(
        &mut s,
        "paint.editGradient",
        json!({"kind": "radial", "stops": [{"offset": 0, "swatch": "Ink"}, {"offset": 1, "color": {"c": 0, "m": 0, "y": 0.6, "k": 0}}]}),
    );
    let (pdf, warnings) = pdf_of(&s);
    assert!(pdf.contains("/ShadingType 3") && pdf.contains("/DeviceN[/Yellow/Ink]/DeviceCMYK"), "a radial DeviceN shading of Yellow and Ink");
    assert!(pdf.contains("/C0[0 1]/C1[0.6 0]"), "the shading's tints: Yellow and Ink");
    assert!(!warnings.iter().any(|w| w.contains("spot")), "{warnings:?}");
}

#[test]
fn linked_stops_round_trip() {
    let (mut s, id) = setup();
    run(&mut s, "paint.editGradient", ink_to_white(30.0));
    let d = doc(&s).clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&d, false)).unwrap();
    assert_eq!(back.node(id).unwrap().appearance.fill_paint(), d.node(id).unwrap().appearance.fill_paint());
    // Stops from before links read as unlinked.
    let old: GradientStop = serde_json::from_value(json!({"offset": 0.5, "color": {"model": "rgb", "r": 1, "g": 0, "b": 0}})).unwrap();
    assert_eq!(old, GradientStop::new(0.5, Color::rgb(1.0, 0.0, 0.0)));
    assert!(!serde_json::to_string(&old).unwrap().contains("tint"));
}

/// `stops` with `extra` (method, dither…) as a gradient edit.
fn with(mut stops: Value, extra: Value) -> Value {
    if let (Some(g), Some(e)) = (stops.as_object_mut(), extra.as_object()) {
        g.extend(e.clone());
    }
    stops
}

#[test]
fn perceptual_spot_gradients_export_as_separation_shadings() {
    let (mut s, _) = setup();
    let ink_to_tint = json!({"stops": [{"offset": 0, "swatch": "Ink"}, {"offset": 1, "swatch": "Ink", "tint": 20}]});
    for (what, stops) in [("Ink → paper", ink_to_white(80.0)), ("Ink → Ink 20 %", ink_to_tint)] {
        run(&mut s, "paint.editGradient", with(stops, json!({"method": "perceptual"})));
        let (pdf, warnings) = pdf_of(&s);
        assert!(!pdf.contains("/Subtype/Image"), "{what}: no image");
        assert!(pdf.contains("/ShadingType 2") && pdf.contains("/Separation/Ink/DeviceCMYK"), "{what}: a Separation shading");
        // The curve's stops: a stitching function of more than the two ends.
        assert!(pdf.contains("/FunctionType 3"), "{what}: tints on the curve");
        assert!(!warnings.iter().any(|w| w.contains("spot")), "{what}: {warnings:?}");
    }
    // A radial one, and one on a stroke, too.
    run(&mut s, "paint.editGradient", with(ink_to_white(100.0), json!({"method": "perceptual", "kind": "radial"})));
    let (pdf, _) = pdf_of(&s);
    assert!(!pdf.contains("/Subtype/Image") && pdf.contains("/Separation/Ink/DeviceCMYK"), "radial");
    run(&mut s, "paint.setFill", json!({"color": "#ffffff"}));
    run(&mut s, "paint.setStroke", json!({"color": "#000000"}));
    run(&mut s, "stroke.set", json!({"weight": 6}));
    run(&mut s, "paint.editGradient", with(ink_to_white(100.0), json!({"method": "perceptual", "stroke": true})));
    let (pdf, _) = pdf_of(&s);
    assert!(!pdf.contains("/Subtype/Image") && pdf.contains("/Separation/Ink/DeviceCMYK"), "stroke");
    // A white spot ink has paper white's lightness at every tint: the stops keep their own tints.
    run(&mut s, "swatch.new", json!({"name": "Opaque", "color": "#ffffff", "spot": true}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    let white =
        json!({"stops": [{"offset": 0, "swatch": "Opaque"}, {"offset": 1, "swatch": "Opaque", "tint": 40}], "method": "perceptual", "stroke": false});
    run(&mut s, "paint.editGradient", white);
    let (pdf, _) = pdf_of(&s);
    let at = pdf.find("/FunctionType").unwrap_or(0);
    assert!(!pdf.contains("/Subtype/Image") && pdf.contains("/Separation/Opaque"), "white ink: {}", &pdf[at..(at + 200).min(pdf.len())]);
    assert!(pdf.contains("/C0[1]") && pdf.contains("/C1[0.4"), "white ink at 100% to 40%: {}", &pdf[at..(at + 200).min(pdf.len())]);
}

#[test]
fn perceptual_spot_gradients_mixed_with_other_inks_export_as_device_n_shadings() {
    let (mut s, _) = setup();
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": {"c": 1, "m": 0, "y": 1, "k": 0}, "spot": true}));
    let ink_to_cyan = json!({"stops": [{"offset": 0, "swatch": "Ink"}, {"offset": 1, "color": {"c": 1, "m": 0, "y": 0, "k": 0}}]});
    let ink_to_leaf = json!({"stops": [{"offset": 0, "swatch": "Ink", "midpoint": 30}, {"offset": 1, "swatch": "Leaf", "tint": 50}]});
    for (what, edit, space) in [
        ("Ink → cyan", with(ink_to_cyan.clone(), json!({"method": "perceptual"})), "/DeviceN[/Cyan/Ink]/DeviceCMYK"),
        ("Ink → Leaf 50 %", with(ink_to_leaf, json!({"method": "perceptual"})), "/DeviceN[/Ink/Leaf]/DeviceCMYK"),
    ] {
        run(&mut s, "paint.editGradient", edit);
        let (pdf, warnings) = pdf_of(&s);
        assert!(!pdf.contains("/Subtype/Image"), "{what}: no image");
        assert!(pdf.contains("/ShadingType 2") && pdf.contains(space), "{what}: a DeviceN shading");
        // The curve's stops: a stitching function of more than the two ends.
        assert!(pdf.contains("/FunctionType 3"), "{what}: inks on the curve");
        assert!(!warnings.iter().any(|w| w.contains("spot")), "{what}: {warnings:?}");
    }
    // The curve starts at the first stop's inks: no cyan, Ink at 100 %.
    run(&mut s, "paint.editGradient", with(ink_to_cyan.clone(), json!({"method": "perceptual"})));
    let (pdf, _) = pdf_of(&s);
    assert!(pdf.contains("/C0[0 1]"), "the first stop's inks");
    // A radial one too.
    run(&mut s, "paint.editGradient", with(ink_to_cyan, json!({"method": "perceptual", "kind": "radial"})));
    let (pdf, _) = pdf_of(&s);
    assert!(!pdf.contains("/Subtype/Image") && pdf.contains("/ShadingType 3") && pdf.contains("/DeviceN[/Cyan/Ink]/DeviceCMYK"), "radial");
}

#[test]
fn spot_gradients_drawn_as_images_warn() {
    let (mut s, _) = setup();
    let spot_warning = |w: &[String]| w.iter().any(|w| w.contains("spot colors in gradients exported as images"));
    let ink_to_green = json!({"stops": [{"offset": 0, "swatch": "Ink"}, {"offset": 1, "color": "#00ff00"}]});
    // More stops than the export finds tints for: an image, as before.
    let n = 4097;
    let stops: Vec<Value> = (0..n)
        .map(|i| {
            let offset = i as f64 / (n - 1) as f64;
            if i % 2 == 0 { json!({"offset": offset, "swatch": "Ink"}) } else { json!({"offset": offset, "color": "#ffffff"}) }
        })
        .collect();
    for (what, edit) in [
        ("Perceptual Ink → RGB green", with(ink_to_green, json!({"method": "perceptual"}))),
        ("dithered Classic Ink → paper", with(ink_to_white(100.0), json!({"dither": true, "method": "classic"}))),
        ("dithered Perceptual Ink → paper", with(ink_to_white(100.0), json!({"dither": true, "method": "perceptual"}))),
        ("Perceptual Ink and paper, 4,097 stops", json!({"stops": stops, "dither": false, "method": "perceptual"})),
    ] {
        run(&mut s, "paint.editGradient", edit);
        let (pdf, warnings) = pdf_of(&s);
        assert!(pdf.contains("/Subtype/Image"), "{what}: an image");
        assert!(spot_warning(&warnings), "{what}: {warnings:?}");
    }
    // A Perceptual gradient without spot inks is an image, without the warning.
    let blue_yellow = json!({"stops": [{"offset": 0, "color": "#0000ff"}, {"offset": 1, "color": "#ffff00"}]});
    run(&mut s, "paint.editGradient", with(blue_yellow, json!({"method": "perceptual", "dither": false})));
    let (pdf, warnings) = pdf_of(&s);
    assert!(pdf.contains("/Subtype/Image") && !pdf.contains("/Separation"), "a process Perceptual gradient");
    assert!(!spot_warning(&warnings), "{warnings:?}");
}
