//! Pattern fills and the pattern-editing preview.
//!
//! A pattern paint is drawn as a repeating image: the tiling's rectangular super-tile
//! ([`PatternDef::period`]) is rasterized once at the current device scale (rounded to a power of
//! two, so it is re-rasterized only when the zoom changes by more than 2×) and used as an image
//! paint with `Extend::Repeat`, with the paint transform mapping pixels → pattern space →
//! document. Tiles are cached per pattern name (and ink plane, see [`crate::ink`]) and invalidated
//! when the definition changes (art `Arc` identity, tile, tile type, overlap) — the pattern's
//! "revision".
//!
//! Pattern editing mode (`Document::pattern_edit`) draws only the temporary tile layer, with
//! dimmed copies of it at the neighbouring tile positions and the tile edge.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use vectorcraft_doc::pattern::{Overlap, PatternDef, TileType};
use vectorcraft_doc::{Document, Node};
use vectorcraft_geom::{Affine, Rect, Shape};
use vello_cpu::peniko::{Extend, ImageQuality, ImageSampler};
use vello_cpu::{Pixmap, RenderContext};

use crate::ink::Ink;
use crate::{Frame, RenderOptions, Rendered, Renderer};

/// Largest super-tile raster side (pixels).
const MAX_TILE_PX: f64 = 2048.0;

struct TileEntry {
    art: Vec<Arc<Node>>,
    params: (Rect, TileType, Overlap),
    scale: f64,
    sx: f64,
    sy: f64,
    pixmap: Arc<Pixmap>,
}

#[derive(Default)]
struct PatternCache {
    /// Per ink ([`Ink`] as index).
    tiles: [HashMap<String, TileEntry>; 3],
    renderer: Option<Box<Renderer>>,
    depth: u32,
}

thread_local! {
    static CACHE: RefCell<PatternCache> = RefCell::new(PatternCache::default());
}

fn same_art(a: &[Arc<Node>], b: &[Arc<Node>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| Arc::ptr_eq(x, y))
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
    r.stamp += 1;
    let px_rect = Rect::new(0.0, 0.0, w as f64, h as f64);
    for o in def.offsets_covering(region) {
        let v = view * def.instance_xf(o);
        let frame = Frame {
            mt: false,
            doc,
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
    ctx.flush();
    let mut pm = Pixmap::new(w, h);
    ctx.render_with(&mut pm, &mut r.resources, r.raster);
    pm
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
    let hit = CACHE.with(|c| {
        c.borrow().tiles[ink as usize]
            .get(name)
            .filter(|e| e.scale == scale && e.params == params && same_art(&e.art, &def.art))
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
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                let tiles = &mut c.tiles[ink as usize];
                if tiles.len() > 64 {
                    tiles.clear();
                }
                tiles.insert(name.to_string(), TileEntry { art: def.art.clone(), params, scale, sx, sy, pixmap: pm.clone() });
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
