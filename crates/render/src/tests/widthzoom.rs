//! A stroke with a width profile keeps its shape when the view zooms out (#995): only a stroke too
//! thin to see is thickened, and then by what its widest point needs, not all of it at once.

use vectorcraft_color::Paint;
use vectorcraft_doc::{Appearance, AppearanceItem, Node, StrokeLayer, WidthProfile};
use vectorcraft_geom::PathData;

use crate::*;

const SIZE: u32 = 200;

/// A line across a 200×200 page, stroked black `weight` pt with `profile`, drawn at `scale`.
fn render(weight: f64, profile: Option<WidthProfile>, scale: f64) -> Rendered {
    let mut st = StrokeLayer::new(Paint::solid(vectorcraft_color::Color::BLACK), weight);
    st.profile = profile;
    let mut p = BezPath::new();
    p.move_to((20.0, 100.0));
    p.line_to((180.0, 100.0));
    let mut d = Document::new(SIZE as f64, SIZE as f64);
    let n = Node::path(d.alloc_id(), PathData::from_bezpath(&p), Appearance { items: vec![AppearanceItem::Stroke(st)], ..Default::default() });
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    Renderer::new().render(&d, SIZE, SIZE, Affine::scale(scale), &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

/// Device pixels of ink in the column through the middle of the line (coverage summed, so a
/// stroke narrower than a pixel still counts what it covers).
fn ink_at_middle(img: &Rendered, scale: f64) -> f64 {
    let x = (100.0 * scale) as u32;
    (0..SIZE).map(|y| 1.0 - img.pixel(x, y)[0] as f64 / 255.0).sum()
}

/// Thin at both ends, six times the weight in the middle.
fn swell() -> WidthProfile {
    WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 6.0, 6.0), (1.0, 1.0, 1.0)] }
}

#[test]
fn a_thin_width_stroke_keeps_its_thickness_in_document_terms_when_zoomed_out() {
    let whole = ink_at_middle(&render(1.0, Some(swell()), 1.0), 1.0);
    assert!((whole - 6.0).abs() < 0.3, "six points at full size: {whole}");
    for scale in [0.5, 0.25, 0.125] {
        let ink = ink_at_middle(&render(1.0, Some(swell()), scale), scale);
        let in_points = ink / scale;
        assert!((in_points - whole).abs() < whole * 0.05, "at {scale}: {in_points} pt against {whole} pt");
    }
}

#[test]
fn a_stroke_too_thin_to_see_is_kept_visible_by_its_widest_point() {
    // 0.1 pt, twice that in the middle: at a quarter size even the middle is a twentieth of a
    // pixel, so it is thickened to the half pixel a plain hairline gets, and the profile's ends
    // stay half of it.
    let thin = WidthProfile { points: vec![(0.0, 0.5, 0.5), (0.5, 1.0, 1.0), (1.0, 0.5, 0.5)] };
    let plain = ink_at_middle(&render(0.1, None, 0.25), 0.25);
    let img = render(0.1, Some(thin), 0.25);
    let shaped = ink_at_middle(&img, 0.25);
    assert!((plain - 0.5).abs() < 0.1, "a plain hairline gets half a pixel: {plain}");
    assert!((shaped - 0.5).abs() < 0.1, "the widest point of a profile gets it too: {shaped}");
    // Seven pixels in, 28 pt along the page: just past the line's start at 20 pt.
    let end: f64 = (0..SIZE).map(|y| 1.0 - img.pixel(7, y)[0] as f64 / 255.0).sum();
    assert!(end < shaped * 0.75, "the thin end stays thinner than the middle: {end} against {shaped}");
}
