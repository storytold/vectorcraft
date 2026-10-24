//! OpenType SVG glyphs: a font's `SVG ` table holds SVG documents whose element `glyph<gid>` draws
//! the glyph, in font units (y down, the origin at the glyph's). Turning SVG into art is the SVG
//! importer's job, a layer above this crate, so the app registers it as the reader
//! ([`set_svg_glyph_reader`]); without one, SVG glyphs keep their outlines.

use std::sync::{Arc, OnceLock};

use skrifa::GlyphId;
use skrifa::raw::TableProvider;
use vectorcraft_doc::Node;
use vectorcraft_doc::color::Color;

/// Turns glyph `gid`'s SVG document (possibly gzipped) of a font of `upem` units per em into its
/// art in font units (y down), in the text's colours `fill` and `stroke`, and says whether it used
/// them; `None` when it can't.
pub type SvgGlyphReader = fn(document: &[u8], gid: u32, upem: f64, fill: Color, stroke: Option<Color>) -> Option<(Node, bool)>;

static READER: OnceLock<SvgGlyphReader> = OnceLock::new();

/// Registers the reader of SVG glyphs. The first one registered stays.
pub fn set_svg_glyph_reader(reader: SvgGlyphReader) {
    let _ = READER.set(reader);
}

/// Does `font` have SVG glyphs?
pub(crate) fn has_svg_glyphs(font: &skrifa::FontRef<'_>) -> bool {
    font.svg().is_ok()
}

/// Glyph `gid`'s SVG document in `font`, if it has one.
fn document<'a>(font: &skrifa::FontRef<'a>, gid: u32) -> Option<&'a [u8]> {
    font.svg().ok().and_then(|t| t.glyph_data(GlyphId::new(gid)))
}

/// Does glyph `gid` of `font` have an SVG document?
pub(crate) fn has_document(font: &skrifa::FontRef<'_>, gid: u32) -> bool {
    document(font, gid).is_some()
}

/// Glyph `gid`'s art from its SVG document, read by the registered reader, and whether it uses
/// the text's colours. `None` without a document, a reader, or art.
pub(crate) fn svg_glyph(font: &skrifa::FontRef<'_>, gid: u32, upem: f64, fill: Color, stroke: Option<Color>) -> (Option<Arc<Node>>, bool) {
    let Some(document) = document(font, gid) else { return (None, false) };
    let Some(reader) = READER.get() else { return (None, false) };
    match reader(document, gid, upem, fill, stroke) {
        Some((art, coloured)) => (Some(Arc::new(art)), coloured),
        None => (None, false),
    }
}
