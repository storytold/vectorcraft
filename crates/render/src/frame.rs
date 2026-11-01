use super::options::{AntiAlias, RenderOptions};
use super::threads::single_threaded_context;
use super::{Affine, Arc, BezPath, Document, FillRule, HashMap, Ink, Node, Rect, RenderContext, SymbolBoxes, TextObject, live};

/// Hasher for pointer-keyed caches: a multiply-xorshift (SipHash showed up in frame profiles).
#[derive(Default, Clone, Copy)]
pub(crate) struct PtrHasher(pub(crate) u64);

impl std::hash::Hasher for PtrHasher {
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0 ^ *b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn write_u64(&mut self, n: u64) {
        let x = (self.0 ^ n).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        self.0 = x ^ (x >> 29);
    }
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

pub(crate) type PtrMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<PtrHasher>>;

/// The key of a cached stroke outline: ([`Renderer::stroke_owner`] of what owns the stroke, the
/// stroke's place in it, width bits, tolerance level).
pub(crate) type StrokeKey = (usize, usize, u64, i32);

/// A stroke expanded to a fill outline: (owning node, outline, last frame used).
pub(crate) struct StrokeEntry {
    pub(crate) node: Arc<Node>,
    pub(crate) outline: Arc<BezPath>,
    pub(crate) stamp: u64,
}

/// The slices of a stroke whose gradient runs along or across it, each with its paint and bounds.
pub(crate) type Slices = Arc<Vec<(BezPath, vectorcraft_color::Paint, Rect)>>;

/// [`Slices`] cached for a stroke: (owning node, slices, last frame used).
pub(crate) struct SliceEntry {
    pub(crate) node: Arc<Node>,
    pub(crate) slices: Slices,
    pub(crate) stamp: u64,
}

/// Cached per-node geometry, keyed by `Arc` identity. Structural sharing means an unchanged node
/// keeps its allocation across edits, so a pointer match (with the Arc kept alive here so the
/// address can't be reused) is an exact cache hit — no invalidation logic needed.
pub(crate) struct GeomEntry {
    pub(crate) node: Arc<Node>,
    pub(crate) bounds: Option<Rect>,
    pub(crate) path: Option<Arc<BezPath>>,
    pub(crate) stamp: u64,
}

/// A clipping path (kept alive so its address can't be reused) and the region it clips to.
pub(crate) type ClipEntry = (Arc<Node>, Option<Arc<(BezPath, FillRule)>>);

/// A clipping path (kept alive) and what it paints: its fills and its strokes ([`Node::clip_paint`]).
pub(crate) type ClipPaintEntry = (Arc<Node>, Option<Arc<Node>>, Option<Arc<Node>>);

/// An object with effects (kept alive), the art drawn for it ([`Renderer::fx_art`]), and, when that
/// art can be made from evaluated live objects (`fx::evaluates_live`) or from type, symbol art or
/// colors shaded through the color settings (`fx::reads_outside`), the inputs it was made with
/// ([`live::Found`]).
pub(crate) type FxArtEntry = (Arc<Node>, Arc<Node>, Option<live::Found>);

/// What symbol art placed by [`Renderer::placed_art`] is kept under: the art's address, the bits
/// of the transform's coefficients, whether strokes scale with it, and the stain ([`stain_bits`]).
pub(crate) type PlacedKey = (usize, [u64; 6], bool, [u32; 6]);

/// Symbol art placed by [`Renderer::placed_art`].
pub(crate) struct PlacedEntry {
    /// The symbol's art, held weakly: the reference keeps the art's allocation, so no other art can
    /// get its address while the entry exists, and does not keep the art's content.
    pub(crate) art: std::sync::Weak<Node>,
    /// The art placed, or `None` for art placed again in every frame ([`depends_on_document`]).
    pub(crate) placed: Option<Arc<Node>>,
    /// The font generation ([`vectorcraft_text::FontDb::generation`]) the art was placed in, when
    /// it holds type: the art is placed again when the fonts change.
    pub(crate) fonts: Option<u64>,
}

/// How deep inline graphics nest (text in a symbol shown inline in text…) before they draw nothing.
pub(crate) const MAX_INLINE_DEPTH: u32 = 4;

/// Set once a missing inline symbol has been logged (it would log every frame otherwise).
pub(crate) static MISSING_INLINE_LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub drawn: usize,
    pub culled: usize,
    /// How many times the frame's symbol boxes measured symbol art ([`SymbolBoxes::art_measured`]):
    /// the art boxes of symbols and how far the effects in their art paint past it. Pattern tiles
    /// and placed documents measure with symbol boxes of their own, which are not counted.
    pub symbol_art: usize,
    /// The drop shadows, outer glows and filtered contents the frame drew from rasters made before
    /// their object moved, or made over such rasters ([`RenderOptions::interacting`]), and those it
    /// drew at a drag's lower resolution ([`Self::previewed`]).
    pub moved: usize,
    /// The filtered contents, shadows and glows the frame drew at a drag's lower resolution
    /// ([`RenderOptions::dragging`]). [`Self::moved`] counts them too.
    pub previewed: usize,
    /// How many threads composed the frame from its two ink planes, in a CMYK document that shows
    /// transparency; 0 for other frames.
    pub ink_threads: usize,
    pub micros: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct Frame<'a> {
    /// Drawing into a multithreaded context: vello filter layers must be rendered offscreen.
    pub(crate) mt: bool,
    pub(crate) doc: &'a Document,
    /// `doc`'s symbol art boxes, which culling measures symbol instances by ([`SymbolBoxes::quick`]).
    pub(crate) symbols: &'a SymbolBoxes<'a>,
    pub(crate) view: Affine,
    /// Visible region in document coordinates (for culling).
    pub(crate) visible: Rect,
    /// Size of one output pixel in document units.
    pub(crate) px: f64,
    pub(crate) opts: &'a RenderOptions,
    /// What colours paint as: screen colours, or one ink plane of a CMYK document (see [`ink`]).
    pub(crate) ink: Ink,
    /// With Separations Preview, the tables placed images show the visible plates through.
    pub(crate) plates: Option<&'a crate::proof::PlateLut>,
}

impl Frame<'_> {
    /// Text space → device pixels when type is snapped to the pixel grid (Type anti-aliasing).
    pub(crate) fn text_snap(&self, t: &TextObject) -> Option<Affine> {
        (self.opts.anti_alias == AntiAlias::Type).then(|| self.view * t.xf)
    }

    /// A render context for drawing this frame's art offscreen (on the calling thread), with its
    /// edge smoothing: hard edges must not depend on where the art is drawn.
    pub(crate) fn offscreen_context(&self, w: u16, h: u16) -> RenderContext {
        #[cfg(test)]
        OFFSCREEN_PIXELS.with(|n| n.set(n.get() + u64::from(w) * u64::from(h)));
        let mut ctx = single_threaded_context(w, h);
        ctx.set_aliasing_threshold(self.opts.anti_alias.threshold());
        ctx
    }
}

#[cfg(test)]
thread_local! {
    /// The pixels of the offscreen contexts [`Frame::offscreen_context`] has made on this thread.
    pub(crate) static OFFSCREEN_PIXELS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
