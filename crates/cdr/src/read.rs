//! Reading: the document `tracedraw-cdr` builds (millimetres, origin at the page's bottom-left
//! corner, Y up) turned into editable VectorCraft art (points, Y down), one artboard per page, side
//! by side.

use std::collections::BTreeSet;
use std::sync::Arc;

use kurbo::{Affine, PathEl, Point, Rect, Shape as _, Vec2};
use tracedraw_core::document::{Layer as TdLayer, Shape as TdShape, ShapeKind, TextSpan};
use tracedraw_core::style::{Arrowhead, FountainKind, LineCap as TdCap, LineJoin as TdJoin};
use tracedraw_core::{Color as TdColor, Document as TdDocument, Fill, Stroke, TextAlign};
use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::text::{CharStyle, Justify, TextKind, TextObject, TextRun};
use vectorcraft_doc::{
    Appearance, AppearanceItem, Artboard, Dash, Document, FillLayer, ImageBlob, ImageObject, LayerColor, LineCap, LineJoin, Node, NodeId, NodeKind,
    StrokeLayer, Unit,
};
use vectorcraft_geom::{FillRule, PathData, shapes};

use crate::{CdrInfo, Error, Result, guarded};

/// Points per millimetre.
pub(crate) const PT_PER_MM: f64 = 72.0 / 25.4;
/// The gap between the artboards of neighbouring pages (pt).
const PAGE_GAP: f64 = 36.0;
/// The most shapes an import visits, the ones left out included (a hostile file can't exhaust memory
/// or time with them, as with a large symbol of invalid shapes placed many times).
const MAX_OBJECTS: usize = 1 << 20;
/// The deepest nesting of groups read.
const MAX_DEPTH: usize = 64;
/// The largest embedded image accepted (pixels per side).
const MAX_IMAGE_SIDE: u32 = 30_000;
/// The width of a hairline outline (0.0762 mm).
const HAIRLINE_PT: f64 = 0.216;

/// Import options.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportOptions {
    /// The art's scale: 1 = its physical size (CorelDRAW millimetres as points).
    pub scale: f64,
    /// Keep layers and groups; off, each page becomes one layer and groups (other than clipping
    /// groups) are ungrouped, every object keeping the hidden and locked state its layer and groups
    /// gave it. Non-printing layers stay layers of their own (objects have no print setting).
    pub preserve_layers: bool,
    /// Turn type into outlines (curves) on import.
    pub text_to_outlines: bool,
    /// List each effect or feature not imported in the warnings (off: one summary line).
    pub report_unsupported: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self { scale: 1.0, preserve_layers: true, text_to_outlines: false, report_unsupported: true }
    }
}

/// An imported CorelDRAW document.
pub struct Imported {
    pub document: Document,
    pub info: CdrInfo,
    /// Compatibility warnings: what was approximated, converted or left out.
    pub warnings: Vec<String>,
    /// The font families the file's type uses (the engine warns about missing ones).
    pub fonts: Vec<String>,
    pub pages: usize,
    pub layers: usize,
    pub objects: usize,
}

/// Reads the bytes with the upstream reader, contained.
pub(crate) fn parse(bytes: &[u8]) -> Result<(TdDocument, tracedraw_cdr::ParseReport)> {
    guarded("reading the document", || tracedraw_cdr::open_bytes(bytes, "CorelDRAW"))?.map_err(Error::from)
}

/// The objects in a list of shapes, counting what groups and clip frames hold.
pub(crate) fn count_shapes(shapes: &[TdShape]) -> usize {
    shapes
        .iter()
        .map(|s| {
            1 + match &s.kind {
                ShapeKind::Group { children } => count_shapes(children),
                ShapeKind::ClipFrame { frame, contents } => count_shapes(std::slice::from_ref(frame)) + count_shapes(contents),
                _ => 0,
            }
        })
        .sum()
}

/// Reads `bytes` as a CorelDRAW document and converts it into a VectorCraft [`Document`].
///
/// `title` names the document, `info` carries the version and container already detected, and `opts`
/// chooses the scale and how layers and type are handled. Everything the conversion can't represent
/// comes back as a warning rather than being dropped silently.
pub(crate) fn import(bytes: &[u8], title: &str, info: CdrInfo, opts: &ImportOptions) -> Result<Imported> {
    let scale = if opts.scale.is_finite() && opts.scale > 0.0 { opts.scale.clamp(1e-3, 1e3) } else { 1.0 };
    let (td, report) = parse(bytes)?;
    if td.pages.is_empty() {
        return Err(Error::Damaged("the document has no page".into()));
    }
    let mut r = Reader {
        doc: Document::new(1.0, 1.0),
        td: &td,
        opts,
        k: PT_PER_MM * scale,
        notes: Notes::default(),
        fonts: BTreeSet::new(),
        layers: 0,
        objects: 0,
        visited: 0,
    };
    r.doc.layers.clear();
    r.doc.artboards.clear();
    r.doc.units = Unit::Millimeters;
    r.doc.title = title.to_string();
    let mut x = 0.0;
    let multi = td.pages.len() > 1;
    for (i, page) in td.pages.iter().enumerate() {
        let (w, h) = (page.size.width, page.size.height);
        if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
            r.notes.warn(format!("page {} has an invalid size and was left out", i + 1));
            continue;
        }
        let (wp, hp) = (w * r.k, h * r.k);
        let rect = Rect::new(x, 0.0, x + wp, hp);
        r.doc.artboards.push(Artboard {
            id: r.doc.artboards.len() as u32 + 1,
            name: if page.name.trim().is_empty() { format!("Page {}", i + 1) } else { page.name.clone() },
            rect,
            locked: false,
            show_center_mark: false,
            show_cross_hairs: false,
            show_video_safe_areas: false,
            video_ruler_pixel_aspect_ratio: 1.0,
            color: None,
        });
        // CorelDRAW page (mm, Y up, origin at the bottom left) → document.
        let m = Affine::new([r.k, 0.0, 0.0, -r.k, x, hp]);
        let prefix = multi.then(|| format!("{}: ", r.doc.artboards.last().map_or(String::new(), |a| a.name.clone())));
        if let Some(bg) = &page.background {
            r.background(bg, rect, prefix.as_deref());
        }
        // Master layers draw under every page; here, under the first one.
        if i == 0 {
            for l in td.master.iter().filter(|l| !l.shapes.is_empty()) {
                if multi {
                    r.notes.warn("master layers were placed on the first page only");
                }
                r.layer(l, m, Some("Master: "));
            }
        }
        let first = r.doc.layers.len();
        for l in &page.layers {
            r.layer(l, m, prefix.as_deref());
        }
        // Off, even a page of one layer becomes the merged layer, its plain groups ungrouped.
        if !opts.preserve_layers && r.doc.layers.len() > first {
            r.merge_layers(first, prefix.as_deref().unwrap_or(""));
        }
        if !page.guides.is_empty() {
            r.notes.warn("page guides were not imported");
        }
        x += wp + PAGE_GAP;
    }
    if r.doc.artboards.is_empty() {
        return Err(Error::Damaged("no page has a valid size".into()));
    }
    if r.doc.layers.is_empty() {
        let id = r.doc.alloc_id();
        r.doc.layers.push(Arc::new(Node::layer(id, "Layer 1", LayerColor::Preset(0))));
    }
    if td.cmyk_profile.is_some() {
        r.notes.warn("the CMYK profile embedded in the file was not applied: CMYK values are kept and shown through the active profile");
    }
    if !td.inks.is_empty() {
        r.notes.warn("spot colours (named inks) became their CMYK process stand-ins");
    }
    if !td.fonts.is_empty() {
        r.notes.warn("fonts embedded in the file were not installed; type uses the system's fonts");
    }
    let mut warnings: Vec<String> = Vec::new();
    if r.objects == 0 && (!report.warnings.is_empty() || report.skipped_objects > 0) {
        warnings.push("no object could be read: the file looks damaged or uses records the reader doesn't know".into());
    }
    if report.skipped_objects > 0 {
        warnings.push(format!("{} object(s) the CDR reader doesn't recognise were left out", report.skipped_objects));
    }
    if opts.report_unsupported {
        warnings.extend(report.warnings.iter().map(|w| format!("CDR reader: {w}")));
        warnings.extend(r.notes.list);
    } else if !r.notes.list.is_empty() || !report.warnings.is_empty() {
        warnings.push(format!(
            "{} feature(s) of the file were approximated or not imported (turn on Report Unsupported Effects to list them)",
            r.notes.list.len() + report.warnings.len()
        ));
    }
    let mut doc = r.doc;
    doc.fix_next_id();
    doc.limit_depth_noting(&mut warnings);
    let layers = doc.layers.len();
    Ok(Imported { pages: doc.artboards.len(), layers, objects: r.objects, fonts: r.fonts.into_iter().collect(), document: doc, info, warnings })
}

/// Warnings, each once.
#[derive(Default)]
struct Notes {
    list: Vec<String>,
}

impl Notes {
    /// Records `w`, ignoring a repeat and stopping at 200 messages.
    fn warn(&mut self, w: impl Into<String>) {
        let w = w.into();
        if !self.list.contains(&w) && self.list.len() < 200 {
            self.list.push(w);
        }
    }
}

struct Reader<'a> {
    doc: Document,
    td: &'a TdDocument,
    opts: &'a ImportOptions,
    /// Document points per file millimetre (with the scale asked for).
    k: f64,
    notes: Notes,
    fonts: BTreeSet<String>,
    layers: u8,
    /// Objects made.
    objects: usize,
    /// Shapes visited, made or left out: what [`MAX_OBJECTS`] caps.
    visited: usize,
}

/// Whether every coefficient of `a` is finite: a damaged file can hold NaN or infinity.
fn finite_affine(a: Affine) -> bool {
    a.as_coeffs().iter().all(|v| v.is_finite())
}

impl Reader<'_> {
    /// The next free node id in the document being built.
    fn id(&mut self) -> NodeId {
        self.doc.alloc_id()
    }

    /// A layer node named `name` holding `children`, coloured from the preset palette in turn.
    fn new_layer(&mut self, name: &str, children: Vec<Arc<Node>>) -> Node {
        let id = self.id();
        let mut n = Node::layer(id, name, LayerColor::Preset(self.layers % 9));
        self.layers = self.layers.wrapping_add(1);
        if let Some(c) = n.children_mut() {
            *c = children;
        }
        n
    }

    /// Converts `l` into a document layer under the transform `m`, keeping its name (behind `prefix`),
    /// visibility, lock and print setting.
    fn layer(&mut self, l: &TdLayer, m: Affine, prefix: Option<&str>) {
        let children: Vec<Arc<Node>> = l.shapes.iter().filter_map(|s| self.shape(s, m, 0)).map(Arc::new).collect();
        let name = format!("{}{}", prefix.unwrap_or(""), if l.name.trim().is_empty() { "Layer" } else { l.name.trim() });
        let mut n = self.new_layer(&name, children);
        n.visible = l.visible;
        n.locked = l.locked;
        if let NodeKind::Layer { printable, .. } = &mut n.kind {
            *printable = l.printable;
        }
        self.doc.layers.push(Arc::new(n));
    }

    /// Merges the layers from `first` on into one, ungrouping plain groups.
    fn merge_layers(&mut self, first: usize, prefix: &str) {
        let page: Vec<Arc<Node>> = self.doc.layers.drain(first..).collect();
        let printable = |l: &Node| !matches!(l.kind, NodeKind::Layer { printable: false, .. });
        let mut flat = Vec::new();
        let mut kept = Vec::new();
        for l in page {
            if !printable(&l) {
                kept.push(l);
                continue;
            }
            // The layer's hidden and locked state moves onto its objects.
            for n in l.children().cloned().unwrap_or_default() {
                ungroup(n, l.visible, l.locked, &mut flat);
            }
        }
        if !kept.is_empty() {
            self.notes.warn("non-printing layers were kept as layers of their own when layers were merged");
        }
        if !flat.is_empty() || kept.is_empty() {
            let l = self.new_layer(&format!("{prefix}Layer 1"), flat);
            self.doc.layers.push(Arc::new(l));
        }
        self.doc.layers.extend(kept);
    }

    /// A page background: a rectangle on a locked layer of its own, under the art.
    fn background(&mut self, fill: &Fill, rect: Rect, prefix: Option<&str>) {
        let Some((paint, _)) = self.fill_paint(fill, rect) else { return };
        let mut n = Node::path(self.id(), PathData::from_bezpath(&rect.to_path(0.1)), Appearance::default());
        n.appearance = Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(paint))], ..Appearance::default() }.into();
        n.name = Some("Page Background".into());
        let mut l = self.new_layer(&format!("{}Page Background", prefix.unwrap_or("")), vec![Arc::new(n)]);
        l.locked = true;
        self.doc.layers.push(Arc::new(l));
    }

    /// A shape (and its subtree) as VectorCraft art; `m` maps the shape's page coordinates into
    /// the document.
    fn shape(&mut self, s: &TdShape, m: Affine, depth: usize) -> Option<Node> {
        if self.visited >= MAX_OBJECTS {
            self.notes.warn(format!("the file has more than {MAX_OBJECTS} objects: the rest were left out"));
            return None;
        }
        self.visited += 1;
        if depth > MAX_DEPTH {
            self.notes.warn("groups nested too deep were left out");
            return None;
        }
        if !finite_affine(s.transform) {
            self.notes.warn("objects with an invalid transform were left out");
            return None;
        }
        self.objects += 1;
        self.note_losses(s);
        let xf = m * s.transform;
        let mut out = match &s.kind {
            ShapeKind::Rect { .. } | ShapeKind::Ellipse { .. } | ShapeKind::Polygon { .. } | ShapeKind::Path { .. } => {
                let bp = xf * s.local_path();
                if !bp.elements().iter().all(el_finite) {
                    self.notes.warn("curves with invalid coordinates were left out");
                    return None;
                }
                let appearance = self.appearance(s, bp.bounding_box(), xf);
                let mut n = Node::path(self.id(), PathData::from_bezpath(&bp), appearance);
                if let NodeKind::Path { rule, .. } = &mut n.kind {
                    // CorelDRAW fills combined curves even-odd.
                    *rule = FillRule::EvenOdd;
                }
                n
            }
            ShapeKind::Group { children } => {
                let kids: Vec<Arc<Node>> = children.iter().filter_map(|c| self.shape(c, xf, depth + 1)).map(Arc::new).collect();
                if kids.is_empty() {
                    return None;
                }
                Node::group(self.id(), kids)
            }
            ShapeKind::ClipFrame { frame, contents } => {
                let clip_path = finite_affine(frame.transform).then(|| xf * frame.transform * frame.local_path());
                match clip_path.filter(|p| p.elements().iter().all(el_finite)) {
                    Some(clip_path) => {
                        let mut clip =
                            Node::path(self.id(), PathData::from_bezpath(&clip_path), self.appearance(frame, clip_path.bounding_box(), xf));
                        if let NodeKind::Path { clipping, .. } = &mut clip.kind {
                            *clipping = true;
                        }
                        let mut kids = vec![Arc::new(clip)];
                        kids.extend(contents.iter().filter_map(|c| self.shape(c, xf, depth + 1)).map(Arc::new));
                        Node::new(self.id(), NodeKind::Group { children: kids, clip: true })
                    }
                    None => {
                        // A frame with invalid numbers can't clip: its contents come in unclipped, as a
                        // plain group, as other shapes with invalid numbers are left out.
                        self.notes.warn("PowerClip frames with invalid coordinates were left out: their contents are not clipped");
                        let kids: Vec<Arc<Node>> = contents.iter().filter_map(|c| self.shape(c, xf, depth + 1)).map(Arc::new).collect();
                        if kids.is_empty() {
                            return None;
                        }
                        Node::group(self.id(), kids)
                    }
                }
            }
            ShapeKind::Table(_) | ShapeKind::SymbolInstance { .. } => {
                if matches!(s.kind, ShapeKind::Table(_)) {
                    self.notes.warn("tables were imported as groups of rectangles and type");
                } else {
                    self.notes.warn("symbol instances were imported as independent groups");
                }
                let parts = s.expand(&self.td.symbols);
                let kids: Vec<Arc<Node>> = parts.iter().filter_map(|c| self.shape(c, xf, depth + 1)).map(Arc::new).collect();
                if kids.is_empty() {
                    return None;
                }
                Node::group(self.id(), kids)
            }
            ShapeKind::Text { spans, origin, frame, align, on_path, .. } => {
                if on_path.is_some() {
                    self.notes.warn("type fitted to a path was imported as plain type, without the path");
                }
                let text = self.text(s, spans, *origin, frame.map(|f| (f.width, f.height)), *align, xf)?;
                let n = Node::new(self.id(), NodeKind::Text(Box::new(text)));
                if self.opts.text_to_outlines {
                    match vectorcraft_effects::outline_text(&n) {
                        Some(o) => o,
                        None => {
                            self.notes.warn("some type couldn't be outlined and stays editable");
                            n
                        }
                    }
                } else {
                    n
                }
            }
            ShapeKind::Bitmap { rect, width_px, height_px, png, .. } => self.bitmap(*rect, *width_px, *height_px, png, xf)?,
        };
        out.name = s.name.clone().filter(|n| !n.trim().is_empty());
        out.visible = s.visible;
        out.locked = s.locked;
        out.opacity = if s.opacity.is_finite() { s.opacity.clamp(0.0, 1.0) as f32 } else { 1.0 };
        Some(out)
    }

    /// Warns about everything `s` carries that the import can't represent as such: drop shadows, live
    /// effects, opacity masks and bitmap effects.
    fn note_losses(&mut self, s: &TdShape) {
        if s.shadow.is_some() {
            self.notes.warn("drop shadows were not imported");
        }
        if !s.effects.is_empty() || !s.parked_effects.is_empty() {
            self.notes.warn(
                "CorelDRAW live effects (blend, extrude, contour, envelope…) were not imported as effects; the shapes the file stores for them are",
            );
        }
        if s.opacity_mask.is_some() {
            self.notes.warn("opacity masks were not imported");
        }
        if s.raster_fx.is_some() {
            self.notes.warn("bitmap effects were imported applied to their image (not editable)");
        }
        if s.overprint_fill || s.overprint_outline {
            self.notes.warn("fill and outline overprint was not imported");
        }
    }

    /// The appearance of `s`: its fills and strokes as appearance items, a stroke marked as sitting
    /// behind the fill placed under it.
    fn appearance(&mut self, s: &TdShape, bounds: Rect, xf: Affine) -> Appearance {
        let mut items = Vec::new();
        let mut above = Vec::new();
        for f in std::iter::once(&s.fill).chain(&s.extra_fills) {
            if let Some((paint, opacity)) = self.fill_paint(f, bounds) {
                let mut l = FillLayer::new(paint);
                l.opacity = opacity;
                items.push(AppearanceItem::Fill(l));
            }
        }
        for st in s.stroke.iter().chain(&s.extra_strokes) {
            let Some(l) = self.stroke(st, bounds, xf) else { continue };
            if st.behind_fill {
                items.insert(0, AppearanceItem::Stroke(l));
            } else {
                above.push(AppearanceItem::Stroke(l));
            }
        }
        items.extend(above);
        Appearance { items, ..Appearance::default() }
    }

    /// Converts a file colour into a document colour.
    ///
    /// Non-finite components clamp to zero, a spot colour becomes its tint in CMYK, and the registration
    /// colour becomes CMYK 100/100/100/100 with a warning.
    fn color(&mut self, c: TdColor) -> Color {
        let f = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        match c {
            TdColor::Rgb { r, g, b } => Color::Rgb { r: f(r), g: f(g), b: f(b) },
            TdColor::Cmyk { c, m, y, k } => Color::Cmyk { c: f(c), m: f(m), y: f(y), k: f(k) },
            TdColor::Gray { v } => Color::Gray { k: 1.0 - f(v) },
            TdColor::Lab { l, a, b } if l.is_finite() && a.is_finite() && b.is_finite() => Color::Lab { l, a, b },
            TdColor::Spot { tint, c, m, y, k, .. } => {
                let t = f(tint);
                Color::Cmyk { c: f(c) * t, m: f(m) * t, y: f(y) * t, k: f(k) * t }
            }
            TdColor::Registration => {
                self.notes.warn("the registration colour became CMYK 100/100/100/100");
                Color::Cmyk { c: 1.0, m: 1.0, y: 1.0, k: 1.0 }
            }
            other => {
                // HSB, HLS, YIQ and invalid Lab: through the screen conversion.
                let [r, g, b] = other.to_rgb_f32();
                Color::Rgb { r: f(r), g: f(g), b: f(b) }
            }
        }
    }

    /// A fill as a VectorCraft paint and its opacity; `bounds` (document space) places
    /// gradients.
    fn fill_paint(&mut self, fill: &Fill, bounds: Rect) -> Option<(Paint, f32)> {
        match fill {
            Fill::None => None,
            Fill::Solid(c) => Some((Paint::solid(self.color(*c)), 1.0)),
            Fill::Fountain(f) => {
                let f = f.to_plain(64);
                if f.stops.len() < 2 {
                    return f.stops.first().map(|s| (Paint::solid(self.color(s.color)), 1.0));
                }
                let mut stops: Vec<GradientStop> = f
                    .stops
                    .iter()
                    .map(|s| GradientStop::new(if s.pos.is_finite() { s.pos.clamp(0.0, 1.0) as f32 } else { 0.0 }, self.color(s.color)))
                    .collect();
                stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
                let kind = match f.kind {
                    FountainKind::Linear => GradientKind::Linear,
                    FountainKind::Radial => GradientKind::Radial,
                    FountainKind::Conical | FountainKind::Square | FountainKind::Shaped => {
                        self.notes.warn("conical, square and shaped gradients were imported as radial gradients");
                        GradientKind::Radial
                    }
                };
                let c = bounds.center();
                let (hw, hh) = (bounds.width() / 2.0, bounds.height() / 2.0);
                let pad = if f.edge_pad.is_finite() { f.edge_pad.clamp(0.0, 0.49) } else { 0.0 };
                let geom = if kind == GradientKind::Linear {
                    let a = if f.angle.is_finite() { f.angle.to_radians() } else { 0.0 };
                    // Counter-clockwise angle with Y up → direction in the document (Y down).
                    let dir = Vec2::new(a.cos(), -a.sin());
                    let half = (hw * dir.x).abs() + (hh * dir.y).abs();
                    let span = half * (1.0 - 2.0 * pad);
                    GradientGeom { start: c - dir * span, end: c + dir * span, aspect: 1.0, focal: None }
                } else {
                    let off = Vec2::new(
                        if f.offset.x.is_finite() { f.offset.x.clamp(-1.0, 1.0) } else { 0.0 },
                        if f.offset.y.is_finite() { -f.offset.y.clamp(-1.0, 1.0) } else { 0.0 },
                    );
                    let centre = Point::new(c.x + off.x * hw, c.y + off.y * hh);
                    let r = hw.hypot(hh) * (1.0 - 2.0 * pad);
                    GradientGeom { start: centre, end: centre + Vec2::new(r.max(1e-6), 0.0), aspect: 1.0, focal: None }
                };
                let mut gp = GradientPaint::new(Gradient { kind, stops, ..Gradient::default() });
                gp.angle = if kind == GradientKind::Linear && f.angle.is_finite() { f.angle } else { 0.0 };
                gp.geom = Some(geom);
                Some((Paint::Gradient(Box::new(gp)), 1.0))
            }
            Fill::Pattern(_) | Fill::Texture(_) => {
                self.notes.warn("pattern and texture fills became 50% grey (there is no editable equivalent)");
                Some((Paint::solid(Color::Gray { k: 0.5 }), 1.0))
            }
            Fill::Mesh(mesh) => {
                self.notes.warn("mesh fills became the average colour of their nodes");
                let n = mesh.nodes.len().max(1) as f32;
                let sum = mesh.nodes.iter().map(|p| self.color(p.color).to_rgb()).fold([0.0f32; 3], |a, c| [a[0] + c[0], a[1] + c[1], a[2] + c[2]]);
                Some((Paint::solid(Color::Rgb { r: sum[0] / n, g: sum[1] / n, b: sum[2] / n }), 1.0))
            }
        }
    }

    /// Converts a file stroke into a stroke layer.
    ///
    /// The width scales with the object when the file marks it so, and falls back to a hairline when it
    /// isn't a usable number.
    fn stroke(&mut self, st: &Stroke, bounds: Rect, xf: Affine) -> Option<StrokeLayer> {
        let paint = match st.fill.as_deref() {
            Some(f) => self.fill_paint(f, bounds).map(|(p, _)| p),
            None => Some(Paint::solid(self.color(st.color))),
        }?;
        // The width scales with the object when marked so (CorelDRAW stores the untransformed
        // object's).
        let scale = if st.scale_with_object { xf.determinant().abs().sqrt() / self.k } else { 1.0 };
        let w = st.width * self.k * if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let width = if w.is_finite() && w > 0.0 { w } else { HAIRLINE_PT };
        let mut l = StrokeLayer::new(paint, width);
        l.cap = match st.cap {
            TdCap::Butt => LineCap::Butt,
            TdCap::Round => LineCap::Round,
            TdCap::Square => LineCap::Square,
        };
        l.join = match st.join {
            TdJoin::Miter => LineJoin::Miter,
            TdJoin::Round => LineJoin::Round,
            TdJoin::Bevel => LineJoin::Bevel,
        };
        let dash: Vec<f64> = st.dash.iter().filter(|d| d.is_finite() && **d >= 0.0).map(|d| d * width).collect();
        if !dash.is_empty() && dash.iter().any(|d| *d > 0.0) {
            l.dash = Some(Dash { pattern: dash, offset: 0.0, align_corners: false });
        }
        if st.start_arrow != Arrowhead::None || st.end_arrow != Arrowhead::None {
            self.notes.warn("arrowheads were not imported");
        }
        if !st.widths.is_empty() {
            self.notes.warn("variable-width outlines were imported with a uniform width");
        }
        if (st.stretch - 1.0).abs() > 1e-6 && st.stretch.is_finite() {
            self.notes.warn("calligraphic outlines (stretched nibs) were imported with a round nib");
        }
        if st.painterly.is_some() {
            self.notes.warn("artistic (brush) outlines were imported as plain lines");
        }
        Some(l)
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        s: &TdShape,
        spans: &[TextSpan],
        origin: Point,
        frame: Option<(f64, f64)>,
        align: TextAlign,
        xf: Affine,
    ) -> Option<TextObject> {
        if !(origin.x.is_finite() && origin.y.is_finite()) {
            return None;
        }
        // Type space in pt with Y down → local mm (Y up) → document. The transform's uniform scale
        // moves into the type size, so sizes read in points.
        let m = xf * Affine::translate(origin.to_vec2()) * Affine::new([1.0 / PT_PER_MM, 0.0, 0.0, -1.0 / PT_PER_MM, 0.0, 0.0]);
        let k = m.determinant().abs().sqrt();
        if !(k.is_finite() && k > 1e-9) {
            return None;
        }
        let mut obj = TextObject::point(Point::ZERO, "", CharStyle::default());
        obj.xf = m * Affine::scale(1.0 / k);
        obj.runs = spans
            .iter()
            .filter(|sp| !sp.text.is_empty())
            .map(|sp| {
                let mut style = CharStyle::default();
                if sp.font_family.trim().is_empty() {
                    self.notes.warn("type without a font name uses the default font");
                } else {
                    style.font_family = sp.font_family.trim().to_string();
                    self.fonts.insert(style.font_family.clone());
                }
                style.font_style = match (sp.bold, sp.italic) {
                    (true, true) => "Bold Italic",
                    (true, false) => "Bold",
                    (false, true) => "Italic",
                    (false, false) => "Regular",
                }
                .into();
                let size = if sp.size_pt.is_finite() && sp.size_pt > 0.0 { sp.size_pt } else { 12.0 };
                style.size = size * k;
                if sp.tracking_pct.is_finite() {
                    // Percent of a space → thousandths of an em.
                    style.tracking = sp.tracking_pct * 10.0;
                }
                if sp.baseline_shift_pt.is_finite() {
                    style.baseline_shift = sp.baseline_shift_pt * k;
                }
                if sp.angle_deg.abs() > 1e-6 || sp.offset_x_pct.abs() > 1e-6 {
                    self.notes.warn("the rotation and shift of single characters were not imported");
                }
                style.underline = sp.underline;
                style.strikethrough = sp.strikethrough;
                let fill = sp.fill.as_ref().unwrap_or(&s.fill);
                style.fill = self.fill_paint(fill, Rect::ZERO).map_or(Paint::None, |(p, _)| p);
                if matches!(style.fill, Paint::Gradient(_)) {
                    // A gradient without a known box: its first stop's colour.
                    style.fill = match fill {
                        Fill::Fountain(f) => f.stops.first().map_or(Paint::None, |st| Paint::solid(self.color(st.color))),
                        _ => style.fill.clone(),
                    };
                    self.notes.warn("type filled with a gradient was imported in the gradient's first colour");
                }
                if let Some(st) = &s.stroke {
                    style.stroke = Paint::solid(self.color(st.color));
                    // Scaled as the type size is (the import scale and the object's transform).
                    let w = st.width * PT_PER_MM * k;
                    style.stroke_width = if w.is_finite() { w.max(0.0) } else { 0.0 };
                }
                TextRun::new(sp.text.replace(['\r', '\u{2029}', '\u{2028}'], "\n"), style)
            })
            .collect();
        if obj.runs.is_empty() {
            return None;
        }
        obj.para.justify = match align {
            TextAlign::Left => Justify::Left,
            TextAlign::Center => Justify::Center,
            TextAlign::Right => Justify::Right,
            TextAlign::Justify => Justify::JustifyLeft,
        };
        if let Some((w, h)) = frame.filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0) {
            // A paragraph text frame hangs from its top-left corner (the origin).
            let r = Rect::new(0.0, 0.0, w * PT_PER_MM * k, h * PT_PER_MM * k);
            obj.kind = TextKind::Area { frame: shapes::rectangle(r) };
        }
        Some(obj)
    }

    /// An image node for the embedded PNG `png`, placed in `rect` under `xf`.
    ///
    /// Returns `None` with a warning when the image can't be read or is larger than the side limit.
    fn bitmap(&mut self, rect: Rect, w: u32, h: u32, png: &[u8], xf: Affine) -> Option<Node> {
        let ok = (|| {
            let mut reader = image::ImageReader::new(std::io::Cursor::new(png)).with_guessed_format().ok()?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(MAX_IMAGE_SIDE);
            limits.max_image_height = Some(MAX_IMAGE_SIDE);
            reader.limits(limits);
            reader.into_dimensions().ok()
        })();
        let Some((pw, ph)) = ok.filter(|(a, b)| *a > 0 && *b > 0) else {
            self.notes.warn("an embedded image that couldn't be read was left out");
            return None;
        };
        if w != pw || h != ph {
            self.notes.warn("images whose declared size differs from their pixels were fitted to the pixels");
        }
        if !(rect.width().is_finite() && rect.height().is_finite()) || rect.width() == 0.0 || rect.height() == 0.0 {
            return None;
        }
        // Pixel (0, 0) is the top-left corner: in local mm, (x0, y1) with Y up.
        let px = Affine::new([rect.width() / f64::from(pw), 0.0, 0.0, -rect.height() / f64::from(ph), rect.x0, rect.y1]);
        let blob = ImageBlob::new("image/png", png.to_vec());
        let key = blob.content_key();
        self.doc.images.entry(key.clone()).or_insert(blob);
        let im = ImageObject { key, width: pw, height: ph, xf: xf * px, link: None, placement: Default::default(), pixels: None };
        Some(Node::new(self.id(), NodeKind::Image(Box::new(im))))
    }
}

/// Whether every point of `e` has finite coordinates.
fn el_finite(e: &PathEl) -> bool {
    let p = |q: &Point| q.x.is_finite() && q.y.is_finite();
    match e {
        PathEl::MoveTo(a) | PathEl::LineTo(a) => p(a),
        PathEl::QuadTo(a, b) => p(a) && p(b),
        PathEl::CurveTo(a, b, c) => p(a) && p(b) && p(c),
        PathEl::ClosePath => true,
    }
}

/// `n` with plain groups ungrouped (clipping groups stay); `visible` and `locked` are the state of
/// the containers taken away above it, which every object it leaves keeps (hidden stays hidden,
/// locked stays locked).
pub(crate) fn ungroup(mut n: Arc<Node>, visible: bool, locked: bool, out: &mut Vec<Arc<Node>>) {
    match &n.kind {
        NodeKind::Group { children, clip: false } if n.opacity >= 0.999 => {
            let (visible, locked) = (visible && n.visible, locked || n.locked);
            for c in children.clone() {
                ungroup(c, visible, locked, out);
            }
        }
        _ => {
            if !visible || locked {
                let m = Arc::make_mut(&mut n);
                m.visible &= visible;
                m.locked |= locked;
            }
            out.push(n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::ShapeId;
    use tracedraw_core::document::Symbol;

    /// A symbol of shapes with invalid transforms, placed many times: every shape left out counts
    /// towards the limit, so the import stops instead of copying the symbol for each instance.
    #[test]
    fn shapes_left_out_count_towards_the_object_limit() {
        let mut bad = TdShape::new(ShapeId(1), ShapeKind::Rect { rect: Rect::new(0.0, 0.0, 1.0, 1.0), radius: 0.0, corners: None });
        bad.transform = Affine::scale(f64::NAN);
        let children = vec![bad; 2_000];
        let mut td = TdDocument::new("t", kurbo::Size::new(10.0, 10.0));
        td.symbols.push(Symbol { name: "s".into(), shape: TdShape::new(ShapeId(2), ShapeKind::Group { children }) });
        let instance = TdShape::new(ShapeId(3), ShapeKind::SymbolInstance { index: 0 });
        let opts = ImportOptions::default();
        let mut r = Reader {
            doc: Document::new(1.0, 1.0),
            td: &td,
            opts: &opts,
            k: PT_PER_MM,
            notes: Notes::default(),
            fonts: BTreeSet::new(),
            layers: 0,
            objects: 0,
            visited: 0,
        };
        for _ in 0..1_000 {
            assert!(r.shape(&instance, Affine::IDENTITY, 0).is_none());
        }
        assert_eq!(r.visited, MAX_OBJECTS);
        assert!(r.notes.list.iter().any(|w| w.contains("more than")), "{:?}", r.notes.list);
    }
}
