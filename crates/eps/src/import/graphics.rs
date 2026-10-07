//! The graphics state and the operators that draw: coordinates, paths, colours, painting,
//! clipping and shadings. Painted paths become document objects as they are drawn ([`Out`]).

use std::rc::Rc;
use std::sync::Arc;

use kurbo::{PathEl, Shape};
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::clipnest::{Clip, Drawn};
use vectorcraft_doc::{Appearance, AppearanceItem, Dash, Document, FillLayer, LineCap, LineJoin, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, Vec2};

use super::interp::{Interp, MAX_GSAVE, matrix_obj};
use super::obj::{DictRef, Key, Obj, Op, PsError, Res, ps_err};

/// Most objects one file makes.
const MAX_NODES: usize = 1 << 20;
/// Objects reaching further than this from the page (points) come from damaged coordinates and
/// are left out.
const LIMIT: f64 = 1e6;
/// Width of a zero-width line (the thinnest a device draws), points.
const HAIRLINE: f64 = 0.25;
/// Most entries an indexed colour space's table has.
const MAX_HIVAL: usize = 4095;
/// Most elements the current path may have.
const MAX_PATH: usize = 1 << 20;
/// Most path elements drawn in all.
const MAX_DRAWN: usize = 1 << 24;
/// Samples taken of a shading function that isn't a plain interpolation.
const SHADING_SAMPLES: usize = 32;

const FAR_AWAY: &str = "objects far outside the page were left out";
const TOO_MUCH: &str = "the file draws more than VectorCraft reads: the rest was left out";
const TILING_PATTERNS: &str = "pattern fills are filled with mid-grey";
const SAMPLED_FUNCTIONS: &str = "shadings whose colours come from sampled functions are filled with their middle colour";

/// A colour space.
#[derive(Clone, Debug)]
pub(crate) enum Space {
    Gray,
    Rgb,
    Cmyk,
    /// Separation and DeviceN: the ink names, the alternate space and the tint transform.
    Inks {
        names: Vec<Rc<str>>,
        alt: Rc<Space>,
        tint: Obj,
    },
    Indexed {
        base: Rc<Space>,
        table: Rc<Vec<f64>>,
    },
    Pattern,
}

impl Space {
    /// Components a colour has in it.
    pub fn n(&self) -> usize {
        match self {
            Self::Gray | Self::Indexed { .. } | Self::Pattern => 1,
            Self::Rgb => 3,
            Self::Cmyk => 4,
            Self::Inks { names, .. } => names.len().max(1),
        }
    }

    /// The colour `setcolorspace` starts with.
    fn initial(&self) -> Vec<f64> {
        match self {
            Self::Cmyk => vec![0.0, 0.0, 0.0, 1.0],
            Self::Inks { names, .. } => vec![1.0; names.len().max(1)],
            s => vec![0.0; s.n()],
        }
    }
}

/// A process colour from components in `space` (grey, RGB or CMYK), each clamped to 0–1.
pub(crate) fn process(space: &Space, c: &[f64]) -> Option<Color> {
    let v = |i: usize| c.get(i).map_or(0.0, |x| if x.is_finite() { x.clamp(0.0, 1.0) as f32 } else { 0.0 });
    Some(match space {
        Space::Gray => Color::gray(1.0 - v(0)),
        Space::Rgb => Color::rgb(v(0), v(1), v(2)),
        Space::Cmyk => Color::cmyk(v(0), v(1), v(2), v(3)),
        _ => return None,
    })
}

#[derive(Clone, Debug)]
pub(crate) struct GState {
    /// User space → PostScript's default space.
    pub ctm: Affine,
    pub space: Rc<Space>,
    /// The `setcolorspace` operand, for `currentcolorspace`.
    pub space_obj: Obj,
    pub comps: Vec<f64>,
    pub paint: Paint,
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter: f64,
    pub dash: Vec<f64>,
    pub dash_offset: f64,
    /// The current path, in document space.
    pub path: BezPath,
    pub cur: Option<Point>,
    pub start: Option<Point>,
    pub clips: Vec<Arc<Clip>>,
    /// The clips `clipsave` saved, for `cliprestore`.
    pub saved_clips: Vec<Rc<[Arc<Clip>]>>,
    pub overprint: bool,
    pub font: Option<DictRef>,
}

impl Default for GState {
    fn default() -> Self {
        Self {
            ctm: Affine::IDENTITY,
            space: Rc::new(Space::Gray),
            space_obj: Obj::name("DeviceGray"),
            comps: vec![0.0],
            paint: Paint::solid(Color::gray(1.0)),
            width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter: 10.0,
            dash: vec![],
            dash_offset: 0.0,
            path: BezPath::new(),
            cur: None,
            start: None,
            clips: vec![],
            saved_clips: vec![],
            overprint: false,
            font: None,
        }
    }
}

/// What the program draws.
pub(crate) struct Out {
    /// The document being made: ids, images and swatches.
    pub doc: Document,
    pub drawn: Drawn,
    pub warnings: Vec<String>,
    /// PostScript's default space → document space.
    pub page: Affine,
    /// The page (the bounding box) in document space.
    pub frame: Rect,
    next_clip: u32,
    /// Path elements drawn so far (see [`MAX_DRAWN`]).
    elements: usize,
    /// The path the last object was filled with: a stroke of the same path right after joins it.
    merge: Option<BezPath>,
    /// Process colours painted, by model (the document opens in CMYK when most are).
    pub cmyk: usize,
    pub rgb: usize,
    /// Spot inks used: name and colour at full tint.
    pub spots: Vec<(String, Color)>,
    /// Rectangles painted with a shading (`shfill`): a clipping group of one is the clip path
    /// filled with it.
    pub shadings: Vec<NodeId>,
}

fn sane(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= LIMIT)
}

impl Out {
    pub fn new(doc: Document, page: Affine, frame: Rect) -> Self {
        Self {
            doc,
            drawn: vec![],
            warnings: vec![],
            page,
            frame,
            next_clip: 0,
            elements: 0,
            merge: None,
            cmyk: 0,
            rgb: 0,
            spots: vec![],
            shadings: vec![],
        }
    }

    pub fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    /// Add `node` under `clips`; `None` when it was left out.
    pub fn push(&mut self, mut node: Node, clips: &[Arc<Clip>]) -> Option<NodeId> {
        self.merge = None;
        self.elements = self.elements.saturating_add(node.path_data().map_or(1, |p| p.subpaths.iter().map(|s| s.anchors.len()).sum()));
        if self.drawn.len() >= MAX_NODES || self.elements > MAX_DRAWN {
            self.warn(TOO_MUCH);
            return None;
        }
        if !node.visual_bounds().is_none_or(sane) {
            self.warn(FAR_AWAY);
            return None;
        }
        node.id = self.doc.alloc_id();
        let id = node.id;
        self.drawn.push((clips.to_vec(), node));
        Some(id)
    }

    /// Count a paint's colour model.
    fn count(&mut self, p: &Paint) {
        match p {
            Paint::Solid { color: Color::Cmyk { .. }, swatch: None, .. } => self.cmyk += 1,
            Paint::Solid { color: Color::Rgb { .. }, swatch: None, .. } => self.rgb += 1,
            _ => {}
        }
    }

    /// Fill `bp` with `paint`.
    fn fill(&mut self, bp: BezPath, rule: FillRule, paint: Paint, overprint: bool, clips: &[Arc<Clip>]) {
        self.count(&paint);
        let item = AppearanceItem::Fill(FillLayer { overprint, ..FillLayer::new(paint) });
        let mut n = Node::path(NodeId(0), PathData::from_bezpath(&bp), Appearance { items: vec![item], ..Appearance::default() });
        if let NodeKind::Path { rule: r, .. } = &mut n.kind {
            *r = rule;
        }
        if self.push(n, clips).is_some() {
            self.merge = Some(bp);
        }
    }

    /// Stroke `bp` with `st`: joins the object just filled with the same path, under the same
    /// clips (`gsave fill grestore stroke`).
    fn stroke(&mut self, bp: BezPath, st: StrokeLayer, clips: &[Arc<Clip>]) {
        self.count(&st.paint);
        if self.merge.as_ref() == Some(&bp)
            && let Some((chain, last)) = self.drawn.last_mut()
            && chain.len() == clips.len()
            && chain.iter().zip(clips).all(|(a, b)| Arc::ptr_eq(a, b))
        {
            last.appearance.items.push(AppearanceItem::Stroke(st));
            self.merge = None;
            return;
        }
        let n = Node::path(NodeId(0), PathData::from_bezpath(&bp), Appearance { items: vec![AppearanceItem::Stroke(st)], ..Appearance::default() });
        self.push(n, clips);
    }

    /// A new clip of `path` inside `clips`; `None` for a rectangle around the whole page, which
    /// clips nothing.
    fn clip(&mut self, path: BezPath, rule: FillRule) -> Option<Arc<Clip>> {
        let lines = path.elements().iter().all(|e| !matches!(e, PathEl::QuadTo(..) | PathEl::CurveTo(..)));
        let around = path.bounding_box().inflate(0.5, 0.5);
        if lines && path.elements().len() <= 6 && around.contains(self.frame.origin()) && around.contains(Point::new(self.frame.x1, self.frame.y1)) {
            return None;
        }
        self.next_clip += 1;
        Some(Arc::new(Clip { id: self.next_clip, path, rule }))
    }
}

/// The smallest angle from `a` to `b` (radians, −π to π).
fn turn(a: f64, b: f64) -> f64 {
    let d = (b - a).rem_euclid(std::f64::consts::TAU);
    if d > std::f64::consts::PI { d - std::f64::consts::TAU } else { d }
}

impl Interp<'_> {
    /// User space → document space.
    pub fn xf(&self) -> Affine {
        self.out.page * self.g.ctm
    }

    /// How much user-space lengths grow in the document.
    pub fn scale(&self) -> f64 {
        let s = self.xf().determinant().abs().sqrt();
        if s.is_finite() { s } else { 0.0 }
    }

    /// The current point in user space.
    pub fn user_point(&self) -> Res<Point> {
        let p = self.g.cur.ok_or(PsError::Ps("nocurrentpoint", String::new()))?;
        let inv = self.xf();
        if inv.determinant().abs() < 1e-12 {
            return ps_err("undefinedresult", "");
        }
        Ok(inv.inverse() * p)
    }

    pub fn gsave(&mut self) -> Res {
        if self.saved.len() >= MAX_GSAVE {
            return Err(PsError::Limit("too many saved graphics states"));
        }
        self.alloc(std::mem::size_of_val(self.g.path.elements()))?;
        self.saved.push(self.g.clone());
        Ok(())
    }

    pub fn grestore(&mut self) {
        if let Some(g) = self.saved.pop() {
            self.g = g;
        }
    }

    // ---------- paths ----------

    /// Room for one more element in the current path.
    pub fn grow(&self) -> Res {
        if self.g.path.elements().len() >= MAX_PATH {
            return Err(PsError::Limit("a path has too many points"));
        }
        Ok(())
    }

    fn move_to(&mut self, p: Point) -> Res {
        self.grow()?;
        // Consecutive movetos: the last one counts.
        if let Some(PathEl::MoveTo(_)) = self.g.path.elements().last() {
            self.g.path.pop();
        }
        self.g.path.move_to(p);
        self.g.cur = Some(p);
        self.g.start = Some(p);
        Ok(())
    }

    fn line_to(&mut self, p: Point) -> Res {
        self.grow()?;
        if self.g.cur.is_none() {
            return ps_err("nocurrentpoint", "");
        }
        self.g.path.line_to(p);
        self.g.cur = Some(p);
        Ok(())
    }

    /// Path elements in user space appended (the first move a line when there is a current point).
    fn append_user(&mut self, els: impl Iterator<Item = PathEl>) -> Res {
        let xf = self.xf();
        for el in els {
            match el {
                PathEl::MoveTo(p) => {
                    let p = xf * p;
                    if self.g.cur.is_some() { self.line_to(p)? } else { self.move_to(p)? }
                }
                PathEl::LineTo(p) => self.line_to(xf * p)?,
                PathEl::QuadTo(a, p) => {
                    self.grow()?;
                    self.g.path.quad_to(xf * a, xf * p);
                    self.g.cur = Some(xf * p);
                }
                PathEl::CurveTo(a, b, p) => {
                    self.grow()?;
                    self.g.path.curve_to(xf * a, xf * b, xf * p);
                    self.g.cur = Some(xf * p);
                }
                PathEl::ClosePath => self.g.path.close_path(),
            }
        }
        Ok(())
    }

    /// `arc` / `arcn`: from angle `a1` to `a2` (degrees, counter-clockwise or clockwise).
    fn arc(&mut self, c: Point, r: f64, a1: f64, a2: f64, ccw: bool) -> Res {
        let (a1, mut a2) = (a1.to_radians(), a2.to_radians());
        let tau = std::f64::consts::TAU;
        if ccw {
            while a2 < a1 {
                a2 += tau;
            }
        } else {
            while a2 > a1 {
                a2 -= tau;
            }
        }
        let sweep = (a2 - a1).clamp(-tau * 4.0, tau * 4.0);
        let start = c + Vec2::new(r * a1.cos(), r * a1.sin());
        self.append_user(std::iter::once(PathEl::MoveTo(start)))?;
        let arc = kurbo::Arc::new(c, Vec2::new(r, r), a1, sweep, 0.0);
        let tol = (0.01 / self.scale().max(1e-9)).max(1e-9);
        self.append_user(arc.append_iter(tol))
    }

    /// `arct` / `arcto`: a line towards `p1` rounded with radius `r` into the line to `p2`; the
    /// tangent points (user space).
    fn arct(&mut self, p1: Point, p2: Point, r: f64) -> Res<[Point; 2]> {
        let p0 = self.user_point()?;
        let (u, v) = (p0 - p1, p2 - p1);
        let (lu, lv) = (u.hypot(), v.hypot());
        let cross = u.cross(v);
        if lu < 1e-12 || lv < 1e-12 || cross.abs() < 1e-12 || r <= 0.0 {
            self.append_user(std::iter::once(PathEl::LineTo(p1)))?;
            return Ok([p1, p1]);
        }
        let (u, v) = (u / lu, v / lv);
        let half = u.dot(v).clamp(-1.0, 1.0).acos() / 2.0;
        let dist = r / half.tan();
        let (t1, t2) = (p1 + u * dist, p1 + v * dist);
        let bis = (u + v).normalize();
        let c = p1 + bis * (r / half.sin());
        let (a1, a2) = ((t1 - c).atan2(), (t2 - c).atan2());
        self.append_user(std::iter::once(PathEl::LineTo(t1)))?;
        let tol = (0.01 / self.scale().max(1e-9)).max(1e-9);
        let arc = kurbo::Arc::new(c, Vec2::new(r, r), a1, turn(a1, a2), 0.0);
        self.append_user(arc.append_iter(tol))?;
        Ok([t1, t2])
    }

    fn rect_path(&self, x: f64, y: f64, w: f64, h: f64) -> BezPath {
        let xf = self.xf();
        let mut bp = BezPath::new();
        bp.move_to(xf * Point::new(x, y));
        bp.line_to(xf * Point::new(x + w, y));
        bp.line_to(xf * Point::new(x + w, y + h));
        bp.line_to(xf * Point::new(x, y + h));
        bp.close_path();
        bp
    }

    /// The rectangles of `rectfill`, `rectstroke` and `rectclip`: `x y w h` or an array of them.
    fn rects(&mut self) -> Res<BezPath> {
        let nums: Vec<f64> = match self.stack.last() {
            Some(Obj::Array { .. }) => {
                let items = self.pop_array()?;
                let v: Option<Vec<f64>> = items.borrow().iter().map(Obj::as_num).collect();
                v.ok_or(PsError::Ps("typecheck", String::new()))?
            }
            _ => self.nums::<4>()?.to_vec(),
        };
        if nums.len() / 4 * 5 >= MAX_PATH {
            return Err(PsError::Limit("a path has too many points"));
        }
        let mut bp = BezPath::new();
        for [x, y, w, h] in nums.as_chunks::<4>().0 {
            bp.extend(self.rect_path(*x, *y, *w, *h));
        }
        Ok(bp)
    }

    // ---------- painting ----------

    fn fill_path(&mut self, bp: BezPath, rule: FillRule) {
        if bp.elements().is_empty() || self.g.paint.is_none() {
            return;
        }
        let (paint, overprint) = (self.g.paint.clone(), self.g.overprint);
        self.out.fill(bp, rule, paint, overprint, &self.g.clips);
    }

    fn stroke_path(&mut self, bp: BezPath) {
        if bp.elements().is_empty() || self.g.paint.is_none() {
            return;
        }
        let k = self.scale();
        let w = self.g.width * k;
        let mut st = StrokeLayer::new(self.g.paint.clone(), if w > 0.0 && w.is_finite() { w } else { HAIRLINE });
        st.cap = self.g.cap;
        st.join = self.g.join;
        st.miter_limit = self.g.miter.clamp(1.0, 500.0);
        st.overprint = self.g.overprint;
        let pattern: Vec<f64> = self.g.dash.iter().map(|d| d * k).collect();
        let dash = Dash { pattern, offset: self.g.dash_offset * k, align_corners: false };
        st.dash = dash.is_dashed().then_some(dash);
        self.out.stroke(bp, st, &self.g.clips);
    }

    fn take_path(&mut self) -> BezPath {
        self.g.cur = None;
        self.g.start = None;
        std::mem::take(&mut self.g.path)
    }

    fn clip_with(&mut self, path: BezPath, rule: FillRule) {
        if let Some(c) = self.out.clip(path, rule) {
            self.g.clips.push(c);
        }
    }

    /// The area the clip leaves, in document space (the page without a clip).
    fn clip_bounds(&self) -> Rect {
        self.g.clips.last().map_or(self.out.frame, |c| c.path.bounding_box())
    }

    // ---------- colour ----------

    /// A colour space operand.
    fn space_of(&mut self, o: &Obj, depth: u32) -> Res<Space> {
        if depth > 4 {
            return ps_err("limitcheck", "setcolorspace");
        }
        let (family, items) = match o {
            Obj::Name(n) | Obj::Exec(n) => (n.clone(), vec![]),
            Obj::Array { items, .. } => {
                let v = items.to_vec();
                let n = v.first().and_then(Obj::text).ok_or(PsError::Ps("typecheck", "setcolorspace".into()))?;
                (n, v)
            }
            _ => return ps_err("typecheck", "setcolorspace"),
        };
        let arg = |i: usize| items.get(i).cloned().ok_or(PsError::Ps("rangecheck", "setcolorspace".into()));
        Ok(match &*family {
            "DeviceGray" | "CIEBasedA" | "CalGray" => Space::Gray,
            "DeviceRGB" | "CIEBasedABC" | "CalRGB" | "Lab" | "CIEBasedDEF" => Space::Rgb,
            "DeviceCMYK" | "CIEBasedDEFG" => Space::Cmyk,
            "Pattern" => Space::Pattern,
            "ICCBased" => match arg(1)? {
                Obj::Dict(d) => match d.borrow().get(&Key::name("N")).and_then(Obj::as_num) {
                    Some(1.0) => Space::Gray,
                    Some(4.0) => Space::Cmyk,
                    _ => Space::Rgb,
                },
                _ => Space::Rgb,
            },
            "Separation" | "DeviceN" => {
                let names: Vec<Rc<str>> = match arg(1)? {
                    Obj::Array { items, .. } => items.borrow().iter().filter_map(Obj::text).collect(),
                    o => o.text().into_iter().collect(),
                };
                let alt = self.space_of(&arg(2)?, depth + 1)?;
                Space::Inks { names, alt: Rc::new(alt), tint: arg(3)? }
            }
            "Indexed" => {
                let base = self.space_of(&arg(1)?, depth + 1)?;
                let hival =
                    arg(2)?.as_num().filter(|v| (0.0..=MAX_HIVAL as f64).contains(v)).ok_or(PsError::Ps("rangecheck", "Indexed".into()))? as usize;
                let n = base.n();
                let table: Vec<f64> = match arg(3)? {
                    Obj::Str(s) => s.borrow().iter().map(|b| f64::from(*b) / 255.0).collect(),
                    p @ Obj::Array { .. } => {
                        let mut t = Vec::with_capacity((hival + 1) * n);
                        for i in 0..=hival {
                            self.push(Obj::Int(i as i64))?;
                            self.call(p.clone())?;
                            let mut c = vec![0.0; n];
                            for slot in c.iter_mut().rev() {
                                *slot = self.pop_num()?;
                            }
                            t.extend(c);
                        }
                        t
                    }
                    _ => return ps_err("typecheck", "Indexed"),
                };
                Space::Indexed { base: Rc::new(base), table: Rc::new(table) }
            }
            _ => return ps_err("undefined", &family),
        })
    }

    fn set_space(&mut self, o: Obj) -> Res {
        let space = self.space_of(&o, 0)?;
        let comps = space.initial();
        self.g.space = Rc::new(space);
        self.g.space_obj = o;
        self.set_comps(comps)
    }

    /// Set the colour to `comps` in the current space.
    fn set_comps(&mut self, comps: Vec<f64>) -> Res {
        let space = self.g.space.clone();
        self.g.paint = self.paint_of(&space, &comps)?;
        self.g.comps = comps;
        Ok(())
    }

    /// Set a process colour, its space with it.
    fn set_process(&mut self, space: Space, name: &str, comps: Vec<f64>) -> Res {
        self.g.space = Rc::new(space);
        self.g.space_obj = Obj::name(name);
        self.set_comps(comps)
    }

    /// The paint of colour `comps` in `space`: a spot ink links to its swatch at the tint.
    pub fn paint_of(&mut self, space: &Space, comps: &[f64]) -> Res<Paint> {
        Ok(match space {
            Space::Inks { names, alt, tint } => {
                let t = comps.first().copied().unwrap_or(1.0).clamp(0.0, 1.0) as f32;
                match names.as_slice() {
                    [one] if &**one == "None" => Paint::None,
                    [one] if &**one == "All" => {
                        Paint::Solid { color: Color::cmyk(1.0, 1.0, 1.0, 1.0).tinted(t), swatch: Some(REGISTRATION.into()), tint: t }
                    }
                    [one] => {
                        let full = match self.out.spots.iter().find(|(n, _)| **n == **one) {
                            Some((_, c)) => *c,
                            None => {
                                let c = self.tint_color(alt, tint, &[1.0])?;
                                self.out.spots.push((one.to_string(), c));
                                c
                            }
                        };
                        Paint::Solid { color: full.tinted(t), swatch: Some(one.to_string()), tint: t }
                    }
                    _ => Paint::solid(self.tint_color(alt, tint, comps)?),
                }
            }
            Space::Indexed { base, table } => {
                let n = base.n();
                let i = comps.first().copied().unwrap_or(0.0).round().max(0.0) as usize;
                let c = table.get(i * n..i * n + n).ok_or(PsError::Ps("rangecheck", "setcolor".into()))?.to_vec();
                Paint::solid(process(base, &c).unwrap_or(Color::BLACK))
            }
            Space::Pattern => Paint::None,
            s => Paint::solid(process(s, comps).unwrap_or(Color::BLACK)),
        })
    }

    /// The alternate colour of inks at `comps`, through the tint transform.
    pub fn tint_color(&mut self, alt: &Space, tint: &Obj, comps: &[f64]) -> Res<Color> {
        let base = self.stack.len();
        for c in comps {
            self.push_num(*c)?;
        }
        self.call(tint.clone())?;
        let n = alt.n();
        let at = self.stack.len().checked_sub(n).filter(|a| *a >= base).ok_or(PsError::Ps("stackunderflow", "tint transform".into()))?;
        let v: Vec<f64> = self.stack.split_off(at).iter().map(|o| o.as_num().unwrap_or(0.0)).collect();
        self.stack.truncate(base);
        Ok(process(alt, &v).unwrap_or(Color::BLACK))
    }

    /// The current colour as grey, RGB or CMYK components (`currentgray` …).
    fn current_rgb(&self) -> [f32; 3] {
        self.g.paint.color().map_or([0.0; 3], |c| c.to_rgb_uncalibrated())
    }

    /// A pattern colour: a shading pattern's gradient (others mid-grey).
    fn pattern_paint(&mut self, d: &DictRef) -> Res<Paint> {
        let kind = d.borrow().get(&Key::name("PatternType")).and_then(Obj::as_num);
        let sh = d.borrow().get(&Key::name("Shading")).cloned();
        match (kind, sh) {
            (Some(2.0), Some(Obj::Dict(sh))) => {
                let m = d.borrow().get(&Key::name("VCMatrix")).and_then(|o| o.items().and_then(|i| super::interp::matrix_of(&i.borrow())));
                let m = self.out.page * m.unwrap_or(self.g.ctm);
                Ok(self.gradient(&sh, m)?.map_or(Paint::None, |g| Paint::Gradient(Box::new(g))))
            }
            _ => {
                self.out.warn(TILING_PATTERNS);
                Ok(Paint::solid(Color::gray(0.5)))
            }
        }
    }

    /// An axial or radial shading as a gradient, `m` mapping its space onto the document.
    fn gradient(&mut self, sh: &DictRef, m: Affine) -> Res<Option<GradientPaint>> {
        let get = |k: &str| sh.borrow().get(&Key::name(k)).cloned();
        let kind = get("ShadingType").and_then(|o| o.as_num()).unwrap_or(0.0);
        if kind != 2.0 && kind != 3.0 {
            self.out.warn("shadings other than axial and radial ones (meshes, functions) were left out");
            return Ok(None);
        }
        let space = self.space_of(&get("ColorSpace").ok_or(PsError::Ps("undefined", "ColorSpace".into()))?, 0)?;
        let nums = |o: Option<Obj>| -> Vec<f64> {
            o.and_then(|o| o.items().map(|i| i.borrow().iter().filter_map(Obj::as_num).collect())).unwrap_or_default()
        };
        let coords = nums(get("Coords"));
        let domain = nums(get("Domain"));
        let (d0, d1) = (domain.first().copied().unwrap_or(0.0), domain.get(1).copied().unwrap_or(1.0));
        let f = get("Function").ok_or(PsError::Ps("undefined", "Function".into()))?;
        // Where the colour changes: the ends, and the bounds of stitched linear functions.
        let ts: Vec<f64> = match linear_points(&f) {
            Some(mut pts) => {
                pts.retain(|t| (d0.min(d1)..=d0.max(d1)).contains(t));
                pts.iter().map(|t| if d1 != d0 { (t - d0) / (d1 - d0) } else { 0.0 }).chain([0.0, 1.0]).collect()
            }
            None => (0..=SHADING_SAMPLES).map(|i| i as f64 / SHADING_SAMPLES as f64).collect(),
        };
        let mut ts = ts;
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let radial = kind == 3.0;
        let c = |i: usize| coords.get(i).copied().unwrap_or(0.0);
        let (r0, r1) = (c(2).max(0.0), c(5).max(1e-6));
        let mut stops = Vec::with_capacity(ts.len());
        for t in ts {
            let comps = self.eval(&f, d0 + (d1 - d0) * t, 0)?;
            let paint = self.paint_of(&space, &comps)?;
            let offset = if radial { (r0 + (r1 - r0) * t) / r1 } else { t };
            let stop = match paint {
                Paint::Solid { color, swatch, tint } => GradientStop { swatch, tint, ..GradientStop::new(offset as f32, color) },
                _ => GradientStop::new(offset as f32, Color::BLACK),
            };
            stops.push(stop);
        }
        let (gk, geom) = if radial {
            let centre = Point::new(c(3), c(4));
            let mut geom = GradientGeom { start: centre, end: centre + Vec2::new(r1, 0.0), aspect: 1.0, focal: None };
            geom.set_focal(Some(Point::new(c(0), c(1))));
            geom.transform(m, GradientKind::Radial);
            (GradientKind::Radial, geom)
        } else {
            (GradientKind::Linear, GradientGeom { start: m * Point::new(c(0), c(1)), end: m * Point::new(c(2), c(3)), aspect: 1.0, focal: None })
        };
        let mut g = GradientPaint::new(Gradient::new(gk, stops));
        g.geom = Some(geom);
        Ok(Some(g))
    }

    /// A function (types 2 and 3; an array of them, one per component) at `t`.
    fn eval(&mut self, f: &Obj, t: f64, depth: u32) -> Res<Vec<f64>> {
        if depth > 8 {
            return ps_err("limitcheck", "Function");
        }
        let d = match f {
            Obj::Dict(d) => d.clone(),
            Obj::Array { items, .. } => {
                let fs = items.to_vec();
                let mut out = vec![];
                for g in &fs {
                    out.extend(self.eval(g, t, depth + 1)?.first().copied());
                }
                return Ok(out);
            }
            _ => return ps_err("typecheck", "Function"),
        };
        let get = |k: &str| d.borrow().get(&Key::name(k)).cloned();
        let nums = |o: Option<Obj>| -> Vec<f64> {
            o.and_then(|o| o.items().map(|i| i.borrow().iter().filter_map(Obj::as_num).collect())).unwrap_or_default()
        };
        let domain = nums(get("Domain"));
        let t = match domain.as_slice() {
            [a, b, ..] => t.clamp(a.min(*b), a.max(*b)),
            _ => t,
        };
        match get("FunctionType").and_then(|o| o.as_num()) {
            Some(2.0) => {
                let c0 = nums(get("C0"));
                let c1 = nums(get("C1"));
                let (c0, c1) = (if c0.is_empty() { vec![0.0] } else { c0 }, if c1.is_empty() { vec![1.0] } else { c1 });
                let n = get("N").and_then(|o| o.as_num()).unwrap_or(1.0);
                let x = t.max(0.0).powf(n);
                Ok(c0.iter().zip(&c1).map(|(a, b)| a + x * (b - a)).collect())
            }
            Some(3.0) => {
                let fs = get("Functions").and_then(|o| o.items().map(|i| i.to_vec())).unwrap_or_default();
                let bounds = nums(get("Bounds"));
                let encode = nums(get("Encode"));
                let (lo, hi) = (domain.first().copied().unwrap_or(0.0), domain.get(1).copied().unwrap_or(1.0));
                let k = bounds.iter().take_while(|b| t >= **b).count().min(fs.len().saturating_sub(1));
                let a = if k == 0 { lo } else { bounds.get(k - 1).copied().unwrap_or(lo) };
                let b = bounds.get(k).copied().unwrap_or(hi);
                let (e0, e1) = (encode.get(2 * k).copied().unwrap_or(0.0), encode.get(2 * k + 1).copied().unwrap_or(1.0));
                let u = if b != a { e0 + (t - a) / (b - a) * (e1 - e0) } else { e0 };
                let g = fs.get(k).cloned().ok_or(PsError::Ps("rangecheck", "Functions".into()))?;
                self.eval(&g, u, depth + 1)
            }
            _ => {
                self.out.warn(SAMPLED_FUNCTIONS);
                let r = nums(get("Range"));
                Ok(r.chunks(2).map(|p| p.iter().sum::<f64>() / 2.0).collect())
            }
        }
    }

    // ---------- the operators ----------

    pub fn graphics_op(&mut self, op: Op) -> Res {
        use Op::*;
        match op {
            Gsave => self.gsave()?,
            Grestore => self.grestore(),
            GrestoreAll => {
                if let Some(g) = self.saved.first().cloned() {
                    self.saved.clear();
                    self.g = g;
                }
            }
            InitGraphics => {
                let font = self.g.font.take();
                self.g = GState { font, ..GState::default() };
            }
            GStateNew => self.push(Obj::GState(Rc::new(self.g.clone())))?,
            CurrentGState => {
                self.pop()?;
                self.push(Obj::GState(Rc::new(self.g.clone())))?;
            }
            SetGState => match self.pop()? {
                Obj::GState(g) => self.g = (*g).clone(),
                _ => return ps_err("typecheck", ""),
            },
            SetLineWidth => self.g.width = self.pop_num()?.abs(),
            CurrentLineWidth => self.push_num(self.g.width)?,
            SetLineCap => {
                self.g.cap = match self.pop_int()? {
                    1 => LineCap::Round,
                    2 => LineCap::Square,
                    _ => LineCap::Butt,
                }
            }
            CurrentLineCap => self.push(Obj::Int(self.g.cap as i64))?,
            SetLineJoin => {
                self.g.join = match self.pop_int()? {
                    1 => LineJoin::Round,
                    2 => LineJoin::Bevel,
                    _ => LineJoin::Miter,
                }
            }
            CurrentLineJoin => self.push(Obj::Int(self.g.join as i64))?,
            SetMiterLimit => self.g.miter = self.pop_num()?,
            CurrentMiterLimit => self.push_num(self.g.miter)?,
            SetDash => {
                let offset = self.pop_num()?;
                let items = self.pop_array()?;
                let v: Option<Vec<f64>> = items.borrow().iter().map(|o| o.as_num().filter(|v| v.is_finite() && *v >= 0.0)).collect();
                self.g.dash = v.ok_or(PsError::Ps("typecheck", String::new()))?;
                self.g.dash_offset = offset;
            }
            CurrentDash => {
                let d = self.g.dash.iter().map(|v| Obj::Real(*v)).collect();
                self.push(Obj::array(d))?;
                self.push_num(self.g.dash_offset)?;
            }
            SetGray => {
                let [g] = self.nums()?;
                self.set_process(Space::Gray, "DeviceGray", vec![g])?;
            }
            SetRgbColor => {
                let c = self.nums::<3>()?;
                self.set_process(Space::Rgb, "DeviceRGB", c.to_vec())?;
            }
            SetHsbColor => {
                let [h, s, b] = self.nums()?;
                let [r, g, bl] =
                    Color::from_hsb((h.rem_euclid(1.0) * 360.0) as f32, s.clamp(0.0, 1.0) as f32, b.clamp(0.0, 1.0) as f32).to_rgb_uncalibrated();
                self.set_process(Space::Rgb, "DeviceRGB", vec![f64::from(r), f64::from(g), f64::from(bl)])?;
            }
            SetCmykColor => {
                let c = self.nums::<4>()?;
                self.set_process(Space::Cmyk, "DeviceCMYK", c.to_vec())?;
            }
            CurrentGray => {
                let [r, g, b] = self.current_rgb();
                self.push_num(f64::from(0.3 * r + 0.59 * g + 0.11 * b))?;
            }
            CurrentRgbColor => {
                for v in self.current_rgb() {
                    self.push_num(f64::from(v))?;
                }
            }
            CurrentCmykColor => {
                let c = self.g.paint.color().map_or([0.0, 0.0, 0.0, 1.0], |c| c.to_cmyk());
                for v in c {
                    self.push_num(f64::from(v))?;
                }
            }
            SetColorSpace => {
                let o = self.pop()?;
                self.set_space(o)?;
            }
            CurrentColorSpace => {
                let o = match &self.g.space_obj {
                    o @ Obj::Array { .. } => o.clone(),
                    o => Obj::array(vec![o.clone()]),
                };
                self.push(o)?;
            }
            SetColor => {
                if matches!(*self.g.space, Space::Pattern) {
                    let Obj::Dict(d) = self.pop()? else { return ps_err("typecheck", "") };
                    self.g.paint = self.pattern_paint(&d)?;
                } else {
                    let n = self.g.space.n();
                    let mut c = vec![0.0; n];
                    for slot in c.iter_mut().rev() {
                        *slot = self.pop_num()?;
                    }
                    self.set_comps(c)?;
                }
            }
            CurrentColor => {
                for v in self.g.comps.clone() {
                    self.push_num(v)?;
                }
            }
            SetPattern => {
                let Obj::Dict(d) = self.pop()? else { return ps_err("typecheck", "") };
                self.g.space = Rc::new(Space::Pattern);
                self.g.space_obj = Obj::name("Pattern");
                self.g.paint = self.pattern_paint(&d)?;
            }
            MakePattern => {
                let m = self.pop_matrix()?;
                let d = self.pop_dict()?;
                let mut copy = d.borrow().clone();
                copy.insert(Key::name("VCMatrix"), matrix_obj(self.g.ctm * m));
                self.push(Obj::dict(copy))?;
            }
            SetOverprint => self.g.overprint = self.pop_bool()?,
            CurrentOverprint => self.push(Obj::Bool(self.g.overprint))?,
            SetFlat | SetHalftone | SetTransfer | SetBlackGeneration | SetUnderColorRemoval | SetColorRendering | SetSmoothness | SetStrokeAdjust
            | SetPageDevice | SetUserParams | SetSystemParams => {
                self.pop()?;
            }
            SetScreen => {
                for _ in 0..3 {
                    self.pop()?;
                }
            }
            SetColorTransfer => {
                for _ in 0..4 {
                    self.pop()?;
                }
            }
            SetColorScreen => {
                for _ in 0..12 {
                    self.pop()?;
                }
            }
            SetCacheLimit => {
                self.pop()?;
            }
            SetUCacheParams => {
                let at = self.stack.iter().rposition(|o| matches!(o, Obj::Mark)).ok_or(PsError::Ps("unmatchedmark", String::new()))?;
                self.stack.truncate(at);
            }
            UCache => {}
            CurrentScreen => {
                self.push(Obj::Real(60.0))?;
                self.push(Obj::Real(45.0))?;
                self.push(Obj::proc(vec![]))?;
            }
            CurrentTransfer | CurrentBlackGeneration | CurrentUnderColorRemoval => self.push(Obj::proc(vec![]))?,
            CurrentColorTransfer => {
                for _ in 0..4 {
                    self.push(Obj::proc(vec![]))?;
                }
            }
            CurrentHalftone => {
                let mut d = super::obj::Dict::new();
                d.insert(Key::name("HalftoneType"), Obj::Int(1));
                d.insert(Key::name("Frequency"), Obj::Real(60.0));
                d.insert(Key::name("Angle"), Obj::Real(45.0));
                d.insert(Key::name("SpotFunction"), Obj::proc(vec![]));
                self.push(Obj::dict(d))?;
            }
            FindEncoding => {
                self.pop()?;
                self.push(Obj::array(vec![Obj::name(".notdef"); 256]))?;
            }
            CurrentFlat => self.push(Obj::Real(1.0))?,
            CurrentStrokeAdjust => self.push(Obj::Bool(false))?,
            CurrentPageDevice => {
                let mut d = super::obj::Dict::new();
                let f = self.out.frame;
                d.insert(Key::name("PageSize"), Obj::array(vec![Obj::Real(f.width()), Obj::Real(f.height())]));
                self.push(Obj::dict(d))?;
            }
            ShowPage => return Err(PsError::Quit),
            CopyPage | ErasePage => {}
            Matrix => self.push(matrix_obj(Affine::IDENTITY))?,
            IdentMatrix | DefaultMatrix => {
                let items = self.pop_array()?;
                self.put_matrix(items, Affine::IDENTITY)?;
            }
            CurrentMatrix => {
                let items = self.pop_array()?;
                self.put_matrix(items, self.g.ctm)?;
            }
            SetMatrix => self.g.ctm = self.pop_matrix()?,
            InitMatrix => self.g.ctm = Affine::IDENTITY,
            Concat => {
                let m = self.pop_matrix()?;
                self.g.ctm *= m;
            }
            ConcatMatrix => {
                let out = self.pop_array()?;
                let (m2, m1) = (self.pop_matrix()?, self.pop_matrix()?);
                self.put_matrix(out, m2 * m1)?;
            }
            Translate | Scale | Rotate => {
                let into = match self.stack.last() {
                    Some(Obj::Array { .. }) => Some(self.pop_array()?),
                    _ => None,
                };
                let m = match op {
                    Rotate => Affine::rotate(self.pop_num()?.to_radians()),
                    Translate => {
                        let [x, y] = self.nums()?;
                        Affine::translate((x, y))
                    }
                    _ => {
                        let [x, y] = self.nums()?;
                        Affine::scale_non_uniform(x, y)
                    }
                };
                match into {
                    Some(items) => self.put_matrix(items, m)?,
                    None => self.g.ctm *= m,
                }
            }
            Transform | ITransform | DTransform | IDTransform => {
                let m = match self.stack.last() {
                    Some(Obj::Array { .. }) => self.pop_matrix()?,
                    _ => self.g.ctm,
                };
                let [x, y] = self.nums()?;
                let inverse = matches!(op, ITransform | IDTransform);
                if inverse && m.determinant().abs() < 1e-12 {
                    return ps_err("undefinedresult", "");
                }
                let m = if inverse { m.inverse() } else { m };
                let p = if matches!(op, DTransform | IDTransform) {
                    let v = m * Point::new(x, y) - m * Point::ZERO;
                    Point::new(v.x, v.y)
                } else {
                    m * Point::new(x, y)
                };
                self.push_num(p.x)?;
                self.push_num(p.y)?;
            }
            InvertMatrix => {
                let out = self.pop_array()?;
                let m = self.pop_matrix()?;
                if m.determinant().abs() < 1e-12 {
                    return ps_err("undefinedresult", "");
                }
                self.put_matrix(out, m.inverse())?;
            }
            NewPath => {
                self.take_path();
            }
            CurrentPoint => {
                let p = self.user_point()?;
                self.push_num(p.x)?;
                self.push_num(p.y)?;
            }
            MoveTo => {
                let [x, y] = self.nums()?;
                let p = self.xf() * Point::new(x, y);
                self.move_to(p)?;
            }
            RMoveTo => {
                let [dx, dy] = self.nums()?;
                let p = self.user_point()? + Vec2::new(dx, dy);
                let p = self.xf() * p;
                self.move_to(p)?;
            }
            LineTo => {
                let [x, y] = self.nums()?;
                self.line_to(self.xf() * Point::new(x, y))?;
            }
            RLineTo => {
                let [dx, dy] = self.nums()?;
                let p = self.user_point()? + Vec2::new(dx, dy);
                self.line_to(self.xf() * p)?;
            }
            CurveTo | RCurveTo => {
                let v = self.nums::<6>()?;
                let o = if op == RCurveTo { self.user_point()?.to_vec2() } else { Vec2::ZERO };
                if self.g.cur.is_none() {
                    return ps_err("nocurrentpoint", "");
                }
                let xf = self.xf();
                let [a, b, p] = [0, 2, 4].map(|i| xf * (Point::new(v[i], v[i + 1]) + o));
                self.grow()?;
                self.g.path.curve_to(a, b, p);
                self.g.cur = Some(p);
            }
            Arc | Arcn => {
                let [x, y, r, a1, a2] = self.nums()?;
                self.arc(Point::new(x, y), r, a1, a2, op == Arc)?;
            }
            Arct | Arcto => {
                let [x1, y1, x2, y2, r] = self.nums()?;
                let [t1, t2] = self.arct(Point::new(x1, y1), Point::new(x2, y2), r)?;
                if op == Arcto {
                    for v in [t1.x, t1.y, t2.x, t2.y] {
                        self.push_num(v)?;
                    }
                }
            }
            ClosePath => {
                if self.g.cur.is_some() {
                    self.g.path.close_path();
                    self.g.cur = self.g.start;
                }
            }
            FlattenPath | ReversePath => {}
            PathBBox => {
                if self.g.path.elements().is_empty() {
                    return ps_err("nocurrentpoint", "");
                }
                let xf = self.xf();
                if xf.determinant().abs() < 1e-12 {
                    return ps_err("undefinedresult", "");
                }
                let b = xf.inverse().transform_rect_bbox(self.g.path.bounding_box());
                for v in [b.x0, b.y0, b.x1, b.y1] {
                    self.push_num(v)?;
                }
            }
            ClipPath => {
                let b = self.clip_bounds();
                self.take_path();
                self.g.path = b.to_path(0.1);
                self.g.cur = Some(b.origin());
                self.g.start = self.g.cur;
            }
            InitClip => self.g.clips.clear(),
            ClipSave => {
                if self.g.saved_clips.len() >= MAX_GSAVE {
                    return Err(PsError::Limit("too many saved clips"));
                }
                self.alloc(std::mem::size_of_val(self.g.clips.as_slice()))?;
                let clips = self.g.clips.as_slice().into();
                self.g.saved_clips.push(clips);
            }
            ClipRestore => {
                if let Some(clips) = self.g.saved_clips.pop() {
                    self.g.clips = clips.to_vec();
                }
            }
            Clip | EoClip => {
                let bp = self.g.path.clone();
                self.clip_with(bp, if op == EoClip { FillRule::EvenOdd } else { FillRule::NonZero });
            }
            RectClip => {
                let bp = self.rects()?;
                self.take_path();
                self.clip_with(bp, FillRule::NonZero);
            }
            Fill | EoFill => {
                let bp = self.take_path();
                self.fill_path(bp, if op == EoFill { FillRule::EvenOdd } else { FillRule::NonZero });
            }
            Stroke => {
                let bp = self.take_path();
                self.stroke_path(bp);
            }
            RectFill => {
                let bp = self.rects()?;
                self.fill_path(bp, FillRule::NonZero);
            }
            RectStroke => {
                let m = match self.stack.last() {
                    Some(Obj::Array { items, .. }) if items.len() == 6 => Some(self.pop_matrix()?),
                    _ => None,
                };
                let mut bp = self.rects()?;
                if let Some(m) = m {
                    let xf = self.xf();
                    bp.apply_affine(xf * m * xf.inverse());
                }
                self.stroke_path(bp);
            }
            ShFill => {
                let d = self.pop_dict()?;
                let m = self.xf();
                if let Some(g) = self.gradient(&d, m)? {
                    let area = self.clip_bounds();
                    let paint = Paint::Gradient(Box::new(g));
                    let clips = self.g.clips.clone();
                    self.out.fill(area.to_path(0.1), FillRule::NonZero, paint, self.g.overprint, &clips);
                    if let Some((_, n)) = self.out.drawn.last() {
                        let id = n.id;
                        self.out.shadings.push(id);
                    }
                }
            }
            _ => self.data_op(op)?,
        }
        Ok(())
    }
}

/// Where a stitched function of linear interpolations changes slope (its bounds), when it is
/// one; `None` for functions that must be sampled.
fn linear_points(f: &Obj) -> Option<Vec<f64>> {
    let Obj::Dict(d) = f else { return None };
    let d = d.borrow();
    let num = |k: &str| d.get(&Key::name(k)).and_then(Obj::as_num);
    match num("FunctionType")? {
        2.0 if num("N").unwrap_or(1.0) == 1.0 => Some(vec![]),
        3.0 => {
            let fs = d.get(&Key::name("Functions"))?.items()?.to_vec();
            if !fs.iter().all(|g| linear_points(g).is_some_and(|p| p.is_empty())) {
                return None;
            }
            d.get(&Key::name("Bounds"))?.items().map(|i| i.borrow().iter().filter_map(Obj::as_num).collect())
        }
        _ => None,
    }
}

/// The clip path a clipping group of one shading rectangle (`shfill` through a clip) can be
/// filled with instead.
pub(crate) fn collapse(nodes: &mut [Arc<Node>], shadings: &[NodeId]) {
    for n in nodes.iter_mut() {
        let NodeKind::Group { children, clip: true } = &n.kind else {
            if let Some(c) = Arc::make_mut(n).children_mut() {
                collapse(c, shadings);
            }
            continue;
        };
        if let [clip, fill] = children.as_slice()
            && shadings.contains(&fill.id)
            && let (NodeKind::Path { path, rule, clipping: true, .. }, NodeKind::Path { .. }) = (&clip.kind, &fill.kind)
        {
            let mut p = Node::path(fill.id, path.clone(), fill.appearance.clone());
            if let NodeKind::Path { rule: r, .. } = &mut p.kind {
                *r = *rule;
            }
            *n = Arc::new(p);
            continue;
        }
        if let Some(c) = Arc::make_mut(n).children_mut() {
            collapse(c, shadings);
        }
    }
}
