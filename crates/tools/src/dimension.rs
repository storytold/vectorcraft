//! The Dimension tool. Hover a segment (linear), a corner (angular) or a circle or perfect arc
//! (radial) and drag outwards: releasing places the linked dimension. Plain clicks pick what to
//! measure instead: two points (linear) or two straight lines (angular), then a click places it;
//! a click on a circle or arc gives its radius. Escape cancels without leaving any artwork.
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};
use serde_json::{Value, json};
use vectorcraft_doc::{
    Node, NodeId,
    dimension::{self, Dimension, DimensionMode, DimensionOptions, DimensionSource, FreePoint, Orientation, PointRef, SegmentRef},
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
    /// Whether the press under way began on an anchor a Linear click picks.
    press_on_anchor: bool,
    /// The second point clicked for the Linear dimension being placed, and where the pointer was
    /// for that click (a snap or Shift can move the point away from it): placed with the pointer
    /// still at either, it is aligned.
    second: Option<(Point, Point)>,
}

/// What a linear dimension between `a` and `b` measures with the pointer at `p` while it is
/// placed (VectorCraft's own rule). Beside the points, where the nearest point of the line
/// through them lies between them, the dimension is aligned: it measures the distance between
/// them. Past either point it measures along an axis. The line through the points and the
/// perpendicular at that point bound a quarter on each side; the pointer's quarter holds either
/// the straight up or down direction, which gives the horizontal distance, or the straight left
/// or right direction, which gives the vertical distance. Points on one horizontal or vertical
/// line only give an aligned dimension.
fn orientation_at(a: Point, b: Point, p: Point) -> Orientation {
    let u = b - a;
    let len = u.hypot();
    if len < 1e-9 {
        return Orientation::Aligned;
    }
    let along = (p - a).dot(u) / (len * len);
    if (0.0..=1.0).contains(&along) {
        return Orientation::Aligned;
    }
    // Out past the nearer point, and the side of the line the pointer is on.
    let (out, side) = (if along > 1.0 { u } else { -u }, u.cross(p - a));
    let axes = [
        (Vec2::new(0.0, 1.0), Orientation::Horizontal),
        (Vec2::new(0.0, -1.0), Orientation::Horizontal),
        (Vec2::new(1.0, 0.0), Orientation::Vertical),
        (Vec2::new(-1.0, 0.0), Orientation::Vertical),
    ];
    let within = len * 1e-9;
    axes.into_iter()
        .find(|(axis, _)| axis.dot(out) > within && u.cross(*axis).abs() > within && u.cross(*axis) * side > 0.0)
        .map_or(Orientation::Aligned, |(_, orientation)| orientation)
}

/// How near an anchor (screen pixels) a pick snaps to it.
const ANCHOR_REACH: f64 = 8.0;

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
        .filter(|(_, a)| a.distance(p) <= cx.tol(ANCHOR_REACH))
        .min_by(|(_, a), (_, b)| a.distance(p).total_cmp(&b.distance(p)))
}

/// The anchor nearest `p` within reach, on any editable path (the topmost on a tie), as a link to
/// its path. Bounded work, and dimension artwork is skipped, as in [`pick`].
fn anchor_under(cx: &ToolContext, p: Point) -> Option<(Point, PointRef)> {
    type Best = Option<(f64, Point, PointRef)>;
    fn visit(n: &Node, p: Point, reach: f64, budget: &mut usize, best: &mut Best) {
        if *budget == 0 || !n.visible || n.locked || n.extras().dimension.is_some() {
            return;
        }
        if let Some(path) = n.path_data() {
            for (subpath, sp) in path.subpaths.iter().enumerate() {
                for (anchor, a) in sp.anchors.iter().enumerate() {
                    if *budget == 0 {
                        return;
                    }
                    *budget -= 1;
                    let d = a.p.distance(p);
                    if d <= reach && best.as_ref().is_none_or(|(nearest, ..)| d < *nearest) {
                        *best = Some((d, a.p, PointRef { node: n.id, subpath, anchor }));
                    }
                }
            }
        }
        if let Some(ch) = n.children() {
            for c in ch.iter().rev() {
                visit(c, p, reach, budget, best);
            }
        }
    }
    let (reach, mut budget, mut best) = (cx.tol(ANCHOR_REACH), 20_000usize, None);
    for n in cx.doc.layers.iter().rev() {
        visit(n, p, reach, &mut budget, &mut best);
    }
    best.map(|(_, at, r)| (at, r))
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

/// What a dimension between two picked points links to. A point on an anchor links to the anchor.
/// A point on a segment keeps its place along the segment. Any other point is held
/// against the anchors of the path under it, or, with no path under it, the path of the other
/// point, so that it moves with that path. None when neither point has a path to link to.
fn points_source(cx: &ToolContext, (a, ra): (Point, &Option<PointRef>), (b, rb): (Point, &Option<PointRef>)) -> Option<DimensionSource> {
    let (edge_a, edge_b) = (pick(cx, a), pick(cx, b));
    let node = |r: &Option<PointRef>, e: &Option<SegmentRef>| r.as_ref().map(|r| r.node).or_else(|| e.as_ref().map(|e| e.node));
    let (path_a, path_b) = (node(ra, &edge_a), node(rb, &edge_b));
    let free = |p: Point, r: &Option<PointRef>, edge: &Option<SegmentRef>, own: Option<NodeId>, other: Option<NodeId>| {
        if r.is_some() {
            return None;
        }
        edge.as_ref().and_then(|e| FreePoint::on_segment(cx.doc, e, p)).or_else(|| own.or(other).and_then(|node| FreePoint::on(cx.doc, node, p)))
    };
    let (free_a, free_b) = (free(a, ra, &edge_a, path_a, path_b), free(b, rb, &edge_b, path_b, path_a));
    (ra.is_some() || rb.is_some() || free_a.is_some() || free_b.is_some()).then(|| DimensionSource::Points {
        a: ra.clone(),
        b: rb.clone(),
        vertex: None,
        free_a,
        free_b,
    })
}

impl DimensionTool {
    /// Where a Linear pick at `p` lands: on the nearest anchor of any path, within reach (kept as
    /// a link to its path); else where Smart Guides put it (another object's anchor or centre, in
    /// line with one, or for the second point on a construction guide through the first), its
    /// guides showing; else on the closest point of the line under the pointer; else `p` itself.
    /// A point the guides leave where the pointer is (right on a guide line) stays there, and with
    /// Snap to Pixel or Snap to Grid on a point stays where they place it.
    fn snap_linear(&mut self, cx: &ToolContext, p: Point) -> (Point, Option<PointRef>) {
        let leave = self.points.last().map(|(first, _)| crate::guides::Leave::segment(cx, *first, false));
        let guided = self.snap.hover(cx, p, &[], leave.as_ref());
        if let Some((point, reference)) = anchor_under(cx, p) {
            self.snap.clear();
            return (point, Some(reference));
        }
        if guided.distance(p) > 1e-9 || !self.snap.guides().is_empty() || cx.snap_to_pixel || cx.snap_to_grid {
            return (guided, None);
        }
        // Not on an anchor or a guide: snap to the closest point of the line (#2432).
        (pick(cx, p).and_then(|e| nearest_on(cx, &e, p)).unwrap_or(p), None)
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
        Dimension {
            mode,
            a,
            b,
            vertex,
            offset: 0.0,
            length: a.distance(b),
            options: self.style.clone(),
            source,
            orientation: Orientation::Aligned,
            moved: None,
        }
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
        if d.orientation != Orientation::Aligned {
            p["orientation"] = json!(d.orientation);
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
        if d.mode == DimensionMode::Linear {
            // The points the dimension measures: a dragged segment's ends, read from the art.
            let ends = match &d.source {
                Some(DimensionSource::Segment { edge }) => edge.endpoints(cx.doc),
                _ => Some((d.a, d.b)),
            };
            let again = self.second.is_some_and(|(at, pointer)| at.distance(p).min(pointer.distance(p)) <= cx.tol(ANCHOR_REACH));
            d.orientation = match ends {
                Some((a, b)) if !again => orientation_at(a, b, p),
                _ => Orientation::Aligned,
            };
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
        self.press_on_anchor = false;
        self.points.clear();
        self.lines.clear();
        self.dragging = false;
        self.began = false;
        self.snap.clear();
        self.snapped = None;
        self.second = None;
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
                    let source = points_source(cx, (*a, ra), (*b, rb));
                    let mut d = self.recipe(DimensionMode::Linear, *a, *b, Point::ZERO, source);
                    if d.resolve(cx.doc).is_ok() {
                        self.second = Some((*b, p));
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
                    self.press = Some((ev.pos, None));
                    return self.preview(cx, ev.pos);
                }
                // A Linear press on an anchor picks the anchor unless it drags past the anchor's
                // reach: moving a few pixels before the release does not drag out the dimension of
                // a segment under the pointer.
                self.press_on_anchor = self.kind == Kind::Linear && anchor_under(cx, ev.pos).is_some();
                self.press = Some((ev.pos, self.target(cx, ev.pos)));
                vec![]
            }
            PointerKind::Drag => {
                if self.pending.is_some() && !self.dragging && self.press.is_some() {
                    return self.preview(cx, ev.pos);
                }
                if self.pending.is_none()
                    && let Some((start, Some(target))) = self.press.clone()
                    && start.distance(ev.pos) > cx.tol(if self.press_on_anchor { ANCHOR_REACH } else { 3.0 })
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
                if self.pending.is_some() {
                    return if self.press.take().is_some() { self.place(cx, ev.pos) } else { vec![] };
                }
                match self.press.take() {
                    Some((start, target)) => self.click(cx, start, target, ev.mods),
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
    /// A dropped drag is canceled as by Escape. A dropped click picks nothing, and the points or
    /// lines earlier clicks picked stay.
    fn drop_press(&mut self, cx: &ToolContext) -> Vec<Action> {
        if self.dragging {
            return self.key(cx, ToolKey::Escape, Mods::default());
        }
        self.press = None;
        if self.pending.is_some() && self.began {
            self.began = false;
            vec![Action::Cancel]
        } else {
            vec![]
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    /// A dropped click picks nothing and keeps the point an earlier click picked. A dropped drag
    /// out from a segment is canceled as by Escape.
    #[test]
    fn a_dropped_press_keeps_the_earlier_picks() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let ev = |kind, x: f64, y: f64| PointerEvent::new(kind, x, y);
        let mut t = DimensionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 400.0, 400.0));
        t.pointer(&c, &ev(PointerKind::Up, 400.0, 400.0));
        assert_eq!(t.points.len(), 1);
        // The engine sends the release after the drop.
        t.pointer(&c, &ev(PointerKind::Down, 450.0, 420.0));
        assert!(t.drop_press(&c).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Up, 450.0, 420.0)).is_empty());
        assert_eq!(t.points.len(), 1, "the earlier pick stays and the dropped click picks nothing");
        assert!(t.pending.is_none() && t.busy());
        let mut t = DimensionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 100.0));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 150.0, 60.0));
        assert!(matches!(acts.first(), Some(Action::Begin(_))), "{acts:?}");
        assert_eq!(t.drop_press(&c), vec![Action::Cancel]);
        assert!(!t.busy());
        assert!(t.pointer(&c, &ev(PointerKind::Up, 150.0, 60.0)).is_empty(), "the release places nothing");
    }

    #[test]
    fn a_dropped_placement_press_does_not_commit_the_dimension() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let ev = |kind, x: f64, y: f64| PointerEvent::new(kind, x, y);
        let mut t = DimensionTool::default();
        for (x, y) in [(400.0, 400.0), (450.0, 420.0)] {
            assert!(t.pointer(&c, &ev(PointerKind::Down, x, y)).is_empty());
            assert!(t.pointer(&c, &ev(PointerKind::Up, x, y)).is_empty());
        }
        assert!(t.pending.is_some());
        assert!(matches!(t.pointer(&c, &ev(PointerKind::Move, 470.0, 450.0)).first(), Some(Action::Begin(_))));
        assert!(matches!(t.pointer(&c, &ev(PointerKind::Down, 470.0, 450.0)).last(), Some(Action::Preview(_, _))));
        assert_eq!(t.drop_press(&c), vec![Action::Cancel]);
        assert!(t.pointer(&c, &ev(PointerKind::Up, 470.0, 450.0)).is_empty());
        assert!(t.pending.is_some(), "the dimension can still be placed");
        assert!(matches!(t.pointer(&c, &ev(PointerKind::Down, 480.0, 460.0)).last(), Some(Action::Preview(_, _))));
        let dragged = t.pointer(&c, &ev(PointerKind::Drag, 490.0, 470.0));
        assert!(matches!(dragged.last(), Some(Action::Preview(_, _))), "{dragged:?}");
        let placed = t.pointer(&c, &ev(PointerKind::Up, 490.0, 470.0));
        assert!(matches!(placed.last(), Some(Action::Commit)), "{placed:?}");
    }

    /// A Linear press on an anchor picks that anchor even when the pointer moves a few pixels
    /// before the release: it does not drag out the dimension of a segment that ends there. Both
    /// picks keep their links to the path, so the dimension measures between the two anchors.
    #[test]
    fn a_press_on_an_anchor_picks_it_when_the_pointer_moves() {
        let (d, id) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let ev = |kind, x: f64, y: f64| PointerEvent::new(kind, x, y);
        let mut t = DimensionTool::default();
        for ((x, y), (dx, dy)) in [((100.0, 100.0), (3.5, 1.0)), ((200.0, 200.0), (-4.0, -2.0))] {
            assert!(t.pointer(&c, &ev(PointerKind::Down, x, y)).is_empty());
            assert!(t.pointer(&c, &ev(PointerKind::Drag, x + dx, y + dy)).is_empty(), "no segment dimension is dragged out");
            assert!(t.pointer(&c, &ev(PointerKind::Up, x + dx, y + dy)).is_empty(), "nothing is committed");
        }
        let d = t.pending.as_ref().expect("both anchors were picked");
        assert_eq!((d.a, d.b), (Point::new(100.0, 100.0), Point::new(200.0, 200.0)));
        let anchor = |anchor| Some(PointRef { node: id, subpath: 0, anchor });
        assert_eq!(d.source, Some(DimensionSource::Points { a: anchor(0), b: anchor(2), vertex: None, free_a: None, free_b: None }));
        // Dragged past the anchor's reach, the press drags out the dimension of a segment there.
        let mut t = DimensionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 100.0, 100.0));
        assert!(t.pointer(&c, &ev(PointerKind::Drag, 103.0, 106.0)).is_empty());
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 100.0, 60.0));
        assert!(matches!(acts.first(), Some(Action::Begin(_))), "{acts:?}");
        assert!(matches!(t.pointer(&c, &ev(PointerKind::Up, 100.0, 60.0)).last(), Some(Action::Commit)));
    }

    /// A point picked on a segment, not on an anchor, keeps its place along the segment. A point
    /// off the art is held against the anchors of the other point's path. Both then follow the
    /// path, as the anchor at the other end does.
    #[test]
    fn points_off_anchors_are_held_against_the_path() {
        use vectorcraft_doc::dimension::FreePoint;
        let (d, id) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let click = |t: &mut DimensionTool, x: f64, y: f64| {
            t.pointer(&c, &PointerEvent::new(PointerKind::Down, x, y));
            t.pointer(&c, &PointerEvent::new(PointerKind::Up, x, y));
        };
        let mut t = DimensionTool::default();
        click(&mut t, 100.0, 100.0);
        click(&mut t, 200.0, 150.0);
        let pending = t.pending.as_ref().expect("both points were picked");
        let anchor = Some(PointRef { node: id, subpath: 0, anchor: 0 });
        let Some(DimensionSource::Points { a, b: None, vertex: None, free_a: None, free_b: Some(FreePoint::OnSegment { edge, t }) }) =
            &pending.source
        else {
            panic!("{:?}", pending.source)
        };
        assert_eq!((a, edge), (&anchor, &SegmentRef { node: id, subpath: 0, segment: 1 }));
        assert!((t - 0.5).abs() < 1e-9, "{t}");
        let mut t = DimensionTool::default();
        click(&mut t, 400.0, 400.0);
        click(&mut t, 200.0, 200.0);
        let pending = t.pending.as_ref().expect("both points were picked");
        let Some(DimensionSource::Points { free_a: Some(free), b: Some(b), .. }) = &pending.source else { panic!("{:?}", pending.source) };
        let FreePoint::Anchors { node, anchors, across } = free else { panic!("{free:?}") };
        assert_eq!((*node, anchors.len(), *across, b.node, b.anchor), (id, 3, 0.0, id, 2));
        let at = free.position(&d).expect("the anchors that hold it exist");
        assert!(at.distance(pending.a) < 1e-9, "{at:?} {:?}", pending.a);
    }

    /// A point that Smart Guides put in line with the path's center is held, where the guide put
    /// it, against the anchors of the path at the other end.
    #[test]
    fn a_point_smart_guides_place_is_held_against_the_path() {
        use vectorcraft_doc::dimension::FreePoint;
        let (d, id) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        assert!(c.smart_guides);
        let mut t = DimensionTool::default();
        for (x, y) in [(200.0, 200.0), (151.5, 330.0)] {
            t.pointer(&c, &PointerEvent::new(PointerKind::Move, x, y));
            t.pointer(&c, &PointerEvent::new(PointerKind::Down, x, y));
            t.pointer(&c, &PointerEvent::new(PointerKind::Up, x, y));
        }
        let pending = t.pending.as_ref().expect("both points were picked");
        assert!((pending.b.x - 150.0).abs() < 1e-9, "in line with the center: {:?}", pending.b);
        let Some(DimensionSource::Points { a: Some(a), free_b: Some(free), .. }) = &pending.source else { panic!("{:?}", pending.source) };
        assert!(matches!(free, FreePoint::Anchors { node, .. } if *node == id), "{free:?}");
        assert_eq!(a.node, id);
        let at = free.position(&d).expect("the anchors that hold it exist");
        assert!(at.distance(pending.b) < 1e-9, "{at:?} {:?}", pending.b);
    }

    /// The anchors of a live rectangle are picked the same way.
    #[test]
    fn a_press_on_a_live_shapes_anchor_picks_it() {
        let (d, id) = doc_with_live_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = DimensionTool::default();
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 200.0, 100.0));
        t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 197.0, 102.0));
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 197.0, 102.0));
        let [(at, Some(r))] = t.points.as_slice() else { panic!("{:?}", t.points) };
        assert_eq!((*at, r.node), (Point::new(200.0, 100.0), id));
    }

    /// An anchor of a compound path inside nested groups, lying just inside another object's
    /// edge, is picked and linked, though that edge is nearer the pointer than any of the
    /// compound path's own segments. A press there that moves a few pixels picks it too.
    #[test]
    fn an_anchor_under_another_objects_edge_is_picked() {
        use std::sync::Arc;
        use vectorcraft_doc::{Appearance, NodeKind};
        use vectorcraft_geom::Rect;
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let (outer, inner, compound) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let mut path = vectorcraft_geom::shapes::rectangle(Rect::new(20.0, 20.0, 40.0, 40.0));
        path.subpaths.extend(vectorcraft_geom::shapes::rectangle(Rect::new(150.0, 103.0, 180.0, 130.0)).subpaths);
        let art = Node::path(compound, path, Appearance::default_art());
        let inner_group = Node::new(inner, NodeKind::Group { children: vec![Arc::new(art)], clip: false });
        let group = Node::new(outer, NodeKind::Group { children: vec![Arc::new(inner_group)], clip: false });
        d.insert(Some(l), 0, group).unwrap();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let corner = (Point::new(150.0, 103.0), Some(PointRef { node: compound, subpath: 1, anchor: 0 }));
        let mut t = DimensionTool::default();
        assert_eq!(t.snap_linear(&c, Point::new(151.0, 101.0)), corner);
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 151.0, 101.0));
        assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 155.0, 100.0)).is_empty(), "no segment dimension is dragged out");
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 155.0, 100.0));
        assert_eq!(t.points, vec![corner]);
    }

    /// Beside the two points the dimension is aligned; past either point it is horizontal when
    /// the pointer's quarter holds the straight up or down direction and vertical when it holds
    /// the straight left or right direction. Points on one horizontal or vertical line, a pointer
    /// on the line through the points, and two equal points give aligned.
    #[test]
    fn the_pointer_chooses_aligned_horizontal_or_vertical() {
        use Orientation::{Aligned, Horizontal, Vertical};
        let at = |a: (f64, f64), b: (f64, f64), p: (f64, f64)| orientation_at(Point::new(a.0, a.1), Point::new(b.0, b.1), Point::new(p.0, p.1));
        // A line rising to the right (y grows downwards), and the same points in the other order.
        for (a, b) in [((100.0, 300.0), (200.0, 200.0)), ((200.0, 200.0), (100.0, 300.0))] {
            for (p, want) in [
                ((120.0, 160.0), Aligned),
                ((200.0, 300.0), Aligned),
                ((150.0, 250.0), Aligned),
                ((200.0, 200.0), Aligned),
                ((100.0, 300.0), Aligned),
                ((210.0, 120.0), Horizontal),
                ((240.0, 150.0), Horizontal),
                ((280.0, 220.0), Vertical),
                ((240.0, 170.0), Vertical),
                ((40.0, 310.0), Vertical),
                ((90.0, 380.0), Horizontal),
                ((250.0, 150.0), Aligned),
            ] {
                assert_eq!(at(a, b, p), want, "{a:?} to {b:?}, pointer at {p:?}");
            }
        }
        // A line falling to the right.
        let (a, b) = ((100.0, 100.0), (300.0, 200.0));
        for (p, want) in
            [((200.0, 50.0), Aligned), ((320.0, 300.0), Horizontal), ((400.0, 210.0), Vertical), ((60.0, 20.0), Horizontal), ((20.0, 90.0), Vertical)]
        {
            assert_eq!(at(a, b, p), want, "pointer at {p:?}");
        }
        for (a, b) in [((100.0, 100.0), (300.0, 100.0)), ((100.0, 100.0), (100.0, 300.0)), ((100.0, 100.0), (100.0, 100.0))] {
            for p in [(50.0, 50.0), (350.0, 50.0), (350.0, 350.0), (50.0, 350.0), (200.0, 60.0), (60.0, 200.0), (400.0, 100.0), (100.0, 400.0)] {
                assert_eq!(at(a, b, p), Aligned, "{a:?} to {b:?}, pointer at {p:?}");
            }
        }
    }

    /// A point that Smart Guides leave where the pointer already is (the pointer right on a
    /// construction guide or in line with a target) stays on the guide the overlays show. It
    /// does not move to the nearest point of a segment within the pick's reach. Off the guides,
    /// a point near a segment still lands on it.
    #[test]
    fn a_point_on_a_guide_stays_on_the_guide() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let click = |t: &mut DimensionTool, x: f64, y: f64| {
            t.pointer(&c, &PointerEvent::new(PointerKind::Move, x, y));
            t.pointer(&c, &PointerEvent::new(PointerKind::Down, x, y));
            t.pointer(&c, &PointerEvent::new(PointerKind::Up, x, y));
        };
        let mut t = DimensionTool::default();
        // In line with the rectangle's center, 6 units right of its right edge.
        assert_eq!(t.snap_linear(&c, Point::new(206.0, 150.0)), (Point::new(206.0, 150.0), None));
        assert!(!t.snap.guides().is_empty());
        // Off the guides, beside the edge: the closest point of the edge.
        let (on_edge, reference) = t.snap_linear(&c, Point::new(206.0, 170.5));
        assert!(on_edge.distance(Point::new(200.0, 170.5)) < 1e-4 && reference.is_none(), "{on_edge:?}");
        // The first point, above the rectangle, then straight down along the vertical
        // construction guide through it, past the right edge.
        click(&mut t, 206.0, 50.0);
        assert_eq!(t.points, vec![(Point::new(206.0, 50.0), None)]);
        for y in [150.0, 170.5, 190.0] {
            assert_eq!(t.snap_linear(&c, Point::new(206.0, y)), (Point::new(206.0, y), None), "on the construction guide at y = {y}");
            assert!(!t.snap.guides().is_empty());
        }
        click(&mut t, 206.0, 170.5);
        let dim = t.pending.as_ref().expect("both points were picked");
        assert_eq!((dim.a, dim.b), (Point::new(206.0, 50.0), Point::new(206.0, 170.5)));
        // The second point, off the edge, is held against the rectangle's anchors.
        let Some(DimensionSource::Points { free_b: Some(free), .. }) = &dim.source else { panic!("{:?}", dim.source) };
        assert!(matches!(free, FreePoint::Anchors { .. }), "{free:?}");
        // A pointer already on a gridline crossing or on a whole pixel stays there too, and one
        // off the grid lands on the grid, not on the edge.
        let grid = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
        let step = grid.grid_step();
        let on_grid = Point::new((200.0 / step).ceil() * step, (170.0 / step).round() * step);
        assert!(on_grid.x > 200.0 + 1e-6 && on_grid.x - 200.0 < 8.0, "{on_grid:?} is off the edge, within the pick's reach");
        assert_eq!(DimensionTool::default().snap_linear(&grid, on_grid), (on_grid, None));
        let off_grid = on_grid + Vec2::new(-step * 0.3, 0.2);
        assert_eq!(DimensionTool::default().snap_linear(&grid, off_grid), (on_grid, None));
        let pixel = ToolContext { snap_to_pixel: true, ..cx(&d, &s, &p) };
        assert_eq!(DimensionTool::default().snap_linear(&pixel, Point::new(206.0, 170.0)), (Point::new(206.0, 170.0), None));
    }

    #[test]
    fn placement_press_previews_without_a_prior_move() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = DimensionTool::default();
        for (x, y) in [(400.0, 400.0), (450.0, 420.0)] {
            assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, x, y)).is_empty());
            assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, x, y)).is_empty());
        }
        assert!(t.pending.is_some());
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Down, 470.0, 450.0));
        assert!(matches!(acts.first(), Some(Action::Begin(_))), "{acts:?}");
        assert!(matches!(acts.last(), Some(Action::Preview(_, _))), "{acts:?}");
    }
}
