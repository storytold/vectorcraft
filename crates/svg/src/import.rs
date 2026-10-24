//! usvg tree → document conversion.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

use usvg::roxmltree;
use vectorcraft_color::{BlendMode, Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{
    Appearance, AppearanceItem, Dash, Document, FillLayer, ForeignAttribute, ImageBlob, ImageObject, LayerColor, LineCap, LineJoin, MAX_SVG_SCRIPTS,
    Node, NodeId, NodeKind, PatternDef, SVG_EVENTS, StrokeLayer, Unit,
};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, Vec2, shapes};

use crate::export::Reuse;
use crate::{ImportOptions, SvgError, fnv1a};
use css::XNode;
use text::TextSlots;

pub use depth::MAX_ELEMENT_DEPTH;
pub(crate) use depth::xml_too_deep;
pub(crate) use fx::BLEND;

mod css;
mod css_transform;
mod depth;
mod files;
mod fx;
mod spread;
mod symbol_units;
mod text;
mod vector_effect;

/// The XML of `svg`.
fn parse(svg: &str) -> Result<roxmltree::Document<'_>, SvgError> {
    if crate::entities::expansion_too_large(svg) {
        return Err(SvgError::Parse("the entities the document declares expand to too much text".into()));
    }
    roxmltree::Document::parse_with_options(svg, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() })
        .map_err(|e| SvgError::Parse(e.to_string()))
}

/// The document SVG text `svg` holds, read with `opts`, and the warnings about what it couldn't keep.
pub(crate) fn import(svg: &str, opts: &ImportOptions) -> Result<(Document, Vec<String>), SvgError> {
    // Before anything reads the tree: the XML reader, usvg, and what is made of its tree, all by
    // recursion.
    let too_deep = || SvgError::TooDeep(depth::MAX_ELEMENT_DEPTH);
    let scan = depth::prescan(svg);
    if scan.too_deep {
        return Err(too_deep());
    }
    let xml = parse(svg)?;
    let Some(height) = depth::height(&xml, depth::MAX_ELEMENT_DEPTH) else {
        return Err(too_deep());
    };
    let (svg, found) = prepass(svg, &scan.markers, &xml, opts);
    // The prepass's edits are in the text: the XML is read again only when it made any.
    let xml = match &svg {
        Cow::Borrowed(_) => xml,
        Cow::Owned(edited) => parse(edited)?,
    };
    let svg = svg.as_ref();
    let units = RootUnits::of(xml.root_element());
    let root_name = root_layer_name(xml.root_element());
    let mut warnings = found.warnings;
    let (src, slots) = text::prepare(svg, &xml, units.dpi, &mut warnings);
    let files = found.files;
    let tree = {
        let opt = usvg::Options {
            dpi: units.dpi as f32,
            font_size: DEFAULT_FONT_SIZE as f32,
            image_href_resolver: files.resolver(height),
            ..usvg::Options::default()
        };
        usvg::Tree::from_str(&src, &opt).map_err(|e| SvgError::Parse(e.to_string()))?
    };
    let size = tree.size();
    let k = units.k;
    let mut doc = Document::new(size.width() as f64 * k, size.height() as f64 * k);
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
        blends: fx::blends(&xml, &scan.markers),
        files,
        non_scaling: found.non_scaling,
        data: found.data,
        events: found.events,
        foreign: found.foreign,
        sublayers: found.sublayers,
        sublayer_groups: HashSet::new(),
        screen: k,
    };
    im.doc.svg_scripts = found.scripts;
    if im.files.nested_text() {
        im.warn("text inside an SVG image isn't imported".into());
    }
    for note in im.files.left_out() {
        im.warn(note);
    }

    // usvg wraps everything in an id-less group carrying the viewBox transform when needed.
    let mut top = tree.root();
    let mut base = Affine::scale(k) * aff(top.transform());
    if let Some(g) = im.wrapper(top) {
        base *= aff(g.transform());
        top = g;
    }
    if let Some(boards) = inkscape_pages(&xml, base) {
        im.doc.artboards = boards;
    }

    // Top-level `<g id>` elements become layers (as in the reference app), named from their
    // labels (`data-name`…) else their ids; otherwise all art goes into "Layer 1". A clip path on
    // one (as on an Inkscape layer, or our clipping layers) makes it a clipping layer. Top-level
    // text joins the layer below it (the first layer if none is).
    let is_layer = |im: &Importer, c: &usvg::Node| matches!(c, usvg::Node::Group(g) if !g.id().is_empty() && !made_up(g.id()) && !im.links.contains_key(g.id()) && !im.uses.contains_key(g.id()) && only_clips(g) && im.text_slot(g).is_none());
    let is_text = |im: &Importer, c: &usvg::Node| matches!(c, usvg::Node::Group(g) if im.text_slot(g).is_some());
    let layer_mode = |im: &Importer, top: &usvg::Group| {
        top.children().iter().any(|c| is_layer(im, c)) && top.children().iter().all(|c| is_layer(im, c) || is_text(im, c))
    };
    // The layers may sit in more wrappers (a transform on all the art, a `<switch>`'s group).
    if !layer_mode(&im, top) {
        let (mut inner, mut ts) = (top, base);
        while let Some(g) = im.wrapper(inner) {
            ts *= aff(g.transform());
            inner = g;
        }
        if layer_mode(&im, inner) {
            (top, base) = (inner, ts);
        }
    }
    if layer_mode(&im, top) {
        im.doc.layers.clear();
        let mut loose = vec![];
        for c in top.children() {
            let usvg::Node::Group(g) = c else { continue };
            if !is_layer(&im, c) {
                let n: Vec<_> = im.node(c, base).map(Arc::new).into_iter().collect();
                match im.doc.layers.pop() {
                    Some(mut l) => {
                        im.add_unclipped(Arc::make_mut(&mut l), usize::MAX, n);
                        im.doc.layers.push(l);
                    }
                    None => loose.extend(n),
                }
                continue;
            }
            let ts = base * aff(g.transform());
            let name = im.name_of(g.id()).to_string();
            let mut art = im.children(g, ts);
            // A clip path with no shape shows nothing of the layer: it stays, empty.
            let clip = match g.clip_path() {
                Some(cp) => {
                    art = im.clipped(cp, ts, art, &format!("layer '{name}'")).unwrap_or_default();
                    !art.is_empty()
                }
                None => false,
            };
            let id = im.doc.alloc_id();
            let preset = (im.doc.layers.len() % vectorcraft_doc::LAYER_COLORS.len()) as u8;
            let mut l = Node::layer(id, &name, LayerColor::Preset(preset));
            l.set_clips(clip);
            if let Some(ch) = l.children_mut() {
                *ch = art;
            }
            im.add_unclipped(&mut l, 0, std::mem::take(&mut loose));
            l.visible = !im.hidden.contains(g.id());
            im.keep_data(g.id(), &mut l);
            im.keep_events(g.id(), &mut l);
            im.keep_foreign(g.id(), &mut l);
            im.doc.layers.push(Arc::new(l));
        }
    } else {
        // An export of a one-layer document names the layer on the root element, its art (groups
        // among it) loose inside: the one layer takes that name.
        let children = im.children(top, base);
        if let Some(l) = im.doc.layers.first_mut() {
            let l = Arc::make_mut(l);
            if let Some(ch) = l.children_mut() {
                *ch = children;
            }
            if root_name.is_some() {
                l.name = root_name;
            }
        }
    }
    if !im.sublayer_groups.is_empty() {
        for l in &mut im.doc.layers {
            promote_sublayers(Arc::make_mut(l), &im.sublayer_groups);
        }
    }
    im.doc.limit_depth_noting(&mut im.warnings);
    Ok((im.doc, im.warnings))
}

/// What marks a nested `<g>` as a sublayer rather than a group: our export writes it on the layers
/// it writes inside layers. (Inkscape's `inkscape:groupmode="layer"` counts too.)
pub(crate) const SUBLAYER: &str = "data-vc-layer";

/// The Inkscape namespace (its layers and their labels).
const INKSCAPE: &str = "http://www.inkscape.org/namespaces/inkscape";

/// The layer name on root element `root` (`<svg id="Main">`, or its `data-name`), as exports of
/// one-layer documents write it. An Inkscape root's id (`svg8`) is one Inkscape made up, not a name.
fn root_layer_name(root: XNode) -> Option<String> {
    let id = root.attribute("id").filter(|_| root.attribute((INKSCAPE, "version")).is_none());
    root.attribute("data-name").or(id).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string)
}

/// Is element `g` a `<g>` marked as a layer ([`SUBLAYER`])? Only one with an id counts: the id (or
/// the label beside it) is its name.
fn marked_layer(g: XNode) -> bool {
    g.tag_name().name() == "g"
        && g.attribute("id").is_some_and(|id| !id.is_empty())
        && (g.has_attribute(SUBLAYER) || g.attribute((INKSCAPE, "groupmode")) == Some("layer"))
}

/// Turn the groups made of elements marked as layers (`marked`, see [`marked_layer`]) right inside
/// layer `l` into its sublayers, in its colour, and theirs in turn. A marked group anywhere else
/// (in a group, a mask, a symbol) stays a group: only layers hold layers.
fn promote_sublayers(l: &mut Node, marked: &HashSet<NodeId>) {
    let NodeKind::Layer { color, children, .. } = &mut l.kind else { return };
    let color = *color;
    for c in children.iter_mut().filter(|c| marked.contains(&c.id)) {
        let c = Arc::make_mut(c);
        let NodeKind::Group { children, clip } = &mut c.kind else { continue };
        let mut sub = Node::layer(c.id, "", color);
        sub.set_clips(*clip);
        if let Some(ch) = sub.children_mut() {
            *ch = std::mem::take(children);
        }
        c.kind = sub.kind;
        promote_sublayers(c, marked);
    }
}

/// SVG's initial `font-size` (`medium`) in user units.
pub(super) const DEFAULT_FONT_SIZE: f64 = 12.0;

/// How lengths become points, from the root `<svg>`'s `width` / `height`.
///
/// A pixel (and a unitless user unit) is a point, as we export, and absolute lengths keep their
/// physical size at 72 pt per inch: `font-size="12pt"` is 12 pt, `1in` is 72 pt. A root size in
/// absolute units (`width="210mm"`) is that physical size, and its user units are CSS pixels of it
/// (96 per inch, 0.75 pt each), so a drawing without a `viewBox` keeps its proportions. With one
/// side absolute and the other not (`width="96pt" height="96"`), both are: the unitless side is
/// CSS pixels too, so the drawing isn't stretched (#1010).
struct RootUnits {
    /// Pixels per inch usvg (and the text pass) convert absolute lengths at.
    dpi: f64,
    /// Points per user unit of the root, along both axes.
    k: f64,
    /// Document units: those of the root `width` (pixels when it has none).
    unit: Unit,
}

impl RootUnits {
    fn of(root: XNode) -> Self {
        let unit = |name: &str| root.attribute(name).and_then(|v| svgtypes::Length::from_str(v.trim()).ok()).map(|l| l.unit);
        use svgtypes::LengthUnit as U;
        let physical = |u: Option<U>| matches!(u, Some(U::In | U::Cm | U::Mm | U::Pt | U::Pc));
        let (w, h) = (unit("width"), unit("height"));
        let (dpi, k) = if physical(w) || physical(h) { (CSS_PX_PER_IN, PT_PER_IN / CSS_PX_PER_IN) } else { (PT_PER_IN, 1.0) };
        let unit = match w {
            Some(U::Mm) => Unit::Millimeters,
            Some(U::Cm) => Unit::Centimeters,
            Some(U::In) => Unit::Inches,
            Some(U::Pt) => Unit::Points,
            Some(U::Pc) => Unit::Picas,
            _ => Unit::Pixels,
        };
        Self { dpi, k, unit }
    }
}

/// `'id' (feA, feB)` for each filter: its id and the primitives it uses, for warnings.
fn filter_names(filters: &[Arc<usvg::filter::Filter>]) -> String {
    use usvg::filter::Kind as K;
    let names: Vec<String> = filters
        .iter()
        .map(|f| {
            let mut kinds: Vec<&str> = Vec::new();
            for p in f.primitives() {
                let k = match p.kind() {
                    K::Blend(_) => "feBlend",
                    K::ColorMatrix(_) => "feColorMatrix",
                    K::ComponentTransfer(_) => "feComponentTransfer",
                    K::Composite(_) => "feComposite",
                    K::ConvolveMatrix(_) => "feConvolveMatrix",
                    K::DiffuseLighting(_) => "feDiffuseLighting",
                    K::DisplacementMap(_) => "feDisplacementMap",
                    K::DropShadow(_) => "feDropShadow",
                    K::Flood(_) => "feFlood",
                    K::GaussianBlur(_) => "feGaussianBlur",
                    K::Image(_) => "feImage",
                    K::Merge(_) => "feMerge",
                    K::Morphology(_) => "feMorphology",
                    K::Offset(_) => "feOffset",
                    K::SpecularLighting(_) => "feSpecularLighting",
                    K::Tile(_) => "feTile",
                    K::Turbulence(_) => "feTurbulence",
                };
                if !kinds.contains(&k) {
                    kinds.push(k);
                }
            }
            let id = if f.id().is_empty() || made_up(f.id()) { String::new() } else { format!("'{}' ", f.id()) };
            format!("{id}({})", kinds.join(", "))
        })
        .collect();
    names.join(", ")
}

/// Inkscape's pages (`<inkscape:page>` in its `<sodipodi:namedview>`, in user units) as artboards
/// through `user` (user units → document points), named by their labels; `None` without any.
fn inkscape_pages(xml: &roxmltree::Document, user: Affine) -> Option<Vec<vectorcraft_doc::Artboard>> {
    let num = |n: roxmltree::Node, a: &str| n.attribute(a).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
    let pages: Vec<vectorcraft_doc::Artboard> = xml
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "page" && n.tag_name().namespace() == Some(INKSCAPE))
        .filter_map(|p| {
            let (w, h) = (num(p, "width")?, num(p, "height")?);
            let (x, y) = (num(p, "x").unwrap_or(0.0), num(p, "y").unwrap_or(0.0));
            // A box too large to add up or to place (1e308 wide) is no page.
            let r = user.transform_rect_bbox(Rect::new(x, y, x + w, y + h));
            (w > 0.0 && h > 0.0 && r.is_finite()).then_some((p, r))
        })
        .take(1000)
        .enumerate()
        .map(|(i, (p, r))| vectorcraft_doc::Artboard {
            id: i as u32 + 1,
            name: p.attribute((INKSCAPE, "label")).filter(|l| !l.trim().is_empty()).map_or_else(|| format!("Artboard {}", i + 1), str::to_string),
            rect: r,
            locked: false,
            show_center_mark: false,
            show_cross_hairs: false,
            show_video_safe_areas: false,
            video_ruler_pixel_aspect_ratio: 1.0,
            color: None,
        })
        .collect();
    (!pages.is_empty()).then_some(pages)
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
    /// Midpoint stops by gradient id (see [`midpoint_stops`]).
    midpoints: HashMap<String, Vec<Option<f32>>>,
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
    /// The `data-*` attributes of elements, by id ([`user_data`]).
    data: HashMap<String, Vec<(String, String)>>,
    /// The event attributes of elements, by id ([`user_events`]).
    events: HashMap<String, Vec<(String, String)>>,
    /// The attributes of elements in other applications' namespaces, by id ([`foreign`]).
    foreign: HashMap<String, Vec<ForeignAttribute>>,
    /// Ids of the `<g>` elements marked as layers ([`marked_layer`]).
    sublayers: HashSet<String>,
    /// The groups made of them, made sublayers where they sit in a layer ([`promote_sublayers`]).
    sublayer_groups: HashSet<NodeId>,
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
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_deref_mut()) {
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

/// The stops our export added for midpoints (`data-vc-midpoint`), by gradient id: for each stop,
/// the midpoint it stands for.
fn midpoint_stops(xml: &roxmltree::Document) -> HashMap<String, Vec<Option<f32>>> {
    xml.descendants()
        .filter(|n| matches!(n.tag_name().name(), "linearGradient" | "radialGradient"))
        .filter_map(|g| {
            let id = g.attribute("id")?;
            let marks: Vec<Option<f32>> = g
                .children()
                .filter(|c| c.tag_name().name() == "stop")
                .map(|c| c.attribute("data-vc-midpoint").and_then(|v| v.parse().ok()))
                .collect();
            marks.iter().any(Option::is_some).then(|| (id.to_string(), marks))
        })
        .collect()
}

/// A gradient's stops, with the midpoint stops `marks` names folded back into the midpoints of
/// the stops before them.
fn gradient_stops(g: &usvg::BaseGradient, marks: Option<&[Option<f32>]>) -> Vec<GradientStop> {
    let marks = marks.filter(|m| m.len() == g.stops().len());
    let mut out: Vec<GradientStop> = Vec::with_capacity(g.stops().len());
    for (i, s) in g.stops().iter().enumerate() {
        if let (Some(mid), Some(prev)) = (marks.and_then(|m| m.get(i).copied().flatten()), out.last_mut()) {
            prev.midpoint = mid.clamp(0.0, 1.0);
            continue;
        }
        let c = s.color();
        out.push(GradientStop { opacity: s.opacity().get(), ..GradientStop::new(s.offset().get(), Color::rgb8(c.red, c.green, c.blue)) });
    }
    out
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
    /// The `data-*` attributes of elements, by id ([`user_data`]).
    data: HashMap<String, Vec<(String, String)>>,
    /// The event attributes of elements, by id ([`user_events`]).
    events: HashMap<String, Vec<(String, String)>>,
    /// The attributes of elements in other applications' namespaces, by id ([`foreign`]).
    foreign: HashMap<String, Vec<ForeignAttribute>>,
    /// The JavaScript files `<script>` elements link to ([`linked_scripts`]).
    scripts: Vec<String>,
    /// Ids of the `<g>` elements marked as layers ([`marked_layer`]).
    sublayers: HashSet<String>,
    warnings: Vec<String>,
}

/// Changes to the SVG text: `(range, replacement)`, an empty range inserting.
#[derive(Default)]
pub(super) struct Edits {
    list: Vec<(std::ops::Range<usize>, String)>,
    /// The ids made up so far, by element.
    ids: HashMap<roxmltree::NodeId, String>,
    /// Declarations added to elements' `style`s, by element: (where they go, whether the element
    /// has a `style` attribute, the declarations, XML-escaped).
    styles: HashMap<roxmltree::NodeId, (usize, bool, String)>,
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

    /// Add the declaration `decl` (XML-escaped already) to the `style` of element `n` (in `svg`),
    /// after the ones already there (so it wins over them and over style sheet rules).
    fn style(&mut self, svg: &str, n: XNode, decl: &str) {
        let (_, _, decls) =
            self.styles.entry(n.id()).or_insert_with(|| match n.attributes().find(|a| a.name() == "style" && a.namespace().is_none()) {
                Some(a) => (a.range_value().end, true, String::new()),
                None => (tag_name_end(svg, n), false, String::new()),
            });
        decls.push(';');
        decls.push_str(decl);
    }

    /// `svg` with the edits made; overlapping ones and ranges off character boundaries are
    /// skipped.
    fn apply(mut self, svg: &str) -> Cow<'_, str> {
        for (at, has_style, decls) in std::mem::take(&mut self.styles).into_values() {
            self.insert(at, if has_style { decls } else { format!(" style=\"{}\"", decls.strip_prefix(';').unwrap_or(&decls)) });
        }
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
/// * objects with event attributes get an id when they have none, recorded with them
///   ([`user_events`]), and the `<script>` links are recorded ([`linked_scripts`]);
/// * so do objects with attributes in other applications' namespaces ([`foreign`]);
/// * linked files are read ([`files`]);
/// * percentage lengths in a `<symbol>` with a `viewBox` become numbers ([`symbol_units`]);
/// * what style sheet rules with a `:not()` set goes in `style` attributes ([`not_rules`]).
///
/// Whether each is there at all is read from `markers`: `svg` with its data URLs left out
/// ([`depth::prescan`]), so an embedded picture isn't searched a dozen times. The edits are made
/// in `svg` itself.
fn prepass<'s>(svg: &'s str, markers: &str, xml: &roxmltree::Document, opts: &ImportOptions) -> (Cow<'s, str>, Found) {
    let mut found = Found::default();
    // (`:` for prefixed elements such as `<svg:image>`.)
    let css_transform = has_css_transform(markers);
    let not = markers.contains(":not(");
    let events = SVG_EVENTS.iter().any(|e| markers.contains(e));
    let foreign_attrs = markers.contains("xmlns:") && has_foreign_attributes(xml);
    if !css_transform
        && !not
        && !events
        && !foreign_attrs
        && !["<a", "display", "<use", ":use", "<image", ":image", vector_effect::NON_SCALING, "data-", "groupmode", "script"]
            .iter()
            .any(|t| markers.contains(t))
    {
        return (svg.into(), found);
    }
    let mut edits = Edits::default();
    let elements = || xml.descendants().filter(|n| n.is_element());
    for a in elements().filter(|n| n.tag_name().name() == "a") {
        if let Some(url) = href(a) {
            found.links.insert(edits.id(svg, a), url.to_string());
        }
    }
    if markers.contains("data-") {
        for e in elements().filter(|e| OBJECT_ELEMENTS.contains(&e.tag_name().name())) {
            let data = user_data(e);
            if !data.is_empty() {
                found.data.insert(edits.id(svg, e), data);
            }
        }
    }
    if foreign_attrs {
        for e in elements().filter(|e| OBJECT_ELEMENTS.contains(&e.tag_name().name())) {
            let kept = foreign(e, &mut found.warnings);
            if !kept.is_empty() {
                found.foreign.insert(edits.id(svg, e), kept);
            }
        }
    }
    if events {
        for e in elements().filter(|e| EVENT_ELEMENTS.contains(&e.tag_name().name())) {
            let events = user_events(e);
            if !events.is_empty() {
                found.events.insert(edits.id(svg, e), events);
            }
        }
    }
    if markers.contains("script") {
        found.scripts = linked_scripts(xml, &mut found.warnings);
    }
    found.sublayers = elements().filter(|g| marked_layer(*g)).filter_map(|g| g.attribute("id")).map(str::to_string).collect();
    let symbols: HashSet<&str> = elements().filter(|n| n.tag_name().name() == "symbol").filter_map(|n| n.attribute("id")).collect();
    for u in elements().filter(|n| n.tag_name().name() == "use") {
        if let Some(sym) = href(u).and_then(|h| h.strip_prefix('#')).filter(|s| symbols.contains(s)) {
            found.uses.insert(edits.id(svg, u), sym.to_string());
        }
    }
    let (display, non_scaling) = (markers.contains("display"), markers.contains(vector_effect::NON_SCALING));
    if display || non_scaling || not {
        let css = css::Styles::new(xml);
        // First: an undisplayed object shown for usvg is shown after these.
        if not && css.has_not_rules() {
            not_rules(svg, xml, &css, &mut edits);
        }
        if display {
            hidden_objects(svg, xml, &css, &mut edits, &mut found.hidden);
        }
        if non_scaling {
            vector_effect::find(svg, xml, &css, &mut edits, &mut found.non_scaling, &mut found.warnings);
        }
    }
    found.files = files::Files::read(svg, xml, opts, &mut edits, &mut found.warnings);
    if !symbols.is_empty() && markers.contains('%') {
        edits.list.extend(symbol_units::rewrites(xml));
    }
    if css_transform {
        edits.list.extend(css_transform::rewrites(svg, xml));
    }
    (edits.apply(svg), found)
}

/// Is there a `transform` CSS declaration in `markers` (`transform` followed by `:`)?
fn has_css_transform(markers: &str) -> bool {
    markers.match_indices("transform").any(|(i, t)| markers.get(i + t.len()..).is_some_and(|s| s.trim_start().starts_with(':')))
}

/// usvg's style sheets drop rules with a `:not()` selector (#1557). The properties such rules set
/// on an element go in its `style` attribute, with the value that wins the cascade there (which
/// can be another rule's or the attribute's own), so usvg reads what CSS would.
fn not_rules(svg: &str, xml: &roxmltree::Document, css: &css::Styles, edits: &mut Edits) {
    for n in xml.descendants().filter(XNode::is_element) {
        for (name, value) in css.set_by_not_rules(n) {
            let value = if name == "transform" { css_transform::to_svg(&value).unwrap_or(value) } else { value };
            edits.style(svg, n, &crate::xml_escape(&format!("{name}:{value}")));
        }
    }
}

/// The elements that are objects (what [`hidden_objects`] shows).
const OBJECTS: &[&str] = &["g", "a", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "image", "text", "use"];

/// Show the undisplayed objects outside `<defs>` (and the like) for usvg, recording their ids. The
/// objects in a `<switch>` count too (some apps wrap all their layers in one).
fn hidden_objects(svg: &str, xml: &roxmltree::Document, css: &css::Styles, edits: &mut Edits, ids: &mut HashSet<String>) {
    let linked: HashSet<&str> = xml.descendants().filter_map(href).filter_map(|h| h.strip_prefix('#')).collect();
    let mut todo: Vec<XNode> = xml.root_element().children().filter(XNode::is_element).collect();
    while let Some(n) = todo.pop() {
        let tag = n.tag_name().name();
        if n.attribute("id").is_some_and(|id| linked.contains(id)) {
            continue;
        }
        if matches!(tag, "g" | "a" | "switch") {
            todo.extend(n.children().filter(XNode::is_element));
        }
        if !OBJECTS.contains(&tag) || css.own(n, "display").as_deref() != Some("none") {
            continue;
        }
        ids.insert(edits.id(svg, n));
        // A `style` declaration wins over the attribute and style sheet rules.
        edits.style(svg, n, "display:inline");
    }
}

/// Where the start tag of element `n` ends its name (where an attribute can go).
fn tag_name_end(svg: &str, n: XNode) -> usize {
    let start = n.range().start;
    svg.get(start + 1..).and_then(|s| s.find(|c: char| c.is_whitespace() || c == '/' || c == '>')).map_or(start + 2, |i| start + 1 + i)
}

/// The elements that become objects, whose `data-*` attributes are kept.
const OBJECT_ELEMENTS: &[&str] = &["g", "a", "use", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "image"];

/// The most `data-*` attributes kept per element (the file is untrusted).
const MAX_DATA: usize = 64;

/// The `data-*` attributes of element `e`, in order, without the `data-` (ours, `data-name` and
/// `data-vc-*`, aside).
fn user_data(e: XNode) -> Vec<(String, String)> {
    e.attributes()
        .filter(|a| a.namespace().is_none())
        .filter_map(|a| Some((a.name().strip_prefix("data-")?, a.value())))
        .filter(|(k, _)| !k.is_empty() && *k != "name" && !k.starts_with("vc-"))
        .take(MAX_DATA)
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// The namespaces whose attributes VectorCraft reads, writes or makes again itself: their
/// attributes aren't kept as another application's ([`foreign`]).
const OWN_NAMESPACES: &[&str] = &[
    "http://www.w3.org/2000/svg",
    "http://www.w3.org/1999/xlink",
    "http://www.w3.org/XML/1998/namespace",
    "http://www.w3.org/2000/xmlns/",
    INKSCAPE,
    "http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd",
    crate::EDITING_NS,
];

/// The most attributes in other applications' namespaces kept per element (the file is untrusted).
pub const MAX_FOREIGN: usize = 64;

/// The longest value of such an attribute kept, in bytes; a longer one is left out.
pub const MAX_FOREIGN_VALUE: usize = 64 * 1024;

/// The namespace of attribute `a` when it is another application's ([`OWN_NAMESPACES`] aside).
fn foreign_ns<'a>(a: &roxmltree::Attribute<'a, '_>) -> Option<&'a str> {
    a.namespace().filter(|ns| !OWN_NAMESPACES.contains(ns))
}

/// Does any element of `xml` have an attribute in another application's namespace? Most files
/// declare only namespaces VectorCraft reads itself (XLink), and skip the scan for them.
fn has_foreign_attributes(xml: &roxmltree::Document) -> bool {
    xml.descendants().filter(XNode::is_element).any(|e| e.attributes().any(|a| foreign_ns(&a).is_some()))
}

/// The attributes of element `e` in other applications' namespaces, in order, with the prefix the
/// file gives each namespace: `plotter:pen="2"`. Those past [`MAX_FOREIGN`], and values longer
/// than [`MAX_FOREIGN_VALUE`], are left out with a warning.
fn foreign(e: XNode, warnings: &mut Vec<String>) -> Vec<ForeignAttribute> {
    let mut kept = vec![];
    let mut left_out = false;
    for a in e.attributes() {
        let Some(ns) = foreign_ns(&a) else { continue };
        if kept.len() == MAX_FOREIGN || a.value().len() > MAX_FOREIGN_VALUE {
            left_out = true;
            continue;
        }
        // A namespaced attribute always has a prefix in the file; `ns` stands in where the only
        // one found is the default namespace's (export makes it unique).
        let prefix = e.lookup_prefix(ns).filter(|p| !p.is_empty()).unwrap_or("ns");
        kept.push(ForeignAttribute { ns: ns.to_string(), prefix: prefix.to_string(), name: a.name().to_string(), value: a.value().to_string() });
    }
    if left_out {
        let w = format!(
            "other applications' attributes past {MAX_FOREIGN} on an element, or longer than {} KiB, were left out",
            MAX_FOREIGN_VALUE / 1024
        );
        if !warnings.contains(&w) {
            warnings.push(w);
        }
    }
    kept
}

/// The elements whose event attributes are kept: the objects, and type.
const EVENT_ELEMENTS: &[&str] = &["g", "a", "use", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "image", "text"];

/// The event attributes of element `e` that the SVG Interactivity panel lists ([`SVG_EVENTS`]),
/// in order: the JavaScript is kept as text and never run.
fn user_events(e: XNode) -> Vec<(String, String)> {
    e.attributes()
        .filter(|a| a.namespace().is_none() && SVG_EVENTS.contains(&a.name()))
        .map(|a| (a.name().to_string(), a.value().to_string()))
        .collect()
}

/// The JavaScript files the `<script>` elements link to, in order and each once, at most
/// [`MAX_SVG_SCRIPTS`] (the file is untrusted): the links are kept as text, and the files are never
/// read or run. Inline scripts and the links past the limit are left out with a warning.
fn linked_scripts(xml: &roxmltree::Document, warnings: &mut Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut over = false;
    for s in xml.descendants().filter(|n| n.is_element() && n.tag_name().name() == "script") {
        match href(s) {
            Some(h) if out.iter().any(|o| o == h) => {}
            Some(h) if out.len() < MAX_SVG_SCRIPTS => out.push(h.to_string()),
            Some(_) => over = true,
            None if s.text().is_some_and(|t| !t.trim().is_empty()) => {
                let w = "inline scripts were left out: only linked JavaScript files are kept".to_string();
                if !warnings.contains(&w) {
                    warnings.push(w);
                }
            }
            None => {}
        }
    }
    if over {
        warnings.push(format!("more than {MAX_SVG_SCRIPTS} JavaScript files are linked: only the first {MAX_SVG_SCRIPTS} were kept"));
    }
    out
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
    g.clip_path().is_none() && only_clips(g)
}

/// Does group `g` leave its art as it is, but for a clip path (no mask, filter, opacity or blend)?
fn only_clips(g: &usvg::Group) -> bool {
    g.mask().is_none() && g.filters().is_empty() && g.opacity().get() >= 1.0 && g.blend_mode() == usvg::BlendMode::Normal
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

    /// The child of `g` when it's all `g` holds and only wraps art: an id-less group that leaves
    /// it as it is (but for a transform) and isn't a text.
    fn wrapper<'g>(&self, g: &'g usvg::Group) -> Option<&'g usvg::Group> {
        match g.children() {
            [usvg::Node::Group(w)] if w.id().is_empty() && is_plain(w) && self.text_slot(w).is_none() => Some(w),
            _ => None,
        }
    }

    fn children(&mut self, g: &usvg::Group, acc: Affine) -> Vec<Arc<Node>> {
        g.children().iter().filter_map(|c| self.node(c, acc)).map(Arc::new).collect()
    }

    /// The object usvg node `n` makes under transform `acc`, with what the file kept on its element
    /// (data, event and other applications' attributes); `None` for what imports as nothing.
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
        self.keep_data(n.id(), &mut out);
        self.keep_events(n.id(), &mut out);
        self.keep_foreign(n.id(), &mut out);
        Some(out)
    }

    /// Give `n` the `data-*` attributes of element `id`, unless it has data already (what an
    /// inner element made of it keeps).
    fn keep_data(&self, id: &str, n: &mut Node) {
        if let Some(data) = self.data.get(id).filter(|_| n.extras().attrs.as_ref().is_none_or(|a| a.data.is_empty())) {
            n.edit_attrs(|a| a.data = data.clone());
        }
    }

    /// Give `n` the event attributes of element `id`, unless it has events already (what an inner
    /// element made of it keeps).
    fn keep_events(&self, id: &str, n: &mut Node) {
        if let Some(events) = self.events.get(id).filter(|_| n.extras().attrs.as_ref().is_none_or(|a| a.events.is_empty())) {
            n.edit_attrs(|a| a.events = events.clone());
        }
    }

    /// Give `n` the attributes of element `id` in other applications' namespaces, unless it has
    /// some already (what an inner element made of it keeps).
    fn keep_foreign(&self, id: &str, n: &mut Node) {
        if let Some(kept) = self.foreign.get(id).filter(|_| n.extras().attrs.as_ref().is_none_or(|a| a.foreign.is_empty())) {
            n.edit_attrs(|a| a.foreign = kept.clone());
        }
    }

    /// A gradient's stops, midpoints restored.
    fn stops(&self, g: &usvg::BaseGradient) -> Vec<GradientStop> {
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
        n.extras_mut().mask = mask;
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
        let mut n = self.named(&name, NodeKind::Text(Box::new(obj)));
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
        // Blurs, shadows and glows become live effects (see [`fx`]).
        let effects = match g.filters() {
            [] => vec![],
            filters => fx::effects(filters, ts, &self.blends).unwrap_or_else(|| {
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
                let mut inner = self.named("", NodeKind::Group { children, clip: false });
                inner.appearance.effects = effects.clone();
                children = vec![Arc::new(inner)];
            }
            let children = self.clipped(cp, ts, children, &label)?;
            self.named(id, NodeKind::Group { children, clip: true })
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
            let mut n = self.named(id, NodeKind::Group { children, clip: false });
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

    /// `<mask>` → opacity mask. An alpha mask's art comes in white with its opacity kept, so its
    /// luminance is its alpha (images in it keep their colours, with a warning). A mask our export wrote gets its
    /// options back ([`mask_flags`]) and its art without what stands for them: the inverting
    /// filter group and the backdrop rectangle.
    fn opacity_mask(&mut self, m: &usvg::Mask, ts: Affine, label: &str) -> Option<Box<vectorcraft_doc::OpacityMask>> {
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
            _ => self.named("", NodeKind::Group { children, clip: false }),
        };
        let mut mask = vectorcraft_doc::OpacityMask::new(art, clip);
        mask.invert = invert;
        Some(Box::new(mask))
    }

    /// `children` clipped by `cp` (on an element whose user space `ts` maps to the document): the
    /// children of a clip group or clipping layer, its clipping path first. A clip path clipped in
    /// turn (`<clipPath clip-path>`) nests one clip group in another, so the clips intersect.
    /// `None` when the clip path has no shape (nothing shows).
    fn clipped(&mut self, cp: &usvg::ClipPath, ts: Affine, mut children: Vec<Arc<Node>>, label: &str) -> Option<Vec<Arc<Node>>> {
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
        Some(ch)
    }

    /// Put `art` in layer `l` at `index` (at most its end) unclipped: a clipping mask on the layer
    /// becomes a clip group in it, holding the art it clipped.
    fn add_unclipped(&mut self, l: &mut Node, index: usize, art: Vec<Arc<Node>>) {
        if art.is_empty() {
            return;
        }
        if let NodeKind::Layer { children, clip: clip @ true, .. } = &mut l.kind {
            *clip = false;
            let clipped = std::mem::take(children);
            children.push(Arc::new(self.named("", NodeKind::Group { children: clipped, clip: true })));
        }
        if let Some(ch) = l.children_mut() {
            let i = index.min(ch.len());
            ch.splice(i..i, art);
        }
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
        // Shared, so a compound path's subpaths don't each copy a gradient's stops.
        let appearance: vectorcraft_doc::Shared<Appearance> = appearance.into();
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
        Some(self.named(
            i.id(),
            NodeKind::Image(Box::new(ImageObject { key, width: pw, height: ph, xf, link, placement: Default::default(), pixels: None })),
        ))
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
            std::mem::take(&mut self.data),
            std::mem::take(&mut self.events),
        );
        let art = self.group_node(tree.root(), acc);
        (self.links, self.hidden, self.uses, self.labels, self.non_scaling, self.data, self.events) = outer;
        art
    }
}
