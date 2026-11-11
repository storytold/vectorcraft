use super::face::FontFace;
use super::{BezPath, OutlinePen};

/// Is glyph `gid` of `face` (as `font`) drawn from an SVG document, if it has one: the face has
/// SVG glyphs, and the glyph no COLR layers, which come first.
pub(super) fn svg_drawn(face: &FontFace, font: &skrifa::FontRef<'_>, gid: u32) -> bool {
    face.svg_glyphs && !crate::colr::has_colour_glyph(font, gid)
}

/// Most path elements one glyph outline may have: a glyph that needs more (a variable font's
/// composite glyph repeating a large component many times) is drawn as nothing, not kept.
pub(crate) const MAX_OUTLINE_ELEMENTS: usize = 1 << 20;

/// A pen drawing outlines y down, as text space has them. It stops taking segments once the path
/// holds [`MAX_OUTLINE_ELEMENTS`], and [`Self::finish`] then gives an empty path.
pub(crate) struct FlipPen(pub(crate) BezPath);

impl FlipPen {
    /// The outline drawn, or an empty path when it passed [`MAX_OUTLINE_ELEMENTS`].
    pub(crate) fn finish(self) -> BezPath {
        if self.0.elements().len() >= MAX_OUTLINE_ELEMENTS { BezPath::new() } else { self.0 }
    }

    fn full(&self) -> bool {
        self.0.elements().len() >= MAX_OUTLINE_ELEMENTS
    }
}

impl OutlinePen for FlipPen {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.full() {
            return;
        }
        self.0.move_to((x as f64, -y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        if self.full() {
            return;
        }
        self.0.line_to((x as f64, -y as f64));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        if self.full() {
            return;
        }
        self.0.quad_to((cx0 as f64, -cy0 as f64), (x as f64, -y as f64));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        if self.full() {
            return;
        }
        self.0.curve_to((cx0 as f64, -cy0 as f64), (cx1 as f64, -cy1 as f64), (x as f64, -y as f64));
    }
    fn close(&mut self) {
        if self.full() {
            return;
        }
        self.0.close_path();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pen_past_the_cap_gives_nothing() {
        let mut pen = FlipPen(BezPath::new());
        pen.move_to(0.0, 0.0);
        for i in 0..MAX_OUTLINE_ELEMENTS + 10 {
            pen.line_to(i as f32, 1.0);
        }
        assert_eq!(pen.0.elements().len(), MAX_OUTLINE_ELEMENTS, "it stops taking segments");
        assert!(pen.finish().elements().is_empty());
        let mut small = FlipPen(BezPath::new());
        small.move_to(0.0, 0.0);
        small.line_to(1.0, 1.0);
        assert_eq!(small.finish().elements().len(), 2);
    }
}
