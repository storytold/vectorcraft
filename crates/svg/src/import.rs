//! usvg tree → document conversion.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

use usvg::roxmltree;
use vectorcraft_color::{BlendMode, Color, Gradient, GradientGeom, GradientInterpolation, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{
    Appearance, AppearanceItem, Dash, Document, FillLayer, ImageBlob, ImageObject, LayerColor, LineCap, LineJoin, Node, NodeKind, PatternDef,
    StrokeLayer, Unit,
};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, Vec2, shapes};

use crate::export::Reuse;
use crate::{ImportOptions, SvgError, fnv1a};
use css::XNode;
use text::TextSlots;

pub(crate) use fx::BLEND;

mod css;
mod files;
mod fx;
mod spread;
mod text;
mod vector_effect;

pub(crate) fn import(svg: &str, opts: &ImportOptions) -> Result<(Document, Vec<String>), SvgError> {
    let (svg, found) = prepass(svg, opts);
    let svg = svg.as_ref();
    let xml = roxmltree::Document::parse_with_options(svg, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() })
        .map_err(|e| SvgError::Parse(e.to_string()))?;
    let units = RootUnits::of(xml.root_element());
    let mut warnings = found.warnings;
    let (src, slots) = text::prepare(svg, &xml, units.dpi, &mut warnings);
    let files = found.files;
    let tree = {
        let opt = usvg::Options {
            dpi: units.dpi as f32,
            font_size: DEFAULT_FONT_SIZE as f32,
            image_href_resolver: files.resolver(),
            ..usvg::Options::default()
        };
        usvg::Tree::from_str(&src, &opt).map_err(|e| SvgError::Parse(e.to_string()))?
    };
    let size = tree.size();
    let (kx, ky) = units.k;
    let mut doc = Document::new(size.width() as f64 * kx, size.height() as f64 * ky);
    doc.units = units.unit;
    let mut im = Importer {
        doc,
        warnings,
        mask_flags: mask_flags(&xml),
        links: found.links,
        slots,
        patterns: HashMap::new(),
        midpoints: midpoint_stops(&xml),
        labels: labels(&xml),
        hidden: found.hidden,
        uses: found.uses,
        symbols: HashMap::new(),
        blends: fx::blends(&xml),
        files,
        non_scaling: found.non_scaling,
        screen: (kx * ky).sqrt(),
    };
    if im.files.nested_text() {
        im.warn("text inside an SVG image isn't imported".into());
    }

    // usvg wraps everything in an id-less group carrying the viewBox transform when needed.
    let mut top = tree.root();
    let mut base = Affine::scale_non_uniform(kx, ky) * aff(top.transform());
    if let [usvg::Node::Group(g)] = top.children()
        && g.id().is_empty()
        && is_plain(g)
        && im.text_slot(g).is_none()
    {
        base *= aff(g.transform());
        top = g;
    }

    // Top-level `<g id>` elements become layers (as in the reference app); otherwise all art goes
    // into "Layer 1". Top-level text joins the layer below it (the first layer if none is).
    let is_layer = |im: &Importer, c: &usvg::Node| matches!(c, usvg::Node::Group(g) if !g.id().is_empty() && !made_up(g.id()) && !im.links.contains_key(g.id()) && !im.uses.contains_key(g.id()) && is_plain(g) && im.text_slot(g).is_none());
    let is_text = |im: &Importer, c: &usvg::Node| matches!(c, usvg::Node::Group(g) if im.text_slot(g).is_some());
    let layer_mode = top.children().iter().any(|c| is_layer(&im, c)) && top.children().iter().all(|c| is_layer(&im, c) || is_text(&im, c));
    if layer_mode {
        im.doc.layers.clear();
        let mut loose = vec![];
        for c in top.children() {
            let usvg::Node::Group(g) = c else { continue };
            if !is_layer(&im, c) {
                let n = im.node(c, base).map(Arc::new);
                match im.doc.layers.last_mut().and_then(|l| Arc::make_mut(l).children_mut()) {
                    Some(ch) => ch.extend(n),
                    None => loose.extend(n),
                }
                continue;
            }
            let mut children = std::mem::take(&mut loose);
            children.extend(im.children(g, base * aff(g.transform())));
            let id = im.doc.alloc_id();
            let preset = (im.doc.layers.len() % vectorcraft_doc::LAYER_COLORS.len()) as u8;
            let name = im.name_of(g.id()).to_string();
            let mut l = Node::layer(id, &name, LayerColor::Preset(preset));
            if let Some(ch) = l.children_mut() {
                *ch = children;
            }
            l.visible = !im.hidden.contains(g.id());
            im.doc.layers.push(Arc::new(l));
        }
    } else {
        let children = im.children(top, base);
        if let Some(l) = im.doc.layers.first_mut()
            && let Some(ch) = Arc::make_mut(l).children_mut()
        {
            *ch = children;
        }
    }
    Ok((im.doc, im.warnings))
}

/// SVG's initial `font-size` (`medium`) in user units.
pub(super) const DEFAULT_FONT_SIZE: f64 = 12.0;

/// How lengths become points, from the root `<svg>`'s `width` / `height`.
///
/// A pixel (and a unitless user unit) is a point, as we export, and absolute lengths keep their
/// physical size at 72 pt per inch: `font-size="12pt"` is 12 pt, `1in` is 72 pt. A root size in
/// absolute units (`width="210mm"`) is that physical size, and its user units are CSS pixels of it
/// (96 per inch, 0.75 pt each), so a drawing without a `viewBox` keeps its proportions.
struct RootUnits {
    /// Pixels per inch usvg (and the text pass) convert absolute lengths at.
    dpi: f64,
    /// Points per user unit of the root along x and y.
    k: (f64, f64),
    /// Document units: those of the root `width` (pixels when it has none).
    unit: Unit,
}

impl RootUnits {
    fn of(root: XNode) -> Self {
        let unit = |name: &str| root.attribute(name).and_then(|v| svgtypes::Length::from_str(v.trim()).ok()).map(|l| l.unit);
        use svgtypes::LengthUnit as U;
        let physical = |u: Option<U>| matches!(u, Some(U::In | U::Cm | U::Mm | U::Pt | U::Pc));
        let (w, h) = (unit("width"), unit("height"));
        let k = |u| if physical(u) { PT_PER_IN / CSS_PX_PER_IN } else { 1.0 };
        let dpi = if physical(w) || physical(h) { CSS_PX_PER_IN } else { PT_PER_IN };
        let unit = match w {
            Some(U::Mm) => Unit::Millimeters,
            Some(U::Cm) => Unit::Centimeters,
            Some(U::In) => Unit::Inches,
            Some(U::Pt) => Unit::Points,
            Some(U::Pc) => Unit::Picas,
            _ => Unit::Pixels,
        };
        Self { dpi, k: (k(w), k(h)), unit }
    }
}

/// How many clip paths deep (`<clipPath clip-path>`) an object's clip is read.
const MAX_CLIP_NEST: usize = 16;

/// Points per inch.
pub(super) const PT_PER_IN: f64 = 72.0;
/// CSS pixels per inch.
const CSS_PX_PER_IN: f64 = 96.0;

struct Importer {
    doc: Document,
    warnings: Vec<String>,
    /// Opacity-mask options our export wrote, by mask id (see [`mask_flags`]).
    mask_flags: HashMap<String, (bool, bool)>,
    /// The URL of each `<a href>`'s group, by its id ([`prepass`]).
    links: HashMap<String, String>,
    /// Texts read from the XML, by placeholder ([`text`]).
    slots: TextSlots,
    /// Pattern swatch made for each usvg pattern.
    patterns: HashMap<usize, String>,
    /// Midpoint and interpolation stops by gradient id (see [`midpoint_stops`]).
    midpoints: HashMap<String, Vec<StopMark>>,
    /// Object names by element id (see [`labels`]).
    labels: HashMap<String, String>,
    /// Ids of the undisplayed objects shown for usvg ([`prepass`]).
    hidden: HashSet<String>,
    /// The `<symbol>` id each `<use>` of a symbol shows, by the `<use>`'s id ([`prepass`]).
    uses: HashMap<String, String>,
    /// The symbols made so far, by `<symbol>` id.
    symbols: HashMap<String, SymbolMade>,
    /// Blend modes of the shadows and glows our export wrote, by filter id ([`fx::blends`]).
    blends: HashMap<String, String>,
    /// The files `<image>` elements link to.
    files: files::Files,
    /// Ids of the shapes whose strokes don't scale ([`vector_effect`]).
    non_scaling: HashSet<String>,
    /// Points per screen pixel (the root's user unit before its `viewBox`): what a non-scaling
    /// stroke's width is measured in.
    screen: f64,
}

/// A `<symbol>` made a symbol: its name, its art with no object ids (what each `<use>` has to show
/// to be an instance of it) and the instance transforms the art can stand for.
struct SymbolMade {
    name: String,
    art: Node,
    reuse: Reuse,
}

/// `n` with every object id (its descendants' and its mask art's too) zeroed, to compare art.
fn without_ids(n: &Node) -> Node {
    let mut n = n.clone();
    n.id = vectorcraft_doc::NodeId(0);
    if let Some(m) = n.mask.as_deref_mut() {
        m.art = Arc::new(without_ids(&m.art));
    }
    for c in n.children_mut().into_iter().flatten() {
        *c = Arc::new(without_ids(c));
    }
    n
}

/// `base`, or `base 2`, `base 3`… : the first that isn't `taken`.
fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut name = base.to_string();
    let mut k = 2;
    while taken(&name) {
        name = format!("{base} {k}");
        k += 1;
    }
    name
}

/// What our export marked a gradient stop as.
#[derive(Clone, Copy, Debug, PartialEq)]
enum StopMark {
    /// A stop of the gradient itself.
    Own,
    /// The colour at the previous stop's midpoint (`data-vc-midpoint`, the midpoint location).
    Midpoint(f32),
    /// A colour added to approximate perceptual interpolation (`data-vc-interpolated`).
    Interpolated,
}

/// The stops our export added for midpoints (`data-vc-midpoint`) and perceptual interpolation
/// (`data-vc-interpolated`), by gradient id: for each stop, what it stands for.
fn midpoint_stops(xml: &roxmltree::Document) -> HashMap<String, Vec<StopMark>> {
    xml.descendants()
        .filter(|n| matches!(n.tag_name().name(), "linearGradient" | "radialGradient"))
        .filter_map(|g| {
            let id = g.attribute("id")?;
            let marks: Vec<StopMark> = g
                .children()
                .filter(|c| c.tag_name().name() == "stop")
                .map(|c| match (c.attribute("data-vc-midpoint").and_then(|v| v.parse::<f32>().ok()), c.attribute("data-vc-interpolated")) {
                    (Some(m), _) if m.is_finite() => StopMark::Midpoint(m),
                    (_, Some(_)) => StopMark::Interpolated,
                    _ => StopMark::Own,
                })
                .collect();
            marks.iter().any(|m| *m != StopMark::Own).then(|| (id.to_string(), marks))
        })
        .collect()
}

/// A gradient's stops, with the midpoint stops `marks` names folded back into the midpoints of
/// the stops before them and the interpolation stops dropped. Also whether there were
/// interpolation stops (the gradient was perceptual).
fn gradient_stops(g: &usvg::BaseGradient, marks: Option<&[StopMark]>) -> (Vec<GradientStop>, bool) {
    let marks = marks.filter(|m| m.len() == g.stops().len());
    let mut out: Vec<GradientStop> = Vec::with_capacity(g.stops().len());
    let mut perceptual = false;
    for (i, s) in g.stops().iter().enumerate() {
        match (marks.and_then(|m| m.get(i).copied()), out.last_mut()) {
            (Some(StopMark::Midpoint(mid)), Some(prev)) => {
                prev.midpoint = mid.clamp(0.0, 1.0);
                continue;
            }
            (Some(StopMark::Interpolated), Some(_)) => {
                perceptual = true;
                continue;
            }
            _ => {}
        }
        let c = s.color();
        out.push(GradientStop { opacity: s.opacity().get(), ..GradientStop::new(s.offset().get(), Color::rgb8(c.red, c.green, c.blue)) });
    }
    (out, perceptual)
}

/// The options [`crate::export::MASK_FLAGS`] records on exported `<mask>` elements, by mask id:
/// (clip, invert).
fn mask_flags(xml: &roxmltree::Document) -> HashMap<String, (bool, bool)> {
    let attr = crate::export::MASK_FLAGS;
    if !xml.input_text().contains(attr) {
        return HashMap::new();
    }
    let flags = |n: roxmltree::Node| {
        let has = |f: &str| n.attribute(attr).is_some_and(|v| v.split_whitespace().any(|x| x == f));
        Some((n.attribute("id")?.to_string(), (!has("noclip"), has("invert"))))
    };
    xml.descendants().filter(|n| n.tag_name().name() == "mask" && n.has_attribute(attr)).filter_map(flags).collect()
}

/// Prefix of the ids [`prepass`] gives the elements it has to find again that have none.
const MADE_UP_ID: &str = "vectorcraft-id-";

/// Is `id` one [`prepass`] made up (not a name)?
fn made_up(id: &str) -> bool {
    id.starts_with(MADE_UP_ID)
}

/// The URL an `<a>` element links to.
pub(super) fn href<'a>(a: XNode<'a, '_>) -> Option<&'a str> {
    a.attribute("href").or_else(|| a.attribute(("http://www.w3.org/1999/xlink", "href"))).filter(|h| !h.is_empty())
}

/// What usvg would lose, found in the XML before it runs ([`prepass`]).
#[derive(Default)]
struct Found {
    /// The URL of each `<a href>`'s group, by its id.
    links: HashMap<String, String>,
    /// Ids of the undisplayed objects shown for usvg: they come back hidden.
    hidden: HashSet<String>,
    /// The `<symbol>` id each `<use>` of a symbol shows, by the `<use>`'s id.
    uses: HashMap<String, String>,
    /// The files `<image>` elements link to.
    files: files::Files,
    /// Ids of the shapes whose strokes don't scale ([`vector_effect`]).
    non_scaling: HashSet<String>,
    warnings: Vec<String>,
}

/// Changes to the SVG text: `(range, replacement)`, an empty range inserting.
#[derive(Default)]
pub(super) struct Edits {
    list: Vec<(std::ops::Range<usize>, String)>,
    /// The ids made up so far, by element.
    ids: HashMap<roxmltree::NodeId, String>,
}

impl Edits {
    fn insert(&mut self, at: usize, text: String) {
        self.list.push((at..at, text));
    }

    /// The id of element `n` (in `svg`), made up and written in when it has none.
    fn id(&mut self, svg: &str, n: XNode) -> String {
        if let Some(id) = n.attribute("id").filter(|id| !id.is_empty()) {
            return id.to_string();
        }
        if let Some(id) = self.ids.get(&n.id()) {
            return id.clone();
        }
        let id = format!("{MADE_UP_ID}{}", self.ids.len());
        self.set(svg, n, "id", &id);
        self.ids.insert(n.id(), id.clone());
        id
    }

    /// Set attribute `name` of element `n` (in `svg`) to `value` (XML-escaped already).
    pub(super) fn set(&mut self, svg: &str, n: XNode, name: &str, value: &str) {
        match n.attributes().find(|a| a.name() == name && a.namespace().is_none()) {
            Some(a) => self.list.push((a.range_value(), value.to_string())),
            None => self.insert(tag_name_end(svg, n), format!(" {name}=\"{value}\"")),
        }
    }

    /// `svg` with the edits made; overlapping ones and ranges off character boundaries are
    /// skipped.
    fn apply(mut self, svg: &str) -> Cow<'_, str> {
        if self.list.is_empty() {
            return svg.into();
        }
        // Stable: inserts at one place keep their order.
        self.list.sort_by_key(|(r, _)| (r.start, r.end));
        let mut out = String::with_capacity(svg.len() + self.list.iter().map(|(_, t)| t.len()).sum::<usize>());
        let mut last = 0;
        for (r, text) in self.list {
            let (Some(part), Some(_)) = (svg.get(last..r.start), svg.get(r.clone())) else { continue };
            out.push_str(part);
            out.push_str(&text);
            last = r.end;
        }
        out.push_str(svg.get(last..).unwrap_or(""));
        out.into()
    }
}

/// usvg reads `<a href>` and `<use>` as plain groups, drops what isn't displayed and reads linked
/// files on its own. Before it runs, the SVG is changed so that what we keep can be found:
/// * every `<a href>` and every `<use>` of a `<symbol>` gets an id when it has none
///   ([`MADE_UP_ID`]…), recorded with its URL or symbol;
/// * undisplayed objects (`display: none`, as Save writes hidden layers) are shown and recorded,
///   to come back hidden; one something links to (a `<use>` template) stays as it is;
/// * shapes with a non-scaling stroke get an id when they have none, recorded ([`vector_effect`]);
/// * linked files are read ([`files`]).
fn prepass<'s>(svg: &'s str, opts: &ImportOptions) -> (Cow<'s, str>, Found) {
    let mut found = Found::default();
    // (`:` for prefixed elements such as `<svg:image>`.)
    if !["<a", "display", "<use", ":use", "<image", ":image", vector_effect::NON_SCALING].iter().any(|t| svg.contains(t)) {
        return (svg.into(), found);
    }
    let Ok(xml) = roxmltree::Document::parse_with_options(svg, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }) else {
        return (svg.into(), found);
    };
    let mut edits = Edits::default();
    let elements = || xml.descendants().filter(|n| n.is_element());
    for a in elements().filter(|n| n.tag_name().name() == "a") {
        if let Some(url) = href(a) {
            found.links.insert(edits.id(svg, a), url.to_string());
        }
    }
    let symbols: HashSet<&str> = elements().filter(|n| n.tag_name().name() == "symbol").filter_map(|n| n.attribute("id")).collect();
    for u in elements().filter(|n| n.tag_name().name() == "use") {
        if let Some(sym) = href(u).and_then(|h| h.strip_prefix('#')).filter(|s| symbols.contains(s)) {
            found.uses.insert(edits.id(svg, u), sym.to_string());
        }
    }
    let (display, non_scaling) = (svg.contains("display"), svg.contains(vector_effect::NON_SCALING));
    if display || non_scaling {
        let css = css::Styles::new(&xml);
        if display {
            hidden_objects(svg, &xml, &css, &mut edits, &mut found.hidden);
        }
        if non_scaling {
            vector_effect::find(svg, &xml, &css, &mut edits, &mut found.non_scaling, &mut found.warnings);
        }
    }
    found.files = files::Files::read(svg, &xml, opts, &mut edits, &mut found.warnings);
    (edits.apply(svg), found)
}

/// The elements that are objects (what [`hidden_objects`] shows).
const OBJECTS: &[&str] = &["g", "a", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "image", "text", "use"];

/// Show the undisplayed objects outside `<defs>` (and the like) for usvg, recording their ids.
fn hidden_objects(svg: &str, xml: &roxmltree::Document, css: &css::Styles, edits: &mut Edits, ids: &mut HashSet<String>) {
    let linked: HashSet<&str> = xml.descendants().filter_map(href).filter_map(|h| h.strip_prefix('#')).collect();
    let mut todo: Vec<XNode> = xml.root_element().children().filter(XNode::is_element).collect();
    while let Some(n) = todo.pop() {
        let tag = n.tag_name().name();
        if !OBJECTS.contains(&tag) || n.attribute("id").is_some_and(|id| linked.contains(id)) {
            continue;
        }
        if matches!(tag, "g" | "a") {
            todo.extend(n.children().filter(XNode::is_element));
        }
        if css.own(n, "display").as_deref() != Some("none") {
            continue;
        }
        ids.insert(edits.id(svg, n));
        // A `style` declaration wins over the attribute and style sheet rules.
        match n.attributes().find(|a| a.name() == "style" && a.namespace().is_none()) {
            Some(a) => edits.insert(a.range_value().end, ";display:inline".to_string()),
            None => edits.insert(tag_name_end(svg, n), " style=\"display:inline\"".to_string()),
        }
    }
}

/// Where the start tag of element `n` ends its name (where an attribute can go).
fn tag_name_end(svg: &str, n: XNode) -> usize {
    let start = n.range().start;
    svg.get(start + 1..).and_then(|s| s.find(|c: char| c.is_whitespace() || c == '/' || c == '>')).map_or(start + 2, |i| start + 1 + i)
}

/// Make `n` link to `url`, unless it links somewhere already (an inner link wins).
fn link(n: &mut Node, url: &str) {
    if n.url().is_none() {
        n.edit_attrs(|a| a.url = url.to_string());
    }
}

/// The names apps keep beside element ids, by id: `data-name`, `inkscape:label`, `serif:id` or
/// `aria-label` (the first one present).
fn labels(xml: &roxmltree::Document) -> HashMap<String, String> {
    const INKSCAPE: &str = "http://www.inkscape.org/namespaces/inkscape";
    const SERIF: &str = "http://www.serif.com/";
    xml.descendants()
        .filter_map(|n| {
            let id = n.attribute("id").filter(|id| !id.is_empty())?;
            let name = n
                .attribute("data-name")
                .or_else(|| n.attribute((INKSCAPE, "label")))
                .or_else(|| n.attribute((SERIF, "id")))
                .or_else(|| n.attribute("aria-label"))?;
            Some((id.to_string(), name.to_string()))
        })
        .collect()
}

fn aff(t: usvg::Transform) -> Affine {
    Affine::new([t.sx as f64, t.ky as f64, t.kx as f64, t.sy as f64, t.tx as f64, t.ty as f64])
}

fn is_plain(g: &usvg::Group) -> bool {
    g.clip_path().is_none() && g.mask().is_none() && g.filters().is_empty() && g.opacity().get() >= 1.0 && g.blend_mode() == usvg::BlendMode::Normal
}

fn blend(b: usvg::BlendMode) -> BlendMode {
    use usvg::BlendMode as B;
    match b {
        B::Normal => BlendMode::Normal,
        B::Multiply => BlendMode::Multiply,
        B::Screen => BlendMode::Screen,
        B::Overlay => BlendMode::Overlay,
        B::Darken => BlendMode::Darken,
        B::Lighten => BlendMode::Lighten,
        B::ColorDodge => BlendMode::ColorDodge,
        B::ColorBurn => BlendMode::ColorBurn,
        B::HardLight => BlendMode::HardLight,
        B::SoftLight => BlendMode::SoftLight,
        B::Difference => BlendMode::Difference,
        B::Exclusion => BlendMode::Exclusion,
        B::Hue => BlendMode::Hue,
        B::Saturation => BlendMode::Saturation,
        B::Color => BlendMode::Color,
        B::Luminosity => BlendMode::Luminosity,
    }
}

fn bezpath(p: &usvg::tiny_skia_path::Path, m: Affine) -> BezPath {
    use usvg::tiny_skia_path::PathSegment as S;
    let pt = |p: usvg::tiny_skia_path::Point| m * Point::new(p.x as f64, p.y as f64);
    let mut bp = BezPath::new();
    for s in p.segments() {
        match s {
            S::MoveTo(p) => bp.move_to(pt(p)),
            S::LineTo(p) => bp.line_to(pt(p)),
            S::QuadTo(a, p) => bp.quad_to(pt(a), pt(p)),
            S::CubicTo(a, b, p) => bp.curve_to(pt(a), pt(b), pt(p)),
            S::Close => bp.close_path(),
        }
    }
    bp
}

/// The shapes of clip path `cp` on an element whose user space `ts` maps to the document.
fn clip_shapes(cp: &usvg::ClipPath, ts: Affine) -> Vec<(BezPath, FillRule)> {
    fn collect(g: &usvg::Group, m: Affine, out: &mut Vec<(BezPath, FillRule)>) {
        for c in g.children() {
            match c {
                usvg::Node::Group(g) => collect(g, m * aff(g.transform()), out),
                usvg::Node::Path(p) => out.push((bezpath(p.data(), m), p.fill().map(|f| rule(f.rule())).unwrap_or_default())),
                _ => {}
            }
        }
    }
    let mut shapes = Vec::new();
    collect(cp.root(), ts * aff(cp.transform()), &mut shapes);
    shapes
}

/// Does clip path `cp` (on an element whose user space `ts` maps to the document) leave `art`,
/// placed by `xf`, whole? (Only its bounds are checked: the box must be inside a clip shape.)
fn uncut(cp: &usvg::ClipPath, ts: Affine, art: &Node, xf: Affine) -> bool {
    let Some(b) = art.visual_bounds() else { return false };
    if cp.clip_path().is_some() {
        return false;
    }
    // Art touching the clip's edge isn't cut by it.
    let e = 1e-6 * (b.width() + b.height()).max(1e-9);
    let b = b.inflate(-e, -e);
    let shapes = clip_shapes(cp, ts);
    let corners = [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)];
    corners.iter().all(|c| shapes.iter().any(|(s, _)| kurbo::Shape::contains(s, xf * *c)))
}

fn rule(r: usvg::FillRule) -> FillRule {
    match r {
        usvg::FillRule::NonZero => FillRule::NonZero,
        usvg::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}

impl Importer {
    fn warn(&mut self, s: String) {
        if !self.warnings.contains(&s) {
            self.warnings.push(s);
        }
    }

    fn children(&mut self, g: &usvg::Group, acc: Affine) -> Vec<Arc<Node>> {
        g.children().iter().filter_map(|c| self.node(c, acc)).map(Arc::new).collect()
    }

    fn node(&mut self, n: &usvg::Node, acc: Affine) -> Option<Node> {
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
        Some(out)
    }

    /// A gradient's stops, midpoints restored, and whether it was perceptual.
    fn stops(&self, g: &usvg::BaseGradient) -> (Vec<GradientStop>, bool) {
        gradient_stops(g, self.midpoints.get(g.id()).map(Vec::as_slice))
    }

    /// The object name for an element id: its label (see [`labels`]), else the id itself.
    fn name_of<'a>(&'a self, id: &'a str) -> &'a str {
        self.labels.get(id).map_or(id, String::as_str)
    }

    /// A new object named after element `id` (unless it has none or a made-up one).
    fn named(&mut self, id: &str, kind: NodeKind) -> Node {
        let nid = self.doc.alloc_id();
        let mut n = Node::new(nid, kind);
        if !id.is_empty() && !made_up(id) {
            n.name = Some(self.name_of(id).to_string());
        }
        n
    }

    fn group(&mut self, g: &usvg::Group, acc: Affine) -> Option<Node> {
        let url = self.links.get(g.id()).cloned();
        let mut n = match self.instance(g, acc) {
            Some(n) => n,
            None => self.group_node(g, acc)?,
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
            _ => self.named("", NodeKind::Group { children: kids, clip: false }),
        };
        // usvg clips a `<use>` of a symbol to its viewport: that only counts when it cuts the art.
        let whole = g.clip_path().is_none_or(|cp| uncut(cp, ts, &art, xf));
        let name = match self.symbols.get(&sym) {
            Some(made) if whole && made.art == without_ids(&art) && made.reuse.allows(xf) => made.name.clone(),
            Some(_) => return None,
            None => {
                let name = unique_name(self.name_of(&sym), |n| self.doc.symbols.iter().any(|s| s.name == n));
                let reuse = Reuse::of(&self.doc, &art);
                let made = SymbolMade { name: name.clone(), art: without_ids(&art), reuse };
                self.doc.symbols.push(vectorcraft_doc::Symbol { name: name.clone(), art: Arc::new(art) });
                self.symbols.insert(sym, made);
                if !whole || !reuse.allows(xf) {
                    return None;
                }
                name
            }
        };
        let label = if made_up(g.id()) { "a symbol instance".to_string() } else { format!("'{}'", g.id()) };
        let mask = g.mask().and_then(|m| self.opacity_mask(m, ts, &label));
        let mut n = self.named(g.id(), NodeKind::SymbolInstance { symbol: name, xf });
        n.opacity = g.opacity().get();
        n.blend = blend(g.blend_mode());
        n.isolate = g.isolate();
        n.mask = mask;
        Some(n)
    }

    /// The text a placeholder group stands for (see [`text`]) and the index of its main path.
    fn text_slot(&self, g: &usvg::Group) -> Option<(usize, usize)> {
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
        // usvg resolved the `url(#…)` paints (the paths after the main one) in the element's user
        // space; runs paint in text space.
        let to_text = obj.xf.inverse();
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
        obj.xf = ts * obj.xf;
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
        let mut n = self.named(&name, NodeKind::Text(Box::new(obj)));
        if let Some(ap) = under {
            n.appearance = ap;
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
        // Blurs, shadows and glows become live effects (see [`fx`]).
        let effects = match g.filters() {
            [] => vec![],
            filters => fx::effects(filters, ts, &self.blends).unwrap_or_else(|| {
                self.warn(format!("filter on {label} ignored"));
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
                let mut inner = self.named("", NodeKind::Group { children, clip: false });
                inner.appearance.effects = effects.clone();
                children = vec![Arc::new(inner)];
            }
            // A clip path clipped in turn (`<clipPath clip-path>`): one clip group in another, so
            // the clips intersect.
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
                children = vec![Arc::new(self.named("", NodeKind::Group { children: ch, clip: true }))];
            }
            let mut ch = vec![Arc::new(outer)];
            ch.extend(children);
            self.named(id, NodeKind::Group { children: ch, clip: true })
        } else {
            if children.is_empty() {
                return None;
            }
            // An id-less wrapper around a single object (usvg adds these for opacity/transform on
            // shapes): fold its opacity, blend and effects into the object (unless the object
            // blends or masks before the effects). A text takes its own mask too.
            if (is_text || id.is_empty() && mask.is_none())
                && children.len() == 1
                && (g.blend_mode() == usvg::BlendMode::Normal || children[0].blend == BlendMode::Normal)
                && (effects.is_empty() || children[0].blend == BlendMode::Normal && children[0].mask.is_none())
                && let Some(only) = children.pop()
            {
                let mut c = Arc::unwrap_or_clone(only);
                c.opacity *= g.opacity().get();
                if g.blend_mode() != usvg::BlendMode::Normal {
                    c.blend = blend(g.blend_mode());
                }
                c.isolate |= g.isolate();
                if mask.is_some() {
                    c.mask = mask;
                }
                c.appearance.effects.extend(effects);
                return Some(c);
            }
            let mut n = self.named(id, NodeKind::Group { children, clip: false });
            n.appearance.effects = effects;
            n
        };
        n.opacity = g.opacity().get();
        n.blend = blend(g.blend_mode());
        n.isolate = g.isolate();
        n.mask = mask;
        Some(n)
    }

    /// `<mask>` → opacity mask (luminance; alpha masks are approximated by their luminance). A
    /// mask our export wrote gets its options back ([`mask_flags`]) and its art without what
    /// stands for them: the inverting filter group and the backdrop rectangle.
    fn opacity_mask(&mut self, m: &usvg::Mask, ts: Affine, label: &str) -> Option<Box<vectorcraft_doc::OpacityMask>> {
        if m.kind() == usvg::MaskType::Alpha {
            self.warn(format!("alpha mask on {label} imported as a luminance opacity mask"));
        }
        if m.mask().is_some() {
            self.warn(format!("nested mask on {label} ignored"));
        }
        let (clip, invert) = self.mask_flags.get(m.id()).copied().unwrap_or((true, false));
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
        let art = match children.len() {
            0 => return None,
            1 => Arc::unwrap_or_clone(children.pop()?),
            _ => self.named("", NodeKind::Group { children, clip: false }),
        };
        let mut mask = vectorcraft_doc::OpacityMask::new(art, clip);
        mask.invert = invert;
        Some(Box::new(mask))
    }

    fn clip_node(&mut self, cp: &usvg::ClipPath, ts: Affine) -> Option<Node> {
        let shapes = clip_shapes(cp, ts);
        if shapes.is_empty() {
            return None;
        }
        let r = shapes[0].1;
        let mut subs = Vec::new();
        for (bp, _) in &shapes {
            subs.extend(PathData::from_bezpath(bp).subpaths);
        }
        let id = cp.id().to_string();
        if subs.len() == 1 {
            let path = PathData::new(subs);
            return Some(self.named(&id, NodeKind::Path { path, rule: r, live: None, clipping: true, guide: false }));
        }
        let children = subs
            .into_iter()
            .map(|sp| {
                let nid = self.doc.alloc_id();
                Arc::new(Node::new(nid, NodeKind::Path { path: PathData::single(sp), rule: r, live: None, clipping: false, guide: false }))
            })
            .collect();
        Some(self.named(&id, NodeKind::Compound { children, rule: r }))
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
        let (mut stops, perceptual) = self.stops(base);
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
            gradient: Gradient {
                interpolation: if perceptual { GradientInterpolation::Perceptual } else { GradientInterpolation::Linear },
                ..Gradient::new(kind, stops)
            },
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
        let mut children = vec![Arc::new(clip)];
        children.extend(content);
        let art = self.named("", NodeKind::Group { children, clip: true });
        let base = if pt.id().is_empty() { "Pattern" } else { pt.id() };
        let name = unique_name(base, |n| self.doc.pattern(n).is_some());
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
        let path = PathData::from_bezpath(&bezpath(p.data(), acc));
        if path.is_empty() {
            return None;
        }
        let (appearance, r) = self.appearance(p, acc, path.bounds());
        let mut n = if path.subpaths.len() > 1 {
            // Multi-subpath SVG paths are compound paths (as in Illustrator).
            let children = path
                .subpaths
                .into_iter()
                .map(|sp| {
                    let id = self.doc.alloc_id();
                    let mut c = Node::new(id, NodeKind::Path { path: PathData::single(sp), rule: r, live: None, clipping: false, guide: false });
                    c.appearance = appearance.clone();
                    Arc::new(c)
                })
                .collect();
            self.named(p.id(), NodeKind::Compound { children, rule: r })
        } else {
            self.named(p.id(), NodeKind::Path { path, rule: r, live: None, clipping: false, guide: false })
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
        let key = format!("img-{:016x}", fnv1a(bytes));
        // A linked file keeps a preview; a placeholder is all a missing one has (its preview).
        let linked = self.files.linked(i.kind());
        let missing = linked.is_some_and(|l| l.missing);
        let link = linked.map(|l| l.link.clone());
        self.doc.images.entry(key.clone()).or_insert_with(|| {
            let blob = ImageBlob { mime: mime.into(), bytes: bytes.clone(), proxy: missing.then(|| bytes.clone()) };
            if link.is_some() && !missing { blob.with_proxy() } else { blob }
        });
        let xf = acc * Affine::scale_non_uniform(size.width() as f64 / pw as f64, size.height() as f64 / ph as f64);
        Some(self.named(i.id(), NodeKind::Image(ImageObject { key, width: pw, height: ph, xf, link, placement: Default::default() })))
    }

    /// An SVG shown as an image (a data URL or a linked file) → its art, `acc` mapping its size.
    /// Its ids are its own: they mean nothing in the SVG around it.
    fn svg_image(&mut self, tree: &usvg::Tree, acc: Affine) -> Option<Node> {
        let outer = (
            std::mem::take(&mut self.links),
            std::mem::take(&mut self.hidden),
            std::mem::take(&mut self.uses),
            std::mem::take(&mut self.labels),
            std::mem::take(&mut self.non_scaling),
        );
        let art = self.group_node(tree.root(), acc);
        (self.links, self.hidden, self.uses, self.labels, self.non_scaling) = outer;
        art
    }
}
