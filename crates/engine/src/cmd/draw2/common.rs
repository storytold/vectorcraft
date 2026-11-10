use super::{
    AngleProfile, Appearance, Arc, Beside, Cap, Color, Document, EngineError, FillRule, Join, Node, NodeId, NodeKind, Paint, PathData, Point,
    PressureProfile, Rect, Result, Selection, Session, SubPath, Value, Vec2, bad, index, json, number_or, po,
};
use vectorcraft_doc::{LineCap, LineJoin, StrokeLayer, WidthProfile};

// ---------- helpers ----------

/// `[[x,y]…]` or `[{x,y}…]`.
pub(super) fn points_param(p: &Value, key: &str, cmd: &str) -> Result<Vec<Point>> {
    Ok(samples_param(p, key, cmd)?.into_iter().map(|s| s.pt).collect())
}

/// One point of a stroke, with what a pen read there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Sample {
    pub(super) pt: Point,
    /// Pressure, 0..1.
    pressure: Option<f64>,
    /// Tilt `[x, y]` in degrees, each −90..90, in document axes (x positive leaning right, y
    /// positive leaning down the page).
    tilt: Option<[f64; 2]>,
    /// Barrel rotation, degrees clockwise.
    rotation: Option<f64>,
}

/// The keys of a point's pen values, in the order a point given as an array holds them after its
/// x and y.
const PEN_KEYS: [&str; 4] = ["pressure", "tiltX", "tiltY", "rotation"];

/// The point `[x, y, …]` or `{x, y, …}` that `v` holds, or an error when its x or y is not a finite
/// number.
pub(super) fn point_entry(v: &Value, cmd: &str) -> Result<Point> {
    let xy = match v {
        Value::Array(xy) => xy.first().and_then(Value::as_f64).zip(xy.get(1).and_then(Value::as_f64)),
        Value::Object(_) => v.get("x").and_then(Value::as_f64).zip(v.get("y").and_then(Value::as_f64)),
        _ => None,
    };
    xy.filter(|(x, y)| x.is_finite() && y.is_finite()).map(|(x, y)| Point::new(x, y)).ok_or_else(|| bad(cmd, "x and y must both be numbers"))
}

/// [`points_param`] with what a pen read at each point, as far as it is given:
/// `[[x,y,pressure,tiltX,tiltY,rotation]…]` or `[{x,y,pressure,tiltX,tiltY,rotation}…]`. Pressure
/// is 0..1; tilt in degrees, each −90..90 (one of the two without the other is 0); rotation in
/// degrees clockwise. A point whose x or y is not a number, or whose pressure, tilt or rotation is
/// neither a number nor null, is an error.
pub(super) fn samples_param(p: &Value, key: &str, cmd: &str) -> Result<Vec<Sample>> {
    let a = p.get(key).and_then(Value::as_array).ok_or_else(|| bad(cmd, format!("missing array `{key}`")))?;
    let pts: Vec<Sample> = a
        .iter()
        .map(|v| {
            let mut pen = [None; 4];
            for (i, (key, slot)) in PEN_KEYS.iter().zip(&mut pen).enumerate() {
                let given = match v {
                    Value::Array(xy) => xy.get(i + 2),
                    _ => v.get(*key),
                };
                *slot = match given {
                    None | Some(Value::Null) => None,
                    Some(n @ Value::Number(_)) => n.as_f64(),
                    Some(v) => return Err(bad(cmd, format!("`{key}` must be a number, not {v}"))),
                };
            }
            let [pressure, tx, ty, rotation] = pen;
            let tilt = (tx.is_some() || ty.is_some()).then(|| [tx, ty].map(|t| t.unwrap_or(0.0).clamp(-90.0, 90.0)));
            Ok(Sample { pt: point_entry(v, cmd)?, pressure, tilt, rotation })
        })
        .collect::<Result<_>>()?;
    if pts.is_empty() {
        return Err(bad(cmd, "need at least one point"));
    }
    Ok(pts)
}

/// Where each sample of a freehand stroke is along its polyline, as a fraction of its length.
fn along(samples: &[Sample]) -> Vec<f64> {
    let mut at = 0.0;
    let mut prev: Option<Point> = None;
    let dist: Vec<f64> = samples
        .iter()
        .map(|s| {
            at += prev.map_or(0.0, |q| q.distance(s.pt));
            prev = Some(s.pt);
            at
        })
        .collect();
    dist.into_iter().map(|s| if at > 1e-9 { s / at } else { 0.0 }).collect()
}

/// The pen pressure along a freehand stroke (positions measured along its polyline), when any of
/// its samples has one; samples without one count as full pressure, as a pen event without one.
fn stroke_pressure(samples: &[Sample]) -> Option<PressureProfile> {
    if samples.iter().all(|s| s.pressure.is_none()) {
        return None;
    }
    PressureProfile::from_samples(along(samples).into_iter().zip(samples).map(|(t, s)| (t, s.pressure.unwrap_or(1.0))))
}

/// A pen that leans less than this many degrees from upright leans no way in particular: its
/// bearing is the one before it.
const UPRIGHT: f64 = 1.0;

/// How far the pen leaned from upright (0 upright to 1 flat), which way (degrees
/// counter-clockwise on the page from the x axis) and how far its barrel turned (degrees
/// counter-clockwise) along a freehand stroke, each when any sample has it. A sample without the
/// tilt or the rotation that others have counts as half way over (45°) or unturned, which draws a
/// brush's own value. The bearing of a pen within 1° of upright is the bearing before it (the
/// first one the stroke has, at its start).
fn stroke_pose(samples: &[Sample]) -> (Option<PressureProfile>, Option<AngleProfile>, Option<AngleProfile>) {
    let at = along(samples);
    // The tilt's lean as a direction on the page, and the angle from upright (as Pointer Events'
    // tiltX and tiltY convert to an altitude and an azimuth).
    let lean = |t: [f64; 2]| t.map(|d| d.to_radians().tan());
    let from_upright = |t: [f64; 2]| {
        let [x, y] = lean(t);
        x.hypot(y).atan().to_degrees()
    };
    let tilt = samples.iter().any(|s| s.tilt.is_some()).then(|| {
        let tilts = at.iter().zip(samples).map(|(t, s)| (*t, s.tilt.map_or(PressureProfile::MID, |q| from_upright(q) / 90.0)));
        PressureProfile::from_samples(tilts)
    });
    let bearing = samples.iter().any(|s| s.tilt.is_some()).then(|| {
        let leans: Vec<Option<f64>> =
            samples.iter().map(|s| s.tilt.filter(|q| from_upright(*q) >= UPRIGHT).map(lean).map(|[x, y]| (-y).atan2(x).to_degrees())).collect();
        let mut last = leans.iter().flatten().next().copied().unwrap_or(0.0);
        AngleProfile::from_samples(at.iter().zip(leans).map(|(t, b)| {
            last = b.unwrap_or(last);
            (*t, last)
        }))
    });
    let rotation = samples
        .iter()
        .any(|s| s.rotation.is_some())
        .then(|| AngleProfile::from_samples(at.iter().zip(samples).map(|(t, s)| (*t, -s.rotation.unwrap_or(0.0)))));
    (tilt.flatten(), bearing.flatten(), rotation.flatten())
}

/// What a pen recorded along a stroke ([`StrokeLayer`]'s `pressure`, `tilt`, `bearing` and
/// `rotation`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct PenRecord {
    pressure: Option<PressureProfile>,
    tilt: Option<PressureProfile>,
    bearing: Option<AngleProfile>,
    rotation: Option<AngleProfile>,
}

impl PenRecord {
    /// What a freehand stroke's samples recorded.
    pub(super) fn of_samples(samples: &[Sample]) -> Self {
        let (tilt, bearing, rotation) = stroke_pose(samples);
        Self { pressure: stroke_pressure(samples), tilt, bearing, rotation }
    }
    pub(super) fn of(st: &StrokeLayer) -> Self {
        Self { pressure: st.pressure.clone(), tilt: st.tilt.clone(), bearing: st.bearing.clone(), rotation: st.rotation.clone() }
    }
    pub(super) fn is_empty(&self) -> bool {
        *self == Self::default()
    }
    /// The record of a path `old_len` long (this one) continued at its end by a stroke
    /// `added_len` long (`added`).
    pub(super) fn extended(&self, old_len: f64, added: &Self, added_len: f64) -> Self {
        Self {
            pressure: PressureProfile::extended(self.pressure.as_ref(), old_len, added.pressure.as_ref(), added_len),
            tilt: PressureProfile::extended(self.tilt.as_ref(), old_len, added.tilt.as_ref(), added_len),
            bearing: AngleProfile::extended(self.bearing.as_ref(), old_len, added.bearing.as_ref(), added_len),
            rotation: AngleProfile::extended(self.rotation.as_ref(), old_len, added.rotation.as_ref(), added_len),
        }
    }
    /// The record along the path reversed.
    pub(super) fn reversed(&self) -> Self {
        Self {
            pressure: self.pressure.as_ref().map(PressureProfile::reversed),
            tilt: self.tilt.as_ref().map(PressureProfile::reversed),
            bearing: self.bearing.as_ref().map(AngleProfile::reversed),
            rotation: self.rotation.as_ref().map(AngleProfile::reversed),
        }
    }
    /// Keep the record on stroke `st`.
    pub(super) fn write(self, st: &mut StrokeLayer) {
        st.pressure = self.pressure;
        st.tilt = self.tilt;
        st.bearing = self.bearing;
        st.rotation = self.rotation;
    }
}

/// Blob Brush and Eraser Tool Options' Size set to Pressure (`sizeMode: "pressure"`, with
/// `sizeVariation` in points): the stroke's pen pressure along it and the variation, `None` for
/// Fixed (the default), no variation or a stroke without pressure (a mouse), which keeps the size.
pub(super) fn size_pressure(p: &Value, samples: &[Sample], cmd: &str) -> Result<Option<(PressureProfile, f64)>> {
    match p.get("sizeMode").filter(|v| !v.is_null()).map(|v| v.as_str().ok_or(v)) {
        None | Some(Ok("fixed")) => Ok(None),
        Some(Ok("pressure")) => {
            let variation = number_or(p, "sizeVariation", 0.0, cmd)?;
            if !(variation.is_finite() && variation >= 0.0) {
                return Err(bad(cmd, "sizeVariation must be 0 or more"));
            }
            Ok(stroke_pressure(samples).filter(|_| variation > 0.0).map(|prof| (prof, variation)))
        }
        Some(Ok(other)) => Err(bad(cmd, format!("sizeMode is \"fixed\" or \"pressure\", not `{other}`"))),
        Some(Err(other)) => Err(bad(cmd, format!("sizeMode is \"fixed\" or \"pressure\", not {other}"))),
    }
}

/// The area a stroke along `spine` covers with Size set to Pressure: as wide at each point as
/// [`vectorcraft_tools::draw2::pressure_size`] gives for the pressure `prof` records there, with
/// round ends and joins, its overlaps united into one outline.
pub(super) fn pressure_region(spine: &PathData, size: f64, variation: f64, prof: &PressureProfile) -> Result<PathData> {
    let widest = size + variation;
    let width = |t: f64| vectorcraft_tools::draw2::pressure_size(size, variation, prof.at(t)) / widest;
    let profile = WidthProfile { points: prof.points.iter().map(|&(t, _)| (t, width(t), width(t))).collect() };
    let round = StrokeLayer { cap: LineCap::Round, join: LineJoin::Round, ..StrokeLayer::new(Paint::None, widest) };
    let outline = PathData::from_bezpath(&vectorcraft_render::width_outline(&spine.to_bezpath(), widest, &profile, &round, 0.05));
    po::try_unite_all(&[(&outline, FillRule::NonZero)]).map_err(|e| EngineError::Other(e.to_string()))
}

/// The length of a subpath.
pub(super) fn subpath_len(sp: &SubPath) -> f64 {
    use kurbo::ParamCurveArclen;
    PathData::single(sp.clone()).to_bezpath().segments().map(|s| s.arclen(0.01)).sum()
}

/// The bound, in points, on the length of a `path.reshapeSegment` delta and on the x and y of a
/// point given to `path.convertAnchor` or `path.freehand`. The blend spine commands set the same
/// bound (`object.blend.spine.reshapeSegment`, `object.blend.spine.moveAnchor`).
pub(super) const MAX_INPUT: f64 = 1e9;

/// Whether `q` is finite and its x and y are each smaller than [`MAX_INPUT`] in size.
pub(super) fn within_input_bound(q: Point) -> bool {
    q.x.is_finite() && q.y.is_finite() && q.x.abs() < MAX_INPUT && q.y.abs() < MAX_INPUT
}

/// The whole number of 0 or more that `key` holds, read with [`index`].
pub(super) fn usize_req(p: &Value, key: &str, cmd: &str) -> Result<usize> {
    p.get(key).and_then(Value::as_u64).map(index).ok_or_else(|| bad(cmd, format!("missing integer `{key}`")))
}

/// The whole number of 0 or more that `key` holds, or `None` when `key` is absent or null. Any other
/// value is [`usize_req`]'s error.
pub(in super::super) fn usize_opt(p: &Value, key: &str, cmd: &str) -> Result<Option<usize>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => usize_req(p, key, cmd).map(Some),
    }
}

pub(super) fn path_mut(d: &mut Document, id: NodeId) -> Result<&mut PathData> {
    let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
    match &mut n.kind {
        NodeKind::Path { path, live, .. } => {
            *live = None;
            Ok(path)
        }
        _ => Err(EngineError::Other(format!("object {id} is not a path"))),
    }
}

pub(super) fn path_of(d: &Document, id: NodeId) -> Result<(PathData, FillRule)> {
    match d.node(id).map(|n| &n.kind) {
        Some(NodeKind::Path { path, rule, .. }) => Ok((path.clone(), *rule)),
        Some(_) => Err(EngineError::Other(format!("object {id} is not a path"))),
        None => Err(EngineError::NoNode(id)),
    }
}

pub(super) fn dist_to_polyline(pts: &[Point], q: Point) -> f64 {
    if pts.len() == 1 {
        return pts[0].distance(q);
    }
    pts.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let ab = b - a;
            let l2 = ab.hypot2();
            let t = if l2 < 1e-18 { 0.0 } else { ((q - a).dot(ab) / l2).clamp(0.0, 1.0) };
            (a + ab * t).distance(q)
        })
        .fold(f64::INFINITY, f64::min)
}

pub(super) fn poly_bounds(pts: &[Point], pad: f64) -> Rect {
    let r = pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p));
    r.inflate(pad, pad)
}

/// The filled area painted by a round brush of diameter `size` along `pts`.
pub(super) fn brush_region(pts: &[Point], size: f64) -> PathData {
    let mut pts = pts.to_vec();
    if pts.len() == 1 {
        pts.push(pts[0] + Vec2::new(0.01, 0.0));
    }
    po::outline_stroke(&PathData::single(SubPath::polyline(&pts, false)), size, Cap::Round, Join::Round, 4.0)
}

/// Path leaves the area tools act on: under the selected objects, or every path when nothing is
/// selected. Only editable, non-guide paths whose bounds meet `area`.
pub(super) fn target_paths(doc: &Document, sel: &Selection, area: Rect) -> Vec<NodeId> {
    let mut out = vec![];
    let mut push = |n: &Node| {
        if let NodeKind::Path { guide: false, path, .. } = &n.kind
            && path.control_bounds().is_some_and(|b| b.inflate(0.5, 0.5).intersect(area).area() > 0.0 || b.width() == 0.0 || b.height() == 0.0)
            && !out.contains(&n.id)
        {
            out.push(n.id);
        }
    };
    if sel.is_empty() {
        doc.walk(&mut push);
    } else {
        for id in &sel.objects {
            if let Some(n) = doc.node(*id) {
                n.walk(&mut |c: &Node| push(c));
            }
        }
    }
    let editable = doc.editable();
    out.retain(|id| editable(*id));
    out
}

/// [`target_paths`] for the tools that cut fills, with each target's geometry and fill rule: the
/// members of a compound path whose members are all closed paths give the compound path itself,
/// once, so its fill (holes included) is cut as one shape under its own fill rule.
pub(super) fn fill_targets(doc: &Document, sel: &Selection, area: Rect) -> Result<Vec<(NodeId, PathData, FillRule)>> {
    let closed_compound = |n: &Node| {
        matches!(&n.kind, NodeKind::Compound { children, .. }
            if children.iter().all(|c| matches!(&c.kind, NodeKind::Path { path, .. } if path.is_closed())))
    };
    let editable = doc.editable();
    let mut out: Vec<(NodeId, PathData, FillRule)> = vec![];
    for id in target_paths(doc, sel, area) {
        let compound = doc.parent_of(id).and_then(|p| doc.node(p)).filter(|p| closed_compound(p) && editable(p.id));
        let target = compound.map_or(id, |c| c.id);
        if out.iter().any(|t| t.0 == target) {
            continue;
        }
        let (pd, rule) = match compound.map(|c| &c.kind) {
            Some(NodeKind::Compound { children, rule }) => {
                (PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.clone()).collect()), *rule)
            }
            _ => path_of(doc, id)?,
        };
        out.push((target, pd, rule));
    }
    Ok(out)
}

/// The members of a compound path with geometry `pd`: one path per subpath, with fresh ids.
fn compound_members(d: &mut Document, pd: PathData) -> Vec<Arc<Node>> {
    pd.subpaths.into_iter().map(|sp| Arc::new(Node::path(d.alloc_id(), PathData::single(sp), Appearance::default()))).collect()
}

/// Give path or compound path `id` the geometry `pd` (dropping a live shape; a compound's members
/// are rebuilt, one per subpath).
pub(in super::super) fn set_geometry(d: &mut Document, id: NodeId, pd: PathData) -> Result<()> {
    if matches!(d.node(id).map(|n| &n.kind), Some(NodeKind::Compound { .. })) {
        let members = compound_members(d, pd);
        if let Some(NodeKind::Compound { children, .. }) = d.node_mut(id).map(|n| &mut n.kind) {
            *children = members;
        }
        return Ok(());
    }
    *path_mut(d, id)? = pd;
    Ok(())
}

/// A copy of path or compound path node `id` (fresh ids, no live shape) with geometry `pd`.
pub(super) fn sibling_with(d: &mut Document, id: NodeId, pd: PathData) -> Result<Node> {
    let mut n = d.node(id).ok_or(EngineError::NoNode(id))?.clone();
    n.id = d.alloc_id();
    match &mut n.kind {
        NodeKind::Path { path, live, .. } => {
            *path = pd;
            *live = None;
        }
        NodeKind::Compound { children, .. } => *children = compound_members(d, pd),
        _ => {}
    }
    Ok(n)
}

/// Replace path or compound path `id` with `pieces` (first keeps the id, the rest go right above
/// it; none removes it). `cut` removes the path or puts the other pieces above it. Returns ids.
pub(in super::super) fn replace_with_pieces(d: &mut Document, cut: &mut Beside, id: NodeId, pieces: Vec<PathData>) -> Result<Vec<NodeId>> {
    let mut it = pieces.into_iter();
    let Some(first) = it.next() else {
        cut.put(d, id, vec![])?;
        return Ok(vec![]);
    };
    let mut ids = vec![id];
    let mut above = vec![];
    for pd in it {
        let n = sibling_with(d, id, pd)?;
        ids.push(n.id);
        above.push(n);
    }
    set_geometry(d, id, first)?;
    if !above.is_empty() {
        let n = d.node(id).cloned().ok_or(EngineError::NoNode(id))?;
        cut.put(d, id, [n].into_iter().chain(above).collect())?;
    }
    Ok(ids)
}

pub(in super::super) fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

pub(super) fn stroke_paint(s: &Session) -> Paint {
    if s.paint.stroke.is_none() { Paint::solid(Color::BLACK) } else { s.paint.stroke.clone() }
}

pub(super) fn path_kind(path: PathData) -> NodeKind {
    NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: false, guide: false }
}
