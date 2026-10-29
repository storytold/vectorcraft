use super::entry::RgbInCmyk;
use super::{
    Affine, Arc, BezPath, BlendMode, Color, Compatibility, Document, Exporter, Fill, FillRule, HashMap, HashSet, Node, NormalizedF32, Path,
    PathBuilder, PathEl, Rect, Shape, Standard, Surface, Transform, cmyk, krilla, luma, rgb,
};

/// How deep inline graphics nest before they are left out.
pub(super) const MAX_INLINE_DEPTH: u32 = 4;

/// How deep tiling patterns nest: deeper pattern fills are written as their tiles.
pub(super) const MAX_PATTERN_DEPTH: u32 = 4;

/// The warning for an image left out because it can't be decoded: the note raster exports give
/// (`vectorcraft-engine`'s `rasterfx::undecoded_warning`).
pub(super) const UNDECODED: &str = "an image couldn't be decoded and is missing from the picture";

impl<'a> Exporter<'a> {
    /// An exporter of `doc` writing images as `set` says; layers whose Print option is off are
    /// left out.
    pub(crate) fn new(doc: &'a Document, set: &'a crate::PdfSettings) -> Self {
        Self {
            doc,
            symbols: doc.symbol_boxes().quick().with_brush_art(),
            warnings: vec![],
            images: HashMap::new(),
            brushes: None,
            knockout: doc.page_knockout,
            lab_spots: vec![],
            interpolate: set.standard != Standard::PdfA2b,
            // PDF 1.3 and 1.4 files are written with the PDF 1.4 settings ([`Writer::new`]).
            sixteen_bit: !matches!(set.compatibility, Compatibility::Pdf13 | Compatibility::Pdf14),
            compression: &set.compression,
            non_printing: false,
            intent: vectorcraft_color::cms::active().settings().intent,
            transparent: None,
            out: Arc::default(),
            convert: true,
            rgb_in_cmyk: RgbInCmyk::default(),
            outline_text: set.advanced.outline_text,
            fonts: HashMap::new(),
            layers: false,
            layer_index: crate::forms::pdf_layers(doc).iter().enumerate().map(|(i, l)| (address(l.node), i)).collect(),
            in_layer: 0,
            symbol_nodes: 0,
            overprint: false,
            overprinted: false,
            invisible: false,
            inline_depth: 0,
            pattern_depth: 0,
            left_out: HashSet::new(),
        }
    }

    /// Whether a layer shows transparency, which CMYK pages put in a group of its own.
    pub(super) fn shows_transparency(&mut self) -> bool {
        let doc = self.doc;
        *self.transparent.get_or_insert_with(|| doc.layers.iter().any(|l| l.shows_transparency()))
    }
}

/// Glyphs written together as real text: the same font at the same size along one baseline.
pub(super) struct GlyphLine {
    pub(super) face: u32,
    pub(super) font: krilla::text::Font,
    pub(super) size: f64,
    /// The glyph space (krilla's, y down, at `size`) → text space, at the line's start.
    pub(super) frame: Affine,
    /// Glyph id, pen position and advance along the line (in glyph space), and the characters it
    /// stands for (bytes of the plain text, or of the list marker's text).
    pub(super) glyphs: Vec<(u32, f64, f64, std::ops::Range<usize>)>,
    /// The list marker the glyphs belong to, by its first glyph's index in the layout
    /// ([`TextLayout::marker_of`]); `None` for glyphs of the plain text.
    pub(super) marker: Option<usize>,
}

/// Most pixels along a side of a freeform gradient's image (its colour field is smooth).
pub(super) const MAX_FIELD_PX: f64 = 512.0;

/// Most pixels along a side of a linear or radial gradient's image.
pub(super) const MAX_GRADIENT_PX: f64 = 4096.0;

/// The largest magnitude a coordinate, length or transform entry may have in the content stream it
/// is drawn into (for glyph spacing, in thousandths of an em). Art that would go beyond it is left
/// out. A PDF/A-2 file may hold only integers from -2,147,483,648 to 2,147,483,647 (ISO 19005-2,
/// 6.1.13). The PDF writer writes a whole number in the `i32` range as an integer, and in builds
/// with overflow checks it panics on -2,147,483,648.
pub(super) const MAX_COORD: f64 = 1e9;

pub(super) fn xf(a: Affine) -> Transform {
    let c = a.as_coeffs();
    Transform::from_row(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32, c[4] as f32, c[5] as f32)
}

/// The current transform of surface `s` in the content stream it draws into, without that stream's
/// root transform (a page's y flip).
pub(super) fn ctm(s: &Surface) -> Affine {
    let t = s.ctm();
    Affine::new([t.sx(), t.ky(), t.kx(), t.sy(), t.tx(), t.ty()].map(f64::from))
}

/// Are `values` finite and within [`MAX_COORD`]?
pub(super) fn in_range(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD)
}

/// Can art spanning `b`, in the space `space` maps into the one `s` draws in, be written: `b`, the
/// transform of `s` followed by `space`, and `b` through that transform all within [`MAX_COORD`]?
pub(super) fn writable(s: &Surface, space: Affine, b: Rect) -> bool {
    let t = ctm(s) * space;
    let to = t.transform_rect_bbox(b);
    in_range(&t.as_coeffs()) && in_range(&[b.x0, b.y0, b.x1, b.y1, to.x0, to.y0, to.x1, to.y1])
}

/// `bp` as a path to draw on `s`; `None` when it is empty or reaches beyond [`MAX_COORD`] there.
pub(super) fn to_path(s: &Surface, bp: &BezPath) -> Option<Path> {
    if !writable(s, Affine::IDENTITY, bp.control_box()) {
        return None;
    }
    let mut pb = PathBuilder::new();
    for el in bp.elements() {
        match *el {
            PathEl::MoveTo(p) => pb.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => pb.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(a, p) => pb.quad_to(a.x as f32, a.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(a, b, p) => pb.cubic_to(a.x as f32, a.y as f32, b.x as f32, b.y as f32, p.x as f32, p.y as f32),
            PathEl::ClosePath => pb.close(),
        }
    }
    pb.finish()
}

pub(super) fn norm(v: f32) -> NormalizedF32 {
    NormalizedF32::new(if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 }).unwrap_or(NormalizedF32::ONE)
}

pub(super) fn rule(r: FillRule) -> krilla::paint::FillRule {
    match r {
        FillRule::NonZero => krilla::paint::FillRule::NonZero,
        FillRule::EvenOdd => krilla::paint::FillRule::EvenOdd,
    }
}

pub(super) fn blend(b: BlendMode) -> krilla::blend::BlendMode {
    use krilla::blend::BlendMode as K;
    match b {
        BlendMode::Normal => K::Normal,
        BlendMode::Darken => K::Darken,
        BlendMode::Multiply => K::Multiply,
        BlendMode::ColorBurn => K::ColorBurn,
        BlendMode::Lighten => K::Lighten,
        BlendMode::Screen => K::Screen,
        BlendMode::ColorDodge => K::ColorDodge,
        BlendMode::Overlay => K::Overlay,
        BlendMode::SoftLight => K::SoftLight,
        BlendMode::HardLight => K::HardLight,
        BlendMode::Difference => K::Difference,
        BlendMode::Exclusion => K::Exclusion,
        BlendMode::Hue => K::Hue,
        BlendMode::Saturation => K::Saturation,
        BlendMode::Color => K::Color,
        BlendMode::Luminosity => K::Luminosity,
    }
}

pub(super) fn q(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub(super) fn color(c: &Color) -> krilla::color::Color {
    match *c {
        Color::Rgb { r, g, b } => rgb::Color::new(q(r), q(g), q(b)).into(),
        Color::Cmyk { c, m, y, k } => cmyk::Color::new(q(c), q(m), q(y), q(k)).into(),
        // VectorCraft grey is ink coverage (0 = white); PDF DeviceGray is lightness.
        Color::Gray { k } => luma::Color::new(q(1.0 - k)).into(),
        Color::Lab { .. } => {
            let [r, g, b] = c.to_rgb();
            rgb::Color::new(q(r), q(g), q(b)).into()
        }
    }
}

/// Fill the page (with a margin) with a grey level (0 = black, 255 = white) at `opacity`: mask
/// backdrops.
pub(super) fn cover(s: &mut Surface, page: Rect, level: u8, opacity: f32) {
    let Some(p) = to_path(s, &page.inflate(1.0, 1.0).to_path(0.1)) else { return };
    s.set_stroke(None);
    s.set_fill(Some(Fill { paint: rgb::Color::new(level, level, level).into(), opacity: norm(opacity), rule: krilla::paint::FillRule::NonZero }));
    s.draw_path(&p);
}

/// An alpha mask of constant `alpha` over the page. A group drawn through it is a transparency
/// group of that opacity that, unlike the writer's opacity groups, is not isolated.
/// The cover is white, so a reader that takes every soft mask as a luminosity mask shows the same.
pub(super) fn constant_mask(s: &mut Surface, page: Rect, alpha: f32) -> krilla::mask::Mask {
    let mut sb = s.stream_builder();
    let mut ms = sb.surface();
    cover(&mut ms, page, 255, alpha);
    ms.finish();
    krilla::mask::Mask::new(sb.finish(), krilla::mask::MaskType::Alpha)
}

/// A node's address, to tell the very node apart from equal ones.
pub(super) fn address(n: &Node) -> usize {
    std::ptr::from_ref(n).addr()
}

pub(super) fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}
