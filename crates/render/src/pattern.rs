//! Pattern fills and the pattern-editing preview.
//!
//! A pattern paint is drawn as a repeating image: the tiling's rectangular super-tile
//! ([`PatternDef::period`]) is rasterized at the device scale it is painted at, rounded up to a
//! power of two and capped ([`raster_scale`]), and used as an image paint with `Extend::Repeat`,
//! with the paint transform mapping pixels → pattern space → document. Tiles are cached per
//! pattern name and raster scale (and ink plane, see [`crate::ink`]) within
//! [`lru::PATTERN_BUDGET`]. A tile is used while the pattern's definition is the one it was
//! rasterized from (art `Arc` identity, tile, tile type, overlap): the pattern's "revision", and,
//! when its art shows more than itself (a symbol, another pattern, an image, type, a brush), while
//! what the document holds of those is what it held then ([`Deps`]), and while the color settings
//! are the ones it was drawn through. A tile whose art has a plug-in effect, or shows symbols or
//! patterns, is also dropped when a plug-in is installed or removed.
//!
//! Pattern editing mode (`Document::pattern_edit`) draws only the temporary tile layer, with
//! dimmed copies of it at the neighbouring tile positions and the tile edge.

use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Weak};

use vectorcraft_color::cms::Cms;
use vectorcraft_doc::pattern::{Overlap, PatternDef, TileType};
use vectorcraft_doc::{Document, Node};
use vectorcraft_geom::{Affine, Rect, Shape, Vec2};
use vello_cpu::peniko::{Extend, ImageQuality, ImageSampler};
use vello_cpu::{Pixmap, RenderContext};

use crate::ink::Ink;
use crate::lru::{self, FrameCache};
use crate::{Frame, RenderOptions, Rendered, Renderer};

/// Largest super-tile raster side (pixels).
const MAX_TILE_PX: f64 = 2048.0;

/// What a tile is kept under: the pattern's name and the bits of its raster scale
/// ([`raster_scale`]).
type TileKey = (String, u64, Ink);

/// What a tile was drawn from besides its pattern's own art, for art that shows more than itself:
/// the art of every pattern and symbol and the pixels of every image in the document, the
/// document's other data (which holds its brushes), and the fonts. Editing a symbol leaves the art
/// of a pattern that shows it as it was, so the art alone kept the tile from before (#1723). Held
/// weakly: a weak reference keeps the allocation, so no other value gets one of these addresses
/// while the tile is kept.
struct Deps {
    art: Vec<Weak<Node>>,
    pixels: Vec<Weak<Vec<u8>>>,
    /// shortcut: any change to the document's other data draws the tile again, not only a brush
    /// edit; compare the brush library alone if that ever costs a frame.
    unknown: Weak<vectorcraft_brush::serde_json::Map<String, vectorcraft_brush::serde_json::Value>>,
    fonts: u64,
}

impl Deps {
    fn art(doc: &Document) -> impl Iterator<Item = &Arc<Node>> {
        doc.patterns.iter().flat_map(|p| &p.art).chain(doc.symbols.iter().map(|s| &s.art))
    }

    fn pixels(doc: &Document) -> impl Iterator<Item = &Arc<Vec<u8>>> {
        doc.images.values().flat_map(|b| std::iter::once(&b.bytes).chain(&b.proxy))
    }

    fn of(doc: &Document) -> Self {
        Self {
            art: Self::art(doc).map(Arc::downgrade).collect(),
            pixels: Self::pixels(doc).map(Arc::downgrade).collect(),
            unknown: vectorcraft_doc::Shared::downgrade(&doc.unknown),
            fonts: vectorcraft_text::FontDb::global().generation(),
        }
    }

    /// Does `doc` hold what the tile was drawn from?
    fn current(&self, doc: &Document) -> bool {
        self.fonts == vectorcraft_text::FontDb::global().generation()
            && std::ptr::eq(self.unknown.as_ptr(), &*doc.unknown)
            && self.art.iter().map(Weak::as_ptr).eq(Self::art(doc).map(Arc::as_ptr))
            && self.pixels.iter().map(Weak::as_ptr).eq(Self::pixels(doc).map(Arc::as_ptr))
    }
}

struct TileEntry {
    art: Vec<Arc<Node>>,
    /// `None` for art that shows only itself.
    deps: Option<Deps>,
    /// The color settings the tile was drawn through, held weakly as [`Deps`] holds its values.
    cms: Weak<Cms>,
    params: (Rect, TileType, Overlap),
    sx: f64,
    sy: f64,
    pixmap: Arc<Pixmap>,
    /// How many images in the art failed to decode while the tile was drawn
    /// ([`crate::paint::decode_failures`]): each frame that draws from the tile counts them
    /// again ([`crate::paint::add_decode_failures`]).
    failed: u64,
    /// Whether an image in the art failed to draw while the tile was drawn: each frame that draws
    /// from the tile counts it again ([`images_left_out`]).
    image_left_out: bool,
}

struct PatternCache {
    /// Per ink ([`Ink`] as index), each within [`lru::PATTERN_BUDGET`], least recently used first out.
    tiles: [FrameCache<TileKey, TileEntry>; 4],
    renderer: Option<Box<Renderer>>,
    depth: u32,
    /// The plug-in revision ([`crate::effects::plugin_revision`]) the tiles were last checked
    /// against (see [`PatternCache::check_plugins`]).
    plugin_revision: u64,
}

impl Default for PatternCache {
    fn default() -> Self {
        Self { tiles: std::array::from_fn(|_| FrameCache::new(lru::PATTERN_BUDGET, usize::MAX)), renderer: None, depth: 0, plugin_revision: 0 }
    }
}

impl PatternCache {
    /// When a plug-in has been installed or removed since the last check, drop the tiles whose art
    /// can draw differently with it: art with a plug-in effect, and art that shows symbols or
    /// patterns ([`crate::fx::Reads::plugins_can_change`]).
    fn check_plugins(&mut self) {
        let revision = crate::effects::plugin_revision();
        if revision == self.plugin_revision {
            return;
        }
        self.plugin_revision = revision;
        for tiles in &mut self.tiles {
            tiles.retain(|e| !e.art.iter().any(|a| crate::fx::Reads::of(a).plugins_can_change()));
        }
    }
}

thread_local! {
    static CACHE: RefCell<PatternCache> = RefCell::new(PatternCache::default());
    static IMAGES_LEFT_OUT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many pattern tiles drawn on this thread so far left out an image of their art: opacity-mask
/// art compares the count before and after it draws ([`Renderer::mask_values`]).
pub(crate) fn images_left_out() -> u64 {
    IMAGES_LEFT_OUT.with(std::cell::Cell::get)
}

fn same_art(a: &[Arc<Node>], b: &[Arc<Node>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| Arc::ptr_eq(x, y))
}

/// Hash what pattern `def`'s tiling is drawn from, as the tile cache checks it: its name, the
/// addresses of its art, its tile, tile type and overlap.
pub(crate) fn hash_tiling(def: &PatternDef, h: &mut impl Hasher) {
    def.name.hash(h);
    def.art.iter().for_each(|a| (Arc::as_ptr(a) as usize).hash(h));
    [def.tile.x0, def.tile.y0, def.tile.x1, def.tile.y1].map(f64::to_bits).hash(h);
    std::mem::discriminant(&def.tile_type).hash(h);
    if let TileType::BrickByRow { offset } | TileType::BrickByColumn { offset } = def.tile_type {
        offset.to_bits().hash(h);
    }
    (def.overlap.right_in_front, def.overlap.bottom_in_front).hash(h);
}

/// Power-of-two raster scale for a pattern seen at device scale `s`, capped so the super-tile
/// stays under [`MAX_TILE_PX`].
pub(crate) fn raster_scale(s: f64, period: (f64, f64)) -> f64 {
    let s = s.clamp(1e-3, 1e3);
    let p2 = 2f64.powf(s.log2().ceil());
    let cap = MAX_TILE_PX / period.0.max(period.1).max(1e-6);
    p2.min(cap).max(1e-3)
}

/// Rasterize `region` (pattern space) of the infinite tiling into a `w`×`h` pixmap, colours as
/// `ink` paints them.
fn rasterize(r: &mut Renderer, doc: &Document, def: &PatternDef, region: Rect, w: u16, h: u16, ink: Ink) -> (Pixmap, bool) {
    let (w, h) = (w.max(1), h.max(1));
    let mut ctx = crate::single_threaded_context(w, h);
    let view = Affine::scale_non_uniform(w as f64 / region.width().max(1e-9), h as f64 / region.height().max(1e-9))
        * Affine::translate(-region.origin().to_vec2());
    let opts = RenderOptions::default();
    r.image_left_out = Some(false);
    let pattern_images_left_out = images_left_out();
    r.check_plugins();
    r.check_symbols(doc);
    let symbols = doc.symbol_boxes().quick();
    r.stamp += 1;
    let px_rect = Rect::new(0.0, 0.0, w as f64, h as f64);
    for o in def.offsets_covering(region) {
        let v = view * def.instance_xf(o);
        let frame = Frame {
            mt: false,
            doc,
            symbols: &symbols,
            view: v,
            visible: v.inverse().transform_rect_bbox(px_rect),
            px: 1.0 / v.determinant().abs().sqrt().max(1e-12),
            opts: &opts,
            ink,
            plates: None,
        };
        for a in &def.art {
            r.draw_arc(&mut ctx, &frame, a);
        }
    }
    (r.finish(&mut ctx, r.raster), r.image_left_out.take().unwrap_or(false) || images_left_out() > pattern_images_left_out)
}

/// Run `f` with the shared off-screen renderer (None when nested too deep: a pattern whose art
/// uses a pattern whose art uses a pattern…).
fn with_tile_renderer<T>(f: impl FnOnce(&mut Renderer) -> T) -> Option<T> {
    let r = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.depth >= 3 {
            return None;
        }
        c.depth += 1;
        Some(c.renderer.take().unwrap_or_else(|| {
            let mut r = Renderer::new();
            r.threads = 0;
            Box::new(r)
        }))
    })?;
    let mut r = r;
    // Restores the depth even if `f` panics and the panic is caught (the renderer is then dropped).
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            CACHE.with(|c| c.borrow_mut().depth -= 1);
        }
    }
    let guard = Restore;
    let out = f(&mut r);
    drop(guard);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.renderer.is_none() {
            c.renderer = Some(r);
        }
    });
    Some(out)
}

/// Set a pattern paint on `ctx` (its current transform maps the paint's user space to pixels), as
/// frame `f` paints it.
pub(crate) fn set_pattern_paint(ctx: &mut RenderContext, name: &str, xf: Affine, f: &Frame) -> bool {
    let (doc, ink) = (f.doc, f.ink);
    let missing = || ink.fixed([128, 128, 128, 255]);
    let Some(def) = doc.pattern(name) else {
        ctx.set_paint(missing());
        return true;
    };
    if def.art.is_empty() {
        return false;
    }
    let dev = *ctx.transform() * xf;
    let s = dev.determinant().abs().sqrt();
    let (pw, ph) = def.period();
    let scale = raster_scale(s, (pw, ph));
    let params = (def.tile, def.tile_type, def.overlap);
    let key = (name.to_string(), scale.to_bits(), ink);
    let cms = vectorcraft_color::cms::active();
    let hit = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.check_plugins();
        let tiles = &mut c.tiles[ink.index()];
        let t = tiles.tick();
        tiles
            .get(&key, t)
            .filter(|e| {
                e.params == params
                    && same_art(&e.art, &def.art)
                    && std::ptr::eq(e.cms.as_ptr(), Arc::as_ptr(&cms))
                    && e.deps.as_ref().is_none_or(|d| d.current(doc))
            })
            .map(|e| (e.pixmap.clone(), e.sx, e.sy, e.failed, e.image_left_out))
    });
    let (pm, sx, sy) = match hit {
        Some((pm, sx, sy, failed, image_left_out)) => {
            crate::paint::add_decode_failures(failed);
            if image_left_out {
                IMAGES_LEFT_OUT.with(|n| n.set(n.get().saturating_add(1)));
            }
            (pm, sx, sy)
        }
        None => {
            let w = (pw * scale).ceil().clamp(1.0, 4096.0) as u16;
            let h = (ph * scale).ceil().clamp(1.0, 4096.0) as u16;
            let before = crate::paint::decode_failures();
            let Some((pm, image_left_out)) = with_tile_renderer(|r| rasterize(r, doc, def, Rect::new(0.0, 0.0, pw, ph), w, h, ink)) else {
                ctx.set_paint(missing());
                return true;
            };
            let failed = crate::paint::decode_failures() - before;
            if image_left_out {
                IMAGES_LEFT_OUT.with(|n| n.set(n.get().saturating_add(1)));
            }
            let pm = Arc::new(pm);
            let (sx, sy) = (w as f64 / pw, h as f64 / ph);
            let deps = crate::fx::Reads::of_all(def.art.iter().map(|a| &**a)).any().then(|| Deps::of(doc));
            // A cancelled export drew part of the tile: it isn't kept for later frames.
            CACHE.with(|c| {
                if vectorcraft_doc::cancel::is_cancelled() {
                    return;
                }
                let mut c = c.borrow_mut();
                let tiles = &mut c.tiles[ink.index()];
                let (t, bytes) = (tiles.tick(), size_of::<TileEntry>() + w as usize * h as usize * 4);
                let entry =
                    TileEntry { art: def.art.clone(), deps, cms: Arc::downgrade(&cms), params, sx, sy, pixmap: pm.clone(), failed, image_left_out };
                tiles.insert(key, entry, bytes, t);
            });
            (pm, sx, sy)
        }
    };
    let sampler = ImageSampler { x_extend: Extend::Repeat, y_extend: Extend::Repeat, quality: ImageQuality::Medium, alpha: 1.0 };
    ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(pm), sampler });
    ctx.set_paint_transform(xf * Affine::scale_non_uniform(1.0 / sx, 1.0 / sy));
    true
}

/// A swatch thumbnail of pattern `name`: the tiling around the tile, fitted into `size`².
pub fn render_pattern_swatch(doc: &Document, name: &str, size: u32) -> Option<Rendered> {
    let def = doc.pattern(name)?;
    let (w, h) = (def.width(), def.height());
    let side = w.max(h);
    let region = Rect::from_center_size((w / 2.0, h / 2.0), (side, side));
    let px = size.clamp(1, 1024) as u16;
    let (pm, _) = with_tile_renderer(|r| rasterize(r, doc, def, region, px, px, Ink::Display))?;
    Some(Rendered { width: px as u32, height: px as u32, pixels: pm.data_as_u8_slice().to_vec() })
}

impl Renderer {
    /// Dimmed pattern-editing preview copies at `offsets`.
    fn draw_dimmed_pattern_copies(&mut self, ctx: &mut RenderContext, f: &Frame, dim: f32, offsets: &[Vec2], children: &[Arc<Node>]) {
        if offsets.is_empty() {
            return;
        }
        let comp = crate::group::Composite { opacity: dim, ..Default::default() };
        self.group(ctx, f, comp, &mut |r, c, fr| {
            for o in offsets {
                let frame = Frame { view: fr.view * Affine::translate(*o), visible: fr.visible - *o, ..*fr };
                for a in children {
                    r.draw_arc(c, &frame, a);
                }
            }
        });
    }

    /// Pattern editing mode: draw the tile layer, its dimmed copies and the tile edge instead of
    /// the document. Returns false when not in pattern editing mode.
    pub(crate) fn draw_pattern_edit(&mut self, ctx: &mut RenderContext, f: &Frame) -> bool {
        let Some(pe) = &f.doc.pattern_edit else { return false };
        let Some(layer) = f.doc.layers.iter().find(|l| l.id == pe.layer) else { return false };
        let Some(def) = f.doc.pattern(&pe.pattern) else { return false };
        let children: Vec<Arc<Node>> = layer.children().cloned().unwrap_or_default();
        if layer.visible && !children.is_empty() {
            let dim = (def.dim_copies / 100.0).clamp(0.0, 1.0);
            let (before, after) = def.preview_offsets_dimmed_sides();
            self.draw_dimmed_pattern_copies(ctx, f, dim, &before, &children);
            self.draw_arc(ctx, f, layer);
            self.draw_dimmed_pattern_copies(ctx, f, dim, &after, &children);
        } else {
            self.draw_arc(ctx, f, layer);
        }
        // The tile edge and swatch bounds are editing aids: exports and thumbnails leave them out,
        // as they do guides (#3261).
        if f.opts.skip_templates {
            return true;
        }
        let [r, g, b] = f.opts.tile_edge;
        if def.show_tile_edge {
            self.hairline(ctx, f, &def.tile.to_path(0.1), [r, g, b, 255]);
        }
        if def.show_swatch_bounds {
            let mut bounds = def.swatch_bounds().to_path(0.1);
            bounds.apply_affine(f.view);
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_stroke(vello_cpu::kurbo::Stroke::new(1.0).with_dashes(0.0, [4.0, 3.0]));
            ctx.set_paint(f.ink.fixed([r, g, b, 255]));
            ctx.stroke_path(&bounds);
        }
        true
    }
}

#[cfg(test)]
mod tests;
