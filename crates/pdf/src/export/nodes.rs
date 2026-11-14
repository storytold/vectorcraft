use std::collections::HashMap;

use super::exporter::{UNDECODED, address, blend, color, constant_mask, cover, in_range, norm, rects_overlap, rule, to_path, writable, xf};
use super::{
    Affine, AppearanceItem, BezPath, BlendMode, Document, Exporter, Fill, FillRule, GradientKind, LineCap, LineJoin, Mark, Node, NodeKind,
    NormalizedF32, Paint, Path, Rect, Shape, Stroke, StrokeAlign, StrokeDash, StrokeLayer, Surface, WrittenShape, krilla, stroke,
};

/// An object's visual box for leaving out what is off the page. A plain group or layer also holds
/// the boxes of the members it is measured through ([`Node::editing_children`], in order), so that
/// the walk measures them once.
struct Measured {
    bounds: Option<Rect>,
    members: Vec<Measured>,
}

#[cfg(test)]
std::thread_local! {
    /// The objects this thread has measured for the walk and for the links (tests).
    pub(crate) static MEASURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Exporter<'_> {
    /// Opacity mask → luminosity soft mask. Outside the art the backdrop is black (clip) or white;
    /// invert is a white Difference rect on top (luminance is linear, so luma(1 − c) = 1 − luma(c)).
    fn soft_mask(&mut self, s: &mut Surface, m: &vectorcraft_doc::OpacityMask, page: Rect) -> Option<krilla::mask::Mask> {
        let transform = s.group_transform();
        let mut sb = s.stream_builder();
        let mut ms = Surface::stream(sb.surface());
        ms.push_transform(&transform);
        // An opaque backdrop when it matters: white outside the art (no clip), or black for the
        // inverting Difference pass below to turn white.
        if !m.clip || m.invert {
            cover(&mut ms, page, if m.clip { 0 } else { 255 }, 1.0);
        }
        // Mask art is a picture of its own: it takes no part in a knockout group around the object,
        // and has no inks to overprint.
        let knockout = std::mem::take(&mut self.knockout);
        let overprint = std::mem::take(&mut self.overprint);
        let image_left_out = self.image_left_out.replace(false);
        self.node(&mut ms, &m.art, page, true);
        let incomplete = self.image_left_out.unwrap_or(false);
        self.image_left_out = image_left_out.map(|outer| outer || incomplete);
        (self.knockout, self.overprint) = (knockout, overprint);
        if incomplete {
            ms.pop();
            ms.finish();
            return None;
        }
        if m.invert {
            ms.push_blend_mode(krilla::blend::BlendMode::Difference);
            cover(&mut ms, page, 255, 1.0);
            ms.pop();
        }
        ms.pop();
        ms.finish();
        Some(krilla::mask::Mask::new(sb.finish(), krilla::mask::MaskType::Luminosity))
    }

    /// The children of a group, as the elements of a knockout group when it is one.
    pub(super) fn children(&mut self, s: &mut Surface, children: &[std::sync::Arc<Node>], page: Rect) {
        self.measured_children(s, children, page, &[], false);
    }

    /// [`Self::children`] with the boxes of the last members in `measured`, in an isolated group
    /// when `isolated`.
    fn measured_children(&mut self, s: &mut Surface, children: &[std::sync::Arc<Node>], page: Rect, measured: &[Measured], isolated: bool) {
        if !self.knockout {
            if isolated {
                s.push_isolated();
            }
            let unmeasured = children.len().saturating_sub(measured.len());
            for (i, c) in children.iter().enumerate() {
                self.measured_node(s, c, page, false, i.checked_sub(unmeasured).and_then(|i| measured.get(i)));
            }
            if isolated {
                s.pop();
            }
            return;
        }
        let elements: Vec<(&Node, Measured)> = Node::knockout_elements(children)
            .into_iter()
            .map(|c| (&**c, self.measure(c)))
            .filter(|(_, m)| m.bounds.is_some_and(|b| rects_overlap(b.inflate(1.0, 1.0), page)))
            .collect();
        if elements.is_empty() {
            return;
        }
        if elements.iter().all(|(c, _)| c.knockout_outline().is_some()) {
            let pieces: Vec<&Node> = elements.iter().map(|(c, _)| *c).collect();
            if isolated {
                s.push_isolated();
            }
            if vectorcraft_effects::warp_pieces_translucent(self.doc, &pieces) {
                self.warp_pieces(s, &elements, page);
            } else {
                for (c, m) in &elements {
                    self.measured_node(s, c, page, false, Some(m));
                }
            }
            if isolated {
                s.pop();
            }
            return;
        }
        let several = elements.len() > 1;
        // A PDF knockout group (`/K`) knocks out by the area each element paints. Where an element
        // above another knocks out by less (its opacity or mask defines its knockout shape, or it
        // paints translucently), each element is drawn through one soft mask of where the
        // elements above it that overlap it paint, while those masks hold no more than
        // [`MAX_KNOCKOUT_MASK_OBJECTS`] objects and no brushed stroke.
        if several && !elements.iter().skip(1).all(|(c, _)| shape_is_coverage(c)) {
            let doc = self.doc;
            let brushes = self.brushes.get_or_insert_with(|| vectorcraft_brush::library(doc));
            match knockout_masks(BrushSearch::new(doc, brushes), &elements) {
                Ok(above) => {
                    // Every mask is made before anything is drawn: when the symbol objects the
                    // masks may hold run out, one would leave out art its element covers, and
                    // the group is written as a PDF knockout group instead.
                    let outer = std::mem::take(&mut self.mask_art_left_out);
                    let before = *self.mask_counter();
                    let mut masks = Vec::with_capacity(above.len());
                    for above in above {
                        let shapes: Vec<_> = above.into_iter().map(|j| &elements[j]).collect();
                        masks.push((!shapes.is_empty()).then(|| self.knockout_mask(s, &shapes, page)));
                    }
                    let left_out = std::mem::replace(&mut self.mask_art_left_out, outer);
                    if left_out {
                        *self.mask_counter() = before;
                        self.warn(KNOCKOUT_SHAPES);
                    } else {
                        if isolated {
                            s.push_isolated();
                        }
                        for ((c, m), mask) in elements.iter().zip(masks) {
                            if let Some(mask) = mask {
                                s.push_mask(mask);
                                self.measured_node(s, c, page, false, Some(m));
                                s.pop();
                            } else {
                                self.measured_node(s, c, page, false, Some(m));
                            }
                        }
                        if isolated {
                            s.pop();
                        }
                        return;
                    }
                }
                Err(warning) => self.warn(warning),
            }
        }
        let group = several || isolated;
        if group {
            s.push_group(krilla::blend::GroupOptions { isolated, knockout: several });
        }
        for (c, m) in &elements {
            // In a PDF knockout group each painting operation knocks out the ones before it. An
            // element drawn with several is one non-isolated group (drawn through a full alpha
            // mask), so that its own fills and strokes don't knock each other out; its opacity
            // or opacity mask makes it one already.
            let unit = several && !one_paint(c) && c.opacity >= 1.0 && c.extras().mask.as_deref().is_none_or(|m| m.disabled);
            if unit {
                let all = constant_mask(s, page, 1.0);
                s.push_mask(all);
            }
            self.measured_node(s, c, page, false, Some(m));
            if unit {
                s.pop();
            }
        }
        if group {
            s.pop();
        }
    }

    /// The pieces of a partly transparent mesh-warped image (`elements`, each with its box) as the
    /// elements of a knockout group ([`vectorcraft_effects::warp_pieces_translucent`]). Each piece
    /// is drawn through one soft mask of where the pieces above it that reach its box don't cover
    /// ([`vectorcraft_effects::knockout_silhouette`]): only neighbors overlap, so each mask holds a
    /// few triangles and no mask nests in another. A piece above that overlaps it only over opaque
    /// pixels ([`vectorcraft_effects::WarpSeams`]) is left out of its mask.
    fn warp_pieces(&mut self, s: &mut Surface, elements: &[(&Node, Measured)], page: Rect) {
        // Each piece's triangle and its box: a piece paints inside its triangle only.
        let silhouette = |c: &Node| {
            let b = c.knockout_outline()?.geometric_bounds()?;
            Some((vectorcraft_effects::knockout_silhouette(c, vectorcraft_doc::color::Color::WHITE)?, b))
        };
        let silhouettes: Vec<Option<(Node, Rect)>> = elements.iter().map(|(c, _)| silhouette(c)).collect();
        let pieces: Vec<&Node> = elements.iter().map(|(c, _)| *c).collect();
        let seams = vectorcraft_effects::WarpSeams::new(self.doc, &pieces);
        for (i, (c, measured)) in elements.iter().enumerate() {
            let Some((_, reach)) = &silhouettes[i] else { continue };
            let reach = reach.inflate(1.0, 1.0);
            let shapes: Vec<(&Node, Option<&Measured>)> = silhouettes
                .iter()
                .enumerate()
                .skip(i + 1)
                .filter_map(|(j, s)| s.as_ref().filter(|(_, b)| rects_overlap(*b, reach) && seams.shows_through(i, j)))
                .map(|(n, _)| (n, None))
                .collect();
            if shapes.is_empty() {
                self.measured_node(s, c, page, false, Some(measured));
                continue;
            }
            let mask = self.shapes_mask(s, &shapes, page);
            s.push_mask(mask);
            self.measured_node(s, c, page, false, Some(measured));
            s.pop();
        }
    }

    /// A luminosity mask that is 1 − the coverage of `shapes` (each with its box when measured):
    /// white, then black through an alpha mask of the shapes drawn one over another.
    fn shapes_mask(&mut self, s: &mut Surface, shapes: &[(&Node, Option<&Measured>)], page: Rect) -> krilla::mask::Mask {
        let transform = s.group_transform();
        let mut sb = s.stream_builder();
        let mut ms = Surface::stream(sb.surface());
        ms.push_transform(&transform);
        cover(&mut ms, page, 255, 1.0);
        let alpha = {
            let transform = ms.group_transform();
            let mut ab = ms.stream_builder();
            let mut als = Surface::stream(ab.surface());
            als.push_transform(&transform);
            let overprint = std::mem::take(&mut self.overprint);
            for (shape, measured) in shapes {
                self.measured_node(&mut als, shape, page, false, *measured);
            }
            self.overprint = overprint;
            als.pop();
            als.finish();
            krilla::mask::Mask::new(ab.finish(), krilla::mask::MaskType::Alpha)
        };
        ms.push_mask(alpha);
        cover(&mut ms, page, 0, 1.0);
        ms.pop();
        ms.pop();
        ms.finish();
        krilla::mask::Mask::new(sb.finish(), krilla::mask::MaskType::Luminosity)
    }

    /// A luminosity mask that is 1 − the knockout shapes of `shapes` (each with its box): white,
    /// then black through an alpha mask of each (at full object opacity without its own mask,
    /// unless those define its shape). The symbol instances in the copies count toward
    /// [`Exporter::mask_symbol_nodes`], not toward the objects the visible art may draw. A
    /// knockout mask drawn inside another one counts toward the same objects as the outer one.
    fn knockout_mask(&mut self, s: &mut Surface, shapes: &[&(&Node, Measured)], page: Rect) -> krilla::mask::Mask {
        if self.in_knockout_mask {
            return self.knockout_mask_art(s, shapes, page);
        }
        self.in_knockout_mask = true;
        let visible = std::mem::replace(&mut self.symbol_nodes, self.mask_symbol_nodes);
        let limit = std::mem::replace(&mut self.symbol_limit, vectorcraft_doc::MAX_SYMBOL_NODES);
        let mask = self.knockout_mask_art(s, shapes, page);
        self.mask_symbol_nodes = std::mem::replace(&mut self.symbol_nodes, visible);
        self.symbol_limit = limit;
        self.in_knockout_mask = false;
        mask
    }

    /// The symbol objects the copies drawn in knockout masks held, together: [`Self::symbol_nodes`]
    /// while a knockout mask is being drawn, [`Exporter::mask_symbol_nodes`] otherwise.
    fn mask_counter(&mut self) -> &mut u64 {
        if self.in_knockout_mask { &mut self.symbol_nodes } else { &mut self.mask_symbol_nodes }
    }

    /// [`Self::knockout_mask`], drawn within the symbol objects counted at the time.
    fn knockout_mask_art(&mut self, s: &mut Surface, shapes: &[&(&Node, Measured)], page: Rect) -> krilla::mask::Mask {
        let transform = s.group_transform();
        let mut sb = s.stream_builder();
        let mut ms = Surface::stream(sb.surface());
        ms.push_transform(&transform);
        cover(&mut ms, page, 255, 1.0);
        for (c, measured) in shapes.iter().map(|e| (e.0, &e.1)) {
            let shape = if c.knockout_shape {
                c.clone()
            } else {
                let mut s = c.clone().with_mask(None);
                s.opacity = 1.0;
                s
            };
            let alpha = {
                let transform = ms.group_transform();
                let mut ab = ms.stream_builder();
                let mut als = Surface::stream(ab.surface());
                als.push_transform(&transform);
                let overprint = std::mem::take(&mut self.overprint);
                self.measured_node(&mut als, &shape, page, false, Some(measured));
                self.overprint = overprint;
                als.pop();
                als.finish();
                krilla::mask::Mask::new(ab.finish(), krilla::mask::MaskType::Alpha)
            };
            ms.push_mask(alpha);
            cover(&mut ms, page, 0, 1.0);
            ms.pop();
        }
        ms.pop();
        ms.finish();
        krilla::mask::Mask::new(sb.finish(), krilla::mask::MaskType::Luminosity)
    }

    /// The box of `n`, and of the members of a plain group or layer.
    fn measure(&self, n: &Node) -> Measured {
        if !matches!(n.kind, NodeKind::Layer { clip: false, .. } | NodeKind::Group { clip: false, .. }) {
            #[cfg(test)]
            MEASURES.with(|m| m.set(m.get() + 1));
            return Measured { bounds: n.visual_bounds_with(&self.symbols), members: vec![] };
        }
        let members: Vec<_> = n.editing_children().iter().map(|c| self.measure(c)).collect();
        Measured { bounds: n.visual_of_members(members.iter().map(|m| m.bounds)), members }
    }

    pub(super) fn node(&mut self, s: &mut Surface, n: &Node, page: Rect, force: bool) {
        self.measured_node(s, n, page, force, None);
    }

    /// [`Self::node`] with the box of `n` in `measured`, when its parent measured it.
    fn measured_node(&mut self, s: &mut Surface, n: &Node, page: Rect, force: bool, measured: Option<&Measured>) {
        if self.layers
            && self.in_layer != address(n)
            && let NodeKind::Layer { printable, .. } = n.kind
            && (printable || self.non_printing)
            && let Some(&i) = self.layer_index.get(&address(n))
        {
            // A PDF layer: hidden layers and sublayers are written too (their group is off).
            let enclosing = std::mem::replace(&mut self.in_layer, address(n));
            let paper = self.paper;
            crate::forms::layer_form(s, i, paper, |s| self.node(s, n, page, true));
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
        // A container written whatever its box is has no use for one.
        let own = (measured.is_none() && !(force && n.is_container())).then(|| self.measure(n));
        let measured = measured.or(own.as_ref());
        match measured.map(|m| m.bounds) {
            Some(Some(b)) if !force && !rects_overlap(b.inflate(1.0, 1.0), page) => return,
            Some(None) if !n.is_container() => return,
            _ => {}
        }
        self.warn_raster(&n.appearance.effects, || format!("{} objects", n.kind_label().to_lowercase()));
        let container = n.is_container() && !matches!(n.kind, NodeKind::Compound { .. });
        // Whether this container's children knock each other out (a knockout group is written as a group).
        let knockout = n.knocks_out(self.knockout);
        let enclosing = std::mem::replace(&mut self.knockout, knockout);
        let (mut pushes, isolated_children) = transparency(s, n, page, container, knockout);
        let mask = n.extras().mask.as_deref().filter(|m| !m.disabled);
        let alpha = match (&n.kind, mask) {
            (NodeKind::Image(im), Some(m)) => self.masked_image(im, m, !n.extras().pixelated),
            _ => None,
        };
        if let Some(m) = mask
            && alpha.is_none()
        {
            let Some(mask) = self.soft_mask(s, m, page) else {
                self.knockout = enclosing;
                for _ in 0..pushes {
                    s.pop();
                }
                return;
            };
            s.push_mask(mask);
            pushes += 1;
        }
        match &n.kind {
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
                self.measured_children(s, children, page, measured.map_or(&[], |m| m.members.as_slice()), isolated_children);
            }
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
            NodeKind::Image(im) => match (n.extras().colorize.as_deref(), self.doc.shown_image(n, im)) {
                (Some(p @ Paint::Solid { .. }), im) if self.doc.image_blob(&im).is_none_or(vectorcraft_doc::ImageBlob::takes_colorize) => {
                    let im = &*im;
                    if writable(s, im.xf, Rect::new(0.0, 0.0, im.width.max(1).into(), im.height.max(1).into())) {
                        if let Some(colorized) = self.colorized_image(im, p, !n.extras().pixelated) {
                            self.image(s, im, Some(colorized), n.extras().pixelated);
                        } else {
                            self.image_left_out = self.image_left_out.map(|_| true);
                            self.warn(UNDECODED);
                        }
                    } else if self.image_left_out.is_some() {
                        self.image_left_out = Some(true);
                        self.warn(super::exporter::UNWRITABLE_IMAGE);
                    }
                }
                (_, im) => self.image(s, &im, alpha, n.extras().pixelated),
            },
            NodeKind::SymbolInstance { symbol, xf } => {
                // Within the objects a frame may draw of symbol instances, as on screen: nested
                // instances multiply.
                // Placing through 9-slice guides needs room for the most objects it can make
                // (`NineSliceMap::most_placed`), and the objects it makes count.
                // A dynamic symbol's instance shows its own appearance changes.
                let doc = self.doc;
                if let Some(sym) = doc.symbols.iter().find(|x| &x.name == symbol) {
                    let changed;
                    let art: &Node = if n.extras().symbol_overrides.is_empty() {
                        &sym.art
                    } else {
                        changed = vectorcraft_doc::symbol_overrides::dynamic_art(&sym.art, n);
                        &changed
                    };
                    let count = art.count() as u64;
                    let map = doc.symbol_slice_map(symbol, *xf);
                    let most = map.map_or(count, |m| m.most_placed(art, doc));
                    let within = self.symbol_nodes.saturating_add(most) <= self.symbol_limit;
                    let placed = within.then(|| {
                        vectorcraft_doc::charge_symbol_art(&mut self.symbol_nodes, most, || {
                            let art = match map {
                                Some(m) => m.place(art, doc),
                                None => {
                                    let mut a = art.clone();
                                    a.transform(*xf, false);
                                    a
                                }
                            };
                            // As `most_placed`, placed art counts its opacity-mask art.
                            let count = if map.is_some() { art.count_with_masks() } else { art.count() } as u64;
                            (art, count)
                        })
                    });
                    match placed.flatten() {
                        Some(art) => self.node(s, &art, page, true),
                        // A knockout mask that leaves the art out no longer matches what it masks.
                        None => self.mask_art_left_out |= self.in_knockout_mask,
                    }
                }
            }
            // A gradient mesh is a smooth patch shading where it can be written as one.
            NodeKind::Mesh(m) if self.mesh(s, m) => {}
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
            // Ruler guides draw on screen only.
            NodeKind::RulerGuide { .. } => {}
        }
        self.knockout = enclosing;
        for _ in 0..pushes {
            s.pop();
        }
    }

    /// Gradient mesh `m` as one PDF Type 6 (Coons patch) shading filling its outline (ISO 32000-1,
    /// 8.7.4.5.7): each patch blends its corner colours bilinearly, as the canvas draws it, and
    /// the viewer or RIP does so smoothly at its own resolution. Cutting the mesh into flat quads
    /// instead banded, left seams between them, and made files with many meshes very large.
    /// Corner colours are written as [`Self::page_color`] writes any colour; RGB among other
    /// models goes through RGB, as the writer needs one colour space a shading (ink-based ones:
    /// CMYK and grey, merge). `false` when it isn't written so: the caller draws it expanded.
    /// That is a mesh with see-through points (their opacities would be a soft mask of another
    /// patch shading, which a common viewer, Poppler, draws with gaps and too opaque; the quads
    /// carry their own opacity), an invalid mesh, or one beyond the coordinates a file holds.
    fn mesh(&mut self, s: &mut Surface, m: &vectorcraft_doc::live::GradientMesh) -> bool {
        if !m.is_valid() || !m.points.iter().all(|p| p.opacity >= 1.0) {
            return false;
        }
        let Some(bounds) = m.bounds() else { return false };
        if !writable(s, Affine::IDENTITY, bounds) {
            return false;
        }
        let Some(path) = to_path(s, &m.outline().to_bezpath()) else { return false };
        let page: Vec<vectorcraft_color::Color> = m.points.iter().map(|p| self.page_color(&p.color)).collect();
        let mixed = page.windows(2).any(|w| w[0].model() != w[1].model());
        let rgb = page.iter().any(|c| matches!(c, vectorcraft_color::Color::Rgb { .. } | vectorcraft_color::Color::Lab { .. }));
        let colors: Vec<krilla::color::Color> =
            page.iter().map(|c| if mixed && rgb { color(&c.in_model(vectorcraft_color::cms::Model::Rgb)) } else { color(c) }).collect();
        let pt = |p: vectorcraft_geom::Point| (p.x as f32, p.y as f32);
        let mut patches = Vec::with_capacity(m.rows as usize * m.cols as usize);
        for r in 0..m.rows as usize {
            for c in 0..m.cols as usize {
                // The corners in the order a Type 6 shading lists them: down the left edge (v),
                // along the bottom (u), back up the right edge and back along the top.
                let corners = [m.idx(r, c), m.idx(r + 1, c), m.idx(r + 1, c + 1), m.idx(r, c + 1)];
                let (left, bottom, right, top) = (m.edge_v(r, c), m.edge_u(r + 1, c), m.edge_v(r, c + 1), m.edge_u(r, c));
                let points =
                    [left.p0, left.p1, left.p2, left.p3, bottom.p1, bottom.p2, bottom.p3, right.p2, right.p1, right.p0, top.p2, top.p1].map(pt);
                let Some(colors) =
                    corners.iter().map(|&i| colors.get(i).cloned()).collect::<Option<Vec<_>>>().and_then(|v| <[_; 4]>::try_from(v).ok())
                else {
                    return false;
                };
                patches.push(krilla::paint::MeshPatch { points, colors, opacities: [NormalizedF32::ONE; 4] });
            }
        }
        let mesh = krilla::paint::MeshGradient { patches, transform: xf(Affine::IDENTITY), anti_alias: false };
        s.set_stroke(None);
        s.set_fill(Some(Fill { paint: mesh.into(), opacity: NormalizedF32::ONE, rule: krilla::paint::FillRule::NonZero }));
        s.draw_path(&path);
        s.set_fill(None);
        true
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
    /// ([`vectorcraft_color::GradientPaint::draws_as_pixels`]), but not a Perceptual one of spot
    /// inks ([`Self::perceptual_spot`]), a conic gradient that can't be a sweep shading (in
    /// PDF/A, or with more than [`super::paint::MAX_SWEEP_STOPS`] stops), a diamond gradient
    /// (four linear gradients, [`Self::diamond`]), or a linear or radial gradient that repeats or
    /// reflects (its stops repeated over the area, [`Self::padded`])?
    pub(super) fn area_paint(&self, p: &Paint, fill: bool) -> bool {
        match p {
            Paint::Pattern { pattern, .. } => self.doc.pattern(pattern).is_some(),
            Paint::Gradient(g) => {
                let sweep_unwritable = || !self.sweeps || g.gradient.expanded().nth(super::paint::MAX_SWEEP_STOPS).is_some();
                (g.draws_as_pixels(fill) && !self.perceptual_spot(g, fill))
                    || (g.gradient.kind == GradientKind::Conic && sweep_unwritable())
                    || g.gradient.kind == GradientKind::Diamond
                    || (g.gradient.is_spread() && matches!(g.gradient.kind, GradientKind::Linear | GradientKind::Radial))
            }
            _ => false,
        }
    }

    /// Paint the area `clip` (filled by `rule`; bounds `area`) with `p` at `transparency` (opacity,
    /// blend mode) when it is an [area paint](Self::area_paint) of a fill (`fill`) or a stroke: a
    /// pattern's [tiling pattern](Self::tiling) (else its tile instances covering it), a diamond
    /// gradient's four linear gradients, a repeating or reflecting linear or radial gradient's
    /// stops repeated over `area`, or an image of a gradient drawn as pixels, placed on
    /// `bounds` (the painted object's box, as on the canvas), clipped to it. False (nothing drawn)
    /// for other paints.
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
                Paint::Gradient(g) => {
                    let pixels = g.draws_as_pixels(fill) && !self.perceptual_spot(g, fill);
                    // A linear or radial gradient that repeats or reflects is drawn by `padded`.
                    let drawn = !pixels && self.padded(s, g, bounds, area);
                    if !drawn && (pixels || g.gradient.kind != GradientKind::Diamond || !self.diamond(s, g, bounds, area)) {
                        self.field_image(s, g, bounds, area, fill);
                    }
                }
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

/// Opens the blend mode and the transparency group `n` is drawn with (`container` when it is a
/// group or layer whose children `knockout` each other). Returns how many were opened, and
/// whether the children are to be drawn in an isolated group of their own: the knockout group of
/// an unclipped group's elements is its isolated group too.
fn transparency(s: &mut Surface, n: &Node, page: Rect, container: bool, knockout: bool) -> (usize, bool) {
    let mut pushes = 0;
    if n.blend != BlendMode::Normal {
        s.push_blend_mode(blend(n.blend));
        pushes += 1;
    }
    let mut isolated_children = false;
    if n.opacity < 1.0 || (container && (n.isolate || n.blend != BlendMode::Normal || knockout)) {
        if !n.isolate && n.blends_through() {
            // Not isolated: blending inside reaches the art below the group.
            let alpha = constant_mask(s, page, n.opacity);
            s.push_mask(alpha);
            pushes += 1;
        } else if n.opacity < 1.0 {
            s.push_opacity(norm(n.opacity));
            pushes += 1;
        } else if knockout && matches!(n.kind, NodeKind::Layer { clip: false, .. } | NodeKind::Group { clip: false, .. }) {
            isolated_children = true;
        } else {
            s.push_isolated();
            pushes += 1;
        }
    }
    (pushes, isolated_children)
}

/// The warning for a knockout group whose elements are too many to draw through soft masks: it is
/// written as a PDF knockout group, where an element knocks out all of the area it paints.
pub(crate) const KNOCKOUT_SHAPES: &str = "knockout groups with too many overlapping translucent elements are written as PDF knockout groups: \
     their translucent elements knock out what they cover fully";

/// The most objects the soft masks of one knockout group's elements draw between them: each mask
/// draws the elements above its own element that overlap it.
pub(crate) const MAX_KNOCKOUT_MASK_OBJECTS: usize = 1 << 16;

/// The most elements of one knockout group drawn through soft masks: finding the elements that
/// overlap compares every pair.
pub(crate) const MAX_MASKED_ELEMENTS: usize = 2048;

/// The warning for a knockout group in which a soft mask would draw an element with a brushed
/// stroke: it is written as a PDF knockout group, where an element knocks out all of the area it
/// paints.
pub(crate) const KNOCKOUT_BRUSHES: &str = "knockout groups with brushed strokes over other translucent elements are written as PDF knockout groups: \
     their translucent elements knock out what they cover fully";

/// For each element of a knockout group (with its box), the indices of the elements above it whose
/// boxes overlap its own. The warning to give instead ([`KNOCKOUT_SHAPES`]) when there are more than
/// [`MAX_MASKED_ELEMENTS`] elements or the masks would draw more than
/// [`MAX_KNOCKOUT_MASK_OBJECTS`] objects, or ([`KNOCKOUT_BRUSHES`]) when a mask would draw a
/// brushed stroke ([`BrushSearch`]), whose brush art the object count leaves out.
fn knockout_masks(mut search: BrushSearch, elements: &[(&Node, Measured)]) -> Result<Vec<Vec<usize>>, &'static str> {
    if elements.len() > MAX_MASKED_ELEMENTS {
        return Err(KNOCKOUT_SHAPES);
    }
    let counts: Vec<usize> = elements.iter().map(|(c, _)| c.count_with_masks()).collect();
    let mut checked: Vec<Option<bool>> = vec![None; elements.len()];
    let mut objects = 0usize;
    let mut out = Vec::with_capacity(elements.len());
    for (i, (_, below)) in elements.iter().enumerate() {
        let mut above = vec![];
        for (j, (over_node, over)) in elements.iter().enumerate().skip(i + 1) {
            if let (Some(a), Some(b)) = (below.bounds, over.bounds)
                && rects_overlap(a.inflate(1.0, 1.0), b.inflate(1.0, 1.0))
            {
                // The mask draws a copy of the element without its own opacity mask, unless the
                // mask defines its knockout shape.
                if *checked[j].get_or_insert_with(|| search.node(over_node, over_node.knockout_shape, 0)) {
                    return Err(if search.exhausted { KNOCKOUT_SHAPES } else { KNOCKOUT_BRUSHES });
                }
                objects = objects.checked_add(counts[j]).filter(|&o| o <= MAX_KNOCKOUT_MASK_OBJECTS).ok_or(KNOCKOUT_SHAPES)?;
                above.push(j);
            }
        }
        out.push(above);
    }
    Ok(out)
}

/// The art a [`BrushSearch`] looks through once: a symbol's or a pattern's, by name, or a
/// symbol's with one instance's appearance changes, by the symbol's name and the changes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SharedArt {
    Symbol,
    Pattern,
    Dynamic,
}

/// A search for the brushed strokes drawing an object draws, whose brush art can hold many more
/// objects than the stroked path.
struct BrushSearch<'a> {
    doc: &'a Document,
    /// The brushes the export draws strokes with ([`Exporter::brush_art`]); a stroke whose brush
    /// is not among them is drawn as a plain stroke.
    brushes: &'a [vectorcraft_brush::Brush],
    /// The answer for each symbol's and pattern's art, and each symbol's art with an instance's
    /// appearance changes, once found: false while it is being looked through, so that art that
    /// uses itself ends the search.
    seen: HashMap<(SharedArt, String), bool>,
    /// The objects the search may still visit or copy: [`MAX_KNOCKOUT_MASK_OBJECTS`] at the start.
    left: usize,
    /// Whether the search stopped because [`Self::left`] ran out; it then answers true.
    exhausted: bool,
}

impl<'a> BrushSearch<'a> {
    fn new(doc: &'a Document, brushes: &'a [vectorcraft_brush::Brush]) -> Self {
        Self { doc, brushes, seen: HashMap::new(), left: MAX_KNOCKOUT_MASK_OBJECTS, exhausted: false }
    }

    /// Takes `n` objects from [`Self::left`]; false, and the search exhausted, when fewer are left.
    fn spend(&mut self, n: usize) -> bool {
        match self.left.checked_sub(n) {
            Some(left) if !self.exhausted => {
                self.left = left;
                true
            }
            _ => {
                (self.left, self.exhausted) = (0, true);
                false
            }
        }
    }

    /// Takes the objects of `n` (its members and opacity-mask art included) from [`Self::left`],
    /// counting no further than what is left; false when they don't fit.
    fn spend_tree(&mut self, n: &Node) -> bool {
        self.spend(1)
            && n.extras().mask.as_deref().is_none_or(|m| self.spend_tree(&m.art))
            && n.children().into_iter().flatten().all(|c| self.spend_tree(c))
    }

    /// Whether drawing `n` draws a brushed stroke: on `n` or its visible members (a compound
    /// path's members are drawn with its own paint), in the art of the opacity masks that are on
    /// (`n`'s own when `own_mask`), of symbol instances and inline graphics, of live objects as
    /// evaluated, and of the patterns that fills, strokes and type paint with. `depth` counts the
    /// objects and the mask, symbol and pattern art above `n`: past
    /// [`vectorcraft_doc::depth::MAX_DRAWN_DEPTH`] the search stops and counts as a brush, and so
    /// it does when the objects it may visit or copy ([`Self::left`]) run out.
    fn node(&mut self, n: &Node, own_mask: bool, depth: usize) -> bool {
        if depth > vectorcraft_doc::depth::MAX_DRAWN_DEPTH || !self.spend(1) {
            return true;
        }
        let depth = depth + 1;
        // A fill is drawn when it is visible, and a stroke when it also paints and has a width
        // (as `Exporter::shape` draws them).
        for item in n.appearance.items.iter().filter(|i| i.visible()) {
            let paint = match item {
                AppearanceItem::Fill(f) => &f.paint,
                AppearanceItem::Stroke(st) if st.paint.is_none() || st.width <= 0.0 => continue,
                AppearanceItem::Stroke(st) => {
                    if st.brush.as_deref().is_some_and(|name| self.brushes.iter().any(|b| b.name == name)) {
                        return true;
                    }
                    &st.paint
                }
            };
            if self.paint(paint, depth) {
                return true;
            }
        }
        if own_mask && n.extras().mask.as_deref().is_some_and(|m| !m.disabled && self.node(&m.art, true, depth)) {
            return true;
        }
        match &n.kind {
            NodeKind::SymbolInstance { symbol, .. } => {
                // A dynamic symbol's instance draws its art with its own appearance changes: the
                // art is copied once for each set of changes, counted before it is copied.
                let doc = self.doc;
                match doc.symbols.iter().find(|s| &s.name == symbol) {
                    Some(s) if !n.extras().symbol_overrides.is_empty() => {
                        let key = format!("{symbol}\u{0}{:?}", n.extras().symbol_overrides);
                        self.shared(SharedArt::Dynamic, &key, |search| {
                            !search.spend_tree(&s.art) || search.node(&vectorcraft_doc::symbol_overrides::dynamic_art(&s.art, n), true, depth)
                        })
                    }
                    _ => self.symbol(symbol, depth),
                }
            }
            NodeKind::Text(t) => t.runs.iter().any(|r| {
                let stroke = r.style.stroke_width > 0.0 && self.paint(&r.style.stroke, depth);
                stroke || self.paint(&r.style.fill, depth) || r.inline.as_ref().is_some_and(|ig| self.symbol(&ig.symbol, depth))
            }),
            NodeKind::Compound { .. } => false,
            // Live objects are drawn as their evaluated art, as `Exporter::measured_node` draws them.
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::Intertwine { .. } => {
                // Each live object is reached once in the art searched (shared art is searched
                // once); its evaluated art is counted before it is searched.
                let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                !self.spend_tree(&g) || g.children().into_iter().flatten().filter(|c| c.visible).any(|c| self.node(c, true, depth))
            }
            _ => n.children().into_iter().flatten().filter(|c| c.visible).any(|c| self.node(c, true, depth)),
        }
    }

    /// Whether the art of the symbol named `name` draws a brushed stroke.
    fn symbol(&mut self, name: &str, depth: usize) -> bool {
        let doc = self.doc;
        let Some(s) = doc.symbols.iter().find(|s| s.name == name) else { return false };
        self.shared(SharedArt::Symbol, name, |search| search.node(&s.art, true, depth))
    }

    /// Whether `paint` is a pattern whose tile art draws a brushed stroke.
    fn paint(&mut self, paint: &Paint, depth: usize) -> bool {
        let Paint::Pattern { pattern, .. } = paint else { return false };
        let doc = self.doc;
        let Some(def) = doc.pattern(pattern) else { return false };
        self.shared(SharedArt::Pattern, pattern, |search| def.art.iter().filter(|c| c.visible).any(|c| search.node(c, true, depth)))
    }

    /// The answer of `search` for the shared art `kind` named `name`, found once.
    fn shared(&mut self, kind: SharedArt, name: &str, search: impl FnOnce(&mut Self) -> bool) -> bool {
        let key = (kind, name.to_string());
        if let Some(&b) = self.seen.get(&key) {
            return b;
        }
        self.seen.insert(key.clone(), false);
        let b = search(self);
        self.seen.insert(key, b);
        b
    }
}

/// Whether `n` is drawn with one painting operation at most: a path or compound path with at most
/// one visible fill or stroke that paints, in a solid color, without effects, a brush or
/// arrowheads.
fn one_paint(n: &Node) -> bool {
    let mut items = n.appearance.items.iter().filter(|i| i.visible() && !matches!(i.paint(), Paint::None));
    let simple = items.next().is_none_or(|i| {
        let plain = match i {
            AppearanceItem::Fill(_) => true,
            AppearanceItem::Stroke(st) => st.brush.is_none() && st.start_arrow.is_none() && st.end_arrow.is_none(),
        };
        plain && i.effects().is_empty() && matches!(i.paint(), Paint::Solid { .. })
    });
    matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) && n.appearance.effects.is_empty() && simple && items.next().is_none()
}

/// Whether element `c` of a knockout group knocks out all of the area it paints, as an element of
/// a PDF knockout group does: its opacity and opacity mask are full or don't define its knockout
/// shape, and every part of it paints opaquely.
fn shape_is_coverage(c: &Node) -> bool {
    let own = !c.knockout_shape || (c.opacity >= 1.0 && c.extras().mask.as_deref().is_none_or(|m| m.disabled));
    own && paints_opaque(c)
}

/// Whether what `n` paints is opaque wherever it paints, at its own opacity and mask: its visible
/// fills and strokes are solid colors or gradients with opaque stops, without effects or brushes,
/// and so are the members of a group or compound path, at full opacity without an opacity mask.
fn paints_opaque(n: &Node) -> bool {
    let paint = |p: &Paint| match p {
        Paint::None | Paint::Solid { .. } => true,
        Paint::Gradient(g) => g.gradient.stops.iter().all(|s| s.opacity >= 1.0),
        _ => false,
    };
    let items = n.appearance.items.iter().all(|i| {
        let brush = matches!(i, AppearanceItem::Stroke(s) if s.brush.is_some());
        !i.visible() || (i.opacity() >= 1.0 && i.effects().is_empty() && !brush && paint(i.paint()))
    });
    let member =
        |c: &std::sync::Arc<Node>| !c.visible || (c.opacity >= 1.0 && c.extras().mask.as_deref().is_none_or(|m| m.disabled) && paints_opaque(c));
    n.appearance.effects.is_empty()
        && items
        && match &n.kind {
            NodeKind::Path { .. } => true,
            NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => children.iter().all(member),
            NodeKind::Mesh(m) => m.points.iter().all(|p| p.opacity >= 1.0),
            _ => false,
        }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vectorcraft_doc::{Appearance, NodeId, Symbol, SymbolOverride};
    use vectorcraft_geom::shapes;

    use super::*;

    /// An instance of a dynamic symbol whose art holds an instance of the same symbol with the same
    /// appearance changes ends the search for brushed strokes where the art comes round again,
    /// after a few objects, instead of recursing until the stack runs out.
    #[test]
    fn a_dynamic_symbol_holding_itself_ends_the_brush_search() {
        let mut d = Document::new(200.0, 200.0);
        let instance = || {
            let mut n = Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: "Loop".into(), xf: Affine::translate((40.0, 0.0)) });
            n.extras_mut().symbol_overrides = vec![SymbolOverride { path: vec![0], opacity: Some(0.5), ..Default::default() }];
            n
        };
        let square = Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(Paint::None, Paint::None, 0.0));
        let art = Node::group(NodeId(0), vec![Arc::new(square), Arc::new(instance())]);
        d.symbols.push(Symbol { dynamic: true, name: "Loop".into(), art: Arc::new(art) });
        let mut search = BrushSearch::new(&d, &[]);
        assert!(!search.node(&instance(), true, 0));
        assert!(!search.exhausted);
        assert!(MAX_KNOCKOUT_MASK_OBJECTS - search.left < 10, "{} objects", MAX_KNOCKOUT_MASK_OBJECTS - search.left);
    }

    /// A group of 1,000 instances of a dynamic symbol whose art holds 100 squares, each instance
    /// with appearance changes of its own: copying and looking through every instance's art would
    /// take about 203,000 objects, and the search stops when [`MAX_KNOCKOUT_MASK_OBJECTS`] are
    /// used, counting as a brush. With the same changes on every instance, the art is copied
    /// once.
    #[test]
    fn the_brush_search_stops_within_its_object_limit() {
        let mut d = Document::new(200.0, 200.0);
        let square =
            Arc::new(Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::basic(Paint::None, Paint::None, 0.0)));
        d.symbols.push(Symbol { dynamic: true, name: "Dots".into(), art: Arc::new(Node::group(NodeId(0), vec![square; 100])) });
        let group = |distinct: bool| {
            let instances = (0..1000u32)
                .map(|i| {
                    let mut n = Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: "Dots".into(), xf: Affine::IDENTITY });
                    let path = if distinct { vec![0, i] } else { vec![0] };
                    n.extras_mut().symbol_overrides = vec![SymbolOverride { path, opacity: Some(0.5), ..Default::default() }];
                    Arc::new(n)
                })
                .collect();
            Node::group(NodeId(0), instances)
        };
        let mut search = BrushSearch::new(&d, &[]);
        assert!(search.node(&group(true), true, 0));
        assert!(search.exhausted);
        assert_eq!(search.left, 0);
        let mut search = BrushSearch::new(&d, &[]);
        assert!(!search.node(&group(false), true, 0));
        assert!(!search.exhausted);
        // The group, the 1,000 instances, and one copy of the art counted and looked through.
        assert_eq!(MAX_KNOCKOUT_MASK_OBJECTS - search.left, 1 + 1000 + 101 + 101);
    }

    /// A placed file read as one square with a 3 pt brushed stroke.
    fn brushed_art() -> (Document, Node, Rect) {
        let mut d = Document::new(40.0, 20.0);
        d.layers.clear();
        let mut st = StrokeLayer::new(Paint::solid(vectorcraft_color::Color::BLACK), 3.0);
        st.brush = Some("3 pt. Round".into());
        let mut square = Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 20.0)), Appearance::basic(Paint::None, Paint::None, 0.0));
        square.appearance.items.push(AppearanceItem::Stroke(st));
        let art = Node::group(NodeId(0), vec![Arc::new(square)]);
        (d, art, Rect::new(0.0, 0.0, 40.0, 20.0))
    }

    /// A placed document is drawn as the art read from its file, and the brushed strokes in that
    /// art count.
    #[test]
    fn brushed_strokes_in_a_placed_documents_art_count() {
        let d = Document::new(200.0, 200.0);
        let placed = vectorcraft_doc::PlacedDocument {
            link: vectorcraft_doc::LinkInfo::new("/art/placed.vectorcraft"),
            key: "placed-1".into(),
            bounding: false,
            page_box: vectorcraft_doc::placed_document::PageBox::Crop,
            width: 40.0,
            height: 20.0,
            xf: Affine::IDENTITY,
            placement: Default::default(),
        };
        // The art the export reads for it before drawing (`Document::with_placed_art`).
        vectorcraft_doc::placed_document::prime(&placed, Arc::new(vec![]), brushed_art());
        let n = Node::new(NodeId(0), NodeKind::PlacedDocument(Box::new(placed)));
        let brushes = vectorcraft_brush::library(&d);
        assert!(BrushSearch::new(&d, &brushes).node(&n, true, 0));
    }
}
