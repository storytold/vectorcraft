use super::raster::ViewCrop;
use super::{
    Affine, AppearanceItem, Arc, BezPath, Channels, Cms, DefaultHasher, FillRule, Hash, Hasher, Node, NodeId, NodeKind, PathEl, Point, RasterFx,
    Rect, Size, Weak, effects,
};

/// What [`Renderer::raster_fx`] paints effects around.
pub(super) struct Content<'a> {
    /// The object the effects belong to (cached shadows are checked against it).
    pub(super) node: &'a Node,
    /// The `Arc` holding `node`, when known: cached rasters keep it and match it by identity.
    pub(super) owner: Option<&'a Arc<Node>>,
    /// First shadow-cache slot of these effects (`None`: don't cache).
    pub(super) cache: Option<usize>,
    /// Document-space bounds of the painted content.
    pub(super) reach: Rect,
    /// Its silhouette: Feather and Inner Glow clip to it (without one, Feather only blurs and
    /// Inner Glow is skipped).
    pub(super) outline: Option<(&'a BezPath, FillRule)>,
    /// What Rasterize makes its image over: the object's [`drawn_bounds`] as far as the effects
    /// before it spread them (`None`: `reach`).
    pub(super) bounds: Option<Rect>,
    /// Where the image of the Rasterize first in these effects is kept for the effects after a
    /// split ([`Renderer::raster_fx`]), which draw this content more than once.
    pub(super) kept: Option<&'a Kept>,
}

/// The image of the last Rasterize before a split in an effect stack ([`Renderer::raster_fx`]). It
/// is made the first time the effects after the split draw the content and drawn from here the
/// other times, so the content before the split is drawn once.
pub(super) struct Kept {
    /// Where the effects after the split take the content from: the visible area grown by how
    /// far they paint.
    pub(super) window: Rect,
    /// The view and pixel size of the split's frame, when the image is over
    /// [`Renderer::rasterize_image`]'s limits: the content is then drawn in that frame.
    pub(super) frame: Option<(Affine, f64)>,
    pub(super) image: std::cell::RefCell<Option<KeptImage>>,
}

#[derive(Clone)]
pub(super) enum KeptImage {
    /// The image on its own grid, as [`Renderer::rasterize_image`] makes it, and how many images
    /// in the content failed to decode while it was made ([`crate::paint::decode_failures`]):
    /// each draw of the image from here counts them again ([`crate::paint::add_decode_failures`]).
    Grid(std::rc::Rc<(Vec<u8>, usize, usize)>, u64),
    /// An image over [`Renderer::rasterize_image`]'s limits: the content's raster with the
    /// Rasterize applied, as the first draw made it, and the transform from its pixels to
    /// document space.
    Raster(Arc<vello_cpu::Pixmap>, Affine),
}

/// A cached shadow / glow raster: premultiplied, already tinted and faded, positioned relative to
/// the view-space position of `anchor` (a document point).
pub(crate) struct ShadowEntry {
    pub(super) node: Drawn,
    pub(super) linear: [u64; 4],
    pub(super) anchor: vectorcraft_geom::Point,
    /// Pixel offset of the raster's top-left from the anchor's view position.
    pub(super) rel: (f64, f64),
    pub(super) image: std::sync::Arc<vello_cpu::Pixmap>,
    /// View pixels per raster pixel: more than 1 for a glow made coarser to fit the budget.
    pub(super) scale: f64,
    /// `None` for a raster of the whole silhouette's reach: it draws after a pan too. For one
    /// cropped to the view, or made over such a raster, the view and the rect it was made for: it
    /// draws only there ([`PixelEntry::view`]).
    pub(super) view: Option<ViewCrop>,
    pub(super) made: Made,
    /// What the silhouette read when the raster was made.
    pub(super) inputs: Inputs,
    /// Made for the object before it moved ([`Renderer::move_shadow`]), or made over rasters
    /// made before their objects moved: only a frame rendered while interacting draws it.
    pub(super) moved: bool,
    pub(crate) stamp: u64,
}

/// A shadow or glow raster ready to draw ([`Renderer::draw_shadow`]).
pub(super) struct Shadow {
    pub(super) image: std::sync::Arc<vello_cpu::Pixmap>,
    /// Raster to view pixels.
    pub(super) at: Affine,
    pub(super) mode: vectorcraft_doc::color::BlendMode,
    /// Made before its object moved ([`ShadowEntry::moved`]).
    pub(super) moved: bool,
    /// For a raster cropped to the view, or made over a raster that is ([`ShadowEntry::view`]), the
    /// rect it was made for, in this frame's device pixels ([`ViewCrop::served`]).
    pub(super) cropped: Option<Rect>,
}

impl ShadowEntry {
    /// The bytes it holds, for [`FX_RASTER_BUDGET`].
    pub(crate) fn bytes(&self) -> usize {
        raster_bytes(&self.image) + self.made.bytes()
    }
}

/// What a cached shadow, glow or filtered raster was made from. While
/// [`crate::RenderOptions::interacting`], an object whose content only moved after the raster
/// was made draws from that raster ([`Renderer::move_shadow`], [`Renderer::move_pixel_content`]).
pub(crate) struct Made {
    /// The content's pixels as drawn into the raster before the effects ran: premultiplied RGBA,
    /// or their alpha alone under a shadow or glow that is one color ([`silhouette`]).
    pub(super) pixels: Vec<u8>,
    /// The effects run on them.
    pub(super) fx: Vec<RasterFx>,
    /// The size of the content's document-space bounds.
    pub(super) size: Size,
    /// The color channels the Photoshop-style filters were told of (none for shadows and glows).
    pub(super) channels: Option<Channels>,
    /// Feather's clip outline, moved to put the raster's anchor at the origin.
    pub(super) outline: Option<(BezPath, FillRule)>,
}

/// How far (document units) the size of a moved object's content and the points of its Feather
/// outline may lie from those of the content a raster was made from.
const MOVED_SLACK: f64 = 1e-6;

impl Made {
    /// The bytes it holds beyond the raster.
    pub(super) fn bytes(&self) -> usize {
        self.pixels.len() + self.outline.as_ref().map_or(0, |(g, _)| crate::lru::path_bytes(g))
    }

    /// Whether content of size `size` under the effects `fx`, told of `channels`, with Feather's
    /// `outline` (moved to put its anchor at the origin) may be this content moved: the same
    /// effects, channels and outline rule, and the size and outline within [`MOVED_SLACK`].
    pub(super) fn fits(&self, fx: &[&RasterFx], size: Size, channels: Option<Channels>, outline: Option<&(BezPath, FillRule)>) -> bool {
        let near = |a: f64, b: f64| (a - b).abs() <= MOVED_SLACK;
        self.fx.len() == fx.len()
            && self.fx.iter().zip(fx).all(|(a, b)| a == *b)
            && near(self.size.width, size.width)
            && near(self.size.height, size.height)
            && self.channels == channels
            && match (&self.outline, outline) {
                (None, None) => true,
                (Some((a, ra)), Some((b, rb))) => ra == rb && same_path(a, b),
                _ => false,
            }
    }
}

/// Whether `a` and `b` have the same elements with points within [`MOVED_SLACK`] of each other.
fn same_path(a: &BezPath, b: &BezPath) -> bool {
    let near = |p: &Point, q: &Point| (*p - *q).hypot() <= MOVED_SLACK;
    a.elements().len() == b.elements().len()
        && a.elements().iter().zip(b.elements()).all(|pair| match pair {
            (PathEl::MoveTo(p), PathEl::MoveTo(q)) | (PathEl::LineTo(p), PathEl::LineTo(q)) => near(p, q),
            (PathEl::QuadTo(p0, p1), PathEl::QuadTo(q0, q1)) => near(p0, q0) && near(p1, q1),
            (PathEl::CurveTo(p0, p1, p2), PathEl::CurveTo(q0, q1, q2)) => near(p0, q0) && near(p1, q1) && near(p2, q2),
            (PathEl::ClosePath, PathEl::ClosePath) => true,
            _ => false,
        })
}

/// Feather's clip outline of `content` when `chain` feathers it, moved to put `anchor` at the
/// origin ([`Made::outline`]).
pub(super) fn feather_outline(chain: &[&RasterFx], content: &Content, anchor: Point) -> Option<(BezPath, FillRule)> {
    let (g, rule) = content.outline.filter(|_| chain.iter().any(|fx| matches!(fx, RasterFx::Feather { .. })))?;
    let mut g = g.clone();
    g.apply_affine(Affine::translate(-anchor.to_vec2()));
    Some((g, rule))
}

/// The object a cached raster was made for: the drawn `Arc` itself when known (kept alive, so its
/// address, the cache key, names it alone and a key match is exact), else a copy compared by value.
pub(crate) enum Drawn {
    Shared(Arc<Node>),
    Copy(Box<Node>),
}

impl Drawn {
    pub(super) fn of(n: &Node, owner: Option<&Arc<Node>>) -> Self {
        match owner {
            Some(a) => Self::Shared(a.clone()),
            None => Self::Copy(Box::new(n.clone())),
        }
    }

    /// Is `n` the object drawn?
    pub(super) fn is(&self, n: &Node) -> bool {
        match self {
            Self::Shared(a) => std::ptr::eq(&**a, n),
            Self::Copy(c) => **c == *n,
        }
    }

    /// The id of the object drawn.
    pub(super) fn id(&self) -> NodeId {
        match self {
            Self::Shared(a) => a.id,
            Self::Copy(c) => c.id,
        }
    }
}

/// What an object's content reads from outside its own nodes, in it or in the opacity masks of the
/// objects in it ([`Reads::of`]): which parts of [`Inputs`] can change what its rasters show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reads {
    /// Pattern paints: the pattern definitions. Their art can hold anything, so content painted
    /// with a pattern reads everything else too.
    pub(super) patterns: bool,
    /// Symbol instances and type with inline graphics: the symbols' art. Symbol art can hold
    /// anything, so content that shows a symbol reads everything else too.
    pub(super) symbols: bool,
    /// Images and placed documents: their pixels and files, and the options and the layer's Dim
    /// Images they are drawn with.
    pub(super) images: bool,
    /// Brushed strokes: the brush library.
    pub(super) brushes: bool,
    /// Type: the fonts, whether overprinting shows ([`crate::proof::overprints`]) and the
    /// substitution highlights.
    pub(super) text: bool,
    /// Plug-in effects on its objects: the plug-ins installed. Those in the symbol and pattern art
    /// it shows are found with the document's inputs ([`DocInputs`]).
    pub(super) plugins: bool,
}

impl Reads {
    /// What the content of `n` reads.
    pub(crate) fn of(n: &Node) -> Self {
        Self::of_all(std::iter::once(n))
    }

    /// What the content of `nodes` reads together. A pattern's art can hold anything, as a
    /// symbol's can, so content painted with a pattern reads everything else too (#1723).
    pub(crate) fn of_all<'a>(nodes: impl Iterator<Item = &'a Node>) -> Self {
        let mut reads = Self::default();
        nodes.for_each(|n| reads.add(n));
        if reads.symbols || reads.patterns {
            return Self { patterns: true, symbols: true, images: true, brushes: true, text: true, plugins: reads.plugins };
        }
        reads
    }

    /// Does the content read anything outside its own nodes?
    pub(crate) fn any(self) -> bool {
        self != Self::default()
    }

    /// Can installing or removing a plug-in change what the content draws? It can when an object in
    /// it has a plug-in effect, and when it shows symbol or pattern art.
    pub(crate) fn plugins_can_change(self) -> bool {
        self.plugins || self.symbols || self.patterns
    }

    /// Does the content show symbol or pattern art?
    pub(crate) fn shows_definitions(self) -> bool {
        self.symbols || self.patterns
    }

    fn add(&mut self, n: &Node) {
        let pattern = |p: &vectorcraft_color::Paint| matches!(p, vectorcraft_color::Paint::Pattern { .. });
        n.walk(&mut |c| {
            self.plugins |= effects::has_plugin_effect(c);
            for item in &c.appearance.items {
                self.patterns |= pattern(item.paint());
                self.brushes |= matches!(item, AppearanceItem::Stroke(s) if s.brush.is_some());
            }
            match &c.kind {
                NodeKind::SymbolInstance { .. } => self.symbols = true,
                NodeKind::Image(_) | NodeKind::PlacedDocument(_) => self.images = true,
                NodeKind::Text(t) => {
                    self.text = true;
                    self.symbols |= t.runs.iter().any(|r| r.inline.is_some());
                    self.patterns |= t.runs.iter().any(|r| pattern(&r.style.fill) || pattern(&r.style.stroke));
                }
                _ => {}
            }
            if let Some(m) = c.extras().mask.as_deref() {
                self.add(&m.art);
            }
        });
    }
}

/// What a cached shadow, glow or filtered raster was drawn from besides its object, its effects
/// and the view's zoom and rotation ([`Renderer::raster_inputs`]). A raster is drawn from only
/// while its inputs equal the frame's.
#[derive(Clone)]
pub(crate) struct Inputs {
    /// What the content reads.
    pub(super) reads: Reads,
    /// A hash of the inputs as found when the raster was made: those every raster reads, those
    /// `reads` names, and what the effects read themselves ([`Inputs::with`]).
    pub(super) key: u64,
    /// The document's resources and the color settings `key` was found with: holding them keeps
    /// the addresses they hashed for their values ([`DocInputs`]).
    pub(super) _doc: Arc<DocInputs>,
}

impl PartialEq for Inputs {
    fn eq(&self, other: &Self) -> bool {
        (self.reads, self.key) == (other.reads, other.key)
    }
}

impl Inputs {
    /// These inputs with `extra`, something the effects read themselves.
    pub(super) fn with(mut self, extra: impl Hash) -> Self {
        let mut h = DefaultHasher::new();
        (self.key, extra).hash(&mut h);
        self.key = h.finish();
        self
    }
}

/// What raster contents read from the document by name or key, and the color settings, as a frame
/// finds them ([`Renderer::doc_inputs`]): hashes of the patterns' tilings
/// ([`crate::pattern::hash_tiling`]), of the symbols' names and art and of the images' keys and
/// pixels, the address of the color settings, and whether the patterns' and symbols' art has a
/// plug-in effect. Art, pixels and previews are hashed by the addresses of their shared values,
/// and names, keys, tiles, tile types and overlaps are hashed by value. The live cache keeps these
/// inputs with each evaluated live object, with each mesh's quads and with what each walk of a
/// live object or of effect art found when it found no placed file (`crate::live`), and
/// [`Renderer::fx_art`] keeps them with art that can be made from evaluated live objects.
pub(crate) struct DocInputs {
    pub(super) patterns: u64,
    pub(super) symbols: u64,
    pub(super) images: u64,
    pub(crate) cms: usize,
    /// Whether an object in a pattern's or a symbol's art, or in an opacity mask there, has a
    /// visible plug-in effect.
    pub(super) plugins: bool,
    /// Weak references to the values whose addresses the hashes hold: a weak reference keeps the
    /// allocation, not the value, so no other value gets one of those addresses while these
    /// inputs are kept.
    pub(super) _held: Held,
}

impl DocInputs {
    /// Whether `self` and `other` were made from the same patterns and symbols: their names,
    /// tilings and art.
    pub(crate) fn same_art(&self, other: &DocInputs) -> bool {
        (self.patterns, self.symbols) == (other.patterns, other.symbols)
    }
}

/// The pattern and symbol art, the image pixels and the color settings, held weakly.
pub(super) type Held = (Vec<Weak<Node>>, Vec<Weak<Vec<u8>>>, Weak<Cms>);

/// Most bytes the shadow and filtered-content rasters of one renderer hold together, with what each
/// was made from ([`Made`]): the least recently drawn go beyond it.
pub(crate) const FX_RASTER_BUDGET: usize = 512 << 20;

pub(super) fn raster_bytes(pm: &vello_cpu::Pixmap) -> usize {
    pm.width() as usize * pm.height() as usize * 4
}

/// Live-effect geometry of a node, kept across frames: the object-level result and each
/// appearance item's own result on it (filled in as items are drawn). The node is kept alive, so
/// its address names it alone and a hit is exact.
pub(crate) struct FxGeom {
    pub(crate) node: Arc<Node>,
    pub(super) g: Arc<BezPath>,
    pub(super) items: Vec<Option<Arc<BezPath>>>,
    pub(crate) stamp: u64,
}
