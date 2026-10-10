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
    /// Opacity mask (the context's size).
    pub mask: Option<vello_cpu::Mask>,
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
struct Region {
    x: u16,
    y: u16,
    w: u16,
    h: u16,
}

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
            return self.group_offscreen(ctx, f, c, draw);
        }
        ctx.set_transform(f.view);
        if let Some((_, rule)) = c.clip {
            ctx.set_fill_rule(fill_rule(rule));
        }
        ctx.push_layer(c.clip.map(|(p, _)| p), Some(blend_mode(c.blend)), Some(c.opacity), c.mask, None);
        self.nested += 1;
        // The layer starts from transparency: no backdrop for the content.
        let backdrop = self.backdrop.take();
        draw(self, ctx, f);
        self.backdrop = backdrop;
        self.nested -= 1;
        ctx.pop_layer();
    }

    fn group_offscreen(&mut self, ctx: &mut RenderContext, f: &Frame, c: Composite, draw: &mut Content) {
        let Composite { clip, blend, opacity, mask, isolated, bounds, .. } = c;
        let Some(r) = region(ctx, f, bounds) else { return };
        let view = Affine::translate((-(r.x as f64), -(r.y as f64))) * f.view;
        let visible = view.inverse().transform_rect_bbox(Rect::new(0.0, 0.0, r.w as f64, r.h as f64));
        let sub = Frame { mt: false, view, visible, ..*f };
        let backdrop = (!isolated).then(|| self.flatten(ctx, r));
        let content = self.offscreen(r, &sub, backdrop.clone(), draw);
        let at = (r.x as f64, r.y as f64);
        let push = |ctx: &mut RenderContext, mode: BlendMode, mask: Option<vello_cpu::Mask>| {
            ctx.set_transform(f.view);
            if let Some((_, rule)) = clip {
                ctx.set_fill_rule(fill_rule(rule));
            }
            ctx.push_layer(clip.map(|(p, _)| p), Some(mode), Some(opacity), mask, None);
        };
        match backdrop {
            // The backdrop under the content, mixed with it by opacity, mask and clip: take that
            // share of the backdrop out, then add the same share of the content.
            Some(_) if blend == Blend::Normal => {
                push(ctx, BlendMode::new(Mix::Normal, Compose::DestOut), mask.clone());
                ctx.set_transform(Affine::IDENTITY);
                ctx.set_paint(peniko::Color::BLACK);
                ctx.fill_rect(&Rect::new(at.0, at.1, at.0 + r.w as f64, at.1 + r.h as f64));
                ctx.pop_layer();
                push(ctx, BlendMode::new(Mix::Normal, Compose::Plus), mask);
                draw_pixmap(ctx, Arc::new(content), at);
            }
            // A blend mode applies to the group as an object of its own: take the backdrop's share
            // out of the content (it needs the content's own coverage), then blend the rest.
            Some(b) => {
                let alone = self.offscreen(r, &sub, None, draw);
                push(ctx, blend_mode(blend), mask);
                draw_pixmap(ctx, Arc::new(remove_backdrop(content, &alone, &b)), at);
            }
            None => {
                push(ctx, blend_mode(blend), mask);
                draw_pixmap(ctx, Arc::new(content), at);
            }
        }
        ctx.pop_layer();
    }

    /// Render what `ctx` holds so far, and start it again from that picture (so later drawing sees
    /// it unchanged). Returns the pixels of `r`.
    fn flatten(&mut self, ctx: &mut RenderContext, r: Region) -> Arc<Pixmap> {
        ctx.flush();
        let mut all = Pixmap::new(ctx.width(), ctx.height());
        ctx.render_with(&mut all, &mut self.resources, self.raster);
        ctx.reset();
        let crop = if (r.x, r.y, r.w, r.h) == (0, 0, all.width(), all.height()) {
            None
        } else {
            let mut out = Pixmap::new(r.w, r.h);
            let (src_w, w) = (all.width() as usize, r.w as usize);
            for row in 0..r.h as usize {
                let from = (r.y as usize + row) * src_w + r.x as usize;
                out.data_mut()[row * w..(row + 1) * w].copy_from_slice(&all.data()[from..from + w]);
            }
            Some(Arc::new(out))
        };
        let all = Arc::new(all);
        draw_pixmap(ctx, all.clone(), (0.0, 0.0));
        for (path, rule, xf) in &self.clip_paths {
            ctx.set_transform(*xf);
            ctx.set_fill_rule(fill_rule(*rule));
            ctx.push_clip_path(path);
        }
        crop.unwrap_or(all)
    }

    /// `draw` rendered into a new context of region `r` (frame `sub`), over `backdrop` if any.
    fn offscreen(&mut self, r: Region, sub: &Frame, backdrop: Option<Arc<Pixmap>>, draw: &mut Content) -> Pixmap {
        let mut ctx = sub.offscreen_context(r.w, r.h);
        if let Some(b) = &backdrop {
            draw_pixmap(&mut ctx, b.clone(), (0.0, 0.0));
        }
        let outer = (std::mem::replace(&mut self.nested, 0), std::mem::replace(&mut self.backdrop, backdrop), std::mem::take(&mut self.clip_paths));
        draw(self, &mut ctx, sub);
        (self.nested, self.backdrop, self.clip_paths) = outer;
        ctx.flush();
        let mut pm = Pixmap::new(r.w, r.h);
        ctx.render_with(&mut pm, &mut self.resources, self.raster);
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
    /// (`DestOut` erases what the elements below drew there, `DestIn` keeps only that part), or
    /// as itself (`Plus` adds it). In a non-isolated knockout group (`backdrop`), the element is
    /// laid over the group's backdrop and cut to its shape, so it composites against the art
    /// below the group instead of the elements below it.
    pub(crate) fn knockout_pass(&mut self, ctx: &mut RenderContext, f: &Frame, c: &Arc<Node>, compose: Compose, backdrop: Option<&Arc<Pixmap>>) {
        ctx.set_transform(Affine::IDENTITY);
        ctx.push_layer(None, Some(BlendMode::new(Mix::Normal, compose)), None, None, None);
        self.nested += 1;
        if let Some(b) = backdrop {
            draw_pixmap(ctx, b.clone(), (0.0, 0.0));
        }
        let shape = compose != Compose::Plus && !c.knockout_shape;
        self.shape_of = if shape { Arc::as_ptr(c) as usize } else { 0 };
        self.draw_arc(ctx, f, c);
        self.shape_of = 0;
        if backdrop.is_some() {
            self.knockout_pass(ctx, f, c, Compose::DestIn, None);
        }
        self.nested -= 1;
        ctx.pop_layer();
    }
}

/// Draw `pm` with its top-left corner at pixel `at` (whole pixels: copied exactly).
pub(crate) fn draw_pixmap(ctx: &mut RenderContext, pm: Arc<Pixmap>, at: (f64, f64)) {
    let (w, h) = (pm.width() as f64, pm.height() as f64);
    ctx.set_transform(Affine::translate(at));
    ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(pm), sampler: peniko::ImageSampler::default() });
    ctx.fill_rect(&Rect::new(0.0, 0.0, w, h));
    ctx.set_transform(Affine::IDENTITY);
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
