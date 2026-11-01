use super::state::{GState, Space, process, tiling_hash};
use super::{
    Affine, Arc, Color, DictRef, FUNCTION_OPS, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, HashSet, Interp, Key, LIMIT,
    MAX_APART, MAX_FUNCTION_VISITS, MAX_HIVAL, MAX_INKS, MAX_STITCH_STOPS, NESTED_PATTERNS, Node, NodeId, NodeKind, Obj, Paint, PatternDef, Point,
    PsError, REGISTRATION, Rc, Rect, Res, SHADING_SAMPLES, Shared, UNKNOWN_FUNCTIONS, UNKNOWN_SHADINGS, Vec2, clamp_between, ps_err,
};

impl Interp<'_> {
    // ---------- colour ----------

    /// A colour space operand.
    pub(in super::super) fn space_of(&mut self, o: &Obj, depth: u32) -> Res<Space> {
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

    pub(super) fn set_space(&mut self, o: Obj) -> Res {
        let space = self.space_of(&o, 0)?;
        let comps = space.initial();
        self.g.space = Rc::new(space);
        self.g.space_obj = o;
        self.set_comps(comps)
    }

    /// Set the colour to `comps` in the current space.
    pub(super) fn set_comps(&mut self, comps: Vec<f64>) -> Res {
        let space = self.g.space.clone();
        self.g.mesh = None;
        self.g.paint = self.paint_of(&space, &comps)?;
        self.g.comps = comps;
        Ok(())
    }

    /// Set a process colour, its space with it.
    pub(super) fn set_process(&mut self, space: Space, name: &str, comps: Vec<f64>) -> Res {
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
                        let full = match self.out.spot_colors.get(one) {
                            Some(c) => *c,
                            None => {
                                let c = self.tint_color(alt, tint, &[1.0])?;
                                self.out.spots.push((one.to_string(), c));
                                // The tint transform may have listed this ink already: the color
                                // listed first stays.
                                self.out.spot_colors.entry(one.clone()).or_insert(c);
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
    pub(super) fn current_rgb(&self) -> [f32; 3] {
        self.g.paint.color().map_or([0.0; 3], |c| c.to_rgb_uncalibrated())
    }

    /// `setcolor` in a pattern space: the pattern operand, and before it an uncoloured
    /// pattern's colour in the underlying space.
    pub(super) fn set_pattern(&mut self, base: Option<Rc<Space>>) -> Res {
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
        let m = d.borrow().get(&Key::name("VCMatrix")).and_then(|o| o.items().and_then(|i| crate::import::interp::matrix_of(&i.borrow())));
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
        let hash = tiling_hash(&key, tile, under.as_ref());
        let made = self.out.tilings.get(&hash).and_then(|list| list.iter().find(|(p, t, u, _)| p.same(&key) && *t == tile && *u == under));
        if let Some((.., name)) = made {
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
        // The document gets its patterns only here, so the n made so far are Pattern 1 to
        // Pattern n.
        let name = format!("Pattern {}", self.out.doc.patterns.len() + 1);
        let mut def = PatternDef::new(&name, art);
        def.tile = tile;
        self.out.doc.patterns.push(def);
        // A cell with a NaN corner equals no cell, so the lookup above could never find it.
        if !tile.is_nan() {
            self.out.tilings.entry(hash).or_insert_with(|| Vec::with_capacity(1)).push((key, tile, under, name.clone()));
        }
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
    pub(super) fn gradient(&mut self, sh: &DictRef, m: Affine) -> Res<Option<GradientPaint>> {
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
        let points = linear_points(&f, 0, &mut visits).filter(|p| p.len() <= MAX_STITCH_STOPS);
        let stitched = points.is_some();
        let ts: Vec<f64> = match points {
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
        let stop = |paint: Paint, offset: f64| match paint {
            Paint::Solid { color, swatch, tint } => GradientStop { swatch, tint, ..GradientStop::new(offset as f32, color) },
            _ => GradientStop::new(offset as f32, Color::BLACK),
        };
        for t in ts {
            let comps = self.eval(&f, &[d0 + (d1 - d0) * t], 0)?;
            let paint = self.paint_of(&space, &comps)?;
            let offset = if radial { (r0 + (r1 - r0) * t) / r1 } else { t };
            // At a bound of a stitched function the pieces need not meet (a hard colour change):
            // the colour just before the bound is a stop at the same offset, ahead of the one
            // after it.
            if stitched && t > 0.0 && t < 1.0 {
                let comps = self.eval(&f, &[d0 + (d1 - d0) * (t - 1e-9)], 0)?;
                let before = stop(self.paint_of(&space, &comps)?, offset);
                if before.color != stop(paint.clone(), offset).color {
                    stops.push(before);
                }
            }
            stops.push(stop(paint, offset));
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

    /// A function (types 0, 2 and 3; an array of them, one per component) at `x` (types 2 and 3
    /// take one input).
    pub fn eval(&mut self, f: &Obj, x: &[f64], depth: u32) -> Res<Vec<f64>> {
        self.eval_inner(f, x, depth, &mut 0)
    }

    /// [`Self::eval`], with `visits` (shared by the whole recursion, incremented on every call)
    /// counting the functions visited against [`MAX_FUNCTION_VISITS`], as [`linear_points`] counts
    /// its nodes. The depth limit alone stops a function that refers to itself, but not one that
    /// fans out through shared arrays: when four levels of arrays each have 100 entries that all
    /// refer to the same array of the level below (at the last level, the same function), one
    /// evaluation would visit 101,010,101 functions. Each function visited also counts
    /// [`FUNCTION_OPS`] against the operation budget.
    fn eval_inner(&mut self, f: &Obj, x: &[f64], depth: u32, visits: &mut usize) -> Res<Vec<f64>> {
        *visits += 1;
        if depth > 8 || *visits > MAX_FUNCTION_VISITS {
            return ps_err("limitcheck", "Function");
        }
        self.spend(FUNCTION_OPS)?;
        let d = match f {
            Obj::Dict(d) => d.clone(),
            Obj::Array { items, .. } => {
                // Each entry is at least one more function: an array with more entries than the
                // visits left raises `limitcheck` before it is copied.
                if items.len() > MAX_FUNCTION_VISITS - *visits {
                    return ps_err("limitcheck", "Function");
                }
                let fs = items.to_vec();
                let mut out = vec![];
                for g in &fs {
                    out.extend(self.eval_inner(g, x, depth + 1, visits)?.first().copied());
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
            [a, b, ..] => clamp_between(t, *a, *b),
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
                self.eval_inner(&g, &[u], depth + 1, visits)
            }
            _ => {
                self.out.warn(UNKNOWN_FUNCTIONS);
                let r = nums(get("Range"));
                Ok(r.chunks(2).map(|p| p.iter().sum::<f64>() / 2.0).collect())
            }
        }
    }
}

/// Is shading `sh` axial or radial (a gradient)?
fn is_gradient(sh: &DictRef) -> bool {
    matches!(sh.borrow().get(&Key::name("ShadingType")).and_then(Obj::as_num), Some(2.0 | 3.0))
}

/// Is shading `sh` one [`Interp::mesh_fill`] paints?
pub(super) fn is_mesh(sh: &DictRef) -> bool {
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
pub(crate) fn collapse(nodes: &mut [Arc<Node>], shadings: &HashSet<NodeId>) {
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
            let mut p = Node::path(fill.id, path.clone(), (*fill.appearance).clone());
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
