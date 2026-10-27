use super::cid::{CidText, FontInfo};
use super::*;

/// Decoded RGBA pixels, width, height and hayro's scale factors.
pub(super) type Decoded = (Vec<u8>, u32, u32, (f32, f32));

pub(super) enum FrameKind {
    Root,
    /// A clip group whose clip path node is stored here.
    Clip(Box<Node>),
    /// A clip that was redundant (covers the page); children go straight to the parent.
    Skip,
    Group {
        opacity: f32,
        blend: BlendMode,
        mask: Option<Arc<MaskSpec>>,
        isolate: bool,
        knockout: bool,
    },
}

pub(super) struct Frame {
    pub(super) kind: FrameKind,
    pub(super) children: Vec<Arc<Node>>,
    /// The first optional content group entered inside (where the frame's art goes).
    pub(super) group: Option<usize>,
    /// The group each child was drawn in, where the frame's art goes when the frame dissolves
    /// into its parent (a layer's form drawing its sublayers' forms).
    pub(super) child_groups: Vec<Option<usize>>,
}

impl Frame {
    pub(super) fn new(kind: FrameKind) -> Self {
        Self { kind, children: vec![], group: None, child_groups: vec![] }
    }
}

/// A dissolving frame's `children`, each with the group it was drawn in (`groups`, else the
/// frame's `group`).
pub(super) fn grouped(
    children: Vec<Arc<Node>>,
    groups: Vec<Option<usize>>,
    group: Option<usize>,
) -> impl Iterator<Item = (Arc<Node>, Option<usize>)> {
    children.into_iter().zip(groups.into_iter().map(move |g| g.or(group)).chain(std::iter::repeat(group)))
}

/// Glyphs drawn consecutively with the same paint, merged into one path.
pub(super) struct GlyphRun {
    pub(super) path: BezPath,
    pub(super) paint: Paint,
    pub(super) opacity: f32,
    pub(super) stroke: Option<StrokeProps>,
    pub(super) scale: f64,
    pub(super) overprint: bool,
}

/// How often a tiling pattern was read, and the art (as its paints) each reading drew with the
/// swatch made for it.
pub(super) type PatternReadings = (u32, Vec<(Vec<Paint>, String)>);

/// Builder state a nested interpretation (soft mask, pattern cell) starts afresh and gives back.
pub(super) struct Saved {
    pub(super) stack: Vec<Frame>,
    pub(super) root_groups: Vec<Option<usize>>,
    pub(super) blend: BlendMode,
    pub(super) mask: Option<Arc<MaskSpec>>,
    pub(super) page: Rect,
    pub(super) pending: bool,
}

pub(super) struct Builder<'p> {
    pub(super) colors: Colors<'p>,
    pub(super) next: u64,
    pub(super) stack: Vec<Frame>,
    /// The optional content group of each child of the root frame.
    pub(super) root_groups: Vec<Option<usize>>,
    pub(super) blend: BlendMode,
    pub(super) page: Rect,
    pub(super) glyphs: Option<GlyphRun>,
    pub(super) text: Option<TextLine>,
    pub(super) text_as: TextAs,
    pub(super) images: HashMap<String, ImageBlob>,
    pub(super) image_keys: HashMap<u128, (String, u32, u32)>,
    /// The images kept with their CMYK samples ([`crate::import_image`]).
    pub(super) cmyk_keys: HashSet<u128>,
    /// Mask keys of CMYK images whose mask turned out to hide nothing.
    pub(super) opaque: HashSet<u128>,
    pub(super) warnings: Vec<String>,
    /// Fonts by cache key → base font name and whether the file carries its glyphs (from
    /// [`scan_page`]).
    pub(super) fonts: FontNames,
    pub(super) font_names: HashMap<u128, FontInfo>,
    pub(super) families: Option<Families>,
    pub(super) missing_fonts: Vec<String>,
    /// Font (cache key) → its characters by glyph, for CID-keyed fonts embedded without a
    /// ToUnicode map.
    pub(super) cid_text: HashMap<u128, Option<Arc<CidText>>>,
    /// Font (cache key) → the character of each glyph its embedded font program maps
    /// ([`font_chars`]), for glyphs the file's ToUnicode map doesn't name (#708).
    pub(super) font_chars: HashMap<u128, Option<Arc<HashMap<u32, char>>>>,
    /// Font (cache key) → the installed face that draws its glyphs, decided from its first glyph
    /// that can be compared ([`matching_face`]); `None`: its glyphs differ, so it stays outlines.
    pub(super) matched: HashMap<u128, Option<Arc<vectorcraft_text::FontFace>>>,
    /// (Font, glyph) → the character its outline matches in the installed face, for glyphs no
    /// mapping names ([`Builder::identified_char`]).
    pub(super) identified: HashMap<(u128, u32), Option<char>>,
    /// Per font: the installed version its type names, when it isn't the one its family and style
    /// resolve to (see `CharStyle::font_version`).
    pub(super) versions: HashMap<u128, Option<String>>,
    /// The soft mask of the graphics state, the art drawn through it so far, and the masks read.
    pub(super) mask: Option<Arc<MaskSpec>>,
    pub(super) masked: Vec<Arc<Node>>,
    pub(super) masks: HashMap<u128, Arc<MaskSpec>>,
    /// Pattern swatches made, and per (pattern, stroke) the art signatures read and their names.
    pub(super) patterns: Vec<PatternDef>,
    pub(super) pattern_keys: HashMap<(u128, bool), PatternReadings>,
    /// Per function-based shading (and the inks it paints with): what it comes in as, read once.
    pub(super) function_paints: HashMap<(u128, Option<Vec<String>>), Paint>,
    /// What is left of the import's function evaluations for those shadings.
    pub(super) function_budget: FunctionBudget,
    /// Swatch names in use.
    pub(super) taken: Vec<String>,
    /// The page as walked by [`scan_page`], and how far the interpreter is through it.
    pub(super) scan: Scan,
    pub(super) tag_at: usize,
    pub(super) group_at: usize,
    /// The fills and strokes of paths drawn so far ([`Scan::paints`]).
    pub(super) paint_at: usize,
    pub(super) aligned: bool,
    /// Whether optional content groups become layers.
    pub(super) route: bool,
    /// Some groups are hidden or don't print: art in marked content whose group can't be told
    /// (once the scan and the interpreter part) is [`UNSORTED`] rather than the page's.
    pub(super) unsure: bool,
    /// The marked-content sequences open: the group each marks.
    pub(super) marked: Vec<Option<usize>>,
    /// A transparency group was just pushed: a form's (whose flags come next) or an image's.
    pub(super) pending: bool,
    pub(super) nested: u32,
    /// Reading an alpha soft mask's art: images are read white with their own alpha, so the
    /// mask's luminance is their alpha, as whitened vector art's is (cairo, and so Inkscape,
    /// rasterizes masks into such images).
    pub(super) white_images: bool,
    /// The last filled path: its node and its outline (a stroke of the same outline joins it).
    pub(super) last_fill: Option<(NodeId, BezPath)>,
    /// What each line of type made tells beyond its text object ([`crate::import_lines`]).
    pub(super) lines: HashMap<NodeId, LineFacts>,
}

pub(super) fn blend(b: craft_pdf::interpret::BlendMode) -> BlendMode {
    use craft_pdf::interpret::BlendMode as H;
    match b {
        H::Normal => BlendMode::Normal,
        H::Multiply => BlendMode::Multiply,
        H::Screen => BlendMode::Screen,
        H::Overlay => BlendMode::Overlay,
        H::Darken => BlendMode::Darken,
        H::Lighten => BlendMode::Lighten,
        H::ColorDodge => BlendMode::ColorDodge,
        H::ColorBurn => BlendMode::ColorBurn,
        H::HardLight => BlendMode::HardLight,
        H::SoftLight => BlendMode::SoftLight,
        H::Difference => BlendMode::Difference,
        H::Exclusion => BlendMode::Exclusion,
        H::Hue => BlendMode::Hue,
        H::Saturation => BlendMode::Saturation,
        H::Color => BlendMode::Color,
        H::Luminosity => BlendMode::Luminosity,
    }
}

pub(super) fn fill_rule(r: craft_pdf::interpret::FillRule) -> FillRule {
    match r {
        craft_pdf::interpret::FillRule::NonZero => FillRule::NonZero,
        craft_pdf::interpret::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}

/// `p` with every subpath closed: how a fill reads it.
pub(crate) fn closed_subpaths(p: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut open = false;
    for el in p.elements() {
        if open && matches!(el, PathEl::MoveTo(_)) {
            out.push(PathEl::ClosePath);
        }
        out.push(*el);
        open = !matches!(el, PathEl::ClosePath);
    }
    if open {
        out.push(PathEl::ClosePath);
    }
    out
}

/// Do a fill of `fill` and a stroke of `stroke` paint one object's outline? Applications write the
/// fill without closing the path (filling closes it) and the stroke closed.
pub(super) fn same_outline(fill: &BezPath, stroke: &BezPath) -> bool {
    fill.elements() == stroke.elements() || closed_subpaths(fill).elements() == closed_subpaths(stroke).elements()
}

pub(super) fn mean_scale(a: Affine) -> f64 {
    a.determinant().abs().sqrt()
}

/// Width a PDF line width of 0 imports as, points: a print hairline. In PDF `0 w` is the thinnest
/// line the device can draw (ISO 32000-1, 8.4.3.2), not no line.
const HAIRLINE: f64 = 0.25;

/// Document width of the stroke `p` drawn at `scale`: `0 w` becomes a [`HAIRLINE`].
pub(super) fn line_width(p: &StrokeProps, scale: f64) -> f64 {
    if p.line_width == 0.0 { HAIRLINE } else { p.line_width as f64 * scale }
}

pub(super) fn stroke_layer(paint: Paint, opacity: f32, p: &StrokeProps, scale: f64) -> StrokeLayer {
    let mut st = StrokeLayer::new(paint, line_width(p, scale));
    st.opacity = opacity;
    st.cap = match p.line_cap {
        kurbo::Cap::Butt => LineCap::Butt,
        kurbo::Cap::Round => LineCap::Round,
        kurbo::Cap::Square => LineCap::Square,
    };
    st.join = match p.line_join {
        kurbo::Join::Miter => LineJoin::Miter,
        kurbo::Join::Round => LineJoin::Round,
        kurbo::Join::Bevel => LineJoin::Bevel,
    };
    st.miter_limit = p.miter_limit as f64;
    // (Every stroke keeps its own copy: a dash array has a few entries, and one with thousands is cut.)
    let dash = Dash {
        pattern: p.dash_array.iter().take(MAX_DASH_ENTRIES).map(|v| *v as f64 * scale).collect(),
        offset: p.dash_offset as f64 * scale,
        align_corners: false,
    };
    // An invalid dash array (a negative value, or all zeros) strokes solid.
    st.dash = dash.is_dashed().then_some(dash);
    st
}

/// The most entries of a dash array kept (an even count keeps it a pattern of dashes and gaps).
const MAX_DASH_ENTRIES: usize = 64;

pub(crate) fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

/// The paints of `nodes`' leaves, in order: what tells two readings of a pattern cell apart.
pub(super) fn paints(nodes: &[Arc<Node>]) -> Vec<Paint> {
    let mut out = vec![];
    for n in nodes {
        n.walk(&mut |c| out.extend(c.appearance.items.iter().map(|i| i.paint().clone())));
    }
    out
}

/// Visual bounds of `nodes`; `None` when one has none.
pub(super) fn bounds(nodes: &[Arc<Node>]) -> Option<Rect> {
    nodes.iter().try_fold(None, |acc, n| n.visual_bounds().map(|b| Some(acc.map_or(b, |a: Rect| a.union(b)))))?
}

pub(super) fn rgba_png(rgba: Vec<u8>, w: u32, h: u32) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(w, h, rgba)?;
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
    Some(out)
}

pub(super) fn resize_alpha(a: &LumaData, w: u32, h: u32) -> Vec<u8> {
    if a.width == w && a.height == h {
        return a.data.clone();
    }
    match image::GrayImage::from_raw(a.width, a.height, a.data.clone()) {
        Some(g) => image::imageops::resize(&g, w, h, image::imageops::FilterType::Triangle).into_raw(),
        None => vec![255; (w * h) as usize],
    }
}
