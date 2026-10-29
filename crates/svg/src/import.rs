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
pub use files::MAX_NESTED_SVG_IMAGES;
pub(crate) use fx::BLEND;

/// Most objects an imported SVG may expand to: a guard against `<use>` and nested-image
/// expansion bombs (#3564).
pub const MAX_NODES: usize = if cfg!(target_arch = "wasm32") { 500_000 } else { 2_000_000 };

mod convert;
mod importer;
mod layers;
mod prepass;

pub(super) use convert::href;
use convert::{aff, made_up, mask_flags, midpoint_stops, only_clips};
pub(super) use importer::PT_PER_IN;
use importer::SymbolMade;
pub(super) use layers::DEFAULT_FONT_SIZE;
pub(crate) use layers::SUBLAYER;
use layers::{RootUnits, inkscape_pages, promote_sublayers, root_layer_name};
pub(super) use prepass::Edits;
pub use prepass::{MAX_FOREIGN, MAX_FOREIGN_VALUE};
use prepass::{labels, prepass};

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
pub(crate) fn import(svg: &str, opts: &ImportOptions, max_nodes: usize) -> Result<(Document, Vec<String>), SvgError> {
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
        converted_nodes: 1,
        max_nodes,
        error: None,
        nested_svg_images: 0,
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
            let Some(id) = im.alloc_id() else {
                return Err(im.error.unwrap_or(SvgError::TooLarge(im.max_nodes)));
            };
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
        if let Some(err) = im.error.clone() {
            return Err(err);
        }
    } else {
        // An export of a one-layer document names the layer on the root element, its art (groups
        // among it) loose inside: the one layer takes that name.
        let children = im.children(top, base);
        if let Some(err) = im.error.clone() {
            return Err(err);
        }
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
    if let Some(err) = im.error {
        return Err(err);
    }
    if !im.sublayer_groups.is_empty() {
        for l in &mut im.doc.layers {
            promote_sublayers(Arc::make_mut(l), &im.sublayer_groups);
        }
    }
    im.doc.limit_depth_noting(&mut im.warnings);
    Ok((im.doc, im.warnings))
}

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
    /// How many objects were converted so far.
    converted_nodes: usize,
    /// Most objects the document may expand to.
    max_nodes: usize,
    /// An error encountered while converting objects (e.g. [`SvgError::TooLarge`]).
    error: Option<SvgError>,
    /// How many nested SVG images were converted so far.
    nested_svg_images: usize,
}
