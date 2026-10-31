use super::output::{sanitize_id, valid_xml_value, xml_attr_escape};
use super::paint::{StrokePlan, has_raster, pixel_gradient};
use super::text::OUTLINE_TOLERANCE;
use super::{
    Affine, AppearanceItem, BezPath, BlendMode, Document, FillRule, Node, NodeKind, Paint, PathData, Props, Rect, StrokeLayer, Writer, Written,
    WrittenShape, blend_css, css, fmt_num, stroke, xml_escape,
};

/// A symbol's `<symbol>` def and the instance transforms that can `<use>` it.
pub(super) struct SymbolDef {
    /// The def's id, once written.
    id: Option<String>,
    reuse: Reuse,
}

/// Which instance transforms paint a symbol's art exactly as the def under that transform (the
/// canvas transforms the art's geometry alone: stroke weights, effects, patterns and unlinked
/// masks stay put).
#[derive(Clone, Copy)]
pub(crate) struct Reuse {
    /// Translations (no pattern paints or unlinked masks).
    moves: bool,
    /// Rotations and reflections (nor effects or brushes).
    turns: bool,
    /// Any other transform (nor strokes or live objects).
    scales: bool,
}

/// How deep symbols may sit in one another's art (deeper instances are left out).
const MAX_SYMBOL_NEST: u32 = 8;

impl Reuse {
    const NONE: Self = Self { moves: false, turns: false, scales: false };

    /// What a def of symbol art `art` stands for.
    pub(crate) fn of(doc: &Document, art: &Node) -> Self {
        let mut r = Self { moves: true, turns: true, scales: true };
        r.scan(doc, art, 0);
        r
    }

    /// Narrow down to what `n` (inside symbols nested `depth` deep) allows.
    fn scan(&mut self, doc: &Document, n: &Node, depth: u32) {
        let pattern = |p: &Paint| matches!(p, Paint::Pattern { .. });
        let text_patterns = matches!(&n.kind, NodeKind::Text(t) if t.runs.iter().any(|r| pattern(&r.style.fill) || pattern(&r.style.stroke)));
        let unlinked_mask = n.extras().mask.as_ref().is_some_and(|m| !m.linked);
        if depth > MAX_SYMBOL_NEST || text_patterns || unlinked_mask || vectorcraft_doc::pattern::uses_pattern(n, None) {
            *self = Self::NONE;
            return;
        }
        let effects = |fx: &[vectorcraft_doc::Effect]| fx.iter().any(|e| e.visible);
        if effects(&n.appearance.effects) || n.appearance.items.iter().any(|i| effects(i.effects())) {
            (self.turns, self.scales) = (false, false);
        }
        for i in &n.appearance.items {
            if let AppearanceItem::Stroke(s) = i
                && s.visible
                && !s.paint.is_none()
                && s.width > 0.0
            {
                // Stroke weights don't scale; brushes can orient their art to the page.
                self.scales = false;
                self.turns &= s.brush.is_none();
            }
        }
        match &n.kind {
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::Intertwine { .. } => (self.turns, self.scales) = (false, false),
            NodeKind::SymbolInstance { symbol, .. } => {
                if let Some(s) = doc.symbols.iter().find(|s| s.name == *symbol) {
                    self.scan(doc, &s.art, depth + 1);
                }
            }
            _ => {}
        }
        if let Some(m) = &n.extras().mask {
            self.scan(doc, &m.art, depth);
        }
        for c in n.children().into_iter().flatten() {
            self.scan(doc, c, depth);
        }
    }

    /// Can the def stand for an instance with transform `xf`?
    pub(crate) fn allows(self, xf: Affine) -> bool {
        const EPS: f64 = 1e-9;
        let [a, b, c, d, _, _] = xf.as_coeffs();
        if (a - 1.0).abs() < EPS && b.abs() < EPS && c.abs() < EPS && (d - 1.0).abs() < EPS {
            return self.moves;
        }
        let rigid = (a * a + b * b - 1.0).abs() < EPS && (c * c + d * d - 1.0).abs() < EPS && (a * c + b * d).abs() < EPS;
        if rigid { self.turns } else { self.scales }
    }
}

/// Art about to be written moved by `m.inverse()` (a symbol instance's): map what the canvas
/// leaves on the page by `m` first, so it stays there. That is every pattern paint's placement
/// and the art of unlinked opacity masks.
fn pin_to_page(n: &mut Node, m: Affine) {
    vectorcraft_doc::pattern::transform_pattern_paints(n, m);
    if let Some(mask) = n.extras_if_any().and_then(|e| e.mask.as_deref_mut()) {
        let linked = mask.linked;
        let art = std::sync::Arc::make_mut(&mut mask.art);
        if !linked {
            art.transform(m, false);
        }
        pin_to_page(art, m);
    }
    for c in n.children_mut().into_iter().flatten() {
        pin_to_page(std::sync::Arc::make_mut(c), m);
    }
}

/// What SVG Tiny 1.2 does without masks.
const TINY_MASKS: &str = "SVG Tiny 1.2 has no masks: opacity masks are left out and knockout groups written as plain groups";

/// A region covering any artwork (mask and filter extents).
const BIG: &str = "x=\"-100000\" y=\"-100000\" width=\"200000\" height=\"200000\"";

/// Attributes of every `<mask>`: user-space units, and luminance taken from the sRGB values (as the
/// canvas and PDF take it) rather than from linearised ones.
pub(super) const MASK: &str = "maskUnits=\"userSpaceOnUse\" color-interpolation=\"sRGB\"";

/// Attribute of an exported opacity mask listing its options that differ from the defaults
/// (clipping, not inverted): `noclip`, `invert`.
pub(crate) const MASK_FLAGS: &str = "data-vectorcraft-mask";

impl Writer<'_> {
    /// Paint a shape (`d`) with a node's appearance stack. `paths` is the same shape in document
    /// space, for strokes that aren't plain (it may be empty when all are). `basic` is the shape
    /// as an SVG basic shape element ([`Writer::basic_shape`]) to paint instead of `<path>`.
    pub(super) fn shape(&mut self, n: &Node, d: &str, basic: Option<(&str, String)>, paths: &[&PathData], rule: FillRule, bounds: Option<Rect>) {
        let (tag, geom) = basic.unwrap_or_else(|| ("path", format!(" d=\"{d}\"")));
        let id = self.id_attr(n);
        let items: Vec<&AppearanceItem> = n
            .appearance
            .items
            .iter()
            .filter(|i| match i {
                AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
                AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0,
            })
            .collect();
        let mut bp = None;
        let plans: Vec<Option<StrokePlan>> = items
            .iter()
            .map(|i| match i {
                AppearanceItem::Stroke(s) => Some(self.stroke_plan(s, &mut bp, paths)),
                AppearanceItem::Fill(_) => None,
            })
            .collect();
        let fills = items.iter().filter(|i| matches!(i, AppearanceItem::Fill(_))).count();
        let strokes = items.len() - fills;
        // A stroke's paint box is the bounds grown by half its weight, as on the canvas.
        let paint_bounds = |s: &StrokeLayer| bounds.map(|b| s.paint_bounds(b));
        let simple = fills <= 1
            && strokes <= 1
            && !items.iter().any(|i| has_raster(i.effects()))
            && items.iter().all(|i| i.blend() == BlendMode::Normal)
            && !items.iter().any(|i| pixel_gradient(i.paint(), matches!(i, AppearanceItem::Fill(_))).is_some())
            && plans.iter().flatten().all(|p| matches!(p, StrokePlan::Written(Written { shape: WrittenShape::Stroke { .. }, side: None })));
        if simple {
            let mut p = Props::new();
            match items.iter().find_map(|i| if let AppearanceItem::Fill(f) = i { Some(f) } else { None }) {
                Some(f) => self.fill_props(f, rule, bounds, &mut p),
                None => {
                    p.push(("fill", "none".into()));
                    if rule == FillRule::EvenOdd {
                        p.push(("fill-rule", "evenodd".into()));
                    }
                }
            }
            if let Some(s) = items.iter().find_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s) } else { None }) {
                self.stroke_props(s, s.width, paint_bounds(s), &mut p);
                if matches!(items.first(), Some(AppearanceItem::Stroke(_))) && fills == 1 {
                    p.push(("paint-order", "stroke".into()));
                }
            }
            p.extend(css::transparency(n));
            let a = self.attrs(&p);
            self.line(&format!("<{tag}{id}{geom}{a}/>"));
            return;
        }
        let a = self.attrs(&css::transparency(n));
        self.line(&format!("<g{id}{a}>"));
        self.depth += 1;
        for (it, plan) in items.into_iter().zip(plans) {
            // The item's own raster effects filter that item alone.
            let filters = self.open_filters(n, it.effects());
            let mut p = Props::new();
            match (it, plan) {
                (AppearanceItem::Fill(f), _) => {
                    let blend = (f.blend != BlendMode::Normal).then(|| ("mix-blend-mode", blend_css(f.blend).to_string()));
                    if let (Some(g), Some(b)) = (pixel_gradient(&f.paint, true), bounds) {
                        p.extend((f.opacity < 1.0).then(|| ("opacity", fmt_num(f.opacity as f64, 3))));
                        p.extend(blend);
                        self.raster_paint(g, b, &[d.to_string()], rule, &p, "", true);
                    } else {
                        self.fill_props(f, rule, bounds, &mut p);
                        p.extend(blend);
                        let a = self.attrs(&p);
                        self.line(&format!("<{tag}{geom}{a}/>"));
                    }
                }
                (AppearanceItem::Stroke(s), Some(StrokePlan::Brush(art))) => {
                    // The brush art takes the stroke's opacity and blend mode as a group.
                    if s.opacity < 1.0 {
                        p.push(("opacity", fmt_num(s.opacity as f64, 3)));
                    }
                    if s.blend != BlendMode::Normal {
                        p.push(("mix-blend-mode", blend_css(s.blend).into()));
                    }
                    let a = self.attrs(&p);
                    self.line(&format!("<g{a}>"));
                    self.depth += 1;
                    for piece in &art {
                        self.node(piece);
                    }
                    self.depth -= 1;
                    self.line("</g>");
                }
                (AppearanceItem::Stroke(s), Some(StrokePlan::Written(w))) => {
                    let side = self.side_attr(w.side, d, rule, w.reach(s, bounds.unwrap_or_default()));
                    let blend = (s.blend != BlendMode::Normal).then(|| ("mix-blend-mode", blend_css(s.blend).to_string()));
                    let opacity = (s.opacity < 1.0).then(|| ("opacity", fmt_num(s.opacity as f64, 3)));
                    match (&w.shape, pixel_gradient(&s.paint, false).zip(paint_bounds(s))) {
                        (WrittenShape::Stroke { width }, Some((g, b))) => {
                            // The stroke's outline clips the gradient's image.
                            let bp = bp.get_or_insert_with(|| doc_path(paths));
                            let outline = stroke::line_outline(bp, s, *width, OUTLINE_TOLERANCE);
                            let clip = self.path_d(&PathData::from_bezpath(&outline), self.xf);
                            p.extend(opacity);
                            p.extend(blend);
                            self.raster_paint(g, b, &[clip], FillRule::NonZero, &p, &side, false);
                        }
                        (WrittenShape::Fill(outlines), Some((g, b))) if s.path_gradient().is_none() => {
                            let clip: Vec<String> = outlines.iter().map(|o| self.path_d(&PathData::from_bezpath(o), self.xf)).collect();
                            p.extend(opacity);
                            p.extend(blend);
                            self.raster_paint(g, b, &clip, FillRule::NonZero, &p, &side, false);
                        }
                        (WrittenShape::Stroke { width }, _) => {
                            p.push(("fill", "none".into()));
                            self.stroke_props(s, *width, paint_bounds(s), &mut p);
                            p.extend(blend);
                            let a = self.attrs(&p);
                            self.line(&format!("<{tag}{geom}{a}{side}/>"));
                        }
                        (WrittenShape::Fill(outlines), _) if s.path_gradient().is_some() => {
                            // A gradient along or across the stroke: slices clipped to its outlines.
                            if let Some(ws) = bp.as_ref().and_then(|bp| stroke::written_slices(bp, rule, s, outlines)) {
                                p.extend(opacity);
                                p.extend(blend);
                                self.sliced(&ws, &p, &side);
                            }
                        }
                        (WrittenShape::Fill(outlines), _) => {
                            let paint = self.paint(&s.paint, paint_bounds(s));
                            let ds: Vec<String> = outlines.iter().map(|o| self.path_d(&PathData::from_bezpath(o), self.xf)).collect();
                            let opacity = (s.opacity < 1.0).then(|| fmt_num(s.opacity as f64, 3));
                            if let [one] = &ds[..] {
                                p.push(("fill", paint));
                                p.extend(opacity.map(|o| ("fill-opacity", o)));
                                p.extend(blend);
                                let a = self.attrs(&p);
                                self.line(&format!("<path d=\"{one}\"{a}{side}/>"));
                            } else if !ds.is_empty() {
                                // The line and its arrowheads overlap: one group takes the opacity.
                                p.extend(opacity.map(|o| ("opacity", o)));
                                p.extend(blend);
                                let a = self.attrs(&p);
                                self.line(&format!("<g{a}{side}>"));
                                self.depth += 1;
                                let fill = self.attrs(&vec![("fill", paint)]);
                                for one in &ds {
                                    self.line(&format!("<path d=\"{one}\"{fill}/>"));
                                }
                                self.depth -= 1;
                                self.line("</g>");
                            }
                        }
                    }
                }
                (AppearanceItem::Stroke(_), None) => {}
            }
            self.close_filters(filters);
        }
        self.depth -= 1;
        self.line("</g>");
    }

    /// A stroke whose gradient runs along or across it ([`stroke::written_slices`]): a group (with
    /// `props` and the `side` attribute) of its slices, clipped to its outlines.
    fn sliced(&mut self, ws: &stroke::WrittenSlices, props: &Props, side: &str) {
        self.warn("gradients along or across strokes are written as slices of linear gradients");
        let cid = self.fresh_id("clip-path");
        let clip = self.path_d(&PathData::from_bezpath(&ws.clip), self.xf);
        self.def(1, &format!("<clipPath id=\"{cid}\">"));
        self.def(2, &format!("<path d=\"{clip}\"/>"));
        self.def(1, "</clipPath>");
        let a = self.attrs(props);
        self.line(&format!("<g{a}{side}>"));
        self.depth += 1;
        self.line(&format!("<g clip-path=\"url(#{cid})\">"));
        self.depth += 1;
        for (shape, paint) in &ws.slices {
            let fill = self.paint(paint, None);
            let fill = self.attrs(&vec![("fill", fill)]);
            let d = self.path_d(&PathData::from_bezpath(shape), self.xf);
            self.line(&format!("<path d=\"{d}\"{fill}/>"));
        }
        self.depth -= 1;
        self.line("</g>");
        self.depth -= 1;
        self.line("</g>");
    }

    /// An object with an opacity mask: `<g mask="url(#…)">` around the unmasked object. The mask
    /// art goes in `<defs>`; no-clip adds a white backdrop and invert a colour-inverting filter.
    fn masked(&mut self, n: &Node, m: &vectorcraft_doc::OpacityMask) {
        if self.tiny() {
            self.warn(TINY_MASKS);
            let mut bare = n.clone();
            bare.extras_mut().mask = None;
            return self.node_body(&bare);
        }
        let mid = self.fresh_id("mask");
        // Mask art is a picture of its own: it takes no part in a knockout group around the object.
        let knockout = std::mem::take(&mut self.knockout);
        let art = self.detached(2, |w| w.node(&m.art));
        self.knockout = knockout;
        let inv = m.invert.then(|| {
            let fid = self.fresh_id("invert");
            self.def(1, &format!("<filter id=\"{fid}\" filterUnits=\"userSpaceOnUse\" color-interpolation-filters=\"sRGB\" {BIG}>"));
            self.def(2, "<feColorMatrix type=\"matrix\" values=\"-1 0 0 0 1 0 -1 0 0 1 0 0 -1 0 1 0 0 0 1 0\"/>");
            self.def(1, "</filter>");
            fid
        });
        // The mask's options, so import restores them (and its art without backdrop and filter).
        let flags: Vec<&str> = [(!m.clip).then_some("noclip"), m.invert.then_some("invert")].into_iter().flatten().collect();
        let data = if flags.is_empty() { String::new() } else { format!(" {MASK_FLAGS}=\"{}\"", flags.join(" ")) };
        self.def(1, &format!("<mask id=\"{mid}\" {MASK} {BIG}{data}>"));
        if let Some(fid) = &inv {
            self.def(1, &format!("<g filter=\"url(#{fid})\">"));
        }
        if !m.clip || m.invert {
            let c = if m.clip { "black" } else { "white" };
            self.def(1, &format!("<rect {BIG} fill=\"{c}\"/>"));
        }
        self.defs.push_str(&art);
        if inv.is_some() {
            self.def(1, "</g>");
        }
        self.def(1, "</mask>");
        self.line(&format!("<g mask=\"url(#{mid})\">"));
        self.depth += 1;
        let mut bare = n.clone();
        bare.extras_mut().mask = None;
        self.node_body(&bare);
        self.depth -= 1;
        self.line("</g>");
    }

    /// What `write` writes to the body, taken out of it (written at `depth`, for `<defs>`).
    fn detached(&mut self, depth: usize, write: impl FnOnce(&mut Self)) -> String {
        let (body, saved) = (std::mem::take(&mut self.body), self.depth);
        self.depth = depth;
        write(self);
        self.depth = saved;
        std::mem::replace(&mut self.body, body)
    }

    /// Props of a group (or layer): a knockout group is isolated, a hidden one not displayed.
    fn group_props(&self, n: &Node) -> Props {
        let mut p = css::transparency(n);
        if !n.isolate && n.knocks_out(self.knockout) {
            p.push(("isolation", "isolate".into()));
        }
        p
    }

    /// The children of group `n`, as the elements of a knockout group when it is one.
    fn group_children(&mut self, n: &Node, children: &[std::sync::Arc<Node>]) {
        let knockout = n.knocks_out(self.knockout);
        let enclosing = std::mem::replace(&mut self.knockout, knockout);
        self.children(children);
        self.knockout = enclosing;
    }

    /// Children of the group being written. SVG has no knockout groups: in one, each element is
    /// drawn through a mask of where the elements above it don't paint (the same look for Normal
    /// blending). The masks nest, so element i sits inside the masks of elements i+1…n.
    pub(super) fn children(&mut self, children: &[std::sync::Arc<Node>]) {
        if self.knockout && self.tiny() {
            self.warn(TINY_MASKS);
        }
        if !self.knockout || self.tiny() {
            for c in children {
                self.node(c);
            }
            return;
        }
        let elements = Node::knockout_elements(children);
        // Where every element over another covers it fully, drawing them in order knocks out as
        // much: the masks would only add seams along the shapes' antialiased edges.
        if elements.iter().skip(1).all(|c| paints_opaque(c)) {
            for c in elements {
                self.node(c);
            }
            return;
        }
        let Some((first, rest)) = elements.split_first() else { return };
        for c in rest.iter().rev() {
            let mid = self.knockout_mask(c);
            self.line(&format!("<g mask=\"url(#{mid})\">"));
            self.depth += 1;
        }
        self.node(first);
        for c in rest {
            self.depth -= 1;
            self.line("</g>");
            self.node(c);
        }
    }

    /// A mask that is 1 − the knockout shape of `c`: white, then `c` painted black (at full object
    /// opacity without its own mask, unless those define its shape). Returns the mask id.
    fn knockout_mask(&mut self, c: &Node) -> String {
        let filter = match &self.knockout_filter {
            Some(f) => f.clone(),
            None => {
                let f = self.fresh_id("knockout-shape");
                self.def(1, &format!("<filter id=\"{f}\" filterUnits=\"userSpaceOnUse\" {BIG}>"));
                self.def(2, "<feColorMatrix type=\"matrix\" values=\"0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0\"/>");
                self.def(1, "</filter>");
                self.knockout_filter = Some(f.clone());
                f
            }
        };
        let shape = if c.knockout_shape {
            c.clone()
        } else {
            let mut s = c.clone().with_mask(None);
            s.opacity = 1.0;
            s
        };
        let anonymous = std::mem::replace(&mut self.anonymous, true);
        let art = self.detached(3, |w| w.node(&shape));
        self.anonymous = anonymous;
        let mid = self.fresh_id("knockout");
        self.def(1, &format!("<mask id=\"{mid}\" {MASK} {BIG}>"));
        self.def(2, &format!("<rect {BIG} fill=\"white\"/>"));
        self.def(2, &format!("<g filter=\"url(#{filter})\">"));
        self.defs.push_str(&art);
        self.def(2, "</g>");
        self.def(1, "</mask>");
        mid
    }

    /// An object, inside `<a>` when it links to a URL (Attributes panel).
    pub(super) fn node(&mut self, n: &Node) {
        let layer = usize::from(n.is_layer());
        self.layer_nest += layer;
        match n.url().filter(|_| n.visible && !self.anonymous) {
            Some(url) => {
                self.line(&format!("<a xlink:href=\"{}\">", xml_escape(url)));
                self.depth += 1;
                self.node_body(n);
                self.depth -= 1;
                self.line("</a>");
            }
            None => self.node_body(n),
        }
        self.layer_nest -= layer;
    }

    pub(super) fn node_body(&mut self, n: &Node) {
        // Hidden layers and objects are left out, unless the options keep them (hidden). Template
        // layers never print.
        if !(n.visible || self.opts.hidden_layers) {
            return;
        }
        if has_raster(&n.appearance.effects) {
            return self.filtered(n);
        }
        if let Some(m) = n.extras().mask.as_deref()
            && !m.disabled
        {
            return self.masked(n, m);
        }
        match &n.kind {
            NodeKind::Layer { template: true, .. } => {}
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
                let id = self.id_attr(n);
                let a = self.attrs(&self.group_props(n));
                self.line(&format!("<g{id}{a}>"));
                self.depth += 1;
                self.group_children(n, children);
                self.depth -= 1;
                self.line("</g>");
            }
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                let Some((clip, rest)) = children.split_first() else { return };
                let cid = self.fresh_id("clip-path");
                self.def(1, &format!("<clipPath id=\"{cid}\">"));
                // The region every output clips to; with nothing to clip by, an empty clip path
                // hides the clipped art.
                if let Some((bp, rule)) = vectorcraft_effects::clip_outline(clip) {
                    let d = self.path_d(&PathData::from_bezpath(&bp), self.xf);
                    let r = if rule == FillRule::EvenOdd { " clip-rule=\"evenodd\"" } else { "" };
                    self.def(2, &format!("<path d=\"{d}\"{r}/>"));
                }
                self.def(1, "</clipPath>");
                let id = self.id_attr(n);
                let a = self.attrs(&self.group_props(n));
                // The clipping path's fill paints behind the clipped art and its stroke over it,
                // outside the clip (the group then wraps both).
                let paint = clip.clip_paint();
                if paint.stroke.is_some() {
                    self.line(&format!("<g{id}{a}>"));
                    self.depth += 1;
                    self.line(&format!("<g clip-path=\"url(#{cid})\">"));
                } else {
                    self.line(&format!("<g{id} clip-path=\"url(#{cid})\"{a}>"));
                }
                self.depth += 1;
                if let Some(fill) = &paint.fill {
                    self.node(fill);
                }
                self.group_children(n, rest);
                self.depth -= 1;
                self.line("</g>");
                if let Some(stroke) = &paint.stroke {
                    // One element keeps the clipping path's id.
                    let anonymous = self.anonymous;
                    self.anonymous |= paint.fill.is_some();
                    self.node(stroke);
                    self.anonymous = anonymous;
                    self.depth -= 1;
                    self.line("</g>");
                }
            }
            NodeKind::Path { guide: true, .. } => {}
            NodeKind::Path { path, rule, .. } => {
                if path.is_empty() {
                    return;
                }
                let d = self.path_d(path, self.xf);
                let dashed =
                    n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(s) if s.dash.as_ref().is_some_and(|d| d.is_dashed())));
                let basic = self.basic_shape(path, self.xf, dashed);
                self.shape(n, &d, basic, &[path], *rule, n.geometric_bounds());
            }
            NodeKind::Compound { children, rule } => {
                let paths: Vec<&PathData> = children.iter().filter(|c| c.visible).filter_map(|c| c.path_data()).filter(|p| !p.is_empty()).collect();
                if paths.is_empty() {
                    return;
                }
                let d: Vec<String> = paths.iter().map(|p| self.path_d(p, self.xf)).collect();
                self.shape(n, &d.join(" "), None, &paths, *rule, n.geometric_bounds());
            }
            NodeKind::Text(t) => self.text_node(n, t),
            NodeKind::Image(im) => {
                let id = self.id_attr(n);
                let m = self.matrix(self.xf * im.xf);
                let mut props = css::transparency(n);
                if n.extras().pixelated {
                    props.push(("image-rendering", "pixelated".into()));
                }
                let a = self.attrs(&props);
                if let Some(shared) = self.shared_images.get(&(im.key.clone(), im.width, im.height)) {
                    let shared = shared.clone();
                    self.line(&format!("<use{id} transform=\"{m}\"{a} xlink:href=\"#{shared}\"/>"));
                    return;
                }
                let Some(href) = self.image_href(im) else { return };
                self.line(&format!(
                    "<image{id} width=\"{}\" height=\"{}\" transform=\"{m}\" preserveAspectRatio=\"none\"{a} xlink:href=\"{}\"/>",
                    im.width,
                    im.height,
                    xml_escape(&href)
                ));
            }
            NodeKind::SymbolInstance { symbol, xf } => {
                let doc = self.doc;
                let Some(sym) = doc.symbols.iter().find(|s| s.name == *symbol) else { return };
                if self.symbol_nest > MAX_SYMBOL_NEST {
                    return;
                }
                let id = self.id_attr(n);
                let a = self.attrs(&css::transparency(n));
                if let Some(def) = self.symbol_use(sym, n, *xf) {
                    let m = self.xf * *xf;
                    let tr = if m == Affine::IDENTITY { String::new() } else { format!(" transform=\"{}\"", self.matrix(m)) };
                    self.line(&format!("<use{id} xlink:href=\"#{def}\"{tr}{a}/>"));
                    return;
                }
                // An instance of its own: the (stained) art moved by the instance, its pattern
                // paints and unlinked masks left in place as on the canvas.
                let mut art = vectorcraft_brush::instance_art(&sym.art, n);
                let moved = self.instance_xf * *xf;
                if moved.determinant().abs() > 1e-12 {
                    pin_to_page(&mut art, moved.inverse());
                }
                let saved = (self.xf, self.instance_xf);
                self.xf = saved.0 * *xf;
                self.instance_xf = moved;
                self.line(&format!("<g{id}{a}>"));
                self.depth += 1;
                // The symbol's art is the instance's own picture, outside any knockout around it.
                let knockout = std::mem::take(&mut self.knockout);
                self.symbol_nest += 1;
                self.node(&art);
                self.symbol_nest -= 1;
                self.knockout = knockout;
                self.depth -= 1;
                self.line("</g>");
                (self.xf, self.instance_xf) = saved;
            }
            // Live blends/envelopes/meshes export their evaluated (expanded) form.
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::Intertwine { .. } => {
                let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                self.share_images(&g);
                self.node_body(&g);
            }
        }
    }

    /// The id of the `<symbol>` def instance `inst` (transform `xf`) can `<use>`, written on first
    /// use. `None` when the instance needs its own copy of the art: stained by its fill, or moved
    /// by a transform under which the def would not look as the canvas paints it ([`Reuse`]).
    fn symbol_use(&mut self, sym: &vectorcraft_doc::Symbol, inst: &Node, xf: Affine) -> Option<String> {
        // SVG Tiny has no `<symbol>`.
        if self.tiny() || vectorcraft_brush::stain(inst).is_some() {
            return None;
        }
        if !self.symbols.contains_key(&sym.name) {
            let reuse = Reuse::of(self.doc, &sym.art);
            self.symbols.insert(sym.name.clone(), SymbolDef { id: None, reuse });
        }
        let def = self.symbols.get(&sym.name)?;
        if !def.reuse.allows(xf) {
            return None;
        }
        if let Some(id) = &def.id {
            return Some(id.clone());
        }
        // The art in symbol space, as its own picture: no knockout around it, and no object ids
        // (every instance shows it).
        let id = self.unique_id(&sanitize_id(&sym.name));
        let saved = (self.xf, self.instance_xf, std::mem::take(&mut self.knockout), std::mem::take(&mut self.names), self.anonymous);
        (self.xf, self.instance_xf, self.anonymous) = (Affine::IDENTITY, Affine::IDENTITY, false);
        self.symbol_nest += 1;
        let art = self.detached(3, |w| w.node(&sym.art));
        self.symbol_nest -= 1;
        (self.xf, self.instance_xf, self.knockout, self.names, self.anonymous) = saved;
        // The symbol's name, when the id had to differ from it, comes back on import.
        let name = if id == sym.name {
            String::new()
        } else if valid_xml_value(&sym.name) {
            format!(" data-name=\"{}\"", xml_attr_escape(&sym.name))
        } else {
            self.warn("Symbol names containing invalid XML characters were left out of SVG name attributes");
            String::new()
        };
        self.def(1, &format!("<symbol id=\"{id}\"{name} overflow=\"visible\">"));
        self.defs.push_str(&art);
        self.def(1, "</symbol>");
        if let Some(def) = self.symbols.get_mut(&sym.name) {
            def.id = Some(id.clone());
        }
        Some(id)
    }
}

/// A shape's paths as one path (document space).
pub(super) fn doc_path(paths: &[&PathData]) -> BezPath {
    let mut bp = BezPath::new();
    for p in paths {
        bp.extend(p.to_bezpath());
    }
    bp
}

/// Does `n` paint opaquely wherever it paints (no transparency, blending, masks or effects in
/// it)? Anything this can't tell is taken as not.
fn paints_opaque(n: &Node) -> bool {
    use vectorcraft_doc::{AppearanceItem, NodeKind};
    let paint = |p: &Paint| match p {
        Paint::None | Paint::Solid { .. } => true,
        Paint::Gradient(g) => g.gradient.stops.iter().all(|s| s.opacity >= 1.0),
        _ => false,
    };
    let items = n.appearance.items.iter().all(|i| match i {
        AppearanceItem::Fill(f) => !f.visible || (f.opacity >= 1.0 && f.blend == BlendMode::Normal && f.effects.is_empty() && paint(&f.paint)),
        // Brush art draws as pieces with opacities of their own (bristles vary theirs).
        AppearanceItem::Stroke(s) => {
            !s.visible || (s.opacity >= 1.0 && s.blend == BlendMode::Normal && s.effects.is_empty() && s.brush.is_none() && paint(&s.paint))
        }
    });
    let own = n.opacity >= 1.0 && n.blend == BlendMode::Normal && n.extras().mask.is_none() && n.appearance.effects.is_empty() && items;
    own && match &n.kind {
        NodeKind::Path { .. } => true,
        NodeKind::Group { children, .. } | NodeKind::Compound { children, .. } => children.iter().all(|c| paints_opaque(c)),
        NodeKind::Mesh(m) => m.points.iter().all(|p| p.opacity >= 1.0),
        _ => false,
    }
}
