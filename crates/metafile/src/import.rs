//! The metafile readers. EMF (with its EMF+ records) and WMF files are read by `craft_metafile`,
//! and `picture` turns what they draw into document objects. QuickDraw pictures (`pict`) play their
//! opcodes on a [`Player`], a small model of the drawing state (pen, brush, font, the clip), which
//! turns what they draw into document objects.
//!
//! Fills and strokes become paths with a fill and a stroke; lines drawn one after another with the
//! same pen join into one path. Clips become clipping groups around what they clip. Images keep
//! their pixels (as PNG), text becomes point type in the font the file names.

pub(crate) mod pict;
mod picture;

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::clipnest::{Clip, Drawn, MAX_NEST, deep_clips, nest};
use vectorcraft_doc::{
    Appearance, AppearanceItem, CharStyle, Dash, Document, FillLayer, ImageBlob, ImageObject, Justify, LayerColor, LineCap, LineJoin, Node, NodeKind,
    StrokeLayer, TextObject,
};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, PathEl, Point, Rect, Shape, shapes};

use crate::dib::{self, Rgba};
use crate::{Imported, Kind};

/// Most objects an import makes (a hostile file can't exhaust memory with them).
const MAX_NODES: usize = 1 << 20;
/// Objects and clips reaching further than this from the origin (points, well inside the canvas)
/// are left out: they come from damaged or hostile coordinates.
const LIMIT: f64 = 1e6;
/// Width of a cosmetic pen (one pixel of a 96 ppi screen), points.
const HAIRLINE: f64 = 0.75;

/// Raster operations.
pub(crate) const SRCCOPY: u32 = 0x00CC_0020;

const SKIPPED_ROPS: &str = "images drawn with raster operations other than a plain copy (masks, inversions) were left out";
const BAD_IMAGE: &str = "an image that couldn't be read was left out";
const TOO_MANY_PIXELS: &str = "images past the most pixels one file may hold were left out";
const FAR_AWAY: &str = "objects far outside the picture were left out";

/// Is `r` finite and within [`LIMIT`]?
fn sane(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= LIMIT)
}

/// Is `path` an upright rectangle around the whole of `frame`? Writers clip to one first; it clips
/// nothing in the picture.
fn around(path: &BezPath, frame: Rect) -> bool {
    if path.elements().len() > 6 {
        return false;
    }
    let b = path.bounding_box();
    let corners = [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)];
    let near = |p: Point, q: Point| (p - q).hypot() <= 1e-6 * (1.0 + b.width().max(b.height()));
    // Each edge, the closing one included, runs along a side: from a corner to itself or to a
    // neighbor (corners are in order around the box), never across it.
    let along = |i: usize, j: usize| (i + 4 - j) % 4 != 2;
    let mut seen = [false; 4];
    let (mut first, mut last) = (None, None);
    for e in path.elements() {
        let (p, start) = match e {
            PathEl::MoveTo(p) => (*p, true),
            PathEl::LineTo(p) => (*p, false),
            PathEl::ClosePath => continue,
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => return false,
        };
        let Some(i) = corners.iter().position(|c| near(*c, p)) else { return false };
        if start {
            if let (Some(f), Some(l)) = (first, last)
                && !along(f, l)
            {
                return false;
            }
            first = Some(i);
        } else if last.is_some_and(|l| !along(l, i)) {
            return false;
        }
        last = Some(i);
        seen[i] = true;
    }
    if let (Some(f), Some(l)) = (first, last)
        && !along(f, l)
    {
        return false;
    }
    let b = b.inflate(0.5, 0.5);
    seen.iter().all(|s| *s) && b.contains(frame.origin()) && b.contains(Point::new(frame.x1, frame.y1))
}

/// A pen: its style flags (`PS_*`), width in logical units, colour and dash entries.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PenObj {
    pub style: u32,
    pub width: f64,
    pub color: [u8; 3],
    /// PS_USERSTYLE entries, logical units.
    pub entries: Vec<f64>,
}

impl PenObj {
    pub fn solid(color: [u8; 3]) -> Self {
        Self { style: 0, width: 0.0, color, entries: vec![] }
    }
}

pub(crate) const PS_NULL: u32 = 5;
const PS_GEOMETRIC: u32 = 0x0001_0000;

/// A font: its height in logical units (negative: the character height, positive: the cell
/// height), escapement (tenths of a degree), weight, italics and face.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FontObj {
    pub height: f64,
    pub escapement: f64,
    pub weight: i32,
    pub italic: bool,
    pub face: String,
}

/// The drawing state.
#[derive(Clone, Debug)]
struct Dc {
    pen: PenObj,
    /// The brush's color.
    brush: [u8; 3],
    font: FontObj,
    text_color: [u8; 3],
    text_align: u32,
    fill_rule: FillRule,
    miter: f64,
    clips: Vec<Arc<Clip>>,
}

impl Default for Dc {
    fn default() -> Self {
        Self {
            pen: PenObj::solid([0; 3]),
            brush: [255; 3],
            font: FontObj::default(),
            text_color: [0; 3],
            text_align: 0,
            fill_rule: FillRule::EvenOdd,
            miter: 10.0,
            clips: vec![],
        }
    }
}

pub(crate) struct Player {
    kind: Kind,
    /// Picture units → document points.
    dev_to_doc: Affine,
    dc: Dc,
    /// The artboard (document space): clips holding all of it clip nothing worth keeping.
    frame: Rect,
    /// Lines drawn one after another, not drawn yet.
    pending: BezPath,
    out: Drawn,
    doc: Document,
    warnings: Vec<String>,
    next_clip: u32,
    /// Pixels the file's bitmaps may still decode to ([`dib::MAX_FILE_PIXELS`]).
    pub pixels_left: u64,
}

fn rgb(c: [u8; 3]) -> Paint {
    Paint::solid(Color::rgb8(c[0], c[1], c[2]))
}

/// A rectangle with elliptical corners `rx` × `ry`.
pub(crate) fn round_rect(r: Rect, rx: f64, ry: f64) -> BezPath {
    let r = r.abs();
    let (rx, ry) = ((rx / 2.0).abs().min(r.width() / 2.0), (ry / 2.0).abs().min(r.height() / 2.0));
    if rx <= 0.0 || ry <= 0.0 {
        return shapes::rectangle(r).to_bezpath();
    }
    // A rectangle with round corners of radius `ry`, squeezed across.
    let s = ry / rx;
    let squeezed = Rect::new(r.x0 * s, r.y0, r.x1 * s, r.y1);
    let mut bp = shapes::rounded_rectangle(squeezed, ry).to_bezpath();
    bp.apply_affine(Affine::scale_non_uniform(1.0 / s, 1.0));
    bp
}

impl Player {
    fn new(kind: Kind, dev_to_doc: Affine, artboard: Rect) -> Self {
        let side = |v: f64| if v.is_finite() { v.clamp(1.0, LIMIT) } else { 1.0 };
        let (w, h) = (side(artboard.width()), side(artboard.height()));
        let mut doc = Document::new(w, h);
        doc.layers.clear();
        Self {
            kind,
            dev_to_doc,
            dc: Dc::default(),
            frame: artboard,
            pending: BezPath::new(),
            out: vec![],
            doc,
            warnings: vec![],
            next_clip: 0,
            pixels_left: dib::MAX_FILE_PIXELS,
        }
    }

    pub fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    // ---------- mapping ----------

    /// Picture units → document.
    fn xf(&self) -> Affine {
        self.dev_to_doc
    }

    /// How much picture lengths grow in the document.
    fn scale(&self) -> f64 {
        let s = self.xf().determinant().abs().sqrt();
        if s.is_finite() { s } else { 0.0 }
    }

    // ---------- output ----------

    fn push(&mut self, mut node: Node) {
        if self.out.len() >= MAX_NODES {
            return;
        }
        if !node.visual_bounds().is_none_or(sane) {
            return self.warn(FAR_AWAY);
        }
        node.id = self.doc.alloc_id();
        self.out.push((self.dc.clips.clone(), node));
    }

    fn fill_paint(&self) -> Paint {
        rgb(self.dc.brush)
    }

    fn stroke_layer(&self) -> Option<StrokeLayer> {
        let p = &self.dc.pen;
        let kind = p.style & 0xf;
        if kind == PS_NULL {
            return None;
        }
        // Geometric pens, and other pens wider than 1, have a width in logical units; the others
        // are one pixel wide.
        let w = p.width * self.scale();
        let width = if w > 0.0 && w.is_finite() && (p.style & PS_GEOMETRIC != 0 || p.width > 1.0) { w } else { HAIRLINE };
        let mut st = StrokeLayer::new(rgb(p.color), width);
        // Round caps and joins unless the style asks for others.
        st.cap = match p.style & 0xf00 {
            0x100 => LineCap::Square,
            0x200 => LineCap::Butt,
            _ => LineCap::Round,
        };
        st.join = match p.style & 0xf000 {
            0x1000 => LineJoin::Bevel,
            0x2000 => LineJoin::Miter,
            _ => LineJoin::Round,
        };
        st.miter_limit = self.dc.miter;
        let unit = width.max(HAIRLINE);
        let pattern: Vec<f64> = match kind {
            1 => vec![3.0 * unit, unit],
            2 => vec![unit, unit],
            3 => vec![3.0 * unit, unit, unit, unit],
            4 => vec![3.0 * unit, unit, unit, unit, unit, unit],
            7 => p.entries.iter().map(|e| e * self.scale()).collect(),
            _ => vec![],
        };
        let dash = Dash { pattern, offset: 0.0, align_corners: false };
        st.dash = dash.is_dashed().then_some(dash);
        Some(st)
    }

    /// Paint `bp` (document space) with the brush and the pen.
    fn draw(&mut self, bp: BezPath, fill: bool, stroke: bool) {
        if bp.elements().is_empty() {
            return;
        }
        let mut items = vec![];
        if fill {
            items.push(AppearanceItem::Fill(FillLayer::new(self.fill_paint())));
        }
        if stroke && let Some(st) = self.stroke_layer() {
            items.push(AppearanceItem::Stroke(st));
        }
        if items.is_empty() {
            return;
        }
        let mut n = Node::path(vectorcraft_doc::NodeId(0), PathData::from_bezpath(&bp), Appearance { items, ..Appearance::default() });
        if let NodeKind::Path { rule, .. } = &mut n.kind {
            *rule = self.dc.fill_rule;
        }
        self.push(n);
    }

    /// Draw the lines drawn one after another so far.
    pub fn flush(&mut self) {
        if !self.pending.elements().is_empty() {
            let bp = std::mem::take(&mut self.pending);
            self.draw(bp, false, true);
        }
    }

    // ---------- clips ----------

    fn push_clip(&mut self, path: BezPath, rule: FillRule, replace: bool) {
        if replace {
            self.dc.clips.clear();
        }
        if !sane(path.bounding_box()) {
            return self.warn(FAR_AWAY);
        }
        if around(&path, self.frame) {
            return;
        }
        if self.dc.clips.len() >= MAX_NEST {
            return self.warn(&deep_clips());
        }
        self.next_clip += 1;
        self.dc.clips.push(Arc::new(Clip { id: self.next_clip, region: Some((path, rule)) }));
    }

    // ---------- images and text ----------

    /// An image whose source rectangle `src` (pixels, from the top-left) is drawn into `dest`
    /// (picture units: origin and signed size) with raster operation `rop`, at `opacity`.
    pub fn image(&mut self, img: Result<Rgba, String>, src: Rect, dest: Rect, rop: u32, opacity: f64) {
        if rop != SRCCOPY {
            self.warn(SKIPPED_ROPS);
            return;
        }
        let img = match img {
            Ok(i) => i,
            Err(e) if e == dib::OVER_BUDGET => return self.warn(TOO_MANY_PIXELS),
            Err(_) => return self.warn(BAD_IMAGE),
        };
        let img = crop(img, src);
        if img.width == 0 || img.height == 0 {
            return;
        }
        let (w, h) = (f64::from(img.width), f64::from(img.height));
        self.image_xf(img, Affine::translate((dest.x0, dest.y0)) * Affine::scale_non_uniform(dest.width() / w, dest.height() / h), opacity);
    }

    /// An image placed by `xf` (its pixels, from the top-left, → picture units), at `opacity`. An
    /// image that `xf` flattens or sends past finite numbers is left out, and adds nothing to the
    /// document's images.
    pub(crate) fn image_xf(&mut self, img: Rgba, xf: Affine, opacity: f64) {
        if img.width == 0 || img.height == 0 {
            return;
        }
        let xf = self.xf() * xf;
        if !xf.as_coeffs().iter().all(|v| v.is_finite()) || xf.determinant().abs() < 1e-12 {
            return;
        }
        let Ok(png) = img.to_png() else { return self.warn(BAD_IMAGE) };
        let blob = ImageBlob::new("image/png", png);
        let key = blob.content_key();
        self.doc.images.entry(key.clone()).or_insert(blob);
        let im = ImageObject { key, width: img.width, height: img.height, xf, link: None, placement: Default::default(), pixels: None };
        let mut n = Node::new(vectorcraft_doc::NodeId(0), NodeKind::Image(Box::new(im)));
        n.opacity = opacity.clamp(0.0, 1.0) as f32;
        self.push(n);
    }

    /// Text at `at` (picture units) in the current font.
    pub fn text(&mut self, at: Point, s: &str) {
        let s: String = s.chars().filter(|c| *c != '\0').collect();
        if s.trim().is_empty() {
            return;
        }
        let align = self.dc.text_align;
        let f = self.dc.font.clone();
        let k = self.scale();
        // A negative height is the em; a positive one the cell (the em and the internal leading).
        let em = if f.height < 0.0 { -f.height } else { f.height * 0.85 };
        let size = if em > 0.0 && (em * k).is_finite() { (em * k).clamp(0.5, 5000.0) } else { 12.0 };
        let mut style = CharStyle { size, fill: rgb(self.dc.text_color), stroke: Paint::None, ..CharStyle::default() };
        if !f.face.trim().is_empty() {
            style.font_family = f.face.trim().to_string();
        }
        style.font_style = match (f.weight >= 600, f.italic) {
            (true, true) => "Bold Italic",
            (true, false) => "Bold",
            (false, true) => "Italic",
            (false, false) => "Regular",
        }
        .into();
        let angle = -(f.escapement / 10.0).to_radians();
        let angle = if angle.is_finite() { angle } else { 0.0 };
        // TA_BASELINE, TA_BOTTOM or TA_TOP (the default): where the baseline is.
        let drop = match align & 0x18 {
            0x18 => 0.0,
            0x08 => -0.2 * size,
            _ => 0.8 * size,
        };
        let origin = self.xf() * at;
        let mut t = TextObject::point(Point::ZERO, &s, style);
        t.xf = Affine::translate(origin.to_vec2()) * Affine::rotate(angle) * Affine::translate((0.0, drop));
        t.para.justify = match align & 0x06 {
            0x06 => Justify::Center,
            0x02 => Justify::Right,
            _ => Justify::Left,
        };
        t.cached_bounds = Some(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).bounds);
        self.push(Node::new(vectorcraft_doc::NodeId(0), NodeKind::Text(Box::new(t))));
    }

    // ---------- the document ----------

    /// The document: what was drawn on one layer, clipped art in clipping groups.
    fn finish(mut self) -> Imported {
        self.flush();
        let out = std::mem::take(&mut self.out);
        let children = nest(&mut self.doc, out);
        let mut layer = Node::layer(self.doc.alloc_id(), "Layer 1", LayerColor::Preset(0));
        if let Some(c) = layer.children_mut() {
            *c = children;
        }
        self.doc.layers = vec![Arc::new(layer)];
        Imported { document: self.doc, kind: self.kind, warnings: self.warnings }
    }
}

/// `img` cut to `src` (pixels from the top-left, within the image).
fn crop(img: Rgba, src: Rect) -> Rgba {
    let (w, h) = (img.width, img.height);
    let s = src.abs();
    let clamp = |v: f64, max: u32| if v.is_finite() { v.round().clamp(0.0, f64::from(max)) as u32 } else { 0 };
    let (x0, y0, x1, y1) = (clamp(s.x0, w), clamp(s.y0, h), clamp(s.x1, w), clamp(s.y1, h));
    if (x0, y0, x1, y1) == (0, 0, w, h) || x1 <= x0 || y1 <= y0 {
        return img;
    }
    let mut pixels = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
    for y in y0..y1 {
        let row = (y * w) as usize * 4;
        if let Some(p) = img.pixels.get(row + x0 as usize * 4..row + x1 as usize * 4) {
            pixels.extend_from_slice(p);
        }
    }
    Rgba { width: x1 - x0, height: y1 - y0, pixels }
}

/// Read an EMF or WMF file.
pub(crate) fn import(bytes: &[u8]) -> Result<Imported, String> {
    picture::import(bytes)
}
