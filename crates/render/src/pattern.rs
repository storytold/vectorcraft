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
use vectorcraft_geom::{Affine, Rect, Shape};
use vello_cpu::peniko::{Extend, ImageQuality, ImageSampler};
use vello_cpu::{Pixmap, RenderContext};

use crate::ink::Ink;
use crate::lru::{self, FrameCache};
use crate::{Frame, RenderOptions, Rendered, Renderer};

/// Largest super-tile raster side (pixels).
const MAX_TILE_PX: f64 = 2048.0;

/// What a tile is kept under: the pattern's name and the bits of its raster scale
/// ([`raster_scale`]).
type TileKey = (String, u64);

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
fn rasterize(r: &mut Renderer, doc: &Document, def: &PatternDef, region: Rect, w: u16, h: u16, ink: Ink) -> Pixmap {
    let (w, h) = (w.max(1), h.max(1));
    let mut ctx = crate::single_threaded_context(w, h);
    let view = Affine::scale_non_uniform(w as f64 / region.width().max(1e-9), h as f64 / region.height().max(1e-9))
        * Affine::translate(-region.origin().to_vec2());
    let opts = RenderOptions::default();
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
        };
        for a in &def.art {
            r.draw_arc(&mut ctx, &frame, a);
        }
    }
    r.finish(&mut ctx, r.raster)
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
    let out = f(&mut r);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.depth -= 1;
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
    let key = (name.to_string(), scale.to_bits());
    let cms = vectorcraft_color::cms::active();
    let hit = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.check_plugins();
        let tiles = &mut c.tiles[ink as usize];
        let t = tiles.tick();
        tiles
            .get(&key, t)
            .filter(|e| {
                e.params == params
                    && same_art(&e.art, &def.art)
                    && std::ptr::eq(e.cms.as_ptr(), Arc::as_ptr(&cms))
                    && e.deps.as_ref().is_none_or(|d| d.current(doc))
            })
            .map(|e| (e.pixmap.clone(), e.sx, e.sy))
    });
    let (pm, sx, sy) = match hit {
        Some(h) => h,
        None => {
            let w = (pw * scale).ceil().clamp(1.0, 4096.0) as u16;
            let h = (ph * scale).ceil().clamp(1.0, 4096.0) as u16;
            let Some(pm) = with_tile_renderer(|r| rasterize(r, doc, def, Rect::new(0.0, 0.0, pw, ph), w, h, ink)) else {
                ctx.set_paint(missing());
                return true;
            };
            let pm = Arc::new(pm);
            let (sx, sy) = (w as f64 / pw, h as f64 / ph);
            let deps = crate::fx::Reads::of_all(def.art.iter().map(|a| &**a)).any().then(|| Deps::of(doc));
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                let tiles = &mut c.tiles[ink as usize];
                let (t, bytes) = (tiles.tick(), size_of::<TileEntry>() + w as usize * h as usize * 4);
                tiles.insert(key, TileEntry { art: def.art.clone(), deps, cms: Arc::downgrade(&cms), params, sx, sy, pixmap: pm.clone() }, bytes, t);
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
    let pm = with_tile_renderer(|r| rasterize(r, doc, def, region, px, px, Ink::Display))?;
    Some(Rendered { width: px as u32, height: px as u32, pixels: pm.data_as_u8_slice().to_vec() })
}

impl Renderer {
    /// Pattern editing mode: draw the tile layer, its dimmed copies and the tile edge instead of
    /// the document. Returns false when not in pattern editing mode.
    pub(crate) fn draw_pattern_edit(&mut self, ctx: &mut RenderContext, f: &Frame) -> bool {
        let Some(pe) = &f.doc.pattern_edit else { return false };
        let Some(layer) = f.doc.layers.iter().find(|l| l.id == pe.layer) else { return false };
        let Some(def) = f.doc.pattern(&pe.pattern) else { return false };
        let children: Vec<Arc<Node>> = layer.children().cloned().unwrap_or_default();
        if layer.visible && !children.is_empty() {
            let dim = (def.dim_copies / 100.0).clamp(0.0, 1.0);
            let comp = crate::group::Composite { opacity: dim, ..Default::default() };
            self.group(ctx, f, comp, &mut |r, c, fr| {
                for o in def.preview_offsets() {
                    let frame = Frame { view: fr.view * Affine::translate(o), visible: fr.visible - o, ..*fr };
                    for a in &children {
                        r.draw_arc(c, &frame, a);
                    }
                }
            });
        }
        self.draw_arc(ctx, f, layer);
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
mod tests {
    use super::*;
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::pattern::{PatternEdit, RepeatSpec, pattern_paint};
    use vectorcraft_doc::{Appearance, NodeId, NodeKind};
    use vectorcraft_geom::shapes;

    /// A 20×20 tile with a red 10×10 square in its top-left corner.
    fn doc_with_pattern(tt: TileType) -> Document {
        let mut d = Document::new(200.0, 200.0);
        let red = Node::path(
            NodeId(900),
            shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)),
            Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0),
        );
        let mut def = PatternDef::new("Checks", vec![Arc::new(red)]);
        def.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
        def.tile_type = tt;
        d.patterns.push(def);
        let id = d.alloc_id();
        let n = Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)), Appearance::basic(pattern_paint("Checks"), Paint::None, 0.0));
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        d
    }

    fn render(d: &Document, scale: f64) -> Rendered {
        let px = (200.0 * scale) as u32;
        Renderer::new().render(d, px, px, Affine::scale(scale), &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
    }

    fn is_red(p: [u8; 4]) -> bool {
        p[0] > 200 && p[1] < 60 && p[2] < 60
    }
    fn is_white(p: [u8; 4]) -> bool {
        p[0] > 200 && p[1] > 200 && p[2] > 200
    }

    /// #1536: a raster effect drawn after a pattern fill is the same as without the pattern (the
    /// pattern's paint transform must not resample the effect's raster).
    #[test]
    fn effects_after_a_pattern_fill_keep_their_place_and_size() {
        let fx = |id: &str, params: serde_json::Value| vectorcraft_doc::Effect { id: id.into(), params, visible: true };
        for effect in [
            fx("blur.gaussian", serde_json::json!({"radius": 4.0})),
            fx("stylize.dropShadow", serde_json::json!({"x": 6, "y": 6, "blur": 3, "opacity": 100})),
        ] {
            let make = |with_pattern: bool| {
                let mut d = doc_with_pattern(TileType::Grid);
                let l = d.layers[0].id;
                let filled = d.children(Some(l)).unwrap()[0].id;
                d.remove(filled).unwrap();
                if with_pattern {
                    let id = d.alloc_id();
                    let n = Node::path(
                        id,
                        shapes::rectangle(Rect::new(0.0, 0.0, 80.0, 80.0)),
                        Appearance::basic(pattern_paint("Checks"), Paint::None, 0.0),
                    );
                    d.insert(Some(l), 0, n).unwrap();
                }
                let id = d.alloc_id();
                let mut n = Node::path(
                    id,
                    shapes::ellipse(Rect::new(120.0, 110.0, 180.0, 170.0)),
                    Appearance::basic(Paint::solid(Color::rgb(0.5, 0.5, 0.5)), Paint::None, 0.0),
                );
                n.appearance.effects.push(effect.clone());
                let at = d.children(Some(l)).map_or(0, Vec::len);
                d.insert(Some(l), at, n).unwrap();
                d
            };
            // Supersampled, as raster exports draw.
            let one = |d: &Document| {
                let mut r = Renderer::new();
                r.render(
                    d,
                    200,
                    200,
                    Affine::IDENTITY,
                    &RenderOptions { background: Some([255, 255, 255, 255]), supersample: true, ..Default::default() },
                )
            };
            let (with, without) = (one(&make(true)), one(&make(false)));
            for (x, y) in [(150, 140), (125, 140), (176, 160), (150, 172), (184, 176)] {
                assert_eq!(with.pixel(x, y), without.pixel(x, y), "{} at ({x}, {y})", effect.id);
            }
        }
    }

    #[test]
    fn pattern_fill_is_periodic() {
        let d = doc_with_pattern(TileType::Grid);
        let r = render(&d, 1.0);
        for (x, y) in [(5, 5), (25, 5), (125, 145), (185, 65)] {
            assert!(is_red(r.pixel(x, y)), "({x},{y}) {:?}", r.pixel(x, y));
        }
        for (x, y) in [(15, 5), (5, 15), (135, 135), (175, 195)] {
            assert!(is_white(r.pixel(x, y)), "({x},{y}) {:?}", r.pixel(x, y));
        }
        // Periodic: pixel (x, y) equals (x + 20k, y + 20m).
        for y in (1..20).step_by(3) {
            for x in (1..20).step_by(3) {
                if x == 10 || y == 10 {
                    continue;
                }
                assert_eq!(r.pixel(x, y), r.pixel(x + 60, y + 100));
            }
        }
    }

    #[test]
    fn pattern_fill_is_clipped_to_shape() {
        let mut d = doc_with_pattern(TileType::Grid);
        let id = d.layers[0].children().unwrap()[0].id;
        *d.node_mut(id).unwrap().path_data_mut().unwrap() = shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0));
        let r = render(&d, 1.0);
        assert!(is_red(r.pixel(45, 45)));
        assert!(is_white(r.pixel(65, 65)));
    }

    #[test]
    fn brick_pattern_shifts_rows() {
        let d = doc_with_pattern(TileType::BrickByRow { offset: 0.5 });
        let r = render(&d, 1.0);
        assert!(is_red(r.pixel(5, 5)));
        // Row 1 is shifted by half a tile.
        assert!(is_white(r.pixel(5, 25)) && is_red(r.pixel(15, 25)), "{:?} {:?}", r.pixel(5, 25), r.pixel(15, 25));
        assert!(is_red(r.pixel(5, 45)));
    }

    #[test]
    fn pattern_follows_zoom_and_paint_transform() {
        let d = doc_with_pattern(TileType::Grid);
        let r = render(&d, 2.0);
        assert!(is_red(r.pixel(10, 10)) && is_white(r.pixel(30, 10)) && is_red(r.pixel(50, 10)));
        let mut d2 = d.clone();
        let id = d2.layers[0].children().unwrap()[0].id;
        vectorcraft_doc::pattern::transform_pattern_paints(d2.node_mut(id).unwrap(), Affine::translate((10.0, 0.0)));
        let r = render(&d2, 1.0);
        assert!(is_white(r.pixel(5, 5)) && is_red(r.pixel(15, 5)));
    }

    /// `doc_with_pattern(TileType::Grid)` with a 40×40 square on top of its rectangle, filled with
    /// the same pattern scaled by `s`, as Transform Pattern Tiles leaves it.
    fn doc_with_two_scales(s: f64) -> Document {
        let mut d = doc_with_pattern(TileType::Grid);
        let square = shapes::rectangle(Rect::new(120.0, 120.0, 160.0, 160.0));
        let mut n = Node::path(d.alloc_id(), square, Appearance::basic(pattern_paint("Checks"), Paint::None, 0.0));
        vectorcraft_doc::pattern::transform_pattern_paints(&mut n, Affine::scale(s));
        let l = d.layers[0].id;
        d.insert(Some(l), 1, n).unwrap();
        d
    }

    /// `d` drawn by `r` at `scale`.
    fn frame(r: &mut Renderer, d: &Document, scale: f64) -> Rendered {
        let px = (200.0 * scale) as u32;
        r.render(d, px, px, Affine::scale(scale), &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
    }

    /// The tiles rasterized on this thread so far: the tile renderer counts a frame for each.
    fn tiles_rasterized() -> u64 {
        CACHE.with(|c| c.borrow().renderer.as_ref().map_or(0, |r| r.stamp))
    }

    #[test]
    fn objects_at_two_pattern_scales_keep_their_tiles_between_frames() {
        // At 100% the rectangle's tile is rasterized at scale 1, and the square's tile is
        // rasterized at 0.25, or at 2 when its tiles are scaled to 101%.
        for s in [0.25, 1.01] {
            let d = doc_with_two_scales(s);
            let mut r = Renderer::new();
            let start = tiles_rasterized();
            let first = frame(&mut r, &d, 1.0).pixels;
            assert_eq!(tiles_rasterized() - start, 2, "tiles scaled by {s}: a tile for each object");
            for _ in 0..3 {
                assert!(frame(&mut r, &d, 1.0).pixels == first, "tiles scaled by {s}");
            }
            assert_eq!(tiles_rasterized() - start, 2, "tiles scaled by {s}: later frames draw both objects from their tiles");
            // After a frame at 200%, the next frame at 100% draws from the tiles made at 100%.
            frame(&mut r, &d, 2.0);
            let zoomed = tiles_rasterized();
            assert!(frame(&mut r, &d, 1.0).pixels == first, "tiles scaled by {s}");
            assert_eq!(tiles_rasterized(), zoomed, "tiles scaled by {s}: back at 100%");
        }
    }

    #[test]
    fn kept_tiles_draw_what_a_new_renderer_draws() {
        // A new renderer on a thread of its own starts with no tiles.
        let fresh = |d: &Document, zoom: f64| {
            let d = d.clone();
            std::thread::spawn(move || frame(&mut Renderer::new(), &d, zoom).pixels).join().unwrap()
        };
        let mut d = doc_with_two_scales(0.25);
        let mut r = Renderer::new();
        // Pixel (130, 130) is red in the square's tiles and clear in the rectangle's, (133, 133) is
        // clear in both, and (103, 103), outside the square, is red in the rectangle's.
        let first = frame(&mut r, &d, 1.0);
        assert!(is_red(first.pixel(130, 130)) && is_white(first.pixel(133, 133)) && is_red(first.pixel(103, 103)));
        for zoom in [1.0, 2.0, 1.0, 0.5, 2.0, 1.0] {
            assert!(frame(&mut r, &d, zoom).pixels == fresh(&d, zoom), "zoom {zoom}");
        }
        // Change the tile type as Pattern Options does.
        d.patterns[0].tile_type = TileType::BrickByRow { offset: 0.5 };
        for zoom in [2.0, 1.0] {
            assert!(frame(&mut r, &d, zoom).pixels == fresh(&d, zoom), "brick, zoom {zoom}");
        }
        // Replace the art as Done Editing Pattern does.
        let blue = Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0);
        d.patterns[0].art = vec![Arc::new(Node::path(NodeId(901), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), blue))];
        for zoom in [1.0, 2.0] {
            assert!(frame(&mut r, &d, zoom).pixels == fresh(&d, zoom), "new art, zoom {zoom}");
        }
    }

    #[test]
    fn kept_tiles_stay_within_the_budget() {
        // The rectangle's tile is 20 pixels a side per 100% of zoom, and the square's tile is 5
        // pixels a side per 100% of zoom. The budget holds the rectangle's tile at 400% and both
        // tiles at 200%; the tiles drawn from 100% to 400% need more.
        let d = doc_with_two_scales(0.25);
        let tile = |side: usize| size_of::<TileEntry>() + side * side * 4;
        let budget = tile(80) + tile(40) + tile(10);
        CACHE.with(|c| c.borrow_mut().tiles[Ink::Display as usize] = FrameCache::new(budget, usize::MAX));
        let kept = || {
            CACHE.with(|c| {
                let c = c.borrow();
                let tiles = &c.tiles[Ink::Display as usize];
                (tiles.len(), tiles.bytes())
            })
        };
        let mut r = Renderer::new();
        for zoom in [1.0, 2.0, 4.0] {
            frame(&mut r, &d, zoom);
            let drawn = tiles_rasterized();
            frame(&mut r, &d, zoom);
            assert_eq!(tiles_rasterized(), drawn, "zoom {zoom}: the next frame draws from the tiles just made");
            let (len, bytes) = kept();
            assert!(len >= 2 && bytes <= budget, "zoom {zoom}: {len} tiles, {bytes} bytes");
        }
        // Tiles were rasterized at five scales (0.25, 0.5, 1, 2 and 4); the cache dropped the least
        // recently used tiles first.
        assert!(kept().0 < 5, "{:?}", kept());
        let before = tiles_rasterized();
        frame(&mut r, &d, 1.0);
        assert!(tiles_rasterized() > before, "a tile of 100% was dropped");
        assert!(kept().1 <= budget, "{:?}", kept());
    }

    #[test]
    fn missing_pattern_is_grey() {
        let mut d = doc_with_pattern(TileType::Grid);
        d.patterns.clear();
        let p = render(&d, 1.0).pixel(50, 50);
        assert_eq!(p, [128, 128, 128, 255]);
    }

    #[test]
    fn swatch_thumbnail_renders() {
        let d = doc_with_pattern(TileType::Grid);
        let r = render_pattern_swatch(&d, "Checks", 20).unwrap();
        assert!(is_red(r.pixel(3, 3)));
        assert_eq!(r.pixel(15, 15)[3], 0);
        assert!(render_pattern_swatch(&d, "Nope", 20).is_none());
    }

    #[test]
    fn edit_mode_draws_only_tile_layer_with_copies() {
        let mut d = doc_with_pattern(TileType::Grid);
        let l = d.add_layer(Some("Pattern Editing Mode"));
        let art = d.patterns[0].art[0].clone();
        d.insert(Some(l), 0, (*art).clone()).unwrap();
        d.pattern_edit = Some(PatternEdit { pattern: "Checks".into(), layer: l, original: None });
        d.patterns[0].dim_copies = 50.0;
        d.patterns[0].show_tile_edge = false;
        let r = render(&d, 1.0);
        // The tile itself at full strength, copies dimmed, the filled document rect hidden.
        assert!(is_red(r.pixel(5, 5)));
        let c = r.pixel(25, 5);
        assert!(c[0] > 200 && c[1] > 100 && c[1] < 160, "{c:?}");
        assert!(is_white(r.pixel(15, 5)));
        assert!(is_white(r.pixel(125, 125)), "outside the copies");
    }

    #[test]
    fn repeat_renders_instances() {
        let mut d = Document::new(200.0, 200.0);
        let sq = Node::path(
            NodeId(50),
            shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        let id = d.alloc_id();
        let l = d.layers[0].id;
        d.insert(Some(l), 0, Node::new(id, NodeKind::Repeat(RepeatSpec::grid(vec![Arc::new(sq)], 10.0, 10.0)))).unwrap();
        let r = render(&d, 1.0);
        for (x, y) in [(5, 5), (25, 5), (45, 45)] {
            assert_eq!(r.pixel(x, y), [0, 0, 0, 255], "({x},{y})");
        }
        assert!(is_white(r.pixel(15, 5)) && is_white(r.pixel(65, 5)));
    }

    #[test]
    fn raster_scale_is_power_of_two_and_capped() {
        assert_eq!(raster_scale(1.0, (10.0, 10.0)), 1.0);
        assert_eq!(raster_scale(1.5, (10.0, 10.0)), 2.0);
        assert_eq!(raster_scale(3.0, (10.0, 10.0)), 4.0);
        // A change within 2× keeps the raster.
        assert_eq!(raster_scale(2.1, (10.0, 10.0)), raster_scale(3.9, (10.0, 10.0)));
        assert!(raster_scale(64.0, (1000.0, 10.0)) * 1000.0 <= MAX_TILE_PX);
    }
}
