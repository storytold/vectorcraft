//! The metafile reader: the format parsers (`emf`, `wmf`) play their records on a [`Player`], a
//! small model of the device context they draw into (objects, transforms, the current position,
//! the path bracket, the clip), which turns what they draw into document objects.
//!
//! Fills and strokes become paths with a fill and a stroke; lines drawn one after another with the
//! same pen join into one path. Clips become clipping groups around what they clip. Images keep
//! their pixels (as PNG), text becomes point type in the font the file names. Records the reader
//! doesn't know are counted and skipped, with one warning.

mod emf;
mod wmf;

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::clipnest::{Clip, Drawn, nest};
use vectorcraft_doc::{
    Appearance, AppearanceItem, CharStyle, Dash, Document, FillLayer, ImageBlob, ImageObject, Justify, LayerColor, LineCap, LineJoin, Node, NodeKind,
    StrokeLayer, TextObject,
};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, PathEl, Point, Rect, Shape, Vec2, shapes};

use crate::dib::Rgba;
use crate::{Imported, Kind};

/// Most objects an import makes (a hostile file can't exhaust memory with them).
const MAX_NODES: usize = 1 << 20;
/// Objects and clips reaching further than this from the origin (points, well inside the canvas)
/// are left out: they come from damaged or hostile coordinates.
const LIMIT: f64 = 1e6;
/// Most nested saved device contexts.
const MAX_SAVED: usize = 1024;
/// Width of a cosmetic pen (one pixel of a 96 ppi screen), points.
const HAIRLINE: f64 = 0.75;
/// Curves made for arcs and ellipses are this close to the true curve (logical units are
/// transformed after, so it is relative).
const ARC_TOLERANCE: f64 = 0.01;

/// Raster operations.
pub(crate) const SRCCOPY: u32 = 0x00CC_0020;
const PATCOPY: u32 = 0x00F0_0021;
const WHITENESS: u32 = 0x00FF_0062;
const BLACKNESS: u32 = 0x0000_0042;

const SKIPPED_ROPS: &str = "images drawn with raster operations other than a plain copy (masks, inversions) were left out";
const HATCHES: &str = "hatched brushes are filled with their colour";
const PATTERN_BRUSHES: &str = "pattern brushes are filled grey";
const EXCLUDED_CLIPS: &str = "clip areas cut out of the clip (excluded rectangles, differences) are ignored";
const BAD_IMAGE: &str = "an image that couldn't be read was left out";
const FAR_AWAY: &str = "objects far outside the picture were left out";

/// Is `r` finite and within [`LIMIT`]?
fn sane(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= LIMIT)
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
    pub const NULL: PenObj = PenObj { style: PS_NULL, width: 0.0, color: [0; 3], entries: vec![] };
    pub fn solid(color: [u8; 3]) -> Self {
        Self { style: 0, width: 0.0, color, entries: vec![] }
    }
}

pub(crate) const PS_NULL: u32 = 5;
const PS_GEOMETRIC: u32 = 0x0001_0000;

/// A brush.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BrushObj {
    Null,
    Solid([u8; 3]),
    Hatched([u8; 3]),
    /// A bitmap pattern (written grey).
    Pattern,
}

impl BrushObj {
    /// From a LOGBRUSH's style and colour.
    pub fn from_log(style: u32, color: [u8; 3]) -> Self {
        match style {
            0 => Self::Solid(color),
            1 => Self::Null,
            2 => Self::Hatched(color),
            _ => Self::Pattern,
        }
    }
}

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

/// A GDI object.
#[derive(Clone, Debug)]
pub(crate) enum Obj {
    Pen(PenObj),
    Brush(BrushObj),
    Font(FontObj),
    /// Palettes, regions, colour spaces: they take a slot but don't change the drawing.
    Other,
}

/// The state records change, saved and restored as a whole.
#[derive(Clone, Debug)]
struct Dc {
    pen: PenObj,
    brush: BrushObj,
    font: FontObj,
    text_color: [u8; 3],
    text_align: u32,
    fill_rule: FillRule,
    miter: f64,
    /// Arcs run counterclockwise (the default).
    arc_ccw: bool,
    /// Logical → page (EMF world transform).
    world: Affine,
    map_mode: u32,
    win_org: Vec2,
    win_ext: Vec2,
    vp_org: Vec2,
    vp_ext: Vec2,
    clips: Vec<Arc<Clip>>,
}

impl Default for Dc {
    fn default() -> Self {
        Self {
            pen: PenObj::solid([0; 3]),
            brush: BrushObj::Solid([255; 3]),
            font: FontObj::default(),
            text_color: [0; 3],
            text_align: 0,
            fill_rule: FillRule::EvenOdd,
            miter: 10.0,
            arc_ccw: true,
            world: Affine::IDENTITY,
            map_mode: 1,
            win_org: Vec2::ZERO,
            win_ext: Vec2::new(1.0, 1.0),
            vp_org: Vec2::ZERO,
            vp_ext: Vec2::new(1.0, 1.0),
            clips: vec![],
        }
    }
}

/// How an arc closes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArcKind {
    Open,
    Chord,
    Pie,
}

pub(crate) struct Player {
    kind: Kind,
    /// Device units (EMF) or logical units (WMF) → document points.
    dev_to_doc: Affine,
    /// Millimetres per device pixel (EMF's fixed mapping modes).
    mm_per_px: f64,
    dc: Dc,
    saved: Vec<Dc>,
    objects: Vec<Option<Obj>>,
    /// The lowest object slot that may be free (WMF numbering).
    free_hint: usize,
    /// The path being recorded (document space), whether a figure is open in it and where that
    /// figure started (logical).
    path: Option<BezPath>,
    figure: bool,
    figure_start: Point,
    /// The artboard (document space): clips holding all of it clip nothing worth keeping.
    frame: Rect,
    /// The current position (logical).
    pos: Point,
    /// Lines drawn one after another outside a path bracket, not drawn yet.
    pending: BezPath,
    out: Drawn,
    doc: Document,
    warnings: Vec<String>,
    skipped: usize,
    next_clip: u32,
    /// The file holds EMF+ records (which aren't read).
    pub emf_plus: bool,
}

fn rgb(c: [u8; 3]) -> Paint {
    Paint::solid(Color::rgb8(c[0], c[1], c[2]))
}

/// The arc of the ellipse in `r` from the ray through `start` to the ray through `end`.
pub(crate) fn arc_path(r: Rect, start: Point, end: Point, ccw: bool, kind: ArcKind) -> BezPath {
    let r = r.abs();
    let c = r.center();
    let radii = Vec2::new(r.width() / 2.0, r.height() / 2.0);
    let mut bp = BezPath::new();
    if radii.x <= 0.0 || radii.y <= 0.0 {
        return bp;
    }
    let angle = |p: Point| ((p.y - c.y) / radii.y).atan2((p.x - c.x) / radii.x);
    let (a0, a1) = (angle(start), angle(end));
    let tau = std::f64::consts::TAU;
    // Counterclockwise on a y-down device is towards smaller angles.
    let sweep = if ccw {
        let s = (a0 - a1).rem_euclid(tau);
        -(if s == 0.0 { tau } else { s })
    } else {
        let s = (a1 - a0).rem_euclid(tau);
        if s == 0.0 { tau } else { s }
    };
    let arc = kurbo::Arc { center: c, radii, start_angle: a0, sweep_angle: sweep, x_rotation: 0.0 };
    let tol = ARC_TOLERANCE * radii.x.max(radii.y).max(1e-9);
    if kind == ArcKind::Pie {
        bp.move_to(c);
        let first = c + Vec2::new(radii.x * a0.cos(), radii.y * a0.sin());
        bp.line_to(first);
        arc.append_iter(tol).for_each(|el| bp.push(el));
    } else {
        kurbo::Shape::path_elements(&arc, tol).for_each(|el| bp.push(el));
    }
    if kind != ArcKind::Open {
        bp.close_path();
    }
    bp
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
    fn new(kind: Kind, dev_to_doc: Affine, mm_per_px: f64, artboard: Rect) -> Self {
        let side = |v: f64| if v.is_finite() { v.clamp(1.0, LIMIT) } else { 1.0 };
        let (w, h) = (side(artboard.width()), side(artboard.height()));
        let mut doc = Document::new(w, h);
        doc.layers.clear();
        Self {
            kind,
            dev_to_doc,
            mm_per_px,
            dc: Dc::default(),
            saved: vec![],
            objects: vec![],
            free_hint: 0,
            path: None,
            figure: false,
            figure_start: Point::ZERO,
            frame: artboard,
            pos: Point::ZERO,
            pending: BezPath::new(),
            out: vec![],
            doc,
            warnings: vec![],
            skipped: 0,
            next_clip: 0,
            emf_plus: false,
        }
    }

    pub fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    /// A record of a kind the reader doesn't know.
    pub fn skip(&mut self) {
        self.skipped += 1;
    }

    // ---------- mapping ----------

    /// Logical → document.
    fn xf(&self) -> Affine {
        if self.kind == Kind::Wmf {
            return self.dev_to_doc;
        }
        let d = &self.dc;
        let page_to_dev = match d.map_mode {
            // Fixed modes: millimetres a unit, y up.
            2..=6 => {
                let mm = [0.1, 0.01, 0.254, 0.0254, 25.4 / 1440.0][(d.map_mode - 2) as usize];
                let s = mm / self.mm_per_px.max(1e-12);
                Affine::translate(d.vp_org) * Affine::scale_non_uniform(s, -s) * Affine::translate(-d.win_org)
            }
            7 | 8 => {
                let ratio = |v: f64, w: f64| if w.abs() > 1e-12 && v.is_finite() { v / w } else { 1.0 };
                let (mut sx, mut sy) = (ratio(d.vp_ext.x, d.win_ext.x), ratio(d.vp_ext.y, d.win_ext.y));
                if d.map_mode == 7 {
                    // Isotropic: the same scale both ways, the smaller one.
                    let s = sx.abs().min(sy.abs());
                    (sx, sy) = (s.copysign(sx), s.copysign(sy));
                }
                Affine::translate(d.vp_org) * Affine::scale_non_uniform(sx, sy) * Affine::translate(-d.win_org)
            }
            _ => Affine::translate(d.vp_org - d.win_org),
        };
        self.dev_to_doc * page_to_dev * d.world
    }

    /// How much logical lengths grow in the document.
    fn scale(&self) -> f64 {
        let s = self.xf().determinant().abs().sqrt();
        if s.is_finite() { s } else { 0.0 }
    }

    pub fn set_map_mode(&mut self, m: u32) {
        self.dc.map_mode = m;
    }
    pub fn set_window_org(&mut self, x: f64, y: f64) {
        self.dc.win_org = Vec2::new(x, y);
    }
    pub fn set_window_ext(&mut self, x: f64, y: f64) {
        self.dc.win_ext = Vec2::new(x, y);
    }
    pub fn set_viewport_org(&mut self, x: f64, y: f64) {
        self.dc.vp_org = Vec2::new(x, y);
    }
    pub fn set_viewport_ext(&mut self, x: f64, y: f64) {
        self.dc.vp_ext = Vec2::new(x, y);
    }
    pub fn scale_ext(&mut self, window: bool, x: (f64, f64), y: (f64, f64)) {
        let e = if window { &mut self.dc.win_ext } else { &mut self.dc.vp_ext };
        if x.1 != 0.0 && y.1 != 0.0 {
            *e = Vec2::new(e.x * x.0 / x.1, e.y * y.0 / y.1);
        }
    }
    /// `SetWorldTransform` / `ModifyWorldTransform` (`mode`: 1 identity, 2 left, 3 right, 4 set).
    pub fn world(&mut self, x: Affine, mode: u32) {
        if !x.as_coeffs().iter().all(|v| v.is_finite()) {
            return;
        }
        self.dc.world = match mode {
            1 => Affine::IDENTITY,
            2 => self.dc.world * x,
            3 => x * self.dc.world,
            _ => x,
        };
    }

    // ---------- state ----------

    pub fn set_fill_mode(&mut self, alternate: bool) {
        self.dc.fill_rule = if alternate { FillRule::EvenOdd } else { FillRule::NonZero };
    }
    pub fn set_miter(&mut self, m: f64) {
        if m.is_finite() && m >= 1.0 {
            self.dc.miter = m;
        }
    }
    pub fn set_arc_ccw(&mut self, ccw: bool) {
        self.dc.arc_ccw = ccw;
    }
    pub fn set_text_color(&mut self, c: [u8; 3]) {
        self.dc.text_color = c;
    }
    pub fn set_text_align(&mut self, a: u32) {
        self.dc.text_align = a;
    }

    pub fn save(&mut self) {
        if self.saved.len() < MAX_SAVED {
            self.saved.push(self.dc.clone());
        }
    }

    /// `RestoreDC(n)`: a negative `n` counts back from the latest save, a positive one is a save's
    /// number.
    pub fn restore(&mut self, n: i32) {
        let len = self.saved.len();
        let keep = if n < 0 { len.checked_sub(n.unsigned_abs() as usize) } else { (n as usize).checked_sub(1).filter(|k| *k < len) };
        if let Some(k) = keep {
            self.saved.truncate(k + 1);
            if let Some(dc) = self.saved.pop() {
                self.dc = dc;
            }
        }
    }

    // ---------- objects ----------

    /// Put `obj` in slot `index` (EMF numbers its objects).
    pub fn create_at(&mut self, index: u32, obj: Obj) {
        let i = index as usize;
        if i == 0 || i > u16::MAX as usize {
            return;
        }
        if self.objects.len() <= i {
            self.objects.resize(i + 1, None);
        }
        if let Some(s) = self.objects.get_mut(i) {
            *s = Some(obj);
        }
    }

    /// Put `obj` in the lowest free slot (WMF).
    pub fn create(&mut self, obj: Obj) {
        let start = self.free_hint.min(self.objects.len());
        let i = self.objects.iter().skip(start).position(Option::is_none).map_or(self.objects.len(), |p| p + start);
        if i > u16::MAX as usize {
            return;
        }
        if i == self.objects.len() {
            self.objects.push(None);
        }
        if let Some(s) = self.objects.get_mut(i) {
            *s = Some(obj);
        }
        self.free_hint = i + 1;
    }

    pub fn delete(&mut self, index: u32) {
        if let Some(s) = self.objects.get_mut(index as usize) {
            *s = None;
            self.free_hint = self.free_hint.min(index as usize);
        }
    }

    /// Select object `index`, or a stock object (EMF: the high bit set).
    pub fn select(&mut self, index: u32) {
        if self.kind == Kind::Emf && index & 0x8000_0000 != 0 {
            match index & 0x7fff_ffff {
                0 | 18 => self.dc.brush = BrushObj::Solid([255; 3]),
                1 => self.dc.brush = BrushObj::Solid([0xc0; 3]),
                2 => self.dc.brush = BrushObj::Solid([0x80; 3]),
                3 => self.dc.brush = BrushObj::Solid([0x40; 3]),
                4 => self.dc.brush = BrushObj::Solid([0; 3]),
                5 => self.dc.brush = BrushObj::Null,
                6 => self.dc.pen = PenObj::solid([255; 3]),
                7 | 19 => self.dc.pen = PenObj::solid([0; 3]),
                8 => self.dc.pen = PenObj::NULL,
                10..=17 => self.dc.font = FontObj::default(),
                _ => {}
            }
            return;
        }
        match self.objects.get(index as usize).cloned().flatten() {
            Some(Obj::Pen(p)) => self.dc.pen = p,
            Some(Obj::Brush(b)) => self.dc.brush = b,
            Some(Obj::Font(f)) => self.dc.font = f,
            _ => {}
        }
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

    fn fill_paint(&mut self) -> Option<Paint> {
        match self.dc.brush.clone() {
            BrushObj::Null => None,
            BrushObj::Solid(c) => Some(rgb(c)),
            BrushObj::Hatched(c) => {
                self.warn(HATCHES);
                Some(rgb(c))
            }
            BrushObj::Pattern => {
                self.warn(PATTERN_BRUSHES);
                Some(rgb([0x80; 3]))
            }
        }
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
        if fill && let Some(p) = self.fill_paint() {
            items.push(AppearanceItem::Fill(FillLayer::new(p)));
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

    /// A figure in logical units: added to the path being recorded, else drawn (filled when it is
    /// an area).
    pub fn figure(&mut self, mut bp: BezPath, area: bool) {
        bp.apply_affine(self.xf());
        match &mut self.path {
            Some(path) => {
                path.extend(bp);
                self.figure = false;
            }
            None => self.draw(bp, area, true),
        }
    }

    /// Polygon / Polyline (and Bézier ones): `pts` in logical units.
    pub fn poly(&mut self, pts: &[Point], closed: bool, bezier: bool) {
        let Some((first, rest)) = pts.split_first() else { return };
        let mut bp = BezPath::new();
        bp.move_to(*first);
        if bezier {
            for c in rest.as_chunks::<3>().0 {
                bp.curve_to(c[0], c[1], c[2]);
            }
        } else {
            rest.iter().for_each(|p| bp.line_to(*p));
        }
        if closed {
            bp.close_path();
        }
        self.figure(bp, closed);
    }

    /// Several polygons (one area) or polylines.
    pub fn poly_poly(&mut self, polys: &[Vec<Point>], closed: bool) {
        let mut bp = BezPath::new();
        for p in polys {
            let Some((first, rest)) = p.split_first() else { continue };
            bp.move_to(*first);
            rest.iter().for_each(|q| bp.line_to(*q));
            if closed {
                bp.close_path();
            }
        }
        self.figure(bp, closed);
    }

    pub fn move_to(&mut self, p: Point) {
        self.pos = p;
        self.figure = false;
    }

    /// LineTo / PolylineTo / PolyBezierTo: from the current position through `pts` (logical).
    pub fn poly_to(&mut self, pts: &[Point], bezier: bool) {
        let Some(last) = pts.last().copied() else { return };
        let xf = self.xf();
        let start = xf * self.pos;
        let target = match &mut self.path {
            Some(path) => {
                if !self.figure {
                    path.move_to(start);
                    self.figure = true;
                    self.figure_start = self.pos;
                }
                path
            }
            None => {
                if self.pending.elements().last().and_then(|e| e.end_point()) != Some(start) {
                    self.pending.move_to(start);
                }
                &mut self.pending
            }
        };
        if bezier {
            for c in pts.as_chunks::<3>().0 {
                target.curve_to(xf * c[0], xf * c[1], xf * c[2]);
            }
        } else {
            pts.iter().for_each(|p| target.line_to(xf * *p));
        }
        self.pos = last;
    }

    pub fn arc(&mut self, r: Rect, start: Point, end: Point, kind: ArcKind) {
        let bp = arc_path(r, start, end, self.dc.arc_ccw, kind);
        self.figure(bp, kind != ArcKind::Open);
    }

    // ---------- paths and clips ----------

    pub fn begin_path(&mut self) {
        self.path = Some(BezPath::new());
        self.figure = false;
    }
    pub fn end_path(&mut self) {
        // The path stays until it is filled, stroked or made the clip.
    }
    pub fn close_figure(&mut self) {
        if let Some(p) = &mut self.path
            && self.figure
        {
            p.close_path();
            self.figure = false;
            self.pos = self.figure_start;
        }
    }
    pub fn abort_path(&mut self) {
        self.path = None;
    }

    /// FillPath / StrokePath / StrokeAndFillPath.
    pub fn paint_path(&mut self, fill: bool, stroke: bool) {
        if let Some(mut bp) = self.path.take() {
            // Filling closes the open figure.
            if fill && self.figure {
                bp.close_path();
            }
            self.draw(bp, fill, stroke);
        }
    }

    fn push_clip(&mut self, path: BezPath, rule: FillRule, replace: bool) {
        if replace {
            self.dc.clips.clear();
        }
        if !sane(path.bounding_box()) {
            return self.warn(FAR_AWAY);
        }
        // A rectangle around the whole picture (what writers clip to first) clips nothing in it.
        let lines = path.elements().iter().all(|e| !matches!(e, PathEl::QuadTo(..) | PathEl::CurveTo(..)));
        let around = path.bounding_box().inflate(0.5, 0.5);
        if lines && path.elements().len() <= 6 && around.contains(self.frame.origin()) && around.contains(Point::new(self.frame.x1, self.frame.y1)) {
            return;
        }
        self.next_clip += 1;
        self.dc.clips.push(Arc::new(Clip { id: self.next_clip, path, rule }));
    }

    /// SelectClipPath (`mode`: 1 and, 5 copy; the others cut out, which isn't read).
    pub fn clip_path(&mut self, mode: u32) {
        let Some(bp) = self.path.take() else { return };
        match mode {
            1 => self.push_clip(bp, self.dc.fill_rule, false),
            5 => self.push_clip(bp, self.dc.fill_rule, true),
            _ => self.warn(EXCLUDED_CLIPS),
        }
    }

    /// IntersectClipRect (logical units).
    pub fn clip_rect(&mut self, r: Rect) {
        let mut bp = shapes::rectangle(r.abs()).to_bezpath();
        bp.apply_affine(self.xf());
        self.push_clip(bp, FillRule::NonZero, false);
    }

    /// A clip region of `rects` in device units (`mode` as [`Self::clip_path`]); none with mode
    /// copy removes the clip.
    pub fn clip_region(&mut self, rects: &[Rect], mode: u32) {
        if rects.is_empty() {
            if mode == 5 {
                self.dc.clips.clear();
            }
            return;
        }
        let mut bp = BezPath::new();
        for r in rects {
            bp.extend(shapes::rectangle(r.abs()).to_bezpath());
        }
        bp.apply_affine(self.dev_to_doc);
        match mode {
            1 => self.push_clip(bp, FillRule::NonZero, false),
            5 => self.push_clip(bp, FillRule::NonZero, true),
            _ => self.warn(EXCLUDED_CLIPS),
        }
    }

    pub fn exclude_clip(&mut self) {
        self.warn(EXCLUDED_CLIPS);
    }

    // ---------- blits, images and text ----------

    /// A raster operation without a source over `dest` (logical): the brush, white or black.
    pub fn pattern_blit(&mut self, dest: Rect, rop: u32) {
        let brush = match rop {
            PATCOPY => self.dc.brush.clone(),
            WHITENESS => BrushObj::Solid([255; 3]),
            BLACKNESS => BrushObj::Solid([0; 3]),
            _ => return self.skip(),
        };
        let pen = std::mem::replace(&mut self.dc.pen, PenObj::NULL);
        let outer = std::mem::replace(&mut self.dc.brush, brush);
        let mut bp = shapes::rectangle(dest.abs()).to_bezpath();
        bp.apply_affine(self.xf());
        self.draw(bp, true, false);
        self.dc.pen = pen;
        self.dc.brush = outer;
    }

    /// An image whose source rectangle `src` (pixels, from the top-left) is drawn into `dest`
    /// (logical: origin and signed size) with raster operation `rop`, at `opacity`.
    pub fn image(&mut self, img: Result<Rgba, String>, src: Rect, dest: Rect, rop: u32, opacity: f64) {
        if rop != SRCCOPY {
            self.warn(SKIPPED_ROPS);
            return;
        }
        let img = match img {
            Ok(i) => i,
            Err(_) => return self.warn(BAD_IMAGE),
        };
        let img = crop(img, src);
        if img.width == 0 || img.height == 0 {
            return;
        }
        let Ok(png) = img.to_png() else { return self.warn(BAD_IMAGE) };
        let blob = ImageBlob::new("image/png", png);
        let key = blob.content_key();
        self.doc.images.entry(key.clone()).or_insert(blob);
        let (w, h) = (f64::from(img.width), f64::from(img.height));
        let xf = self.xf() * Affine::translate((dest.x0, dest.y0)) * Affine::scale_non_uniform(dest.width() / w, dest.height() / h);
        if !xf.as_coeffs().iter().all(|v| v.is_finite()) || xf.determinant().abs() < 1e-12 {
            return;
        }
        let im = ImageObject { key, width: img.width, height: img.height, xf, link: None, placement: Default::default() };
        let mut n = Node::new(vectorcraft_doc::NodeId(0), NodeKind::Image(im));
        n.opacity = opacity.clamp(0.0, 1.0) as f32;
        self.push(n);
    }

    /// Text at `at` (logical; the current position with TA_UPDATECP) in the current font.
    pub fn text(&mut self, at: Point, s: &str) {
        let s: String = s.chars().filter(|c| *c != '\0').collect();
        if s.trim().is_empty() {
            return;
        }
        let align = self.dc.text_align;
        let at = if align & 1 != 0 { self.pos } else { at };
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
        if self.skipped > 0 {
            let n = self.skipped;
            self.warnings.push(format!(
                "{n} record{} of kinds Vector W3K2 doesn't read {} skipped",
                if n == 1 { "" } else { "s" },
                if n == 1 { "was" } else { "were" }
            ));
        }
        let out = std::mem::take(&mut self.out);
        if out.is_empty() && self.emf_plus {
            self.warnings.push("the picture is drawn with EMF+ records, which aren't read yet: nothing was imported".into());
        }
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

/// Read a metafile.
pub(crate) fn import(bytes: &[u8]) -> Result<Imported, String> {
    match crate::sniff(bytes) {
        Some(Kind::Emf) => emf::play(bytes),
        Some(Kind::Wmf) => wmf::play(bytes),
        None => Err("not an EMF or WMF file".into()),
    }
}
