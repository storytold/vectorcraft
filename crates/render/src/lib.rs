//! VectorCraft renderer: document → premultiplied RGBA pixels.
//!
//! The backend is `vello_cpu` (SIMD, sparse strips). Callers give a *view transform* mapping
//! document points to output pixels; the renderer culls by bounds, evaluates appearance stacks
//! (multiple fills/strokes, opacity, blend modes, stroke alignment, dashes), clip groups,
//! gradients, images and text (via `vectorcraft-text` glyph outlines).
#![forbid(unsafe_code)]

mod brush_fx;
mod changed;
mod dither;
pub mod encode;
mod freeform;
mod fx;
mod group;
mod images;
pub use images::{image_cached, keep_decoded_image};
mod ink;
mod live;
mod lru;
mod paint;
pub use paint::{decode_failures, image_decodes, reduced_size};
mod pattern;
pub mod placed_document;
pub mod proof;

use std::collections::HashMap;
use std::sync::Arc;

use vectorcraft_doc::{AppearanceItem, Document, Node, NodeId, NodeKind, StrokeAlign, StrokeLayer, SymbolBoxes, TextObject, TraceView};
use vectorcraft_geom::{Affine, BezPath, FillRule, Rect, Shape};
use vello_cpu::kurbo;
use vello_cpu::peniko::{self, BlendMode, Compose, Mix};
use vello_cpu::{Pixmap, RenderContext, Resources};

use group::Composite;
use ink::Ink;
use lru::FrameCache;

pub use brush_fx::instance_art;
pub use changed::{Changed, changed, placed_area, view_pixels};
pub use effects::stroke::width_outline;
pub use images::{IMAGE_CACHE_MAX, image_cache_bytes, newly_undecodable, set_image_cache_budget};
pub use live::expand_live;
pub use pattern::render_pattern_swatch;
pub use vectorcraft_effects as effects;
pub use vello_cpu;

/// Reusable renderer (keeps the render context, geometry and glyph caches between frames; decoded
/// images are shared by all renderers, see [`set_image_cache_budget`]).
pub struct Renderer {
    /// Glyph geometry per text object, keyed by `Arc` identity like `geom` (see [`Self::text_geom_of`]).
    texts: FrameCache<usize, (Arc<Node>, Arc<TextGeom>)>,
    /// The [`vectorcraft_text::FontDb::generation`] `texts` was laid out with: type lays out
    /// again when fonts are added or rescanned.
    text_fonts: u64,
    /// Clip regions per clipping path (see [`Self::clip_of`]).
    clips: FrameCache<usize, ClipEntry>,
    /// Worker threads for the multithreaded rasterizer (0 = single-threaded).
    pub threads: u16,
    geom: PtrMap<usize, GeomEntry>,
    /// The symbols (name and art) of the document last drawn, which `geom`'s bounds measured
    /// symbol instances by (see [`Self::check_symbols`]). The art is held weakly: the reference
    /// keeps the art's allocation, so no other art can get its address, and does not keep the
    /// art's content.
    symbols: Vec<(String, std::sync::Weak<Node>)>,
    /// The plug-in revision ([`effects::plugin_revision`]) that `geom`, `fx_geom` and `fx_arts` were
    /// last checked against (see [`Self::check_plugins`]).
    plugin_revision: u64,
    /// Expanded stroke outlines keyed by ([`Self::stroke_owner`], stroke layer address, width bits,
    /// tolerance level): panning re-renders reuse them instead of re-expanding every stroke.
    strokes: PtrMap<StrokeKey, StrokeEntry>,
    /// Expanded character stroke outlines, with the glyph geometry kept in `texts` they belong to,
    /// keyed by ([`Self::stroke_owner`] of that geometry, run, width bits, tolerance level): type
    /// that doesn't change draws them again instead of expanding its strokes in every frame. Within
    /// a byte budget, and swept by frame, so the outlines of type that changes in every frame go
    /// within a few frames.
    char_strokes: FrameCache<StrokeKey, (Arc<TextGeom>, Arc<BezPath>)>,
    /// The node being drawn on the plain-path fast path, or the path of brush art being drawn
    /// ([`Self::draw_brush_pieces`]): the owner of cached stroke outlines.
    cur: Option<Arc<Node>>,
    /// Whether the art being drawn is made again in every frame: symbol art that
    /// [`Self::placed_art`] did not keep, evaluations of live objects and effect art that their
    /// caches did not keep, and brush art that the brush cache did not keep.
    made_again: bool,
    /// The node `draw_shape_fx` may cache the live-effect geometry of (the one `draw_arc` is drawing).
    /// In Outline view its path comes from `geom` too ([`Self::outline_path`]), and
    /// [`Self::draw_outline_member`] sets it to each member of a clip group or shape of an Image
    /// Trace object that it draws.
    fx_owner: Option<Arc<Node>>,
    /// The live object [`Self::draw_kept_node`] is drawing (kept symbol art, an opacity mask's art,
    /// a member of a clip group in Outline view, kept brush art): [`Self::draw_content`] draws its
    /// evaluation from the live cache.
    live_root: Option<Arc<Node>>,
    /// Live-effect geometry per node ([`fx::FxGeom`]), keyed by `Arc` identity like `geom`.
    fx_geom: PtrMap<usize, fx::FxGeom>,
    stamp: u64,
    /// Opacity folded into paint alpha for the leaf being drawn (avoids a compositing layer).
    alpha: f32,
    /// The contexts kept for the next frames, with the threads they draw on, the last used last: a
    /// frame of the size of one reuses it instead of making a context and its threads. Two are
    /// kept, so a view's frames and the parts of them drawn again ([`changed`]) each keep theirs.
    contexts: Vec<(u16, RenderContext)>,
    resources: Resources,
    /// Statistics of the last frame.
    pub stats: FrameStats,
    /// Brush art per brushed stroke.
    brushes: brush_fx::BrushCache,
    /// Evaluated blends/envelopes and tessellated meshes.
    live: live::LiveCache,
    /// Blurred, tinted drop shadow / outer glow rasters per object and effect (see `fx`).
    shadows: PtrMap<(usize, usize, Ink), fx::ShadowEntry>,
    /// Objects' content run through Photoshop-style effects, per object and content slot (see
    /// `fx`): kept only while drawn.
    pixel_fx: PtrMap<(usize, usize, Ink), fx::PixelEntry>,
    /// What the contents of those rasters read from the document, and the color settings, as last
    /// found for a frame (`stamp`) and document (address) (see [`Self::doc_inputs`]).
    doc_inputs: Option<((u64, usize), Arc<fx::DocInputs>)>,
    /// Whether a document's layers, symbol art and pattern art show transparency, as last found
    /// (see [`Self::blends_in_cmyk`]), with that art in order, held weakly: a weak reference keeps
    /// the allocation, so no other art gets one of those addresses while the answer is kept.
    transparency: Option<(Vec<std::sync::Weak<Node>>, bool)>,
    /// Whether the group being drawn is a knockout group (what its neutral children inherit).
    knockout: bool,
    /// Address of the knockout-group element being drawn as its knockout shape: at full object
    /// opacity and without its opacity mask (see [`Self::draw_knockout`]); 0 = none.
    shape_of: usize,
    /// The art drawn for objects with effects that apply through it (see `fx::has_object_fx`).
    fx_arts: FrameCache<usize, FxArtEntry>,
    /// Symbol art as instances and inline graphics draw it (see [`Self::placed_art`]).
    placed_arts: FrameCache<PlacedKey, PlacedEntry>,
    /// The bitmap clock of [`placed_document`] when this frame first drew a placed document from
    /// a bitmap: the bitmaps drawn since then are not dropped to make room for those it asks for.
    placed_frame: Option<u64>,
    /// Layers open around the drawing point in the current context: a non-isolated group can copy
    /// its backdrop only when there are none (see [`Self::group`]).
    nested: u32,
    /// Copy of the backdrop of the non-isolated group being drawn offscreen, in its context's
    /// pixels: the elements of a non-isolated knockout group composite against it.
    backdrop: Option<Arc<Pixmap>>,
    /// [`Node::blends_through`] per container, keyed like `geom`.
    blends: PtrMap<usize, (Arc<Node>, bool, u64)>,
    /// Clip paths pushed on the current context (path, rule, transform): starting the context
    /// again from a picture of it pushes them again (see [`Self::group`]).
    clip_paths: Vec<(BezPath, FillRule, Affine)>,
    /// The picture [`Self::flatten`] started the current context again from, with 8-bit
    /// compositing: [`Self::finish`] renders the context over it.
    picture: Option<Arc<Pixmap>>,
    /// Pixels of the groups being drawn offscreen (contexts and backdrop copies), and the most
    /// they may hold (set from the frame when the first opens, see [`Self::group`]); both reset
    /// at the start of every frame.
    offscreen_live: u64,
    offscreen_budget: u64,
    /// Objects the symbol instances drawn in this frame held, together ([`vectorcraft_doc::MAX_SYMBOL_NODES`]).
    symbol_nodes: u64,
    /// Groups drawn twice (blending with their backdrop) that hold what is being drawn.
    twice_drawn: u32,
    /// What clipping paths paint, per clipping path (see [`Self::clip_paint_of`]).
    clip_paints: FrameCache<usize, ClipPaintEntry>,
    /// Gradients along or across strokes (see [`Self::fill_path_gradient`]), keyed by
    /// ([`Self::stroke_owner`], stroke layer address, tolerance level).
    stroke_slices: PtrMap<(usize, usize, i32), SliceEntry>,
    /// What the image keys colour adjustments give their recoloured images name: the image
    /// recoloured and the adjustment, with its hash (see [`Self::adjusted_image`]).
    adjusts: HashMap<String, (String, u64, vectorcraft_effects::ColorMap)>,
    /// Layer Options → Dim Images to, of the layer being drawn (screen views only): images show
    /// faded to this opacity over white.
    dim_images: Option<f32>,
    /// Inline graphics being drawn inside inline graphics (a symbol whose art holds text showing
    /// it): drawing stops at [`MAX_INLINE_DEPTH`].
    inline_depth: u32,
    /// How the frame being drawn is rasterized ([`RenderOptions::precise`]): its offscreen groups,
    /// masks and patterns too.
    raster: vello_cpu::RasterizerSettings,
    /// [`RenderOptions::isolated`]'s layers and groups, from the top layer down to it, for the
    /// frame (`stamp`) and document they were found in.
    isolation: Option<(u64, usize, NodeId, Vec<NodeId>)>,
    /// Drawing inside the isolated container or a dimmed object: nothing further dims.
    isolation_settled: bool,
    /// The art the last frame drew with Overprint Preview or Separations Preview: the next frame
    /// draws it again for the objects that have not changed ([`proof::prepare`]).
    proof_art: Option<proof::Prepared>,
    /// Width of [`Self::hairline`] in the pixels being drawn: 1, or the supersampling factor while
    /// a supersampled render draws its larger frame, so outlines stay one output pixel wide. Type
    /// outlines and dotted guides scale by it too.
    hairline: f64,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
use tests::fontchange as tests_fontchange;

mod blobs;
mod bounds;
mod clock;
mod frame;
mod geometry;
mod node_checks;
mod options;
mod renderer;
mod text_geom;
mod threads;

pub use bounds::{drawn_bounds, drawn_outline, effect_bounds, painted_bounds};
pub use frame::FrameStats;
use frame::{ClipEntry, ClipPaintEntry, Frame, FxArtEntry, GeomEntry, PlacedEntry, PlacedKey, PtrHasher, PtrMap, SliceEntry, StrokeEntry, StrokeKey};
#[cfg(test)]
use frame::{OFFSCREEN_PIXELS, Slices};
pub use geometry::region_pixels;
use node_checks::holds_plugin_effect;
#[cfg(test)]
use node_checks::painted_items;
pub use options::{AntiAlias, ISOLATION_DIM, MAX_RASTER_PIXELS, MAX_RASTER_SIDE, RenderOptions, Rendered, raster_size, supersample_factor};
pub(crate) use paint::blend_mode;
use paint::fill_rule;
use text_geom::TextGeom;
#[cfg(test)]
pub(crate) use text_geom::is_opentype_alternate;
#[cfg(test)]
use text_geom::text_geom;
pub(crate) use threads::single_threaded_context;
pub use threads::{default_threads, set_default_threads};
#[cfg(test)]
mod tests;
