//! VectorCraft renderer: document → premultiplied RGBA pixels.
//!
//! The backend is `vello_cpu` (SIMD, sparse strips). Callers give a *view transform* mapping
//! document points to output pixels; the renderer culls by bounds, evaluates appearance stacks
//! (multiple fills/strokes, opacity, blend modes, stroke alignment, dashes), clip groups,
//! gradients, images and text (via `vectorcraft-text` glyph outlines).
#![forbid(unsafe_code)]

mod brush_fx;
pub mod encode;
mod freeform;
mod fx;
mod group;
mod images;
mod ink;
mod live;
mod lru;
mod paint;
pub use paint::decode_failures;
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
pub use effects::stroke::width_outline;
pub use images::{IMAGE_CACHE_MAX, image_cache_bytes, set_image_cache_budget};
pub use live::expand_live;
pub use pattern::render_pattern_swatch;
pub use vectorcraft_effects as effects;
pub use vello_cpu;

/// Bounds of everything `n` paints, as the renderer culls it: its members, strokes, brush art and
/// geometry effects, and the shadows and glows of it and its members. Symbol instances are
/// measured by their art in `doc` ([`Node::visual_bounds_with`]).
pub fn painted_bounds(doc: &Document, n: &Node) -> Option<Rect> {
    brush_fx::cull_bounds(&doc.symbol_boxes(), n)
}

/// [`painted_bounds`] of `n` without the room it leaves for brush art past the brushed strokes of
/// `n` and of a symbol instance's art (the brush art of the objects in a blend, envelope or repeat
/// still counts). A group or layer without effects of its own is measured by its members, leaving
/// out the transparent source art of a Shaper group as [`Node::visual_bounds_with`] does. Symbol
/// instances are measured by their art in `symbols`.
pub fn effect_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    match &n.kind {
        NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } if !fx::has_object_fx(n) => children
            .iter()
            .skip(usize::from(n.extras().shaper.is_some()))
            .fold(None, |acc, c| vectorcraft_geom::union_opt(acc, effect_bounds(symbols, c))),
        _ => fx::cull_bounds(symbols, n),
    }
}

/// Bounds of what `n` draws with its own and its members' geometry effects applied and with the
/// reach of its own and its members' raster effects, measured as [`Node::visual_bounds_with`]
/// measures art without effects: what Object › Rasterize makes its image over.
pub fn drawn_bounds(doc: &Document, n: &Node) -> Option<Rect> {
    fx::painted_reach(&doc.symbol_boxes(), n)
}

/// The outline, with `n`'s geometry effects applied, that the canvas clips `n`'s Rasterize with
/// Create Clipping Mask to. Object › Rasterize's clipping mask follows it.
pub fn drawn_outline(doc: &Document, n: &Node) -> Option<(BezPath, FillRule)> {
    fx::drawn_outline(doc, n)
}

/// Largest raster an export may ask [`Renderer::render_region`] for, per side. The CPU rasteriser
/// addresses at most `u16::MAX` pixels per side (and panics at that edge).
pub const MAX_RASTER_SIDE: u32 = 32_768;
/// Largest raster an export may ask for in total (16384², 1 GiB of RGBA pixels).
pub const MAX_RASTER_PIXELS: u64 = 1 << 28;

/// Pixel size of `region` rendered at `scale` (as [`Renderer::render_region`] rounds it), or an error
/// when it exceeds [`MAX_RASTER_SIDE`] / [`MAX_RASTER_PIXELS`]. Raster exports check this first,
/// because the renderer can only clamp (wrong picture) or fail (abort) at those sizes.
pub fn raster_size(region: Rect, scale: f64) -> Result<(u32, u32), String> {
    if !(scale.is_finite() && scale > 0.0) {
        return Err(format!("invalid scale {scale}"));
    }
    let (w, h) = region_pixels(region, scale);
    if !(w <= MAX_RASTER_SIDE && h <= MAX_RASTER_SIDE && u64::from(w) * u64::from(h) <= MAX_RASTER_PIXELS) {
        return Err(format!(
            "the image would be {w} × {h} pixels; raster exports are limited to {MAX_RASTER_SIDE} pixels per side and {} megapixels: lower the scale or resolution",
            MAX_RASTER_PIXELS / 1_000_000
        ));
    }
    Ok((w, h))
}

/// Rendering options.
#[derive(Clone, Debug)]
pub struct RenderOptions {
    /// Outline (wireframe) view: 1 px black paths, no paint.
    pub outline: bool,
    /// Pasteboard colour behind everything (premultiplied RGBA8); `None` = transparent.
    pub background: Option<[u8; 4]>,
    /// Draw artboards as white rectangles (screen view). Export sets this to false.
    pub artboards: bool,
    /// Objects not to draw (e.g. the object being edited by a live drag preview).
    pub hidden: Vec<NodeId>,
    /// Draw the layers as their Layer Options show them on screen: Preview off in outline, Dim
    /// Images fading their images (a template layer's too), a template in Preview in Outline view.
    pub dim_templates: bool,
    /// Soft proof / separations preview (see [`proof`]).
    pub proof: Option<proof::ProofSetup>,
    /// Overprint Preview (see [`proof`] for what overprints).
    pub overprint_preview: bool,
    /// Trim View: clip the artwork to the artboards (nothing on the pasteboard is drawn).
    pub trim: bool,
    /// Leave template layers and guides out (exports and thumbnails: they're aids to drawing, not
    /// artwork).
    pub skip_templates: bool,
    /// View › Hide Template: template layers left out of the screen view (guides stay).
    pub hide_templates: bool,
    /// Pattern editing mode's tile edge and swatch bounds colour (Object → Pattern → Tile Edge
    /// Color), RGB.
    pub tile_edge: [u8; 3],
    /// Colour and style of the paths made into guides (Preferences › Guides & Grid, as the ruler
    /// guides the UI draws): RGB, and whether they are dotted.
    pub guide: ([u8; 3], bool),
    /// View Opacity Mask (Alt-click the mask thumbnail): instead of the artwork, show this object's
    /// opacity mask alone as its coverage in greyscale (white = opaque, black = transparent).
    pub mask_view: Option<NodeId>,
    /// Screen view: highlight substituted fonts and glyphs as Document Setup asks.
    pub highlight_substitutions: bool,
    /// Screen view: Preferences › Type › Highlight Alternate Glyphs. Pink behind glyphs OpenType
    /// features substituted (ligatures, contextual forms), not Document Setup's missing-glyph
    /// highlights.
    pub highlight_alternate_glyphs: bool,
    /// Edge smoothing (raster export option).
    pub anti_alias: AntiAlias,
    /// Screen view: placed documents draw from cached bitmaps made in the background (see
    /// [`placed_document`]); off, they draw exactly, read when needed.
    pub progressive_placed: bool,
    /// Screen view: Image Trace objects draw as their View asks (outlines, the source image…);
    /// off, they draw their tracing result, as exports and printing do.
    pub trace_views: bool,
    /// Images are sampled smoothly when scaled or rotated; off, each pixel takes its nearest image
    /// pixel (Pixel Preview with File Handling › Display Bitmaps as Anti-aliased Images off).
    pub smooth_images: bool,
    /// Screen view in isolation mode: the isolated group or layer. Everything around it draws
    /// dimmed ([`ISOLATION_DIM`]).
    pub isolated: Option<NodeId>,
    /// Composite in floating point (exports): translucent art over opaque art comes out exactly
    /// opaque, where 8-bit compositing leaves alpha 254 on some anti-aliased edges (#787). Off,
    /// the faster 8-bit pipeline (the canvas).
    pub precise: bool,
    /// Exports: Art Optimized edges are supersampled ([`supersample_factor`]): the art is drawn
    /// with hard edges at several times the size and averaged down, so shapes that meet edge to
    /// edge leave no seam of the background between them (#983). Off, edges are smoothed shape by
    /// shape (the canvas).
    pub supersample: bool,
    /// Screen views: an image drawn at half its size or smaller samples a copy shrunk by halves
    /// (box-filtered from the full image, kept with the decoded images), so the full image needn't
    /// stay in memory or be sampled. Off, images always sample their full pixels (exports).
    pub image_mips: bool,
    /// Screen views while an interaction is in progress (a drag, a dialog's preview): an object
    /// whose content only moved since an earlier frame draws its cached drop shadows, outer glows
    /// and filtered content from the rasters made then, at the nearest whole pixel, without
    /// running the effects again (see `fx`). Off, no raster made that way, or made over one, is
    /// drawn: the effects run again.
    pub interacting: bool,
    /// Screen views during a tool's drag on the canvas: the renderer draws filtered content within
    /// `fx::DRAG_PIXEL_BUDGET` pixels, at a lower resolution where it would need more, unless it
    /// can draw the content from a raster kept from an earlier frame. With [`Renderer::threads`]
    /// above 0 it does the same for drop shadows, outer glows and inner glows. No raster made that
    /// way is kept (see `fx`). Off, they render as in a still frame.
    pub dragging: bool,
    /// Appearance of Black's Rich Black: pure CMYK black (black ink alone at 100%) shows as RGB
    /// black. Off, it shows through the CMYK profile, a dark grey, as other colours do.
    pub rich_black: bool,
}

/// The opacity of the art around an isolated group or layer.
pub const ISOLATION_DIM: f32 = 0.5;

/// How edges are rasterized (raster export option).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AntiAlias {
    /// Hard edges: a pixel is painted when a path, text or image covers at least half of it.
    /// Raster effects (blur, shadows, glows) and pattern tiles stay smooth.
    None,
    /// Smooth edges everywhere.
    #[default]
    Art,
    /// Smooth edges with type snapped to the pixel grid (see `TextLayout::snap_to_pixels`):
    /// crisper small text.
    Type,
}

impl AntiAlias {
    pub const ALL: [Self; 3] = [Self::None, Self::Art, Self::Type];

    /// The id used in command params (`none`, `art`, `type`).
    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Art => "art",
            Self::Type => "type",
        }
    }

    /// The mode an id names (any case).
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id().eq_ignore_ascii_case(id))
    }

    /// The name shown in options dialogs.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Art => "Art Optimized",
            Self::Type => "Type Optimized",
        }
    }

    /// vello's aliasing threshold: paint a pixel only above half coverage when edges are hard.
    fn threshold(self) -> Option<u8> {
        (self == Self::None).then_some(127)
    }
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            outline: false,
            background: None,
            artboards: false,
            hidden: vec![],
            dim_templates: true,
            proof: None,
            overprint_preview: false,
            trim: false,
            skip_templates: false,
            hide_templates: false,
            tile_edge: vectorcraft_doc::LAYER_COLORS[0].1,
            guide: ([0x4a, 0xd8, 0xff], false),
            mask_view: None,
            highlight_substitutions: false,
            highlight_alternate_glyphs: false,
            anti_alias: AntiAlias::Art,
            progressive_placed: false,
            trace_views: false,
            smooth_images: true,
            isolated: None,
            precise: false,
            supersample: false,
            image_mips: false,
            interacting: false,
            dragging: false,
            rich_black: false,
        }
    }
}

/// The most pixels a supersampled render draws (the frame and its compositing buffers hold a few
/// times this many bytes).
const MAX_SUPERSAMPLED_PIXELS: u64 = 40_000_000;

/// How many times wider and taller a `w`×`h` render is drawn when supersampled: 4, or less where
/// that would pass [`MAX_SUPERSAMPLED_PIXELS`] or the renderer's 65,535-pixel side; `None` when not
/// even twice fits (the edges are then smoothed shape by shape).
pub fn supersample_factor(w: u32, h: u32) -> Option<u32> {
    (2..=4u32).rev().find(|k| {
        let (sw, sh) = (u64::from(w) * u64::from(*k), u64::from(h) * u64::from(*k));
        sw <= u64::from(u16::MAX) && sh <= u64::from(u16::MAX) && sw * sh <= MAX_SUPERSAMPLED_PIXELS
    })
}

/// Average each `k`×`k` block of `src` (`w`×`h` pixels of 4 premultiplied bytes) into one pixel.
fn downsample(src: &[u8], w: usize, h: usize, k: usize) -> Vec<u8> {
    let (ow, oh) = (w / k.max(1), h / k.max(1));
    let n = (k * k).max(1) as u32;
    let mut out = vec![0u8; ow * oh * 4];
    for (o, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (ox, oy) = (o % ow.max(1), o / ow.max(1));
        let mut acc = [0u32; 4];
        for y in oy * k..(oy + 1) * k {
            let row = y * w + ox * k;
            for p in src.get(row * 4..(row + k) * 4).unwrap_or_default().as_chunks::<4>().0 {
                for (a, v) in acc.iter_mut().zip(p) {
                    *a += u32::from(*v);
                }
            }
        }
        for (d, a) in px.iter_mut().zip(acc) {
            *d = ((a + n / 2) / n).min(255) as u8;
        }
    }
    out
}

/// A rendered image (premultiplied RGBA8, row-major).
pub struct Rendered {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Rendered {
    /// Un-premultiplied RGBA8 copy.
    pub fn to_straight(&self) -> Vec<u8> {
        let mut out = self.pixels.clone();
        for px in out.as_chunks_mut::<4>().0 {
            let a = px[3] as u32;
            if a != 0 && a != 255 {
                for c in &mut px[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        out
    }
    /// Encode as PNG without metadata (thumbnails, screenshots, embedded rasters; exports use
    /// [`encode`]).
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        let img = image::RgbaImage::from_raw(self.width, self.height, self.to_straight())
            .ok_or("PNG encoding failed: pixel buffer doesn't match the image size")?;
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).map_err(|e| format!("PNG encoding failed: {e}"))?;
        Ok(buf)
    }
    /// Encode as JPEG (flattened on white) at `quality` 1..=100.
    pub fn to_jpeg(&self, quality: u8) -> Result<Vec<u8>, String> {
        encode::jpeg(self, quality, None)
    }
    /// Encode as lossless WebP.
    pub fn to_webp(&self) -> Result<Vec<u8>, String> {
        encode::webp(self)
    }
    /// Straight-alpha RGBA at (x, y).
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        let p = &self.pixels[i..i + 4];
        let a = p[3] as u32;
        if a == 0 {
            return [0, 0, 0, 0];
        }
        let un = |c: u8| ((c as u32 * 255 + a / 2) / a).min(255) as u8;
        [un(p[0]), un(p[1]), un(p[2]), p[3]]
    }
}

/// Hasher for pointer-keyed caches: a multiply-xorshift (SipHash showed up in frame profiles).
#[derive(Default, Clone, Copy)]
struct PtrHasher(u64);

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

type PtrMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<PtrHasher>>;

/// A stroke expanded to a fill outline: (owning node, outline, last frame used).
struct StrokeEntry {
    node: Arc<Node>,
    outline: Arc<BezPath>,
    stamp: u64,
}

/// The slices of a stroke whose gradient runs along or across it, each with its paint and bounds.
type Slices = Arc<Vec<(BezPath, vectorcraft_color::Paint, Rect)>>;

/// [`Slices`] cached for a stroke: (owning node, slices, last frame used).
struct SliceEntry {
    node: Arc<Node>,
    slices: Slices,
    stamp: u64,
}

/// Cached per-node geometry, keyed by `Arc` identity. Structural sharing means an unchanged node
/// keeps its allocation across edits, so a pointer match (with the Arc kept alive here so the
/// address can't be reused) is an exact cache hit — no invalidation logic needed.
struct GeomEntry {
    node: Arc<Node>,
    bounds: Option<Rect>,
    path: Option<Arc<BezPath>>,
    stamp: u64,
}

/// A clipping path (kept alive so its address can't be reused) and the region it clips to.
type ClipEntry = (Arc<Node>, Option<Arc<(BezPath, FillRule)>>);
/// A clipping path (kept alive) and what it paints: its fills and its strokes ([`Node::clip_paint`]).
type ClipPaintEntry = (Arc<Node>, Option<Arc<Node>>, Option<Arc<Node>>);
/// An object with effects (kept alive), the art drawn for it ([`Renderer::fx_art`]), and, when that
/// art can be made from evaluated live objects (`fx::evaluates_live`) or from type, symbol art or
/// colors shaded through the color settings (`fx::reads_outside`), the inputs it was made with
/// ([`live::Found`]).
type FxArtEntry = (Arc<Node>, Arc<Node>, Option<live::Found>);
/// What symbol art placed by [`Renderer::placed_art`] is kept under: the art's address, the bits
/// of the transform's coefficients, whether strokes scale with it, and the stain ([`stain_bits`]).
type PlacedKey = (usize, [u64; 6], bool, [u32; 6]);

/// Symbol art placed by [`Renderer::placed_art`].
struct PlacedEntry {
    /// The symbol's art, held weakly: the reference keeps the art's allocation, so no other art can
    /// get its address while the entry exists, and does not keep the art's content.
    art: std::sync::Weak<Node>,
    /// The art placed, or `None` for art placed again in every frame ([`depends_on_document`]).
    placed: Option<Arc<Node>>,
    /// The font generation ([`vectorcraft_text::FontDb::generation`]) the art was placed in, when
    /// it holds type: the art is placed again when the fonts change.
    fonts: Option<u64>,
}

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
    /// Context reused when rendering single-threaded (`threads == 0`).
    ctx_st: Option<RenderContext>,
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
    strokes: PtrMap<(usize, usize, u64, i32), StrokeEntry>,
    /// The node being drawn on the plain-path fast path (owner of cached stroke outlines).
    cur: Option<Arc<Node>>,
    /// Whether the art being drawn is made again in every frame: symbol art that
    /// [`Self::placed_art`] did not keep, and evaluations of live objects and effect art that their
    /// caches did not keep.
    made_again: bool,
    /// The node `draw_shape_fx` may cache the live-effect geometry of (the one `draw_arc` is drawing).
    /// In Outline view its path comes from `geom` too ([`Self::outline_path`]), and
    /// [`Self::draw_outline_member`] sets it to each member of a clip group or shape of an Image
    /// Trace object that it draws.
    fx_owner: Option<Arc<Node>>,
    /// Live-effect geometry per node ([`fx::FxGeom`]), keyed by `Arc` identity like `geom`.
    fx_geom: PtrMap<usize, fx::FxGeom>,
    stamp: u64,
    /// Opacity folded into paint alpha for the leaf being drawn (avoids a compositing layer).
    alpha: f32,
    ctx: Option<RenderContext>,
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
}

/// How deep inline graphics nest (text in a symbol shown inline in text…) before they draw nothing.
const MAX_INLINE_DEPTH: u32 = 4;

/// Set once a missing inline symbol has been logged (it would log every frame otherwise).
static MISSING_INLINE_LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub drawn: usize,
    pub culled: usize,
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

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
struct Frame<'a> {
    /// Drawing into a multithreaded context: vello filter layers must be rendered offscreen.
    mt: bool,
    doc: &'a Document,
    /// `doc`'s symbol art boxes, which culling measures symbol instances by ([`SymbolBoxes::quick`]).
    symbols: &'a SymbolBoxes<'a>,
    view: Affine,
    /// Visible region in document coordinates (for culling).
    visible: Rect,
    /// Size of one output pixel in document units.
    px: f64,
    opts: &'a RenderOptions,
    /// What colours paint as: screen colours, or one ink plane of a CMYK document (see [`ink`]).
    ink: Ink,
}

impl Frame<'_> {
    /// Text space → device pixels when type is snapped to the pixel grid (Type anti-aliasing).
    fn text_snap(&self, t: &TextObject) -> Option<Affine> {
        (self.opts.anti_alias == AntiAlias::Type).then(|| self.view * t.xf)
    }

    /// A render context for drawing this frame's art offscreen (on the calling thread), with its
    /// edge smoothing: hard edges must not depend on where the art is drawn.
    fn offscreen_context(&self, w: u16, h: u16) -> RenderContext {
        let mut ctx = single_threaded_context(w, h);
        ctx.set_aliasing_threshold(self.opts.anti_alias.threshold());
        ctx
    }
}

impl Renderer {
    pub fn new() -> Self {
        Self {
            texts: FrameCache::new(lru::TEXT_BUDGET, 1024),
            text_fonts: 0,
            clips: FrameCache::new(lru::CLIP_BUDGET, 256),
            ctx_st: None,
            threads: default_threads(),
            geom: PtrMap::default(),
            symbols: vec![],
            plugin_revision: 0,
            strokes: PtrMap::default(),
            cur: None,
            made_again: false,
            fx_owner: None,
            fx_geom: PtrMap::default(),
            stamp: 0,
            alpha: 1.0,
            ctx: None,
            resources: Resources::new(),
            stats: FrameStats::default(),
            brushes: Default::default(),
            shadows: PtrMap::default(),
            pixel_fx: PtrMap::default(),
            doc_inputs: None,
            transparency: None,
            live: live::LiveCache::default(),
            knockout: false,
            shape_of: 0,
            fx_arts: FrameCache::new(lru::FX_ART_BUDGET, 256),
            placed_arts: FrameCache::new(lru::PLACED_ART_BUDGET, 256),
            nested: 0,
            backdrop: None,
            blends: PtrMap::default(),
            clip_paths: vec![],
            picture: None,
            clip_paints: FrameCache::new(lru::CLIP_BUDGET, 256),
            stroke_slices: PtrMap::default(),
            adjusts: HashMap::new(),
            dim_images: None,
            inline_depth: 0,
            raster: vello_cpu::RasterizerSettings::default(),
            isolation: None,
            isolation_settled: false,
            proof_art: None,
        }
    }

    /// Render `doc` into a `width`×`height` image using `view` (document → pixel transform).
    pub fn render(&mut self, doc: &Document, width: u32, height: u32, view: Affine, opts: &RenderOptions) -> Rendered {
        self.render_as(doc, width, height, view, opts, false)
    }

    /// [`Self::render`], or with `inks` the ink amounts in the working CMYK space instead of
    /// screen colours (4 bytes a pixel, see [`Self::render_region_inks`]).
    fn render_as(&mut self, doc: &Document, width: u32, height: u32, view: Affine, opts: &RenderOptions, inks: bool) -> Rendered {
        let start = now();
        if opts.supersample && opts.anti_alias == AntiAlias::Art && opts.mask_view.is_none() {
            let (w, h) = (width.clamp(1, u16::MAX as u32), height.clamp(1, u16::MAX as u32));
            if let Some(k) = supersample_factor(w, h) {
                let hard = RenderOptions { anti_alias: AntiAlias::None, supersample: false, ..opts.clone() };
                let big = self.render_as(doc, w * k, h * k, Affine::scale(f64::from(k)) * view, &hard, inks);
                let pixels = downsample(&big.pixels, big.width as usize, big.height as usize, k as usize);
                self.stats.micros = now().saturating_sub(start);
                return Rendered { width: w, height: h, pixels };
            }
        }
        self.check_plugins();
        // The document's own symbols: proofing rewrites the colors and blend modes of their art.
        self.check_symbols(doc);
        let prepared = proof::prepare(doc, opts, &mut self.proof_art);
        let doc: &Document = &prepared;
        let w = width.clamp(1, u16::MAX as u32) as u16;
        let h = height.clamp(1, u16::MAX as u32) as u16;
        // Raster filters (drop shadow, glows, blur) are rendered offscreen on this thread when the
        // main context is multithreaded (see `fx::with_filters`).
        let threads = self.threads;
        self.stats = FrameStats::default();
        let inv = view.inverse();
        let visible = inv.transform_rect_bbox(Rect::new(0.0, 0.0, w as f64, h as f64));
        let px = 1.0 / view.determinant().abs().sqrt().max(1e-12);
        let screen = if opts.rich_black { Ink::RichBlack } else { Ink::Display };
        let symbols = doc.symbol_boxes().quick();
        let frame = Frame { mt: threads > 0, doc, symbols: &symbols, view, visible, px, opts, ink: screen };
        self.begin_frame();
        // View Opacity Mask: the mask's coverage in greyscale instead of the artwork.
        if let Some(m) = opts.mask_view.and_then(|id| doc.node(id)).and_then(|n| n.extras().mask.as_deref()) {
            let pixels = self.mask_values(&frame, m, w, h).into_iter().flat_map(|v| [v, v, v, 255]).collect();
            self.end_frame();
            self.stats.micros = now().saturating_sub(start);
            return Rendered { width: w as u32, height: h as u32, pixels };
        }
        let mut pixels = if inks || self.blends_in_cmyk(doc, opts) {
            // Blending in CMYK: one frame per ink plane, then their inks shown on screen.
            let cmy = self.draw_frame(&Frame { ink: Ink::Cmy, ..frame }, w, h);
            let stats = self.stats;
            let k = self.draw_frame(&Frame { ink: Ink::K, ..frame }, w, h);
            self.stats = stats;
            if inks {
                ink::amounts(&cmy, &k)
            } else {
                let (pixels, composed) = ink::compose(&cmy, &k, opts, threads);
                self.stats.ink_threads = composed;
                pixels
            }
        } else {
            self.draw_frame(&frame, w, h)
        };
        self.end_frame();
        if !inks {
            proof::post(&mut pixels, opts);
        }
        self.stats.micros = now().saturating_sub(start);
        Rendered { width: w as u32, height: h as u32, pixels }
    }

    /// Start a frame: cache entries used from now on are marked with its stamp.
    fn begin_frame(&mut self) {
        self.stamp += 1;
    }

    /// End a frame: drop cache entries not used for a few frames (every render, thumbnails too,
    /// so the nodes they hold go with them).
    fn end_frame(&mut self) {
        let g = self.stamp;
        if self.geom.len() > 1024 {
            self.geom.retain(|_, e| g - e.stamp <= 3);
        }
        if self.strokes.len() > 1024 {
            self.strokes.retain(|_, e| g - e.stamp <= 3);
        }
        if self.stroke_slices.len() > 256 {
            self.stroke_slices.retain(|_, e| g - e.stamp <= 3);
        }
        if self.shadows.len() > 256 {
            self.shadows.retain(|_, e| g - e.stamp <= 3);
        }
        if self.fx_geom.len() > 256 {
            self.fx_geom.retain(|_, e| g - e.stamp <= 3);
        }
        // Filtered rasters are large: only those of the last frame stay.
        self.pixel_fx.retain(|_, e| e.stamp == g);
        self.texts.sweep(g);
        self.clips.sweep(g);
        self.clip_paints.sweep(g);
        self.fx_arts.sweep(g);
        self.placed_arts.sweep(g);
        self.brushes.entries.sweep(g);
        self.live.sweep(g);
    }

    /// Draw frame `f` (`w`×`h` pixels): the background, the artboards and the page's art.
    fn draw_frame(&mut self, f: &Frame, w: u16, h: u16) -> Vec<u8> {
        let (threads, opts) = (self.threads, f.opts);
        let slot = if threads == 0 { self.ctx_st.take() } else { self.ctx.take() };
        let mut ctx = match slot {
            Some(mut c) if c.width() == w && c.height() == h => {
                c.reset();
                c
            }
            _ => RenderContext::new_with(w, h, vello_cpu::RenderSettings { num_threads: threads, ..Default::default() }),
        };
        // `reset` keeps the threshold of the previous render: set it every time.
        ctx.set_aliasing_threshold(opts.anti_alias.threshold());
        let render_mode = if opts.precise { vello_cpu::RenderMode::OptimizeQuality } else { vello_cpu::RenderMode::OptimizeSpeed };
        self.raster = vello_cpu::RasterizerSettings { render_mode, ..Default::default() };
        if let Some(bg) = opts.background {
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_paint(f.ink.fixed(bg));
            ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, w as f64, h as f64));
        }
        if opts.artboards && !opts.outline {
            // Paper: white on screen, no ink on the ink planes.
            ctx.set_transform(f.view);
            ctx.set_paint(peniko::Color::WHITE);
            for ab in &f.doc.artboards {
                ctx.fill_rect(&ab.rect);
            }
        }
        (self.nested, self.backdrop, self.picture) = (0, None, None);
        self.clip_paths.clear();
        if opts.trim && !f.doc.artboards.is_empty() {
            let mut clip = BezPath::new();
            for ab in &f.doc.artboards {
                clip.extend(ab.rect.path_elements(0.1));
            }
            let blends = self.children_blend(&f.doc.layers);
            let trim = Composite { clip: Some((&clip, FillRule::NonZero)), blends, ..Default::default() };
            self.group(&mut ctx, f, trim, &mut |r, c, fr| r.draw_page(c, fr));
        } else {
            self.draw_page(&mut ctx, f);
        }
        let pm = self.finish(&mut ctx, self.raster);
        if threads == 0 {
            self.ctx_st = Some(ctx);
        } else {
            self.ctx = Some(ctx);
        }
        pm.data_as_u8_slice().to_vec()
    }

    /// The page's art: the layers (or the pattern being edited), inside the page group when the
    /// page has one.
    fn draw_page(&mut self, ctx: &mut RenderContext, f: &Frame) {
        if self.draw_pattern_edit(ctx, f) {
            return;
        }
        let doc = f.doc;
        // While editing an opacity mask its art is seen only through the mask.
        let mask_layer = doc.mask_edit.map(|m| m.layer);
        let layers: Vec<Arc<Node>> = doc.layers.iter().filter(|l| Some(l.id) != mask_layer).cloned().collect();
        // Page Isolated Blending / Page Knockout Group: the page is a group of its own.
        let page_group = !f.opts.outline && (doc.page_isolate || doc.page_knockout);
        let knockout = page_group && doc.page_knockout;
        let mut draw = |r: &mut Self, c: &mut RenderContext, fr: &Frame| {
            r.knockout = knockout;
            r.draw_children(c, fr, &layers, knockout);
            r.knockout = false;
        };
        if page_group {
            let blends = self.children_blend(&layers);
            self.group(ctx, f, Composite { isolated: doc.page_isolate, blends, ..Default::default() }, &mut draw);
        } else {
            draw(self, ctx, f);
        }
    }

    /// Render one artboard (or any document rect) at `scale` pixels per point, transparent or on white,
    /// as exported: template layers and guides are left out.
    pub fn render_region(&mut self, doc: &Document, region: Rect, scale: f64, white: bool) -> Rendered {
        let opts = RenderOptions { background: white.then_some([255, 255, 255, 255]), skip_templates: true, ..Default::default() };
        self.render_region_with(doc, region, scale, &opts)
    }

    /// Render a document rect at `scale` pixels per point with `opts`.
    pub fn render_region_with(&mut self, doc: &Document, region: Rect, scale: f64, opts: &RenderOptions) -> Rendered {
        let (w, h) = region_pixels(region, scale);
        let view = Affine::scale(scale) * Affine::translate((-region.x0, -region.y0));
        self.render(doc, w, h, view, opts)
    }

    /// `region` of `doc` as ink amounts in the working CMYK space, 4 bytes a pixel (0 = no ink):
    /// drawn as the two ink planes of [`ink`], so CMYK colours keep their own inks (black type
    /// stays black ink alone) and RGB ones are separated with the colour settings. Where nothing
    /// is drawn (no background) there is no ink. CMYK exports draw this.
    pub fn render_region_inks(&mut self, doc: &Document, region: Rect, scale: f64, opts: &RenderOptions) -> Vec<u8> {
        let (w, h) = region_pixels(region, scale);
        let view = Affine::scale(scale) * Affine::translate((-region.x0, -region.y0));
        self.render_as(doc, w, h, view, opts, true).pixels
    }

    /// Render a single node (thumbnails, previews) fitted into `size`×`size` pixels.
    pub fn render_thumbnail(&mut self, doc: &Document, id: NodeId, size: u32) -> Option<Rendered> {
        self.render_node_thumbnail(doc, doc.node(id)?, size, None)
    }

    /// Render node `n` of `doc` (its resources) into `w`×`h` transparent pixels through `view`.
    pub fn render_node(&mut self, doc: &Document, n: &Arc<Node>, w: u16, h: u16, view: Affine) -> Option<Rendered> {
        self.check_plugins();
        self.check_symbols(doc);
        let inv = view.inverse();
        let visible = inv.transform_rect_bbox(Rect::new(0.0, 0.0, w as f64, h as f64));
        let px = 1.0 / view.determinant().abs().sqrt().max(1e-12);
        let mut ctx = single_threaded_context(w, h);
        let opts = RenderOptions::default();
        let symbols = doc.symbol_boxes().quick();
        let frame = Frame { mt: false, doc, symbols: &symbols, view, visible, px, opts: &opts, ink: Ink::Display };
        self.begin_frame();
        (self.knockout, self.nested, self.backdrop, self.picture) = (false, 0, None, None);
        self.clip_paths.clear();
        self.draw_arc(&mut ctx, &frame, n);
        let pm = self.finish(&mut ctx, vello_cpu::RasterizerSettings::default());
        self.end_frame();
        Some(Rendered { width: w as u32, height: h as u32, pixels: pm.data_as_u8_slice().to_vec() })
    }

    /// Render any node (also one outside the tree, e.g. opacity-mask art) fitted into
    /// `size`×`size` pixels, optionally over a solid premultiplied background. A symbol instance
    /// is fitted by its symbol's art in `doc`.
    pub fn render_node_thumbnail(&mut self, doc: &Document, n: &Node, size: u32, background: Option<[u8; 4]>) -> Option<Rendered> {
        self.render_node_thumbnail_with(doc, n, size, background, &RenderOptions::default())
    }

    /// [`Self::render_thumbnail`] with `opts` (e.g. [`RenderOptions::image_mips`]).
    pub fn render_thumbnail_with(&mut self, doc: &Document, id: NodeId, size: u32, opts: &RenderOptions) -> Option<Rendered> {
        self.render_node_thumbnail_with(doc, doc.node(id)?, size, None, opts)
    }

    /// [`Self::render_node_thumbnail`] with `opts` (e.g. [`RenderOptions::image_mips`]).
    pub fn render_node_thumbnail_with(
        &mut self,
        doc: &Document,
        n: &Node,
        size: u32,
        background: Option<[u8; 4]>,
        opts: &RenderOptions,
    ) -> Option<Rendered> {
        self.check_plugins();
        self.check_symbols(doc);
        let b = brush_fx::cull_bounds(&doc.symbol_boxes(), n)?;
        let s = (size as f64 - 2.0) / b.width().max(b.height()).max(1e-6);
        let view = Affine::translate((size as f64 / 2.0, size as f64 / 2.0)) * Affine::scale(s) * Affine::translate(-b.center().to_vec2());
        let w = size.clamp(1, u16::MAX as u32) as u16;
        let mut ctx = single_threaded_context(w, w);
        if let Some(bg) = background {
            ctx.set_paint(peniko::Color::from_rgba8(bg[0], bg[1], bg[2], bg[3]));
            ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, w as f64, w as f64));
        }
        let symbols = doc.symbol_boxes().quick();
        let frame = Frame { mt: false, doc, symbols: &symbols, view, visible: b.inflate(1.0, 1.0), px: 1.0 / s, opts, ink: Ink::Display };
        self.begin_frame();
        (self.knockout, self.nested, self.backdrop, self.picture) = (false, 0, None, None);
        self.clip_paths.clear();
        self.draw_node(&mut ctx, &frame, n, true);
        let pm = self.finish(&mut ctx, vello_cpu::RasterizerSettings::default());
        self.end_frame();
        Some(Rendered { width: size, height: size, pixels: pm.data_as_u8_slice().to_vec() })
    }

    /// Cached cull bounds of a node (containers union their cached children), symbol instances
    /// measured by their art box from `symbols`.
    fn bounds_of(&mut self, symbols: &SymbolBoxes, a: &Arc<Node>) -> Option<Rect> {
        let key = Arc::as_ptr(a) as usize;
        if let Some(e) = self.geom.get_mut(&key)
            && Arc::ptr_eq(&e.node, a)
        {
            e.stamp = self.stamp;
            return e.bounds;
        }
        let b = match &a.kind {
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } if !fx::has_object_fx(a) => {
                let mut acc: Option<Rect> = None;
                // An Image Trace object's hidden source image shows in some of its views.
                let source = a.extras().trace.is_some().then(|| children.first()).flatten();
                for c in children {
                    if c.visible || source.is_some_and(|s| Arc::ptr_eq(s, c)) {
                        acc = vectorcraft_geom::union_opt(acc, self.bounds_of(symbols, c));
                    }
                }
                acc
            }
            _ => brush_fx::cull_bounds(symbols, a),
        };
        self.geom.insert(key, GeomEntry { node: a.clone(), bounds: b, path: None, stamp: self.stamp });
        b
    }

    /// Drop the cached bounds of symbol instances and of the objects that hold them when `doc`'s
    /// symbols are not those of the document last drawn (by name and art): those bounds measured
    /// the instances by their symbol's art.
    fn check_symbols(&mut self, doc: &Document) {
        let same = self.symbols.len() == doc.symbols.len()
            && self.symbols.iter().zip(&doc.symbols).all(|((name, art), s)| *name == s.name && std::ptr::eq(art.as_ptr(), Arc::as_ptr(&s.art)));
        if !same {
            self.geom.retain(|_, e| !holds_instance(&e.node));
            self.symbols = doc.symbols.iter().map(|s| (s.name.clone(), Arc::downgrade(&s.art))).collect();
        }
    }

    /// When a plug-in has been installed or removed since the last check
    /// ([`effects::plugin_revision`]), drop what the caches keep from plug-in effects: the effect
    /// geometry of paths with a plug-in effect, the cull bounds of objects that have one or hold one,
    /// of symbol instances and of the objects that hold them, and the art drawn for the effects of
    /// objects with a plug-in effect. Rasters of raster effects and evaluations of live objects hold
    /// the revision in their keys when what they draw has a plug-in effect (`fx::Inputs`,
    /// `live::resources_key`).
    fn check_plugins(&mut self) {
        let revision = effects::plugin_revision();
        if revision == self.plugin_revision {
            return;
        }
        self.plugin_revision = revision;
        self.fx_geom.retain(|_, e| !effects::has_plugin_effect(&e.node));
        self.geom.retain(|_, e| !holds_plugin_effect(&e.node) && !holds_instance(&e.node));
        self.fx_arts.retain(|(node, ..)| !effects::has_plugin_effect(node));
    }

    /// Cached region a clip group's clipping path `a` clips to ([`effects::clip_outline`]):
    /// outlining text and uniting shapes is too slow to repeat every frame.
    fn clip_of(&mut self, a: &Arc<Node>) -> Option<Arc<(BezPath, FillRule)>> {
        let key = Arc::as_ptr(a) as usize;
        if let Some((node, region)) = self.clips.get(&key, self.stamp)
            && Arc::ptr_eq(node, a)
        {
            return region.clone();
        }
        let region = effects::clip_outline(a).map(Arc::new);
        let bytes = size_of::<ClipEntry>() + region.as_ref().map_or(0, |r| lru::path_bytes(&r.0));
        self.clips.insert(key, (a.clone(), region.clone()), bytes, self.stamp);
        region
    }

    /// Cached fills and strokes of clipping path `a` ([`Node::clip_paint`]): the parts keep their
    /// allocations between frames, so their own geometry stays cached.
    fn clip_paint_of(&mut self, a: &Arc<Node>) -> (Option<Arc<Node>>, Option<Arc<Node>>) {
        let key = Arc::as_ptr(a) as usize;
        if let Some((node, fill, stroke)) = self.clip_paints.get(&key, self.stamp)
            && Arc::ptr_eq(node, a)
        {
            return (fill.clone(), stroke.clone());
        }
        let paint = a.clip_paint();
        let (fill, stroke) = (paint.fill.map(Arc::new), paint.stroke.map(Arc::new));
        let bytes = size_of::<ClipPaintEntry>() + [&fill, &stroke].iter().filter_map(|p| p.as_deref()).map(lru::node_bytes).sum::<usize>();
        self.clip_paints.insert(key, (a.clone(), fill.clone(), stroke.clone()), bytes, self.stamp);
        (fill, stroke)
    }

    /// Cached BezPath of a path or compound path node (its bounds cached with it, see
    /// [`Self::bounds_of`]).
    fn path_of(&mut self, symbols: &SymbolBoxes, a: &Arc<Node>) -> Option<Arc<BezPath>> {
        let key = Arc::as_ptr(a) as usize;
        if let Some(e) = self.geom.get(&key)
            && Arc::ptr_eq(&e.node, a)
            && let Some(p) = &e.path
        {
            return Some(p.clone());
        }
        let p = Arc::new(a.stroke_path()?);
        let bounds = self.bounds_of(symbols, a);
        self.geom.insert(key, GeomEntry { node: a.clone(), bounds, path: Some(p.clone()), stamp: self.stamp });
        Some(p)
    }

    /// The cached path of path or compound path `n` ([`Self::path_of`]) when the frame is in
    /// Outline view and `n` is `fx_owner`, else `None`.
    fn outline_path(&mut self, f: &Frame, n: &Node) -> Option<Arc<BezPath>> {
        let owner = self.fx_owner.as_ref().filter(|o| f.opts.outline && std::ptr::eq(&***o, n)).cloned()?;
        self.path_of(f.symbols, &owner)
    }

    /// Whether `a` is left out of this frame: hidden, a skipped template or guide, or culled.
    fn skipped(&mut self, f: &Frame, a: &Arc<Node>) -> bool {
        let skipped_template = (f.opts.skip_templates || f.opts.hide_templates) && matches!(a.kind, NodeKind::Layer { template: true, .. })
            || f.opts.skip_templates && matches!(a.kind, NodeKind::Path { guide: true, .. });
        if !a.visible || skipped_template || f.opts.hidden.contains(&a.id) {
            return true;
        }
        match self.bounds_of(f.symbols, a) {
            Some(b) => {
                let pad = f.px * 2.0;
                // Level of detail: leaves smaller than a quarter pixel are invisible.
                let culled =
                    !rects_overlap(b.inflate(pad, pad), f.visible) || (!a.is_container() && b.width() < f.px * 0.25 && b.height() < f.px * 0.25);
                self.stats.culled += culled as usize;
                culled
            }
            None => !a.is_container(),
        }
    }

    fn draw_arc(&mut self, ctx: &mut RenderContext, f: &Frame, a: &Arc<Node>) {
        if self.skipped(f, a) {
            return;
        }
        if let Some(m) = a.extras().mask.as_deref()
            && !m.disabled
            && !f.opts.outline
            && !self.is_shape(a)
        {
            // A masked object is a non-isolated group of its own.
            let mask = Some(self.opacity_mask(f, m, ctx.width(), ctx.height()));
            let blends = a.blend != vectorcraft_color::BlendMode::Normal || (!a.isolate && self.blends_through(a));
            let bounds = if blends { self.bounds_of(f.symbols, a) } else { None };
            self.group(ctx, f, Composite { mask, blends, bounds, ..Default::default() }, &mut |r, c, fr| r.draw_arc_body(c, fr, a));
            return;
        }
        self.draw_arc_body(ctx, f, a);
    }

    /// Render an opacity mask's art offscreen and turn its luminance into a coverage mask.
    fn opacity_mask(&mut self, f: &Frame, m: &vectorcraft_doc::OpacityMask, w: u16, h: u16) -> vello_cpu::Mask {
        vello_cpu::Mask::from_parts(self.mask_values(f, m, w, h), w, h)
    }

    /// The coverage of opacity mask `m` per output pixel (row-major): its art rendered offscreen,
    /// as luminance ([`mask_value`]).
    fn mask_values(&mut self, f: &Frame, m: &vectorcraft_doc::OpacityMask, w: u16, h: u16) -> Vec<u8> {
        let mut mctx = f.offscreen_context(w, h);
        // Mask art is a picture of its own: it takes no part in a knockout group around the object,
        // and its luminance is that of its screen colours.
        let outer = (
            std::mem::take(&mut self.knockout),
            std::mem::take(&mut self.nested),
            self.backdrop.take(),
            std::mem::take(&mut self.clip_paths),
            self.picture.take(),
        );
        self.draw_node(&mut mctx, &Frame { mt: false, ink: Ink::Display, ..*f }, &m.art, true);
        let pm = self.finish(&mut mctx, self.raster);
        (self.knockout, self.nested, self.backdrop, self.clip_paths, self.picture) = outer;
        pm.data().iter().map(|p| mask_value(p.r, p.g, p.b, p.a, m.clip, m.invert)).collect()
    }

    /// Everything [`Self::draw_arc`] does after culling.
    fn draw_arc_body(&mut self, ctx: &mut RenderContext, f: &Frame, a: &Arc<Node>) {
        // Fast path for plain paths: cached geometry, opacity folded into the paint.
        // A blend mode on a single plain fill applies per draw (exact for one paint operation);
        // a blend layer would composite the whole viewport.
        let blended = a.blend != vectorcraft_color::BlendMode::Normal;
        if let NodeKind::Path { rule, guide: false, .. } = &a.kind
            && !f.opts.outline
            && (!blended || single_plain_fill(a))
            && !a.isolate
            && !fx::has_fx(a)
            && (self.opacity_of(a) >= 1.0 || painted_items(a) == 1)
            && let Some(bp) = self.path_of(f.symbols, a)
        {
            self.alpha = self.opacity_of(a).clamp(0.0, 1.0);
            self.cur = Some(a.clone());
            if blended {
                ctx.set_blend_mode(blend_mode(a.blend));
            }
            self.draw_shape(ctx, f, a, &bp, *rule);
            if blended {
                ctx.set_blend_mode(blend_mode(vectorcraft_color::BlendMode::Normal));
            }
            self.cur = None;
            self.alpha = 1.0;
            self.stats.drawn += 1;
            return;
        }
        if fx::has_object_fx(a) {
            return self.draw_object_fx(ctx, f, a, true);
        }
        if let NodeKind::Text(t) = &a.kind
            && self.opacity_of(a) >= 1.0
            && a.blend == vectorcraft_color::BlendMode::Normal
            && !fx::has_fx(a)
        {
            let t = &*f.doc.inline_resolved(t);
            let g = match f.text_snap(t) {
                Some(xf) => Arc::new(text_geom_snapped(t, Some(xf))),
                None => self.text_geom_of(a, t),
            };
            self.draw_text_geom(ctx, f, a, t, &g);
            self.stats.drawn += 1;
            return;
        }
        if vectorcraft_doc::live::is_live(a) || effects::has_pathfinder(a) {
            return self.draw_live(ctx, f, a);
        }
        // Its live-effect geometry depends on the node alone, and for plug-in effects on the
        // plug-ins installed (`check_plugins`), so it can be kept across frames.
        let outer = self.fx_owner.replace(a.clone());
        self.draw_node(ctx, f, a, true);
        self.fx_owner = outer;
    }

    fn draw_node(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, force: bool) {
        if !force && (!n.visible || f.opts.hidden.contains(&n.id)) {
            return;
        }
        // A cancelled export stops drawing; whoever asked for the picture drops it (see `cancel`).
        if vectorcraft_doc::cancel::is_cancelled() {
            return;
        }
        if force {
            // Bounds were already checked by draw_arc (or the caller wants it drawn regardless).
        } else if let Some(b) = fx::cull_bounds(f.symbols, n) {
            let pad = f.px * 2.0;
            if !rects_overlap(b.inflate(pad, pad), f.visible) {
                self.stats.culled += n.count();
                return;
            }
        } else if !n.is_container() {
            return;
        }
        // Layer Options on screen: a layer whose Preview is off draws in outline, and Dim Images
        // fades its images, a template's too (exports and thumbnails, which leave templates out,
        // ignore both).
        if let NodeKind::Layer { preview, dim_images, template, .. } = &n.kind
            && f.opts.dim_templates
            && !f.opts.skip_templates
        {
            // A template layer stays in Preview while the view is in Outline (and whatever its own
            // Preview option says): its picture is there to be traced (#1486).
            if *template && f.opts.outline {
                let opts = RenderOptions { outline: false, ..f.opts.clone() };
                return self.draw_node(ctx, &Frame { opts: &opts, ..*f }, n, force);
            }
            if !preview && !template && !f.opts.outline {
                let opts = RenderOptions { outline: true, ..f.opts.clone() };
                return self.draw_node(ctx, &Frame { opts: &opts, ..*f }, n, force);
            }
            if let Some(p) = *dim_images {
                let outer = self.dim_images;
                let dim = f32::from(p).clamp(0.0, 100.0) / 100.0;
                self.dim_images = Some(outer.map_or(dim, |o| o.min(dim)));
                self.draw_layer_node(ctx, f, n);
                self.dim_images = outer;
                return;
            }
        }
        self.draw_layer_node(ctx, f, n);
    }

    /// [`Self::draw_node`] after culling and Layer Options.
    fn draw_layer_node(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node) {
        // Isolation mode: the art around the isolated container draws dimmed (#833).
        if let Some((isolated, path)) = self.isolation_path(f) {
            // A container on the way down to it draws as usual: its members decide.
            if path.contains(&n.id) && n.id != isolated {
                return self.draw_layer_node_now(ctx, f, n);
            }
            self.isolation_settled = true;
            if n.id == isolated {
                self.draw_layer_node_now(ctx, f, n);
            } else {
                self.dimmed(ctx, f, &mut |r, c, fr| r.draw_layer_node_now(c, fr, n));
            }
            self.isolation_settled = false;
            return;
        }
        self.draw_layer_node_now(ctx, f, n);
    }

    /// In isolation mode, while drawing outside the isolated container and the art dimmed around
    /// it: the isolated container and its layers and groups, from the top layer down to it.
    fn isolation_path(&mut self, f: &Frame) -> Option<(NodeId, Vec<NodeId>)> {
        // Opacity-mask editing isolates the temporary layer for tools; that layer is not painted on
        // the page (the mask applies on the object), so canvas dimming would wash out everything.
        let isolated = f.opts.isolated.filter(|_| f.doc.mask_edit.is_none() && !self.isolation_settled)?;
        let doc = std::ptr::from_ref(f.doc) as usize;
        if self.isolation.as_ref().is_none_or(|(stamp, d, id, _)| (*stamp, *d, *id) != (self.stamp, doc, isolated)) {
            self.isolation = Some((self.stamp, doc, isolated, f.doc.ancestry(isolated).unwrap_or_default()));
        }
        let path = self.isolation.as_ref().map(|(.., path)| path.clone()).filter(|p| !p.is_empty())?;
        Some((isolated, path))
    }

    /// `draw` at [`ISOLATION_DIM`], with nothing inside dimmed again.
    fn dimmed(&mut self, ctx: &mut RenderContext, f: &Frame, draw: &mut group::Content) {
        let settled = std::mem::replace(&mut self.isolation_settled, true);
        let comp = Composite { opacity: ISOLATION_DIM, ..Default::default() };
        self.group(ctx, f, comp, draw);
        self.isolation_settled = settled;
    }

    /// [`Self::draw_layer_node`] past isolation mode's dimming.
    fn draw_layer_node_now(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node) {
        if fx::has_object_fx(n) {
            return self.draw_object_fx(ctx, f, &Arc::new(n.clone()), false);
        }
        let outline = f.opts.outline;
        let opacity = self.opacity_of(n);
        // Whether this container's children knock each other out.
        let knockout = !outline && n.knocks_out(self.knockout);
        let enclosing = std::mem::replace(&mut self.knockout, knockout);
        if !outline && (opacity < 1.0 || n.blend != vectorcraft_color::BlendMode::Normal || n.isolate || knockout) {
            let blends = self.blends_through(n);
            let bounds = if blends { self.node_bounds(f.symbols, n) } else { None };
            let comp = Composite { blend: n.blend, opacity, isolated: n.isolate, blends, bounds, ..Default::default() };
            self.group(ctx, f, comp, &mut |r, c, fr| r.draw_content(c, fr, n, knockout));
        } else {
            self.draw_content(ctx, f, n, knockout);
        }
        self.knockout = enclosing;
        self.stats.drawn += 1;
    }

    /// What `n` draws inside its transparency group (`knockout`: its children knock each other out).
    fn draw_content(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, knockout: bool) {
        match &n.kind {
            NodeKind::Group { children, clip: false } if f.opts.trace_views && !f.opts.outline && n.trace_view() != TraceView::Result => {
                self.draw_trace_view(ctx, f, children, n.trace_view(), knockout)
            }
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
                self.draw_children(ctx, f, children, knockout)
            }
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } if f.opts.outline => {
                for c in children {
                    self.draw_outline_member(ctx, f, c);
                }
            }
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                // Nothing to clip by hides the clipped art (as in the SVG and PDF output). A clip
                // group doesn't isolate: blending inside it reaches the art below. The clipping
                // path's fill paints behind the clipped art and its stroke over it, unclipped.
                if let Some((clip, rest)) = children.split_first()
                    && let Some(region) = self.clip_of(clip)
                {
                    let (fill, stroke) = self.clip_paint_of(clip);
                    // An image inside its own frame painted straight into the clipping path, without
                    // a clip layer (envelopes cut distorted images into many such pieces).
                    if let ([image], None, None) = (rest, &fill, &stroke)
                        && let NodeKind::Image(im) = &image.kind
                        && plain_image(image, im, &region.0)
                    {
                        self.draw_image_in(ctx, f, im, Some(&region));
                        self.stats.drawn += 1;
                        return;
                    }
                    let blends = self.blends_through(n);
                    let bounds = if blends { Some(region.0.bounding_box()) } else { None };
                    let comp = Composite { clip: Some((&region.0, region.1)), blends, bounds, ..Default::default() };
                    self.group(ctx, f, comp, &mut |r, c, fr| {
                        if let Some(fill) = &fill {
                            r.draw_arc(c, fr, fill);
                        }
                        r.draw_children(c, fr, rest, knockout)
                    });
                    if let Some(stroke) = &stroke {
                        self.draw_arc(ctx, f, stroke);
                    }
                }
            }
            NodeKind::Path { path, rule, guide, .. } => {
                let bp = self.outline_path(f, n).unwrap_or_else(|| Arc::new(path.to_bezpath()));
                if *guide {
                    let ([r, g, b], dots) = f.opts.guide;
                    if dots {
                        let mut screen = BezPath::clone(&bp);
                        screen.apply_affine(f.view);
                        ctx.set_transform(Affine::IDENTITY);
                        ctx.set_stroke(kurbo::Stroke::new(1.5).with_dashes(0.0, [1.5, 2.5]));
                        ctx.set_paint(f.ink.fixed([r, g, b, 255]));
                        ctx.stroke_path(&screen);
                    } else {
                        self.hairline(ctx, f, &bp, [r, g, b, 255]);
                    }
                } else {
                    self.draw_shape(ctx, f, n, &bp, *rule);
                }
            }
            NodeKind::Compound { children, rule } => {
                if let Some(bp) = self.outline_path(f, n) {
                    return self.draw_shape(ctx, f, n, &bp, *rule);
                }
                let mut bp = BezPath::new();
                for c in children {
                    if let Some(p) = c.path_data() {
                        bp.extend(p.to_bezpath());
                    }
                }
                self.draw_shape(ctx, f, n, &bp, *rule);
            }
            NodeKind::Text(t) => self.draw_text(ctx, f, n, t),
            NodeKind::Image(im) => self.draw_image(ctx, f, im),
            NodeKind::SymbolInstance { symbol, xf } => {
                if let Some(sym) = f.doc.symbols.iter().find(|s| &s.name == symbol) {
                    let (art, kept) = self.placed_art(&sym.art, *xf, false, Some(n));
                    let outer = std::mem::replace(&mut self.made_again, !kept);
                    self.draw_node(ctx, f, &art, true);
                    self.made_again = outer;
                }
            }
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::Intertwine { .. } => self.draw_live_node(ctx, f, n),
        }
    }

    /// An Image Trace object's `children` (the hidden source image, then the traced shapes) as
    /// `view` shows them on screen.
    fn draw_trace_view(&mut self, ctx: &mut RenderContext, f: &Frame, children: &[Arc<Node>], view: TraceView, knockout: bool) {
        let Some((source, shapes)) = children.split_first() else { return };
        if view.shows_source() && matches!(source.kind, NodeKind::Image(_)) {
            self.draw_node(ctx, f, source, true);
        }
        if view.shows_result() {
            self.draw_children(ctx, f, shapes, knockout);
        }
        if view.shows_outlines() {
            let opts = RenderOptions { outline: true, ..f.opts.clone() };
            let frame = Frame { opts: &opts, ..*f };
            for c in shapes {
                self.draw_outline_member(ctx, &frame, c);
            }
        }
    }

    /// [`Self::draw_node`] for `c`, a member of a clip group or a shape of an Image Trace object in
    /// Outline view, culled by its cached bounds ([`Self::bounds_of`]) and drawn with `c` as
    /// `fx_owner`, so its path and live-effect geometry come from the caches.
    fn draw_outline_member(&mut self, ctx: &mut RenderContext, f: &Frame, c: &Arc<Node>) {
        if !c.visible || f.opts.hidden.contains(&c.id) {
            return;
        }
        match self.bounds_of(f.symbols, c) {
            Some(b) => {
                let pad = f.px * 2.0;
                if !rects_overlap(b.inflate(pad, pad), f.visible) {
                    self.stats.culled += c.count();
                    return;
                }
            }
            None if !c.is_container() => return,
            None => {}
        }
        let outer = self.fx_owner.replace(c.clone());
        self.draw_node(ctx, f, c, true);
        self.fx_owner = outer;
    }

    /// The children of a group: as the elements of a knockout group when `knockout`.
    fn draw_children(&mut self, ctx: &mut RenderContext, f: &Frame, children: &[Arc<Node>], knockout: bool) {
        if knockout {
            self.draw_knockout(ctx, f, children);
        } else if let Some((_, path)) = self.isolation_path(f) {
            // Isolation mode, in a container on the way down to the isolated one: each run of
            // members around it draws dimmed as one (#833).
            let mut rest = children;
            while let Some(i) = rest.iter().position(|c| path.contains(&c.id)) {
                let (around, from) = rest.split_at(i);
                if !around.is_empty() {
                    self.dimmed(ctx, f, &mut |r, c, fr| around.iter().for_each(|n| r.draw_arc(c, fr, n)));
                }
                let Some((on_path, after)) = from.split_first() else { break };
                self.draw_arc(ctx, f, on_path);
                rest = after;
            }
            if !rest.is_empty() {
                self.dimmed(ctx, f, &mut |r, c, fr| rest.iter().for_each(|n| r.draw_arc(c, fr, n)));
            }
        } else {
            for c in children {
                self.draw_arc(ctx, f, c);
            }
        }
    }

    /// Cull bounds of `n` (symbol instances measured by their art box from `symbols`): cached when
    /// it is a node of the tree being drawn.
    fn node_bounds(&mut self, symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
        match self.geom.get(&(n as *const Node as usize)) {
            // The entry keeps its node alive, so the address is that node's.
            Some(e) if std::ptr::eq(Arc::as_ptr(&e.node), n) => e.bounds,
            _ => fx::cull_bounds(symbols, n),
        }
    }

    /// The children of a knockout group ([`Node::knockout_elements`]). Each element first erases
    /// what the elements below it drew wherever it paints (its knockout shape: its coverage at full
    /// object opacity without its opacity mask, or as drawn when its opacity and mask define the
    /// knockout shape), then adds itself: it composites against the group's backdrop instead of
    /// over the elements below it (the art below the group when the group is non-isolated and
    /// drawn over a copy of it, see [`Self::group`] and [`Self::knockout_over`]).
    fn draw_knockout(&mut self, ctx: &mut RenderContext, f: &Frame, children: &[Arc<Node>]) {
        let backdrop = self.backdrop.take();
        for c in Node::knockout_elements(children) {
            if self.skipped(f, c) {
                continue;
            }
            self.knockout_pass(ctx, f, c, Compose::DestOut);
            match &backdrop {
                Some(b) => self.knockout_over(ctx, f, c, b),
                None => self.knockout_pass(ctx, f, c, Compose::Plus),
            }
        }
        self.backdrop = backdrop;
    }

    /// Whether `n` is the knockout element being drawn as its shape (see [`Self::draw_knockout`]).
    fn is_shape(&self, n: &Node) -> bool {
        self.shape_of == n as *const Node as usize
    }

    /// The object opacity `n` is drawn with (full while it is drawn as its knockout shape).
    pub(crate) fn opacity_of(&self, n: &Node) -> f32 {
        if self.is_shape(n) { 1.0 } else { n.opacity }
    }

    fn hairline(&mut self, ctx: &mut RenderContext, f: &Frame, bp: &BezPath, rgba: [u8; 4]) {
        ctx.set_transform(Affine::IDENTITY);
        let mut screen = bp.clone();
        screen.apply_affine(f.view);
        ctx.set_stroke(kurbo::Stroke::new(1.0));
        ctx.set_paint(f.ink.fixed(rgba));
        ctx.stroke_path(&screen);
    }

    fn draw_shape(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, bp: &BezPath, rule: FillRule) {
        if !drawable(bp, f.view, n) {
            return;
        }
        if fx::has_fx(n) {
            return self.draw_shape_fx(ctx, f, n, bp, rule);
        }
        if f.opts.outline {
            self.hairline(ctx, f, bp, [0, 0, 0, 255]);
            return;
        }
        // Bounds only matter to gradients and patterns (their geometry is relative to the object).
        let solid = |p: &vectorcraft_color::Paint| matches!(p, vectorcraft_color::Paint::Solid { .. } | vectorcraft_color::Paint::None);
        let needs_bounds = n.appearance.items.iter().any(|i| match i {
            AppearanceItem::Fill(fl) => !solid(&fl.paint),
            AppearanceItem::Stroke(st) => !solid(&st.paint),
        });
        let bounds = if needs_bounds { bp.bounding_box() } else { Rect::ZERO };
        for item in &n.appearance.items {
            match item {
                AppearanceItem::Fill(fl) => {
                    if !fl.visible || fl.paint.is_none() {
                        continue;
                    }
                    let layered = fl.opacity < 1.0 || fl.blend != vectorcraft_color::BlendMode::Normal;
                    if layered {
                        ctx.set_transform(Affine::IDENTITY);
                        ctx.push_layer(None, Some(blend_mode(fl.blend)), Some(fl.opacity), None, None);
                    }
                    ctx.set_transform(f.view);
                    if paint::set_paint(ctx, &fl.paint, bounds, f) {
                        self.fold_alpha(ctx, f.ink, &fl.paint);
                        ctx.set_fill_rule(fill_rule(rule));
                        ctx.fill_path(bp);
                    }
                    if layered {
                        ctx.pop_layer();
                    }
                }
                AppearanceItem::Stroke(st) => {
                    if !st.visible || st.paint.is_none() || st.width <= 0.0 {
                        continue;
                    }
                    if st.brush.is_some() && self.draw_brush(ctx, f, n, bp, st) {
                        continue;
                    }
                    self.draw_stroke(ctx, f, Affine::IDENTITY, bp, rule, st, bounds);
                }
            }
        }
    }

    /// Multiply the folded object opacity into a solid paint.
    fn fold_alpha(&self, ctx: &mut RenderContext, ink: Ink, p: &vectorcraft_color::Paint) {
        if self.alpha < 1.0
            && let vectorcraft_color::Paint::Solid { color, .. } = p
        {
            ctx.set_paint(ink.color(color, self.alpha));
        }
    }

    /// Paint stroke `st` of the shape `bp` (in the space `xf` maps to the document; `bounds`: its
    /// geometric bounds there) with its alignment, dashes, profile, arrowheads, opacity and blend.
    #[allow(clippy::too_many_arguments)]
    fn draw_stroke(&mut self, ctx: &mut RenderContext, f: &Frame, xf: Affine, bp: &BezPath, rule: FillRule, st: &StrokeLayer, bounds: Rect) {
        let pieces = effects::stroke::stroke_pieces(bp, st);
        // The line and its heads overlap: they take the opacity once, in one layer, instead of
        // each folding it into its paint.
        let folded = if pieces.heads.is_empty() { 1.0 } else { std::mem::replace(&mut self.alpha, 1.0) };
        let opacity = st.opacity * folded;
        let layered = opacity < 1.0 || st.blend != vectorcraft_color::BlendMode::Normal || st.align == StrokeAlign::Outside;
        if layered {
            ctx.set_transform(Affine::IDENTITY);
            ctx.push_layer(None, Some(blend_mode(st.blend)), Some(opacity), None, None);
        }
        ctx.set_transform(f.view * xf);
        let closed = effects::stroke::is_closed(bp);
        // Keep hairlines visible when zoomed far out (at least ~1 device pixel).
        let width = effects::stroke::aligned_width(st, closed).max(f.px * 0.5 / xf.determinant().abs().sqrt().max(1e-9));
        let inside = st.align == StrokeAlign::Inside && closed;
        if inside {
            ctx.set_fill_rule(fill_rule(rule));
            ctx.push_clip_layer(bp);
        }
        // One paint box for the line and the heads, so a gradient runs on into the heads.
        let paint_bounds = bounds.inflate(st.width / 2.0, st.width / 2.0);
        if let Some(g) = st.path_gradient() {
            self.fill_path_gradient(ctx, f, bp, rule, st, &pieces, width, &g.gradient);
        } else if paint::set_paint(ctx, &st.paint, paint_bounds, f) {
            self.fold_alpha(ctx, f.ink, &st.paint);
            ctx.set_fill_rule(peniko::Fill::NonZero);
            match self.cached_stroke(f, &pieces.line, st, width) {
                Some(o) => ctx.fill_path(&o),
                None => ctx.fill_path(&effects::stroke::line_outline(&pieces.line, st, width, f.px * 0.25)),
            }
            for head in &pieces.heads {
                ctx.fill_path(&head.outline);
            }
        }
        if inside {
            ctx.pop_layer();
        }
        if st.align == StrokeAlign::Outside && closed {
            // Punch out the interior.
            ctx.set_blend_mode(BlendMode::new(Mix::Normal, Compose::DestOut));
            ctx.set_paint(peniko::Color::BLACK);
            ctx.set_fill_rule(fill_rule(rule));
            ctx.fill_path(bp);
            ctx.set_blend_mode(BlendMode::default());
        }
        if layered {
            ctx.pop_layer();
        }
        if !pieces.heads.is_empty() {
            self.alpha = folded;
        }
    }

    /// Paint the line and arrowheads (`pieces`) of stroke `st` of `bp` (filled with `rule`) with
    /// its gradient `g` laid along or across the path ([`effects::stroke::gradient_slices`]). Each
    /// piece clips the slices, grown a pixel into each other and each replacing what it covers, so
    /// neighbours meet without seams and translucent colours paint once.
    #[allow(clippy::too_many_arguments)]
    fn fill_path_gradient(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        bp: &BezPath,
        rule: FillRule,
        st: &StrokeLayer,
        pieces: &effects::stroke::StrokePieces,
        width: f64,
        g: &vectorcraft_color::Gradient,
    ) {
        let level = (f.px * 0.25).log2().floor() as i32;
        let make = || -> Slices {
            let tol = 2f64.powi(level);
            // Reach a pixel past the stroke, so its antialiased edge has colour under it.
            let slices = effects::stroke::gradient_slices(bp, rule, st, tol, f.px + tol, f.px);
            Arc::new(
                slices
                    .into_iter()
                    .map(|s| {
                        let (b, paint) = (s.shape.bounding_box(), s.paint(g));
                        (s.shape, paint, b)
                    })
                    .collect(),
            )
        };
        let slices = match self.cur.clone().filter(|n| owns_stroke(n, st)) {
            Some(node) => {
                let key = (self.stroke_owner(&node), st as *const StrokeLayer as usize, level);
                match self.stroke_slices.get_mut(&key) {
                    Some(e) if Arc::ptr_eq(&e.node, &node) => {
                        e.stamp = self.stamp;
                        e.slices.clone()
                    }
                    _ => {
                        let slices = make();
                        self.stroke_slices.insert(key, SliceEntry { node, slices: slices.clone(), stamp: self.stamp });
                        slices
                    }
                }
            }
            None => make(),
        };
        let line = match self.cached_stroke(f, &pieces.line, st, width) {
            Some(o) => o,
            None => Arc::new(effects::stroke::line_outline(&pieces.line, st, width, f.px * 0.25)),
        };
        ctx.set_fill_rule(peniko::Fill::NonZero);
        for clip in std::iter::once(&*line).chain(pieces.heads.iter().map(|h| &h.outline)) {
            let cb = clip.bounding_box();
            if clip.elements().is_empty() || !cb.is_finite() {
                continue;
            }
            ctx.push_clip_layer(clip);
            ctx.set_blend_mode(BlendMode::new(Mix::Normal, Compose::Copy));
            for (shape, paint, b) in slices.iter() {
                if rects_overlap(*b, cb) && paint::set_paint(ctx, paint, *b, f) {
                    ctx.fill_path(shape);
                }
            }
            ctx.set_blend_mode(BlendMode::default());
            ctx.pop_layer();
        }
    }

    /// The line part of a stroke of the fast-path node `self.cur` (see
    /// [`effects::stroke::line_outline`]), cached across frames under [`Self::stroke_owner`] and
    /// the stroke layer. The tolerance (vello's 0.25 device px) is bucketed to powers of two, never
    /// coarser than needed, so zooming reuses outlines until the level changes. Only the node's own
    /// strokes are cached: for the strokes of the brush art drawn for it, this returns `None`.
    fn cached_stroke(&mut self, f: &Frame, line: &BezPath, st: &StrokeLayer, width: f64) -> Option<Arc<BezPath>> {
        let node = self.cur.clone().filter(|n| owns_stroke(n, st))?;
        let level = (f.px * 0.25).log2().floor() as i32;
        let key = (self.stroke_owner(&node), st as *const StrokeLayer as usize, width.to_bits(), level);
        if let Some(e) = self.strokes.get_mut(&key)
            && Arc::ptr_eq(&e.node, &node)
        {
            e.stamp = self.stamp;
            return Some(e.outline.clone());
        }
        let outline = Arc::new(effects::stroke::line_outline(line, st, width, 2f64.powi(level)));
        self.strokes.insert(key, StrokeEntry { node, outline: outline.clone(), stamp: self.stamp });
        Some(outline)
    }

    /// What the `strokes` and `stroke_slices` keys hold for the fast-path node `node`: its address,
    /// or 0 while art made again in every frame is drawn ([`Self::made_again`]).
    fn stroke_owner(&self, node: &Arc<Node>) -> usize {
        if self.made_again { 0 } else { Arc::as_ptr(node) as usize }
    }

    fn draw_text(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, t: &TextObject) {
        let t = &*f.doc.inline_resolved(t);
        // Type drawn as `fx_owner` (the node `draw_arc` is drawing, or type art that `fx_arts`
        // keeps, see `draw_fx_art`) takes its glyph geometry from `texts`, as on the fast path,
        // except in frames that snap type to pixels.
        let owner = self.fx_owner.as_ref().filter(|o| std::ptr::eq(&***o, n)).cloned();
        let g = match (f.text_snap(t), owner) {
            (None, Some(a)) => self.text_geom_of(&a, t),
            (snap, _) => Arc::new(text_geom_snapped(t, snap)),
        };
        self.draw_text_geom(ctx, f, n, t, &g);
    }

    /// Draw the inline graphics of type `t` (laid out in `g`): each symbol's art through the
    /// normal node path, placed by the layout (inside the text's own transparency group, so its
    /// opacity and blend mode apply). A missing symbol draws nothing.
    fn draw_inlines(&mut self, ctx: &mut RenderContext, f: &Frame, t: &TextObject, g: &TextGeom) {
        if g.inlines.is_empty() && !t.runs.iter().any(|r| r.inline.is_some()) {
            return;
        }
        if self.inline_depth >= MAX_INLINE_DEPTH {
            return;
        }
        for r in t.runs.iter().filter_map(|r| r.inline.as_ref()) {
            if !f.doc.symbols.iter().any(|s| s.name == r.symbol) && !MISSING_INLINE_LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::warn!("inline graphic shows symbol {:?}, which the document doesn't have: it draws nothing", r.symbol);
            }
        }
        self.inline_depth += 1;
        for ig in &g.inlines {
            let Some(art) = t.runs.get(ig.run).and_then(|r| r.inline.as_ref()) else { continue };
            let Some(sym) = f.doc.symbols.iter().find(|s| s.name == art.symbol) else { continue };
            // Strokes scale with the art (it is sized to the type, as in the SVG `<use>`).
            let (node, kept) = self.placed_art(&sym.art, t.xf * ig.xf * f.doc.symbol_natural_xf(&art.symbol), true, None);
            let outer = std::mem::replace(&mut self.made_again, !kept);
            self.draw_node(ctx, f, &node, true);
            self.made_again = outer;
        }
        self.inline_depth -= 1;
    }

    /// Symbol art `art` as an instance or an inline graphic draws it ([`place`]), and whether it is
    /// kept. Art that is a group or a layer without an appearance of its own is kept between frames
    /// under the art's `Arc`, `xf`, `scale_strokes` and the stain of instance `inst`: its members
    /// keep their `Arc`s from frame to frame, and the caches keyed by a node's address (geometry,
    /// stroke outlines, glyphs, live-effect geometry) hit for them. Other art is placed again in
    /// every frame, and so is art that holds symbol instances, type with inline graphics, images,
    /// placed documents, live blends, envelopes or repeats, or raster effects
    /// ([`depends_on_document`]), and art that the cache has no room for. Art that holds type is
    /// placed again when the fonts change.
    fn placed_art(&mut self, art: &Arc<Node>, xf: Affine, scale_strokes: bool, inst: Option<&Node>) -> (Arc<Node>, bool) {
        if !matches!(art.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) || fx::has_object_fx(art) {
            return (place(art, xf, scale_strokes, inst), false);
        }
        let stain = stain_bits(inst.and_then(vectorcraft_brush::stain));
        let key = (Arc::as_ptr(art) as usize, xf.as_coeffs().map(f64::to_bits), scale_strokes, stain);
        if let Some(e) = self.placed_arts.get(&key, self.stamp)
            && std::ptr::eq(e.art.as_ptr(), Arc::as_ptr(art))
            && e.fonts.is_none_or(|g| g == vectorcraft_text::FontDb::global().generation())
        {
            return match &e.placed {
                Some(placed) => (placed.clone(), true),
                None => (place(art, xf, scale_strokes, inst), false),
            };
        }
        let placed = place(art, xf, scale_strokes, inst);
        let kept = (!depends_on_document(art)).then(|| placed.clone());
        let keeps = kept.is_some();
        let fonts = (keeps && holds_type(art)).then(|| vectorcraft_text::FontDb::global().generation());
        // The bytes count the key and the entry, the art's allocation that the weak reference
        // keeps, and the kept art.
        let bytes = size_of::<(PlacedKey, PlacedEntry, Node)>() + kept.as_deref().map_or(0, lru::node_bytes);
        let inserted = self.placed_arts.insert(key, PlacedEntry { art: Arc::downgrade(art), placed: kept, fonts }, bytes, self.stamp);
        (placed, keeps && inserted)
    }

    /// Cached glyph geometry for a text node (keyed by Arc identity like paths).
    /// Text `t` that is not the node's own, such as the copy whose inline graphics
    /// `Document::inline_resolved` sized again, is laid out without the cache.
    fn text_geom_of(&mut self, a: &Arc<Node>, t: &TextObject) -> Arc<TextGeom> {
        if !matches!(&a.kind, NodeKind::Text(own) if std::ptr::eq(&**own, t)) {
            return Arc::new(text_geom(t));
        }
        let fonts = vectorcraft_text::FontDb::global().generation();
        if fonts != self.text_fonts {
            self.texts.clear();
            self.text_fonts = fonts;
        }
        let key = Arc::as_ptr(a) as usize;
        if let Some((node, g)) = self.texts.get(&key, self.stamp)
            && Arc::ptr_eq(node, a)
        {
            return g.clone();
        }
        let g = Arc::new(text_geom(t));
        self.texts.insert(key, (a.clone(), g.clone()), g.bytes(), self.stamp);
        g
    }

    fn draw_text_geom(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, t: &TextObject, g: &TextGeom) {
        let xf = f.view * t.xf;
        if f.opts.outline {
            let mut p = g.all.clone();
            p.apply_affine(xf);
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_stroke(kurbo::Stroke::new(1.0));
            ctx.set_paint(peniko::Color::BLACK);
            ctx.stroke_path(&p);
            self.draw_inlines(ctx, f, t, g);
            return;
        }
        let tb = t.xf.transform_rect_bbox(g.bounds);
        // Overprinting characters darken per draw, like overprinting fills and strokes
        // ([`proof`]); the object blend mode is Normal here or applied by an enclosing layer.
        let overprints = proof::overprints(f.opts);
        let overprint = |ctx: &mut RenderContext, on: bool| {
            if overprints {
                ctx.set_blend_mode(if on { blend_mode(vectorcraft_color::BlendMode::Darken) } else { BlendMode::default() });
            }
        };
        // The object's own fills and strokes paint the whole outline: those below the Characters
        // row under the characters' own fill and stroke, the others over them.
        let (below, above) = n.appearance.split_contents();
        let all = (!n.appearance.items.is_empty()).then(|| {
            let mut all = g.all.clone();
            all.apply_affine(t.xf);
            all
        });
        if let Some(all) = &all {
            self.draw_text_items(ctx, f, n, below, all, tb);
        }
        if f.opts.highlight_substitutions {
            let setup = &f.doc.setup;
            for (on, path) in [
                (setup.highlight_substituted_fonts, Some(&g.substituted_fonts)),
                (setup.highlight_substituted_glyphs, Some(&g.substituted_glyphs)),
                // Measured only when shown: it lays the text out again.
                (f.opts.highlight_alternate_glyphs, f.opts.highlight_alternate_glyphs.then(|| g.alternate_glyphs(t))),
            ] {
                let Some(path) = path else { continue };
                if on && !path.elements().is_empty() {
                    ctx.set_transform(xf);
                    ctx.set_fill_rule(peniko::Fill::NonZero);
                    ctx.set_paint(SUBSTITUTED);
                    ctx.fill_path(path);
                }
            }
        }
        for (i, run) in t.runs.iter().enumerate() {
            let path = g.run(i);
            let path = path.as_ref();
            if path.elements().is_empty() {
                continue;
            }
            ctx.set_transform(xf);
            ctx.set_fill_rule(peniko::Fill::NonZero);
            if paint::set_paint(ctx, &run.style.fill, g.bounds, f) {
                self.fold_alpha(ctx, f.ink, &run.style.fill);
                overprint(ctx, run.style.overprint_fill);
                ctx.fill_path(path);
            }
            if run.style.has_stroke() {
                // Character strokes are drawn in text space, with their cap, join and dashes.
                overprint(ctx, run.style.overprint_stroke);
                self.draw_stroke(ctx, f, t.xf, path, FillRule::NonZero, &run.style.stroke_layer(), g.bounds);
            }
        }
        overprint(ctx, false);
        self.draw_inlines(ctx, f, t, g);
        if let Some(all) = &all {
            self.draw_text_items(ctx, f, n, above, all, tb);
        }
    }

    /// Paint some of type `n`'s own fills and strokes (`items`) on its glyph outlines `all`
    /// (document space; `tb` their bounds). Fills composite with their own opacity and blend mode;
    /// strokes take every stroke option, as on paths.
    fn draw_text_items(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, items: &[AppearanceItem], all: &BezPath, tb: Rect) {
        for item in items.iter().filter(|i| i.visible() && !i.paint().is_none()) {
            match item {
                AppearanceItem::Fill(fl) => {
                    let layered = fl.opacity < 1.0 || fl.blend != vectorcraft_color::BlendMode::Normal;
                    if layered {
                        ctx.set_transform(Affine::IDENTITY);
                        ctx.push_layer(None, Some(blend_mode(fl.blend)), Some(fl.opacity), None, None);
                    }
                    ctx.set_transform(f.view);
                    if paint::set_paint(ctx, &fl.paint, tb, f) {
                        ctx.set_fill_rule(peniko::Fill::NonZero);
                        ctx.fill_path(all);
                    }
                    if layered {
                        ctx.pop_layer();
                    }
                }
                AppearanceItem::Stroke(st) if st.width > 0.0 => {
                    if st.brush.is_none() || !self.draw_brush(ctx, f, n, all, st) {
                        self.draw_stroke(ctx, f, Affine::IDENTITY, all, FillRule::NonZero, st, tb);
                    }
                }
                AppearanceItem::Stroke(_) => {}
            }
        }
    }

    fn draw_image(&mut self, ctx: &mut RenderContext, f: &Frame, im: &vectorcraft_doc::ImageObject) {
        self.draw_image_in(ctx, f, im, None);
    }

    /// Draw an image, or with `area` (a region in the document inside the image's frame, see
    /// [`plain_image`]) the image painted into that region.
    fn draw_image_in(&mut self, ctx: &mut RenderContext, f: &Frame, im: &vectorcraft_doc::ImageObject, area: Option<&(BezPath, FillRule)>) {
        let rect = Rect::new(0.0, 0.0, im.width as f64, im.height as f64);
        // Outline mode draws the image's frame, and with Document Setup → Show Images in Outline
        // Mode its pixels in greyscale under the frame.
        let outline = f.opts.outline;
        // With mips the full pixels are only looked at (not marked used) until they're drawn.
        let mips = f.opts.image_mips;
        let pixels = if outline && !f.doc.setup.outline_images { None } else { self.image_pixmap(f.doc, &im.key, outline, !mips) };
        if let Some((key, pm)) = pixels {
            let (key, pm) = if outline { (key, pm) } else { ink::ink_image(&key, &pm, f.ink, f.doc.images.get(&im.key), !mips) };
            let pm = if mips { image_mip(&key, pm, f.view * im.xf * Affine::scale_non_uniform(im.width as f64, im.height as f64)) } else { pm };
            let sx = im.width as f64 / pm.width().max(1) as f64;
            let sy = im.height as f64 / pm.height().max(1) as f64;
            let quality = if f.opts.smooth_images { peniko::ImageQuality::Medium } else { peniko::ImageQuality::Low };
            let sampler = peniko::ImageSampler { quality, ..Default::default() };
            ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(pm), sampler });
            match area {
                Some((bp, rule)) => {
                    ctx.set_transform(f.view);
                    ctx.set_paint_transform(im.xf * Affine::scale_non_uniform(sx, sy));
                    ctx.set_fill_rule(fill_rule(*rule));
                    ctx.fill_path(bp);
                }
                None => {
                    ctx.set_transform(f.view * im.xf);
                    ctx.set_paint_transform(Affine::scale_non_uniform(sx, sy));
                    ctx.fill_rect(&rect);
                }
            }
            ctx.reset_paint_transform();
            // A dimmed layer's images: white over them, so they show at that opacity on paper.
            if let Some(dim) = self.dim_images.filter(|d| *d < 1.0 && !outline) {
                ctx.set_paint(f.ink.fixed([255, 255, 255, ((1.0 - dim) * 255.0).round() as u8]));
                match area {
                    Some((bp, rule)) => {
                        ctx.set_transform(f.view);
                        ctx.set_fill_rule(fill_rule(*rule));
                        ctx.fill_path(bp);
                    }
                    None => {
                        ctx.set_transform(f.view * im.xf);
                        ctx.fill_rect(&rect);
                    }
                }
            }
        }
        if outline {
            let mut p = rect.to_path(0.1);
            p.apply_affine(im.xf);
            self.hairline(ctx, f, &p, [0, 0, 0, 255]);
        }
    }

    /// Name image `key` recoloured by `map` (its hash `hash`) as `derived` (the key the adjusted
    /// art's image takes): drawing `derived` draws the recoloured pixels, made when first drawn and
    /// kept with the decoded images. Names past a limit drop the art made with them too, so it is
    /// made again.
    fn adjusted_image(&mut self, key: &str, derived: String, hash: u64, map: &vectorcraft_effects::ColorMap) {
        if self.adjusts.contains_key(&derived) {
            return;
        }
        if self.adjusts.len() >= 1024 {
            self.adjusts.clear();
            self.fx_arts.clear();
        }
        self.adjusts.insert(derived, (key.to_string(), hash, map.clone()));
    }

    /// The pixels of image `key` (a blob of `doc`, or a recoloured one named by
    /// [`Self::adjusted_image`]), or a greyscale copy of them, with their key in the shared store.
    /// `touch` marks them used there.
    fn image_pixmap(&mut self, doc: &Document, key: &str, grey: bool, touch: bool) -> Option<(images::ImageKey, Arc<Pixmap>)> {
        let (key, colour) = match doc.images.get(key) {
            Some(blob) => blob_pixmap(blob, touch)?,
            None => {
                let (src, hash, map) = self.adjusts.get(key)?.clone();
                let (src_key, src) = self.image_pixmap(doc, &src, false, touch)?;
                let key = images::ImageKey { adjust: Some(hash), ..src_key };
                match lookup(&key, touch) {
                    Some(pm) => (key, pm),
                    None => {
                        let mut pm = (*src).clone();
                        let mut memo: HashMap<[u8; 4], [u8; 3]> = HashMap::new();
                        for px in pm.data_mut().iter_mut().filter(|p| p.a > 0) {
                            let a = px.a as u32;
                            let [r, g, b] = *memo.entry([px.r, px.g, px.b, px.a]).or_insert_with(|| {
                                // Premultiplied to straight, adjusted, then premultiplied again.
                                let straight = [px.r, px.g, px.b].map(|v| ((v as u32 * 255 + a / 2) / a).min(255) as u8);
                                map.apply_rgb8(straight).map(|v| ((v as u32 * a + 127) / 255) as u8)
                            });
                            (px.r, px.g, px.b) = (r, g, b);
                        }
                        let pm = images::store().insert(key.clone(), Arc::new(pm));
                        (key, pm)
                    }
                }
            }
        };
        if !grey {
            return Some((key, colour));
        }
        let key = images::ImageKey { grey: true, ..key };
        if let Some(p) = lookup(&key, touch) {
            return Some((key, p));
        }
        let mut pm = (*colour).clone();
        for px in pm.data_mut() {
            // The luma of premultiplied components is the premultiplied luma.
            let l = (0.2126 * px.r as f32 + 0.7152 * px.g as f32 + 0.0722 * px.b as f32 + 0.5) as u8;
            (px.r, px.g, px.b) = (l, l, l);
        }
        let pm = images::store().insert(key.clone(), Arc::new(pm));
        Some((key, pm))
    }
}

/// `key`'s pixels in the shared image store; `touch` marks them used.
fn lookup(key: &images::ImageKey, touch: bool) -> Option<Arc<Pixmap>> {
    let mut s = images::store();
    if touch { s.get(key) } else { s.peek(key) }
}

/// The decoded pixels of `blob` (a CMYK image's inks shown through the working CMYK profile, again
/// when the colour settings change), from the shared store, and their key there.
fn blob_pixmap(blob: &vectorcraft_doc::ImageBlob, touch: bool) -> Option<(images::ImageKey, Arc<Pixmap>)> {
    let id = images::blob_id(blob);
    let lut = id.cmyk.then(ink::display_lut);
    let key =
        images::ImageKey { content: id.content, adjust: None, grey: false, ink: Ink::Display, luts: (lut.as_ref().map_or(0, |l| l.1), 0), mip: 0 };
    if let Some(pm) = lookup(&key, touch) {
        return Some((key, pm));
    }
    // Decoded outside the lock: other renderers carry on meanwhile.
    let pm = match lut.and_then(|(lut, _)| ink::cmyk_pixmap(blob, &lut)) {
        Some(pm) => pm,
        None => paint::decode_pixmap(&blob.bytes)?,
    };
    let pm = images::store().insert(key.clone(), Arc::new(pm));
    Some((key, pm))
}

/// Image pixels `pm` (stored as `key`) to draw through `m` (unit square → device pixels), shrunk
/// by halves while still at least as fine as the device pixels ([`images::mip_level`]); the
/// pixels drawn are marked used in the store.
fn image_mip(key: &images::ImageKey, pm: Arc<Pixmap>, m: Affine) -> Arc<Pixmap> {
    let (w, h) = (pm.width(), pm.height());
    let level = images::mip_level(m * Affine::scale_non_uniform(1.0 / w.max(1) as f64, 1.0 / h.max(1) as f64), w, h);
    let mut s = images::store();
    if level == 0 {
        let _ = s.get(key); // Marks the full pixels used (they were only looked at).
        return pm;
    }
    let at = |l: u8| images::ImageKey { mip: l, ..key.clone() };
    if let Some(p) = s.get(&at(level)) {
        return p;
    }
    // From the finest level kept below it.
    let (mut l, mut cur) = (1..level).rev().find_map(|l| s.peek(&at(l)).map(|p| (l, p))).unwrap_or((0, pm));
    drop(s);
    while l < level {
        l += 1;
        let half = Arc::new(images::half(&cur));
        cur = images::store().insert(at(l), half);
    }
    cur
}

/// Can image node `n` (`im`) clipped by `region` be painted straight into the region: visible,
/// with no transparency, effects or paint of its own, and the region inside the image's frame
/// (within a pixel), so no paint past the image's edge shows?
fn plain_image(n: &Node, im: &vectorcraft_doc::ImageObject, region: &BezPath) -> bool {
    if !(n.visible && n.has_default_transparency() && n.appearance.items.is_empty() && n.appearance.effects.is_empty()) {
        return false;
    }
    let det = im.xf.determinant();
    if !(det.abs() > 1e-12 && det.is_finite()) {
        return false;
    }
    let b = im.xf.inverse().transform_rect_bbox(region.bounding_box());
    b.x0 >= -1.0 && b.y0 >= -1.0 && b.x1 <= im.width as f64 + 1.0 && b.y1 <= im.height as f64 + 1.0
}

/// Opacity-mask coverage of one premultiplied pixel: luminance, with the area outside the mask
/// art black (clip) or white (no clip), optionally inverted.
fn mask_value(r: u8, g: u8, b: u8, a: u8, clip: bool, invert: bool) -> u8 {
    let [kr, kg, kb] = vectorcraft_color::blend::MASK_LUM;
    let mut l = (kr * r as f32 + kg * g as f32 + kb * b as f32) / 255.0;
    if !clip {
        l += 1.0 - a as f32 / 255.0;
    }
    if invert {
        l = 1.0 - l;
    }
    (l.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Most pixels from the origin that a shape handed to the rasterizer may reach, or a stroke be
/// wide. A document's art stays within 4 million points, 2.6 billion pixels at the closest zoom.
/// Geometry far beyond that (from a malformed file) makes vello_cpu panic on a worker thread or ask
/// for more memory than there is, either of which ends the app: it is left undrawn.
const MAX_DEVICE: f64 = 1e10;

/// Can shape `bp` of `n` be drawn through `view`: its control points (finite) within
/// [`MAX_DEVICE`] pixels, and its visible strokes no wider?
fn drawable(bp: &BezPath, view: Affine, n: &Node) -> bool {
    let b = view.transform_rect_bbox(bp.control_box());
    let scale = view.determinant().abs().sqrt();
    let widest = n.appearance.items.iter().filter_map(|i| if let AppearanceItem::Stroke(st) = i { Some(st.width) } else { None }).fold(0.0, f64::max);
    [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_DEVICE) && (widest * scale).is_finite() && widest * scale <= MAX_DEVICE
}

/// A render context on the calling thread. vello_cpu's `RenderContext::new` defaults to a
/// multithreaded dispatcher, which panics on filter effects (glows, shadows, blur).
pub(crate) fn single_threaded_context(w: u16, h: u16) -> RenderContext {
    RenderContext::new_with(w, h, vello_cpu::RenderSettings { num_threads: 0, ..Default::default() })
}

/// Exactly one visible painted item, a fill with its own Normal blend and full opacity (one
/// paint operation, so an object blend mode can be applied per draw).
fn single_plain_fill(n: &Node) -> bool {
    let mut painted = n.appearance.items.iter().filter(|i| match i {
        AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
        AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0,
    });
    matches!((painted.next(), painted.next()), (Some(AppearanceItem::Fill(f)), None) if f.blend == vectorcraft_color::BlendMode::Normal && f.opacity >= 1.0 && !matches!(f.paint, vectorcraft_color::Paint::Pattern { .. }))
}

/// Number of visible painted fill/stroke items (opacity folding is exact only for one).
fn painted_items(n: &Node) -> usize {
    n.appearance
        .items
        .iter()
        .filter(|i| match i {
            AppearanceItem::Fill(f) => f.visible && !f.paint.is_none() && matches!(f.paint, vectorcraft_color::Paint::Solid { .. }),
            AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0 && matches!(s.paint, vectorcraft_color::Paint::Solid { .. }) && s.dash.is_none(),
        })
        .count()
        .max(if n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Fill(f) if f.visible && !matches!(f.paint, vectorcraft_color::Paint::Solid { .. } | vectorcraft_color::Paint::None)) || matches!(i, AppearanceItem::Stroke(s) if s.visible && !matches!(s.paint, vectorcraft_color::Paint::Solid { .. } | vectorcraft_color::Paint::None))) { 2 } else { 0 })
}

/// Whether `st` is one of `n`'s own strokes (the strokes of the brush art drawn for `n` are not).
fn owns_stroke(n: &Node, st: &StrokeLayer) -> bool {
    n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(s) if std::ptr::eq(s, st)))
}

/// Symbol art `art` stained by instance `inst` ([`instance_art`]) and placed by `xf`, with
/// strokes scaled by `xf` when `scale_strokes`.
fn place(art: &Node, xf: Affine, scale_strokes: bool, inst: Option<&Node>) -> Arc<Node> {
    let mut placed = match inst {
        Some(i) => brush_fx::instance_art(art, i),
        None => art.clone(),
    };
    placed.transform(xf, scale_strokes);
    Arc::new(placed)
}

/// Does `n` hold, in it or in the opacity masks of the objects in it, a symbol instance, type
/// with an inline graphic, an image, a placed document, a blend, an envelope, a repeat, or an
/// object or a fill or stroke with a visible raster effect?
fn depends_on_document(n: &Node) -> bool {
    let raster = |fx: &[vectorcraft_doc::Effect]| fx.iter().any(|e| e.visible && effects::is_raster(&e.id));
    let mut depends = false;
    n.walk(&mut |c| {
        depends |= match &c.kind {
            NodeKind::SymbolInstance { .. }
            | NodeKind::Image(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Repeat(_) => true,
            NodeKind::Text(t) => t.runs.iter().any(|r| r.inline.is_some()),
            _ => false,
        } || raster(&c.appearance.effects)
            || c.appearance.items.iter().any(|i| raster(i.effects()))
            || c.extras().mask.as_ref().is_some_and(|m| depends_on_document(&m.art));
    });
    depends
}

/// Does `n` hold type, in it or in the opacity masks of the objects in it?
fn holds_type(n: &Node) -> bool {
    let mut holds = false;
    n.walk(&mut |c| holds |= matches!(c.kind, NodeKind::Text(_)) || c.extras().mask.as_ref().is_some_and(|m| holds_type(&m.art)));
    holds
}

/// Is `n` a symbol instance, or does it hold one? Its cull bounds measure the instance by its
/// symbol's art.
fn holds_instance(n: &Node) -> bool {
    let mut holds = false;
    n.walk(&mut |c| holds |= matches!(c.kind, NodeKind::SymbolInstance { .. }));
    holds
}

/// Does `n` hold an object with a visible plug-in effect ([`effects::has_plugin_effect`]), in it or
/// in the opacity masks of the objects in it?
fn holds_plugin_effect(n: &Node) -> bool {
    let mut holds = false;
    n.walk(&mut |c| holds |= effects::has_plugin_effect(c) || c.extras().mask.as_ref().is_some_and(|m| holds_plugin_effect(&m.art)));
    holds
}

/// A symbol instance's stain ([`vectorcraft_brush::stain`]) as bits for a cache key: its color
/// model, the color's components and the amount (all 0 without a stain).
fn stain_bits(stain: Option<(vectorcraft_color::Color, f32)>) -> [u32; 6] {
    use vectorcraft_color::Color;
    let Some((color, amount)) = stain else { return [0; 6] };
    let (model, [p, q, r, s]) = match color {
        Color::Rgb { r, g, b } => (1, [r, g, b, 0.0]),
        Color::Cmyk { c, m, y, k } => (2, [c, m, y, k]),
        Color::Gray { k } => (3, [k, 0.0, 0.0, 0.0]),
        Color::Lab { l, a, b } => (4, [l, a, b, 0.0]),
    };
    [model, p.to_bits(), q.to_bits(), r.to_bits(), s.to_bits(), amount.to_bits()]
}

/// Preferred rasterizer thread count set by the app (Preferences → Performance); negative = automatic.
static THREADS_OVERRIDE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Override the worker-thread count used by renderers created from now on (`None` = automatic).
pub fn set_default_threads(n: Option<u16>) {
    THREADS_OVERRIDE.store(n.map_or(-1, i32::from), std::sync::atomic::Ordering::Relaxed);
}

/// Rasterizer worker threads: 0 on wasm; otherwise one per core up to 8, leaving a core free. On a
/// 10-core machine 8 workers render a 50,000-path page 1.4× faster than 4 (9 are slower again);
/// the pixels don't depend on the count (`pixels_do_not_depend_on_the_thread_count`).
pub fn default_threads() -> u16 {
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let o = THREADS_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
        if o >= 0 {
            return o.min(64) as u16;
        }
        if std::env::var_os("VECTORCRAFT_RENDER_THREADS").is_some() {
            return std::env::var("VECTORCRAFT_RENDER_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        }
        std::thread::available_parallelism().map(|n| (n.get().saturating_sub(1)).min(8) as u16).unwrap_or(0)
    }
}

/// The whole text's glyph outlines as one path, and which of them each run draws.
struct TextGeom {
    all: BezPath,
    /// Spans of `all`'s elements drawn by a run, in order: (run, start, end). Outlines of runs
    /// the text doesn't have are in `all` only.
    spans: Vec<(usize, usize, usize)>,
    bounds: Rect,
    /// The boxes of characters whose font is missing (drawn in a substitute), and of glyphs the
    /// font lacks (drawn from a fallback font): Document Setup's substitution highlights.
    substituted_fonts: BezPath,
    substituted_glyphs: BezPath,
    /// Glyphs OpenType may have substituted (a ligature, or not the face's cmap glyph for their
    /// character), each with its box: what Preferences › Highlight Alternate Glyphs weighs
    /// ([`TextGeom::alternate_glyphs`]).
    alternate_candidates: Vec<AlternateCandidate>,
    /// The boxes of the candidates that are alternates, measured the first time the preference
    /// shows them.
    alternate_glyphs: std::sync::OnceLock<BezPath>,
    /// Inline graphics placed by the layout.
    inlines: Vec<vectorcraft_text::InlineGlyph>,
}

impl TextGeom {
    /// The outlines run `i` draws: the glyphs of `all` it has, in their order. Kept once rather
    /// than per run as well, which held every outline twice.
    fn run(&self, i: usize) -> std::borrow::Cow<'_, BezPath> {
        let els = self.all.elements();
        if let [(r, 0, end)] = self.spans[..]
            && r == i
            && end == els.len()
        {
            return std::borrow::Cow::Borrowed(&self.all);
        }
        let mut p = BezPath::new();
        for &(_, start, end) in self.spans.iter().filter(|s| s.0 == i) {
            p.extend(els.get(start..end).unwrap_or_default().iter().copied());
        }
        std::borrow::Cow::Owned(p)
    }

    /// The boxes of the glyphs of `t` (the text this geometry is of) that optional OpenType features
    /// chose (Preferences › Highlight Alternate Glyphs): candidates the text's required shaping
    /// alone doesn't give ([`required_shaping`]). Arabic letters joining, Indic conjuncts and the
    /// lam-alef ligature are what the script needs, not alternates. Measured once.
    fn alternate_glyphs(&self, t: &TextObject) -> &BezPath {
        self.alternate_glyphs.get_or_init(|| {
            let mut out = BezPath::new();
            if self.alternate_candidates.is_empty() {
                return out;
            }
            let required = required_shaping(t);
            for c in &self.alternate_candidates {
                if !required.get(&(c.byte, c.len)).is_some_and(|glyphs| glyphs.contains(&(c.font_id, c.gid))) {
                    out.extend(c.cell.iter());
                }
            }
            out
        })
    }

    /// Approximate heap bytes, for the cache's budget.
    fn bytes(&self) -> usize {
        size_of::<Self>()
            + [&self.all, &self.substituted_fonts, &self.substituted_glyphs].into_iter().map(lru::path_bytes).sum::<usize>()
            + self.alternate_candidates.iter().map(|c| size_of::<AlternateCandidate>() + lru::path_bytes(&c.cell)).sum::<usize>()
            + self.alternate_glyphs.get().map_or(0, lru::path_bytes)
            + self.spans.len() * size_of::<(usize, usize, usize)>()
            + self.inlines.len() * size_of::<vectorcraft_text::InlineGlyph>()
    }
}

/// OpenType-substituted form: a ligature (several source characters → one glyph) or a glyph whose
/// id is not the face's cmap mapping for its first character (contextual / stylistic alternate).
pub(crate) fn is_opentype_alternate(nominal_gid: u32, shaped_gid: u32, cluster_chars: usize) -> bool {
    shaped_gid != 0 && (cluster_chars > 1 || shaped_gid != nominal_gid)
}

/// A glyph [`is_opentype_alternate`] marks, in its cluster of the text (`byte`, `len`), with its
/// box.
#[derive(Debug)]
struct AlternateCandidate {
    byte: usize,
    len: usize,
    font_id: u32,
    gid: u32,
    cell: BezPath,
}

/// The glyphs `t` is shaped to by required shaping alone, by cluster (`byte`, `len`) → `(font,
/// glyph)`: laid out again with every optional feature off (standard and contextual ligatures and
/// alternates, and the features its characters turn on: stylistic sets, swashes, discretionary
/// ligatures, figure styles…). What is left is what the script needs: `ccmp`, `locl`, `rlig`,
/// Arabic's `init`, `medi`, `fina` and `isol`, the Indic and other complex scripts' required sets.
fn required_shaping(t: &TextObject) -> std::collections::HashMap<(usize, usize), Vec<(u32, u32)>> {
    let mut plain = t.clone();
    for run in &mut plain.runs {
        run.style.features = vec!["-liga".into(), "-calt".into()];
    }
    let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &plain);
    let mut glyphs: std::collections::HashMap<(usize, usize), Vec<(u32, u32)>> = std::collections::HashMap::new();
    for g in &layout.glyphs {
        glyphs.entry((g.byte, g.len)).or_default().push((g.font_id, g.gid));
    }
    glyphs
}

fn text_geom(t: &TextObject) -> TextGeom {
    text_geom_snapped(t, None)
}

/// Glyph geometry of `t`; `snap` (text space → device pixels) puts the glyphs on whole pixels.
fn text_geom_snapped(t: &TextObject, snap: Option<Affine>) -> TextGeom {
    let db = vectorcraft_text::FontDb::global();
    let mut layout = vectorcraft_text::layout(db, t);
    if let Some(xf) = snap {
        layout.snap_to_pixels(xf);
    }
    let mut all = BezPath::new();
    let mut spans: Vec<(usize, usize, usize)> = vec![];
    // Append `outline` to `all`, drawn by `run` when the text has that run.
    let mut push = |all: &mut BezPath, run: usize, outline: &BezPath| {
        let start = all.elements().len();
        all.extend(outline.iter());
        let end = all.elements().len();
        if run >= t.runs.len() || start == end {
            return;
        }
        match spans.last_mut() {
            Some((r, _, e)) if *r == run && *e == start => *e = end,
            _ => spans.push((run, start, end)),
        }
    };
    // Per run: is its family missing, and the face its glyphs should come from.
    let faces: Vec<(bool, Option<u32>)> = t
        .runs
        .iter()
        .map(|r| match db.resolve(&r.style.font_family, &r.style.font_style) {
            // The version the type names, when it's installed (see `FontDb::face_version`), at its
            // Variable Font settings (see `FontDb::style_face`).
            Some((f, m)) => {
                let face = db.style_face(&r.style).unwrap_or(f);
                (m == vectorcraft_text::FontMatch::Missing, Some(face.id()))
            }
            None => (true, None),
        })
        .collect();
    let (mut substituted_fonts, mut substituted_glyphs, mut alternate_candidates) = (BezPath::new(), BezPath::new(), vec![]);
    let plain = t.plain_text();
    for g in &layout.glyphs {
        push(&mut all, g.run, &g.outline);
        let Some(&(missing, face)) = faces.get(g.run) else { continue };
        let Some(line) = layout.lines.get(g.line) else { continue };
        let mut cell = Rect::new(g.origin.x, g.origin.y - line.ascent, g.origin.x + g.advance, g.origin.y + line.descent).to_path(0.1);
        cell.apply_affine(Affine::rotate_about(g.angle, g.origin));
        if missing {
            substituted_fonts.extend(cell.iter());
            continue;
        }
        if face.is_some_and(|f| f != g.font_id) {
            substituted_glyphs.extend(cell.iter());
            continue;
        }
        // Same face: OpenType may have swapped the glyph (liga, calt, …).
        let Some(cluster) = plain.get(g.byte..g.byte.saturating_add(g.len)) else { continue };
        let cluster_chars = cluster.chars().count();
        let Some(first) = cluster.chars().next() else { continue };
        let nominal = db.face_by_id(g.font_id).map(|f| f.glyph_for(first)).unwrap_or(0);
        if is_opentype_alternate(nominal, g.gid, cluster_chars) {
            alternate_candidates.push(AlternateCandidate { byte: g.byte, len: g.len, font_id: g.font_id, gid: g.gid, cell });
        }
    }
    // Underline and strikethrough bars, painted as their run's type is (#847).
    for (run, bar) in vectorcraft_text::decorations(&layout, db, t) {
        push(&mut all, run, &bar);
    }
    TextGeom {
        all,
        spans,
        bounds: layout.bounds,
        substituted_fonts,
        substituted_glyphs,
        alternate_candidates,
        alternate_glyphs: std::sync::OnceLock::new(),
        inlines: layout.inlines,
    }
}

/// Document Setup's and Preferences › Highlight Alternate Glyphs pink (screen only).
const SUBSTITUTED: peniko::Color = peniko::Color::from_rgba8(255, 120, 190, 110);
/// The pixel size of `region` rendered at `scale` pixels per point (at least 1×1).
pub fn region_pixels(region: Rect, scale: f64) -> (u32, u32) {
    ((region.width() * scale).round().max(1.0) as u32, (region.height() * scale).round().max(1.0) as u32)
}

fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

fn fill_rule(r: FillRule) -> peniko::Fill {
    match r {
        FillRule::NonZero => peniko::Fill::NonZero,
        FillRule::EvenOdd => peniko::Fill::EvenOdd,
    }
}

pub(crate) fn blend_mode(b: vectorcraft_color::BlendMode) -> BlendMode {
    use vectorcraft_color::BlendMode as B;
    let mix = match b {
        B::Normal => Mix::Normal,
        B::Darken => Mix::Darken,
        B::Multiply => Mix::Multiply,
        B::ColorBurn => Mix::ColorBurn,
        B::Lighten => Mix::Lighten,
        B::Screen => Mix::Screen,
        B::ColorDodge => Mix::ColorDodge,
        B::Overlay => Mix::Overlay,
        B::SoftLight => Mix::SoftLight,
        B::HardLight => Mix::HardLight,
        B::Difference => Mix::Difference,
        B::Exclusion => Mix::Exclusion,
        B::Hue => Mix::Hue,
        B::Saturation => Mix::Saturation,
        B::Color => Mix::Color,
        B::Luminosity => Mix::Luminosity,
    };
    BlendMode::new(mix, Compose::SrcOver)
}

#[cfg(not(target_arch = "wasm32"))]
fn now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)
}
#[cfg(target_arch = "wasm32")]
fn now() -> u64 {
    0
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_adjust;
#[cfg(test)]
mod tests_alternate;
#[cfg(test)]
mod tests_blend;
#[cfg(test)]
mod tests_charstroke;
#[cfg(test)]
mod tests_clip;
#[cfg(test)]
mod tests_cmykblend;
#[cfg(test)]
mod tests_container;
#[cfg(test)]
mod tests_fontchange;
#[cfg(test)]
mod tests_freeform;
#[cfg(test)]
mod tests_fxzoom;
#[cfg(test)]
mod tests_isolation;
#[cfg(test)]
mod tests_knockout;
#[cfg(test)]
mod tests_layeropts;
#[cfg(test)]
mod tests_memory;
#[cfg(test)]
mod tests_objectfx;
#[cfg(test)]
mod tests_placed_document;
#[cfg(test)]
mod tests_rasterize;
#[cfg(test)]
mod tests_setup;
#[cfg(test)]
mod tests_strokegradient;
#[cfg(test)]
mod tests_tileedge;
#[cfg(test)]
mod tests_viewraster;
