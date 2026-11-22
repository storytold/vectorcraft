//! `.cdr` drawings of versions 1 to 27 (`.cdr`, and `.cdt` templates): the pages, layers,
//! guidelines, groups, curves, rectangles, ellipses, arcs and polygons with uniform fills and
//! outlines, their hidden and locked states, fill rules and uniform transparency that
//! `craft-cdr` reads, mapped to editable VectorCraft art. Pages become
//! artboards side by side; layers of the same name on different pages become one layer where the
//! pages stack them in the same order. What the
//! reader leaves out is listed in the warnings, one line for each kind ([`note`]). `.cmx` exchange
//! files with 32-bit coordinates are mapped the same way, with their own warnings ([`cmx_note`]).

use std::collections::HashMap;
use std::sync::Arc;

use craft_cdr::model::{self as cd, LayerKind};
use craft_cdr::{Limits, Warning};
use kurbo::{Affine, Rect};
use vectorcraft_doc::{Artboard, ColorMode, Document, Guide, LayerColor, Node, NodeKind};

use super::load::err;
use crate::Result;

mod shapes;

const UNSUPPORTED_OBJECTS: &str = "CDR text, bitmaps, blends, contours, brush strokes and other objects VectorCraft doesn't read yet were left out";
const UNSUPPORTED_FILLS: &str = "CDR fountain, pattern, texture and PostScript fills were left out";
const UNKNOWN_COLORS: &str = "colors in CDR color models VectorCraft doesn't read were opened as black";
const SPOT_LOOKS: &str = "CDR library colors that the drawing stores only by number were opened as black spot colors";
const ARROWHEADS: &str = "CDR outline arrowheads were left out";
const ENVELOPES: &str = "CDR envelopes were not applied, and the shapes under them keep their outlines";
const LENSES: &str = "CDR lenses and object transparency were not applied, and the objects they are on show opaque";
const TOO_DEEP: &str = "parts of the drawing nested too deeply, or past the import limits, were left out";
const DAMAGED: &str = "damaged parts of the drawing were left out";
const PAGES_CUT: &str = "CDR pages longer than 163,830 pt were opened at that length, and art near their edges lies outside the artboard";

const CMX_UNSUPPORTED_OBJECTS: &str =
    "CMX text, images, rectangles, ellipses, clipping paths and other objects VectorCraft doesn't read yet were left out";
const CMX_UNSUPPORTED_FILLS: &str = "CMX fills other than uniform colors were left out";
const CMX_UNKNOWN_COLORS: &str = "colors in CMX color models VectorCraft doesn't read were opened as black";
const CMX_SPOT_LOOKS: &str = "CMX library colors that the file stores only by number were opened as black spot colors";
const CMX_ARROWHEADS: &str = "CMX outline arrowheads were left out";
const CMX_LENSES: &str = "CMX lenses were not applied";
const CMX_PAGES_CUT: &str = "CMX pages longer than 163,830 pt were opened at that length, and art near their edges lies outside the artboard";

/// What reading a drawing left out or changed, for each kind the reader reports.
fn note(w: Warning) -> &'static str {
    match w {
        Warning::UnsupportedObjects => UNSUPPORTED_OBJECTS,
        Warning::UnsupportedFills => UNSUPPORTED_FILLS,
        Warning::UnknownColors => UNKNOWN_COLORS,
        Warning::SpotLooks => SPOT_LOOKS,
        Warning::Arrowheads => ARROWHEADS,
        Warning::Envelopes => ENVELOPES,
        Warning::Lenses => LENSES,
        Warning::TooDeep => TOO_DEEP,
        Warning::Damaged => DAMAGED,
    }
}

/// What reading a `.cmx` file left out or changed. The reader reports no envelopes in one.
fn cmx_note(w: Warning) -> &'static str {
    match w {
        Warning::UnsupportedObjects | Warning::Envelopes => CMX_UNSUPPORTED_OBJECTS,
        Warning::UnsupportedFills => CMX_UNSUPPORTED_FILLS,
        Warning::UnknownColors => CMX_UNKNOWN_COLORS,
        Warning::SpotLooks => CMX_SPOT_LOOKS,
        Warning::Arrowheads => CMX_ARROWHEADS,
        Warning::Lenses => CMX_LENSES,
        Warning::TooDeep => TOO_DEEP,
        Warning::Damaged => DAMAGED,
    }
}

/// The page size used when neither the page nor the document settings give one: US Letter,
/// VectorCraft's own choice.
const FALLBACK_PAGE: (f64, f64) = (612.0, 792.0);
/// The layer that holds guidelines at angles other than horizontal and vertical, as guide paths.
const GUIDES_LAYER: &str = "Guides";
/// The hidden layer that holds hidden guidelines.
const HIDDEN_GUIDES_LAYER: &str = "Hidden guides";
/// The most pages read, as artboards: the most a new document may have.
const MAX_PAGES: usize = crate::cmd::newdoc::MAX_ARTBOARDS as usize;
/// The longest page side, in points: the longest artboard side a new document may have.
const MAX_PAGE_SIDE: f64 = 16383.0 * 10.0;
/// The gap between pages laid side by side, in points.
const PAGE_GAP: f64 = 36.0;
/// The most objects, subpaths and path segments copied from master layers onto pages, all pages
/// together: the `.af` importer's budget for its master pages (`affinity/master_copies.rs`).
const MAX_MASTER_COPIES: usize = 1 << 23;
/// The most guides and guide paths made, the master page's copied onto every page included.
const MAX_GUIDES: usize = 100_000;
/// The most layers made from the master page's and the pages' layers: VectorCraft's own limit.
const MAX_LAYERS: usize = 10_000;

/// Is `bytes` a `.cdr` drawing this importer reads?
pub(super) fn is_cdr(bytes: &[u8]) -> bool {
    craft_cdr::is_cdr(bytes)
}

pub(super) fn import(bytes: &[u8]) -> Result<(Document, Vec<String>)> {
    let d = craft_cdr::read(bytes, Limits::default()).map_err(|e| match e {
        craft_cdr::Error::Version(v) => {
            let version = v / 100;
            err(format!("CDR {version} drawings can't be opened: VectorCraft reads versions 1 to 27"))
        }
        why => err(format!("this CDR drawing can't be read ({why})")),
    })?;
    Ok(build(&d, note, PAGES_CUT))
}

/// Is `bytes` a `.cmx` file, of any byte order or coordinate size?
pub(super) fn is_cmx(bytes: &[u8]) -> bool {
    craft_cdr::is_cmx(bytes)
}

/// A `.cmx` file with 32-bit coordinates: its pages, layers, groups and curves with uniform fills
/// and outlines, mapped as a `.cdr` drawing's are.
pub(super) fn import_cmx(bytes: &[u8]) -> Result<(Document, Vec<String>)> {
    let d = craft_cdr::read_cmx(bytes, Limits::default()).map_err(|why| err(format!("this CMX file can't be read ({why})")))?;
    Ok(build(&d, cmx_note, CMX_PAGES_CUT))
}

/// The document of `d`, and one warning for each kind of content left out or changed.
fn build(d: &cd::Drawing, note: fn(Warning) -> &'static str, pages_cut: &str) -> (Document, Vec<String>) {
    let mut b = shapes::Builder::new();
    let mut doc = b.document(d);
    let mut warnings: Vec<String> = Vec::new();
    for w in d.warnings.iter().chain(&b.notes) {
        if !warnings.iter().any(|x| x == note(*w)) {
            warnings.push(note(*w).to_string());
        }
    }
    if b.pages_cut {
        warnings.push(pages_cut.to_string());
    }
    doc.limit_depth_noting(&mut warnings);
    (doc, warnings)
}

/// Where each page lies in the document: its artboard, and the map from its own space (points
/// from its centre, y up) into the document.
struct Placed {
    rect: Rect,
    to_doc: Affine,
}

/// Pages laid out by [`place_pages`].
struct Layout {
    placed: Vec<Placed>,
    /// Pages past [`MAX_PAGES`] or past the document's range were left out.
    left_out: bool,
    /// A page side longer than [`MAX_PAGE_SIDE`] was cut to it.
    cut: bool,
}

/// The pages' artboards side by side, each side held to 1 to [`MAX_PAGE_SIDE`] points.
fn place_pages(d: &cd::Drawing) -> Layout {
    let mut x = 0.0;
    let size = |p: &cd::Page| p.size.or(d.page_size).unwrap_or(FALLBACK_PAGE);
    let sizes: Vec<(f64, f64)> =
        if d.pages.is_empty() { vec![d.page_size.unwrap_or(FALLBACK_PAGE)] } else { d.pages.iter().take(MAX_PAGES).map(size).collect() };
    let mut placed = Vec::with_capacity(sizes.len());
    let mut cut = false;
    for (w, h) in sizes {
        let long = w > MAX_PAGE_SIDE || h > MAX_PAGE_SIDE;
        let (w, h) = (w.clamp(1.0, MAX_PAGE_SIDE), h.clamp(1.0, MAX_PAGE_SIDE));
        if x + w > crate::MAX_COORD {
            break;
        }
        cut |= long;
        let rect = Rect::new(x, 0.0, x + w, h);
        placed.push(Placed { rect, to_doc: Affine::translate((x + w / 2.0, h / 2.0)) * Affine::FLIP_Y });
        x += w + PAGE_GAP;
    }
    let left_out = placed.len() < d.pages.len();
    Layout { placed, left_out, cut }
}

impl shapes::Builder {
    fn document(&mut self, d: &cd::Drawing) -> Document {
        self.learn_spots(d);
        let Layout { placed, left_out, cut } = place_pages(d);
        if left_out {
            self.notes.push(Warning::TooDeep);
        }
        self.pages_cut = cut;
        let (w, h) = placed.first().map_or(FALLBACK_PAGE, |p| (p.rect.width(), p.rect.height()));
        let mut doc = Document::new(w, h);
        doc.artboards = placed
            .iter()
            .enumerate()
            .map(|(i, p)| Artboard {
                id: i as u32 + 1,
                name: format!("Page {}", i + 1),
                rect: p.rect,
                locked: false,
                show_center_mark: false,
                show_cross_hairs: false,
                show_video_safe_areas: false,
                video_ruler_pixel_aspect_ratio: 1.0,
                color: None,
                guide_label: None,
            })
            .collect();
        self.doc = doc;
        self.name_spots();
        // Bottom to top: the master page's layers, then the pages' in the order first seen.
        let mut layers: Vec<LayerOut> = Vec::new();
        // A layer is known by its name and how many layers of that name the page has below it, so
        // two layers of one name on a page stay two layers. Each key keeps every layer made for
        // it, bottom to top.
        let mut by_name: HashMap<(String, usize), Vec<usize>> = HashMap::new();
        self.master_layers(d, &placed, &mut layers);
        // The pages' layers go above the master page's.
        let pages_from = layers.len();
        for (i, (page, p)) in d.pages.iter().zip(&placed).enumerate() {
            let artboard = Some(i as u32 + 1);
            let mut seen: HashMap<&str, usize> = HashMap::new();
            // The layer this page's last layer went to: the next one goes above it.
            let mut below: Option<usize> = None;
            // The file lists layers front to back.
            for (j, l) in page.layers.iter().rev().enumerate() {
                self.guides(&l.guides, p, artboard);
                if l.kind != LayerKind::Drawing && l.kind != LayerKind::Desktop {
                    continue;
                }
                let art = self.objects(&l.objects, p.to_doc);
                let n = seen.entry(l.name.as_str()).or_insert(0);
                let key = (l.name.clone(), *n);
                *n += 1;
                // A layer an earlier page stacked below this page's last one would put this page's
                // art in the wrong order, so the page gets a new layer of that name instead, just
                // above its last one. Of the layers of that name above the last one, the lowest
                // takes the art.
                match by_name.get(&key).and_then(|is| is.iter().copied().find(|&i| below.is_none_or(|b| i > b))) {
                    Some(i) => {
                        if let Some(out) = layers.get_mut(i) {
                            out.add(art, l.flags);
                        }
                        below = Some(i);
                    }
                    None if layers.len() >= MAX_LAYERS => self.notes.push(Warning::TooDeep),
                    None => {
                        // The page's bottom layer goes below the lowest layer that the rest of the
                        // page shares with earlier pages, so the page keeps its own order. When the
                        // page shares none, it goes on top, in the order the layers were first seen.
                        let at = match below {
                            Some(b) => b + 1,
                            None => {
                                let mut counts = seen.clone();
                                page.layers
                                    .iter()
                                    .rev()
                                    .skip(j + 1)
                                    .filter(|l| l.kind == LayerKind::Drawing || l.kind == LayerKind::Desktop)
                                    .filter_map(|l| {
                                        let n = counts.entry(l.name.as_str()).or_insert(0);
                                        let key = (l.name.clone(), *n);
                                        *n += 1;
                                        by_name.get(&key).and_then(|is| is.first().copied())
                                    })
                                    .min()
                                    .unwrap_or(layers.len())
                                    .max(pages_from)
                            }
                        };
                        if at < layers.len() {
                            for i in by_name.values_mut().flatten().filter(|i| **i >= at) {
                                *i += 1;
                            }
                        }
                        let is = by_name.entry(key).or_default();
                        is.insert(is.partition_point(|&i| i < at), at);
                        let mut out = LayerOut::new(l.name.clone());
                        out.add(art, l.flags);
                        layers.insert(at, out);
                        below = Some(at);
                    }
                }
            }
        }
        // Guidelines every page shows, on each page's artboard.
        for (i, p) in placed.iter().enumerate() {
            self.guides(&d.guides, p, Some(i as u32 + 1));
        }
        if !self.guide_paths.is_empty() {
            let paths = std::mem::take(&mut self.guide_paths);
            // At the layer limit the guide paths go on the top shown layer instead of a layer of
            // their own; when every layer is hidden they go on the top one, with the limit warning.
            let full = layers.len() >= MAX_LAYERS;
            let top = layers.iter().rposition(|l| !l.hidden()).unwrap_or(layers.len().saturating_sub(1));
            match layers.get_mut(top) {
                Some(out) if full => {
                    if out.hidden() {
                        self.notes.push(Warning::TooDeep);
                    }
                    out.kids.extend(paths.into_iter().map(|p| (p, cd::Flags::default())));
                }
                _ => {
                    let mut out = LayerOut::new(GUIDES_LAYER.to_string());
                    out.kids.extend(paths.into_iter().map(|p| (p, cd::Flags::default())));
                    layers.push(out);
                }
            }
        }
        // Hidden guidelines go on a hidden layer of their own on top; past the layer limit they
        // are left out.
        if !self.hidden_guides.is_empty() || !self.hidden_paths.is_empty() {
            if layers.len() >= MAX_LAYERS {
                self.notes.push(Warning::TooDeep);
            } else {
                let mut art = std::mem::take(&mut self.hidden_paths);
                for g in std::mem::take(&mut self.hidden_guides) {
                    let kind = NodeKind::RulerGuide { vertical: g.vertical, pos: g.pos, artboard: g.artboard };
                    art.push(Arc::new(Node::new(self.doc.alloc_id(), kind)));
                }
                let mut out = LayerOut::new(HIDDEN_GUIDES_LAYER.to_string());
                out.add(art, cd::Flags { hidden: true, locked: false });
                layers.push(out);
            }
        }
        let mut doc = std::mem::replace(&mut self.doc, Document::new(1.0, 1.0));
        doc.layers = layers.into_iter().enumerate().map(|(i, out)| out.finish(&mut doc, i)).collect();
        if doc.layers.is_empty() {
            let l = layer(&mut doc, "Layer 1", 0, Vec::new(), cd::Flags::default());
            doc.layers.push(l);
        }
        // Ruler guides go at the top of the top shown layer, in the order they were read. When
        // every layer is hidden they go on a layer of their own on top; past the layer limit they
        // go on the hidden top layer, with the limit warning.
        let guides: Vec<Arc<Node>> = std::mem::take(&mut self.guide_list)
            .into_iter()
            .map(|g| Arc::new(Node::new(doc.alloc_id(), NodeKind::RulerGuide { vertical: g.vertical, pos: g.pos, artboard: g.artboard })))
            .collect();
        if !guides.is_empty() && !doc.layers.iter().any(|l| l.visible) && doc.layers.len() < MAX_LAYERS {
            let i = doc.layers.len();
            let l = layer(&mut doc, GUIDES_LAYER, i, Vec::new(), cd::Flags::default());
            doc.layers.push(l);
        }
        let top = doc.layers.iter().rposition(|l| l.visible).unwrap_or(doc.layers.len().saturating_sub(1));
        if !guides.is_empty() && doc.layers.get(top).is_some_and(|l| !l.visible) {
            self.notes.push(Warning::TooDeep);
        }
        if let Some(kids) = doc.layers.get_mut(top).and_then(|l| Arc::make_mut(l).children_mut()) {
            kids.extend(guides);
        }
        self.finish_swatches(&mut doc);
        if self.cmyk > self.rgb {
            doc.color_mode = ColorMode::Cmyk;
        }
        doc
    }

    /// The master page's layers: the desktop's objects once, on the first page's space; other
    /// layers' objects on every page, as many copies as [`MAX_MASTER_COPIES`] allows.
    fn master_layers(&mut self, d: &cd::Drawing, placed: &[Placed], layers: &mut Vec<LayerOut>) {
        for l in d.master.iter().rev() {
            if layers.len() >= MAX_LAYERS {
                self.notes.push(Warning::TooDeep);
                break;
            }
            let frames: Vec<&Placed> = if l.kind == LayerKind::Desktop { placed.iter().take(1).collect() } else { placed.iter().collect() };
            let mut art = Vec::new();
            let count = copy_cost(&l.objects);
            for p in frames {
                if count > self.copies_left {
                    self.notes.push(Warning::TooDeep);
                    break;
                }
                self.copies_left -= count;
                art.extend(self.objects(&l.objects, p.to_doc));
            }
            let name = if l.name.is_empty() { "Master".to_string() } else { l.name.clone() };
            let mut out = LayerOut::new(name);
            out.add(art, l.flags);
            layers.push(out);
        }
    }

    /// `guides` of a page placed by `p`: horizontal and vertical ones as guides of the page's
    /// artboard, others as guide paths.
    fn guides(&mut self, guides: &[cd::Guideline], p: &Placed, artboard: Option<u32>) {
        for g in guides {
            if self.guide_list.len() + self.guide_paths.len() + self.hidden_guides.len() + self.hidden_paths.len() >= MAX_GUIDES {
                self.notes.push(Warning::TooDeep);
                return;
            }
            let at = p.to_doc * kurbo::Point::new(g.at.x, g.at.y);
            let turn = g.angle.rem_euclid(180.0);
            if !(at.x.is_finite() && at.y.is_finite() && at.x.abs() <= crate::MAX_COORD && at.y.abs() <= crate::MAX_COORD) {
                self.notes.push(Warning::TooDeep);
                continue;
            }
            let list = if g.hidden { &mut self.hidden_guides } else { &mut self.guide_list };
            if turn.abs() < 1e-9 {
                list.push(Guide { artboard, ..Guide::new(false, at.y) });
            } else if (turn - 90.0).abs() < 1e-9 {
                list.push(Guide { artboard, ..Guide::new(true, at.x) });
            } else {
                self.angled_guide(at, g.angle, p.rect, g.hidden);
            }
        }
    }
}

/// What one copy of `objects` takes: each object, each subpath and segment of its path, and each
/// dash of its outline, since every copy builds its own path and dash pattern.
fn copy_cost(objects: &[cd::Object]) -> usize {
    let mut n: usize = 0;
    let mut stack: Vec<&cd::Object> = objects.iter().collect();
    while let Some(o) = stack.pop() {
        n = n.saturating_add(1);
        match o {
            cd::Object::Group { objects: kids, .. } => stack.extend(kids),
            cd::Object::Shape(s) => n = n.saturating_add(shape_cost(s)),
            cd::Object::Clip { container, objects } => {
                // A container whose outline is behind its fill is built twice, as that outline
                // and as the clipping path, inside a group of their own.
                n = n.saturating_add(shape_cost(container));
                // The second path, without the dashes, which only the outline shape has, and the
                // group around the two.
                if container.outline.as_ref().is_some_and(|o| o.behind_fill) {
                    n = n.saturating_add(2 + path_cost(container));
                }
                stack.extend(objects);
            }
        }
    }
    n
}

/// What one copy of shape `s` takes past the object itself: its path's subpaths and segments and
/// its outline's dashes.
fn shape_cost(s: &cd::Shape) -> usize {
    path_cost(s).saturating_add(s.outline.as_ref().map_or(0, |o| o.dashes.len()))
}

/// The subpaths and segments of shape `s`'s path.
fn path_cost(s: &cd::Shape) -> usize {
    match &s.geometry {
        cd::Geometry::Path(p) => p.subpaths.iter().fold(0usize, |n, sp| n.saturating_add(1 + sp.segments.len())),
        _ => 0,
    }
}

/// A document layer being made from the drawing's layers of one name: its art bottom to top, each
/// part with the flags of the layer it came from, and those flags for every layer merged into it.
struct LayerOut {
    name: String,
    kids: Vec<(Arc<Node>, cd::Flags)>,
    merged: Vec<cd::Flags>,
}

impl LayerOut {
    fn new(name: String) -> Self {
        LayerOut { name, kids: Vec::new(), merged: Vec::new() }
    }

    /// Whether the layer node will be hidden: every drawing layer merged into it is.
    fn hidden(&self) -> bool {
        !self.merged.is_empty() && self.merged.iter().all(|f| f.hidden)
    }

    /// Adds the art of a drawing layer with flags `flags`.
    fn add(&mut self, art: Vec<Arc<Node>>, flags: cd::Flags) {
        self.kids.extend(art.into_iter().map(|n| (n, flags)));
        self.merged.push(flags);
    }

    /// The layer node, as the `i`th layer from the bottom. It is hidden or locked when every
    /// drawing layer merged into it is; otherwise the art of a hidden or locked drawing layer is
    /// hidden or locked object by object.
    fn finish(self, doc: &mut Document, i: usize) -> Arc<Node> {
        let all = |f: fn(&cd::Flags) -> bool| !self.merged.is_empty() && self.merged.iter().all(f);
        let own = cd::Flags { hidden: all(|f| f.hidden), locked: all(|f| f.locked) };
        let kids = self
            .kids
            .into_iter()
            .map(|(n, f)| shapes::with_flags(n, cd::Flags { hidden: f.hidden && !own.hidden, locked: f.locked && !own.locked }))
            .collect();
        layer(doc, &self.name, i, kids, own)
    }
}

/// A layer of the document named `name` holding `kids` (bottom to top), hidden or locked as
/// `flags` say.
fn layer(doc: &mut Document, name: &str, i: usize, kids: Vec<Arc<Node>>, flags: cd::Flags) -> Arc<Node> {
    let name = if name.is_empty() { format!("Layer {}", i + 1) } else { name.to_string() };
    let mut n = Node::layer(doc.alloc_id(), &name, LayerColor::Preset((i % 9) as u8));
    n.visible = !flags.hidden;
    n.locked = flags.locked;
    if let Some(c) = n.children_mut() {
        *c = kids;
    }
    Arc::new(n)
}
