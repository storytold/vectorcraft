use super::*;

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
    pub(super) fn initial(&self) -> Vec<f64> {
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
    /// The dash pattern, shared by the states saved or copied from this one.
    pub dash: Rc<[f64]>,
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
            dash: Rc::from([]),
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
    pub(super) groups: Vec<Arc<Clip>>,
    pub(super) too_deep: usize,
    /// Path elements (and the clips and groups of each object) drawn so far (see [`MAX_DRAWN`]).
    elements: usize,
    /// The path the last object was filled with: a stroke of the same path right after joins it.
    pub(super) merge: Option<BezPath>,
    /// Process colours painted, by model (the document opens in CMYK when most are).
    pub cmyk: usize,
    pub rgb: usize,
    /// Spot inks used: name and colour at full tint.
    pub spots: Vec<(String, Color)>,
    /// The color at full tint of each spot ink in [`Self::spots`], by name.
    pub(super) spot_colors: HashMap<Rc<str>, Color>,
    /// Rectangles painted with a shading (`shfill`): a clipping group of one is the clip path
    /// filled with it.
    pub shadings: HashSet<NodeId>,
    /// Tiling patterns made into pattern swatches: the pattern's `PaintProc` (a pattern made
    /// again with `makepattern` shares it) and cell, the colour an uncoloured one painted in, and
    /// the swatch's name. Each list holds the patterns of one [`tiling_hash`]; a pattern whose
    /// cell has a NaN corner is left out.
    pub(super) tilings: HashMap<u64, Vec<Tiling>>,
    /// How deep art drawn apart (a pattern's cell, a Type 3 glyph) is nested.
    pub apart: u32,
}

fn sane(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= LIMIT)
}

/// A tiling pattern made into a pattern swatch, as [`Out::tilings`] lists it.
pub(super) type Tiling = (Shared<Obj>, Rect, Option<Paint>, String);

/// A hash of what tells tiling patterns apart in [`Out::tilings`]. Patterns that compare equal
/// hash alike: each number goes in by its bits, and −0 goes in as 0.
pub(super) fn tiling_hash(proc: &Shared<Obj>, tile: Rect, under: Option<&Paint>) -> u64 {
    let mut h = DefaultHasher::new();
    proc.hash(&mut h);
    [tile.x0, tile.y0, tile.x1, tile.y1].map(|v| (v + 0.0).to_bits()).hash(&mut h);
    if let Some(Paint::Solid { color, swatch, tint }) = under {
        let c = match *color {
            Color::Rgb { r, g, b } => [r, g, b, 0.0],
            Color::Cmyk { c, m, y, k } => [c, m, y, k],
            Color::Gray { k } => [k, 0.0, 0.0, 0.0],
            Color::Lab { l, a, b } => [l, a, b, 0.0],
        };
        c.map(|v| (v + 0.0).to_bits()).hash(&mut h);
        swatch.hash(&mut h);
        (tint + 0.0).to_bits().hash(&mut h);
    }
    h.finish()
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
            spot_colors: HashMap::new(),
            shadings: HashSet::new(),
            tilings: HashMap::new(),
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
    pub(super) fn fill(&mut self, bp: BezPath, rule: FillRule, paint: Paint, overprint: bool, clips: &[Arc<Clip>]) {
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
    pub(super) fn stroke(&mut self, bp: BezPath, st: StrokeLayer, clips: &[Arc<Clip>]) {
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
    pub(super) fn clip(&mut self, path: BezPath, rule: FillRule) -> Option<Arc<Clip>> {
        // Only a polygon that really covers the page: a diamond touching its edges has the page as
        // its bounding box but cuts its corners away (#1542).
        if path.elements().len() <= 6 && vectorcraft_geom::hit::polygon_covers_rect(&path, self.frame, 0.5) {
            return None;
        }
        self.next_clip += 1;
        Some(Arc::new(Clip { id: self.next_clip, region: Some((path, rule)) }))
    }
}

/// The smallest angle from `a` to `b` (radians, −π to π).
pub(super) fn turn(a: f64, b: f64) -> f64 {
    let d = (b - a).rem_euclid(std::f64::consts::TAU);
    if d > std::f64::consts::PI { d - std::f64::consts::TAU } else { d }
}

/// The angle from `a` counterclockwise to `b` (degrees, 0 to 360), worked out from what is left
/// of each after whole turns.
pub(super) fn ccw_turn(a: f64, b: f64) -> f64 {
    (b % 360.0 - a % 360.0).rem_euclid(360.0)
}
