//! Conic gradients as function-based shadings (ISO 32000-1, 8.7.4.5.2) whose calculator function
//! turns the angle about the center into the ramp: written exactly, read back as conic gradients,
//! and drawn as images where a sweep shading can't be written.

use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_testkit::raster::{assert_similar, render_artboard};

use crate::*;

/// A page-sized square filled with `gradient` placed on `geom`.
fn doc(gradient: Gradient, geom: GradientGeom) -> Document {
    let mut gp = GradientPaint::new(gradient);
    gp.geom = Some(geom);
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let ap = Appearance::basic(Paint::Gradient(Box::new(gp)), Paint::None, 0.0);
    d.insert(Some(l), 0, Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)), ap)).unwrap();
    d
}

/// Red to green to blue, once around.
fn rgb_conic() -> Gradient {
    let stops = vec![
        GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)),
        GradientStop::new(0.5, Color::rgb(0.0, 1.0, 0.0)),
        GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0)),
    ];
    Gradient::new(GradientKind::Conic, stops)
}

/// The first gradient fill in `d`.
fn gradient(d: &Document) -> GradientPaint {
    let mut out = None;
    d.walk(|n| {
        if let (None, Paint::Gradient(g)) = (&out, n.appearance.fill_paint()) {
            out = Some(*g.clone());
        }
    });
    out.expect("a gradient fill")
}

/// Held while a test imports function-based shadings, so tests that count evaluations count only
/// their own ([`super::import_fidelity::SHADINGS`]).
fn shadings() -> std::sync::MutexGuard<'static, ()> {
    super::import_fidelity::SHADINGS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn uncompressed(d: &Document, o: &PdfOptions) -> (String, Vec<String>) {
    let report = export_with_report(d, o).unwrap();
    (String::from_utf8_lossy(&report.bytes).into_owned(), report.warnings)
}

#[test]
fn a_conic_is_written_as_a_sweep_shading() {
    let geom = GradientGeom { start: Point::new(100.0, 100.0), end: Point::new(180.0, 60.0), aspect: 0.6, focal: None };
    let (text, warnings) = uncompressed(&doc(rgb_conic(), geom), &PdfOptions::uncompressed());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(text.contains("/ShadingType 1"), "a function-based shading");
    assert!(text.contains("/FunctionType 4") && text.contains("atan"), "its function turns the angle into the ramp");
    assert!(!text.contains("/ShadingType 2") && !text.contains("/ShadingType 3"), "no axial or radial stand-in");
    // One without an extent still has its sweep.
    let c = Point::new(100.0, 100.0);
    let (text, _) = uncompressed(&doc(rgb_conic(), GradientGeom { start: c, end: c, aspect: 1.0, focal: None }), &PdfOptions::uncompressed());
    assert!(text.contains("/ShadingType 1"));
}

/// Written and read back, a conic is a conic on the same frame with the same colors around it,
/// squashed, turned or mirrored.
#[test]
fn a_conic_comes_back_as_a_conic() {
    let _held = shadings();
    let c = Point::new(100.0, 100.0);
    for geom in [
        GradientGeom { start: c, end: Point::new(170.0, 100.0), aspect: 1.0, focal: None },
        GradientGeom { start: Point::new(90.0, 110.0), end: Point::new(150.0, 60.0), aspect: 0.5, focal: None },
        GradientGeom { start: c, end: Point::new(100.0, 40.0), aspect: -1.0, focal: None },
    ] {
        let d = doc(rgb_conic(), geom);
        let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
        let g = gradient(&back);
        assert_eq!(g.gradient.kind, GradientKind::Conic, "{geom:?}");
        let got = g.geom.unwrap();
        assert!(got.start.distance(geom.start) < 0.05 && got.is_mirrored() == geom.is_mirrored(), "{geom:?} → {got:?}");
        for p in [Point::new(30.0, 100.0), Point::new(100.0, 170.0), Point::new(140.0, 60.0), Point::new(60.0, 40.0), Point::new(185.0, 130.0)] {
            let want = rgb_conic().sample(geom.param_at(GradientKind::Conic, p) as f32).0.to_rgb();
            let have = g.gradient.sample(got.param_at(GradientKind::Conic, p) as f32).0.to_rgb();
            assert!(want.iter().zip(have).all(|(a, b)| (a - b).abs() < 0.03), "{geom:?} at {p:?}: {want:?} vs {have:?}");
        }
        assert_similar(&render_artboard(&back), &render_artboard(&d), 4.0, 0.01);
    }
}

/// A conic whose center lies off the painted square, so the shading's domain does not hold its
/// origin, comes back as a conic with the colors the square shows, the seam among them or not.
#[test]
fn a_conic_centered_off_the_object_comes_back_as_a_conic() {
    let _held = shadings();
    for geom in [
        // The seam points into the square.
        GradientGeom { start: Point::new(-50.0, 100.0), end: Point::new(30.0, 100.0), aspect: 1.0, focal: None },
        // The seam points away from it.
        GradientGeom { start: Point::new(-50.0, 100.0), end: Point::new(-50.0, 20.0), aspect: 1.0, focal: None },
        // Above it, mirrored and squashed.
        GradientGeom { start: Point::new(120.0, -60.0), end: Point::new(170.0, -60.0), aspect: -0.5, focal: None },
    ] {
        let d = doc(rgb_conic(), geom);
        let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
        let g = gradient(&back);
        assert_eq!(g.gradient.kind, GradientKind::Conic, "{geom:?}");
        let got = g.geom.unwrap();
        assert!(got.start.distance(geom.start) < 0.05 && got.is_mirrored() == geom.is_mirrored(), "{geom:?} → {got:?}");
        // Off the seam: on it, either side's color is right.
        for p in [Point::new(5.0, 112.0), Point::new(100.0, 195.0), Point::new(195.0, 5.0), Point::new(60.0, 40.0), Point::new(185.0, 130.0)] {
            let want = rgb_conic().sample(geom.param_at(GradientKind::Conic, p) as f32).0.to_rgb();
            let have = g.gradient.sample(got.param_at(GradientKind::Conic, p) as f32).0.to_rgb();
            assert!(want.iter().zip(have).all(|(a, b)| (a - b).abs() < 0.03), "{geom:?} at {p:?}: {want:?} vs {have:?}");
        }
        assert_similar(&render_artboard(&back), &render_artboard(&d), 4.0, 0.01);
    }
}

/// A conic with translucent stops comes back as a conic with their opacity.
#[test]
fn a_translucent_conic_comes_back_with_its_opacity() {
    let _held = shadings();
    let mut ramp = rgb_conic();
    ramp.stops[1].opacity = 0.4;
    let geom = GradientGeom { start: Point::new(100.0, 100.0), end: Point::new(170.0, 100.0), aspect: 1.0, focal: None };
    let d = doc(ramp.clone(), geom);
    let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
    let g = gradient(&back);
    assert_eq!(g.gradient.kind, GradientKind::Conic);
    let got = g.geom.unwrap();
    for p in [Point::new(30.0, 100.0), Point::new(100.0, 170.0), Point::new(140.0, 60.0), Point::new(185.0, 130.0)] {
        let (wc, wo) = ramp.sample(geom.param_at(GradientKind::Conic, p) as f32);
        let (hc, ho) = g.gradient.sample(got.param_at(GradientKind::Conic, p) as f32);
        assert!((wo - ho).abs() < 0.03, "at {p:?}: opacity {wo} vs {ho}");
        assert!(wc.to_rgb().iter().zip(hc.to_rgb()).all(|(a, b)| (a - b).abs() < 0.03), "at {p:?}");
    }
    assert_similar(&render_artboard(&back), &render_artboard(&d), 4.0, 0.01);
}

/// A sweep shading holds up to 97 stops: a conic with that many is a sweep, and one more makes
/// it an image.
#[test]
fn a_conic_of_97_stops_is_a_sweep_and_98_an_image() {
    let geom = GradientGeom { start: Point::new(100.0, 100.0), end: Point::new(180.0, 100.0), aspect: 1.0, focal: None };
    let stops = |n: usize| -> Gradient {
        let v = (0..n).map(|i| GradientStop::new(i as f32 / (n - 1) as f32, Color::rgb((i % 2) as f32, 0.0, 1.0 - (i % 2) as f32))).collect();
        Gradient::new(GradientKind::Conic, v)
    };
    for (n, sweep) in [(97, true), (98, false)] {
        let g = stops(n);
        assert_eq!(g.expanded().count(), n, "every stop is its own");
        let (text, warnings) = uncompressed(&doc(g, geom), &PdfOptions::uncompressed());
        assert_eq!(text.contains("/FunctionType 4"), sweep, "{n} stops");
        assert_eq!(text.contains("/ShadingType 1"), sweep, "{n} stops");
        assert_eq!(warnings.iter().any(|w| w.contains("conic gradients") && w.contains("images")), !sweep, "{n} stops: {warnings:?}");
    }
}

/// PDF/A forbids calculator functions, and a sweep shading holds at most 97 stops: such a conic
/// is drawn as an image of its colors, with a warning.
#[test]
fn a_conic_a_sweep_cannot_hold_is_an_image() {
    let _held = shadings();
    let geom = GradientGeom { start: Point::new(100.0, 100.0), end: Point::new(180.0, 100.0), aspect: 1.0, focal: None };
    let mut o = PdfOptions::uncompressed();
    (o.settings.standard, o.settings.compatibility) = (Standard::PdfA2b, Standard::PdfA2b.version());
    let many: Vec<GradientStop> =
        (0..=120).map(|i| GradientStop::new(i as f32 / 120.0, Color::rgb(i as f32 / 120.0, 0.0, 1.0 - i as f32 / 120.0))).collect();
    for (case, d, o) in
        [("PDF/A", doc(rgb_conic(), geom), o), ("121 stops", doc(Gradient::new(GradientKind::Conic, many), geom), PdfOptions::uncompressed())]
    {
        let (text, warnings) = uncompressed(&d, &o);
        assert!(!text.contains("/FunctionType 4") && !text.contains("atan"), "{case}: no calculator function");
        assert!(text.contains("/Image") && !text.contains("/ShadingType"), "{case}: an image of the sweep");
        assert!(warnings.iter().any(|w| w.contains("conic gradients") && w.contains("images")), "{case}: {warnings:?}");
        // Read back, the image shows the sweep where the canvas draws it.
        let back = import(&export(&d, &o).unwrap()).unwrap();
        let (want, got) = (render_artboard(&d), render_artboard(&back));
        for (x, y) in [(150, 120), (100, 160), (40, 100), (100, 40), (160, 80), (30, 30), (170, 170)] {
            let (a, b) = (want.over_white(x, y), got.over_white(x, y));
            assert!(a.iter().zip(b).all(|(p, q)| p.abs_diff(q) <= 12), "{case} at ({x}, {y}): {a:?} vs {b:?}");
        }
    }
}
