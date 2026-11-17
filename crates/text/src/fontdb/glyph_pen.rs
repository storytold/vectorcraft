use super::face::FontFace;
use super::{BezPath, OutlinePen};

/// Is glyph `gid` of `face` (as `font`) drawn from an SVG document, if it has one: the face has
/// SVG glyphs, and the glyph no COLR layers, which come first.
pub(super) fn svg_drawn(face: &FontFace, font: &skrifa::FontRef<'_>, gid: u32) -> bool {
    face.svg_glyphs && !crate::colr::has_colour_glyph(font, gid)
}

/// The most path elements one glyph outline keeps. Real outlines have hundreds; a variable
/// composite glyph can repeat a big base outline into millions. Elements past it are dropped.
pub(crate) const MAX_OUTLINE_ELEMENTS: usize = 1 << 20;

/// A pen drawing outlines y down, as text space has them.
pub(crate) struct FlipPen(pub(crate) BezPath);

impl FlipPen {
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
    fn a_pen_keeps_at_most_the_element_cap() {
        let mut pen = FlipPen(BezPath::new());
        pen.move_to(0.0, 0.0);
        for i in 0..MAX_OUTLINE_ELEMENTS + 10 {
            pen.line_to(i as f32, 0.0);
        }
        assert_eq!(pen.0.elements().len(), MAX_OUTLINE_ELEMENTS);
    }
}
