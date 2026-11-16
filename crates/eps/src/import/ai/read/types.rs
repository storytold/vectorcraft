use super::{
    Affine, BlendMode, Dash, FillRule, HAIRLINE, Knockout, LayerColor, LineCap, LineJoin, Named, Node, NodeId, Obj, PageBox, Paint, Rect, V,
};

/// Deepest nesting of layers, groups and dictionaries.
pub(super) const MAX_DEPTH: usize = 256;

/// Most nested pattern and symbol readers on one call stack.
pub(super) const MAX_SUBREADERS: usize = 32;

/// Most operands on the stack.
pub(super) const MAX_STACK: usize = 1 << 20;

/// Most objects read. The `.ai` editing data writer writes no more ([`crate::import::MAX_EDITING_OBJECTS`]).
pub(crate) const MAX_NODES: usize = 4_000_000;

/// The most objects all the blends of one import may lay out to be checked (each twice) and made
/// live; past it, blends keep their drawn steps.
pub(super) const BLEND_BUDGET: usize = 50_000;

/// Most points in one path.
pub(super) const MAX_POINTS: usize = 4_000_000;

/// Largest placed file's preview read (pixels).
pub(super) const MAX_PIXELS: u64 = 1 << 25;
/// Most placed files whose other placement ([`super::Structure::placed_elsewhere`]) is kept.
pub(super) const MAX_PLACED_ELSEWHERE: usize = 4096;

/// The note for an image past the hard limits ([`super::raster::Limits`]) in a file without page
/// art to replace it.
pub(super) const IMAGE_TOO_LARGE: &str = "an image too large to read from the editing data was left out";

/// What an image past the hard limits is, as the notes name it: with page art, its box becomes
/// that art (#1479).
pub(super) const LARGE_IMAGES: &str = "very large images";

/// Largest side of the document and its artboards (points).
pub(super) const MAX_SIDE: f64 = 1e6;

/// Art space (y up) → a pattern's space in the document's terms (y down).
pub(crate) const FLIP: Affine = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);

/// Most patterns read (others can't fill art).
pub(super) const MAX_PATTERNS: usize = 4096;

/// Most symbol definitions read (instances of others come in as the page draws them).
pub(crate) const MAX_SYMBOLS: usize = 4096;

/// Most names a symbol list orders.
pub(super) const MAX_LISTED: usize = 4 * MAX_SYMBOLS;

/// Most characters of a path's note read (the version 7 specification allows 254, §5.3.1).
pub(super) const MAX_NOTE: usize = 1024;

/// Most object tags (`XT`) kept in one object's data. SVG import keeps the same number of `data-*`
/// attributes per element.
pub(super) const MAX_TAGS: usize = 64;

/// The longest identifier or string an object tag (`XT`) may have, in bytes: SVG import keeps
/// another application's attribute values up to the same length.
pub(super) const MAX_TAG_TEXT: usize = 64 * 1024;

/// The object tag that holds the object's URL (version 7 specification §7).
pub(super) const URL_TAG: &str = "AdobeURL";

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
    /// The note (`%AI3_Note:`) the paths that follow get, until another replaces it (`None`: no
    /// note comment yet; empty: none).
    pub(super) note: Option<String>,
    /// Show Center (`1 Ap`) for the paths that follow (`None`: the path's default).
    pub(super) show_center: Option<bool>,
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
            note: None,
            show_center: None,
        }
    }
}

impl GState {
    /// The width its stroke draws at (see [`line_width`]).
    pub(crate) fn line_width(&self) -> f64 {
        line_width(self.width)
    }
}

/// The width a stroke of `w w` draws at: `0 w` is the thinnest line a device draws, a [`HAIRLINE`].
pub(super) fn line_width(w: f64) -> f64 {
    if w > 0.0 { w } else { HAIRLINE }
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
    pub(super) template: bool,
    pub(super) dim: Option<u8>,
    pub(super) color: Option<LayerColor>,
}

#[derive(Debug)]
pub(super) enum Kind {
    Layer(Box<LayerAttrs>),
    Group,
    /// Wraparound text (`*w` … `*W`, version 7 specification §15.4.2): a group whose objects other
    /// than its type are wrap objects.
    Wrap,
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
    /// [`Reader::mesh_ended`] when it opened.
    pub(super) mesh_ended: bool,
    /// The state when it opened (a container's lock and visibility; a dictionary's state to go
    /// back to).
    pub(super) gs: GState,
    /// Unresolved pattern paint saved by `q`, which restores graphics state at `Q`.
    pub(super) pending_unread_fill: bool,
    pub(super) pending_unread_stroke: bool,
    /// The operand stack's height when it opened.
    pub(super) base: usize,
    /// The clipping paths' indexes among the children.
    pub(super) clips: Vec<usize>,
    /// A compound member group explicitly declares its subpaths' winding with `D`.
    pub(super) compound_members: bool,
    /// Unread kinds belonging to this top-level object.
    pub(super) unread: Vec<String>,
    /// A dictionary in the art: the transparency (`Xy`) its art set last, which stays after it.
    pub(super) state: Option<(f32, BlendMode, bool, Knockout, bool)>,
}

/// An envelope whose dictionary was read, waiting for its drawn art and its plugin's `XP` (see
/// [`super::super::envelope`]).
#[derive(Debug)]
pub(super) struct PendingEnvelope {
    pub(super) obj: Obj,
    /// The group on `%_` lines before the dictionary.
    pub(super) before: Option<Node>,
    /// The art in the dictionary's `AIDeformArtKey` block.
    pub(super) inner: Vec<Node>,
    /// How many frames were open when the dictionary closed, and how many children the innermost
    /// had: its drawn art is the one child after them.
    pub(super) depth: usize,
    pub(super) siblings: usize,
    /// Its contents hold type: it is settled once the type is laid out
    /// ([`super::super::envelope::TypedEnvelope`]).
    pub(super) typed: bool,
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

/// What a reader of a symbol's definition has read of the symbol besides its art.
#[derive(Debug, Default)]
pub(super) struct SymbolHead {
    /// `AI13PatternRegistrationType` in the dictionary after the art.
    pub(super) registration: Option<f64>,
    /// Art the reader left out (type, a placed file, non-native art, an image it can't read or
    /// place, a gradient it can't read): the symbol isn't kept, and its instances come in as the
    /// page draws them, within the box of the rest.
    pub(super) left_out: bool,
    /// A gradient instance the reader can't make a paint of (a gradient the file doesn't define,
    /// or one it can't place) is the next path's fill or stroke: the art is left out when that
    /// path paints it.
    pub(super) missing_fill: bool,
    pub(super) missing_stroke: bool,
}

/// The colors a brush's art uses, counted as the page's are once a brush that is kept uses the art.
#[derive(Debug, Default)]
pub(super) struct BrushArtColors {
    /// The named colors its color operators define.
    pub(super) spots: Vec<Named>,
    /// The named colors its gradient stops name.
    pub(super) stop_spots: Vec<Named>,
    /// Its colors set in each model.
    pub(super) cmyk: usize,
    pub(super) rgb: usize,
}

impl BrushArtColors {
    /// Keep each name's first definition only, its color operators' before its gradient stops',
    /// and at most `limit` names in all. The process-color counts stay as they are.
    pub(super) fn keep_first(&mut self, limit: usize) {
        let mut seen = std::collections::HashSet::new();
        let mut keep = |n: &Named| seen.len() < limit && seen.insert(n.name.clone());
        self.spots.retain(|n| keep(n));
        self.stop_spots.retain(|n| keep(n));
    }
}

/// An image's operands, from `XN` and before `XI`.
#[derive(Debug, Default)]
pub(super) struct Raster {
    pub(super) space: Option<String>,
    /// The ink of an image in a Separation space (`XC`): its paint at 100 % and the named colour
    /// for its swatch (`paint::separation_ink`).
    pub(super) ink: Option<(Paint, Option<Named>)>,
    /// `XI`'s operands when its samples follow as hex in comment lines (`%FF00…`) up to
    /// `%AI5_EndRaster`, the form the version 7 specification documents (§8).
    pub(super) operands: Option<Vec<V>>,
    /// The bytes of those hex digits, up to `max`.
    pub(super) samples: Vec<u8>,
    /// The bytes of samples the image reads (0 when it isn't read).
    pub(super) max: usize,
    /// A digit waiting for its pair.
    pub(super) half: Option<u8>,
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
    /// The file's folder relative to the document's (`src/`), from the string between
    /// `%AI28_BeginPlacedRelativePath` and its end.
    pub(super) folder: Option<String>,
    /// The operand stack's height at `%AI28_BeginPlacedRelativePath`, until its end.
    pub(super) folder_at: Option<usize>,
    /// Its `` ` `` was read.
    pub(super) ticked: bool,
    /// It began on a `%_` line, in a hidden object or on a layer that doesn't show.
    pub(super) hidden: bool,
    /// What it holds can't be read: operands of another shape, a second `` ` ``, or a document
    /// included in it that doesn't end.
    pub(super) broken: bool,
}

/// The longest relative folder of a placed file read (bytes).
pub(super) const MAX_PLACED_FOLDER: usize = 4096;

/// The dictionary entry after a placed PDF or `.ai` file that holds the page shown (1-based).
pub(super) const PLACED_PAGE: &str = "AI11 PlacedPDFPageIndex";
/// The dictionary entry after a placed PDF or `.ai` file that holds the box it is cropped to.
pub(super) const PLACED_CROP: &str = "AI11 PlacedPDFCropTo";

/// The placed file just read, until the dictionary after it: it may say the file is a page of a
/// PDF or `.ai` file.
#[derive(Debug)]
pub(super) struct PlacedTail {
    /// Its linked image.
    pub(super) id: NodeId,
    /// Its image's key in the document's images.
    pub(super) key: String,
    /// The size of its box.
    pub(super) size: (f64, f64),
    /// Its image's blob was stored for it, not shared with an earlier image.
    pub(super) fresh: bool,
    /// The frames open when it was read: its dictionary closes at the same depth.
    pub(super) depth: usize,
}

/// A placed file the dictionary after it says is a page of a PDF or `.ai` file. Its linked image
/// becomes a placed document once the layers have been compared with the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedPage {
    /// The linked image.
    pub id: NodeId,
    /// The page shown (1-based).
    pub page: u32,
    /// The art's bounds are shown, not `page_box`.
    pub bounding: bool,
    /// The box of the page shown.
    pub page_box: PageBox,
    /// The size of the placed file's box.
    pub size: (f64, f64),
}

/// What the open result says of placed files: they show the preview the file keeps of them.
pub(super) const LINKED_PREVIEWS: &str =
    "linked pictures show the low-resolution preview saved in the file until their files are found: relink them in the Links panel";

/// What the open result says of linked images the data keeps no pixels of (`XF`): they show a
/// placeholder.
pub const LINKED_PLACEHOLDERS: &str =
    "linked pictures with no preview in the file show a placeholder until their files are found: relink them in the Links panel";

/// Operators that change nothing the reader keeps.
pub(super) const IGNORED: &[&str] = &[
    "Ae",
    "AE",
    "As",
    "Xd",
    "Xr",
    "Xh",
    "XH",
    "Bc",
    "XP",
    "Np",
    // Flatness (`i`): how finely curves print.
    "i",
    // Path resolution (`Ar`, §12): how finely curves print.
    "Ar",
    "Xs",
    // The samples of the pen that `XD` notes on the object before them (`Reader::pen_samples`).
    "Xt",
    "Xi",
    "XI",
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
/// Patterns, symbols and the symbol list are read before this list is looked at; their markers
/// stay here for sections of the same kind nested in them to be skipped.
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
    // The printer's crop marks, drawn after the layers (`userdict /AI3_noCropMarks known not {
    // … } if`); the crop area they mark is `%AI3_Cropmarks`.
    ("BeginCrops", "EndCrops"),
];

/// The note for versioned content whose `%AI17_End_Versioned_Content` is missing: every
/// alternative after the first is read too, so some art may come in twice.
pub const VERSIONED_UNENDED: &str = "versioned content that doesn't end was read whole: some art may come in twice";

/// The note for a `/ForeignObject` (non-native art, the content of a placed PDF, which the editor
/// shows but doesn't edit) whose PDF can't be read.
pub const NON_NATIVE_ART_UNREAD: &str = "non-native art (a placed PDF's content) that VectorCraft can't read is left out";

/// The name of the group a `/ForeignObject`'s art comes in as.
pub const NON_NATIVE_ART: &str = "Non-native art";

/// The note for a file with a graph (`%_Gs` … `%_GS`) whose data and settings can't be kept: its
/// drawn art comes in as plain art.
pub const GRAPH_PLAIN: &str = "graphs come in as plain art, without their data or settings";

/// The note for a graph that starts inside another or doesn't end before its layer does: the art
/// read so far is kept, and what follows the layer is read as usual.
pub const GRAPH_UNENDED: &str = "a graph's records don't end where they should: its art was read as it is";

/// A graph's records (the version 7 specification's chapter 17 and appendix A): two-letter
/// operators starting with `G`, read with their operands.
pub(super) const GRAPH_OPS: &[&str] = &[
    "G1", "G+", "Ga", "GA", "Gb", "Gc", "GC", "Gd", "GD", "Ge", "Gf", "Gg", "Gi", "Gj", "Gl", "Gm", "Go", "Gp", "Gr", "Gs", "GS", "Gt", "GT", "Gv",
    "Gw", "GX", "Gx", "Gy", "Gz",
];
