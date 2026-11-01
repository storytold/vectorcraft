//! The document walk: each visible object is written into the page in paint order, in document
//! space (the page's first operator maps it onto PostScript's y-up space). Mirrors the PDF
//! writer's walk, without transparency.

use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;

use kurbo::Shape;
use vectorcraft_color::{BlendMode, Color, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{
    AppearanceItem, ColorMode, Document, Effect, ImageBlob, LineCap, LineJoin, Node, NodeKind, StrokeAlign, StrokeLayer, SymbolBoxes, TextObject,
};
use vectorcraft_effects::stroke::{self, WrittenShape};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, Vec2};

use crate::ps::{self, clip_op, fill_op, push_nums, push_path};
use crate::{EpsOptions, Level, Overprint};

/// How deep symbols and brush art may nest in one another (deeper art is left out).
const MAX_NEST: u32 = 8;
/// Most pixels along a side of a freeform gradient's image (its colour field is smooth).
const MAX_FIELD_PX: f64 = 512.0;
/// Most bands a stepped gradient is drawn with.
const MAX_STEPS: usize = 1024;
/// Thinnest band of a stepped gradient (points).
const MIN_BAND: f64 = 0.25;
/// Largest image written (pixels).
const MAX_IMAGE_PIXELS: u64 = 1 << 26;

const TRANSPARENCY: &str =
    "transparency (opacity, blending modes, opacity masks, see-through gradients) is written opaque: EPS has none, flatten it first";
const RASTER_FX: &str = "raster effects (shadows, glows, blurs, feathering) are left out: EPS has no transparency, flatten them first";
const IMAGE_ALPHA_L2: &str = "transparent image pixels are written white at PostScript Level 2 (Level 3 masks them out)";
const IMAGE_PARTLY: &str = "partly transparent image pixels are written over white";
/// The warning for an image left out because it can't be decoded: the note raster exports give
/// (`vectorcraft-engine`'s `rasterfx::undecoded_warning`).
const UNDECODED: &str = "an image couldn't be decoded and is missing from the picture";
const LARGE_IMAGE: &str = "an image that is too large for EPS was left out";
const SPOT_GRADIENTS_RGB: &str = "gradients mixing a spot color with RGB or Lab colors are written without the spot ink";
const SPOT_GRADIENTS_L2: &str =
    "gradients mixing a spot color with other inks are written in process colors at PostScript Level 2 (Level 3 keeps the spot inks)";
const SPOT_GRADIENTS_SHARED: &str = "gradients whose spot inks can't share a DeviceN color space are written without the spot inks";
const SPOT_GRADIENTS_PERCEPTUAL: &str = "Perceptual gradients of spot colors are written in process colors";
/// The most colorants a DeviceN color space may have (PostScript's limit).
const MAX_DEVICE_N: usize = 32;
const MISSING_PATTERN: &str = "missing patterns are written as mid-grey";
const TOO_LARGE: &str = "the artwork is too large for EPS: what follows 256 MB of PostScript was left out";
/// The most PostScript the page body may grow to: symbols and patterns that hold one another
/// expand into every instance, so nesting depth alone doesn't bound the output.
const MAX_BODY: usize = 256 << 20;

/// What the walk wrote: definitions for the setup, the page, the spot colours it uses (name and
/// CMYK equivalent) and the warnings.
pub(crate) struct Page {
    pub setup: String,
    pub body: String,
    pub custom: Vec<(String, [f32; 4])>,
    pub warnings: Vec<String>,
}

/// The colour spaces process colours are written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Space {
    Gray,
    Rgb,
    Cmyk,
}

impl Space {
    fn name(self) -> &'static str {
        match self {
            Space::Gray => "/DeviceGray",
            Space::Rgb => "/DeviceRGB",
            Space::Cmyk => "/DeviceCMYK",
        }
    }
    fn n(self) -> usize {
        match self {
            Space::Gray => 1,
            Space::Rgb => 3,
            Space::Cmyk => 4,
        }
    }
}

/// The color space a gradient's stops are written in: a process space, or one defined in the
/// setup (a spot color's Separation space or a DeviceN space of spot and process inks) by name,
/// with its number of components.
#[derive(Clone, Debug, PartialEq)]
enum GradientSpace {
    Process(Space),
    Inks(String, usize),
}

impl GradientSpace {
    fn n(&self) -> usize {
        match self {
            GradientSpace::Process(s) => s.n(),
            GradientSpace::Inks(_, n) => *n,
        }
    }
}

/// A gradient's stops as written: offsets from 0 to 1 and the components in its space.
type Stops = Vec<(f64, Vec<f32>)>;
/// A gradient's stops before their offsets are put in order and run from 0 to 1.
type RawStops = Vec<(f32, Vec<f32>)>;

/// A colour as written: in a process space (its first [`Space::n`] components) or a tint of a
/// spot colour space defined in the setup.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ink {
    Process(Space, [f32; 4]),
    Spot { space: usize, tint: f32 },
}

fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

/// Can `a` be drawn through (a finite, non-flat map)?
fn invertible(a: Affine) -> bool {
    let det = a.determinant();
    det.is_finite() && det.abs() > 1e-12
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

pub(crate) struct Scene<'a> {
    doc: &'a Document,
    /// The art boxes of `doc`'s symbols, which the walk measures symbol instances by when it
    /// leaves out objects outside the region ([`SymbolBoxes::quick`]). The box of an instance whose
    /// art holds brushed strokes grows by as far as their brush art can paint
    /// ([`SymbolBoxes::with_brush_art`]).
    symbols: SymbolBoxes<'a>,
    o: &'a EpsOptions,
    /// RGB and Lab colours are written as CMYK.
    cmyk: bool,
    cms: Arc<vectorcraft_color::cms::Cms>,
    body: String,
    setup: String,
    /// Spot colour spaces defined in the setup (`VCsp<i>`): swatch name and CMYK equivalent.
    spots: Vec<(String, [f32; 4])>,
    /// DeviceN color spaces defined in the setup (`VCdn<i>`), as their definitions.
    device_n: Vec<String>,
    warnings: Vec<String>,
    brushes: Option<Vec<vectorcraft_brush::Brush>>,
    /// Image keys → what draws them (`None`: left out).
    images: HashMap<String, Option<Arc<String>>>,
    /// The source key and solid paint together identify a colorized encoding.
    colorized_images: HashMap<(String, String), Option<Arc<String>>>,
    /// Symbols and brush art being written inside one another.
    nest: u32,
}

impl<'a> Scene<'a> {
    pub fn new(doc: &'a Document, o: &'a EpsOptions) -> Self {
        Self {
            doc,
            symbols: doc.symbol_boxes().quick().with_brush_art(),
            o,
            cmyk: o.cmyk || doc.color_mode == ColorMode::Cmyk,
            cms: vectorcraft_color::cms::active(),
            body: String::new(),
            setup: String::new(),
            spots: vec![],
            device_n: vec![],
            warnings: vec![],
            brushes: None,
            images: HashMap::new(),
            colorized_images: HashMap::new(),
            nest: 0,
        }
    }

    pub fn run(mut self) -> Page {
        let doc = self.doc;
        for l in &doc.layers {
            self.node(l, false);
        }
        self.page()
    }

    /// Write one object (art outside the document: a print job's marks) instead of the layers.
    pub fn run_node(mut self, n: &Node) -> Page {
        self.node(n, false);
        self.page()
    }

    fn page(self) -> Page {
        let custom = self.spots.iter().filter(|(n, _)| n != vectorcraft_color::swatch::REGISTRATION).cloned().collect();
        Page { setup: self.setup, body: self.body, custom, warnings: self.warnings }
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn line(&mut self, s: &str) {
        self.body.push_str(s);
        self.body.push('\n');
    }

    // ---------- colour ----------

    /// A process colour in the space it is written in.
    fn process(&self, c: &Color) -> Ink {
        match *c {
            Color::Cmyk { c, m, y, k } => Ink::Process(Space::Cmyk, [c, m, y, k].map(unit)),
            // Grey is ink coverage; PostScript's grey is lightness.
            Color::Gray { k } => Ink::Process(Space::Gray, [unit(1.0 - k), 0.0, 0.0, 0.0]),
            _ if self.cmyk => Ink::Process(Space::Cmyk, self.cms.to_cmyk(c, self.cms.settings().intent).map(unit)),
            Color::Rgb { r, g, b } => Ink::Process(Space::Rgb, [r, g, b, 0.0].map(unit)),
            Color::Lab { .. } => {
                let [r, g, b] = c.to_rgb();
                Ink::Process(Space::Rgb, [r, g, b, 0.0].map(unit))
            }
        }
    }

    /// A solid paint: a tint of its spot colour (or of Registration, every plate) when it is linked
    /// to one, else a process colour.
    fn solid(&mut self, c: &Color, link: Option<&str>, tint: f32) -> Ink {
        match link.and_then(|n| self.spot_space(n)) {
            Some(space) => Ink::Spot { space, tint: unit(tint) },
            None => self.process(c),
        }
    }

    /// The index of the Separation colour space of spot swatch (or Registration) `name`, defined
    /// in the setup the first time; `None` when `name` is neither.
    fn spot_space(&mut self, name: &str) -> Option<usize> {
        if let Some(i) = self.spots.iter().position(|(n, _)| n == name) {
            return Some(i);
        }
        let registration = name == vectorcraft_color::swatch::REGISTRATION;
        let cmyk = if registration {
            [1.0; 4]
        } else {
            let doc = self.doc;
            let sw = doc.swatch(name).filter(|s| s.spot)?;
            let color = doc.linked_color(sw.paint.color()?, true);
            self.cms.to_cmyk(&color, self.cms.settings().intent).map(unit)
        };
        let i = self.spots.len();
        let _ = write!(self.setup, "/VCsp{i} [/Separation ");
        if registration {
            self.setup.push_str("/All /DeviceCMYK {dup dup dup}");
        } else {
            let [c, m, y, k] = cmyk.map(|v| ps::num(f64::from(v)));
            let _ = write!(self.setup, "{} cvn /DeviceCMYK {{dup {c} mul exch dup {m} mul exch dup {y} mul exch {k} mul}}", ps::string(name));
        }
        self.setup.push_str("] def\n");
        self.spots.push((name.to_string(), cmyk));
        Some(i)
    }

    fn set_ink(&mut self, ink: Ink) {
        match ink {
            Ink::Process(space, v) => {
                for x in v.iter().take(space.n()) {
                    ps::push_num(&mut self.body, f64::from(*x));
                    self.body.push(' ');
                }
                self.line(match space {
                    Space::Gray => "g",
                    Space::Rgb => "rg",
                    Space::Cmyk => "k",
                });
            }
            Ink::Spot { space, tint } => {
                let _ = writeln!(self.body, "VCsp{space} setcolorspace {} setcolor", ps::num(f64::from(tint)));
            }
        }
    }

    /// Turn overprinting on for one paint (when overprints are preserved); `false` when it stays off.
    fn overprint_on(&mut self, overprint: bool) -> bool {
        let on = overprint && self.o.overprint == Overprint::Preserve;
        if on {
            self.line("true op");
        }
        on
    }

    fn overprint_off(&mut self, on: bool) {
        if on {
            self.line("false op");
        }
    }

    // ---------- objects ----------

    fn node(&mut self, n: &Node, force: bool) {
        if !force && !n.visible {
            return;
        }
        if let NodeKind::Layer { template: true, .. } = n.kind {
            return;
        }
        if self.body.len() >= MAX_BODY {
            self.warn(TOO_LARGE);
            return;
        }
        match n.visual_bounds_with(&self.symbols) {
            Some(b) if !force && !overlaps(b.inflate(1.0, 1.0), self.o.region) => return,
            None if !n.is_container() => return,
            _ => {}
        }
        self.note_losses(n);
        let was = self.body.len();
        match &n.kind {
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
                children.iter().for_each(|c| self.node(c, false));
            }
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => self.clip_group(children),
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
            NodeKind::Text(t) => self.text(n, t),
            NodeKind::Image(im) => {
                let blob = self.doc.image_blob(im);
                match n.extras().colorize.as_deref() {
                    Some(p @ Paint::Solid { .. }) if blob.is_none_or(|b| b.gray_source().is_some()) => {
                        self.colorized_image(&im.key, blob, im.width, im.height, im.xf, p);
                    }
                    _ => self.image(&im.key, blob, im.width, im.height, im.xf),
                }
            }
            NodeKind::SymbolInstance { symbol, xf } => {
                if let Some(sym) = self.doc.symbols.iter().find(|s| &s.name == symbol)
                    && self.nest < MAX_NEST
                {
                    let mut art = vectorcraft_doc::symbol_overrides::dynamic_art(&sym.art, n);
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
            | NodeKind::Intertwine { .. } => {
                let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                for c in g.children().into_iter().flatten() {
                    self.node(c, false);
                }
            }
        }
        if self.body.len() > MAX_BODY {
            self.body.truncate(was);
            self.warn(TOO_LARGE);
        }
    }

    /// A clipping group: the art inside its first child's outline. The clipping path's fills
    /// paint behind the art (clipped) and its strokes over it (not clipped).
    fn clip_group(&mut self, children: &[Arc<Node>]) {
        let Some((clip, rest)) = children.split_first() else { return };
        // Nothing to clip by hides the clipped art.
        let Some((outline, rule)) = vectorcraft_effects::clip_outline(clip) else { return };
        let paint = clip.clip_paint();
        self.line("q");
        push_path(&mut self.body, &outline);
        self.body.push_str(clip_op(rule));
        if let Some(fill) = &paint.fill {
            self.node(fill, false);
        }
        rest.iter().for_each(|c| self.node(c, false));
        self.line("Q");
        if let Some(stroke) = &paint.stroke {
            self.node(stroke, false);
        }
    }

    /// Warn about what EPS can't hold in `n`'s look.
    fn note_losses(&mut self, n: &Node) {
        if n.opacity < 1.0 || n.blend != BlendMode::Normal || n.extras().mask.as_ref().is_some_and(|m| !m.disabled) {
            self.warn(TRANSPARENCY);
        }
        let raster = |fx: &[Effect]| fx.iter().any(|e| e.visible && vectorcraft_effects::is_raster(&e.id));
        if raster(&n.appearance.effects) || n.appearance.items.iter().any(|i| raster(i.effects())) {
            self.warn(RASTER_FX);
        }
    }

    /// Warn when a fill's or stroke's own opacity or blending mode can't be written.
    fn note_item(&mut self, opacity: f32, blend: BlendMode) {
        if opacity < 1.0 || blend != BlendMode::Normal {
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
                AppearanceItem::Fill(fl) if fl.visible && !fl.paint.is_none() => {
                    self.note_item(fl.opacity, fl.blend);
                    self.fill(bp, rule, &fl.paint, bounds, fl.overprint);
                }
                AppearanceItem::Stroke(st) if st.visible && !st.paint.is_none() && st.width > 0.0 && st.width.is_finite() => {
                    self.stroke(bp, rule, st, bounds);
                }
                _ => {}
            }
        }
    }

    /// Fill `bp` (by `rule`) with paint `p`; `bounds` places unplaced gradients and patterns.
    fn fill(&mut self, bp: &BezPath, rule: FillRule, p: &Paint, bounds: Rect, overprint: bool) {
        if p.is_none() {
            return;
        }
        let on = self.overprint_on(overprint);
        match p {
            Paint::None => {}
            Paint::Solid { color, swatch, tint } => {
                let ink = self.solid(color, swatch.as_deref(), *tint);
                self.set_ink(ink);
                push_path(&mut self.body, bp);
                self.body.push_str(fill_op(rule));
            }
            _ => {
                self.line("q");
                push_path(&mut self.body, bp);
                self.body.push_str(clip_op(rule));
                self.paint_area(p, bounds, bp.bounding_box());
                self.line("Q");
            }
        }
        self.overprint_off(on);
    }

    /// Paint all of `area` (inside the current clip) with gradient or pattern `p`, placed on
    /// `bounds`.
    fn paint_area(&mut self, p: &Paint, bounds: Rect, area: Rect) {
        let doc = self.doc;
        match p {
            Paint::Gradient(g) if g.gradient.kind == GradientKind::Freeform => self.field_image(g, bounds, area),
            Paint::Gradient(g) => self.gradient(g, bounds, area),
            Paint::Pattern { pattern, xf } => match doc.pattern(pattern) {
                Some(def) => {
                    for inst in def.instances_in(*xf, area) {
                        self.node(&inst, true);
                    }
                }
                None => {
                    self.warn(MISSING_PATTERN);
                    self.set_ink(Ink::Process(Space::Gray, [0.5, 0.0, 0.0, 0.0]));
                    self.rect_fill(area);
                }
            },
            Paint::Solid { color, swatch, tint } => {
                let ink = self.solid(color, swatch.as_deref(), *tint);
                self.set_ink(ink);
                self.rect_fill(area);
            }
            Paint::None => {}
        }
    }

    /// Fill rectangle `r` (a little larger, so its edges fall outside any clip it fills).
    fn rect_fill(&mut self, r: Rect) {
        let r = r.abs().inflate(1.0, 1.0);
        push_nums(&mut self.body, &[r.x0, r.y0, r.width(), r.height()]);
        self.line("rectfill");
    }

    /// Paint stroke `st` of the shape `bp` (filled by `rule`; geometric bounds `bounds`).
    fn stroke(&mut self, bp: &BezPath, rule: FillRule, st: &StrokeLayer, bounds: Rect) {
        if !stroke::is_plain(st) && self.brush_art(bp, st) {
            return;
        }
        self.note_item(st.opacity, st.blend);
        let paint_bounds = st.paint_bounds(bounds);
        let area_paint = match &st.paint {
            Paint::Pattern { .. } => true,
            Paint::Gradient(g) => g.gradient.kind == GradientKind::Freeform,
            _ => false,
        };
        if area_paint {
            // The area the stroke paints, as Outline Stroke makes it, painted with the pattern's
            // tiles or the freeform's image.
            let region = stroke::outline_region(&PathData::from_bezpath(bp), rule, st).to_bezpath();
            self.fill(&region, FillRule::NonZero, &st.paint, paint_bounds, st.overprint);
            return;
        }
        let w = stroke::for_writer(bp, st);
        let sided = match w.side {
            Some(StrokeAlign::Inside) => {
                self.line("q");
                push_path(&mut self.body, bp);
                self.body.push_str(clip_op(rule));
                true
            }
            Some(StrokeAlign::Outside) => {
                // Everything outside the path: a frame around all the stroke reaches plus the
                // path, even-odd.
                let mut outside = w.reach(st, bounds).inflate(1.0, 1.0).to_path(0.1);
                outside.extend(bp.iter());
                self.line("q");
                push_path(&mut self.body, &outside);
                self.body.push_str(clip_op(FillRule::EvenOdd));
                true
            }
            _ => false,
        };
        match &w.shape {
            WrittenShape::Stroke { width } => {
                let on = self.overprint_on(st.overprint);
                self.line_style(st, *width);
                match &st.paint {
                    Paint::Solid { color, swatch, tint } => {
                        let ink = self.solid(color, swatch.as_deref(), *tint);
                        self.set_ink(ink);
                        push_path(&mut self.body, bp);
                        self.line("S");
                    }
                    p => {
                        // A gradient within the stroke: the stroke's outline as a clip.
                        self.line("q");
                        push_path(&mut self.body, bp);
                        self.line("strokepath W");
                        self.paint_area(p, paint_bounds, w.reach(st, bounds));
                        self.line("Q");
                    }
                }
                self.overprint_off(on);
            }
            WrittenShape::Fill(outlines) if st.path_gradient().is_some() => {
                // A gradient along or across the stroke: slices clipped to its outlines.
                if let Some(ws) = stroke::written_slices(bp, rule, st, outlines) {
                    self.line("q");
                    push_path(&mut self.body, &ws.clip);
                    self.body.push_str(clip_op(FillRule::NonZero));
                    for (shape, paint) in &ws.slices {
                        self.fill(shape, FillRule::NonZero, paint, shape.bounding_box(), st.overprint);
                    }
                    self.line("Q");
                }
            }
            WrittenShape::Fill(outlines) => {
                for o in outlines {
                    self.fill(o, FillRule::NonZero, &st.paint, paint_bounds, st.overprint);
                }
            }
        }
        if sided {
            self.line("Q");
        }
    }

    /// The line width, cap, join, miter limit and dashes of `st` at `width`.
    fn line_style(&mut self, st: &StrokeLayer, width: f64) {
        let cap = match st.cap {
            LineCap::Butt => 0,
            LineCap::Round => 1,
            LineCap::Square => 2,
        };
        let join = match st.join {
            LineJoin::Miter => 0,
            LineJoin::Round => 1,
            LineJoin::Bevel => 2,
        };
        let s = &mut self.body;
        ps::push_num(s, width);
        let _ = write!(s, " w {cap} J {join} j ");
        ps::push_num(s, if st.miter_limit.is_finite() { st.miter_limit.max(1.0) } else { 10.0 });
        s.push_str(" M [");
        let dash = st.dash.as_ref().filter(|d| d.is_dashed());
        if let Some(d) = dash {
            push_nums(s, &d.pattern);
        }
        s.push_str("] ");
        ps::push_num(s, dash.map_or(0.0, |d| d.offset));
        s.push_str(" d\n");
    }

    /// Paint stroke `st` of `bp` with its brush art. False when it has no known brush.
    fn brush_art(&mut self, bp: &BezPath, st: &StrokeLayer) -> bool {
        if self.nest >= MAX_NEST {
            return false;
        }
        let doc = self.doc;
        let brushes = self.brushes.get_or_insert_with(|| vectorcraft_brush::library(doc));
        let Some(b) = st.brush.as_deref().and_then(|name| brushes.iter().find(|b| b.name == name)) else { return false };
        let art = vectorcraft_brush::stroke_pieces(doc, b, bp, st);
        self.note_item(st.opacity, st.blend);
        self.nest += 1;
        for piece in &art {
            self.node(piece, false);
        }
        self.nest -= 1;
        true
    }

    // ---------- type ----------

    /// Type as glyph outlines: the object's own fills and strokes below and above the
    /// characters, each run's fill and stroke in text space.
    fn text(&mut self, n: &Node, t: &TextObject) {
        if !invertible(t.xf) {
            return;
        }
        let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let tb = t.xf.transform_rect_bbox(layout.bounds);
        let (below, above) = n.appearance.split_contents();
        let all = (!n.appearance.items.is_empty()).then(|| {
            let mut all = layout.to_bezpath();
            all.apply_affine(t.xf);
            all
        });
        if let Some(bp) = &all {
            self.text_items(below, bp, tb);
        }
        self.line("q");
        let _ = writeln!(self.body, "{} cm", ps::matrix(t.xf));
        for (i, run) in t.runs.iter().enumerate() {
            let mut bp = BezPath::new();
            for g in layout.glyphs.iter().filter(|g| g.run == i) {
                bp.extend(g.outline.iter());
                // Colour bitmap glyphs (emoji): their images, which the run's paint doesn't paint.
                if let Some((image, to_text)) = g.placed_image() {
                    let key = format!("\u{0}glyph\u{0}{}", image.blob.content_key());
                    self.image(&key, Some(&image.blob), image.width, image.height, to_text);
                }
                // COLR glyphs: their layers, as art (in text space, as the images).
                if let Some(art) = g.placed_colour(Affine::IDENTITY) {
                    self.node(&art, true);
                }
            }
            // Ruby (furigana) glyphs in their base text's run's paint, as the canvas draws them.
            for g in layout.ruby.iter().filter(|g| g.run == i) {
                bp.extend(g.outline.iter());
            }
            if bp.elements().is_empty() {
                continue;
            }
            self.fill(&bp, FillRule::NonZero, &run.style.fill, layout.bounds, run.style.overprint_fill);
            if run.style.has_stroke() {
                // Character strokes are drawn in text space, with their cap, join and dashes.
                self.stroke(&bp, FillRule::NonZero, &run.style.stroke_layer(), layout.bounds);
            }
        }
        self.line("Q");
        if let Some(bp) = &all {
            self.text_items(above, bp, tb);
        }
    }

    /// Some of a type object's own fills and strokes on its glyph outlines `bp` (bounds `tb`).
    fn text_items(&mut self, items: &[AppearanceItem], bp: &BezPath, tb: Rect) {
        for item in items {
            match item {
                AppearanceItem::Fill(fl) if fl.visible && !fl.paint.is_none() => {
                    self.note_item(fl.opacity, fl.blend);
                    self.fill(bp, FillRule::NonZero, &fl.paint, tb, fl.overprint);
                }
                AppearanceItem::Stroke(st) if st.visible && !st.paint.is_none() && st.width > 0.0 && st.width.is_finite() => {
                    self.stroke(bp, FillRule::NonZero, st, tb);
                }
                _ => {}
            }
        }
    }

    // ---------- gradients ----------

    /// The colour space a gradient's stops are written in, and its stops in it from offset 0 to 1
    /// (midpoints as stops of their own).
    fn gradient_stops(&mut self, g: &vectorcraft_color::Gradient) -> (GradientSpace, Stops) {
        if g.stops.iter().any(|s| s.opacity < 1.0) {
            self.warn(TRANSPARENCY);
        }
        let (space, raw) = match self.ink_stops(g) {
            Some(inks) => inks,
            None => {
                let (space, raw) = self.process_stops(g);
                (GradientSpace::Process(space), raw)
            }
        };
        let mut out: Stops = vec![];
        let mut last = 0.0f64;
        for (o, c) in raw {
            let o = if o.is_finite() { f64::from(o).clamp(last, 1.0) } else { last };
            last = o;
            out.push((o, c));
        }
        match (out.first().cloned(), out.last().cloned()) {
            (Some(first), Some(end)) => {
                if first.0 > 0.0 {
                    out.insert(0, (0.0, first.1));
                }
                if end.0 < 1.0 {
                    out.push((1.0, end.1));
                }
            }
            _ => out = vec![(0.0, vec![0.0; space.n()]), (1.0, vec![0.0; space.n()])],
        }
        (space, out)
    }

    /// The stops of a gradient with a spot ink as tints of its inks: a Separation space when every
    /// stop is a tint of one spot ink or paper white, else (at Level 3) a DeviceN space of the
    /// process inks the stops use, in CMYK order, and the spot inks, in the order the stops name
    /// them. Paper white adds no ink and a gray stop is black ink. An off-center midpoint is a stop
    /// with the inks halfway between its two stops. `None` for a gradient without a spot ink, and,
    /// with a warning, for one whose inks can't be kept or that is mixed by the Perceptual method;
    /// it is then written in process colors.
    fn ink_stops(&mut self, g: &vectorcraft_color::Gradient) -> Option<(GradientSpace, RawStops)> {
        let doc = self.doc;
        let spot_of = |s: &vectorcraft_color::GradientStop| -> Option<String> {
            s.swatch.as_deref().filter(|n| doc.swatch(n).is_some_and(|w| w.spot)).map(str::to_string)
        };
        // The spot inks, collected up to one more than a DeviceN space can hold.
        let mut names: Vec<String> = vec![];
        for s in &g.stops {
            if let Some(n) = spot_of(s)
                && !names.contains(&n)
            {
                names.push(n);
                if names.len() > MAX_DEVICE_N {
                    break;
                }
            }
        }
        if names.is_empty() {
            return None;
        }
        if !g.method.is_classic() {
            self.warn(SPOT_GRADIENTS_PERCEPTUAL);
            return None;
        }
        if names.len() > MAX_DEVICE_N {
            self.warn(SPOT_GRADIENTS_SHARED);
            return None;
        }
        // Paper white exactly: any ink at all, however little, is kept.
        let paper = |c: &Color| match *c {
            Color::Gray { k } => k <= 0.0,
            Color::Cmyk { c, m, y, k } => [c, m, y, k].iter().all(|&v| v <= 0.0),
            Color::Rgb { r, g, b } => [r, g, b].iter().all(|&v| v >= 1.0),
            Color::Lab { l, a, b } => l >= 100.0 && a == 0.0 && b == 0.0,
        };
        // Each stop's ink coverage: the four process inks, then one tint per spot ink.
        let mut inks: Vec<Vec<f32>> = Vec::with_capacity(g.stops.len());
        for s in &g.stops {
            let mut v = vec![0.0; 4 + names.len()];
            match spot_of(s) {
                Some(n) => {
                    if let Some(i) = names.iter().position(|m| *m == n) {
                        v[4 + i] = unit(s.tint);
                    }
                }
                None if paper(&s.color) => {}
                None => match self.process(&s.color) {
                    Ink::Process(Space::Cmyk, c) => v[..4].copy_from_slice(&c),
                    Ink::Process(Space::Gray, [lightness, ..]) => v[3] = unit(1.0 - lightness),
                    _ => {
                        self.warn(SPOT_GRADIENTS_RGB);
                        return None;
                    }
                },
            }
            inks.push(v);
        }
        let process: Vec<usize> = (0..4).filter(|&i| inks.iter().any(|v| v[i] > 0.0)).collect();
        let space = if let ([name], true) = (names.as_slice(), process.is_empty()) {
            let Some(i) = self.spot_space(name) else {
                self.warn(SPOT_GRADIENTS_SHARED);
                return None;
            };
            GradientSpace::Inks(format!("VCsp{i}"), 1)
        } else if self.o.level != Level::Three {
            self.warn(SPOT_GRADIENTS_L2);
            return None;
        } else {
            let Some(space) = self.device_n_space(&process, &names) else {
                self.warn(SPOT_GRADIENTS_SHARED);
                return None;
            };
            space
        };
        let pick = |v: &[f32]| -> Vec<f32> { process.iter().map(|&i| v[i]).chain(v[4..].iter().copied()).collect() };
        let mut out = vec![];
        for (i, s) in g.stops.iter().enumerate() {
            out.push((s.offset, pick(&inks[i])));
            if let (Some(n), Some(next)) = (g.stops.get(i + 1), inks.get(i + 1))
                && (s.midpoint - 0.5).abs() > 1e-3
            {
                let half: Vec<f32> = inks[i].iter().zip(next).map(|(a, b)| (a + b) / 2.0).collect();
                out.push((s.offset + (n.offset - s.offset) * s.midpoint, pick(&half)));
            }
        }
        Some((space, out))
    }

    /// A DeviceN color space of process inks `process` (indices into CMYK) and spot inks `names`,
    /// defined in the setup the first time, each spot ink with the CMYK equivalent its Separation
    /// space has. `None` when a spot ink has no Separation space, two colorants share a name, a
    /// spot ink has the name of a process ink, `All` or `None`, or there are more than
    /// [`MAX_DEVICE_N`] colorants.
    fn device_n_space(&mut self, process: &[usize], names: &[String]) -> Option<GradientSpace> {
        const PROCESS: [&str; 4] = ["Cyan", "Magenta", "Yellow", "Black"];
        if process.len() + names.len() > MAX_DEVICE_N {
            return None;
        }
        let mut colorants: Vec<&str> = process.iter().map(|&i| PROCESS[i]).collect();
        let mut weights: Vec<[f32; 4]> = process.iter().map(|&i| [0, 1, 2, 3].map(|j| if i == j { 1.0 } else { 0.0 })).collect();
        for (i, name) in names.iter().enumerate() {
            if PROCESS.contains(&name.as_str()) || name == "All" || name == "None" || names[..i].contains(name) {
                return None;
            }
        }
        for name in names {
            colorants.push(name);
            let i = self.spot_space(name)?;
            weights.push(self.spots.get(i)?.1);
        }
        let mut def = String::from("[/DeviceN [");
        for (i, c) in colorants.iter().enumerate() {
            if i < process.len() {
                let _ = write!(def, "/{c} ");
            } else {
                let _ = write!(def, "{} cvn ", ps::string(c));
            }
        }
        // The tint transform: the components into an array, then each CMYK ink the sum of the
        // components times their amounts of it.
        let n = weights.len();
        let _ = write!(def, "] /DeviceCMYK {{{n} array astore");
        for j in 0..4 {
            def.push_str(" 0");
            for (i, w) in weights.iter().enumerate().filter(|(_, w)| w[j] > 0.0) {
                let _ = write!(def, " {} index {i} get {} mul add", j + 1, ps::num(f64::from(w[j])));
            }
        }
        def.push_str(" 5 -1 roll pop}]");
        let i = match self.device_n.iter().position(|d| *d == def) {
            Some(i) => i,
            None => {
                let i = self.device_n.len();
                let _ = writeln!(self.setup, "/VCdn{i} {def} def");
                self.device_n.push(def);
                i
            }
        };
        Some(GradientSpace::Inks(format!("VCdn{i}"), n))
    }

    /// The process color space a gradient's stops are written in, and its stops in it (midpoints
    /// as stops of their own).
    fn process_stops(&mut self, g: &vectorcraft_color::Gradient) -> (Space, RawStops) {
        let stops = g.expanded_stops();
        let space = if self.cmyk || stops.iter().any(|(_, c, _)| matches!(c, Color::Cmyk { .. })) {
            Space::Cmyk
        } else if stops.iter().all(|(_, c, _)| matches!(c, Color::Gray { .. })) {
            Space::Gray
        } else {
            Space::Rgb
        };
        let comps = |c: &Color| -> [f32; 4] {
            match (space, *c) {
                (Space::Cmyk, _) => self.cms.to_cmyk(c, self.cms.settings().intent).map(unit),
                (Space::Gray, Color::Gray { k }) => [unit(1.0 - k), 0.0, 0.0, 0.0],
                (_, Color::Rgb { r, g, b }) => [r, g, b, 0.0].map(unit),
                _ => {
                    let [r, g, b] = c.to_rgb();
                    [r, g, b, 0.0].map(unit)
                }
            }
        };
        let out = stops.iter().map(|(o, c, _)| (*o, comps(c)[..space.n()].to_vec())).collect();
        (space, out)
    }

    /// Paint all of `area` (inside the current clip) with linear or radial gradient `g`, placed on
    /// `bounds`: a smooth shading at Level 3, else stepped fills.
    fn gradient(&mut self, g: &GradientPaint, bounds: Rect, area: Rect) {
        let geom = g.resolve(bounds);
        let (space, stops) = self.gradient_stops(&g.gradient);
        if !stops.windows(2).any(|w| w[1].0 > w[0].0) {
            if let Some((_, c)) = stops.first() {
                self.set_comps(&space, c);
                self.rect_fill(area);
            }
            return;
        }
        let space = &space;
        let smooth = self.o.level == Level::Three && !self.o.compatible_gradients;
        match g.gradient.kind {
            GradientKind::Radial => {
                let r = geom.length().max(1e-6);
                let squash = geom.radial_squash();
                let inv = squash.inverse();
                let (f, c) = (inv * geom.focal_point(), geom.start);
                self.line("q");
                let _ = writeln!(self.body, "{} cm", ps::matrix(squash));
                if smooth {
                    self.shading(3, space, &[f.x, f.y, 0.0, c.x, c.y, r], &stops);
                } else {
                    self.radial_steps(f, c, r, inv.transform_rect_bbox(area), space, &stops);
                }
                self.line("Q");
            }
            _ => {
                let (s, mut e) = (geom.start, geom.end);
                if s.distance(e) < 1e-9 {
                    e = s + Vec2::new(1.0, 0.0);
                }
                if smooth {
                    self.shading(2, space, &[s.x, s.y, e.x, e.y], &stops);
                } else {
                    self.linear_steps(s, e, area, space, &stops);
                }
            }
        }
    }

    /// A Level 3 smooth shading of `kind` (2 axial, 3 radial) at `coords`, extended both ways.
    fn shading(&mut self, kind: u8, space: &GradientSpace, coords: &[f64], stops: &[(f64, Vec<f32>)]) {
        let n = space.n();
        let comps = |c: &[f32]| -> String {
            let mut s = String::from("[");
            push_nums(&mut s, &c.iter().take(n).map(|v| f64::from(*v)).collect::<Vec<_>>());
            s.pop();
            s.push(']');
            s
        };
        let segments: Vec<_> = stops.windows(2).filter(|w| w[1].0 > w[0].0).collect();
        let interp = |w: &&[(f64, Vec<f32>)]| format!("<< /FunctionType 2 /Domain [0 1] /C0 {} /C1 {} /N 1 >>", comps(&w[0].1), comps(&w[1].1));
        let function = match segments.as_slice() {
            [one] => interp(one),
            many => {
                let mut f = String::from("<< /FunctionType 3 /Domain [0 1] /Functions [\n");
                for w in many {
                    f.push_str(&interp(w));
                    f.push('\n');
                }
                f.push_str("] /Bounds [");
                let bounds: Vec<f64> = many.iter().skip(1).map(|w| w[0].0).collect();
                push_nums(&mut f, &bounds);
                f.push_str("] /Encode [");
                f.push_str(&"0 1 ".repeat(many.len()));
                f.push_str("] >>");
                f
            }
        };
        let name = match space {
            GradientSpace::Process(s) => s.name(),
            GradientSpace::Inks(name, _) => name,
        };
        let _ = write!(self.body, "<< /ShadingType {kind} /ColorSpace {name} /Coords [");
        push_nums(&mut self.body, coords);
        let _ = writeln!(self.body, "] /Extend [true true]\n/Function {function} >> shfill");
    }

    /// The bands of each stretch between stops: how many, for colours `a` to `b` over `length`
    /// points (one per colour level, none thinner than [`MIN_BAND`]).
    fn bands(a: &[f32], b: &[f32], length: f64, segments: usize) -> usize {
        let delta = a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        let by_color = (f64::from(delta) * 255.0).ceil().max(1.0) as usize;
        let by_size = (length / MIN_BAND).ceil().max(1.0) as usize;
        by_color.min(by_size).min(MAX_STEPS / segments.max(1)).max(1)
    }

    fn set_comps(&mut self, space: &GradientSpace, c: &[f32]) {
        match space {
            GradientSpace::Process(s) => {
                let mut v = [0.0; 4];
                for (x, y) in v.iter_mut().zip(c) {
                    *x = *y;
                }
                self.set_ink(Ink::Process(*s, v));
            }
            GradientSpace::Inks(name, _) => {
                let _ = write!(self.body, "{name} setcolorspace ");
                push_nums(&mut self.body, &c.iter().map(|v| f64::from(*v)).collect::<Vec<_>>());
                self.line("setcolor");
            }
        }
    }

    /// A linear gradient from `s` to `e` as bands across it, covering `area`: each band painted
    /// from where it starts to the far end, so the next one leaves no seam.
    fn linear_steps(&mut self, s: Point, e: Point, area: Rect, space: &GradientSpace, stops: &[(f64, Vec<f32>)]) {
        let d = e - s;
        let len = d.hypot();
        // Gradient space: x along the gradient (0 to 1), y across it in points.
        let frame = Affine::new([d.x, d.y, -d.y / len, d.x / len, s.x, s.y]);
        let local = frame.inverse().transform_rect_bbox(area.inflate(1.0, 1.0));
        let (t0, t1) = (local.x0.min(0.0), local.x1.max(1.0));
        self.line("q");
        let _ = writeln!(self.body, "{} cm", ps::matrix(frame));
        let band = |sc: &mut Self, from: f64| {
            push_nums(&mut sc.body, &[from, local.y0, t1 - from, local.height()]);
            sc.line("rectfill");
        };
        if let Some((_, c)) = stops.first() {
            self.set_comps(space, c);
            band(self, t0);
        }
        let segments: Vec<_> = stops.windows(2).filter(|w| w[1].0 > w[0].0).collect();
        for w in &segments {
            let ((oa, ca), (ob, cb)) = (&w[0], &w[1]);
            let n = Self::bands(ca, cb, (ob - oa) * len, segments.len());
            for k in 0..n {
                let at = oa + (ob - oa) * k as f64 / n as f64;
                self.set_comps(space, &mix(ca, cb, (k as f32 + 0.5) / n as f32));
                band(self, at);
            }
        }
        if let Some((_, c)) = stops.last() {
            self.set_comps(space, c);
            band(self, 1.0);
        }
        self.line("Q");
    }

    /// A radial gradient from focal point `f` to the circle of radius `r` around `c` as discs,
    /// largest first, over the last colour across `area` (all in the gradient's own space).
    fn radial_steps(&mut self, f: Point, c: Point, r: f64, area: Rect, space: &GradientSpace, stops: &[(f64, Vec<f32>)]) {
        if let Some((_, last)) = stops.last() {
            self.set_comps(space, last);
            self.rect_fill(area);
        }
        let segments: Vec<_> = stops.windows(2).filter(|w| w[1].0 > w[0].0).collect();
        for w in segments.iter().rev() {
            let ((oa, ca), (ob, cb)) = (&w[0], &w[1]);
            let n = Self::bands(ca, cb, (ob - oa) * r, segments.len());
            for k in (0..n).rev() {
                let t = oa + (ob - oa) * (k + 1) as f64 / n as f64;
                self.set_comps(space, &mix(ca, cb, (k as f32 + 0.5) / n as f32));
                let at = f + (c - f) * t;
                push_nums(&mut self.body, &[at.x, at.y, r * t]);
                self.line("0 360 newpath arc fill");
            }
        }
    }

    // ---------- images ----------

    /// Paint all of `area` with an image of freeform gradient `g` placed on `bounds`, sampled at
    /// the document's raster effects resolution (at most [`MAX_FIELD_PX`] a side).
    fn field_image(&mut self, g: &GradientPaint, bounds: Rect, area: Rect) {
        use vectorcraft_color::freeform::{painted_box, spread_scale};
        let Some(b) = painted_box(bounds) else { return };
        let area = area.abs();
        let long = area.width().max(area.height());
        if !(long > 1e-9 && long.is_finite()) {
            return;
        }
        let k = (self.doc.raster_effects_ppi / 72.0).min(MAX_FIELD_PX / long);
        let along = |len: f64| (len * k).ceil().clamp(1.0, MAX_FIELD_PX) as u16;
        let (cols, rows) = (along(area.width()), along(area.height()));
        let field = g.freeform_on(b).field_with(spread_scale(b), &|c| c.to_rgb());
        let q = |v: f32| (unit(v) * 255.0).round() as u8;
        let rgba: Vec<u8> = field.grid(area, cols, rows).flat_map(|(c, a)| [q(c[0]), q(c[1]), q(c[2]), q(a)]).collect();
        let place = Affine::translate(area.origin().to_vec2()) * Affine::scale_non_uniform(area.width(), area.height());
        if let Some(image) = self.encode_image(&rgba, u32::from(cols), u32::from(rows)) {
            self.place_image(&image, place);
        }
    }

    /// A placed image (`blob`, cached under `key`): its pixel grid of `width` × `height` placed
    /// by `xf` (each image file encoded once).
    fn image(&mut self, key: &str, blob: Option<&ImageBlob>, width: u32, height: u32, xf: Affine) {
        let place = xf * Affine::scale_non_uniform(f64::from(width.max(1)), f64::from(height.max(1)));
        if !invertible(place) {
            return;
        }
        let encoded = match self.images.get(key) {
            Some(e) => e.clone(),
            None => {
                let e = self.decode_image(blob).and_then(|img| self.encode_image(img.as_raw(), img.width(), img.height())).map(Arc::new);
                self.images.insert(key.to_string(), e.clone());
                e
            }
        };
        if let Some(e) = encoded {
            self.place_image(&e, place);
        }
    }

    /// Gray coverage in the selected process or Separation ink. Binary coverage is a stencil;
    /// continuous coverage leaves zero pixels untouched at Level 3 and writes them as paper at Level 2.
    fn colorized_image(&mut self, key: &str, blob: Option<&ImageBlob>, width: u32, height: u32, xf: Affine, paint: &Paint) {
        let place = xf * Affine::scale_non_uniform(f64::from(width.max(1)), f64::from(height.max(1)));
        if !invertible(place) {
            return;
        }
        let cache = (key.to_string(), format!("{paint:?}"));
        let encoded = match self.colorized_images.get(&cache) {
            Some(e) => e.clone(),
            None => {
                let e = self.encode_colorized(blob, paint).map(Arc::new);
                self.colorized_images.insert(cache, e.clone());
                e
            }
        };
        if let Some(e) = encoded {
            self.place_image(&e, place);
        }
    }

    fn encode_colorized(&mut self, blob: Option<&ImageBlob>, paint: &Paint) -> Option<String> {
        let Some((w, h, coverage)) = blob.and_then(ImageBlob::coverage) else {
            self.warn(UNDECODED);
            return None;
        };
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
            self.warn(LARGE_IMAGE);
            return None;
        }
        let Paint::Solid { color, swatch, tint } = paint else { return None };
        let ink = self.solid(color, swatch.as_deref(), *tint);
        let binary = coverage.iter().all(|&v| v == 0 || v == 255);
        let clear = coverage.contains(&0);
        if coverage.iter().any(|&v| v > 0 && v < 255) {
            self.warn(IMAGE_PARTLY);
        }
        let keyed = !binary && clear && self.o.level == Level::Three;
        if !binary && clear && !keyed {
            self.warn(IMAGE_ALPHA_L2);
        }
        // Zero tint is uncommon for covered spot pixels; process uses the same rare key as
        // ordinary images. Nudge a covered collision so only clear pixels match the mask.
        let key = match ink {
            Ink::Spot { .. } => [0, 0, 0, 0],
            Ink::Process(Space::Gray, ..) => [0, 0, 0, 0],
            Ink::Process(..) => [255, 0, 255, 0],
        };
        let mut data = Vec::new();
        if binary {
            let stride = (w as usize).div_ceil(8);
            data.resize(stride.saturating_mul(h as usize), 0);
            for (i, &v) in coverage.iter().enumerate() {
                if v == 255
                    && let Some(byte) = data.get_mut((i / w as usize) * stride + (i % w as usize) / 8)
                {
                    *byte |= 0x80 >> (i % w as usize % 8);
                }
            }
        } else {
            for &v in &coverage {
                if v == 0 && keyed {
                    let n = match ink {
                        Ink::Spot { .. } => 1,
                        Ink::Process(space, ..) => space.n(),
                    };
                    data.extend(key.iter().take(n));
                    continue;
                }
                let a = f32::from(v) / 255.0;
                match ink {
                    Ink::Spot { tint, .. } => {
                        let mut sample = (255.0 * tint * a).round() as u8;
                        if keyed && sample == key[0] {
                            sample = 1;
                        }
                        data.push(sample);
                    }
                    Ink::Process(space, components) => {
                        let paper = if space == Space::Cmyk { 0.0 } else { 1.0 };
                        let n = space.n();
                        let mut samples = [0; 4];
                        for (sample, &component) in samples.iter_mut().zip(components.iter()).take(n) {
                            *sample = (255.0 * (paper + (component - paper) * a)).round() as u8;
                        }
                        if keyed && samples.iter().take(n).eq(key.iter().take(n)) {
                            samples[if n > 1 { 1 } else { 0 }] = 1;
                        }
                        data.extend(samples.iter().take(n));
                    }
                }
            }
        }
        let (filter, encoded) = match self.o.level {
            Level::Three => ("/FlateDecode", ps::deflate(&data)),
            Level::Two => {
                let mut rl = Vec::new();
                ps::packbits(&data, &mut rl);
                rl.push(128);
                ("/RunLengthDecode", rl)
            }
        };
        let mut out = String::new();
        if binary {
            // imagemask takes its current ink from the graphics state.
            out.push_str(&self.ink_code(ink));
        } else {
            match ink {
                Ink::Spot { space, .. } => {
                    let _ = writeln!(out, "VCsp{space} setcolorspace");
                }
                Ink::Process(space, ..) => {
                    let _ = writeln!(out, "{} setcolorspace", space.name());
                }
            }
        }
        let n = match ink {
            Ink::Spot { .. } => 1,
            Ink::Process(space, ..) => space.n(),
        };
        let decode = if binary { "1 0".to_string() } else { "0 1 ".repeat(n).trim_end().to_string() };
        let _ = write!(
            out,
            "{{ /VCsrc currentfile /ASCII85Decode filter def\n<< /ImageType {} /Width {w} /Height {h} /BitsPerComponent {} /Decode [{decode}] /ImageMatrix [{w} 0 0 {h} 0 0]",
            if keyed { 4 } else { 1 },
            if binary { 1 } else { 8 },
        );
        if keyed {
            let mask: Vec<String> = key.iter().take(n).map(|v| format!("{v} {v}")).collect();
            let _ = write!(out, " /MaskColor [{}]", mask.join(" "));
        }
        let _ = writeln!(out, "\n/DataSource VCsrc {filter} filter >> {}\nVCsrc flushfile }} exec", if binary { "imagemask" } else { "image" });
        out.push_str(&ps::ascii85(&encoded));
        Some(out)
    }

    /// Ink-setting operators in the cached stencil body, built with the same path as solid fills.
    fn ink_code(&mut self, ink: Ink) -> String {
        let body = std::mem::take(&mut self.body);
        self.set_ink(ink);
        std::mem::replace(&mut self.body, body)
    }

    /// Image `blob` as RGBA pixels; `None` (with a warning) when it can't be written.
    fn decode_image(&mut self, blob: Option<&ImageBlob>) -> Option<image::RgbaImage> {
        let Some(img) = blob.and_then(|blob| image::load_from_memory(&blob.bytes).ok()) else {
            self.warn(UNDECODED);
            return None;
        };
        if u64::from(img.width()) * u64::from(img.height()) > MAX_IMAGE_PIXELS {
            self.warn(LARGE_IMAGE);
            return None;
        }
        Some(img.to_rgba8())
    }

    /// Draw an image of [`Self::encode_image`] on the unit square mapped by `place`.
    fn place_image(&mut self, encoded: &str, place: Affine) {
        if !invertible(place) {
            return;
        }
        if self.body.len().saturating_add(encoded.len()).saturating_add(64) > MAX_BODY {
            self.warn(TOO_LARGE);
            return;
        }
        self.line("q");
        let _ = writeln!(self.body, "{} cm", ps::matrix(place));
        self.body.push_str(encoded);
        self.line("Q");
    }

    /// Straight RGBA pixels (`w` × `h`, rows from the top) as the operators and data that draw
    /// them on the unit square: in the process colour space, transparent pixels masked out at
    /// Level 3 (a key colour no opaque pixel has), white at Level 2; partly transparent ones over
    /// white. `None` for an empty image.
    fn encode_image(&mut self, rgba: &[u8], w: u32, h: u32) -> Option<String> {
        if w == 0 || h == 0 {
            return None;
        }
        let px = rgba.as_chunks::<4>().0;
        let clear = px.iter().any(|p| p[3] == 0);
        if px.iter().any(|p| p[3] > 0 && p[3] < 255) {
            self.warn(IMAGE_PARTLY);
        }
        let keyed = clear && self.o.level == Level::Three;
        if clear && !keyed {
            self.warn(IMAGE_ALPHA_L2);
        }
        let space = if self.cmyk { Space::Cmyk } else { Space::Rgb };
        let n = space.n();
        // The key colour, which opaque pixels that happen to have it are nudged off.
        let key: [u8; 4] = [255, 0, 255, 0];
        // White: no ink, or full light.
        let white: [u8; 4] = if space == Space::Cmyk { [0; 4] } else { [255; 4] };
        let mut cache: HashMap<[u8; 3], [u8; 4]> = HashMap::new();
        let cms = self.cms.clone();
        let intent = cms.settings().intent;
        let mut data = Vec::with_capacity(px.len() * n);
        for p in px {
            if p[3] == 0 {
                data.extend(if keyed { &key } else { &white }.iter().take(n));
                continue;
            }
            let al = u32::from(p[3]);
            let rgb = [0, 1, 2].map(|i| ((u32::from(p[i]) * al + 255 * (255 - al) + 127) / 255) as u8);
            let mut v = match space {
                Space::Cmyk => *cache
                    .entry(rgb)
                    .or_insert_with(|| cms.srgb_to_cmyk(rgb.map(|x| f32::from(x) / 255.0), intent).map(|x| (unit(x) * 255.0).round() as u8)),
                _ => [rgb[0], rgb[1], rgb[2], 0],
            };
            if keyed && v.iter().take(n).eq(key.iter().take(n)) {
                v[1] = 1;
            }
            data.extend(v.iter().take(n));
        }
        let (filter, encoded) = match self.o.level {
            Level::Three => ("/FlateDecode", ps::deflate(&data)),
            Level::Two => {
                let mut rl = Vec::with_capacity(data.len() / 2);
                ps::packbits(&data, &mut rl);
                // End of data.
                rl.push(128);
                ("/RunLengthDecode", rl)
            }
        };
        let mut out = String::with_capacity(encoded.len() * 5 / 4 + 400);
        let _ = writeln!(out, "{} setcolorspace", space.name());
        let decode = "0 1 ".repeat(n);
        let _ = write!(
            out,
            "{{ /VCsrc currentfile /ASCII85Decode filter def\n<< /ImageType {} /Width {w} /Height {h} /BitsPerComponent 8 /Decode [{}] /ImageMatrix [{w} 0 0 {h} 0 0]",
            if keyed { 4 } else { 1 },
            decode.trim_end()
        );
        if keyed {
            let mask: Vec<String> = key.iter().take(n).map(|v| format!("{v} {v}")).collect();
            let _ = write!(out, " /MaskColor [{}]", mask.join(" "));
        }
        // The procedure is read whole before it runs: the image data follows it, and flushing the
        // ASCII85 filter afterwards reads past its end marker.
        let _ = writeln!(out, "\n/DataSource VCsrc {filter} filter >> image\nVCsrc flushfile }} exec");
        out.push_str(&ps::ascii85(&encoded));
        Some(out)
    }
}

/// `a` to `b` at `t`.
fn mix(a: &[f32], b: &[f32], t: f32) -> Vec<f32> {
    a.iter().zip(b).map(|(x, y)| x + (y - x) * t).collect()
}
