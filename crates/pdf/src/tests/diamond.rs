//! Diamond gradients (PDF has no diamond shading): four axial shadings, one for each quadrant of
//! the diamond's frame and equal to it there, drawn as the canvas draws the diamond; an image
//! where the gradient is drawn as pixels or has no extent.

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

/// Red to green to blue from the center out.
fn rgb_diamond() -> Gradient {
    let stops = vec![
        GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)),
        GradientStop::new(0.5, Color::rgb(0.0, 1.0, 0.0)),
        GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0)),
    ];
    Gradient::new(GradientKind::Diamond, stops)
}

/// Turned and stretched across its vector, off the square's center.
fn placed() -> GradientGeom {
    GradientGeom { start: Point::new(90.0, 110.0), end: Point::new(150.0, 70.0), aspect: 0.6, focal: None }
}

fn uncompressed(d: &Document, o: &PdfOptions) -> (String, Vec<String>) {
    let report = export_with_report(d, o).unwrap();
    (String::from_utf8_lossy(&report.bytes).into_owned(), report.warnings)
}

/// Written as four axial shadings and read back, a diamond draws as the canvas draws it, with no
/// hairline between the quadrants: level and placed, and in PDF/A, which allows axial shadings.
#[test]
fn a_diamond_is_written_as_four_axial_shadings() {
    let level = GradientGeom::fit(GradientKind::Diamond, Rect::new(0.0, 0.0, 200.0, 200.0), 0.0);
    let mut pdfa = PdfOptions::uncompressed();
    (pdfa.settings.standard, pdfa.settings.compatibility) = (Standard::PdfA2b, Standard::PdfA2b.version());
    for (case, d, o) in [
        ("level", doc(rgb_diamond(), level), PdfOptions::uncompressed()),
        ("placed", doc(rgb_diamond(), placed()), PdfOptions::uncompressed()),
        ("PDF/A", doc(rgb_diamond(), placed()), pdfa),
    ] {
        let (text, warnings) = uncompressed(&d, &o);
        assert_eq!(text.matches("/ShadingType 2").count(), 4, "{case}");
        for other in ["/ShadingType 1", "/ShadingType 3", "/Subtype /Image"] {
            assert!(!text.contains(other), "{case}: {other}");
        }
        assert!(warnings.iter().any(|w| w.contains("diamond gradients are exported as four linear gradients")), "{case}: {warnings:?}");
        let back = import(&export(&d, &o).unwrap()).unwrap();
        assert_similar(&render_artboard(&back), &render_artboard(&d), 4.0, 0.01);
    }
}

/// A Perceptual diamond of one spot ink running out to paper white is not drawn as pixels: its
/// four quadrants are axial shadings in that ink's Separation space, as a Perceptual linear
/// gradient of spot inks is.
#[test]
fn a_perceptual_diamond_of_a_spot_ink_is_four_separation_shadings() {
    let gold = Color::cmyk(0.0, 0.2, 0.8, 0.1);
    let mut stops = vec![];
    for (offset, tint) in [(0.0, 1.0), (0.5, 0.4)] {
        let mut s = GradientStop::new(offset, gold.tinted(tint));
        (s.swatch, s.tint) = (Some("Gold".into()), tint);
        stops.push(s);
    }
    stops.push(GradientStop::new(1.0, Color::cmyk(0.0, 0.0, 0.0, 0.0)));
    let g = Gradient { method: vectorcraft_color::GradientMethod::Perceptual, ..Gradient::new(GradientKind::Diamond, stops) };
    let mut d = doc(g, placed());
    d.color_mode = vectorcraft_doc::ColorMode::Cmyk;
    d.swatches.push(vectorcraft_color::Swatch { name: "Gold".into(), paint: Paint::solid(gold), global: true, spot: true });
    let (text, warnings) = uncompressed(&d, &PdfOptions::uncompressed());
    assert_eq!(text.matches("/ShadingType 2").count(), 4, "{text}");
    assert!(text.contains("[/Separation/Gold"), "{text}");
    assert!(!text.contains("/Subtype /Image"), "{text}");
    assert!(warnings.iter().any(|w| w.contains("diamond gradients are exported as four linear gradients")), "{warnings:?}");
}

/// A dithered fill, a Perceptual gradient of process colors, one with translucent stops (whose
/// quadrants would add up their opacity where they overlap) and a diamond without an extent are
/// images of what the canvas draws, with a warning.
#[test]
fn a_diamond_drawn_as_pixels_or_without_an_extent_is_an_image() {
    let c = Point::new(100.0, 100.0);
    let dithered = Gradient { dither: true, ..rgb_diamond() };
    let perceptual = Gradient { method: vectorcraft_color::GradientMethod::Perceptual, ..rgb_diamond() };
    let mut translucent = rgb_diamond();
    translucent.stops[1].opacity = 0.4;
    for (case, d) in [
        ("dithered", doc(dithered, placed())),
        ("perceptual", doc(perceptual, placed())),
        ("translucent", doc(translucent, placed())),
        ("no vector", doc(rgb_diamond(), GradientGeom { start: c, end: c, aspect: 1.0, focal: None })),
        ("flat", doc(rgb_diamond(), GradientGeom { aspect: 0.0, ..placed() })),
    ] {
        let (text, warnings) = uncompressed(&d, &PdfOptions::uncompressed());
        assert!(text.contains("/Image") && !text.contains("/ShadingType"), "{case}");
        assert!(warnings.iter().any(|w| w.contains("diamond gradients drawn as pixels")), "{case}: {warnings:?}");
        let back = import(&export(&d, &PdfOptions::uncompressed()).unwrap()).unwrap();
        let (want, got) = (render_artboard(&d), render_artboard(&back));
        for (x, y) in [(150, 120), (100, 160), (40, 100), (100, 40), (90, 110), (30, 30), (170, 170)] {
            let (a, b) = (want.over_white(x, y), got.over_white(x, y));
            assert!(a.iter().zip(b).all(|(p, q)| p.abs_diff(q) <= 12), "{case} at ({x}, {y}): {a:?} vs {b:?}");
        }
    }
}
