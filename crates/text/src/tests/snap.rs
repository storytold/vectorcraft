//! Pixel snapping for type-optimized anti-aliasing.

use kurbo::{Affine, Shape};
use vectorcraft_doc::CharStyle;

use crate::*;

fn text_at(x: f64, y: f64) -> TextObject {
    TextObject::point(Point::new(x, y), "Hill", CharStyle { size: 9.0, ..CharStyle::default() })
}

#[test]
fn glyph_starts_land_on_whole_device_pixels() {
    let t = text_at(10.3, 20.6);
    let mut l = layout(FontDb::global(), &t);
    let before: Vec<Point> = l.glyphs.iter().map(|g| g.origin).collect();
    let to_device = Affine::scale(2.0) * t.xf;
    assert!(l.snap_to_pixels(to_device));
    for (g, was) in l.glyphs.iter().zip(before) {
        let p = to_device * g.origin;
        assert!((p.x - p.x.round()).abs() < 1e-9 && (p.y - p.y.round()).abs() < 1e-9, "{p:?}");
        assert!((g.origin - was).hypot() <= 0.5f64.hypot(0.5) / 2.0 + 1e-9, "moved at most half a pixel");
        // The outline moved with its pen position.
        assert!(!g.outline.elements().is_empty());
    }
    let ink = |l: &TextLayout| l.glyphs[0].outline.bounding_box();
    let fresh = layout(FontDb::global(), &t);
    let d = ink(&l).origin() - ink(&fresh).origin();
    assert!((d - (l.glyphs[0].origin - fresh.glyphs[0].origin)).hypot() < 1e-9);
}

#[test]
fn rotated_text_is_left_alone() {
    let t = text_at(10.3, 20.6);
    let mut l = layout(FontDb::global(), &t);
    let before: Vec<Point> = l.glyphs.iter().map(|g| g.origin).collect();
    assert!(!l.snap_to_pixels(Affine::rotate(0.3) * t.xf));
    assert_eq!(l.glyphs.iter().map(|g| g.origin).collect::<Vec<_>>(), before);
}
