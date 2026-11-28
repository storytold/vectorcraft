//! The document walk both writers share: each visible object over the region becomes drawing
//! operations ([`Op`]) in paint order, in document space. What a format can't hold is decided
//! here once: with clipping (EMF), gradients become images clipped to what they paint and
//! patterns their tiles clipped to the shape; without (WMF), clipped art is written whole and
//! gradients and patterns as one colour.

use std::collections::HashMap;
use std::sync::Arc;

use kurbo::PathEl;
use vectorcraft_color::freeform::{painted_box, spread_scale};
use vectorcraft_color::{BlendMode, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{AppearanceItem, Dash, Document, ImageObject, LineCap, LineJoin, Node, NodeKind, StrokeLayer, SymbolBoxes};
use vectorcraft_effects::stroke;
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, Shape};

use craft_metafile::write::Image;

/// How deep symbols, brush art and pattern tiles may nest in one another (deeper art is left out).
const MAX_NEST: u32 = 8;
/// Most pattern tiles written in all. One fill tiles at most 250,000 (`PatternDef::offsets_covering`),
/// but a tile whose art is filled with a pattern again multiplies that at every level of nesting.
const MAX_TILES: usize = 250_000;
/// Most pixels along a side of a gradient's image (its colours are smooth).
const MAX_GRADIENT_PX: f64 = 512.0;
/// Opacity at or above this is written as opaque.
const OPAQUE: f32 = 0.998;

const TRANSPARENCY: &str = "transparency is left out of paths: transparent fills and strokes are written opaque";
const BLENDING: &str = "blending modes are left out";
const MASKS: &str = "opacity masks are left out";
const RASTER_FX: &str = "raster effects (shadows, glows, blurs, feathering) are left out";
const CLIPPING: &str = "clipping masks are not applied: the art they clip is written whole";
const DISTORTED_IMAGES: &str = "images changed by Perspective Distort or Free Distort are written as the nearest parallelogram";
const FLAT_GRADIENTS: &str = "gradients are written as one colour, the average of their stops";
const FLAT_PATTERNS: &str = "pattern fills are written as one colour";
const TOO_MANY_TILES: &str = "pattern fills past 250,000 tiles in all are written as one colour";
const TEXT_OUTLINES: &str = "type is written as outlines: it can't be searched, selected or read aloud as text";
const PATTERN_TILES: &str = "pattern fills are written as their tiles, one shape each";
const PATTERN_SHAPES: &str = "pattern fills are written as their tiles, one shape for each part of the tile";
/// How far (points) art may lie past a convex clip, or a pattern's art past its tile, and still
/// count as inside (what the device units round away).
const INSIDE_TOL: f64 = 1e-6;
/// The warning for an image left out because it can't be decoded: the note raster exports give
/// (`vectorcraft-engine`'s `rasterfx::undecoded_warning`).
const UNDECODED: &str = "an image couldn't be decoded and is missing from the picture";
/// The warning about how distorted images are written.
pub(crate) const DISTORTED_IMAGES_NOTE: &str = DISTORTED_IMAGES;

/// A stroke as a pen draws it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pen {
    pub rgb: [u8; 3],
    /// The solid paint `rgb` comes from (`None`: a gradient's or pattern's one colour).
    pub paint: Option<Paint>,
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
        /// The solid paint `rgb` comes from (`None`: a gradient's or pattern's one colour).
        paint: Option<Paint>,
    },
    Stroke {
        path: BezPath,
        pen: Pen,
    },
    /// An image: its pixel space (0..w, 0..h) maps into the document by `xf`; its alpha times
    /// `opacity`.
    Image {
        image: Arc<Image>,
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

/// `p` when it is a solid paint.
fn solid(p: &Paint) -> Option<Paint> {
    matches!(p, Paint::Solid { .. }).then(|| p.clone())
}

fn q(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub(crate) struct Scene<'a> {
    doc: &'a Document,
    /// The art boxes of `doc`'s symbols, which the walk measures symbol instances by when it
    /// leaves out objects outside the region ([`SymbolBoxes::quick`]).
    symbols: SymbolBoxes<'a>,
    region: Rect,
    /// The format clips (EMF).
    clips: bool,
    ops: Vec<Op>,
    warnings: Vec<String>,
    /// Decoded images by key (`None`: can't be decoded, left out).
    images: HashMap<String, Option<Arc<Image>>>,
    brushes: Option<Vec<vectorcraft_brush::Brush>>,
    /// The opacity of the containers around the current object.
    opacity: f32,
    /// Symbols, brush art and pattern tiles being written inside one another.
    nest: u32,
    /// Pattern tiles left to write ([`MAX_TILES`]).
    tiles_left: usize,
    /// Pattern tiles being written that may be merged: their parts aren't left out off the page,
    /// so each tile draws the same parts (the fill's clip still trims them).
    whole_tiles: bool,
    /// While the pieces of a distorted image are written: the box of the clips around the current
    /// object (`Some(None)` before the first), to which each piece's image is cropped.
    crop: Option<Option<Rect>>,
}

impl<'a> Scene<'a> {
    pub fn new(doc: &'a Document, region: Rect, clips: bool) -> Self {
        Self {
            doc,
            symbols: doc.symbol_boxes().quick(),
            region,
            clips,
            ops: vec![],
            warnings: vec![],
            images: HashMap::new(),
            brushes: None,
            opacity: 1.0,
            nest: 0,
            tiles_left: MAX_TILES,
            whole_tiles: false,
            crop: None,
        }
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

    /// The operations of each top-level layer of the document, in its paint order, and the
    /// warnings. Unlike [`Scene::run`], nothing is clipped to the region.
    pub fn run_by_layer(mut self) -> (Vec<Vec<Op>>, Vec<String>) {
        let doc = self.doc;
        let mut layers = Vec::with_capacity(doc.layers.len());
        for l in &doc.layers {
            self.node(l, false);
            layers.push(std::mem::take(&mut self.ops));
        }
        (layers, self.warnings)
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn node(&mut self, n: &Node, force: bool) {
        // Ruler guides draw on screen only: no record and no loss warning.
        if n.is_ruler_guide() || n.is_template() {
            return;
        }
        if !force && !n.visible {
            return;
        }
        if !n.is_layer() && !self.whole_tiles {
            match n.visual_bounds_with(&self.symbols) {
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
                    self.warn(TEXT_OUTLINES);
                    self.node(&o, true);
                }
            }
            NodeKind::Image(im) => match vectorcraft_doc::image_quad::nearest_affine(n) {
                // A format that clips writes a distorted image as the pieces of its mesh; one that
                // doesn't, at the affine placement nearest its corners.
                // Each piece stores only the pixels inside its clips.
                Some(_) if self.clips => {
                    let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                    let outer = self.crop.replace(None);
                    for c in g.children().into_iter().flatten() {
                        self.node(c, false);
                    }
                    self.crop = outer;
                }
                Some(xf) => {
                    self.warn(DISTORTED_IMAGES);
                    self.image(&ImageObject { xf, ..self.doc.shown_image(n, im).into_owned() });
                }
                None => self.image(&self.doc.shown_image(n, im)),
            },
            NodeKind::SymbolInstance { symbol, xf } => {
                if let Some(sym) = self.doc.symbols.iter().find(|s| &s.name == symbol)
                    && self.nest < MAX_NEST
                    && let art = vectorcraft_doc::symbol_overrides::instance_art(&sym.art, n)
                    && let Some(art) =
                        if self.clips { self.doc.place_symbol_art(symbol, &art, *xf) } else { self.doc.place_symbol_art_unclipped(symbol, &art, *xf) }
                {
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
            | NodeKind::Intertwine { .. } => {
                let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                for c in g.children().into_iter().flatten() {
                    self.node(c, false);
                }
            }
            // Ruler guides draw on screen only.
            NodeKind::RulerGuide { .. } => {}
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
        let outer_crop = self.crop;
        if let Some((path, rule)) = outline {
            if let Some(c) = self.crop {
                let b = path.bounding_box();
                self.crop = Some(Some(c.map_or(b, |c| c.intersect(b))));
            }
            self.ops.push(Op::Clip { path, rule });
        }
        if let Some(f) = &paint.fill {
            self.node(f, false);
        }
        rest.iter().for_each(|c| self.node(c, false));
        self.crop = outer_crop;
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
        if n.extras().mask.as_ref().is_some_and(|m| !m.disabled) {
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
            Paint::Pattern { pattern, xf } if self.clips && self.nest < MAX_NEST && self.tiles_left > 0 && self.doc.pattern(pattern).is_some() => {
                let area = bp.bounding_box().intersect(self.region);
                let tiles = self.doc.pattern(pattern).map(|def| def.instances_in(*xf, area)).unwrap_or_default();
                if tiles.len() > self.tiles_left {
                    self.tiles_left = 0;
                    self.warn(TOO_MANY_TILES);
                    self.flat_fill(bp, rule, paint, opacity);
                    return;
                }
                self.tiles_left -= tiles.len();
                // Tiles that can't overlap one another (grid and brick tilings whose art stays in
                // its tile) may be merged: see `merge_tiles`.
                let per_tile = self.doc.pattern(pattern).filter(|def| tiles_apart(def)).map(|def| def.art.len());
                let outer = self.opacity;
                self.opacity *= opacity.clamp(0.0, 1.0);
                self.ops.push(Op::Clip { path: bp.clone(), rule });
                self.nest += 1;
                let mut chunks = Vec::new();
                let (whole, left) = (self.whole_tiles, self.tiles_left);
                self.whole_tiles = per_tile.is_some();
                for (i, t) in tiles.iter().enumerate() {
                    if per_tile.is_some_and(|k| k > 0 && i % k == 0) {
                        chunks.push(self.ops.len());
                    }
                    self.node(t, true);
                }
                self.whole_tiles = whole;
                let merged = per_tile.is_some() && merge_tiles(&mut self.ops, &chunks);
                // Tiles that couldn't merge are written as before, their parts off the page left out.
                if per_tile.is_some() && !merged {
                    if let Some(&first) = chunks.first() {
                        self.ops.truncate(first);
                    }
                    // Patterns inside the tiles take from the budget once.
                    self.tiles_left = left;
                    tiles.iter().for_each(|t| self.node(t, true));
                }
                self.nest -= 1;
                if !tiles.is_empty() {
                    self.warn(if merged { PATTERN_SHAPES } else { PATTERN_TILES });
                }
                self.ops.push(Op::Unclip);
                self.opacity = outer;
            }
            p => self.flat_fill(bp, rule, p, opacity),
        }
    }

    /// Fill `bp` with one colour for `paint` at `opacity`.
    fn flat_fill(&mut self, bp: &BezPath, rule: FillRule, paint: &Paint, opacity: f32) {
        let Some(rgb) = self.flat(paint) else { return };
        self.check_opacity(opacity);
        self.ops.push(Op::Fill { path: bp.clone(), rule, rgb, paint: solid(paint) });
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
            let field = g.freeform_on(b).model_field(spread_scale(b));
            field.display_grid(area, cols, rows).flat_map(|(c, a)| [q(c[0]), q(c[1]), q(c[2]), q(a)]).collect()
        } else {
            let geom = g.resolve(bounds);
            let (sample, spread) = (g.gradient.sampler(), g.gradient.spreader());
            let (cw, ch) = (area.width() / f64::from(cols), area.height() / f64::from(rows));
            (0..usize::from(rows) * usize::from(cols))
                .flat_map(|i| {
                    let (x, y) = (i % usize::from(cols), i / usize::from(cols));
                    let p = Point::new(area.x0 + (x as f64 + 0.5) * cw, area.y0 + (y as f64 + 0.5) * ch);
                    let t = spread(geom.param_at(g.gradient.kind, p));
                    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
                    let (c, a) = sample(t as f32);
                    c.to_rgba8(a)
                })
                .collect()
        };
        let image = Image { width: u32::from(cols), height: u32::from(rows), rgba: pixels };
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
        let pen = Pen {
            rgb,
            paint: solid(&st.paint),
            width: st.width,
            cap: st.cap,
            join: st.join,
            miter: st.miter_limit,
            dash: st.dash.clone().filter(|d| d.is_dashed()),
        };
        self.ops.push(Op::Stroke { path: bp.clone(), pen });
    }

    fn brush_art(&mut self, bp: &BezPath, st: &StrokeLayer) -> Option<Vec<Node>> {
        let doc = self.doc;
        let brushes = self.brushes.get_or_insert_with(|| vectorcraft_brush::library(doc));
        let b = st.brush.as_deref().and_then(|name| brushes.iter().find(|b| b.name == name))?;
        Some(vectorcraft_brush::stroke_pieces(doc, b, bp, st))
    }

    /// A placed image, decoded once per image.
    fn image(&mut self, im: &ImageObject) {
        let Some(img) = self.decoded(im) else { return };
        let (dw, dh) = (f64::from(img.width), f64::from(img.height));
        let xf = im.xf * Affine::scale_non_uniform(f64::from(im.width.max(1)) / dw, f64::from(im.height.max(1)) / dh);
        let (img, xf) = match self.crop.flatten() {
            Some(c) => match cropped(&img, xf, c) {
                Some((part, x0, y0)) => (Arc::new(part), xf * Affine::translate((f64::from(x0), f64::from(y0)))),
                None => return,
            },
            None => (img, xf),
        };
        self.ops.push(Op::Image { image: img, xf, opacity: self.opacity.clamp(0.0, 1.0) });
    }

    /// Image `im`'s pixels (scaled down to what a metafile stores), `None` with a warning when it
    /// has none or can't be decoded.
    fn decoded(&mut self, im: &ImageObject) -> Option<Arc<Image>> {
        let key = im.key.as_str();
        if let Some(i) = self.images.get(key) {
            return i.clone();
        }
        let blob = self.doc.image_blob(im).filter(|b| !b.bytes.is_empty());
        let img = blob
            .and_then(|b| craft_image_codecs::decode(&b.bytes).ok())
            .and_then(|i| image::RgbaImage::from_raw(i.width(), i.height(), i.to_rgba8()))
            .map(crate::dib::fitted)
            .filter(|i| i.width > 0 && i.height > 0)
            .map(Arc::new);
        if img.is_none() {
            self.warn(UNDECODED);
        }
        self.images.insert(key.to_string(), img.clone());
        img
    }
}

/// The pixels of `img`, drawn by `xf`, that `clip` (a box in the document) shows, with one pixel
/// more on each side: the part and its top left pixel. The whole image when `xf` cannot be
/// inverted; `None` when `clip` shows none of it.
fn cropped(img: &Image, xf: Affine, clip: Rect) -> Option<(Image, u32, u32)> {
    if xf.determinant().abs() < 1e-12 || !xf.determinant().is_finite() {
        return Some((img.clone(), 0, 0));
    }
    let inv = xf.inverse();
    let corners =
        [Point::new(clip.x0, clip.y0), Point::new(clip.x1, clip.y0), Point::new(clip.x1, clip.y1), Point::new(clip.x0, clip.y1)].map(|p| inv * p);
    let (w, h) = (f64::from(img.width), f64::from(img.height));
    let lo = |f: fn(&Point) -> f64| corners.iter().map(f).fold(f64::INFINITY, f64::min);
    let hi = |f: fn(&Point) -> f64| corners.iter().map(f).fold(f64::NEG_INFINITY, f64::max);
    let (x0, y0) = ((lo(|p| p.x).floor() - 1.0).max(0.0), (lo(|p| p.y).floor() - 1.0).max(0.0));
    let (x1, y1) = ((hi(|p| p.x).ceil() + 1.0).min(w), (hi(|p| p.y).ceil() + 1.0).min(h));
    if !(x0 < x1 && y0 < y1) {
        return None;
    }
    let (x0, y0, x1, y1) = (x0 as u32, y0 as u32, x1 as u32, y1 as u32);
    if (x0, y0, x1, y1) == (0, 0, img.width, img.height) {
        return Some((img.clone(), 0, 0));
    }
    let row = img.width as usize * 4;
    let mut pixels = Vec::with_capacity((x1 - x0) as usize * (y1 - y0) as usize * 4);
    for y in y0..y1 {
        let start = y as usize * row + x0 as usize * 4;
        pixels.extend_from_slice(img.rgba.get(start..start + (x1 - x0) as usize * 4)?);
    }
    Some((Image { width: x1 - x0, height: y1 - y0, rgba: pixels }, x0, y0))
}

/// Do the instances of `def` stay apart? A grid or brick tiling places its tiles side by side, so
/// art that stays in its tile never overlaps another instance's; hex tilings overlap their tiles.
fn tiles_apart(def: &vectorcraft_doc::PatternDef) -> bool {
    use vectorcraft_doc::TileType;
    let tiling = matches!(def.tile_type, TileType::Grid | TileType::BrickByRow { .. } | TileType::BrickByColumn { .. });
    let inside = def.art.iter().all(|a| painted_bounds(a).is_some_and(|b| def.tile.inflate(INSIDE_TOL, INSIDE_TOL).contains_rect(b)));
    tiling && !def.art.is_empty() && inside
}

/// The extent `n` paints: a clipping group's clip path, else its visual bounds.
fn painted_bounds(n: &Node) -> Option<Rect> {
    match &n.kind {
        NodeKind::Group { children, clip: true } => children.first().and_then(|c| c.path_data()).map(|p| p.to_bezpath().bounding_box()),
        _ => n.visual_bounds(),
    }
}

/// Write a pattern fill's tiles (`ops` from each start in `starts` to the next, the last to the
/// end) with fewer shapes, when every tile drew the same plain fills: each tile's clip is left out
/// where what it clips lies inside it already, then the n-th fill of every tile becomes one path.
/// The tiles don't overlap ([`tiles_apart`]), so the picture is the same. → whether they merged.
fn merge_tiles(ops: &mut Vec<Op>, starts: &[usize]) -> bool {
    let Some(&first) = starts.first() else { return false };
    let ends = starts.iter().skip(1).copied().chain([ops.len()]);
    let mut tiles: Vec<Vec<(BezPath, FillRule, [u8; 3])>> = Vec::with_capacity(starts.len());
    for (start, end) in starts.iter().copied().zip(ends) {
        let Some(fills) = ops.get(start..end).and_then(plain_fills) else { return false };
        // A tile off the page draws nothing.
        if !fills.is_empty() {
            tiles.push(fills);
        }
    }
    let Some((shape, rest)) = tiles.split_first() else { return false };
    let same = |t: &Vec<(BezPath, FillRule, [u8; 3])>| t.len() == shape.len() && t.iter().zip(shape).all(|(a, b)| (a.1, a.2) == (b.1, b.2));
    if shape.is_empty() || !rest.iter().all(same) {
        return false;
    }
    let mut merged: Vec<Op> = shape.iter().map(|(_, rule, rgb)| Op::Fill { path: BezPath::new(), rule: *rule, rgb: *rgb, paint: None }).collect();
    for tile in &tiles {
        for (op, (path, _, _)) in merged.iter_mut().zip(tile) {
            if let Op::Fill { path: all, .. } = op {
                all.extend(path.elements().iter().copied());
            }
        }
    }
    ops.truncate(first);
    ops.extend(merged);
    true
}

/// A tile's ops as plain fills: fills, or fills inside one clip that lies around them all
/// already (a tile clipped to its cell). `None` for anything else (strokes, images, clips that cut).
fn plain_fills(ops: &[Op]) -> Option<Vec<(BezPath, FillRule, [u8; 3])>> {
    let inner = match ops {
        [Op::Clip { path, .. }, inner @ .., Op::Unclip] => {
            let hull = convex_polygon(path)?;
            inner
                .iter()
                .all(|op| matches!(op, Op::Fill { path, .. } if path.elements().iter().all(|el| el_points(el).all(|p| inside(&hull, p)))))
                .then_some(inner)?
        }
        all => all,
    };
    inner
        .iter()
        .map(|op| match op {
            Op::Fill { path, rule, rgb, .. } => Some((path.clone(), *rule, *rgb)),
            _ => None,
        })
        .collect()
}

/// The points of a path element (a curve lies inside its control points' hull).
fn el_points(el: &PathEl) -> impl Iterator<Item = Point> {
    let pts: Vec<Point> = match *el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => vec![p],
        PathEl::QuadTo(a, b) => vec![a, b],
        PathEl::CurveTo(a, b, c) => vec![a, b, c],
        PathEl::ClosePath => vec![],
    };
    pts.into_iter()
}

/// `bp` as the corners of a convex polygon, counter-clockwise: one closed figure of straight
/// segments that turns one way. `None` for any other path.
fn convex_polygon(bp: &BezPath) -> Option<Vec<Point>> {
    let mut pts = Vec::new();
    for (i, el) in bp.elements().iter().enumerate() {
        match (i, el) {
            (0, PathEl::MoveTo(p)) | (1.., PathEl::LineTo(p)) => pts.push(*p),
            (1.., PathEl::ClosePath) if i + 1 == bp.elements().len() => {}
            _ => return None,
        }
    }
    if pts.len() > 1 && pts.first().zip(pts.last()).is_some_and(|(a, b)| (*a - *b).hypot() <= INSIDE_TOL) {
        pts.pop();
    }
    if pts.len() < 3 {
        return None;
    }
    let n = pts.len();
    let turn = |i: usize| {
        let (a, b, c) = (pts[i], pts[(i + 1) % n], pts[(i + 2) % n]);
        (b - a).cross(c - b)
    };
    let turns: Vec<f64> = (0..n).map(turn).collect();
    if turns.iter().any(|t| !t.is_finite()) {
        return None;
    }
    if turns.iter().all(|t| *t >= -INSIDE_TOL) && turns.iter().any(|t| *t > INSIDE_TOL) {
        Some(pts)
    } else if turns.iter().all(|t| *t <= INSIDE_TOL) && turns.iter().any(|t| *t < -INSIDE_TOL) {
        pts.reverse();
        Some(pts)
    } else {
        None
    }
}

/// Is `p` inside (or on) the counter-clockwise convex polygon `hull`?
fn inside(hull: &[Point], p: Point) -> bool {
    let n = hull.len();
    (0..n).all(|i| {
        let (a, b) = (hull[i], hull[(i + 1) % n]);
        let edge = b - a;
        let len = edge.hypot();
        len > 0.0 && edge.cross(p - a) / len >= -INSIDE_TOL
    })
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    fn poly(pts: &[(f64, f64)]) -> BezPath {
        let mut bp = BezPath::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 { bp.move_to((*x, *y)) } else { bp.line_to((*x, *y)) }
        }
        bp.close_path();
        bp
    }

    #[test]
    fn convex_polygons_in_either_direction_hold_what_is_inside_them() {
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let reversed: Vec<(f64, f64)> = square.iter().rev().copied().collect();
        for pts in [&square[..], &reversed[..]] {
            let hull = convex_polygon(&poly(pts)).unwrap();
            assert!(inside(&hull, Point::new(5.0, 5.0)));
            assert!(inside(&hull, Point::new(10.0, 3.0)), "on an edge");
            assert!(!inside(&hull, Point::new(10.5, 3.0)));
        }
        // A rotated square too.
        let diamond = convex_polygon(&poly(&[(5.0, 0.0), (10.0, 5.0), (5.0, 10.0), (0.0, 5.0)])).unwrap();
        assert!(inside(&diamond, Point::new(5.0, 5.0)) && !inside(&diamond, Point::new(1.0, 1.0)));
    }

    #[test]
    fn concave_curved_and_open_clips_are_not_convex_polygons() {
        assert!(convex_polygon(&poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (5.0, 5.0), (0.0, 10.0)])).is_none(), "concave");
        let mut curved = BezPath::new();
        curved.move_to((0.0, 0.0));
        curved.curve_to((5.0, -5.0), (10.0, -5.0), (10.0, 0.0));
        curved.line_to((5.0, 10.0));
        curved.close_path();
        assert!(convex_polygon(&curved).is_none(), "curved");
        let mut two = poly(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)]);
        two.extend(poly(&[(5.0, 5.0), (6.0, 5.0), (6.0, 6.0)]));
        assert!(convex_polygon(&two).is_none(), "two figures");
        assert!(convex_polygon(&poly(&[(0.0, 0.0), (1.0, 1.0), (2.0, 2.0)])).is_none(), "a line");
    }

    /// A tile whose fill reaches past its clip keeps the clip, so the tiles aren't merged.
    #[test]
    fn a_clip_that_cuts_its_fill_is_kept() {
        let clip = Op::Clip { path: poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]), rule: FillRule::NonZero };
        let past =
            Op::Fill { path: poly(&[(0.0, 0.0), (12.0, 0.0), (12.0, 10.0), (0.0, 10.0)]), rule: FillRule::NonZero, rgb: [255, 0, 0], paint: None };
        assert!(plain_fills(&[clip.clone(), past, Op::Unclip]).is_none());
        let within =
            Op::Fill { path: poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]), rule: FillRule::NonZero, rgb: [255, 0, 0], paint: None };
        assert_eq!(plain_fills(&[clip, within, Op::Unclip]).map(|f| f.len()), Some(1));
    }
}
