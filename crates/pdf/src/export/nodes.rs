use super::exporter::{address, blend, constant_mask, cover, in_range, norm, rects_overlap, rule, to_path, writable};
use super::{
    Affine, AppearanceItem, BezPath, BlendMode, Exporter, Fill, FillRule, LineCap, LineJoin, Mark, Node, NodeKind, NormalizedF32, Paint, Path, Rect,
    Shape, Stroke, StrokeAlign, StrokeDash, StrokeLayer, Surface, WrittenShape, krilla, stroke,
};

impl Exporter<'_> {
    /// Opacity mask → luminosity soft mask. Outside the art the backdrop is black (clip) or white;
    /// invert is a white Difference rect on top (luminance is linear, so luma(1 − c) = 1 − luma(c)).
    fn soft_mask(&mut self, s: &mut Surface, m: &vectorcraft_doc::OpacityMask, page: Rect) -> krilla::mask::Mask {
        let mut sb = s.stream_builder();
        let mut ms = sb.surface();
        // An opaque backdrop when it matters: white outside the art (no clip), or black for the
        // inverting Difference pass below to turn white.
        if !m.clip || m.invert {
            cover(&mut ms, page, if m.clip { 0 } else { 255 }, 1.0);
        }
        // Mask art is a picture of its own: it takes no part in a knockout group around the object,
        // and has no inks to overprint.
        let knockout = std::mem::take(&mut self.knockout);
        let overprint = std::mem::take(&mut self.overprint);
        self.node(&mut ms, &m.art, page, true);
        (self.knockout, self.overprint) = (knockout, overprint);
        if m.invert {
            ms.push_blend_mode(krilla::blend::BlendMode::Difference);
            cover(&mut ms, page, 255, 1.0);
            ms.pop();
        }
        ms.finish();
        krilla::mask::Mask::new(sb.finish(), krilla::mask::MaskType::Luminosity)
    }

    /// The children of a group, as the elements of a knockout group when it is one.
    pub(super) fn children(&mut self, s: &mut Surface, children: &[std::sync::Arc<Node>], page: Rect) {
        if !self.knockout {
            for c in children {
                self.node(s, c, page, false);
            }
            return;
        }
        // The PDF writer has no knockout groups: each element is drawn through a soft mask of
        // where the elements above it don't paint (the same look for Normal blending). The masks
        // nest, so element i sits inside the masks of elements i+1…n: open them outermost first.
        self.warn("knockout groups are written as soft-masked groups (same look, but not editable as knockout groups)");
        let elements: Vec<_> = Node::knockout_elements(children)
            .into_iter()
            .filter(|c| c.visual_bounds_with(&self.symbols).is_some_and(|b| rects_overlap(b.inflate(1.0, 1.0), page)))
            .collect();
        let Some((first, rest)) = elements.split_first() else { return };
        for c in rest.iter().rev() {
            let mask = self.knockout_mask(s, c, page);
            s.push_mask(mask);
        }
        self.node(s, first, page, false);
        for c in rest {
            s.pop();
            self.node(s, c, page, false);
        }
    }

    /// A luminosity mask that is 1 − the knockout shape of `c`: white, then black through an alpha
    /// mask of `c` (at full object opacity without its own mask, unless those define its shape).
    fn knockout_mask(&mut self, s: &mut Surface, c: &Node, page: Rect) -> krilla::mask::Mask {
        let shape = if c.knockout_shape {
            c.clone()
        } else {
            let mut s = c.clone().with_mask(None);
            s.opacity = 1.0;
            s
        };
        let mut sb = s.stream_builder();
        let mut ms = sb.surface();
        cover(&mut ms, page, 255, 1.0);
        let alpha = {
            let mut ab = ms.stream_builder();
            let mut als = ab.surface();
            let overprint = std::mem::take(&mut self.overprint);
            self.node(&mut als, &shape, page, false);
            self.overprint = overprint;
            als.finish();
            krilla::mask::Mask::new(ab.finish(), krilla::mask::MaskType::Alpha)
        };
        ms.push_mask(alpha);
        cover(&mut ms, page, 0, 1.0);
        ms.pop();
        ms.finish();
        krilla::mask::Mask::new(sb.finish(), krilla::mask::MaskType::Luminosity)
    }

    pub(super) fn node(&mut self, s: &mut Surface, n: &Node, page: Rect, force: bool) {
        if self.layers
            && self.in_layer != address(n)
            && let NodeKind::Layer { printable, .. } = n.kind
            && (printable || self.non_printing)
            && let Some(&i) = self.layer_index.get(&address(n))
        {
            // A PDF layer: hidden layers and sublayers are written too (their group is off).
            let enclosing = std::mem::replace(&mut self.in_layer, address(n));
            crate::forms::form(s, Mark::Layer(i), |s| self.node(s, n, page, true));
            self.in_layer = enclosing;
            return;
        }
        if !force && !n.visible {
            return;
        }
        if let NodeKind::Layer { template, printable, .. } = n.kind
            && (template || !(printable || self.non_printing))
        {
            return;
        }
        match n.visual_bounds_with(&self.symbols) {
            Some(b) if !force && !rects_overlap(b.inflate(1.0, 1.0), page) => return,
            None if !n.is_container() => return,
            _ => {}
        }
        self.warn_raster(&n.appearance.effects, || format!("{} objects", n.kind_label().to_lowercase()));
        let container = n.is_container() && !matches!(n.kind, NodeKind::Compound { .. });
        // Whether this container's children knock each other out (a knockout group is written as a group).
        let knockout = n.knocks_out(self.knockout);
        let enclosing = std::mem::replace(&mut self.knockout, knockout);
        let mut pushes = 0;
        if n.blend != BlendMode::Normal {
            s.push_blend_mode(blend(n.blend));
            pushes += 1;
        }
        if n.opacity < 1.0 || (container && (n.isolate || n.blend != BlendMode::Normal || knockout)) {
            if !n.isolate && n.blends_through() {
                // Not isolated: blending inside reaches the art below the group.
                let alpha = constant_mask(s, page, n.opacity);
                s.push_mask(alpha);
            } else if n.opacity < 1.0 {
                s.push_opacity(norm(n.opacity));
            } else {
                s.push_isolated();
            }
            pushes += 1;
        }
        let mask = n.extras().mask.as_deref().filter(|m| !m.disabled);
        let alpha = match (&n.kind, mask) {
            (NodeKind::Image(im), Some(m)) => self.masked_image(im, m, !n.extras().pixelated),
            _ => None,
        };
        if let Some(m) = mask
            && alpha.is_none()
        {
            let mask = self.soft_mask(s, m, page);
            s.push_mask(mask);
            pushes += 1;
        }
        match &n.kind {
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => self.children(s, children, page),
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                // The region every output clips to; nothing to clip by hides the clipped art. The
                // clipping path's fill paints behind the clipped art and its stroke over it, unclipped.
                if let Some((clip, rest)) = children.split_first()
                    && let Some((p, r)) = vectorcraft_effects::clip_outline(clip).and_then(|(bp, r)| to_path(s, &bp).map(|p| (p, r)))
                {
                    let paint = clip.clip_paint();
                    s.push_clip_path(&p, &rule(r));
                    if let Some(fill) = &paint.fill {
                        self.node(s, fill, page, false);
                    }
                    self.children(s, rest, page);
                    s.pop();
                    if let Some(stroke) = &paint.stroke {
                        self.node(s, stroke, page, false);
                    }
                }
            }
            NodeKind::Path { path, rule, guide, .. } => {
                if !*guide {
                    self.shape(s, n, &path.to_bezpath(), *rule, page);
                }
            }
            NodeKind::Compound { children, rule } => {
                let mut bp = BezPath::new();
                for c in children {
                    if let Some(p) = c.path_data() {
                        bp.extend(p.to_bezpath());
                    }
                }
                self.shape(s, n, &bp, *rule, page);
            }
            NodeKind::Text(t) => self.text(s, n, t, page),
            NodeKind::Image(im) => {
                let colorized = n.extras().colorize.as_deref().and_then(|p| self.colorized_image(im, p, !n.extras().pixelated));
                self.image(s, im, colorized.or(alpha), n.extras().pixelated)
            }
            NodeKind::SymbolInstance { symbol, xf } => {
                // Within the objects a frame may draw of symbol instances, as on screen: nested
                // instances multiply.
                let sym = self.doc.symbols.iter().find(|x| &x.name == symbol);
                let total = sym.map_or(0, |sym| self.symbol_nodes.saturating_add(sym.art.count() as u64));
                if let Some(sym) = sym.filter(|_| total <= vectorcraft_doc::MAX_SYMBOL_NODES) {
                    self.symbol_nodes = total;
                    let mut art = (*sym.art).clone();
                    art.transform(*xf, false);
                    self.node(s, &art, page, true);
                }
            }
            // Live blends/envelopes/meshes export their evaluated (expanded) form.
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::Intertwine { .. } => {
                let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                for c in g.children().into_iter().flatten() {
                    self.node(s, c, page, false);
                }
            }
        }
        self.knockout = enclosing;
        for _ in 0..pushes {
            s.pop();
        }
    }

    fn shape(&mut self, s: &mut Surface, n: &Node, bp: &BezPath, r: FillRule, page: Rect) {
        let Some(path) = to_path(s, bp) else { return };
        let bounds = bp.bounding_box();
        for item in &n.appearance.items {
            match item {
                AppearanceItem::Fill(fl) => {
                    if !fl.visible || fl.paint.is_none() {
                        continue;
                    }
                    self.warn_raster(&fl.effects, || "fills".into());
                    self.overprinting(s, fl.overprint, &fl.paint, |ex, s| {
                        if ex.area(s, &path, rule(r), &fl.paint, (fl.opacity, fl.blend), bounds, bounds, true) {
                            return;
                        }
                        let Some(paint) = ex.paint(s, &fl.paint, bounds) else { return };
                        let bl = fl.blend != BlendMode::Normal;
                        if bl {
                            s.push_blend_mode(blend(fl.blend));
                        }
                        s.set_stroke(None);
                        s.set_fill(Some(Fill { paint, opacity: norm(fl.opacity), rule: rule(r) }));
                        s.draw_path(&path);
                        if bl {
                            s.pop();
                        }
                    });
                }
                AppearanceItem::Stroke(st) => {
                    if !st.visible || st.paint.is_none() || st.width <= 0.0 {
                        continue;
                    }
                    self.warn_raster(&st.effects, || "strokes".into());
                    self.overprinting(s, st.overprint, &st.paint, |ex, s| ex.stroke(s, bp, &path, r, st, page, bounds));
                }
            }
        }
        s.set_fill(None);
        s.set_stroke(None);
    }

    /// Draw a fill or stroke of `paint` with `draw`: as a form marked to overprint when it
    /// `overprints`, paints and overprints are kept (but not a white one with Discard White
    /// Overprint).
    pub(super) fn overprinting(&mut self, s: &mut Surface, overprints: bool, paint: &Paint, draw: impl FnOnce(&mut Self, &mut Surface)) {
        let white = || self.doc.setup.discard_white_overprint && vectorcraft_doc::overprint::is_white(paint);
        if overprints && self.overprint && !paint.is_none() && !white() {
            self.overprinted = true;
            crate::forms::form(s, Mark::Overprint, |s| draw(self, s));
        } else {
            draw(self, s);
        }
    }

    /// Is `p` a paint PDF has no paint for when it paints a fill (`fill`) or a stroke, which
    /// [`Self::area`] draws as art: a pattern swatch or a gradient drawn as pixels
    /// ([`vectorcraft_color::GradientPaint::draws_as_pixels`])?
    pub(super) fn area_paint(&self, p: &Paint, fill: bool) -> bool {
        match p {
            Paint::Pattern { pattern, .. } => self.doc.pattern(pattern).is_some(),
            Paint::Gradient(g) => g.draws_as_pixels(fill),
            _ => false,
        }
    }

    /// Paint the area `clip` (filled by `rule`; bounds `area`) with `p` at `transparency` (opacity,
    /// blend mode) when it is an [area paint](Self::area_paint) of a fill (`fill`) or a stroke: a
    /// pattern's [tiling pattern](Self::tiling) (else its tile instances covering it), or an image
    /// of a gradient drawn as pixels placed on `bounds` (the painted object's box, as on the
    /// canvas), clipped to it. False (nothing drawn) for other paints.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn area(
        &mut self,
        s: &mut Surface,
        clip: &Path,
        rule: krilla::paint::FillRule,
        p: &Paint,
        transparency: (f32, BlendMode),
        bounds: Rect,
        area: Rect,
        fill: bool,
    ) -> bool {
        if !self.area_paint(p, fill) {
            return false;
        }
        let (opacity, mode) = transparency;
        let tiling = match p {
            Paint::Pattern { pattern, xf } => self.tiling(s, pattern, *xf),
            _ => None,
        };
        let mut pushes = 0;
        if mode != BlendMode::Normal {
            s.push_blend_mode(blend(mode));
            pushes += 1;
        }
        if let Some(paint) = tiling {
            s.set_stroke(None);
            s.set_fill(Some(Fill { paint, opacity: norm(opacity), rule }));
            s.draw_path(clip);
            s.set_fill(None);
        } else {
            s.push_clip_path(clip, &rule);
            pushes += 1;
            if opacity < 1.0 {
                s.push_opacity(norm(opacity));
                pushes += 1;
            }
            let doc = self.doc;
            match p {
                Paint::Pattern { pattern, xf } => {
                    if let Some(def) = doc.pattern(pattern) {
                        self.warn("pattern fills nested more than 4 patterns deep, or with tiles too large for PDF, are written as their tiles");
                        for inst in def.instances_in(*xf, area) {
                            self.node(s, &inst, area, true);
                        }
                    }
                }
                Paint::Gradient(g) => self.field_image(s, g, bounds, area, fill),
                _ => {}
            }
        }
        for _ in 0..pushes {
            s.pop();
        }
        true
    }

    /// Paint stroke `st` of the shape `bp` (`path`), whose geometric bounds `bounds` place its
    /// unplaced gradients.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn stroke(&mut self, s: &mut Surface, bp: &BezPath, path: &Path, r: FillRule, st: &StrokeLayer, page: Rect, bounds: Rect) {
        if !stroke::is_plain(st) && self.brush_art(s, bp, st, page) {
            return;
        }
        if self.area_paint(&st.paint, false) && st.path_gradient().is_none() {
            // The area the stroke paints (dashes, caps, joins, profile, arrowheads, alignment), as
            // Outline Stroke makes it, painted with the pattern's tiles or the gradient's image (a
            // gradient along or across the stroke is written as slices below).
            let region = stroke::outline_region(&vectorcraft_geom::PathData::from_bezpath(bp), r, st).to_bezpath();
            if let Some(clip) = to_path(s, &region) {
                let area = region.bounding_box();
                self.area(s, &clip, krilla::paint::FillRule::NonZero, &st.paint, (st.opacity, st.blend), st.paint_bounds(bounds), area, false);
            }
            return;
        }
        let Some(paint) = self.paint(s, &st.paint, st.paint_bounds(bounds)) else { return };
        let w = stroke::for_writer(bp, st);
        // A stroke whose width, caps or miter spikes reach beyond [`MAX_COORD`] is left out.
        if !writable(s, Affine::IDENTITY, w.reach(st, bp.control_box()).inflate(1.0, 1.0)) {
            return;
        }
        let mut pushes = 0;
        if st.blend != BlendMode::Normal {
            s.push_blend_mode(blend(st.blend));
            pushes += 1;
        }
        match w.side {
            Some(StrokeAlign::Inside) => {
                s.push_clip_path(path, &rule(r));
                pushes += 1;
            }
            Some(StrokeAlign::Outside) => {
                // Clip to everything outside the path: a frame around all the stroke reaches
                // (miter spikes included) plus the path, even-odd.
                let mut outside = w.reach(st, bounds).inflate(1.0, 1.0).to_path(0.1);
                outside.extend(bp.iter());
                if let Some(p) = to_path(s, &outside) {
                    s.push_clip_path(&p, &krilla::paint::FillRule::EvenOdd);
                    pushes += 1;
                }
            }
            _ => {}
        }
        match &w.shape {
            WrittenShape::Stroke { width } => {
                let dash = st.dash.as_ref().filter(|d| d.is_dashed()).map(|d| {
                    let mut pat: Vec<f32> = d.pattern.iter().map(|v| *v as f32).collect();
                    if pat.len() % 2 == 1 {
                        pat.extend(pat.clone());
                    }
                    // An offset beyond [`MAX_COORD`] is written as its remainder after whole
                    // periods of the pattern.
                    let period: f64 = pat.iter().copied().map(f64::from).sum();
                    let offset = if in_range(&[d.offset]) { d.offset } else { d.offset.rem_euclid(period) };
                    StrokeDash { array: pat, offset: if offset.is_finite() { offset as f32 } else { 0.0 } }
                });
                s.set_fill(None);
                s.set_stroke(Some(Stroke {
                    paint,
                    width: *width as f32,
                    miter_limit: st.miter_limit.max(1.0) as f32,
                    line_cap: match st.cap {
                        LineCap::Butt => krilla::paint::LineCap::Butt,
                        LineCap::Round => krilla::paint::LineCap::Round,
                        LineCap::Square => krilla::paint::LineCap::Square,
                    },
                    line_join: match st.join {
                        LineJoin::Miter => krilla::paint::LineJoin::Miter,
                        LineJoin::Round => krilla::paint::LineJoin::Round,
                        LineJoin::Bevel => krilla::paint::LineJoin::Bevel,
                    },
                    opacity: norm(st.opacity),
                    dash,
                }));
                s.draw_path(path);
                s.set_stroke(None);
            }
            WrittenShape::Fill(outlines) if st.path_gradient().is_some() => {
                // A gradient along or across the stroke: slices clipped to its outlines, under
                // the stroke's opacity as a group.
                if let Some(ws) = stroke::written_slices(bp, r, st, outlines)
                    && let Some(clip) = to_path(s, &ws.clip)
                {
                    self.warn("gradients along or across strokes are exported as slices of linear gradients");
                    if st.opacity < 1.0 {
                        s.push_opacity(norm(st.opacity));
                        pushes += 1;
                    }
                    s.push_clip_path(&clip, &krilla::paint::FillRule::NonZero);
                    s.set_stroke(None);
                    for (shape, paint) in &ws.slices {
                        let b = shape.bounding_box();
                        if let (Some(paint), Some(p)) = (self.paint(s, paint, b), to_path(s, shape)) {
                            s.set_fill(Some(Fill { paint, opacity: NormalizedF32::ONE, rule: krilla::paint::FillRule::NonZero }));
                            s.draw_path(&p);
                        }
                    }
                    s.set_fill(None);
                    s.pop();
                }
            }
            WrittenShape::Fill(outlines) => {
                // The line and its arrowheads overlap: they take the opacity once, as a group.
                let grouped = outlines.len() > 1 && st.opacity < 1.0;
                if grouped {
                    s.push_opacity(norm(st.opacity));
                    pushes += 1;
                }
                let opacity = if grouped { NormalizedF32::ONE } else { norm(st.opacity) };
                s.set_stroke(None);
                s.set_fill(Some(Fill { paint, opacity, rule: krilla::paint::FillRule::NonZero }));
                for outline in outlines {
                    if let Some(p) = to_path(s, outline) {
                        s.draw_path(&p);
                    }
                }
                s.set_fill(None);
            }
        }
        for _ in 0..pushes {
            s.pop();
        }
    }

    /// Paint stroke `st` of the shape `bp` with its brush art (the stroke's opacity and blend mode
    /// over all of it). False when it has no known brush.
    fn brush_art(&mut self, s: &mut Surface, bp: &BezPath, st: &StrokeLayer, page: Rect) -> bool {
        let doc = self.doc;
        let brushes = self.brushes.get_or_insert_with(|| vectorcraft_brush::library(doc));
        let Some(b) = st.brush.as_deref().and_then(|name| brushes.iter().find(|b| b.name == name)) else { return false };
        let art = vectorcraft_brush::stroke_pieces(doc, b, bp, st);
        let mut pushes = 0;
        if st.blend != BlendMode::Normal {
            s.push_blend_mode(blend(st.blend));
            pushes += 1;
        }
        if st.opacity < 1.0 {
            s.push_opacity(norm(st.opacity));
            pushes += 1;
        }
        for piece in &art {
            self.node(s, piece, page, false);
        }
        for _ in 0..pushes {
            s.pop();
        }
        true
    }
}
