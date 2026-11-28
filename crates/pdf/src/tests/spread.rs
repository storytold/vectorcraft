//! Gradients that repeat or reflect. PDF's axial and radial shadings pad past their ends, so a
//! linear, radial or diamond gradient is written with its stops repeated as many times as the
//! shape needs (axial and radial shadings, which PDF/A allows); a conic's sweep shading runs over
//! one run and repeats or reflects it. Read back, each draws as the canvas draws the gradient.

use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientSpread, GradientStop, Paint};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_geom::{Point, Rect, Vec2, shapes};
use vectorcraft_testkit::raster::render_artboard;

use crate::*;

/// A page-sized square filled (or stroked 16 pt wide, `stroke`) with a red-to-blue gradient of
/// `kind` placed on `geom`, spreading by `spread` 3 times.
fn doc(kind: GradientKind, spread: GradientSpread, geom: GradientGeom, stroke: bool) -> Document {
    doc_n(kind, spread, geom, stroke, 3)
}

/// [`doc`], spreading `repeats` times.
fn doc_n(kind: GradientKind, spread: GradientSpread, geom: GradientGeom, stroke: bool, repeats: u32) -> Document {
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let mut gp = GradientPaint::new(Gradient { spread, repeats, ..Gradient::new(kind, stops) });
    gp.geom = Some(geom);
    let paint = Paint::Gradient(Box::new(gp));
    let ap = if stroke { Appearance::basic(Paint::None, paint, 16.0) } else { Appearance::basic(paint, Paint::None, 0.0) };
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    d.insert(Some(l), 0, Node::path(id, shapes::rectangle(Rect::new(20.0, 20.0, 180.0, 180.0)), ap)).unwrap();
    d
}

fn cases() -> Vec<(&'static str, GradientKind, GradientGeom)> {
    let c = Point::new(100.0, 100.0);
    let geom = GradientGeom { start: c, end: c + Vec2::new(60.0, -25.0), aspect: 0.7, focal: None };
    vec![
        ("linear", GradientKind::Linear, GradientGeom { start: Point::new(70.0, 80.0), end: Point::new(150.0, 110.0), aspect: 1.0, focal: None }),
        ("radial", GradientKind::Radial, geom),
        ("off-center radial", GradientKind::Radial, GradientGeom { focal: Some(c + Vec2::new(-15.0, 12.0)), ..geom }),
        ("conic", GradientKind::Conic, geom),
        ("diamond", GradientKind::Diamond, geom),
    ]
}

/// Does the PDF text hold an image XObject?
fn image(text: &str) -> bool {
    text.contains("/Subtype/Image") || text.contains("/Subtype /Image")
}

fn uncompressed(d: &Document, o: &PdfOptions) -> String {
    String::from_utf8_lossy(&export_with_report(d, o).unwrap().bytes).into_owned()
}

/// Every kind, filled and stroked, repeating and reflecting, written and read back, looks as the
/// canvas paints it, inside the vector and past its ends.
#[test]
fn spread_gradients_come_back_as_the_canvas_draws_them() {
    let _held = super::import_fidelity::SHADINGS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for (what, kind, geom) in cases() {
        for spread in [GradientSpread::Repeat, GradientSpread::Reflect] {
            for stroke in [false, true] {
                let d = doc(kind, spread, geom, stroke);
                let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
                // Hard edges (where Repeat starts the stops again) differ by a pixel's
                // antialiasing at most.
                let (a, b) = (render_artboard(&back), render_artboard(&d));
                let st = vectorcraft_testkit::raster::perceptual_diff(&a, &b, 10.0);
                assert!(
                    (st.bad_pixels as f64) < 0.03 * st.pixels as f64,
                    "{what} {spread:?} stroke {stroke}: {} of {} pixels differ",
                    st.bad_pixels,
                    st.pixels
                );
            }
        }
    }
}

/// A linear, radial or diamond gradient is written as axial or radial shadings (in PDF/A too); a
/// conic as its sweep shading.
#[test]
fn the_shadings_written() {
    let mut pdfa = PdfOptions::uncompressed();
    (pdfa.settings.standard, pdfa.settings.compatibility) = (Standard::PdfA2b, Standard::PdfA2b.version());
    for (what, kind, geom) in cases() {
        for o in [PdfOptions::uncompressed(), pdfa.clone()] {
            let text = uncompressed(&doc(kind, GradientSpread::Reflect, geom, false), &o);
            let want = match kind {
                GradientKind::Linear | GradientKind::Diamond => "/ShadingType 2",
                GradientKind::Radial => "/ShadingType 3",
                _ => "/ShadingType 1",
            };
            if kind == GradientKind::Conic && o.settings.standard != Standard::None {
                // PDF/A has no calculator functions: the sweep is an image.
                continue;
            }
            assert!(text.contains(want), "{what}: {want}");
            assert!(!image(&text), "{what}: no image");
        }
    }
}

/// An axial shading with a sharp step between two colors reads back with the step where it was,
/// not blurred across the samples around it.
#[test]
fn a_sharp_step_in_a_shading_reads_back_sharp() {
    let red = Color::rgb(1.0, 0.0, 0.0);
    let blue = Color::rgb(0.0, 0.0, 1.0);
    let stops = vec![GradientStop::new(0.0, red), GradientStop::new(0.37, red), GradientStop::new(0.37, blue), GradientStop::new(1.0, blue)];
    let mut gp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    gp.geom = Some(GradientGeom { start: Point::new(0.0, 100.0), end: Point::new(200.0, 100.0), aspect: 1.0, focal: None });
    let mut d = Document::new(200.0, 200.0);
    let (l, id) = (d.layers[0].id, d.alloc_id());
    let ap = Appearance::basic(Paint::Gradient(Box::new(gp)), Paint::None, 0.0);
    d.insert(Some(l), 0, Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)), ap)).unwrap();
    let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
    let mut found = None;
    back.walk(|n| {
        if let (None, Paint::Gradient(g)) = (&found, n.appearance.fill_paint()) {
            found = Some(*g.clone());
        }
    });
    let g = found.expect("a gradient");
    let geom = g.geom.unwrap();
    for (x, want) in [(73.0, red), (72.0, red), (75.0, blue), (76.0, blue)] {
        let t = geom.param_at(GradientKind::Linear, Point::new(x, 100.0));
        let got = g.gradient.sample(t as f32).0.to_rgb();
        assert!(got.iter().zip(want.to_rgb()).all(|(a, b)| (a - b).abs() < 0.02), "x {x}: {got:?}");
    }
}

/// A gradient written with 256 runs of its stops (as many as PDF export writes), so that every
/// sample the reader takes across the shading lands at the start of a run, reads back with its
/// runs rather than as one color.
#[test]
fn many_runs_read_back_as_runs() {
    let _held = super::import_fidelity::SHADINGS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // A run of 0.4 pt from the shape's left edge, across 160 pt: capped at 256 runs, 102.4 pt.
    let geom = GradientGeom { start: Point::new(20.0, 100.0), end: Point::new(60.0, 100.0), aspect: 1.0, focal: None };
    let d = doc_n(GradientKind::Linear, GradientSpread::Repeat, geom, false, 100);
    let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
    let mut found = None;
    back.walk(|n| {
        if let (None, Paint::Gradient(g)) = (&found, n.appearance.fill_paint()) {
            found = Some(*g.clone());
        }
    });
    let g = found.expect("a gradient");
    let geom = g.geom.unwrap();
    for run in [0.0, 1.0, 50.0, 200.0, 255.0] {
        for (u, want) in [(0.25, [0.75, 0.0, 0.25]), (0.75, [0.25, 0.0, 0.75])] {
            let x = 20.0 + 0.4 * (run + u);
            let t = geom.param_at(GradientKind::Linear, Point::new(x, 100.0));
            let got = g.gradient.sample(t as f32).0.to_rgb();
            assert!(got.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.05), "run {run} at {u}: {got:?}");
        }
    }
}

/// A diamond centered in the shape is written in PDF as four linear gradients, each with the runs
/// from the center out to the shape's corner, not padded short of it.
#[test]
fn a_diamond_writes_each_quadrant_from_its_center_out() {
    // From the center to a corner: 16 lengths of the vector, 10 runs each.
    let c = Point::new(100.0, 100.0);
    let geom = GradientGeom { start: c, end: c + Vec2::new(10.0, 0.0), aspect: 1.0, focal: None };
    let d = doc_n(GradientKind::Diamond, GradientSpread::Repeat, geom, false, 10);
    let report = export_with_report(&d, &PdfOptions::uncompressed()).unwrap();
    assert!(!report.warnings.iter().any(|w| w.contains("at most 256 runs")), "{:?}", report.warnings);
}

/// A repeating gradient whose run reaches past the coordinates PDF holds is drawn as an image
/// rather than left out, for a linear, a radial and a diamond gradient.
#[test]
fn a_run_too_long_for_pdf_is_an_image() {
    let c = Point::new(100.0, 100.0);
    for (what, kind) in [("linear", GradientKind::Linear), ("radial", GradientKind::Radial), ("diamond", GradientKind::Diamond)] {
        // Three runs of the vector, each longer than PDF's coordinates reach.
        let geom = GradientGeom { start: c, end: c + Vec2::new(1.2e10, 0.0), aspect: 1.0, focal: None };
        let text = uncompressed(&doc(kind, GradientSpread::Repeat, geom, false), &PdfOptions::uncompressed());
        assert!(image(&text), "{what}: drawn as an image");
    }
}

/// A Perceptual gradient of a spot ink stays Separation shadings in that ink while its repeated
/// stops' ink curve fits, and is an image (rather than tints mixed the Classic way) once it no
/// longer does, for a linear and a diamond gradient.
#[test]
fn a_perceptual_spot_gradient_too_long_once_repeated_is_an_image() {
    let gold = Color::cmyk(0.0, 0.2, 0.8, 0.1);
    // Gold and paper white in turn: a curve of many tints in each run.
    let stops: Vec<GradientStop> = (0..=4)
        .map(|i| match i % 2 {
            0 => {
                let mut s = GradientStop::new(i as f32 / 4.0, gold);
                (s.swatch, s.tint) = (Some("Gold".into()), 1.0);
                s
            }
            _ => GradientStop::new(i as f32 / 4.0, Color::cmyk(0.0, 0.0, 0.0, 0.0)),
        })
        .collect();
    let c = Point::new(100.0, 100.0);
    for (kind, short, long) in [
        (
            GradientKind::Linear,
            GradientGeom { start: Point::new(20.0, 100.0), end: Point::new(20.4, 100.0), aspect: 1.0, focal: None },
            GradientGeom { start: Point::new(20.0, 100.0), end: Point::new(180.0, 100.0), aspect: 1.0, focal: None },
        ),
        (
            GradientKind::Diamond,
            GradientGeom { start: c, end: c + Vec2::new(0.3, 0.0), aspect: 1.0, focal: None },
            GradientGeom { start: c, end: c + Vec2::new(80.0, 0.0), aspect: 1.0, focal: None },
        ),
    ] {
        for (geom, repeats, image_wanted) in [(long, 2, false), (short, 1, true)] {
            let g = Gradient {
                method: vectorcraft_color::GradientMethod::Perceptual,
                spread: GradientSpread::Repeat,
                repeats,
                ..Gradient::new(kind, stops.clone())
            };
            let padded = g.padded_over(&geom, Rect::new(20.0, 20.0, 180.0, 180.0)).unwrap();
            assert!(g.fits_ink_tints() && padded.gradient.fits_ink_tints() != image_wanted, "{kind:?} {repeats}");
            let mut gp = GradientPaint::new(g);
            gp.geom = Some(geom);
            let mut d = Document::new(200.0, 200.0);
            d.color_mode = vectorcraft_doc::ColorMode::Cmyk;
            d.swatches.push(vectorcraft_color::Swatch { name: "Gold".into(), paint: Paint::solid(gold), global: true, spot: true });
            let (l, id) = (d.layers[0].id, d.alloc_id());
            let ap = Appearance::basic(Paint::Gradient(Box::new(gp)), Paint::None, 0.0);
            d.insert(Some(l), 0, Node::path(id, shapes::rectangle(Rect::new(20.0, 20.0, 180.0, 180.0)), ap)).unwrap();
            let text = uncompressed(&d, &PdfOptions::uncompressed());
            assert_eq!(image(&text), image_wanted, "{kind:?} {repeats}");
            assert_eq!(text.contains("[/Separation/Gold"), !image_wanted, "{kind:?} {repeats}");
        }
    }
}

/// A gradient that repeats with no stops (a file can hold one) exports, filled and stroked.
#[test]
fn a_spread_gradient_without_stops_exports() {
    for kind in [GradientKind::Linear, GradientKind::Radial, GradientKind::Conic, GradientKind::Diamond] {
        let g = GradientPaint::new(Gradient { spread: GradientSpread::Repeat, repeats: 3, ..Gradient::new(kind, vec![]) });
        let paint = Paint::Gradient(Box::new(g));
        let mut d = Document::new(200.0, 200.0);
        let (l, id) = (d.layers[0].id, d.alloc_id());
        let ap = Appearance::basic(paint.clone(), paint, 4.0);
        d.insert(Some(l), 0, Node::path(id, shapes::rectangle(Rect::new(20.0, 20.0, 180.0, 180.0)), ap)).unwrap();
        export(&d, &PdfOptions::default()).unwrap();
    }
}

/// A run far too short for the shape is written with at most 256 runs, padding past them, with a
/// warning.
#[test]
fn far_repeats_are_capped_with_a_warning() {
    let geom = GradientGeom { start: Point::new(20.0, 20.0), end: Point::new(20.5, 20.0), aspect: 1.0, focal: None };
    let report = export_with_report(&doc(GradientKind::Linear, GradientSpread::Repeat, geom, false), &PdfOptions::uncompressed()).unwrap();
    assert!(report.warnings.iter().any(|w| w.contains("at most 256 runs")), "{:?}", report.warnings);
}
