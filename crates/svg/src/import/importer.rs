use super::convert::{aff, bezpath, blend, clip_shapes, drawn_ellipse, gradient_stops, is_plain, made_up, rule, uncut, unique_name, without_ids};
use super::layers::filter_names;
use super::prepass::link;
use super::{
    Affine, Appearance, AppearanceItem, Arc, BlendMode, Color, Dash, FillLayer, FillRule, Gradient, GradientGeom, GradientKind, GradientPaint,
    GradientStop, ImageBlob, ImageObject, Importer, LineCap, LineJoin, Node, NodeId, NodeKind, Paint, PathData, PatternDef, Point, Rect, Reuse,
    StrokeLayer, Vec2, fnv1a, fx, shapes, spread, text, vector_effect,
};
use crate::SvgError;
use kurbo::BezPath;

/// How many clip paths deep (`<clipPath clip-path>`) an object's clip is read.
const MAX_CLIP_NEST: usize = 16;

/// Points per inch.
pub(crate) const PT_PER_IN: f64 = 72.0;

/// CSS pixels per inch.
pub(super) const CSS_PX_PER_IN: f64 = 96.0;

/// A `<symbol>` made a symbol: its name, its art with no object ids (what each `<use>` has to show
/// to be an instance of it) and the instance transforms the art can stand for.
pub(super) struct SymbolMade {
    name: String,
    art: Node,
    reuse: Reuse,
}

/// The image key of a linked image of the file at `path` that shows `bytes` (the file's pixels or
/// a placeholder): the key of the bytes and the path's. Images of other files that show the same
/// pixels have keys of their own, and reading one file's pixels in changes that file's images only.
fn linked_key(bytes: &[u8], path: &str) -> String {
    format!("img-{:016x}-{}", fnv1a(bytes), vectorcraft_doc::links::hash_bytes(path.as_bytes()))
}

impl Importer {
    pub(super) fn warn(&mut self, s: String) {
        if !self.warnings.contains(&s) {
            self.warnings.push(s);
        }
    }

    /// The child of `g` when it's all `g` holds and only wraps art: an id-less group that leaves
    /// it as it is (but for a transform) and isn't a text.
    pub(super) fn wrapper<'g>(&self, g: &'g usvg::Group) -> Option<&'g usvg::Group> {
        match g.children() {
            [usvg::Node::Group(w)] if w.id().is_empty() && is_plain(w) && self.text_slot(w).is_none() => Some(w),
            _ => None,
        }
    }

    pub(super) fn children(&mut self, g: &usvg::Group, acc: Affine) -> Vec<Arc<Node>> {
        if self.error.is_some() {
            return vec![];
        }
        let mut out = Vec::new();
        for c in g.children() {
            if self.error.is_some() {
                break;
            }
            if let Some(node) = self.node(c, acc) {
                out.push(Arc::new(node));
            }
        }
        out
    }

    /// The object usvg node `n` makes under transform `acc`, with what the file kept on its element
    /// (data, event and other applications' attributes); `None` for what imports as nothing.
    pub(super) fn node(&mut self, n: &usvg::Node, acc: Affine) -> Option<Node> {
        if self.error.is_some() {
            return None;
        }
        let mut out = match n {
            usvg::Node::Group(g) => self.group(g, acc),
            usvg::Node::Path(p) => self.path(p, acc),
            usvg::Node::Image(i) => self.image(i, acc),
            // Only present when usvg had fonts; text is read from the XML instead.
            usvg::Node::Text(_) => None,
        }?;
        if self.hidden.contains(n.id()) {
            out.visible = false;
        }
        self.keep_data(n.id(), &mut out);
        self.keep_events(n.id(), &mut out);
        self.keep_foreign(n.id(), &mut out);
        Some(out)
    }

    /// Give `n` the `data-*` attributes and classes of element `id`, unless it has data already (what an
    /// inner element made of it keeps).
    pub(super) fn keep_data(&self, id: &str, n: &mut Node) {
        let Some((data, classes)) = self.data.get(id) else { return };
        if n.extras().attrs.as_ref().is_none_or(|a| a.data.is_empty()) {
            n.edit_attrs(|a| a.data = data.clone());
        }
        if n.extras().attrs.as_ref().is_none_or(|a| a.classes.is_empty()) {
            n.edit_attrs(|a| a.classes = classes.clone());
        }
    }

    /// Give `n` the event attributes of element `id`, unless it has events already (what an inner
    /// element made of it keeps).
    pub(super) fn keep_events(&self, id: &str, n: &mut Node) {
        if let Some(events) = self.events.get(id).filter(|_| n.extras().attrs.as_ref().is_none_or(|a| a.events.is_empty())) {
            n.edit_attrs(|a| a.events = events.clone());
        }
    }

    /// Give `n` the attributes of element `id` in other applications' namespaces, unless it has
    /// some already (what an inner element made of it keeps).
    pub(super) fn keep_foreign(&self, id: &str, n: &mut Node) {
        if let Some(kept) = self.foreign.get(id).filter(|_| n.extras().attrs.as_ref().is_none_or(|a| a.foreign.is_empty())) {
            n.edit_attrs(|a| a.foreign = kept.clone());
        }
    }

    /// A gradient's stops, midpoints restored, and mixed in linear light when it asks for that.
    fn stops(&self, g: &usvg::BaseGradient) -> Vec<GradientStop> {
        let stops = gradient_stops(g, self.midpoints.get(g.id()).map(Vec::as_slice));
        if self.linear_rgb.contains(g.id()) { super::linear_rgb::bake(stops) } else { stops }
    }

    /// The object name for an element id: its label (see [`labels`]), else the id itself.
    pub(super) fn name_of<'a>(&'a self, id: &'a str) -> &'a str {
        self.labels.get(id).map_or(id, String::as_str)
    }

    /// Allocate a fresh node id within [`Importer::max_nodes`].
    pub(super) fn alloc_id(&mut self) -> Option<NodeId> {
        if self.error.is_some() {
            return None;
        }
        if self.converted_nodes >= self.max_nodes {
            self.error = Some(SvgError::TooLarge(self.max_nodes));
            return None;
        }
        self.converted_nodes += 1;
        Some(self.doc.alloc_id())
    }

    /// A new object named after element `id` (unless it has none or a made-up one).
    fn named(&mut self, id: &str, kind: NodeKind) -> Option<Node> {
        let nid = self.alloc_id()?;
        let mut n = Node::new(nid, kind);
        if !id.is_empty() && !made_up(id) {
            n.name = Some(self.name_of(id).to_string());
        }
        Some(n)
    }

    fn group(&mut self, g: &usvg::Group, acc: Affine) -> Option<Node> {
        if self.error.is_some() {
            return None;
        }
        let url = self.links.get(g.id()).cloned();
        let mut n = match self.instance(g, acc) {
            Some(n) => n,
            None => {
                let n = self.group_node(g, acc)?;
                self.filter_regions(g, acc * aff(g.transform()), n)?
            }
        };
        if let Some(url) = url {
            link(&mut n, &url);
        }
        Some(n)
    }

    /// The group of a `<use>` of a `<symbol>` → an instance of the symbol made from it (on its
    /// first `<use>`). `None` when the group has to be plain art: it shows the symbol differently
    /// (properties inherited from the `<use>`), or the canvas would paint the instance differently
    /// under its transform (see [`Reuse`]), or its viewport cuts the art, or it filters.
    fn instance(&mut self, g: &usvg::Group, acc: Affine) -> Option<Node> {
        let sym = self.uses.get(g.id())?.clone();
        if !g.filters().is_empty() {
            return None;
        }
        // usvg puts the art in a group moved by the `<use>`'s position and the symbol's viewBox.
        let ts = acc * aff(g.transform());
        let (mut xf, mut inner) = (ts, g);
        while let [usvg::Node::Group(s)] = inner.children()
            && s.id().is_empty()
            && is_plain(s)
            && self.text_slot(s).is_none()
        {
            xf *= aff(s.transform());
            inner = s;
        }
        let mut kids = self.children(inner, Affine::IDENTITY);
        let art = match kids.len() {
            0 => return None,
            1 => Arc::unwrap_or_clone(kids.pop()?),
            _ => self.named("", NodeKind::Group { children: kids, clip: false })?,
        };
        // usvg clips a `<use>` of a symbol to its viewport: that only counts when it cuts the art.
        let whole = g.clip_path().is_none_or(|cp| uncut(cp, ts, &art, xf, &|p, m| self.clip_text(p, m)));
        let name = match self.symbols.get(&sym) {
            Some(made) if whole && made.art == without_ids(&art) && made.reuse.allows(xf) => made.name.clone(),
            Some(_) => return None,
            None => {
                let base = self.name_of(&sym).to_string();
                let names = &self.symbol_names;
                let name = unique_name(&base, &mut self.symbol_suffixes, |n| names.contains(n));
                self.symbol_names.insert(name.clone());
                let reuse = Reuse::of(&self.doc, &art);
                let made = SymbolMade { name: name.clone(), art: without_ids(&art), reuse };
                self.doc.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: name.clone(), art: Arc::new(art) });
                self.symbols.insert(sym, made);
                if !whole || !reuse.allows(xf) {
                    return None;
                }
                name
            }
        };
        let label = if made_up(g.id()) { "a symbol instance".to_string() } else { format!("'{}'", g.id()) };
        let mask = g.mask().and_then(|m| self.opacity_mask(m, ts, &label));
        let mut n = self.named(g.id(), NodeKind::SymbolInstance { symbol: name, xf })?;
        n.opacity = g.opacity().get();
        n.blend = blend(g.blend_mode());
        n.isolate = g.isolate();
        n.extras_mut().mask = mask;
        Some(n)
    }

    /// The text a placeholder group stands for (see [`text`]) and the index of its main path.
    pub(super) fn text_slot(&self, g: &usvg::Group) -> Option<(usize, usize)> {
        g.children().iter().enumerate().find_map(|(k, c)| match c {
            usvg::Node::Path(p) => self.slots.find(p).map(|i| (i, k)),
            _ => None,
        })
    }

    /// A text placeholder group → its text object (nothing when the text is hidden).
    fn text(&mut self, g: &usvg::Group, ts: Affine) -> Option<Vec<Arc<Node>>> {
        let (i, main) = self.text_slot(g)?;
        let kids = g.children();
        if !matches!(kids.get(main), Some(usvg::Node::Path(p)) if p.is_visible()) {
            return Some(vec![]);
        }
        let text::PendingText { name, mut obj, servers, paints, non_scaling, under, .. } = self.slots.texts.get(i)?.clone();
        // The scale the type draws at becomes its size (last, below): text space grows by `grow`.
        let xf = ts * obj.xf;
        let grow = text::size_scale(&obj, xf);
        // usvg resolved the `url(#…)` paints (the paths after the main one) in the element's user
        // space; runs paint in text space.
        let to_text = Affine::scale(grow) * obj.xf.inverse();
        let slot_paint = |im: &mut Self, k: usize, m: Affine| match kids.get(main + 1 + k) {
            Some(usvg::Node::Path(p)) => p.fill().map_or(Paint::None, |f| {
                let area = kurbo::Shape::bounding_box(&bezpath(p.data(), m));
                im.paint(f.paint(), m, Some(area))
            }),
            _ => Paint::None,
        };
        let resolved: Vec<Paint> = (0..paints).map(|k| slot_paint(self, k, to_text)).collect();
        let server = |k: usize| resolved.get(k).cloned().unwrap_or(Paint::None);
        for (run, (fill, stroke)) in obj.runs.iter_mut().zip(servers) {
            if let Some(k) = fill {
                run.style.fill = server(k);
            }
            if let Some(k) = stroke {
                run.style.stroke = server(k);
            }
        }
        obj.xf = xf;
        if non_scaling && !vector_effect::unscale_text(&mut obj, self.screen) {
            let label = if name.is_empty() || made_up(&name) { "a text".to_string() } else { format!("text '{name}'") };
            self.warn(format!("non-scaling stroke on {label} approximated: the text is stretched or skewed"));
        }
        // A stroke under the characters is the object's own: it paints in the document.
        let under = under.map(|(mut sl, k)| {
            if let Some(k) = k {
                sl.paint = slot_paint(self, k, ts);
            }
            let mut ap = Appearance { items: vec![AppearanceItem::Stroke(sl)], ..Default::default() };
            ap.scale_strokes(if non_scaling { self.screen } else { obj.xf.determinant().abs().sqrt() });
            ap.set_contents_at(1);
            ap
        });
        text::fold_scale(&mut obj, grow);
        let mut n = self.named(&name, NodeKind::Text(Box::new(obj)))?;
        if let Some(ap) = under {
            n.appearance = ap.into();
        }
        Some(vec![Arc::new(n)])
    }

    fn group_node(&mut self, g: &usvg::Group, acc: Affine) -> Option<Node> {
        let ts = acc * aff(g.transform());
        // A text placeholder becomes its text, named after the element (the group stays unnamed).
        let text = self.text(g, ts);
        if text.as_ref().is_some_and(Vec::is_empty) {
            return None;
        }
        let is_text = text.is_some();
        // Made-up ids aren't names.
        let id = if is_text || made_up(g.id()) { "" } else { g.id() };
        let label = match (&text, id) {
            (Some(t), _) => t.first().and_then(|n| n.name.as_deref()).map_or_else(|| "a text".to_string(), |n| format!("text '{n}'")),
            (None, "") => "a group".to_string(),
            (None, id) => format!("'{id}'"),
        };
        let mask = g.mask().and_then(|m| self.opacity_mask(m, ts, &label));
        // Blurs, shadows and glows become live effects (see [`fx`]); other filters become Effect ›
        // SVG Filters effects that keep their XML.
        let effects = match g.filters() {
            [] => vec![],
            filters => fx::effects(filters, ts, &self.blends).or_else(|| fx::kept(filters, ts, &self.filter_sources)).unwrap_or_else(|| {
                // usvg wraps a filtered element in a group of its own: name the element then.
                let on = match g.children() {
                    [usvg::Node::Group(_)] | [] => label.clone(),
                    [one] if made_up(g.id()) || g.id().is_empty() => {
                        if one.id().is_empty() || made_up(one.id()) {
                            "an object".to_string()
                        } else {
                            format!("'{}'", one.id())
                        }
                    }
                    _ => label.clone(),
                };
                self.warn(format!("filter {} on {on} isn't supported; drawn without it", filter_names(filters)));
                vec![]
            }),
        };
        let mut children = match text {
            Some(t) => t,
            None => self.children(g, ts),
        };
        let mut n = if let Some(cp) = g.clip_path() {
            // SVG filters before it clips: the effects go on the art inside the clip.
            if !effects.is_empty() {
                let mut inner = self.named("", NodeKind::Group { children, clip: false })?;
                inner.appearance.effects = effects.clone();
                children = vec![Arc::new(inner)];
            }
            let children = self.clipped(cp, ts, children, &label)?;
            self.named(id, NodeKind::Group { children, clip: true })?
        } else {
            if children.is_empty() {
                return None;
            }
            // An id-less wrapper around a single object (usvg adds these for opacity/transform on
            // shapes): fold its opacity, blend and effects into the object (unless the object
            // blends or masks before the effects). A text takes its own mask too. A group with
            // `data-*` or event attributes of its own stays a group, and keeps them.
            let own = self.data.contains_key(g.id()) || self.events.contains_key(g.id());
            if (is_text || id.is_empty() && mask.is_none() && !own)
                && children.len() == 1
                && (g.blend_mode() == usvg::BlendMode::Normal || children[0].blend == BlendMode::Normal)
                && (effects.is_empty() || children[0].blend == BlendMode::Normal && children[0].extras().mask.is_none())
                && let Some(only) = children.pop()
            {
                let mut c = Arc::unwrap_or_clone(only);
                c.opacity *= g.opacity().get();
                if g.blend_mode() != usvg::BlendMode::Normal {
                    c.blend = blend(g.blend_mode());
                }
                c.isolate |= g.isolate();
                if mask.is_some() {
                    c.extras_mut().mask = mask;
                }
                c.appearance.effects.extend(effects);
                return Some(c);
            }
            let mut n = self.named(id, NodeKind::Group { children, clip: false })?;
            n.appearance.effects = effects;
            n
        };
        n.opacity = g.opacity().get();
        n.blend = blend(g.blend_mode());
        n.isolate = g.isolate();
        n.extras_mut().mask = mask;
        if !is_text && self.sublayers.contains(g.id()) {
            self.sublayer_groups.insert(n.id);
        }
        Some(n)
    }

    /// `n`, made of group `g` (whose user space `ts` maps to the document), clipped to the regions
    /// of the filters it imported as effects: SVG 1.1 §15.5
    /// (<https://www.w3.org/TR/SVG11/filters.html#FilterEffectsRegion>) cuts a filter's result at
    /// its region. A filter that didn't import (drawn without it) clips nothing.
    fn filter_regions(&mut self, g: &usvg::Group, ts: Affine, mut n: Node) -> Option<Node> {
        if g.filters().is_empty() || fx::effects(g.filters(), ts, &self.blends).is_none() {
            return Some(n);
        }
        for f in g.filters() {
            n = self.region_clipped(n, f.rect(), ts)?;
        }
        Some(n)
    }

    /// `n` in a clip group cut to `region` (a filter's or mask's, in the user space `ts` maps to
    /// the document), unless all it paints (its visual bounds, grown by the reach of its effects)
    /// lies inside the region. `None` only past the object limit.
    fn region_clipped(&mut self, n: Node, region: usvg::NonZeroRect, ts: Affine) -> Option<Node> {
        let r = Rect::new(region.left() as f64, region.top() as f64, region.right() as f64, region.bottom() as f64);
        let det = ts.determinant();
        let Some(bounds) = n.visual_bounds_with(&self.doc.symbol_boxes()) else { return Some(n) };
        if !det.is_finite() || det.abs() < 1e-12 {
            return Some(n);
        }
        // A clip group paints inside its clipping path; anything else as far as its effects reach
        // (as export sizes the filter regions it writes: an Outer Glow grows by a quarter of a
        // raster pixel first).
        let reach = if matches!(n.kind, NodeKind::Group { clip: true, .. }) {
            0.0
        } else {
            let grow = 18.0 / self.doc.raster_ppi();
            vectorcraft_effects::raster_effects(&n.appearance.effects)
                .iter()
                .map(|f| f.outset(bounds) + if matches!(f, vectorcraft_effects::RasterFx::OuterGlow { .. }) { grow } else { 0.0 })
                .sum()
        };
        let painted = bounds.inflate(reach, reach);
        let to_user = ts.inverse();
        let eps = 1e-6 * (1.0 + r.width().max(r.height()));
        let inside =
            [(painted.x0, painted.y0), (painted.x1, painted.y0), (painted.x1, painted.y1), (painted.x0, painted.y1)].into_iter().all(|(x, y)| {
                let p = to_user * Point::new(x, y);
                p.x >= r.x0 - eps && p.x <= r.x1 + eps && p.y >= r.y0 - eps && p.y <= r.y1 + eps
            });
        if inside {
            return Some(n);
        }
        let path = shapes::rectangle(r).transformed(ts);
        let clip = self.named("", NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: true, guide: false })?;
        self.named("", NodeKind::Group { children: vec![Arc::new(clip), Arc::new(n)], clip: true })
    }

    /// `<mask>` → opacity mask. An alpha mask's art comes in white with its opacity kept, so its
    /// luminance is its alpha (images in it keep their colours, with a warning). A mask our export wrote gets its
    /// options back ([`mask_flags`]) and its art without what stands for them: the inverting
    /// filter group and the backdrop rectangle.
    fn opacity_mask(&mut self, m: &usvg::Mask, ts: Affine, label: &str) -> Option<Box<vectorcraft_doc::OpacityMask>> {
        if m.mask().is_some() {
            self.warn(format!("nested mask on {label} ignored"));
        }
        let (clip, invert) = self.mask_flags.get(m.id()).copied().unwrap_or((true, false));
        let user = ts;
        let (mut nodes, mut ts) = (m.root().children(), ts);
        if invert
            && let [usvg::Node::Group(g)] = nodes
            && !g.filters().is_empty()
        {
            (nodes, ts) = (g.children(), ts * aff(g.transform()));
        }
        if (!clip || invert)
            && let [usvg::Node::Path(_), rest @ ..] = nodes
        {
            nodes = rest;
        }
        let mut children: Vec<Arc<Node>> = nodes.iter().filter_map(|c| self.node(c, ts)).map(Arc::new).collect();
        if m.kind() == usvg::MaskType::Alpha {
            children.iter_mut().for_each(|c| vectorcraft_doc::node::whiten(Arc::make_mut(c)));
            let mut image = false;
            children.iter().for_each(|c| c.walk(&mut |n| image |= matches!(n.kind, NodeKind::Image(_))));
            if image {
                self.warn(format!("image in the alpha mask on {label} counts by its luminance"));
            }
        }
        // Mask art that is all hidden draws nothing: that is an empty mask, not no mask (black all
        // over when clipped, white when not, inverted when the mask is).
        let art = match children.len() {
            1 => Arc::unwrap_or_clone(children.pop()?),
            _ => self.named("", NodeKind::Group { children, clip: false })?,
        };
        // SVG 1.1 §14.4 (https://www.w3.org/TR/SVG11/masking.html#MaskElement): the mask's art
        // counts only inside its region.
        let art = self.region_clipped(art, m.rect(), user)?;
        let mut mask = vectorcraft_doc::OpacityMask::new(art, clip);
        mask.invert = invert;
        Some(Box::new(mask))
    }

    /// `children` clipped by `cp` (on an element whose user space `ts` maps to the document): the
    /// children of a clip group or clipping layer, its clipping path first. A clip path clipped in
    /// turn (`<clipPath clip-path>`) nests one clip group in another, so the clips intersect.
    /// `None` when the clip path has no shape (nothing shows).
    pub(super) fn clipped(&mut self, cp: &usvg::ClipPath, ts: Affine, mut children: Vec<Arc<Node>>, label: &str) -> Option<Vec<Arc<Node>>> {
        let mut clips = vec![self.clip_node(cp, ts)?];
        let mut next = cp.clip_path();
        while let Some(c) = next {
            if clips.len() > MAX_CLIP_NEST {
                self.warn(format!("clip paths nested more than {MAX_CLIP_NEST} deep on {label}: the deeper ones are ignored"));
                break;
            }
            clips.push(self.clip_node(c, ts)?);
            next = c.clip_path();
        }
        let outer = clips.pop()?;
        for clip in clips {
            let mut ch = vec![Arc::new(clip)];
            ch.extend(children);
            children = vec![Arc::new(self.named("", NodeKind::Group { children: ch, clip: true })?)];
        }
        let mut ch = vec![Arc::new(outer)];
        ch.extend(children);
        Some(ch)
    }

    /// Put `art` in layer `l` at `index` (at most its end) unclipped: a clipping mask on the layer
    /// becomes a clip group in it, holding the art it clipped. The layer's ruler guides stay last,
    /// outside the clip group.
    pub(super) fn add_unclipped(&mut self, l: &mut Node, index: usize, art: Vec<Arc<Node>>) {
        if art.is_empty() {
            return;
        }
        if let NodeKind::Layer { children, clip: clip @ true, .. } = &mut l.kind {
            *clip = false;
            let mut clipped = std::mem::take(children);
            let guides = clipped.split_off(Node::guide_run_start(&clipped));
            if let Some(group) = self.named("", NodeKind::Group { children: clipped, clip: true }) {
                children.push(Arc::new(group));
            }
            children.extend(guides);
        }
        if let Some(ch) = l.children_mut() {
            let i = index.min(Node::guide_run_start(ch));
            ch.splice(i..i, art);
        }
    }

    fn clip_node(&mut self, cp: &usvg::ClipPath, ts: Affine) -> Option<Node> {
        if let Some(n) = self.clip_type(cp, ts) {
            return Some(n);
        }
        let mut shapes = clip_shapes(cp, ts, &|p, m| self.clip_text(p, m));
        let id = cp.id().to_string();
        if shapes.len() == 1 {
            let (bp, r) = shapes.pop()?;
            let kind = self.clip_shape(&bp, r, true)?;
            return self.named(&id, kind);
        }
        // SVG 1.1 §14.3.5: the clip is the union of the children's silhouettes, each under its own
        // clip-rule, so they don't cancel or share one rule: a group of them, which clips to that
        // union (`Node::clip_shapes`).
        let mut children = Vec::with_capacity(shapes.len());
        for (bp, r) in &shapes {
            let kind = self.clip_shape(bp, *r, false)?;
            children.push(Arc::new(Node::new(self.alloc_id()?, kind)));
        }
        if children.is_empty() {
            return None;
        }
        self.named(&id, NodeKind::Group { children, clip: false })
    }

    /// The glyph outlines, in the document, of the text a clip placeholder path `p` stands for (see
    /// [`text`]), whose user space `m` maps to the document; nothing when the text is hidden. `None`
    /// when `p` isn't a placeholder: in a clip path only its id tells (usvg paints clip path content
    /// black).
    fn clip_text(&self, p: &usvg::Path, m: Affine) -> Option<Vec<(BezPath, FillRule)>> {
        if p.id().is_empty() {
            return None;
        }
        let i = self.slots.find(p)?;
        let t = self.slots.texts.get(i)?;
        if !p.is_visible() {
            return Some(vec![]);
        }
        let mut bp = BezPath::clone(&*self.slots.outlines(i)?);
        bp.apply_affine(m * self.slots.inner(p) * t.obj.xf);
        let r = p.fill().map(|f| rule(f.rule())).unwrap_or_default();
        Some(if bp.elements().is_empty() { vec![] } else { vec![(bp, r)] })
    }

    /// Clip path `cp` (on an element whose user space `ts` maps to the document) as live type, when
    /// all it holds is one visible text that clips by the non-zero rule and has no clip path of its
    /// own: the text object with no fill or stroke, the same structure Make Clipping Mask produces
    /// for type.
    fn clip_type(&mut self, cp: &usvg::ClipPath, ts: Affine) -> Option<Node> {
        let mut m = ts * aff(cp.transform());
        let mut kids = cp.root().children();
        while let [usvg::Node::Group(g)] = kids
            && g.clip_path().is_none()
        {
            m *= aff(g.transform());
            kids = g.children();
        }
        let [usvg::Node::Path(p)] = kids else { return None };
        if p.id().is_empty() || !p.is_visible() || p.fill().is_some_and(|f| f.rule() == usvg::FillRule::EvenOdd) {
            return None;
        }
        let mut obj = self.slots.texts.get(self.slots.find(p)?)?.obj.clone();
        obj.xf = m * self.slots.inner(p) * obj.xf;
        let grow = text::size_scale(&obj, obj.xf);
        for run in &mut obj.runs {
            run.style.fill = Paint::None;
            run.style.stroke = Paint::None;
        }
        text::fold_scale(&mut obj, grow);
        self.named(cp.id(), NodeKind::Text(Box::new(obj)))
    }

    /// One clip path child: a path, or a compound path of its subpaths when it has several.
    fn clip_shape(&mut self, bp: &BezPath, r: FillRule, clipping: bool) -> Option<NodeKind> {
        let subs = PathData::from_bezpath(bp).subpaths;
        if subs.len() == 1 {
            return Some(NodeKind::Path { path: PathData::new(subs), rule: r, live: None, clipping, guide: false });
        }
        let mut children = Vec::with_capacity(subs.len());
        for sp in subs {
            let nid = self.alloc_id()?;
            children
                .push(Arc::new(Node::new(nid, NodeKind::Path { path: PathData::single(sp), rule: r, live: None, clipping: false, guide: false })));
        }
        Some(NodeKind::Compound { children, rule: r })
    }

    /// A usvg paint mapped by `m`, for art covering `area` (after `m`: where a reflected or
    /// repeated gradient has to reach).
    fn paint(&mut self, p: &usvg::Paint, m: Affine, area: Option<Rect>) -> Paint {
        let (kind, base, mut geom) = match p {
            usvg::Paint::Color(c) => return Paint::solid(Color::rgb8(c.red, c.green, c.blue)),
            usvg::Paint::Pattern(pt) => return self.pattern(pt, m),
            usvg::Paint::LinearGradient(lg) => {
                let geom = GradientGeom {
                    start: Point::new(lg.x1() as f64, lg.y1() as f64),
                    end: Point::new(lg.x2() as f64, lg.y2() as f64),
                    aspect: 1.0,
                    focal: None,
                };
                let base: &usvg::BaseGradient = lg;
                (GradientKind::Linear, base, geom)
            }
            usvg::Paint::RadialGradient(rg) => {
                if rg.fr().get() > 1e-4 {
                    self.warn(format!("gradient '{}': focal radius ignored", rg.id()));
                }
                let c = Point::new(rg.cx() as f64, rg.cy() as f64);
                let mut geom = GradientGeom { start: c, end: c + Vec2::new(rg.r().get() as f64, 0.0), aspect: 1.0, focal: None };
                // A focal point outside the circle is pulled inside it.
                geom.set_focal(Some(Point::new(rg.fx() as f64, rg.fy() as f64)));
                let base: &usvg::BaseGradient = rg;
                (GradientKind::Radial, base, geom)
            }
        };
        geom.transform(m * aff(base.transform()), kind);
        let mut stops = self.stops(base);
        let reflect = match base.spread_method() {
            usvg::SpreadMethod::Pad => None,
            usvg::SpreadMethod::Reflect => Some(true),
            usvg::SpreadMethod::Repeat => Some(false),
        };
        if let Some(reflect) = reflect {
            match area.map(|a| spread::expand(kind, &mut geom, &mut stops, reflect, a)) {
                Some(spread::Expanded::Whole) => {}
                Some(spread::Expanded::Capped) => {
                    self.warn(format!("gradient '{}': spreadMethod repeats more than {} times; it pads after that", base.id(), spread::MAX_PERIODS))
                }
                None => self.warn(format!("gradient '{}': spreadMethod approximated as pad", base.id())),
            }
        }
        Paint::Gradient(Box::new(GradientPaint {
            gradient: Gradient::new(kind, stops),
            geom: Some(geom),
            angle: geom.angle_deg(),
            swatch: None,
            freeform: None,
        }))
    }

    /// `<pattern>` → a pattern swatch painted with the pattern's tile placement. The swatch art is
    /// the pattern content clipped to the tile, as SVG draws it.
    fn pattern(&mut self, pt: &Arc<usvg::Pattern>, m: Affine) -> Paint {
        let r = pt.rect();
        let xf = m * aff(pt.transform()) * Affine::translate((r.x() as f64, r.y() as f64));
        let key = Arc::as_ptr(pt) as usize;
        if let Some(name) = self.patterns.get(&key) {
            return Paint::Pattern { pattern: name.clone(), xf };
        }
        let tile = Rect::new(0.0, 0.0, r.width() as f64, r.height() as f64);
        let content = self.children(pt.root(), aff(pt.root().transform()));
        if content.is_empty() {
            return Paint::None;
        }
        let clip =
            self.named("", NodeKind::Path { path: shapes::rectangle(tile), rule: FillRule::NonZero, live: None, clipping: true, guide: false });
        let Some(clip) = clip else { return Paint::None };
        let mut children = vec![Arc::new(clip)];
        children.extend(content);
        let art = self.named("", NodeKind::Group { children, clip: true });
        let Some(art) = art else { return Paint::None };
        let base = if pt.id().is_empty() { "Pattern" } else { pt.id() };
        let doc = &self.doc;
        let name = unique_name(base, &mut self.pattern_suffixes, |n| doc.pattern(n).is_some());
        let mut def = PatternDef::new(&name, vec![Arc::new(art)]);
        def.tile = tile;
        self.doc.patterns.push(def);
        self.patterns.insert(key, name.clone());
        Paint::Pattern { pattern: name, xf }
    }

    /// The fill and stroke of `p` mapped by `m`; `area`: the bounds of the path after `m`.
    fn appearance(&mut self, p: &usvg::Path, m: Affine, area: Option<Rect>) -> (Appearance, FillRule) {
        let mut fill = None;
        let mut r = FillRule::NonZero;
        if let Some(f) = p.fill() {
            let paint = self.paint(f.paint(), m, area);
            let mut fl = FillLayer::new(paint);
            fl.opacity = f.opacity().get();
            r = rule(f.rule());
            fill = Some(AppearanceItem::Fill(fl));
        }
        let mut stroke = None;
        if let Some(s) = p.stroke() {
            // A non-scaling stroke is as wide on screen whatever `m` does: the path takes `m`, so a
            // stroke of that width in the document is exact.
            let scale = if self.non_scaling.contains(p.id()) { self.screen } else { m.determinant().abs().sqrt() };
            // The stroke reaches past the path (miters further: a generous margin).
            let reach = s.width().get() as f64 * scale * (s.miterlimit().get() as f64).max(1.0);
            let paint = self.paint(s.paint(), m, area.map(|a| a.inflate(reach, reach)));
            let mut sl = StrokeLayer::new(paint, s.width().get() as f64 * scale);
            sl.opacity = s.opacity().get();
            sl.cap = match s.linecap() {
                usvg::LineCap::Butt => LineCap::Butt,
                usvg::LineCap::Round => LineCap::Round,
                usvg::LineCap::Square => LineCap::Square,
            };
            sl.join = match s.linejoin() {
                usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => LineJoin::Miter,
                usvg::LineJoin::Round => LineJoin::Round,
                usvg::LineJoin::Bevel => LineJoin::Bevel,
            };
            sl.miter_limit = s.miterlimit().get() as f64;
            if let Some(d) = s.dasharray() {
                sl.dash = Some(Dash {
                    pattern: d.iter().map(|v| *v as f64 * scale).collect(),
                    offset: s.dashoffset() as f64 * scale,
                    align_corners: false,
                });
            }
            stroke = Some(AppearanceItem::Stroke(sl));
        }
        let items = match p.paint_order() {
            usvg::PaintOrder::FillAndStroke => [fill, stroke],
            usvg::PaintOrder::StrokeAndFill => [stroke, fill],
        };
        (Appearance { items: items.into_iter().flatten().collect(), ..Default::default() }, r)
    }

    fn path(&mut self, p: &usvg::Path, acc: Affine) -> Option<Node> {
        if !p.is_visible() {
            return None;
        }
        let ellipse = if self.ellipses.contains(p.id()) { drawn_ellipse(p.data()) } else { None };
        let path = ellipse.map_or_else(|| PathData::from_bezpath(&bezpath(p.data(), acc)), |e| e.transformed(acc));
        if path.is_empty() {
            return None;
        }
        let (appearance, r) = self.appearance(p, acc, path.bounds());
        // Shared, so a compound path's subpaths don't each copy a gradient's stops.
        let appearance: vectorcraft_doc::Shared<Appearance> = appearance.into();
        let mut n = if path.subpaths.len() > 1 {
            // Multi-subpath SVG paths are compound paths (as in Illustrator).
            let mut children = Vec::with_capacity(path.subpaths.len());
            for sp in path.subpaths {
                let id = self.alloc_id()?;
                let mut c = Node::new(id, NodeKind::Path { path: PathData::single(sp), rule: r, live: None, clipping: false, guide: false });
                c.appearance = appearance.clone();
                children.push(Arc::new(c));
            }
            self.named(p.id(), NodeKind::Compound { children, rule: r })?
        } else {
            self.named(p.id(), NodeKind::Path { path, rule: r, live: None, clipping: false, guide: false })?
        };
        n.appearance = appearance;
        Some(n)
    }

    fn image(&mut self, i: &usvg::Image, acc: Affine) -> Option<Node> {
        if !i.is_visible() {
            return None;
        }
        let (bytes, mime) = match i.kind() {
            usvg::ImageKind::JPEG(d) => (d, "image/jpeg"),
            usvg::ImageKind::PNG(d) => (d, "image/png"),
            usvg::ImageKind::GIF(d) => (d, "image/gif"),
            usvg::ImageKind::WEBP(d) => (d, "image/webp"),
            usvg::ImageKind::SVG(tree) => return self.svg_image(tree, acc),
        };
        let size = i.size();
        let (pw, ph) = image::ImageReader::new(std::io::Cursor::new(&bytes[..]))
            .with_guessed_format()
            .ok()
            .and_then(|r| r.into_dimensions().ok())
            .unwrap_or((size.width().round().max(1.0) as u32, size.height().round().max(1.0) as u32));
        // A linked file keeps a preview; a placeholder is all a missing one has (its preview).
        let linked = self.files.linked(i.kind());
        let missing = linked.is_some_and(|l| l.missing);
        let link = linked.map(|l| l.link.clone());
        let key = match &link {
            Some(l) => linked_key(bytes, &l.path),
            None => format!("img-{:016x}", fnv1a(bytes)),
        };
        self.doc.images.entry(key.clone()).or_insert_with(|| {
            let blob = ImageBlob { mime: mime.into(), bytes: bytes.clone(), proxy: missing.then(|| bytes.clone()), file_font: Default::default() };
            if link.is_some() && !missing { blob.with_proxy() } else { blob }
        });
        let xf = acc * Affine::scale_non_uniform(size.width() as f64 / pw as f64, size.height() as f64 / ph as f64);
        let mut node = self.named(
            i.id(),
            NodeKind::Image(Box::new(ImageObject { key, width: pw, height: ph, xf, link, placement: Default::default(), pixels: None })),
        )?;

        if i.rendering_mode() == usvg::ImageRendering::Pixelated {
            node.extras_mut().pixelated = true;
        }

        Some(node)
    }

    /// An SVG shown as an image (a data URL or a linked file) → its art, `acc` mapping its size.
    /// Its ids are its own: they mean nothing in the SVG around it.
    fn svg_image(&mut self, tree: &usvg::Tree, acc: Affine) -> Option<Node> {
        if self.error.is_some() {
            return None;
        }
        if self.nested_svg_images >= crate::MAX_NESTED_SVG_IMAGES {
            self.error = Some(SvgError::TooManyImages(crate::MAX_NESTED_SVG_IMAGES));
            return None;
        }
        self.nested_svg_images += 1;
        let outer = (
            std::mem::take(&mut self.links),
            std::mem::take(&mut self.hidden),
            std::mem::take(&mut self.uses),
            std::mem::take(&mut self.labels),
            std::mem::take(&mut self.non_scaling),
            std::mem::take(&mut self.data),
            std::mem::take(&mut self.events),
            std::mem::take(&mut self.ellipses),
            std::mem::take(&mut self.filter_sources),
        );
        let art = self.group_node(tree.root(), acc);
        (self.links, self.hidden, self.uses, self.labels, self.non_scaling, self.data, self.events, self.ellipses, self.filter_sources) = outer;
        art
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::files::Files;
    use crate::import::text::TextSlots;
    use std::collections::{HashMap, HashSet};
    use vectorcraft_doc::Document;

    #[test]
    fn svg_image_cap_sets_too_many_images_error() {
        let mut im = Importer {
            doc: Document::new(100.0, 100.0),
            warnings: vec![],
            mask_flags: HashMap::new(),
            links: HashMap::new(),
            slots: TextSlots::default(),
            patterns: HashMap::new(),
            midpoints: HashMap::new(),
            linear_rgb: HashSet::new(),
            labels: HashMap::new(),
            hidden: HashSet::new(),
            uses: HashMap::new(),
            symbols: HashMap::new(),
            symbol_names: HashSet::new(),
            symbol_suffixes: HashMap::new(),
            pattern_suffixes: HashMap::new(),
            blends: HashMap::new(),
            filter_sources: HashMap::new(),
            files: Files::default(),
            non_scaling: HashSet::new(),
            data: HashMap::new(),
            events: HashMap::new(),
            foreign: HashMap::new(),
            sublayers: HashSet::new(),
            sublayer_groups: HashSet::new(),
            ellipses: HashSet::new(),
            screen: 1.0,
            converted_nodes: 0,
            max_nodes: 1000,
            error: None,
            nested_svg_images: crate::MAX_NESTED_SVG_IMAGES,
        };
        let tree = usvg::Tree::from_str(r#"<svg xmlns="http://www.w3.org/2000/svg"/>"#, &usvg::Options::default()).unwrap();
        let res = im.svg_image(&tree, Affine::IDENTITY);
        assert!(res.is_none());
        assert!(matches!(im.error, Some(SvgError::TooManyImages(crate::MAX_NESTED_SVG_IMAGES))));
        assert_eq!(im.error.unwrap().to_string(), "the SVG expands to more than 256 nested SVG images");
    }
}
