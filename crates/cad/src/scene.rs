//! The document walk: each visible object becomes DXF entities on its layer's DXF layer, in paint
//! order. Fills are their outlines then solid hatches (the outlines alone in R12), strokes
//! polylines and cubic splines with a lineweight and dashes, or filled outlines where the look
//! needs them.

use std::collections::HashMap;
use std::io::Cursor;

use kurbo::PathEl;
use vectorcraft_color::Paint;
use vectorcraft_doc::{AppearanceItem, Dash, Document, ImageObject, Node, NodeKind, StrokeLayer, SymbolBoxes, TextKind, TextObject, Unit};
use vectorcraft_effects::stroke;
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect};

use crate::aci::nearest_aci;
use crate::writer::{Codes, Drawing, EntityStyle, Handle, ImageDef, Layer, LineType, fixed};
use crate::{CAP_HEIGHT, ColorDepth, DxfImage, DxfOptions, DxfOutput, MAX_NEST, Preserve, RasterFormat};

/// Curves are flattened to within this distance (points) where DXF needs straight segments
/// (hatch boundaries, R12 polylines).
const FLATNESS: f64 = 0.05;
/// Longest text value older DXF versions read.
const MAX_TEXT: usize = 250;
/// The lineweights DXF knows (hundredths of a millimetre).
const LINEWEIGHTS: [i32; 24] = [0, 5, 9, 13, 15, 18, 20, 25, 30, 35, 40, 50, 53, 60, 70, 80, 90, 100, 106, 120, 140, 158, 200, 211];

const NO_TRUE_COLOR: &str = "true colour needs DXF 2004 or later: colours are written as the nearest of the 256 indexed colours";
const NO_TRANSPARENCY: &str = "transparency needs DXF 2004 or later: transparent objects are written opaque";
const NO_FILLS: &str = "DXF R12 has no fills: filled areas are written as their outlines";
const NO_IMAGES: &str = "DXF R12 has no images: placed images are left out";
const WIDE_LINES: &str =
    "lines wider than 2.11 mm (the widest lineweight) are written at 2.11 mm: Alter Paths for Appearance writes their true width";
const STROKE_EXTRAS: &str = "stroke alignment, width profiles, arrowheads and brushes are left out (Maximize Editability writes strokes as lines)";
const CLIPPING: &str = "clipping masks are not applied: the art they clip is written whole";
const TYPE_ITEMS: &str = "fills and strokes added to whole type objects are left out of text entities (outline the text to keep them)";
/// The warning for an image left out because it can't be decoded: the note raster exports give
/// (`vectorcraft-engine`'s `rasterfx::undecoded_warning`).
const UNDECODED: &str = "an image couldn't be decoded and is missing from the picture";

/// One segment of a subpath, cubics for every curve.
#[derive(Clone, Copy)]
enum Seg {
    Line(Point),
    Cubic(Point, Point, Point),
}

impl Seg {
    fn end(self) -> Point {
        match self {
            Seg::Line(p) | Seg::Cubic(_, _, p) => p,
        }
    }
}

/// A subpath: its start, its segments and whether it closes.
struct Sub {
    start: Point,
    segs: Vec<Seg>,
    closed: bool,
}

/// A subpath close enough to a circle to preserve as a native DXF CIRCLE/ARC. VectorCraft's
/// ellipse helper uses the standard four-cubic circle approximation, so recognition needs a small
/// relative tolerance rather than exact control-point tests.
struct CircularSub {
    centre: Point,
    radius: f64,
    samples: Vec<Point>,
}

fn cubic_point(p0: Point, p1: Point, p2: Point, p3: Point, t: f64) -> Point {
    let u = 1.0 - t;
    let a = u * u * u;
    let b = 3.0 * u * u * t;
    let c = 3.0 * u * t * t;
    let d = t * t * t;
    Point::new(a * p0.x + b * p1.x + c * p2.x + d * p3.x, a * p0.y + b * p1.y + c * p2.y + d * p3.y)
}

/// Conservatively fit a curved subpath to one circle. Straight segments are never promoted. The
/// 5e-4 radial tolerance is just above the maximum radial error of the usual kappa cubic circle
/// approximation (~2.7e-4 r), while rejecting visibly elliptical or arbitrary cubic curves.
fn circular_subpath(sub: &Sub) -> Option<CircularSub> {
    if sub.segs.is_empty() || sub.segs.iter().any(|s| matches!(s, Seg::Line(_))) {
        return None;
    }
    let mut samples = vec![sub.start];
    let mut from = sub.start;
    for seg in &sub.segs {
        let Seg::Cubic(a, b, to) = *seg else { return None };
        for i in 1..=8 {
            samples.push(cubic_point(from, a, b, to, f64::from(i) / 8.0));
        }
        from = to;
    }
    if samples.len() < 4 {
        return None;
    }
    let mut anchors = vec![sub.start];
    anchors.extend(sub.segs.iter().map(|s| s.end()));
    if sub.closed && anchors.last().is_some_and(|p| p.distance(sub.start) <= 1e-9) {
        anchors.pop();
    }
    let (p0, p1, p2) = if anchors.len() >= 3 {
        (anchors[0], anchors[anchors.len() / 3], anchors[anchors.len() * 2 / 3])
    } else {
        (samples[0], samples[samples.len() / 2], *samples.last()?)
    };
    let d = 2.0 * (p0.x * (p1.y - p2.y) + p1.x * (p2.y - p0.y) + p2.x * (p0.y - p1.y));
    let scale2 = p0.distance(p1).max(p1.distance(p2)).max(p2.distance(p0)).powi(2);
    if !d.is_finite() || d.abs() <= scale2 * 1e-9 {
        return None;
    }
    let q0 = p0.x * p0.x + p0.y * p0.y;
    let q1 = p1.x * p1.x + p1.y * p1.y;
    let q2 = p2.x * p2.x + p2.y * p2.y;
    let centre = Point::new(
        (q0 * (p1.y - p2.y) + q1 * (p2.y - p0.y) + q2 * (p0.y - p1.y)) / d,
        (q0 * (p2.x - p1.x) + q1 * (p0.x - p2.x) + q2 * (p1.x - p0.x)) / d,
    );
    let radius = centre.distance(p0);
    if !centre.x.is_finite() || !centre.y.is_finite() || !radius.is_finite() || radius <= 1e-9 {
        return None;
    }
    let tolerance = (radius * 5e-4).max(1e-6);
    if samples.iter().any(|p| (centre.distance(*p) - radius).abs() > tolerance) {
        return None;
    }
    Some(CircularSub { centre, radius, samples })
}

fn angle_deg(p: Point, centre: Point) -> f64 {
    (p.y - centre.y).atan2(p.x - centre.x).to_degrees().rem_euclid(360.0)
}

fn on_ccw_arc(start: f64, end: f64, angle: f64) -> bool {
    (angle - start).rem_euclid(360.0) <= (end - start).rem_euclid(360.0) + 1e-7
}

/// The subpaths of `bp` (quadratics raised to cubics).
fn subpaths(bp: &BezPath) -> Vec<Sub> {
    let mut out = vec![];
    let mut cur: Option<Sub> = None;
    let mut last = Point::ZERO;
    for el in bp.elements() {
        let seg = match *el {
            PathEl::MoveTo(p) => {
                out.extend(cur.replace(Sub { start: p, segs: vec![], closed: false }));
                last = p;
                continue;
            }
            PathEl::ClosePath => {
                if let Some(mut s) = cur.take() {
                    s.closed = true;
                    last = s.start;
                    out.push(s);
                }
                continue;
            }
            PathEl::LineTo(p) => Seg::Line(p),
            PathEl::QuadTo(q, p) => Seg::Cubic(last + (q - last) * (2.0 / 3.0), p + (q - p) * (2.0 / 3.0), p),
            PathEl::CurveTo(a, b, p) => Seg::Cubic(a, b, p),
        };
        last = seg.end();
        cur.get_or_insert_with(|| Sub { start: last, segs: vec![], closed: false }).segs.push(seg);
    }
    out.extend(cur);
    out
}

/// The most pixels of an image written (it is held decoded, as RGBA and encoded at once).
const MAX_IMAGE_PIXELS: u64 = 64 << 20;

/// The most entities (handles) a drawing is written with.
const MAX_ENTITIES: u32 = 10_000_000;

/// The largest x or y a path may have to be flattened, and the most points its polygons keep.
const MAX_COORD: f64 = 1e9;
const MAX_POINTS: usize = 4_000_000;

/// `bp` flattened into polygons (each closing implicitly), consecutive repeats dropped.
fn polygons(bp: &BezPath) -> Vec<Vec<Point>> {
    // Far outside any drawing, a curve flattens to millions of points at this tolerance.
    let b = bp.control_box();
    if !matches!(
        b.x0.abs().max(b.x1.abs()).max(b.y0.abs()).max(b.y1.abs()).partial_cmp(&MAX_COORD),
        Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
    ) {
        return vec![];
    }
    let mut out: Vec<Vec<Point>> = vec![];
    let mut cur: Vec<Point> = vec![];
    let mut points = 0usize;
    let mut push = |p: Point, cur: &mut Vec<Point>| {
        if points < MAX_POINTS && cur.last().is_none_or(|q| q.distance(p) > 1e-9) {
            points += 1;
            cur.push(p);
        }
    };
    kurbo::flatten(bp.iter(), FLATNESS, |el| match el {
        PathEl::MoveTo(p) => {
            out.push(std::mem::take(&mut cur));
            push(p, &mut cur);
        }
        PathEl::LineTo(p) => push(p, &mut cur),
        PathEl::ClosePath => out.push(std::mem::take(&mut cur)),
        _ => {}
    });
    out.push(cur);
    for poly in &mut out {
        if poly.len() > 1 && poly.first().zip(poly.last()).is_some_and(|(a, b)| a.distance(*b) <= 1e-9) {
            poly.pop();
        }
    }
    out.retain(|p| p.len() >= 2);
    out
}

/// A DXF symbol name (layer, text style) from `name`: from DXF 2000 up to 255 characters
/// without `<>/\":;?*|=`` ` ``; before, up to 31 letters, digits, `$`, `-` and `_`, in capitals.
fn symbol_name(name: &str, long: bool) -> String {
    let s: String = if long {
        let bad = |c: char| c.is_control() || "<>/\\\":;?*|=`".contains(c);
        name.trim().chars().map(|c| if bad(c) { '_' } else { c }).take(255).collect()
    } else {
        name.trim().chars().map(|c| if c.is_ascii_alphanumeric() || "$-_".contains(c) { c.to_ascii_uppercase() } else { '_' }).take(31).collect()
    };
    s.trim().to_string()
}

/// The names a DXF table holds (compared in any case), and the next suffix to try for each base,
/// so identical names don't each search the suffixes before.
#[derive(Default)]
struct Names {
    taken: std::collections::HashSet<String>,
    next: HashMap<String, usize>,
}

impl Names {
    /// Names that are there already.
    fn with(names: &[&str]) -> Self {
        Self { taken: names.iter().map(|n| n.to_lowercase()).collect(), next: HashMap::new() }
    }

    /// `base` made unique by `_2`, `_3`…, within `max` characters; the name is taken from now on.
    fn unique(&mut self, base: &str, max: usize) -> String {
        let base = if base.is_empty() { "Layer".to_string() } else { base.to_string() };
        if self.taken.insert(base.to_lowercase()) {
            return base;
        }
        let next = self.next.entry(base.to_lowercase()).or_insert(2);
        loop {
            let suffix = format!("_{next}");
            *next += 1;
            let keep: String = base.chars().take(max.saturating_sub(suffix.len())).collect();
            let name = format!("{keep}{suffix}");
            if self.taken.insert(name.to_lowercase()) {
                return name;
            }
        }
    }
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// The layer flags art inherits from the layers around it.
#[derive(Clone, Copy)]
struct LayerFlags {
    off: bool,
    locked: bool,
    plot: bool,
}

pub(crate) struct Scene<'a> {
    doc: &'a Document,
    /// The art boxes of `doc`'s symbols, which the walk measures symbol instances by when it
    /// crops to the region ([`SymbolBoxes::quick`]).
    symbols: SymbolBoxes<'a>,
    o: &'a DxfOptions,
    /// Document space → drawing units (y up, origin at the region's bottom-left).
    to_dxf: Affine,
    /// Drawing units per point.
    k: f64,
    /// The entities written so far.
    e: Codes,
    next: u32,
    layers: Vec<Layer>,
    layer_names: Names,
    /// The DXF layer art goes on, and its flags.
    layer: String,
    flags: LayerFlags,
    linetypes: Vec<LineType>,
    /// (style name, font family).
    styles: Vec<(String, String)>,
    style_names: Names,
    images: Vec<ImageDef>,
    files: Vec<DxfImage>,
    /// Image keys → index into `images` (`None`: left out).
    by_key: HashMap<String, Option<usize>>,
    warnings: Vec<String>,
    brushes: Option<Vec<vectorcraft_brush::Brush>>,
    extents: Option<[f64; 4]>,
    /// The opacity of the containers around the current object.
    opacity: f32,
    /// Symbols and brush art being written inside one another.
    nest: u32,
}

impl<'a> Scene<'a> {
    pub fn new(doc: &'a Document, o: &'a DxfOptions) -> Self {
        let k = o.scale / o.unit.points();
        let r = o.region;
        Self {
            doc,
            symbols: doc.symbol_boxes().quick(),
            o,
            to_dxf: Affine::new([k, 0.0, 0.0, -k, -r.x0 * k, r.y1 * k]),
            k,
            e: Codes::new(o.version),
            next: fixed::FIRST_FREE,
            layers: vec![],
            layer_names: Names::with(&["0", "Defpoints"]),
            layer: "0".into(),
            flags: LayerFlags { off: false, locked: false, plot: true },
            linetypes: vec![],
            styles: vec![],
            style_names: Names::with(&[crate::writer::STANDARD_STYLE]),
            images: vec![],
            files: vec![],
            by_key: HashMap::new(),
            warnings: vec![],
            brushes: None,
            extents: None,
            opacity: 1.0,
            nest: 0,
        }
    }

    pub fn run(mut self) -> DxfOutput {
        if self.o.colors == ColorDepth::True && !self.o.version.true_color() {
            self.warn(NO_TRUE_COLOR);
        }
        let doc = self.doc;
        for l in &doc.layers {
            self.node(l, false);
        }
        let r = self.o.region;
        let limits = [0.0, 0.0, r.width() * self.k, r.height() * self.k];
        let one = (self.o.scale - 1.0).abs() < 1e-12;
        let insunits = if one { crate::insunits_code(self.o.unit).unwrap_or(0) } else { 0 };
        let drawing = Drawing {
            version: self.o.version,
            layers: self.layers,
            linetypes: self.linetypes,
            styles: self.styles,
            images: self.images,
            entities: self.e.into_string(),
            extents: self.extents.unwrap_or(limits),
            limits,
            insunits,
            metric: matches!(self.o.unit, Unit::Millimeters | Unit::Centimeters | Unit::Meters),
            next: self.next,
        };
        DxfOutput { bytes: drawing.write().into_bytes(), images: self.files, warnings: self.warnings }
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn handle(&mut self) -> Handle {
        let h = Handle(self.next);
        self.next = self.next.saturating_add(1);
        h
    }

    /// A document point in drawing units, counted in the extents.
    fn pt(&mut self, p: Point) -> Point {
        let q = self.to_dxf * p;
        let e = self.extents.get_or_insert([q.x, q.y, q.x, q.y]);
        *e = [e[0].min(q.x), e[1].min(q.y), e[2].max(q.x), e[3].max(q.y)];
        q
    }

    fn node(&mut self, n: &Node, force: bool) {
        if let NodeKind::Layer { template: true, .. } = n.kind {
            return;
        }
        // Symbols that hold symbols are written in full for each instance: depth alone doesn't
        // bound how many entities that is.
        if self.next.saturating_sub(fixed::FIRST_FREE) > MAX_ENTITIES {
            self.warn("the drawing is too large for DXF: what follows 10 million entities was left out");
            return;
        }
        // Hidden layers are written, switched off.
        let layer = n.is_layer();
        if !force && !n.visible && !layer {
            return;
        }
        if self.o.crop && !layer {
            match n.visual_bounds_with(&self.symbols) {
                Some(b) if !overlaps(b, self.o.region) => return,
                None if !n.is_container() => return,
                _ => {}
            }
        }
        self.note_losses(n);
        let opacity = self.opacity;
        self.opacity *= n.opacity.clamp(0.0, 1.0);
        match &n.kind {
            NodeKind::Layer { children, printable, clip, .. } => {
                let outer = (self.layer.clone(), self.flags);
                self.flags =
                    LayerFlags { off: self.flags.off || !n.visible, locked: self.flags.locked || n.locked, plot: self.flags.plot && *printable };
                self.layer = self.add_layer(n);
                self.children(children, *clip);
                (self.layer, self.flags) = outer;
            }
            NodeKind::Group { children, clip } => self.children(children, *clip),
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
            | NodeKind::Intertwine { .. } => {
                let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                for c in g.children().into_iter().flatten() {
                    self.node(c, false);
                }
            }
        }
        self.opacity = opacity;
    }

    /// A container's children; a clipping container's first child is its clipping path, which
    /// paints its fills below the others and its strokes over them.
    fn children(&mut self, children: &[std::sync::Arc<Node>], clip: bool) {
        let Some((first, rest)) = children.split_first().filter(|_| clip) else {
            children.iter().for_each(|c| self.node(c, false));
            return;
        };
        self.warn(CLIPPING);
        let paint = first.clip_paint();
        if let Some(f) = &paint.fill {
            self.node(f, false);
        }
        rest.iter().for_each(|c| self.node(c, false));
        if let Some(s) = &paint.stroke {
            self.node(s, false);
        }
    }

    /// Warn about what DXF can't hold in `n`'s look.
    fn note_losses(&mut self, n: &Node) {
        if n.blend != vectorcraft_color::BlendMode::Normal {
            self.warn("blending modes are left out");
        }
        if n.extras().mask.as_ref().is_some_and(|m| !m.disabled) {
            self.warn("opacity masks are left out");
        }
        let raster = |fx: &[vectorcraft_doc::Effect]| fx.iter().any(|e| e.visible && vectorcraft_effects::is_raster(&e.id));
        if raster(&n.appearance.effects) || n.appearance.items.iter().any(|i| raster(i.effects())) {
            self.warn("raster effects (shadows, glows, blurs, feathering) are left out");
        }
    }

    /// A DXF layer for document layer `n`, with a name no other layer has.
    fn add_layer(&mut self, n: &Node) -> String {
        let long = self.o.version.r2000();
        let base = symbol_name(n.name.as_deref().unwrap_or("Layer"), long);
        let name = self.layer_names.unique(&base, if long { 255 } else { 31 });
        let aci = match n.kind {
            NodeKind::Layer { color, .. } => nearest_aci(color.rgb(), ColorDepth::Aci256),
            _ => 7,
        };
        let f = self.flags;
        self.layers.push(Layer { name: name.clone(), aci, off: f.off, locked: f.locked, plot: f.plot });
        name
    }

    /// One colour for a paint: a solid colour, the average of a gradient's stops, or a pattern's
    /// first solid fill.
    fn paint_rgb(&mut self, p: &Paint) -> Option<[u8; 3]> {
        let rgb = |c: &vectorcraft_color::Color| {
            let [r, g, b, _] = c.to_rgba8(1.0);
            [r, g, b]
        };
        match p {
            Paint::None => None,
            Paint::Solid { color, .. } => Some(rgb(color)),
            Paint::Gradient(g) => {
                self.warn("gradients are written as one colour, the average of their stops");
                let stops = &g.gradient.stops;
                let n = stops.len().max(1) as f64;
                let sum = stops.iter().map(|s| rgb(&s.color)).fold([0.0; 3], |a, c| [0, 1, 2].map(|i| a[i] + f64::from(c[i])));
                Some(sum.map(|v| (v / n).round().clamp(0.0, 255.0) as u8))
            }
            Paint::Pattern { pattern, .. } => {
                self.warn("pattern fills are written as one colour");
                let first = self.doc.pattern(pattern).and_then(|def| def.art.iter().find_map(|a| first_solid(a)));
                Some(first.map_or([128; 3], |c| rgb(&c)))
            }
        }
    }

    /// The entity style of colour `rgb` at `opacity` (times the containers'), with an optional
    /// stroke weight (points) and linetype.
    fn style(&mut self, rgb: [u8; 3], opacity: f32, weight: Option<f64>, linetype: Option<String>) -> EntityStyle {
        let v = self.o.version;
        let opacity = (self.opacity * opacity).clamp(0.0, 1.0);
        let alpha = if opacity >= 0.998 {
            None
        } else if v.true_color() {
            Some((opacity * 255.0).round() as u8)
        } else {
            self.warn(NO_TRANSPARENCY);
            None
        };
        EntityStyle {
            aci: nearest_aci(rgb, self.o.colors),
            rgb: (self.o.colors == ColorDepth::True).then(|| u32::from_be_bytes([0, rgb[0], rgb[1], rgb[2]])),
            lineweight: weight.filter(|_| v.r2000()).map(|w| self.lineweight(w)),
            linetype,
            alpha,
        }
    }

    /// The DXF lineweight nearest to `width` points (scaled with the drawing when asked).
    fn lineweight(&mut self, width: f64) -> i32 {
        let scale = if self.o.scale_lineweights { self.o.scale } else { 1.0 };
        let hundredths = width * 2540.0 / 72.0 * scale;
        let max = LINEWEIGHTS[LINEWEIGHTS.len() - 1];
        if hundredths > f64::from(max) + 5.0 {
            self.warn(WIDE_LINES);
        }
        LINEWEIGHTS.into_iter().min_by(|a, b| (f64::from(*a) - hundredths).abs().total_cmp(&(f64::from(*b) - hundredths).abs())).unwrap_or(max)
    }

    /// A linetype for dash pattern `d` (one per distinct pattern).
    fn linetype(&mut self, d: &Dash) -> String {
        let mut lengths: Vec<f64> = d.pattern.iter().map(|v| v * self.k).collect();
        if lengths.len() % 2 == 1 {
            lengths.extend(lengths.clone());
        }
        let pattern: Vec<f64> = lengths.iter().enumerate().map(|(i, v)| if i % 2 == 0 { *v } else { -v }).collect();
        let same = |l: &LineType| l.pattern.len() == pattern.len() && l.pattern.iter().zip(&pattern).all(|(a, b)| (a - b).abs() < 1e-9);
        if let Some(l) = self.linetypes.iter().find(|l| same(l)) {
            return l.name.clone();
        }
        let name = format!("DASHED{}", self.linetypes.len() + 1);
        self.linetypes.push(LineType { name: name.clone(), pattern });
        name
    }

    /// A path's fills and strokes, in paint order.
    fn shape(&mut self, n: &Node, bp: &BezPath, rule: FillRule) {
        if bp.elements().is_empty() {
            return;
        }
        for item in &n.appearance.items {
            match item {
                AppearanceItem::Fill(f) if f.visible => {
                    if let Some(rgb) = self.paint_rgb(&f.paint) {
                        let style = self.style(rgb, f.opacity, None, None);
                        self.area(bp, rule, &style, false);
                    }
                }
                AppearanceItem::Stroke(st) if st.visible && st.width > 0.0 && st.width.is_finite() => self.stroke(bp, rule, st),
                _ => {}
            }
        }
    }

    fn stroke(&mut self, bp: &BezPath, rule: FillRule, st: &StrokeLayer) {
        let appearance = self.o.preserve == Preserve::Appearance;
        if appearance
            && st.brush.is_some()
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
        let Some(rgb) = self.paint_rgb(&st.paint) else { return };
        let plain = stroke::is_plain(st);
        if self.o.alter_paths || (appearance && !plain) {
            let region = stroke::outline_region(&PathData::from_bezpath(bp), rule, st);
            let style = self.style(rgb, st.opacity, None, None);
            self.area(&region.to_bezpath(), FillRule::NonZero, &style, true);
            return;
        }
        if !plain {
            self.warn(STROKE_EXTRAS);
        }
        let linetype = st.dash.as_ref().filter(|d| d.is_dashed()).map(|d| self.linetype(d));
        let style = self.style(rgb, st.opacity, Some(st.width), linetype);
        self.lines(bp, &style, st.width);
    }

    fn brush_art(&mut self, bp: &BezPath, st: &StrokeLayer) -> Option<Vec<Node>> {
        let doc = self.doc;
        let brushes = self.brushes.get_or_insert_with(|| vectorcraft_brush::library(doc));
        let b = st.brush.as_deref().and_then(|name| brushes.iter().find(|b| b.name == name))?;
        Some(vectorcraft_brush::stroke_pieces(doc, b, bp, st))
    }

    /// A filled area as its outlines followed by a solid hatch (R12: the outlines alone). Laser
    /// and cutter software reads the outlines and skips hatches. `clean`: the path has no
    /// overlaps, so the hatch's even-odd rule reads it as it is.
    fn area(&mut self, bp: &BezPath, rule: FillRule, style: &EntityStyle, clean: bool) {
        if !self.o.version.handles() {
            self.warn(NO_FILLS);
            self.lines(bp, style, 0.0);
            return;
        }
        let several = bp.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(_))).count() > 1;
        let normalized = (rule == FillRule::NonZero && several && !clean)
            .then(|| vectorcraft_pathops::try_normalize(&PathData::from_bezpath(bp), rule).ok())
            .flatten()
            .map(|p| p.to_bezpath());
        let outline = normalized.as_ref().unwrap_or(bp);
        let loops: Vec<Vec<Point>> = polygons(outline).into_iter().filter(|p| p.len() >= 3).collect();
        if loops.is_empty() {
            return;
        }
        self.lines(outline, style, 0.0);
        let h = self.handle();
        let layer = self.layer.clone();
        self.e.entity("HATCH", h, fixed::MODEL_RECORD, &layer, style, "AcDbHatch");
        self.e.xyz(10, 0.0, 0.0);
        self.e.normal();
        self.e.raw(2, "SOLID");
        self.e.int(70, 1);
        self.e.int(71, 0);
        self.e.int(91, loops.len() as i64);
        for l in &loops {
            // An external polyline boundary, closed, without bulges.
            self.e.int(92, 3);
            self.e.int(72, 0);
            self.e.int(73, 1);
            self.e.int(93, l.len() as i64);
            for p in l {
                let q = self.pt(*p);
                self.e.xy(10, q.x, q.y);
            }
            self.e.int(97, 0);
        }
        self.e.int(75, 0);
        self.e.int(76, 1);
        self.e.int(98, 0);
    }

    /// Path `bp` as lines: polylines for straight subpaths (and for curves in R12, flattened),
    /// cubic splines for curved ones. `width`: the stroke weight, which DXF before 2000 gives
    /// polylines as a width.
    fn lines(&mut self, bp: &BezPath, style: &EntityStyle, width: f64) {
        let v = self.o.version;
        let width = (!v.r2000() && width > 0.0).then_some(width * self.k);
        for sub in subpaths(bp) {
            if sub.segs.is_empty() {
                continue;
            }
            if sub.segs.iter().all(|s| matches!(s, Seg::Line(_))) {
                let mut pts: Vec<Point> = std::iter::once(sub.start).chain(sub.segs.iter().map(|s| s.end())).collect();
                if sub.closed && pts.len() > 2 && pts.last().is_some_and(|p| p.distance(sub.start) <= 1e-9) {
                    pts.pop();
                }
                self.polyline(&pts, sub.closed, style, width);
            } else if (v.handles() || width.is_none()) && self.circular(&sub, style) {
                // Native circles/arcs are preferred by CAD/CAM software. In R12, however, a
                // stroked curve must stay on the flattened POLYLINE path so groups 40/41 retain
                // its width. Non-circular curves continue through the existing fallback.
            } else if v.handles() {
                self.spline(&sub, style);
            } else {
                let mut path = BezPath::new();
                path.move_to(sub.start);
                for s in &sub.segs {
                    match *s {
                        Seg::Line(p) => path.line_to(p),
                        Seg::Cubic(a, b, p) => path.curve_to(a, b, p),
                    }
                }
                for pts in polygons(&path) {
                    self.polyline(&pts, sub.closed, style, width);
                }
            }
        }
    }

    /// A polyline through `pts` (document space): lightweight from R14, else a polyline with a
    /// vertex entity per point.
    fn polyline(&mut self, pts: &[Point], closed: bool, style: &EntityStyle, width: Option<f64>) {
        if pts.len() < 2 {
            return;
        }
        let pts: Vec<Point> = pts.iter().map(|p| self.pt(*p)).collect();
        let layer = self.layer.clone();
        let h = self.handle();
        let flags = i64::from(closed);
        if self.o.version.lwpolyline() {
            self.e.entity("LWPOLYLINE", h, fixed::MODEL_RECORD, &layer, style, "AcDbPolyline");
            self.e.int(90, pts.len() as i64);
            self.e.int(70, flags);
            if let Some(w) = width {
                self.e.num(43, w);
            }
            for p in &pts {
                self.e.xy(10, p.x, p.y);
            }
            return;
        }
        self.e.entity("POLYLINE", h, fixed::MODEL_RECORD, &layer, style, "AcDb2dPolyline");
        self.e.int(66, 1);
        self.e.xyz(10, 0.0, 0.0);
        self.e.int(70, flags);
        if let Some(w) = width {
            self.e.num(40, w);
            self.e.num(41, w);
        }
        for p in &pts {
            let vh = self.handle();
            self.e.entity("VERTEX", vh, h, &layer, style, "AcDbVertex");
            if self.o.version.handles() {
                self.e.raw(100, "AcDb2dVertex");
            }
            self.e.xyz(10, p.x, p.y);
        }
        let end = self.handle();
        self.e.entity("SEQEND", end, h, &layer, style, "");
    }

    /// Write a safely recognized circular subpath as a native DXF CIRCLE or ARC. Returns false
    /// when the curve is not circular enough, leaving the caller to write it as a spline/polyline.
    fn circular(&mut self, sub: &Sub, style: &EntityStyle) -> bool {
        let Some(last) = sub.segs.last().map(|s| s.end()) else { return false };
        let geometrically_closed = last.distance(sub.start) <= 1e-9;
        // ClosePath can add an implicit straight closing edge. Do not replace that geometry with
        // a full CIRCLE; likewise, an open curve that returns to its start is not an ARC. Check
        // before fitting, updating extents or allocating an entity handle.
        if sub.closed != geometrically_closed {
            return false;
        }
        let Some(circle) = circular_subpath(sub) else { return false };
        let centre = self.to_dxf * circle.centre;
        let radius = circle.radius * self.k.abs();
        if !centre.x.is_finite() || !centre.y.is_finite() || !radius.is_finite() || radius <= 1e-12 {
            return false;
        }

        // Count sampled points in the drawing extents. For a full circle include all cardinal
        // points exactly; for an arc, the dense samples conservatively cover its visible span.
        if sub.closed {
            let r = circle.radius;
            for p in [
                Point::new(circle.centre.x - r, circle.centre.y),
                Point::new(circle.centre.x + r, circle.centre.y),
                Point::new(circle.centre.x, circle.centre.y - r),
                Point::new(circle.centre.x, circle.centre.y + r),
            ] {
                self.pt(p);
            }
        } else {
            for p in &circle.samples {
                self.pt(*p);
            }
        }

        let layer = self.layer.clone();
        let h = self.handle();
        if sub.closed {
            self.e.entity("CIRCLE", h, fixed::MODEL_RECORD, &layer, style, "AcDbCircle");
            self.e.xyz(10, centre.x, centre.y);
            self.e.num(40, radius);
            self.e.normal();
            return true;
        }

        let start = self.to_dxf * sub.start;
        let end = self.to_dxf * last;
        let middle = self.to_dxf * circle.samples[circle.samples.len() / 2];
        let mut a0 = angle_deg(start, centre);
        let mut a1 = angle_deg(end, centre);
        let am = angle_deg(middle, centre);
        if !on_ccw_arc(a0, a1, am) {
            std::mem::swap(&mut a0, &mut a1);
        }
        self.e.entity("ARC", h, fixed::MODEL_RECORD, &layer, style, "AcDbCircle");
        self.e.xyz(10, centre.x, centre.y);
        self.e.num(40, radius);
        self.e.normal();
        if self.o.version.handles() {
            self.e.raw(100, "AcDbArc");
        }
        self.e.num(50, a0);
        self.e.num(51, a1);
        true
    }

    /// A curved subpath as one cubic B-spline whose clamped knots repeat at every joint, so it
    /// passes through every anchor (straight segments become straight cubics).
    fn spline(&mut self, sub: &Sub, style: &EntityStyle) {
        let end = sub.segs.last().map_or(sub.start, |s| s.end());
        let closing = (sub.closed && end.distance(sub.start) > 1e-9).then_some(Seg::Line(sub.start));
        let mut ctrl = vec![sub.start];
        let mut last = sub.start;
        for seg in sub.segs.iter().copied().chain(closing) {
            match seg {
                Seg::Line(p) => ctrl.extend([last.lerp(p, 1.0 / 3.0), last.lerp(p, 2.0 / 3.0), p]),
                Seg::Cubic(a, b, p) => ctrl.extend([a, b, p]),
            }
            last = seg.end();
        }
        let n = (ctrl.len() - 1) / 3;
        let mut knots = vec![0.0; 4];
        for i in 1..n {
            knots.extend([i as f64; 3]);
        }
        knots.extend([n as f64; 4]);
        let ctrl: Vec<Point> = ctrl.iter().map(|p| self.pt(*p)).collect();
        let layer = self.layer.clone();
        let h = self.handle();
        self.e.entity("SPLINE", h, fixed::MODEL_RECORD, &layer, style, "AcDbSpline");
        self.e.normal();
        // Planar (bit 3) and, if the subpath is closed, also closed (bit 0). A geometrically
        // closed cut needs the closed flag set so CAM/CAM tools (e.g. VCarve) treat it as a
        // pocket/profile rather than asking the operator to join endpoints. (#1299)
        self.e.int(70, if sub.closed { 8 | 1 } else { 8 });
        self.e.int(71, 3);
        self.e.int(72, knots.len() as i64);
        self.e.int(73, ctrl.len() as i64);
        self.e.int(74, 0);
        for k in knots {
            self.e.num(40, k);
        }
        for p in ctrl {
            self.e.xyz(10, p.x, p.y);
        }
    }

    /// Type: glyph outlines (Preserve Appearance, Outline Text, type on a path), else one text
    /// entity per run of each line.
    fn text(&mut self, n: &Node, t: &TextObject) {
        let outline = self.o.preserve == Preserve::Appearance || self.o.outline_text || matches!(t.kind, TextKind::OnPath { .. });
        if outline {
            if let Some(o) = vectorcraft_effects::outline_text(n) {
                self.node(&o, true);
            }
            return;
        }
        if !n.appearance.items.is_empty() {
            self.warn(TYPE_ITEMS);
        }
        let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let plain = t.plain_text();
        let xf = self.to_dxf * t.xf;
        let [a, b, c, d, _, _] = xf.as_coeffs();
        // Along and across the baseline, in drawing units per text-space unit.
        let along = a.hypot(b);
        if along <= 1e-12 {
            return;
        }
        let across = (a * d - b * c).abs() / along;
        let angle = b.atan2(a).to_degrees();
        for line in &layout.lines {
            let Some(glyphs) = layout.glyphs.get(line.glyph_start..line.glyph_end) else { continue };
            // A list marker is a TEXT entity of its own, holding the marker's characters where its
            // characters' glyphs are drawn (not its tab's).
            let marked: Vec<(usize, &vectorcraft_text::PositionedGlyph, Option<&vectorcraft_text::ListMarker>)> =
                glyphs.iter().enumerate().map(|(k, g)| (line.glyph_start + k, g, layout.marker_of(line.glyph_start + k))).collect();
            for chunk in marked.chunk_by(|x, y| x.1.run == y.1.run && x.2.is_some() == y.2.is_some()) {
                let run: Vec<&vectorcraft_text::PositionedGlyph> =
                    chunk.iter().filter(|(k, _, m)| m.is_none_or(|m| m.tab != *k)).map(|(_, g, _)| *g).collect();
                let Some(&first) = run.first() else { continue };
                let lo = run.iter().map(|g| g.byte).min().unwrap_or(first.byte);
                let hi = run.iter().map(|g| g.byte + g.len).max().unwrap_or(lo);
                // A right-to-left paragraph's marker is drawn with its tab first and its number's
                // period at the number's left; the TEXT entity holds the characters in that order.
                let marker = chunk.first().and_then(|(_, _, m)| *m).map(|m| match m.text.strip_suffix('.') {
                    Some(number) if m.tab == m.glyphs.start => format!(".{number}"),
                    _ => m.text.clone(),
                });
                let Some(s) = marker.as_deref().or_else(|| plain.get(lo..hi).map(str::trim_end)).filter(|s| !s.trim().is_empty()) else { continue };
                let Some(cs) = t.runs.get(first.run).map(|r| &r.style) else { continue };
                let paint = if cs.fill.is_none() { &cs.stroke } else { &cs.fill };
                let Some(rgb) = self.paint_rgb(paint) else { continue };
                let style = self.style(rgb, 1.0, None, None);
                let text_style = self.text_style(&cs.font_family);
                // The height is that of the size the layout draws: Shrink Text to Fit scales the size by `layout.fit_scale`.
                let height = cs.size * layout.fit_scale * cs.v_scale / 100.0 * CAP_HEIGHT * across;
                let width = if cs.v_scale > 0.0 { cs.h_scale / cs.v_scale } else { 1.0 };
                let origin = run.iter().map(|g| g.origin).min_by(|p, q| p.x.total_cmp(&q.x)).unwrap_or(first.origin);
                let p = self.pt(t.xf * origin);
                let layer = self.layer.clone();
                let h = self.handle();
                self.e.entity("TEXT", h, fixed::MODEL_RECORD, &layer, &style, "AcDbText");
                self.e.xyz(10, p.x, p.y);
                self.e.num(40, height);
                let s: String = s.chars().take(MAX_TEXT).collect();
                self.e.str(1, &s);
                if angle.abs() > 1e-9 {
                    self.e.num(50, angle);
                }
                if (width - 1.0).abs() > 1e-9 {
                    self.e.num(41, width);
                }
                self.e.str(7, &text_style);
                if self.o.version.handles() {
                    self.e.raw(100, "AcDbText");
                }
            }
        }
    }

    /// The text style for font `family` (one per family).
    fn text_style(&mut self, family: &str) -> String {
        if let Some((name, _)) = self.styles.iter().find(|(_, f)| f == family) {
            return name.clone();
        }
        let long = self.o.version.r2000();
        let name = self.style_names.unique(&symbol_name(family, long), if long { 255 } else { 31 });
        self.styles.push((name.clone(), family.to_string()));
        name
    }

    /// A placed image, linked to its file (written once per image).
    fn image(&mut self, im: &ImageObject) {
        if !self.o.version.handles() {
            self.warn(NO_IMAGES);
            return;
        }
        let Some(i) = self.image_def(im) else { return };
        let Some((dw, dh)) = self.images.get(i).map(|d| (f64::from(d.size.0), f64::from(d.size.1))) else { return };
        let (w, h) = (f64::from(im.width.max(1)), f64::from(im.height.max(1)));
        let map = self.to_dxf * im.xf;
        let lin = |v: kurbo::Vec2| (map * v.to_point()) - (map * Point::ZERO);
        // Insertion at the bottom-left corner; u along one file pixel, v one pixel up.
        let u = lin(kurbo::Vec2::new(w / dw, 0.0));
        let v = lin(kurbo::Vec2::new(0.0, -h / dh));
        for corner in [Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(0.0, h), Point::new(w, h)] {
            self.pt(im.xf * corner);
        }
        let at = map * Point::new(0.0, h);
        let handle = self.handle();
        let reactor = self.handle();
        let Some(def) = self.images.get_mut(i) else { return };
        def.reactors.push((reactor, handle));
        let def = def.handle;
        let layer = self.layer.clone();
        let style = EntityStyle { aci: 7, ..EntityStyle::default() };
        self.e.entity("IMAGE", handle, fixed::MODEL_RECORD, &layer, &style, "AcDbRasterImage");
        self.e.int(90, 0);
        self.e.xyz(10, at.x, at.y);
        self.e.xyz(11, u.x, u.y);
        self.e.xyz(12, v.x, v.y);
        self.e.xy(13, dw, dh);
        self.e.handle(340, def);
        // Shown, also when not aligned with the screen; PNG with its transparency.
        self.e.int(70, if self.o.raster == RasterFormat::Png { 11 } else { 3 });
        self.e.int(280, 0);
        self.e.int(281, 50);
        self.e.int(282, 50);
        self.e.int(283, 0);
        self.e.handle(360, reactor);
        self.e.int(71, 1);
        self.e.int(91, 2);
        self.e.xy(14, -0.5, -0.5);
        self.e.xy(14, dw - 0.5, dh - 0.5);
        if self.o.version >= crate::DxfVersion::R2010 {
            self.e.int(290, 0);
        }
    }

    /// The image file for image `im`, encoded once in the chosen format (`None`: left out).
    fn image_def(&mut self, im: &ImageObject) -> Option<usize> {
        let key = im.key.as_str();
        if let Some(i) = self.by_key.get(key) {
            return *i;
        }
        let made = self.encode_image(im);
        let i = match made {
            Ok((file, size)) => {
                let handle = self.handle();
                self.images.push(ImageDef { name: file.name.clone(), size, handle, reactors: vec![] });
                self.files.push(file);
                Some(self.images.len() - 1)
            }
            Err(e) => {
                self.warn(&e);
                None
            }
        };
        self.by_key.insert(key.to_string(), i);
        i
    }

    /// Image `im` as a file in the chosen format, and its size in pixels.
    fn encode_image(&self, im: &ImageObject) -> Result<(DxfImage, (u32, u32)), String> {
        let blob = self.doc.image_blob(im).filter(|b| !b.bytes.is_empty()).ok_or_else(|| "an image has no pixels and was left out".to_string())?;
        // The pixels it converts to (RGBA, 4 bytes each) are read from its header before decoding.
        let (w, h) = image::ImageReader::new(Cursor::new(blob.bytes.as_slice()))
            .with_guessed_format()
            .ok()
            .and_then(|r| r.into_dimensions().ok())
            .ok_or_else(|| UNDECODED.to_string())?;
        if u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
            return Err("an image is too large for DXF and was left out".to_string());
        }
        let img = image::load_from_memory(&blob.bytes).map_err(|_| UNDECODED.to_string())?;
        let size = (img.width(), img.height());
        let mut bytes = vec![];
        let written = match self.o.raster {
            RasterFormat::Png => img.to_rgba8().write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png),
            RasterFormat::Jpeg => {
                // No alpha in JPEG: transparent pixels over white.
                let mut rgba = img.to_rgba8();
                for p in rgba.pixels_mut() {
                    let a = u32::from(p[3]);
                    for c in 0..3 {
                        p[c] = ((u32::from(p[c]) * a + 255 * (255 - a)) / 255) as u8;
                    }
                }
                let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
                rgb.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90))
            }
        };
        written.map_err(|e| format!("an image could not be written: {e}"))?;
        Ok((DxfImage { name: format!("{}.{}", blob.content_key(), self.o.raster.extension()), bytes }, size))
    }
}

/// The first visible solid fill colour in `n` (pattern art).
fn first_solid(n: &Node) -> Option<vectorcraft_color::Color> {
    let own = n.appearance.items.iter().find_map(|i| match i {
        AppearanceItem::Fill(f) if f.visible => f.paint.color(),
        _ => None,
    });
    own.or_else(|| n.children().into_iter().flatten().find_map(|c| first_solid(c)))
}
