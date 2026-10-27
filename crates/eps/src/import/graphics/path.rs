use super::state::{ccw_turn, turn};
use super::*;

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

    /// Count `n` path elements an operator makes, reads or copies against the operation budget,
    /// an operation each: making or reading an element takes up to about as long as an operation,
    /// and copying one takes less (measured).
    pub(super) fn spend_elements(&mut self, n: usize) -> Res {
        self.spend(n as u64)
    }

    pub(super) fn move_to(&mut self, p: Point) -> Res {
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

    pub(super) fn line_to(&mut self, p: Point) -> Res {
        self.grow()?;
        if self.g.cur.is_none() {
            return ps_err("nocurrentpoint", "");
        }
        self.g.path.line_to(p);
        self.g.cur = Some(p);
        Ok(())
    }

    /// Path elements in user space appended (the first move a line when there is a current point),
    /// each counted against the operation budget.
    fn append_user(&mut self, els: impl Iterator<Item = PathEl>) -> Res {
        let xf = self.xf();
        for el in els {
            self.spend_elements(1)?;
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
                PathEl::ClosePath => {
                    self.grow()?;
                    self.g.path.close_path();
                }
            }
        }
        Ok(())
    }

    /// `arc` / `arcn`: from angle `a1` to `a2` (degrees, counter-clockwise or clockwise).
    pub(super) fn arc(&mut self, c: Point, r: f64, a1: f64, a2: f64, ccw: bool) -> Res {
        // `arc` raises `a2` by multiples of 360 until it is at least `a1`, and `arcn` lowers it
        // until it is at most `a1` (PLRM 3rd ed., `arc` and `arcn` in chapter 8). Those sweeps,
        // and the first endpoint, are worked out from what is left of each angle after whole turns.
        let sweep = if ccw && a2 < a1 {
            ccw_turn(a1, a2)
        } else if !ccw && a2 > a1 {
            -ccw_turn(a2, a1)
        } else {
            a2 - a1
        };
        let tau = std::f64::consts::TAU;
        let sweep = sweep.to_radians().clamp(-tau * 4.0, tau * 4.0);
        let a1 = (a1 % 360.0).to_radians();
        let start = c + Vec2::new(r * a1.cos(), r * a1.sin());
        self.append_user(std::iter::once(PathEl::MoveTo(start)))?;
        let arc = kurbo::Arc::new(c, Vec2::new(r, r), a1, sweep, 0.0);
        let tol = (0.01 / self.scale().max(1e-9)).max(1e-9);
        self.append_user(arc.append_iter(tol))
    }

    /// `arct` / `arcto`: a line towards `p1` rounded with radius `r` into the line to `p2`; the
    /// tangent points (user space).
    pub(super) fn arct(&mut self, p1: Point, p2: Point, r: f64) -> Res<[Point; 2]> {
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
    pub(super) fn rects(&mut self) -> Res<BezPath> {
        let nums: Vec<f64> = match self.stack.last() {
            Some(Obj::Array { .. }) => {
                let items = self.pop_array()?;
                // Five elements a rectangle, counted before the array is read.
                self.spend_elements(items.len() / 4 * 5)?;
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

    pub(super) fn fill_path(&mut self, bp: BezPath, rule: FillRule) -> Res {
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

    pub(super) fn stroke_path(&mut self, bp: BezPath) -> Res {
        if bp.elements().is_empty() || self.g.paint.is_none() || self.g.null {
            return Ok(());
        }
        let st = self.stroke_layer()?;
        self.out.stroke(bp, st, &self.g.clips);
        Ok(())
    }

    /// The stroke the graphics state draws, in document space. Each entry of the dash pattern it
    /// copies counts as an operation.
    pub(super) fn stroke_layer(&mut self) -> Res<StrokeLayer> {
        self.spend(self.g.dash.len() as u64)?;
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
        Ok(st)
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
    pub(super) fn path_for_all(&mut self) -> Res {
        let close = self.pop_proc()?;
        let curve = self.pop_proc()?;
        let line = self.pop_proc()?;
        let mv = self.pop_proc()?;
        self.spend_elements(self.g.path.elements().len())?;
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
    pub(super) fn user_path(&mut self, up: Obj) -> Res {
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
            Obj::Array { items, .. } => {
                // Each entry of the data array and each byte of the operator string counts as an
                // element read.
                self.spend_elements(items.len().saturating_add(ops.len()))?;
                items.borrow().iter().filter_map(Obj::as_num).collect()
            }
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
                // Each operator run counts as an operation.
                self.spend(1)?;
                self.op(op)?;
            }
        }
        Ok(())
    }

    /// `ufill`, `ueofill`, `ustroke` (with an optional matrix): paint a user path, keeping the
    /// current path.
    pub(super) fn paint_user_path(&mut self, op: Op) -> Res {
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
                Op::UStroke => self.stroke_path(bp),
                _ => self.fill_path(bp, if op == Op::UEoFill { FillRule::EvenOdd } else { FillRule::NonZero }),
            }
        });
        self.saved.truncate(depth);
        self.grestore();
        r
    }

    /// `upath`: the current path as a user path (a procedure, in user space).
    pub(super) fn upath(&mut self) -> Res {
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
    pub(super) fn exec_form(&mut self) -> Res {
        let d = self.pop_dict()?;
        let get = |k: &str| d.borrow().get(&Key::name(k)).cloned();
        let m = get("Matrix").and_then(|o| o.items().and_then(|i| crate::import::interp::matrix_of(&i.borrow()))).unwrap_or(Affine::IDENTITY);
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

    pub(in super::super) fn take_path(&mut self) -> BezPath {
        self.g.cur = None;
        self.g.start = None;
        std::mem::take(&mut self.g.path)
    }

    pub(super) fn clip_with(&mut self, path: BezPath, rule: FillRule) {
        let Some(c) = self.out.clip(path, rule) else { return };
        if self.g.clips.len() < MAX_NEST {
            self.g.clips.push(c);
        } else {
            self.out.warn(&deep_clips());
        }
    }

    /// The area the clip leaves, in document space (the page without a clip). The clip path read
    /// counts against the operation budget.
    pub(super) fn clip_bounds(&mut self) -> Res<Rect> {
        let (bounds, len) =
            self.g.clips.iter().rev().find_map(|c| c.region.as_ref()).map_or((self.out.frame, 0), |(p, _)| (p.bounding_box(), p.elements().len()));
        self.spend_elements(len)?;
        Ok(bounds)
    }
}
