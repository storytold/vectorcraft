//! Document → PDF (krilla). Mirrors the tree walk of `vectorcraft-render`.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft_pdf::write::backend as krilla;
use craft_pdf::write::{self, Metadata};
use krilla::color::separation::{Color as SepColor, SeparationColorant, SeparationSpace};
use krilla::color::{cmyk, luma, rgb};
use krilla::configure::{Archival, PdfVersion};
use krilla::geom::{Path, PathBuilder, Size, Transform};
use krilla::image::Image;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, LinearGradient, RadialGradient, SpreadMethod, Stop, Stroke, StrokeDash};
use krilla::surface::Surface;
use krilla::text::{Glyph, GlyphId, KrillaGlyph};
use kurbo::{PathEl, Shape, Vec2};
use vectorcraft_color::{BlendMode, Color, GradientKind, Paint};
use vectorcraft_doc::{AppearanceItem, Document, LineCap, LineJoin, Node, NodeKind, StrokeAlign, StrokeLayer, SymbolBoxes, TextObject};
use vectorcraft_effects::stroke::{self, WrittenShape};
use vectorcraft_geom::{Affine, BezPath, FillRule, Rect};
use vectorcraft_text::embed::Embedding;
use vectorcraft_text::{ListMarker, TextLayout};

use crate::forms::Mark;
use crate::lab_spot::{find, rfind};
use crate::marks::PageBoxes;
use crate::output::ColorOut;
use crate::{Compatibility, ExportReport, PdfError, PdfOptions, Standard};

mod entry;
mod exporter;
mod images;
mod nodes;
mod paint;
mod text;
mod writer;

pub(crate) use entry::{RgbInCmyk, leaving_out_undecodable};
pub use entry::{export, export_with_report, page_areas};
#[cfg(test)]
use exporter::{MAX_COORD, writable, xf};
#[cfg(test)]
use images::{png_8_bit, png_16_bit};
pub(crate) use writer::{Sheet, Writer};

pub(crate) struct Exporter<'a> {
    doc: &'a Document,
    /// The art boxes of `doc`'s symbols, which the walk measures symbol instances by when it
    /// leaves out objects outside the page ([`SymbolBoxes::quick`]). The box of an instance whose
    /// art holds brushed strokes grows by as far as their brush art can paint
    /// ([`SymbolBoxes::with_brush_art`]).
    symbols: SymbolBoxes<'a>,
    warnings: Vec<String>,
    images: HashMap<String, Option<Image>>,
    /// The brush library, parsed when the first brushed stroke is written.
    brushes: Option<Vec<vectorcraft_brush::Brush>>,
    /// Whether the group being written is a knockout group (what its neutral children inherit).
    knockout: bool,
    /// Spot colours written with a Lab alternate ([`crate::lab_spot`]): colorant name, Lab values.
    lab_spots: Vec<(String, vectorcraft_color::cms::Lab)>,
    /// Images may ask viewers to smooth them (not in PDF/A).
    interpolate: bool,
    /// Images may have 16 bits a sample (PDF 1.5 and later); in earlier files they are written at
    /// 8 bits ([`embed`]).
    sixteen_bit: bool,
    /// How images are resampled and compressed.
    compression: &'a crate::CompressionSettings,
    /// Layers whose Print option is off are written too.
    pub non_printing: bool,
    /// How colours are separated into CMYK (the colour settings' intent unless print sets one).
    pub intent: vectorcraft_color::cms::Intent,
    /// Whether a layer shows transparency (found out on the first page that asks).
    transparent: Option<bool>,
    /// How colours are written (the Output settings; print writes them as they are).
    pub out: Arc<ColorOut>,
    /// Colours are converted to the destination (printer's marks aren't).
    convert: bool,
    /// RGB colours and images of a CMYK document drawn so far.
    rgb_in_cmyk: RgbInCmyk,
    /// Text as glyph outlines; else the characters' fills are real text in embedded fonts.
    outline_text: bool,
    /// The fonts real text is written in, by face id, with their units per em; `None` for faces
    /// that can't be embedded.
    fonts: HashMap<u32, Option<(krilla::text::Font, f64)>>,
    /// Layers and sublayers are drawn as forms marked for their optional content groups
    /// ([`crate::forms`]).
    pub layers: bool,
    /// Each layer's index in [`crate::forms::pdf_layers`], by address.
    layer_index: HashMap<usize, usize>,
    /// The address of the layer whose form is being drawn.
    in_layer: usize,
    /// Overprinting fills and strokes are drawn as forms marked to overprint.
    pub overprint: bool,
    /// An overprinting fill or stroke was drawn so.
    pub overprinted: bool,
    /// Characters with neither fill nor stroke, or whose fill is drawn as art (outlines filled
    /// with an image or pattern), were written as text in forms marked invisible.
    pub invisible: bool,
    /// Inline graphics being drawn inside inline graphics (text in a symbol shown inline in
    /// text…): deeper ones are left out.
    inline_depth: u32,
    /// Tiling patterns being drawn inside tiling patterns ([`Self::tiling`]).
    pattern_depth: u32,
    /// Keys of the images left out because the writer couldn't decode them
    /// ([`leaving_out_undecodable`]).
    pub left_out: HashSet<String>,
    /// Objects the symbol instances written held, together
    /// ([`vectorcraft_doc::MAX_SYMBOL_NODES`]).
    symbol_nodes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writable_bounds_values_the_transform_and_values_through_it() {
        let mut pdf = krilla::Document::new();
        let mut page = pdf.start_page_with(PageSettings::new(Size::from_wh(100.0, 100.0).unwrap()));
        let mut s = page.surface();
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(writable(&s, Affine::IDENTITY, r));
        assert!(!writable(&s, Affine::IDENTITY, Rect::new(-2_147_483_649.0, 0.0, 10.0, 10.0)));
        assert!(!writable(&s, Affine::IDENTITY, Rect::new(f64::NAN, 0.0, 10.0, 10.0)));
        assert!(!writable(&s, Affine::translate((0.0, 2e9)), r));
        assert!(!writable(&s, Affine::scale(2e9), Rect::ZERO));
        // `writable` also checks the surface's own transform: with the surface moved to
        // x = MAX_COORD, a point at the origin fits and a 10 by 10 box does not.
        s.push_transform(&xf(Affine::translate((MAX_COORD, 0.0))));
        assert!(writable(&s, Affine::IDENTITY, Rect::ZERO));
        assert!(!writable(&s, Affine::IDENTITY, r));
        s.pop();
        s.finish();
        page.finish();
    }

    #[test]
    fn a_16_bit_png_at_8_bits_keeps_its_color_type_and_icc_profile() {
        use image::{ImageDecoder, ImageEncoder};
        let icc = vectorcraft_color::cms::icc_bytes(vectorcraft_color::cms::GRAY).unwrap().to_vec();
        let gray = image::DynamicImage::ImageLuma16(image::ImageBuffer::from_fn(4, 2, |x, y| image::Luma([x as u16 * 20000 + y as u16])));
        let mut png = vec![];
        let mut encoder = image::codecs::png::PngEncoder::new(&mut png);
        encoder.set_icc_profile(icc.clone()).unwrap();
        gray.write_with_encoder(encoder).unwrap();
        assert!(png_16_bit(&png));
        let out = png_8_bit(&png).unwrap();
        assert!(!png_16_bit(&out));
        let mut decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(out.as_slice())).unwrap();
        assert_eq!(decoder.icc_profile().unwrap(), Some(icc));
        assert_eq!(decoder.color_type(), image::ColorType::L8);
        assert_eq!(image::DynamicImage::from_decoder(decoder).unwrap().to_luma8(), gray.to_luma8());
    }
}
