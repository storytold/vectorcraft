//! The document walk both writers share: each visible object over the region becomes drawing
//! operations ([`Op`]) in paint order, in document space. What a format can't hold is decided
//! here once: with clipping (EMF), gradients become images clipped to what they paint and
//! patterns their tiles clipped to the shape; without (WMF), clipped art is written whole and
//! gradients and patterns as one colour.

use std::collections::HashMap;
use std::sync::Arc;

use vectorcraft_color::freeform::{painted_box, spread_scale};
use vectorcraft_color::{BlendMode, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{AppearanceItem, Dash, Document, ImageObject, LineCap, LineJoin, Node, NodeKind, StrokeLayer};
use vectorcraft_effects::stroke;
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, Shape};

use crate::dib::Rgba;

/// How deep symbols, brush art and pattern tiles may nest in one another (deeper art is left out).
const MAX_NEST: u32 = 8;
/// Most pixels along a side of a gradient's image (its colours are smooth).
const MAX_GRADIENT_PX: f64 = 512.0;
/// Opacity at or above this is written as opaque.
const OPAQUE: f32 = 0.998;

const TRANSPARENCY: &str = "transparency is left out of paths: transparent fills and strokes are written opaque";
const BLENDING: &str = "blending modes are left out";
const MASKS: &str = "opacity masks are left out";
const RASTER_FX: &str = "raster effects (shadows, glows, blurs, feathering) are left out";
const CLIPPING: &str = "clipping masks are not applied: the art they clip is written whole";
const FLAT_GRADIENTS: &str = "gradients are written as one colour, the average of their stops";
const FLAT_PATTERNS: &str = "pattern fills are written as one colour";

/// A stroke as a pen draws it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pen {
    pub rgb: [u8; 3],
    /// Points.
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter: f64,
    /// A dash pattern that draws dashes (points).
    pub dash: Option<Dash>,
}

/// One drawing operation, in document space.
#[derive(Clone, Debug)]
pub(crate) enum Op {
    Fill {
        path: BezPath,
        rule: FillRule,
        rgb: [u8; 3],
    },
    Stroke {
        path: BezPath,
        pen: Pen,
    },
    /// An image: its pixel space (0..w, 0..h) maps into the document by `xf`; its alpha times
    /// `opacity`.
    Image {
        image: Arc<Rgba>,
        xf: Affine,
        opacity: f32,
    },
    /// What follows, up to the matching [`Op::Unclip`], is clipped to `path` (inside the clips
    /// around it). Only formats with clipping get these.
    Clip {
        path: BezPath,
        rule: FillRule,
    },
    Unclip,
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

fn rgb_of(c: &vectorcraft_color::Color) -> [u8; 3] {
    let [r, g, b, _] = c.to_rgba8(1.0);
    [r, g, b]
}

/// The first visible solid fill colour in `n` (a pattern tile's art).
fn first_solid(n: &Node) -> Option<vectorcraft_color::Color> {
    let own = n.appearance.items.iter().find_map(|i| match i {
        AppearanceItem::Fill(f) if f.visible => f.paint.color(),
        _ => None,
    });
    own.or_else(|| n.children().into_iter().flatten().find_map(|c| first_solid(c)))
}

/// `bp` cut into its dashes (as it is when `dash` draws none), for pens that can't draw them.
pub(crate) fn dashed(bp: &BezPath, dash: Option<&Dash>) -> BezPath {
    dash.and_then(|d| stroke::dash(bp, d)).map_or_else(|| bp.clone(), |d| d.path)
}

fn q(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub(crate) struct Scene<'a> {
    doc: &'a Document,
    region: Rect,
    /// The format clips (EMF).
    clips: bool,
    ops: Vec<Op>,
    warnings: Vec<String>,
    /// Decoded images by key (`None`: can't be decoded, left out).
    images: HashMap<String, Option<Arc<Rgba>>>,
    brushes: Option<Vec<vectorcraft_brush::Brush>>,
    /// The opacity of the containers around the current object.
    opacity: f32,
    /// Symbols, brush art and pattern tiles being written inside one another.
    nest: u32,
}

impl<'a> Scene<'a> {
    pub fn new(doc: &'a Document, region: Rect, clips: bool) -> Self {
        Self { doc, region, clips, ops: vec![], warnings: vec![], images: HashMap::new(), brushes: None, opacity: 1.0, nest: 0 }
    }

    /// The operations (clipped to the region when the format clips) and the warnings.
    pub fn run(mut self) -> (Vec<Op>, Vec<String>) {
        let doc = self.doc;
        if self.clips {
            self.ops.push(Op::Clip { path: self.region.to_path(0.1), rule: FillRule::NonZero });
        }
        for l in &doc.layers {
            self.node(l, false);
        }
        if self.clips {
            self.ops.push(Op::Unclip);
        }
        (self.ops, self.warnings)
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn node(&mut self, n: &Node, force: bool) {
        if let NodeKind::Layer { template: true, .. } = n.kind {
            return;
        }
        if !force && !n.visible {
            return;
        }
        if !n.is_layer() {
            match n.visual_bounds() {
                Some(b) if !overlaps(b, self.region) => return,
                None if !n.is_container() => return,
                _ => {}
            }
        }
        self.note_losses(n);
        let opacity = self.opacity;
        self.opacity *= n.opacity.clamp(0.0, 1.0);
        match &n.kind {
            NodeKind::Layer { children, clip, .. } | NodeKind::Group { children, clip } => self.children(children, *clip),
            NodeKind::Path { path, rule, guide, .. } => {
                if !*guide {
                    self.shape(n, &path.to_bezpath(), *rule);
                }
            }
            NodeKind::Compound { children, rule } => {
                let mut bp = BezPath::new();
                for p in children.iter().filter_map(|c| c.path_data()) {
                    bp.extend(p.to_bezpath());
                }
                self.shape(n, &bp, *rule);
            }
            NodeKind::Text(_) => {
                if let Some(o) = vectorcraft_effects::outline_text(n) {
                    self.node(&o, true);
                }
            }
            NodeKind::Image(im) => self.image(im),
            NodeKind::SymbolInstance { symbol, xf } => {
                if let Some(sym) = self.doc.symbols.iter().find(|s| &s.name == symbol)
                    && self.nest < MAX_NEST
                {
                    let mut art = (*sym.art).clone();
                    art.transform(*xf, false);
                    self.nest += 1;
                    self.node(&art, true);
                    self.nest -= 1;
                }
            }
            // Live blends, envelopes, meshes and repeats are written as their evaluated art.
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::CompoundShape { .. } => {
                let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                for c in g.children().into_iter().flatten() {
                    self.node(c, false);
                }
            }
        }
        self.opacity = opacity;
    }

    /// A container's children. A clipping container's first child clips the rest (its fill paints
    /// below them, its stroke over them); without clipping they are written whole.
    fn children(&mut self, children: &[Arc<Node>], clip: bool) {
        let Some((first, rest)) = children.split_first().filter(|_| clip) else {
            children.iter().for_each(|c| self.node(c, false));
            return;
        };
        let paint = first.clip_paint();
        let outline = if self.clips {
            // Nothing to clip by hides the clipped art (as on the canvas).
            let Some((path, rule)) = vectorcraft_effects::clip_outline(first) else { return };
            Some((path, rule))
        } else {
            self.warn(CLIPPING);
            None
        };
        let clipped = outline.is_some();
        if let Some((path, rule)) = outline {
            self.ops.push(Op::Clip { path, rule });
        }
        if let Some(f) = &paint.fill {
            self.node(f, false);
        }
        rest.iter().for_each(|c| self.node(c, false));
        if clipped {
            self.ops.push(Op::Unclip);
        }
        if let Some(s) = &paint.stroke {
            self.node(s, false);
        }
    }

    /// Warn about what a metafile can't hold in `n`'s look.
    fn note_losses(&mut self, n: &Node) {
        if n.blend != BlendMode::Normal {
            self.warn(BLENDING);
        }
        if n.mask.as_ref().is_some_and(|m| !m.disabled) {
            self.warn(MASKS);
        }
        let raster = |fx: &[vectorcraft_doc::Effect]| fx.iter().any(|e| e.visible && vectorcraft_effects::is_raster(&e.id));
        if raster(&n.appearance.effects) || n.appearance.items.iter().any(|i| raster(i.effects())) {
            self.warn(RASTER_FX);
        }
    }

    /// Warn when a path paints at `opacity` (times the containers').
    fn check_opacity(&mut self, opacity: f32) {
        if self.opacity * opacity < OPAQUE {
            self.warn(TRANSPARENCY);
        }
    }

    /// A path's fills and strokes, in paint order.
    fn shape(&mut self, n: &Node, bp: &BezPath, rule: FillRule) {
        if bp.elements().is_empty() {
            return;
        }
        let bounds = bp.bounding_box();
        for item in &n.appearance.items {
            match item {
                AppearanceItem::Fill(f) if f.visible => {
                    if f.blend != BlendMode::Normal {
                        self.warn(BLENDING);
                    }
                    self.fill(bp, rule, &f.paint, f.opacity, bounds);
                }
                AppearanceItem::Stroke(st) if st.visible && st.width > 0.0 && st.width.is_finite() => self.stroke(bp, rule, st, bounds),
                _ => {}
            }
        }
    }

    /// Fill `bp` with `paint` at `opacity`; `bounds` places a gradient that isn't placed.
    fn fill(&mut self, bp: &BezPath, rule: FillRule, paint: &Paint, opacity: f32, bounds: Rect) {
        match paint {
            Paint::None => {}
            Paint::Gradient(g) if self.clips => self.gradient(bp, rule, g, opacity, bounds),
            Paint::Pattern { pattern, xf } if self.clips && self.nest < MAX_NEST && self.doc.pattern(pattern).is_some() => {
                let area = bp.bounding_box().intersect(self.region);
                let tiles = self.doc.pattern(pattern).map(|def| def.instances_in(*xf, area)).unwrap_or_default();
                let outer = self.opacity;
                self.opacity *= opacity.clamp(0.0, 1.0);
                self.ops.push(Op::Clip { path: bp.clone(), rule });
                self.nest += 1;
                tiles.iter().for_each(|t| self.node(t, true));
                self.nest -= 1;
                self.ops.push(Op::Unclip);
                self.opacity = outer;
            }
            p => {
                let Some(rgb) = self.flat(p) else { return };
                self.check_opacity(opacity);
                self.ops.push(Op::Fill { path: bp.clone(), rule, rgb });
            }
        }
    }

    /// One colour for a paint: a solid colour, the average of a gradient's stops, or a pattern's
    /// first solid fill.
    fn flat(&mut self, p: &Paint) -> Option<[u8; 3]> {
        match p {
            Paint::None => None,
            Paint::Solid { color, .. } => Some(rgb_of(color)),
            Paint::Gradient(g) => {
                self.warn(FLAT_GRADIENTS);
                let stops = &g.gradient.stops;
                let n = stops.len().max(1) as f64;
                let sum = stops.iter().map(|s| rgb_of(&s.color)).fold([0.0; 3], |a, c| [0, 1, 2].map(|i| a[i] + f64::from(c[i])));
                Some(sum.map(|v| (v / n).round().clamp(0.0, 255.0) as u8))
            }
            Paint::Pattern { pattern, .. } => {
                self.warn(FLAT_PATTERNS);
                let first = self.doc.pattern(pattern).and_then(|def| def.art.iter().find_map(|a| first_solid(a)));
                Some(first.map_or([128; 3], |c| rgb_of(&c)))
            }
        }
    }

    /// A gradient fill: an image of its colours over the shape's box, clipped to the shape,
    /// sampled at the document's raster effects resolution.
    fn gradient(&mut self, bp: &BezPath, rule: FillRule, g: &GradientPaint, opacity: f32, bounds: Rect) {
        let area = bp.bounding_box().intersect(self.region);
        let long = area.width().max(area.height());
        if !(long > 1e-9 && long.is_finite()) {
            return;
        }
        let k = (self.doc.raster_effects_ppi / 72.0).min(MAX_GRADIENT_PX / long);
        let along = |len: f64| (len * k).ceil().clamp(1.0, MAX_GRADIENT_PX) as u16;
        let (cols, rows) = (along(area.width()), along(area.height()));
        let pixels: Vec<u8> = if g.gradient.kind == GradientKind::Freeform {
            let Some(b) = painted_box(bounds) else { return };
            let field = g.freeform_on(b).field_with(spread_scale(b), &|c| c.to_rgb());
            field.grid(area, cols, rows).flat_map(|(c, a)| [q(c[0]), q(c[1]), q(c[2]), q(a)]).collect()
        } else {
            let geom = g.resolve(bounds);
            let (cw, ch) = (area.width() / f64::from(cols), area.height() / f64::from(rows));
            (0..usize::from(rows) * usize::from(cols))
                .flat_map(|i| {
                    let (x, y) = (i % usize::from(cols), i / usize::from(cols));
                    let p = Point::new(area.x0 + (x as f64 + 0.5) * cw, area.y0 + (y as f64 + 0.5) * ch);
                    let t = geom.param_at(g.gradient.kind, p);
                    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
                    let (c, a) = g.gradient.sample(t as f32);
                    c.to_rgba8(a)
                })
                .collect()
        };
        let image = Rgba { width: u32::from(cols), height: u32::from(rows), pixels };
        let xf =
            Affine::translate(area.origin().to_vec2()) * Affine::scale_non_uniform(area.width() / f64::from(cols), area.height() / f64::from(rows));
        self.ops.push(Op::Clip { path: bp.clone(), rule });
        self.ops.push(Op::Image { image: Arc::new(image), xf, opacity: (self.opacity * opacity).clamp(0.0, 1.0) });
        self.ops.push(Op::Unclip);
    }

    fn stroke(&mut self, bp: &BezPath, rule: FillRule, st: &StrokeLayer, bounds: Rect) {
        if st.brush.is_some()
            && self.nest < MAX_NEST
            && let Some(art) = self.brush_art(bp, st)
        {
            let opacity = self.opacity;
            self.opacity *= st.opacity.clamp(0.0, 1.0);
            self.nest += 1;
            art.iter().for_each(|piece| self.node(piece, true));
            self.nest -= 1;
            self.opacity = opacity;
            return;
        }
        if st.blend != BlendMode::Normal {
            self.warn(BLENDING);
        }
        // A pen paints one colour along the path itself: anything else is the stroke's outline,
        // filled.
        let flat_paint = matches!(st.paint, Paint::Solid { .. }) || !self.clips;
        if !stroke::is_plain(st) || !flat_paint {
            let region = stroke::outline_region(&PathData::from_bezpath(bp), rule, st).to_bezpath();
            self.fill(&region, FillRule::NonZero, &st.paint, st.opacity, st.paint_bounds(bounds));
            return;
        }
        let Some(rgb) = self.flat(&st.paint) else { return };
        self.check_opacity(st.opacity);
        let pen = Pen { rgb, width: st.width, cap: st.cap, join: st.join, miter: st.miter_limit, dash: st.dash.clone().filter(|d| d.is_dashed()) };
        self.ops.push(Op::Stroke { path: bp.clone(), pen });
    }

    fn brush_art(&mut self, bp: &BezPath, st: &StrokeLayer) -> Option<Vec<Node>> {
        let doc = self.doc;
        let brushes = self.brushes.get_or_insert_with(|| vectorcraft_brush::library(doc));
        let b = st.brush.as_deref().and_then(|name| brushes.iter().find(|b| b.name == name))?;
        Some(vectorcraft_brush::stroke_pieces(b, bp, st))
    }

    /// A placed image, decoded once per image.
    fn image(&mut self, im: &ImageObject) {
        let Some(img) = self.decoded(&im.key) else { return };
        let (dw, dh) = (f64::from(img.width), f64::from(img.height));
        let xf = im.xf * Affine::scale_non_uniform(f64::from(im.width.max(1)) / dw, f64::from(im.height.max(1)) / dh);
        self.ops.push(Op::Image { image: img, xf, opacity: self.opacity.clamp(0.0, 1.0) });
    }

    /// Image `key`'s pixels (scaled down to what a metafile stores), `None` with a warning when it
    /// has none or can't be decoded.
    fn decoded(&mut self, key: &str) -> Option<Arc<Rgba>> {
        if let Some(i) = self.images.get(key) {
            return i.clone();
        }
        let blob = self.doc.images.get(key).filter(|b| !b.bytes.is_empty());
        let img = blob
            .and_then(|b| image::load_from_memory(&b.bytes).ok())
            .map(|i| Rgba::fitted(i.to_rgba8()))
            .filter(|i| i.width > 0 && i.height > 0)
            .map(Arc::new);
        if img.is_none() {
            self.warn(&format!("image '{key}' could not be decoded and was left out"));
        }
        self.images.insert(key.to_string(), img.clone());
        img
    }
}
