use super::{Affine, BlendMode, Dash, FillRule, Knockout, LayerColor, LineCap, LineJoin, Node, Obj, Paint, Rect, V};

/// Deepest nesting of layers, groups and dictionaries.
pub(super) const MAX_DEPTH: usize = 256;

/// Most operands on the stack.
pub(super) const MAX_STACK: usize = 1 << 20;

/// Most objects read.
pub(super) const MAX_NODES: usize = 4_000_000;

/// The most objects all the blends of one import may lay out to be checked (each twice) and made
/// live; past it, blends keep their drawn steps.
pub(super) const BLEND_BUDGET: usize = 50_000;

/// Most points in one path.
pub(super) const MAX_POINTS: usize = 4_000_000;

/// Largest image read (pixels).
pub(super) const MAX_PIXELS: u64 = 1 << 25;
/// Most placed files whose other placement ([`super::Structure::placed_elsewhere`]) is kept.
pub(super) const MAX_PLACED_ELSEWHERE: usize = 4096;

/// The note for an image past [`MAX_PIXELS`]: the file then opens from its PDF part, which reads
/// larger images (#1479).
pub const IMAGE_TOO_LARGE: &str = "an image too large to read from the editing data was left out";

/// Most hex digits of an image's samples read from comment lines (CMYK and alpha at the largest size).
pub(super) const MAX_HEX: usize = (MAX_PIXELS as usize) * 10;

/// Largest side of the document and its artboards (points).
pub(super) const MAX_SIDE: f64 = 1e6;

/// Art space (y up) → a pattern's space in the document's terms (y down).
pub(crate) const FLIP: Affine = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);

/// Most patterns read (others can't fill art).
pub(super) const MAX_PATTERNS: usize = 4096;

#[derive(Clone, Debug)]
pub(crate) struct GState {
    pub(crate) fill: Paint,
    pub(crate) stroke: Paint,
    pub(crate) width: f64,
    pub(crate) cap: LineCap,
    pub(crate) join: LineJoin,
    pub(crate) miter: f64,
    pub(crate) dash: Option<Dash>,
    pub(super) rule: FillRule,
    pub(super) opacity: f32,
    pub(super) blend: BlendMode,
    pub(super) isolate: bool,
    pub(super) knockout: Knockout,
    pub(super) knockout_shape: bool,
    pub(crate) overprint_fill: bool,
    pub(crate) overprint_stroke: bool,
    pub(super) locked: bool,
    pub(super) hidden: bool,
}

impl Default for GState {
    fn default() -> Self {
        let black = Paint::solid(vectorcraft_color::Color::BLACK);
        Self {
            fill: black.clone(),
            stroke: black,
            width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter: 10.0,
            dash: None,
            rule: FillRule::NonZero,
            opacity: 1.0,
            blend: BlendMode::Normal,
            isolate: false,
            knockout: Knockout::Off,
            knockout_shape: false,
            overprint_fill: false,
            overprint_stroke: false,
            locked: false,
            hidden: false,
        }
    }
}

/// The blend modes by their number in `Xy`.
pub(super) const BLENDS: [BlendMode; 16] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::HardLight,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
];

/// The name effects take for blend mode `n` of `Xy` (and of effects' `blnd`).
pub(in super::super) fn blend_name(n: usize) -> String {
    format!("{:?}", blend_mode(n))
}

/// Blend mode `n` of `Xy` (Normal when it is out of range).
pub(in super::super) fn blend_mode(n: usize) -> BlendMode {
    BLENDS.get(n).copied().unwrap_or(BlendMode::Normal)
}

#[derive(Debug, Default)]
pub(super) struct LayerAttrs {
    pub(super) name: String,
    pub(super) visible: bool,
    pub(super) preview: bool,
    pub(super) locked: bool,
    pub(super) printable: bool,
    pub(super) dim: Option<u8>,
    pub(super) color: Option<LayerColor>,
}

#[derive(Debug)]
pub(super) enum Kind {
    Layer(Box<LayerAttrs>),
    Group,
    Compound,
    Clip,
    Mask,
    Obj(Box<Obj>),
}

#[derive(Debug)]
pub(super) struct Frame {
    pub(super) kind: Kind,
    pub(super) children: Vec<Node>,
    /// Opened on a `%_` line.
    pub(super) hidden: bool,
    /// [`Reader::after_object`] when it opened.
    pub(super) after: bool,
    /// The state when it opened (a container's lock and visibility; a dictionary's state to go
    /// back to).
    pub(super) gs: GState,
    /// The operand stack's height when it opened.
    pub(super) base: usize,
    /// The clipping paths' indexes among the children.
    pub(super) clips: Vec<usize>,
    /// A compound member group explicitly declares its subpaths' winding with `D`.
    pub(super) compound_members: bool,
}

/// What a reader of a pattern's definition has of its head, `(name) llx lly urx ury`: the operands
/// before its first operator.
#[derive(Debug, Default)]
pub(super) struct PatternHead {
    pub(super) taken: bool,
    pub(super) name: String,
    /// The tile in the pattern's space (y up).
    pub(super) tile: Option<Rect>,
}

/// An image's operands, from `XN` and before `XI`.
#[derive(Debug, Default)]
pub(super) struct Raster {
    pub(super) space: Option<String>,
    /// `XI`'s operands when its samples follow as hex in comment lines (`%FF00…`) up to
    /// `%AI5_EndRaster`, the form the version 7 specification documents (§8).
    pub(super) operands: Option<Vec<V>>,
    /// Those hex digits.
    pub(super) hex: Vec<u8>,
}

/// A placed file's operands, from `` ` `` and its preview, until `~`.
#[derive(Debug, Default)]
pub(super) struct Place {
    /// The canvas, where the file's pixels are → art space.
    pub(super) matrix: Option<Affine>,
    /// The file's box (`llx lly urx ury`): its size is the file's, in pixels.
    pub(super) bounds: Option<Rect>,
    pub(super) path: String,
    /// The preview as a PNG, and its size in pixels.
    pub(super) preview: Option<(Vec<u8>, u32, u32)>,
}

/// What the open result says of placed files: they show the preview the file keeps of them.
pub(super) const LINKED_PREVIEWS: &str =
    "linked pictures show the low-resolution preview saved in the file until their files are found: relink them in the Links panel";

/// Operators that change nothing the reader keeps.
pub(super) const IGNORED: &[&str] = &[
    "Ae",
    "AE",
    "Ap",
    "As",
    "Xd",
    "Xr",
    "XG",
    // `/DeviceGray XC` after a linked image's file name (`XG`): its colour space.
    "XC",
    "Xh",
    "XH",
    "XF",
    "Bc",
    "XP",
    "Np",
    // Flatness (`i`): how finely curves print.
    "i",
    // Path resolution (`Ar`, §12): how finely curves print.
    "Ar",
    "TZ",
    "Xs",
    "Xt",
    // A brushed path's width at each anchor (`(…) Xt n XD`), on its hidden source path.
    "XD",
    "Xi",
    "XI",
    "`",
    "LB2",
    "Lc",
    "Xv",
    "XV",
    "Xq",
    "XQ",
    "Xg",
    "Xn",
    "Bn",
    "gsave",
    "grestore",
    "showpage",
    "annotatepage",
];

/// Sections skipped whole, by their markers without the `AI<version>_` prefix: begin → end.
pub(super) const SKIPPED: &[(&str, &str)] = &[
    ("BeginPattern", "EndPattern"),
    ("BeginBrushPattern", "EndBrushPattern"),
    ("BeginSVGFilter", "EndSVGFilter"),
    ("BeginSymbol", "EndSymbol"),
    ("BeginPluginObject", "EndPluginObject"),
    ("BeginArtStyles", "EndArtStyles"),
    ("BeginArtStyleList", "EndArtStyleList"),
    ("BeginSymbolList", "EndSymbolList"),
    ("BeginTextDocument", "EndTextDocument"),
    ("BeginEncoding", "EndEncoding"),
];

/// The note for versioned content whose `%AI17_End_Versioned_Content` is missing: every
/// alternative after the first is read too, so some art may come in twice.
pub const VERSIONED_UNENDED: &str = "versioned content that doesn't end was read whole: some art may come in twice";

/// The note for a `/ForeignObject` (non-native art, the content of a placed PDF, which the editor
/// shows but doesn't edit) whose PDF can't be read.
pub const NON_NATIVE_ART_UNREAD: &str = "non-native art (a placed PDF's content) that VectorCraft can't read is left out";

/// The name of the group a `/ForeignObject`'s art comes in as.
pub const NON_NATIVE_ART: &str = "Non-native art";
