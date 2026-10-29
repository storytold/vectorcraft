//! Transparency groups: groups with opacity or a blend mode, clip groups, opacity masks, the page
//! group. Most are vello layers, which are isolated (their content starts from transparency). A
//! non-isolated group whose content blends is drawn offscreen over a copy of its backdrop, so its
//! children blend with the art below it, then mixed back in (backdrop-copy compositing).

use std::sync::Arc;

use vectorcraft_color::BlendMode as Blend;
use vectorcraft_doc::Node;
use vectorcraft_geom::{Affine, BezPath, FillRule, Rect};
use vello_cpu::peniko::{self, BlendMode, Compose, Mix};
use vello_cpu::{Pixmap, RenderContext};

use crate::{Frame, Renderer, blend_mode, fill_rule};

/// How a group composites onto what is below it.
pub(crate) struct Composite<'a> {
    /// Clipping path (document space).
    pub clip: Option<(&'a BezPath, FillRule)>,
    pub blend: Blend,
    pub opacity: f32,
    /// Opacity mask: the coverage of each pixel (row-major) of the region `bounds` covers
    /// ([`area`]). A masked group is drawn offscreen over that region only.
    pub mask: Option<Vec<u8>>,
    /// The content starts from transparency instead of from the art below.
    pub isolated: bool,
    /// Blending inside reaches the group's backdrop ([`Node::blends_through`]).
    pub blends: bool,
    /// Area the content covers (document space); `None` = anywhere.
    pub bounds: Option<Rect>,
}

impl Default for Composite<'_> {
    fn default() -> Self {
        Self { clip: None, blend: Blend::Normal, opacity: 1.0, mask: None, isolated: false, blends: false, bounds: None }
    }
}

/// Group content drawn into a context (called once, or twice for a non-isolated group with a blend
/// mode: over the backdrop copy, then alone for its shape).
pub(crate) type Content<'d> = dyn FnMut(&mut Renderer, &mut RenderContext, &Frame) + 'd;

/// A pixel region of a context: origin and size.
#[derive(Clone, Copy)]
pub(crate) struct Region {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

/// Groups drawn twice ([`Renderer::group_offscreen`]: blending with their backdrop) that may be
/// nested in each other before deeper ones are drawn isolated, once.
const MAX_TWICE_DRAWN: u32 = 4;

impl Renderer {
    /// Draw a group's content with `draw` and composite it as `c` says. Blending content is drawn
    /// offscreen: over a copy of the backdrop when the group isn't isolated (possible only with
    /// no layer open around it, else it is drawn isolated), from transparency when it is (so
    /// non-isolated groups inside can copy their backdrop).
    pub(crate) fn group(&mut self, ctx: &mut RenderContext, f: &Frame, c: Composite, draw: &mut Content) {
        if c.blends
            && !c.isolated
            && c.opacity >= 1.0
            && c.blend == Blend::Normal
            && c.mask.is_none()
            && let Some((path, rule)) = c.clip
        {
            // A plain clip clips each drawing instead of compositing a layer: blending inside
            // reaches the art below directly.
            ctx.set_transform(f.view);
            ctx.set_fill_rule(fill_rule(rule));
            ctx.push_clip_path(path);
            self.clip_paths.push((path.clone(), rule, f.view));
            draw(self, ctx, f);
            self.clip_paths.pop();
            ctx.pop_clip_path();
            return;
        }
        if c.blends && (c.isolated || self.nested == 0) {
            let Some(r) = region(ctx, f, c.bounds) else { return };
            // Past the budget for offscreen pixels, the group is drawn as a layer (isolated).
            // A masked group is drawn offscreen whatever the budget: its mask scales its pixels.
            if c.mask.is_some() || self.offscreen_fits(ctx, r, c.isolated) {
                return self.group_offscreen(ctx, f, c, r, draw);
            }
        }
        if c.mask.is_some() {
            // Its backdrop out of reach (or not needed), a masked group starts from transparency.
            let Some(r) = region(ctx, f, c.bounds) else { return };
            return self.group_offscreen(ctx, f, Composite { isolated: true, ..c }, r, draw);
        }
        ctx.set_transform(f.view);
        if let Some((_, rule)) = c.clip {
            ctx.set_fill_rule(fill_rule(rule));
        }
        ctx.push_layer(c.clip.map(|(p, _)| p), Some(blend_mode(c.blend)), Some(c.opacity), None, None);
        self.nested += 1;
        // The layer starts from transparency: no backdrop for the content.
        let backdrop = self.backdrop.take();
        draw(self, ctx, f);
        self.backdrop = backdrop;
        self.nested -= 1;
        ctx.pop_layer();
    }

    /// Whether a group drawn offscreen over region `r` keeps the offscreen pixels alive within
    /// [`OFFSCREEN_FRAMES`] times the frame `ctx` (the frame's own context when none is alive).
    fn offscreen_fits(&self, ctx: &RenderContext, r: Region, isolated: bool) -> bool {
        let budget =
            if self.offscreen_live == 0 { OFFSCREEN_FRAMES * u64::from(ctx.width()) * u64::from(ctx.height()) } else { self.offscreen_budget };
        self.offscreen_live.saturating_add(offscreen_cost(r, isolated)) <= budget
    }

    fn group_offscreen(&mut self, ctx: &mut RenderContext, f: &Frame, c: Composite, r: Region, draw: &mut Content) {
        let Composite { clip, blend, opacity, mask, isolated, .. } = c;
        // A group that blends with its backdrop is drawn twice (over the backdrop, and alone for
        // its coverage), and so is everything in it, so such groups nested in each other cost 2ⁿ.
        // Past [`MAX_TWICE_DRAWN`] of them, a deeper one is drawn as an isolated group.
        let twice = !isolated && blend != Blend::Normal;
        let isolated = isolated || twice && self.twice_drawn >= MAX_TWICE_DRAWN;
        let twice = twice && !isolated;
        if self.offscreen_live == 0 {
            self.offscreen_budget = OFFSCREEN_FRAMES * u64::from(ctx.width()) * u64::from(ctx.height());
        }
        let cost = offscreen_cost(r, isolated);
        self.offscreen_live = self.offscreen_live.saturating_add(cost);
        let sub = frame_over(f, r);
        let backdrop = (!isolated).then(|| self.flatten(ctx, r));
        self.twice_drawn += u32::from(twice);
        let content = self.offscreen(r, &sub, backdrop.clone(), draw);
        let at = (r.x as f64, r.y as f64);
        let push = |ctx: &mut RenderContext, mode: BlendMode| {
            ctx.set_transform(f.view);
            if let Some((_, rule)) = clip {
                ctx.set_fill_rule(fill_rule(rule));
            }
            ctx.push_layer(clip.map(|(p, _)| p), Some(mode), Some(opacity), None, None);
        };
        // The mask scales what the group draws, as a layer mask does.
        let masked = |pm: Pixmap| match &mask {
            Some(m) => apply_mask(pm, m),
            None => pm,
        };
        match backdrop {
            // The backdrop under the content, mixed with it by opacity, mask and clip: take that
            // share of the backdrop out, then add the same share of the content.
            Some(_) if blend == Blend::Normal => {
                push(ctx, BlendMode::new(Mix::Normal, Compose::DestOut));
                if mask.is_some() {
                    let mut black = Pixmap::new(r.w, r.h);
                    black.data_mut().iter_mut().for_each(|p| p.a = 255);
                    draw_pixmap(ctx, Arc::new(masked(black)), at);
                } else {
                    ctx.set_transform(Affine::IDENTITY);
                    ctx.set_paint(peniko::Color::BLACK);
                    ctx.fill_rect(&Rect::new(at.0, at.1, at.0 + r.w as f64, at.1 + r.h as f64));
                }
                ctx.pop_layer();
                push(ctx, BlendMode::new(Mix::Normal, Compose::Plus));
                draw_pixmap(ctx, Arc::new(masked(content)), at);
            }
            // A blend mode applies to the group as an object of its own: take the backdrop's share
            // out of the content (it needs the content's own coverage), then blend the rest.
            Some(b) => {
                let alone = self.offscreen(r, &sub, None, draw);
                push(ctx, blend_mode(blend));
                draw_pixmap(ctx, Arc::new(masked(remove_backdrop(content, &alone, &b))), at);
            }
            None => {
                push(ctx, blend_mode(blend));
                draw_pixmap(ctx, Arc::new(masked(content)), at);
            }
        }
        ctx.pop_layer();
        self.twice_drawn -= u32::from(twice);
        self.offscreen_live = self.offscreen_live.saturating_sub(cost);
    }

    /// Render what `ctx` holds so far, and start it again from that picture (so later drawing sees
    /// it unchanged). Returns the pixels of `r`.
    fn flatten(&mut self, ctx: &mut RenderContext, r: Region) -> Arc<Pixmap> {
        let all = self.finish(ctx, self.raster);
        ctx.reset();
        let crop = if (r.x, r.y, r.w, r.h) == (0, 0, all.width(), all.height()) { None } else { Some(Arc::new(cropped(&all, r))) };
        let all = Arc::new(all);
        // With 8-bit compositing, rendering over the picture gives the pixels that drawing it back
        // as an image gives: the context starts again empty, and `finish` renders what it draws
        // next over the picture. With floating-point compositing the picture is drawn back.
        if self.raster.render_mode == vello_cpu::RenderMode::OptimizeSpeed {
            self.picture = Some(all.clone());
        } else {
            draw_pixmap(ctx, all.clone(), (0.0, 0.0));
        }
        for (path, rule, xf) in &self.clip_paths {
            ctx.set_transform(*xf);
            ctx.set_fill_rule(fill_rule(*rule));
            ctx.push_clip_path(path);
        }
        crop.unwrap_or(all)
    }

    /// Render what `ctx` holds into a pixmap of its size: over the picture [`Self::flatten`]
    /// started it again from, if any, else over transparency.
    pub(crate) fn finish(&mut self, ctx: &mut RenderContext, settings: vello_cpu::RasterizerSettings) -> Pixmap {
        ctx.flush();
        let Some(below) = self.picture.take() else {
            let mut pm = Pixmap::new(ctx.width(), ctx.height());
            ctx.render_with(&mut pm, &mut self.resources, settings);
            return pm;
        };
        let mut pm = Arc::unwrap_or_clone(below);
        let over = vello_cpu::RasterizerSettings { composite_mode: vello_cpu::CompositeMode::SrcOver, ..settings };
        ctx.render_with(&mut pm, &mut self.resources, over);
        pm
    }

    /// `draw` rendered into a new context of region `r` (frame `sub`), over `backdrop` if any.
    fn offscreen(&mut self, r: Region, sub: &Frame, backdrop: Option<Arc<Pixmap>>, draw: &mut Content) -> Pixmap {
        let mut ctx = sub.offscreen_context(r.w, r.h);
        if let Some(b) = &backdrop {
            draw_pixmap(&mut ctx, b.clone(), (0.0, 0.0));
        }
        let outer = (
            std::mem::replace(&mut self.nested, 0),
            std::mem::replace(&mut self.backdrop, backdrop),
            std::mem::take(&mut self.clip_paths),
            self.picture.take(),
        );
        draw(self, &mut ctx, sub);
        let pm = self.finish(&mut ctx, self.raster);
        (self.nested, self.backdrop, self.clip_paths, self.picture) = outer;
        pm
    }

    /// Run `draw` inside layers (or into another context) it opens itself: groups drawn there can't
    /// copy their backdrop, and see none.
    pub(crate) fn inside_layer<T>(&mut self, draw: impl FnOnce(&mut Self) -> T) -> T {
        self.nested += 1;
        let backdrop = self.backdrop.take();
        let out = draw(self);
        self.backdrop = backdrop;
        self.nested -= 1;
        out
    }

    /// Whether blending inside `n` reaches its backdrop ([`Node::blends_through`], cached per
    /// container).
    pub(crate) fn blends_through(&mut self, n: &Node) -> bool {
        n.blends_through_with(&mut |c| self.blends_of(c))
    }

    /// [`Self::blends_through`] of the group `children` make up.
    pub(crate) fn children_blend(&mut self, children: &[Arc<Node>]) -> bool {
        Node::children_blend(children, &mut |c| self.blends_of(c))
    }

    fn blends_of(&mut self, a: &Arc<Node>) -> bool {
        if !a.is_container() {
            return a.blends_through();
        }
        let key = Arc::as_ptr(a) as usize;
        if let Some(e) = self.blends.get_mut(&key)
            && Arc::ptr_eq(&e.0, a)
        {
            e.2 = self.stamp;
            return e.1;
        }
        let v = a.blends_through_with(&mut |c| self.blends_of(c));
        if self.blends.len() > 1024 {
            let g = self.stamp;
            self.blends.retain(|_, e| g - e.2 <= 3);
        }
        self.blends.insert(key, (a.clone(), v, self.stamp));
        v
    }

    /// Draw knockout element `c` in a layer composited with `compose`: as its knockout shape
    /// (`DestOut` erases what the elements below drew there, `SrcOver` draws the shape), or as
    /// itself (`Plus` adds it).
    pub(crate) fn knockout_pass(&mut self, ctx: &mut RenderContext, f: &Frame, c: &Arc<Node>, compose: Compose) {
        ctx.set_transform(Affine::IDENTITY);
        ctx.push_layer(None, Some(BlendMode::new(Mix::Normal, compose)), None, None, None);
        self.nested += 1;
        let shape = compose != Compose::Plus && !c.knockout_shape;
        self.shape_of = if shape { Arc::as_ptr(c) as usize } else { 0 };
        self.draw_arc(ctx, f, c);
        self.shape_of = 0;
        self.nested -= 1;
        ctx.pop_layer();
    }

    /// Add knockout element `c` of a non-isolated knockout group drawn over a copy of its backdrop
    /// (`backdrop`, in `ctx`'s pixels), once its `DestOut` pass has erased what the elements below
    /// it drew: `c` composited with the backdrop at a shape of 1, times its knockout shape
    /// (ISO 32000-1 §11.4.6). That is `c` drawn over the backdrop, less (1 − shape) × backdrop.
    /// `c` over the backdrop and its shape are drawn offscreen over `c`'s bounds, and `c` is drawn
    /// as in the other knockout passes (groups in it can't copy their backdrop).
    pub(crate) fn knockout_over(&mut self, ctx: &mut RenderContext, f: &Frame, c: &Arc<Node>, backdrop: &Arc<Pixmap>) {
        let bounds = self.bounds_of(f.symbols, c);
        let Some(r) = region(ctx, f, bounds) else { return };
        let view = Affine::translate((-(r.x as f64), -(r.y as f64))) * f.view;
        let visible = view.inverse().transform_rect_bbox(Rect::new(0.0, 0.0, r.w as f64, r.h as f64));
        let sub = Frame { mt: false, view, visible, ..*f };
        let whole = (r.x, r.y, r.w, r.h) == (0, 0, backdrop.width(), backdrop.height());
        let below = if whole { backdrop.clone() } else { Arc::new(cropped(backdrop, r)) };
        let over = self.offscreen(r, &sub, Some(below.clone()), &mut |s, cx, fr| s.inside_layer(|s| s.draw_arc(cx, fr, c)));
        let shape = self.offscreen(r, &sub, None, &mut |s, cx, fr| s.knockout_pass(cx, fr, c, Compose::SrcOver));
        ctx.set_transform(Affine::IDENTITY);
        ctx.push_layer(None, Some(BlendMode::new(Mix::Normal, Compose::Plus)), None, None, None);
        draw_pixmap(ctx, Arc::new(inside_shape(over, &shape, &below)), (r.x as f64, r.y as f64));
        ctx.pop_layer();
    }
}

/// Draw `pm` with its top-left corner at pixel `at` (whole pixels: copied exactly).
pub(crate) fn draw_pixmap(ctx: &mut RenderContext, pm: Arc<Pixmap>, at: (f64, f64)) {
    let (w, h) = (pm.width() as f64, pm.height() as f64);
    #[cfg(test)]
    PIXELS_DRAWN.with(|n| n.set(n.get() + u64::from(pm.width()) * u64::from(pm.height())));
    ctx.set_transform(Affine::translate(at));
    // The pixmap maps 1:1 onto the rect: a paint transform left by an earlier fill (a pattern,
    // radial or freeform gradient) would resample it (#2426).
    ctx.reset_paint_transform();
    ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(pm), sampler: peniko::ImageSampler::default() });
    ctx.fill_rect(&Rect::new(0.0, 0.0, w, h));
    ctx.set_transform(Affine::IDENTITY);
}

#[cfg(test)]
thread_local! {
    /// The pixels [`draw_pixmap`] has drawn on this thread.
    pub(crate) static PIXELS_DRAWN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The offscreen pixels that groups drawn offscreen may keep alive at once, in frames: deeper
/// nested blending groups are drawn as layers instead (#3581).
const OFFSCREEN_FRAMES: u64 = 4;

/// The pixels a group drawn offscreen over `r` keeps alive: its content, and when it isn't
/// isolated the backdrop copy and the content drawn alone.
fn offscreen_cost(r: Region, isolated: bool) -> u64 {
    u64::from(r.w) * u64::from(r.h) * if isolated { 1 } else { 3 }
}

/// The pixel region of `ctx` that `bounds` (document space) covers, with a margin for
/// antialiasing; `None` when it is off the context.
fn region(ctx: &RenderContext, f: &Frame, bounds: Option<Rect>) -> Option<Region> {
    let full = Rect::new(0.0, 0.0, ctx.width() as f64, ctx.height() as f64);
    let r = match bounds {
        Some(b) => f.view.transform_rect_bbox(b).inflate(2.0, 2.0).intersect(full),
        None => full,
    };
    let (x0, y0, x1, y1) = (r.x0.floor().max(0.0), r.y0.floor().max(0.0), r.x1.ceil().min(full.x1), r.y1.ceil().min(full.y1));
    (x1 > x0 && y1 > y0).then_some(Region { x: x0 as u16, y: y0 as u16, w: (x1 - x0) as u16, h: (y1 - y0) as u16 })
}

/// The pixel region of `ctx` that `bounds` covers ([`region`]), and the frame that draws into a
/// context of that region's size.
pub(crate) fn area<'a>(ctx: &RenderContext, f: &Frame<'a>, bounds: Option<Rect>) -> Option<(Region, Frame<'a>)> {
    let r = region(ctx, f, bounds)?;
    Some((r, frame_over(f, r)))
}

/// Frame `f` drawing into a context of region `r`'s size (on the calling thread).
fn frame_over<'a>(f: &Frame<'a>, r: Region) -> Frame<'a> {
    let view = Affine::translate((-(r.x as f64), -(r.y as f64))) * f.view;
    let visible = view.inverse().transform_rect_bbox(Rect::new(0.0, 0.0, r.w as f64, r.h as f64));
    Frame { mt: false, view, visible, ..*f }
}

/// `pm` (premultiplied) with each pixel scaled by its coverage in `mask` (row-major, `pm`'s size).
fn apply_mask(mut pm: Pixmap, mask: &[u8]) -> Pixmap {
    let scale = |v: u8, m: u8| ((v as u32 * m as u32 + 127) / 255) as u8;
    for (p, &m) in pm.data_mut().iter_mut().zip(mask) {
        (p.r, p.g, p.b, p.a) = (scale(p.r, m), scale(p.g, m), scale(p.b, m), scale(p.a, m));
    }
    pm
}

/// The pixels of region `r` of `pm`.
fn cropped(pm: &Pixmap, r: Region) -> Pixmap {
    let mut out = Pixmap::new(r.w, r.h);
    let (src_w, w) = (pm.width() as usize, r.w as usize);
    for row in 0..r.h as usize {
        let from = (r.y as usize + row) * src_w + r.x as usize;
        out.data_mut()[row * w..(row + 1) * w].copy_from_slice(&pm.data()[from..from + w]);
    }
    out
}

/// The group's own colour from `content` (the group drawn over `backdrop`) and `alone` (drawn
/// over transparency, for its coverage): content = group + (1 − group alpha) × backdrop, all
/// premultiplied.
fn remove_backdrop(mut content: Pixmap, alone: &Pixmap, backdrop: &Pixmap) -> Pixmap {
    for ((c, a), b) in content.data_mut().iter_mut().zip(alone.data()).zip(backdrop.data()) {
        let keep = 255 - a.a as u32;
        let sub = |v: u8, bv: u8| (v as u32).saturating_sub((keep * bv as u32 + 127) / 255).min(a.a as u32) as u8;
        (c.r, c.g, c.b, c.a) = (sub(c.r, b.r), sub(c.g, b.g), sub(c.b, b.b), a.a);
    }
    content
}

/// What a knockout element adds once its knockout shape has erased what was below it: `over` (the
/// element drawn over `backdrop`) less (1 − shape alpha) × `backdrop`, with `shape` the knockout
/// shape, all premultiplied. Each color is kept within the alpha left.
fn inside_shape(mut over: Pixmap, shape: &Pixmap, backdrop: &Pixmap) -> Pixmap {
    for ((o, s), b) in over.data_mut().iter_mut().zip(shape.data()).zip(backdrop.data()) {
        let outside = 255 - s.a as u32;
        let sub = |v: u8, bv: u8| (v as u32).saturating_sub((outside * bv as u32 + 127) / 255);
        let a = sub(o.a, b.a);
        (o.r, o.g, o.b, o.a) = (sub(o.r, b.r).min(a) as u8, sub(o.g, b.g).min(a) as u8, sub(o.b, b.b).min(a) as u8, a as u8);
    }
    over
}
