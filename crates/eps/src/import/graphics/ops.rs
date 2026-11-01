use super::paint::is_mesh;
use super::state::{GState, Space};
use super::{
    Affine, Color, FillRule, Interp, Key, LineCap, LineJoin, MAX_DASH_ENTRIES, MAX_DASHES, MAX_GSAVE, MAX_PATH, Obj, Op, Paint, Point, PsError, Rc,
    Res, Shape, Vec2, matrix_obj, ps_err,
};

impl Interp<'_> {
    // ---------- the operators ----------

    fn replace_path_with_stroke_outline(&mut self) -> Res {
        let bp = self.take_path();
        let mut st = self.stroke_layer()?;
        // A dash pattern far finer than the path would make countless pieces.
        let period: f64 = st.dash.as_ref().map_or(0.0, |d| d.pattern.iter().sum());
        let periods = if st.dash.is_some() && period > 0.0 { bp.perimeter(1e-3) / period } else { 0.0 };
        if st.dash.is_some() && !(period > 0.0 && periods < MAX_DASHES) {
            st.dash = None;
        }
        let tol = (0.01 / self.scale().max(1e-9)).clamp(1e-4, 1.0);
        let outline = vectorcraft_effects::stroke::line_outline(&bp, &st, st.width, tol);
        // The path, each entry of the dash pattern in each period, and the outline count as elements.
        let dashes = st.dash.as_ref().map_or(0.0, |d| periods * d.pattern.len() as f64) as usize;
        self.spend_elements(bp.elements().len().saturating_add(dashes).saturating_add(outline.elements().len()))?;
        if outline.elements().len() >= MAX_PATH {
            return Err(PsError::Limit("a path has too many points"));
        }
        self.g.cur = outline.elements().iter().rev().find_map(|e| e.end_point());
        self.g.start = self.g.cur;
        self.g.path = outline;
        Ok(())
    }

    fn push_hsb_color(&mut self) -> Res {
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
        Ok(())
    }

    fn set_current_color(&mut self) -> Res {
        if let Space::Pattern(base) = &*self.g.space {
            self.set_pattern(base.clone())?;
        } else {
            let mut components = vec![0.0; self.g.space.n()];
            for slot in components.iter_mut().rev() {
                *slot = self.pop_num()?;
            }
            self.set_comps(components)?;
        }
        Ok(())
    }

    fn transform_point(&mut self, op: Op) -> Res {
        let m = match self.stack.last() {
            Some(Obj::Array { .. }) => self.pop_matrix()?,
            _ => self.g.ctm,
        };
        let [x, y] = self.nums()?;
        let inverse = matches!(op, Op::ITransform | Op::IDTransform);
        if inverse && m.determinant().abs() < 1e-12 {
            return ps_err("undefinedresult", "");
        }
        let m = if inverse { m.inverse() } else { m };
        let p = if matches!(op, Op::DTransform | Op::IDTransform) {
            let v = m * Point::new(x, y) - m * Point::ZERO;
            Point::new(v.x, v.y)
        } else {
            m * Point::new(x, y)
        };
        self.push_num(p.x)?;
        self.push_num(p.y)
    }

    // `[/Pattern <current space>] setcolorspace setcolor`, unless already in a pattern space.
    fn set_pattern_color(&mut self) -> Res {
        if !matches!(*self.g.space, Space::Pattern(_)) {
            self.g.space = Rc::new(Space::Pattern(Some(self.g.space.clone())));
            self.g.space_obj = Obj::array(vec![Obj::name("Pattern"), self.g.space_obj.clone()]);
        }
        let Space::Pattern(base) = &*self.g.space else { return ps_err("typecheck", "") };
        self.set_pattern(base.clone())
    }

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
            StrokePath => self.replace_path_with_stroke_outline()?,
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
            CurrentHsbColor => self.push_hsb_color()?,
            CurrentColorRendering => {
                let d = crate::import::obj::Dict::from([(Key::name("ColorRenderingType"), Obj::Int(1))]);
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
            GStateNew => {
                self.spend_elements(self.g.path.elements().len())?;
                self.alloc(std::mem::size_of_val(self.g.path.elements()))?;
                self.push(Obj::GState(Rc::new(self.g.clone())))?;
            }
            CurrentGState => {
                self.pop()?;
                self.spend_elements(self.g.path.elements().len())?;
                self.alloc(std::mem::size_of_val(self.g.path.elements()))?;
                self.push(Obj::GState(Rc::new(self.g.clone())))?;
            }
            SetGState => match self.pop()? {
                Obj::GState(g) => {
                    self.spend_elements(g.path.elements().len())?;
                    self.g = (*g).clone();
                }
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
                if items.len() > MAX_DASH_ENTRIES {
                    return ps_err("limitcheck", "");
                }
                // Each entry read counts as an operation.
                self.spend(items.len() as u64)?;
                let v: Option<Vec<f64>> = items.borrow().iter().map(|o| o.as_num().filter(|v| v.is_finite() && *v >= 0.0)).collect();
                self.g.dash = v.ok_or(PsError::Ps("typecheck", String::new()))?.into();
                self.g.dash_offset = offset;
            }
            CurrentDash => {
                // The array made counts against the memory budget.
                self.alloc(self.g.dash.len() * std::mem::size_of::<Obj>())?;
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
            SetColor => self.set_current_color()?,
            CurrentColor => {
                for v in self.g.comps.clone() {
                    self.push_num(v)?;
                }
            }
            SetPattern => self.set_pattern_color()?,
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
                let mut d = crate::import::obj::Dict::new();
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
                let mut d = crate::import::obj::Dict::new();
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
                let d: crate::import::obj::Dict = keys.iter().map(|k| (Key::name(k), Obj::Int(1 << 20))).collect();
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
            Transform | ITransform | DTransform | IDTransform => self.transform_point(op)?,
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
                    self.grow()?;
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
                self.spend_elements(self.g.path.elements().len())?;
                let b = xf.inverse().transform_rect_bbox(self.g.path.bounding_box());
                for v in [b.x0, b.y0, b.x1, b.y1] {
                    self.push_num(v)?;
                }
            }
            ClipPath => {
                let b = self.clip_bounds()?;
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
                self.spend_elements(self.g.path.elements().len())?;
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
                self.stroke_path(bp)?;
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
                self.stroke_path(bp)?;
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
                    let area = self.clip_bounds()?;
                    let paint = Paint::Gradient(Box::new(g));
                    let clips = self.g.clips.clone();
                    self.out.fill(area.to_path(0.1), FillRule::NonZero, paint, self.g.overprint, &clips);
                    if let Some((_, n)) = self.out.drawn.last() {
                        let id = n.id;
                        self.out.shadings.insert(id);
                    }
                }
            }
            _ => self.data_op(op)?,
        }
        Ok(())
    }
}
