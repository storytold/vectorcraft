use super::face::FontFace;
use super::*;

/// Is glyph `gid` of `face` (as `font`) drawn from an SVG document, if it has one: the face has
/// SVG glyphs, and the glyph no COLR layers, which come first.
pub(super) fn svg_drawn(face: &FontFace, font: &skrifa::FontRef<'_>, gid: u32) -> bool {
    face.svg_glyphs && !crate::colr::has_colour_glyph(font, gid)
}

/// A pen drawing outlines y down, as text space has them.
pub(crate) struct FlipPen(pub(crate) BezPath);

impl OutlinePen for FlipPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((x as f64, -y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((x as f64, -y as f64));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to((cx0 as f64, -cy0 as f64), (x as f64, -y as f64));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to((cx0 as f64, -cy0 as f64), (cx1 as f64, -cy1 as f64), (x as f64, -y as f64));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}
