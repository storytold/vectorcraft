//! The graphics state and the operators that draw: coordinates, paths, colours, painting,
//! clipping and shadings. Painted paths become document objects as they are drawn ([`Out`]).

use std::rc::Rc;
use std::sync::Arc;

use kurbo::{PathEl, Shape};
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::clipnest::{Clip, Drawn, MAX_NEST, deep_clips};
use vectorcraft_doc::pattern::PatternDef;
use vectorcraft_doc::{Appearance, AppearanceItem, Dash, Document, FillLayer, LineCap, LineJoin, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, Vec2};

use super::interp::{Interp, MAX_GSAVE, matrix_obj};
use super::obj::{DictRef, Key, Obj, Op, PsError, Res, Shared, ps_err};

/// Most objects one file makes.
const MAX_NODES: usize = 1 << 20;
/// Objects reaching further than this from the page (points) come from damaged coordinates and
/// are left out.
const LIMIT: f64 = 1e6;
/// Width of a zero-width line (the thinnest a device draws), points.
const HAIRLINE: f64 = 0.25;
/// Most inks a `DeviceN` space has (PostScript's own limit).
const MAX_INKS: usize = 32;
/// Most dashes `strokepath` makes of a path (more and it outlines the path undashed).
const MAX_DASHES: f64 = 1e5;
/// Most entries an indexed colour space's table has.
const MAX_HIVAL: usize = 4095;
/// Most elements the current path may have.
const MAX_PATH: usize = 1 << 20;
/// Most path elements drawn in all (each object's clips and groups counting as elements too).
const MAX_DRAWN: usize = 1 << 24;
/// Samples taken of a shading function that isn't a plain interpolation.
const SHADING_SAMPLES: usize = 32;
/// Most bounds of a stitched linear function that become a gradient's stops: past it, the
/// function is sampled like any other (each stop evaluates the whole function).
const MAX_STITCH_STOPS: usize = 1024;

pub(super) const FAR_AWAY: &str = "objects far outside the page were left out";
pub(super) const TOO_MUCH: &str = "the file draws more than VectorCraft reads: the rest was left out";
const NESTED_PATTERNS: &str = "patterns nested too deeply are filled with mid-grey";
/// Deepest patterns and glyph procedures drawn inside each other's art.
pub(crate) const MAX_APART: u32 = 8;
const UNKNOWN_FUNCTIONS: &str = "shadings whose functions are of an unknown type are filled with their middle colour";
const UNKNOWN_SHADINGS: &str = "shadings of an unknown type were left out";

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
    /// Patterns; an uncoloured one paints in a colour of the underlying space.
    Pattern(Option<Rc<Space>>),
}

impl Space {
    /// Components a colour has in it.
    pub fn n(&self) -> usize {
        match self {
            Self::Gray | Self::Indexed { .. } | Self::Pattern(_) => 1,
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
    /// `nulldevice`: painting draws nothing.
    pub null: bool,
    /// The colour is a shading pattern whose shading is a mesh: a fill paints it (the shading
    /// and pattern space → the document).
    pub mesh: Option<(DictRef, Affine)>,
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
            null: false,
            mesh: None,
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
    /// Clip and group ids are given in the order they open, so a chain of both sorts by id.
    next_clip: u32,
    /// The groups open (outermost first), and how many more opened past [`MAX_NEST`] (their art
    /// goes in the innermost group kept).
    groups: Vec<Arc<Clip>>,
    too_deep: usize,
    /// Path elements (and the clips and groups of each object) drawn so far (see [`MAX_DRAWN`]).
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
    /// Tiling patterns made into pattern swatches: the pattern's `PaintProc` (a pattern made
    /// again with `makepattern` shares it) and cell, the colour an uncoloured one painted in, and
    /// the swatch's name.
    tilings: Vec<(Shared<Obj>, Rect, Option<Paint>, String)>,
    /// How deep art drawn apart (a pattern's cell, a Type 3 glyph) is nested.
    pub apart: u32,
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
            groups: vec![],
            too_deep: 0,
            elements: 0,
            merge: None,
            cmyk: 0,
            rgb: 0,
            spots: vec![],
            shadings: vec![],
            tilings: vec![],
            apart: 0,
        }
    }

    pub fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    /// Open a group: what is drawn until it ends ([`Self::end_group`]) goes in it.
    pub fn begin_group(&mut self) {
        if self.groups.len() >= MAX_NEST {
            self.too_deep += 1;
            self.warn(&format!("groups nested more than {MAX_NEST} deep were read as part of the group around them"));
            return;
        }
        self.next_clip += 1;
        self.groups.push(Arc::new(Clip { id: self.next_clip, region: None }));
    }

    /// End the innermost group open (an end without a beginning ends nothing).
    pub fn end_group(&mut self) {
        if self.too_deep > 0 {
            self.too_deep -= 1;
        } else {
            self.groups.pop();
        }
    }

    /// The chain an object drawn under `clips` is drawn in: the clips and the groups open, in the
    /// order they opened.
    pub fn chain(&self, clips: &[Arc<Clip>]) -> Vec<Arc<Clip>> {
        if self.groups.is_empty() {
            return clips.to_vec();
        }
        let mut chain = Vec::with_capacity(clips.len() + self.groups.len());
        let (mut a, mut b) = (clips.iter().peekable(), self.groups.iter().peekable());
        while let (Some(x), Some(y)) = (a.peek(), b.peek()) {
            let next = if x.id < y.id { a.next() } else { b.next() };
            chain.extend(next.cloned());
        }
        chain.extend(a.chain(b).cloned());
        chain
    }

    /// Add `node` under `clips` (in the groups open); `None` when it was left out.
    pub fn push(&mut self, mut node: Node, clips: &[Arc<Clip>]) -> Option<NodeId> {
        self.merge = None;
        let chain = self.chain(clips);
        let size = node.path_data().map_or(1, |p| p.subpaths.iter().map(|s| s.anchors.len()).sum()) + chain.len();
        self.elements = self.elements.saturating_add(size);
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
        self.drawn.push((chain, node));
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
        if self.merge.as_ref() == Some(&bp) {
            let here = self.chain(clips);
            if let Some((chain, last)) = self.drawn.last_mut()
                && chain.iter().map(|c| c.id).eq(here.iter().map(|c| c.id))
            {
                last.appearance.items.push(AppearanceItem::Stroke(st));
                self.merge = None;
                return;
            }
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
        Some(Arc::new(Clip { id: self.next_clip, region: Some((path, rule)) }))
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

    fn fill_path(&mut self, bp: BezPath, rule: FillRule) -> Res {
        if bp.elements().is_empty() || self.g.null {
            return Ok(());
        }
        // A mesh pattern paints its shading inside the path.
        if let Some((sh, m)) = self.g.mesh.clone() {
            let clips = self.g.clips.clone();
            self.clip_with(bp, rule);
            let r = self.mesh_fill(&sh, m);
            self.g.clips = clips;
            return r;
        }
        if self.g.paint.is_none() {
            return Ok(());
        }
        let (paint, overprint) = (self.g.paint.clone(), self.g.overprint);
        self.out.fill(bp, rule, paint, overprint, &self.g.clips);
        Ok(())
    }

    fn stroke_path(&mut self, bp: BezPath) {
        if bp.elements().is_empty() || self.g.paint.is_none() || self.g.null {
            return;
        }
        let st = self.stroke_layer();
        self.out.stroke(bp, st, &self.g.clips);
    }

    /// The stroke the graphics state draws, in document space.
    fn stroke_layer(&self) -> StrokeLayer {
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
        st
    }

    /// The current path in user space, each element as its operator and points (quadratic
    /// segments as cubic ones).
    fn user_elements(&self) -> Res<Vec<(Op, Vec<Point>)>> {
        let xf = self.xf();
        if xf.determinant().abs() < 1e-12 {
            return ps_err("undefinedresult", "");
        }
        let inv = xf.inverse();
        let mut last = Point::ZERO;
        let mut out = Vec::with_capacity(self.g.path.elements().len());
        for el in self.g.path.elements() {
            let (op, pts) = match *el {
                PathEl::MoveTo(p) => (Op::MoveTo, vec![p]),
                PathEl::LineTo(p) => (Op::LineTo, vec![p]),
                PathEl::QuadTo(a, p) => (Op::CurveTo, vec![last + (a - last) * (2.0 / 3.0), p + (a - p) * (2.0 / 3.0), p]),
                PathEl::CurveTo(a, b, p) => (Op::CurveTo, vec![a, b, p]),
                PathEl::ClosePath => (Op::ClosePath, vec![]),
            };
            last = pts.last().copied().unwrap_or(last);
            out.push((op, pts.into_iter().map(|p| inv * p).collect()));
        }
        Ok(out)
    }

    /// `pathforall`: the current path in user space, each element through its procedure.
    fn path_for_all(&mut self) -> Res {
        let close = self.pop_proc()?;
        let curve = self.pop_proc()?;
        let line = self.pop_proc()?;
        let mv = self.pop_proc()?;
        for (op, pts) in self.user_elements()? {
            for p in pts {
                self.push_num(p.x)?;
                self.push_num(p.y)?;
            }
            let proc = match op {
                Op::MoveTo => &mv,
                Op::LineTo => &line,
                Op::CurveTo => &curve,
                _ => &close,
            };
            if !self.body(proc)? {
                break;
            }
        }
        Ok(())
    }

    /// Append user path `up` to the current path: a procedure of path operators, or the encoded
    /// form (numbers and a string of operator codes).
    fn user_path(&mut self, up: Obj) -> Res {
        let items = up.items().cloned().ok_or(PsError::Ps("typecheck", "a user path".into()))?;
        let encoded = match (items.get(0), items.get(1), items.len()) {
            (Some(nums @ (Obj::Array { .. } | Obj::Str(_))), Some(Obj::Str(ops)), 2) => Some((nums, ops.to_vec())),
            _ => None,
        };
        let Some((nums, ops)) = encoded else { return self.call(Obj::Array { items, exec: true }) };
        // The operators by code, with how many operands each takes.
        const OPS: [(Op, usize); 12] = [
            (Op::SetBBox, 4),
            (Op::MoveTo, 2),
            (Op::RMoveTo, 2),
            (Op::LineTo, 2),
            (Op::RLineTo, 2),
            (Op::CurveTo, 6),
            (Op::RCurveTo, 6),
            (Op::Arc, 5),
            (Op::Arcn, 5),
            (Op::Arct, 5),
            (Op::ClosePath, 0),
            (Op::UCache, 0),
        ];
        let nums: Vec<f64> = match nums {
            Obj::Array { items, .. } => items.borrow().iter().filter_map(Obj::as_num).collect(),
            _ => return ps_err("typecheck", "an encoded user path"),
        };
        let mut next = nums.into_iter();
        let mut repeat = 1usize;
        for code in ops {
            if code >= 32 {
                repeat = usize::from(code - 32);
                continue;
            }
            let &(op, n) = OPS.get(usize::from(code)).ok_or(PsError::Ps("rangecheck", "a user path".into()))?;
            for _ in 0..std::mem::replace(&mut repeat, 1) {
                for _ in 0..n {
                    let v = next.next().ok_or(PsError::Ps("rangecheck", "a user path".into()))?;
                    self.push_num(v)?;
                }
                self.op(op)?;
            }
        }
        Ok(())
    }

    /// `ufill`, `ueofill`, `ustroke` (with an optional matrix): paint a user path, keeping the
    /// current path.
    fn paint_user_path(&mut self, op: Op) -> Res {
        let m = match (op, self.stack.last()) {
            (Op::UStroke, Some(Obj::Array { items, exec: false })) if items.len() == 6 && self.stack.len() >= 2 => Some(self.pop_matrix()?),
            _ => None,
        };
        let up = self.pop()?;
        self.gsave()?;
        let depth = self.saved.len();
        self.take_path();
        let r = self.user_path(up).and_then(|()| {
            if let Some(m) = m {
                self.g.ctm *= m;
            }
            let bp = self.take_path();
            match op {
                Op::UStroke => {
                    self.stroke_path(bp);
                    Ok(())
                }
                _ => self.fill_path(bp, if op == Op::UEoFill { FillRule::EvenOdd } else { FillRule::NonZero }),
            }
        });
        self.saved.truncate(depth);
        self.grestore();
        r
    }

    /// `upath`: the current path as a user path (a procedure, in user space).
    fn upath(&mut self) -> Res {
        self.pop_bool()?;
        let elements = self.user_elements()?;
        let b = self.xf().inverse().transform_rect_bbox(self.g.path.bounding_box());
        let mut items: Vec<Obj> = [b.x0, b.y0, b.x1, b.y1].into_iter().map(Obj::Real).collect();
        items.push(Obj::Op(Op::SetBBox));
        for (op, pts) in elements {
            items.extend(pts.iter().flat_map(|p| [Obj::Real(p.x), Obj::Real(p.y)]));
            items.push(Obj::Op(op));
        }
        self.alloc(items.len() * std::mem::size_of::<Obj>())?;
        self.push(Obj::proc(items))
    }

    /// `execform`: the form's `PaintProc` run with its matrix, clipped to its box.
    fn exec_form(&mut self) -> Res {
        let d = self.pop_dict()?;
        let get = |k: &str| d.borrow().get(&Key::name(k)).cloned();
        let m = get("Matrix").and_then(|o| o.items().and_then(|i| super::interp::matrix_of(&i.borrow()))).unwrap_or(Affine::IDENTITY);
        let bbox: Vec<f64> = get("BBox").and_then(|o| o.items().map(|i| i.borrow().iter().filter_map(Obj::as_num).collect())).unwrap_or_default();
        let paint = get("PaintProc").ok_or(PsError::Ps("undefined", "PaintProc".into()))?;
        self.gsave()?;
        let depth = self.saved.len();
        self.g.ctm *= m;
        if let [x0, y0, x1, y1] = bbox[..] {
            let bp = self.rect_path(x0, y0, x1 - x0, y1 - y0);
            self.take_path();
            self.clip_with(bp, FillRule::NonZero);
        }
        self.push(Obj::Dict(d))?;
        let r = self.call(paint);
        self.saved.truncate(depth);
        self.grestore();
        r
    }

    pub(super) fn take_path(&mut self) -> BezPath {
        self.g.cur = None;
        self.g.start = None;
        std::mem::take(&mut self.g.path)
    }

    fn clip_with(&mut self, path: BezPath, rule: FillRule) {
        let Some(c) = self.out.clip(path, rule) else { return };
        if self.g.clips.len() < MAX_NEST {
            self.g.clips.push(c);
        } else {
            self.out.warn(&deep_clips());
        }
    }

    /// The area the clip leaves, in document space (the page without a clip).
    fn clip_bounds(&self) -> Rect {
        self.g.clips.iter().rev().find_map(|c| c.region.as_ref()).map_or(self.out.frame, |(p, _)| p.bounding_box())
    }

    // ---------- colour ----------

    /// A colour space operand.
    pub(super) fn space_of(&mut self, o: &Obj, depth: u32) -> Res<Space> {
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
            "Pattern" => Space::Pattern(match items.get(1) {
                Some(base) => Some(Rc::new(self.space_of(base, depth + 1)?)),
                None => None,
            }),
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
                if names.len() > MAX_INKS {
                    return ps_err("limitcheck", "DeviceN");
                }
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
        self.g.mesh = None;
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
            Space::Pattern(_) => Paint::None,
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

    /// `setcolor` in a pattern space: the pattern operand, and before it an uncoloured
    /// pattern's colour in the underlying space.
    fn set_pattern(&mut self, base: Option<Rc<Space>>) -> Res {
        let Obj::Dict(d) = self.pop()? else { return ps_err("typecheck", "") };
        let uncoloured = d.borrow().get(&Key::name("PaintType")).and_then(Obj::as_num) == Some(2.0);
        let under = match base.filter(|_| uncoloured) {
            Some(base) => {
                let mut c = vec![0.0; base.n()];
                for slot in c.iter_mut().rev() {
                    *slot = self.pop_num()?;
                }
                Some(self.paint_of(&base, &c)?)
            }
            None => None,
        };
        self.g.mesh = None;
        self.g.paint = self.pattern_paint(&d, under)?;
        Ok(())
    }

    /// A pattern colour: a shading pattern's gradient, a tiling pattern's swatch (`under`: the
    /// colour an uncoloured one paints in).
    fn pattern_paint(&mut self, d: &DictRef, under: Option<Paint>) -> Res<Paint> {
        let kind = d.borrow().get(&Key::name("PatternType")).and_then(Obj::as_num);
        let sh = d.borrow().get(&Key::name("Shading")).cloned();
        let m = d.borrow().get(&Key::name("VCMatrix")).and_then(|o| o.items().and_then(|i| super::interp::matrix_of(&i.borrow())));
        // Pattern space → the document.
        let m = self.out.page * m.unwrap_or(self.g.ctm);
        match (kind, sh) {
            (Some(2.0), Some(Obj::Dict(sh))) if is_mesh(&sh) => {
                self.g.mesh = Some((sh, m));
                Ok(Paint::None)
            }
            (Some(2.0), Some(Obj::Dict(sh))) => Ok(self.gradient(&sh, m)?.map_or(Paint::None, |g| Paint::Gradient(Box::new(g)))),
            (Some(2.0), _) => ps_err("typecheck", "Shading"),
            _ => self.tiling(d, under, m),
        }
    }

    /// A tiling pattern as a pattern swatch (made the first time): its `PaintProc` drawn in
    /// pattern space, one cell of `XStep` × `YStep` from the corner of its `BBox`; `m` maps pattern
    /// space onto the document.
    fn tiling(&mut self, d: &DictRef, under: Option<Paint>, m: Affine) -> Res<Paint> {
        let get = |k: &str| d.borrow().get(&Key::name(k)).cloned();
        let bbox: Vec<f64> = get("BBox").and_then(|o| o.items().map(|i| i.borrow().iter().filter_map(Obj::as_num).collect())).unwrap_or_default();
        let [x0, y0, x1, y1] = bbox[..] else { return ps_err("rangecheck", "BBox") };
        let step = |k: &str| get(k).and_then(|o| o.as_num()).map(f64::abs).filter(|v| v.is_finite() && *v > 1e-9 && *v < LIMIT);
        let (Some(xs), Some(ys)) = (step("XStep"), step("YStep")) else { return ps_err("rangecheck", "XStep") };
        let tile = Rect::from_origin_size((x0.min(x1), y0.min(y1)), (xs, ys));
        let xf = m * Affine::translate(tile.origin().to_vec2());
        let proc = get("PaintProc").ok_or(PsError::Ps("undefined", "PaintProc".into()))?;
        let key = proc.items().cloned().ok_or(PsError::Ps("typecheck", "PaintProc".into()))?;
        if let Some((.., name)) = self.out.tilings.iter().find(|(p, t, u, _)| p.same(&key) && *t == tile && *u == under) {
            return Ok(Paint::Pattern { pattern: name.clone(), xf });
        }
        if self.out.apart >= MAX_APART {
            self.out.warn(NESTED_PATTERNS);
            return Ok(Paint::solid(Color::gray(0.5)));
        }
        let paint = under.clone();
        let art = self.draw_apart(Rect::new(x0, y0, x1, y1).abs(), |it| {
            if let Some(p) = paint {
                it.g.paint = p;
            }
            it.push(Obj::Dict(d.clone()))?;
            it.call(proc)
        })?;
        if art.is_empty() {
            return Ok(Paint::None);
        }
        let name =
            (self.out.doc.patterns.len() + 1..).map(|i| format!("Pattern {i}")).find(|n| self.out.doc.pattern(n).is_none()).unwrap_or_default();
        let mut def = PatternDef::new(&name, art);
        def.tile = tile;
        self.out.doc.patterns.push(def);
        self.out.tilings.push((key, tile, under, name.clone()));
        Ok(Paint::Pattern { pattern: name, xf })
    }

    /// The art `draw` draws on its own, in its own space (user space is the document's, the page
    /// `frame`), from a fresh graphics state with the current font: a pattern's cell, a glyph.
    pub fn draw_apart(&mut self, frame: Rect, draw: impl FnOnce(&mut Self) -> Res) -> Res<Vec<Arc<Node>>> {
        let fresh = GState { font: self.g.font.clone(), ..GState::default() };
        let g = std::mem::replace(&mut self.g, fresh);
        let saved = std::mem::take(&mut self.saved);
        let drawn = std::mem::take(&mut self.out.drawn);
        let page = std::mem::replace(&mut self.out.page, Affine::IDENTITY);
        let frame = std::mem::replace(&mut self.out.frame, frame);
        let merge = self.out.merge.take();
        // The art isn't in the groups open around it.
        let groups = std::mem::take(&mut self.out.groups);
        let too_deep = std::mem::take(&mut self.out.too_deep);
        self.out.apart += 1;
        let r = draw(self);
        self.out.apart -= 1;
        let art = std::mem::replace(&mut self.out.drawn, drawn);
        (self.out.page, self.out.frame, self.out.merge) = (page, frame, merge);
        (self.out.groups, self.out.too_deep) = (groups, too_deep);
        (self.g, self.saved) = (g, saved);
        r?;
        let mut nodes = vectorcraft_doc::clipnest::nest(&mut self.out.doc, art);
        if !self.out.shadings.is_empty() {
            collapse(&mut nodes, &self.out.shadings);
        }
        Ok(nodes)
    }

    /// An axial or radial shading as a gradient, `m` mapping its space onto the document.
    fn gradient(&mut self, sh: &DictRef, m: Affine) -> Res<Option<GradientPaint>> {
        let get = |k: &str| sh.borrow().get(&Key::name(k)).cloned();
        let kind = get("ShadingType").and_then(|o| o.as_num()).unwrap_or(0.0);
        if !is_gradient(sh) {
            self.out.warn(UNKNOWN_SHADINGS);
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
        // Where the colour changes: the ends, and the bounds of stitched linear functions (unless
        // there are too many).
        let mut visits = 0;
        let ts: Vec<f64> = match linear_points(&f, 0, &mut visits).filter(|p| p.len() <= MAX_STITCH_STOPS) {
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
            let comps = self.eval(&f, &[d0 + (d1 - d0) * t], 0)?;
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
        let mut g = GradientPaint::new(Gradient { kind: gk, stops });
        g.geom = Some(geom);
        Ok(Some(g))
    }

    /// A function (types 0, 2 and 3; an array of them, one per component) at `x` (types 2 and 3
    /// take one input).
    pub fn eval(&mut self, f: &Obj, x: &[f64], depth: u32) -> Res<Vec<f64>> {
        if depth > 8 {
            return ps_err("limitcheck", "Function");
        }
        let d = match f {
            Obj::Dict(d) => d.clone(),
            Obj::Array { items, .. } => {
                let fs = items.to_vec();
                let mut out = vec![];
                for g in &fs {
                    out.extend(self.eval(g, x, depth + 1)?.first().copied());
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
        let t = x.first().copied().unwrap_or(0.0);
        let t = match domain.as_slice() {
            [a, b, ..] => t.clamp(a.min(*b), a.max(*b)),
            _ => t,
        };
        match get("FunctionType").and_then(|o| o.as_num()) {
            Some(0.0) => self.sampled(&d, x),
            Some(2.0) => {
                let c0 = nums(get("C0"));
                let c1 = nums(get("C1"));
                let (c0, c1) = (if c0.is_empty() { vec![0.0] } else { c0 }, if c1.is_empty() { vec![1.0] } else { c1 });
                let n = get("N").and_then(|o| o.as_num()).unwrap_or(1.0);
                let x = t.max(0.0).powf(n);
                Ok(c0.iter().zip(&c1).map(|(a, b)| a + x * (b - a)).collect())
            }
            Some(3.0) => {
                // Read the pieces through the borrow and clone only the one we recurse into: a
                // `Functions` array of millions (shared, or reached again through a cycle) must
                // not be copied on every call and at every live recursion level.
                let functions = get("Functions");
                let fs = functions.as_ref().and_then(Obj::items);
                let n = fs.map_or(0, Shared::len);
                let bounds = nums(get("Bounds"));
                let encode = nums(get("Encode"));
                let (lo, hi) = (domain.first().copied().unwrap_or(0.0), domain.get(1).copied().unwrap_or(1.0));
                let k = bounds.iter().take_while(|b| t >= **b).count().min(n.saturating_sub(1));
                let a = if k == 0 { lo } else { bounds.get(k - 1).copied().unwrap_or(lo) };
                let b = bounds.get(k).copied().unwrap_or(hi);
                let (e0, e1) = (encode.get(2 * k).copied().unwrap_or(0.0), encode.get(2 * k + 1).copied().unwrap_or(1.0));
                let u = if b != a { e0 + (t - a) / (b - a) * (e1 - e0) } else { e0 };
                let g = fs.and_then(|fs| fs.get(k)).ok_or(PsError::Ps("rangecheck", "Functions".into()))?;
                self.eval(&g, &[u], depth + 1)
            }
            _ => {
                self.out.warn(UNKNOWN_FUNCTIONS);
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
                self.g = GState { font, null: self.g.null, ..GState::default() };
            }
            NullDevice => {
                self.g.null = true;
                self.g.ctm = Affine::IDENTITY;
                self.g.clips.clear();
            }
            ExecForm => self.exec_form()?,
            StrokePath => {
                let bp = self.take_path();
                let mut st = self.stroke_layer();
                // A dash pattern far finer than the path would make countless pieces.
                let period: f64 = st.dash.as_ref().map_or(0.0, |d| d.pattern.iter().sum());
                if st.dash.is_some() && !(period > 0.0 && bp.perimeter(1e-3) / period < MAX_DASHES) {
                    st.dash = None;
                }
                let tol = (0.01 / self.scale().max(1e-9)).clamp(1e-4, 1.0);
                let outline = vectorcraft_effects::stroke::line_outline(&bp, &st, st.width, tol);
                if outline.elements().len() >= MAX_PATH {
                    return Err(PsError::Limit("a path has too many points"));
                }
                self.g.cur = outline.elements().iter().rev().find_map(|e| e.end_point());
                self.g.start = self.g.cur;
                self.g.path = outline;
            }
            PathForAll => self.path_for_all()?,
            UFill | UEoFill | UStroke => self.paint_user_path(op)?,
            UAppend => {
                let up = self.pop()?;
                self.user_path(up)?;
            }
            UPath => self.upath()?,
            SetBBox => {
                self.nums::<4>()?;
            }
            CurrentHsbColor => {
                let [r, g, b] = self.current_rgb();
                let (max, min) = (r.max(g).max(b), r.min(g).min(b));
                let d = max - min;
                let h = if d <= 0.0 {
                    0.0
                } else if max == r {
                    ((g - b) / d).rem_euclid(6.0) / 6.0
                } else if max == g {
                    ((b - r) / d + 2.0) / 6.0
                } else {
                    ((r - g) / d + 4.0) / 6.0
                };
                let s = if max > 0.0 { d / max } else { 0.0 };
                for v in [h, s, max] {
                    self.push_num(f64::from(v))?;
                }
            }
            CurrentColorRendering => {
                let d = super::obj::Dict::from([(Key::name("ColorRenderingType"), Obj::Int(1))]);
                self.push(Obj::dict(d))?;
            }
            FindColorRendering => {
                self.pop()?;
                self.push(Obj::name("DefaultColorRendering"))?;
                self.push(Obj::Bool(false))?;
            }
            CurrentColorScreen => {
                for _ in 0..4 {
                    self.push(Obj::Real(60.0))?;
                    self.push(Obj::Real(45.0))?;
                    self.push(Obj::proc(vec![]))?;
                }
            }
            CurrentSmoothness => self.push(Obj::Real(0.02))?,
            SetHalftonePhase => {
                self.nums::<2>()?;
            }
            CurrentHalftonePhase => {
                self.push(Obj::Int(0))?;
                self.push(Obj::Int(0))?;
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
                if let Space::Pattern(base) = &*self.g.space {
                    self.set_pattern(base.clone())?;
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
            // `[/Pattern <current space>] setcolorspace setcolor`, unless in a pattern space already.
            SetPattern => {
                if !matches!(*self.g.space, Space::Pattern(_)) {
                    self.g.space = Rc::new(Space::Pattern(Some(self.g.space.clone())));
                    self.g.space_obj = Obj::array(vec![Obj::name("Pattern"), self.g.space_obj.clone()]);
                }
                let Space::Pattern(base) = &*self.g.space else { return ps_err("typecheck", "") };
                self.set_pattern(base.clone())?;
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
            // The cache and stack sizes programs read (they `get` them without asking first).
            CurrentUserParams | CurrentSystemParams => {
                let keys: &[&str] = if op == CurrentUserParams {
                    &["MaxFontItem", "MaxFormItem", "MaxPatternItem", "MaxUPathItem", "MaxOpStack", "MaxDictStack", "MaxExecStack"]
                } else {
                    &["MaxFontCache", "MaxFormCache", "MaxPatternCache", "MaxUPathCache", "MaxScreenStorage", "MaxDisplayList"]
                };
                let d: super::obj::Dict = keys.iter().map(|k| (Key::name(k), Obj::Int(1 << 20))).collect();
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
                self.fill_path(bp, if op == EoFill { FillRule::EvenOdd } else { FillRule::NonZero })?;
            }
            Stroke => {
                let bp = self.take_path();
                self.stroke_path(bp);
            }
            RectFill => {
                let bp = self.rects()?;
                self.fill_path(bp, FillRule::NonZero)?;
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
                if self.g.null {
                    return Ok(());
                }
                if is_mesh(&d) {
                    let m = self.xf();
                    return self.mesh_fill(&d, m);
                }
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

/// Is shading `sh` axial or radial (a gradient)?
fn is_gradient(sh: &DictRef) -> bool {
    matches!(sh.borrow().get(&Key::name("ShadingType")).and_then(Obj::as_num), Some(2.0 | 3.0))
}

/// Is shading `sh` one [`Interp::mesh_fill`] paints?
fn is_mesh(sh: &DictRef) -> bool {
    matches!(sh.borrow().get(&Key::name("ShadingType")).and_then(Obj::as_num), Some(1.0 | 4.0 | 5.0 | 6.0 | 7.0))
}

/// Where a stitched function of linear interpolations changes slope (its bounds), when it is
/// one; `None` for functions that must be sampled. `depth` bounds the nesting the same way
/// [`Interp::eval`] does (`> 8`) and `visits` (shared by the whole recursion, incremented on
/// every call) bounds the total nodes looked at. Both are needed: the depth limit alone stops a
/// function that refers to itself in a cycle, but not one that fans out through shared children
/// (1025 references to a child a few levels deep is 1025^levels nodes without the visit cap). A
/// well-formed stitch of linear pieces is one level, at most `MAX_STITCH_STOPS + 1` pieces, so it
/// costs at most `MAX_STITCH_STOPS + 2` visits and is never cut off.
fn linear_points(f: &Obj, depth: u32, visits: &mut usize) -> Option<Vec<f64>> {
    *visits += 1;
    if depth > 8 || *visits > 2 * (MAX_STITCH_STOPS + 1) {
        return None;
    }
    let Obj::Dict(d) = f else { return None };
    let d = d.borrow();
    let num = |k: &str| d.get(&Key::name(k)).and_then(Obj::as_num);
    match num("FunctionType")? {
        2.0 if num("N").unwrap_or(1.0) == 1.0 => Some(vec![]),
        3.0 => {
            let items = d.get(&Key::name("Functions"))?.items()?;
            // More pieces than the stop cap allows: sample the function instead (tested before
            // `to_vec`, so a huge `Functions` array is not copied only to be rejected). Each stop
            // runs [`Interp::eval`], which reads one piece, so a short `Bounds` with a huge
            // `Functions` would still be O(stops * Functions) to find each piece without this.
            if items.len() > MAX_STITCH_STOPS + 1 {
                return None;
            }
            let fs = items.to_vec();
            if !fs.iter().all(|g| linear_points(g, depth + 1, visits).is_some_and(|p| p.is_empty())) {
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
