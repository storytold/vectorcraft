//! Export: VectorCraft art (points, Y down) turned into the `tracedraw-cdr` document
//! (millimetres, Y up), written by its CDR writer and, by default, read back to validate the file
//! before it is handed over.
//!
//! Each artboard becomes a page; each top-level object of a layer goes to the page whose artboard
//! holds its centre. Live effects are applied first (the shapes they draw); what CDR can't hold
//! becomes a warning. Nothing is rasterized here: raster effects only become images when the user
//! allows it (the engine flattens them before calling [`export`]).

use std::sync::Arc;

use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape as _};
use tracedraw_core::document::{Layer as TdLayer, Page as TdPage, Shape as TdShape, ShapeKind, TextSpan};
use tracedraw_core::style::{Fountain, FountainKind, LineCap as TdCap, LineJoin as TdJoin, Stop};
use tracedraw_core::{Color as TdColor, Document as TdDocument, Fill, Stroke, TextAlign};
use vectorcraft_color::{BlendMode, Color, GradientKind, Paint};
use vectorcraft_doc::text::{Justify, TextKind, TextObject};
use vectorcraft_doc::{AppearanceItem, Document, ImageObject, LineCap, LineJoin, Node, NodeKind, StrokeAlign};
use vectorcraft_geom::{FillRule, PathData};
use vectorcraft_pathops::BoolOp;

pub use tracedraw_cdr::write::{Png, Previews};

use crate::read::{PT_PER_MM, count_shapes};
use crate::{Error, NEWEST_EXPORT_VERSION, OLDEST_EXPORT_VERSION, Result, VERIFIED_SINCE, guarded};

/// The deepest nesting of symbols and groups walked.
const MAX_DEPTH: usize = 64;
/// The largest image written (pixels per side).
const MAX_IMAGE_SIDE: u32 = 16_384;

/// Export options.
#[derive(Clone, Debug)]
pub struct ExportOptions {
    /// The format version (7 to 22).
    pub version: u16,
    /// Write type as outlines; off, point type stays editable (the default from version 16, the
    /// first whose written type was verified in CorelDRAW).
    pub text_to_outlines: bool,
    /// Read the written file back and compare it with what was written.
    pub validate: bool,
    /// Thumbnails stored in the package (version 16 on; CorelDRAW's file dialogs show them).
    pub previews: Previews,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self { version: crate::DEFAULT_EXPORT_VERSION, text_to_outlines: false, validate: true, previews: Previews::default() }
    }
}

/// A written CDR file.
#[derive(Clone, Debug)]
pub struct Exported {
    pub bytes: Vec<u8>,
    pub version: u16,
    /// What was approximated, converted or left out.
    pub warnings: Vec<String>,
    /// The post-export validation report (`None`: off).
    pub validation: Option<String>,
    pub pages: usize,
    pub objects: usize,
}

/// Writes `doc` as a CorelDRAW file of the version named in `opts`.
///
/// Fails when the version is outside the writable range or the document has no artboard (each
/// artboard becomes a page). Effects are baked first, and everything the format can't hold comes back
/// as a warning beside the bytes.
pub(crate) fn export(doc: &Document, opts: &ExportOptions) -> Result<Exported> {
    let version = opts.version;
    if !(OLDEST_EXPORT_VERSION..=NEWEST_EXPORT_VERSION).contains(&version) {
        return Err(Error::Export(format!("version {version} isn't written; choose {OLDEST_EXPORT_VERSION} to {NEWEST_EXPORT_VERSION}")));
    }
    if doc.artboards.is_empty() {
        return Err(Error::Export("the document has no artboard (each artboard becomes a page)".into()));
    }
    let baked = vectorcraft_effects::bake_document(doc);
    let doc = baked.as_ref().unwrap_or(doc);
    let mut w = Writer {
        doc,
        notes: Vec::new(),
        td: TdDocument::new(doc.title.clone(), kurbo::Size::new(1.0, 1.0)),
        outline_text: opts.text_to_outlines,
        in_clip: 0,
    };
    if version < VERIFIED_SINCE {
        w.warn(format!(
            "version {version} is experimental: the writer follows that version's sample files, but opening in CorelDRAW was not verified"
        ));
        if !opts.text_to_outlines && has_text(doc) {
            w.warn("editable type before version 16 was not verified in CorelDRAW; if the type doesn't show, export with Convert Text to Outlines");
        }
    }
    let td = w.document()?;
    let pages = td.pages.len();
    let objects: usize = td.all_layers().map(|l| count_shapes(&l.shapes)).sum();
    let previews = if version >= 16 {
        opts.previews.clone()
    } else {
        Previews { thumbnail: None, pages: opts.previews.pages.iter().take(1).cloned().collect() }
    };
    let bytes = guarded("writing CDR", || tracedraw_cdr::write::document_to_cdr_version_with(&td, version, &previews))?;
    let mut warnings = w.notes;
    let validation = if opts.validate {
        Some(validate_written(&bytes, version, &td).map_err(Error::Validation)?)
    } else {
        warnings.push("post-export validation was off: the file was not checked".into());
        None
    };
    Ok(Exported { bytes, version, warnings, validation, pages, objects })
}

/// Reads `bytes` back and checks the version, pages, page sizes and object count against `td`.
fn validate_written(bytes: &[u8], version: u16, td: &TdDocument) -> std::result::Result<String, String> {
    let info = crate::detect_version(bytes).map_err(|e| format!("signature: {e}"))?;
    if info.version != version {
        return Err(format!("the file declares version {} instead of {version}", info.version));
    }
    let (back, report) = crate::read::parse(bytes).map_err(|e| format!("reading back: {e}"))?;
    if back.pages.len() != td.pages.len() {
        return Err(format!("{} page(s) written, {} read back", td.pages.len(), back.pages.len()));
    }
    for (i, (a, b)) in td.pages.iter().zip(&back.pages).enumerate() {
        if (a.size.width - b.size.width).abs() > 0.05 || (a.size.height - b.size.height).abs() > 0.05 {
            return Err(format!(
                "page {} was written at {:.2} × {:.2} mm and read back at {:.2} × {:.2} mm",
                i + 1,
                a.size.width,
                a.size.height,
                b.size.width,
                b.size.height
            ));
        }
    }
    let written: usize = td.pages.iter().flat_map(|p| &p.layers).map(|l| count_shapes(&l.shapes)).sum();
    let read: usize = back.pages.iter().flat_map(|p| &p.layers).map(|l| count_shapes(&l.shapes)).sum();
    if report.skipped_objects > 0 || read < written {
        return Err(format!("{written} object(s) written, {read} read back ({} skipped by the reader)", report.skipped_objects));
    }
    Ok(format!(
        "validated: {} {} ({}), {} page(s), {written} object(s) read back without error",
        info.product,
        if version >= VERIFIED_SINCE { "— a version verified in CorelDRAW by the upstream project" } else { "— an experimental version" },
        info.container.label(),
        back.pages.len()
    ))
}

/// Whether `doc` holds any text object.
fn has_text(doc: &Document) -> bool {
    let mut any = false;
    doc.walk(|n| any |= matches!(n.kind, NodeKind::Text(_)));
    any
}

struct Writer<'a> {
    doc: &'a Document,
    notes: Vec<String>,
    td: TdDocument,
    outline_text: bool,
    /// The clipping groups around the current node.
    in_clip: u32,
}

/// `v` in document points as file millimetres.
fn mm(v: f64) -> f64 {
    v / PT_PER_MM
}

/// Whether every edge of `r` is finite.
fn finite(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite())
}

impl Writer<'_> {
    /// Records `w`, ignoring a repeat.
    fn warn(&mut self, w: impl Into<String>) {
        let w = w.into();
        if !self.notes.contains(&w) {
            self.notes.push(w);
        }
    }

    /// A new shape of `kind`, with the next free shape id.
    fn new_shape(&mut self, kind: ShapeKind) -> TdShape {
        let id = self.td.ids_mut().shape();
        TdShape::new(id, kind)
    }

    /// Builds the file document: one page per artboard, each with its own transform into page
    /// millimetres (Y up, origin at the bottom left).
    ///
    /// Fails when an artboard has a non-finite or empty size.
    fn document(&mut self) -> Result<TdDocument> {
        let doc = self.doc;
        let boards: Vec<Rect> = doc.artboards.iter().map(|a| a.rect.abs()).collect();
        if boards.iter().any(|r| !finite(*r) || r.width() <= 0.0 || r.height() <= 0.0) {
            return Err(Error::Export("an artboard has an invalid size".into()));
        }
        // Document → page i (mm, Y up, origin at the bottom left).
        let to_page: Vec<Affine> = boards.iter().map(|r| Affine::new([1.0 / PT_PER_MM, 0.0, 0.0, -1.0 / PT_PER_MM, -mm(r.x0), mm(r.y1)])).collect();
        let mut pages: Vec<TdPage> = Vec::new();
        for (i, a) in doc.artboards.iter().enumerate() {
            let r = boards.get(i).copied().unwrap_or(a.rect);
            let id = self.td.ids_mut().page();
            pages.push(TdPage {
                id,
                name: if a.name.trim().is_empty() { format!("Page {}", i + 1) } else { a.name.clone() },
                size: kurbo::Size::new(mm(r.width()), mm(r.height())),
                layers: Vec::new(),
                guides: Vec::new(),
                background: None,
            });
        }
        if !doc.guides.is_empty() {
            self.warn("guides are not exported");
        }
        for layer in &doc.layers {
            if layer.is_template() {
                self.warn("template layers are not exported");
                continue;
            }
            if !layer.visible {
                self.warn("hidden layers and objects are not exported: the CDR writer keeps no hidden state (show them to export them)");
                continue;
            }
            let (printable, name) = match &layer.kind {
                NodeKind::Layer { printable, .. } => (*printable, layer.name.clone().unwrap_or_else(|| "Layer".into())),
                _ => (true, "Layer".into()),
            };
            // Each page gets its part of the layer.
            let mut per_page: Vec<Vec<TdShape>> = vec![Vec::new(); pages.len()];
            for child in layer.children().into_iter().flatten() {
                let page = self.page_of(child, &boards);
                let Some(m) = to_page.get(page).copied() else { continue };
                let shapes = self.node(child, m, layer.opacity.clamp(0.0, 1.0), 0);
                if let Some(v) = per_page.get_mut(page) {
                    v.extend(shapes);
                }
            }
            for (p, shapes) in pages.iter_mut().zip(per_page) {
                let id = self.td.ids_mut().layer();
                let mut l = TdLayer::new(id, name.clone());
                l.visible = layer.visible;
                l.locked = layer.locked;
                l.printable = printable;
                l.shapes = shapes;
                p.layers.push(l);
            }
            if (layer.opacity - 1.0).abs() > 1e-3 {
                self.warn("layer opacity was applied to the layers' objects");
            }
        }
        for p in &mut pages {
            if p.layers.is_empty() {
                let id = self.td.ids_mut().layer();
                p.layers.push(TdLayer::new(id, "Layer 1"));
            }
        }
        let mut td = std::mem::replace(&mut self.td, TdDocument::new("", kurbo::Size::new(1.0, 1.0)));
        td.pages = pages;
        td.title = doc.title.clone();
        td.metadata.author = doc.metadata.author.clone();
        Ok(td)
    }

    /// An object's page: the artboard holding its centre (else the one it overlaps most, else the
    /// first).
    fn page_of(&self, n: &Node, boards: &[Rect]) -> usize {
        let Some(b) = n.geometric_bounds().filter(|b| finite(*b)) else { return 0 };
        let c = b.center();
        if let Some(i) = boards.iter().position(|r| r.contains(c)) {
            return i;
        }
        boards
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.intersect(b).area().max(0.0)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .filter(|(_, a)| *a > 0.0)
            .map_or(0, |(i, _)| i)
    }

    /// Warns about what `n` carries that the format can't hold: blending modes, opacity masks and raster
    /// effects.
    fn note_losses(&mut self, n: &Node) {
        if n.blend != BlendMode::Normal {
            self.warn("blending modes don't exist in the format and were written as Normal");
        }
        if n.extras().mask.as_ref().is_some_and(|m| !m.disabled) {
            self.warn("opacity masks are not exported (the object is written without its mask)");
        }
        let raster = |fx: &[vectorcraft_doc::Effect]| fx.iter().any(|e| e.visible && vectorcraft_effects::is_raster(&e.id));
        if raster(&n.appearance.effects) || n.appearance.items.iter().any(|i| raster(i.effects())) {
            self.warn("raster effects (shadows, glows, blurs) were left out; to keep them, allow rasterizing them in the export options");
        }
    }

    /// `n` as page shapes (`m`: document → page); `opacity`: the containers' above it.
    fn node(&mut self, n: &Node, m: Affine, opacity: f32, depth: usize) -> Vec<TdShape> {
        if depth > MAX_DEPTH {
            self.warn("art nested too deep was left out");
            return vec![];
        }
        if !n.visible {
            self.warn("hidden layers and objects are not exported: the CDR writer keeps no hidden state (show them to export them)");
            return vec![];
        }
        self.note_losses(n);
        let alpha = opacity * n.opacity.clamp(0.0, 1.0);
        let mut out: Vec<TdShape> = match &n.kind {
            NodeKind::Layer { children, clip, .. } | NodeKind::Group { children, clip } => {
                if n.is_layer() {
                    self.warn("sublayers were exported as groups");
                }
                self.container(children, *clip, m, alpha, depth)
            }
            NodeKind::Path { path, guide, .. } => {
                if *guide {
                    return vec![];
                }
                self.path(n, &path.to_bezpath(), m, alpha).into_iter().collect()
            }
            NodeKind::Compound { children, .. } => {
                let mut bp = BezPath::new();
                for p in children.iter().filter_map(|c| c.path_data()) {
                    bp.extend(p.to_bezpath());
                }
                self.path(n, &bp, m, alpha).into_iter().collect()
            }
            NodeKind::Text(t) => {
                // Type inside a clipping group is clipped as curves; the writer only writes artistic
                // text (one line per paragraph), so area type, whose line breaks would be lost, goes
                // out as outlines.
                let editable = !self.outline_text && self.in_clip == 0 && !t.vertical && matches!(t.kind, TextKind::Point);
                if !editable && !self.outline_text {
                    if matches!(t.kind, TextKind::Area { .. }) && self.in_clip == 0 {
                        self.warn(
                            "area type was converted to outlines: the written format only holds artistic text, and the line breaks would be lost",
                        );
                    } else if self.in_clip > 0 {
                        self.warn("type inside clipping groups was converted to outlines to be clipped");
                    } else {
                        self.warn("vertical type and type on a path were converted to outlines");
                    }
                }
                match editable.then(|| self.text(t, m, alpha)).flatten() {
                    Some(s) => vec![s],
                    None => match vectorcraft_effects::outline_text(n) {
                        Some(o) => self.node(&o, m, opacity, depth + 1),
                        None => {
                            self.warn("type that couldn't be converted was left out (missing font?)");
                            vec![]
                        }
                    },
                }
            }
            NodeKind::Image(im) => self.image(im, m, alpha).into_iter().collect(),
            NodeKind::SymbolInstance { symbol, xf } => {
                let Some(sym) = self.doc.symbols.iter().find(|s| &s.name == symbol) else { return vec![] };
                self.warn("symbol instances were exported as independent groups");
                let mut art = (*sym.art).clone();
                art.transform(*xf, false);
                let kids = self.node(&art, m, alpha, depth + 1);
                self.group(kids)
            }
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::Intertwine { .. } => {
                self.warn("blends, envelopes, meshes, repeats and placed documents were exported as the art they draw (not editable as effects)");
                let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                let kids: Vec<TdShape> = g.children().into_iter().flatten().flat_map(|c| self.node(c, m, alpha, depth + 1)).collect();
                self.group(kids)
            }
        };
        let name = n.name.clone().filter(|s| !s.trim().is_empty());
        for s in &mut out {
            if s.name.is_none() {
                s.name = name.clone();
            }
            s.visible &= n.visible;
            s.locked |= n.locked;
        }
        out
    }

    /// `kids` as one shape: nothing when empty, the shape itself when alone, a group otherwise.
    fn group(&mut self, kids: Vec<TdShape>) -> Vec<TdShape> {
        match kids.len() {
            0 => vec![],
            1 => kids,
            _ => vec![self.new_shape(ShapeKind::Group { children: kids })],
        }
    }

    /// Writes `children` under the transform `m`, grouping the result.
    ///
    /// When `clip` is set the first child clips the rest: the format's PowerClip isn't written around
    /// vectors, so the clip is applied to the geometry as a boolean intersection, while clipped bitmaps
    /// get the format's own crop path.
    fn container(&mut self, children: &[Arc<Node>], clip: bool, m: Affine, alpha: f32, depth: usize) -> Vec<TdShape> {
        let Some((first, rest)) = children.split_first().filter(|_| clip) else {
            let kids: Vec<TdShape> = children.iter().flat_map(|c| self.node(c, m, alpha, depth + 1)).collect();
            return self.group(kids);
        };
        // A clipping group: its first child clips the rest. The writer doesn't write PowerClip
        // around vectors, so the clip is applied to the geometry (a boolean intersection, which
        // stays editable); clipped bitmaps get the format's own crop path.
        let Some((outline, rule)) = vectorcraft_effects::clip_outline(first) else { return vec![] };
        let clip_path = PathData::from_bezpath(&(m * outline));
        self.warn("clipping groups were exported with the clip applied to the geometry (clipped strokes become filled areas); clipped bitmaps get a crop path");
        let mut out = Vec::new();
        let frame_paint = first.path_data().map(|p| {
            let bp = m * p.to_bezpath();
            let b = bp.bounding_box();
            (bp, self.paints(first, alpha, b))
        });
        if let Some((bp, (fill, _, opacity))) = &frame_paint
            && !matches!(fill, Fill::None)
        {
            let mut f = self.new_shape(ShapeKind::Path { closed: true, path: bp.clone() });
            f.fill = fill.clone();
            f.opacity = f64::from(*opacity);
            out.push(f);
        }
        self.in_clip += 1;
        let contents: Vec<TdShape> = rest.iter().flat_map(|c| self.node(c, m, alpha, depth + 1)).collect();
        self.in_clip -= 1;
        for s in contents {
            let clipped = self.clip_shape(s, &clip_path, rule);
            out.extend(clipped);
        }
        if let Some((bp, (_, Some(stroke), opacity))) = frame_paint {
            let mut f = self.new_shape(ShapeKind::Path { closed: true, path: bp });
            f.stroke = Some(stroke);
            f.opacity = f64::from(opacity);
            out.push(f);
        }
        self.group(out)
    }

    /// `s` (page coordinates) clipped by `clip`.
    fn clip_shape(&mut self, mut s: TdShape, clip: &PathData, rule: FillRule) -> Vec<TdShape> {
        let inside = |p: &BezPath| {
            let r = vectorcraft_pathops::boolean(&PathData::from_bezpath(p), FillRule::EvenOdd, clip, rule, BoolOp::Intersect);
            (!r.is_empty()).then(|| r.to_bezpath())
        };
        match &mut s.kind {
            ShapeKind::Group { children } => {
                let mut kids = Vec::new();
                for c in std::mem::take(children) {
                    let clipped = self.clip_shape(c, clip, rule);
                    kids.extend(clipped);
                }
                if kids.is_empty() {
                    return vec![];
                }
                if let ShapeKind::Group { children } = &mut s.kind {
                    *children = kids;
                }
                vec![s]
            }
            ShapeKind::Path { path, .. } => {
                let source = s.transform * path.clone();
                let mut out = Vec::new();
                if !matches!(s.fill, Fill::None)
                    && let Some(p) = inside(&source)
                {
                    let mut f = self.new_shape(ShapeKind::Path { closed: true, path: p });
                    f.fill = s.fill.clone();
                    f.opacity = s.opacity;
                    f.name = s.name.clone();
                    f.visible = s.visible;
                    f.locked = s.locked;
                    out.push(f);
                }
                if let Some(st) = s.stroke.clone() {
                    if !st.dash.is_empty() {
                        self.warn("dashed strokes inside clipping groups were written solid");
                    }
                    let cap = match st.cap {
                        TdCap::Butt => vectorcraft_pathops::Cap::Butt,
                        TdCap::Round => vectorcraft_pathops::Cap::Round,
                        TdCap::Square => vectorcraft_pathops::Cap::Square,
                    };
                    let join = match st.join {
                        TdJoin::Miter => vectorcraft_pathops::Join::Miter,
                        TdJoin::Round => vectorcraft_pathops::Join::Round,
                        TdJoin::Bevel => vectorcraft_pathops::Join::Bevel,
                    };
                    let region = vectorcraft_pathops::outline_stroke(&PathData::from_bezpath(&source), st.width, cap, join, 4.0).to_bezpath();
                    if let Some(p) = inside(&region) {
                        let mut f = self.new_shape(ShapeKind::Path { closed: true, path: p });
                        f.fill = Fill::Solid(st.color);
                        f.opacity = s.opacity;
                        f.visible = s.visible;
                        f.locked = s.locked;
                        if st.behind_fill {
                            out.insert(0, f);
                        } else {
                            out.push(f);
                        }
                    }
                }
                out
            }
            ShapeKind::Bitmap { .. } => {
                let frame = self.new_shape(ShapeKind::Path { closed: true, path: clip.to_bezpath() });
                vec![self.new_shape(ShapeKind::ClipFrame { frame: Box::new(frame), contents: vec![s] })]
            }
            ShapeKind::ClipFrame { frame, .. } => {
                // A bitmap already clipped: the two crop paths intersect.
                if let ShapeKind::Path { path, .. } = &mut frame.kind {
                    let p = frame.transform * path.clone();
                    match inside(&p) {
                        Some(q) => {
                            *path = q;
                            frame.transform = Affine::IDENTITY;
                        }
                        None => return vec![],
                    }
                }
                vec![s]
            }
            _ => {
                self.warn("an object inside a clipping group was written unclipped");
                vec![s]
            }
        }
    }

    /// Writes the path `bp` of `n` under `m` as a shape, returning `None` with a warning when its
    /// coordinates aren't all finite.
    fn path(&mut self, n: &Node, bp: &BezPath, m: Affine, alpha: f32) -> Option<TdShape> {
        if bp.elements().is_empty() {
            return None;
        }
        let page = m * bp.clone();
        if !page.elements().iter().all(el_finite) {
            self.warn("paths with invalid coordinates were left out");
            return None;
        }
        let closed = page.elements().iter().any(|e| matches!(e, PathEl::ClosePath));
        let bounds = page.bounding_box();
        let (fill, stroke, opacity) = self.paints(n, alpha, bounds);
        let mut s = self.new_shape(ShapeKind::Path { path: page, closed });
        s.fill = fill;
        s.stroke = stroke;
        s.opacity = f64::from(opacity);
        Some(s)
    }

    /// The first visible fill and stroke of `n` (CDR holds one of each) and the object's uniform
    /// opacity.
    fn paints(&mut self, n: &Node, alpha: f32, bounds: Rect) -> (Fill, Option<Stroke>, f32) {
        let mut fill = None;
        let mut stroke: Option<Stroke> = None;
        let mut fill_opacity = 1.0f32;
        let mut stroke_first = false;
        let visible: Vec<&AppearanceItem> = n.appearance.items.iter().filter(|i| i.visible()).collect();
        let fills = visible.iter().filter(|i| matches!(i, AppearanceItem::Fill(_))).count();
        let strokes = visible.iter().filter(|i| matches!(i, AppearanceItem::Stroke(_))).count();
        if fills > 1 || strokes > 1 {
            self.warn("objects with several fills or strokes were written with the first of each only");
        }
        for item in visible {
            match item {
                AppearanceItem::Fill(f) if fill.is_none() => {
                    if f.blend != BlendMode::Normal {
                        self.warn("blending modes don't exist in the format and were written as Normal");
                    }
                    fill = Some(self.fill(&f.paint, bounds));
                    fill_opacity = f.opacity.clamp(0.0, 1.0);
                }
                AppearanceItem::Stroke(st) if stroke.is_none() && st.width.is_finite() && st.width > 0.0 => {
                    stroke_first = fill.is_none();
                    stroke = self.stroke(st, bounds);
                    if (st.opacity - fill_opacity).abs() > 0.01 && fill.is_some() {
                        self.warn("a fill and a stroke at different opacities were written at the fill's opacity");
                    }
                    if fill.is_none() {
                        fill_opacity = st.opacity.clamp(0.0, 1.0);
                    }
                }
                _ => {}
            }
        }
        if let Some(s) = stroke.as_mut() {
            s.behind_fill = stroke_first && fill.is_some();
        }
        (fill.unwrap_or(Fill::None), stroke, (alpha * fill_opacity).clamp(0.0, 1.0))
    }

    /// Converts a document colour into a file colour; Lab is written as RGB with a warning.
    fn color(&mut self, c: &Color) -> TdColor {
        match *c {
            Color::Rgb { r, g, b } => TdColor::Rgb { r, g, b },
            Color::Cmyk { c, m, y, k } => TdColor::Cmyk { c, m, y, k },
            Color::Gray { k } => TdColor::Gray { v: 1.0 - k },
            Color::Lab { .. } => {
                self.warn("Lab colours were written as RGB");
                let [r, g, b] = c.to_rgb();
                TdColor::Rgb { r, g, b }
            }
        }
    }

    /// Converts a paint into a file fill.
    ///
    /// The format holds no gradient stop opacity and no midpoints, so both are dropped with a warning.
    fn fill(&mut self, p: &Paint, bounds: Rect) -> Fill {
        match p {
            Paint::None => Fill::None,
            Paint::Solid { color, .. } => Fill::Solid(self.color(color)),
            Paint::Gradient(g) => {
                let stops: Vec<Stop> =
                    g.gradient.stops.iter().map(|s| Stop { pos: f64::from(s.offset.clamp(0.0, 1.0)), color: self.color(&s.color) }).collect();
                if g.gradient.stops.iter().any(|s| s.opacity < 0.999) {
                    self.warn("gradient stop opacity doesn't exist in the format and was dropped");
                }
                if g.gradient.stops.iter().any(|s| (s.midpoint - 0.5).abs() > 0.01) {
                    self.warn("gradient midpoints were written at the middle");
                }
                if stops.is_empty() {
                    return Fill::None;
                }
                let kind = match g.gradient.kind {
                    GradientKind::Linear => FountainKind::Linear,
                    GradientKind::Radial => FountainKind::Radial,
                    GradientKind::Freeform => {
                        self.warn("freeform gradients were written as linear gradients of the same colours");
                        FountainKind::Linear
                    }
                };
                let mut f = Fountain { kind, stops, ..Fountain::default() };
                match g.geom {
                    Some(geom) => {
                        // Geometry in the document (Y down) → counter-clockwise angle with Y up.
                        let d = geom.end - geom.start;
                        if kind == FountainKind::Linear && d.hypot() > 1e-9 {
                            f.angle = (-d.y).atan2(d.x).to_degrees();
                        }
                        if kind == FountainKind::Radial && finite(bounds) && bounds.width() > 0.0 && bounds.height() > 0.0 {
                            let c = bounds.center();
                            f.offset = Point::new(
                                ((geom.start.x - c.x) / (bounds.width() / 2.0)).clamp(-1.0, 1.0),
                                (-(geom.start.y - c.y) / (bounds.height() / 2.0)).clamp(-1.0, 1.0),
                            );
                            if (geom.aspect - 1.0).abs() > 0.01 {
                                self.warn("elliptical radial gradients were written as circular");
                            }
                        }
                    }
                    None => f.angle = if g.angle.is_finite() { g.angle } else { 0.0 },
                }
                Fill::Fountain(f)
            }
            Paint::Pattern { pattern, .. } => {
                self.warn("pattern fills were written in the pattern's first solid colour");
                let c = self.doc.pattern(pattern).and_then(|def| def.art.iter().find_map(|a| first_solid(a)));
                Fill::Solid(c.map_or(TdColor::Gray { v: 0.5 }, |c| self.color(&c)))
            }
        }
    }

    /// Converts a stroke layer into a file stroke, or `None` when it paints nothing.
    ///
    /// A gradient or pattern stroke is written in a solid colour, with a warning.
    fn stroke(&mut self, st: &vectorcraft_doc::StrokeLayer, bounds: Rect) -> Option<Stroke> {
        let color = match &st.paint {
            Paint::None => return None,
            Paint::Solid { color, .. } => self.color(color),
            other => {
                // A gradient stroke: CDR holds an outline fill, which the writer doesn't write; the
                // colour is.
                self.warn("gradient and pattern strokes were written in a solid colour");
                match self.fill(other, bounds) {
                    Fill::Solid(c) => c,
                    Fill::Fountain(f) => f.stops.first().map_or(TdColor::BLACK, |s| s.color),
                    _ => TdColor::BLACK,
                }
            }
        };
        let width = mm(st.width);
        let mut s = Stroke::new(color, width);
        s.cap = match st.cap {
            LineCap::Butt => TdCap::Butt,
            LineCap::Round => TdCap::Round,
            LineCap::Square => TdCap::Square,
        };
        s.join = match st.join {
            LineJoin::Miter => TdJoin::Miter,
            LineJoin::Round => TdJoin::Round,
            LineJoin::Bevel => TdJoin::Bevel,
        };
        s.scale_with_object = false;
        if let Some(d) = st.dash.as_ref().filter(|d| d.is_dashed()) {
            // Dashes as multiples of the width.
            s.dash = d.pattern.iter().map(|v| if st.width > 0.0 { v / st.width } else { *v }).filter(|v| v.is_finite()).take(10).collect();
            if d.pattern.len() > 10 {
                self.warn("dash patterns of more than 10 entries were shortened");
            }
        }
        if st.align != StrokeAlign::Center {
            self.warn("strokes aligned inside or outside were written centred");
        }
        if st.start_arrow.is_some() || st.end_arrow.is_some() {
            self.warn("arrowheads are not exported");
        }
        if st.profile.is_some() {
            self.warn("variable width profiles were written with a uniform width");
        }
        if st.brush.is_some() {
            self.warn("brushes were written as plain strokes");
        }
        Some(s)
    }

    /// Writes the text object `t` under `m`, or `None` when its matrix or frame isn't usable.
    ///
    /// The type matrix's uniform scale goes into the type size, because CorelDRAW scales a paragraph
    /// frame with its object but not its letters.
    fn text(&mut self, t: &TextObject, m: Affine, alpha: f32) -> Option<TdShape> {
        // The type matrix's uniform scale goes into the type size (CorelDRAW scales a paragraph
        // frame with its object, but not its letters).
        let s = t.xf.determinant().abs().sqrt();
        if !(s.is_finite() && s > 1e-9) {
            return None;
        }
        let (frame, offset) = match &t.kind {
            TextKind::Area { frame } => {
                let b = frame.to_bezpath().bounding_box();
                if frame.subpaths.len() != 1 || !finite(b) || b.width() <= 0.0 || b.height() <= 0.0 {
                    return None;
                }
                (Some(kurbo::Size::new(mm(b.width() * s), mm(b.height() * s))), b.origin().to_vec2())
            }
            _ => (None, kurbo::Vec2::ZERO),
        };
        // The inverse of the import: local mm (Y up, the type's origin, a frame's corner for
        // paragraph text, at (0, 0)) → type space (pt, Y down) → document → page.
        let xf = t.xf * Affine::translate(offset) * Affine::scale(1.0 / s);
        let transform = m * xf * Affine::new([PT_PER_MM, 0.0, 0.0, -PT_PER_MM, 0.0, 0.0]);
        if !transform.as_coeffs().iter().all(|v| v.is_finite()) {
            return None;
        }
        let spans: Vec<TextSpan> = t
            .runs
            .iter()
            .filter(|r| !r.text.is_empty())
            .map(|r| {
                let st = &r.style;
                let size = if st.size.is_finite() && st.size > 0.0 { st.size } else { 12.0 };
                let mut sp = TextSpan::new(r.text.clone(), st.font_family.clone(), size * s);
                let style = st.font_style.to_ascii_lowercase();
                sp.bold = ["bold", "black", "heavy"].iter().any(|w| style.contains(w));
                sp.italic = style.contains("italic") || style.contains("oblique");
                sp.underline = st.underline;
                sp.strikethrough = st.strikethrough;
                if st.tracking.is_finite() {
                    sp.tracking_pct = st.tracking / 10.0;
                }
                if st.baseline_shift.is_finite() {
                    sp.baseline_shift_pt = st.baseline_shift * s;
                }
                sp.fill = match &st.fill {
                    Paint::None => Some(Fill::None),
                    p => Some(self.fill(p, Rect::ZERO)),
                };
                if (st.h_scale - 1.0).abs() > 1e-3 || (st.v_scale - 1.0).abs() > 1e-3 || st.rotation.abs() > 1e-6 {
                    self.warn("horizontal and vertical character scale and character rotation were not exported");
                }
                sp
            })
            .collect();
        if spans.is_empty() {
            return None;
        }
        if t.runs.iter().any(|r| r.inline.is_some()) {
            self.warn("inline graphics in type were not exported");
        }
        let stroke = self.text_stroke(t, s);
        let align = justify(t.para.justify);
        let mut shape = self.new_shape(ShapeKind::Text { spans, origin: Point::ZERO, frame, align, para: Default::default(), on_path: None });
        shape.transform = transform;
        shape.stroke = stroke;
        shape.opacity = f64::from(alpha);
        Some(shape)
    }

    /// The outline of type: CDR holds one per text object, so the first stroked run's (in mm, its
    /// width scaled as the type size is by `s`), with a warning when the runs' outlines differ.
    fn text_stroke(&mut self, t: &TextObject, s: f64) -> Option<Stroke> {
        let stroked =
            |st: &vectorcraft_doc::text::CharStyle| !matches!(st.stroke, Paint::None) && st.stroke_width.is_finite() && st.stroke_width > 0.0;
        let runs: Vec<&vectorcraft_doc::text::CharStyle> = t.runs.iter().filter(|r| !r.text.is_empty()).map(|r| &r.style).collect();
        let first = runs.iter().copied().find(|st| stroked(st))?;
        let differ = runs.iter().any(|st| !stroked(st) || st.stroke != first.stroke || (st.stroke_width - first.stroke_width).abs() > 1e-6);
        if differ {
            self.warn("type whose characters have different outlines was written with one outline (the first one)");
        }
        let color = match &first.stroke {
            Paint::Solid { color, .. } => self.color(color),
            other => {
                self.warn("gradient and pattern outlines of type were written in a solid colour");
                match self.fill(other, Rect::ZERO) {
                    Fill::Solid(c) => c,
                    Fill::Fountain(f) => f.stops.first().map_or(TdColor::BLACK, |st| st.color),
                    _ => TdColor::BLACK,
                }
            }
        };
        let width = mm(first.stroke_width * s);
        if !(width.is_finite() && width > 0.0) {
            return None;
        }
        let mut out = Stroke::new(color, width);
        out.cap = match first.stroke_cap {
            LineCap::Butt => TdCap::Butt,
            LineCap::Round => TdCap::Round,
            LineCap::Square => TdCap::Square,
        };
        out.join = match first.stroke_join {
            LineJoin::Miter => TdJoin::Miter,
            LineJoin::Round => TdJoin::Round,
            LineJoin::Bevel => TdJoin::Bevel,
        };
        if first.stroke_dash.as_ref().is_some_and(|d| d.is_dashed()) {
            self.warn("dashed outlines of type were written solid");
        }
        Some(out)
    }

    /// Writes the image object `im` under `m`, or `None` with a warning when its pixels aren't available
    /// or can't be decoded within the side limit.
    fn image(&mut self, im: &ImageObject, m: Affine, alpha: f32) -> Option<TdShape> {
        let Some(blob) = self.doc.images.get(&im.key).filter(|b| !b.bytes.is_empty()) else {
            self.warn("linked images whose pixels aren't available were left out");
            return None;
        };
        let decoded = (|| {
            let mut reader = image::ImageReader::new(std::io::Cursor::new(blob.bytes.as_slice())).with_guessed_format().ok()?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(MAX_IMAGE_SIDE);
            limits.max_image_height = Some(MAX_IMAGE_SIDE);
            reader.limits(limits);
            reader.decode().ok()
        })();
        let Some(img) = decoded else {
            self.warn("an image that couldn't be decoded was left out");
            return None;
        };
        let (pw, ph) = (img.width(), img.height());
        let png = if blob.mime == "image/png" {
            blob.bytes.as_ref().clone()
        } else {
            let mut out = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
            out
        };
        if im.link.is_some() {
            self.warn("linked images were embedded in the CDR file");
        }
        // Pixel (px, py) → document is `im.xf` (on the im.width × im.height grid); in local mm the
        // image spans (0, 0)–(pw, ph) with Y up.
        let grid = Affine::scale_non_uniform(f64::from(im.width.max(1)) / f64::from(pw.max(1)), f64::from(im.height.max(1)) / f64::from(ph.max(1)));
        let flip = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, f64::from(ph)]);
        let transform = m * im.xf * grid * flip;
        if !transform.as_coeffs().iter().all(|v| v.is_finite()) {
            return None;
        }
        let rect = Rect::new(0.0, 0.0, f64::from(pw), f64::from(ph));
        let mut s = self.new_shape(ShapeKind::Bitmap { rect, width_px: pw, height_px: ph, png, fx: None });
        s.transform = transform;
        s.opacity = f64::from(alpha);
        Some(s)
    }
}

/// The file text alignment matching the document justification `j`.
fn justify(j: Justify) -> TextAlign {
    match j {
        Justify::Center | Justify::JustifyCenter => TextAlign::Center,
        Justify::Right | Justify::JustifyRight => TextAlign::Right,
        Justify::JustifyLeft => TextAlign::Justify,
        _ => TextAlign::Left,
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

/// The first visible solid colour in `n` (a pattern's art).
fn first_solid(n: &Node) -> Option<Color> {
    let own = n.appearance.items.iter().find_map(|i| match i {
        AppearanceItem::Fill(f) if f.visible => f.paint.color(),
        _ => None,
    });
    own.or_else(|| n.children().into_iter().flatten().find_map(|c| first_solid(c)))
}
