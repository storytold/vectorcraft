//! Affinity documents (`.af`, `.afdesign`, `.afphoto`, `.afpub`): the native layers, curves,
//! shapes, artboards, text, tables, master pages, layer effects and images that
//! `vectorcraft-affinity` reads, mapped to editable VectorCraft art. Whatever the reader could not map is listed in the warnings. A file whose
//! native document can't be read opens as its embedded preview instead, with a warning that says
//! why, and can't be placed.

use std::io::Cursor;
use std::sync::Arc;

use kurbo::{Affine, BezPath, Point, Rect, Vec2};
use serde_json::json;
use vectorcraft_affinity::model::{self as af, Align as AfAlign, Blend, Kind, Pixels};
use vectorcraft_affinity::paint::{self as ap, Cap, GradientKind as AfKind, Join};
use vectorcraft_color::{BlendMode, Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::appearance::{Appearance, AppearanceItem, Dash, Effect, FillLayer, LineCap, LineJoin, StrokeAlign, StrokeLayer};
use vectorcraft_doc::geom::{FillRule, PathData, shapes};
use vectorcraft_doc::text::{CharStyle, FirstBaseline, Justify, TextKind, TextObject, TextRun, VerticalAlign};
use vectorcraft_doc::{Artboard, Document, ImageObject, LayerColor, Node, NodeKind, OpacityMask};

use super::load::{err, raster_doc};
use crate::Result;

/// What an Affinity file opened as.
pub(super) struct Imported {
    pub doc: Document,
    pub warnings: Vec<String>,
    /// Only the embedded preview: never placed, used as a template or mined for styles.
    pub preview_only: bool,
}

pub(super) fn import(name: &str, bytes: &[u8]) -> Result<Imported> {
    match vectorcraft_affinity::read(bytes, vectorcraft_affinity::Limits::default()) {
        Ok(d) => {
            let mut b = Builder::new(72.0 / d.dpi);
            b.document(&d)?;
            b.doc.limit_depth_noting(&mut b.warnings);
            let mut warnings = d.warnings.iter().map(|w| format!("Affinity: not imported or approximated: {w}.")).collect::<Vec<_>>();
            warnings.extend(b.warnings.iter().map(|w| format!("Affinity: {w}.")));
            Ok(Imported { doc: b.doc, warnings, preview_only: false })
        }
        Err(native) => {
            let (doc, w) = preview(name, bytes, &native)?;
            Ok(Imported { doc, warnings: vec![w], preview_only: true })
        }
    }
}

/// The embedded preview, for files whose native document can't be read.
fn preview(name: &str, bytes: &[u8], native: &vectorcraft_affinity::Error) -> Result<(Document, String)> {
    let p = vectorcraft_affinity::preview(bytes).map_err(|e| err(format!("{native}; its embedded preview can't be read either ({e})")))?;
    // `raster_doc` normally probes PNG dimensions and keeps its encoded bytes. Verify this
    // untrusted preview's complete pixels first, so corrupt data cannot create a blank image.
    let mut reader = image::ImageReader::with_format(Cursor::new(p.png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 << 20);
    reader.limits(limits);
    let _pixels = reader.decode().map_err(err)?;
    let d = raster_doc(name, p.png)?;
    Ok((
        d,
        format!(
            "The native Affinity document could not be read ({native}). Opened only its embedded {}×{} PNG preview, which may be smaller than the document: no layers, vectors, text or pages. Save a new copy; the Affinity source cannot be saved back. Export SVG or PDF from Affinity for editable vectors.",
            p.width, p.height
        ),
    ))
}

struct Builder {
    doc: Document,
    warnings: Vec<String>,
    /// Points per document pixel.
    k: f64,
    /// Layers made so far (their colours cycle).
    layer: u8,
    /// How deep inside embedded Affinity documents the node being mapped is: their artboards
    /// are only clip groups, never artboards of this document.
    embedded: usize,
    /// Each master spread's art as shown on pages, in the order the masters are first used.
    masters: Vec<(usize, Vec<Arc<Node>>)>,
    /// The document's CMYK profile, until [`Builder::document_cmyk`] makes its table.
    cmyk_profile: Option<Vec<u8>>,
    /// That table, once made (`Some(None)`: there is none).
    cmyk_lut: Option<Option<Arc<vectorcraft_color::cms::CmykLut>>>,
    /// The working CMYK profile's table, once a CMYK pixel layer needs it.
    working_cmyk: Option<Arc<vectorcraft_color::cms::CmykLut>>,
    /// What baking colour and gradient overlays may still take ([`overlay::MAX_WORK`]).
    overlay_work: u64,
    /// What copying master pages onto the pages that show them may still take.
    copy_left: CopyCost,
    /// Each master spread's objects as its pages show them, made the first time one does.
    master_parts: std::collections::HashMap<usize, Arc<[MasterPart]>>,
}

/// What copying a subtree takes: its nodes, characters and table cells (`work`), and its pixels.
#[derive(Clone, Copy, Debug, Default)]
struct CopyCost {
    work: u64,
    pixels: u64,
}

impl CopyCost {
    fn plus(self, o: CopyCost) -> CopyCost {
        CopyCost { work: self.work.saturating_add(o.work), pixels: self.pixels.saturating_add(o.pixels) }
    }
    fn minus(self, o: CopyCost) -> Option<CopyCost> {
        Some(CopyCost { work: self.work.checked_sub(o.work)?, pixels: self.pixels.checked_sub(o.pixels)? })
    }
}

/// Master page copies of one import together: as many objects, characters and pixels as a
/// document may hold, a few times over. A master shown on thousands of pages, or a huge one
/// on many, would otherwise multiply them.
const MAX_MASTER_COPIES: CopyCost = CopyCost { work: 1 << 23, pixels: 512 << 20 };

/// One object a master spread shows on the pages that show it: a top-level node of the spread
/// (`child` `None`) or one object in one of its layers, with its bounds and what copying it takes.
#[derive(Clone, Copy, Debug)]
struct MasterPart {
    node: usize,
    child: Option<usize>,
    bounds: Option<af::Rect>,
    cost: CopyCost,
}

fn copy_cost(n: &af::Node, depth: usize) -> CopyCost {
    if depth > MAX_DEPTH {
        return CopyCost::default();
    }
    let len = |runs: &[af::TextRun]| runs.iter().map(|r| r.text.len() as u64).sum::<u64>();
    let own = match &n.kind {
        Kind::Text(t) => CopyCost { work: 1 + len(&t.runs), pixels: 0 },
        Kind::Table(t) => CopyCost { work: 1 + t.cells.iter().map(|c| 1 + len(&c.runs)).sum::<u64>(), pixels: 0 },
        Kind::Image(img) => CopyCost { work: 1, pixels: u64::from(img.width) * u64::from(img.height) },
        _ => CopyCost { work: 1, pixels: 0 },
    };
    n.children.iter().fold(own, |c, ch| c.plus(copy_cost(ch, depth + 1)))
}

/// Deepest node tree mapped (the reader bounds trees well below this).
const MAX_DEPTH: usize = 160;

fn rect_of(r: af::Rect) -> Rect {
    Rect::new(r.x0, r.y0, r.x1, r.y1)
}

fn affine(a: af::Affine) -> Affine {
    Affine::new(a.0)
}

/// Where text goes: a rigid placement (translation and rotation) and the scale that maps node
/// space into it (applied to the frame's geometry, not the type), plus the type's own scale.
struct Placement {
    /// Node space to the frame's space, before `place`.
    to_frame: Affine,
    place: Affine,
    /// Points per node unit of type size.
    size_scale: f64,
    /// Horizontal scale of the type relative to `size_scale`.
    h_scale: f64,
}

impl Builder {
    fn new(k: f64) -> Self {
        Self {
            doc: Document::new(0.0, 0.0),
            warnings: Vec::new(),
            k,
            layer: 0,
            embedded: 0,
            masters: Vec::new(),
            cmyk_profile: None,
            cmyk_lut: None,
            working_cmyk: None,
            overlay_work: vectorcraft_affinity::overlay::MAX_WORK,
            copy_left: MAX_MASTER_COPIES,
            master_parts: std::collections::HashMap::new(),
        }
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn document(&mut self, d: &af::Document) -> Result<()> {
        self.cmyk_profile = d.cmyk_profile.clone();
        // Spreads (Publisher pages) sit side by side, 36 pt apart; a single spread keeps its origin.
        let mut layers = Vec::new();
        let mut artboards = Vec::new();
        let mut x = 0.0;
        for (i, spread) in d.spreads.iter().enumerate() {
            let shift = if i == 0 { 0.0 } else { x - spread.bounds.x0 * self.k };
            let to_doc = Affine::translate((shift, 0.0)) * Affine::scale(self.k);
            let before = artboards.len();
            let mut loose: Vec<Arc<Node>> = Vec::new();
            for n in &spread.nodes {
                let top = match &n.kind {
                    Kind::Layer => self.node(n, to_doc, &mut artboards, true, 0),
                    Kind::Artboard { .. } => {
                        self.node(n, to_doc, &mut artboards, true, 0).map(|art| self.layer_of(&n.name, art, n.visible, n.locked))
                    }
                    _ => {
                        if let Some(art) = self.node(n, to_doc, &mut artboards, false, 0) {
                            loose.push(art);
                        }
                        continue;
                    }
                };
                if !loose.is_empty() {
                    layers.push(self.layer_with(None, std::mem::take(&mut loose)));
                }
                layers.extend(top);
            }
            if !loose.is_empty() {
                layers.push(self.layer_with(None, loose));
            }
            self.master_pages(d, spread, to_doc);
            // Without artboards, each page (or the spread) becomes one.
            if artboards.len() == before {
                let rects = if spread.pages.is_empty() { vec![spread.bounds] } else { spread.pages.clone() };
                for r in rects {
                    let name = format!("{} {}", if d.spreads.len() > 1 || spread.pages.len() > 1 { "Page" } else { "Artboard" }, artboards.len() + 1);
                    artboards.push((name, to_doc.transform_rect_bbox(rect_of(r))));
                }
            }
            // (Earlier spreads' artboards are all left of `x` already.)
            x = artboards.get(before..).unwrap_or_default().iter().map(|(_, r)| r.x1).fold(x, f64::max) + 36.0;
            if spread.transparent && i == 0 {
                self.warn("transparent page backgrounds show as white artboards");
            }
        }
        self.doc.artboards = artboards
            .into_iter()
            .enumerate()
            .filter(|(_, (_, r))| r.width() > 0.0 && r.height() > 0.0)
            .map(|(i, (name, rect))| Artboard {
                id: i as u32 + 1,
                name,
                rect,
                locked: false,
                show_center_mark: false,
                show_cross_hairs: false,
                show_video_safe_areas: false,
                video_ruler_pixel_aspect_ratio: 1.0,
                color: None,
            })
            .collect();
        if self.doc.artboards.is_empty() {
            return Err(err("the Affinity document has no page with a size"));
        }
        // Master pages are behind everything on the pages that show them.
        let mut masters = std::mem::take(&mut self.masters);
        masters.sort_by_key(|(m, _)| *m);
        let mut bottom = Vec::new();
        for (m, art) in masters {
            let mut l = (*self.layer_with(Some(&format!("Master {}", master_prefix(m))), art)).clone();
            l.locked = true;
            bottom.push(Arc::new(l));
        }
        // Bottom to top, as Affinity lists them.
        bottom.extend(layers);
        self.doc.layers = bottom;
        if self.doc.layers.is_empty() {
            let empty = self.layer_with(None, Vec::new());
            self.doc.layers.push(empty);
        }
        Ok(())
    }

    /// The master pages `spread`'s pages show, placed on those pages (`to_doc` places the spread).
    /// Each master page shows only the objects on it, moved from its page to the one showing it.
    fn master_pages(&mut self, d: &af::Document, spread: &af::Spread, to_doc: Affine) {
        let instances = master_instances(&spread.nodes);
        if instances.is_empty() {
            return;
        }
        let owners = page_masters(&instances, spread.pages.len());
        for (pi, page) in spread.pages.iter().enumerate() {
            let Some(&Some((master, first, master_page))) = owners.get(pi) else { continue };
            let Some(m) = d.masters.get(master) else { continue };
            let index = master_page.saturating_add(pi.saturating_sub(first));
            let r = m.pages.get(index).or_else(|| m.pages.last()).copied().unwrap_or(m.bounds);
            let parts = self.master_parts(master, m);
            let on_page = |b: &Option<af::Rect>| b.is_some_and(|b| b.x1 > r.x0 && b.x0 < r.x1 && b.y1 > r.y0 && b.y0 < r.y1);
            // Looking through the master's objects takes one each, copying them what they hold.
            let looked = CopyCost { work: parts.len() as u64, pixels: 0 };
            let cost = parts.iter().filter(|p| on_page(&p.bounds)).fold(looked, |c, p| c.plus(p.cost));
            let Some(left) = self.copy_left.minus(cost) else {
                self.warn("master pages shown on more pages than one import copies them to (left out on the rest)");
                return;
            };
            self.copy_left = left;
            let place = to_doc * Affine::translate((page.x0 - r.x0, page.y0 - r.y0));
            let mut art = Vec::new();
            let mut boards = Vec::new();
            self.embedded += 1;
            let mut i = 0;
            while let Some(p) = parts.get(i).copied() {
                let Some(n) = m.nodes.get(p.node) else {
                    i += 1;
                    continue;
                };
                if p.child.is_none() {
                    if on_page(&p.bounds) {
                        art.extend(self.node(n, place, &mut boards, false, 0));
                    }
                    i += 1;
                    continue;
                }
                // A layer of the master spread: just its objects on this page.
                let mut kids: Vec<Arc<Node>> = Vec::new();
                while let Some(q) = parts.get(i).copied().filter(|q| q.node == p.node) {
                    if on_page(&q.bounds)
                        && let Some(c) = q.child.and_then(|c| n.children.get(c))
                    {
                        kids.extend(self.node(c, place, &mut boards, false, 1));
                    }
                    i += 1;
                }
                if !kids.is_empty() {
                    let mut g = self.base(n, NodeKind::Group { children: kids, clip: false });
                    g.isolate = n.blend != Blend::PassThrough;
                    art.push(Arc::new(g));
                }
            }
            self.embedded -= 1;
            if art.is_empty() {
                continue;
            }
            let mut g = Node::group(self.doc.alloc_id(), art);
            g.name = Some(format!("Master {} (page {})", master_prefix(master), pi + 1));
            match self.masters.iter_mut().find(|(i, _)| *i == master) {
                Some((_, v)) => v.push(Arc::new(g)),
                None => self.masters.push((master, vec![Arc::new(g)])),
            }
        }
    }

    /// The objects of master spread `m` (number `index`) as its pages show them: its layers'
    /// objects one by one and its other top-level objects, with their bounds and costs.
    fn master_parts(&mut self, index: usize, m: &af::Spread) -> Arc<[MasterPart]> {
        if let Some(parts) = self.master_parts.get(&index) {
            return parts.clone();
        }
        if !master_instances(&m.nodes).is_empty() {
            self.warn("master pages based on other master pages (only their own objects are used)");
        }
        let part = |node: usize, child: Option<usize>, n: &af::Node| MasterPart {
            node,
            child,
            bounds: af::nodes_bounds(std::slice::from_ref(n)),
            cost: copy_cost(n, 0),
        };
        let mut parts = Vec::new();
        for (i, n) in m.nodes.iter().enumerate() {
            if matches!(n.kind, Kind::Layer) {
                parts.extend(n.children.iter().enumerate().map(|(c, child)| part(i, Some(c), child)));
            } else {
                parts.push(part(i, None, n));
            }
        }
        let parts: Arc<[MasterPart]> = parts.into();
        self.master_parts.insert(index, parts.clone());
        parts
    }

    fn layer_with(&mut self, name: Option<&str>, children: Vec<Arc<Node>>) -> Arc<Node> {
        let id = self.doc.alloc_id();
        self.layer = self.layer.wrapping_add(1);
        let label = name.map(str::to_string).unwrap_or_else(|| format!("Layer {}", self.layer));
        let mut n = Node::layer(id, &label, LayerColor::Preset(self.layer % 9));
        if let Some(c) = n.children_mut() {
            *c = children;
        }
        Arc::new(n)
    }

    /// An artboard at the top level becomes a layer of its name holding its clip group.
    fn layer_of(&mut self, name: &str, art: Arc<Node>, visible: bool, locked: bool) -> Arc<Node> {
        let mut l = (*self.layer_with(Some(if name.is_empty() { "Artboard" } else { name }), vec![art])).clone();
        l.visible = visible;
        l.locked = locked;
        Arc::new(l)
    }

    fn base(&mut self, n: &af::Node, kind: NodeKind) -> Node {
        let mut out = Node::new(self.doc.alloc_id(), kind);
        out.name = (!n.name.is_empty()).then(|| n.name.clone());
        out.visible = n.visible;
        out.locked = n.locked;
        out.opacity = n.opacity as f32;
        out.blend = self.blend(n.blend);
        out
    }

    fn blend(&mut self, b: Blend) -> BlendMode {
        match b {
            Blend::Normal | Blend::PassThrough => BlendMode::Normal,
            Blend::Darken => BlendMode::Darken,
            Blend::Multiply => BlendMode::Multiply,
            Blend::ColorBurn => BlendMode::ColorBurn,
            Blend::Lighten => BlendMode::Lighten,
            Blend::Screen => BlendMode::Screen,
            Blend::ColorDodge => BlendMode::ColorDodge,
            Blend::Overlay => BlendMode::Overlay,
            Blend::SoftLight => BlendMode::SoftLight,
            Blend::HardLight => BlendMode::HardLight,
            Blend::Difference => BlendMode::Difference,
            Blend::Exclusion => BlendMode::Exclusion,
            Blend::Hue => BlendMode::Hue,
            Blend::Saturation => BlendMode::Saturation,
            Blend::Color => BlendMode::Color,
            Blend::Luminosity => BlendMode::Luminosity,
            _ => {
                self.warn("blend modes VectorCraft doesn't have (Add, Linear Light, Vivid Light, Pin Light, Hard Mix, Subtract, Darker/Lighter Colour, Average, Negation, Reflect, Glow, Erase) import as Normal");
                BlendMode::Normal
            }
        }
    }

    /// `n` with its colour and gradient overlays baked into its paint and pixels (VectorCraft has
    /// no overlay effect).
    fn overlaid<'n>(&mut self, n: &'n af::Node) -> std::borrow::Cow<'n, af::Node> {
        // Each kind of message once, however often it is heard.
        let mut warnings: Vec<String> = Vec::new();
        let mut work = self.overlay_work;
        let out = vectorcraft_affinity::overlay::apply(n, &mut work, &mut |w| {
            if !warnings.iter().any(|x| x == w) {
                warnings.push(w.to_string());
            }
        });
        self.overlay_work = work;
        for w in warnings {
            self.warn(&w);
        }
        out
    }

    /// One Affinity node (and its subtree) as VectorCraft art; `to_doc` maps document pixels to
    /// points. `layer_level`: the node is directly in a spread or a layer, so a layer stays a layer.
    fn node(&mut self, n: &af::Node, to_doc: Affine, boards: &mut Vec<(String, Rect)>, layer_level: bool, depth: usize) -> Option<Arc<Node>> {
        if depth > MAX_DEPTH {
            self.warn("objects nested too deeply (left out)");
            return None;
        }
        // VectorCraft documents stay within `MAX_COORD` of the origin (renderers and exports rely
        // on it): objects reaching beyond it are left out.
        if let Some(b) = af::kind_bounds(&n.kind) {
            let r = to_doc.transform_rect_bbox(rect_of(b));
            if ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= crate::MAX_COORD) {
                self.warn("objects beyond the document range VectorCraft supports (left out)");
                return None;
            }
        }
        let n = &*self.overlaid(n);
        // Register before descending so nested artboards keep their document order. Inside an
        // embedded document an artboard is just its content's frame.
        if let Kind::Artboard { rect, .. } = &n.kind
            && self.embedded == 0
        {
            boards.push((
                if n.name.is_empty() { format!("Artboard {}", boards.len() + 1) } else { n.name.clone() },
                to_doc.transform_rect_bbox(rect_of(*rect)),
            ));
        }
        let child_level = matches!(n.kind, Kind::Layer) && layer_level;
        let embeds = n.class == vectorcraft_affinity::stream::Tag::of(b"EmbN");
        self.embedded += usize::from(embeds);
        let children: Vec<Arc<Node>> = n.children.iter().filter_map(|c| self.node(c, to_doc, boards, child_level, depth + 1)).collect();
        self.embedded -= usize::from(embeds);
        let mut out = match &n.kind {
            Kind::Layer if layer_level => {
                let mut l = (*self.layer_with(Some(if n.name.is_empty() { "Layer" } else { &n.name }), children)).clone();
                l.visible = n.visible;
                l.locked = n.locked;
                l.opacity = n.opacity as f32;
                l.blend = self.blend(n.blend);
                l
            }
            Kind::Layer | Kind::Group => {
                let mut g = self.base(n, NodeKind::Group { children, clip: false });
                // Affinity groups isolate their blending unless they pass through.
                g.isolate = n.blend != Blend::PassThrough;
                g
            }
            Kind::Shape { path, fills, strokes, even_odd } => {
                let data = self.path(path, to_doc);
                let rule = if *even_odd { FillRule::EvenOdd } else { FillRule::NonZero };
                let mut appearance = self.appearance(fills, strokes, to_doc);
                // An outline effect on a shape is one more stroke.
                self.outline_stroke(&n.effects, &mut appearance);
                if children.is_empty() {
                    let mut p = self.base(n, NodeKind::Path { path: data, rule, live: None, clipping: false, guide: false });
                    p.appearance = appearance.into();
                    p
                } else {
                    // A shape clips its children: VectorCraft's clipping path keeps its paint, its
                    // fill behind the clipped art and its stroke over it, as Affinity draws it.
                    let mut clip = Node::new(self.doc.alloc_id(), NodeKind::Path { path: data, rule, live: None, clipping: true, guide: false });
                    clip.appearance = appearance.into();
                    let mut kids = vec![Arc::new(clip)];
                    kids.extend(children);
                    self.base(n, NodeKind::Group { children: kids, clip: true })
                }
            }
            Kind::Artboard { path, even_odd, background, strokes, .. } => {
                let mut clip = Node::new(
                    self.doc.alloc_id(),
                    NodeKind::Path {
                        path: self.path(path, to_doc),
                        rule: if *even_odd { FillRule::EvenOdd } else { FillRule::NonZero },
                        live: None,
                        clipping: true,
                        guide: false,
                    },
                );
                clip.appearance = self.appearance(background, strokes, to_doc).into();
                let mut kids = vec![Arc::new(clip)];
                kids.extend(children);
                let mut g = self.base(n, NodeKind::Group { children: kids, clip: true });
                g.name = Some(if n.name.is_empty() { "Artboard".into() } else { n.name.clone() });
                g
            }
            Kind::Text(t) => {
                let Some((mut text, alpha)) = self.text(t, to_doc) else {
                    // The text is left out (warned); what it holds still comes in.
                    if children.is_empty() {
                        return None;
                    }
                    let g = self.base(n, NodeKind::Group { children, clip: false });
                    return Some(Arc::new(g));
                };
                self.outline_text(&n.effects, &mut text);
                let mut node = self.base(n, NodeKind::Text(Box::new(text)));
                node.opacity *= alpha;
                if !children.is_empty() {
                    let mut kids = vec![Arc::new(node)];
                    kids.extend(children);
                    node = Node::group(self.doc.alloc_id(), kids);
                }
                node
            }
            Kind::Table(t) => {
                let mut kids = self.table(t, to_doc);
                if kids.is_empty() && children.is_empty() {
                    return None;
                }
                kids.extend(children);
                let mut g = self.base(n, NodeKind::Group { children: kids, clip: false });
                g.name.get_or_insert_with(|| "Table".into());
                g
            }
            Kind::Image(img) => match self.image(img, to_doc) {
                Some(kind) => self.base(n, kind),
                None if children.is_empty() => return None,
                None => self.base(n, NodeKind::Group { children, clip: false }),
            },
            // Shown on the pages through the master's layer (see `master_pages`).
            Kind::MasterInstance { .. } => return None,
            Kind::Unsupported if children.is_empty() => return None,
            Kind::Unsupported => self.base(n, NodeKind::Group { children, clip: false }),
        };
        if !matches!(n.kind, Kind::Shape { .. } | Kind::Text(_)) && n.effects.iter().any(|e| matches!(e, af::Effect::Outline { .. })) {
            self.warn("outline effects on groups, images and tables (left out)");
        }
        if let Some(img) = &n.pixel_mask {
            // A pixel mask: its grey levels show (white) or hide (black) the node, and nothing
            // shows outside it, as an opacity mask that clips.
            if let Some(kind) = self.image(img, to_doc) {
                let art = Node::new(self.doc.alloc_id(), kind);
                out.extras_mut().mask = Some(Box::new(OpacityMask::new(art, true)));
            }
        }
        if let Some(mask) = &n.mask {
            // A vector mask: the node shows only inside it.
            let clip = Node::new(
                self.doc.alloc_id(),
                NodeKind::Path { path: self.path(mask, to_doc), rule: FillRule::NonZero, live: None, clipping: true, guide: false },
            );
            let (opacity, blend, visible, name) = (out.opacity, out.blend, out.visible, out.name.clone());
            out.opacity = 1.0;
            out.blend = BlendMode::Normal;
            let mut g = Node::new(self.doc.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(out)], clip: true });
            (g.opacity, g.blend, g.visible, g.name) = (opacity, blend, visible, name);
            out = g;
        }
        // Shadows and glows apply to what the node shows, mask and all.
        let fx = self.effects(&n.effects);
        if !fx.is_empty() {
            out.appearance.effects.extend(fx);
        }
        Some(Arc::new(out))
    }

    /// Drop shadows, inner shadows and outer glows as VectorCraft's Stylize effects. Affinity's
    /// blur radius sets the same Gaussian as VectorCraft's blur (σ = half of it), and its angle
    /// is the direction the shadow goes in.
    fn effects(&mut self, effects: &[af::Effect]) -> Vec<Effect> {
        let k = self.k;
        let len = move |v: f64| if v.is_finite() { (v * k).clamp(0.0, 1000.0) } else { 0.0 };
        let num = |v: f64| (v * 1000.0).round() / 1000.0;
        let mut out = Vec::new();
        for e in effects {
            let (id, params) = match e {
                af::Effect::DropShadow { color, opacity, radius, distance, angle } => {
                    let (color, alpha) = Self::color(*color);
                    let d = len(*distance);
                    let a = if angle.is_finite() { *angle } else { 0.0 };
                    let p = json!({
                        "mode": "normal",
                        "opacity": num(opacity * f64::from(alpha) * 100.0),
                        "x": num(d * a.cos()),
                        "y": num(d * a.sin()),
                        "blur": num(len(*radius)),
                        "color": color,
                    });
                    ("stylize.dropShadow", p)
                }
                af::Effect::InnerShadow { color, opacity, radius, distance, .. } => {
                    self.warn("inner shadows (imported as inner glows at the edge, without their offset)");
                    let (color, alpha) = Self::color(*color);
                    let p = json!({
                        "mode": "normal",
                        "opacity": num(opacity * f64::from(alpha) * 100.0),
                        "blur": num(len(*radius) + len(*distance)),
                        "color": color,
                        "source": "edge",
                    });
                    ("stylize.innerGlow", p)
                }
                af::Effect::OuterGlow { color, opacity, radius } => {
                    let (color, alpha) = Self::color(*color);
                    let p =
                        json!({ "mode": "normal", "opacity": num(opacity * f64::from(alpha) * 100.0), "blur": num(len(*radius)), "color": color });
                    ("stylize.outerGlow", p)
                }
                af::Effect::Bevel { .. } => {
                    self.warn("bevel and emboss effects (left out)");
                    continue;
                }
                // Outlines become strokes (`outline_stroke`, `outline_text`), overlays are baked
                // into the paint (`overlaid`).
                af::Effect::Outline { .. } | af::Effect::ColorOverlay { .. } | af::Effect::GradientOverlay { .. } => continue,
            };
            out.push(Effect { id: id.into(), params, visible: true });
        }
        out
    }

    /// An outline effect on a shape: a stroke layer. Affinity outlines what the shape shows, its own
    /// strokes included: an outline outside goes under them (from the path out to as far past them as
    /// it is wide), one centred on or inside that edge over them. A centred one reaches as far into
    /// the shape as out of it here, which only differs by the width of the shape's own stroke.
    fn outline_stroke(&mut self, effects: &[af::Effect], appearance: &mut Appearance) {
        // How far the shape's own strokes reach outside its path.
        let reach = appearance
            .items
            .iter()
            .filter_map(|i| match i {
                AppearanceItem::Stroke(s) => Some(match s.align {
                    StrokeAlign::Center => s.width / 2.0,
                    StrokeAlign::Outside => s.width,
                    StrokeAlign::Inside => 0.0,
                }),
                _ => None,
            })
            .filter(|w| w.is_finite())
            .fold(0.0, f64::max);
        for e in effects {
            let af::Effect::Outline { color, opacity, width, align } = e else { continue };
            let w = width * self.k;
            if !(w.is_finite() && w > 0.0) {
                continue;
            }
            let (c, alpha) = Self::color(*color);
            let (align, w) = match align {
                1 => (StrokeAlign::Inside, w),
                2 => (StrokeAlign::Center, w + 2.0 * reach),
                _ => (StrokeAlign::Outside, w + reach),
            };
            let mut l = StrokeLayer::new(Paint::solid(c), w.min(1000.0));
            l.opacity = (opacity.clamp(0.0, 1.0) as f32) * alpha;
            l.join = LineJoin::Round;
            l.align = align;
            let item = AppearanceItem::Stroke(l);
            if align == StrokeAlign::Outside {
                // Under the shape's strokes, over its fills.
                let at = appearance.items.iter().position(|i| matches!(i, AppearanceItem::Stroke(_))).unwrap_or(appearance.items.len());
                appearance.items.insert(at, item);
            } else {
                appearance.items.push(item);
            }
        }
    }

    /// An outline effect on text: the characters' stroke, where they have none of their own.
    fn outline_text(&mut self, effects: &[af::Effect], text: &mut TextObject) {
        for e in effects {
            let af::Effect::Outline { color, opacity, width, align } = e else { continue };
            let w = width * self.k;
            if !(w.is_finite() && w > 0.0) {
                continue;
            }
            let (c, alpha) = Self::color(*color);
            if opacity * f64::from(alpha) < 0.999 {
                self.warn("semi-transparent outline effects on text (imported opaque)");
            }
            if *align != 2 {
                self.warn("outline effects on text inside or outside the characters (centred on them)");
            }
            for r in &mut text.runs {
                if r.style.stroke == Paint::None {
                    r.style.stroke = Paint::solid(c);
                    // Centred on the outline, so twice as wide to show as far outside.
                    r.style.stroke_width = if *align == 2 { w } else { 2.0 * w }.min(1000.0);
                }
            }
        }
    }

    fn path(&self, p: &af::Path, to_doc: Affine) -> PathData {
        let mut bp = BezPath::new();
        for s in &p.subpaths {
            let pt = |q: af::Point| Point::new(q.x, q.y);
            bp.move_to(pt(s.start));
            for [c1, c2, e] in &s.segments {
                if c1 == c2 && c2 == e {
                    bp.line_to(pt(*e));
                } else {
                    bp.curve_to(pt(*c1), pt(*c2), pt(*e));
                }
            }
            if s.closed {
                bp.close_path();
            }
        }
        PathData::from_bezpath(&(to_doc * bp))
    }

    fn color(c: ap::Color) -> (Color, f32) {
        let f = |v: f64| if v.is_finite() { v.clamp(0.0, 1.0) as f32 } else { 0.0 };
        match c {
            ap::Color::Rgb { r, g, b, a } => (Color::Rgb { r: f(r), g: f(g), b: f(b) }, f(a)),
            ap::Color::Cmyk { c, m, y, k, a } => (Color::Cmyk { c: f(c), m: f(m), y: f(y), k: f(k) }, f(a)),
            ap::Color::Gray { v, a } => (Color::Gray { k: 1.0 - f(v) }, f(a)),
            ap::Color::Lab { l, a, b, alpha } => (Color::Lab { l: l as f32, a: a as f32, b: b as f32 }, f(alpha)),
        }
    }

    /// A paint and the opacity its colour's alpha becomes.
    fn paint(&mut self, p: &ap::Paint, to_doc: Affine) -> Option<(Paint, f32)> {
        match p {
            ap::Paint::None => None,
            ap::Paint::Solid(c) => {
                let (color, a) = Self::color(*c);
                Some((Paint::solid(color), a))
            }
            ap::Paint::Gradient(g) => {
                let stops = g
                    .stops
                    .iter()
                    .map(|s| {
                        let (color, opacity) = Self::color(s.color);
                        let mut stop = GradientStop::new(s.offset as f32, color);
                        stop.opacity = opacity;
                        stop.midpoint = s.half_point().clamp(0.13, 0.87) as f32;
                        stop
                    })
                    .collect();
                let m = to_doc * affine(g.transform);
                let (o, x, y) = (m * Point::ZERO, m * Point::new(1.0, 0.0), m * Point::new(0.0, 1.0));
                let kind = match g.kind {
                    AfKind::Linear => GradientKind::Linear,
                    AfKind::Radial => GradientKind::Radial,
                    AfKind::Conical => {
                        self.warn("conical gradients import as radial gradients");
                        GradientKind::Radial
                    }
                };
                let (u, v): (Vec2, Vec2) = (x - o, y - o);
                if kind == GradientKind::Radial && u.hypot() > 0.0 && (u.dot(v) / (u.hypot() * v.hypot().max(1e-12))).abs() > 0.01 {
                    self.warn("skewed elliptical gradients import as upright ellipses");
                }
                let aspect = if u.hypot() > 0.0 { (v.hypot() / u.hypot()).clamp(0.01, 100.0) } else { 1.0 };
                let geom = GradientGeom { start: o, end: x, aspect: if kind == GradientKind::Radial { aspect } else { 1.0 }, focal: None };
                let mut gp = GradientPaint::new(Gradient::new(kind, stops));
                gp.angle = (-(u.y)).atan2(u.x).to_degrees();
                gp.geom = Some(geom);
                Some((Paint::Gradient(Box::new(gp)), 1.0))
            }
        }
    }

    fn stroke_layer(&mut self, s: &ap::Stroke, to_doc: Affine) -> Option<StrokeLayer> {
        let (paint, opacity) = self.paint(&s.paint, to_doc)?;
        let mut l = StrokeLayer::new(paint, s.width * self.k);
        l.opacity = opacity;
        l.cap = match s.cap {
            Cap::Butt => LineCap::Butt,
            Cap::Round => LineCap::Round,
            Cap::Square => LineCap::Square,
        };
        l.join = match s.join {
            Join::Miter => LineJoin::Miter,
            Join::Round => LineJoin::Round,
            Join::Bevel => LineJoin::Bevel,
        };
        l.miter_limit = s.miter_limit;
        l.align = match s.align {
            ap::Align::Center => StrokeAlign::Center,
            ap::Align::Inside => StrokeAlign::Inside,
            ap::Align::Outside => StrokeAlign::Outside,
        };
        l.dash = s.dash.as_ref().map(|(pattern, phase)| Dash {
            pattern: pattern.iter().map(|v| v * self.k).collect(),
            offset: phase * self.k,
            align_corners: false,
        });
        Some(l)
    }

    fn appearance(&mut self, fills: &[ap::Paint], strokes: &[ap::Stroke], to_doc: Affine) -> Appearance {
        let mut items = Vec::new();
        let mut above = Vec::new();
        for f in fills {
            if let Some((paint, opacity)) = self.paint(f, to_doc) {
                let mut l = FillLayer::new(paint);
                l.opacity = opacity;
                items.push(AppearanceItem::Fill(l));
            }
        }
        for s in strokes {
            let Some(l) = self.stroke_layer(s, to_doc) else { continue };
            if s.behind {
                items.insert(0, AppearanceItem::Stroke(l));
            } else {
                above.push(AppearanceItem::Stroke(l));
            }
        }
        items.extend(above);
        Appearance { items, ..Appearance::default() }
    }

    /// Where text placed by `m` (node space to points) goes, its type scaled by `type_m`.
    fn placement(&mut self, m: Affine, type_m: Affine) -> Option<Placement> {
        let [a, b, c, d, e, f] = m.as_coeffs();
        let sx = a.hypot(b);
        let det = a * d - b * c;
        if !(sx > 1e-9 && det.is_finite() && det.abs() > 1e-12 && e.is_finite() && f.is_finite()) {
            self.warn("text with an invalid transform (left out)");
            return None;
        }
        let sy = det / sx;
        let shear = (a * c + b * d) / (sx * sx);
        if shear.abs() > 1e-3 || sy < 0.0 {
            self.warn("skewed or flipped text (imported without the skew or flip)");
        }
        // The type's vertical scale gives the font size, its horizontal scale relative to that the
        // characters' width.
        let [ta, tb, tc, td, ..] = type_m.as_coeffs();
        let (tsx, tdet) = (ta.hypot(tb), ta * td - tb * tc);
        let (size_scale, h_scale) = if tsx > 1e-9 && tdet.is_finite() && tdet.abs() > 1e-12 {
            let tsy = (tdet / tsx).abs();
            (tsy, tsx / tsy)
        } else {
            self.warn("text with an invalid text scale (imported unscaled)");
            (det.abs().sqrt(), 1.0)
        };
        Some(Placement {
            to_frame: Affine::scale_non_uniform(sx, sy.abs()),
            place: Affine::translate((e, f)) * Affine::rotate(b.atan2(a)),
            size_scale,
            h_scale,
        })
    }

    /// Text → its object and the opacity its characters' shared transparency gives the node.
    fn text(&mut self, t: &af::Text, to_doc: Affine) -> Option<(TextObject, f32)> {
        let p = self.placement(to_doc * affine(t.transform), to_doc * affine(t.text_transform))?;
        let alpha = self.text_alpha(&t.runs);
        let mut obj = self.type_object(&t.runs, &p)?;
        obj.para.justify = justify(t.align);
        if let Some(first) = obj.paras.first_mut() {
            first.justify = obj.para.justify;
        }
        match t.frame {
            Some(f) => {
                let (a, b) = (p.to_frame * Point::new(f.x0, f.y0), p.to_frame * Point::new(f.x1, f.y1));
                let r = Rect::from_points(a, b);
                obj.xf = p.place * Affine::translate(r.origin().to_vec2());
                obj.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, r.width(), r.height())) };
                // Affinity hangs a frame's first line from its capitals' tops, not the font's ascent.
                obj.area.first_baseline = FirstBaseline::CapHeight;
                if fit_lines(&mut obj, r.width(), r.height()) {
                    self.warn("text frames whose last lines hang out of them (made taller so the lines show)");
                }
                // Affinity shows a line in a frame shorter than it (#1423); here none of it would draw.
                if vectorcraft_text::fit_first_line(vectorcraft_text::FontDb::global(), &mut obj) {
                    self.warn("text frames too short for their first line (made taller so the line shows)");
                }
            }
            None => {
                align_lines(&mut obj);
                obj.xf = p.place * Affine::translate((p.to_frame * Point::new(t.anchor.x, t.anchor.y)).to_vec2());
            }
        }
        Some((obj, alpha))
    }

    /// The opacity text takes from its characters' colour: VectorCraft's characters have none of
    /// their own, so text whose characters share one becomes an object with that opacity.
    fn text_alpha(&mut self, runs: &[af::TextRun]) -> f32 {
        let alphas: Vec<f64> =
            runs.iter().filter_map(|r| if let ap::Paint::Solid(c) = &r.fill { Some(c.alpha()) } else { None }).filter(|a| a.is_finite()).collect();
        let (lo, hi) = alphas.iter().fold((1.0f64, 0.0f64), |(lo, hi), a| (lo.min(*a), hi.max(*a)));
        if alphas.is_empty() || lo >= 1.0 {
            return 1.0;
        }
        if hi - lo > 0.01 {
            self.warn("text with characters of different transparency (imported opaque)");
            return 1.0;
        }
        lo.clamp(0.0, 1.0) as f32
    }

    /// The characters and paragraphs of `runs` as a text object at the origin of text space.
    fn type_object(&mut self, runs: &[af::TextRun], p: &Placement) -> Option<TextObject> {
        let (v, h) = (p.size_scale, p.size_scale * p.h_scale);
        let mut obj = TextObject::point(Point::ZERO, "", CharStyle::default());
        let mut formats: Vec<af::Paragraph> = Vec::new();
        let mut out: Vec<TextRun> = Vec::new();
        let mut paragraph = 0usize;
        // Whether each paragraph here continues the one before after a line break (U+2028):
        // VectorCraft has no line break inside a paragraph, so the lines become paragraphs.
        let mut continues = vec![false];
        for r in runs {
            for c in r.text.chars() {
                match c {
                    '\u{2028}' => continues.push(true),
                    '\u{2029}' | '\r' | '\n' => continues.push(false),
                    _ => {}
                }
            }
            // Affinity separates paragraphs with U+2029 and lines with U+2028; other control
            // characters don't show.
            let text: String = r
                .text
                .chars()
                .filter_map(|c| match c {
                    '\u{2029}' | '\u{2028}' | '\r' | '\n' => Some('\n'),
                    '\t' => Some('\t'),
                    c if c.is_control() => None,
                    c => Some(c),
                })
                .collect();
            if text.is_empty() {
                continue;
            }
            for c in text.chars() {
                if formats.len() <= paragraph {
                    formats.push(r.paragraph);
                }
                if c == '\n' {
                    paragraph += 1;
                }
            }
            let mut style = self.char_style(r, (v, p.h_scale));
            if style.leading.is_none() {
                style.leading = r.paragraph.leading.map(|l| l * v).filter(|l| l.is_finite() && *l > 0.0).map(|l| l.min(5000.0));
            }
            out.push(TextRun::new(text, style));
        }
        // Affinity ends a story with a paragraph mark; VectorCraft's last paragraph has none.
        if let Some(last) = out.last_mut()
            && last.text.ends_with('\n')
        {
            last.text.pop();
            if last.text.is_empty() {
                out.pop();
            }
        }
        obj.runs = out;
        let count = obj.runs.iter().map(|r| r.text.matches('\n').count()).sum::<usize>() + 1;
        let mut paras = Vec::with_capacity(count);
        let mut previous_after = 0.0;
        // The run of each paragraph's first character.
        let mut first_runs = Vec::with_capacity(count);
        {
            let mut at_start = true;
            for (i, r) in obj.runs.iter().enumerate() {
                for c in r.text.chars() {
                    if at_start {
                        first_runs.push(i);
                        at_start = false;
                    }
                    if c == '\n' {
                        at_start = true;
                    }
                }
            }
        }
        // Line gaps by font, each looked up once.
        let mut gaps: std::collections::HashMap<(&str, &str), f64> = std::collections::HashMap::new();
        for i in 0..count {
            let f = formats.get(i).or(formats.last()).copied().unwrap_or_default();
            let pt = |x: f64, k: f64| if (x * k).is_finite() { (x * k).clamp(-5000.0, 5000.0) } else { 0.0 };
            let mut ps = vectorcraft_doc::text::ParaStyle { justify: justify(f.align), ..Default::default() };
            // Affinity places the first line and the others from the left edge; VectorCraft's
            // first-line indent is relative to its left indent.
            ps.left_indent = pt(f.left, h);
            ps.first_line_indent = pt(f.first - f.left, h);
            ps.right_indent = pt(f.right, h);
            // Between two paragraphs Affinity keeps the larger of the space after the first and
            // before the second; VectorCraft adds them.
            let (mut before, mut after) = (pt(f.space_before, v).max(0.0), pt(f.space_after, v).max(0.0));
            // The lines of one Affinity paragraph: no space between them, and the first-line
            // indent only on its first.
            if continues.get(i).copied().unwrap_or(false) {
                before = 0.0;
                ps.first_line_indent = 0.0;
            }
            if continues.get(i + 1).copied().unwrap_or(false) {
                after = 0.0;
            }
            ps.space_before = if i == 0 { before } else { (before - previous_after).max(0.0) };
            ps.space_after = after;
            previous_after = after;
            // Automatic leading: the font size plus the font's line gap, times the paragraph's
            // factor, for artistic and frame text alike. Fitted on Affinity's thumbnails: Bebas
            // Neue Pro and Impact lines about 1 em apart, DIN Condensed (a gap of 0.2 em) 1.2 em,
            // and the public Arial frame fixture fits this better than the font's line height.
            let (family, style) =
                first_runs.get(i).and_then(|r| obj.runs.get(*r)).map_or(("", ""), |r| (r.style.font_family.as_str(), r.style.font_style.as_str()));
            let gap = *gaps.entry((family, style)).or_insert_with(|| line_gap(family, style));
            let ratio = (1.0 + gap) * f.auto_leading;
            // Affinity documents use tiny factors for spacer paragraphs (0.15 of a line); VectorCraft
            // takes 0–500 %.
            if ratio.is_finite() && (0.01..=5.0).contains(&ratio) {
                ps.justification.auto_leading = ratio * 100.0;
            }
            paras.push(ps);
        }
        if let Some(first) = paras.first() {
            obj.para = first.clone();
        }
        obj.paras = paras;
        obj.normalize_paras();
        Some(obj)
    }

    /// A run's character style; `(size_scale, h_scale)` the type's vertical scale to points and
    /// its horizontal scale relative to that.
    fn char_style(&mut self, r: &af::TextRun, (size_scale, h_scale): (f64, f64)) -> CharStyle {
        let finite = |v: f64, d: f64| if v.is_finite() { v } else { d };
        let mut style = CharStyle::default();
        if !r.family.is_empty() {
            style.font_family = r.family.clone();
        } else if !r.postscript.is_empty() {
            style.font_family = r.postscript.clone();
        } else {
            self.warn("text without a font name uses the default font");
        }
        // A family/weight pair cannot identify width variants such as Arial Narrow.
        // Resolve a known PostScript identity to the exact installed family and style.
        if let Some((family, face_style)) = vectorcraft_text::FontDb::global().by_postscript_name(&r.postscript) {
            style.font_family = family;
            style.font_style = face_style;
        } else {
            style.font_style = font_style(r.weight, r.italic);
        }
        style.features = r.features.clone();
        if r.features.iter().any(|feature| !vectorcraft_text::OtFeatures::known_tag(feature)) {
            self.warn("unsupported text OpenType features (retained but not rendered)");
        }
        style.all_caps = r.all_caps;
        style.underline = r.underline;
        style.strikethrough = r.strikethrough;
        style.size = finite(r.size * size_scale, 12.0).clamp(0.1, 1296.0);
        style.leading = r.leading.map(|l| finite(l * size_scale, 0.0)).filter(|l| *l > 0.0).map(|l| l.min(5000.0));
        style.tracking = finite(r.tracking * 1000.0, 0.0).clamp(-1000.0, 10000.0);
        // Text squeezed or stretched by its transform keeps that as horizontal scale, on top of
        // its own.
        let hs = h_scale * r.h_scale;
        if hs.is_finite() && (hs - 1.0).abs() > 1e-3 {
            style.h_scale = (hs * 100.0).clamp(1.0, 1000.0);
        }
        // Transparency is the object's (see `text_alpha`); the paint's transform doesn't matter
        // for a solid colour, and gradients on type are laid out in text space.
        style.fill = match self.paint(&r.fill, Affine::scale(size_scale)) {
            Some((p, _)) => p,
            None => Paint::None,
        };
        if let Some(st) = &r.stroke {
            let w = st.width * size_scale;
            if let Some((paint, _)) = self.paint(&st.paint, Affine::scale(size_scale))
                && w.is_finite()
                && w > 0.0
            {
                style.stroke = paint;
                style.stroke_width = w.min(1000.0);
            }
        }
        style
    }

    /// A table: its cells as area type and the lines on their edges, as VectorCraft has no
    /// tables.
    fn table(&mut self, t: &af::Table, to_doc: Affine) -> Vec<Arc<Node>> {
        let mut out = Vec::new();
        let (ncols, nrows) = (t.ncols(), t.nrows());
        if ncols == 0 || nrows == 0 {
            return out;
        }
        self.warn("tables (imported as text frames and lines)");
        let m = to_doc * affine(t.transform);
        let Some(p) = self.placement(m, to_doc * affine(t.text_transform)) else { return out };
        let col = |i: usize| t.columns.get(i).copied().unwrap_or(0.0);
        let row = |i: usize| t.rows.get(i).copied().unwrap_or(0.0);
        for (i, cell) in t.cells.iter().enumerate() {
            if cell.merged_left || cell.runs.is_empty() {
                continue;
            }
            let (r, c) = (i / ncols, i % ncols);
            // Cells to the right merged into this one widen it.
            let span = 1 + t.cells.iter().skip(i + 1).take(ncols - c - 1).take_while(|n| n.merged_left).count();
            let [l, tp, rt, bt] = cell.inset;
            let node = af::Rect { x0: col(c) + l, y0: row(r) + tp, x1: col(c + span) - rt, y1: row(r + 1) - bt };
            let (a, b) = (p.to_frame * Point::new(node.x0, node.y0), p.to_frame * Point::new(node.x1, node.y1));
            let rect = Rect::from_points(a, b);
            if !(rect.width() > 0.0 && rect.height() > 0.0) {
                continue;
            }
            let Some(mut obj) = self.type_object(&cell.runs, &p) else { continue };
            obj.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, rect.width(), rect.height())) };
            obj.area.vertical_align = match cell.valign {
                af::VAlign::Top => VerticalAlign::Top,
                af::VAlign::Center => VerticalAlign::Center,
                af::VAlign::Bottom => VerticalAlign::Bottom,
            };
            obj.area.first_baseline = FirstBaseline::CapHeight;
            let dy = place_cell(&mut obj, rect.width(), rect.height());
            obj.xf = p.place * Affine::translate(rect.origin().to_vec2() + Vec2::new(0.0, dy));
            // A cell's text never vanishes for want of a line's height.
            vectorcraft_text::fit_first_line(vectorcraft_text::FontDb::global(), &mut obj);
            let alpha = self.text_alpha(&cell.runs);
            let mut n = Node::new(self.doc.alloc_id(), NodeKind::Text(Box::new(obj)));
            n.opacity = alpha;
            out.push(Arc::new(n));
        }
        // Lines over the cells: the vertical edges row by row (none inside merged cells), then
        // the horizontal ones.
        let mut line = |b: &mut Self, s: &Option<ap::Stroke>, from: (f64, f64), to: (f64, f64)| {
            let Some(s) = s else { return };
            let Some(layer) = b.stroke_layer(s, to_doc) else { return };
            let mut bp = BezPath::new();
            bp.move_to(m * Point::new(from.0, from.1));
            bp.line_to(m * Point::new(to.0, to.1));
            let mut n = Node::new(
                b.doc.alloc_id(),
                NodeKind::Path { path: PathData::from_bezpath(&bp), rule: FillRule::NonZero, live: None, clipping: false, guide: false },
            );
            n.appearance = Appearance { items: vec![AppearanceItem::Stroke(layer)], ..Appearance::default() }.into();
            out.push(Arc::new(n));
        };
        for r in 0..nrows {
            for k in 0..=ncols {
                let merged = k > 0 && k < ncols && t.cells.get(r * ncols + k).is_some_and(|c| c.merged_left);
                if merged {
                    continue;
                }
                if let Some(s) = t.vertical.get(r * (ncols + 1) + k) {
                    line(self, s, (col(k), row(r)), (col(k), row(r + 1)));
                }
            }
        }
        for j in 0..=nrows {
            for k in 0..ncols {
                if let Some(s) = t.horizontal.get(j * ncols + k) {
                    line(self, s, (col(k), row(j)), (col(k + 1), row(j)));
                }
            }
        }
        out
    }

    /// A CMYK pixel layer as an image file. Through the document's own CMYK profile when it
    /// embeds one (as Affinity shows it): RGBA. Otherwise, opaque, a CMYK image that keeps its
    /// inks (shown and separated through the working CMYK profile, as placed CMYK images are);
    /// with transparency, RGBA through the working CMYK profile.
    fn cmyk_image(&mut self, width: u32, height: u32, cmyka: &[u8]) -> Option<Vec<u8>> {
        let n = (width as usize).checked_mul(height as usize)?;
        let px = cmyka.get(..n.checked_mul(5)?)?.as_chunks::<5>().0;
        if let Some(lut) = self.document_cmyk() {
            let rgba = px
                .iter()
                .flat_map(|p| {
                    let [r, g, b, _] = lut.apply8([p[0], p[1], p[2], p[3]]);
                    [r, g, b, p[4]]
                })
                .collect();
            return png(width, height, rgba);
        }
        if px.iter().all(|p| p[4] == 255) {
            let inks = px.iter().flat_map(|p| [p[0], p[1], p[2], p[3]]).collect();
            if let Some(blob) = vectorcraft_doc::cmyk::Inks::new(width, height, inks).and_then(|i| vectorcraft_doc::ImageBlob::cmyk_tiff(&i)) {
                return Some(Arc::unwrap_or_clone(blob.bytes));
            }
        }
        self.warn("CMYK pixel layers with transparency (converted to RGB through the working CMYK profile)");
        let lut = self
            .working_cmyk
            .get_or_insert_with(|| {
                Arc::new(vectorcraft_color::cms::CmykLut::build(|[c, m, y, k]| {
                    let [r, g, b] = Color::cmyk(c, m, y, k).to_rgb();
                    [r, g, b, 0.0]
                }))
            })
            .clone();
        let rgba = px
            .iter()
            .flat_map(|p| {
                let [r, g, b, _] = lut.apply8([p[0], p[1], p[2], p[3]]);
                [r, g, b, p[4]]
            })
            .collect();
        png(width, height, rgba)
    }

    /// The document's CMYK profile as a table from ink amounts to sRGB (relative colorimetric,
    /// with black-point compensation, as Affinity's pixel layers convert), made once; `None` when
    /// the document embeds none, or not one that converts.
    fn document_cmyk(&mut self) -> Option<Arc<vectorcraft_color::cms::CmykLut>> {
        use vectorcraft_color::cms::{self, CmykLut, IccProfile, Intent};
        if let Some(lut) = &self.cmyk_lut {
            return lut.clone();
        }
        let embedded = self.cmyk_profile.is_some();
        let lut = self.cmyk_profile.take().and_then(|bytes| {
            let profile = IccProfile::from_bytes(None, &bytes).ok()?;
            let srgb = cms::icc::builtin_rgb(cms::SRGB)?;
            profile.to_rgb_of(&srgb, &[0.0; 4], Intent::RelativeColorimetric, true)?;
            Some(Arc::new(CmykLut::build(|inks| {
                let [r, g, b] = profile
                    .to_rgb_of(&srgb, &inks, Intent::RelativeColorimetric, true)
                    .unwrap_or_else(|| Color::cmyk(inks[0], inks[1], inks[2], inks[3]).to_rgb());
                [r, g, b, 0.0]
            })))
        });
        if embedded && lut.is_none() {
            self.warn("CMYK pixel layers whose document's colour profile could not be read (shown through the working CMYK profile)");
        }
        self.cmyk_lut = Some(lut.clone());
        lut
    }

    fn image(&mut self, img: &af::Image, to_doc: Affine) -> Option<NodeKind> {
        let bytes = match &img.pixels {
            Pixels::Encoded(b) => b.clone(),
            Pixels::Rgba8(rgba) => png(img.width, img.height, rgba.clone())?,
            Pixels::Cmyka8(cmyka) => self.cmyk_image(img.width, img.height, cmyka)?,
        };
        let r = match super::load::raster_image(&bytes) {
            Ok(r) => r,
            Err(_) => {
                self.warn("an embedded image that could not be decoded was left out");
                return None;
            }
        };
        // The pixel grid maps onto the Affinity image's own size, whatever the stored file's.
        let sx = img.width as f64 / f64::from(r.width.max(1));
        let sy = img.height as f64 / f64::from(r.height.max(1));
        let xf = to_doc * affine(img.transform) * Affine::scale_non_uniform(sx, sy);
        self.doc.images.entry(r.key.clone()).or_insert(r.blob);
        Some(NodeKind::Image(Box::new(ImageObject {
            key: r.key,
            width: r.width,
            height: r.height,
            xf,
            link: None,
            placement: Default::default(),
            pixels: None,
        })))
    }
}

/// The master page instances among `nodes` (also inside layers and groups): (master, first page,
/// first master page, page count).
fn master_instances(nodes: &[af::Node]) -> Vec<(usize, usize, usize, usize)> {
    fn visit(n: &af::Node, depth: usize, out: &mut Vec<(usize, usize, usize, usize)>) {
        if depth > MAX_DEPTH || !n.visible {
            return;
        }
        match n.kind {
            Kind::MasterInstance { master, page, master_page, count } => out.push((master, page, master_page, count)),
            Kind::Layer | Kind::Group => n.children.iter().for_each(|c| visit(c, depth + 1, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    nodes.iter().for_each(|n| visit(n, 0, &mut out));
    out
}

/// Which master page instance each of `pages` pages shows: (master, first page, first master
/// page) of the first instance in `instances` that covers it. Each page is settled once (the
/// next page still open is found by a path-halving jump), so this is about linear in pages and
/// instances however much their ranges overlap.
fn page_masters(instances: &[(usize, usize, usize, usize)], pages: usize) -> Vec<Option<(usize, usize, usize)>> {
    fn open_from(next: &mut [usize], mut i: usize) -> usize {
        while let Some(&n) = next.get(i) {
            if n == i {
                break;
            }
            let skip = next.get(n).copied().unwrap_or(n);
            if let Some(slot) = next.get_mut(i) {
                *slot = skip;
            }
            i = n;
        }
        i
    }
    let mut owner = vec![None; pages];
    // `next[p]` leads towards the first page from `p` on with no master yet (`pages`: none).
    let mut next: Vec<usize> = (0..=pages).collect();
    for &(master, first, master_page, count) in instances {
        let end = first.saturating_add(count).min(pages);
        let mut p = open_from(&mut next, first.min(pages));
        while p < end {
            if let Some(o) = owner.get_mut(p) {
                *o = Some((master, first, master_page));
            }
            if let Some(n) = next.get_mut(p) {
                *n = p + 1;
            }
            p = open_from(&mut next, p + 1);
        }
    }
    owner
}

/// Master names: A, B … Z, then A1, B1 ….
fn master_prefix(index: usize) -> String {
    let letter = char::from(b'A' + (index % 26) as u8);
    match index / 26 {
        0 => letter.to_string(),
        n => format!("{letter}{n}"),
    }
}

/// A table cell's text `w` × `h` at the origin, placed as Affinity places it. Affinity aligns the
/// block from the first line's capitals' tops to the last baseline in the cell, and draws it even
/// where it doesn't fit (centred cells measured on a document's thumbnail: two lines in a cell
/// shorter than them reach out of it at both ends). VectorCraft aligns the block down to the last
/// line's descenders and keeps lines inside the frame: the frame moves by what this returns (down
/// the cell) and grows where the lines need more room.
fn place_cell(obj: &mut TextObject, w: f64, h: f64) -> f64 {
    let share = match obj.area.vertical_align {
        VerticalAlign::Center => 0.5,
        VerticalAlign::Bottom => 1.0,
        _ => 0.0,
    };
    let mut tall = obj.clone();
    tall.area.vertical_align = VerticalAlign::Top;
    tall.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, w, h * 4.0 + 10_000.0)) };
    let lines = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &tall).lines;
    let Some(last) = lines.iter().rfind(|l| l.region == 0) else { return 0.0 };
    let (baseline, descent) = (last.baseline, last.descent);
    if !(baseline.is_finite() && descent.is_finite() && h.is_finite()) {
        return 0.0;
    }
    // (A little more than the lines need, for the rounding of the layout.)
    let frame = h.max(baseline + descent + 0.05);
    obj.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, w, frame)) };
    // Where Affinity has the block's top, less where VectorCraft puts it in the frame.
    (h - baseline) * share - (frame - baseline - descent) * share
}

/// Area type `w` × `h` at the origin whose lines don't all fit. Affinity shows each line whose
/// baseline is inside the frame, its descenders hanging out of it; VectorCraft's area type leaves
/// such a line out. The frame is made taller, from its top, by what those lines need. Whether it was.
fn fit_lines(obj: &mut TextObject, w: f64, h: f64) -> bool {
    let db = vectorcraft_text::FontDb::global();
    if !(w > 0.0 && h > 0.0 && h.is_finite()) {
        return false;
    }
    let first = vectorcraft_text::layout(db, obj);
    if !first.overflow {
        return false;
    }
    // Where the lines go in a frame with room for all of them.
    let mut tall = obj.clone();
    tall.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, w, h * 4.0 + 10_000.0)) };
    let need = vectorcraft_text::layout(db, &tall)
        .lines
        .iter()
        .filter(|l| l.region == 0 && l.baseline <= h + 1e-6)
        .map(|l| l.baseline + l.descent)
        .filter(|b| b.is_finite())
        .fold(0.0, f64::max);
    if need <= h {
        return false;
    }
    let before = first.lines.len();
    // (A little more than they need, for the rounding of the layout.)
    let short = std::mem::replace(&mut obj.kind, TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, w, need + 0.05)) });
    // Something else keeps the lines out: as the source has it.
    if vectorcraft_text::layout(db, obj).lines.len() <= before {
        obj.kind = short;
        return false;
    }
    true
}

/// RGBA pixels as a PNG file.
fn png(width: u32, height: u32, rgba: Vec<u8>) -> Option<Vec<u8>> {
    let buf = image::RgbaImage::from_raw(width, height, rgba)?;
    let mut png = Vec::new();
    buf.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

/// Where a line of `w` starts in a span from 0 to `room` (as a share of `room - w`).
fn share(j: Justify) -> f64 {
    match j {
        Justify::Center | Justify::JustifyCenter => 0.5,
        Justify::Right | Justify::JustifyRight => 1.0,
        _ => 0.0,
    }
}

/// Artistic text whose paragraphs are aligned differently. Affinity aligns each line within the
/// width of the longest, and places that block at the anchor by the text's own alignment (its
/// first paragraph's); VectorCraft's point type aligns each line on the anchor. Indents move the
/// lines to where Affinity has them and keep their alignment: a centred line centres between its
/// indents and a right-aligned one ends at its right indent, whatever its width.
fn align_lines(obj: &mut TextObject) {
    let block = share(obj.para.justify);
    if obj.paras.iter().all(|p| share(p.justify) == block) {
        return;
    }
    let lines = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), obj).lines;
    let w = lines.iter().map(|l| l.x1 - l.x0).filter(|w| w.is_finite()).fold(0.0, f64::max);
    if !(w > 0.0 && w < 1e6) {
        return;
    }
    // The block's left edge, from the anchor.
    let left = -w * block;
    for p in &mut obj.paras {
        let at = share(p.justify);
        let shift = left + w * at;
        // A left-aligned line starts at its left indent, a right-aligned one ends at minus its right
        // indent, and a centred one centres on half their difference.
        if at < 0.25 {
            p.left_indent += shift;
        } else if at > 0.75 {
            p.right_indent -= shift;
        } else {
            p.left_indent += shift;
            p.right_indent -= shift;
        }
    }
    if let Some(first) = obj.paras.first() {
        obj.para = first.clone();
    }
}

fn justify(a: AfAlign) -> Justify {
    match a {
        AfAlign::Left => Justify::Left,
        AfAlign::Center => Justify::Center,
        AfAlign::Right => Justify::Right,
        AfAlign::Justify => Justify::JustifyLeft,
    }
}

/// A font's line gap in em, as VectorCraft will set it (the installed face, or the fallback for
/// a missing one); 0 when no font is loaded.
fn line_gap(family: &str, style: &str) -> f64 {
    let defaults = CharStyle::default();
    let family = if family.is_empty() { defaults.font_family.as_str() } else { family };
    let style = if style.is_empty() { defaults.font_style.as_str() } else { style };
    let Some(face) = vectorcraft_text::FontDb::global().face(family, style) else { return 0.0 };
    let upem = face.units_per_em();
    let gap = if upem > 0.0 { face.line_gap() / upem } else { 0.0 };
    if gap.is_finite() { gap.clamp(0.0, 2.0) } else { 0.0 }
}

fn font_style(weight: i64, italic: bool) -> String {
    let w = match weight {
        ..=150 => "Thin",
        151..=250 => "ExtraLight",
        251..=325 => "Light",
        326..=450 => "Regular",
        451..=550 => "Medium",
        551..=650 => "SemiBold",
        651..=750 => "Bold",
        751..=850 => "ExtraBold",
        _ => "Black",
    };
    match (w, italic) {
        ("Regular", true) => "Italic".into(),
        (w, true) => format!("{w} Italic"),
        (w, false) => w.into(),
    }
}
