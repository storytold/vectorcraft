//! The Dimension tool. Hover a segment (linear), a corner (angular) or a circle or perfect arc
//! (radial) and drag outwards: releasing places the linked dimension. Plain clicks pick what to
//! measure instead: two points (linear) or two straight lines (angular), then a click places it;
//! a click on a circle or arc gives its radius. Escape cancels without leaving any artwork.
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};
use serde_json::{Value, json};
use vectorcraft_doc::{
    Node,
    dimension::{self, Dimension, DimensionMode, DimensionOptions, DimensionSource, PointRef, SegmentRef},
};
use vectorcraft_geom::{BezPath, Point, Vec2};

/// The three kinds of dimension the tool draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Linear,
    Angular,
    Radial,
}

impl Kind {
    pub fn id(self) -> &'static str {
        match self {
            Kind::Linear => "linear",
            Kind::Angular => "angular",
            Kind::Radial => "radial",
        }
    }
    /// `radius` and `diameter` are the radial kind.
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "linear" => Some(Kind::Linear),
            "angular" => Some(Kind::Angular),
            "radial" | "radius" | "diameter" => Some(Kind::Radial),
            _ => None,
        }
    }
}

/// What the pointer is over.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    Segment(SegmentRef),
    Corner { first: SegmentRef, second: SegmentRef, at: Point },
    Radial { source: DimensionSource, center: Point, radius: f64 },
}

#[derive(Default)]
pub struct DimensionTool {
    kind: Kind,
    style: DimensionOptions,
    /// Where the button went down, and what was under it.
    press: Option<(Point, Option<Target>)>,
    /// Plain-click picks: points (linear) or straight lines (angular).
    points: Vec<(Point, Option<PointRef>)>,
    lines: Vec<SegmentRef>,
    /// A dimension being placed: previews follow the pointer.
    pending: Option<Dimension>,
    dragging: bool,
    hover: Option<Target>,
    at: Point,
    /// Whether Shift was down at the last pointer event: the picked line shows constrained.
    shift: bool,
    began: bool,
    /// Smart Guides for the Linear picks, hovering too: the guides show before each click.
    snap: crate::guides::DrawSnap,
    /// Where the next Linear pick lands, as last worked out for the pointer (for the overlays).
    snapped: Option<(Point, Option<PointRef>)>,
}

/// `at` as a Shift click picks it: the direction from `last` snapped to 45° steps (from the
/// constrain angle), the distance kept.
fn constrained(cx: &ToolContext, last: Point, at: Point) -> Point {
    last + vectorcraft_geom::constrain_angle_from(at - last, 45.0, cx.constrain_angle)
}

/// Nearest editable segment, with bounded work and no snapping to dimension artwork itself.
pub fn pick(cx: &ToolContext, p: Point) -> Option<SegmentRef> {
    let mut best = None;
    let mut dist = cx.tol(8.0);
    let mut budget = 20_000usize;
    fn visit(n: &Node, p: Point, budget: &mut usize, dist: &mut f64, best: &mut Option<SegmentRef>) {
        if *budget == 0 || !n.visible || n.locked || n.extras().dimension.is_some() {
            return;
        }
        if let Some(path) = n.path_data() {
            for (si, sp) in path.subpaths.iter().enumerate() {
                for i in 0..sp.segment_count() {
                    if *budget == 0 {
                        return;
                    }
                    *budget -= 1;
                    use vectorcraft_geom::kurbo::ParamCurveNearest;
                    let near = sp.segment(i).nearest(p, 1e-4);
                    if near.distance_sq < *dist * *dist {
                        *dist = near.distance_sq.sqrt();
                        *best = Some(SegmentRef { node: n.id, subpath: si, segment: i });
                    }
                }
            }
        }
        if let Some(ch) = n.children() {
            for c in ch.iter().rev() {
                visit(c, p, budget, dist, best);
            }
        }
    }
    for n in cx.doc.layers.iter().rev() {
        visit(n, p, &mut budget, &mut dist, &mut best);
    }
    best
}

/// The anchor of `edge` within reach of `p`, if any.
fn anchor_near(cx: &ToolContext, edge: &SegmentRef, p: Point) -> Option<(usize, Point)> {
    let sp = cx.doc.node(edge.node)?.path_data()?.subpaths.get(edge.subpath)?;
    let next = if edge.segment + 1 < sp.anchors.len() { edge.segment + 1 } else { 0 };
    [edge.segment, next]
        .into_iter()
        .filter_map(|i| Some((i, sp.anchors.get(i)?.p)))
        .filter(|(_, a)| a.distance(p) <= cx.tol(8.0))
        .min_by(|(_, a), (_, b)| a.distance(p).total_cmp(&b.distance(p)))
}

/// The point of `edge` closest to `p`.
fn nearest_on(cx: &ToolContext, edge: &SegmentRef, p: Point) -> Option<Point> {
    use vectorcraft_geom::kurbo::ParamCurveNearest;
    let sp = cx.doc.node(edge.node)?.path_data()?.subpaths.get(edge.subpath)?;
    if edge.segment >= sp.segment_count() {
        return None;
    }
    let c = sp.segment(edge.segment);
    let t = c.nearest(p, 1e-4).t;
    use vectorcraft_geom::kurbo::ParamCurve;
    let q = c.eval(t);
    (q.x.is_finite() && q.y.is_finite()).then_some(q)
}

impl DimensionTool {
    /// Where a Linear pick at `p` lands: on an anchor of the line under the pointer, within reach
    /// (kept as a link to its path); else where Smart Guides put it (another object's anchor or
    /// centre, in line with one, or for the second point on a construction guide through the
    /// first), its guides showing; else on the closest point of that line; else `p` itself.
    fn snap_linear(&mut self, cx: &ToolContext, p: Point) -> (Point, Option<PointRef>) {
        let leave = self.points.last().map(|(first, _)| crate::guides::Leave::segment(cx, *first, false));
        let guided = self.snap.hover(cx, p, &[], leave.as_ref());
        let edge = pick(cx, p);
        if let Some(edge) = &edge
            && let Some((anchor, point)) = anchor_near(cx, edge, p)
        {
            self.snap.clear();
            return (point, Some(PointRef { node: edge.node, subpath: edge.subpath, anchor }));
        }
        if guided.distance(p) > 1e-9 {
            return (guided, None);
        }
        // Not on an anchor or a guide: snap to the closest point of the line (#2432).
        (edge.and_then(|e| nearest_on(cx, &e, p)).unwrap_or(p), None)
    }
    fn target(&self, cx: &ToolContext, p: Point) -> Option<Target> {
        let edge = pick(cx, p)?;
        match self.kind {
            Kind::Linear => Some(Target::Segment(edge)),
            Kind::Angular => {
                let (anchor, at) = anchor_near(cx, &edge, p)?;
                let (first, second) = dimension::corner(cx.doc, edge.node, edge.subpath, anchor)?;
                Some(Target::Corner { first, second, at })
            }
            Kind::Radial => {
                if let Some((center, radius)) = dimension::circle(cx.doc, edge.node) {
                    return Some(Target::Radial { source: DimensionSource::Circle { node: edge.node }, center, radius });
                }
                let (center, radius) = dimension::arc(cx.doc, &edge)?;
                Some(Target::Radial { source: DimensionSource::Arc { edge }, center, radius })
            }
        }
    }
    fn recipe(&self, mode: DimensionMode, a: Point, b: Point, vertex: Point, source: Option<DimensionSource>) -> Dimension {
        Dimension { mode, a, b, vertex, offset: 0.0, length: a.distance(b), options: self.style.clone(), source, moved: None }
    }
    /// The dimension a target gives when dragged from.
    fn dimension_for(&self, target: &Target, p: Point) -> Option<Dimension> {
        let mut d = match target {
            Target::Segment(edge) => self.recipe(
                DimensionMode::Linear,
                Point::ZERO,
                Point::new(1.0, 0.0),
                Point::ZERO,
                Some(DimensionSource::Segment { edge: edge.clone() }),
            ),
            Target::Corner { first, second, .. } => self.recipe(
                DimensionMode::Angular,
                Point::ZERO,
                Point::new(1.0, 0.0),
                Point::ZERO,
                Some(DimensionSource::Angle { first: first.clone(), second: second.clone() }),
            ),
            Target::Radial { source, center, radius } => {
                // Dragged outwards: the radius; inwards: the diameter.
                let mode = if p.distance(*center) >= *radius { DimensionMode::Radius } else { DimensionMode::Diameter };
                self.recipe(mode, *center, p, Point::ZERO, Some(source.clone()))
            }
        };
        d.offset = 30.0;
        Some(d)
    }
    fn params(d: &Dimension) -> Value {
        let mut p = json!({"mode":d.mode,"a":[d.a.x,d.a.y],"b":[d.b.x,d.b.y],"vertex":[d.vertex.x,d.vertex.y],"offset":d.offset,"options":d.options});
        if let Some(source) = &d.source {
            p["source"] = json!(source);
        }
        p
    }
    fn preview(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        let Some(d) = &mut self.pending else { return vec![] };
        if matches!(d.mode, DimensionMode::Radius | DimensionMode::Diameter)
            && let Some(Target::Radial { center, radius, .. }) = self.press.as_ref().and_then(|(_, t)| t.as_ref())
        {
            d.mode = if p.distance(*center) >= *radius { DimensionMode::Radius } else { DimensionMode::Diameter };
        }
        d.place(p);
        if d.resolve(cx.doc).is_err() {
            return vec![];
        }
        let mut actions = vec![];
        if !self.began {
            actions.push(Action::Begin("Create Dimension".into()));
            self.began = true;
        }
        actions.push(Action::Preview("dimension.create".into(), Self::params(d)));
        actions
    }
    /// Places the pending dimension at `p` and commits it.
    fn place(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        let mut out = self.preview(cx, p);
        if self.began {
            out.push(Action::Commit);
        }
        self.reset();
        out
    }
    fn reset(&mut self) {
        self.pending = None;
        self.press = None;
        self.points.clear();
        self.lines.clear();
        self.dragging = false;
        self.began = false;
        self.snap.clear();
        self.snapped = None;
    }
    /// A plain click: picks a point or a line, or measures a circle's radius.
    fn click(&mut self, cx: &ToolContext, p: Point, target: Option<Target>, mods: Mods) -> Vec<Action> {
        match self.kind {
            Kind::Linear => {
                let (mut at, mut reference) = self.snap_linear(cx, p);
                if mods.shift
                    && let Some((last, _)) = self.points.last()
                {
                    at = constrained(cx, *last, at);
                    reference = reference.filter(|r| r.position(cx.doc) == Some(at));
                }
                self.points.push((at, reference));
                if let [(a, ra), (b, rb)] = self.points.as_slice() {
                    let source = (ra.is_some() || rb.is_some()).then(|| DimensionSource::Points { a: ra.clone(), b: rb.clone(), vertex: None });
                    let mut d = self.recipe(DimensionMode::Linear, *a, *b, Point::ZERO, source);
                    if d.resolve(cx.doc).is_ok() {
                        self.pending = Some(d);
                    }
                    self.points.clear();
                }
                vec![]
            }
            Kind::Angular => {
                let Some(edge) = pick(cx, p).filter(|e| e.is_straight(cx.doc)) else { return vec![] };
                if self.lines.first().is_some_and(|first| *first == edge) {
                    return vec![];
                }
                self.lines.push(edge);
                if let [first, second] = self.lines.as_slice() {
                    let mut d = self.recipe(
                        DimensionMode::Angular,
                        Point::ZERO,
                        Point::new(1.0, 0.0),
                        Point::ZERO,
                        Some(DimensionSource::Angle { first: first.clone(), second: second.clone() }),
                    );
                    if d.resolve(cx.doc).is_ok() {
                        d.offset = 30.0;
                        self.pending = Some(d);
                    }
                    self.lines.clear();
                }
                vec![]
            }
            Kind::Radial => {
                let Some(target @ Target::Radial { .. }) = target else { return vec![] };
                let Some(mut d) = self.dimension_for(&target, p) else { return vec![] };
                d.mode = DimensionMode::Radius;
                self.pending = Some(d);
                self.press = Some((p, Some(target)));
                self.place(cx, p)
            }
        }
    }
}

impl Tool for DimensionTool {
    fn id(&self) -> &'static str {
        "dimension"
    }
    fn busy(&self) -> bool {
        self.pending.is_some() || !self.points.is_empty() || !self.lines.is_empty()
    }
    fn cursor(&self, _: &ToolContext, _: Point, _: Mods) -> Cursor {
        Cursor::Crosshair
    }
    fn options(&self) -> Value {
        json!({"mode":self.kind.id(),"style":self.style})
    }
    fn set_option(&mut self, key: &str, value: &Value) {
        if key == "mode"
            && let Some(kind) = value.as_str().and_then(Kind::parse)
        {
            self.kind = kind;
        }
        if key == "style"
            && let Ok(o) = serde_json::from_value::<DimensionOptions>(value.clone())
            && o.validate().is_ok()
        {
            self.style = o;
        }
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        self.at = ev.pos;
        self.shift = ev.mods.shift;
        match ev.kind {
            PointerKind::Move => {
                if self.pending.is_some() {
                    // Placing: the picks are done, their guides go.
                    self.snap.clear();
                    self.snapped = None;
                    return self.preview(cx, ev.pos);
                }
                self.hover = self.target(cx, ev.pos);
                self.snapped = (self.kind == Kind::Linear && self.lines.is_empty()).then(|| self.snap_linear(cx, ev.pos));
                if self.snapped.is_none() {
                    self.snap.clear();
                }
                vec![]
            }
            PointerKind::Down => {
                if self.pending.is_some() && !self.dragging {
                    // The second click of a picked measurement places it.
                    return self.place(cx, ev.pos);
                }
                self.press = Some((ev.pos, self.target(cx, ev.pos)));
                vec![]
            }
            PointerKind::Drag => {
                if self.pending.is_none()
                    && let Some((start, Some(target))) = self.press.clone()
                    && start.distance(ev.pos) > cx.tol(3.0)
                    && self.points.is_empty()
                    && self.lines.is_empty()
                {
                    self.pending = self.dimension_for(&target, ev.pos);
                    self.dragging = self.pending.is_some();
                }
                if self.dragging { self.preview(cx, ev.pos) } else { vec![] }
            }
            PointerKind::Up => {
                if self.dragging {
                    return self.place(cx, ev.pos);
                }
                match self.press.take() {
                    Some((start, target)) if self.pending.is_none() => self.click(cx, start, target, ev.mods),
                    _ => vec![],
                }
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _: &ToolContext, key: ToolKey, _: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            let cancel = self.began;
            self.reset();
            return if cancel { vec![Action::Cancel] } else { vec![] };
        }
        vec![]
    }
    fn deactivate(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.key(cx, ToolKey::Escape, Mods::default())
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let color = [60, 150, 255];
        let mut out = vec![];
        let segment = |edge: &SegmentRef, out: &mut Vec<Overlay>| {
            if let Some(sp) = cx.doc.node(edge.node).and_then(Node::path_data).and_then(|p| p.subpaths.get(edge.subpath))
                && edge.segment < sp.segment_count()
            {
                let c = sp.segment(edge.segment);
                let mut path = BezPath::new();
                path.move_to(c.p0);
                path.curve_to(c.p1, c.p2, c.p3);
                out.push(Overlay::Path { path, color, width: 3.0, dashed: false });
            }
        };
        if self.pending.is_none() {
            match &self.hover {
                Some(Target::Segment(edge)) => {
                    segment(edge, &mut out);
                    if let Some((a, b)) = edge.endpoints(cx.doc) {
                        for p in [a, b] {
                            out.push(Overlay::Anchor { p, color, filled: true, size: 6.0 });
                        }
                        if cx.measurement_labels
                            && let Ok(text) = self.style.label(a.distance(b), false)
                        {
                            out.push(Overlay::Measure { p: self.at + Vec2::new(12.0, 12.0), text });
                        }
                    }
                }
                Some(Target::Corner { first, second, at }) => {
                    segment(first, &mut out);
                    segment(second, &mut out);
                    out.push(Overlay::Anchor { p: *at, color, filled: true, size: 7.0 });
                }
                Some(Target::Radial { center, radius, .. }) => {
                    out.push(Overlay::Anchor { p: *center, color, filled: false, size: 6.0 });
                    let toward = self.at - *center;
                    if toward.hypot() > 1e-9 {
                        out.push(Overlay::Line { a: *center, b: *center + toward / toward.hypot() * *radius, color, dashed: true });
                    }
                }
                None => {}
            }
        }
        for (p, _) in &self.points {
            out.push(Overlay::Anchor { p: *p, color, filled: true, size: 6.0 });
        }
        for edge in &self.lines {
            segment(edge, &mut out);
        }
        // Linear: show where the next click lands (anchor ring, or a dot on a line), and draw the
        // rubber band to that point, so the user sees the snap before clicking (#2516).
        let linear_pick = self.kind == Kind::Linear && self.pending.is_none() && self.lines.is_empty();
        let snapped = self.snapped.clone().filter(|_| linear_pick);
        if let Some((q, reference)) = &snapped
            && (reference.is_some() || q.distance(self.at) > 1e-9)
        {
            let on_anchor = reference.is_some();
            out.push(Overlay::Anchor { p: *q, color, filled: !on_anchor, size: if on_anchor { 10.0 } else { 6.0 } });
        }
        if let Some((p, _)) = self.points.last() {
            let target = snapped.map_or(self.at, |(q, _)| q);
            let b = if self.shift { constrained(cx, *p, target) } else { target };
            out.push(Overlay::Line { a: *p, b, color, dashed: true });
        }
        // The Smart Guides of the point the next click picks (after the rubber band).
        if linear_pick {
            out.extend(self.snap.guides().iter().cloned());
        }
        out
    }
}
